//! Content-negotiation boundary tests (main spec §22, §25/Example 18, §28,
//! §28.4, §5.2, §44) driven through the GENERATED fixture-14 pair: the
//! router is served over real TCP and driven by the generated client;
//! hostile/broken endpoints that the generated server cannot produce are
//! hand-written; §28.5 wire shapes go straight through
//! `tower::ServiceExt::oneshot`.

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use openapi_conformance::fixtures::fixture_14_negotiation as fx14;
use openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use openapi_support::limits::BodyLimits;
use tower::ServiceExt;

/// What the application observed for `POST /mirror`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MirrorObservation {
    Json(String),
    Any { content_type: String, body: Vec<u8> },
}

struct NegotiationApp {
    octet_mode: AtomicBool,
    either_any_mode: AtomicBool,
    mirror: Mutex<Vec<MirrorObservation>>,
    note_handler_ran: AtomicBool,
    streamed: Mutex<(usize, Vec<u8>)>,
}

impl NegotiationApp {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            octet_mode: AtomicBool::new(false),
            either_any_mode: AtomicBool::new(false),
            mirror: Mutex::new(Vec::new()),
            note_handler_ran: AtomicBool::new(false),
            streamed: Mutex::new((0, Vec::new())),
        })
    }
}

fn problem(title: &str) -> fx14::models::ProblemDetails {
    fx14::models::ProblemDetails {
        title: title.to_owned(),
        detail: openapi_support::optional::OptionalField::Absent,
    }
}

async fn drain(body: ::axum::body::Body) -> (usize, Vec<u8>) {
    let mut stream = body.into_data_stream();
    let mut chunks = 0_usize;
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks += 1;
        bytes.extend_from_slice(&chunk.expect("body chunk"));
    }
    (chunks, bytes)
}

#[async_trait]
impl fx14::server::Api for NegotiationApp {
    async fn get_report(&self, _id: String) -> fx14::server::GetReportResponse {
        if self.octet_mode.load(Ordering::SeqCst) {
            return fx14::server::GetReportResponse::Ok200(
                fx14::server::GetReport200Content::OctetStream(::axum::body::Body::from(vec![
                    0xAB_u8, 0xCD,
                ])),
            );
        }
        fx14::server::GetReportResponse::Ok200(fx14::server::GetReport200Content::Json(
            fx14::models::Report {
                id: "r1".to_owned(),
                summary: openapi_support::optional::OptionalField::Absent,
            },
        ))
    }

    async fn post_mirror(
        &self,
        body: fx14::server::PostMirrorRequestBody,
    ) -> fx14::server::PostMirrorResponse {
        match body {
            fx14::server::PostMirrorRequestBody::Json(payload) => {
                self.mirror
                    .lock()
                    .expect("mirror lock")
                    .push(MirrorObservation::Json(payload.kind));
            }
            fx14::server::PostMirrorRequestBody::Any { content_type, body } => {
                let (chunks, bytes) = drain(body).await;
                assert_eq!(chunks, 1, "single small upload arrives as one chunk");
                self.mirror
                    .lock()
                    .expect("mirror lock")
                    .push(MirrorObservation::Any {
                        content_type: content_type.essence_str().to_owned(),
                        body: bytes,
                    });
            }
        }
        fx14::server::PostMirrorResponse::Accepted202
    }

    async fn get_raw_text(&self) -> fx14::server::GetRawTextResponse {
        // The server cannot know a concrete type behind `text/*`; it supplies
        // one anyway because the §22 payload requires it.
        let content_type: ::mime::Mime = "text/csv".parse().expect("valid mime");
        let chunks: Vec<Result<Bytes, std::convert::Infallible>> = (0..16)
            .map(|index| Ok(common::pattern_chunk(index, CHUNK_LEN)))
            .collect();
        fx14::server::GetRawTextResponse::Ok200(fx14::server::GetRawText200 {
            content_type,
            body: ::axum::body::Body::from_stream(futures_util::stream::iter(chunks)),
        })
    }

    async fn get_either(&self) -> fx14::server::GetEitherResponse {
        if self.either_any_mode.load(Ordering::SeqCst) {
            return fx14::server::GetEitherResponse::Ok200(
                fx14::server::GetEither200Content::Any {
                    content_type: "text/csv".parse().expect("valid mime"),
                    body: ::axum::body::Body::from("fallback bytes"),
                },
            );
        }
        fx14::server::GetEitherResponse::Ok200(fx14::server::GetEither200Content::Json(
            fx14::models::Payload {
                kind: "explicit".to_owned(),
            },
        ))
    }

    async fn put_note(&self, _id: String, _body: String) -> fx14::server::PutNoteResponse {
        self.note_handler_ran.store(true, Ordering::SeqCst);
        fx14::server::PutNoteResponse::NoContent204
    }

    async fn post_stream_note(
        &self,
        body: ::axum::body::Body,
    ) -> fx14::server::PostStreamNoteResponse {
        let (chunks, bytes) = drain(body).await;
        *self.streamed.lock().expect("streamed lock") = (chunks, bytes);
        fx14::server::PostStreamNoteResponse::NoContent204
    }
}

const CHUNK_LEN: usize = 64 * 1024;

fn client(address: std::net::SocketAddr) -> fx14::client::Client {
    fx14::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds")
}

fn spawn(app: Arc<NegotiationApp>) -> std::net::SocketAddr {
    let limits = BodyLimits::process_default();
    let hook: Arc<dyn EncodeOverflowHook> = Arc::new(NoOpEncodeOverflowHook);
    common::spawn_router(fx14::server::router(app, limits, hook))
}

// ----------------------------------------------------------------------
// §22 — explicit beats wildcard on RESPONSE decoding
// ----------------------------------------------------------------------

#[tokio::test]
async fn exact_entry_beats_wildcard_fallback_on_responses() {
    let app = NegotiationApp::new();
    let address = spawn(app.clone());
    let client = client(address);

    // Explicit application/json wins over `*/*` when the server sends JSON.
    let response = client.get_either().await.expect("200 documented");
    match response {
        fx14::client::GetEitherResponse::Ok200(fx14::client::GetEither200Content::Json(
            payload,
        )) => assert_eq!(payload.kind, "explicit"),
        other => panic!("exact entry must decode as Json, got {other:?}"),
    }

    // A representation matching only `*/*` falls through to the wildcard
    // variant, which stays response-owned streaming.
    app.either_any_mode.store(true, Ordering::SeqCst);
    let response = client.get_either().await.expect("200 documented");
    match response {
        fx14::client::GetEitherResponse::Ok200(fx14::client::GetEither200Content::Any(raw)) => {
            assert_eq!(
                raw.headers()
                    .get(::http::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                Some("text/csv"),
                "the wildcard variant carries the server-supplied type"
            );
            let bytes = raw.bytes().await.expect("raw bytes");
            assert_eq!(&bytes[..], b"fallback bytes");
        }
        other => panic!("wildcard-only representation must decode as Any, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// §5.2 range mode — `text/*` streams unbounded through BOTH sides
// ----------------------------------------------------------------------

#[tokio::test]
async fn text_range_response_streams_one_mib_chunked() {
    let app = NegotiationApp::new();
    let address = spawn(app);
    let client = client(address);

    let response = client.get_raw_text().await.expect("200 documented");
    let fx14::client::GetRawTextResponse::Ok200(wrapper) = response;
    let mut stream = wrapper.into_bytes_stream();
    let mut reassembled = Vec::with_capacity(1024 * 1024);
    let mut chunks = 0_usize;
    while let Some(chunk) = stream.next().await {
        chunks += 1;
        reassembled.extend_from_slice(&chunk.expect("response chunk"));
    }
    let expected = common::pattern_payload(16 * CHUNK_LEN, CHUNK_LEN);
    assert_eq!(reassembled.len(), 1024 * 1024, "1 MiB passes through");
    assert_eq!(reassembled, expected, "byte-for-byte passthrough");
    assert!(chunks > 1, "1 MiB must arrive chunked, got {chunks}");
}

#[tokio::test]
async fn x_rust_body_stream_round_trips_raw_and_unbounded() {
    let app = NegotiationApp::new();
    let address = spawn(app.clone());
    let client = client(address);

    // §44 override: the request side takes reqwest::Body; a chunked producer
    // reaches the handler without any bounded String materialization.
    let chunks: Vec<Result<Bytes, std::convert::Infallible>> = (0..8)
        .map(|index| Ok(common::pattern_chunk(index, CHUNK_LEN)))
        .collect();
    let response = client
        .post_stream_note(::reqwest::Body::wrap_stream(futures_util::stream::iter(
            chunks,
        )))
        .await
        .expect("204 documented");
    assert!(matches!(
        response,
        fx14::client::PostStreamNoteResponse::NoContent204
    ));
    let (chunks, bytes) = app.streamed.lock().expect("streamed lock").clone();
    let expected = common::pattern_payload(8 * CHUNK_LEN, CHUNK_LEN);
    assert_eq!(bytes, expected, "512 KiB streams through byte-for-byte");
    assert!(chunks > 1, "upload must stay chunked, got {chunks}");
}

// ----------------------------------------------------------------------
// §25 Example 18 — one status, three negotiated variants
// ----------------------------------------------------------------------

struct TrioApp {
    script: Mutex<VecDeque<u8>>,
}

#[async_trait]
impl fx14::server::Api for TrioApp {
    async fn get_report(&self, _id: String) -> fx14::server::GetReportResponse {
        let next = self
            .script
            .lock()
            .expect("script lock")
            .pop_front()
            .expect("trio script entry");
        fx14::server::GetReportResponse::BadRequest400(match next {
            0 => fx14::server::GetReport400Content::ProblemJson(problem("problem")),
            1 => fx14::server::GetReport400Content::Json(fx14::models::LegacyError {
                code: 1253,
                message: openapi_support::optional::OptionalField::Absent,
            }),
            _ => fx14::server::GetReport400Content::TextPlain("plain failure".to_owned()),
        })
    }

    async fn post_mirror(
        &self,
        _body: fx14::server::PostMirrorRequestBody,
    ) -> fx14::server::PostMirrorResponse {
        unreachable!("not exercised by the trio test")
    }

    async fn get_raw_text(&self) -> fx14::server::GetRawTextResponse {
        unreachable!("not exercised by the trio test")
    }

    async fn get_either(&self) -> fx14::server::GetEitherResponse {
        unreachable!("not exercised by the trio test")
    }

    async fn put_note(&self, _id: String, _body: String) -> fx14::server::PutNoteResponse {
        unreachable!("not exercised by the trio test")
    }

    async fn post_stream_note(
        &self,
        _body: ::axum::body::Body,
    ) -> fx14::server::PostStreamNoteResponse {
        unreachable!("not exercised by the trio test")
    }
}

#[tokio::test]
async fn trio_variants_reach_the_client_in_declaration_order() {
    let app = Arc::new(TrioApp {
        script: Mutex::new([0_u8, 1, 2].into_iter().collect()),
    });
    let limits = BodyLimits::process_default();
    let hook: Arc<dyn EncodeOverflowHook> = Arc::new(NoOpEncodeOverflowHook);
    let address = common::spawn_router(fx14::server::router(app.clone(), limits, hook));
    let client = client(address);

    for expected in ["problem-json", "legacy-json", "plain"] {
        let response = client.get_report("r1").await.expect("400 documented");
        match (expected, response) {
            (
                "problem-json",
                fx14::client::GetReportResponse::BadRequest400(
                    fx14::client::GetReport400Content::ProblemJson(details),
                ),
            ) => assert_eq!(details.title, "problem", "{expected}"),
            (
                "legacy-json",
                fx14::client::GetReportResponse::BadRequest400(
                    fx14::client::GetReport400Content::Json(legacy),
                ),
            ) => assert_eq!(legacy.code, 1253, "{expected}"),
            (
                "plain",
                fx14::client::GetReportResponse::BadRequest400(
                    fx14::client::GetReport400Content::TextPlain(message),
                ),
            ) => assert_eq!(message, "plain failure", "{expected}"),
            (expected, other) => panic!("{expected}: unexpected variant {other:?}"),
        }
    }
}

// ----------------------------------------------------------------------
// §28 dispatch + §28.5 wildcard-incoming rule on REQUESTS
// ----------------------------------------------------------------------

#[tokio::test]
async fn request_wildcard_entry_accepts_arbitrary_content_type_as_stream() {
    let app = NegotiationApp::new();
    let address = spawn(app.clone());

    // Exact entry: the generated client sends bounded JSON.
    let sent = fx14::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds")
        .post_mirror(fx14::client::PostMirrorRequestBody::Json(
            fx14::models::Payload {
                kind: "gizmo".to_owned(),
            },
        ))
        .await
        .expect("202 documented");
    assert!(matches!(
        sent,
        fx14::client::PostMirrorResponse::Accepted202
    ));

    // Arbitrary concrete type: only the `*/*` entry matches, so the router
    // hands the handler the streaming wildcard branch with the negotiated
    // Content-Type (§28 precedence step 5).
    let response = fx14::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
    )
    .oneshot(
        ::http::Request::builder()
            .method(::http::Method::POST)
            .uri("/mirror")
            .header(::http::header::CONTENT_TYPE, "text/csv")
            .body(axum::body::Body::from(b"\xff\x00custom".as_slice()))
            .expect("request"),
    )
    .await
    .expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::ACCEPTED);
    // Clone the observation out of the lock before the next await point.
    let wildcard_observation = app.mirror.lock().expect("mirror lock")[1].clone();
    assert_eq!(
        wildcard_observation,
        MirrorObservation::Any {
            content_type: "text/csv".to_owned(),
            body: b"\xff\x00custom".to_vec(),
        },
        "the wildcard branch must stream arbitrary payloads verbatim"
    );

    // §28.5: a wildcard Content-Type SENT by the caller never selects among
    // multiple documented entries → 415 before the handler runs.
    let rejected = fx14::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
    )
    .oneshot(
        ::http::Request::builder()
            .method(::http::Method::POST)
            .uri("/mirror")
            .header(::http::header::CONTENT_TYPE, "*/*")
            .body(axum::body::Body::from("{\"kind\":\"x\"}"))
            .expect("request"),
    )
    .await
    .expect("in-memory service");
    assert_eq!(
        rejected.status(),
        ::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "wildcard incoming CT with two entries must be a 415"
    );
}

// ----------------------------------------------------------------------
// §28.4 — charset outside the UTF-8 family is rejected on BOTH sides
// ----------------------------------------------------------------------

#[tokio::test]
async fn latin1_charset_is_rejected_by_client_and_server() {
    // Client side: a hostile endpoint documents the trio status but declares
    // latin-1 text — impossible from the generated server.
    let hostile = axum::Router::new().route(
        "/reports/r1",
        axum::routing::get(|| async {
            (
                ::http::StatusCode::BAD_REQUEST,
                [(
                    ::http::header::CONTENT_TYPE,
                    "text/plain; charset=latin1".to_owned(),
                )],
                "erreur",
            )
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("std listener");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, hostile).await.expect("serve") });

    let error = client(address)
        .get_report("r1")
        .await
        .expect_err("latin-1 must fail decoding");
    match error {
        openapi_support::client_error::ClientError::Decode { source, .. } => {
            assert!(
                source.to_string().contains("latin1"),
                "expected UnsupportedCharset(latin1), got {source}"
            );
        }
        other => panic!("expected Decode error, got {other:?}"),
    }

    // Server side: a bounded text/plain request declaring latin-1 is a
    // MalformedBody 400 BEFORE the handler runs (§39).
    let app = NegotiationApp::new();
    let router = fx14::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
    );
    let response = router
        .oneshot(
            ::http::Request::builder()
                .method(::http::Method::PUT)
                .uri("/notes/n1")
                .header(::http::header::CONTENT_TYPE, "text/plain;charset=latin1")
                .body(axum::body::Body::from("note"))
                .expect("request"),
        )
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::BAD_REQUEST);
    assert!(
        !app.note_handler_ran.load(Ordering::SeqCst),
        "rejected bodies must never reach the handler"
    );
}
