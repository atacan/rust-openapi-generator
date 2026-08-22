//! Default body limits (main spec §33, DECISIONS.md D-impl-limits).

/// Purpose-separated body limits for generated clients and routers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyLimits {
    /// Maximum decoded request-body size for structured media types (§30.2).
    pub structured_request_bytes: usize,
    /// Maximum decoded response-body size for structured media types (§30.2).
    pub structured_response_bytes: usize,
    /// Maximum decoded error-response body size collected for problem decoding.
    pub error_response_bytes: usize,
    /// Bounded serialization budget for finite bodies (section 34).
    pub structured_encode_bytes: usize,
    /// Maximum plain-text body size accumulated before decoding.
    pub text_body_bytes: usize,
    /// Maximum buffered size of a single scalar or metadata part in multipart (section 17.1).
    pub multipart_scalar_part_bytes: usize,
    /// Maximum single-record size in record-framed streaming bodies (NDJSON, SSE).
    pub max_stream_record_bytes: usize,
    /// Maximum number of parts accepted per multipart message (section 17.1).
    pub max_multipart_parts: usize,
    /// Maximum bytes spent on any single part's headers (section 17.1).
    pub max_part_header_bytes: usize,
    /// Maximum field-name length in multipart parts.
    pub max_field_name_bytes: usize,
    /// Maximum file-name length in multipart file parts.
    pub max_file_name_bytes: usize,
    /// First-frame peek cap for optional-body presence detection (sections 28.2, 17.1).
    pub peek_buffer_bytes: usize,
    /// Nested multipart framing depth guard.
    pub max_multipart_depth: usize,
}

impl BodyLimits {
    /// Defaults from DECISIONS.md D-impl-limits; overridable through generator configuration.
    #[must_use]
    pub fn process_default() -> Self {
        const MIB: usize = 1024 * 1024;
        const KIB: usize = 1024;
        Self {
            structured_request_bytes: 8 * MIB,
            structured_response_bytes: 8 * MIB,
            error_response_bytes: MIB,
            structured_encode_bytes: 8 * MIB,
            text_body_bytes: 8 * MIB,
            multipart_scalar_part_bytes: MIB,
            max_stream_record_bytes: MIB,
            max_multipart_parts: 1000,
            max_part_header_bytes: 64 * KIB,
            max_field_name_bytes: 256,
            max_file_name_bytes: 1024,
            peek_buffer_bytes: 8 * KIB,
            max_multipart_depth: 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_default_matches_decision_table() {
        let mib = 1024 * 1024;
        let limits = BodyLimits::process_default();
        assert_eq!(limits.structured_request_bytes, 8 * mib);
        assert_eq!(limits.structured_response_bytes, 8 * mib);
        assert_eq!(limits.error_response_bytes, mib);
        assert_eq!(limits.structured_encode_bytes, 8 * mib);
        assert_eq!(limits.text_body_bytes, 8 * mib);
        assert_eq!(limits.multipart_scalar_part_bytes, mib);
        assert_eq!(limits.max_stream_record_bytes, mib);
        assert_eq!(limits.max_multipart_parts, 1000);
        assert_eq!(limits.max_part_header_bytes, 64 * 1024);
        assert_eq!(limits.max_field_name_bytes, 256);
        assert_eq!(limits.max_file_name_bytes, 1024);
        assert_eq!(limits.peek_buffer_bytes, 8 * 1024);
        assert_eq!(limits.max_multipart_depth, 4);
    }

    #[test]
    fn limits_are_copy_and_comparable() {
        let a = BodyLimits::process_default();
        let b = a;
        assert_eq!(a, b);
    }
}
