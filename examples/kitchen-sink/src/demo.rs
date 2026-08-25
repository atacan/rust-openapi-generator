//! Kitchen-sink demo application and full-operation sweep driver.
//!
//! [`KitchenSinkApp`] is the single Mode A application implementing all 22
//! documented operations of the generated [`crate::api::server::Api`] trait
//! over static in-memory data; binary payloads live as files under a UNIQUE
//! `std::env::temp_dir()` directory and are written/read strictly chunk-wise
//! (never aggregated). [`run_sweep`] drives the generated reqwest client
//! through EVERY operation once, asserting expected variants, typed headers,
//! streamed byte counts, absent-vs-null echo distinctions, and view
//! directionality along the way; both binaries and the ignored smoke test
//! share it verbatim.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use openapi_support::client_error::ClientError;
use openapi_support::hooks::{
    EncodeOverflowHook, NoOpEncodeOverflowHook, NoOpStreamFailureHook, StreamFailureHook,
};
use openapi_support::limits::BodyLimits;
use openapi_support::optional::OptionalField;
use openapi_support::stream_errors::ServerStreamError;
use tokio::io::AsyncWriteExt;

use crate::api::client as capi;
use crate::api::client::{Client, ClientBuilder};
use crate::api::models::{
    Account, Ack, CreateSessionForm, CreateWidget, Document, DocumentMetadata, Dog, DogKind, Event,
    FullWidget, MatrixRecord, Metric, Pet, ProblemDetails, Record, Session, StringStatus,
    SuccessEnvelope, ThumbnailMetadata, Widget,
};
use crate::api::server as sapi;
use crate::api::views::{
    AccountRead, AccountWrite, AuditEntryRead, SyncedRecordRead, SyncedRecordWrite,
};

/// Static PNG-flavored blob served by the `Any` wildcard branch of
/// `getThumbnail` (small on purpose; it still streams chunk-by-chunk).
pub const PNG_BLOB: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
];

/// Chunk count of the octet-stream upload/download round trip.
pub const OBJECT_CHUNKS: usize = 4;
/// Byte length of one octet-stream transfer chunk.
pub const OBJECT_CHUNK_LEN: usize = 64 * 1024;

/// Chunk count of the multipart file-part upload.
pub const UPLOAD_CHUNKS: usize = 3;
/// Byte length of one multipart upload chunk.
pub const UPLOAD_CHUNK_LEN: usize = 32 * 1024;

/// Chunk count of the vendor echo payload.
pub const VENDOR_CHUNKS: usize = 8;
/// Byte length of one vendor echo chunk.
pub const VENDOR_CHUNK_LEN: usize = 16 * 1024;

/// Every documented operation id; the sweep must touch each at least once.
pub const REQUIRED_OPERATIONS: [&str; 22] = [
    "createWidget",
    "createSession",
    "putNote",
    "putObject",
    "getObject",
    "getThumbnail",
    "uploadDocument",
    "streamEvents",
    "exportRecords",
    "pushMetrics",
    "exportMetrics",
    "postVendorDocument",
    "probeStatus",
    "deleteTask",
    "getWidget",
    "headWidget",
    "echoNote",
    "createAccount",
    "listAuditEntries",
    "syncRecord",
    "createPet",
    "createRecord",
];

/// One recorded outcome of [`run_sweep`]: `ok == false` means an assertion
/// inside the step failed (unexpected variant, missing header, wrong bytes…).
#[derive(Debug, Clone)]
pub struct SweepStep {
    /// Operation id under test.
    pub op: &'static str,
    /// Whether every in-step assertion held.
    pub ok: bool,
    /// Human-readable summary or failure reason.
    pub message: String,
}

/// Byte `offset` of every patterned demo payload (`% 251` keeps it inside one
/// byte without ever materializing the whole payload).
#[must_use]
pub fn pattern_byte(offset: usize) -> u8 {
    (offset % 251) as u8
}

/// One patterned chunk: byte `j` of chunk `index` is `(index * len + j) % 251`.
#[must_use]
pub fn pattern_chunk(index: usize, len: usize) -> Bytes {
    let start = index * len;
    let mut data = Vec::with_capacity(len);
    for offset in 0..len {
        data.push(pattern_byte(start + offset));
    }
    Bytes::from(data)
}

/// A lazy streaming producer of `chunks` patterned chunks (in-memory source,
/// wrapped in a streaming body — never aggregated).
pub fn pattern_producer(
    chunks: usize,
    len: usize,
) -> impl futures_core::Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    futures_util::stream::iter((0..chunks).map(move |index| Ok(pattern_chunk(index, len))))
}

fn problem_details(title: &str, detail: Option<&str>) -> ProblemDetails {
    ProblemDetails {
        title: title.to_owned(),
        detail: detail
            .map(str::to_owned)
            .map_or(OptionalField::Absent, OptionalField::Present),
    }
}

/// Demo application state: static records plus the temp-dir object store.
pub struct KitchenSinkApp {
    objects_dir: PathBuf,
    widgets: Mutex<HashMap<String, Widget>>,
}

impl KitchenSinkApp {
    /// Creates the app storing binary objects under `objects_dir` (a UNIQUE
    /// directory per process under `std::env::temp_dir()`).
    #[must_use]
    pub fn new(objects_dir: PathBuf) -> Self {
        let mut widgets = HashMap::new();
        widgets.insert(
            "w-static".to_owned(),
            Widget {
                id: "w-static".to_owned(),
                name: "static widget".to_owned(),
                description: OptionalField::Present("seeded before any request".to_owned()),
            },
        );
        Self {
            objects_dir,
            widgets: Mutex::new(widgets),
        }
    }

    /// Safe join of an object id onto the store directory: rejects empty ids
    /// and anything path-traversal shaped instead of trusting the segment.
    fn object_path(&self, id: &str) -> Option<PathBuf> {
        if id.is_empty() || matches!(id, "." | "..") || id.contains('/') || id.contains('\\') {
            return None;
        }
        Some(self.objects_dir.join(id))
    }

    /// Ensures the store directory exists, then opens `name` inside it for
    /// writing from scratch.
    async fn open_store_file(&self, name: &str) -> std::io::Result<tokio::fs::File> {
        tokio::fs::create_dir_all(&self.objects_dir).await?;
        tokio::fs::File::create(self.objects_dir.join(name)).await
    }
}

#[async_trait]
impl sapi::Api for KitchenSinkApp {
    async fn create_widget(&self, body: CreateWidget) -> sapi::CreateWidgetResponse {
        let widget = Widget {
            id: format!("w-{}", body.name),
            name: body.name,
            description: body.description,
        };
        self.widgets
            .lock()
            .expect("widgets lock")
            .insert(widget.id.clone(), widget.clone());
        sapi::CreateWidgetResponse::Created201(widget)
    }

    async fn create_session(&self, body: CreateSessionForm) -> sapi::CreateSessionResponse {
        let session = Session {
            id: format!("s-{}", body.username),
            token: OptionalField::Absent,
        };
        match sapi::CreateSession201::new(
            format!("/sessions/{}", session.id),
            Some("\"session-demo-etag\"".to_owned()),
            session,
        ) {
            Ok(payload) => sapi::CreateSessionResponse::Created201(payload),
            Err(error) => panic!("checked header constructor rejected a legal value: {error}"),
        }
    }

    async fn put_note(&self, _id: String, _body: String) -> sapi::PutNoteResponse {
        sapi::PutNoteResponse::NoContent204
    }

    /// Streams the raw request body to `<temp-dir>/<id>` one chunk at a time;
    /// nothing aggregates the octet-stream payload.
    async fn put_object(&self, id: String, body: axum::body::Body) -> sapi::PutObjectResponse {
        if self.object_path(&id).is_none() {
            return sapi::PutObjectResponse::BadRequest400(problem_details(
                "object id rejected",
                Some(&id),
            ));
        }
        let mut file = match self.open_store_file(&id).await {
            Ok(file) => file,
            Err(error) => {
                return sapi::PutObjectResponse::BadRequest400(problem_details(
                    "object storage unavailable",
                    Some(&error.to_string()),
                ));
            }
        };
        let mut chunks = body.into_data_stream();
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(chunk) => {
                    if let Err(error) = file.write_all(&chunk).await {
                        return sapi::PutObjectResponse::BadRequest400(problem_details(
                            "object write failed",
                            Some(&error.to_string()),
                        ));
                    }
                }
                Err(error) => {
                    return sapi::PutObjectResponse::BadRequest400(problem_details(
                        "request body failed mid-upload",
                        Some(&error.to_string()),
                    ));
                }
            }
        }
        if let Err(error) = file.flush().await {
            return sapi::PutObjectResponse::BadRequest400(problem_details(
                "object write failed",
                Some(&error.to_string()),
            ));
        }
        sapi::PutObjectResponse::Created201
    }

    /// Re-streams the stored file from disk via `ReaderStream`; ETag and
    /// Content-Length come from the path/id and file metadata respectively.
    async fn get_object(&self, id: String) -> sapi::GetObjectResponse {
        let Some(path) = self.object_path(&id) else {
            return sapi::GetObjectResponse::NotFound404(problem_details("object not found", None));
        };
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            return sapi::GetObjectResponse::NotFound404(problem_details("object not found", None));
        };
        let Ok(file) = tokio::fs::File::open(&path).await else {
            return sapi::GetObjectResponse::NotFound404(problem_details("object not found", None));
        };
        let length = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        sapi::GetObjectResponse::Ok200(sapi::GetObject200 {
            e_tag: Some(format!("\"ks-{id}-{length}\"")),
            content_length: Some(length),
            body: axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
        })
    }

    /// Nested media enum demo: one id serves JSON `ThumbnailMetadata`, another
    /// the `Any` wildcard branch (static PNG blob behind `image/png`).
    async fn get_thumbnail(&self, id: String) -> sapi::GetThumbnailResponse {
        match id.as_str() {
            "meta-json" => sapi::GetThumbnailResponse::Ok200(sapi::GetThumbnail200Content::Json(
                ThumbnailMetadata {
                    id: "meta-json".to_owned(),
                    name: "demo thumbnail".to_owned(),
                    size: OptionalField::Present(1337),
                },
            )),
            "png-blob" => sapi::GetThumbnailResponse::Ok200(sapi::GetThumbnail200Content::Any {
                content_type: mime::IMAGE_PNG,
                body: axum::body::Body::from_stream(futures_util::stream::iter([Ok::<
                    Bytes,
                    Infallible,
                >(
                    Bytes::from_static(PNG_BLOB),
                )])),
            }),
            _ => {
                sapi::GetThumbnailResponse::NotFound404(problem_details("no such thumbnail", None))
            }
        }
    }

    /// Consumes the bounded scalar/JSON parts, then streams the single binary
    /// part straight into the temp dir (chunk-wise, never aggregated).
    async fn upload_document(
        &self,
        mut input: sapi::UploadDocumentMultipartInput,
    ) -> sapi::UploadDocumentResponse {
        let title = input
            .metadata
            .as_ref()
            .or(input.file.trailing_parts.metadata.as_ref())
            .map(|metadata| metadata.title.clone())
            .unwrap_or_else(|| "untitled".to_owned());
        let doc_id = format!("doc-{title}");
        let mut file = self
            .open_store_file(&format!("upload-{doc_id}"))
            .await
            .unwrap_or_else(|error| panic!("upload storage failed: {error}"));
        let part = &mut input.file;
        while let Some(chunk) = part.next_chunk().await.expect("multipart stream decodes") {
            file.write_all(&chunk)
                .await
                .unwrap_or_else(|error| panic!("upload write failed: {error}"));
        }
        // The `part` borrow ends here; trailing parts decoded behind the
        // stream are readable on `input.file.trailing_parts` afterwards.
        file.flush()
            .await
            .unwrap_or_else(|error| panic!("upload flush failed: {error}"));
        let payload =
            sapi::UploadDocument201::new(format!("/documents/{doc_id}"), Document { id: doc_id })
                .unwrap_or_else(|error| {
                    panic!("checked header constructor rejected a legal value: {error}")
                });
        sapi::UploadDocumentResponse::Created201(payload)
    }

    /// Four static events through the generated SSE stream variant.
    async fn stream_events(&self) -> sapi::StreamEventsResponse {
        let items: Vec<Result<Event, ServerStreamError>> = (0..4_i64)
            .map(|seq| {
                Ok(Event {
                    seq,
                    payload: OptionalField::Present(format!("tick-{seq}")),
                })
            })
            .collect();
        sapi::StreamEventsResponse::Ok200(Box::pin(futures_util::stream::iter(items)))
    }

    /// Five static records through the generated NDJSON stream variant.
    async fn export_records(&self) -> sapi::ExportRecordsResponse {
        let items: Vec<Result<Record, ServerStreamError>> = (1..=5)
            .map(|index| {
                Ok(Record {
                    id: format!("r-{index}"),
                    value: OptionalField::Present(f64::from(index) + 0.5),
                })
            })
            .collect();
        sapi::ExportRecordsResponse::Ok200(Box::pin(futures_util::stream::iter(items)))
    }

    /// Drains the typed json-seq item stream item-by-item, counting only.
    async fn push_metrics(
        &self,
        mut body: sapi::PushMetricsJsonSeqInput,
    ) -> sapi::PushMetricsResponse {
        let mut received = 0_i32;
        while let Some(_metric) = body.next_item().await.expect("json-seq record decodes") {
            received = received.saturating_add(1);
        }
        sapi::PushMetricsResponse::Accepted202(Ack {
            accepted: true,
            received: OptionalField::Present(received),
        })
    }

    /// Three static metrics through the generated json-seq stream variant.
    async fn export_metrics(&self) -> sapi::ExportMetricsResponse {
        let items: Vec<Result<Metric, ServerStreamError>> = (0..3)
            .map(|index| {
                Ok(Metric {
                    name: format!("m-{index}"),
                    value: OptionalField::Present(f64::from(index)),
                })
            })
            .collect();
        sapi::ExportMetricsResponse::Ok200(Box::pin(futures_util::stream::iter(items)))
    }

    /// Pure passthrough: the raw vendor body is echoed back verbatim with
    /// zero buffering — the very `Body` handed in becomes the response body.
    async fn post_vendor_document(
        &self,
        body: axum::body::Body,
    ) -> sapi::PostVendorDocumentResponse {
        sapi::PostVendorDocumentResponse::Ok200(sapi::PostVendorDocument200 { body })
    }

    /// Id-prefix dispatch so the demo can drive all four documented shapes:
    /// `ok*` → 200, `2xx*` → 202 envelope, `4xx*` → 409 problem, else → 599
    /// through `default`.
    async fn probe_status(&self, id: String) -> sapi::ProbeStatusResponse {
        if id.starts_with("ok") {
            return sapi::ProbeStatusResponse::Ok200(Widget {
                id,
                name: "probe-ok".to_owned(),
                description: OptionalField::Absent,
            });
        }
        if id.starts_with("2xx") {
            let mut data = serde_json::Map::new();
            data.insert("probed".to_owned(), serde_json::Value::String(id));
            return sapi::ProbeStatusResponse::success_2xx(
                http::StatusCode::ACCEPTED,
                SuccessEnvelope { data },
            )
            .expect("202 lies inside 200..300");
        }
        if id.starts_with("4xx") {
            return sapi::ProbeStatusResponse::client_error_4xx(
                http::StatusCode::CONFLICT,
                problem_details("probed conflict", None),
            )
            .expect("409 lies inside 400..500");
        }
        sapi::ProbeStatusResponse::default_status(
            http::StatusCode::from_u16(599).expect("599 is a valid status code"),
            problem_details("undocumented territory", None),
        )
        .expect("599 falls to Default alone")
    }

    async fn delete_task(&self, id: String) -> sapi::DeleteTaskResponse {
        if id == "t-1" {
            sapi::DeleteTaskResponse::NoContent204
        } else {
            sapi::DeleteTaskResponse::NotFound404(problem_details("missing task", Some(&id)))
        }
    }

    async fn get_widget(&self, id: String) -> sapi::GetWidgetResponse {
        match self.widgets.lock().expect("widgets lock").get(&id) {
            Some(widget) => sapi::GetWidgetResponse::Ok200(widget.clone()),
            None => sapi::GetWidgetResponse::NotFound404,
        }
    }

    /// Header-only HEAD probe: Content-Length mirrors the GET representation's
    /// serialized size; no body exists to fabricate.
    async fn head_widget(&self, id: String) -> sapi::HeadWidgetResponse {
        let widget = self.widgets.lock().expect("widgets lock").get(&id).cloned();
        match widget {
            Some(widget) => {
                let content_length = ::openapi_support::encode::serialize_json_limited(
                    &widget,
                    openapi_support::limits::BodyLimits::process_default().structured_encode_bytes,
                )
                .map(|bytes| bytes.len())
                .unwrap_or(0);
                sapi::HeadWidgetResponse::Ok200 {
                    e_tag: format!("\"{id}\""),
                    content_length: i64::try_from(content_length).unwrap_or(i64::MAX),
                }
            }
            None => sapi::HeadWidgetResponse::NotFound404,
        }
    }

    /// Mirrors the three §26 body forms injectively: absent → the sentinel
    /// string, JSON null → null, value → itself.
    async fn echo_note(&self, body: Option<Option<String>>) -> sapi::EchoNoteResponse {
        match body {
            None => sapi::EchoNoteResponse::Ok200(Some("[absent]".to_owned())),
            Some(None) => sapi::EchoNoteResponse::Ok200(None),
            Some(Some(note)) => sapi::EchoNoteResponse::Ok200(Some(note)),
        }
    }

    /// The router already reconstructed `Account` losslessly from
    /// `AccountWrite`; the response view drops `password` (writeOnly).
    async fn create_account(&self, body: Account) -> sapi::CreateAccountResponse {
        sapi::CreateAccountResponse::Created201(AccountRead {
            id: body.id,
            note: body.note,
        })
    }

    async fn list_audit_entries(&self, _id: String) -> sapi::ListAuditEntriesResponse {
        sapi::ListAuditEntriesResponse::Ok200(AuditEntryRead {
            created_at: "2026-08-25T00:00:00Z".to_owned(),
            metadata: Some("audit-demo".to_owned()),
        })
    }

    /// The trait takes `SyncedRecordWrite` (readOnly fields structurally
    /// absent) and answers `SyncedRecordRead` (writeOnly secret never
    /// surfaces); server-owned readOnly values are filled here, never faked
    /// from the request.
    async fn sync_record(&self, body: SyncedRecordWrite) -> sapi::SyncRecordResponse {
        sapi::SyncRecordResponse::Ok200(SyncedRecordRead {
            id: "synced-1".to_owned(),
            label: body.label,
            reviewed_by: OptionalField::Present("demo-reviewer".to_owned()),
        })
    }

    async fn create_pet(&self, body: Pet) -> sapi::CreatePetResponse {
        let name = match body {
            Pet::Dog(_) => "rex",
            Pet::Cat(_) => "mia",
        };
        sapi::CreatePetResponse::Created201(FullWidget {
            id: "pet-1".to_owned(),
            name: OptionalField::Present(name.to_owned()),
            created_at: "2026-08-25T12:00:00Z".to_owned(),
        })
    }

    async fn create_record(&self, body: MatrixRecord) -> sapi::CreateRecordResponse {
        sapi::CreateRecordResponse::Created201(body)
    }
}

// ----------------------------------------------------------------------
// Full-operation sweep driver (shared by both binaries and the smoke test)
// ----------------------------------------------------------------------

/// Wires the generated router around `app` with process-default body limits
/// and the NoOp hook trio (encode overflow §34.1, stream failure §40).
pub fn demo_router(app: Arc<KitchenSinkApp>) -> axum::Router {
    let limits = BodyLimits::process_default();
    let encode_overflow_hook: Arc<dyn EncodeOverflowHook> = Arc::new(NoOpEncodeOverflowHook);
    let stream_failure_hook: Arc<dyn StreamFailureHook> = Arc::new(NoOpStreamFailureHook);
    sapi::router(app, limits, encode_overflow_hook, stream_failure_hook)
}

/// Builds the generated client against `base_url` with process defaults.
pub fn build_client(base_url: &str) -> Result<Client, ClientError> {
    ClientBuilder::new().base_url(base_url.to_owned()).build()
}

fn record(steps: &mut Vec<SweepStep>, op: &'static str, outcome: Result<String, String>) {
    let (ok, message) = match outcome {
        Ok(message) => (true, message),
        Err(message) => (false, format!("FAILED: {message}")),
    };
    steps.push(SweepStep { op, ok, message });
}

async fn step_create_widget(client: &Client) -> Result<String, String> {
    let body = CreateWidget {
        name: "gizmo".to_owned(),
        description: OptionalField::Present("kitchen-sink demo widget".to_owned()),
    };
    match client
        .create_widget(&body)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::CreateWidgetResponse::Created201(widget) => {
            if widget.id != "w-gizmo" {
                return Err(format!("unexpected widget id `{}`", widget.id));
            }
            Ok(format!("id={} name={}", widget.id, widget.name))
        }
        capi::CreateWidgetResponse::BadRequest400(problem) => {
            Err(format!("unexpected 400 problem `{}`", problem.title))
        }
    }
}

async fn step_head_widget(client: &Client) -> Result<String, String> {
    match client
        .head_widget("w-gizmo")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::HeadWidgetResponse::Ok200 {
            e_tag,
            content_length,
        } => {
            if content_length <= 0 {
                return Err(format!(
                    "Content-Length must be positive, got {content_length}"
                ));
            }
            Ok(format!("etag={e_tag} content-length={content_length}"))
        }
        capi::HeadWidgetResponse::NotFound404 => Err("created widget vanished".to_owned()),
    }
}

async fn step_get_widget(client: &Client) -> Result<String, String> {
    match client
        .get_widget("w-gizmo")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::GetWidgetResponse::Ok200(widget) => {
            if widget.name != "gizmo" {
                return Err(format!("unexpected name `{}`", widget.name));
            }
            match client
                .get_widget("w-missing")
                .await
                .map_err(|error| error.to_string())?
            {
                capi::GetWidgetResponse::NotFound404 => Ok(format!(
                    "id={} plus a clean 404 for a missing id",
                    widget.id
                )),
                capi::GetWidgetResponse::Ok200(other) => {
                    Err(format!("missing id unexpectedly returned `{}`", other.id))
                }
            }
        }
        capi::GetWidgetResponse::NotFound404 => Err("created widget vanished".to_owned()),
    }
}

async fn step_create_session(client: &Client) -> Result<String, String> {
    let body = CreateSessionForm {
        username: "ada".to_owned(),
        password: "s3cret&=+".to_owned(),
        remember_me: OptionalField::Present(true),
    };
    match client
        .create_session(&body)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::CreateSessionResponse::Created201(payload) => {
            if payload.location != "/sessions/s-ada" {
                return Err(format!("unexpected Location `{}`", payload.location));
            }
            if payload.e_tag.as_deref() != Some("\"session-demo-etag\"") {
                return Err(format!("unexpected ETag {:?}", payload.e_tag));
            }
            if payload.body.id != "s-ada" {
                return Err(format!("unexpected session id `{}`", payload.body.id));
            }
            Ok(format!(
                "location={} etag={:?}",
                payload.location, payload.e_tag
            ))
        }
        capi::CreateSessionResponse::Unauthorized401(problem) => {
            Err(format!("unexpected 401 problem `{}`", problem.title))
        }
    }
}

async fn step_put_note(client: &Client) -> Result<String, String> {
    match client
        .put_note("n-1", "hello note")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::PutNoteResponse::NoContent204 => Ok("text/plain body stored as 204".to_owned()),
    }
}

async fn step_put_object(client: &Client) -> Result<String, String> {
    match client
        .put_object(
            "blob-1",
            reqwest::Body::wrap_stream(pattern_producer(OBJECT_CHUNKS, OBJECT_CHUNK_LEN)),
        )
        .await
        .map_err(|error| error.to_string())?
    {
        capi::PutObjectResponse::Created201 => Ok(format!(
            "{} bytes streamed up in {} chunks",
            OBJECT_CHUNKS * OBJECT_CHUNK_LEN,
            OBJECT_CHUNKS
        )),
        capi::PutObjectResponse::BadRequest400(problem) => {
            Err(format!("unexpected 400 problem `{}`", problem.title))
        }
    }
}

async fn step_get_object(client: &Client) -> Result<String, String> {
    let expected_len = OBJECT_CHUNKS * OBJECT_CHUNK_LEN;
    match client
        .get_object("blob-1")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::GetObjectResponse::Ok200(wrapper) => {
            let e_tag = wrapper.e_tag.clone().ok_or("ETag header missing")?;
            if wrapper.content_length != Some(i64::try_from(expected_len).expect("small length")) {
                return Err(format!(
                    "unexpected Content-Length {:?}",
                    wrapper.content_length
                ));
            }
            let (chunks, bytes) =
                drain_pattern_stream(wrapper.into_bytes_stream(), expected_len).await?;
            if chunks == 0 {
                return Err("download produced no chunks".to_owned());
            }
            Ok(format!("etag={e_tag} bytes={bytes} chunks={chunks}"))
        }
        capi::GetObjectResponse::NotFound404(problem) => {
            Err(format!("uploaded object missing: {}", problem.title))
        }
    }
}

async fn step_thumbnail_json(client: &Client) -> Result<String, String> {
    match client
        .get_thumbnail("meta-json")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::GetThumbnailResponse::Ok200(capi::GetThumbnail200Content::Json(metadata)) => {
            if metadata.id != "meta-json" || metadata.size != OptionalField::Present(1337) {
                return Err(format!("unexpected metadata {metadata:?}"));
            }
            Ok(format!("json branch id={}", metadata.id))
        }
        capi::GetThumbnailResponse::Ok200(_) => Err("expected the Json branch".to_owned()),
        capi::GetThumbnailResponse::NotFound404(problem) => {
            Err(format!("unexpected 404 problem `{}`", problem.title))
        }
    }
}

async fn step_thumbnail_png(client: &Client) -> Result<String, String> {
    match client
        .get_thumbnail("png-blob")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::GetThumbnailResponse::Ok200(capi::GetThumbnail200Content::Any(response)) => {
            let content_type = response
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            if !content_type.starts_with("image/png") {
                return Err(format!("unexpected Content-Type `{content_type}`"));
            }
            let (chunks, bytes) = drain_exact_stream(response.bytes_stream(), PNG_BLOB).await?;
            if chunks == 0 {
                return Err("wildcard branch streamed no bytes".to_owned());
            }
            Ok(format!(
                "any branch bytes={bytes} chunks={chunks} content-type={content_type}"
            ))
        }
        capi::GetThumbnailResponse::Ok200(_) => Err("expected the Any wildcard branch".to_owned()),
        capi::GetThumbnailResponse::NotFound404(problem) => {
            Err(format!("unexpected 404 problem `{}`", problem.title))
        }
    }
}

async fn step_upload_document(client: &Client) -> Result<String, String> {
    let request = capi::UploadDocumentRequest {
        metadata: DocumentMetadata {
            title: "spec".to_owned(),
            pages: OptionalField::Present(12),
        },
        tags: vec!["alpha".to_owned(), "zulu".to_owned()],
        file: reqwest::Body::wrap_stream(pattern_producer(UPLOAD_CHUNKS, UPLOAD_CHUNK_LEN)),
        file_name: Some("demo.bin".to_owned()),
        file_content_type: Some(mime::APPLICATION_OCTET_STREAM),
    };
    match client
        .upload_document(request)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::UploadDocumentResponse::Created201(payload) => {
            if payload.location != "/documents/doc-spec" {
                return Err(format!("unexpected Location `{}`", payload.location));
            }
            if payload.body.id != "doc-spec" {
                return Err(format!("unexpected document id `{}`", payload.body.id));
            }
            Ok(format!(
                "location={} id={} ({} upload chunks)",
                payload.location, payload.body.id, UPLOAD_CHUNKS
            ))
        }
    }
}

async fn step_stream_events(client: &Client) -> Result<String, String> {
    match client
        .stream_events()
        .await
        .map_err(|error| error.to_string())?
    {
        capi::StreamEventsResponse::Ok200(wrapper) => {
            let mut stream = Box::pin(wrapper.into_sse_stream());
            let mut seqs = Vec::new();
            while let Some(item) = stream.next().await {
                let event = item.map_err(|error| format!("SSE decode failed: {error}"))?;
                seqs.push(event.seq);
            }
            if seqs != [0, 1, 2, 3] {
                return Err(format!("unexpected event sequence {seqs:?}"));
            }
            Ok(format!("{} SSE events counted item-by-item", seqs.len()))
        }
        capi::StreamEventsResponse::Unauthorized401(problem) => {
            Err(format!("unexpected 401 problem `{}`", problem.title))
        }
    }
}

async fn step_export_records(client: &Client) -> Result<String, String> {
    match client
        .export_records()
        .await
        .map_err(|error| error.to_string())?
    {
        capi::ExportRecordsResponse::Ok200(wrapper) => {
            let mut stream = Box::pin(wrapper.into_ndjson_stream());
            let mut ids = Vec::new();
            while let Some(item) = stream.next().await {
                let record = item.map_err(|error| format!("NDJSON decode failed: {error}"))?;
                ids.push(record.id);
            }
            if ids != ["r-1", "r-2", "r-3", "r-4", "r-5"] {
                return Err(format!("unexpected record ids {ids:?}"));
            }
            Ok(format!("{} NDJSON records counted item-by-item", ids.len()))
        }
        capi::ExportRecordsResponse::Unauthorized401(problem) => {
            Err(format!("unexpected 401 problem `{}`", problem.title))
        }
    }
}

async fn step_push_metrics(client: &Client) -> Result<String, String> {
    let metrics: Vec<Metric> = (0..3)
        .map(|index| Metric {
            name: format!("m-{index}"),
            value: OptionalField::Present(f64::from(index)),
        })
        .collect();
    let body = Box::pin(futures_util::stream::iter(metrics));
    match client
        .push_metrics(body)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::PushMetricsResponse::Accepted202(ack) => {
            if !ack.accepted || ack.received != OptionalField::Present(3) {
                return Err(format!("unexpected ack {ack:?}"));
            }
            Ok(format!("202 ack received={:?}", ack.received))
        }
    }
}

async fn step_export_metrics(client: &Client) -> Result<String, String> {
    match client
        .export_metrics()
        .await
        .map_err(|error| error.to_string())?
    {
        capi::ExportMetricsResponse::Ok200(wrapper) => {
            let mut stream = Box::pin(wrapper.into_jsonseq_stream());
            let mut names = Vec::new();
            while let Some(item) = stream.next().await {
                let metric = item.map_err(|error| format!("json-seq decode failed: {error}"))?;
                names.push(metric.name);
            }
            if names != ["m-0", "m-1", "m-2"] {
                return Err(format!("unexpected metric names {names:?}"));
            }
            Ok(format!(
                "{} json-seq records counted item-by-item",
                names.len()
            ))
        }
        capi::ExportMetricsResponse::Unauthorized401(problem) => {
            Err(format!("unexpected 401 problem `{}`", problem.title))
        }
    }
}

async fn step_vendor_echo(client: &Client) -> Result<String, String> {
    let expected_len = VENDOR_CHUNKS * VENDOR_CHUNK_LEN;
    match client
        .post_vendor_document(reqwest::Body::wrap_stream(pattern_producer(
            VENDOR_CHUNKS,
            VENDOR_CHUNK_LEN,
        )))
        .await
        .map_err(|error| error.to_string())?
    {
        capi::PostVendorDocumentResponse::Ok200(wrapper) => {
            let content_type = wrapper
                .response
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            if content_type != "application/vnd.acme.document-v7" {
                return Err(format!("unexpected Content-Type `{content_type}`"));
            }
            let (chunks, bytes) =
                drain_pattern_stream(wrapper.into_bytes_stream(), expected_len).await?;
            if chunks <= 1 {
                return Err("echo collapsed into a single chunk".to_owned());
            }
            Ok(format!("verbatim echo bytes={bytes} chunks={chunks}"))
        }
        capi::PostVendorDocumentResponse::BadRequest400(problem) => {
            Err(format!("unexpected 400 problem `{}`", problem.title))
        }
    }
}

async fn step_probe_status(client: &Client, id: &str, expect: &str) -> Result<String, String> {
    match client
        .probe_status(id)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::ProbeStatusResponse::Ok200(widget) => {
            if expect != "ok" {
                return Err(format!("expected {expect}, got 200 (`{}`)", widget.id));
            }
            Ok(format!("id={id} status=200 variant=Ok200"))
        }
        capi::ProbeStatusResponse::Success2xx { status, body } => {
            if expect != "2xx" {
                return Err(format!("expected {expect}, got Success2xx {status}"));
            }
            if status != http::StatusCode::ACCEPTED || body.data.get("probed").is_none() {
                return Err(format!(
                    "unexpected Success2xx status={status} data={:?}",
                    body.data
                ));
            }
            Ok(format!("id={id} status={status} variant=Success2xx"))
        }
        capi::ProbeStatusResponse::ClientError4xx { status, body } => {
            if expect != "4xx" {
                return Err(format!("expected {expect}, got ClientError4xx {status}"));
            }
            if status != http::StatusCode::CONFLICT {
                return Err(format!("unexpected ClientError4xx status={status}"));
            }
            Ok(format!(
                "id={id} status={status} variant=ClientError4xx title=`{}`",
                body.title
            ))
        }
        capi::ProbeStatusResponse::Default { status, body } => {
            if expect != "default" {
                return Err(format!("expected {expect}, got Default {status}"));
            }
            if status.as_u16() != 599 {
                return Err(format!("unexpected Default status={status}"));
            }
            Ok(format!(
                "id={id} status={status} variant=Default title=`{}`",
                body.title
            ))
        }
    }
}

async fn step_delete_task(client: &Client) -> Result<String, String> {
    match client
        .delete_task("t-1")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::DeleteTaskResponse::NoContent204 => Ok("task t-1 deleted".to_owned()),
        capi::DeleteTaskResponse::NotFound404(problem) => {
            Err(format!("unexpected 404 problem `{}`", problem.title))
        }
    }
}

async fn step_echo_note(
    client: &Client,
    form: &str,
    body: Option<&Option<String>>,
    expected: Option<&str>,
) -> Result<String, String> {
    match client
        .echo_note(body)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::EchoNoteResponse::Ok200(echoed) => {
            if echoed.as_deref() != expected {
                return Err(format!("{form}: echoed {echoed:?}, expected {expected:?}"));
            }
            Ok(format!("{form} -> {:?}", echoed))
        }
    }
}

async fn step_create_account(client: &Client) -> Result<String, String> {
    let write = AccountWrite {
        id: "acc-1".to_owned(),
        password: "s3cret-pw".to_owned(),
        note: OptionalField::Present("demo account".to_owned()),
    };
    match client
        .create_account(&write)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::CreateAccountResponse::Created201(read) => {
            if read.id != "acc-1" || read.note != write.note {
                return Err(format!("unexpected AccountRead {read:?}"));
            }
            // `AccountRead` structurally cannot carry `password` (writeOnly):
            // directionality proven at compile time, round trip at runtime.
            Ok(format!(
                "id={} note preserved; password absent from Read view",
                read.id
            ))
        }
    }
}

async fn step_list_audit_entries(client: &Client) -> Result<String, String> {
    match client
        .list_audit_entries("a-1")
        .await
        .map_err(|error| error.to_string())?
    {
        capi::ListAuditEntriesResponse::Ok200(entry) => {
            if entry.created_at != "2026-08-25T00:00:00Z" {
                return Err(format!("unexpected AuditEntryRead {entry:?}"));
            }
            // `draftNote` is writeOnly: the Read view has NO such field at
            // all (compile-level proof), only createdAt + metadata surface.
            Ok(format!(
                "createdAt={} metadata={:?}; draftNote absent from the Read view",
                entry.created_at, entry.metadata
            ))
        }
    }
}

async fn step_sync_record(client: &Client) -> Result<String, String> {
    let write = SyncedRecordWrite {
        label: "abc".to_owned(),
        secret_token: OptionalField::Present("token-xyz".to_owned()),
    };
    match client
        .sync_record(&write)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::SyncRecordResponse::Ok200(read) => {
            if read.label != write.label || read.id != "synced-1" {
                return Err(format!("unexpected SyncedRecordRead {read:?}"));
            }
            if read.reviewed_by != OptionalField::Present("demo-reviewer".to_owned()) {
                return Err(format!(
                    "readOnly reviewedBy not served: {:?}",
                    read.reviewed_by
                ));
            }
            Ok(format!(
                "label={} id={} reviewedBy served; secretToken never surfaced",
                read.label, read.id
            ))
        }
    }
}

async fn step_create_pet(client: &Client) -> Result<String, String> {
    let pet = Pet::Dog(Dog {
        kind: DogKind::Dog,
        bark: true,
    });
    match client
        .create_pet(&pet)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::CreatePetResponse::Created201(full) => {
            if full.name != OptionalField::Present("rex".to_owned())
                || full.created_at != "2026-08-25T12:00:00Z"
            {
                return Err(format!("unexpected FullWidget {full:?}"));
            }
            Ok(format!(
                "dog -> FullWidget id={} name={:?}",
                full.id, full.name
            ))
        }
    }
}

async fn step_create_record(client: &Client) -> Result<String, String> {
    let record = MatrixRecord {
        req_plain: "rp".to_owned(),
        req_nullable: None,
        opt_plain: OptionalField::Present(7),
        opt_nullable: Some(9),
        status: OptionalField::Present(StringStatus::InReview),
    };
    match client
        .create_record(&record)
        .await
        .map_err(|error| error.to_string())?
    {
        capi::CreateRecordResponse::Created201(echoed) => {
            if echoed != record {
                return Err(format!("matrix round trip diverged: {echoed:?}"));
            }
            Ok("presence/nullability matrix round-tripped verbatim".to_owned())
        }
    }
}

/// Drives EVERY documented operation once through `client` against a running
/// kitchen-sink server, returning one [`SweepStep`] per call. Steps never
/// panic on unexpected outcomes — they record failures so callers can print
/// everything first and then decide how to surface them.
pub async fn run_sweep(client: &Client) -> Vec<SweepStep> {
    let mut steps = Vec::new();

    record(&mut steps, "createWidget", step_create_widget(client).await);
    record(&mut steps, "headWidget", step_head_widget(client).await);
    record(&mut steps, "getWidget", step_get_widget(client).await);
    record(
        &mut steps,
        "createSession",
        step_create_session(client).await,
    );
    record(&mut steps, "putNote", step_put_note(client).await);
    record(&mut steps, "putObject", step_put_object(client).await);
    record(&mut steps, "getObject", step_get_object(client).await);
    record(
        &mut steps,
        "getThumbnail",
        step_thumbnail_json(client).await,
    );
    record(&mut steps, "getThumbnail", step_thumbnail_png(client).await);
    record(
        &mut steps,
        "uploadDocument",
        step_upload_document(client).await,
    );
    record(&mut steps, "streamEvents", step_stream_events(client).await);
    record(
        &mut steps,
        "exportRecords",
        step_export_records(client).await,
    );
    record(&mut steps, "pushMetrics", step_push_metrics(client).await);
    record(
        &mut steps,
        "exportMetrics",
        step_export_metrics(client).await,
    );
    record(
        &mut steps,
        "postVendorDocument",
        step_vendor_echo(client).await,
    );
    record(
        &mut steps,
        "probeStatus",
        step_probe_status(client, "ok-probe", "ok").await,
    );
    record(
        &mut steps,
        "probeStatus",
        step_probe_status(client, "2xx-probe", "2xx").await,
    );
    record(
        &mut steps,
        "probeStatus",
        step_probe_status(client, "4xx-probe", "4xx").await,
    );
    record(
        &mut steps,
        "probeStatus",
        step_probe_status(client, "zzz-default", "default").await,
    );
    record(&mut steps, "deleteTask", step_delete_task(client).await);

    let value = Some(Some("hello note".to_owned()));
    let null = Some(None);
    record(
        &mut steps,
        "echoNote",
        step_echo_note(client, "value form", value.as_ref(), Some("hello note")).await,
    );
    record(
        &mut steps,
        "echoNote",
        step_echo_note(client, "null form", null.as_ref(), None).await,
    );
    record(
        &mut steps,
        "echoNote",
        step_echo_note(client, "absent form", None, Some("[absent]")).await,
    );

    record(
        &mut steps,
        "createAccount",
        step_create_account(client).await,
    );
    record(
        &mut steps,
        "listAuditEntries",
        step_list_audit_entries(client).await,
    );
    record(&mut steps, "syncRecord", step_sync_record(client).await);
    record(&mut steps, "createPet", step_create_pet(client).await);
    record(&mut steps, "createRecord", step_create_record(client).await);

    steps
}

/// Consumes `stream` chunk-by-chunk, verifying every byte against the
/// `% 251` pattern WITHOUT ever aggregating the received body.
async fn drain_pattern_stream<S>(stream: S, expected_len: usize) -> Result<(usize, usize), String>
where
    S: futures_core::Stream<Item = reqwest::Result<Bytes>>,
{
    let mut stream = std::pin::pin!(stream);
    let mut chunks = 0_usize;
    let mut offset = 0_usize;
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|error| format!("stream failed after {offset} bytes: {error}"))?;
        chunks += 1;
        for byte in &chunk {
            if offset >= expected_len {
                return Err(format!("stream exceeded the expected {expected_len} bytes"));
            }
            if *byte != pattern_byte(offset) {
                return Err(format!("byte {offset} diverged from the sent pattern"));
            }
            offset += 1;
        }
    }
    if offset != expected_len {
        return Err(format!("stream ended at {offset} of {expected_len} bytes"));
    }
    Ok((chunks, offset))
}

/// Consumes `stream` chunk-by-chunk against an exact static expectation,
/// counting chunks without accumulating the received body.
async fn drain_exact_stream<S>(stream: S, expected: &[u8]) -> Result<(usize, usize), String>
where
    S: futures_core::Stream<Item = reqwest::Result<Bytes>>,
{
    let mut stream = std::pin::pin!(stream);
    let mut chunks = 0_usize;
    let mut offset = 0_usize;
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|error| format!("stream failed after {offset} bytes: {error}"))?;
        chunks += 1;
        let end = offset + chunk.len();
        if end > expected.len() {
            return Err(format!(
                "stream carried {} bytes beyond the expected {}",
                end - expected.len(),
                expected.len()
            ));
        }
        if expected[offset..end] != chunk[..] {
            return Err(format!(
                "chunk at byte {offset} diverged from the static blob"
            ));
        }
        offset = end;
    }
    if offset != expected.len() {
        return Err(format!(
            "stream ended at {offset} of {} bytes",
            expected.len()
        ));
    }
    Ok((chunks, offset))
}
