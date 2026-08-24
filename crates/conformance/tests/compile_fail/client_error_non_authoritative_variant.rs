//! §36 negative proof (compile half): the authoritative `ClientError` surface
//! has no item standing in for a documented response status.
//!
//! A documented response status is an enum variant of the generated response
//! enum, never a `ClientError` (main spec §36); reaching for an invented
//! out-of-spec variant on the support crate's `client_error` module must fail
//! to resolve.

fn main() {
    let _error = openapi_support::client_error::VariantNotInSpec;
}
