//! Axum server/router emission (main spec §8 Output B, §9–§14, §21–§26,
//! §28, §30.4, §34.1, §35, §37 Mode A, §38, §39, §41, §48; DECISIONS.md
//! D-impl-server-mode-a, D-impl-async-trait, D-impl-typed-headers-phase2,
//! D-impl-forms-phase2, D-impl-charset-rejection, D-impl-singlefile-layout).
//!
//! Consumes the shared [`crate::codegen::plan`] plus the normalized document
//! (tag sanitation feeds the trait name) and renders ONE deterministic
//! `server.rs`: axum-native response enums mirroring the client shapes with
//! `::axum::body::Body` streaming payloads, the Mode A API trait, per-route
//! handlers running the §38 pipeline (identity-only content coding →
//! parameter decoding → the §28 Content-Type state machine → bounded body
//! collection), §39 protocol rejections kept outside the documented enums,
//! and the §41/§34.1 bounded response encoder with §48 checked range
//! constructors.
//!
//! Generated code references only `::openapi_support`, `::axum`, `::http`,
//! `::mime`, `::bytes`, `serde_json`, and `super::models`.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::document::{
    HttpMethod, MediaClass, ParameterLocation, ParameterStyle, RangeClass, ResponseStatusKey,
};
use crate::normalize::naming::{self, NameStyle};
use crate::normalize::NormalizedDocument;

use super::plan::{PlannedApi, PlannedContent, PlannedOperation, PlannedParameter, PlannedStatus};
use super::Emitter;

const RUSTFMT_MAX_WIDTH: usize = 100;

/// rustfmt's default maximum width for a call argument list.
const FN_CALL_WIDTH: usize = 60;

/// Empirical rustfmt budget for an UNBROKEN call inside a match-arm block
/// (see [`emit_call_arm_expr`]).
const ARM_BODY_CALL_BUDGET: usize = 77;

/// Renders ONE generated `server.rs` for the planned API (main spec §3,
/// D-impl-singlefile-layout).
#[must_use]
pub fn generate_server(doc: &NormalizedDocument, plan: &PlannedApi) -> String {
    let mut flags = Flags::default();
    let api_trait = trait_name(doc);

    let mut used_names = reserved_names(doc, plan, &api_trait);
    let layout = ServerLayout::new(plan, &mut used_names);
    for operation in &plan.operations {
        flags.scan_operation(operation);
    }

    let mut emitter = Emitter::new();
    emit_header(&mut emitter, doc);

    // The generated encoders and no-body arms CALL `IntoResponse::
    // into_response`, so the trait must be in scope; importing it whenever
    // any caller exists keeps the import used under `-D warnings`.
    flags.needs_into_response_trait = flags.needs_serialize_json
        || flags.needs_encode_text
        || flags.needs_stream_response
        || flags.needs_any_response
        || plan.operations.iter().any(|operation| {
            operation
                .statuses
                .iter()
                .any(|status| effective_contents(status).is_empty())
        });
    // §39 rejection constructors are only emitted with their call sites.
    flags.needs_invalid_parameter = flags.needs_param_helpers;
    flags.has_request_bodies = plan
        .operations
        .iter()
        .any(|operation| !operation.request_contents.is_empty());
    flags.needs_unsupported_media_type = flags.has_request_bodies;
    flags.needs_malformed_body = flags.needs_content_type_gate
        || flags.needs_collect_body
        || flags.needs_collect_stream
        || flags.needs_json_decode
        || flags.needs_text_decode
        || flags.needs_charset_check
        || flags.needs_cookie_decode;
    emit_imports(&mut emitter, &flags);

    let mut invalid_status_range_emitted = false;
    for (op_index, operation) in plan.operations.iter().enumerate() {
        emit_operation_types(&mut emitter, op_index, operation, &layout);
        emit_encoding_impl(
            &mut emitter,
            op_index,
            operation,
            &layout,
            &mut invalid_status_range_emitted,
        );
    }
    emit_trait(&mut emitter, plan, &layout, &api_trait);
    emit_state(&mut emitter, &api_trait);
    for (op_index, operation) in plan.operations.iter().enumerate() {
        emit_handler(&mut emitter, op_index, operation, &layout);
    }
    emit_router(&mut emitter, plan, &api_trait);
    emit_module_helpers(&mut emitter, &flags);
    emitter.finish()
}

// ----------------------------------------------------------------------
// Naming
// ----------------------------------------------------------------------

/// Names already taken in the module scope: assigned schema types (including
/// `<Type>Fallback` shapes), every response/request enum, the API trait name,
/// and the shared checked-constructor error. Generated nested names never
/// collide after numeric suffixing (companion §10).
fn reserved_names(
    doc: &NormalizedDocument,
    plan: &PlannedApi,
    api_trait: &str,
) -> BTreeSet<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    for schema in doc.schemas.values() {
        used.insert(schema.rust_type.clone());
        used.insert(format!("{}Fallback", schema.rust_type));
    }
    for (_, enum_name) in &doc.names.response_enums {
        used.insert(enum_name.clone());
    }
    for operation in &plan.operations {
        if let Some(enum_name) = &operation.request_body_enum_name {
            used.insert(enum_name.clone());
        }
    }
    used.insert(api_trait.to_owned());
    used.insert("InvalidStatusRange".to_owned());
    used
}

/// Phase 1 single-file layout: the trait name comes from the first declared
/// tag through the companion §10 sanitation pipeline, or `Api` when the
/// document declares no tags.
fn trait_name(doc: &NormalizedDocument) -> String {
    for operation in &doc.operations {
        if let Some(tag) = operation.tags.first() {
            return format!("{}Api", naming::ident(tag, NameStyle::Pascal));
        }
    }
    "Api".to_owned()
}

/// Deterministic collision-free type name in the server module scope.
fn fresh_name(used: &mut BTreeSet<String>, base: String) -> String {
    let sanitized = naming::sanitize_joined(&base);
    let mut candidate = sanitized.clone();
    let mut counter = 1_u32;
    while !used.insert(candidate.clone()) {
        counter += 1;
        candidate = naming::sanitize_joined(&format!("{sanitized}_{counter}"));
    }
    candidate
}

/// Wrapper payload shape of a single-content streaming status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapperShape {
    /// Binary/raw entry: the wrapper carries the raw body (main spec §32).
    Stream,
    /// Wildcard entry: the application supplies the concrete Content-Type
    /// because `*/*` is not one (main spec §22).
    Wildcard,
}

/// Module-level type-name registry for one API: content enums, streaming
/// wrappers, and wildcard single-request structs get `<Op><Status>[Content]`
/// names with numeric collision suffixes ordered by document position
/// (companion §10).
#[derive(Debug, Default)]
struct ServerLayout {
    /// (operation index, status index) → generated content-enum name.
    content_enums: BTreeMap<(usize, usize), String>,
    /// (operation index, status index) → wrapper name + shape.
    wrappers: BTreeMap<(usize, usize), (String, WrapperShape)>,
    /// Operation index → wildcard single-request struct name (§22).
    wildcard_requests: BTreeMap<usize, String>,
}

impl ServerLayout {
    fn new(plan: &PlannedApi, used: &mut BTreeSet<String>) -> Self {
        let mut layout = Self::default();
        for (op_index, operation) in plan.operations.iter().enumerate() {
            for (status_index, status) in operation.statuses.iter().enumerate() {
                if !status.is_no_body_status && status.contents.len() >= 2 {
                    let base = format!("{}{}Content", operation.pascal, status_name_part(status));
                    let name = fresh_name(used, base);
                    layout.content_enums.insert((op_index, status_index), name);
                }
                if let Some(shape) = wrapper_shape(status) {
                    let base = format!("{}{}", operation.pascal, status_name_part(status));
                    let name = fresh_name(used, base);
                    layout
                        .wrappers
                        .insert((op_index, status_index), (name, shape));
                }
            }
            if let [content] = operation.request_contents.as_slice() {
                if content.is_wildcard {
                    let base = format!("{}RequestBody", operation.pascal);
                    let name = fresh_name(used, base);
                    layout.wildcard_requests.insert(op_index, name);
                }
            }
        }
        layout
    }

    fn content_enum(&self, op_index: usize, status_index: usize) -> Option<&str> {
        self.content_enums
            .get(&(op_index, status_index))
            .map(String::as_str)
    }

    fn wildcard_request(&self, op_index: usize) -> Option<&str> {
        self.wildcard_requests.get(&op_index).map(String::as_str)
    }

    fn wrapper(&self, op_index: usize, status_index: usize) -> Option<&(String, WrapperShape)> {
        self.wrappers.get(&(op_index, status_index))
    }
}

/// Single-content statuses whose payload streams own a wrapper struct:
/// binary/raw carry the raw body; wildcards carry the application-supplied
/// Content-Type beside it. No-body statuses never wrap (§35).
fn wrapper_shape(status: &PlannedStatus) -> Option<WrapperShape> {
    if status.is_no_body_status {
        return None;
    }
    let [content] = status.contents.as_slice() else {
        return None;
    };
    if content.is_wildcard {
        return Some(WrapperShape::Wildcard);
    }
    match content.media_class {
        MediaClass::Binary | MediaClass::RawUnknown => Some(WrapperShape::Stream),
        _ => None,
    }
}

/// Status portion of derived type names per the §4 table (`GetObject200`,
/// `GetArtifact200Content`): explicit codes contribute their digits; ranges
/// and `default` use the full variant name.
fn status_name_part(status: &PlannedStatus) -> String {
    match status.key {
        ResponseStatusKey::Explicit(code) => code.to_string(),
        _ => status.enum_variant.clone(),
    }
}

/// Documented contents of a status with the §35 no-body rule applied:
/// 204/205/304 never expose bodies regardless of declared entries.
fn effective_contents(status: &PlannedStatus) -> &[PlannedContent] {
    if status.is_no_body_status {
        &[]
    } else {
        &status.contents
    }
}

/// True for range/default statuses, which carry the wire status (§23–§24).
fn struct_variant_status(status: &PlannedStatus) -> bool {
    !matches!(status.key, ResponseStatusKey::Explicit(_))
}

/// Structured entries bound-collect and decode; streaming/wildcard entries
/// pass the body through untouched.
fn is_decodable_content(content: &PlannedContent) -> bool {
    !content.is_wildcard
        && matches!(
            content.media_class,
            MediaClass::JsonFamily | MediaClass::PlainText
        )
}

// ----------------------------------------------------------------------
// Emission flags gathered in one deterministic scan so imports and module
// helpers contain exactly what the bodies reference.
// ----------------------------------------------------------------------

#[derive(Debug, Default)]
struct Flags {
    model_types: BTreeSet<String>,
    has_operations: bool,
    needs_into_response_trait: bool,
    needs_invalid_parameter: bool,
    needs_malformed_body: bool,
    needs_unsupported_media_type: bool,
    has_request_bodies: bool,
    needs_content_coding: bool,
    needs_content_type_gate: bool,
    needs_collect_body: bool,
    needs_collect_stream: bool,
    needs_peek: bool,
    needs_charset_check: bool,
    needs_json_decode: bool,
    needs_text_decode: bool,
    needs_serialize_json: bool,
    needs_encode_text: bool,
    needs_stream_response: bool,
    needs_any_response: bool,
    needs_mime_of: bool,
    needs_invalid_status_range: bool,
    needs_param_helpers: bool,
    needs_expect_text: bool,
    needs_expect_texts: bool,
    needs_parse_scalar: bool,
    needs_parse_texts: bool,
    needs_split_query_pairs: bool,
    needs_path_decode_simple: bool,
    needs_path_decode_shaped: bool,
    needs_query_decode_shaped: bool,
    needs_header_decode: bool,
    needs_cookie_decode: bool,
    needs_path_extractor: bool,
    needs_raw_query: bool,
    needs_headers_extractor: bool,
}

impl Flags {
    fn scan_operation(&mut self, operation: &PlannedOperation) {
        self.has_operations = true;
        let has_body = !operation.request_contents.is_empty();
        let decodable_request = operation.request_contents.iter().any(is_decodable_content);
        if has_body {
            self.needs_content_coding = true;
            self.needs_content_type_gate = true;
            if operation.request_body_required {
                if decodable_request {
                    self.needs_collect_body = true;
                } else {
                    // Pure streaming upload: the missing-Content-Type branch
                    // peeks presence to separate the §28.2 400/415 rows
                    // without buffering anything.
                    self.needs_peek = true;
                }
            } else {
                self.needs_peek = true;
                if decodable_request {
                    self.needs_collect_stream = true;
                }
            }
        }
        for content in &operation.request_contents {
            self.scan_request_content(content);
        }
        for status in &operation.statuses {
            self.scan_status(status);
        }
        for parameter in &operation.parameters {
            self.scan_parameter(parameter);
        }
    }

    fn scan_request_content(&mut self, content: &PlannedContent) {
        if content.is_wildcard {
            self.needs_mime_of = true;
            return;
        }
        match content.media_class {
            MediaClass::JsonFamily => {
                self.model_types
                    .extend(model_type_names(&content.model_expr));
                self.needs_json_decode = true;
                self.needs_charset_check = true;
            }
            MediaClass::PlainText => {
                self.needs_text_decode = true;
                self.needs_charset_check = true;
            }
            MediaClass::Binary | MediaClass::RawUnknown => {}
            // Planning rejects forms/multipart/SSE/NDJSON/JSON-seq; they are
            // Phase 2 deliverables (D-impl-forms-phase2) and never reach us.
            _ => unreachable!(
                "planner emitted Phase 2 media class {:?}",
                content.media_class
            ),
        }
    }

    fn scan_status(&mut self, status: &PlannedStatus) {
        for content in effective_contents(status) {
            if content.is_wildcard {
                self.needs_any_response = true;
                continue;
            }
            match content.media_class {
                MediaClass::JsonFamily => {
                    self.model_types
                        .extend(model_type_names(&content.model_expr));
                    self.needs_serialize_json = true;
                }
                MediaClass::PlainText => self.needs_encode_text = true,
                MediaClass::Binary | MediaClass::RawUnknown => {
                    self.needs_stream_response = true;
                }
                _ => unreachable!(
                    "planner emitted Phase 2 media class {:?}",
                    content.media_class
                ),
            }
        }
        if struct_variant_status(status) {
            self.needs_invalid_status_range = true;
        }
    }

    fn scan_parameter(&mut self, parameter: &PlannedParameter) {
        self.needs_param_helpers = true;
        match parameter.location {
            ParameterLocation::Path => {
                self.needs_path_extractor = true;
                match parameter.style {
                    ParameterStyle::Label | ParameterStyle::Matrix => {
                        self.needs_path_decode_shaped = true;
                    }
                    _ => self.needs_path_decode_simple = true,
                }
            }
            ParameterLocation::Query => {
                self.needs_split_query_pairs = true;
                self.needs_query_decode_shaped = true;
                self.needs_raw_query = true;
            }
            ParameterLocation::Header => {
                self.needs_header_decode = true;
                self.needs_headers_extractor = true;
            }
            ParameterLocation::Cookie => {
                self.needs_cookie_decode = true;
                self.needs_headers_extractor = true;
            }
        }
        if parameter.rust_type.starts_with("Vec<") {
            self.needs_expect_texts = true;
            if leaf_type(&parameter.rust_type) != "String" {
                self.needs_parse_scalar = true;
                self.needs_parse_texts = true;
            }
        } else {
            self.needs_expect_text = true;
            if leaf_type(&parameter.rust_type) != "String" {
                self.needs_parse_scalar = true;
            }
        }
    }
}

/// Leaf scalar type of a parameter representation (`Vec<T>` unwraps to `T`).
fn leaf_type(rust_type: &str) -> &str {
    rust_type
        .strip_prefix("Vec<")
        .map_or(rust_type, |inner| inner.trim_end_matches('>'))
}

/// Bare model type names referenced by an expression (`Option<T>`,
/// `Vec<T>`, tuples) for the import block; composite wrappers like
/// `serde_json::*` resolve through full paths instead. Inline scalars
/// (`String`, primitives) never live in `super::models` and are skipped.
fn model_type_names(expr: &str) -> Vec<String> {
    let cleaned = expr.replace(['&', '(', ')', ','], " ");
    let mut names = Vec::new();
    for token in cleaned.split_whitespace() {
        let mut inner = token;
        while let Some(rest) = inner
            .strip_prefix("Option<")
            .or_else(|| inner.strip_prefix("Vec<"))
        {
            inner = rest;
        }
        let inner = inner.trim_end_matches('>');
        if inner.is_empty()
            || inner.contains("::")
            || inner.starts_with(char::is_lowercase)
            || matches!(
                inner,
                "String" | "bool" | "i8" | "i16" | "i32" | "i64" | "f32" | "f64"
            )
        {
            continue;
        }
        names.push(inner.to_owned());
    }
    names
}

// ----------------------------------------------------------------------
// Module header and gated imports
// ----------------------------------------------------------------------

fn emit_header(emitter: &mut Emitter, doc: &NormalizedDocument) {
    emitter.docs(
        0,
        &[
            "Axum server generated from the OpenAPI document (main spec §8 \
             Output B)."
                .to_owned(),
            String::new(),
            "Mode A traits (§37), bounded JSON/text bodies (§34), streaming \
             raw payloads (§32), pre-handler protocol rejections outside the \
             documented enums (§39), identity-only inbound content coding \
             (§30.4), and the §28 Content-Type dispatch state machine. The \
             source document declares OpenAPI "
                .to_owned()
                + &doc.raw_version
                + ".",
            "Generated deterministically byte-for-byte (main spec §50 test \
             39); do not edit by hand."
                .to_owned(),
        ],
    );
}

/// Emits a brace import; collapses to one line when it fits within the
/// rustfmt maximum width, otherwise uses rustfmt's packed continuation form.
fn emit_brace_import(emitter: &mut Emitter, prefix: &str, items: &[&str]) {
    if items.len() == 1 {
        emitter.line(0, &format!("{prefix}{};", items[0]));
        return;
    }
    let joined = items.join(", ");
    let single = format!("{prefix}{{{joined}}};");
    if fits(0, &single) {
        emitter.line(0, &single);
        return;
    }
    emitter.line(0, &format!("{prefix}{{"));
    let packed = format!("{joined},");
    if fits(1, &packed) {
        emitter.line(1, &packed);
    } else {
        for item in items {
            emitter.line(1, &format!("{item},"));
        }
    }
    emitter.line(0, "};");
}

fn emit_imports(emitter: &mut Emitter, flags: &Flags) {
    let path_extractor = flags.needs_path_extractor;
    if !flags.model_types.is_empty() {
        let models: Vec<&str> = flags.model_types.iter().map(String::as_str).collect();
        if models.len() == 1 {
            emitter.line(0, &format!("use super::models::{};", models[0]));
        } else {
            emit_brace_import(emitter, "use super::models::", &models);
        }
    }

    if flags.needs_into_response_trait {
        emitter.line(0, "use ::axum::response::IntoResponse;");
    }
    if flags.needs_collect_body || flags.needs_collect_stream {
        emitter.line(
            0,
            "use ::openapi_support::collect::{collect_body_limited, CollectLimitedError};",
        );
    }
    if flags.needs_content_coding {
        emitter.line(
            0,
            "use ::openapi_support::content_coding::ensure_identity_content_coding;",
        );
    }
    if flags.needs_serialize_json {
        emitter.line(0, "use ::openapi_support::encode::serialize_json_limited;");
    }
    if flags.has_operations {
        emitter.line(
            0,
            "use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};",
        );
        emitter.line(0, "use ::openapi_support::limits::BodyLimits;");
        if flags.needs_content_type_gate {
            emit_brace_import(
                emitter,
                "use ::openapi_support::mediatype::",
                &[
                    "is_wildcard_incoming",
                    "match_entry",
                    "parse_content_type",
                    "EntryMatch",
                    "ParsedMediaType",
                ],
            );
        }
        emit_params_import(emitter, flags);
        if flags.needs_peek {
            emitter.line(
                0,
                "use ::openapi_support::peek::{detect_body_presence, BodyPresence};",
            );
        }
        // `RejectionKind` only appears at §39 constructor / bounded-collection
        // call sites; without any it stays unimported (unused-import hygiene).
        if flags.needs_invalid_parameter
            || flags.needs_malformed_body
            || flags.needs_unsupported_media_type
            || flags.needs_collect_body
            || flags.needs_collect_stream
            || flags.needs_json_decode
        {
            emitter.line(
                0,
                "use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};",
            );
        } else {
            emitter.line(0, "use ::openapi_support::rejection::ProtocolRejection;");
        }
        // rustfmt orders extern imports by crate name (openapi_support < std).
        if path_extractor {
            emitter.line(0, "use ::std::collections::HashMap;");
        }
    }
}

fn emit_params_import(emitter: &mut Emitter, flags: &Flags) {
    let mut items: Vec<&'static str> = Vec::new();
    if flags.needs_header_decode {
        items.push("decode_header_value");
    }
    if flags.needs_path_decode_simple {
        items.push("decode_path_segment");
    }
    if flags.needs_path_decode_shaped {
        items.push("decode_path_segment_shaped");
    }
    if flags.needs_query_decode_shaped || flags.needs_cookie_decode {
        items.push("decode_query_shaped");
    }
    if flags.needs_cookie_decode {
        items.push("parse_cookie_header");
    }
    let shaped = flags.needs_path_decode_shaped
        || flags.needs_query_decode_shaped
        || flags.needs_cookie_decode;
    if shaped {
        items.push("ParamShape");
    }
    if flags.needs_param_helpers {
        items.push("ParamSpec");
        items.push("ParamStyle");
        items.push("ParamValue");
    }
    if !items.is_empty() {
        emit_brace_import(emitter, "use ::openapi_support::params::", &items);
    }
}

// ----------------------------------------------------------------------
// Per-operation type definitions
// ----------------------------------------------------------------------

fn emit_operation_types(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
) {
    emitter.blank();
    let mut first = true;
    if let Some(enum_name) = &operation.request_body_enum_name {
        emit_request_body_enum(emitter, operation, enum_name);
        first = false;
    }
    if let Some(name) = layout.wildcard_request(op_index) {
        if !first {
            emitter.blank();
        }
        emit_wildcard_request_struct(emitter, operation, name);
        first = false;
    }
    for (status_index, status) in operation.statuses.iter().enumerate() {
        if let Some(name) = layout.content_enum(op_index, status_index) {
            if !first {
                emitter.blank();
            }
            emit_content_enum(emitter, operation, status, name);
            first = false;
        }
        if let Some((name, shape)) = layout.wrapper(op_index, status_index) {
            if !first {
                emitter.blank();
            }
            emit_wrapper(emitter, operation, status, name, *shape);
            first = false;
        }
    }
    if !first {
        emitter.blank();
    }
    emit_response_enum(emitter, op_index, operation, layout);
}

/// Payload type of one concrete media entry (§6/§7 tables, server side):
/// bounded models/`String` for structured classes and the raw axum body for
/// streaming classes (§32).
fn payload_type(content: &PlannedContent) -> String {
    match content.media_class {
        MediaClass::JsonFamily => content.model_expr.clone(),
        MediaClass::PlainText => "String".to_owned(),
        MediaClass::Binary | MediaClass::RawUnknown => "::axum::body::Body".to_owned(),
        _ => unreachable!(
            "planner emitted Phase 2 media class {:?}",
            content.media_class
        ),
    }
}

fn emit_request_body_enum(emitter: &mut Emitter, operation: &PlannedOperation, enum_name: &str) {
    emitter.docs(
        0,
        &[format!(
            "Request payloads for `{}` (main spec §12/§43): structured \
                 variants decode bounded; streaming variants attach the raw \
                 body verbatim; the wildcard variant carries the negotiated \
                 Content-Type.",
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub enum {enum_name} {{"));
    for content in &operation.request_contents {
        if content.is_wildcard {
            emit_any_struct_variant(emitter, 1, &content.variant_name);
            continue;
        }
        emitter.line(
            1,
            &format!("{}({}),", content.variant_name, payload_type(content)),
        );
    }
    emitter.line(0, "}");
}

fn emit_wildcard_request_struct(emitter: &mut Emitter, operation: &PlannedOperation, name: &str) {
    emitter.docs(
        0,
        &[format!(
            "Wildcard request payload for `{}` (main spec §22): `*/*` is not \
                 a concrete media type, so the negotiated Content-Type rides \
                 beside the raw body.",
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub struct {name} {{"));
    emitter.line(1, "pub content_type: ::mime::Mime,");
    emitter.line(1, "pub body: ::axum::body::Body,");
    emitter.line(0, "}");
}

/// The wildcard struct variant shared by request/response/content enums
/// (main spec §22 Output B).
fn emit_any_struct_variant(emitter: &mut Emitter, indent: usize, variant_name: &str) {
    emitter.line(indent, &format!("{variant_name} {{"));
    emitter.line(indent + 1, "content_type: ::mime::Mime,");
    emitter.line(indent + 1, "body: ::axum::body::Body,");
    emitter.line(indent, "},");
}

fn emit_content_enum(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    name: &str,
) {
    emitter.docs(
        0,
        &[format!(
            "Documented representations for status {} of `{}` (main spec \
                 §11): the router selects through Content-Type matching (§28).",
            crate::normalize::status_label(&status.key),
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub enum {name} {{"));
    for content in effective_contents(status) {
        if content.is_wildcard {
            emit_any_struct_variant(emitter, 1, &content.variant_name);
            continue;
        }
        emitter.line(
            1,
            &format!("{}({}),", content.variant_name, payload_type(content)),
        );
    }
    emitter.line(0, "}");
}

fn emit_wrapper(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    name: &str,
    shape: WrapperShape,
) {
    let payload_doc = match shape {
        WrapperShape::Stream => {
            "the body streams verbatim; typed documented-header fields \
             arrive in Phase 2 (D-impl-typed-headers-phase2)"
        }
        WrapperShape::Wildcard => {
            "`*/*` is not a concrete media type so the application supplies \
             the actual Content-Type (main spec §22); the body streams \
             verbatim"
        }
    };
    emitter.docs(
        0,
        &[format!(
            "Payload for status {} of `{}` (main spec §32): {}.",
            crate::normalize::status_label(&status.key),
            operation.method,
            payload_doc
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub struct {name} {{"));
    if shape == WrapperShape::Wildcard {
        emitter.line(1, "pub content_type: ::mime::Mime,");
    }
    emitter.line(1, "pub body: ::axum::body::Body,");
    emitter.line(0, "}");
}

fn emit_response_enum(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
) {
    emitter.docs(
        0,
        &[format!(
            "Documented outcomes for `{}` (main spec §8/§13): exhaustive \
                 match required; deliberately not `#[non_exhaustive]` (§47).",
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub enum {} {{", operation.response_enum_name));
    for (status_index, status) in operation.statuses.iter().enumerate() {
        emit_variant_doc(emitter, status);
        if let Some((wrapper, _shape)) = layout.wrapper(op_index, status_index) {
            emitter.line(1, &format!("{}({wrapper}),", status.enum_variant));
            continue;
        }
        let contents = effective_contents(status);
        if struct_variant_status(status) {
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emitter.line(2, "status: ::http::StatusCode,");
            match contents.len() {
                0 => {}
                1 => {
                    let content = &contents[0];
                    if content.is_wildcard {
                        emitter.line(2, "content_type: ::mime::Mime,");
                        emitter.line(2, "body: ::axum::body::Body,");
                    } else {
                        emitter.line(2, &format!("body: {},", payload_type(content)));
                    }
                }
                _ => {
                    let content_enum = layout
                        .content_enum(op_index, status_index)
                        .expect("registered");
                    emitter.line(2, &format!("body: {content_enum},"));
                }
            }
            emitter.line(1, "},");
            continue;
        }
        match contents.len() {
            0 => {
                emitter.line(1, &format!("{},", status.enum_variant));
            }
            1 => {
                emitter.line(
                    1,
                    &format!("{}({}),", status.enum_variant, payload_type(&contents[0])),
                );
            }
            _ => {
                let content_enum = layout
                    .content_enum(op_index, status_index)
                    .expect("registered");
                emitter.line(1, &format!("{}({content_enum}),", status.enum_variant));
            }
        }
    }
    emitter.line(0, "}");
}

fn emit_variant_doc(emitter: &mut Emitter, status: &PlannedStatus) {
    let line = match status.key {
        ResponseStatusKey::Explicit(code) => match super::plan::reason_phrase(code) {
            Some(phrase) => format!("HTTP {code} {phrase}."),
            None => format!("HTTP {code}."),
        },
        ResponseStatusKey::RangeClass(RangeClass::Success2xx) => {
            "Any HTTP 2XX success status.".to_owned()
        }
        ResponseStatusKey::RangeClass(RangeClass::Redirection3xx) => {
            "Any HTTP 3XX redirection status.".to_owned()
        }
        ResponseStatusKey::RangeClass(RangeClass::ClientError4xx) => {
            "Any HTTP 4XX client-error status.".to_owned()
        }
        ResponseStatusKey::RangeClass(RangeClass::ServerError5xx) => {
            "Any HTTP 5XX server-error status.".to_owned()
        }
        ResponseStatusKey::Default => "Any other status (`default`).".to_owned(),
    };
    emitter.docs(1, &[line]);
}

// ----------------------------------------------------------------------
// Bounded response encoding (main spec §8 Output B, §34.1, §41, §48)
// ----------------------------------------------------------------------

fn emit_encoding_impl(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    invalid_status_range_emitted: &mut bool,
) {
    let has_ranges = operation.statuses.iter().any(struct_variant_status);
    if has_ranges && !*invalid_status_range_emitted {
        emitter.blank();
        emit_invalid_status_range(emitter);
        *invalid_status_range_emitted = true;
    }

    let enum_name = &operation.response_enum_name;
    let op_id = operation_id_literal(operation);

    emitter.blank();
    emitter.docs(
        0,
        &[format!(
            "Bounded encoder for [`{enum_name}`] (main spec §8 Output B, \
                 §41): JSON/text serialize under `structured_encode_bytes`; \
                 overflow discards partial output, fires the hook, and emits \
                 a fixed empty 500 (§34.1). Range/default statuses validate \
                 their carried status (§48)."
        )],
    );
    emitter.line(0, &format!("impl {enum_name} {{"));
    emitter.docs(
        1,
        &["Encodes the documented outcome with the configured limits.".to_owned()],
    );
    emitter.line(1, "pub fn into_response_with_limits(");
    emitter.line(2, "self,");
    emitter.line(2, "limits: &BodyLimits,");
    emitter.line(2, "hook: &dyn EncodeOverflowHook,");
    emitter.line(1, ") -> ::axum::response::Response {");
    emitter.line(2, "match self {");
    for status_index in 0..operation.statuses.len() {
        emit_encode_arm(emitter, operation, layout, op_index, status_index, &op_id);
    }
    emitter.line(2, "}");
    emitter.line(1, "}");

    let ranges: Vec<usize> = operation
        .statuses
        .iter()
        .enumerate()
        .filter(|(_, status)| struct_variant_status(status))
        .map(|(status_index, _)| status_index)
        .collect();
    if !ranges.is_empty() {
        emitter.blank();
        emitter.docs(
            1,
            &["Checked constructors validating the carried status (main \
               spec §24/§48); the `IntoResponse` path only asserts in debug \
               builds."
                .to_owned()],
        );
        for status_index in ranges {
            emit_checked_ctor(emitter, operation, layout, op_index, status_index);
        }
    }
    emitter.line(0, "}");

    emitter.blank();
    emitter.line(
        0,
        &format!("impl ::axum::response::IntoResponse for {enum_name} {{"),
    );
    emitter.line(1, "fn into_response(self) -> ::axum::response::Response {");
    let delegate =
        "self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)";
    if fits(2, delegate) {
        emitter.line(2, delegate);
    } else {
        emitter.line(2, "self.into_response_with_limits(");
        emitter.line(3, "&BodyLimits::process_default(),");
        emitter.line(3, "&NoOpEncodeOverflowHook,");
        emitter.line(2, ")");
    }
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_invalid_status_range(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "A carried status fell outside its variant's documented range \
             (main spec §48)."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
    emitter.line(0, "pub struct InvalidStatusRange;");
    emitter.blank();
    emitter.line(0, "impl std::fmt::Display for InvalidStatusRange {");
    emitter.line(
        1,
        "fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {",
    );
    emitter.line(
        2,
        "f.write_str(\"status outside the variant's documented range\")",
    );
    emitter.line(1, "}");
    emitter.line(0, "}");
    emitter.blank();
    emitter.line(0, "impl std::error::Error for InvalidStatusRange {}");
}

/// Stable hook identifier: the raw `operationId` when declared, otherwise
/// the sanitized method name.
fn operation_id_literal(operation: &PlannedOperation) -> String {
    operation
        .operation_id
        .clone()
        .unwrap_or_else(|| operation.method.clone())
}

/// Constructor expression for an EXPLICIT status; range/default arms use the
/// carried `status` binding instead (`None`).
fn explicit_status_expr(key: ResponseStatusKey) -> Option<String> {
    match key {
        ResponseStatusKey::Explicit(code) => match status_code_const(code) {
            Some(const_name) => Some(format!("::http::StatusCode::{const_name}")),
            None => Some(format!(
                "::http::StatusCode::from_u16({code}).expect(\"valid status code\")"
            )),
        },
        _ => None,
    }
}

/// One bounded-encoder call rendered inside an arm.
struct EncodeCall {
    callee: &'static str,
    args: Vec<String>,
}

fn structured_call(
    content: &PlannedContent,
    status_arg: &str,
    access: &str,
    limits_arg: &str,
    op_id: &str,
    variant: &str,
) -> EncodeCall {
    let head = vec![
        status_arg.to_owned(),
        rust_string_literal(&content.media_type_literal),
    ];
    let tail = vec![
        limits_arg.to_owned(),
        "hook".to_owned(),
        rust_string_literal(op_id),
        rust_string_literal(variant),
    ];
    match content.media_class {
        MediaClass::JsonFamily => EncodeCall {
            callee: "encode_json_limited",
            args: [head, vec![format!("&{access}")], tail].concat(),
        },
        MediaClass::PlainText => EncodeCall {
            callee: "encode_text_limited",
            args: [head, vec![format!("&{access}")], tail].concat(),
        },
        // The caller passes the exact expression of the raw axum body; no
        // extra field access is appended here.
        MediaClass::Binary | MediaClass::RawUnknown => EncodeCall {
            callee: "stream_response",
            args: [head, vec![access.to_owned()]].concat(),
        },
        other => unreachable!("Phase 2 media class {other:?}"),
    }
}

fn any_call(status_arg: &str) -> EncodeCall {
    EncodeCall {
        callee: "any_response",
        args: vec![
            status_arg.to_owned(),
            "content_type".to_owned(),
            "body".to_owned(),
        ],
    }
}

/// Emits `PATTERN => callee(args),` following rustfmt's preference order:
/// inline when it fits; otherwise a block-wrapped UNBROKEN call when the
/// call stays within rustfmt's arm-body call budget; only then vertical
/// argument breaking.
///
/// The budget below was derived empirically from rustfmt (style edition
/// default, max_width 100): for match-arm bodies of this shape rustfmt keeps
/// an unbroken call inside a block iff the rendered call is at most 77
/// columns, INDEPENDENT of nesting depth; anything longer breaks arguments
/// vertically WITHOUT a wrapping block. The committed snapshot suite re-verifies
/// every emitted file against real rustfmt, so any drift fails loudly.
fn emit_call_arm_expr(emitter: &mut Emitter, indent: usize, pattern: &str, call: &EncodeCall) {
    let joined = call.args.join(", ");
    let inline = format!("{pattern} => {}({joined}),", call.callee);
    if fits(indent, &inline) {
        emitter.line(indent, &inline);
        return;
    }
    let block_body = format!("{}({joined})", call.callee);
    if fits(indent + 1, &block_body) && block_body.chars().count() <= ARM_BODY_CALL_BUDGET {
        emitter.line(indent, &format!("{pattern} => {{"));
        emitter.line(indent + 1, &block_body);
        emitter.line(indent, "}");
        return;
    }
    emitter.line(indent, &format!("{pattern} => {}(", call.callee));
    for arg in &call.args {
        emitter.line(indent + 1, &format!("{arg},"));
    }
    emitter.line(indent, "),");
}

/// Emits a bare `callee(args)` expression at the given indent.
fn emit_call_at(emitter: &mut Emitter, indent: usize, call: &EncodeCall) {
    let joined = call.args.join(", ");
    let inline = format!("{}({joined})", call.callee);
    if fits(indent, &inline) {
        emitter.line(indent, &inline);
        return;
    }
    emitter.line(indent, &format!("{}(", call.callee));
    for arg in &call.args {
        emitter.line(indent + 1, &format!("{arg},"));
    }
    emitter.line(indent, ")");
}

fn emit_encode_arm(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
    status_index: usize,
    op_id: &str,
) {
    let status = &operation.statuses[status_index];
    let variant = &status.enum_variant;
    let contents = effective_contents(status);

    // Range/default statuses carry the wire status (§23–§24).
    if struct_variant_status(status) {
        emit_range_default_arm(emitter, operation, layout, op_index, status_index, op_id);
        return;
    }

    let constant = explicit_status_expr(status.key).expect("explicit statuses carry constants");

    // Streaming/wildcard wrappers (§32/§22). The arm binds the wrapper value
    // as `wrapper`, so field accesses must use that binding, not the type.
    if let Some((_, shape)) = layout.wrapper(op_index, status_index) {
        let literal = rust_string_literal(&contents[0].media_type_literal);
        let call = match shape {
            WrapperShape::Stream => EncodeCall {
                callee: "stream_response",
                args: vec![constant.clone(), literal, "wrapper.body".to_owned()],
            },
            WrapperShape::Wildcard => EncodeCall {
                callee: "any_response",
                args: vec![
                    constant.clone(),
                    "wrapper.content_type".to_owned(),
                    "wrapper.body".to_owned(),
                ],
            },
        };
        emit_call_arm_expr(emitter, 3, &format!("Self::{variant}(wrapper)"), &call);
        return;
    }

    match contents.len() {
        // Unit/no-body statuses: the typed status alone (§35).
        0 => {
            let inline = format!("Self::{variant} => {constant}.into_response(),");
            if fits(3, &inline) {
                emitter.line(3, &inline);
            } else {
                emitter.line(3, &format!("Self::{variant} => {{"));
                emitter.line(4, &format!("let status = {constant};"));
                emitter.line(4, "status.into_response()");
                emitter.line(3, "}");
            }
        }
        1 => {
            // Single-content streaming statuses are wrapper statuses handled
            // above, so `value` here is either a model or the raw axum body
            // itself; no extra field access applies.
            let access = "value";
            let call = structured_call(&contents[0], &constant, access, "limits", op_id, variant);
            emit_call_arm_expr(emitter, 3, &format!("Self::{variant}(value)"), &call);
        }
        _ => {
            let content_enum = layout
                .content_enum(op_index, status_index)
                .expect("registered");
            emitter.line(3, &format!("Self::{variant}(content) => match content {{"));
            emit_nested_content_arms(
                emitter,
                content_enum,
                contents,
                &constant,
                "limits",
                op_id,
                variant,
                4,
            );
            emitter.line(3, "},");
        }
    }
}

/// Nested content-enum arms: every representation encodes with its own
/// Content-Type literal (main spec §11/§41); wildcards stream with the
/// application-supplied mime (§22).
#[allow(clippy::too_many_arguments)]
fn emit_nested_content_arms(
    emitter: &mut Emitter,
    content_enum: &str,
    contents: &[PlannedContent],
    status_arg: &str,
    limits_arg: &str,
    op_id: &str,
    variant: &str,
    indent: usize,
) {
    for content in contents {
        if content.is_wildcard {
            let pattern = format!(
                "{content_enum}::{} {{ content_type, body }}",
                content.variant_name
            );
            emit_call_arm_expr(emitter, indent, &pattern, &any_call(status_arg));
            continue;
        }
        // The arm binds the variant payload as `value`; streaming payloads
        // ARE the raw axum body, so `value` is passed through unchanged.
        let call = structured_call(content, status_arg, "value", limits_arg, op_id, variant);
        let pattern = format!("{content_enum}::{}(value)", content.variant_name);
        emit_call_arm_expr(emitter, indent, &pattern, &call);
    }
}

/// Range/default arm: debug-assert membership, then encode the carried
/// status with its documented body shape (§23–§24, §48).
fn emit_range_default_arm(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
    status_index: usize,
    op_id: &str,
) {
    let status = &operation.statuses[status_index];
    let variant = &status.enum_variant;
    let contents = effective_contents(status);

    let (assertion, message) = match status.key {
        ResponseStatusKey::RangeClass(range) => {
            let (low, high) = range_bounds_of(range);
            (
                format!("({low}..{high}).contains(&status.as_u16())"),
                format!("{variant} carries a status outside {low}..{high}"),
            )
        }
        ResponseStatusKey::Default => (
            "status.as_u16() >= 200".to_owned(),
            format!("{variant} carries an informational status"),
        ),
        ResponseStatusKey::Explicit(_) => unreachable!("handled elsewhere"),
    };

    let fields = match contents.len() {
        0 => "status",
        1 if contents[0].is_wildcard => "status, content_type, body",
        _ => "status, body",
    };
    emitter.line(3, &format!("Self::{variant} {{ {fields} }} => {{"));
    emitter.line(4, "debug_assert!(");
    emitter.line(5, &format!("{assertion},"));
    emitter.line(5, &format!("{},", rust_string_literal(&message)));
    emitter.line(4, ");");

    match contents.len() {
        0 => {
            emitter.line(4, "status.into_response()");
        }
        1 => {
            let content = &contents[0];
            let call = if content.is_wildcard {
                any_call("status")
            } else {
                structured_call(content, "status", "body", "limits", op_id, variant)
            };
            emit_call_at(emitter, 4, &call);
        }
        _ => {
            let content_enum = layout
                .content_enum(op_index, status_index)
                .expect("registered");
            emitter.line(4, "match body {");
            emit_nested_content_arms(
                emitter,
                content_enum,
                contents,
                "status",
                "limits",
                op_id,
                variant,
                5,
            );
            emitter.line(4, "}");
        }
    }
    emitter.line(3, "}");
}

/// Checked constructor validating the carried status (main spec §48).
fn emit_checked_ctor(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
    status_index: usize,
) {
    let status = &operation.statuses[status_index];
    let variant = &status.enum_variant;
    let constructor = checked_ctor_name(status);
    let contents = effective_contents(status);

    let params = match contents.len() {
        0 => vec!["status: ::http::StatusCode".to_owned()],
        _ => vec![
            "status: ::http::StatusCode".to_owned(),
            format!(
                "body: {}",
                ctor_body_type(operation, layout, op_index, status_index)
            ),
        ],
    };

    match status.key {
        ResponseStatusKey::RangeClass(range) => {
            let (low, high) = range_bounds_of(range);
            emitter.docs(
                1,
                &[format!(
                    "Checked constructor: `{variant}` accepts only statuses \
                     inside {low}..{high}."
                )],
            );
            emit_ctor_signature(emitter, constructor, &params);
            emitter.line(
                2,
                &format!("if ({low}..{high}).contains(&status.as_u16()) {{"),
            );
            emit_ctor_ok(emitter, status, 3);
            emitter.line(2, "} else {");
            emitter.line(3, "Err(InvalidStatusRange)");
            emitter.line(2, "}");
        }
        ResponseStatusKey::Default => {
            emitter.docs(
                1,
                &[format!(
                    "Checked constructor: `{variant}` accepts every status no \
                     other documented variant covers and no informational \
                     status (main spec §24/§35)."
                )],
            );
            emit_ctor_signature(emitter, constructor, &params);
            emitter.line(2, "if status.as_u16() < 200 {");
            emitter.line(3, "return Err(InvalidStatusRange);");
            emitter.line(2, "}");
            for condition in default_exclusions(operation) {
                emitter.line(2, &format!("if {condition} {{"));
                emitter.line(3, "return Err(InvalidStatusRange);");
                emitter.line(2, "}");
            }
            emitter.line(
                2,
                &format!(
                    "Ok(Self::{} {{ {} }})",
                    status.enum_variant,
                    match contents.len() {
                        0 => "status".to_owned(),
                        1 if contents[0].is_wildcard => {
                            "status, content_type, body".to_owned()
                        }
                        _ => "status, body".to_owned(),
                    }
                ),
            );
        }
        ResponseStatusKey::Explicit(_) => unreachable!("checked ctors cover ranges/default only"),
    }
    emitter.line(1, "}");
}

/// Emits the checked-constructor signature, collapsing when it fits.
fn emit_ctor_signature(emitter: &mut Emitter, constructor: &str, params: &[String]) {
    let joined = params.join(", ");
    let inline = format!("pub fn {constructor}({joined}) -> Result<Self, InvalidStatusRange> {{");
    if fits(1, &inline) {
        emitter.line(1, &inline);
    } else {
        emitter.line(1, &format!("pub fn {constructor}("));
        for param in params {
            emitter.line(2, &format!("{param},"));
        }
        emitter.line(1, ") -> Result<Self, InvalidStatusRange> {");
    }
}

/// Body type of a checked constructor parameter; multi-content ranges/default
/// ride their generated content enum.
fn ctor_body_type(
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
    status_index: usize,
) -> String {
    let contents = effective_contents(&operation.statuses[status_index]);
    match contents.len() {
        1 => {
            let content = &contents[0];
            if content.is_wildcard {
                "::mime::Mime".to_owned()
            } else {
                payload_type(content)
            }
        }
        _ => layout
            .content_enum(op_index, status_index)
            .expect("content enum registered for multi-content range/default")
            .to_owned(),
    }
}

fn emit_ctor_ok(emitter: &mut Emitter, status: &PlannedStatus, indent: usize) {
    let contents = effective_contents(status);
    let expression = match contents.len() {
        0 => format!("Ok(Self::{} {{ status }})", status.enum_variant),
        1 if contents[0].is_wildcard => {
            format!(
                "Ok(Self::{} {{ status, content_type, body }})",
                status.enum_variant
            )
        }
        _ => format!("Ok(Self::{} {{ status, body }})", status.enum_variant),
    };
    emitter.line(indent, &expression);
}

/// Checked-constructor names per the §48 example style.
fn checked_ctor_name(status: &PlannedStatus) -> &'static str {
    match status.key {
        ResponseStatusKey::RangeClass(RangeClass::Success2xx) => "success_2xx",
        ResponseStatusKey::RangeClass(RangeClass::Redirection3xx) => "redirection_3xx",
        ResponseStatusKey::RangeClass(RangeClass::ClientError4xx) => "client_error_4xx",
        ResponseStatusKey::RangeClass(RangeClass::ServerError5xx) => "server_error_5xx",
        ResponseStatusKey::Default => "default_status",
        ResponseStatusKey::Explicit(_) => unreachable!("checked ctors cover ranges/default only"),
    }
}

/// Statuses the `Default` constructor refuses because another documented
/// variant covers them (main spec §24).
fn default_exclusions(operation: &PlannedOperation) -> Vec<String> {
    let mut conditions: Vec<String> = Vec::new();
    for status in &operation.statuses {
        let condition = match status.key {
            ResponseStatusKey::Explicit(code) => Some(format!("status.as_u16() == {code}")),
            ResponseStatusKey::RangeClass(range) => {
                let (low, high) = range_bounds_of(range);
                Some(format!("({low}..{high}).contains(&status.as_u16())"))
            }
            ResponseStatusKey::Default => None,
        };
        if let Some(condition) = condition {
            if !conditions.contains(&condition) {
                conditions.push(condition);
            }
        }
    }
    conditions
}

fn range_bounds_of(range: RangeClass) -> (u16, u16) {
    match range {
        RangeClass::Success2xx => (200, 300),
        RangeClass::Redirection3xx => (300, 400),
        RangeClass::ClientError4xx => (400, 500),
        RangeClass::ServerError5xx => (500, 600),
    }
}

// ----------------------------------------------------------------------
// API trait (main spec §37 Mode A), state, handlers (§38), router
// ----------------------------------------------------------------------

fn emit_trait(emitter: &mut Emitter, plan: &PlannedApi, layout: &ServerLayout, api_trait: &str) {
    emitter.blank();
    emitter.docs(
        0,
        &[
            "Application contract implemented by the service (main spec §37 \
             Mode A): implementations translate internal failures into \
             documented variants."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[::async_trait::async_trait]");
    emitter.line(
        0,
        &format!("pub trait {api_trait}: Send + Sync + 'static {{"),
    );
    for (op_index, operation) in plan.operations.iter().enumerate() {
        if op_index > 0 {
            emitter.blank();
        }
        emit_trait_method(emitter, operation, layout, op_index);
    }
    emitter.line(0, "}");
}

fn emit_trait_method(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
) {
    let mut docs = vec![format!(
        "`{}` `{}`.",
        operation.http.as_keyword().to_ascii_uppercase(),
        operation.path_template
    )];
    if let Some(operation_id) = &operation.operation_id {
        docs.push(format!("Operation `{operation_id}`."));
    }
    emitter.docs(1, &docs);
    if operation.deprecated {
        emitter.line(1, "#[deprecated]");
    }
    let mut arguments: Vec<String> = vec!["&self".to_owned()];
    arguments.extend(trait_arguments(operation, layout, op_index));
    let args_inline = arguments.join(", ");
    let inline = format!(
        "async fn {}({args_inline}) -> {};",
        operation.method, operation.response_enum_name
    );
    if fits(1, &inline) {
        emitter.line(1, &inline);
        return;
    }
    emitter.line(1, &format!("async fn {}(", operation.method));
    for argument in &arguments {
        emitter.line(2, &format!("{argument},"));
    }
    emitter.line(1, &format!(") -> {};", operation.response_enum_name));
}

/// Trait parameters in contract order: path, query, header, cookie, body.
/// Required parameters arrive as owned values; optionality rides `Option<T>`
/// (main spec §26/§43).
fn trait_arguments(
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
) -> Vec<String> {
    let mut arguments: Vec<String> = Vec::new();
    for location in [
        ParameterLocation::Path,
        ParameterLocation::Query,
        ParameterLocation::Header,
        ParameterLocation::Cookie,
    ] {
        for parameter in operation
            .parameters
            .iter()
            .filter(|candidate| candidate.location == location)
        {
            let ty = if parameter.required {
                parameter.rust_type.clone()
            } else {
                format!("Option<{}>", parameter.rust_type)
            };
            arguments.push(format!("{}: {ty}", parameter.rust_name));
        }
    }
    if !operation.request_contents.is_empty() {
        let body_type = trait_body_type(operation, layout, op_index);
        arguments.push(format!("body: {body_type}"));
    }
    arguments
}

fn trait_body_type(operation: &PlannedOperation, layout: &ServerLayout, op_index: usize) -> String {
    let wrap = |base: String| {
        if operation.request_body_required {
            base
        } else {
            format!("Option<{base}>")
        }
    };
    if let Some(enum_name) = &operation.request_body_enum_name {
        return wrap(enum_name.clone());
    }
    let [content] = operation.request_contents.as_slice() else {
        unreachable!("single-content bodies only reach here");
    };
    if content.is_wildcard {
        let name = layout.wildcard_request(op_index).expect("registered");
        return wrap(name.to_owned());
    }
    wrap(payload_type(content))
}

fn emit_state(emitter: &mut Emitter, api_trait: &str) {
    emitter.blank();
    emitter.docs(
        0,
        &["Shared state threaded through every generated handler.".to_owned()],
    );
    emitter.line(0, "#[derive(Clone)]");
    emitter.line(0, "struct ServerState {");
    emitter.line(1, &format!("api: ::std::sync::Arc<dyn {api_trait}>,"));
    emitter.line(1, "limits: BodyLimits,");
    emitter.line(
        1,
        "encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,",
    );
    emitter.line(0, "}");
}

fn emit_handler(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
) {
    emitter.blank();
    emitter.docs(
        0,
        &[format!(
            "Route handler for `{}` `{}` (main spec §38): identity-only \
             content coding, parameter decoding, the §28 Content-Type \
             state machine, and bounded collection all run before the \
             application observes the request; every failure returns a \
             `ProtocolRejection` outside the documented enum (§39 rule 1).",
            operation.http.as_keyword().to_ascii_uppercase(),
            operation.path_template
        )],
    );

    let has_path = operation
        .parameters
        .iter()
        .any(|parameter| parameter.location == ParameterLocation::Path);
    let has_query = operation
        .parameters
        .iter()
        .any(|parameter| parameter.location == ParameterLocation::Query);
    let needs_headers = operation.parameters.iter().any(|parameter| {
        matches!(
            parameter.location,
            ParameterLocation::Header | ParameterLocation::Cookie
        )
    }) || !operation.request_contents.is_empty();
    let has_body = !operation.request_contents.is_empty();

    let mut extractors: Vec<String> =
        vec!["::axum::extract::State(__state): ::axum::extract::State<ServerState>".to_owned()];
    if has_path {
        extractors.push(
            "::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>"
                .to_owned(),
        );
    }
    if has_query {
        extractors.push("::axum::extract::RawQuery(__query): ::axum::extract::RawQuery".to_owned());
    }
    if needs_headers {
        extractors.push("__headers: ::http::HeaderMap".to_owned());
    }
    if has_body {
        extractors.push("body: ::axum::body::Body".to_owned());
    }

    emitter.line(0, &format!("async fn route_{}(", operation.method));
    for extractor in &extractors {
        emitter.line(1, &format!("{extractor},"));
    }
    emitter.line(
        0,
        ") -> Result<::axum::response::Response, ProtocolRejection> {",
    );

    if has_body {
        emitter.line(1, "ensure_identity_content_coding(&__headers)?;");
    }
    emitter.line(1, "let limits = __state.limits;");
    emitter.line(1, "let hook = __state.encode_overflow_hook.as_ref();");

    emit_handler_parameters(emitter, operation);

    if has_body {
        emit_body_acquisition(emitter, operation, layout, op_index);
    }

    emitter.line(1, "let api = __state.api.as_ref();");
    let invoke_args = handler_invoke_args(operation);
    let joined = invoke_args.join(", ");
    let inline = format!("let response = api.{}({joined}).await;", operation.method);
    if fits(1, &inline) {
        emitter.line(1, &inline);
    } else {
        emitter.line(1, "let response = api");
        emitter.line(2, &format!(".{}(", operation.method));
        for argument in &invoke_args {
            emitter.line(3, &format!("{argument},"));
        }
        emitter.line(2, ")");
        emitter.line(2, ".await;");
    }
    emitter.line(1, "Ok(response.into_response_with_limits(&limits, hook))");
    emitter.line(0, "}");
}

/// Handler-local names mirroring the trait order; the request body binds to
/// `request_body` so it can never collide with an extractor binding.
fn handler_invoke_args(operation: &PlannedOperation) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for location in [
        ParameterLocation::Path,
        ParameterLocation::Query,
        ParameterLocation::Header,
        ParameterLocation::Cookie,
    ] {
        for parameter in operation
            .parameters
            .iter()
            .filter(|candidate| candidate.location == location)
        {
            names.push(parameter.rust_name.clone());
        }
    }
    if !operation.request_contents.is_empty() {
        names.push("request_body".to_owned());
    }
    names
}

// ----------------------------------------------------------------------
// Handler parameter decoding (companion §6 via openapi_support::params)
// ----------------------------------------------------------------------

fn emit_handler_parameters(emitter: &mut Emitter, operation: &PlannedOperation) {
    let path_parameters: Vec<&PlannedParameter> = operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Path)
        .collect();
    let query_parameters: Vec<&PlannedParameter> = operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Query)
        .collect();
    let header_parameters: Vec<&PlannedParameter> = operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Header)
        .collect();
    let cookie_parameters: Vec<&PlannedParameter> = operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Cookie)
        .collect();

    for parameter in &path_parameters {
        emit_path_parameter(emitter, parameter);
    }
    if !query_parameters.is_empty() {
        emitter.line(1, "let query_pairs = match __query {");
        emitter.line(2, "Some(query) => split_query_pairs(&query),");
        emitter.line(2, "None => Vec::new(),");
        emitter.line(1, "};");
        for parameter in &query_parameters {
            emit_pairs_parameter(emitter, parameter, "query", "query_pairs");
        }
    }
    for parameter in &header_parameters {
        emit_header_parameter(emitter, parameter);
    }
    if !cookie_parameters.is_empty() {
        emitter.line(
            1,
            "let cookie_text = match __headers.get(::http::header::COOKIE) {",
        );
        emitter.line(2, "None => None,");
        emitter.line(2, "Some(raw_value) => match raw_value.to_str() {");
        emitter.line(3, "Ok(text) => Some(text),");
        emitter.line(3, "Err(_) => {");
        emitter.line(
            4,
            "return Err(invalid_parameter(\"`Cookie` header is not valid UTF-8\"));",
        );
        emitter.line(3, "}");
        emitter.line(2, "},");
        emitter.line(1, "};");
        emitter.line(1, "let cookie_pairs = match cookie_text {");
        emitter.line(2, "Some(text) => parse_cookie_header(text),");
        emitter.line(2, "None => Vec::new(),");
        emitter.line(1, "};");
        for parameter in &cookie_parameters {
            emit_pairs_parameter(emitter, parameter, "cookie", "cookie_pairs");
        }
    }
}

fn param_spec_line(parameter: &PlannedParameter) -> String {
    let style = param_style_name(parameter.style);
    format!(
        "let spec = ParamSpec::new({}, ParamStyle::{style}, {}, {});",
        rust_string_literal(&parameter.wire_name),
        parameter.explode,
        parameter.allow_reserved
    )
}

fn emit_path_parameter(emitter: &mut Emitter, parameter: &PlannedParameter) {
    let name = &parameter.wire_name;
    emitter.line(1, &param_spec_line(parameter));
    emitter.line(
        1,
        &format!(
            "let raw_segment = match __path.get({}) {{",
            rust_string_literal(name)
        ),
    );
    emitter.line(2, "Some(segment) => segment.as_str(),");
    emitter.line(2, "None => {");
    emitter.line(
        3,
        &format!(
            "return Err(invalid_parameter({}));",
            rust_string_literal(&format!("missing path parameter `{name}`"))
        ),
    );
    emitter.line(2, "}");
    emitter.line(1, "};");

    let err_literal = rust_string_literal(&format!("path parameter `{name}` is malformed"));
    let shaped = matches!(
        parameter.style,
        ParameterStyle::Label | ParameterStyle::Matrix
    );
    if shaped {
        let shape = param_shape(parameter);
        emitter.line(1, "let decoded = match decode_path_segment_shaped(");
        emitter.line(2, "&spec,");
        emitter.line(2, "raw_segment,");
        emitter.line(2, &format!("ParamShape::{shape},"));
        emitter.line(1, ") {");
    } else {
        emitter.line(
            1,
            "let decoded = match decode_path_segment(&spec, raw_segment) {",
        );
    }
    emitter.line(2, "Ok(value) => value,");
    emitter.line(2, "Err(_) => {");
    emitter.line(3, &format!("return Err(invalid_parameter({err_literal}));"));
    emitter.line(2, "}");
    emitter.line(1, "};");
    emit_typed_binding(emitter, parameter, "decoded");
}

/// Query and cookie parameters share the shaped pairs decoder; cookies are
/// form-encoded inside the `Cookie` header (companion §6).
fn emit_pairs_parameter(
    emitter: &mut Emitter,
    parameter: &PlannedParameter,
    kind: &str,
    pairs: &str,
) {
    let name = &parameter.wire_name;
    let err_literal = rust_string_literal(&format!("{kind} parameter `{name}` is malformed"));
    emitter.line(1, &param_spec_line(parameter));
    let shape = param_shape(parameter);
    emitter.line(1, "let decoded = match decode_query_shaped(");
    emitter.line(2, "&spec,");
    emitter.line(
        2,
        &format!("{pairs}.iter().map(|(key, value)| (key.as_str(), value.as_str())),"),
    );
    emitter.line(2, &format!("ParamShape::{shape},"));
    emitter.line(1, ") {");
    emitter.line(2, "Ok(value) => value,");
    emitter.line(2, "Err(_) => {");
    emitter.line(3, &format!("return Err(invalid_parameter({err_literal}));"));
    emitter.line(2, "}");
    emitter.line(1, "};");
    emit_typed_binding(emitter, parameter, "decoded");
}

fn emit_header_parameter(emitter: &mut Emitter, parameter: &PlannedParameter) {
    let name = &parameter.wire_name;
    emitter.line(1, &param_spec_line(parameter));
    emitter.line(
        1,
        &format!(
            "let header_text = match __headers.get({}) {{",
            rust_string_literal(name)
        ),
    );
    emitter.line(2, "None => None,");
    emitter.line(2, "Some(raw_value) => match raw_value.to_str() {");
    emitter.line(3, "Ok(text) => Some(text),");
    emitter.line(3, "Err(_) => {");
    emitter.line(
        4,
        &format!(
            "return Err(invalid_parameter({}));",
            rust_string_literal(&format!("header `{name}` is not valid UTF-8"))
        ),
    );
    emitter.line(3, "}");
    emitter.line(2, "},");
    emitter.line(1, "};");
    emitter.line(1, "let decoded = match header_text {");
    emitter.line(2, "None => None,");
    emitter.line(2, "Some(text) => match decode_header_value(&spec, text) {");
    emitter.line(3, "Ok(value) => value,");
    emitter.line(3, "Err(_) => {");
    emitter.line(
        4,
        &format!(
            "return Err(invalid_parameter({}));",
            rust_string_literal(&format!("header `{name}` is malformed"))
        ),
    );
    emitter.line(3, "}");
    emitter.line(2, "},");
    emitter.line(1, "};");
    emit_typed_binding(emitter, parameter, "decoded");
}

fn param_shape(parameter: &PlannedParameter) -> &'static str {
    if parameter.rust_type.starts_with("Vec<") {
        "Array"
    } else {
        "Scalar"
    }
}

fn param_style_name(style: ParameterStyle) -> &'static str {
    match style {
        ParameterStyle::Matrix => "Matrix",
        ParameterStyle::Label => "Label",
        ParameterStyle::Form => "Form",
        ParameterStyle::Simple => "Simple",
        ParameterStyle::SpaceDelimited => "SpaceDelimited",
        ParameterStyle::PipeDelimited => "PipeDelimited",
        ParameterStyle::DeepObject => "DeepObject",
    }
}

/// Converts a decoded `Option<ParamValue>` into the parameter's Rust type.
fn emit_typed_binding(emitter: &mut Emitter, parameter: &PlannedParameter, decoded: &str) {
    let name = &parameter.rust_name;
    let rust_type = parameter.rust_type.as_str();
    let wire_lit = rust_string_literal(&parameter.wire_name);

    if parameter.required {
        match rust_type {
            "String" => {
                emitter.line(
                    1,
                    &format!("let {name}: String = expect_text({decoded}, {wire_lit})?;"),
                );
            }
            leaf if leaf.starts_with("Vec<") => {
                let element = leaf_type(leaf);
                if element == "String" {
                    emitter.line(
                        1,
                        &format!("let {name}: {leaf} = expect_texts({decoded}, {wire_lit})?;"),
                    );
                } else {
                    let rhs =
                        format!("parse_texts({wire_lit}, &expect_texts({decoded}, {wire_lit})?)?");
                    let inline = format!("let {name}: {leaf} = {rhs};");
                    if fits(1, &inline) {
                        emitter.line(1, &inline);
                    } else {
                        emitter.line(1, &format!("let {name}: {leaf} ="));
                        emitter.line(2, &format!("{rhs};"));
                    }
                }
            }
            leaf => {
                let inner = format!("expect_text({decoded}, {wire_lit})?");
                emitter.line(
                    1,
                    &format!("let {name}: {leaf} = parse_param_text({wire_lit}, &{inner})?;"),
                );
            }
        }
        return;
    }

    // Optional parameters keep `Option<T>`.
    match rust_type {
        "String" => {
            emitter.line(
                1,
                &format!("let {name}: Option<String> = match {decoded} {{"),
            );
            emitter.line(2, "Some(value) => {");
            emitter.line(
                3,
                &format!("let text = expect_text(Some(value), {wire_lit})?;"),
            );
            emitter.line(3, "Some(text)");
            emitter.line(2, "}");
            emitter.line(2, "None => None,");
            emitter.line(1, "};");
        }
        leaf if leaf.starts_with("Vec<") => {
            let element = leaf_type(leaf);
            emitter.line(
                1,
                &format!("let {name}: Option<{leaf}> = match {decoded} {{"),
            );
            emitter.line(2, "Some(value) => {");
            emitter.line(
                3,
                &format!("let texts = expect_texts(Some(value), {wire_lit})?;"),
            );
            if element == "String" {
                emitter.line(3, "Some(texts)");
            } else {
                emitter.line(3, &format!("Some(parse_texts({wire_lit}, &texts)?)"));
            }
            emitter.line(2, "}");
            emitter.line(2, "None => None,");
            emitter.line(1, "};");
        }
        leaf => {
            emitter.line(
                1,
                &format!("let {name}: Option<{leaf}> = match {decoded} {{"),
            );
            emitter.line(2, "Some(value) => {");
            emitter.line(
                3,
                &format!("let text = expect_text(Some(value), {wire_lit})?;"),
            );
            emitter.line(3, &format!("Some(parse_param_text({wire_lit}, &text)?)"));
            emitter.line(2, "}");
            emitter.line(2, "None => None,");
            emitter.line(1, "};");
        }
    }
}

// ----------------------------------------------------------------------
// Body acquisition (main spec §26 optional bodies, §28 dispatch, §38 order)
// ----------------------------------------------------------------------

/// How one documented entry reaches the trait method.
enum EntryPayload {
    /// Single-content JSON: decodes to the model value itself.
    Json {
        model: String,
    },
    /// Single-content text: decodes to `String`.
    Text,
    /// Single-content streaming: the raw body passes through.
    RawBody,
    /// Single-content wildcard: the generated `<Op>RequestBody` struct.
    WildcardStruct {
        struct_name: String,
    },
    EnumJson {
        enum_name: String,
        variant: String,
    },
    EnumText {
        enum_name: String,
        variant: String,
    },
    EnumRaw {
        enum_name: String,
        variant: String,
    },
    EnumWildcard {
        enum_name: String,
        variant: String,
    },
}

impl EntryPayload {
    fn is_decodable(&self) -> bool {
        matches!(
            self,
            Self::Json { .. } | Self::Text | Self::EnumJson { .. } | Self::EnumText { .. }
        )
    }

    fn is_json(&self) -> bool {
        matches!(self, Self::Json { .. } | Self::EnumJson { .. })
    }
}

fn entry_payload(
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
    index: usize,
) -> EntryPayload {
    let content = &operation.request_contents[index];
    let enum_name = operation.request_body_enum_name.clone();
    if content.is_wildcard {
        return match enum_name {
            Some(enum_name) => EntryPayload::EnumWildcard {
                enum_name,
                variant: content.variant_name.clone(),
            },
            None => EntryPayload::WildcardStruct {
                struct_name: layout
                    .wildcard_request(op_index)
                    .expect("wildcard struct registered")
                    .to_owned(),
            },
        };
    }
    match content.media_class {
        MediaClass::JsonFamily => match enum_name {
            Some(enum_name) => EntryPayload::EnumJson {
                enum_name,
                variant: content.variant_name.clone(),
            },
            None => EntryPayload::Json {
                model: content.model_expr.clone(),
            },
        },
        MediaClass::PlainText => match enum_name {
            Some(enum_name) => EntryPayload::EnumText {
                enum_name,
                variant: content.variant_name.clone(),
            },
            None => EntryPayload::Text,
        },
        MediaClass::Binary | MediaClass::RawUnknown => match enum_name {
            Some(enum_name) => EntryPayload::EnumRaw {
                enum_name,
                variant: content.variant_name.clone(),
            },
            None => EntryPayload::RawBody,
        },
        other => unreachable!("Phase 2 media class {other:?}"),
    }
}

fn route_is_buffered(operation: &PlannedOperation) -> bool {
    !operation.request_contents.is_empty()
        && operation.request_contents.iter().all(is_decodable_content)
}

#[allow(clippy::too_many_lines)]
fn emit_body_acquisition(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
) {
    let literals: Vec<String> = operation
        .request_contents
        .iter()
        .map(|content| rust_string_literal(&content.media_type_literal))
        .collect();
    let required = operation.request_body_required;

    if required {
        emitter.line(1, "let parsed = parse_single_content_type(&__headers)?;");
        emit_classify_match_head(emitter, &literals, 1);
        emit_absent_content_type_arm(emitter, operation, 2);
        emit_unmatched_arm(emitter, 2);
        for index in 0..operation.request_contents.len() {
            let payload = entry_payload(operation, layout, op_index, index);
            emit_entry_arm(emitter, &payload, operation, index, 2, true);
        }
        emitter.line(
            2,
            "RequestEntryMatch::Entry(_) => unreachable!(\"request entry index out of range\"),",
        );
        emitter.line(1, "};");
        return;
    }

    // Optional bodies decide presence by first-frame peek before any
    // Content-Type inspection (§28.2 peek-and-preserve).
    emitter.line(1, "let (presence, replay) =");
    emitter.line(
        2,
        "detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;",
    );
    emitter.line(1, "let request_body = match presence {");
    emitter.line(2, "BodyPresence::Empty => None,");
    emitter.line(2, "BodyPresence::Failed => {");
    emitter.line(
        3,
        "return Err(malformed_body(\"request body stream failed\"));",
    );
    emitter.line(2, "}");
    emitter.line(2, "BodyPresence::NonEmpty(_) => {");
    emitter.line(3, "let parsed = parse_single_content_type(&__headers)?;");
    emit_classify_match_head(emitter, &literals, 3);
    // The head helper emits the opening brace; arms follow at indent+1.
    emitter.line(
        4,
        "RequestEntryMatch::AbsentContentType | RequestEntryMatch::Unmatched => {",
    );
    emitter.line(5, "return Err(unsupported_media_type(");
    emitter.line(
        6,
        "\"nonempty optional body arrived without a usable Content-Type\",",
    );
    emitter.line(5, "));");
    emitter.line(4, "}");
    for index in 0..operation.request_contents.len() {
        let payload = entry_payload(operation, layout, op_index, index);
        emit_entry_arm(emitter, &payload, operation, index, 4, false);
    }
    emitter.line(
        4,
        "RequestEntryMatch::Entry(_) => unreachable!(\"request entry index out of range\"),",
    );
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "};");
}

/// §28.2 rows 1–2: without any Content-Type, emptiness separates the 400
/// missing-body rejection from the 415 unsupported-media-type rejection.
/// Decodable ops probe with a bounded collection; pure streaming ops peek
/// presence so nothing aggregates.
fn emit_absent_content_type_arm(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    indent: usize,
) {
    emitter.line(indent, "RequestEntryMatch::AbsentContentType => {");
    if operation.request_contents.iter().any(is_decodable_content) {
        let limit_field = probe_limit_field(operation);
        emitter.line(
            indent + 1,
            &format!("let probe = body_bytes(body, limits.{limit_field}).await?;"),
        );
        emitter.line(indent + 1, "if probe.is_empty() {");
        emitter.line(
            indent + 2,
            "return Err(malformed_body(\"required request body arrived empty\"));",
        );
        emitter.line(indent + 1, "}");
    } else {
        emitter.line(indent + 1, "let (presence, _replay) =");
        emitter.line(
            indent + 2,
            "detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;",
        );
        emitter.line(indent + 1, "if matches!(presence, BodyPresence::Empty) {");
        emitter.line(
            indent + 2,
            "return Err(malformed_body(\"required request body arrived empty\"));",
        );
        emitter.line(indent + 1, "}");
    }
    emitter.line(indent + 1, "return Err(unsupported_media_type(");
    emitter.line(indent + 2, "\"request arrived without a Content-Type\",");
    emitter.line(indent + 1, "));");
    emitter.line(indent, "}");
}

/// Emits the `match classify_request_entry(...)` opener: one line when the
/// whole slice fits within the rustfmt width, vertical slice otherwise. Only
/// the required-body call site (indent 1) binds the match result to
/// `request_body`; the optional-body site nests inside
/// `BodyPresence::NonEmpty`, where the match is a plain arm expression.
fn emit_classify_match_head(emitter: &mut Emitter, literals: &[String], indent: usize) {
    let joined = literals.join(", ");
    let prefix = if indent == 1 {
        "let request_body = "
    } else {
        ""
    };
    // rustfmt's canonical layouts, most-preferred first: whole head on one
    // line; otherwise the argument list stays horizontal and only the match
    // brace drops; only when even the head cannot fit do the slice entries
    // break vertically.
    let head = format!("{prefix}match classify_request_entry(parsed.as_ref(), &[{joined}])");
    let open_brace = format!("{head} {{");
    if fits(indent, &open_brace) {
        emitter.line(indent, &open_brace);
        return;
    }
    if fits(indent, &head) {
        emitter.line(indent, &head);
        emitter.line(indent, "{");
        return;
    }
    emitter.line(
        indent,
        &format!("{prefix}match classify_request_entry(parsed.as_ref(), &["),
    );
    for literal in literals {
        emitter.line(indent + 1, &format!("{literal},"));
    }
    emitter.line(indent, "]) {");
}

fn probe_limit_field(operation: &PlannedOperation) -> &'static str {
    if operation.request_contents.len() == 1
        && operation.request_contents[0].media_class == MediaClass::PlainText
    {
        "text_body_bytes"
    } else {
        "structured_request_bytes"
    }
}

fn emit_unmatched_arm(emitter: &mut Emitter, indent: usize) {
    emitter.line(indent, "RequestEntryMatch::Unmatched => {");
    emitter.line(indent + 1, "return Err(unsupported_media_type(");
    emitter.line(
        indent + 2,
        "\"no documented request media type matches the request Content-Type\",",
    );
    emitter.line(indent + 1, "));");
    emitter.line(indent, "}");
}

/// One `RequestEntryMatch::Entry(i)` arm. Required arms yield the value;
/// optional arms yield `Some(..)` fed from the replayed peek stream so the
/// peeked bytes decode exactly once (§28.2 invariant).
fn emit_entry_arm(
    emitter: &mut Emitter,
    payload: &EntryPayload,
    _operation: &PlannedOperation,
    index: usize,
    indent: usize,
    required: bool,
) {
    // Streaming/wildcard arms carry no statements, so rustfmt renders them
    // as expression arms; structured arms keep block form around their
    // charset check, bounded collection, and decode.
    if !payload.is_decodable() {
        emit_entry_yield_expr(emitter, indent, index, payload, required);
        return;
    }
    emitter.line(indent, &format!("RequestEntryMatch::Entry({index}) => {{"));
    if payload.is_decodable() {
        emitter.line(indent + 1, "ensure_utf8_charset(parsed.as_ref())?;");
        let limit_field = match payload {
            EntryPayload::Text | EntryPayload::EnumText { .. } => "text_body_bytes",
            _ => "structured_request_bytes",
        };
        if required {
            emitter.line(
                indent + 1,
                &format!("let bytes = body_bytes(body, limits.{limit_field}).await?;"),
            );
            if payload.is_json() {
                emitter.line(indent + 1, "if bytes.is_empty() {");
                emitter.line(
                    indent + 2,
                    "return Err(malformed_body(\"documented request body arrived empty\"));",
                );
                emitter.line(indent + 1, "}");
            }
        } else {
            emitter.line(
                indent + 1,
                &format!("let bytes = stream_bytes(replay, limits.{limit_field}).await?;"),
            );
        }
        match payload {
            EntryPayload::Json { model } => {
                let bind = format!("let value: {model} = decode_json_body(&bytes)?;");
                if fits(indent + 1, &bind) {
                    emitter.line(indent + 1, &bind);
                } else {
                    emitter.line(indent + 1, &format!("let value: {model} ="));
                    emitter.line(indent + 2, "decode_json_body(&bytes)?;");
                }
            }
            EntryPayload::EnumJson { .. } => {
                emitter.line(indent + 1, "let value = decode_json_body(&bytes)?;");
            }
            _ => {
                emitter.line(indent + 1, "let value = decode_text_body(bytes)?;");
            }
        }
    }
    emit_entry_yield(emitter, indent + 1, payload, required);
    emitter.line(indent, "}");
}

/// Emits `RequestEntryMatch::Entry(i) => EXPR,` with rustfmt-canonical
/// wrapping for each constructor shape.
fn emit_entry_yield_expr(
    emitter: &mut Emitter,
    indent: usize,
    index: usize,
    payload: &EntryPayload,
    required: bool,
) {
    let pattern = format!("RequestEntryMatch::Entry({index}) =>");
    let replay_body = "::axum::body::Body::from_stream(replay)";
    let emit_inline = |emitter: &mut Emitter, text: String| {
        emitter.line(indent, &format!("{pattern} {text},"));
    };

    match payload {
        EntryPayload::RawBody => {
            if required {
                emit_inline(emitter, "body".to_owned());
            } else {
                emit_inline(emitter, format!("Some({replay_body})"));
            }
        }
        EntryPayload::WildcardStruct { struct_name } => {
            let head_line = if required {
                format!("{pattern} {struct_name} {{")
            } else {
                format!("{pattern} Some({struct_name} {{")
            };
            emitter.line(indent, &head_line);
            emitter.line(indent + 1, "content_type: mime_of(parsed.as_ref()),");
            if required {
                emitter.line(indent + 1, "body,");
            } else {
                emitter.line(indent + 1, &format!("body: {replay_body},"));
            }
            if required {
                emitter.line(indent, "},");
            } else {
                emitter.line(indent, "}),");
            }
        }
        EntryPayload::EnumJson { enum_name, variant }
        | EntryPayload::EnumText { enum_name, variant } => {
            emit_inline(emitter, format!("{enum_name}::{variant}(value)"));
        }
        EntryPayload::EnumRaw { enum_name, variant } => {
            let inner = if required {
                format!("{enum_name}::{variant}(body)")
            } else {
                format!("{enum_name}::{variant}({replay_body})")
            };
            if required || fits(indent + 1, &inner) {
                emit_inline(emitter, inner);
            } else {
                emitter.line(indent, &format!("{pattern} Some({enum_name}::{variant}("));
                emitter.line(indent + 1, &format!("{replay_body},"));
                emitter.line(indent, ")),");
            }
        }
        EntryPayload::EnumWildcard { enum_name, variant } => {
            let head_line = if required {
                format!("{pattern} {enum_name}::{variant} {{")
            } else {
                format!("{pattern} Some({enum_name}::{variant} {{")
            };
            emitter.line(indent, &head_line);
            emitter.line(indent + 1, "content_type: mime_of(parsed.as_ref()),");
            if required {
                emitter.line(indent + 1, "body,");
            } else {
                emitter.line(indent + 1, &format!("body: {replay_body},"));
            }
            if required {
                emitter.line(indent, "},");
            } else {
                emitter.line(indent, "}),");
            }
        }
        EntryPayload::Json { .. } | EntryPayload::Text => {
            unreachable!("decodable payloads keep block arms");
        }
    }
}

/// The arm's trailing expression, laid out the way rustfmt renders each
/// constructor shape.
fn emit_entry_yield(emitter: &mut Emitter, indent: usize, payload: &EntryPayload, required: bool) {
    let replay_body = "::axum::body::Body::from_stream(replay)";
    let wrap = |emitter: &mut Emitter, inner_lines: Vec<String>| {
        if required {
            for line in inner_lines {
                emitter.line(indent, &line);
            }
        } else {
            emitter.line(indent, &format!("Some({}", inner_lines[0]));
            for line in &inner_lines[1..] {
                emitter.line(indent, line);
            }
            emitter.line(indent, ")");
        }
    };

    match payload {
        EntryPayload::Json { .. } | EntryPayload::Text => {
            // Optional bodies yield `Some(..)` so the outer presence match
            // separates Empty (None) from a decoded document (§28.2).
            if required {
                emitter.line(indent, "value");
            } else {
                emitter.line(indent, "Some(value)");
            }
        }
        EntryPayload::RawBody => {
            if required {
                emitter.line(indent, "body");
            } else {
                emitter.line(indent, &format!("Some({replay_body})"));
            }
        }
        EntryPayload::WildcardStruct { struct_name } => {
            wrap(
                emitter,
                vec![
                    format!("{struct_name} {{"),
                    "    content_type: mime_of(parsed.as_ref()),".to_owned(),
                    if required {
                        "    body,".to_owned()
                    } else {
                        format!("    body: {replay_body},")
                    },
                    "}".to_owned(),
                ],
            );
        }
        EntryPayload::EnumJson { enum_name, variant }
        | EntryPayload::EnumText { enum_name, variant } => {
            if required {
                emitter.line(indent, &format!("{enum_name}::{variant}(value)"));
            } else {
                emitter.line(indent, &format!("Some({enum_name}::{variant}(value))"));
            }
        }
        EntryPayload::EnumRaw { enum_name, variant } => {
            let inner = if required {
                format!("{enum_name}::{variant}(body)")
            } else {
                format!("{enum_name}::{variant}({replay_body})")
            };
            if required || fits(indent, &format!("Some({inner})")) {
                if required {
                    emitter.line(indent, &inner);
                } else {
                    emitter.line(indent, &format!("Some({inner})"));
                }
            } else {
                emitter.line(indent, &format!("Some({enum_name}::{variant}("));
                emitter.line(indent, &format!("    {replay_body},"));
                emitter.line(indent, "))");
            }
        }
        EntryPayload::EnumWildcard { enum_name, variant } => {
            wrap(
                emitter,
                vec![
                    format!("{enum_name}::{variant} {{"),
                    "    content_type: mime_of(parsed.as_ref()),".to_owned(),
                    if required {
                        "    body,".to_owned()
                    } else {
                        format!("    body: {replay_body},")
                    },
                    "}".to_owned(),
                ],
            );
        }
    }
}

// ----------------------------------------------------------------------
// Router registration (axum 0.8 keeps `{param}` placeholders verbatim)
// ----------------------------------------------------------------------

fn emit_router(emitter: &mut Emitter, plan: &PlannedApi, api_trait: &str) {
    emitter.blank();
    emitter.docs(
        0,
        &[
            "Builds the Axum router serving every documented operation (main \
             spec §38): buffered-body routes install `DefaultBodyLimit` at \
             `structured_request_bytes`; streaming-body routes remain exempt \
             because nothing aggregates them."
                .to_owned(),
        ],
    );
    emitter.line(0, "pub fn router(");
    emitter.line(1, &format!("api: ::std::sync::Arc<dyn {api_trait}>,"));
    emitter.line(1, "limits: BodyLimits,");
    emitter.line(
        1,
        "encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,",
    );
    emitter.line(0, ") -> ::axum::Router {");
    emitter.line(1, "let state = ServerState {");
    emitter.line(2, "api,");
    emitter.line(2, "limits,");
    emitter.line(2, "encode_overflow_hook,");
    emitter.line(1, "};");
    if plan.operations.is_empty() {
        emitter.line(1, "::axum::Router::new().with_state(state)");
        emitter.line(0, "}");
        return;
    }
    emitter.line(1, "::axum::Router::new()");
    for operation in &plan.operations {
        let routing = routing_fn(operation.http);
        let handler = format!("route_{}", operation.method);
        let path = rust_string_literal(&operation.path_template);
        if route_is_buffered(operation) {
            let head = format!(
                "::axum::routing::{routing}({handler}).layer(::axum::extract::DefaultBodyLimit::max("
            );
            emitter.line(2, ".route(");
            emitter.line(3, &format!("{path},"));
            let layer_inline = format!("::axum::routing::{routing}({handler}).layer(");
            // rustfmt breaks the head even at exactly 100 columns, so this
            // tier requires one column of slack.
            if fits(3, &head) && head.chars().count() + 3 * 4 < RUSTFMT_MAX_WIDTH {
                // Head plus nested `max(` fit together; only the argument
                // breaks.
                emitter.line(3, &head);
                emitter.line(4, "limits.structured_request_bytes,");
                emitter.line(3, ")),");
            } else if fits(3, &layer_inline) {
                emitter.line(3, &layer_inline);
                emitter.line(
                    4,
                    "::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),",
                );
                emitter.line(3, "),");
            } else {
                emitter.line(3, &format!("::axum::routing::{routing}({handler})"));
                emitter.line(4, ".layer(::axum::extract::DefaultBodyLimit::max(");
                emitter.line(5, "limits.structured_request_bytes,");
                emitter.line(4, ")),");
            }
            emitter.line(2, ")");
        } else {
            let inline = format!(".route({path}, ::axum::routing::{routing}({handler}))");
            // rustfmt also bounds each call's argument list by
            // `fn_call_width`, which forces vertical route args even when
            // the whole element would fit the line width.
            let route_args = format!("{path}, ::axum::routing::{routing}({handler})");
            if fits(2, &inline) && route_args.chars().count() <= FN_CALL_WIDTH {
                emitter.line(2, &inline);
            } else {
                emitter.line(2, ".route(");
                emitter.line(3, &format!("{path},"));
                emitter.line(3, &format!("::axum::routing::{routing}({handler}),"));
                emitter.line(2, ")");
            }
        }
    }
    emitter.line(2, ".with_state(state)");
    emitter.line(0, "}");
}

fn routing_fn(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "get",
        HttpMethod::Put => "put",
        HttpMethod::Post => "post",
        HttpMethod::Delete => "delete",
        HttpMethod::Options => "options",
        HttpMethod::Head => "head",
        HttpMethod::Patch => "patch",
        HttpMethod::Trace => "trace",
    }
}

// ----------------------------------------------------------------------
// Module helpers emitted into the generated file (gated by scan flags so
// nothing unused survives `-D warnings`)
// ----------------------------------------------------------------------

fn emit_module_helpers(emitter: &mut Emitter, flags: &Flags) {
    if !flags.has_operations {
        return;
    }

    // Each §39 constructor is emitted only when some generated call site
    // references it, keeping the module free of dead code under
    // `-D warnings`.
    if flags.needs_invalid_parameter {
        emitter.blank();
        emit_invalid_parameter(emitter);
    }
    if flags.needs_malformed_body {
        emitter.blank();
        emit_malformed_body(emitter);
    }
    if flags.needs_unsupported_media_type {
        emitter.blank();
        emit_unsupported_media_type(emitter);
    }

    if flags.needs_content_type_gate {
        emitter.blank();
        emit_parse_single_content_type(emitter);
        emitter.blank();
        emit_request_entry_matching(emitter);
    }
    if flags.needs_collect_body {
        emitter.blank();
        emit_body_bytes(emitter);
    }
    if flags.needs_collect_stream {
        emitter.blank();
        emit_stream_bytes(emitter);
    }
    if flags.needs_json_decode {
        emitter.blank();
        emit_decode_json_body(emitter);
    }
    if flags.needs_text_decode {
        emitter.blank();
        emit_decode_text_body(emitter);
    }
    if flags.needs_charset_check {
        emitter.blank();
        emit_ensure_utf8_charset(emitter);
    }
    if flags.needs_mime_of {
        emitter.blank();
        emit_mime_of(emitter);
    }
    if flags.needs_expect_text {
        emitter.blank();
        emit_expect_text(emitter);
    }
    if flags.needs_expect_texts {
        emitter.blank();
        emit_expect_texts(emitter);
    }
    if flags.needs_parse_scalar {
        emitter.blank();
        emit_parse_param_text(emitter);
    }
    if flags.needs_parse_texts {
        emitter.blank();
        emit_parse_texts(emitter);
    }
    if flags.needs_split_query_pairs {
        emitter.blank();
        emit_split_query_pairs(emitter);
    }
    if flags.needs_serialize_json || flags.needs_encode_text {
        emitter.blank();
        emit_encode_overflow_fallback(emitter);
    }
    if flags.needs_serialize_json {
        emitter.blank();
        emit_encode_json_limited(emitter);
    }
    if flags.needs_encode_text {
        emitter.blank();
        emit_encode_text_limited(emitter);
    }
    if flags.needs_stream_response {
        emitter.blank();
        emit_stream_response(emitter);
    }
    if flags.needs_any_response {
        emitter.blank();
        emit_any_response(emitter);
    }
}

fn emit_invalid_parameter(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Canonical §39 mapping row 1: invalid or missing required  path/query/header parameter → 400."
                .to_owned(),
        ],
    );
    emit_cow_signature(emitter, "invalid_parameter");
    emitter.line(
        1,
        "ProtocolRejection::new(RejectionKind::InvalidParameter).with_detail(detail)",
    );
    emitter.line(0, "}");
}

fn emit_malformed_body(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "§39 mapping row 2: syntactically malformed framing → 400; empty  bodies on required-body operations count as missing (§28.3)."
                .to_owned(),
        ],
    );
    emit_cow_signature(emitter, "malformed_body");
    emitter.line(
        1,
        "ProtocolRejection::new(RejectionKind::MalformedBody).with_detail(detail)",
    );
    emitter.line(0, "}");
}

fn emit_unsupported_media_type(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &["Missing, unparsable, wildcard, or unmatched Content-Type on a  body-bearing request → 415 (§28.2, §28.5, §39 table)."
            .to_owned()],
    );
    emit_cow_signature(emitter, "unsupported_media_type");
    emitter.line(
        1,
        "ProtocolRejection::new(RejectionKind::UnsupportedMediaType).with_detail(detail)",
    );
    emitter.line(0, "}");
}

/// Emits one rejection-constructor signature: joined when it fits within the
/// rustfmt width, vertical otherwise.
fn emit_cow_signature(emitter: &mut Emitter, name: &str) {
    let inline = format!(
        "fn {name}(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {{"
    );
    if fits(0, &inline) {
        emitter.line(0, &inline);
    } else {
        emitter.line(0, &format!("fn {name}("));
        emitter.line(1, "detail: impl Into<::std::borrow::Cow<'static, str>>,");
        emitter.line(0, ") -> ProtocolRejection {");
    }
}

fn emit_parse_single_content_type(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Reads exactly one parsable `Content-Type` header (§28 steps 1–2, \
             §28.1): duplicate headers are ambiguous, a missing header yields \
             `Ok(None)`, and malformed values are never ignored or defaulted."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn parse_single_content_type(");
    emitter.line(1, "headers: &::http::HeaderMap,");
    emitter.line(
        0,
        ") -> Result<Option<ParsedMediaType>, ProtocolRejection> {",
    );
    emitter.line(
        1,
        "let mut lines = headers.get_all(::http::header::CONTENT_TYPE).iter();",
    );
    emitter.line(1, "let Some(first) = lines.next() else {");
    emitter.line(2, "return Ok(None);");
    emitter.line(1, "};");
    emitter.line(1, "if lines.next().is_some() {");
    emitter.line(
        2,
        "return Err(malformed_body(\"duplicate Content-Type headers\"));",
    );
    emitter.line(1, "};");
    emitter.line(1, "let text = first");
    emitter.line(2, ".to_str()");
    emitter.line(
        2,
        ".map_err(|_| malformed_body(\"Content-Type is not valid UTF-8\"))?;",
    );
    emitter.line(1, "parse_content_type(text)");
    emitter.line(2, ".map(Some)");
    emitter.line(
        2,
        ".map_err(|_| malformed_body(\"malformed Content-Type\"))",
    );
    emitter.line(0, "}");
}

fn emit_request_entry_matching(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &["Outcome of matching the incoming Content-Type against the \
             documented request entries (§28 precedence list)."
            .to_owned()],
    );
    emitter.line(0, "enum RequestEntryMatch {");
    emitter.line(1, "/// No usable Content-Type header arrived.");
    emitter.line(1, "AbsentContentType,");
    emitter.line(
        1,
        "/// A parsable Content-Type matched no documented entry.",
    );
    emitter.line(1, "Unmatched,");
    emitter.line(1, "/// Index into the operation's documented entries.");
    emitter.line(1, "Entry(usize),");
    emitter.line(0, "}");
    emitter.blank();
    emitter.docs(
        0,
        &[
            "Ranks Exact beats suffix family beats range beats wildcard (§28); \
             a wildcard INCOMING type never selects among multiple entries \
             (§28.5) unless exactly one entry exists."
                .to_owned(),
        ],
    );
    emitter.line(
        0,
        "fn classify_request_entry(parsed: Option<&ParsedMediaType>, entries: &[&str]) -> RequestEntryMatch {",
    );
    emitter.line(1, "let Some(parsed) = parsed else {");
    emitter.line(2, "return RequestEntryMatch::AbsentContentType;");
    emitter.line(1, "};");
    emitter.line(1, "match best_request_entry(parsed, entries) {");
    emitter.line(2, "Some(index) => RequestEntryMatch::Entry(index),");
    emitter.line(2, "None => RequestEntryMatch::Unmatched,");
    emitter.line(1, "}");
    emitter.line(0, "}");
    emitter.blank();
    emitter.docs(
        0,
        &[String::from(
            "Best documented entry for one parsed incoming type; ties \
             resolve to the earliest declaration position.",
        )],
    );
    emitter.line(0, "#[must_use]");
    emitter.line(
        0,
        "fn best_request_entry(parsed: &ParsedMediaType, entries: &[&str]) -> Option<usize> {",
    );
    emitter.line(1, "if is_wildcard_incoming(parsed) {");
    emitter.line(2, "return if entries.len() == 1 { Some(0) } else { None };");
    emitter.line(1, "}");
    emitter.line(1, "let mut best: Option<(u8, usize)> = None;");
    emitter.line(1, "for (index, entry) in entries.iter().enumerate() {");
    emitter.line(2, "if let Some(matched) = match_entry(parsed, entry) {");
    emitter.line(3, "let rank = negotiation_rank(matched);");
    emitter.line(3, "if best.is_none_or(|(seen, _)| rank < seen) {");
    emitter.line(4, "best = Some((rank, index));");
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(1, "best.map(|(_, index)| index)");
    emitter.line(0, "}");
    emitter.blank();
    emitter.docs(0, &["§28 dispatch ranking.".to_owned()]);
    emitter.line(0, "#[must_use]");
    emitter.line(0, "fn negotiation_rank(matched: EntryMatch) -> u8 {");
    emitter.line(1, "match matched {");
    emitter.line(2, "EntryMatch::Exact => 0,");
    emitter.line(2, "EntryMatch::SuffixFamily => 1,");
    emitter.line(2, "EntryMatch::RangeMatch => 2,");
    emitter.line(2, "EntryMatch::Wildcard => 3,");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_body_bytes(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Bounded collection of an aggregated request body (§30.2, §38):  over-limit → 413, transport failure → 400."
                .to_owned(),
        ],
    );
    emitter.line(0, "async fn body_bytes(");
    emitter.line(1, "body: ::axum::body::Body,");
    emitter.line(1, "limit: usize,");
    emitter.line(0, ") -> Result<::bytes::Bytes, ProtocolRejection> {");
    emitter.line(
        1,
        "match collect_body_limited(body.into_data_stream(), limit).await {",
    );
    emitter.line(2, "Ok(bytes) => Ok(bytes),");
    emitter.line(2, "Err(CollectLimitedError::TooLarge { .. }) => {");
    emitter.line(
        3,
        "Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))",
    );
    emitter.line(2, "}");
    emitter.line(
        2,
        "Err(CollectLimitedError::Source(_)) => Err(malformed_body(\"request body stream failed\")),",
    );
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_stream_bytes(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Bounded collection over the replayed peek stream (§28.2): the  peeked prefix is consumed exactly once."
                .to_owned(),
        ],
    );
    emitter.line(0, "async fn stream_bytes(");
    emitter.line(1, "stream: ::axum::body::BodyDataStream,");
    emitter.line(1, "limit: usize,");
    emitter.line(0, ") -> Result<::bytes::Bytes, ProtocolRejection> {");
    emitter.line(1, "match collect_body_limited(stream, limit).await {");
    emitter.line(2, "Ok(bytes) => Ok(bytes),");
    emitter.line(2, "Err(CollectLimitedError::TooLarge { .. }) => {");
    emitter.line(
        3,
        "Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))",
    );
    emitter.line(2, "}");
    emitter.line(
        2,
        "Err(CollectLimitedError::Source(_)) => Err(malformed_body(\"request body stream failed\")),",
    );
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_decode_json_body(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Maps bounded JSON decode failures onto §39 kinds: syntax/io →  MalformedBody 400, data errors (missing fields/types) →  SchemaViolation 422 (D-impl-runtime-validation-timing)."
                .to_owned(),
        ],
    );
    emitter.line(
        0,
        "fn decode_json_body<T>(bytes: &[u8]) -> Result<T, ProtocolRejection>",
    );
    emitter.line(0, "where");
    emitter.line(1, "T: serde::de::DeserializeOwned,");
    emitter.line(0, "{");
    emitter.line(1, "serde_json::from_slice(bytes).map_err(|error| {");
    emitter.line(2, "if error.is_data() {");
    emitter.line(3, "ProtocolRejection::new(RejectionKind::SchemaViolation)");
    emitter.line(
        4,
        ".with_detail(\"well-formed body failed schema validation\")",
    );
    emitter.line(2, "} else {");
    emitter.line(3, "malformed_body(\"malformed JSON body\")");
    emitter.line(2, "}");
    emitter.line(1, "})");
    emitter.line(0, "}");
}

fn emit_decode_text_body(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Strict UTF-8 for bounded textual bodies (§28.4): invalid bytes  are protocol failures, never replacement characters."
                .to_owned(),
        ],
    );
    emitter.line(
        0,
        "fn decode_text_body(bytes: ::bytes::Bytes) -> Result<String, ProtocolRejection> {",
    );
    emitter.line(
        1,
        "String::from_utf8(bytes.to_vec()).map_err(|_| malformed_body(\"text body is not UTF-8\"))",
    );
    emitter.line(0, "}");
}

fn emit_ensure_utf8_charset(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "§28.4 charset policy (D-impl-charset-rejection): textual media  decode as UTF-8; any other declared charset is MalformedBody 400."
                .to_owned(),
        ],
    );
    emitter.line(
        0,
        "fn ensure_utf8_charset(parsed: Option<&ParsedMediaType>) -> Result<(), ProtocolRejection> {",
    );
    emitter.line(1, "let Some(parsed) = parsed else {");
    emitter.line(2, "return Ok(());");
    emitter.line(1, "};");
    emitter.line(
        1,
        "if let Some((_, value)) = parsed.parameters.iter().find(|(name, _)| name == \"charset\") {",
    );
    emitter.line(2, "let lowered = value.to_ascii_lowercase();");
    emitter.line(2, "if lowered != \"utf-8\" && lowered != \"utf8\" {");
    emitter.line(
        3,
        "return Err(malformed_body(\"charset outside the UTF-8 family\"));",
    );
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(1, "Ok(())");
    emitter.line(0, "}");
}

fn emit_mime_of(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Builds the `mime::Mime` carried by wildcard payloads (§22); the  type already parsed successfully, so the fallback never fires."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[must_use]");
    emitter.line(
        0,
        "fn mime_of(parsed: Option<&ParsedMediaType>) -> ::mime::Mime {",
    );
    emitter.line(1, "let Some(parsed) = parsed else {");
    emitter.line(2, "return ::mime::STAR_STAR;");
    emitter.line(1, "};");
    emitter.line(1, "let subtype = match &parsed.suffix {");
    emitter.line(
        2,
        "Some(suffix) => format!(\"{}+{}\", parsed.subtype, suffix),",
    );
    emitter.line(2, "None => parsed.subtype.clone(),");
    emitter.line(1, "};");
    emitter.line(
        1,
        "format!(\"{}/{}\", parsed.ty, subtype).parse().unwrap_or(::mime::STAR_STAR)",
    );
    emitter.line(0, "}");
}

fn emit_expect_text(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Unwraps a scalar decode product; `None` means the parameter was  absent (required parameters reject with 400, §39 row 1)."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn expect_text(");
    emitter.line(1, "decoded: Option<ParamValue>,");
    emitter.line(1, "parameter: &'static str,");
    emitter.line(0, ") -> Result<String, ProtocolRejection> {");
    emitter.line(1, "match decoded {");
    emitter.line(2, "Some(ParamValue::Text(text)) => Ok(text),");
    emitter.line(2, "Some(_) => Err(invalid_parameter(format!(");
    emitter.line(3, "\"parameter `{parameter}` has an unexpected shape\"");
    emitter.line(2, "))),");
    emitter.line(2, "None => Err(invalid_parameter(format!(");
    emitter.line(3, "\"missing required parameter `{parameter}`\"");
    emitter.line(2, "))),");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_expect_texts(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &["Unwraps an array decode product into its text items.".to_owned()],
    );
    emitter.line(0, "fn expect_texts(");
    emitter.line(1, "decoded: Option<ParamValue>,");
    emitter.line(1, "parameter: &'static str,");
    emitter.line(0, ") -> Result<Vec<String>, ProtocolRejection> {");
    emitter.line(1, "let items = match decoded {");
    emitter.line(2, "Some(ParamValue::Array(items)) => items,");
    emitter.line(2, "item => {");
    emitter.line(3, "return Err(invalid_parameter(format!(");
    emitter.line(
        4,
        "\"parameter `{parameter}` must be an array of scalars, got {item:?}\"",
    );
    emitter.line(3, ")));");
    emitter.line(2, "}");
    emitter.line(1, "};");
    emitter.line(1, "let mut texts = Vec::with_capacity(items.len());");
    emitter.line(1, "for item in items {");
    emitter.line(2, "match item {");
    emitter.line(3, "ParamValue::Text(text) => texts.push(text),");
    emitter.line(3, "item => {");
    emitter.line(4, "return Err(invalid_parameter(format!(");
    emitter.line(
        5,
        "\"parameter `{parameter}` has an unexpected shape: {item:?}\"",
    );
    emitter.line(4, ")));");
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(1, "Ok(texts)");
    emitter.line(0, "}");
}

fn emit_parse_param_text(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &["Type-parses one decoded scalar against the schema's Rust type  (§38 pre-handler validation; failures → 400)."
            .to_owned()],
    );
    emitter.line(0, "fn parse_param_text<T: ::std::str::FromStr>(");
    emitter.line(1, "parameter: &'static str,");
    emitter.line(1, "text: &str,");
    emitter.line(0, ") -> Result<T, ProtocolRejection> {");
    emitter.line(1, "text.parse().map_err(|_| {");
    emitter.line(
        2,
        "invalid_parameter(format!(\"parameter `{parameter}` has an invalid value\"))",
    );
    emitter.line(1, "})");
    emitter.line(0, "}");
}

fn emit_parse_texts(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &["Type-parses every item of a decoded array parameter.".to_owned()],
    );
    emitter.line(0, "fn parse_texts<T: ::std::str::FromStr>(");
    emitter.line(1, "parameter: &'static str,");
    emitter.line(1, "texts: &[String],");
    emitter.line(0, ") -> Result<Vec<T>, ProtocolRejection> {");
    emitter.line(1, "let mut out = Vec::with_capacity(texts.len());");
    emitter.line(1, "for text in texts {");
    emitter.line(2, "out.push(parse_param_text::<T>(parameter, text)?);");
    emitter.line(1, "}");
    emitter.line(1, "Ok(out)");
    emitter.line(0, "}");
}

fn emit_split_query_pairs(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Splits a raw query string into still-percent-encoded pairs,  preserving wire order for the companion §6 decoder."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[must_use]");
    emitter.line(0, "fn split_query_pairs(raw: &str) -> Vec<(&str, &str)> {");
    emitter.line(1, "raw.split('&')");
    emitter.line(2, ".filter(|pair| !pair.is_empty())");
    emitter.line(2, ".map(|pair| match pair.split_once('=') {");
    emitter.line(3, "Some((key, value)) => (key, value),");
    emitter.line(3, "None => (pair, \"\"),");
    emitter.line(2, "})");
    emitter.line(2, ".collect()");
    emitter.line(0, "}");
}

fn emit_encode_overflow_fallback(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "§34.1 steps 1–3: partial output is discarded, nothing partial  reaches the wire, and the hook carries the operation id, variant,  and limit for observability."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn encode_overflow_fallback(");
    emitter.line(1, "hook: &dyn EncodeOverflowHook,");
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(1, "variant: &'static str,");
    emitter.line(1, "limit: usize,");
    emitter.line(0, ") -> ::axum::response::Response {");
    emitter.line(1, "hook.on_encode_overflow(operation_id, variant, limit);");
    emitter.line(
        1,
        "::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()",
    );
    emitter.line(0, "}");
}

fn emit_encode_json_limited(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Bounded JSON response encoding (§34/§41); the literal keeps  distinct types such as application/problem+json separate from  application/json."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn encode_json_limited<T>(");
    emitter.line(1, "status: ::http::StatusCode,");
    emitter.line(1, "content_type: &'static str,");
    emitter.line(1, "value: &T,");
    emitter.line(1, "limits: &BodyLimits,");
    emitter.line(1, "hook: &dyn EncodeOverflowHook,");
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(1, "variant: &'static str,");
    emitter.line(0, ") -> ::axum::response::Response");
    emitter.line(0, "where");
    emitter.line(1, "T: serde::Serialize,");
    emitter.line(0, "{");
    emitter.line(1, "let budget = limits.structured_encode_bytes;");
    emitter.line(1, "match serialize_json_limited(value, budget) {");
    emitter.line(2, "Ok(bytes) => {");
    emitter.line(3, "let mut response = (status, bytes).into_response();");
    emitter.line(3, "response.headers_mut().insert(");
    emitter.line(4, "::http::header::CONTENT_TYPE,");
    emitter.line(4, "::http::HeaderValue::from_static(content_type),");
    emitter.line(3, ");");
    emitter.line(3, "response");
    emitter.line(2, "}");
    emitter.line(
        2,
        "Err(error) => encode_overflow_fallback(hook, operation_id, variant, error.limit),",
    );
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_encode_text_limited(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &["Bounded plain-text response encoding (§34/§41).".to_owned()],
    );
    emitter.line(0, "fn encode_text_limited(");
    emitter.line(1, "status: ::http::StatusCode,");
    emitter.line(1, "content_type: &'static str,");
    emitter.line(1, "value: &str,");
    emitter.line(1, "limits: &BodyLimits,");
    emitter.line(1, "hook: &dyn EncodeOverflowHook,");
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(1, "variant: &'static str,");
    emitter.line(0, ") -> ::axum::response::Response {");
    emitter.line(1, "let budget = limits.structured_encode_bytes;");
    emitter.line(1, "if value.len() > budget {");
    emitter.line(
        2,
        "return encode_overflow_fallback(hook, operation_id, variant, budget);",
    );
    emitter.line(1, "}");
    emitter.line(
        1,
        "let payload = ::bytes::Bytes::copy_from_slice(value.as_bytes());",
    );
    emitter.line(1, "let mut response = (status, payload).into_response();");
    emitter.line(1, "response.headers_mut().insert(");
    emitter.line(2, "::http::header::CONTENT_TYPE,");
    emitter.line(2, "::http::HeaderValue::from_static(content_type),");
    emitter.line(1, ");");
    emitter.line(1, "response");
    emitter.line(0, "}");
}

fn emit_stream_response(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Streams a documented binary/raw payload verbatim behind its  literal Content-Type (§32/§41); nothing aggregates it."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn stream_response(");
    emitter.line(1, "status: ::http::StatusCode,");
    emitter.line(1, "content_type: &'static str,");
    emitter.line(1, "body: ::axum::body::Body,");
    emitter.line(0, ") -> ::axum::response::Response {");
    emitter.line(1, "let mut response = (status, body).into_response();");
    emitter.line(1, "response.headers_mut().insert(");
    emitter.line(2, "::http::header::CONTENT_TYPE,");
    emitter.line(2, "::http::HeaderValue::from_static(content_type),");
    emitter.line(1, ");");
    emitter.line(1, "response");
    emitter.line(0, "}");
}

fn emit_any_response(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Streams a wildcard payload behind the application-supplied media  type (§22); `essence_str` drops parameters such as charset."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn any_response(");
    emitter.line(1, "status: ::http::StatusCode,");
    emitter.line(1, "content_type: ::mime::Mime,");
    emitter.line(1, "body: ::axum::body::Body,");
    emitter.line(0, ") -> ::axum::response::Response {");
    emitter.line(1, "let declared = content_type.essence_str().to_owned();");
    emitter.line(1, "let mut response = (status, body).into_response();");
    emitter.line(1, "let header = ::http::HeaderValue::try_from(declared)");
    emitter.line(2, ".unwrap_or(::http::HeaderValue::from_static(\"*/*\"));");
    emitter.line(
        1,
        "response.headers_mut().insert(::http::header::CONTENT_TYPE, header);",
    );
    emitter.line(1, "response");
    emitter.line(0, "}");
}

// ----------------------------------------------------------------------
// Small emission utilities shared with the client emitter's conventions
// ----------------------------------------------------------------------

fn fits(indent: usize, text: &str) -> bool {
    indent * 4 + text.chars().count() <= RUSTFMT_MAX_WIDTH
}

/// `http::StatusCode` constant names derived from the §4 reason phrase
/// (SCREAMING_SNAKE); matches the `http` crate names for standard codes.
fn status_code_const(code: u16) -> Option<String> {
    super::plan::reason_phrase(code).map(screaming_snake)
}

fn screaming_snake(pascal: &str) -> String {
    let chars: Vec<char> = pascal.chars().collect();
    let mut out = String::new();
    for (index, current) in chars.iter().copied().enumerate() {
        let previous = if index == 0 {
            None
        } else {
            Some(chars[index - 1])
        };
        let next = chars.get(index + 1).copied();
        let boundary = current.is_ascii_uppercase()
            && previous.is_some_and(|previous| {
                previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase()
                        && next.is_some_and(|next| next.is_ascii_lowercase()))
            });
        if boundary && !out.is_empty() {
            out.push('_');
        }
        out.push(current.to_ascii_uppercase());
    }
    out
}

/// Escapes a value into a deterministic double-quoted Rust string literal.
fn rust_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if control.is_control() => {
                out.push_str(&format!("\\u{{{:x}}}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
