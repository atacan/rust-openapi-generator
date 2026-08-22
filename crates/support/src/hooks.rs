//! Object-safe observability hooks (DECISIONS.md D-impl-hooks; main spec §34.1 step 3, §40 step 3).

/// Fires when bounded response encoding overflows its limit (section 34.1).
pub trait EncodeOverflowHook: Send + Sync {
    fn on_encode_overflow(&self, operation_id: &str, variant: &str, limit: usize);
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
        assert_eq!(
            *hook.calls.lock().expect("recording hook lock"),
            vec![("listWidgets".to_owned(), "Ok200".to_owned(), 4096)]
        );
    }
}
