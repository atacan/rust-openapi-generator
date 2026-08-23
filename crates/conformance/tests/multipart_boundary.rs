//! Multipart conformance (main spec §5.5, §17, §17.1, §38, §39): generated
//! client ↔ generated router round trips plus hand-framed raw requests for
//! the rejection paths the typed client cannot produce.
//!
//! Coverage: 2 MiB streamed upload arriving in many chunks byte-identical;
//! wire-order collection of a repeated textual part; unknown parts surfaced
//! without failure; missing required part → 422 with the handler NOT
//! invoked; duplicate single-valued part → 422; oversized JSON part → 413;
//! corrupt boundary framing → 400 MalformedBody. Fixture 12 pins the
//! file-first wire order (§17.1 enforcement is wire-arrival-based): required
//! parts arriving BEHIND the streaming handoff round trip through
//! `trailing_parts`, and a stream ending without them surfaces exactly one
//! terminal SchemaViolation from `next_chunk` — which the application
//! answers with a DOCUMENTED failure instead of fabricated success.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use openapi_conformance::fixtures::fixture_11_multipart as fx11;
use openapi_conformance::fixtures::fixture_12_multipart_order as fx12;
use openapi_support::hooks::EncodeOverflowHook;
use openapi_support::limits::BodyLimits;

const BOUNDARY: &str = "XbOuNdArYx";
const FILE_CHUNK: usize = 64 * 1024;
const FILE_CHUNKS: usize = 32; // 2 MiB

/// Hand-frames one `form-data` part exactly as browsers do.
fn frame_part(name: &str, extra_headers: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\
         {extra_headers}\r\n"
    )
    .into_bytes();
    out.extend_from_slice(payload);
    out.extend_from_slice(b"\r\n");
    out
}

fn frame_close() -> Vec<u8> {
    format!("--{BOUNDARY}--\r\n").into_bytes()
}

fn multipart_body(parts: &[Vec<u8>]) -> axum::body::Body {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(part);
    }
    body.extend_from_slice(&frame_close());
    axum::body::Body::from(body)
}

fn multipart_request(body: axum::body::Body) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method(http::Method::POST)
        .uri("/documents")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(body)
        .expect("request builds")
}

fn small_scalar_budget() -> BodyLimits {
    BodyLimits {
        multipart_scalar_part_bytes: 64,
        ..BodyLimits::process_default()
    }
}

// ----------------------------------------------------------------------
// Application driving the generated trait
// ----------------------------------------------------------------------

#[derive(Default)]
struct DocAppShared {
    stored: Mutex<Vec<u8>>,
    received_chunks: AtomicUsize,
    invoked: AtomicBool,
    observed_unknown: Mutex<Vec<String>>,
    tag_order: Mutex<Vec<String>>,
    last_title: Mutex<String>,
}

struct DocApp {
    shared: Arc<DocAppShared>,
}

#[async_trait]
impl fx11::server::Api for DocApp {
    /// Drains the streaming file part chunk-wise; nothing aggregates beyond
    /// the application's own buffer.
    async fn upload_document(
        &self,
        mut input: fx11::server::UploadDocumentMultipartInput,
    ) -> fx11::server::UploadDocumentResponse {
        self.shared.invoked.store(true, Ordering::SeqCst);
        // Required metadata may have arrived before OR behind the streaming
        // handoff; the wire-arrival split decides where the value lives.
        let title = input
            .metadata
            .as_ref()
            .or(input.file.trailing_parts.metadata.as_ref())
            .map(|metadata| metadata.title.clone())
            .unwrap_or_default();
        self.shared.last_title.lock().unwrap().clone_from(&title);
        self.shared
            .tag_order
            .lock()
            .unwrap()
            .clone_from(&input.tags);

        // Borrow (never move) the streaming part so `unknown_part_names`
        // can be read AFTER the drain: late part names surface exactly once
        // the sequential stream has flowed past them (§51.4).
        {
            let part = &mut input.file;
            let mut stored = Vec::new();
            while let Some(chunk) = part.next_chunk().await.expect("streaming part") {
                self.shared.received_chunks.fetch_add(1, Ordering::SeqCst);
                stored.extend_from_slice(&chunk);
            }
            *self.shared.stored.lock().unwrap() = stored;
        }
        *self.shared.observed_unknown.lock().unwrap() = input.unknown_part_names();

        fx11::server::UploadDocumentResponse::Created201(fx11::server::UploadDocument201 {
            location: format!("/documents/{title}"),
            body: fx11::models::Document {
                id: format!("doc-{title}"),
            },
        })
    }
}

fn app_with_limits(limits: BodyLimits) -> (Arc<DocAppShared>, axum::Router) {
    let shared = Arc::new(DocAppShared::default());
    let hook: Arc<dyn EncodeOverflowHook> =
        Arc::new(openapi_support::hooks::NoOpEncodeOverflowHook);
    let router = fx11::server::router(
        Arc::new(DocApp {
            shared: shared.clone(),
        }),
        limits,
        hook,
    );
    (shared, router)
}

// ----------------------------------------------------------------------
// Full round trip: streamed file, wire-order tags
// ----------------------------------------------------------------------

#[tokio::test]
async fn multipart_upload_round_trips_two_mib_stream_in_many_chunks() {
    let shared = Arc::new(DocAppShared::default());
    let (limits, hook): (BodyLimits, Arc<dyn EncodeOverflowHook>) = common::router_args();
    let address = common::spawn_router(fx11::server::router(
        Arc::new(DocApp {
            shared: shared.clone(),
        }),
        limits,
        hook,
    ));

    let client = fx11::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    // 2 MiB patterned file on disk; from_file streams it lazily.
    let expected = common::pattern_payload(FILE_CHUNKS * FILE_CHUNK, FILE_CHUNK);
    let path = std::env::temp_dir().join(format!("o2r-multipart-upload-{}", std::process::id()));
    tokio::fs::write(&path, &expected).await.expect("seed file");

    let request = fx11::client::UploadDocumentRequest::from_file(
        fx11::models::DocumentMetadata {
            title: "spec".to_owned(),
            pages: openapi_support::optional::OptionalField::Absent,
        },
        vec!["alpha".to_owned(), "zulu".to_owned()],
        &path,
    )
    .await
    .expect("file opens");

    let response = client
        .upload_document(request)
        .await
        .expect("201 documented");
    let _ = tokio::fs::remove_file(&path).await;

    let fx11::client::UploadDocumentResponse::Created201(created) = response;
    assert_eq!(created.location, "/documents/spec");
    assert_eq!(created.body.id, "doc-spec");

    // The file arrived through MANY chunks and stayed byte-identical.
    assert!(
        shared.received_chunks.load(Ordering::SeqCst) > 1,
        "a 2 MiB upload must arrive in many chunks"
    );
    assert_eq!(*shared.stored.lock().unwrap(), expected);
    // Repeated textual parts collected both values in WIRE ORDER.
    assert_eq!(*shared.tag_order.lock().unwrap(), vec!["alpha", "zulu"]);
}

// ----------------------------------------------------------------------
// Rejection paths (raw requests; the typed client cannot send these)
// ----------------------------------------------------------------------

#[tokio::test]
async fn missing_required_file_rejects_422_and_never_invokes_the_handler() {
    let (shared, router) = app_with_limits(BodyLimits::process_default());
    let metadata = br#"{"title":"spec"}"#.to_vec();
    let body = multipart_body(&[
        frame_part("metadata", "", &metadata),
        // `file` never appears: the stream ends without it (§17.1).
    ]);

    let response = tower::ServiceExt::oneshot(router, multipart_request(body))
        .await
        .expect("infallible");
    assert_eq!(
        response.status(),
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "missing required part is a SchemaViolation 422"
    );
    assert!(
        !shared.invoked.load(Ordering::SeqCst),
        "the application handler must not run"
    );
}

#[tokio::test]
async fn duplicate_metadata_part_rejects_422() {
    let (shared, router) = app_with_limits(BodyLimits::process_default());
    let metadata = br#"{"title":"spec"}"#.to_vec();
    let body = multipart_body(&[
        frame_part("metadata", "", &metadata),
        frame_part("metadata", "", &metadata),
        frame_part("file", "", b"payload"),
    ]);

    let response = tower::ServiceExt::oneshot(router, multipart_request(body))
        .await
        .expect("infallible");
    assert_eq!(response.status(), http::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!shared.invoked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn oversized_json_part_rejects_413() {
    let (shared, router) = app_with_limits(small_scalar_budget());
    // Well over the configured 64-byte scalar-part budget.
    let big_title = "x".repeat(512);
    let metadata = format!(r#"{{"title":"{big_title}"}}"#).into_bytes();
    let body = multipart_body(&[
        frame_part("metadata", "", &metadata),
        frame_part("file", "", b"payload"),
    ]);

    let response = tower::ServiceExt::oneshot(router, multipart_request(body))
        .await
        .expect("infallible");
    assert_eq!(
        response.status(),
        http::StatusCode::PAYLOAD_TOO_LARGE,
        "a JSON part over multipart_scalar_part_bytes is BodyTooLarge"
    );
    assert!(!shared.invoked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn unknown_extra_part_is_ignored_surfaced_and_still_201() {
    let (shared, router) = app_with_limits(BodyLimits::process_default());
    let metadata = br#"{"title":"spec"}"#.to_vec();
    let body = multipart_body(&[
        frame_part("metadata", "", &metadata),
        frame_part("extra_before", "", b"mystery one"),
        frame_part("file", "", b"payload"),
        // Arrives BEHIND the streaming part: surfaced once the application
        // drains it (sequential semantics, §51.4).
        frame_part("extra_after", "", b"mystery two"),
    ]);

    let response = tower::ServiceExt::oneshot(router, multipart_request(body))
        .await
        .expect("infallible");
    assert_eq!(
        response.status(),
        http::StatusCode::CREATED,
        "unknown fields are ignored by default (§17.1)"
    );
    // Surfaced once the streaming part was drained inside the handler.
    let mut observed = shared.observed_unknown.lock().unwrap().clone();
    observed.sort();
    assert_eq!(
        observed,
        vec!["extra_after".to_owned(), "extra_before".to_owned()],
        "unknown part names must be observable on both sides of the stream"
    );
    // The streaming payload still arrived intact.
    assert_eq!(*shared.stored.lock().unwrap(), b"payload".to_vec());
}

#[tokio::test]
async fn corrupt_boundary_framing_rejects_400_malformed_body() {
    let (shared, router) = app_with_limits(BodyLimits::process_default());
    // The declared boundary never matches the body's delimiters: LF-only
    // framing that can never parse (strict CRLF, §28.1 philosophy).
    let body_bytes: Vec<u8> =
        b"--XbOuNdArYx\nContent-Disposition: form-data; name=\"metadata\"\n\n{}\n--XbOuNdArYx--\n"
            .to_vec();
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("/documents")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(axum::body::Body::from(body_bytes))
        .expect("request builds");

    let response = tower::ServiceExt::oneshot(router, request)
        .await
        .expect("infallible");
    assert_eq!(
        response.status(),
        http::StatusCode::BAD_REQUEST,
        "malformed framing maps to MalformedBody 400"
    );
    assert!(!shared.invoked.load(Ordering::SeqCst));
}

// ----------------------------------------------------------------------
// Fixture 12 — file-first wire order (§17.1/§38): enforcement follows
// wire arrival; late required parts ride `trailing_parts`
// ----------------------------------------------------------------------

#[derive(Default)]
struct FileFirstShared {
    invoked: AtomicBool,
    /// True when `metadata` was decoded BEFORE the streaming handoff.
    metadata_pre_handoff: AtomicBool,
    stored: Mutex<Vec<u8>>,
    received_chunks: AtomicUsize,
    late_title: Mutex<String>,
    late_source: Mutex<String>,
    late_tags: Mutex<Vec<String>>,
    /// Detail of the SchemaViolation observed through `next_chunk`.
    violation_detail: Mutex<Option<String>>,
    /// True when a follow-up `next_chunk` observed the documented terminal
    /// `Ok(None)` after the violation.
    violation_was_terminal: AtomicBool,
}

struct FileFirstApp {
    shared: Arc<FileFirstShared>,
}

#[async_trait]
impl fx12::server::Api for FileFirstApp {
    async fn upload_document(
        &self,
        mut input: fx12::server::UploadDocumentMultipartInput,
    ) -> fx12::server::UploadDocumentResponse {
        self.shared.invoked.store(true, Ordering::SeqCst);
        self.shared
            .metadata_pre_handoff
            .store(input.metadata.is_some(), Ordering::SeqCst);

        {
            let part = &mut input.file;
            loop {
                match part.next_chunk().await {
                    Ok(Some(chunk)) => {
                        self.shared.received_chunks.fetch_add(1, Ordering::SeqCst);
                        self.shared.stored.lock().unwrap().extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(rejection) => {
                        // The framing ended WITHOUT pending required parts:
                        // exactly one terminal SchemaViolation (§17.1). The
                        // application answers with the DOCUMENTED failure,
                        // never fabricating success.
                        *self.shared.violation_detail.lock().unwrap() =
                            rejection.detail.clone().map(|detail| detail.into_owned());
                        self.shared.violation_was_terminal.store(
                            matches!(part.next_chunk().await, Ok(None)),
                            Ordering::SeqCst,
                        );
                        return fx12::server::UploadDocumentResponse::Conflict409(
                            fx12::models::Rejection {
                                reason: String::from("required multipart part never arrived"),
                            },
                        );
                    }
                }
            }
        }

        // The drain flowed past every late part: their values now live on
        // the live part's `trailing_parts` (wire-arrival split).
        let trailing = &input.file.trailing_parts;
        if let Some(metadata) = &trailing.metadata {
            *self.shared.late_title.lock().unwrap() = metadata.title.clone();
        }
        if let Some(source) = &trailing.source {
            *self.shared.late_source.lock().unwrap() = source.clone();
        }
        *self.shared.late_tags.lock().unwrap() = trailing.tags.clone();

        fx12::server::UploadDocumentResponse::Created201(fx12::server::UploadDocument201 {
            location: format!("/documents/{}", self.shared.late_title.lock().unwrap()),
            body: fx12::models::Document {
                id: self.shared.late_title.lock().unwrap().clone(),
            },
        })
    }
}

fn file_first_router(limits: BodyLimits) -> (Arc<FileFirstShared>, axum::Router) {
    let shared = Arc::new(FileFirstShared::default());
    let hook: Arc<dyn EncodeOverflowHook> =
        Arc::new(openapi_support::hooks::NoOpEncodeOverflowHook);
    let router = fx12::server::router(
        Arc::new(FileFirstApp {
            shared: shared.clone(),
        }),
        limits,
        hook,
    );
    (shared, router)
}

#[tokio::test]
async fn file_first_upload_round_trips_with_required_parts_arriving_behind_the_stream() {
    let (shared, router) = file_first_router(BodyLimits::process_default());

    // FILE FIRST, then required metadata/source/tags: lawful wire order.
    let expected = common::pattern_payload(96 * 1024, FILE_CHUNK);
    let metadata = br#"{"title":"late"}"#.to_vec();
    let mut body = Vec::new();
    for part in [
        frame_part("file", "", &expected),
        frame_part("metadata", "", &metadata),
        frame_part("source", "", b"scanner"),
        frame_part("tags", "", b"alpha"),
        frame_part("tags", "", b"zulu"),
    ] {
        body.extend_from_slice(&part);
    }
    body.extend_from_slice(&frame_close());
    // Deliver in small chunks so the streaming handoff happens mid-message.
    let chunked = axum::body::Body::from_stream(futures_util::stream::iter(
        body.chunks(FILE_CHUNK)
            .map(<[u8]>::to_vec)
            .map(Ok::<Vec<u8>, std::convert::Infallible>)
            .collect::<Vec<_>>(),
    ));

    let response = tower::ServiceExt::oneshot(router, multipart_request(chunked))
        .await
        .expect("infallible");
    assert_eq!(
        response.status(),
        http::StatusCode::CREATED,
        "a file-first upload with later required parts MUST succeed"
    );

    // The handler ran and saw metadata only BEHIND the handoff.
    assert!(shared.invoked.load(Ordering::SeqCst));
    assert!(
        !shared.metadata_pre_handoff.load(Ordering::SeqCst),
        "`metadata` arrived behind the streaming part"
    );
    // The file streamed through MANY chunks byte-identical.
    assert!(
        shared.received_chunks.load(Ordering::SeqCst) > 1,
        "the file must arrive chunked"
    );
    assert_eq!(*shared.stored.lock().unwrap(), expected);
    // Late required/repeated parts decoded onto `trailing_parts`.
    assert_eq!(*shared.late_title.lock().unwrap(), "late");
    assert_eq!(*shared.late_source.lock().unwrap(), "scanner");
    assert_eq!(
        *shared.late_tags.lock().unwrap(),
        vec!["alpha".to_owned(), "zulu".to_owned()]
    );
    assert!(shared.violation_detail.lock().unwrap().is_none());
}

#[tokio::test]
async fn file_first_stream_ending_without_required_parts_yields_terminal_schema_violation() {
    let (shared, router) = file_first_router(BodyLimits::process_default());
    // Only `file` arrives: `metadata`/`source` never do, so the CLEAN
    // end-of-message must surface them through the live part's terminal
    // error (§17.1) — the handler runs first by design (sequential
    // streaming), and the documented response must never fake success.
    let body = multipart_body(&[frame_part("file", "", b"payload")]);

    let response = tower::ServiceExt::oneshot(router, multipart_request(body))
        .await
        .expect("infallible");
    assert_eq!(
        response.status(),
        http::StatusCode::CONFLICT,
        "the application answered with the DOCUMENTED 409, not a fabricated 201"
    );

    assert!(shared.invoked.load(Ordering::SeqCst));
    let detail = shared.violation_detail.lock().unwrap().clone();
    let detail = detail.expect("next_chunk observed the SchemaViolation");
    assert!(
        detail.contains("metadata") && detail.contains("source"),
        "the terminal error names every outstanding required part: {detail}"
    );
    assert!(
        !detail.contains("file"),
        "the already-delivered part is not reported missing: {detail}"
    );
    // The violation was exactly once and terminal afterwards.
    assert!(shared.violation_was_terminal.load(Ordering::SeqCst));
    // Payload chunks delivered BEFORE the end still reached the app.
    assert_eq!(*shared.stored.lock().unwrap(), b"payload".to_vec());
}
