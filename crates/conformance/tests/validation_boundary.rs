//! Bucket-2 runtime-validation conformance (companion §9, D-§2 bucket 2,
//! DECISIONS.md D-impl-runtime-validation-timing Phase 2 half), driven
//! against the GENERATED fixture-13 router and client:
//!
//! - a violating JSON body rejects 422 SchemaViolation with the handler NOT
//!   invoked (§39 rule 1);
//! - boundary values (exclusive bounds, minItems, minProperties) pass and
//!   reach the handler;
//! - declared formats accept/reject exactly at the validator level;
//! - the CLIENT stays lenient (it encodes a violating body happily) while
//!   the server rejects it — companion §9 default asymmetry;
//! - plain-text bodies of constrained scalar aliases validate post-decode;
//! - multipart textual parts backed by a constrained alias reject 422
//!   inside the collector, before any application code runs.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use openapi_conformance::fixtures::fixture_13_validation as fx13;
use openapi_support::client_error::ClientError;
use openapi_support::optional::OptionalField;
use tower::ServiceExt;

const BOUNDARY: &str = "XvAlIdBoUnDaRyX";

fn router13(api: Arc<dyn fx13::server::Api>) -> axum::Router {
    let (limits, hook) = common::router_args();
    fx13::server::router(api, limits, hook)
}

fn json_request(uri: &str, body: serde_json::Value) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .expect("request")
}

fn text_request(uri: &str, body: &str) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method(http::Method::POST)
        .uri(uri)
        .header(http::header::CONTENT_TYPE, "text/plain")
        .body(axum::body::Body::from(body.to_owned()))
        .expect("request")
}

fn frame_part(name: &str, payload: &str) -> Vec<u8> {
    let mut out =
        format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
            .into_bytes();
    out.extend_from_slice(payload.as_bytes());
    out.extend_from_slice(b"\r\n");
    out
}

fn multipart_request(parts: &[Vec<u8>]) -> http::Request<axum::body::Body> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(part);
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    http::Request::builder()
        .method(http::Method::POST)
        .uri("/uploads")
        .header(
            http::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(axum::body::Body::from(body))
        .expect("request")
}

/// Handler that records whether the application was ever observed.
#[derive(Default)]
struct SpyApp {
    invoked: AtomicBool,
}

fn spy() -> Arc<SpyApp> {
    Arc::new(SpyApp::default())
}

#[async_trait]
impl fx13::server::Api for SpyApp {
    async fn create_ticket(
        &self,
        _body: fx13::models::Ticket,
    ) -> fx13::server::CreateTicketResponse {
        self.invoked.store(true, Ordering::SeqCst);
        fx13::server::CreateTicketResponse::Created201(_body)
    }

    async fn register_slug(&self, _body: String) -> fx13::server::RegisterSlugResponse {
        self.invoked.store(true, Ordering::SeqCst);
        fx13::server::RegisterSlugResponse::Created201(fx13::models::RegisteredSlug {
            slug: OptionalField::Present(_body),
        })
    }

    async fn post_note(&self, _body: String) -> fx13::server::PostNoteResponse {
        self.invoked.store(true, Ordering::SeqCst);
        fx13::server::PostNoteResponse::Created201(fx13::models::NoteReceipt { stored: true })
    }

    async fn send_feedback(
        &self,
        _body: fx13::models::FeedbackForm,
    ) -> fx13::server::SendFeedbackResponse {
        self.invoked.store(true, Ordering::SeqCst);
        fx13::server::SendFeedbackResponse::NoContent204
    }

    async fn upload_attachment(
        &self,
        _body: fx13::server::UploadAttachmentMultipartInput,
    ) -> fx13::server::UploadAttachmentResponse {
        self.invoked.store(true, Ordering::SeqCst);
        fx13::server::UploadAttachmentResponse::Created201(serde_json::Map::new())
    }
}

/// A minimal ticket at every BOUNDARY of fixture 13's constraints.
fn boundary_ticket() -> serde_json::Value {
    serde_json::json!({
        // pattern + minLength >= 8 both hold exactly
        "code": "ABC-1234",
        // valid date-time
        "when": "2026-08-24T12:00:00Z",
        // exclusiveMinimum > 0 boundary
        "seats": 1,
        // minimum >= 0 inclusive boundary; multipleOf 0.5 exact
        "price": 0.0,
        // minItems == 2 boundary, unique
        "tags": ["a", "b"],
        // minProperties == 1 boundary over the schema-valued map
        "meta": {"origin": "web"},
        // nested chain with its own exclusive bound
        "nested": {"label": "root", "next": {"weight": 1}}
    })
}

// ----------------------------------------------------------------------
// Violation → 422, handler not invoked
// ----------------------------------------------------------------------

#[tokio::test]
async fn violating_json_body_rejects_422_and_never_invokes_the_handler() {
    let app = spy();
    let response = router13(app.clone())
        .oneshot(json_request(
            "/tickets",
            serde_json::json!({
                "code": "nope",
                "when": "2026-08-24T12:00:00Z",
                "seats": 1,
                "price": 0.0,
                "tags": ["a", "b"]
            }),
        ))
        .await
        .expect("in-memory service");
    assert_eq!(
        response.status(),
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "pattern violation is a SchemaViolation 422"
    );
    assert!(
        !app.invoked.load(Ordering::SeqCst),
        "pre-handler validation must keep the application unobserved"
    );
}

#[tokio::test]
async fn boundary_values_pass_and_reach_the_handler() {
    let app = spy();
    let response = router13(app.clone())
        .oneshot(json_request("/tickets", boundary_ticket()))
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), http::StatusCode::CREATED);
    assert!(app.invoked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn exclusive_and_cardinality_boundaries_reject_outside_values() {
    let app = spy();
    // seats == 0 violates exclusiveMinimum > 0.
    let mut body = boundary_ticket();
    body["seats"] = serde_json::json!(0);
    let response = router13(app.clone())
        .oneshot(json_request("/tickets", body))
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

    // tags with ONE item violates minItems >= 2.
    let mut body = boundary_ticket();
    body["tags"] = serde_json::json!(["a"]);
    let response = router13(app.clone())
        .oneshot(json_request("/tickets", body))
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

    // duplicate tags violate uniqueItems (string elements only, v1).
    let mut body = boundary_ticket();
    body["tags"] = serde_json::json!(["a", "a"]);
    let response = router13(app.clone())
        .oneshot(json_request("/tickets", body))
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

    // empty meta object violates minProperties >= 1.
    let mut body = boundary_ticket();
    body["meta"] = serde_json::json!({});
    let response = router13(app.clone())
        .oneshot(json_request("/tickets", body))
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), http::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        !app.invoked.load(Ordering::SeqCst),
        "all four must pre-reject"
    );
}

// ----------------------------------------------------------------------
// Formats accept/reject
// ----------------------------------------------------------------------

#[tokio::test]
async fn format_violations_reject_before_the_handler() {
    let app = spy();
    for field in ["when", "contact"] {
        let mut body = boundary_ticket();
        body[field] = serde_json::json!("not-a-date-or-email");
        let response = router13(app.clone())
            .oneshot(json_request("/tickets", body))
            .await
            .expect("in-memory service");
        assert_eq!(
            response.status(),
            http::StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: invalid format must 422"
        );
    }

    // A VALID optional email passes the format check end to end.
    let mut body = boundary_ticket();
    body["contact"] = serde_json::json!("user@example.com");
    let response = router13(app.clone())
        .oneshot(json_request("/tickets", body))
        .await
        .expect("in-memory service");
    assert_eq!(response.status(), http::StatusCode::CREATED);
}

// ----------------------------------------------------------------------
// Constrained scalar alias bodies: JSON + plain text
// ----------------------------------------------------------------------

#[tokio::test]
async fn scalar_alias_json_body_validates_post_decode() {
    let app = spy();
    let good = router13(app.clone())
        .oneshot(json_request("/slugs", serde_json::json!("abc-12")))
        .await
        .expect("in-memory service");
    assert_eq!(good.status(), http::StatusCode::CREATED);

    let bad = router13(app.clone())
        .oneshot(json_request("/slugs", serde_json::json!("Abc")))
        .await
        .expect("in-memory service");
    assert_eq!(bad.status(), http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn plain_text_alias_body_validates_post_decode() {
    let app = spy();
    let good = router13(app.clone())
        .oneshot(text_request("/notes", "release-note"))
        .await
        .expect("in-memory service");
    assert_eq!(good.status(), http::StatusCode::CREATED);

    let bad = router13(app.clone())
        .oneshot(text_request("/notes", "-leading-hyphen"))
        .await
        .expect("in-memory service");
    assert_eq!(bad.status(), http::StatusCode::UNPROCESSABLE_ENTITY);
}

// ----------------------------------------------------------------------
// Form bodies validate after the bounded form decode
// ----------------------------------------------------------------------

#[tokio::test]
async fn form_body_violations_reject_422() {
    let app = spy();
    let bad = router13(app.clone())
        .oneshot(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/feedback")
                .header(
                    http::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(axum::body::Body::from("rating=0&comment=ok!"))
                .expect("request"),
        )
        .await
        .expect("in-memory service");
    assert_eq!(bad.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

    let good = router13(app.clone())
        .oneshot(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/feedback")
                .header(
                    http::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(axum::body::Body::from("rating=5&comment=great"))
                .expect("request"),
        )
        .await
        .expect("in-memory service");
    assert_eq!(good.status(), http::StatusCode::NO_CONTENT);
    assert!(app.invoked.load(Ordering::SeqCst));
}

// ----------------------------------------------------------------------
// Multipart scalar part backed by a constrained alias
// ----------------------------------------------------------------------

#[tokio::test]
async fn multipart_scalar_part_violation_rejects_422_pre_handler() {
    let app = spy();
    let bad_kind = router13(app.clone())
        .oneshot(multipart_request(&[
            frame_part("title", "Release photo"),
            frame_part("kind", "Bad Kind"),
        ]))
        .await
        .expect("in-memory service");
    assert_eq!(
        bad_kind.status(),
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "the `kind` part carries the Slug alias; violations reject in the collector"
    );
    assert!(!app.invoked.load(Ordering::SeqCst));

    let good = router13(app.clone())
        .oneshot(multipart_request(&[
            frame_part("title", "Release photo"),
            frame_part("kind", "photo-jpg"),
        ]))
        .await
        .expect("in-memory service");
    assert_eq!(good.status(), http::StatusCode::CREATED);
    assert!(app.invoked.load(Ordering::SeqCst));
}

// ----------------------------------------------------------------------
// Lenient client vs rejecting server (companion §9 default)
// ----------------------------------------------------------------------

#[tokio::test]
async fn lenient_client_encodes_while_server_rejects_with_422() {
    struct RejectingApp;

    #[async_trait]
    impl fx13::server::Api for RejectingApp {
        async fn create_ticket(
            &self,
            _body: fx13::models::Ticket,
        ) -> fx13::server::CreateTicketResponse {
            panic!("handler must never observe a violating body");
        }
        async fn register_slug(&self, _body: String) -> fx13::server::RegisterSlugResponse {
            panic!("not exercised");
        }
        async fn post_note(&self, _body: String) -> fx13::server::PostNoteResponse {
            panic!("not exercised");
        }
        async fn send_feedback(
            &self,
            _body: fx13::models::FeedbackForm,
        ) -> fx13::server::SendFeedbackResponse {
            panic!("not exercised");
        }
        async fn upload_attachment(
            &self,
            _body: fx13::server::UploadAttachmentMultipartInput,
        ) -> fx13::server::UploadAttachmentResponse {
            panic!("not exercised");
        }
    }

    let address = common::spawn_router(router13(Arc::new(RejectingApp)));
    let client = fx13::client::ClientBuilder::new()
        .base_url(common::base_url(address))
        .build()
        .expect("client builds");

    // The client ENCODES this violating ticket without complaint — client
    // decoding/encoding stays lenient under companion §9 defaults.
    let violating = fx13::models::Ticket {
        code: String::from("nope"),
        when: String::from("2026-08-24T12:00:00Z"),
        contact: OptionalField::Absent,
        seats: 1,
        price: 0.0,
        tags: vec![String::from("a"), String::from("b")],
        meta: OptionalField::Absent,
        nested: OptionalField::Absent,
    };
    let error = client
        .create_ticket(&violating)
        .await
        .expect_err("violating body must 422");
    match error {
        ClientError::UndocumentedStatus { status } => {
            assert_eq!(status, http::StatusCode::UNPROCESSABLE_ENTITY);
        }
        other => panic!("expected UndocumentedStatus(422), got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// Validator unit surface (models.rs side, no HTTP)
// ----------------------------------------------------------------------

#[test]
fn violation_details_carry_field_paths() {
    let ticket = fx13::models::Ticket {
        code: String::from("short"),
        when: String::from("2026-08-24T12:00:00Z"),
        contact: OptionalField::Absent,
        seats: 1,
        price: 0.0,
        tags: vec![String::from("a"), String::from("b")],
        meta: OptionalField::Absent,
        nested: OptionalField::Absent,
    };
    let violation = ticket.validate_request().expect_err("pattern fails");
    assert!(
        violation.to_string().contains("field `code`"),
        "detail names the field: {violation}"
    );

    // Nested chain composes paths Ticket → nested → next → weight.
    let deep = fx13::models::LevelA {
        label: String::from("root"),
        next: fx13::models::LevelB { weight: 0 },
    };
    let violation = deep.validate_request().expect_err("weight fails");
    assert!(
        violation.to_string().contains("field `next`")
            && violation.to_string().contains("exclusiveMinimum"),
        "nested path + constraint detail: {violation}"
    );

    // The free alias validator validates the raw scalar.
    assert!(fx13::models::validate_slug_request("abc-12").is_ok());
    assert!(fx13::models::validate_slug_request("Bad Kind").is_err());

    // Silence unused-import lint when OptionalField variants change shape.
    let _ = BTreeMap::<String, String>::new();
}
