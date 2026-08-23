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

/// Generator configuration for planning (main spec §29 configured preference
/// order hook).
#[derive(Debug, Clone, Default)]
pub struct PlanConfig {
    /// Media-type literals listed here are ordered first in every operation's
    /// `Accept` header, in the given preference order; everything else follows
    /// document declaration order (the default when this list is empty).
    pub accept_preference: Vec<String>,
}

/// Planned API: every operation in document order.
#[derive(Debug, Clone)]
pub struct PlannedApi {
    pub operations: Vec<PlannedOperation>,
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
    /// payload type from [`MediaClass`]).
    pub model_expr: String,
    pub is_wildcard: bool,
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
    let mut operations = Vec::with_capacity(doc.operations.len());
    for operation in &doc.operations {
        operations.push(plan_operation(doc, operation, config, &mut diags));
    }
    diags.into_result(PlannedApi { operations })
}

fn plan_operation(
    doc: &NormalizedDocument,
    operation: &NormalizedOperation,
    config: &PlanConfig,
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
        request_contents =
            plan_contents(doc, &location, &body.content, diags, ContentSide::Request);
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

/// Which side of an operation a content list belongs to: UrlEncodedForm is
/// honored for REQUESTS in this phase (main spec §16) while every response
/// list still stops on it (D-impl-forms-phase2 scope note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentSide {
    Request,
    Response,
}

/// Plans one content list: Phase 1 media classes plus request-side
/// UrlEncodedForm; everything else stop-and-reports (Multipart/EventStream/
/// Ndjson/JsonSeq and response-side forms are later-phase deliverables,
/// DECISIONS.md D-impl-forms-phase2 and main spec §52).
fn plan_contents(
    doc: &NormalizedDocument,
    location: &DocumentPath,
    contents: &[ContentEntryIr],
    diags: &mut Diagnostics,
    side: ContentSide,
) -> Vec<PlannedContent> {
    let mut planned = Vec::with_capacity(contents.len());
    let mut used_variants: BTreeSet<String> = BTreeSet::new();
    for entry in contents {
        let form_rejected = matches!(entry.media_class, MediaClass::UrlEncodedForm)
            && side == ContentSide::Response;
        if form_rejected
            || matches!(
                entry.media_class,
                MediaClass::Multipart
                    | MediaClass::EventStream
                    | MediaClass::Ndjson
                    | MediaClass::JsonSeq
            )
        {
            diags.error(
                location.key("content").key(entry.media_type.clone()),
                "client_media_class_phase1",
                format!(
                    "this phase does not generate media class {:?} for `{}` \
                     here; forms decode on requests only, multipart, SSE, \
                     NDJSON, and JSON sequences remain later phases",
                    entry.media_class, entry.media_type
                ),
            );
            continue;
        }
        let base_literal = base_media_literal(&entry.media_type);
        let variant_base = content_variant_base(&base_literal, entry.is_wildcard);
        let variant = unique_variant(&variant_base, &mut used_variants);
        let model_expr = match entry.media_class {
            MediaClass::JsonFamily => json_model_expr(doc, entry.schema, location, diags),
            MediaClass::UrlEncodedForm => json_model_expr(doc, entry.schema, location, diags),
            MediaClass::PlainText => "String".to_owned(),
            // Binary/RawUnknown stream; each emitter renders its own payload.
            _ => String::new(),
        };
        planned.push(PlannedContent {
            variant_name: variant,
            media_class: entry.media_class,
            media_type_literal: base_literal,
            model_expr,
            is_wildcard: entry.is_wildcard,
        });
    }
    planned
}

/// `application/problem+json;charset=utf-8` → `application/problem+json`.
fn base_media_literal(media_type: &str) -> String {
    media_type.split(';').next().unwrap_or("").trim().to_owned()
}

/// Content variant name from the subtype (§4/§25 examples): wildcards →
/// `Any`, `text/*` → `Text<Pascal>`, everything else → `<Pascal(subtype)>`.
fn content_variant_base(base_literal: &str, is_wildcard: bool) -> String {
    if is_wildcard {
        return "Any".to_owned();
    }
    let Some((ty, subtype)) = base_literal.split_once('/') else {
        return naming::ident(base_literal, NameStyle::Pascal);
    };
    if ty.eq_ignore_ascii_case("text") {
        format!("Text{}", naming::ident(subtype, NameStyle::Pascal))
    } else {
        naming::ident(subtype, NameStyle::Pascal)
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
