//! Kitchen-sink demo application.
//!
//! [`KitchenSinkApp`] is the single Mode A application implementing all 22
//! documented operations of the generated [`crate::server::Api`] trait
//! over static in-memory data; binary payloads live as files under a UNIQUE
//! `std::env::temp_dir()` directory and are written/read strictly chunk-wise
//! (never aggregated). [`demo_router`] wires the generated router around the
//! shared by the binary and the client crate's ignored smoke test.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use openapi_support::hooks::{
    EncodeOverflowHook, NoOpEncodeOverflowHook, NoOpStreamFailureHook, StreamFailureHook,
};
use openapi_support::limits::BodyLimits;
use openapi_support::optional::OptionalField;
use openapi_support::stream_errors::ServerStreamError;
use tokio::io::AsyncWriteExt;

use crate::server as sapi;
use kitchen_sink_models::models::{
    Account, Ack, CreateSessionForm, CreateWidget, Document, Event, FullWidget, MatrixRecord,
    Metric, Pet, ProblemDetails, Record, Session, SuccessEnvelope, ThumbnailMetadata, Widget,
};
use kitchen_sink_models::views::{
    AccountRead, AuditEntryRead, SyncedRecordRead, SyncedRecordWrite,
};

/// Static PNG-flavored blob served by the `Any` wildcard branch of
/// `getThumbnail` (small on purpose; it still streams chunk-by-chunk).
/// MIRROR of `kitchen_sink_client::sweep::PNG_BLOB` (the two crates stay
/// independently compilable, so the served bytes are repeated on purpose):
/// any divergence makes the client sweep fail loudly at runtime.
pub const PNG_BLOB: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
];

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

/// Wires the generated router around `app` with process-default body limits
/// and the NoOp hook trio (encode overflow §34.1, stream failure §40).
pub fn demo_router(app: Arc<KitchenSinkApp>) -> axum::Router {
    let limits = BodyLimits::process_default();
    let encode_overflow_hook: Arc<dyn EncodeOverflowHook> = Arc::new(NoOpEncodeOverflowHook);
    let stream_failure_hook: Arc<dyn StreamFailureHook> = Arc::new(NoOpStreamFailureHook);
    sapi::router(app, limits, encode_overflow_hook, stream_failure_hook)
}
