//! Compile-conformance harness (main spec §50 test 38, compile half).
//!
//! The build script runs every fixture under
//! `crates/generator/fixtures/` through the full public pipeline and writes
//! the four generated artifacts into this crate's `$OUT_DIR`; the per-fixture
//! modules below `include!` them so the emitters' `super::models::…`
//! references resolve inside each fixture's own module tree. Compilation of
//! this crate IS the compile-conformance assertion for all fixtures.
//!
//! Runtime differential round trips live in the integration tests
//! (`tests/roundtrip.rs`, `tests/contract_boundary.rs`); artifact hygiene
//! scanning lives in `tests/compile_clean.rs`.

pub mod fixtures {
    pub mod fixture_01_json_roundtrip {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/01_json_roundtrip/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/01_json_roundtrip/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/01_json_roundtrip/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/01_json_roundtrip/server.rs"));
        }
    }

    pub mod fixture_02_streaming_binary {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/02_streaming_binary/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/02_streaming_binary/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/02_streaming_binary/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/02_streaming_binary/server.rs"));
        }
    }

    pub mod fixture_03_nested_content {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/03_nested_content/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/03_nested_content/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/03_nested_content/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/03_nested_content/server.rs"));
        }
    }

    pub mod fixture_04_status_ranges {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/04_status_ranges/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/04_status_ranges/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/04_status_ranges/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/04_status_ranges/server.rs"));
        }
    }

    pub mod fixture_05_composition {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/05_composition/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/05_composition/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/05_composition/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/05_composition/server.rs"));
        }
    }

    pub mod fixture_06a_oas30 {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/06a_oas30/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/06a_oas30/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/06a_oas30/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/06a_oas30/server.rs"));
        }
    }

    pub mod fixture_06b_oas31 {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/06b_oas31/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/06b_oas31/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/06b_oas31/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/06b_oas31/server.rs"));
        }
    }

    pub mod fixture_07_matrix {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/07_matrix/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/07_matrix/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/07_matrix/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/07_matrix/server.rs"));
        }
    }

    pub mod fixture_08_views {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/08_views/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/08_views/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/08_views/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/08_views/server.rs"));
        }
    }

    pub mod fixture_09_optional_body {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/09_optional_body/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/09_optional_body/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/09_optional_body/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/09_optional_body/server.rs"));
        }
    }

    pub mod fixture_10_forms_headers {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/10_forms_headers/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/10_forms_headers/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/10_forms_headers/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/10_forms_headers/server.rs"));
        }
    }

    pub mod fixture_11_multipart {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/11_multipart/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/11_multipart/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/11_multipart/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/11_multipart/server.rs"));
        }
    }

    pub mod fixture_12_multipart_order {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/12_multipart_order/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/12_multipart_order/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/12_multipart_order/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/12_multipart_order/server.rs"));
        }
    }

    pub mod fixture_13_validation {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/13_validation/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/13_validation/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/13_validation/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/13_validation/server.rs"));
        }
    }

    pub mod fixture_14_negotiation {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/14_negotiation/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/14_negotiation/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/14_negotiation/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/14_negotiation/server.rs"));
        }
    }

    pub mod fixture_15_streams {
        pub mod models {
            include!(concat!(env!("OUT_DIR"), "/15_streams/models.rs"));
        }
        pub mod views {
            include!(concat!(env!("OUT_DIR"), "/15_streams/views.rs"));
        }
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/15_streams/client.rs"));
        }
        pub mod server {
            include!(concat!(env!("OUT_DIR"), "/15_streams/server.rs"));
        }
    }
}
