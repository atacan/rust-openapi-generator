//! §50 test 41 fuzz target: RFC 7464 JSON Text Sequence framing + JSON
//! decode (main spec §20).
//!
//! Same harness contract as `sse_decode`/`ndjson_decode`; the declared error
//! set additionally includes `MissingRecordSeparator` (bytes outside RS-
//! introduced records must reject, never be guessed into records).

#![no_main]

#[path = "common/mod.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use openapi_support::jsonseq::decode_jsonseq;

fuzz_target!(|data: &[u8]| {
    let data = common::capped(data);
    let chunkings = [vec![data.to_vec()], common::pseudo_random_chunks(data)];
    for parts in chunkings {
        for limit in [4096_usize, 64] {
            let stream = decode_jsonseq::<serde_json::Value, _, _>(
                common::byte_frames(parts.clone()),
                limit,
            );
            let outcome = common::drive(Box::pin(stream), common::ITEM_CAP);
            assert!(outcome.items() <= common::ITEM_CAP);
        }
    }
});
