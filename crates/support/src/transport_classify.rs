//! Read-side transport classification for committed response streams (main
//! spec §40).
//!
//! §40 makes premature termination of a committed server stream observable
//! CLIENT-SIDE as the framing-specific truncation variant
//! ([`SseDecodeError::Truncated`](crate::stream_errors::SseDecodeError::Truncated),
//! [`NdjsonDecodeError::Truncated`](crate::stream_errors::NdjsonDecodeError::Truncated),
//! or
//! [`JsonSeqDecodeError::Truncated`](crate::stream_errors::JsonSeqDecodeError::Truncated)),
//! never as clean end-of-stream. Over reqwest/hyper an abrupt server abort
//! does not surface as EOF; it surfaces as a transport error whose `source()`
//! chain contains a [`hyper::Error`] describing a READ that ended before the
//! promised message was complete. This module recognizes exactly those shapes,
//! so generated stream adapters can remap them to the `Truncated` variant
//! while every other transport failure keeps flowing through as the framing's
//! `Source` variant with its cause preserved for diagnostics.
//!
//! # Predicates chosen (hyper 1.x public API)
//!
//! [`is_premature_body_end`] walks every link of the error source chain,
//! downcasts each link to [`hyper::Error`], and accepts when ANY link answers
//! one of the two read-side rules below. Everything else keeps flowing through
//! as the framing's `Source` variant.
//!
//! 1. [`hyper::Error::is_incomplete_message`] — "the connection closed before
//!    a message could complete": the underlying IO reported EOF/closed while
//!    hyper's HTTP state still required more of the message (a partially
//!    received head, a body cut short on a connection hyper tracks as
//!    incomplete).
//! 2. Read-side IO termination beneath the hyper link: the hyper error is
//!    neither parse-, user-, nor timeout-classified, and its own source chain
//!    bottoms out in a [`std::io::Error`] whose kind reports the CONNECTION
//!    ended — [`std::io::ErrorKind::UnexpectedEof`], `ConnectionReset`,
//!    `ConnectionAborted`, or `BrokenPipe`. This is the shape actually
//!    observed over real TCP for an abrupt post-commit abort of an HTTP/1
//!    chunked body: reqwest ("error decoding response body") → hyper
//!    (`Kind::Body`, "error reading a body from connection"; hyper 1.x
//!    exposes NO public predicate for that kind) → io ("unexpected EOF during
//!    chunk size line", kind `UnexpectedEof`). A read-side body error rooted
//!    in one of those IO kinds means the body could not be read to
//!    completion — precisely §40 truncation.
//!
//! Deliberately NOT treated as truncation (these stay `...DecodeError::Source`):
//!
//! - [`hyper::Error::is_parse`] (and the finer `is_parse_*`) — malformed
//!   messages are representation faults, not remote termination.
//! - [`hyper::Error::is_user`] — application-code faults, including the
//!   SERVER-side producer error that caused an abort; clients only ever see
//!   its consequence (an ended connection), never the marker itself.
//! - [`hyper::Error::is_timeout`] — header-read/user timers are policy
//!   failures with their own diagnostics; checked BEFORE rule 2 so a timed-out
//!   read whose cause chain carries an IO link still stays `Source`.
//! - [`hyper::Error::is_body_write_aborted`] — WRITE-side only: the local
//!   sender dropped an outgoing body; it never describes the remote ending
//!   early.
//! - [`hyper::Error::is_shutdown`] — failure during local graceful-close write
//!   sequencing, not remote termination.
//! - [`hyper::Error::is_closed`] / `is_canceled` — LOCAL channel/request
//!   cancellation (e.g. the consumer dropped the stream); the opposite of a
//!   remote premature end.
//! - hyper links whose cause chain bottoms out in anything else (TLS layer
//!   errors, h2 protocol reasons, channel states) stay `Source`: hyper does
//!   not expose a public predicate vouching for their completion state
//!   (documented limitation: some HTTP/2 resets may therefore remain
//!   `Source`).
//!
//! Chains without any hyper link (plain [`std::io::Error`]s, custom transport
//! wrappers) never match: without hyper's own completion state this module
//! refuses to guess.
//!
//! # Testing note
//!
//! `hyper::Error` exposes no public constructor (`new_incomplete`,
//! `new_io`, … are all `pub(super)` in hyper 1.x), so a real
//! incomplete-message instance cannot be synthesized cheaply inside a unit
//! test. The unit tests below pin the negative space (non-hyper chains of
//! several shapes); the positive path is pinned END-TO-END over real TCP by
//! the conformance suite (§50 test 33:
//! `crates/conformance/tests/stream_boundary.rs`, including the probe that
//! asserts the abrupt-abort mapping stays `Truncated`).

/// Decides whether an error chain describes a READ-side premature response
/// body end (main spec §40): some link is a [`hyper::Error`] whose HTTP state
/// says the connection closed before the promised message completed.
///
/// Used by generated stream adapters to remap such failures to their framing's
/// `Truncated` decode-error variant; everything else stays as the framing's
/// `Source` variant.
///
/// The `'static` object bound matches how every classified chain is produced
/// (`Box<dyn Error + Send + Sync>` carries the implicit `'static`) and is
/// required by [`std::error::Error::downcast_ref`], the mechanism the walk
/// uses to recognize hyper links.
#[must_use]
pub fn is_premature_body_end(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    // Walking `source()` drops Send/Sync from the object type (the trait's
    // declared return type is `&(dyn Error + 'static)`), so the loop runs at
    // that width; `downcast_ref` needs exactly this much.
    let mut current = Some(error as &(dyn std::error::Error + 'static));
    while let Some(link) = current {
        if let Some(hyper_error) = link.downcast_ref::<hyper::Error>() {
            if hyper_read_premature(hyper_error) {
                return true;
            }
        }
        current = link.source();
    }
    false
}

/// Applies the two read-side rules to one [`hyper::Error`] link (see module
/// docs): canonical incomplete-message state, or a non-parse/non-user/
/// non-timeout hyper error whose cause chain bottoms out in an IO error
/// reporting the connection ended.
fn hyper_read_premature(error: &hyper::Error) -> bool {
    // Rule 1: hyper's own closed-before-complete state.
    if error.is_incomplete_message() {
        return true;
    }
    // Explicit rejections first: malformed messages and application faults are
    // never remote termination, and a timed-out read stays a policy failure
    // even when its chain carries an IO link.
    if error.is_parse() || error.is_user() || error.is_timeout() {
        return false;
    }
    // Rule 2: read-side connection termination rooted in IO. Only kinds that
    // report the CONNECTION ended count; a TLS alert, protocol reason, or any
    // non-IO bottom keeps the failure as `Source`.
    let mut cause = std::error::Error::source(error);
    while let Some(link) = cause {
        if let Some(io_error) = link.downcast_ref::<std::io::Error>() {
            return matches!(
                io_error.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            );
        }
        cause = link.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Leaf(&'static str);

    impl std::fmt::Display for Leaf {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Leaf {}

    /// Two-link chain: wrapper → leaf, neither hyper.
    #[derive(Debug)]
    struct Wrapper(Leaf);

    impl std::fmt::Display for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("wrapper")
        }
    }

    impl std::error::Error for Wrapper {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    /// Three-link chain built out of boxed sources, mirroring how reqwest
    /// nests transport errors (`reqwest -> hyper -> io` shape minus hyper).
    #[derive(Debug)]
    struct Boxed {
        message: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    }

    impl std::fmt::Display for Boxed {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for Boxed {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self.source.as_ref())
        }
    }

    #[test]
    fn plain_io_error_is_not_a_premature_body_end() {
        let error = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        assert!(!is_premature_body_end(&error));
    }

    #[test]
    fn multi_link_non_hyper_chain_walks_to_the_end_without_matching() {
        let inner = Boxed {
            message: "io layer",
            source: Box::new(std::io::Error::other("reset")),
        };
        let outer = Boxed {
            message: "transport layer",
            source: Box::new(inner),
        };
        assert!(!is_premature_body_end(&outer));
    }

    #[test]
    fn custom_wrappers_without_hyper_links_never_match() {
        let wrapped = Wrapper(Leaf("leaf"));
        assert!(!is_premature_body_end(&wrapped));
        assert!(!is_premature_body_end(&Leaf("alone")));
    }

    #[test]
    fn empty_and_unit_chains_are_rejected() {
        #[derive(Debug, thiserror::Error)]
        #[error("sourceless")]
        struct Sourceless;

        assert!(!is_premature_body_end(&Sourceless));
    }
}
