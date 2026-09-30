//! Object-safe observability hooks (DECISIONS.md D-impl-hooks; main spec §34.1 step 3, §40 step 3).

/// Observes bounded response encoding overflows and header conversion failures.
pub trait EncodeOverflowHook: Send + Sync {
    /// Fires when bounded response encoding overflows its limit (section 34.1).
    ///
    /// For compatibility, the default [`Self::on_header_encode_failure`] also
    /// calls this method with `limit = 0`. Override that method to distinguish
    /// header failures from body overflows, including a genuine zero-byte limit.
    fn on_encode_overflow(&self, operation_id: &str, variant: &str, limit: usize);

    /// Fires when a documented response header cannot become an HTTP value.
    ///
    /// `header` is the wire name; the invalid value is deliberately omitted.
    /// The encoder discards the response and emits the fixed empty-bodied 500.
    /// The default delegates to [`Self::on_encode_overflow`] with `limit = 0`
    /// so existing implementations retain their notifications. An override
    /// replaces that delegation; the encoder calls this method only once.
    fn on_header_encode_failure(&self, operation_id: &str, variant: &str, _header: &str) {
        self.on_encode_overflow(operation_id, variant, 0);
    }
}

/// Fires when a committed stream fails mid-production (section 40).
pub trait StreamFailureHook: Send + Sync {
    fn on_stream_failure(&self, operation_id: &str, error: &(dyn std::error::Error + Send + Sync));
}

/// Silent default encode-overflow hook.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoOpEncodeOverflowHook;

impl EncodeOverflowHook for NoOpEncodeOverflowHook {
    fn on_encode_overflow(&self, _operation_id: &str, _variant: &str, _limit: usize) {}
}

/// Silent default stream-failure hook.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoOpStreamFailureHook;

impl StreamFailureHook for NoOpStreamFailureHook {
    fn on_stream_failure(
        &self,
        _operation_id: &str,
        _error: &(dyn std::error::Error + Send + Sync),
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("boom")]
    struct Boom;

    #[test]
    fn no_op_hooks_are_callable_and_object_safe() {
        let encode_hook: &dyn EncodeOverflowHook = &NoOpEncodeOverflowHook;
        let stream_hook: &dyn StreamFailureHook = &NoOpStreamFailureHook;
        encode_hook.on_encode_overflow("op", "Ok200", 8);
        encode_hook.on_header_encode_failure("op", "Ok200", "etag");
        let error = Boom;
        stream_hook.on_stream_failure("op", &error as &(dyn std::error::Error + Send + Sync));
    }

    #[test]
    fn recording_hooks_capture_arguments() {
        #[derive(Default)]
        struct RecordingEncodeHook {
            calls: std::sync::Mutex<Vec<(String, String, usize)>>,
        }

        impl EncodeOverflowHook for RecordingEncodeHook {
            fn on_encode_overflow(&self, operation_id: &str, variant: &str, limit: usize) {
                self.calls.lock().expect("recording hook lock").push((
                    operation_id.to_owned(),
                    variant.to_owned(),
                    limit,
                ));
            }
        }

        let hook = RecordingEncodeHook::default();
        hook.on_encode_overflow("listWidgets", "Ok200", 4096);
        let object: &dyn EncodeOverflowHook = &hook;
        object.on_header_encode_failure("listWidgets", "Ok200", "etag");
        assert_eq!(
            *hook.calls.lock().expect("recording hook lock"),
            vec![
                ("listWidgets".to_owned(), "Ok200".to_owned(), 4096),
                ("listWidgets".to_owned(), "Ok200".to_owned(), 0),
            ]
        );
    }
}
