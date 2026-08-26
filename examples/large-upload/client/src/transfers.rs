//! Large-upload streaming-transfer driver (client side).
//!
//! [`run_transfers`] synthesizes a deterministic WAV file of configurable
//! size (written chunk-wise, hashed incrementally — never held in memory),
//! uploads the SAME file through BOTH documented media types via
//! `reqwest::Body::wrap_stream(ReaderStream…)`, verifies both receipts, and
//! wraps the uploads in a [`large_upload_memmon::Monitor`] so callers can
//! enforce the bounded-memory threshold. The client binary and the smoke
//! tests share it verbatim.

use std::path::Path;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::task::{Context, Poll};

use bytes::Bytes;
use large_upload_memmon as memmon;
use openapi_support::client_error::ClientError;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};
use tokio_util::io::ReaderStream;

use crate::client::{Client, ClientBuilder, PutAudioTrackResponse, PutBlobResponse};
use large_upload_models::models::{ProblemDetails, UploadReceipt};

/// Byte length of one I/O chunk used everywhere (file synthesis, the
/// client-side `ReaderStream` capacity, disk writes stay wire-chunked):
/// comfortably above syscall noise, five orders of magnitude below the
/// payloads it streams.
pub const TRANSFER_CHUNK_BYTES: usize = 256 * 1024;

/// Length of the canonical PCM WAV header synthesized by [`wave_header`].
pub const WAVE_HEADER_LEN: usize = 44;

/// Builds the canonical 44-byte PCM WAV header (RIFF/WAVE, fmt + data) for
/// `data_len` bytes of 16-bit stereo @ 44.1 kHz payload.
#[must_use]
pub fn wave_header(data_len: u32) -> [u8; WAVE_HEADER_LEN] {
    let riff_size = 36_u32
        .checked_add(data_len)
        .expect("wav payload must fit u32");
    let mut header = [0_u8; WAVE_HEADER_LEN];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..18].copy_from_slice(&16_u16.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1_u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&2_u16.to_le_bytes()); // channels
    header[24..28].copy_from_slice(&44_100_u32.to_le_bytes()); // sample rate
    header[28..32].copy_from_slice(&(44_100_u32 * 4).to_le_bytes()); // byte rate
    header[32..34].copy_from_slice(&4_u16.to_le_bytes()); // block align
    header[34..36].copy_from_slice(&16_u16.to_le_bytes()); // bits/sample
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_len.to_le_bytes());
    header
}

/// Deterministic payload byte at absolute file `offset` (`% 251` keeps it in
/// one byte without ever materializing the whole payload).
#[must_use]
fn pattern_byte(offset: usize) -> u8 {
    (offset % 251) as u8
}

/// One patterned chunk starting at absolute `offset`.
fn pattern_chunk(offset: usize, len: usize) -> Bytes {
    let mut data = Vec::with_capacity(len);
    for index in 0..len {
        data.push(pattern_byte(offset + index));
    }
    Bytes::from(data)
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

/// Streams `path` through SHA-256 chunk-wise (bounded memory even here).
///
/// # Errors
/// Propagates filesystem errors.
pub async fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut reader = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; TRANSFER_CHUNK_BYTES];
    loop {
        let read = tokio::io::AsyncReadExt::read(&mut reader, &mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(digest_hex(hasher))
}

/// Synthesizes a deterministic WAV file at `path` with `data_len` bytes of
/// patterned payload (plus [`WAVE_HEADER_LEN`] header), written and hashed
/// strictly chunk-wise. Returns the lowercase-hex SHA-256 over the WHOLE
/// file (header included).
///
/// # Errors
/// Propagates filesystem errors.
pub async fn generate_wave_file(path: &Path, data_len: u64) -> std::io::Result<String> {
    let data_len_u32 =
        u32::try_from(data_len).map_err(|_| std::io::Error::other("wav payload exceeds u32"))?;
    let mut writer = tokio::io::BufWriter::with_capacity(
        TRANSFER_CHUNK_BYTES,
        tokio::fs::File::create(path).await?,
    );
    let mut hasher = Sha256::new();

    let header = wave_header(data_len_u32);
    writer.write_all(&header).await?;
    hasher.update(header);

    let mut written = 0_u64;
    while written < data_len {
        let len =
            usize::try_from((data_len - written).min(TRANSFER_CHUNK_BYTES as u64)).unwrap_or(1);
        let chunk = pattern_chunk(WAVE_HEADER_LEN + written as usize, len);
        writer.write_all(&chunk).await?;
        hasher.update(&chunk);
        written += len as u64;
    }
    writer.flush().await?;
    Ok(digest_hex(hasher))
}

/// Placeholder report for runs aborted before any transfer started.
fn aborted_report() -> memmon::Report {
    memmon::Report {
        role: "client".to_owned(),
        baseline_rss: 0,
        sampled_peak_rss: 0,
        kernel_high_water_rss: 0,
        elapsed: std::time::Duration::ZERO,
    }
}

/// Builds the generated client against `base_url` with process defaults.
pub fn build_client(base_url: &str) -> Result<Client, ClientError> {
    ClientBuilder::new().base_url(base_url.to_owned()).build()
}

/// One recorded outcome of [`run_transfers`]: `ok == false` means an
/// assertion inside the step failed (transport error, receipt mismatch…).
#[derive(Debug, Clone)]
pub struct StepOutcome {
    /// Step under test (`generate`, `putBlob`, `putAudioTrack`).
    pub op: &'static str,
    /// Whether every in-step assertion held.
    pub ok: bool,
    /// Human-readable summary or failure reason.
    pub message: String,
}

/// Wraps any [`AsyncRead`] and counts every byte that leaves it — feeds the
/// live progress printer while `ReaderStream` turns reads into wire chunks.
struct CountingReader<R> {
    inner: R,
    counter: Arc<AtomicU64>,
}

impl<R: AsyncRead + Unpin> AsyncRead for CountingReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Self is Unpin whenever R is (plain fields only).
        let this = self.get_mut();
        let filled_before = buf.filled().len();
        let result = std::task::ready!(Pin::new(&mut this.inner).poll_read(cx, buf));
        this.counter.fetch_add(
            (buf.filled().len() - filled_before) as u64,
            Ordering::Relaxed,
        );
        Poll::Ready(result)
    }
}

/// Verifies a receipt against locally-known size + digest.
fn receipt_matches(receipt: &UploadReceipt, total: u64, sha256: &str) -> Result<(), String> {
    let received = u64::try_from(receipt.bytes_received).unwrap_or(u64::MAX);
    if received != total {
        return Err(format!(
            "receipt byte count mismatch: got {}, expected {total}",
            receipt.bytes_received
        ));
    }
    if receipt.sha256 != sha256 {
        return Err(format!(
            "receipt sha256 mismatch: got {}, expected {sha256}",
            receipt.sha256
        ));
    }
    Ok(())
}

/// Outcome of one issued generated call, pre-flattened to
/// `receipt | problem | transport-message`. Lifetime-parameterized because
/// the generated futures borrow the client.
type ReceiptFuture<'a> = Pin<
    Box<
        dyn std::future::Future<Output = Result<Result<UploadReceipt, ProblemDetails>, String>>
            + Send
            + 'a,
    >,
>;

/// One full streaming upload of `path` through `upload`: opens the file,
/// wraps it `CountingReader → ReaderStream → reqwest::Body`, prints paired
/// progress/RSS lines while chunks leave, awaits the flattened attempt, and
/// verifies the receipt against the locally-known size/digest.
async fn upload_file(
    label: String,
    path: &Path,
    total: u64,
    sha256: &str,
    client: &Client,
    id: &str,
    issue: for<'a> fn(&'a Client, &'a str, ::reqwest::Body) -> ReceiptFuture<'a>,
) -> Result<String, String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let counter = Arc::new(AtomicU64::new(0));
    let reader = CountingReader {
        inner: file,
        counter: Arc::clone(&counter),
    };
    let stream = ReaderStream::with_capacity(reader, TRANSFER_CHUNK_BYTES);
    let printer = memmon::ProgressPrinter::spawn(label.clone(), Arc::clone(&counter), total);
    let attempt = issue(client, id, ::reqwest::Body::wrap_stream(stream)).await;
    printer.finish().await;
    match attempt {
        Err(message) => Err(message),
        Ok(Err(problem)) => Err(format!("server rejected upload: {}", problem.title)),
        Ok(Ok(receipt)) => receipt_matches(&receipt, total, sha256).map(|()| {
            format!(
                "{label}: {} bytes accepted, sha256 verified",
                receipt.bytes_received
            )
        }),
    }
}

/// Issues `PUT /blobs/{id}` with `body`, flattening the exhaustive status
/// enum into the shared receipt-attempt shape.
fn issue_put_blob<'a>(client: &'a Client, id: &'a str, body: ::reqwest::Body) -> ReceiptFuture<'a> {
    let future = client.put_blob(id, body);
    Box::pin(async move {
        future
            .await
            .map_err(|error| format!("transport failed: {error}"))
            .map(|response| match response {
                PutBlobResponse::Created201(receipt) => Ok(receipt),
                PutBlobResponse::BadRequest400(problem) => Err(problem),
            })
    })
}

/// Issues `PUT /audio/{id}` with `body` (same flattening).
fn issue_put_audio_track<'a>(
    client: &'a Client,
    id: &'a str,
    body: ::reqwest::Body,
) -> ReceiptFuture<'a> {
    let future = client.put_audio_track(id, body);
    Box::pin(async move {
        future
            .await
            .map_err(|error| format!("transport failed: {error}"))
            .map(|response| match response {
                PutAudioTrackResponse::Created201(receipt) => Ok(receipt),
                PutAudioTrackResponse::BadRequest400(problem) => Err(problem),
            })
    })
}

/// Full client-side demonstration over ONE synthesized WAV file uploaded
/// through BOTH documented media types:
///
/// 1. `generate` — synthesize `<work_dir>/track.wav` (`size_mib` MiB payload);
/// 2. `putBlob` — stream it through `PUT /blobs/{id}` (octet-stream);
/// 3. `putAudioTrack` — stream the SAME bytes through `PUT /audio/{id}`
///    (audio/wav), verifying each receipt against the local size/digest.
///
/// Uploads run under a [`memmon::Monitor`]; the final report is returned so
/// binaries and tests can enforce the bounded-memory threshold.
pub async fn run_transfers(
    client: &Client,
    work_dir: &Path,
    size_mib: u64,
) -> (Vec<StepOutcome>, memmon::Report) {
    let data_len = size_mib * 1024 * 1024;
    let total = WAVE_HEADER_LEN as u64 + data_len;
    let path = work_dir.join("track.wav");

    let mut steps = Vec::new();
    let expected_sha256 = match generate_wave_file(&path, data_len).await {
        Ok(sha256) => {
            steps.push(StepOutcome {
                op: "generate",
                ok: true,
                message: format!(
                    "synthesized {} ({} MiB payload) chunk-wise",
                    path.display(),
                    size_mib
                ),
            });
            sha256
        }
        Err(error) => {
            steps.push(StepOutcome {
                op: "generate",
                ok: false,
                message: format!("FAILED: {error}"),
            });
            return (steps, aborted_report());
        }
    };

    let monitor = memmon::Monitor::start("client");

    let blob_outcome = upload_file(
        "send blob demo-blob".to_owned(),
        &path,
        total,
        &expected_sha256,
        client,
        "demo-blob",
        issue_put_blob,
    )
    .await;
    steps.push(match blob_outcome {
        Ok(message) => StepOutcome {
            op: "putBlob",
            ok: true,
            message,
        },
        Err(message) => StepOutcome {
            op: "putBlob",
            ok: false,
            message,
        },
    });

    let track_outcome = upload_file(
        "send track demo-track".to_owned(),
        &path,
        total,
        &expected_sha256,
        client,
        "demo-track",
        issue_put_audio_track,
    )
    .await;
    steps.push(match track_outcome {
        Ok(message) => StepOutcome {
            op: "putAudioTrack",
            ok: true,
            message,
        },
        Err(message) => StepOutcome {
            op: "putAudioTrack",
            ok: false,
            message,
        },
    });

    let report = monitor.finish().await;
    (steps, report)
}
