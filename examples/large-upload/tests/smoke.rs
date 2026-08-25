//! Ignored end-to-end smoke tests over real TCP (main spec §50 spirit):
//!
//! * `smoke_disk_round_trip` — spawns the demo application's generated
//!   router in DISK mode on an ephemeral loopback port and drives the
//!   identical transfer sweep the client binary runs at a small size,
//!   asserting every step passes AND the bounded-memory threshold holds
//!   (generous margin; CI machines are noisy).
//! * `smoke_proxy_chain` — spawns a DISK-mode backend plus a PROXY-mode
//!   frontend pointing at it, so the streamed bytes traverse TWO axum/reqwest
//!   hops before landing on disk; receipts must still verify.
//!
//! Gated behind `#[ignore]` so plain `cargo test` stays hermetic and fast;
//! run with `cargo test -p large-upload -- --ignored`.

use std::net::SocketAddr;
use std::sync::Arc;

use large_upload::demo::{self, LargeUploadApp};
use large_upload::memmon;

/// Spawns an axum server on an OS-assigned loopback port serving `router`
/// (helper copied from crates/conformance/tests/common/mod.rs; the task lives
/// for the remainder of the test process — hermeticity comes from the
/// ephemeral port).
fn spawn_router(router: axum::Router) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let listener = tokio::net::TcpListener::from_std(listener).expect("std listener");
    let address = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("server terminated unexpectedly");
    });
    address
}

/// Unique per-process scratch directory under the system temp dir.
fn scratch(tag: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("large-upload-smoke-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("create scratch dir");
    path
}

async fn assert_transfers_clean(base_url: &str, work_dir: &std::path::Path) {
    let client = demo::build_client(base_url).expect("client builds");
    let (steps, report) = demo::run_transfers(&client, work_dir, 8).await; // 8 MiB

    for step in &steps {
        println!(
            "{:>15}  {:>4}  {}",
            step.op,
            if step.ok { "ok" } else { "FAIL" },
            step.message
        );
    }
    println!("{}", report.render(memmon::DEFAULT_MAX_RSS_DELTA_MIB));

    let failures: Vec<&demo::StepOutcome> = steps.iter().filter(|step| !step.ok).collect();
    assert!(
        failures.is_empty(),
        "{} of {} steps failed against {base_url}: {failures:?}",
        failures.len(),
        steps.len()
    );
    // Generous margin: the assertion targets gross aggregation bugs (an
    // 8 MiB upload buffered whole would already breach it), not noise.
    assert!(
        !report.breaches(64),
        "memory delta {} MiB exceeded the generous smoke margin",
        memmon::mib(report.delta_bytes())
    );
}

#[tokio::test]
#[ignore = "end-to-end smoke over real TCP; run with cargo test -p large-upload -- --ignored"]
async fn smoke_disk_round_trip() {
    let store_dir = scratch("store");
    let work_dir = scratch("work");
    let app = Arc::new(LargeUploadApp::new(store_dir.clone(), None));
    let address = spawn_router(demo::demo_router(app));

    assert_transfers_clean(&format!("http://{address}"), &work_dir).await;

    let _unused = std::fs::remove_dir_all(store_dir);
    let _unused = std::fs::remove_dir_all(work_dir);
}

#[tokio::test]
#[ignore = "end-to-end proxy-chain smoke over real TCP; run with cargo test -p large-upload -- --ignored"]
async fn smoke_proxy_chain() {
    let backend_store = scratch("backend-store");
    let frontend_work = scratch("frontend-work");

    // Backend: disk mode.
    let backend_app = Arc::new(LargeUploadApp::new(backend_store.clone(), None));
    let backend_address = spawn_router(demo::demo_router(Arc::clone(&backend_app)));

    // Frontend: proxy mode, forwarding everything to the backend.
    let frontend_app = Arc::new(LargeUploadApp::new(
        scratch("frontend-unused"),
        Some(format!("http://{backend_address}")),
    ));
    let frontend_address = spawn_router(demo::demo_router(Arc::clone(&frontend_app)));

    assert_transfers_clean(&format!("http://{frontend_address}"), &frontend_work).await;

    let _unused = std::fs::remove_dir_all(backend_store);
    let _unused = std::fs::remove_dir_all(frontend_work);
}
