//! §50 test 41 fuzz target: NDJSON framing + JSON decode (main spec §19).
//!
//! Invariants over each input:
//! - fed as ONE chunk and as deterministic 1–7-byte chunks;
//! - at generous (4096) and tight (64) per-record limits so
//!   `RecordTooLarge` paths fire too;
//! - items are counted then dropped (never aggregated past the cap), the
//!   poll hang-guard bounds work, and any terminal error is one of the
//!   declared `NdjsonDecodeError` variants followed by a clean `None`.

#![no_main]

#[path = "common/mod.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use openapi_support::ndjson::decode_ndjson;

fuzz_target!(|data: &[u8]| {
    let data = common::capped(data);
    let chunkings = [vec![data.to_vec()], common::pseudo_random_chunks(data)];
    for parts in chunkings {
        for limit in [4096_usize, 64] {
            let stream = decode_ndjson::<serde_json::Value, _, _>(
                common::byte_frames(parts.clone()),
                limit,
            );
            let outcome = common::drive(Box::pin(stream), common::ITEM_CAP);
            assert!(outcome.items() <= common::ITEM_CAP);
        }
    }
});
