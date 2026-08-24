//! Shared helpers for the runtime conformance suites: hermetic ephemeral-port
//! servers (127.0.0.1:0) hosting GENERATED routers.

#![allow(dead_code)] // each test binary uses a different subset

use std::net::SocketAddr;
use std::sync::Arc;

use openapi_support::hooks::{
    EncodeOverflowHook, NoOpEncodeOverflowHook, NoOpStreamFailureHook, StreamFailureHook,
};
use openapi_support::limits::BodyLimits;

/// Spawns an axum server on an OS-assigned loopback port serving `router`.
/// The task lives for the remainder of the test process; hermeticity comes
/// from the ephemeral port.
pub fn spawn_router(router: axum::Router) -> SocketAddr {
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

/// Builds the generated clients against a spawned server.
pub fn base_url(address: SocketAddr) -> String {
    format!("http://{address}")
}

/// Convenience: default limits + silent hook trio used by every router
/// (encode overflow §34.1, stream failure §40).
pub fn router_args() -> (
    BodyLimits,
    Arc<dyn EncodeOverflowHook>,
    Arc<dyn StreamFailureHook>,
) {
    (
        BodyLimits::process_default(),
        Arc::new(NoOpEncodeOverflowHook),
        Arc::new(NoOpStreamFailureHook),
    )
}

/// Builds one deterministic patterned byte block: byte `j` of chunk `index`
/// is `(index * len + j) % 251`. Cheap to produce lazily chunk-by-chunk.
#[must_use]
pub fn pattern_chunk(index: usize, len: usize) -> bytes::Bytes {
    let start = index * len;
    let mut data = Vec::with_capacity(len);
    for offset in 0..len {
        data.push(((start + offset) % 251) as u8);
    }
    bytes::Bytes::from(data)
}

/// Full expected payload for the streaming round trip.
#[must_use]
pub fn pattern_payload(total: usize, chunk_len: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(total);
    for index in 0..total / chunk_len {
        data.extend_from_slice(&pattern_chunk(index, chunk_len));
    }
    data
}
