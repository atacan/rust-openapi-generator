//! Ignored end-to-end smoke test (main spec §50 spirit): spawns the SAME
//! demo application's generated router — from the server crate, as a
//! dev-dependency — on an ephemeral loopback port over real TCP, drives the
//! identical full-operation sweep this crate's binary runs, and ASSERTS on
//! the outcome: every step passes and every documented operation was
//! exercised the expected number of times.
//!
//! Gated behind `#[ignore]` so plain `cargo test` stays hermetic and fast;
//! run with `cargo test -p kitchen-sink-client -- --ignored`.

use std::net::SocketAddr;
use std::sync::Arc;

use kitchen_sink_client::sweep::{self, REQUIRED_OPERATIONS};
use kitchen_sink_server::app::{self, KitchenSinkApp};

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

#[tokio::test]
#[ignore = "end-to-end smoke over real TCP; run with cargo test -p kitchen-sink-client -- --ignored"]
async fn smoke_round_trip() {
    let objects_dir =
        std::env::temp_dir().join(format!("kitchen-sink-smoke-{}", std::process::id()));
    let app = Arc::new(KitchenSinkApp::new(objects_dir.clone()));
    let address = spawn_router(app::demo_router(app));

    let base_url = format!("http://{address}");
    let client = sweep::build_client(&base_url).expect("client builds");
    let steps = sweep::run_sweep(&client).await;

    for step in &steps {
        println!(
            "{:>20}  {:>4}  {}",
            step.op,
            if step.ok { "ok" } else { "FAIL" },
            step.message
        );
    }

    let failures: Vec<&sweep::SweepStep> = steps.iter().filter(|step| !step.ok).collect();
    assert!(
        failures.is_empty(),
        "{} of {} sweep steps failed against {base_url}: {failures:?}",
        failures.len(),
        steps.len()
    );

    // Every documented operation exercised exactly as often as the sweep
    // intends (getThumbnail twice, probeStatus four times, echoNote three
    // times for absent/null/value, everything else once).
    for op in REQUIRED_OPERATIONS {
        let expected = match op {
            "getThumbnail" => 2,
            "probeStatus" => 4,
            "echoNote" => 3,
            _ => 1,
        };
        let seen = steps.iter().filter(|step| step.op == op).count();
        assert_eq!(
            seen, expected,
            "operation `{op}` ran {seen} times, expected {expected}"
        );
    }
    assert_eq!(
        steps.len(),
        28,
        "sweep must stay a fixed, complete itinerary"
    );

    let _ = std::fs::remove_dir_all(objects_dir);
}
