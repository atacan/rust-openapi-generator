//! Large-upload demo application (server side).
//!
//! [`LargeUploadApp`] implements the generated [`crate::server::Api`] trait
//! in TWO modes:
//!
//! * **disk mode** (default): every request body is consumed strictly
//!   chunk-by-chunk (`into_data_stream()`), appended to a file under a
//!   UNIQUE `std::env::temp_dir()` directory while an incremental SHA-256
//!   digests the same chunks — nothing ever aggregates the payload;
//! * **proxy mode** (`--proxy-url <base>`): the INBOUND body stream is
//!   wrapped directly into the outbound `reqwest::Body` (`wrap_stream`) and
//!   forwarded to `{base}/blobs/{id}` / `{base}/audio/{id}`, so bytes pass
//!   through without buffering; the upstream's JSON receipt flows back.
//!   Pointing a proxy-mode instance at a disk-mode instance chains three
//!   streaming processes end-to-end.
//!
//! Both operations answer [`large_upload_models::models::UploadReceipt`]
//! ({bytes_received, sha256}) proving the FULL payload was handled without
//! re-downloading it.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use large_upload_memmon as memmon;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::server as sapi;
use large_upload_models::models::{ProblemDetails, UploadReceipt};

fn problem_details(title: &str) -> ProblemDetails {
    ProblemDetails {
        title: title.to_owned(),
    }
}

/// Demo application state: the temp-dir store plus optional proxy target.
pub struct LargeUploadApp {
    store_dir: PathBuf,
    /// Trailing-slash-trimmed proxy base (`None` = disk mode).
    proxy_base: Option<String>,
    /// Reused upstream transport for proxy mode.
    upstream: reqwest::Client,
}

impl LargeUploadApp {
    /// Creates the app. Disk mode stores uploads under `store_dir` (UNIQUE
    /// directory per process under `std::env::temp_dir()`); `proxy_base`
    /// switches to proxy mode, forwarding every body to
    /// `{proxy_base}/blobs|audio/{id}`.
    ///
    /// # Panics
    /// Panics when the shared reqwest transport cannot be built (rare
    /// process-level misconfiguration).
    #[must_use]
    pub fn new(store_dir: PathBuf, proxy_base: Option<String>) -> Self {
        let upstream = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("upstream reqwest client builds");
        Self {
            store_dir,
            proxy_base: proxy_base
                .as_deref()
                .map(|base| base.trim_end_matches('/').to_owned()),
            upstream,
        }
    }

    /// Whether this instance forwards instead of storing.
    #[must_use]
    pub fn is_proxy_mode(&self) -> bool {
        self.proxy_base.is_some()
    }

    /// Safe join of a resource id onto the store directory: rejects empty
    /// ids and anything path-traversal shaped instead of trusting the
    /// segment.
    fn store_path(&self, id: &str) -> Option<PathBuf> {
        if id.is_empty() || matches!(id, "." | "..") || id.contains('/') || id.contains('\\') {
            return None;
        }
        Some(self.store_dir.join(id))
    }

    /// Ensures the store directory exists, then opens `name` inside it for
    /// writing from scratch.
    async fn open_store_file(&self, name: &str) -> std::io::Result<tokio::fs::File> {
        tokio::fs::create_dir_all(&self.store_dir).await?;
        tokio::fs::File::create(self.store_dir.join(name)).await
    }

    /// DISK MODE core: consumes `body` chunk-by-chunk into `<store>/<name>`
    /// while hashing incrementally; periodic [`memmon::ByteProgress`] lines
    /// pair received MiB with the CURRENT rss (flatness made observable).
    async fn stream_body_to_store(
        &self,
        name: &str,
        label: String,
        body: axum::body::Body,
    ) -> Result<UploadReceipt, String> {
        let mut file = self
            .open_store_file(name)
            .await
            .map_err(|error| format!("object storage unavailable: {error}"))?;
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        let mut progress = memmon::ByteProgress::new(label);
        let mut chunks = body.into_data_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk =
                chunk.map_err(|error| format!("request body failed mid-upload: {error}"))?;
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("object write failed: {error}"))?;
            hasher.update(&chunk);
            received += chunk.len() as u64;
            progress.tick(received);
        }
        file.flush()
            .await
            .map_err(|error| format!("object write failed: {error}"))?;
        Ok(UploadReceipt {
            bytes_received: i64::try_from(received).unwrap_or(i64::MAX),
            sha256: digest_hex(hasher),
        })
    }

    /// PROXY MODE core: forwards the inbound stream verbatim as the outbound
    /// body to `{base}{path}` (`wrap_stream` over the raw inbound chunks —
    /// zero buffering), then relays the upstream receipt.
    async fn forward_stream(
        &self,
        path: &str,
        content_type: &'static str,
        body: axum::body::Body,
    ) -> Result<UploadReceipt, String> {
        let base = self
            .proxy_base
            .as_deref()
            .expect("forward_stream requires proxy mode");
        let url = format!("{base}{path}");
        let response = self
            .upstream
            .put(&url)
            .header(::http::header::CONTENT_TYPE, content_type)
            .body(::reqwest::Body::wrap_stream(body.into_data_stream()))
            .send()
            .await
            .map_err(|error| format!("proxy transport to {url} failed: {error}"))?;
        match response.status() {
            ::http::StatusCode::CREATED => {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| format!("proxy receipt read failed: {error}"))?;
                serde_json::from_slice::<UploadReceipt>(&bytes)
                    .map_err(|error| format!("proxy receipt decode failed: {error}"))
            }
            status => Err(format!("proxy upstream answered {status}")),
        }
    }
}

/// Lowercase-hex digest of a finalized SHA-256 hasher.
fn digest_hex(hasher: Sha256) -> String {
    let finalized = hasher.finalize();
    let mut out = String::with_capacity(finalized.len() * 2);
    for byte in finalized {
        use std::fmt::Write as _;
        let _unused = write!(out, "{byte:02x}");
    }
    out
}

#[async_trait]
impl sapi::Api for LargeUploadApp {
    /// Raw octet-stream PUT: disk mode persists chunk-wise; proxy mode
    /// forwards the untouched inbound stream.
    async fn put_blob(&self, id: String, body: axum::body::Body) -> sapi::PutBlobResponse {
        if self.store_path(&id).is_none() {
            return sapi::PutBlobResponse::BadRequest400(problem_details(&format!(
                "blob id rejected: {id}"
            )));
        }
        let outcome = match &self.proxy_base {
            Some(_) => {
                self.forward_stream(&format!("/blobs/{id}"), "application/octet-stream", body)
                    .await
            }
            None => {
                self.stream_body_to_store(&format!("blob-{id}"), format!("recv blob {id}"), body)
                    .await
            }
        };
        match outcome {
            Ok(receipt) => sapi::PutBlobResponse::Created201(receipt),
            Err(message) => sapi::PutBlobResponse::BadRequest400(problem_details(&message)),
        }
    }

    /// audio/wav PUT: identical streaming shape over the second media type —
    /// the generator classifies `audio/*` exactly like octet-stream.
    async fn put_audio_track(
        &self,
        id: String,
        body: axum::body::Body,
    ) -> sapi::PutAudioTrackResponse {
        if self.store_path(&id).is_none() {
            return sapi::PutAudioTrackResponse::BadRequest400(problem_details(&format!(
                "track id rejected: {id}"
            )));
        }
        let outcome = match &self.proxy_base {
            Some(_) => {
                self.forward_stream(&format!("/audio/{id}"), "audio/wav", body)
                    .await
            }
            None => {
                self.stream_body_to_store(&format!("track-{id}"), format!("recv track {id}"), body)
                    .await
            }
        };
        match outcome {
            Ok(receipt) => sapi::PutAudioTrackResponse::Created201(receipt),
            Err(message) => sapi::PutAudioTrackResponse::BadRequest400(problem_details(&message)),
        }
    }
}

/// Wires the generated router around `app` with process-default body limits
/// and the NoOp hook trio (encode overflow §34.1, stream failure §40).
pub fn demo_router(app: Arc<LargeUploadApp>) -> axum::Router {
    use openapi_support::hooks::{
        EncodeOverflowHook, NoOpEncodeOverflowHook, NoOpStreamFailureHook, StreamFailureHook,
    };
    use openapi_support::limits::BodyLimits;

    let limits = BodyLimits::process_default();
    let encode_overflow_hook: Arc<dyn EncodeOverflowHook> = Arc::new(NoOpEncodeOverflowHook);
    let stream_failure_hook: Arc<dyn StreamFailureHook> = Arc::new(NoOpStreamFailureHook);
    sapi::router(app, limits, encode_overflow_hook, stream_failure_hook)
}
