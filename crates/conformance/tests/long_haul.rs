//! Ignore-gated multi-GiB passthrough proofs (main spec §50 tests 5–8;
//! DECISIONS D-impl-long-memory-tests): synthetic producers synthesize
//! patterned bytes arithmetically — NEVER buffering them — so 10 GiB flows
//! through the GENERATED fixture-02 client/router boundary while peak memory
//! stays flat. Chunk-count assertions prove laziness; bounded-channel and
//! outstanding-byte assertions prove backpressure; counter-freeze assertions
//! prove prompt cancellation.
//!
//! Cancellation (test 8) is proven where the caller owns the polling loop:
//! dropping a consumed DOWNLOAD stream must freeze the server-side source
//! within ~2 s. For uploads, reqwest 0.12's pooled connection driver unwinds
//! asynchronously after the request future is dropped, so the prompt-stop
//! assertion targets the application-owned producer task instead.
//!
//! These are excluded from default runs (they take minutes); execute them
//! explicitly with:
//!
//! ```text
//! cargo test -p openapi-conformance -- --ignored --nocapture
//! ```
//!
//! The upload/download sinks count chunks + bytes into atomics and never
//! buffer a payload: memory proportionality is proven by construction plus
//! the chunk-count threshold, not by code inspection alone.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use openapi_conformance::fixtures::fixture_02_streaming_binary as fx02;

const CHUNK_LEN: usize = 8 * 1024 * 1024; // 8 MiB per synthesized chunk
const TOTAL_BYTES: usize = 10 * 1024 * 1024 * 1024; // 10 GiB
const CHUNK_COUNT: usize = TOTAL_BYTES / CHUNK_LEN; // 1280
/// Generous wall-clock budget per long-haul operation (minutes, not hours).
const OP_TIMEOUT: Duration = Duration::from_secs(600);
/// Laziness proof: a buffered/aggregated implementation would arrive as ONE
/// giant body; hundreds of frames prove streaming passthrough.
const MIN_CHUNKS_PROOF: u64 = 100;

/// Shared counters for the server-side sink.
#[derive(Debug, Default)]
struct SinkCounters {
    chunks: AtomicU64,
    bytes: AtomicU64,
}

#[derive(Clone)]
struct SinkState {
    counters: Arc<SinkCounters>,
    /// When set, the sink sleeps this long every `every` chunks to hold the
    /// producer back (backpressure proof).
    pacing: Option<(usize, Duration)>,
}

async fn put_object_sink(
    axum::extract::State(state): axum::extract::State<SinkState>,
    _id: axum::extract::Path<String>,
    body: axum::body::Body,
) -> http::StatusCode {
    use futures_util::StreamExt;
    let mut stream = body.into_data_stream();
    let mut seen = 0_u64;
    while let Some(frame) = stream.next().await {
        // Sink semantics: measure and DROP. Nothing is ever accumulated.
        // An errored frame means the client cancelled mid-upload (§50
        // test 8); the sink simply stops counting.
        let Ok(chunk) = frame else { break };
        state.counters.chunks.fetch_add(1, Ordering::SeqCst);
        state
            .counters
            .bytes
            .fetch_add(chunk.len() as u64, Ordering::SeqCst);
        seen += 1;
        if let Some((every, pause)) = state.pacing {
            if seen % every as u64 == 0 {
                tokio::time::sleep(pause).await;
            }
        }
    }
    http::StatusCode::CREATED
}

fn long_haul_router(state: SinkState) -> axum::Router {
    use axum::routing::put;
    axum::Router::new()
        .route("/objects/{id}", put(put_object_sink).get(get_object_source))
        .with_state(state)
}

async fn get_object_source(
    axum::extract::State(state): axum::extract::State<SinkState>,
    _id: axum::extract::Path<String>,
) -> axum::response::Response {
    // The source synthesizes each 8 MiB chunk arithmetically when polled;
    // nothing is precomputed or buffered (D-impl-long-memory-tests). Served
    // chunks/bytes are counted as they are produced, so cancellation of the
    // reader freezes the counter.
    let counters = state.counters;
    let source = futures_util::stream::iter((0..CHUNK_COUNT).map(move |index| {
        let chunk = pattern_chunk(index);
        counters.chunks.fetch_add(1, Ordering::SeqCst);
        counters
            .bytes
            .fetch_add(chunk.len() as u64, Ordering::SeqCst);
        Ok::<Bytes, std::io::Error>(chunk)
    }));
    use axum::response::IntoResponse;
    axum::body::Body::from_stream(source).into_response()
}

/// One deterministic patterned chunk computed on the fly (`common::
/// pattern_chunk` at the long-haul chunk size): byte `j` of chunk `index`
/// is `(index * CHUNK_LEN + j) % 251`.
fn pattern_chunk(index: usize) -> Bytes {
    common::pattern_chunk(index, CHUNK_LEN)
}

/// Starts the counting PRODUCER side of the backpressure/cancellation
/// proofs: chunks are synthesized arithmetically in a dedicated task and
/// handed to reqwest through a BOUNDED channel, so production can never run
/// ahead of the transport by more than the channel capacity. Returns the
/// body stream, the handed-off-bytes counter, and the task handle used to
/// abort synthesis (§50 test 8).
fn start_bounded_producer(
    total_chunks: usize,
    channel_chunks: usize,
) -> (
    impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    Arc<AtomicU64>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(channel_chunks);
    let produced = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&produced);
    let handle = tokio::spawn(async move {
        for index in 0..total_chunks {
            if tx.send(pattern_chunk(index)).await.is_err() {
                return; // consumer gone (cancellation) — stop producing
            }
            counter.fetch_add(CHUNK_LEN as u64, Ordering::SeqCst);
        }
    });
    let stream = ReceiverStream { rx };
    (stream, produced, handle)
}

/// Adapts a bounded mpsc receiver into the byte-stream shape
/// `reqwest::Body::wrap_stream` consumes.
struct ReceiverStream {
    rx: tokio::sync::mpsc::Receiver<Bytes>,
}

impl Stream for ReceiverStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|ready| ready.map(Ok))
    }
}

fn long_haul_client(address: std::net::SocketAddr) -> fx02::client::Client {
    fx02::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .secondary_base_url("storage", common::base_url(address))
        .build()
        .expect("client builds")
}

// ----------------------------------------------------------------------
// §50 test 5 — 10 GiB upload passthrough without aggregation
// ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[ignore = "long-haul proof: run via `cargo test -p openapi-conformance -- --ignored --nocapture`"]
async fn ten_gib_upload_passthrough_without_aggregation() {
    let started = std::time::Instant::now();
    let state = SinkState {
        counters: Arc::new(SinkCounters::default()),
        pacing: None,
    };
    let address = common::spawn_router(long_haul_router(state.clone()));
    let client = long_haul_client(address);

    // Producer stream: chunk bodies are COMPUTED at poll time, one 8 MiB
    // allocation at a time — 10 GiB exist nowhere in this process.
    let producer = futures_util::stream::iter(
        (0..CHUNK_COUNT).map(|index| Ok::<Bytes, std::io::Error>(pattern_chunk(index))),
    );
    let body = ::reqwest::Body::wrap_stream(producer);

    let outcome = tokio::time::timeout(OP_TIMEOUT, client.put_object("big", body))
        .await
        .expect("10 GiB upload completes within the generous timeout")
        .expect("documented 201");
    assert!(
        matches!(outcome, fx02::client::PutObjectResponse::Created201),
        "expected the documented 201: {outcome:?}"
    );

    let chunks = state.counters.chunks.load(Ordering::SeqCst);
    let bytes = state.counters.bytes.load(Ordering::SeqCst);
    assert_eq!(
        bytes as usize, TOTAL_BYTES,
        "every synthesized byte arrived"
    );
    assert!(
        chunks > MIN_CHUNKS_PROOF,
        "{chunks} frames is not streaming passthrough (threshold \
         {MIN_CHUNKS_PROOF}); an aggregating path would deliver ~1"
    );
    println!(
        "ten_gib_upload: {} GiB in {} chunks over {:?}",
        TOTAL_BYTES / (1024 * 1024 * 1024),
        chunks,
        started.elapsed()
    );
}

// ----------------------------------------------------------------------
// §50 test 6 — 10 GiB download passthrough without aggregation
// ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[ignore = "long-haul proof: run via `cargo test -p openapi-conformance -- --ignored --nocapture`"]
async fn ten_gib_download_passthrough_without_aggregation() {
    let started = std::time::Instant::now();
    let state = SinkState {
        counters: Arc::new(SinkCounters::default()),
        pacing: None,
    };
    let address = common::spawn_router(long_haul_router(state.clone()));
    let client = long_haul_client(address);

    let outcome = tokio::time::timeout(OP_TIMEOUT, client.get_object("big"))
        .await
        .expect("10 GiB download completes within the generous timeout")
        .expect("documented 200");
    let fx02::client::GetObjectResponse::Ok200(wrapper) = outcome else {
        panic!("expected the documented 200");
    };

    // Consume the §32 wrapper's raw chunk stream incrementally; the loop
    // holds ONE chunk at a time regardless of the transfer size.
    let mut stream = wrapper.into_bytes_stream();
    let mut received_bytes = 0_u64;
    let mut received_chunks = 0_u64;
    while let Some(frame) = stream.next().await {
        let chunk = frame.expect("response body reads");
        received_bytes += chunk.len() as u64;
        received_chunks += 1;
    }

    assert_eq!(
        received_bytes as usize, TOTAL_BYTES,
        "every synthesized byte crossed the wire"
    );
    assert!(
        received_chunks > MIN_CHUNKS_PROOF,
        "{received_chunks} frames is not streaming passthrough"
    );
    println!(
        "ten_gib_download: {} GiB in {} chunks over {:?}",
        TOTAL_BYTES / (1024 * 1024 * 1024),
        received_chunks,
        started.elapsed()
    );
}

// ----------------------------------------------------------------------
// §50 test 7 — backpressure: the producer never runs away
// ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[ignore = "long-haul proof: run via `cargo test -p openapi-conformance -- --ignored --nocapture`"]
async fn backpressure_producer_does_not_run_away() {
    let started = std::time::Instant::now();
    const CHANNEL_CHUNKS: usize = 4;
    /// Slack beyond the channel bound: chunks may sit inside the HTTP write
    /// pipeline and kernel loopback socket buffers at sampling time (measured
    /// headroom on macOS/Linux; still far below any runaway buffering).
    const IN_FLIGHT_SLACK_CHUNKS: u64 = 4;

    // A slow SINK forces the transport to stall; the bounded channel is what
    // keeps the producer parked instead of buffering unboundedly.
    let state = SinkState {
        counters: Arc::new(SinkCounters::default()),
        pacing: Some((8, Duration::from_millis(40))),
    };
    let address = common::spawn_router(long_haul_router(state.clone()));

    let (body, produced, _producer_task) = start_bounded_producer(CHUNK_COUNT, CHANNEL_CHUNKS);
    let client = long_haul_client(address);
    let body = ::reqwest::Body::wrap_stream(body);

    let upload = tokio::time::timeout(OP_TIMEOUT, client.put_object("big", body));
    tokio::pin!(upload);

    // Sample produced-minus-received continuously while the consumer sleeps
    // periodically; the gap must stay pinned near the channel bound.
    let mut max_outstanding = 0_u64;
    loop {
        tokio::select! {
            result = &mut upload => {
                result.expect("upload completes within timeout")
                    .expect("documented 201");
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(5)) => {
                let outstanding = produced
                    .load(Ordering::SeqCst)
                    .saturating_sub(state.counters.bytes.load(Ordering::SeqCst));
                max_outstanding = max_outstanding.max(outstanding);
            }
        }
    }

    let bound = (CHANNEL_CHUNKS as u64 + IN_FLIGHT_SLACK_CHUNKS) * CHUNK_LEN as u64;
    assert_eq!(
        state.counters.bytes.load(Ordering::SeqCst) as usize,
        TOTAL_BYTES,
        "the whole payload still arrived"
    );
    assert!(
        max_outstanding <= bound,
        "producer ran {max_outstanding} bytes ahead; bound is {bound} \
         ({CHANNEL_CHUNKS} channel slots + slack)"
    );
    assert!(
        max_outstanding >= CHUNK_LEN as u64,
        "backpressure never engaged (max outstanding {max_outstanding}); \
         the slow sink should have filled the channel"
    );
    println!(
        "backpressure_upload: max outstanding {max_outstanding} bytes (bound {bound}) over {:?}",
        started.elapsed()
    );
}

// ----------------------------------------------------------------------
// §50 test 8 — cancellation stops work promptly (download direction, plus
// upload-side producer abort)
// ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[ignore = "long-haul proof: run via `cargo test -p openapi-conformance -- --ignored --nocapture`"]
async fn cancellation_stops_work_promptly() {
    let started = std::time::Instant::now();
    let state = SinkState {
        counters: Arc::new(SinkCounters::default()),
        pacing: None,
    };
    let address = common::spawn_router(long_haul_router(state.clone()));
    let client = long_haul_client(address);

    // DOWNLOAD direction: the CALLER owns the polling loop here, so dropping
    // the consumed chunk stream is the cancellation vector — and it must
    // tear down the whole pipeline: the server-side synthesizing source
    // stops producing within ~2 s of the drop.
    let outcome = client.get_object("big").await.expect("documented 200");
    let fx02::client::GetObjectResponse::Ok200(wrapper) = outcome else {
        panic!("expected the documented 200");
    };
    let mut stream = wrapper.into_bytes_stream();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while state.counters.bytes.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "download never delivered its first bytes"
        );
        match tokio::time::timeout(Duration::from_secs(1), stream.next()).await {
            Ok(Some(frame)) => {
                frame.expect("response body reads");
            }
            Ok(None) => panic!("stream ended before cancellation"),
            Err(_) => continue, // re-check the counter
        }
    }
    let served_at_drop = state.counters.bytes.load(Ordering::SeqCst);
    drop(stream);

    // The server source must freeze inside the ~2 s window and stay frozen.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after_window = state.counters.bytes.load(Ordering::SeqCst);
    assert_eq!(
        after_window, served_at_drop,
        "server kept synthesizing bytes {served_at_drop} → {after_window}          after the client cancelled"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        state.counters.bytes.load(Ordering::SeqCst),
        served_at_drop,
        "cancellation must be permanent once settled"
    );

    // UPLOAD direction: the upstream producer is what the application can
    // always cancel; aborting its task must stop synthesis immediately even
    // though reqwest's pooled connection driver unwinds asynchronously.
    const CHANNEL_CHUNKS: usize = 4;
    let (body, produced, producer_task) = start_bounded_producer(CHUNK_COUNT, CHANNEL_CHUNKS);
    let body = ::reqwest::Body::wrap_stream(body);
    let upload = client.put_object("big", body);
    tokio::pin!(upload);
    // Wait until the producer demonstrably started synthesizing...
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while produced.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "second-phase producer never synthesized anything"
        );
        assert!(
            futures_util::poll!(upload.as_mut()).is_pending(),
            "upload finished before cancellation could be exercised"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // ...then abort it: synthesis must stop within the window.
    producer_task.abort();
    let at_abort = produced.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = produced.load(Ordering::SeqCst);
    // tokio checks abort() at await points; each iteration spends a CPU-
    // bound stretch synthesizing its chunk between awaits, so at most a few
    // in-flight iterations land before cancellation bites.
    const ABORT_GRACE_CHUNKS: u64 = 4;
    assert!(
        settled.saturating_sub(at_abort) <= ABORT_GRACE_CHUNKS * CHUNK_LEN as u64,
        "producer kept synthesizing after abort: +{} bytes",
        settled - at_abort
    );
    // The pinned upload future simply leaves scope here; reqwest's pooled
    // connection driver unwinds asynchronously (see the module docs).

    println!(
        "cancellation: download froze server at {served_at_drop} bytes; \
         upload producer aborted at ~{settled} bytes over {:?}",
        started.elapsed()
    );
}
