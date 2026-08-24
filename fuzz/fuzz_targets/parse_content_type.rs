//! §50 test 41 fuzz target: `parse_content_type` (main spec §28/§28.1).
//!
//! Invariant: any byte string is either parsed into a `ParsedMediaType` or
//! rejected with `MalformedContentType` — never defaulted, never panicked.
//! Successful parses additionally drive the §28 precedence matcher
//! (`match_entry`) and, for `multipart/*`, the boundary extraction so the
//! parameter scanner (quoted strings, escapes) is exercised end to end.

#![no_main]

#[path = "common/mod.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use openapi_support::mediatype::{is_wildcard_incoming, match_entry, parse_content_type};

/// Fixed entry keys spanning the §28 precedence ranks: exact, `+json`
/// suffix family, `type/*` range, `*/*` wildcard.
const ENTRIES: [&str; 4] = [
    "application/json",
    "application/vnd.foo+json",
    "text/*",
    "*/*",
];

fuzz_target!(|data: &[u8]| {
    // Lossy conversion keeps the target on the `&str` surface while still
    // letting fuzzer bytes reach every non-ASCII branch.
    let raw = String::from_utf8_lossy(common::capped(data));

    let Ok(parsed) = parse_content_type(&raw) else {
        return; // rejection without panic is exactly the contract
    };

    for entry in ENTRIES {
        let _rank = match_entry(&parsed, entry);
    }
    let _wildcard = is_wildcard_incoming(&parsed);

    if parsed.ty == "multipart" {
        let _boundary =
            openapi_support::multipart::extract_boundary(&parsed);
    }
});
