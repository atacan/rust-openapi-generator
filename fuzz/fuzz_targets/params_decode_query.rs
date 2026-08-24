//! §50 test 41 fuzz target: query-parameter form-style decode (companion §6).
//!
//! Invariant: arbitrary bytes, treated as a raw `k=v&k=v` query string and
//! decoded through the OAS form style (`ParamSpec`, explode/allowReserved
//! bits derived from input) either yield a `ParamValue` or a declared
//! `ParamDecodeError` — never a panic. The schema-shaped variants run for
//! every [`ParamShape`] to invert all wire shapes exactly.

#![no_main]

#[path = "common/mod.rs"]
mod common;

use common::DeclaredError as _;
use libfuzzer_sys::fuzz_target;
use openapi_support::params::{
    decode_query, decode_query_shaped, ParamDecodeError, ParamShape, ParamSpec, ParamStyle,
};

impl common::DeclaredError for ParamDecodeError {
    fn assert_declared(&self) {
        match self {
            Self::Malformed(_) | Self::UnsupportedShape(_) => {}
        }
    }
}

/// Wire names the spec pretends the document declared; every pair iterator
/// is offered to whichever name the selector byte picks.
const NAMES: [&str; 4] = ["q", "id", "filter", "tags"];

fuzz_target!(|data: &[u8]| {
    let data = common::capped(data);
    let text = String::from_utf8_lossy(data);

    // Raw pair iterator straight from arbitrary bytes: split on `&`, then on
    // the first `=` per segment (segments without `=` are not pairs).
    let pairs: Vec<(&str, &str)> =
        text.split('&').filter_map(|segment| segment.split_once('=')).collect();

    let sel = data.first().copied().unwrap_or(0);
    let spec = ParamSpec::new(
        NAMES[usize::from(sel) % NAMES.len()],
        // Work-package scope: OAS default form style for query.
        ParamStyle::Form,
        sel & 0b1 != 0,
        sel & 0b10 != 0,
    );

    if let Err(error) = decode_query(&spec, pairs.iter().copied()) {
        error.assert_declared();
    }
    for shape in [ParamShape::Scalar, ParamShape::Array, ParamShape::Object] {
        if let Err(error) = decode_query_shaped(&spec, pairs.iter().copied(), shape) {
            error.assert_declared();
        }
    }
});
