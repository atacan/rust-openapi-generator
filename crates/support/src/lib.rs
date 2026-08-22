//! Support runtime shipped with generated code.
//!
//! Bounded-I/O core: body limits (main spec §33), bounded collection of streamed
//! bodies (§30.2), fail-fast bounded serialization (§34), and observability hooks
//! (§34.1, §40). Protocol layer: pre-handler rejections (§39), identity-only
//! inbound content coding (§30.4), stream decode errors and committed-stream
//! failures (§40), and the presence/nullability matrix (companion §2.1).

pub mod collect;
pub mod content_coding;
pub mod encode;
pub mod hooks;
pub mod limits;
pub mod optional;
pub mod rejection;
pub mod stream_errors;
