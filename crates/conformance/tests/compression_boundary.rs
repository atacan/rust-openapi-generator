//! §30.2 transparent response decompression, end to end (main spec §50 test
//! 32): the fixture-01 client regenerated with `response_decompression.gzip`
//! pre-wires `.gzip(true)` on its transport, so a hostile server answering a
//! gzipped 201 body is decoded BENEATH the generated bounded collectors.
//! Proofs:
//!
//! 1. transparent decode equality — the gzipped wire body surfaces as the
//!    decoded `Widget` model;
//! 2. decoded-byte accounting — with `structured_response_bytes` lowered
//!    between the WIRE size and the DECODED size, the call fails with
//!    `ClientError::BodyTooLarge { direction: Decode }`, proving the limit
//!    counts decompressed bytes (the support-level twin triangulates the
//!    same property against raw wire collection).

mod common;

use openapi_conformance::fixtures::fixture_01_json_roundtrip_gzip as fxg;
use openapi_support::client_error::{BodyLimitDirection, ClientError};
use openapi_support::limits::BodyLimits;

/// `gzip --best` of the 294-byte JSON
/// `{"id":"w-1","name":"widget","blob":"aaa…a"}` — precomputed constant so
/// no compression crate is needed to build the hostile response. Wire size
/// 56; strictly between them sits the lowered limit below.
const GZIP_JSON_WIRE: [u8; 56] = [
    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 171, 86, 202, 76, 81, 178, 82, 42, 215, 53, 84, 210, 81,
    202, 75, 204, 77, 5, 113, 50, 83, 210, 83, 75, 128, 252, 164, 156, 252, 36, 32, 63, 113, 132,
    3, 165, 90, 0, 251, 232, 12, 232, 38, 1, 0, 0,
];
/// Strictly between the wire size (56) and the decoded size (294).
const LOWERED_LIMIT: usize = 128;

/// Hostile server: answers every POST /widgets with the gzipped constant,
/// coded exactly like a real gzip-speaking origin would.
fn gzip_app() -> axum::Router {
    axum::Router::new().route(
        "/widgets",
        axum::routing::post(|| async {
            (
                ::http::StatusCode::CREATED,
                [
                    (::http::header::CONTENT_TYPE, "application/json"),
                    (::http::header::CONTENT_ENCODING, "gzip"),
                ],
                &GZIP_JSON_WIRE[..],
            )
        }),
    )
}

fn sent() -> fxg::models::CreateWidget {
    fxg::models::CreateWidget {
        name: "w-1".to_owned(),
        description: openapi_support::optional::OptionalField::Absent,
    }
}

#[tokio::test]
async fn generated_client_decodes_the_gzipped_body_transparently() {
    let address = common::spawn_router(gzip_app());
    let client = fxg::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let response = client.create_widget(&sent()).await.expect("201 decodes");
    match response {
        fxg::client::CreateWidgetResponse::Created201(widget) => {
            assert_eq!(widget.id, "w-1");
            assert_eq!(widget.name, "widget");
            assert_eq!(
                widget.description,
                openapi_support::optional::OptionalField::Absent
            );
        }
        other => panic!("expected Created201, got {other:?}"),
    }
}

#[tokio::test]
async fn decoded_size_over_the_lowered_limit_is_body_too_large_decode() {
    assert!(
        GZIP_JSON_WIRE.len() < LOWERED_LIMIT,
        "fixture invariant broken: wire must fit the lowered limit"
    );
    let address = common::spawn_router(gzip_app());
    let client = fxg::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .limits(BodyLimits {
            structured_response_bytes: LOWERED_LIMIT,
            ..BodyLimits::process_default()
        })
        .build()
        .expect("client builds");

    let error = client
        .create_widget(&sent())
        .await
        .expect_err("294 decoded bytes exceed the 128-byte limit despite the 56-byte wire");
    match error {
        ClientError::BodyTooLarge {
            direction: BodyLimitDirection::Decode,
            limit,
        } => assert_eq!(limit, LOWERED_LIMIT),
        other => panic!("expected BodyTooLarge{{Decode}}, got {other:?}"),
    }
}
