//! Compile-conformance harness (main spec §50 test 38, compile half).
//!
//! The build script runs every fixture under
//! `crates/generator/fixtures/` through the full public pipeline and writes
//! the four generated artifacts plus a `mod.rs` file-module index into this
//! crate's `$OUT_DIR`; the per-fixture modules below `include!` one index
//! each. The index declares the artifacts as `#[path]` file modules so the
//! emitters' `super::models::…` references resolve inside each fixture's own
//! module tree AND the artifacts' `//!` module docs compile as real file
//! modules (issue #6: `//!` headers are rejected under a direct
//! `pub mod … { include!(…) }`, E0753). Compilation of this crate IS the
//! compile-conformance assertion for all fixtures.
//!
//! Runtime differential round trips live in the integration tests
//! (`tests/roundtrip.rs`, `tests/contract_boundary.rs`); artifact hygiene
//! scanning lives in `tests/compile_clean.rs`.

pub mod fixtures {
    pub mod fixture_01_json_roundtrip {
        include!(concat!(env!("OUT_DIR"), "/01_json_roundtrip/mod.rs"));
    }

    pub mod fixture_02_streaming_binary {
        include!(concat!(env!("OUT_DIR"), "/02_streaming_binary/mod.rs"));
    }

    pub mod fixture_03_nested_content {
        include!(concat!(env!("OUT_DIR"), "/03_nested_content/mod.rs"));
    }

    pub mod fixture_04_status_ranges {
        include!(concat!(env!("OUT_DIR"), "/04_status_ranges/mod.rs"));
    }

    pub mod fixture_05_composition {
        include!(concat!(env!("OUT_DIR"), "/05_composition/mod.rs"));
    }

    pub mod fixture_06a_oas30 {
        include!(concat!(env!("OUT_DIR"), "/06a_oas30/mod.rs"));
    }

    pub mod fixture_06b_oas31 {
        include!(concat!(env!("OUT_DIR"), "/06b_oas31/mod.rs"));
    }

    pub mod fixture_07_matrix {
        include!(concat!(env!("OUT_DIR"), "/07_matrix/mod.rs"));
    }

    pub mod fixture_08_views {
        include!(concat!(env!("OUT_DIR"), "/08_views/mod.rs"));
    }

    pub mod fixture_09_optional_body {
        include!(concat!(env!("OUT_DIR"), "/09_optional_body/mod.rs"));
    }

    pub mod fixture_10_forms_headers {
        include!(concat!(env!("OUT_DIR"), "/10_forms_headers/mod.rs"));
    }

    pub mod fixture_11_multipart {
        include!(concat!(env!("OUT_DIR"), "/11_multipart/mod.rs"));
    }

    pub mod fixture_12_multipart_order {
        include!(concat!(env!("OUT_DIR"), "/12_multipart_order/mod.rs"));
    }

    pub mod fixture_13_validation {
        include!(concat!(env!("OUT_DIR"), "/13_validation/mod.rs"));
    }

    pub mod fixture_14_negotiation {
        include!(concat!(env!("OUT_DIR"), "/14_negotiation/mod.rs"));
    }

    pub mod fixture_15_streams {
        include!(concat!(env!("OUT_DIR"), "/15_streams/mod.rs"));
    }

    /// Fixture 16 under the DEFAULT configuration (all codecs OFF): every
    /// XML/CBOR/MessagePack entry stays in the §5.9 raw-streaming fallback.
    /// Compilation of this module IS the raw-fallback conformance proof.
    pub mod fixture_16_codecs {
        include!(concat!(env!("OUT_DIR"), "/16_codecs/mod.rs"));
    }

    /// Fixture 16 with ONLY the `xml` codec enabled (main spec §45).
    pub mod fixture_16_codecs_xml {
        include!(concat!(env!("OUT_DIR"), "/16_codecs.xml/mod.rs"));
    }

    /// Fixture 16 with ONLY the `cbor` codec enabled.
    pub mod fixture_16_codecs_cbor {
        include!(concat!(env!("OUT_DIR"), "/16_codecs.cbor/mod.rs"));
    }

    /// Fixture 16 with ONLY the `msgpack` codec enabled.
    pub mod fixture_16_codecs_msgpack {
        include!(concat!(env!("OUT_DIR"), "/16_codecs.msgpack/mod.rs"));
    }

    /// Fixture 16 with every codec enabled PLUS ForceStreaming overrides on
    /// the JSON operation and the XML operation (D-impl-override-precedence:
    /// override > claiming plugin).
    pub mod fixture_16_codecs_force_stream {
        include!(concat!(env!("OUT_DIR"), "/16_codecs.force-stream/mod.rs"));
    }

    /// Fixture 01 regenerated with §30.2 gzip decompression enabled: the
    /// emitted builder pre-wires `.gzip(true)`, so every structured response
    /// this client collects has its content coding removed BENEATH the
    /// bounded collectors (§50 test 32's end-to-end half). Compilation of
    /// this module is also the proof that emitted decompression opt-ins
    /// compile against a Reqwest built with the matching feature.
    pub mod fixture_01_json_roundtrip_gzip {
        include!(concat!(env!("OUT_DIR"), "/01_json_roundtrip.gzip/mod.rs"));
    }

    /// Fixture 17 (§35 HEAD probes): typed response headers WITHOUT any body
    /// accessor on the client, and a server wrapper carrying exactly those
    /// fields.
    pub mod fixture_17_head {
        include!(concat!(env!("OUT_DIR"), "/17_head/mod.rs"));
    }
}
