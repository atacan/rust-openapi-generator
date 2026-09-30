//! Runtime regressions compiled beside the generated fixture-10 server.

pub mod server;

#[cfg(test)]
mod tests {
    use super::server::{CreateSession201, CreateSessionResponse};
    use api_types::models::Session;
    use axum::body::HttpBody;
    use openapi_support::hooks::EncodeOverflowHook;
    use openapi_support::limits::BodyLimits;
    use openapi_support::optional::OptionalField;
    use std::sync::Mutex;

    #[derive(Debug, PartialEq, Eq)]
    enum Event {
        Overflow(String, String, usize),
        Header(String, String, String),
    }

    #[derive(Default)]
    struct RecordingHook(Mutex<Vec<Event>>);

    impl EncodeOverflowHook for RecordingHook {
        fn on_encode_overflow(&self, op: &str, variant: &str, limit: usize) {
            self.0
                .lock()
                .unwrap()
                .push(Event::Overflow(op.into(), variant.into(), limit));
        }

        fn on_header_encode_failure(&self, op: &str, variant: &str, header: &str) {
            self.0
                .lock()
                .unwrap()
                .push(Event::Header(op.into(), variant.into(), header.into()));
        }
    }

    #[derive(Default)]
    struct LegacyHook(Mutex<Vec<Event>>);

    impl EncodeOverflowHook for LegacyHook {
        fn on_encode_overflow(&self, op: &str, variant: &str, limit: usize) {
            self.0
                .lock()
                .unwrap()
                .push(Event::Overflow(op.into(), variant.into(), limit));
        }
    }

    fn outcome(etag: Option<&str>) -> CreateSessionResponse {
        // The fields are public: applications can store invalid values even
        // when a checked constructor exists, so the encoder must handle them.
        CreateSessionResponse::Created201(CreateSession201 {
            location: "/sessions/s".into(),
            e_tag: etag.map(str::to_owned),
            body: Session {
                id: "s".into(),
                token: OptionalField::Absent,
            },
        })
    }

    fn assert_empty_500(response: axum::response::Response) {
        assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            response.headers().is_empty(),
            "discard all original headers"
        );
        assert_eq!(response.body().size_hint().exact(), Some(0));
    }

    #[test]
    fn invalid_header_reports_wire_name_once_and_discards_original_response() {
        let hook = RecordingHook::default();
        let response = outcome(Some("bad\r\nsplit"))
            .into_response_with_limits(&BodyLimits::process_default(), &hook);
        assert_empty_500(response);
        assert_eq!(
            *hook.0.lock().unwrap(),
            vec![Event::Header(
                "createSession".into(),
                "Created201".into(),
                "etag".into(),
            )]
        );
    }

    #[test]
    fn legacy_hook_still_receives_exactly_one_notification() {
        let hook = LegacyHook::default();
        let response = outcome(Some("bad\r\nsplit"))
            .into_response_with_limits(&BodyLimits::process_default(), &hook);
        assert_empty_500(response);
        assert_eq!(
            *hook.0.lock().unwrap(),
            vec![Event::Overflow(
                "createSession".into(),
                "Created201".into(),
                0,
            )]
        );
    }

    #[test]
    fn genuine_zero_byte_body_overflow_uses_only_overflow_hook() {
        let hook = RecordingHook::default();
        let limits = BodyLimits {
            structured_encode_bytes: 0,
            ..BodyLimits::process_default()
        };
        let response = CreateSessionResponse::Unauthorized401(api_types::models::ProblemDetails {
            title: "unauthorized".into(),
            detail: OptionalField::Absent,
        })
        .into_response_with_limits(&limits, &hook);
        assert_empty_500(response);
        assert_eq!(
            *hook.0.lock().unwrap(),
            vec![Event::Overflow(
                "createSession".into(),
                "Unauthorized401".into(),
                0,
            )]
        );
    }

    #[test]
    fn valid_and_absent_optional_headers_do_not_fire_hooks() {
        for etag in [Some("\"valid\""), None] {
            let hook = RecordingHook::default();
            let response =
                outcome(etag).into_response_with_limits(&BodyLimits::process_default(), &hook);
            assert_eq!(response.status(), http::StatusCode::CREATED);
            assert_eq!(response.headers()[http::header::LOCATION], "/sessions/s");
            assert_eq!(
                response
                    .headers()
                    .get(http::header::ETAG)
                    .map(|v| v.to_str().unwrap()),
                etag
            );
            assert!(hook.0.lock().unwrap().is_empty());
        }
    }
}
