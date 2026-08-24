/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{validate_slug_request, FeedbackForm, NoteReceipt, RegisteredSlug, Ticket};
use ::axum::response::IntoResponse;
use ::openapi_support::collect::{collect_body_limited, CollectLimitedError};
use ::openapi_support::content_coding::ensure_identity_content_coding;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::form::decode_form_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{
    is_wildcard_incoming, match_entry, parse_content_type, EntryMatch, ParsedMediaType,
};
use ::openapi_support::multipart::{
    extract_boundary, stream_multipart, MultipartError, MultipartEvent, MultipartLimits,
};
use ::openapi_support::peek::{detect_body_presence, BodyPresence};
use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};

/// Documented outcomes for `create_ticket` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateTicketResponse {
    /// HTTP 201 Created.
    Created201(Ticket),
}

/// Bounded encoder for [`CreateTicketResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreateTicketResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "createTicket",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for CreateTicketResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `register_slug` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum RegisterSlugResponse {
    /// HTTP 201 Created.
    Created201(RegisteredSlug),
}

/// Bounded encoder for [`RegisterSlugResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl RegisterSlugResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "registerSlug",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for RegisterSlugResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `post_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostNoteResponse {
    /// HTTP 201 Created.
    Created201(NoteReceipt),
}

/// Bounded encoder for [`PostNoteResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PostNoteResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "postNote",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for PostNoteResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `send_feedback` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum SendFeedbackResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

/// Bounded encoder for [`SendFeedbackResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl SendFeedbackResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        _limits: &BodyLimits,
        _hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::NoContent204 => ::http::StatusCode::NO_CONTENT.into_response(),
        }
    }
}

impl ::axum::response::IntoResponse for SendFeedbackResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Live streaming view over binary part `attachment` of `upload_attachment` (main spec §17 Output B): `next_chunk` yields payload bytes with backpressure as they arrive; nothing aggregates the part.
/// Sequential semantics (§51.4): while this part is open the rest of the message cannot advance. Trailing parts flow through this type's tail scan instead of the pre-handler pass: declared scalar/JSON parts arriving behind the stream decode bounded onto `trailing_parts`, required wire names the clean end-of-message still lacks surface exactly one terminal SchemaViolation from `next_chunk` (§17.1 enforced on wire arrival), duplicate single-valued reopenings reject, every observed name is recorded for `unknown_part_names`, and all remaining payloads drain without buffering.
pub struct UploadAttachmentAttachmentPart {
    pub file_name: Option<String>,
    pub content_type: Option<::mime::Mime>,
    /// Scalar/JSON parts that arrived BEHIND this streaming part, decoded bounded as their boundaries closed (§38 application-owned tail); their pre-handoff siblings live on [`UploadAttachmentMultipartInput`].
    pub trailing_parts: UploadAttachmentTrailingParts,
    events: MultipartEvents,
    log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,
    stage: UploadAttachmentAttachmentPartTailStage,
    scalar_limit: usize,
    buffer: Vec<u8>,
    finished: bool,
    /// Required scalar/JSON wire names still unseen at the streaming handoff (§17.1): satisfied by trailing arrivals, otherwise reported once at the clean end-of-message.
    pending_required: Vec<&'static str>,
    /// Declared SINGLE-VALUED wire names already consumed anywhere before now (pre-handoff or behind the stream): any reopening violates §17.1.
    seen_single_valued: Vec<String>,
}

/// Tail-scan stages of [`UploadAttachmentAttachmentPart`] (§51.4 sequential semantics): `Idle` delivers payload chunks to the application; `Drain` discards them; the remaining stages bounded-buffer one trailing scalar/JSON part behind the stream.
#[derive(Debug, Clone, Copy)]
enum UploadAttachmentAttachmentPartTailStage {
    Idle,
    Drain,
    Title,
    Kind,
}

/// Scalar/JSON parts observed BEHIND [`UploadAttachmentAttachmentPart`] (main spec §17 Output B): decoded bounded as their boundaries closed. Parts consumed BEFORE the streaming handoff live on [`UploadAttachmentMultipartInput`]; the split mirrors wire arrival (§38 application-owned tail).
#[derive(Debug, Default)]
pub struct UploadAttachmentTrailingParts {
    /// Textual part `title`.
    pub title: Option<String>,
    /// Textual part `kind`.
    pub kind: Option<String>,
}

impl UploadAttachmentAttachmentPart {
    /// Advances to the next payload chunk of this part (`None` at its clean end). Violations encountered while scanning trailing parts surface here as protocol rejections because sequential streaming cannot validate them any earlier; at the clean end-of-message, required parts still pending produce exactly one terminal SchemaViolation naming them (§17.1) and later calls keep returning `None`.
    #[allow(clippy::missing_errors_doc)]
    pub async fn next_chunk(&mut self) -> Result<Option<::bytes::Bytes>, ProtocolRejection> {
        if self.finished {
            return Ok(None);
        }
        loop {
            let event = match next_multipart_event(&mut self.events).await {
                Some(event) => event,
                None => {
                    self.finished = true;
                    if !self.pending_required.is_empty() {
                        let names = self.pending_required.join("`, `");
                        return Err(schema_violation(format!(
                            "missing required part(s) `{names}`",
                        )));
                    }
                    return Ok(None);
                }
            };
            match event {
                Err(error) => return Err(multipart_rejection(&error)),
                Ok(MultipartEvent::PartBegin(headers)) => {
                    if self.seen_single_valued.contains(&headers.name) {
                        return Err(schema_violation(format!(
                            "duplicate single-valued part `{}` after the streaming part",
                            headers.name
                        )));
                    }
                    match headers.name.as_str() {
                        "title" => {
                            self.seen_single_valued.push("title".to_owned());
                            self.pending_required.retain(|name| *name != "title");
                            self.buffer.clear();
                            self.stage = UploadAttachmentAttachmentPartTailStage::Title;
                        }
                        "kind" => {
                            self.seen_single_valued.push("kind".to_owned());
                            self.pending_required.retain(|name| *name != "kind");
                            self.buffer.clear();
                            self.stage = UploadAttachmentAttachmentPartTailStage::Kind;
                        }
                        other => {
                            multipart_record_unknown(&self.log, other);
                            self.stage = UploadAttachmentAttachmentPartTailStage::Drain;
                        }
                    }
                }
                Ok(MultipartEvent::PartChunk(chunk)) => match self.stage {
                    UploadAttachmentAttachmentPartTailStage::Idle => return Ok(Some(chunk)),
                    UploadAttachmentAttachmentPartTailStage::Drain => {}
                    UploadAttachmentAttachmentPartTailStage::Title
                    | UploadAttachmentAttachmentPartTailStage::Kind => {
                        self.buffer.extend_from_slice(&chunk);
                        if self.buffer.len() > self.scalar_limit {
                            return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));
                        }
                    }
                },
                Ok(MultipartEvent::PartEnd) => match self.stage {
                    UploadAttachmentAttachmentPartTailStage::Idle
                    | UploadAttachmentAttachmentPartTailStage::Drain => {}
                    UploadAttachmentAttachmentPartTailStage::Title => {
                        let value = multipart_scalar_text::<String>("title", &self.buffer)?;
                        self.trailing_parts.title = Some(value);
                        self.stage = UploadAttachmentAttachmentPartTailStage::Idle;
                    }
                    UploadAttachmentAttachmentPartTailStage::Kind => {
                        let value = multipart_scalar_text::<String>("kind", &self.buffer)?;
                        require_valid_request("part `kind`", validate_slug_request(&value))?;
                        self.trailing_parts.kind = Some(value);
                        self.stage = UploadAttachmentAttachmentPartTailStage::Idle;
                    }
                },
            }
        }
    }
}

/// Streaming multipart input for `upload_attachment` (main spec §17 Output B): scalar/JSON parts were bounded-buffered and decoded during the router's single incremental pass up to the streaming handoff; binary parts stay live streams over the request body.
/// Required-part enforcement is wire-arrival-based (§17.1, §38): parts arriving BEFORE the first binary validate pre-handler in that pass; parts arriving behind the live stream decode onto its `trailing_parts`, and required names a clean end-of-message never delivers reject through the live part's terminal error instead of a pre-handler rejection.
pub struct UploadAttachmentMultipartInput {
    /// Textual part `title`: `Some` when it arrived before the streaming part; otherwise decoded onto the live part's `trailing_parts`.
    pub title: Option<String>,
    /// Textual part `kind`: `Some` when it arrived before the streaming part; otherwise decoded onto the live part's `trailing_parts`.
    pub kind: Option<String>,
    /// Streaming binary part `attachment`.
    pub attachment: Option<UploadAttachmentAttachmentPart>,
    unknown_log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,
}

impl UploadAttachmentMultipartInput {
    /// Wire names of every unrecognized or late-arriving part observed so far (§17.1 unknown-fields-ignore default): their payloads stream past without buffering and never reject. Names behind a streaming part appear once the application drains it through `next_chunk`.
    pub fn unknown_part_names(&self) -> Vec<String> {
        match self.unknown_log.lock() {
            Ok(guard) => guard.names.clone(),
            Err(poisoned) => poisoned.into_inner().names.clone(),
        }
    }
}

/// Documented outcomes for `upload_attachment` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum UploadAttachmentResponse {
    /// HTTP 201 Created.
    Created201(serde_json::Map<String, serde_json::Value>),
}

/// Bounded encoder for [`UploadAttachmentResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl UploadAttachmentResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "uploadAttachment",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for UploadAttachmentResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `POST` `/tickets`.
    /// Operation `createTicket`.
    async fn create_ticket(&self, body: Ticket) -> CreateTicketResponse;

    /// `POST` `/slugs`.
    /// Operation `registerSlug`.
    async fn register_slug(&self, body: String) -> RegisterSlugResponse;

    /// `POST` `/notes`.
    /// Operation `postNote`.
    async fn post_note(&self, body: String) -> PostNoteResponse;

    /// `POST` `/feedback`.
    /// Operation `sendFeedback`.
    async fn send_feedback(&self, body: FeedbackForm) -> SendFeedbackResponse;

    /// `POST` `/uploads`.
    /// Operation `uploadAttachment`.
    async fn upload_attachment(
        &self,
        body: UploadAttachmentMultipartInput,
    ) -> UploadAttachmentResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
}

/// Route handler for `POST` `/tickets` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_ticket(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: Ticket = decode_json_body(&bytes)?;
            require_valid_request("body", value.validate_request())?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.create_ticket(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/slugs` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_register_slug(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: String = decode_json_body(&bytes)?;
            require_valid_request("body", validate_slug_request(&value))?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.register_slug(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/notes` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_post_note(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["text/plain"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.text_body_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.text_body_bytes).await?;
            let value = decode_text_body(bytes)?;
            require_valid_request("body", validate_slug_request(&value))?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.post_note(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/feedback` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_send_feedback(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body =
        match classify_request_entry(parsed.as_ref(), &["application/x-www-form-urlencoded"]) {
            RequestEntryMatch::AbsentContentType => {
                let probe = body_bytes(body, limits.structured_request_bytes).await?;
                if probe.is_empty() {
                    return Err(malformed_body("required request body arrived empty"));
                }
                return Err(unsupported_media_type(
                    "request arrived without a Content-Type",
                ));
            }
            RequestEntryMatch::Unmatched => {
                return Err(unsupported_media_type(
                    "no documented request media type matches the request Content-Type",
                ));
            }
            RequestEntryMatch::Entry(0) => {
                ensure_utf8_charset(parsed.as_ref())?;
                let bytes = body_bytes(body, limits.structured_request_bytes).await?;
                if bytes.is_empty() {
                    return Err(malformed_body("documented request body arrived empty"));
                }
                let value: FeedbackForm =
                    decode_form_body(&bytes, limits.structured_request_bytes)?;
                require_valid_request("body", value.validate_request())?;
                value
            }
            RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
        };
    let api = __state.api.as_ref();
    let response = api.send_feedback(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/uploads` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_upload_attachment(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["multipart/form-data"]) {
        RequestEntryMatch::AbsentContentType => {
            let (presence, _replay) =
                detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;
            if matches!(presence, BodyPresence::Empty) {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            let request_body =
                collect_upload_attachment_multipart(body, parsed.as_ref(), &limits).await?;
            request_body
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.upload_attachment(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Runs the §38 pre-handler pipeline for the `multipart/form-data` body of `upload_attachment` (main spec §5.5/§17/§17.1): one incremental pass up to the streaming handoff; scalar/JSON parts buffer only up to `multipart_scalar_part_bytes`; duplicate-single-valued parts reject before the trait runs, and required names the pass never observed reject here unless they may still arrive behind the live stream — those ride its `pending_required` set (wire-arrival-based enforcement, §17.1/§38).
#[allow(clippy::missing_errors_doc)]
#[allow(clippy::too_many_lines)]
async fn collect_upload_attachment_multipart(
    body: ::axum::body::Body,
    parsed: Option<&ParsedMediaType>,
    limits: &BodyLimits,
) -> Result<UploadAttachmentMultipartInput, ProtocolRejection> {
    let parsed = match parsed {
        Some(parsed) => parsed,
        None => {
            return Err(malformed_body("multipart body requires a Content-Type"));
        }
    };
    let boundary = match extract_boundary(parsed) {
        Ok(boundary) => boundary,
        Err(_) => {
            return Err(malformed_body(
                "multipart Content-Type lacks a usable boundary",
            ));
        }
    };
    let mut events: MultipartEvents = Box::pin(stream_multipart(
        body.into_data_stream(),
        boundary,
        MultipartLimits::from_body_limits(limits),
    ));
    let unknown_log =
        ::std::sync::Arc::new(::std::sync::Mutex::new(MultipartUnknownLog::default()));

    #[derive(Clone, Copy)]
    enum Stage {
        Idle,
        Drain,
        Title,
        Kind,
    }
    let mut stage = Stage::Idle;
    let mut title: Option<String> = None;
    let mut seen_title = false;
    let mut kind: Option<String> = None;
    let mut seen_kind = false;
    let mut attachment_part: Option<UploadAttachmentAttachmentPart> = None;
    let mut buffer: Vec<u8> = Vec::new();

    loop {
        let event = match next_multipart_event(&mut events).await {
            Some(event) => event,
            None => break,
        };
        match event {
            Err(error) => return Err(multipart_rejection(&error)),
            Ok(MultipartEvent::PartBegin(headers)) => {
                stage = match headers.name.as_str() {
                    "title" => {
                        if seen_title {
                            return Err(schema_violation(format!(
                                "duplicate single-valued part `{}`",
                                headers.name
                            )));
                        }
                        seen_title = true;
                        buffer.clear();
                        Stage::Title
                    }
                    "kind" => {
                        if seen_kind {
                            return Err(schema_violation(format!(
                                "duplicate single-valued part `{}`",
                                headers.name
                            )));
                        }
                        seen_kind = true;
                        buffer.clear();
                        Stage::Kind
                    }
                    "attachment" => {
                        let mut pending_required: Vec<&'static str> = Vec::new();
                        if title.is_none() {
                            pending_required.push("title");
                        }
                        if kind.is_none() {
                            pending_required.push("kind");
                        }
                        let mut seen_single_valued: Vec<String> = Vec::new();
                        if seen_title {
                            seen_single_valued.push("title".to_owned());
                        }
                        if seen_kind {
                            seen_single_valued.push("kind".to_owned());
                        }
                        seen_single_valued.push("attachment".to_owned());
                        attachment_part = Some(UploadAttachmentAttachmentPart {
                            file_name: headers.filename,
                            content_type: headers.content_type,
                            trailing_parts: UploadAttachmentTrailingParts::default(),
                            events,
                            log: ::std::sync::Arc::clone(&unknown_log),
                            stage: UploadAttachmentAttachmentPartTailStage::Idle,
                            scalar_limit: limits.multipart_scalar_part_bytes,
                            buffer: Vec::new(),
                            finished: false,
                            pending_required,
                            seen_single_valued,
                        });
                        break;
                    }
                    other => {
                        multipart_record_unknown(&unknown_log, other);
                        Stage::Drain
                    }
                };
            }
            Ok(MultipartEvent::PartChunk(chunk)) => match stage {
                Stage::Idle | Stage::Drain => {}
                _ => {
                    buffer.extend_from_slice(&chunk);
                    if buffer.len() > limits.multipart_scalar_part_bytes {
                        return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));
                    }
                }
            },
            Ok(MultipartEvent::PartEnd) => match stage {
                Stage::Idle | Stage::Drain => {}
                Stage::Title => {
                    let value = multipart_scalar_text::<String>("title", &buffer)?;
                    title = Some(value);
                    stage = Stage::Idle;
                }
                Stage::Kind => {
                    let value = multipart_scalar_text::<String>("kind", &buffer)?;
                    require_valid_request("part `kind`", validate_slug_request(&value))?;
                    kind = Some(value);
                    stage = Stage::Idle;
                }
            },
        }
    }
    let handed_off = attachment_part.is_some();
    let title = match title {
        Some(value) => Some(value),
        None if handed_off => None,
        None => {
            return Err(schema_violation("missing required part `title`"));
        }
    };
    let kind = match kind {
        Some(value) => Some(value),
        None if handed_off => None,
        None => {
            return Err(schema_violation("missing required part `kind`"));
        }
    };

    Ok(UploadAttachmentMultipartInput {
        title,
        kind,
        attachment: attachment_part,
        unknown_log,
    })
}

/// Builds the Axum router serving every documented operation (main spec §38): buffered-body routes install `DefaultBodyLimit` at `structured_request_bytes`; streaming-body routes remain exempt because nothing aggregates them.
pub fn router(
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
    _stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
) -> ::axum::Router {
    let state = ServerState {
        api,
        limits,
        encode_overflow_hook,
    };
    ::axum::Router::new()
        .route(
            "/tickets",
            ::axum::routing::post(route_create_ticket).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route(
            "/slugs",
            ::axum::routing::post(route_register_slug).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route(
            "/notes",
            ::axum::routing::post(route_post_note).layer(::axum::extract::DefaultBodyLimit::max(
                limits.structured_request_bytes,
            )),
        )
        .route(
            "/feedback",
            ::axum::routing::post(route_send_feedback).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route("/uploads", ::axum::routing::post(route_upload_attachment))
        .with_state(state)
}

/// §39 mapping row 2: syntactically malformed framing → 400; empty  bodies on required-body operations count as missing (§28.3).
fn malformed_body(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::MalformedBody).with_detail(detail)
}

/// Missing, unparsable, wildcard, or unmatched Content-Type on a  body-bearing request → 415 (§28.2, §28.5, §39 table).
fn unsupported_media_type(
    detail: impl Into<::std::borrow::Cow<'static, str>>,
) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::UnsupportedMediaType).with_detail(detail)
}

/// Reads exactly one parsable `Content-Type` header (§28 steps 1–2, §28.1): duplicate headers are ambiguous, a missing header yields `Ok(None)`, and malformed values are never ignored or defaulted.
fn parse_single_content_type(
    headers: &::http::HeaderMap,
) -> Result<Option<ParsedMediaType>, ProtocolRejection> {
    let mut lines = headers.get_all(::http::header::CONTENT_TYPE).iter();
    let Some(first) = lines.next() else {
        return Ok(None);
    };
    if lines.next().is_some() {
        return Err(malformed_body("duplicate Content-Type headers"));
    };
    let text = first
        .to_str()
        .map_err(|_| malformed_body("Content-Type is not valid UTF-8"))?;
    parse_content_type(text)
        .map(Some)
        .map_err(|_| malformed_body("malformed Content-Type"))
}

/// Outcome of matching the incoming Content-Type against the documented request entries (§28 precedence list).
enum RequestEntryMatch {
    /// No usable Content-Type header arrived.
    AbsentContentType,
    /// A parsable Content-Type matched no documented entry.
    Unmatched,
    /// Index into the operation's documented entries.
    Entry(usize),
}

/// Ranks Exact beats suffix family beats range beats wildcard (§28); a wildcard INCOMING type never selects among multiple entries (§28.5) unless exactly one entry exists.
fn classify_request_entry(parsed: Option<&ParsedMediaType>, entries: &[&str]) -> RequestEntryMatch {
    let Some(parsed) = parsed else {
        return RequestEntryMatch::AbsentContentType;
    };
    match best_request_entry(parsed, entries) {
        Some(index) => RequestEntryMatch::Entry(index),
        None => RequestEntryMatch::Unmatched,
    }
}

/// Best documented entry for one parsed incoming type; ties resolve to the earliest declaration position.
#[must_use]
fn best_request_entry(parsed: &ParsedMediaType, entries: &[&str]) -> Option<usize> {
    if is_wildcard_incoming(parsed) {
        return if entries.len() == 1 { Some(0) } else { None };
    }
    let mut best: Option<(u8, usize)> = None;
    for (index, entry) in entries.iter().enumerate() {
        if let Some(matched) = match_entry(parsed, entry) {
            let rank = negotiation_rank(matched);
            if best.is_none_or(|(seen, _)| rank < seen) {
                best = Some((rank, index));
            }
        }
    }
    best.map(|(_, index)| index)
}

/// §28 dispatch ranking.
#[must_use]
fn negotiation_rank(matched: EntryMatch) -> u8 {
    match matched {
        EntryMatch::Exact => 0,
        EntryMatch::SuffixFamily => 1,
        EntryMatch::RangeMatch => 2,
        EntryMatch::Wildcard => 3,
    }
}

/// Bounded collection of an aggregated request body (§30.2, §38):  over-limit → 413, transport failure → 400.
async fn body_bytes(
    body: ::axum::body::Body,
    limit: usize,
) -> Result<::bytes::Bytes, ProtocolRejection> {
    match collect_body_limited(body.into_data_stream(), limit).await {
        Ok(bytes) => Ok(bytes),
        Err(CollectLimitedError::TooLarge { .. }) => {
            Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))
        }
        Err(CollectLimitedError::Source(_)) => Err(malformed_body("request body stream failed")),
    }
}

/// Maps bounded JSON decode failures onto §39 kinds: syntax/io →  MalformedBody 400, data errors (missing fields/types) →  SchemaViolation 422 (D-impl-runtime-validation-timing).
fn decode_json_body<T>(bytes: &[u8]) -> Result<T, ProtocolRejection>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| {
        if error.is_data() {
            ProtocolRejection::new(RejectionKind::SchemaViolation)
                .with_detail("well-formed body failed schema validation")
        } else {
            malformed_body("malformed JSON body")
        }
    })
}

/// Strict UTF-8 for bounded textual bodies (§28.4): invalid bytes  are protocol failures, never replacement characters.
fn decode_text_body(bytes: ::bytes::Bytes) -> Result<String, ProtocolRejection> {
    String::from_utf8(bytes.to_vec()).map_err(|_| malformed_body("text body is not UTF-8"))
}

fn decode_form_body<T>(bytes: &[u8], limit: usize) -> Result<T, ProtocolRejection>
where
    T: serde::de::DeserializeOwned,
{
    match decode_form_limited(bytes, limit) {
        Ok(value) => Ok(value),
        Err(::openapi_support::form::FormDecodeError::TooLarge { .. }) => {
            Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))
        }
        Err(error) => {
            if error.is_syntax() {
                Err(malformed_body("malformed form body"))
            } else {
                Err(ProtocolRejection::new(RejectionKind::SchemaViolation)
                    .with_detail("well-formed body failed schema validation"))
            }
        }
    }
}

/// Boxed multipart event stream shared between the router's single-pass validation and the live streaming part handed to the application (main spec §17).
type MultipartEvents = ::std::pin::Pin<
    Box<
        dyn ::futures_core::Stream<Item = Result<MultipartEvent, MultipartError>>
            + ::std::marker::Send,
    >,
>;

/// Polls the next framing event without extension traits.
#[allow(clippy::needless_pass_by_ref_mut)]
async fn next_multipart_event(
    events: &mut MultipartEvents,
) -> Option<Result<MultipartEvent, MultipartError>> {
    ::std::future::poll_fn(|cx| events.as_mut().poll_next(cx)).await
}

/// Observability log of unrecognized/late part names (§17.1 unknown-fields-ignore default); payloads are never retained.
#[derive(Default)]
struct MultipartUnknownLog {
    names: Vec<String>,
}

/// Records one observed name, surviving mutex poisoning.
fn multipart_record_unknown(log: &::std::sync::Mutex<MultipartUnknownLog>, name: &str) {
    match log.lock() {
        Ok(mut guard) => guard.names.push(name.to_owned()),
        Err(poisoned) => poisoned.into_inner().names.push(name.to_owned()),
    }
}

/// Maps a multipart framing failure onto the §39 mapping table: cardinality limits (part count, header budget, name lengths) are bounded-collection rejections; truncation and malformed framing are syntactic failures.
fn multipart_rejection(error: &MultipartError) -> ProtocolRejection {
    match error {
        MultipartError::TooManyParts { .. }
        | MultipartError::PartHeaderTooLarge { .. }
        | MultipartError::FieldNameTooLong { .. }
        | MultipartError::FileNameTooLong { .. } => {
            ProtocolRejection::new(RejectionKind::BodyTooLarge)
        }
        MultipartError::Truncated => {
            malformed_body("multipart stream ended before the closing boundary")
        }
        MultipartError::MalformedFraming => malformed_body("malformed multipart framing"),
    }
}

/// Well-formed multipart content failing schema validation (missing required part, duplicate single-valued part, bad scalar value) → 422 (§17.1, §39 mapping row 6).
fn schema_violation(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::SchemaViolation).with_detail(detail)
}

/// Decodes one bounded textual scalar part (§17.1): non-UTF-8 bytes are MalformedBody 400; well-formed text failing its Rust type is SchemaViolation 422.
fn multipart_scalar_text<T>(wire: &'static str, bytes: &[u8]) -> Result<T, ProtocolRejection>
where
    T: ::std::str::FromStr,
{
    let text = std::str::from_utf8(bytes)
        .map_err(|_| malformed_body("multipart part is not valid UTF-8"))?;
    text.parse()
        .map_err(|_| schema_violation(format!("part `{wire}` failed schema validation")))
}

/// Runs one companion §9 request-body/part validator after decode: a violation rejects 422 SchemaViolation outside the documented enum, with a location-prefixed diagnostic detail (§39 rows 6; details stay off the wire per rule 3).
fn require_valid_request(
    location: &str,
    validation: ::std::result::Result<(), ::openapi_support::validation::Violation>,
) -> ::std::result::Result<(), ProtocolRejection> {
    validation.map_err(|violation| {
        ProtocolRejection::new(RejectionKind::SchemaViolation).with_detail(format!(
            "request body failed schema validation at `{location}`: {violation}",
        ))
    })
}

/// §28.4 charset policy (D-impl-charset-rejection): textual media  decode as UTF-8; any other declared charset is MalformedBody 400.
fn ensure_utf8_charset(parsed: Option<&ParsedMediaType>) -> Result<(), ProtocolRejection> {
    let Some(parsed) = parsed else {
        return Ok(());
    };
    if let Some((_, value)) = parsed.parameters.iter().find(|(name, _)| name == "charset") {
        let lowered = value.to_ascii_lowercase();
        if lowered != "utf-8" && lowered != "utf8" {
            return Err(malformed_body("charset outside the UTF-8 family"));
        }
    }
    Ok(())
}

/// §34.1 steps 1–3: partial output is discarded, nothing partial  reaches the wire, and the hook carries the operation id, variant,  and limit for observability.
fn encode_overflow_fallback(
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
    limit: usize,
) -> ::axum::response::Response {
    hook.on_encode_overflow(operation_id, variant, limit);
    ::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// Bounded JSON response encoding (§34/§41); the literal keeps  distinct types such as application/problem+json separate from  application/json.
fn encode_json_limited<T>(
    status: ::http::StatusCode,
    content_type: &'static str,
    value: &T,
    limits: &BodyLimits,
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
) -> ::axum::response::Response
where
    T: serde::Serialize,
{
    let budget = limits.structured_encode_bytes;
    match serialize_json_limited(value, budget) {
        Ok(bytes) => {
            let mut response = (status, bytes).into_response();
            response.headers_mut().insert(
                ::http::header::CONTENT_TYPE,
                ::http::HeaderValue::from_static(content_type),
            );
            response
        }
        Err(error) => encode_overflow_fallback(hook, operation_id, variant, error.limit),
    }
}
