//! Streaming record-format boundary conformance (main spec §5.6–§5.8, §18,
//! §19, §20, §40; §50 tests 17, 18, 19, 33, 37): the GENERATED server router
//! for fixture 15 is served over real TCP and driven by the GENERATED
//! reqwest client, exercising SSE/NDJSON/JSON-seq in both directions plus
//! the committed-stream failure contract.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use openapi_conformance::fixtures::fixture_15_streams as fx15;
use openapi_support::hooks::{EncodeOverflowHook, NoOpStreamFailureHook, StreamFailureHook};
use openapi_support::limits::BodyLimits;
use openapi_support::stream_errors::ServerStreamError;

// ----------------------------------------------------------------------
// Shared recording stream-failure hook (§40 step 3)
// ----------------------------------------------------------------------

#[derive(Default)]
struct RecordingStreamHook {
    calls: Mutex<Vec<(String, String)>>,
}

impl StreamFailureHook for RecordingStreamHook {
    fn on_stream_failure(&self, operation_id: &str, error: &(dyn std::error::Error + Send + Sync)) {
        // Walk the source chain so errors WRAPPED by generated machinery
        // (e.g. `ServerStreamError`) report their root cause too.
        let mut chain = vec![error.to_string()];
        let mut source = std::error::Error::source(error);
        while let Some(current) = source {
            chain.push(current.to_string());
            source = current.source();
        }
        self.calls
            .lock()
            .expect("hook lock")
            .push((operation_id.to_owned(), chain.join(" :: ")));
    }
}

/// Counts produced items and reports drops, so cancellation is observable.
struct CountingProducer {
    emitted: Arc<AtomicUsize>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for CountingProducer {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

// ----------------------------------------------------------------------
// Fixture 15 application
// ----------------------------------------------------------------------

struct StreamsApp {
    producer: Mutex<Option<CountingProducer>>,
    received_metrics: Mutex<Vec<fx15::models::Metric>>,
}

impl StreamsApp {
    fn new() -> Self {
        Self {
            producer: Mutex::new(None),
            received_metrics: Mutex::new(Vec::new()),
        }
    }

    fn install_producer(&self, producer: CountingProducer) {
        *self.producer.lock().expect("producer lock") = Some(producer);
    }

    fn take_producer(&self) -> Option<CountingProducer> {
        self.producer.lock().expect("producer lock").take()
    }
}

#[async_trait]
impl fx15::server::Api for StreamsApp {
    async fn export_records(&self) -> fx15::server::ExportRecordsResponse {
        let emitted = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // The producer keeps yielding until dropped by the encoder/consumer;
        // `emitted` counts every item handed to the generated §40 encoder.
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<fx15::models::Record, ServerStreamError>>(1);
        self.install_producer(CountingProducer {
            emitted: Arc::clone(&emitted),
            dropped: Arc::clone(&dropped),
        });
        tokio::spawn(async move {
            // Keep the guard alive for the whole task lifetime.
            let _guard = CountingProducer {
                emitted: Arc::clone(&emitted),
                dropped: Arc::clone(&dropped),
            };
            let mut seq = 0_u32;
            loop {
                seq += 1;
                // Records grow past any small per-record bound eventually,
                // which t19 uses to force a mid-stream rejection.
                let id = if seq <= 5 {
                    format!("r-{seq}")
                } else {
                    format!("r-{seq}-{pad}", pad = "x".repeat(300))
                };
                let record = fx15::models::Record {
                    id,
                    value: openapi_support::optional::OptionalField::Present(f64::from(seq)),
                };
                if tx.send(Ok(record)).await.is_err() {
                    return;
                }
                emitted.fetch_add(1, Ordering::SeqCst);
            }
        });
        // The erased alias is fed from the receiver side.
        let stream = tokio_stream_from_receiver(rx);
        fx15::server::ExportRecordsResponse::Ok200(Box::pin(stream))
    }

    async fn stream_events(&self) -> fx15::server::StreamEventsResponse {
        // Paced production: each event waits, so items MUST arrive
        // incrementally at the client (§50 test 17).
        let events: Vec<fx15::models::Event> = (0..4_u64)
            .map(|seq| fx15::models::Event {
                seq: i64::try_from(seq).expect("small seq"),
                payload: openapi_support::optional::OptionalField::Present(format!("tick-{seq}")),
            })
            .collect();
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<fx15::models::Event, ServerStreamError>>(1);
        tokio::spawn(async move {
            for event in events {
                tokio::time::sleep(Duration::from_millis(60)).await;
                if tx.send(Ok(event)).await.is_err() {
                    return;
                }
            }
        });
        fx15::server::StreamEventsResponse::Ok200(Box::pin(tokio_stream_from_receiver(rx)))
    }

    async fn stream_envelope_events(&self) -> fx15::server::StreamEnvelopeEventsResponse {
        // §18.1 override: the wire item type is EventPayload even though the
        // schema documents an envelope.
        let payload = fx15::models::EventPayload {
            code: 7,
            message: openapi_support::optional::OptionalField::Present("override".to_owned()),
        };
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<fx15::models::EventPayload, ServerStreamError>>(1);
        tokio::spawn(async move {
            for _ in 0..2 {
                if tx
                    .send(Ok(fx15::models::EventPayload::clone(&payload)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        fx15::server::StreamEnvelopeEventsResponse::Ok200(Box::pin(tokio_stream_from_receiver(rx)))
    }

    async fn push_metrics(
        &self,
        mut body: fx15::server::PushMetricsJsonSeqInput,
    ) -> fx15::server::PushMetricsResponse {
        // Drain the streamed request in order (§6 request-direction streams).
        let mut received = Vec::new();
        while let Some(metric) = body.next_item().await.expect("streamed metric") {
            received.push(metric);
        }
        *self.received_metrics.lock().expect("metrics lock") = received.clone();
        fx15::server::PushMetricsResponse::Accepted202(fx15::models::Ack {
            accepted: true,
            received: openapi_support::optional::OptionalField::Present(
                i32::try_from(received.len()).unwrap_or(i32::MAX),
            ),
        })
    }
}

/// Adapts a tokio mpsc receiver into the boxed erased stream the generated
/// aliases expect (no extra dependencies beyond the conformance crate's).
fn tokio_stream_from_receiver<T: Send + 'static>(
    mut rx: tokio::sync::mpsc::Receiver<Result<T, ServerStreamError>>,
) -> impl futures_core::Stream<Item = Result<T, ServerStreamError>> + Send {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx).map(|item| item.or(None)))
}

fn spawn_with_hooks(
    app: Arc<StreamsApp>,
    stream_hook: Arc<dyn StreamFailureHook>,
) -> std::net::SocketAddr {
    let limits = BodyLimits::process_default();
    let encode_hook: Arc<dyn EncodeOverflowHook> =
        Arc::new(openapi_support::hooks::NoOpEncodeOverflowHook);
    common::spawn_router(fx15::server::router(app, limits, encode_hook, stream_hook))
}

fn client(address: std::net::SocketAddr, limits: BodyLimits) -> fx15::client::Client {
    fx15::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .limits(limits)
        .build()
        .expect("client builds")
}

// ----------------------------------------------------------------------
// t17 — SSE produces events incrementally (§50 test 17)
// ----------------------------------------------------------------------

#[tokio::test]
async fn t17_sse_events_arrive_incrementally() {
    let app = Arc::new(StreamsApp::new());
    let address = spawn_with_hooks(Arc::clone(&app), Arc::new(NoOpStreamFailureHook));
    let http = client(address, BodyLimits::process_default());

    let response = http.stream_events().await.expect("200 documented");
    let fx15::client::StreamEventsResponse::Ok200(wrapper) = response else {
        panic!("expected Ok200");
    };
    let started = std::time::Instant::now();
    let mut arrivals = Vec::new();
    let mut stream = Box::pin(wrapper.into_sse_stream());
    while let Some(item) = stream.next().await {
        let event = item.expect("well-formed event");
        arrivals.push((started.elapsed(), event.seq));
    }

    assert_eq!(arrivals.len(), 4, "all four paced events arrive");
    // The server sleeps 60 ms between items: arrival gaps prove incremental
    // delivery rather than one aggregated body.
    for window in arrivals.windows(2) {
        let gap = window[1].0.saturating_sub(window[0].0);
        assert!(
            gap >= Duration::from_millis(30),
            "events must not arrive together (gap {gap:?})"
        );
    }
    assert_eq!(
        arrivals.iter().map(|(_, seq)| *seq).collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "in-order delivery"
    );
}

// ----------------------------------------------------------------------
// t18 — NDJSON round trip across arbitrary byte boundaries (§50 test 18)
// ----------------------------------------------------------------------

#[tokio::test]
async fn t18_ndjson_round_trip_is_chunk_boundary_independent() {
    let app = Arc::new(StreamsApp::new());
    let address = spawn_with_hooks(app, Arc::new(NoOpStreamFailureHook));
    let http = client(address, BodyLimits::process_default());

    let response = http.export_records().await.expect("200 documented");
    let fx15::client::ExportRecordsResponse::Ok200(wrapper) = response else {
        panic!("expected Ok200");
    };
    let mut stream = Box::pin(wrapper.into_ndjson_stream());
    let mut seen = Vec::new();
    for _ in 0..25 {
        let record = stream.next().await.expect("item").expect("decodes");
        seen.push(record);
    }
    // The unbounded producer never ends on its own; drop to cancel it.
    drop(stream);
    assert_eq!(seen[0].id, "r-1");
    let numeric = |record: &fx15::models::Record| -> u32 {
        record
            .id
            .trim_start_matches("r-")
            .split('-')
            .next()
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(0)
    };
    assert!(
        seen.windows(2)
            .all(|w| numeric(&w[0]) + 1 == numeric(&w[1])),
        "in order"
    );

    // Boundary independence at the integration level: every single split of
    // a canonical NDJSON body decodes identically to the unsplit run.
    let records: Vec<fx15::models::Record> = (1..=3)
        .map(|seq| fx15::models::Record {
            id: format!("r-{seq}"),
            value: openapi_support::optional::OptionalField::Present(f64::from(seq)),
        })
        .collect();
    let mut body = Vec::new();
    for record in &records {
        let json = serde_json::to_vec(record).expect("serialize");
        body.extend_from_slice(&json);
        body.push(b'\n');
    }
    for offset in 1..body.len() {
        let parts: Vec<Result<bytes::Bytes, std::io::Error>> = vec![
            Ok(bytes::Bytes::copy_from_slice(&body[..offset])),
            Ok(bytes::Bytes::copy_from_slice(&body[offset..])),
        ];
        let decoded: Vec<fx15::models::Record> =
            openapi_support::ndjson::decode_ndjson::<fx15::models::Record, _, _>(
                futures_util::stream::iter(parts),
                BodyLimits::process_default().max_stream_record_bytes,
            )
            .map(|item| item.expect("decodes"))
            .collect()
            .await;
        assert_eq!(decoded, records, "split at byte {offset}");
    }
}

// ----------------------------------------------------------------------
// t19 — oversized single record rejects without draining the producer
//       (§50 test 19)
// ----------------------------------------------------------------------

#[tokio::test]
async fn t19_oversized_record_rejects_and_cancels_the_producer() {
    let app = Arc::new(StreamsApp::new());
    let address = spawn_with_hooks(Arc::clone(&app), Arc::new(NoOpStreamFailureHook));

    // Tiny per-record limit on the CLIENT only: the first five records fit,
    // the grown ones do not.
    let tiny = BodyLimits {
        max_stream_record_bytes: 64,
        ..BodyLimits::process_default()
    };
    let http = client(address, tiny);

    let response = http.export_records().await.expect("200 documented");
    let fx15::client::ExportRecordsResponse::Ok200(wrapper) = response else {
        panic!("expected Ok200");
    };
    let mut stream = Box::pin(wrapper.into_ndjson_stream());
    let first = stream.next().await.expect("first item").expect("fits");
    assert_eq!(first.id, "r-1");

    let mut delivered = 1_usize;
    let error = loop {
        match stream.next().await {
            Some(Ok(_)) => delivered += 1,
            Some(Err(error)) => break error,
            None => panic!("must terminate with RecordTooLarge, not clean EOF"),
        }
    };
    assert!(delivered >= 2, "some records precede the oversized one");
    drop(stream);
    assert!(
        matches!(
            error,
            openapi_support::stream_errors::NdjsonDecodeError::RecordTooLarge { limit: 64 }
        ),
        "expected RecordTooLarge {{ limit: 64 }}, got {error:?}"
    );

    // Cancellation propagates asynchronously (client drop → hyper → body
    // stream → producer), so poll the counter to rest: an unbounded
    // producer would keep counting forever.
    let producer = app.take_producer().expect("producer registered");
    let mut previous = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let current = producer.emitted.load(Ordering::SeqCst);
        if previous == Some(current) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "producer never stopped (emitted {current})"
        );
        assert!(
            current < 10_000,
            "emitted count must stay bounded ({current})"
        );
        previous = Some(current);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(producer.dropped.load(Ordering::SeqCst), "producer dropped");
}

// ----------------------------------------------------------------------
// t33 — post-commit failure terminates abruptly; hook observes (§50 test 33)
// ----------------------------------------------------------------------

#[tokio::test]
async fn t33_app_failure_mid_stream_fires_hook_and_truncates_client() {
    struct FailingApp {
        hook_calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl fx15::server::Api for FailingApp {
        async fn export_records(&self) -> fx15::server::ExportRecordsResponse {
            let hook_calls = Arc::clone(&self.hook_calls);
            let (tx, rx) =
                tokio::sync::mpsc::channel::<Result<fx15::models::Record, ServerStreamError>>(1);
            tokio::spawn(async move {
                for seq in 1..=3 {
                    let record = fx15::models::Record {
                        id: format!("r-{seq}"),
                        value: openapi_support::optional::OptionalField::Absent,
                    };
                    if tx.send(Ok(record)).await.is_err() {
                        return;
                    }
                }
                // §40 step 4 violated deliberately: fail DURING production.
                let _ = tx
                    .send(Err(ServerStreamError::new("mid-production failure")))
                    .await;
                let _ = hook_calls; // observed via injected router hook below
            });
            fx15::server::ExportRecordsResponse::Ok200(Box::pin(tokio_stream_from_receiver(rx)))
        }

        async fn stream_events(&self) -> fx15::server::StreamEventsResponse {
            unreachable!("not exercised");
        }

        async fn stream_envelope_events(&self) -> fx15::server::StreamEnvelopeEventsResponse {
            unreachable!("not exercised");
        }

        async fn push_metrics(
            &self,
            _body: fx15::server::PushMetricsJsonSeqInput,
        ) -> fx15::server::PushMetricsResponse {
            unreachable!("not exercised");
        }
    }

    let hook = Arc::new(RecordingStreamHook::default());
    let encode_hook: Arc<dyn EncodeOverflowHook> =
        Arc::new(openapi_support::hooks::NoOpEncodeOverflowHook);
    let app = Arc::new(FailingApp {
        hook_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let address = common::spawn_router(fx15::server::router(
        Arc::clone(&app) as Arc<dyn fx15::server::Api>,
        BodyLimits::process_default(),
        encode_hook,
        Arc::clone(&hook) as Arc<dyn StreamFailureHook>,
    ));

    let http = client(address, BodyLimits::process_default());
    let response = http.export_records().await.expect("200 documented");
    let fx15::client::ExportRecordsResponse::Ok200(wrapper) = response else {
        panic!("expected Ok200");
    };

    let mut stream = Box::pin(wrapper.into_ndjson_stream());
    let mut delivered = 0_usize;
    let terminal = loop {
        match stream.next().await {
            Some(Ok(_)) => delivered += 1,
            Some(Err(error)) => break error,
            None => panic!("clean end-of-stream after mid-production failure"),
        }
    };
    let _ = &app;

    assert!(
        delivered >= 2,
        "at least two items must be delivered before the failure"
    );
    // The client observes an EXPLICIT terminal decode error — never a clean
    // `None` masquerading as success (§40 client-visible effect): the abrupt
    // abort surfaces through reqwest/hyper as a read-side body failure rooted
    // in an EOF io error (`transport_classify` rule 2), which classifies as
    // `Truncated`, distinct from both clean EOF and every other transport
    // failure (`Source`).
    let detail = terminal.to_string();
    assert!(
        matches!(
            terminal,
            openapi_support::stream_errors::NdjsonDecodeError::Truncated
        ),
        "terminal error must be `Truncated` for an abrupt post-commit abort, \
         distinct from clean EOF and from `Source`: {detail}"
    );

    // §40 step 3: the configured stream-failure hook fires with the
    // operation id before the body is dropped.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = hook.calls.lock().expect("hook lock").clone();
        if !snapshot.is_empty() {
            assert_eq!(snapshot[0].0, "exportRecords", "operation id reported");
            assert!(
                snapshot[0].1.contains("mid-production failure"),
                "hook carries the application error: {:?}",
                snapshot
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "hook must fire promptly: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ----------------------------------------------------------------------
// t37 — SSE framing semantics (§50 test 37, §18.2)
// ----------------------------------------------------------------------

#[tokio::test]
async fn t37_sse_multiline_joins_comments_skip_and_malformed_terminates() {
    use openapi_support::stream_errors::SseDecodeError;

    #[derive(Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    struct Payload {
        text: String,
    }

    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = vec![
        // Comment-only event: skipped entirely.
        Ok(bytes::Bytes::from_static(b": keep-alive\n\n")),
        // Metadata fields are ignored; multi-line data joins with \n
        // (the split sits BETWEEN JSON tokens, so the join is whitespace).
        Ok(bytes::Bytes::from_static(
            b"id: 42\nevent: tick\ndata: {\"text\":\ndata: \"line1-line2\"}\n\n",
        )),
        // Malformed JSON terminates the stream fail-fast.
        Ok(bytes::Bytes::from_static(b"data: {broken}\n\n")),
        // Never reached: no skip-and-continue (§18.2).
        Ok(bytes::Bytes::from_static(b"data: {\"text\":\"after\"}\n\n")),
    ];

    let decoded: Vec<Result<Payload, SseDecodeError>> =
        openapi_support::sse::decode_sse_json::<Payload, _, _>(
            futures_util::stream::iter(chunks),
            1024,
        )
        .collect()
        .await;

    match decoded.first() {
        Some(Ok(payload)) => {
            assert_eq!(
                payload.text, "line1-line2",
                "multi-line data joins with \n before ONE parse"
            );
        }
        other => panic!("first event must decode: {other:?}"),
    }
    match &decoded[1] {
        Err(SseDecodeError::MalformedJson(_)) => {}
        other => panic!("malformed event must terminate with MalformedJson: {other:?}"),
    }
    // Fail-fast: nothing after the malformed event is surfaced.
    assert_eq!(
        decoded.len(),
        2,
        "no skip-and-continue after malformed JSON"
    );

    // The generated SSE encoder round-trips through the decoder.
    let encoded = openapi_support::sse::encode_sse_event(
        &Payload {
            text: "hello".to_owned(),
        },
        256,
    )
    .expect("under limit");
    let round: Vec<Payload> = openapi_support::sse::decode_sse_json::<Payload, _, _>(
        futures_util::stream::iter(vec![Ok::<_, std::io::Error>(encoded)]),
        256,
    )
    .map(|item| item.expect("decodes"))
    .collect()
    .await;
    assert_eq!(
        round,
        vec![Payload {
            text: "hello".to_owned()
        }]
    );
}

// ----------------------------------------------------------------------
// Request direction — pushMetrics round trip + oversized item mapping
// ----------------------------------------------------------------------

#[tokio::test]
async fn push_metrics_round_trips_json_seq_items_in_order() {
    let app = Arc::new(StreamsApp::new());
    let address = spawn_with_hooks(Arc::clone(&app), Arc::new(NoOpStreamFailureHook));
    let http = client(address, BodyLimits::process_default());

    let sent: Vec<fx15::models::Metric> = ["latency", "throughput", "errors"]
        .iter()
        .enumerate()
        .map(|(index, name)| fx15::models::Metric {
            name: (*name).to_owned(),
            value: openapi_support::optional::OptionalField::Present(index as f64 * 1.5),
        })
        .collect();
    let cloned = sent.clone();
    let body: fx15::client::PushMetricsJsonSeqBody = Box::pin(futures_util::stream::iter(cloned));
    let response = http.push_metrics(body).await.expect("202 documented");
    let fx15::client::PushMetricsResponse::Accepted202(ack) = response;
    assert!(ack.accepted);
    assert_eq!(
        ack.received,
        openapi_support::optional::OptionalField::Present(3)
    );
    assert_eq!(
        *app.received_metrics.lock().expect("metrics lock"),
        sent,
        "items arrive decoded and in order"
    );
}

#[tokio::test]
async fn oversized_first_request_item_returns_body_too_large_before_sending() {
    let app = Arc::new(StreamsApp::new());
    let address = spawn_with_hooks(app, Arc::new(NoOpStreamFailureHook));

    // Client-side per-item bound small enough that the FIRST item overflows:
    // the eager head encode returns BodyTooLarge BEFORE any wire traffic
    // (§34.2 analog documented on `<Op><Framing>Body`).
    let tiny = BodyLimits {
        max_stream_record_bytes: 8,
        ..BodyLimits::process_default()
    };
    let http = client(address, tiny);

    let big = fx15::models::Metric {
        name: "x".repeat(64),
        value: openapi_support::optional::OptionalField::Absent,
    };
    let body: fx15::client::PushMetricsJsonSeqBody =
        Box::pin(futures_util::stream::iter(vec![big]));
    let error = http.push_metrics(body).await.expect_err("head overflow");
    assert!(
        matches!(
            error,
            openapi_support::client_error::ClientError::BodyTooLarge {
                direction: openapi_support::client_error::BodyLimitDirection::Encode,
                limit: 8,
            }
        ),
        "documented mapping: {error:?}"
    );
}

// One-off empirical probe (kept as a test so the mapping stays pinned):
// abrupt server termination over real hyper surfaces through reqwest as
// "error decoding response body" → hyper "error reading a body from
// connection" → io "unexpected EOF during chunk size line"; the hyper link's
// incomplete-message state classifies the failure as a premature body end, so
// the generated adapter pins `Truncated` (§40) — never clean `None`, and no
// longer `Source`. A body ending cleanly on a record boundary still yields
// `Ok(None)` (pinned by t17/t18).
#[tokio::test]
async fn probe_which_terminal_variant_hyper_surfaces() {
    struct AbortApp;
    use openapi_support::optional::OptionalField;

    #[async_trait]
    impl fx15::server::Api for AbortApp {
        async fn export_records(&self) -> fx15::server::ExportRecordsResponse {
            let (tx, rx) =
                tokio::sync::mpsc::channel::<Result<fx15::models::Record, ServerStreamError>>(1);
            tokio::spawn(async move {
                for seq in 1..=3 {
                    let _ = tx
                        .send(Ok(fx15::models::Record {
                            id: format!("r-{seq}"),
                            value: OptionalField::Absent,
                        }))
                        .await;
                }
                let _ = tx.send(Err(ServerStreamError::new("boom"))).await;
            });
            fx15::server::ExportRecordsResponse::Ok200(Box::pin(tokio_stream_from_receiver(rx)))
        }
        async fn stream_events(&self) -> fx15::server::StreamEventsResponse {
            unreachable!()
        }
        async fn stream_envelope_events(&self) -> fx15::server::StreamEnvelopeEventsResponse {
            unreachable!()
        }
        async fn push_metrics(
            &self,
            _: fx15::server::PushMetricsJsonSeqInput,
        ) -> fx15::server::PushMetricsResponse {
            unreachable!()
        }
    }

    let encode_hook: Arc<dyn EncodeOverflowHook> =
        Arc::new(openapi_support::hooks::NoOpEncodeOverflowHook);
    let address = common::spawn_router(fx15::server::router(
        Arc::new(AbortApp),
        BodyLimits::process_default(),
        encode_hook,
        Arc::new(NoOpStreamFailureHook),
    ));
    let http = client(address, BodyLimits::process_default());
    let response = http.export_records().await.expect("200");
    let fx15::client::ExportRecordsResponse::Ok200(wrapper) = response else {
        panic!()
    };
    use futures_util::StreamExt as _;
    let mut stream = Box::pin(wrapper.into_ndjson_stream());
    let mut terminal = None;
    while let Some(item) = stream.next().await {
        if item.is_err() {
            terminal = item.err();
        }
    }
    match terminal {
        Some(openapi_support::stream_errors::NdjsonDecodeError::Truncated) => {
            println!("PROBE: Truncated");
        }
        Some(other) => panic!("expected Truncated for the abrupt abort, got {other:?}"),
        None => panic!("clean end-of-stream after an abrupt post-commit abort"),
    }
}
