//! §50 test 41 fuzz target: incremental multipart/form-data framing
//! (main spec §5.5, §17.1).
//!
//! Input format: byte 0 selects a boundary from a fixed table (boundaries
//! are configuration, not attacker-controlled here — they arrive via the
//! Content-Type header in production); the remainder is the raw body. Each
//! body runs through an axum `BodyDataStream` (exactly the server surface)
//! once as ONE frame and once as deterministic 1–7-byte chunks.
//!
//! Invariants: events are counted then dropped (bounded memory), cardinality
//! limits bound part/header/name growth, any terminal error is one of the
//! declared `MultipartError` variants followed by a clean `None`, and the
//! poll hang-guard bounds total work.

#![no_main]

#[path = "common/mod.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use openapi_support::multipart::{stream_multipart, MultipartLimits};

/// Safe boundaries only: CR/LF/NUL can never appear (they would make every
/// input reject instantly and teach libFuzzer nothing).
const BOUNDARIES: [&str; 4] = ["XyZzy123", "Z", "sep", "abcdefgh12345678"];

/// §17.1-shaped limits matching the crate tests' fixture budget, with a
/// slightly higher part count so multi-part fuzz bodies surface events.
fn limits() -> MultipartLimits {
    MultipartLimits {
        max_parts: 16,
        max_header_bytes: 512,
        max_field_name_bytes: 64,
        max_file_name_bytes: 128,
    }
}

fuzz_target!(|data: &[u8]| {
    let data = common::capped(data);
    let Some((selector, body)) = data.split_first() else {
        return;
    };
    let boundary = BOUNDARIES[usize::from(*selector) % BOUNDARIES.len()];

    let chunkings = [vec![body.to_vec()], common::pseudo_random_chunks(body)];
    for parts in chunkings {
        let frames = futures_util::stream::iter(
            parts.into_iter().map(|part| Ok::<bytes::Bytes, axum::Error>(bytes::Bytes::from(part))),
        );
        let body_stream = axum::body::Body::from_stream(frames).into_data_stream();
        let stream = stream_multipart(body_stream, boundary.to_owned(), limits());
        let outcome = common::drive(Box::pin(stream), common::ITEM_CAP);
        assert!(outcome.items() <= common::ITEM_CAP);
    }
});
