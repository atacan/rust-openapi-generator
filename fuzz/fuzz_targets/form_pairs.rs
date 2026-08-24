//! §50 test 41 fuzz target: form body parsing + flat-struct deserialization
//! (main spec §16).
//!
//! Invariant: arbitrary bytes are rejected with a declared
//! `FormDecodeError` variant or decoded into ordered pairs; those pairs
//! deserialize into a generated-shaped FLAT struct (scalars, optionals,
//! one `Vec<String>` sequence field) without panic. The bounded decode gate
//! (`decode_form_limited`) runs at derived limits to exercise 413 ordering.

#![no_main]

#[path = "common/mod.rs"]
mod common;

use common::DeclaredError as _;
use libfuzzer_sys::fuzz_target;
use openapi_support::form::{
    decode_form_limited, deserialize_form_pairs, parse_form_bytes, FormDecodeError,
};

/// Generated-ish flat form model (§16): scalars, an optional, and one
/// repeated-key sequence field. Unknown keys are ignored by default — the
/// documented serde default behavior this target also exercises.
// Fields are consumed through serde's derived visitor, which dead-code
// analysis cannot see.
#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct FuzzForm {
    name: Option<String>,
    count: Option<u32>,
    ratio: Option<f64>,
    flag: Option<bool>,
    tags: Vec<String>,
}

impl common::DeclaredError for FormDecodeError {
    fn assert_declared(&self) {
        match self {
            Self::Malformed
            | Self::NotUtf8
            | Self::DuplicateField(_)
            | Self::Schema(_)
            | Self::TooLarge { .. } => {}
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // Form grouping rescans pairs linearly; keep input small enough that the
    // O(pairs²) group step stays well inside the fuzzer time budget.
    const MAX_FORM_BYTES: usize = 4 * 1024;
    let data = &common::capped(data)[..common::capped(data).len().min(MAX_FORM_BYTES)];

    match parse_form_bytes(data) {
        Ok(pairs) => {
            // Pair count is bounded by segments in the input itself.
            assert!(pairs.len() <= data.len().max(1));
            if let Err(error) = deserialize_form_pairs::<FuzzForm>(&pairs) {
                error.assert_declared();
            }
        }
        Err(error) => error.assert_declared(),
    }

    // Bounded-gate ordering: at the full length the size gate must pass
    // through to the parser verdict; at half length it may fire TooLarge —
    // both are declared outcomes, neither may panic.
    for limit in [data.len(), data.len() / 2] {
        if let Err(error) = decode_form_limited::<FuzzForm>(data, limit) {
            error.assert_declared();
        }
    }
});
