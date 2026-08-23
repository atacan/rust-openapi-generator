//! Contract-boundary spot tests (main spec §50 tests 21, 22, 29, 31, 45,
//! 46, 47) driven through the GENERATED pair or directly against the
//! GENERATED router via `tower::ServiceExt::oneshot`.

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use openapi_conformance::fixtures::fixture_01_json_roundtrip as fx01;
use openapi_conformance::fixtures::fixture_02_streaming_binary as fx02;
use openapi_conformance::fixtures::fixture_09_optional_body as fx09;
use openapi_support::optional::OptionalField;
use tower::ServiceExt;

fn router09(api: Arc<dyn fx09::server::Api>) -> axum::Router {
    let (limits, hook) = common::router_args();
    fx09::server::router(api, limits, hook)
}

// ----------------------------------------------------------------------
// §50 test 21 — the 204 variant writes NO body
// ----------------------------------------------------------------------

struct TasksApp;

#[async_trait]
impl fx09::server::Api for TasksApp {
    async fn echo_note(&self, body: Option<Option<String>>) -> fx09::server::EchoNoteResponse {
        fx09::server::EchoNoteResponse::Ok200(body.flatten())
    }

    async fn delete_task(&self, _id: String) -> fx09::server::DeleteTaskResponse {
        fx09::server::DeleteTaskResponse::NoContent204
    }
}

#[tokio::test]
async fn test21_no_content_variant_writes_zero_body_bytes() {
    // Raw wire inspection: oneshot straight into the generated router.
    let response = router09(Arc::new(TasksApp))
        .oneshot(
            ::http::Request::builder()
                .method(::http::Method::DELETE)
                .uri("/tasks/t1")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::NO_CONTENT);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(bytes.is_empty(), "204 must write no body, got {bytes:?}");

    // Client side: the unit variant carries nothing to decode.
    let address = common::spawn_router(router09(Arc::new(TasksApp)));
    let client = fx09::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");
    let response = client.delete_task("t1").await.expect("documented");
    assert!(
        matches!(response, fx09::client::DeleteTaskResponse::NoContent204),
        "expected unit NoContent204, got {response:?}"
    );
}

// ----------------------------------------------------------------------
// §50 tests 22 + 47 — optional body: ABSENT vs JSON null vs value
// ----------------------------------------------------------------------

struct EchoApp {
    observed: Mutex<Vec<Option<Option<String>>>>,
}

#[async_trait]
impl fx09::server::Api for EchoApp {
    async fn echo_note(&self, body: Option<Option<String>>) -> fx09::server::EchoNoteResponse {
        self.observed
            .lock()
            .expect("observed lock")
            .push(body.clone());
        fx09::server::EchoNoteResponse::Ok200(body.flatten())
    }

    async fn delete_task(&self, _id: String) -> fx09::server::DeleteTaskResponse {
        unreachable!("not exercised")
    }
}

#[tokio::test]
async fn tests22_47_optional_body_distinguishes_absent_from_null() {
    let app = Arc::new(EchoApp {
        observed: Mutex::new(Vec::new()),
    });
    let address = common::spawn_router(router09(app.clone()));
    let client = fx09::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    // 1. NO body at all → trait sees `None`.
    let fx09::client::EchoNoteResponse::Ok200(echoed) =
        client.echo_note(None).await.expect("documented");
    assert_eq!(echoed, None);

    // 2. JSON document `null` (a present body whose value is null) → trait
    //    sees `Some(None)`; strictly distinct from the absent case.
    let null_body: Option<String> = None;
    let fx09::client::EchoNoteResponse::Ok200(echoed) = client
        .echo_note(Some(&null_body))
        .await
        .expect("documented");
    assert_eq!(echoed, None);

    // 3. A real value round trips.
    let note = Some("hello".to_owned());
    let fx09::client::EchoNoteResponse::Ok200(echoed) =
        client.echo_note(Some(&note)).await.expect("documented");
    assert_eq!(echoed, Some("hello".to_owned()));

    // The server observed exactly the three distinct shapes.
    assert_eq!(
        *app.observed.lock().expect("observed lock"),
        vec![None, Some(None), Some(Some("hello".to_owned()))]
    );
}

// ----------------------------------------------------------------------
// §50 test 29 — documented JSON status with an EMPTY body is a decode error
// ----------------------------------------------------------------------

#[tokio::test]
async fn test29_client_surfaces_decode_error_on_empty_documented_json() {
    // A tiny hand-written app answers 201 with an empty body — something the
    // generated SERVER cannot produce (impossible by construction server-side;
    // §28.3) but a hostile/broken endpoint can.
    let app = axum::Router::new().route(
        "/widgets",
        axum::routing::post(|| async {
            (
                ::http::StatusCode::CREATED,
                [(::http::header::CONTENT_TYPE, "application/json".to_owned())],
            )
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("std listener");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let client = fx01::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");
    let sent = fx01::models::CreateWidget {
        name: "x".to_owned(),
        description: OptionalField::Absent,
    };
    let error = client
        .create_widget(&sent)
        .await
        .expect_err("empty JSON body");
    match error {
        openapi_support::client_error::ClientError::Decode { source, .. } => {
            assert!(
                source.to_string().contains("empty"),
                "expected empty-body source, got {source}"
            );
        }
        other => panic!("expected Decode error, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// §50 test 31 — redirects OFF by default; opt-in follows buffered bodies
// ----------------------------------------------------------------------

/// Redirect app for fixture-02's PUT route answering 307 first, then 201 at
/// the moved location.
fn redirect_app() -> axum::Router {
    axum::Router::new()
        .route(
            "/objects/{id}",
            axum::routing::put(
                |axum::extract::Path(id): axum::extract::Path<String>| async move {
                    (
                        ::http::StatusCode::TEMPORARY_REDIRECT,
                        [(::http::header::LOCATION, format!("/objects/{id}/moved"))],
                    )
                },
            ),
        )
        .route(
            "/objects/{id}/moved",
            axum::routing::put(|| async { ::http::StatusCode::CREATED }),
        )
}

fn spawn_std(app: axum::Router) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let listener = tokio::net::TcpListener::from_std(listener).expect("std listener");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    address
}

#[tokio::test]
async fn test31_redirects_off_by_default_then_opt_in_follows_buffered_body() {
    let address = spawn_std(redirect_app());
    let url = common::base_url(address);

    // Default builder: redirects are DISABLED (§30.1). The raw 307 surfaces
    // as an undocumented status because fixture-02 documents only 201/400.
    let default_client = fx02::client::ClientBuilder::new()
        .base_url(url.clone())
        .secondary_base_url("storage", url.clone())
        .build()
        .expect("client builds");
    let buffered = || ::reqwest::Body::from(vec![1_u8, 2, 3]);
    let error = default_client
        .put_object("blob", buffered())
        .await
        .expect_err("307 undocumented");
    match error {
        openapi_support::client_error::ClientError::UndocumentedStatus { status } => {
            assert_eq!(status, ::http::StatusCode::TEMPORARY_REDIRECT);
        }
        other => panic!("expected UndocumentedStatus, got {other:?}"),
    }

    // Opt-in following (`follow_redirects`): reqwest REPLAYS BUFFERED bodies
    // across 307 redirects automatically, so the final 201 arrives and the
    // documented Created201 variant is reached. (Documented behavior:
    // `RedirectRequiresReplayableBody` is reserved for one-shot STREAMING
    // bodies that cannot be rewound; a `Body::from(Vec<u8>)` is replayable,
    // so following succeeds here without any replay error.)
    let following_client = fx02::client::ClientBuilder::new()
        .base_url(url)
        .secondary_base_url("storage", common::base_url(address))
        .follow_redirects(::reqwest::redirect::Policy::default())
        .build()
        .expect("client builds");
    let followed = following_client
        .put_object("blob", buffered())
        .await
        .expect("redirect followed to 201");
    assert!(
        matches!(followed, fx02::client::PutObjectResponse::Created201),
        "expected Created201 after following, got {followed:?}"
    );
}

// ----------------------------------------------------------------------
// §50 test 45 — gzip Content-Encoding rejected BEFORE any decoding
// ----------------------------------------------------------------------

#[tokio::test]
async fn test45_gzip_content_encoding_rejected_with_415_before_decode() {
    struct NeverInvokedApp(Arc<AtomicUsize>);

    #[async_trait]
    impl fx01::server::Api for NeverInvokedApp {
        async fn create_widget(
            &self,
            _body: fx01::models::CreateWidget,
        ) -> fx01::server::CreateWidgetResponse {
            self.0.fetch_add(1, Ordering::SeqCst);
            unreachable!("application must never observe a coded request");
        }
    }

    let invocations = Arc::new(AtomicUsize::new(0));
    let api = Arc::new(NeverInvokedApp(invocations.clone()));
    let (limits, hook) = common::router_args();
    let router = fx01::server::router(api, limits, hook);

    // The body need not be valid gzip: identity-only content coding (§30.4)
    // rejects the HEADER before anything touches the body stream.
    let response = router
        .oneshot(
            ::http::Request::builder()
                .method(::http::Method::POST)
                .uri("/widgets")
                .header(::http::header::CONTENT_TYPE, "application/json")
                .header(::http::header::CONTENT_ENCODING, "gzip")
                .body(axum::body::Body::from(vec![0x1f, 0x8b, 0x00, 0x01]))
                .expect("request"),
        )
        .await
        .expect("in-memory service");

    assert_eq!(
        response.status(),
        ::http::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "handler must not run"
    );
}

// ----------------------------------------------------------------------
// §50 test 46 — chunked optional-body presence through peek-and-preserve
// ----------------------------------------------------------------------

#[tokio::test]
async fn test46_chunked_stream_presence_and_empty_stream_absence() {
    let app = Arc::new(EchoApp {
        observed: Mutex::new(Vec::new()),
    });

    // Two-chunk body with NO Content-Length framing hint (streaming request):
    // peek-and-preserve must classify it present and decode `"hi"`.
    let chunks: Vec<Result<Bytes, std::convert::Infallible>> = vec![
        Ok(Bytes::from_static(b"\"h")),
        Ok(Bytes::from_static(b"i\"")),
    ];
    let response = router09(app.clone())
        .oneshot(
            ::http::Request::builder()
                .method(::http::Method::POST)
                .uri("/echo-note")
                .header(::http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from_stream(futures_util::stream::iter(
                    chunks,
                )))
                .expect("request"),
        )
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&bytes[..], b"\"hi\"");
    assert_eq!(
        *app.observed.lock().expect("lock"),
        vec![Some(Some("hi".to_owned()))],
        "chunked body must decode as PRESENT"
    );

    // Empty stream: classified absent even with a Content-Type attached.
    app.observed.lock().expect("lock").clear();
    let response = router09(app.clone())
        .oneshot(
            ::http::Request::builder()
                .method(::http::Method::POST)
                .uri("/echo-note")
                .header(::http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&bytes[..], b"null");
    assert_eq!(
        *app.observed.lock().expect("lock"),
        vec![None],
        "empty stream must decode as ABSENT"
    );
}
