//! Support runtime shipped with generated code.
//!
//! Bounded-I/O core: body limits (main spec §33), bounded collection of streamed
//! bodies (§30.2), fail-fast bounded serialization (§34), and observability hooks
//! (§34.1, §40). Protocol layer: pre-handler rejections (§39), identity-only
//! inbound content coding (§30.4), stream decode errors and committed-stream
//! failures (§40), and the presence/nullability matrix (companion §2.1).
//! Dispatch layer: media-type parsing/matching and peek-and-preserve presence
//! detection (§28), percent encoding (companion §8), the parameter
//! style × explode matrix (companion §6), and the authoritative
//! `ClientError` (§36).

pub mod collect;
pub mod content_coding;
pub mod encode;
pub mod hooks;
pub mod limits;
pub mod mediatype;
pub mod optional;
pub mod params;
pub mod percent;
pub mod rejection;
pub mod stream_errors;

#[cfg(feature = "client")]
pub mod client_error;
#[cfg(feature = "server")]
pub mod peek;
