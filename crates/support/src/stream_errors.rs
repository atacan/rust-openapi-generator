//! Decode errors for record-framed streaming bodies and the committed-stream
//! failure type (main spec §40).
//!
//! §40 contract: client-side truncation is distinct from clean end-of-stream,
//! so callers never mistake truncation for success; server-side, once a stream
//! is committed no status can change and no in-band error frames exist — the
//! encoder reports through [`StreamFailureHook`](crate::hooks::StreamFailureHook)
//! and aborts the body. No fabricated statuses after commit.

/// Client decode error for an SSE body (main spec §40).
#[derive(Debug, thiserror::Error)]
pub enum SseDecodeError {
    /// Stream ended mid-record: distinct from clean end-of-stream.
    #[error("stream ended mid-record before a complete SSE event")]
    Truncated,
    /// Single event exceeded the configured per-record limit.
    #[error("SSE event exceeds the configured record limit of {limit} bytes")]
    RecordTooLarge {
        /// Configured per-record limit (`max_stream_record_bytes`).
        limit: usize,
    },
    /// Event data was not valid JSON.
    #[error("SSE event data is not valid JSON")]
    MalformedJson(
        /// Underlying JSON parse failure.
        #[source]
        serde_json::Error,
    ),
    /// Event bytes were not valid UTF-8.
    #[error("SSE event is not valid UTF-8")]
    NotUtf8,
}

/// Client decode error for an NDJSON body (main spec §40).
#[derive(Debug, thiserror::Error)]
pub enum NdjsonDecodeError {
    /// Stream ended mid-record: distinct from clean end-of-stream.
    #[error("stream ended mid-record before a complete NDJSON line")]
    Truncated,
    /// Single line exceeded the configured per-record limit.
    #[error("NDJSON line exceeds the configured record limit of {limit} bytes")]
    RecordTooLarge {
        /// Configured per-record limit (`max_stream_record_bytes`).
        limit: usize,
    },
    /// Line content was not valid JSON.
    #[error("NDJSON line is not valid JSON")]
    MalformedJson(
        /// Underlying JSON parse failure.
        #[source]
        serde_json::Error,
    ),
    /// Line bytes were not valid UTF-8.
    #[error("NDJSON line is not valid UTF-8")]
    NotUtf8,
}

/// Client decode error for a JSON Text Sequence body (RFC 7464; main spec §40).
#[derive(Debug, thiserror::Error)]
pub enum JsonSeqDecodeError {
    /// Stream ended mid-record: distinct from clean end-of-stream.
    #[error("stream ended mid-record before a complete JSON sequence record")]
    Truncated,
    /// Single record exceeded the configured per-record limit.
    #[error("JSON sequence record exceeds the configured record limit of {limit} bytes")]
    RecordTooLarge {
        /// Configured per-record limit (`max_stream_record_bytes`).
        limit: usize,
    },
    /// Record content was not valid JSON.
    #[error("JSON sequence record is not valid JSON")]
    MalformedJson(
        /// Underlying JSON parse failure.
        #[source]
        serde_json::Error,
    ),
    /// Record bytes were not valid UTF-8.
    #[error("JSON sequence record is not valid UTF-8")]
    NotUtf8,
    /// A record was not introduced by the mandatory record separator (RS).
    #[error("JSON sequence record missing its leading record separator (RS)")]
    MissingRecordSeparator,
}

/// Failure raised by an application stream after the response was committed
/// (main spec §40 steps 1–3).
///
/// Generated stream encoders feed this to
/// [`StreamFailureHook`](crate::hooks::StreamFailureHook) and then terminate
/// the body abruptly: no fabricated statuses and no in-band error frames.
#[derive(Debug, thiserror::Error)]
#[error("application stream failed")]
pub struct ServerStreamError(#[source] Box<dyn std::error::Error + Send + Sync>);

impl ServerStreamError {
    #[must_use]
    pub fn new(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::StreamFailureHook;

    fn json_source() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").expect_err("truncated JSON object")
    }

    #[test]
    fn sse_variants_display_and_carry_sources() {
        assert_eq!(
            SseDecodeError::Truncated.to_string(),
            "stream ended mid-record before a complete SSE event"
        );
        let too_large = SseDecodeError::RecordTooLarge { limit: 1024 };
        assert_eq!(
            too_large.to_string(),
            "SSE event exceeds the configured record limit of 1024 bytes"
        );
        let malformed = SseDecodeError::MalformedJson(json_source());
        assert!(malformed.to_string().contains("not valid JSON"));
        assert!(std::error::Error::source(&malformed).is_some());
        assert_eq!(
            SseDecodeError::NotUtf8.to_string(),
            "SSE event is not valid UTF-8"
        );
    }

    #[test]
    fn ndjson_truncation_is_distinct_from_clean_eof() {
        let truncated = NdjsonDecodeError::Truncated.to_string();
        assert!(truncated.contains("stream ended mid-record"), "{truncated}");
        assert!(!truncated.contains("clean"));
    }

    #[test]
    fn json_seq_has_the_extra_separator_variant() {
        assert_eq!(
            JsonSeqDecodeError::MissingRecordSeparator.to_string(),
            "JSON sequence record missing its leading record separator (RS)"
        );
        let too_large = JsonSeqDecodeError::RecordTooLarge { limit: 1 };
        assert!(too_large.to_string().contains("record limit of 1 bytes"));
        let malformed = JsonSeqDecodeError::MalformedJson(json_source());
        assert!(std::error::Error::source(&malformed).is_some());
    }

    #[test]
    fn server_stream_error_wraps_any_application_error() {
        #[derive(Debug, thiserror::Error)]
        #[error("producer exploded")]
        struct Exploded;

        let error = ServerStreamError::new(Exploded);
        assert_eq!(error.to_string(), "application stream failed");
        assert_eq!(
            std::error::Error::source(&error)
                .expect("wrapped error surfaces as source")
                .to_string(),
            "producer exploded"
        );

        let boxed = ServerStreamError::new(Box::new(Exploded));
        assert_eq!(boxed.to_string(), "application stream failed");
    }

    #[test]
    fn server_stream_error_feeds_the_stream_failure_hook() {
        #[derive(Default)]
        struct RecordingHook(std::sync::Mutex<Vec<String>>);

        impl StreamFailureHook for RecordingHook {
            fn on_stream_failure(
                &self,
                operation_id: &str,
                _error: &(dyn std::error::Error + Send + Sync),
            ) {
                self.0
                    .lock()
                    .expect("hook lock")
                    .push(operation_id.to_owned());
            }
        }

        let hook = RecordingHook::default();
        let error = ServerStreamError::new("mid-production failure");
        let dyn_hook: &dyn StreamFailureHook = &hook;
        dyn_hook.on_stream_failure("listWidgets", &error);
        assert_eq!(
            *hook.0.lock().expect("hook lock"),
            vec!["listWidgets".to_owned()]
        );
    }
}
