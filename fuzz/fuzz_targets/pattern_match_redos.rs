//! §50 test 41 fuzz target: pattern matcher ReDoS resistance (companion §9).
//!
//! Input format: byte 0 caps the pattern length (≤ 48 bytes); next bytes are
//! the ECMA-262-subset pattern; the remainder (≤ 4 KiB) is the subject.
//!
//! Invariant: [`evaluate_pattern`] always RETURNS within its internal step
//! budget — `Match`, `NoMatch`, or `Unsupported` — never hangs on
//! catastrophic-backtracking shapes like `(a+)+$`; the exhaustive decision
//! match plus the libFuzzer timeout are the observable guards.
//! `validate_string` additionally exercises the lenient-skip policy
//! (`Unsupported` must skip, never reject).

#![no_main]

#[path = "common/mod.rs"]
mod common;

use libfuzzer_sys::fuzz_target;
use openapi_support::validation::{
    evaluate_pattern, validate_string, PatternDecision, StringConstraints,
};

/// Longest pattern accepted per run; keeps compile cost bounded.
const MAX_PATTERN_BYTES: usize = 48;

/// Subject cap: long enough to reach deep backtracking states, short enough
/// that even quadratic behavior stays inside the fuzzer time budget.
const MAX_SUBJECT_BYTES: usize = 4 * 1024;

fuzz_target!(|data: &[u8]| {
    let data = common::capped(data);
    let Some((&length_byte, rest)) = data.split_first() else {
        return;
    };

    let pattern_len = length_byte as usize;
    let pattern_len = pattern_len.min(MAX_PATTERN_BYTES).min(rest.len());
    let pattern = String::from_utf8_lossy(&rest[..pattern_len]);
    let subject_raw = &rest[pattern_len..];
    let subject =
        String::from_utf8_lossy(&subject_raw[..subject_raw.len().min(MAX_SUBJECT_BYTES)]);

    // Must RETURN (budgeted), one of exactly three decisions, never panic.
    match evaluate_pattern(&pattern, &subject) {
        PatternDecision::Match | PatternDecision::NoMatch | PatternDecision::Unsupported => {}
    }

    let constraints = StringConstraints {
        pattern: Some(&pattern),
        min_length: None,
        max_length: None,
    };
    // Lenient-skip path: Unsupported patterns must NOT produce violations,
    // and with the other constraints unset only `Violation::Pattern` may
    // ever surface.
    if let Err(violation) = validate_string(&subject, &constraints) {
        assert!(
            matches!(violation, openapi_support::validation::Violation::Pattern { .. }),
            "only a pattern violation may surface from these constraints: {violation}"
        );
    }
});
