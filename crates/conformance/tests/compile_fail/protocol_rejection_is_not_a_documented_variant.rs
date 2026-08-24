//! §49/§39 negative proof (compile half): a `ProtocolRejection` is never a
//! documented response variant.
//!
//! Protocol failures (§28 rejections) are returned OUTSIDE the generated
//! documented-status enum (main spec §39 rule 1): `BadRequest400` carries the
//! schema model `ProblemDetails`, so handing it a `ProtocolRejection` must
//! fail to typecheck.

use openapi_conformance::fixtures::fixture_01_json_roundtrip::client::CreateWidgetResponse;
use openapi_support::rejection::{ProtocolRejection, RejectionKind};

fn main() {
    let _response = CreateWidgetResponse::BadRequest400(ProtocolRejection::new(
        RejectionKind::MalformedBody,
    ));
}
