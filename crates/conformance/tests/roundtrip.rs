//! Differential round trips: the GENERATED server router (Mode A trait
//! implemented by a hand-written application) is served over real TCP on an
//! ephemeral loopback port and driven by the GENERATED reqwest client, so
//! both directions of every wire shape are exercised against each other.

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use openapi_conformance::fixtures::fixture_01_json_roundtrip as fx01;
use openapi_conformance::fixtures::fixture_02_streaming_binary as fx02;
use openapi_conformance::fixtures::fixture_03_nested_content as fx03;
use openapi_conformance::fixtures::fixture_04_status_ranges as fx04;
use openapi_conformance::fixtures::fixture_10_forms_headers as fx10;
use openapi_support::hooks::EncodeOverflowHook;
use openapi_support::limits::BodyLimits;
use openapi_support::optional::OptionalField;

// ----------------------------------------------------------------------
// Fixture 01 — JSON round trip + BadRequest path
// ----------------------------------------------------------------------

struct WidgetApp {
    problem_mode: AtomicBool,
}

#[async_trait]
impl fx01::server::Api for WidgetApp {
    async fn create_widget(
        &self,
        body: fx01::models::CreateWidget,
    ) -> fx01::server::CreateWidgetResponse {
        if self.problem_mode.load(Ordering::SeqCst) {
            return fx01::server::CreateWidgetResponse::BadRequest400(
                fx01::models::ProblemDetails {
                    title: "invalid widget".to_owned(),
                    detail: OptionalField::Present("name is required".to_owned()),
                },
            );
        }
        fx01::server::CreateWidgetResponse::Created201(fx01::models::Widget {
            id: format!("w-{}", body.name),
            name: body.name.clone(),
            description: body.description.clone(),
        })
    }
}

#[tokio::test]
async fn fixture_01_create_widget_round_trips_both_directions() {
    let app = Arc::new(WidgetApp {
        problem_mode: AtomicBool::new(false),
    });
    let (limits, hook) = common::router_args();
    let address = common::spawn_router(fx01::server::router(app.clone(), limits, hook));

    let client = fx01::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let sent = fx01::models::CreateWidget {
        name: "gizmo".to_owned(),
        description: OptionalField::Present("a fine widget".to_owned()),
    };
    let expected = fx01::models::Widget {
        id: "w-gizmo".to_owned(),
        name: "gizmo".to_owned(),
        description: OptionalField::Present("a fine widget".to_owned()),
    };
    let response = client.create_widget(&sent).await.expect("201 documented");
    match response {
        fx01::client::CreateWidgetResponse::Created201(widget) => {
            assert_eq!(widget, expected);
        }
        other => panic!("expected Created201, got {other:?}"),
    }
}

#[tokio::test]
async fn fixture_01_bad_request_path_carries_problem_details() {
    let app = Arc::new(WidgetApp {
        problem_mode: AtomicBool::new(true),
    });
    let (limits, hook) = common::router_args();
    let address = common::spawn_router(fx01::server::router(app.clone(), limits, hook));

    let client = fx01::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let sent = fx01::models::CreateWidget {
        name: "broken".to_owned(),
        description: OptionalField::Absent,
    };
    let response = client.create_widget(&sent).await.expect("400 documented");
    match response {
        fx01::client::CreateWidgetResponse::BadRequest400(problem) => {
            assert_eq!(problem.title, "invalid widget");
            assert_eq!(
                problem.detail,
                OptionalField::Present("name is required".to_owned())
            );
        }
        other => panic!("expected BadRequest400, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// Fixture 02 — streaming upload/download over the secondary base
// ----------------------------------------------------------------------

const CHUNK_LEN: usize = 64 * 1024;
const CHUNK_COUNT: usize = 128; // 8 MiB total

struct StorageShared {
    stored: Mutex<Vec<u8>>,
    received_chunks: AtomicUsize,
}

struct StorageApp {
    shared: Arc<StorageShared>,
}

#[async_trait]
impl fx02::server::Api for StorageApp {
    /// Streams the upload chunk-wise; nothing aggregates beyond the handler's
    /// own bounded buffer.
    async fn put_object(
        &self,
        _id: String,
        body: ::axum::body::Body,
    ) -> fx02::server::PutObjectResponse {
        let mut stream = body.into_data_stream();
        let mut buffered = Vec::new();
        while let Some(chunk) = stream.next().await {
            self.shared.received_chunks.fetch_add(1, Ordering::SeqCst);
            let chunk: Bytes = chunk.expect("request body chunk");
            buffered.extend_from_slice(&chunk);
        }
        *self.shared.stored.lock().expect("stored lock") = buffered;
        fx02::server::PutObjectResponse::Created201
    }

    /// Streams the stored bytes back out in multiple chunks.
    async fn get_object(&self, _id: String) -> fx02::server::GetObjectResponse {
        let stored = self.shared.stored.lock().expect("stored lock").clone();
        if stored.is_empty() {
            return fx02::server::GetObjectResponse::NotFound404(fx02::models::ProblemDetails {
                title: "missing".to_owned(),
            });
        }
        let chunks: Vec<Result<Bytes, std::convert::Infallible>> = stored
            .chunks(CHUNK_LEN)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        fx02::server::GetObjectResponse::Ok200(fx02::server::GetObject200 {
            // Typed documented headers (main spec §15): both optional here.
            e_tag: Some("\"etag-1\"".to_owned()),
            content_length: Some(stored.len() as i64),
            body: ::axum::body::Body::from_stream(futures_util::stream::iter(chunks)),
        })
    }
}

#[tokio::test]
async fn fixture_02_streaming_round_trip_stays_chunked_and_hits_both_bases() {
    let shared = Arc::new(StorageShared {
        stored: Mutex::new(Vec::new()),
        received_chunks: AtomicUsize::new(0),
    });
    let (limits, hook): (BodyLimits, Arc<dyn EncodeOverflowHook>) = common::router_args();
    let address = common::spawn_router(fx02::server::router(
        Arc::new(StorageApp {
            shared: shared.clone(),
        }),
        limits,
        hook,
    ));

    // Both bases point at the SAME test server; the assertion is that the
    // request ARRIVES through the secondary base (`/storage`), not which
    // physical host it lands on (companion §8 base separation).
    let url = common::base_url(address);
    let client = fx02::client::ClientBuilder::new()
        .base_url(url.clone())
        .secondary_base_url("storage", url)
        .build()
        .expect("client builds");

    // Upload: a lazy producer yields 128 patterned chunks; wrap_stream keeps
    // them streaming so the transfer never buffers the whole payload.
    let produced = Arc::new(AtomicUsize::new(0));
    let counter = produced.clone();
    let producer = futures_util::stream::iter(0..CHUNK_COUNT).map(move |index| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok::<Bytes, std::convert::Infallible>(common::pattern_chunk(index, CHUNK_LEN))
    });
    let response = client
        .put_object("blob", ::reqwest::Body::wrap_stream(producer))
        .await
        .expect("201 documented");
    assert!(
        matches!(response, fx02::client::PutObjectResponse::Created201),
        "expected Created201, got {response:?}"
    );

    // The producer ran to completion AND arrived in more than one chunk:
    // evidence the pipeline consumed it lazily instead of one buffering shot.
    assert_eq!(produced.load(Ordering::SeqCst), CHUNK_COUNT);
    let received = shared.received_chunks.load(Ordering::SeqCst);
    assert!(
        received > 1,
        "upload must arrive in multiple chunks, got {received}"
    );

    // Download: streamed back and reassembled byte-for-byte equal, with the
    // typed documented headers decoded beside the raw response (§15).
    let expected = common::pattern_payload(CHUNK_COUNT * CHUNK_LEN, CHUNK_LEN);
    let downloaded = client.get_object("blob").await.expect("200 documented");
    let fx02::client::GetObjectResponse::Ok200(wrapper) = downloaded else {
        panic!("expected Ok200 wrapper");
    };
    assert_eq!(wrapper.e_tag.as_deref(), Some("\"etag-1\""));
    assert_eq!(wrapper.content_length, Some(expected.len() as i64));
    let mut stream = wrapper.into_bytes_stream();
    let mut reassembled = Vec::with_capacity(expected.len());
    let mut download_chunks = 0_usize;
    while let Some(chunk) = stream.next().await {
        download_chunks += 1;
        reassembled.extend_from_slice(&chunk.expect("response chunk"));
    }
    assert_eq!(reassembled.len(), expected.len());
    assert!(reassembled == expected);
    assert!(
        download_chunks > 1,
        "download must be chunked, got {download_chunks}"
    );
}

// ----------------------------------------------------------------------
// Fixture 03 — negotiated JSON / octet-stream responses
// ----------------------------------------------------------------------

struct ArtifactApp {
    octet_mode: AtomicBool,
}

#[async_trait]
impl fx03::server::Api for ArtifactApp {
    async fn get_artifact(&self, _id: String) -> fx03::server::GetArtifactResponse {
        if self.octet_mode.load(Ordering::SeqCst) {
            return fx03::server::GetArtifactResponse::Ok200(
                fx03::server::GetArtifact200Content::OctetStream(::axum::body::Body::from(vec![
                    0xAB_u8, 0xCD, 0xEF,
                ])),
            );
        }
        fx03::server::GetArtifactResponse::Ok200(fx03::server::GetArtifact200Content::Json(
            fx03::models::ArtifactMetadata {
                id: "a1".to_owned(),
                name: "artifact".to_owned(),
                size: OptionalField::Present(42),
            },
        ))
    }
}

#[tokio::test]
async fn fixture_03_negotiates_json_and_octet_stream_variants() {
    let app = Arc::new(ArtifactApp {
        octet_mode: AtomicBool::new(false),
    });
    let (limits, hook) = common::router_args();
    let address = common::spawn_router(fx03::server::router(app.clone(), limits, hook));

    let client = fx03::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    // JSON variant.
    let json = client.get_artifact("a1").await.expect("200 documented");
    match json {
        fx03::client::GetArtifactResponse::Ok200(fx03::client::GetArtifact200Content::Json(
            metadata,
        )) => {
            assert_eq!(
                metadata,
                fx03::models::ArtifactMetadata {
                    id: "a1".to_owned(),
                    name: "artifact".to_owned(),
                    size: OptionalField::Present(42),
                }
            );
        }
        other => panic!("expected Json variant, got {other:?}"),
    }

    // Octet-stream variant: the raw response streams untouched.
    app.octet_mode.store(true, Ordering::SeqCst);
    let octet = client.get_artifact("a1").await.expect("200 documented");
    match octet {
        fx03::client::GetArtifactResponse::Ok200(
            fx03::client::GetArtifact200Content::OctetStream(response),
        ) => {
            let bytes = response.bytes().await.expect("raw bytes");
            assert_eq!(&bytes[..], &[0xAB_u8, 0xCD, 0xEF]);
        }
        other => panic!("expected OctetStream variant, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// Fixture 04 — explicit beats range; ranges carry arbitrary statuses
// ----------------------------------------------------------------------

struct StatusScriptApp {
    script: Mutex<VecDeque<u16>>,
}

impl StatusScriptApp {
    fn next_code(&self) -> u16 {
        self.script
            .lock()
            .expect("script lock")
            .pop_front()
            .expect("script entry")
    }
}

#[async_trait]
impl fx04::server::Api for StatusScriptApp {
    async fn get_widget(&self, _id: String) -> fx04::server::GetWidgetResponse {
        match self.next_code() {
            200 => fx04::server::GetWidgetResponse::Ok200(fx04::models::Widget {
                id: "w1".to_owned(),
            }),
            202 => fx04::server::GetWidgetResponse::success_2xx(
                ::http::StatusCode::ACCEPTED,
                fx04::models::SuccessEnvelope {
                    data: serde_json::Map::from_iter([(
                        "k".to_owned(),
                        serde_json::Value::String("v".to_owned()),
                    )]),
                },
            )
            .expect("202 inside 200..300"),
            other => {
                let problem = fx04::models::ProblemDetails {
                    title: format!("status {other}"),
                };
                let status = ::http::StatusCode::from_u16(other).expect("valid status");
                if (400..500).contains(&other) {
                    fx04::server::GetWidgetResponse::client_error_4xx(status, problem)
                        .expect("inside 400..500")
                } else {
                    // 599-style undocumented status: `Default` catches it.
                    fx04::server::GetWidgetResponse::default_status(status, problem)
                        .expect("599 covered by Default only")
                }
            }
        }
    }
}

fn status_client(
    address: std::net::SocketAddr,
    app: &Arc<StatusScriptApp>,
    codes: &[u16],
) -> fx04::client::Client {
    *app.script.lock().expect("script lock") = codes.iter().copied().collect();
    fx04::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds")
}
#[tokio::test]
async fn fixture_04_status_precedence_and_range_carry() {
    let app = Arc::new(StatusScriptApp {
        script: Mutex::new(VecDeque::new()),
    });
    let (limits, hook) = common::router_args();
    let address = common::spawn_router(fx04::server::router(app.clone(), limits, hook));

    // Explicit 200 beats the 2XX range: Ok200, not Success2xx.
    let client = status_client(address, &app, &[200]);
    match client.get_widget("w1").await.expect("documented") {
        fx04::client::GetWidgetResponse::Ok200(widget) => {
            assert_eq!(widget.id, "w1");
        }
        other => panic!("explicit 200 must map to Ok200, got {other:?}"),
    }

    // Success2xx carries an arbitrary 2XX status chosen by the application
    // (checked constructor validated membership).
    let client = status_client(address, &app, &[202]);
    match client.get_widget("w1").await.expect("documented") {
        fx04::client::GetWidgetResponse::Success2xx { status, body } => {
            assert_eq!(status, ::http::StatusCode::ACCEPTED);
            assert_eq!(
                body.data.get("k").and_then(|value| value.as_str()),
                Some("v")
            );
        }
        other => panic!("202 must map to Success2xx, got {other:?}"),
    }

    // Default catches a deliberately undocumented status (no other variant
    // covers 599).
    let client = status_client(address, &app, &[599]);
    match client.get_widget("w1").await.expect("documented") {
        fx04::client::GetWidgetResponse::Default { status, body } => {
            assert_eq!(status.as_u16(), 599);
            assert_eq!(body.title, "status 599");
        }
        other => panic!("599 must fall into Default, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// Fixture 10 — URL-encoded forms (§16) + typed response headers (§15)
// ----------------------------------------------------------------------

struct SessionApp {
    /// When false, the 201 omits the optional ETag (absent vs present).
    etag_enabled: AtomicBool,
}

#[async_trait]
impl fx10::server::Api for SessionApp {
    async fn create_session(
        &self,
        form: fx10::models::CreateSessionForm,
    ) -> fx10::server::CreateSessionResponse {
        // The decoded form drives the response so equality is observable
        // end to end.
        let etag = if self.etag_enabled.load(Ordering::SeqCst) {
            Some(format!("\"{}\"", form.username))
        } else {
            None
        };
        let session = fx10::models::Session {
            id: format!("s-{}", form.username),
            token: OptionalField::Absent,
        };
        // Checked constructor path (§48): validated eagerly.
        match fx10::server::CreateSession201::new(
            format!("/sessions/s-{}", form.username),
            etag,
            session,
        ) {
            Ok(payload) => fx10::server::CreateSessionResponse::Created201(payload),
            Err(error) => panic!("checked ctor rejected a legal value: {error}"),
        }
    }

    async fn get_session(&self, _id: String) -> fx10::server::GetSessionResponse {
        unreachable!("not exercised in this suite")
    }

    async fn put_cache_entry(
        &self,
        _key: String,
        _body: fx10::models::CacheEntryForm,
    ) -> fx10::server::PutCacheEntryResponse {
        fx10::server::PutCacheEntryResponse::NoContent204
    }
}

fn spawn_sessions(app: Arc<SessionApp>) -> std::net::SocketAddr {
    let (limits, hook) = common::router_args();
    common::spawn_router(fx10::server::router(app, limits, hook))
}

fn sessions_client(address: std::net::SocketAddr) -> fx10::client::Client {
    fx10::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds")
}

/// Form POST round trip BOTH directions: client bounded-encodes the form
/// (§34/§16), the router decodes it into the flat struct, and the 201
/// carries typed Location (required) + ETag (optional) headers that the
/// client parses beside the JSON body (§15 Output A).
#[tokio::test]
async fn fixture_10_form_round_trip_with_typed_headers() {
    let app = Arc::new(SessionApp {
        etag_enabled: AtomicBool::new(true),
    });
    let address = spawn_sessions(app.clone());
    let client = sessions_client(address);

    let sent = fx10::models::CreateSessionForm {
        username: "ada".to_owned(),
        password: "s3cret&=+".to_owned(), // exercises percent/+ encoding
        remember_me: OptionalField::Present(true),
    };
    let response = client.create_session(&sent).await.expect("201 documented");
    match response {
        fx10::client::CreateSessionResponse::Created201(payload) => {
            assert_eq!(payload.location, "/sessions/s-ada");
            assert_eq!(payload.e_tag.as_deref(), Some("\"ada\""));
            assert_eq!(
                payload.body,
                fx10::models::Session {
                    id: "s-ada".to_owned(),
                    token: OptionalField::Absent,
                }
            );
        }
        other => panic!("expected Created201, got {other:?}"),
    }
}

/// Optional documented header ABSENT on the wire: the client still decodes
/// the variant with `e_tag: None` (required Location stays mandatory).
#[tokio::test]
async fn fixture_10_absent_optional_header_decodes_as_none() {
    let app = Arc::new(SessionApp {
        etag_enabled: AtomicBool::new(false),
    });
    let address = spawn_sessions(app);
    let client = sessions_client(address);

    let sent = fx10::models::CreateSessionForm {
        username: "bob".to_owned(),
        password: "hunter2".to_owned(),
        remember_me: OptionalField::Absent,
    };
    let response = client.create_session(&sent).await.expect("201 documented");
    match response {
        fx10::client::CreateSessionResponse::Created201(payload) => {
            assert_eq!(payload.e_tag, None, "absent ETag must decode as None");
            assert_eq!(payload.location, "/sessions/s-bob");
            // remember_me absent on the wire round-trips as Absent.
            let _ = sent;
        }
        other => panic!("expected Created201, got {other:?}"),
    }
}

/// A required documented header missing from the response is a protocol
/// error (`MissingRequiredHeader`), never a silently-filled variant (§15).
#[tokio::test]
async fn fixture_10_missing_required_location_header_is_a_client_error() {
    // Hostile endpoint answering 201 WITHOUT the required Location header —
    // impossible from the generated server, so hand-written.
    let hostile = axum::Router::new().route(
        "/sessions",
        axum::routing::post(|| async {
            (
                ::http::StatusCode::CREATED,
                [(::http::header::CONTENT_TYPE, "application/json".to_owned())],
                "{\"id\":\"s-x\"}",
            )
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("std listener");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, hostile).await.expect("serve") });

    let client = sessions_client(address);
    let error = client
        .create_session(&fx10::models::CreateSessionForm {
            username: "x".to_owned(),
            password: "y".to_owned(),
            remember_me: OptionalField::Absent,
        })
        .await
        .expect_err("missing Location must fail");
    match error {
        openapi_support::client_error::ClientError::MissingRequiredHeader { name } => {
            assert_eq!(name, ::http::HeaderName::from_static("location"));
        }
        other => panic!("expected MissingRequiredHeader, got {other:?}"),
    }
}
