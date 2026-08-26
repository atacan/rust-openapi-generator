//! Process memory monitoring for the large-upload demo.
//!
//! Two complementary measurement sources, because neither alone proves the
//! "memory never grows with payload size" claim:
//!
//! * **Sampled RSS** (`memory-stats`, resident set size of the own process,
//!   mach `task_info` on macOS / `/proc` on Linux): a 50 ms background
//!   sampler tracks the running maximum so the SHAPE of the run stays
//!   visible (baseline vs peak during the transfer).
//! * **Kernel high-water mark** (getrusage(2) `ru_maxrss`): maintained by
//!   the kernel for the whole process lifetime, so even a spike shorter than
//!   one sampling interval cannot hide from the final verdict.
//!
//! Both are compared against the RSS baseline captured right before the
//! transfers start; the demo fails (non-zero exit / failed assertion) when
//! the delta exceeds [`DEFAULT_MAX_RSS_DELTA_MIB`] (overridable via the
//! `LARGE_UPLOAD_MAX_RSS_DELTA_MIB` environment variable).
//!
//! Also provided: the progress printers used during transfers — a polled
//! task for the client side ([`ProgressPrinter`]) and an inline tick helper
//! for server handlers ([`ByteProgress`]) — so every printed line pairs
//! bytes transferred with the CURRENT rss, making flatness observable live.
//!
//! This module lives ONCE as plain source under `memmon/mod.rs` — not a
//! Cargo crate — and is included by BOTH transport crates
//! (`large-upload-client` and `large-upload-server` via
//! `#[path = "../../memmon/mod.rs"]`, relative to each crate's `src/lib.rs`)
//! precisely so neither depends on the other; it pulls in no axum/reqwest
//! of its own. The host manifests carry its three dependencies (tokio time,
//! memory-stats, libc) directly.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

/// How often the background sampler re-reads the process RSS.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(50);

/// Progress lines are emitted every time this many bytes crossed the wire.
const PROGRESS_MARK_BYTES: u64 = 128 * 1024 * 1024;

/// Default bound on (peak − baseline) RSS for a transfer to count as
/// streaming-clean. Deliberately generous against noise pages yet three
/// orders of magnitude below the 1 GiB payload it must NOT grow with.
pub const DEFAULT_MAX_RSS_DELTA_MIB: u64 = 32;

/// Environment variable overriding the threshold (`MiB` units).
pub const THRESHOLD_ENV_VAR: &str = "LARGE_UPLOAD_MAX_RSS_DELTA_MIB";

/// Effective per-process threshold in MiB (env override aware).
#[must_use]
pub fn threshold_mib() -> u64 {
    std::env::var(THRESHOLD_ENV_VAR)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_RSS_DELTA_MIB)
}

/// Current resident set size of THIS process, in bytes (`None` if the OS
/// query fails — callers treat that as "skip this sample").
#[must_use]
fn current_rss() -> Option<u64> {
    memory_stats::memory_stats().map(|stats| stats.physical_mem as u64)
}

/// Kernel-maintained high-water-mark RSS of THIS process, in bytes.
///
/// macOS reports `ru_maxrss` in BYTES and Linux in KiB (see getrusage(2),
/// NOTES); normalized to bytes here.
// Sole FFI site of the crate (lints set `unsafe_code = "deny"`): two plain
// libc calls with a zero-initialized out-struct; no invariants to uphold.
#[allow(unsafe_code)]
#[must_use]
fn kernel_max_rss() -> Option<u64> {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if rc != 0 {
        return None;
    }
    #[cfg(target_os = "macos")]
    let bytes = usage.ru_maxrss as u64;
    #[cfg(not(target_os = "macos"))]
    let bytes = u64::try_from(usage.ru_maxrss).ok()?.saturating_mul(1024);
    Some(bytes)
}

/// Formats a byte count as `NN.N MiB`.
#[must_use]
pub fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

/// Final outcome of one monitored run: both measurements plus timing.
#[derive(Debug, Clone)]
pub struct Report {
    /// Human-readable role for the header line (`client`, `server`, …).
    pub role: String,
    /// RSS sampled immediately before the monitored work started.
    pub baseline_rss: u64,
    /// Maximum RSS observed by the 50 ms sampler during the run.
    pub sampled_peak_rss: u64,
    /// Kernel high-water mark over the WHOLE process lifetime
    /// (`ru_maxrss`; 0 when the syscall was unavailable).
    pub kernel_high_water_rss: u64,
    /// Wall-clock duration of the monitored span.
    pub elapsed: Duration,
}

impl Report {
    /// Peak delta against the baseline across BOTH measurement sources
    /// (saturating at zero).
    #[must_use]
    pub fn delta_bytes(&self) -> u64 {
        self.sampled_peak_rss
            .max(self.kernel_high_water_rss)
            .saturating_sub(self.baseline_rss)
    }

    /// Whether the run breached `limit_mib`.
    #[must_use]
    pub fn breaches(&self, limit_mib: u64) -> bool {
        self.delta_bytes() > limit_mib * 1024 * 1024
    }

    /// Multi-line human-readable summary including the pass/fail verdict.
    #[must_use]
    pub fn render(&self, limit_mib: u64) -> String {
        let verdict = if self.breaches(limit_mib) {
            "FAIL"
        } else {
            "PASS"
        };
        format!(
            "=== memory report [{role}] ===\n\
             baseline RSS       : {:>10} MiB\n\
             sampled peak RSS   : {:>10} MiB   (sampler @ {interval:?})\n\
             kernel high-water  : {:>10} MiB   (getrusage ru_maxrss)\n\
             peak delta vs base : {:>10} MiB   (limit {limit} MiB -> {verdict})\n\
             elapsed            : {:>10.1} s",
            mib(self.baseline_rss),
            mib(self.sampled_peak_rss),
            mib(self.kernel_high_water_rss),
            mib(self.delta_bytes()),
            self.elapsed.as_secs_f32(),
            role = self.role,
            interval = SAMPLE_INTERVAL,
            limit = limit_mib,
        )
    }
}

/// Running monitor: captures the baseline at construction, samples RSS in a
/// background task, and folds everything into a final [`Report`] on
/// [`Self::finish`]. Must be created inside a tokio runtime (the sampler is
/// a spawned task); cheap to clone-free move into `finish`.
pub struct Monitor {
    stop: Arc<AtomicBool>,
    sampler: JoinHandle<()>,
    started: Instant,
    baseline_rss: u64,
    sampled_peak_rss: Arc<AtomicU64>,
    role: &'static str,
}

impl Monitor {
    /// Starts sampling: records the CURRENT RSS as the baseline and spawns
    /// the 50 ms peak tracker.
    ///
    /// # Panics
    /// Panics outside a tokio runtime (task spawn) or when the very first
    /// RSS read fails (the whole demo premise is unmeasurable then).
    #[must_use]
    pub fn start(role: &'static str) -> Self {
        let baseline_rss =
            current_rss().expect("initial RSS sample failed; cannot demonstrate bounded memory");
        let stop = Arc::new(AtomicBool::new(false));
        let sampled_peak_rss = Arc::new(AtomicU64::new(baseline_rss));
        let sampler_stop = Arc::clone(&stop);
        let sampler_peak = Arc::clone(&sampled_peak_rss);
        let sampler = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while !sampler_stop.load(Ordering::Relaxed) {
                ticker.tick().await;
                if let Some(rss) = current_rss() {
                    sampler_peak.fetch_max(rss, Ordering::Relaxed);
                }
            }
        });
        Self {
            stop,
            sampler,
            started: Instant::now(),
            baseline_rss,
            sampled_peak_rss,
            role,
        }
    }

    /// Stops the sampler and produces the final [`Report`].
    pub async fn finish(self) -> Report {
        self.stop.store(true, Ordering::Relaxed);
        let _unused = self.sampler.await;
        Report {
            role: self.role.to_owned(),
            baseline_rss: self.baseline_rss,
            sampled_peak_rss: self.sampled_peak_rss.load(Ordering::Relaxed),
            kernel_high_water_rss: kernel_max_rss().unwrap_or(0),
            elapsed: self.started.elapsed(),
        }
    }
}

/// Client-side progress printer: polls a shared transferred-bytes counter
/// every 100 ms and prints one line per crossed [`PROGRESS_MARK_BYTES`]
/// boundary plus the closing line, each pairing bytes sent with the CURRENT
/// process RSS (flatness made observable live).
pub struct ProgressPrinter {
    handle: JoinHandle<()>,
}

impl ProgressPrinter {
    /// Starts printing progress for `label` until `counter` reaches `total`.
    #[must_use]
    pub fn spawn(label: String, counter: Arc<AtomicU64>, total: u64) -> Self {
        let handle = tokio::spawn(async move {
            let total_mib = total / (1024 * 1024);
            let mut printed_mark = 0_u64;
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let done = counter.load(Ordering::Relaxed);
                let mark = done / PROGRESS_MARK_BYTES;
                let finished = done >= total;
                if mark > printed_mark || finished {
                    printed_mark = mark;
                    let rss = current_rss().map(mib).unwrap_or_else(|| "?".into());
                    println!(
                        "[{label}] {}/{} MiB sent, rss={rss} MiB",
                        done / (1024 * 1024),
                        total_mib
                    );
                }
                if finished {
                    break;
                }
            }
        });
        Self { handle }
    }

    /// Stops the printer (aborts if still waiting on a stalled transfer).
    pub async fn finish(self) {
        self.handle.abort();
        let _unused = self.handle.await;
    }
}

/// Server-side inline progress ticker: handlers call [`Self::tick`] once per
/// received chunk; a line is printed per crossed [`PROGRESS_MARK_BYTES`]
/// boundary, pairing bytes received with the CURRENT process RSS.
#[derive(Debug)]
pub struct ByteProgress {
    label: String,
    next_mark: u64,
}

impl ByteProgress {
    /// New ticker printing under `label` starting at the first
    /// [`PROGRESS_MARK_BYTES`] boundary.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            next_mark: PROGRESS_MARK_BYTES,
        }
    }

    /// Records `bytes_so_far` and prints when a boundary was crossed.
    pub fn tick(&mut self, bytes_so_far: u64) {
        if bytes_so_far < self.next_mark {
            return;
        }
        // Advance past every boundary crossed by one (large) chunk.
        while self.next_mark <= bytes_so_far {
            self.next_mark += PROGRESS_MARK_BYTES;
        }
        let rss = current_rss().map(mib).unwrap_or_else(|| "?".into());
        println!(
            "[{}] {} MiB received, rss={rss} MiB",
            self.label,
            bytes_so_far / (1024 * 1024)
        );
    }
}
