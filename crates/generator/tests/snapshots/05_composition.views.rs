/// Directional view types generated from the OpenAPI document (companion §5):
/// `Write` views carry the client-request-encode / server-request-decode wire
/// shape with `readOnly` properties omitted; `Read` views carry the server-
/// response-encode / client-response-decode wire shape with `writeOnly`
/// properties treated as absent.
///
/// Requiredness is directional (companion §5): required `writeOnly` fields are
/// required only in `Write`, required `readOnly` fields only in `Read`, and
/// directionless required fields stay required in both. Every surviving field
/// applies the companion §2.1 presence/nullability matrix identically to
/// `super::models`.
///
/// Conversions are intentionally asymmetric (companion §5): projections
/// `From<&SharedModel> for *View` always exist; reconstructions
/// `From<&*View> for SharedModel` exist only when every view-omitted field is
/// optional in the shared model, so no value is ever fabricated. Models
/// without `readOnly`/`writeOnly` properties receive no view types.
///
/// This file is generated deterministically byte-for-byte (main spec §50 test
/// 39); do not edit by hand.
use serde::{Deserialize, Serialize};

use super::models::TreeNode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DogKind {
    #[serde(rename = "dog")]
    Dog,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CatKind {
    #[serde(rename = "cat")]
    Cat,
}
