/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{Document, DocumentMetadata, Rejection};
use ::axum::response::IntoResponse;
use ::openapi_support::content_coding::ensure_identity_content_coding;
use ::openapi_support::encode::serialize_json_limited;
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

/// Live streaming view over binary part `file` of `upload_document` (main spec §17 Output B): `next_chunk` yields payload bytes with backpressure as they arrive; nothing aggregates the part.
/// Sequential semantics (§51.4): while this part is open the rest of the message cannot advance. Trailing parts flow through this type's tail scan instead of the pre-handler pass: declared scalar/JSON parts arriving behind the stream decode bounded onto `trailing_parts`, required wire names the clean end-of-message still lacks surface exactly one terminal SchemaViolation from `next_chunk` (§17.1 enforced on wire arrival), duplicate single-valued reopenings reject, every observed name is recorded for `unknown_part_names`, and all remaining payloads drain without buffering.
pub struct UploadDocumentFilePart {
    pub file_name: Option<String>,
    pub content_type: Option<::mime::Mime>,
    /// Scalar/JSON parts that arrived BEHIND this streaming part, decoded bounded as their boundaries closed (§38 application-owned tail); their pre-handoff siblings live on [`UploadDocumentMultipartInput`].
    pub trailing_parts: UploadDocumentTrailingParts,
    events: MultipartEvents,
    log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,
    stage: UploadDocumentFilePartTailStage,
    scalar_limit: usize,
    buffer: Vec<u8>,
    finished: bool,
    /// Required scalar/JSON wire names still unseen at the streaming handoff (§17.1): satisfied by trailing arrivals, otherwise reported once at the clean end-of-message.
    pending_required: Vec<&'static str>,
    /// Declared SINGLE-VALUED wire names already consumed anywhere before now (pre-handoff or behind the stream): any reopening violates §17.1.
    seen_single_valued: Vec<String>,
}

/// Tail-scan stages of [`UploadDocumentFilePart`] (§51.4 sequential semantics): `Idle` delivers payload chunks to the application; `Drain` discards them; the remaining stages bounded-buffer one trailing scalar/JSON part behind the stream.
#[derive(Debug, Clone, Copy)]
enum UploadDocumentFilePartTailStage {
    Idle,
    Drain,
    Metadata,
    Source,
    TagsElement,
}

/// Scalar/JSON parts observed BEHIND [`UploadDocumentFilePart`] (main spec §17 Output B): decoded bounded as their boundaries closed. Parts consumed BEFORE the streaming handoff live on [`UploadDocumentMultipartInput`]; the split mirrors wire arrival (§38 application-owned tail).
#[derive(Debug, Default)]
pub struct UploadDocumentTrailingParts {
    /// JSON part `metadata`.
    pub metadata: Option<DocumentMetadata>,
    /// Textual part `source`.
    pub source: Option<String>,
    /// Textual part `tags`; repeated parts collect in wire order.
    pub tags: Vec<String>,
}

impl UploadDocumentFilePart {
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
                        "metadata" => {
                            self.seen_single_valued.push("metadata".to_owned());
                            self.pending_required.retain(|name| *name != "metadata");
                            self.buffer.clear();
                            self.stage = UploadDocumentFilePartTailStage::Metadata;
                        }
                        "source" => {
                            self.seen_single_valued.push("source".to_owned());
                            self.pending_required.retain(|name| *name != "source");
                            self.buffer.clear();
                            self.stage = UploadDocumentFilePartTailStage::Source;
                        }
                        "tags" => {
                            self.buffer.clear();
                            self.stage = UploadDocumentFilePartTailStage::TagsElement;
                        }
                        other => {
                            multipart_record_unknown(&self.log, other);
                            self.stage = UploadDocumentFilePartTailStage::Drain;
                        }
                    }
                }
                Ok(MultipartEvent::PartChunk(chunk)) => match self.stage {
                    UploadDocumentFilePartTailStage::Idle => return Ok(Some(chunk)),
                    UploadDocumentFilePartTailStage::Drain => {}
                    UploadDocumentFilePartTailStage::Metadata
                    | UploadDocumentFilePartTailStage::Source
                    | UploadDocumentFilePartTailStage::TagsElement => {
                        self.buffer.extend_from_slice(&chunk);
                        if self.buffer.len() > self.scalar_limit {
                            return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));
                        }
                    }
                },
                Ok(MultipartEvent::PartEnd) => match self.stage {
                    UploadDocumentFilePartTailStage::Idle
                    | UploadDocumentFilePartTailStage::Drain => {}
                    UploadDocumentFilePartTailStage::Metadata => {
                        let value: DocumentMetadata = decode_json_body(&self.buffer)?;
                        self.trailing_parts.metadata = Some(value);
                        self.stage = UploadDocumentFilePartTailStage::Idle;
                    }
                    UploadDocumentFilePartTailStage::Source => {
                        let value = multipart_scalar_text::<String>("source", &self.buffer)?;
                        self.trailing_parts.source = Some(value);
                        self.stage = UploadDocumentFilePartTailStage::Idle;
                    }
                    UploadDocumentFilePartTailStage::TagsElement => {
                        let value = multipart_scalar_text::<String>("tags", &self.buffer)?;
                        self.trailing_parts.tags.push(value);
                        self.stage = UploadDocumentFilePartTailStage::Idle;
                    }
                },
            }
        }
    }
}

/// Streaming multipart input for `upload_document` (main spec §17 Output B): scalar/JSON parts were bounded-buffered and decoded during the router's single incremental pass up to the streaming handoff; binary parts stay live streams over the request body.
/// Required-part enforcement is wire-arrival-based (§17.1, §38): parts arriving BEFORE the first binary validate pre-handler in that pass; parts arriving behind the live stream decode onto its `trailing_parts`, and required names a clean end-of-message never delivers reject through the live part's terminal error instead of a pre-handler rejection.
pub struct UploadDocumentMultipartInput {
    /// Streaming binary part `file`.
    pub file: UploadDocumentFilePart,
    /// JSON part `metadata`: `Some` when it arrived before the streaming part; otherwise decoded onto the live part's `trailing_parts`.
    pub metadata: Option<DocumentMetadata>,
    /// Textual part `source`: `Some` when it arrived before the streaming part; otherwise decoded onto the live part's `trailing_parts`.
    pub source: Option<String>,
    /// Textual part `tags`; repeated parts collect in wire order.
    pub tags: Vec<String>,
    unknown_log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,
}

impl UploadDocumentMultipartInput {
    /// Wire names of every unrecognized or late-arriving part observed so far (§17.1 unknown-fields-ignore default): their payloads stream past without buffering and never reject. Names behind a streaming part appear once the application drains it through `next_chunk`.
    pub fn unknown_part_names(&self) -> Vec<String> {
        match self.unknown_log.lock() {
            Ok(guard) => guard.names.clone(),
            Err(poisoned) => poisoned.into_inner().names.clone(),
        }
    }
}

/// Payload for status 201 of `upload_document`: typed payload (main spec §15 Output B): required documented headers as plain fields, optional ones as `Option<T>`, then the body stored as a domain value for the bounded encoder.
#[derive(Debug)]
pub struct UploadDocument201 {
    /// Documented response header `Location` (required).
    pub location: String,
    pub body: Document,
}

/// Documented outcomes for `upload_document` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum UploadDocumentResponse {
    /// HTTP 201 Created.
    Created201(UploadDocument201),
    /// HTTP 409 Conflict.
    Conflict409(Rejection),
}

/// Bounded encoder for [`UploadDocumentResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl UploadDocumentResponse {
    /// Encodes the documented outcome with the configured limits.
    #[allow(clippy::vec_init_then_push)]
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(wrapper) => {
                let mut typed_headers = Vec::<(&'static str, String)>::new();
                typed_headers.push(("location", wrapper.location.clone()));
                let encoded = encode_json_limited(
                    ::http::StatusCode::CREATED,
                    "application/json",
                    &wrapper.body,
                    limits,
                    hook,
                    "uploadDocument",
                    "Created201",
                );
                write_typed_headers(
                    encoded,
                    hook,
                    "uploadDocument",
                    "Created201",
                    &typed_headers,
                )
            }
            Self::Conflict409(value) => encode_json_limited(
                ::http::StatusCode::CONFLICT,
                "application/json",
                &value,
                limits,
                hook,
                "uploadDocument",
                "Conflict409",
            ),
        }
    }
}

/// Checked payload constructors validating every convertible documented header eagerly (main spec §15/§48).
/// Checked constructor for [`UploadDocument201`] (§48).
impl UploadDocument201 {
    pub fn new(
        location: String,
        body: Document,
    ) -> Result<Self, ::openapi_support::response_headers::InvalidResponseHeader> {
        ::openapi_support::response_headers::checked_value("location", &location)?;
        Ok(Self { location, body })
    }
}

impl ::axum::response::IntoResponse for UploadDocumentResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `POST` `/documents`.
    /// Operation `uploadDocument`.
    async fn upload_document(&self, body: UploadDocumentMultipartInput) -> UploadDocumentResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
}

/// Route handler for `POST` `/documents` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_upload_document(
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
                collect_upload_document_multipart(body, parsed.as_ref(), &limits).await?;
            request_body
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.upload_document(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Runs the §38 pre-handler pipeline for the `multipart/form-data` body of `upload_document` (main spec §5.5/§17/§17.1): one incremental pass up to the streaming handoff; scalar/JSON parts buffer only up to `multipart_scalar_part_bytes`; duplicate-single-valued parts reject before the trait runs, and required names the pass never observed reject here unless they may still arrive behind the live stream — those ride its `pending_required` set (wire-arrival-based enforcement, §17.1/§38).
#[allow(clippy::missing_errors_doc)]
#[allow(clippy::too_many_lines)]
async fn collect_upload_document_multipart(
    body: ::axum::body::Body,
    parsed: Option<&ParsedMediaType>,
    limits: &BodyLimits,
) -> Result<UploadDocumentMultipartInput, ProtocolRejection> {
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
        Metadata,
        Source,
        TagsElement,
    }
    let mut stage = Stage::Idle;
    let mut file_part: Option<UploadDocumentFilePart> = None;
    let mut metadata: Option<DocumentMetadata> = None;
    let mut seen_metadata = false;
    let mut source: Option<String> = None;
    let mut seen_source = false;
    let mut tags: Vec<String> = Vec::new();
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
                    "file" => {
                        let mut pending_required: Vec<&'static str> = Vec::new();
                        if metadata.is_none() {
                            pending_required.push("metadata");
                        }
                        if source.is_none() {
                            pending_required.push("source");
                        }
                        let mut seen_single_valued: Vec<String> = Vec::new();
                        if seen_metadata {
                            seen_single_valued.push("metadata".to_owned());
                        }
                        if seen_source {
                            seen_single_valued.push("source".to_owned());
                        }
                        seen_single_valued.push("file".to_owned());
                        file_part = Some(UploadDocumentFilePart {
                            file_name: headers.filename,
                            content_type: headers.content_type,
                            trailing_parts: UploadDocumentTrailingParts::default(),
                            events,
                            log: ::std::sync::Arc::clone(&unknown_log),
                            stage: UploadDocumentFilePartTailStage::Idle,
                            scalar_limit: limits.multipart_scalar_part_bytes,
                            buffer: Vec::new(),
                            finished: false,
                            pending_required,
                            seen_single_valued,
                        });
                        break;
                    }
                    "metadata" => {
                        if seen_metadata {
                            return Err(schema_violation(format!(
                                "duplicate single-valued part `{}`",
                                headers.name
                            )));
                        }
                        seen_metadata = true;
                        buffer.clear();
                        Stage::Metadata
                    }
                    "source" => {
                        if seen_source {
                            return Err(schema_violation(format!(
                                "duplicate single-valued part `{}`",
                                headers.name
                            )));
                        }
                        seen_source = true;
                        buffer.clear();
                        Stage::Source
                    }
                    "tags" => {
                        buffer.clear();
                        Stage::TagsElement
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
                Stage::Metadata => {
                    let value: DocumentMetadata = decode_json_body(&buffer)?;
                    metadata = Some(value);
                    stage = Stage::Idle;
                }
                Stage::Source => {
                    let value = multipart_scalar_text::<String>("source", &buffer)?;
                    source = Some(value);
                    stage = Stage::Idle;
                }
                Stage::TagsElement => {
                    let value = multipart_scalar_text::<String>("tags", &buffer)?;
                    tags.push(value);
                    stage = Stage::Idle;
                }
            },
        }
    }
    let handed_off = file_part.is_some();
    let file = match file_part {
        Some(part) => part,
        None => {
            return Err(schema_violation("missing required part `file`"));
        }
    };
    let metadata = match metadata {
        Some(value) => Some(value),
        None if handed_off => None,
        None => {
            return Err(schema_violation("missing required part `metadata`"));
        }
    };
    let source = match source {
        Some(value) => Some(value),
        None if handed_off => None,
        None => {
            return Err(schema_violation("missing required part `source`"));
        }
    };

    Ok(UploadDocumentMultipartInput {
        file,
        metadata,
        source,
        tags,
        unknown_log,
    })
}

/// Builds the Axum router serving every documented operation (main spec §38): buffered-body routes install `DefaultBodyLimit` at `structured_request_bytes`; streaming-body routes remain exempt because nothing aggregates them.
pub fn router(
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
) -> ::axum::Router {
    let state = ServerState {
        api,
        limits,
        encode_overflow_hook,
    };
    ::axum::Router::new()
        .route("/documents", ::axum::routing::post(route_upload_document))
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

/// Appends typed documented response headers (main spec §15). A value that cannot become a `HeaderValue` fires the encode hook and emits the fixed empty 500 (§34.1 machinery; limit `0` is the recorded sentinel for non-size encode failures such as this one).
fn write_typed_headers(
    mut response: ::axum::response::Response,
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
    headers: &[(&'static str, String)],
) -> ::axum::response::Response {
    for (wire, value) in headers {
        match ::http::HeaderValue::try_from(value.as_str()) {
            Ok(header) => {
                response
                    .headers_mut()
                    .insert(::http::HeaderName::from_static(wire), header);
            }
            Err(_) => {
                return header_encode_failure(hook, operation_id, variant);
            }
        }
    }
    response
}

/// Fixed fallback for a documented header value that fails HTTP header conversion at encode time (main spec §48's internal error path): hook first, then the empty-bodied 500.
fn header_encode_failure(
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
) -> ::axum::response::Response {
    hook.on_encode_overflow(operation_id, variant, 0);
    ::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
