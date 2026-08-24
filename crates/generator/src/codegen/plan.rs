//! Shared operation planning consumed by BOTH emitters ([`crate::codegen::
//! client`] and the parallel server package): status precedence (main spec
//! §23–§24), variant naming (§4), media classification into Phase 1
//! representations (§5–§7, §21–§25), the companion §6 parameter matrix, and
//! the operation-wide `Accept` union (§29).
//!
//! Planning is pure and deterministic (declaration order everywhere,
//! stable sorts only); shapes Phase 1 cannot honor surface as Error
//! diagnostics instead of improvised output (stop-and-report policy).

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostics::{Diagnostic, Diagnostics, DocumentPath};
use crate::ir::document::{
    ContentEntryIr, HttpMethod, MediaClass, ParameterIr, ParameterLocation, ParameterStyle,
    RangeClass, ResponseEntryIr, ResponseStatusKey, ServerIr,
};
use crate::ir::schema::{SchemaId, SchemaKind};
use crate::normalize::composition::ResolvedKind;
use crate::normalize::naming::{self, NameStyle};
use crate::normalize::{NormalizedDocument, NormalizedOperation};

use super::validation::{analyze, Analysis};

/// Generator configuration for planning (main spec §29 configured preference
/// order hook).
#[derive(Debug, Clone)]
pub struct PlanConfig {
    /// Media-type literals listed here are ordered first in every operation's
    /// `Accept` header, in the given preference order; everything else follows
    /// document declaration order (the default when this list is empty).
    pub accept_preference: Vec<String>,
    /// Companion §9 runtime-validation policy (D-impl-runtime-validation-
    /// timing Phase 2 half): when true (the default) generated routers call
    /// the emitted `validate_request` validators on decoded server request
    /// bodies and reject via `SchemaViolation` 422. Turning it off skips the
    /// CALLS only; validators are still emitted into models.rs.
    pub server_runtime_validation: bool,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            accept_preference: Vec::new(),
            server_runtime_validation: true,
        }
    }
}

/// Planned API: every operation in document order.
#[derive(Debug, Clone)]
pub struct PlannedApi {
    pub operations: Vec<PlannedOperation>,
    /// Effective companion §9 policy copied from [`PlanConfig`] so the
    /// server emitter gates its validation calls without extra plumbing.
    pub server_runtime_validation: bool,
}

/// One operation with every codegen decision precomputed so both emitters
/// agree byte-for-byte on names, ordering, and representations.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// snake_case method name from the naming pipeline (companion §10).
    pub method: String,
    /// PascalCase operation stem used to derive nested type names (§4 table).
    pub pascal: String,
    pub http: HttpMethod,
    pub path_template: String,
    /// Raw `operationId` when declared (used only in doc comments).
    pub operation_id: Option<String>,
    pub response_enum_name: String,
    /// `<Op>RequestBody` type name when the body admits ≥2 media types.
    pub request_body_enum_name: Option<String>,
    pub request_body_required: bool,
    /// Request media entries in declaration order (empty when no body).
    pub request_contents: Vec<PlannedContent>,
    /// Explicit codes sorted ascending first, then ranges in declaration
    /// order, then `Default` LAST (main spec §23–§24).
    pub statuses: Vec<PlannedStatus>,
    /// Merged path-level + operation-level parameters, document order.
    pub parameters: Vec<PlannedParameter>,
    /// §29 deterministic union over all decodable response content literals;
    /// empty when nothing is decodable anywhere.
    pub accept_header_value: String,
    /// Effective servers per companion §8 precedence.
    pub servers: Vec<ServerIr>,
    pub deprecated: bool,
}

/// One documented response status.
#[derive(Debug, Clone)]
pub struct PlannedStatus {
    pub key: ResponseStatusKey,
    /// `Ok200`-style per main spec §4 (`Success2xx`/`Redirection3xx`/
    /// `ClientError4xx`/`ServerError5xx` for ranges, `Default` last).
    pub enum_variant: String,
    pub contents: Vec<PlannedContent>,
    /// Typed documented headers (main spec §15): wire names verbatim,
    /// declaration order; empty when none are documented.
    pub headers: Vec<PlannedResponseHeader>,
    /// 204/205/304 (§35): unit variant ignoring any documented bytes.
    pub is_no_body_status: bool,
    /// True for explicit 2xx codes and the `2XX` range; selects the bounded
    /// collection limit (structured vs. error budget, Example 1 pattern).
    pub is_success_class: bool,
}

/// One typed response header planned from a Header Object (main spec §15):
/// scalar schemas only in v1 — string→String, integer int32→i32 else i64,
/// number→f64, boolean→bool; arrays and composites stop with an Error
/// diagnostic. `rust_name` runs the snake_case naming pipeline with numeric
/// collision suffixes by declaration order (companion §10/D-§6); `wire_name`
/// stays verbatim and is validated as an RFC 9110 field name so generated
/// code can use `http::HeaderName::from_static`.
#[derive(Debug, Clone)]
pub struct PlannedResponseHeader {
    pub rust_name: String,
    pub wire_name: String,
    pub required: bool,
    pub rust_type: String,
}

/// One media-type entry of a request body or response status.
#[derive(Debug, Clone)]
pub struct PlannedContent {
    /// Subtype PascalCase per the Phase 1 rules (`problem+json` →
    /// `ProblemJson`, `octet-stream` → `OctetStream`, `text/plain` →
    /// `TextPlain`, wildcards → `Any`); collisions get numeric suffixes by
    /// declaration order.
    pub variant_name: String,
    pub media_class: MediaClass,
    /// Base type/subtype (parameters stripped) kept verbatim for `Accept`,
    /// `Content-Type`, and runtime matching.
    pub media_type_literal: String,
    /// Rust type path into `super::models` for JsonFamily; `String` for
    /// PlainText; empty for streaming classes (each emitter renders its own
    /// payload type from [`MediaClass`] or [`Self::stream`]).
    pub model_expr: String,
    pub is_wildcard: bool,
    /// Typed field plan for `multipart/form-data` request entries (§17):
    /// populated when [`MediaClass::Multipart`] is planned for a REQUEST;
    /// always `None` elsewhere (response-side multipart still
    /// stop-and-reports).
    pub multipart_spec: Option<PlannedMultipart>,
    /// Record-framing plan for SSE/NDJSON/JSON-sequence entries (§5.6–§5.8,
    /// §18): populated when [`MediaClass`] is one of the three streaming
    /// record classes; `None` elsewhere.
    pub stream: Option<PlannedStream>,
    /// How the decoded body validates (companion §9); `None` when nothing
    /// bucket-2 survives normalization for this entry. The router runs the
    /// check after a successful decode and maps failures onto
    /// SchemaViolation 422 — only when [`PlanConfig::server_runtime_validation`]
    /// is on (the default).
    pub body_validation: Option<PlannedBodyValidation>,
}

/// Wire framing of one streaming record entry (main spec §5.6–§5.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamFraming {
    /// `text/event-stream` (§5.6/§18.2).
    Sse,
    /// NDJSON aliases (§5.7/§19).
    Ndjson,
    /// `application/json-seq` (§5.8/§20).
    JsonSeq,
}

impl StreamFraming {
    /// snake_case token used in generated method names
    /// (`into_ndjson_stream`) and module paths.
    #[must_use]
    pub fn as_snake(self) -> &'static str {
        match self {
            Self::Sse => "sse",
            Self::Ndjson => "ndjson",
            Self::JsonSeq => "jsonseq",
        }
    }

    /// PascalCase token used in generated type names
    /// (`<Op>JsonSeqBody`).
    #[must_use]
    pub fn as_pascal(self) -> &'static str {
        match self {
            Self::Sse => "Sse",
            Self::Ndjson => "Ndjson",
            Self::JsonSeq => "JsonSeq",
        }
    }
}

/// Resolved item typing of one streaming record entry (main spec §18.1):
/// the item schema is the entry's schema UNLESS `x-rust-stream-item`
/// overrides it (the override wins), resolved to a models.rs type path like
/// JsonPart mapping does.
#[derive(Debug, Clone)]
pub struct PlannedStream {
    pub framing: StreamFraming,
    /// Rust type path into `super::models` of ONE streamed item.
    pub item_model_path: String,
}

impl PlannedContent {
    /// Wire framing when this entry is one of the three streaming record
    /// classes (§5.6–§5.8).
    #[must_use]
    pub fn framing(&self) -> Option<StreamFraming> {
        self.stream.as_ref().map(|stream| stream.framing)
    }
}

/// How one decoded request body validates (companion §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedBodyValidation {
    /// Composite payload with an emitted models.rs validator: call the
    /// inherent `<value>.validate_request()`.
    Inherent,
    /// Constrained scalar alias: call the free models.rs
    /// `validate_<snake>_request(&value)`.
    ScalarFn(String),
}

/// Planned field set of one `multipart/form-data` schema (main spec §17):
/// object properties in declaration order with their per-part typing,
/// cardinality, and declared per-part content type (`encoding.contentType`).
#[derive(Debug, Clone)]
pub struct PlannedMultipart {
    pub fields: Vec<PlannedMultipartField>,
}

/// One planned multipart part (main spec §17/§17.1).
#[derive(Debug, Clone)]
pub struct PlannedMultipartField {
    /// Object property name verbatim; it IS the wire part name.
    pub wire_name: String,
    /// snake_case Rust identifier from the naming pipeline (companion §10),
    /// numeric-suffixed on collisions by declaration order.
    pub rust_name: String,
    pub kind: PlannedMultipartFieldKind,
    /// Required per the object's `required` array.
    pub required: bool,
    /// Declared per-part content type from the media type's `encoding`
    /// object (`encoding.{field}.contentType`), verbatim; `None` when
    /// undeclared.
    pub content_type: Option<String>,
    /// True when the property schema is an array; the wire may then repeat
    /// the part name and values collect in arrival order (§17.1).
    pub repeated: bool,
    /// Free validator function from models.rs when the part's schema is a
    /// constrained scalar alias (companion §9); validated at decode time in
    /// the collector / tail scan.
    pub scalar_validator: Option<String>,
}

/// Per-part payload representation (§17 source mapping).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedMultipartFieldKind {
    /// Textual scalar part decoded into the carried Rust type (parameter
    /// typing rules: string→String, int32→i32 else i64, number→f64,
    /// boolean→bool).
    ScalarText(String),
    /// Bounded JSON part decoded into the named `super::models` type (or an
    /// inline-expressible JSON target such as `serde_json::Value`).
    JsonPart(String),
    /// Streaming binary part (`format: binary`); never buffered.
    BinaryPart,
}

/// One merged parameter with its Phase 1 Rust representation.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub rust_name: String,
    pub wire_name: String,
    pub location: ParameterLocation,
    pub style: ParameterStyle,
    pub explode: bool,
    pub allow_reserved: bool,
    pub required: bool,
    /// Base scalar type (`String`/`i32`/`i64`/`f64`/`bool`) or `Vec<T>`;
    /// optionality rides on [`PlannedParameter::required`] and is applied by
    /// each emitter.
    pub rust_type: String,
}

/// Plans the whole document with default configuration.
///
/// # Errors
///
/// Returns every collected diagnostic when any Error-severity shape was hit
/// (unsupported Phase 1 media classes, unrepresentable parameter schemas,
/// anonymous composite JSON schemas).
pub fn plan_api(doc: &NormalizedDocument) -> Result<PlannedApi, Vec<Diagnostic>> {
    plan_api_with_config(doc, &PlanConfig::default())
}

/// Plans the whole document with an explicit configuration (§29 preference
/// hook).
///
/// # Errors
///
/// Same contract as [`plan_api`].
pub fn plan_api_with_config(
    doc: &NormalizedDocument,
    config: &PlanConfig,
) -> Result<PlannedApi, Vec<Diagnostic>> {
    let mut diags = Diagnostics::new();
    let analysis = analyze(doc);
    let mut operations = Vec::with_capacity(doc.operations.len());
    for operation in &doc.operations {
        operations.push(plan_operation(
            doc, operation, config, &analysis, &mut diags,
        ));
    }
    diags.into_result(PlannedApi {
        operations,
        server_runtime_validation: config.server_runtime_validation,
    })
}

fn plan_operation(
    doc: &NormalizedDocument,
    operation: &NormalizedOperation,
    config: &PlanConfig,
    analysis: &Analysis,
    diags: &mut Diagnostics,
) -> PlannedOperation {
    let location = operation_location(operation);

    // Request body planning; unsupported media classes stop-and-report.
    // UrlEncodedForm requests are honored in this phase (main spec §16,
    // D-impl-forms-phase2 superseding the deferral); response-side forms and
    // the remaining Phase 2 media classes still stop-and-report.
    let mut request_contents = Vec::new();
    let mut request_body_enum_name = None;
    let mut request_body_required = false;
    if let Some(body) = &operation.request_body {
        request_body_required = body.required;
        request_contents = plan_contents(
            doc,
            &location,
            &body.content,
            diags,
            ContentSide::Request,
            analysis,
        );
        if request_contents.len() >= 2 {
            let stem = operation
                .response_enum
                .trim_end_matches("Response")
                .to_owned();
            request_body_enum_name = Some(format!("{stem}RequestBody"));
        }
    }

    // Responses: explicit ascending, then ranges in declaration order, then
    // Default LAST (main spec §23–§24). The stable sort preserves declaration
    // order inside every class.
    let mut planned_statuses: Vec<(SortKey, PlannedStatus)> = operation
        .responses
        .iter()
        .enumerate()
        .map(|(index, response)| {
            let contents = plan_contents(
                doc,
                &location,
                &response.content,
                diags,
                ContentSide::Response,
                analysis,
            );
            let headers = plan_response_headers(doc, response, operation.method, &location, diags);
            let status = PlannedStatus {
                key: response.status,
                enum_variant: variant_name(&response.status),
                headers,
                is_no_body_status: matches!(
                    response.status,
                    ResponseStatusKey::Explicit(204 | 205 | 304)
                ),
                is_success_class: is_success_class(&response.status),
                contents,
            };
            (SortKey::new(&response.status, index), status)
        })
        .collect();
    planned_statuses.sort_by_key(|(key, _)| *key);
    let statuses: Vec<PlannedStatus> = planned_statuses
        .into_iter()
        .map(|(_, status)| status)
        .collect();

    // Parameters: the merged list already carries companion §6 overrides.
    let parameters = plan_parameters(doc, operation, &location, diags);

    // §29 Accept: deterministic union over every decodable response content
    // across ALL statuses; no-body statuses (§35) and HEAD decode nothing.
    let mut candidates: Vec<String> = Vec::new();
    for status in &statuses {
        if status.is_no_body_status || operation.method == HttpMethod::Head {
            continue;
        }
        for content in &status.contents {
            if !candidates.contains(&content.media_type_literal) {
                candidates.push(content.media_type_literal.clone());
            }
        }
    }

    PlannedOperation {
        method: operation.method_name.clone(),
        pascal: naming::ident(&operation.method_name, NameStyle::Pascal),
        http: operation.method,
        path_template: operation.path_template.clone(),
        operation_id: operation.operation_id.clone(),
        response_enum_name: operation.response_enum.clone(),
        request_body_enum_name,
        request_body_required,
        request_contents,
        statuses,
        parameters,
        accept_header_value: accept_value(&candidates, config),
        servers: operation.effective_servers.clone(),
        deprecated: operation.deprecated,
    }
}

/// Stable ordering: explicit codes ascending (rank 0), ranges in declaration
/// order (rank 1), `Default` last (rank 2); declaration index only breaks
/// ties inside rank 0+1 deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    class_rank: u8,
    code: u16,
    declaration_index: usize,
}

impl SortKey {
    fn new(key: &ResponseStatusKey, declaration_index: usize) -> Self {
        let (class_rank, code) = match key {
            ResponseStatusKey::Explicit(code) => (0_u8, *code),
            ResponseStatusKey::RangeClass(_) => (1_u8, 0_u16),
            ResponseStatusKey::Default => (2_u8, 0_u16),
        };
        Self {
            class_rank,
            code,
            declaration_index,
        }
    }
}

fn is_success_class(key: &ResponseStatusKey) -> bool {
    match key {
        ResponseStatusKey::Explicit(code) => (200..300).contains(code),
        ResponseStatusKey::RangeClass(RangeClass::Success2xx) => true,
        _ => false,
    }
}

/// §4 status variant naming: standard codes carry their reason phrase
/// (`Ok200`, `BadRequest400`); nonstandard codes degrade to `Status299`.
fn variant_name(key: &ResponseStatusKey) -> String {
    match key {
        ResponseStatusKey::Explicit(code) => match reason_phrase(*code) {
            Some(phrase) => format!("{phrase}{code}"),
            None => format!("Status{code}"),
        },
        ResponseStatusKey::RangeClass(range) => match range {
            RangeClass::Success2xx => "Success2xx".to_owned(),
            RangeClass::Redirection3xx => "Redirection3xx".to_owned(),
            RangeClass::ClientError4xx => "ClientError4xx".to_owned(),
            RangeClass::ServerError5xx => "ServerError5xx".to_owned(),
        },
        ResponseStatusKey::Default => "Default".to_owned(),
    }
}

/// Canonical reason phrases for standard codes (PascalCase, alphanumerics
/// only); anything absent degrades to `Status{code}` (§4). 1xx never reaches
/// planning (rejected at load time, main spec §35).
pub(crate) fn reason_phrase(code: u16) -> Option<&'static str> {
    Some(match code {
        200 => "Ok",
        201 => "Created",
        202 => "Accepted",
        203 => "NonAuthoritativeInfo",
        204 => "NoContent",
        205 => "ResetContent",
        206 => "PartialContent",
        207 => "MultiStatus",
        208 => "AlreadyReported",
        226 => "ImUsed",
        300 => "MultipleChoices",
        301 => "MovedPermanently",
        302 => "Found",
        303 => "SeeOther",
        304 => "NotModified",
        305 => "UseProxy",
        307 => "TemporaryRedirect",
        308 => "PermanentRedirect",
        400 => "BadRequest",
        401 => "Unauthorized",
        402 => "PaymentRequired",
        403 => "Forbidden",
        404 => "NotFound",
        405 => "MethodNotAllowed",
        406 => "NotAcceptable",
        407 => "ProxyAuthenticationRequired",
        408 => "RequestTimeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "LengthRequired",
        412 => "PreconditionFailed",
        413 => "PayloadTooLarge",
        414 => "UriTooLong",
        415 => "UnsupportedMediaType",
        416 => "RangeNotSatisfiable",
        417 => "ExpectationFailed",
        418 => "ImATeapot",
        421 => "MisdirectedRequest",
        422 => "UnprocessableEntity",
        423 => "Locked",
        424 => "FailedDependency",
        425 => "TooEarly",
        426 => "UpgradeRequired",
        428 => "PreconditionRequired",
        429 => "TooManyRequests",
        431 => "RequestHeaderFieldsTooLarge",
        451 => "UnavailableForLegalReasons",
        500 => "InternalServerError",
        501 => "NotImplemented",
        502 => "BadGateway",
        503 => "ServiceUnavailable",
        504 => "GatewayTimeout",
        505 => "HttpVersionNotSupported",
        506 => "VariantAlsoNegotiates",
        507 => "InsufficientStorage",
        508 => "LoopDetected",
        510 => "NotExtended",
        511 => "NetworkAuthenticationRequired",
        _ => return None,
    })
}

/// Which side of an operation a content list belongs to: UrlEncodedForm and
/// Multipart are honored for REQUESTS in this phase (main spec §16/§17,
/// superseding the D-impl-forms-phase2 deferral) while every response list
/// still stops on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentSide {
    Request,
    Response,
}

/// Plans one content list: Phase 1 media classes plus request-side
/// UrlEncodedForm and Multipart plus the three streaming record classes
/// (SSE/NDJSON/JSON-seq, §5.6–§5.8) in BOTH directions; everything else
/// stop-and-reports (response-side forms/multipart are later-phase
/// deliverables, main spec §52).
fn plan_contents(
    doc: &NormalizedDocument,
    location: &DocumentPath,
    contents: &[ContentEntryIr],
    diags: &mut Diagnostics,
    side: ContentSide,
    analysis: &Analysis,
) -> Vec<PlannedContent> {
    let mut planned = Vec::with_capacity(contents.len());
    let mut used_variants: BTreeSet<String> = BTreeSet::new();
    for entry in contents {
        let form_rejected = matches!(entry.media_class, MediaClass::UrlEncodedForm)
            && side == ContentSide::Response;
        let multipart_rejected = matches!(entry.media_class, MediaClass::Multipart)
            && (side == ContentSide::Response || entry.is_wildcard);
        if form_rejected || multipart_rejected {
            if matches!(entry.media_class, MediaClass::UrlEncodedForm) {
                diags.error(
                    location.key("content").key(entry.media_type.clone()),
                    "client_media_class_phase1",
                    format!(
                        "this phase does not generate media class {:?} for `{}` \
                         here; forms decode on requests only",
                        entry.media_class, entry.media_type
                    ),
                );
            } else {
                diags.error(
                    location.key("content").key(entry.media_type.clone()),
                    "multipart_media_class_phase1",
                    format!(
                        "this phase does not generate media class {:?} for `{}` \
                         here; multipart generates on requests with a concrete \
                         media type only",
                        entry.media_class, entry.media_type
                    ),
                );
            }
            continue;
        }
        // §44 override (D-impl-x-rust-body-stream): `x-rust-body: stream`
        // turns a bounded plain-text entry into the raw streaming family for
        // BOTH directions; the literal (and therefore runtime matching,
        // Accept, and the TextPlain variant name) stays verbatim.
        let effective_class = if entry.stream_override && entry.media_class == MediaClass::PlainText
        {
            MediaClass::Binary
        } else {
            entry.media_class
        };
        let base_literal = base_media_literal(&entry.media_type);
        let variant_base = content_variant_base(&base_literal, entry.is_wildcard);
        let variant = unique_variant(&variant_base, &mut used_variants);
        let mut multipart_spec = None;
        // §18.1: the item schema is the entry schema UNLESS the
        // `x-rust-stream-item` override is present (the override wins).
        // Resolution mirrors JsonPart mapping: named components use their
        // models.rs names; anonymous composites stop with an Error.
        let mut stream = None;
        if let Some(framing) = match effective_class {
            MediaClass::EventStream => Some(StreamFraming::Sse),
            MediaClass::Ndjson => Some(StreamFraming::Ndjson),
            MediaClass::JsonSeq => Some(StreamFraming::JsonSeq),
            _ => None,
        } {
            let item_schema = entry.stream_item_override.unwrap_or(entry.schema);
            stream = Some(PlannedStream {
                framing,
                item_model_path: json_model_expr(doc, item_schema, location, diags),
            });
        }
        let model_expr = match effective_class {
            MediaClass::JsonFamily => json_model_expr(doc, entry.schema, location, diags),
            MediaClass::UrlEncodedForm => json_model_expr(doc, entry.schema, location, diags),
            MediaClass::PlainText => "String".to_owned(),
            MediaClass::Multipart => {
                multipart_spec = plan_multipart_spec(doc, entry, location, diags, analysis);
                String::new()
            }
            // Binary/RawUnknown/stream classes; each emitter renders its own
            // payload type from [`MediaClass`] or [`PlannedContent::stream`].
            _ => String::new(),
        };
        // Constrained scalar aliases carry their models.rs free validator
        // (companion §9); named composite payloads validate through their
        // inherent `validate_request`. Anonymous shapes never validate:
        // inline scalars have no validator to call (documented leniency),
        // and anonymous composites are plan-time Errors anyway.
        let body_validation = match effective_class {
            MediaClass::JsonFamily | MediaClass::UrlEncodedForm | MediaClass::PlainText => {
                let effective = doc.resolve_alias(entry.schema);
                match analysis.scalar_alias(effective) {
                    Some(alias) => Some(PlannedBodyValidation::ScalarFn(alias.fn_name.clone())),
                    None if matches!(
                        entry.media_class,
                        MediaClass::JsonFamily | MediaClass::UrlEncodedForm
                    ) && analysis.has_validator(effective)
                        && component_name(doc, effective).is_some() =>
                    {
                        Some(PlannedBodyValidation::Inherent)
                    }
                    None => None,
                }
            }
            _ => None,
        };
        planned.push(PlannedContent {
            variant_name: variant,
            media_class: effective_class,
            media_type_literal: base_literal,
            model_expr,
            is_wildcard: entry.is_wildcard,
            multipart_spec,
            stream,
            body_validation,
        });
    }
    planned
}

// ----------------------------------------------------------------------
// Multipart request planning (main spec §17, §17.1)
// ----------------------------------------------------------------------

/// Plans the typed field set of one `multipart/form-data` schema: the top
/// schema must be an object (stop-and-report otherwise); each property maps
/// per the §17 source rules — scalars via the parameter typing table,
/// object-typed properties through their models.rs name, binary strings to
/// streaming parts — with arrays marked repeated and
/// `encoding.{field}.contentType` carried verbatim. A body may carry AT MOST
/// ONE binary part (§51.4 sequential semantics; see the diagnostic below).
fn plan_multipart_spec(
    doc: &NormalizedDocument,
    entry: &ContentEntryIr,
    location: &DocumentPath,
    diags: &mut Diagnostics,
    analysis: &Analysis,
) -> Option<PlannedMultipart> {
    let schema_location = location.key("content").key(entry.media_type.clone());
    let effective = doc.resolve_alias(entry.schema);
    let properties = match doc.resolution(effective).kind.clone() {
        ResolvedKind::MergedObject(merged) => merged.properties,
        ResolvedKind::ClosedEnum(_) => {
            diags.error(
                schema_location.key("schema"),
                "multipart_schema_unsupported",
                "multipart/form-data schemas must be objects whose properties \
                 map onto parts (main spec §17); an enum cannot",
            );
            return None;
        }
        ResolvedKind::Plain => match doc.arena.get(effective).kind.clone() {
            SchemaKind::Object { properties, .. } => properties,
            _ => {
                diags.error(
                    schema_location.key("schema"),
                    "multipart_schema_unsupported",
                    "multipart/form-data schemas must be objects whose \
                     properties map onto parts (main spec §17)",
                );
                return None;
            }
        },
        ResolvedKind::IntersectedScalar(_) | ResolvedKind::RawValueFallback(_) => {
            diags.error(
                schema_location.key("schema"),
                "multipart_schema_unsupported",
                "multipart/form-data schemas must be plain or merged objects \
                 whose properties map onto parts (main spec §17); composite \
                 fallbacks cannot become part lists",
            );
            return None;
        }
        ResolvedKind::Alias(_) => unreachable!("aliases chased by resolve_alias"),
    };

    let encoding: BTreeMap<&str, &str> = entry
        .encoding
        .iter()
        .map(|(field, content_type)| (field.as_str(), content_type.as_str()))
        .collect();
    // rust_name collisions inside one body get numeric suffixes by property
    // declaration order (companion §10 rule).
    let mut used_names: BTreeMap<String, u32> = BTreeMap::new();
    let mut fields = Vec::with_capacity(properties.len());
    for property in &properties {
        let base = naming::ident(&property.wire_name, NameStyle::Snake);
        let counter = used_names.entry(base.clone()).or_insert(0);
        *counter += 1;
        let rust_name = if *counter == 1 {
            base
        } else {
            naming::sanitize_joined(&format!("{base}_{counter}"))
        };
        let field_location = schema_location
            .key("properties")
            .key(property.wire_name.clone());
        let Some((kind, repeated)) =
            plan_multipart_field_kind(doc, property.schema.target, &field_location, diags)
        else {
            continue;
        };
        fields.push(PlannedMultipartField {
            wire_name: property.wire_name.clone(),
            rust_name,
            scalar_validator: scalar_validator_for(doc, property.schema.target, analysis),
            kind,
            required: property.required,
            content_type: encoding
                .get(property.wire_name.as_str())
                .map(|content_type| (*content_type).to_owned()),
            repeated,
        });
    }
    // One live-part slot exists on the server (§51.4 sequential semantics):
    // a SECOND binary field would reach the collector's unknown/drain arm
    // and silently discard an application-meaningful stream. Mirroring the
    // conservative stop-and-report style, this is a plan-time Error instead
    // of improvised queueing of unbounded streams.
    let binary_parts = fields
        .iter()
        .filter(|field| matches!(field.kind, PlannedMultipartFieldKind::BinaryPart))
        .count();
    if binary_parts > 1 {
        diags.error(
            schema_location.key("schema"),
            "multipart_schema_unsupported",
            "multipart/form-data bodies support at most one binary (format: \
             binary) part: parts stream sequentially behind a single \
             live-part slot (main spec §51.4), so further binary parts would \
             silently drain; merge them into one part or split the operation",
        );
        return None;
    }
    Some(PlannedMultipart { fields })
}

/// Constrained scalar alias behind one schema edge → the models.rs free
/// validator name (companion §9). Only textual scalar parts can carry it;
/// JSON parts validate through their model's inherent method.
fn scalar_validator_for(
    doc: &NormalizedDocument,
    schema: SchemaId,
    analysis: &Analysis,
) -> Option<String> {
    let effective = doc.resolve_alias(schema);
    analysis
        .scalar_alias(effective)
        .map(|alias| alias.fn_name.clone())
}

/// Maps one property schema onto its part representation, returning the kind
/// plus whether the schema is an array (`repeated`).
///
/// Scalars follow the parameter typing table; `format: binary` streams;
/// object/enum shapes resolve through their assigned models.rs name (an
/// anonymous composite stops with an Error diagnostic instead of
/// improvising one); free-form objects and unconstrained schemas decode as
/// raw JSON targets mirroring [`scalar_target`]. Nullability wraps JSON
/// parts in `Option<T>` (a textual scalar part has no null form on the
/// wire). Arrays map onto repeated parts; nested arrays stop-and-report.
fn plan_multipart_field_kind(
    doc: &NormalizedDocument,
    schema: SchemaId,
    location: &DocumentPath,
    diags: &mut Diagnostics,
) -> Option<(PlannedMultipartFieldKind, bool)> {
    let effective = doc.resolve_alias(schema);
    let nullable = doc.resolution(effective).nullable;
    let json_part = |model: &str| Some((wrap_optional(model.to_owned(), nullable), false));
    let resolved_kind = doc.resolution(effective).kind.clone();
    let (kind, repeated) = match resolved_kind {
        // Nominal definitions in models.rs.
        ResolvedKind::MergedObject(_) | ResolvedKind::ClosedEnum(_) => {
            match component_name(doc, effective) {
                Some(model) => (wrap_optional(model, nullable), false),
                None => {
                    diags.error(
                        location.clone(),
                        "client_anonymous_json_schema",
                        "multipart JSON parts reference JSON bodies through \
                         super::models; this composite schema has no models.rs \
                         type. Promote it to components/schemas",
                    );
                    return None;
                }
            }
        }
        ResolvedKind::IntersectedScalar(scalar) => (
            multipart_scalar_kind(&scalar.base_kind).unwrap_or_else(|| {
                PlannedMultipartFieldKind::JsonPart("serde_json::Value".to_owned())
            }),
            matches!(scalar.base_kind, SchemaKind::Array { .. }),
        ),
        ResolvedKind::RawValueFallback(_) => (
            wrap_optional(named_or_value(doc, effective), nullable),
            false,
        ),
        ResolvedKind::Alias(_) => unreachable!("aliases chased by resolve_alias"),
        ResolvedKind::Plain => match doc.arena.get(effective).kind.clone() {
            SchemaKind::Object { .. } | SchemaKind::Enum { .. } => {
                match component_name(doc, effective) {
                    Some(model) => (wrap_optional(model, nullable), false),
                    None => {
                        diags.error(
                            location.clone(),
                            "client_anonymous_json_schema",
                            "multipart JSON parts reference JSON bodies \
                             through super::models; an anonymous composite \
                             schema has no models.rs type. Promote it to \
                             components/schemas",
                        );
                        return None;
                    }
                }
            }
            other => match multipart_scalar_kind(&other) {
                Some(kind) => (kind, false),
                None => match other {
                    SchemaKind::Array { items } => {
                        let nested_location = location.clone();
                        let (inner, inner_repeated) =
                            plan_multipart_field_kind(doc, items.target, &nested_location, diags)?;
                        if inner_repeated {
                            diags.error(
                                nested_location,
                                "multipart_schema_unsupported",
                                "arrays of arrays have no single-part text \
                                 form; flatten the schema or use a JSON part",
                            );
                            return None;
                        }
                        (inner, true)
                    }
                    SchemaKind::Tuple { .. } => {
                        diags.error(
                            location.clone(),
                            "multipart_schema_unsupported",
                            "tuple schemas have no single-part text form; use \
                             a JSON part instead",
                        );
                        return None;
                    }
                    SchemaKind::FreeFormObject => {
                        return json_part("serde_json::Map<String, serde_json::Value>");
                    }
                    SchemaKind::AnyValue | SchemaKind::NotSupported { .. } => {
                        return json_part("serde_json::Value");
                    }
                    // Object/Enum handled above.
                    _ => unreachable!("object and enum kinds matched above"),
                },
            },
        },
    };
    if repeated && matches!(kind, PlannedMultipartFieldKind::BinaryPart) {
        // Arrays of streaming parts have no single live-part home on the
        // server (§51.4 sequential semantics) and cannot be cloned into
        // repeated reqwest parts on the client; stop-and-report instead of
        // improvising a queue of unbounded streams.
        diags.error(
            location.clone(),
            "multipart_schema_unsupported",
            "repeated (array) binary parts are not representable by this \
             phase's streaming input shape; declare a single binary part or a \
             non-binary array",
        );
        return None;
    }
    Some((kind, repeated))
}

/// Scalar/array-of-scalar part typing shared by plain and intersected
/// shapes: binary strings stream; everything else follows the parameter
/// table. `None` marks shapes needing the caller's composite handling.
fn multipart_scalar_kind(kind: &SchemaKind) -> Option<PlannedMultipartFieldKind> {
    Some(match kind {
        SchemaKind::String_ { binary: true, .. } => PlannedMultipartFieldKind::BinaryPart,
        SchemaKind::String_ { .. } => PlannedMultipartFieldKind::ScalarText("String".to_owned()),
        SchemaKind::Boolean => PlannedMultipartFieldKind::ScalarText("bool".to_owned()),
        SchemaKind::Integer { format } => {
            PlannedMultipartFieldKind::ScalarText(match format.as_deref() {
                Some("int32") => "i32".to_owned(),
                _ => "i64".to_owned(),
            })
        }
        SchemaKind::Number { .. } => PlannedMultipartFieldKind::ScalarText("f64".to_owned()),
        _ => return None,
    })
}

fn wrap_optional(model: String, nullable: bool) -> PlannedMultipartFieldKind {
    if nullable {
        PlannedMultipartFieldKind::JsonPart(format!("Option<{model}>"))
    } else {
        PlannedMultipartFieldKind::JsonPart(model)
    }
}

/// `application/problem+json;charset=utf-8` → `application/problem+json`.
fn base_media_literal(media_type: &str) -> String {
    media_type.split(';').next().unwrap_or("").trim().to_owned()
}

/// Content variant name from the subtype (§4/§25 examples): wildcards →
/// `Any`, `text/*` → `Text<Pascal>`, everything else → `<Pascal(subtype)>`.
/// Structured-suffix `+` separators join into the identifier
/// (`application/problem+json` → `ProblemJson`, per Example 18).
fn content_variant_base(base_literal: &str, is_wildcard: bool) -> String {
    if is_wildcard {
        return "Any".to_owned();
    }
    let Some((ty, subtype)) = base_literal.split_once('/') else {
        return naming::ident(&base_literal.replace('+', "-"), NameStyle::Pascal);
    };
    if ty.eq_ignore_ascii_case("text") {
        format!(
            "Text{}",
            naming::ident(&subtype.replace('+', "-"), NameStyle::Pascal)
        )
    } else {
        naming::ident(&subtype.replace('+', "-"), NameStyle::Pascal)
    }
}

/// First occurrence keeps the clean variant name; later collisions get
/// numeric suffixes by declaration order (companion §10 rule).
fn unique_variant(base: &str, used: &mut BTreeSet<String>) -> String {
    let sanitized = naming::sanitize_joined(base);
    let mut candidate = sanitized.clone();
    let mut counter = 1_u32;
    while !used.insert(candidate.clone()) {
        counter += 1;
        candidate = naming::sanitize_joined(&format!("{sanitized}_{counter}"));
    }
    candidate
}

/// §29 ordering: configured preference first, then declaration order; the
/// single-entry case stays alone and duplicates were already collapsed.
fn accept_value(candidates: &[String], config: &PlanConfig) -> String {
    if candidates.len() <= 1 {
        return candidates.first().cloned().unwrap_or_default();
    }
    let mut ordered: Vec<(usize, String)> = candidates
        .iter()
        .map(|literal| {
            let position = config
                .accept_preference
                .iter()
                .position(|preferred| preferred == literal)
                .unwrap_or(usize::MAX);
            (position, literal.clone())
        })
        .collect();
    ordered.sort_by_key(|(position, _)| *position);
    let literals: Vec<String> = ordered.into_iter().map(|(_, literal)| literal).collect();
    literals.join(", ")
}

// ----------------------------------------------------------------------
// JSON model type resolution (models.rs naming parity)
// ----------------------------------------------------------------------

/// Rust type expression for a JsonFamily content schema: named components use
/// their assigned models.rs name (`<Type>Fallback` for raw/value fallbacks);
/// anonymous composite shapes cannot be referenced by Phase 1 operation code
/// and stop with an Error diagnostic instead of improvising a divergent name.
fn json_model_expr(
    doc: &NormalizedDocument,
    schema: SchemaId,
    location: &DocumentPath,
    diags: &mut Diagnostics,
) -> String {
    let effective = doc.resolve_alias(schema);
    let expr = match doc.resolution(effective).kind.clone() {
        // Nominal definitions in models.rs (children before parents).
        ResolvedKind::MergedObject(_) | ResolvedKind::ClosedEnum(_) => {
            named_or_diagnostic(doc, effective, location, diags)
        }
        ResolvedKind::IntersectedScalar(scalar) => scalar_target(doc, &scalar.base_kind),
        // Raw/value fallbacks keep their models.rs identity when named.
        ResolvedKind::RawValueFallback(_) => named_or_value(doc, effective),
        ResolvedKind::Plain => match doc.arena.get(effective).kind.clone() {
            // Nominal shapes in models.rs: objects and enums become
            // definitions; a named free-form object is an alias.
            SchemaKind::Object { .. } | SchemaKind::Enum { .. } => {
                named_or_diagnostic(doc, effective, location, diags)
            }
            SchemaKind::FreeFormObject => component_name(doc, effective)
                .unwrap_or_else(|| "serde_json::Map<String, serde_json::Value>".to_owned()),
            other => scalar_target(doc, &other),
        },
        ResolvedKind::Alias(_) => unreachable!("aliases chased by resolve_alias"),
    };
    if doc.resolution(effective).nullable {
        format!("Option<{expr}>")
    } else {
        expr
    }
}

/// Assigned models.rs name for a component whose source is `effective`,
/// including the `<Type>Fallback` suffix decision from models.rs.
fn component_name(doc: &NormalizedDocument, effective: SchemaId) -> Option<String> {
    let schema = doc
        .schemas
        .values()
        .find(|entry| entry.source == effective)?;
    let is_fallback = match doc.resolution(effective).kind.clone() {
        ResolvedKind::RawValueFallback(_) => true,
        ResolvedKind::Plain => matches!(
            doc.arena.get(effective).kind,
            SchemaKind::AnyValue | SchemaKind::NotSupported { .. }
        ),
        _ => false,
    };
    Some(if is_fallback {
        format!("{}Fallback", schema.rust_type)
    } else {
        schema.rust_type.clone()
    })
}

fn named_or_diagnostic(
    doc: &NormalizedDocument,
    effective: SchemaId,
    location: &DocumentPath,
    diags: &mut Diagnostics,
) -> String {
    if let Some(name) = component_name(doc, effective) {
        return name;
    }
    diags.error(
        location.clone(),
        "client_anonymous_json_schema",
        "Phase 1 client/server codecs reference JSON bodies through \
         super::models; an anonymous composite schema has no models.rs type. \
         Promote it to components/schemas or inline only scalars/arrays",
    );
    "serde_json::Value".to_owned()
}

fn named_or_value(doc: &NormalizedDocument, effective: SchemaId) -> String {
    component_name(doc, effective).unwrap_or_else(|| "serde_json::Value".to_owned())
}

/// Inline-expressible targets mirroring models.rs `scalar_target`: free-form
/// objects and unconstrained schemas are raw JSON per D-§4.4.
fn scalar_target(doc: &NormalizedDocument, kind: &SchemaKind) -> String {
    match kind {
        SchemaKind::Boolean => "bool".to_owned(),
        SchemaKind::Integer { format } => match format.as_deref() {
            Some("int32") => "i32".to_owned(),
            _ => "i64".to_owned(),
        },
        SchemaKind::Number { .. } => "f64".to_owned(),
        SchemaKind::String_ { .. } => "String".to_owned(),
        SchemaKind::FreeFormObject | SchemaKind::AnyValue | SchemaKind::NotSupported { .. } => {
            "serde_json::Value".to_owned()
        }
        SchemaKind::Array { items } => {
            let element = doc.resolve_alias(items.target);
            let inner = scalar_target(doc, effective_kind(doc, element).as_ref());
            format!("Vec<{inner}>")
        }
        SchemaKind::Tuple { prefix_items, .. } => {
            let elements: Vec<String> = prefix_items
                .iter()
                .map(|edge| {
                    let item = doc.resolve_alias(edge.target);
                    scalar_target(doc, effective_kind(doc, item).as_ref())
                })
                .collect();
            format!("({})", elements.join(", "))
        }
        // Object/Enum shapes never reach here (nominal branch above).
        _ => "serde_json::Value".to_owned(),
    }
}

/// Effective [`SchemaKind`] after composition resolution: intersected scalars
/// expose their unified base kind; everything plain keeps its own.
fn effective_kind(doc: &NormalizedDocument, effective: SchemaId) -> Box<SchemaKind> {
    match doc.resolution(effective).kind.clone() {
        ResolvedKind::IntersectedScalar(scalar) => Box::new(scalar.base_kind),
        _ => Box::new(doc.arena.get(effective).kind.clone()),
    }
}

// ----------------------------------------------------------------------
// Typed response headers (main spec §15)
// ----------------------------------------------------------------------

/// Plans the typed header fields of one response (main spec §15): scalar
/// schemas map like parameters; arrays/composites stop with an Error
/// diagnostic. No-body statuses (§35) refuse documented headers outright —
/// their unit variants would silently drop contract information.
fn plan_response_headers(
    doc: &NormalizedDocument,
    response: &ResponseEntryIr,
    http_method: HttpMethod,
    location: &DocumentPath,
    diags: &mut Diagnostics,
) -> Vec<PlannedResponseHeader> {
    if response.headers.is_empty() {
        return Vec::new();
    }
    let header_location = || location.key("responses").key(status_label(response));
    if matches!(
        response.status,
        ResponseStatusKey::Explicit(204 | 205 | 304)
    ) {
        diags.error(
            header_location().key("headers"),
            "headers_on_no_body_status",
            format!(
                "status {} documents headers, but no-body statuses (204/205/304, \
                 main spec §35) keep unit variants and cannot carry them; move \
                 the headers to another status",
                crate::normalize::status_label(&response.status)
            ),
        );
        return Vec::new();
    }
    if http_method == HttpMethod::Head {
        // §35 wants HEAD decoders to surface typed headers without touching
        // the body; that dedicated decoder shape remains a later-phase
        // deliverable, so stop-and-report instead of improvising.
        diags.error(
            header_location().key("headers"),
            "headers_on_head_operation",
            "operation documents response headers on a HEAD request (main \
             spec §35); the typed-header HEAD decoder shape is a \
             later-phase deliverable"
                .to_owned(),
        );
        return Vec::new();
    }
    // rust_name collisions inside one status get numeric suffixes ordered by
    // declaration position (companion §10/D-§6).
    let mut used_names: BTreeMap<String, u32> = BTreeMap::new();
    let mut planned = Vec::with_capacity(response.headers.len());
    for (wire_name, header) in &response.headers {
        if !is_valid_field_name(wire_name) {
            diags.error(
                header_location().key("headers").key(wire_name.clone()),
                "header_wire_name_invalid",
                format!(
                    "`{wire_name}` is not a valid HTTP field name, so generated \
                     code could not address it on the wire"
                ),
            );
            continue;
        }
        let base = naming::ident(wire_name, NameStyle::Snake);
        let counter = used_names.entry(base.clone()).or_insert(0);
        *counter += 1;
        let rust_name = if *counter == 1 {
            base
        } else {
            naming::sanitize_joined(&format!("{base}_{counter}"))
        };
        let schema = doc.resolve_alias(header.schema);
        let kind = effective_kind(doc, schema);
        let Some(rust_type) = header_rust_type(&kind) else {
            diags.error(
                header_location().key("headers").key(wire_name.clone()),
                "header_schema_unsupported",
                format!(
                    "header `{wire_name}` needs a composite or array schema, which \
                     this phase's typed response headers cannot represent; use a \
                     scalar (string/integer/number/boolean)"
                ),
            );
            continue;
        };
        planned.push(PlannedResponseHeader {
            rust_name,
            wire_name: wire_name.clone(),
            required: header.required,
            rust_type,
        });
    }
    planned
}

fn status_label(response: &ResponseEntryIr) -> String {
    crate::normalize::status_label(&response.status)
}

/// RFC 9110 field-name token: ASCII alphanumerics plus `!#$%&'*+-.^_`|~`.
/// Planning rejects anything else so generated code can rely on
/// `http::HeaderName::from_static`.
fn is_valid_field_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Scalar header representation mirroring the parameter mapping minus
/// arrays: string→String, integer int32→i32 else i64, number→f64,
/// boolean→bool; every other shape stops at the caller with a diagnostic.
fn header_rust_type(kind: &SchemaKind) -> Option<String> {
    match kind {
        SchemaKind::Boolean => Some("bool".to_owned()),
        SchemaKind::Integer { format } => Some(match format.as_deref() {
            Some("int32") => "i32".to_owned(),
            _ => "i64".to_owned(),
        }),
        SchemaKind::Number { .. } => Some("f64".to_owned()),
        SchemaKind::String_ { .. } => Some("String".to_owned()),
        _ => None,
    }
}

// ----------------------------------------------------------------------
// Parameters (companion §6 matrix, Phase 1 scalar/array representations)
// ----------------------------------------------------------------------

fn plan_parameters(
    doc: &NormalizedDocument,
    operation: &NormalizedOperation,
    location: &DocumentPath,
    diags: &mut Diagnostics,
) -> Vec<PlannedParameter> {
    let mut planned = Vec::with_capacity(operation.merged_parameters.len());
    // rust_name collisions inside one operation get numeric suffixes ordered
    // by document position (companion §10).
    let mut used_names: BTreeMap<String, u32> = BTreeMap::new();
    for merged in &operation.merged_parameters {
        let parameter = &merged.parameter;
        validate_style_location(parameter, location, diags);
        let base = naming::ident(&parameter.name, NameStyle::Snake);
        let counter = used_names.entry(base.clone()).or_insert(0);
        *counter += 1;
        let rust_name = if *counter == 1 {
            base
        } else {
            naming::sanitize_joined(&format!("{base}_{counter}"))
        };
        let schema = doc.resolve_alias(parameter.schema);
        let kind = effective_kind(doc, schema);
        let rust_type = match param_rust_type(doc, &kind) {
            Some(rust_type) => rust_type,
            None => {
                diags.error(
                    location.key("parameters").key(parameter.name.clone()),
                    "client_param_schema_unsupported",
                    format!(
                        "parameter `{}` needs an object/enum/composite schema, which \
                         Phase 1 parameter serialization cannot represent; use a \
                         scalar or array of scalars",
                        parameter.name
                    ),
                );
                continue;
            }
        };
        planned.push(PlannedParameter {
            rust_name,
            wire_name: parameter.name.clone(),
            location: parameter.location,
            style: parameter.style,
            explode: parameter.explode,
            allow_reserved: parameter.allow_reserved,
            required: parameter.required,
            rust_type,
        });
    }
    planned
}

/// Companion §6 legality subset Phase 1 refuses to improvise around:
/// `deepObject` is query-only, `label`/`matrix` are path-only.
fn validate_style_location(
    parameter: &ParameterIr,
    location: &DocumentPath,
    diags: &mut Diagnostics,
) {
    let legal = match parameter.style {
        ParameterStyle::DeepObject => parameter.location == ParameterLocation::Query,
        ParameterStyle::Label | ParameterStyle::Matrix => {
            parameter.location == ParameterLocation::Path
        }
        ParameterStyle::Form
        | ParameterStyle::Simple
        | ParameterStyle::SpaceDelimited
        | ParameterStyle::PipeDelimited => true,
    };
    if !legal {
        diags.error(
            location.key("parameters").key(parameter.name.clone()),
            "client_param_style_location",
            format!(
                "style `{:?}` is not valid for location `{:?}` (companion §6)",
                parameter.style, parameter.location
            ),
        );
    }
}

/// Phase 1 parameter representation: string→String, integer int32→i32 else
/// i64, number→f64, boolean→bool, array<T>→Vec<T>.
fn param_rust_type(doc: &NormalizedDocument, kind: &SchemaKind) -> Option<String> {
    Some(match kind {
        SchemaKind::Boolean => "bool".to_owned(),
        SchemaKind::Integer { format } => match format.as_deref() {
            Some("int32") => "i32".to_owned(),
            _ => "i64".to_owned(),
        },
        SchemaKind::Number { .. } => "f64".to_owned(),
        SchemaKind::String_ { .. } => "String".to_owned(),
        SchemaKind::Array { items } => {
            let element = doc.resolve_alias(items.target);
            let inner = param_rust_type(doc, effective_kind(doc, element).as_ref())?;
            format!("Vec<{inner}>")
        }
        _ => return None,
    })
}

/// RFC 6901-ish breadcrumb for diagnostics under this operation.
fn operation_location(operation: &NormalizedOperation) -> DocumentPath {
    DocumentPath::root()
        .key("paths")
        .key(operation.path_template.clone())
        .key(operation.method.as_keyword())
}
