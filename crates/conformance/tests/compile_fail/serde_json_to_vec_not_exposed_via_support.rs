//! §49 negative proof (compile half): the support crate exposes ONLY bounded
//! serialization.
//!
//! `serde_json::to_vec(&value)` is forbidden on documented finite-body paths
//! (main spec §49, bounded-encode half); the §34 counterpart
//! `openapi_support::encode::serialize_json_limited` is the whole encode API.
//! An unbounded `to_vec_unbounded` escape hatch must not exist to call.

fn main() {
    let _payload = openapi_support::encode::to_vec_unbounded(&"value");
}
