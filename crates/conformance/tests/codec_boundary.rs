//! Runtime conformance for the §45 optional codec families
//! (DECISIONS.md D-impl-codec-plugins, D-impl-override-precedence):
//!
//! - typed round trips BOTH directions for XML, CBOR, and MessagePack under
//!   single-codec configurations;
//! - the 413 size gate fires BEFORE codec parsing (oversized + malformed
//!   body → 413, never 400);
//! - §50 test 52, first half (§39 Codec exception): a malformed (small)
//!   codec body maps onto MalformedBody 400 through the documented enum
//!   (server-side deviation documented in `codegen::codecs`);
//! - §50 test 52, second half: constraint violations on codec paths still
//!   produce SchemaViolation 422 through companion §9 post-decode
//!   validation — the handler never observes a violating body;
//! - a garbage response surfaces client-side as `ClientError::Decode`;
//! - ForceStreaming overrides beat enabled codecs and pass bytes through
//!   unbounded (1 MiB proof with chunk-count evidence).

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;

use openapi_conformance::fixtures::fixture_16_codecs_cbor as fxCbor;
use openapi_conformance::fixtures::fixture_16_codecs_force_stream as fxStream;
use openapi_conformance::fixtures::fixture_16_codecs_msgpack as fxMsgPack;
use openapi_conformance::fixtures::fixture_16_codecs_xml as fxXml;
use openapi_support::hooks::{NoOpEncodeOverflowHook, NoOpStreamFailureHook};
use openapi_support::limits::BodyLimits;

fn xml_doc(title: &str) -> fxXml::models::XmlDocument {
    fxXml::models::XmlDocument {
        title: title.to_owned(),
        revision: 7,
    }
}

// ----------------------------------------------------------------------
// XML round trip — both directions through the generated pair
// ----------------------------------------------------------------------

struct XmlEchoApp;

#[async_trait]
impl fxXml::server::Api for XmlEchoApp {
    async fn create_xml_document(
        &self,
        body: fxXml::models::XmlDocument,
    ) -> fxXml::server::CreateXmlDocumentResponse {
        // Decoded server-side from application/xml; re-encoded into the 201.
        fxXml::server::CreateXmlDocumentResponse::Created201(body)
    }
    async fn put_cbor_state(
        &self,
        _body: ::axum::body::Body,
    ) -> fxXml::server::PutCborStateResponse {
        unreachable!("cbor stays raw under the xml-only config")
    }
    async fn post_msg_pack_event(
        &self,
        _body: ::axum::body::Body,
    ) -> fxXml::server::PostMsgPackEventResponse {
        unreachable!("msgpack stays raw under the xml-only config")
    }
    async fn echo_json(&self, _body: fxXml::models::JsonPing) -> fxXml::server::EchoJsonResponse {
        unreachable!("not exercised here")
    }
}

#[tokio::test]
async fn xml_codec_round_trips_both_directions() {
    let api = Arc::new(XmlEchoApp);
    let address = common::spawn_router(fxXml::server::router(
        api,
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    ));
    let client = fxXml::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let sent = xml_doc("spec §45");
    match client.create_xml_document(&sent).await.expect("201") {
        fxXml::client::CreateXmlDocumentResponse::Created201(doc) => {
            assert_eq!(doc, sent);
            assert_eq!(doc.revision, 7);
        }
        other => panic!("expected Created201, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// CBOR round trip
// ----------------------------------------------------------------------

struct CborEchoApp {
    handler_ran: AtomicBool,
}

#[async_trait]
impl fxCbor::server::Api for CborEchoApp {
    async fn create_xml_document(
        &self,
        _body: ::axum::body::Body,
    ) -> fxCbor::server::CreateXmlDocumentResponse {
        unreachable!("xml stays raw under the cbor-only config")
    }
    async fn put_cbor_state(
        &self,
        body: fxCbor::models::CborState,
    ) -> fxCbor::server::PutCborStateResponse {
        self.handler_ran.store(true, Ordering::SeqCst);
        fxCbor::server::PutCborStateResponse::Ok200(body)
    }
    async fn post_msg_pack_event(
        &self,
        _body: ::axum::body::Body,
    ) -> fxCbor::server::PostMsgPackEventResponse {
        unreachable!("msgpack stays raw under the cbor-only config")
    }
    async fn echo_json(&self, _body: fxCbor::models::JsonPing) -> fxCbor::server::EchoJsonResponse {
        unreachable!("not exercised here")
    }
}

#[tokio::test]
async fn cbor_codec_round_trips_both_directions() {
    let app = Arc::new(CborEchoApp {
        handler_ran: AtomicBool::new(false),
    });
    let address = common::spawn_router(fxCbor::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    ));
    let client = fxCbor::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let sent = fxCbor::models::CborState {
        slot: "alpha".to_owned(),
        level: 1_048_576,
    };
    match client.put_cbor_state(&sent).await.expect("200") {
        fxCbor::client::PutCborStateResponse::Ok200(state) => assert_eq!(state, sent),
        other => panic!("expected Ok200, got {other:?}"),
    }
    assert!(app.handler_ran.load(Ordering::SeqCst), "handler ran");
}

// ----------------------------------------------------------------------
// MessagePack round trip + malformed-body mapping
// ----------------------------------------------------------------------

struct MsgPackApp {
    handler_ran: AtomicBool,
}

#[async_trait]
impl fxMsgPack::server::Api for MsgPackApp {
    async fn create_xml_document(
        &self,
        _body: ::axum::body::Body,
    ) -> fxMsgPack::server::CreateXmlDocumentResponse {
        unreachable!("not exercised here")
    }
    async fn put_cbor_state(
        &self,
        _body: ::axum::body::Body,
    ) -> fxMsgPack::server::PutCborStateResponse {
        unreachable!("not exercised here")
    }
    async fn post_msg_pack_event(
        &self,
        body: fxMsgPack::models::MsgPackEvent,
    ) -> fxMsgPack::server::PostMsgPackEventResponse {
        self.handler_ran.store(true, Ordering::SeqCst);
        fxMsgPack::server::PostMsgPackEventResponse::Ok200(body)
    }
    async fn echo_json(
        &self,
        _body: fxMsgPack::models::JsonPing,
    ) -> fxMsgPack::server::EchoJsonResponse {
        unreachable!("not exercised here")
    }
}

#[tokio::test]
async fn msgpack_codec_round_trips_both_directions() {
    let app = Arc::new(MsgPackApp {
        handler_ran: AtomicBool::new(false),
    });
    let address = common::spawn_router(fxMsgPack::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    ));
    let client = fxMsgPack::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let sent = fxMsgPack::models::MsgPackEvent {
        kind: "tick".to_owned(),
        seq: -3,
    };
    match client.post_msg_pack_event(&sent).await.expect("200") {
        fxMsgPack::client::PostMsgPackEventResponse::Ok200(event) => assert_eq!(event, sent),
        other => panic!("expected Ok200, got {other:?}"),
    }
    assert!(app.handler_ran.load(Ordering::SeqCst), "handler ran");
}

/// §50 test 52, first half (§39 Codec exception): a small MALFORMED
/// MessagePack body is well-framed HTTP but fails codec parsing: the
/// generated router maps it onto MalformedBody 400, which the documenter 400
/// (problem+json) absorbs into `BadRequest400` — the documented deviation
/// recorded in `codegen::codecs` (the schema/data distinction is not
/// portable across codecs).
#[tokio::test]
async fn t52_codec_data_errors_map_to_400_malformed_body() {
    use tower::ServiceExt;

    let app = Arc::new(MsgPackApp {
        handler_ran: AtomicBool::new(false),
    });
    let router = fxMsgPack::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    );

    // Valid msgpack framing of a MAP but wrong shape for `MsgPackEvent`
    // (fixmap of one entry) — parse succeeds at the wire layer, decode fails
    // against the model.
    let request = ::http::Request::builder()
        .method(::http::Method::POST)
        .uri("/msgpack/events")
        .header(::http::header::CONTENT_TYPE, "application/msgpack")
        .body(axum::body::Body::from(vec![
            0x81, 0xa5, b'h', b'e', b'l', b'l', b'o',
        ]))
        .expect("request");

    let response = router.oneshot(request).await.expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::BAD_REQUEST);
    assert!(
        !app.handler_ran.load(Ordering::SeqCst),
        "a malformed body must never reach the handler"
    );
}

/// §50 test 52, second half: a WELL-FORMED codec payload whose constrained
/// field violates its companion §9 constraint still rejects through
/// post-decode validation with SchemaViolation 422 — never through the §39
/// Codec exception's MalformedBody 400 — and the handler stays uninvoked.
/// The generated client encodes the payload itself, so the wire bytes are
/// guaranteed-valid codec data; only bucket-2 validation can reject. Proven
/// for MessagePack first, then CBOR for symmetry.
#[tokio::test]
async fn t52_constraint_violations_still_422_through_post_decode_validation_on_codec_paths() {
    // MessagePack: decode succeeds, then minLength("kind") fails.
    let app = Arc::new(MsgPackApp {
        handler_ran: AtomicBool::new(false),
    });
    let address = common::spawn_router(fxMsgPack::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    ));
    let client = fxMsgPack::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");
    let violation = fxMsgPack::models::MsgPackEvent {
        kind: "x".to_owned(),
        seq: -3,
    };
    match client.post_msg_pack_event(&violation).await {
        Err(openapi_support::client_error::ClientError::UndocumentedStatus { status }) => {
            // 422 is undocumented on this operation, so the peer-generated
            // rejection surfaces as UndocumentedStatus carrying the status.
            assert_eq!(status, ::http::StatusCode::UNPROCESSABLE_ENTITY);
        }
        other => panic!("expected UndocumentedStatus 422, got {other:?}"),
    }
    assert!(
        !app.handler_ran.load(Ordering::SeqCst),
        "a constraint-violating body must never reach the handler"
    );

    // CBOR symmetry: decode succeeds, then minLength("slot") fails.
    let app = Arc::new(CborEchoApp {
        handler_ran: AtomicBool::new(false),
    });
    let address = common::spawn_router(fxCbor::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    ));
    let client = fxCbor::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");
    let violation = fxCbor::models::CborState {
        slot: "x".to_owned(),
        level: 1_048_576,
    };
    match client.put_cbor_state(&violation).await {
        Err(openapi_support::client_error::ClientError::UndocumentedStatus { status }) => {
            assert_eq!(status, ::http::StatusCode::UNPROCESSABLE_ENTITY);
        }
        other => panic!("expected UndocumentedStatus 422, got {other:?}"),
    }
    assert!(
        !app.handler_ran.load(Ordering::SeqCst),
        "a constraint-violating body must never reach the handler"
    );
}

/// The 413 size gate fires BEFORE codec parsing: an OVER LIMIT body that is
/// also malformed msgpack yields 413 (size gate first), never 400 (parse).
#[tokio::test]
async fn oversized_msgpack_body_is_a_413_before_any_parsing() {
    use tower::ServiceExt;

    let app = Arc::new(MsgPackApp {
        handler_ran: AtomicBool::new(false),
    });
    let tiny = BodyLimits {
        structured_request_bytes: 64,
        ..BodyLimits::process_default()
    };
    let router = fxMsgPack::server::router(
        app.clone(),
        tiny,
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    );

    // 4 KiB of INVALID msgpack: if any parsing happened first this would be
    // a 400; the bounded collection gate wins with 413.
    let request = ::http::Request::builder()
        .method(::http::Method::POST)
        .uri("/msgpack/events")
        .header(::http::header::CONTENT_TYPE, "application/msgpack")
        .body(axum::body::Body::from(vec![0xc1_u8; 4096]))
        .expect("request");

    let response = router.oneshot(request).await.expect("in-memory service");
    assert_eq!(response.status(), ::http::StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        !app.handler_ran.load(Ordering::SeqCst),
        "handler must not observe rejected bodies"
    );
}

// ----------------------------------------------------------------------
// Client-side Decode error on a garbage response
// ----------------------------------------------------------------------

/// A hand-rolled service returning a 200 with garbage CBOR bytes proves the
/// CLIENT half of the contract: bounded collect succeeds, codec parse fails,
/// and the failure surfaces as `ClientError::Decode` with the negotiated
/// content type.
#[tokio::test]
async fn garbage_cbor_response_surfaces_as_client_decode_error() {
    let garbage = || {
        axum::Router::new().route(
            "/cbor/state",
            axum::routing::put(|| async {
                (
                    [(
                        ::http::header::CONTENT_TYPE,
                        ::http::HeaderValue::from_static("application/cbor"),
                    )],
                    Bytes::from_static(b"\xff\xff\xff not cbor"),
                )
            }),
        )
    };
    let address = common::spawn_router(garbage());

    let client = fxCbor::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    let state = fxCbor::models::CborState {
        slot: "s".to_owned(),
        level: 1,
    };
    let error = client
        .put_cbor_state(&state)
        .await
        .expect_err("garbage cannot decode");
    match error {
        openapi_support::client_error::ClientError::Decode { content_type, .. } => {
            let mime = content_type.expect("content type carried");
            assert_eq!(mime.essence_str(), "application/cbor");
        }
        other => panic!("expected ClientError::Decode, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// ForceStreaming override beats codecs (1 MiB passthrough proof)
// ----------------------------------------------------------------------

const MIB: usize = 1024 * 1024;

struct ForcedStreamApp {
    received_chunks: AtomicUsize,
    received: std::sync::Mutex<Vec<u8>>,
}

#[async_trait]
impl fxStream::server::Api for ForcedStreamApp {
    /// The override forced this XML operation RAW even though the xml codec
    /// is ENABLED in this configuration (D-impl-override-precedence): the
    /// handler receives the untouched streaming body.
    async fn create_xml_document(
        &self,
        body: ::axum::body::Body,
    ) -> fxStream::server::CreateXmlDocumentResponse {
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            self.received_chunks.fetch_add(1, Ordering::SeqCst);
            let chunk: Bytes = chunk.expect("request chunk");
            self.received
                .lock()
                .expect("received lock")
                .extend_from_slice(&chunk);
        }
        fxStream::server::CreateXmlDocumentResponse::Created201(
            fxStream::server::CreateXmlDocument201 {
                body: ::axum::body::Body::from(
                    self.received.lock().expect("received lock").clone(),
                ),
            },
        )
    }
    async fn put_cbor_state(
        &self,
        _body: fxStream::models::CborState,
    ) -> fxStream::server::PutCborStateResponse {
        unreachable!("not exercised here")
    }
    async fn post_msg_pack_event(
        &self,
        _body: fxStream::models::MsgPackEvent,
    ) -> fxStream::server::PostMsgPackEventResponse {
        unreachable!("not exercised here")
    }
    /// The JSON operation is ALSO ForceStreaming-overridden in this config.
    async fn echo_json(&self, _body: ::axum::body::Body) -> fxStream::server::EchoJsonResponse {
        unreachable!("not exercised here")
    }
}

#[tokio::test]
async fn force_streaming_override_passes_one_mib_through_unbounded() {
    let app = Arc::new(ForcedStreamApp {
        received_chunks: AtomicUsize::new(0),
        received: std::sync::Mutex::new(Vec::new()),
    });
    let address = common::spawn_router(fxStream::server::router(
        app.clone(),
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    ));

    let payload = vec![0xAB_u8; MIB];
    // The overridden operation takes a raw reqwest::Body even though every
    // codec is enabled in this configuration.
    let client = fxStream::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");
    let body = ::reqwest::Body::from(payload.clone());
    match client.create_xml_document(body).await.expect("201") {
        fxStream::client::CreateXmlDocumentResponse::Created201(wrapper) => {
            // Response side streams verbatim too: drain the wrapper's chunk
            // stream and prove the byte count without ever aggregating in
            // generated code.
            let mut stream = wrapper.response.bytes_stream();
            let mut total = 0_usize;
            while let Some(chunk) = stream.next().await {
                total += chunk.expect("response chunk").len();
            }
            assert_eq!(total, MIB, "1 MiB must survive both directions");
        }
        other => panic!("expected Created201, got {other:?}"),
    }

    let chunks = app.received_chunks.load(Ordering::SeqCst);
    assert!(
        chunks > 1,
        "passthrough must stream, not aggregate ({chunks})"
    );
    assert_eq!(*app.received.lock().expect("lock"), payload);
}
