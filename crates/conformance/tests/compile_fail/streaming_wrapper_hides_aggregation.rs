//! §49 negative proof (compile half): streaming wrappers hide whole-body
//! collectors.
//!
//! The generated `<Op><Status>Stream` payload for fixture 15's `exportRecords`
//! exposes only `into_ndjson_stream` (main spec §19/§32); the §49-forbidden
//! `response.text()` aggregation has no inherent method on the wrapper and
//! must fail to typecheck.

use openapi_conformance::fixtures::fixture_15_streams::client::ExportRecords200Stream;

fn main() {
    let wrapper = ExportRecords200Stream {
        response: ::reqwest::Response::from(::http::Response::new(&b""[..])),
        limits: ::openapi_support::limits::BodyLimits::process_default(),
    };
    let _text = wrapper.text();
}
