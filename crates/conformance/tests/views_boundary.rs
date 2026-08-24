//! Directional view boundary conformance (companion §5, main spec §50 test
//! 50): the GENERATED fixture-08 server/client pair runs over real TCP while
//! a middleware captures the raw wires, proving that
//!
//! - t50a — the request wire carries a required `writeOnly` field but never
//!   a `readOnly` one even when the caller supplies a shared-model-shaped
//!   value through the lossless projection, and a body missing the required
//!   write-only field rejects 422 before any handler code runs;
//! - t50b — the response wire drops `writeOnly` fields while the
//!   client-decoded read view carries the required `readOnly` ones;
//! - t50c — a mixed readOnly/writeOnly model round trips both directions
//!   (optional cells included), the non-lossless trait contract receives the
//!   write view itself, and the write view's own validator keeps companion §9
//!   guarantees (short labels reject 422 pre-handler).

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use openapi_conformance::fixtures::fixture_08_views as fx08;
use openapi_support::optional::OptionalField;

/// Application half of the Mode A contract, recording everything the
/// generated router hands across the view boundary.
struct ViewsApp {
    /// Accounts observed AFTER the router's lossless reconstruction
    /// (`AccountWrite` → `Account` conversion happened in the router).
    received_accounts: Mutex<Vec<fx08::models::Account>>,
    /// SyncedRecord WRITE views observed verbatim (non-lossless direction:
    /// the trait contract takes the view itself, no fabricated `id`).
    received_synced: Mutex<Vec<fx08::views::SyncedRecordWrite>>,
}

impl ViewsApp {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            received_accounts: Mutex::new(Vec::new()),
            received_synced: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl fx08::server::Api for ViewsApp {
    async fn create_account(
        &self,
        body: fx08::models::Account,
    ) -> fx08::server::CreateAccountResponse {
        self.received_accounts
            .lock()
            .expect("accounts lock")
            .push(body.clone());
        // Server responses are built as READ views (or projected from the
        // shared model); the password cannot ride the response either way.
        fx08::server::CreateAccountResponse::Created201(fx08::views::AccountRead::from(&body))
    }

    async fn list_audit_entries(&self, id: String) -> fx08::server::ListAuditEntriesResponse {
        fx08::server::ListAuditEntriesResponse::Ok200(fx08::views::AuditEntryRead {
            created_at: format!("ts-{id}"),
            metadata: Some(format!("audit {id}")),
        })
    }

    async fn sync_record(
        &self,
        body: fx08::views::SyncedRecordWrite,
    ) -> fx08::server::SyncRecordResponse {
        self.received_synced
            .lock()
            .expect("synced lock")
            .push(body.clone());
        // The application fabricates its own server-side identity; the READ
        // response projects the shared shape minus the writeOnly token.
        let stored = fx08::models::SyncedRecord {
            id: "srv-1".to_owned(),
            label: body.label.clone(),
            secret_token: body.secret_token.clone(),
            reviewed_by: OptionalField::Present("auditor".to_owned()),
        };
        fx08::server::SyncRecordResponse::Ok200(fx08::views::SyncedRecordRead::from(&stored))
    }
}

/// Captured raw HTTP bodies per direction (request lines first, response
/// lines second), so assertions run against BYTES rather than decoded types.
#[derive(Default, Clone)]
struct Wire {
    request_bodies: Arc<Mutex<Vec<String>>>,
    response_bodies: Arc<Mutex<Vec<String>>>,
}

async fn capture_wire(
    wire: Wire,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("request body collects");
    if !bytes.is_empty() {
        wire.request_bodies
            .lock()
            .expect("wire lock")
            .push(String::from_utf8_lossy(&bytes).into_owned());
    }
    let response = next
        .run(axum::extract::Request::from_parts(
            parts,
            axum::body::Body::from(bytes),
        ))
        .await;
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("response body collects");
    wire.response_bodies
        .lock()
        .expect("wire lock")
        .push(String::from_utf8_lossy(&bytes).into_owned());
    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
}

/// Spawns the fixture-08 router behind the capturing layer; returns the
/// base URL plus the wire recorder.
fn spawn(app: Arc<ViewsApp>) -> (String, Wire) {
    let wire = Wire::default();
    let (limits, hook, stream_hook) = common::router_args();
    let recorder = wire.clone();
    let router =
        fx08::server::router(app, limits, hook, stream_hook).layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let wire = recorder.clone();
                async move { capture_wire(wire, request, next).await }
            },
        ));
    (common::base_url(common::spawn_router(router)), wire)
}

fn client(base_url: &str) -> fx08::client::Client {
    fx08::client::ClientBuilder::new()
        .base_url(base_url)
        .build()
        .expect("client builds")
}

// ----------------------------------------------------------------------
// t50a — request direction: required writeOnly mandatory, readOnly absent
// ----------------------------------------------------------------------

#[tokio::test]
async fn t50a_request_wire_carries_password_only_and_missing_password_is_422_pre_handler() {
    let app = ViewsApp::new();
    let (base_url, wire) = spawn(app.clone());
    let client = client(&base_url);

    // The caller supplies a SHARED-MODEL-shaped value (every field present,
    // as an application would hold internally) and sends it through the
    // lossless projection `From<&Account> for AccountWrite`.
    let shared = fx08::models::Account {
        id: "a-1".to_owned(),
        password: "hunter2".to_owned(),
        note: OptionalField::Present("primary".to_owned()),
    };
    let response = client
        .create_account(&fx08::views::AccountWrite::from(&shared))
        .await
        .expect("201 documented");
    // Exhaustive by construction: one documented status, one variant.
    let fx08::client::CreateAccountResponse::Created201(read) = response;
    // The response side is the READ view: same id/note, and the password
    // structurally cannot exist on the type.
    assert_eq!(read.id, "a-1");
    assert_eq!(read.note, OptionalField::Present("primary".to_owned()));

    // Raw REQUEST wire: password present, createdAt absent.
    let sent = wire.request_bodies.lock().expect("wire lock")[0].clone();
    assert!(
        sent.contains("\"password\""),
        "required writeOnly must ride the request wire: {sent}"
    );
    assert!(
        !sent.contains("createdAt") && !sent.contains("created"),
        "readOnly fields must be omitted from the request wire: {sent}"
    );

    // Lossless conversion path: the ROUTER reconstructed the shared model,
    // so the trait observed exactly the caller's original value.
    {
        let received = app.received_accounts.lock().expect("accounts lock");
        assert_eq!(received.len(), 1);
        assert_eq!(received[0], shared);
    }

    // A body MISSING the required writeOnly field rejects before the
    // handler: Serde decode fails on `AccountWrite.password` and maps onto
    // SchemaViolation 422 (D-impl-runtime-validation-timing).
    let raw = reqwest::Client::new()
        .post(format!("{base_url}/accounts"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .body("{\"id\":\"a-2\"}")
        .send()
        .await
        .expect("raw request sends");
    assert_eq!(
        raw.status(),
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "missing writeOnly password must reject 422"
    );
    assert_eq!(
        app.received_accounts.lock().expect("accounts lock").len(),
        1,
        "the handler must not observe rejected bodies (only the original call remains)"
    );
}

// ----------------------------------------------------------------------
// t50b — response direction: writeOnly absent from wire, readOnly decoded
// ----------------------------------------------------------------------

#[tokio::test]
async fn t50b_response_wire_lacks_write_only_and_client_decodes_created_at() {
    let app = ViewsApp::new();
    let (base_url, wire) = spawn(app);
    let client = client(&base_url);

    let response = client
        .list_audit_entries("42")
        .await
        .expect("200 documented");
    let fx08::client::ListAuditEntriesResponse::Ok200(read) = response;
    // Required readOnly survives the whole pipeline into the typed
    // client-side read view.
    assert_eq!(read.created_at, "ts-42");
    assert_eq!(read.metadata, Some("audit 42".to_owned()));

    // Raw RESPONSE wire: the optional writeOnly draftNote never leaves the
    // server, and no secretToken key exists anywhere on this API surface.
    let sent = wire.response_bodies.lock().expect("wire lock")[0].clone();
    assert!(
        sent.contains("\"createdAt\""),
        "required readOnly must ride the response wire: {sent}"
    );
    assert!(
        !sent.contains("draftNote"),
        "writeOnly fields must be treated as absent on the response wire: {sent}"
    );
    assert!(
        !wire
            .request_bodies
            .lock()
            .expect("wire lock")
            .iter()
            .chain(wire.response_bodies.lock().unwrap().iter())
            .any(|body| body.contains("secretToken")),
        "the client never observes the writeOnly secretToken key"
    );
}

// ----------------------------------------------------------------------
// t50c — mixed directions, optional cells, validator continuity
// ----------------------------------------------------------------------

#[tokio::test]
async fn t50c_synced_record_round_trips_both_directions_and_validates_kept_constraints() {
    let app = ViewsApp::new();
    let (base_url, wire) = spawn(app.clone());
    let client = client(&base_url);

    // Request direction: mixed ro/wo write view with PRESENT optional cells.
    let sent = fx08::views::SyncedRecordWrite {
        label: "release".to_owned(),
        secret_token: OptionalField::Present("s3cret".to_owned()),
    };
    let response = client.sync_record(&sent).await.expect("200 documented");
    let fx08::client::SyncRecordResponse::Ok200(read) = response;
    // Read direction: server-fabricated readOnly id, projected reviewer, and
    // NO secretToken on the type at all.
    assert_eq!(read.id, "srv-1");
    assert_eq!(read.label, "release");
    assert_eq!(
        read.reviewed_by,
        OptionalField::Present("auditor".to_owned())
    );
    {
        let received = app.received_synced.lock().expect("synced lock");
        assert_eq!(received.len(), 1);
        assert_eq!(received[0], sent);
    }

    // Raw wires both ways: secretToken rides the request only.
    let request_wire = wire.request_bodies.lock().expect("wire lock")[0].clone();
    assert!(request_wire.contains("\"secretToken\""), "{request_wire}");
    assert!(!request_wire.contains("\"id\""), "{request_wire}");
    let response_wire = wire.response_bodies.lock().expect("wire lock")[0].clone();
    assert!(
        !response_wire.contains("secretToken"),
        "writeOnly must never reach the response wire: {response_wire}"
    );

    // Optional cells may also stay ABSENT in both directions.
    let sparse = fx08::views::SyncedRecordWrite {
        label: "sparse".to_owned(),
        secret_token: OptionalField::Absent,
    };
    client.sync_record(&sparse).await.expect("200 documented");
    let second = app.received_synced.lock().expect("synced lock")[1].clone();
    assert_eq!(second, sparse);

    // Companion §9 continuity: constraints on KEPT write-view fields are
    // enforced by the view's own validate_request — a short label rejects
    // 422 before the handler runs.
    let invalid = fx08::views::SyncedRecordWrite {
        label: "ab".to_owned(),
        secret_token: OptionalField::Absent,
    };
    let raw = reqwest::Client::new()
        .put(format!("{base_url}/synced"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .body("{\"label\":\"ab\"}")
        .send()
        .await
        .expect("raw request sends");
    assert_eq!(
        raw.status(),
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "kept-field constraint violations must reject 422"
    );
    assert_eq!(
        app.received_synced.lock().expect("synced lock").len(),
        2,
        "the violating body never reached the handler"
    );

    // And the typed client still encodes the same violation leniently
    // (validation is a SERVER-side request policy); the wire carries it and
    // the router rejects identically as an undocumented status.
    let rejected = client.sync_record(&invalid).await;
    match rejected {
        Err(openapi_support::client_error::ClientError::UndocumentedStatus { status }) => {
            assert_eq!(status, http::StatusCode::UNPROCESSABLE_ENTITY);
        }
        other => panic!("expected undocumented-status error, got {other:?}"),
    }
}
