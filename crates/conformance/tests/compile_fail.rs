//! Compile-failure conformance for forbidden patterns (main spec §50 test
//! 38's negative half, against §49).
//!
//! Each file under `tests/compile_fail/` is a small standalone crate that
//! misuses a REAL generated surface (the same fixture-01/02/11/15 artifacts
//! the positive suite compiles, exposed through this crate's public
//! `fixtures` modules). trybuild asserts every one fails to typecheck and
//! that the committed `.stderr` snapshots pin the exact diagnostics — so an
//! API change that re-enables a §49-forbidden pattern breaks CI here.

#[test]
fn forbidden_patterns_fail_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
