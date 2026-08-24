//! Shared harness helpers for the §50 test 41 fuzz targets.
//!
//! Every target feeds bounded input through one of the crate's streaming or
//! parsing surfaces and asserts the fuzz contract: rejection without panic —
//! a malformed body produces the decoder's declared error enum (never an
//! abort, never an unbounded buffer, and never output after a terminal
//! error). Loops are hard-capped so memory and time stay bounded regardless
//! of libFuzzer flags; `-rss_limit_mb` and the libFuzzer timeout remain the
//! outer backstops.

#![allow(dead_code)]

use futures_util::Stream;
use openapi_support::multipart::MultipartError;
use openapi_support::stream_errors::{JsonSeqDecodeError, NdjsonDecodeError, SseDecodeError};
use std::pin::Pin;
use std::task::{Context, Poll};

/// Hard cap on harness input, independent of libFuzzer's `-max_len`: targets
/// truncate before any allocation-heavy work so bounds hold even when run
/// with a huge corpus entry.
pub const MAX_INPUT_BYTES: usize = 32 * 1024;

/// Hard cap on consumed items per driven decode stream (`§50 test 41`
/// "bounded memory"): the consumer drops every item immediately after
/// counting it.
pub const ITEM_CAP: usize = 4096;

/// Hang guard on polls per driven stream. The decoders are poll-driven over
/// always-ready sources, so legitimate runs finish far below this.
pub const POLL_CAP: u64 = 4_000_000;

/// Truncates raw fuzzer input to the harness cap.
#[must_use]
pub fn capped(data: &[u8]) -> &[u8] {
    &data[..data.len().min(MAX_INPUT_BYTES)]
}

/// Deterministic pseudo-random re-chunking into 1–7-byte pieces derived from
/// the input bytes themselves (no RNG state): chunk `i` takes
/// `1 + byte[i mod len] % 7` bytes.
///
/// Splitting at arbitrary byte offsets is the adversarial dimension the
/// property tests cover exhaustively (§50 test 42); here it exercises the
/// same framing paths under mutation.
#[must_use]
pub fn pseudo_random_chunks(data: &[u8]) -> Vec<Vec<u8>> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut index = 0usize;
    while index < data.len() {
        let step = 1 + usize::from(data[index % data.len()] % 7);
        let end = (index + step).min(data.len());
        parts.push(data[index..end].to_vec());
        index = end;
    }
    parts
}

/// Builds the `Result<Bytes, Infallible>` chunk streams the decoders accept
/// (same shape as the in-crate tests use).
#[must_use]
pub fn byte_frames(
    parts: Vec<Vec<u8>>,
) -> impl Stream<Item = Result<bytes::Bytes, std::convert::Infallible>> {
    futures_util::stream::iter(parts.into_iter().map(|part| Ok(bytes::Bytes::from(part))))
}

/// Compile-time-exhaustive "declared enum set" witnesses: matching each
/// decoder error exhaustively documents that rejection can only surface as
/// the documented variants (§39/§40 taxonomy), and gives fuzz crashes a
/// stable place to land if a new variant forgets its bound.
pub trait DeclaredError {
    /// Exhaustive match over the declared variants; panics never — the point
    /// is that the compiler enforces coverage of the whole set.
    fn assert_declared(&self);
}

impl DeclaredError for SseDecodeError {
    fn assert_declared(&self) {
        match self {
            Self::Truncated
            | Self::RecordTooLarge { .. }
            | Self::MalformedJson(_)
            | Self::NotUtf8
            | Self::Source(_) => {}
        }
    }
}

impl DeclaredError for NdjsonDecodeError {
    fn assert_declared(&self) {
        match self {
            Self::Truncated
            | Self::RecordTooLarge { .. }
            | Self::MalformedJson(_)
            | Self::NotUtf8
            | Self::Source(_) => {}
        }
    }
}

impl DeclaredError for JsonSeqDecodeError {
    fn assert_declared(&self) {
        match self {
            Self::Truncated
            | Self::RecordTooLarge { .. }
            | Self::MalformedJson(_)
            | Self::NotUtf8
            | Self::MissingRecordSeparator
            | Self::Source(_) => {}
        }
    }
}

impl DeclaredError for MultipartError {
    fn assert_declared(&self) {
        match self {
            Self::TooManyParts { .. }
            | Self::PartHeaderTooLarge { .. }
            | Self::FieldNameTooLong { .. }
            | Self::FileNameTooLong { .. }
            | Self::Truncated
            | Self::MalformedFraming => {}
        }
    }
}

/// Outcome summary of driving one decoder to termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driven {
    /// Clean end-of-stream after `items` items.
    Clean(usize),
    /// Item cap hit; consumption stopped early (bounded-memory proof).
    Capped(usize),
    /// Terminal decoder error after `items` items (rejection without panic).
    Rejected(usize),
}

impl Driven {
    #[must_use]
    pub const fn items(self) -> usize {
        match self {
            Self::Clean(items) | Self::Capped(items) | Self::Rejected(items) => items,
        }
    }
}

/// Drives a decoder stream to completion with no executor: sources are
/// always-ready chunk iterators, so a no-op-waker poll loop terminates.
///
/// Asserts the fuzz invariants:
/// 1. bounded work — polls stay under [`POLL_CAP`] (hang guard; the libFuzzer
///    timeout is the backstop);
/// 2. bounded memory — consumption stops once `cap` items were surfaced;
/// 3. rejection without panic — errors arrive typed, pass their
///    [`DeclaredError`] exhaustive match, and are terminal (the next poll
///    yields `None`, never more output).
pub fn drive<T, E, S>(mut stream: S, cap: usize) -> Driven
where
    S: Stream<Item = Result<T, E>> + Unpin,
    E: DeclaredError,
{
    let mut pinned = Pin::new(&mut stream);
    // Sources here never park; Waker::noop() suffices (stabilized 1.85).
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut items = 0usize;
    let mut polls = 0u64;
    loop {
        polls += 1;
        assert!(polls <= POLL_CAP, "decoder exceeded the poll hang-guard");
        match pinned.as_mut().poll_next(&mut cx) {
            Poll::Ready(Some(Ok(item))) => {
                drop(item); // never aggregated: count only (bounded memory)
                items += 1;
                if items >= cap {
                    return Driven::Capped(items);
                }
            }
            Poll::Ready(Some(Err(error))) => {
                error.assert_declared();
                polls += 1;
                assert!(polls <= POLL_CAP, "decoder exceeded the poll hang-guard");
                assert!(
                    matches!(pinned.as_mut().poll_next(&mut cx), Poll::Ready(None)),
                    "decoder produced output after a terminal error",
                );
                return Driven::Rejected(items);
            }
            Poll::Ready(None) => return Driven::Clean(items),
            // Always-ready sources make Pending unreachable; if some future
            // source parks, the poll cap above still terminates the target.
            Poll::Pending => {}
        }
    }
}
