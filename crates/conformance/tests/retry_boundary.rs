//! §31 replayable-upload retry boundary conformance (main spec §31,
//! §30.1; DECISIONS D-impl-retry): the GENERATED `_replaying` client twins
//! driven against a real TCP front that resets the connection BEFORE any
//! response headers on its first attempt(s).
//!
//! Coverage: a pre-response connection reset IS retried through the factory
//! (factory called exactly once per attempt, success on attempt two); a
//! DOCUMENTED status arriving after response headers is final regardless of
//! policy (no second factory call); `RetryPolicy::none()` behaves single-
//! attempt; and the fixture-11 multipart twin rebuilds the ENTIRE form on
//! its second attempt. The retryability predicate itself is pinned against
//! the REAL reqwest error surfaced by the reset.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use openapi_conformance::fixtures::fixture_02_streaming_binary as fx02;
use openapi_conformance::fixtures::fixture_11_multipart as fx11;
use openapi_support::optional::OptionalField;
use openapi_support::retry::{is_retryable_transport, RetryPolicy};

/// Front-proxy behavior: how many initial connections get aborted after
/// their request head (before ANY response bytes), and where to proxy the
/// rest.
struct FlakyFront {
    remaining_resets: Arc<AtomicUsize>,
    backend: std::net::SocketAddr,
}

/// Spawns a raw TCP front on an ephemeral port. Connections while
/// `remaining_resets` > 0 are drained up to the end of their request head
/// and then DROPPED without a single response byte — exactly the pre-
/// response transport failure §31 allows retrying. Every later connection
/// is proxied byte-for-byte to the backend.
fn spawn_flaky_front(front: FlakyFront) -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let listener = tokio::net::TcpListener::from_std(listener).expect("std listener");
    let address = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(_) => return,
            };
            let claimed_reset = front
                .remaining_resets
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    (current > 0).then(|| current - 1)
                })
                .is_ok();
            if claimed_reset {
                // Read past the request head so the failure happens mid-
                // exchange, then abort the connection before any response
                // byte is written.
                let mut head = Vec::new();
                let mut buffer = vec![0_u8; 8192];
                loop {
                    match tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            head.extend_from_slice(&buffer[..read]);
                            if head.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                drop(socket);
            } else {
                let Ok(mut backend_socket) = tokio::net::TcpStream::connect(front.backend).await
                else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut socket, &mut backend_socket).await;
            }
        }
    });
    address
}

/// Backend for the fixture-02 `putObject` operation: drains the streamed
/// body, records how many requests REACHED it, and answers per the
/// configured mode (201 stored, or the documented 400 problem+json).
#[derive(Clone)]
struct ObjectBackend {
    hits: Arc<AtomicUsize>,
    answer_bad_request: bool,
}

async fn put_object_handler(
    axum::extract::State(backend): axum::extract::State<ObjectBackend>,
    _id: axum::extract::Path<String>,
    body: axum::body::Body,
) -> http::Response<axum::body::Body> {
    backend.hits.fetch_add(1, Ordering::SeqCst);
    // Drain the streaming payload completely before answering.
    use futures_util::StreamExt;
    let mut stream = body.into_data_stream();
    while let Some(frame) = stream.next().await {
        frame.expect("request body reads");
    }
    if backend.answer_bad_request {
        http::Response::builder()
            .status(http::StatusCode::BAD_REQUEST)
            .header(http::header::CONTENT_TYPE, "application/problem+json")
            .body(axum::body::Body::from(
                br#"{"title":"invalid object"}"#.to_vec(),
            ))
            .expect("response builds")
    } else {
        http::Response::builder()
            .status(http::StatusCode::CREATED)
            .body(axum::body::Body::empty())
            .expect("response builds")
    }
}

fn object_backend_router(backend: ObjectBackend) -> axum::Router {
    use axum::routing::put;
    axum::Router::new()
        .route("/objects/{id}", put(put_object_handler))
        .with_state(backend)
}

fn flaky_policy(attempts: u32) -> RetryPolicy {
    RetryPolicy {
        max_attempts: attempts,
        initial_backoff_ms: 1,
        max_backoff_ms: 5,
    }
}

fn fx02_client(address: std::net::SocketAddr) -> fx02::client::Client {
    // The fixture's secondary `/storage` base is relative by design; these
    // tests exercise only `putObject` on the primary, but build() demands
    // an absolute value for every base (D-impl-relative-servers).
    fx02::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .secondary_base_url("storage", common::base_url(address))
        .build()
        .expect("client builds")
}

// ----------------------------------------------------------------------
// Twin × pre-response reset: retried through the factory
// ----------------------------------------------------------------------

#[tokio::test]
async fn replaying_twin_retries_pre_response_reset_and_rebuilds_body_once_per_attempt() {
    let hits = Arc::new(AtomicUsize::new(0));
    let backend = ObjectBackend {
        hits: Arc::clone(&hits),
        answer_bad_request: false,
    };
    let backend_address = common::spawn_router(object_backend_router(backend));
    let front = spawn_flaky_front(FlakyFront {
        remaining_resets: Arc::new(AtomicUsize::new(1)),
        backend: backend_address,
    });

    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let client = fx02_client(front);
    let outcome = client
        .put_object_replaying(
            "obj-1",
            move || {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(::reqwest::Body::from("replayable-payload")) }
            },
            flaky_policy(2),
        )
        .await
        .expect("second attempt reaches the documented 201");

    assert!(
        matches!(outcome, fx02::client::PutObjectResponse::Created201),
        "expected the documented 201: {outcome:?}"
    );
    // One body built per ATTEMPT: reset consumed attempt one, success was
    // attempt two.
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "factory called per attempt"
    );
    // Only the surviving attempt ever reached the application behind the
    // front.
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

// ----------------------------------------------------------------------
// Documented status AFTER headers: final regardless of policy
// ----------------------------------------------------------------------

#[tokio::test]
async fn documented_error_status_is_final_and_never_reinvokes_the_factory() {
    let hits = Arc::new(AtomicUsize::new(0));
    let backend = ObjectBackend {
        hits: Arc::clone(&hits),
        answer_bad_request: true,
    };
    let address = common::spawn_router(object_backend_router(backend));

    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let client = fx02_client(address);
    let outcome = client
        .put_object_replaying(
            "obj-1",
            move || {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(::reqwest::Body::from("payload")) }
            },
            // Generous budget: the classification must make it irrelevant.
            flaky_policy(4),
        )
        .await
        .expect("a documented status is an enum variant, never an error");

    assert!(
        matches!(outcome, fx02::client::PutObjectResponse::BadRequest400(_)),
        "expected the documented 400 variant: {outcome:?}"
    );
    // Headers arrived: the outcome is FINAL (§31/D-impl-retry) no matter
    // how many attempts the policy allows.
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry after headers");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

// ----------------------------------------------------------------------
// RetryPolicy::none(): single-attempt behavior
// ----------------------------------------------------------------------

#[tokio::test]
async fn none_policy_twin_behaves_single_attempt_on_a_reset_connection() {
    let hits = Arc::new(AtomicUsize::new(0));
    let backend = ObjectBackend {
        hits: Arc::clone(&hits),
        answer_bad_request: false,
    };
    let backend_address = common::spawn_router(object_backend_router(backend));
    let front = spawn_flaky_front(FlakyFront {
        remaining_resets: Arc::new(AtomicUsize::new(1)),
        backend: backend_address,
    });

    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let client = fx02_client(front);
    let error = client
        .put_object_replaying(
            "obj-1",
            move || {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(::reqwest::Body::from("payload")) }
            },
            RetryPolicy::none(),
        )
        .await
        .expect_err("the reset must surface without a second attempt");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "policy::none never retries"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    // Pin the predicate against the REAL reqwest error the reset produced:
    // pre-response faults classify retryable (this is why a policy WOULD
    // have retried here).
    let openapi_support::client_error::ClientError::Transport(transport) = error else {
        panic!("expected a transport error, got {error:?}");
    };
    assert!(
        is_retryable_transport(&transport),
        "a connection aborted before any response header must classify \
         retryable: {transport}"
    );
}

// ----------------------------------------------------------------------
// Multipart twin (§17 Output A): entire form rebuilt per attempt
// ----------------------------------------------------------------------

#[tokio::test]
async fn multipart_replaying_twin_rebuilds_the_whole_form_after_a_reset() {
    use openapi_conformance::fixtures::fixture_11_multipart::client::UploadDocumentRequest;

    let hooks: (
        openapi_support::limits::BodyLimits,
        Arc<dyn openapi_support::hooks::EncodeOverflowHook>,
        Arc<dyn openapi_support::hooks::StreamFailureHook>,
    ) = common::router_args();
    let backend_address = common::spawn_router(fx11::server::router(
        Arc::new(MultipartApp),
        hooks.0,
        hooks.1,
        hooks.2,
    ));
    let front = spawn_flaky_front(FlakyFront {
        remaining_resets: Arc::new(AtomicUsize::new(1)),
        backend: backend_address,
    });

    let calls = Arc::new(AtomicUsize::new(0));
    let factory_calls = Arc::clone(&calls);
    let client = fx11::client::ClientBuilder::new()
        .base_url(common::base_url(front))
        .build()
        .expect("client builds");

    let outcome = client
        .upload_document_replaying(
            move || {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    // EVERY attempt rebuilds the whole input struct; scalar/
                    // JSON parts re-encode through the bounded serializers
                    // inside the twin before anything goes on the wire.
                    Ok(UploadDocumentRequest {
                        metadata: fx11::models::DocumentMetadata {
                            title: "spec".to_owned(),
                            pages: OptionalField::Absent,
                        },
                        tags: vec!["alpha".to_owned()],
                        file: ::reqwest::Body::from("multipart-payload"),
                        file_name: None,
                        file_content_type: None,
                    })
                }
            },
            flaky_policy(2),
        )
        .await
        .expect("second attempt uploads the rebuilt form");

    let fx11::client::UploadDocumentResponse::Created201(created) = outcome;
    assert_eq!(created.body.id, "doc-spec");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "form rebuilt per attempt");
}

/// Minimal fixture-11 application: verifies the rebuilt form arrived whole.
struct MultipartApp;

#[async_trait::async_trait]
impl fx11::server::Api for MultipartApp {
    async fn upload_document(
        &self,
        mut input: fx11::server::UploadDocumentMultipartInput,
    ) -> fx11::server::UploadDocumentResponse {
        let title = input
            .metadata
            .as_ref()
            .map(|metadata| metadata.title.clone())
            .unwrap_or_default();
        let mut stored = Vec::new();
        while let Some(chunk) = input.file.next_chunk().await.expect("streaming part") {
            stored.extend_from_slice(&chunk);
        }
        assert_eq!(stored, b"multipart-payload".to_vec(), "file part intact");
        assert_eq!(*input.tags, vec!["alpha".to_owned()]);
        fx11::server::UploadDocumentResponse::Created201(fx11::server::UploadDocument201 {
            location: format!("/documents/{title}"),
            body: fx11::models::Document {
                id: format!("doc-{title}"),
            },
        })
    }
}
