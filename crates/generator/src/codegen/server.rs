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
//! `::mime`, `::bytes`, `serde_json`, and the shared types surface
//! (`models`/`views`) at the configured location — sibling `super::models`
//! by default, or an external base path for split workspace layouts
//! ([`super::config::TypesLocation`], D-impl-selective-artifacts).
//!
//! # Directional views (companion §5, main spec §50 test 50)
//!
//! Request payloads decode into `<M>Write` for view-carrying components
//! (readOnly fields structurally absent from the wire; required-in-write
//! fields mandatory at Serde decode → data errors reject 422 pre-handler);
//! response payloads carry `<M>Read` so writeOnly fields never reach the
//! wire. Views do NOT set `deny_unknown_fields` (unless a schema declares
//! `additionalProperties: false`): surplus keys — e.g. a client sending an
//! off-direction field — are IGNORED on decode like the shared models'
//! default policy.
//!
//! Recorded trait-signature contract decision (router auto-converts when
//! lossless): a single-content structured body whose `<M>Write` view can
//! reconstruct the shared model (`From<&<M>Write> for <M>` exists, §5
//! lossless rule) keeps the SHARED model on the API-trait signature; the
//! router decodes the view, runs its `validate_request`, converts through
//! the lossless `From`, and passes the shared value to the trait. When
//! reconstruction would lose data (a dropped required field), the trait
//! takes the `<M>Write` view itself. Multi-content request enums always
//! carry decoded views end-to-end; handlers convert per-variant.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::document::{
    HttpMethod, MediaClass, ParameterLocation, ParameterStyle, RangeClass, ResponseStatusKey,
};
use crate::normalize::naming::{self, NameStyle};
use crate::normalize::NormalizedDocument;

use super::codecs::{default_registry as codec_registry, helper_prefix};
use super::config::{CodegenConfig, TypesLocation};
use super::plan::{
    PlannedApi, PlannedBodyValidation, PlannedContent, PlannedMultipartFieldKind, PlannedOperation,
    PlannedParameter, PlannedStatus, StreamFraming,
};
use super::Emitter;

const RUSTFMT_MAX_WIDTH: usize = 100;

/// rustfmt's default maximum width for a call argument list.
const FN_CALL_WIDTH: usize = 60;

/// Empirical rustfmt budget for an UNBROKEN call inside a match-arm block
/// (see [`emit_call_arm_expr`]).
const ARM_BODY_CALL_BUDGET: usize = 77;

/// Renders ONE generated `server.rs` for the planned API with the shared
/// types as SIBLING modules (`super::models`, `super::views`; main spec §3,
/// D-impl-singlefile-layout). Backward-compatible wrapper over
/// [`generate_server_with_config`].
#[must_use]
pub fn generate_server(doc: &NormalizedDocument, plan: &PlannedApi) -> String {
    generate_server_with_config(doc, plan, &CodegenConfig::default())
}

/// Renders ONE generated `server.rs` under an explicit shared-types location
/// (DECISIONS.md D-impl-selective-artifacts): [`TypesLocation::Sibling`]
/// keeps the historical sibling imports, [`TypesLocation::External`] points
/// the model/view imports at an externally generated namespace instead.
#[must_use]
pub fn generate_server_with_config(
    doc: &NormalizedDocument,
    plan: &PlannedApi,
    config: &CodegenConfig,
) -> String {
    let mut flags = Flags::default();
    let api_trait = trait_name(doc);

    let mut used_names = reserved_names(doc, plan, &api_trait);
    let layout = ServerLayout::new(plan, &mut used_names);
    for operation in &plan.operations {
        flags.scan_operation(operation);
    }

    let mut emitter = Emitter::new();
    emit_header(&mut emitter, doc, &flags);

    // The generated encoders and no-body arms CALL `IntoResponse::
    // into_response`, so the trait must be in scope; importing it whenever
    // any caller exists keeps the import used under `-D warnings`.
    flags.needs_into_response_trait = flags.needs_serialize_json
        || flags.needs_encode_text
        || flags.needs_stream_response
        || flags.needs_any_response
        || !flags.encode_codecs.is_empty()
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
        || flags.needs_form_decode
        || flags.needs_charset_check
        || flags.needs_cookie_decode
        || flags.needs_multipart
        || !flags.request_framings.is_empty()
        || !flags.decode_codecs.is_empty();
    // Companion §9: the shared 422 constructor exists only with call sites.
    flags.needs_request_schema_violation = plan.server_runtime_validation
        && plan.operations.iter().any(|operation| {
            operation.request_contents.iter().any(|content| {
                content.body_validation.is_some()
                    || content.multipart_spec.as_ref().is_some_and(|spec| {
                        spec.fields
                            .iter()
                            .any(|field| field.scalar_validator.is_some())
                    })
            })
        });
    emit_imports(&mut emitter, &flags, &config.types_location);
    // Shared boxed-stream alias behind every erased streaming payload and
    // typed request input (§18–§20); kept short so every use site stays on
    // one line.
    if !flags.response_framings.is_empty() || !flags.request_framings.is_empty() {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "Boxed erased item stream shared by every generated streaming                  wrapper (§18–§20): producers box their producer stream into                  this type."
                    .to_owned(),
            ],
        );
        emitter.line(0, "#[doc(hidden)]");
        emitter.line(0, "pub type ErasedItems<T> =");
        emitter.line(
            1,
            "::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>;",
        );
    }

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
    let has_stream_responses = !flags.response_framings.is_empty();
    emit_state(&mut emitter, &api_trait, has_stream_responses);
    for (op_index, operation) in plan.operations.iter().enumerate() {
        emit_handler(&mut emitter, op_index, operation, &layout);
        if layout.multipart_input(op_index).is_some() {
            emit_multipart_collector(&mut emitter, op_index, operation, &layout);
        }
    }
    emit_router(&mut emitter, plan, &api_trait, has_stream_responses);
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
    used.insert("ErasedItems".to_owned());
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
    /// Decodable single-content entry with documented headers (main spec
    /// §15): the wrapper carries the typed header fields beside the body.
    TypedHeaders,
    /// Record-framed stream entry on an EXPLICIT status with documented
    /// headers (§15/§40): the wrapper carries the typed header fields beside
    /// the erased item-stream alias; the encoder commits status,
    /// Content-Type, and headers before attaching the streaming body.
    StreamHeaders,
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
    /// Operation index → `<Op>MultipartInput` streaming input struct (§17).
    multipart_inputs: BTreeMap<usize, String>,
    /// (operation index, field index) → `<Op><Field>Part` streaming part.
    multipart_parts: BTreeMap<(usize, usize), String>,
    /// Operation index → `<Op>TrailingParts` carrier for scalar/JSON parts
    /// decoded BEHIND a live streaming part (wire-arrival-based §17.1).
    multipart_trailing: BTreeMap<usize, String>,
    /// (operation index, status index, content index) → `<…>Stream` erased
    /// item-stream alias (§18–§20 Output B): `Pin<Box<dyn Stream<Item =
    /// Result<Item, ServerStreamError>> + Send>>`.
    stream_aliases: BTreeMap<(usize, usize, usize), String>,
    /// (operation index, request content index) → `<Op><Framing>Input`
    /// streaming request wrapper exposing `next_item`
    /// (D-impl-request-direction-streams).
    stream_inputs: BTreeMap<(usize, usize), String>,
    /// Companion §9 policy (D-impl-runtime-validation-timing): when true the
    /// emitted routes run `validate_request`/free validators after decode.
    runtime_validation: bool,
}

impl ServerLayout {
    fn new(plan: &PlannedApi, used: &mut BTreeSet<String>) -> Self {
        let runtime_validation = plan.server_runtime_validation;
        let mut layout = Self {
            runtime_validation,
            ..Self::default()
        };
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
            for (content_index, content) in operation.request_contents.iter().enumerate() {
                if let Some(stream) = &content.stream {
                    let base = format!("{}{}Input", operation.pascal, stream.framing.as_pascal());
                    let name = fresh_name(used, base);
                    layout.stream_inputs.insert((op_index, content_index), name);
                }
            }
            for (status_index, status) in operation.statuses.iter().enumerate() {
                for (content_index, content) in status.contents.iter().enumerate() {
                    if !status.is_no_body_status && content.stream.is_some() {
                        let base = if status.contents.len() >= 2 {
                            format!(
                                "{}{}{}Stream",
                                operation.pascal,
                                status_name_part(status),
                                content.variant_name
                            )
                        } else {
                            format!("{}{}Stream", operation.pascal, status_name_part(status))
                        };
                        let name = fresh_name(used, base);
                        layout
                            .stream_aliases
                            .insert((op_index, status_index, content_index), name);
                    }
                }
            }
            if operation
                .request_contents
                .iter()
                .any(|content| content.media_class == MediaClass::Multipart)
            {
                let base = format!("{}MultipartInput", operation.pascal);
                let name = fresh_name(used, base);
                layout.multipart_inputs.insert(op_index, name);
                let Some(spec) = operation
                    .request_contents
                    .iter()
                    .find(|content| content.media_class == MediaClass::Multipart)
                    .and_then(|content| content.multipart_spec.as_ref())
                else {
                    continue;
                };
                for (field_index, field) in spec.fields.iter().enumerate() {
                    if matches!(field.kind, PlannedMultipartFieldKind::BinaryPart) {
                        let field_pascal = naming::ident(&field.rust_name, NameStyle::Pascal);
                        let base = format!("{}{field_pascal}Part", operation.pascal);
                        let name = fresh_name(used, base);
                        layout.multipart_parts.insert((op_index, field_index), name);
                    }
                }
                // The trailing carrier exists only when a live part can
                // actually decode scalar/JSON fields behind itself.
                let has_binary = spec
                    .fields
                    .iter()
                    .any(|field| matches!(field.kind, PlannedMultipartFieldKind::BinaryPart));
                let has_buffered = spec.fields.iter().any(|field| {
                    matches!(
                        field.kind,
                        PlannedMultipartFieldKind::JsonPart(_)
                            | PlannedMultipartFieldKind::ScalarText(_)
                    )
                });
                if has_binary && has_buffered {
                    let base = format!("{}TrailingParts", operation.pascal);
                    let name = fresh_name(used, base);
                    layout.multipart_trailing.insert(op_index, name);
                }
            }
        }
        layout
    }

    fn multipart_input(&self, op_index: usize) -> Option<&str> {
        self.multipart_inputs.get(&op_index).map(String::as_str)
    }

    fn multipart_part(&self, op_index: usize, field_index: usize) -> Option<&str> {
        self.multipart_parts
            .get(&(op_index, field_index))
            .map(String::as_str)
    }

    fn multipart_trailing(&self, op_index: usize) -> Option<&str> {
        self.multipart_trailing.get(&op_index).map(String::as_str)
    }

    fn stream_input(&self, op_index: usize, content_index: usize) -> Option<&str> {
        self.stream_inputs
            .get(&(op_index, content_index))
            .map(String::as_str)
    }

    fn stream_alias(
        &self,
        op_index: usize,
        status_index: usize,
        content_index: usize,
    ) -> Option<&str> {
        self.stream_aliases
            .get(&(op_index, status_index, content_index))
            .map(String::as_str)
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

/// Single-content statuses whose payload owns a wrapper struct: binary/raw
/// carry the raw body; wildcards carry the application-supplied
/// Content-Type beside it; decodable entries WITH documented headers carry
/// the typed header fields (main spec §15, superseding
/// D-impl-typed-headers-phase2). No-body statuses never wrap (§35), and
/// range/default statuses with headers keep their struct variants (which
/// must carry the wire status, §23–§24) instead of wrapping.
fn wrapper_shape(status: &PlannedStatus) -> Option<WrapperShape> {
    if status.is_no_body_status {
        return None;
    }
    let [content] = status.contents.as_slice() else {
        return None;
    };
    if content.is_wildcard {
        return if status.headers.is_empty() || !struct_variant_status(status) {
            Some(WrapperShape::Wildcard)
        } else {
            None
        };
    }
    match content.media_class {
        // Codec-claimed entries (§45) are decodable: they follow the typed
        // §15 header-wrapper logic instead of owning the raw response.
        MediaClass::Binary | MediaClass::RawUnknown if content.codec.is_some() => {
            if !status.headers.is_empty() && !struct_variant_status(status) {
                Some(WrapperShape::TypedHeaders)
            } else {
                None
            }
        }
        MediaClass::Binary | MediaClass::RawUnknown => {
            if status.headers.is_empty() || !struct_variant_status(status) {
                Some(WrapperShape::Stream)
            } else {
                None
            }
        }
        // Record-framed streams on explicit statuses with documented
        // headers: headers ride a wrapper beside the erased item stream.
        _ if content.stream.is_some()
            && !struct_variant_status(status)
            && !status.headers.is_empty() =>
        {
            Some(WrapperShape::StreamHeaders)
        }
        _ if !status.headers.is_empty() && !struct_variant_status(status) => {
            Some(WrapperShape::TypedHeaders)
        }
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
/// pass the body through untouched. Form bodies decode through the bounded
/// support decoder (§16); codec-claimed entries (§45) bound-collect then
/// parse through their generated per-codec helper.
fn is_decodable_content(content: &PlannedContent) -> bool {
    !content.is_wildcard
        && (content.codec.is_some()
            || matches!(
                content.media_class,
                MediaClass::JsonFamily | MediaClass::PlainText | MediaClass::UrlEncodedForm
            ))
}

// ----------------------------------------------------------------------
// Emission flags gathered in one deterministic scan so imports and module
// helpers contain exactly what the bodies reference.
// ----------------------------------------------------------------------

#[derive(Debug, Default)]
struct Flags {
    /// Shared-model type names referenced somewhere (`use super::models::…`).
    model_types: BTreeSet<String>,
    /// Directional-view type names referenced somewhere
    /// (`use super::views::…`, companion §5): decoded `<M>Write` request
    /// payloads and encoded `<M>Read` response payloads.
    view_types: BTreeSet<String>,
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
    needs_form_decode: bool,
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
    /// Documented response headers exist somewhere (main spec §15): pulls in
    /// the typed-header writer plus its §34.1-style failure fallback.
    needs_typed_headers: bool,
    /// A multipart request body exists somewhere (main spec §17 Output B):
    /// pulls in the streaming engine wiring plus rejection mapping.
    needs_multipart: bool,
    /// A typed (non-String) textual multipart part exists somewhere: pulls
    /// in the FromStr-based scalar part decoder.
    needs_multipart_scalar: bool,
    /// Companion §9 validators are CALLED somewhere (policy on + at least
    /// one constrained body/part): pulls in the shared SchemaViolation 422
    /// constructor for post-decode validation failures.
    needs_request_schema_violation: bool,
    /// Free validator functions referenced from super::models; imported ONLY
    /// when the validation policy is on (the calls disappear otherwise).
    body_validator_fns: BTreeSet<String>,
    /// Streaming record classes used by RESPONSE statuses somewhere: pulls
    /// in the erased-alias machinery (§40 encoder, hook, ServerStreamError).
    response_framings: BTreeSet<StreamFraming>,
    /// Streaming record classes used by REQUEST bodies somewhere: pulls in
    /// the per-framing decoder + decode-error pair and the typed input
    /// wrappers (D-impl-request-direction-streams).
    request_framings: BTreeSet<StreamFraming>,
    /// Codec plugin ids whose REQUEST decode helper is referenced somewhere
    /// (main spec §45): pulls in the codec's use lines plus its generated
    /// bounded decode helper.
    decode_codecs: BTreeSet<String>,
    /// Codec plugin ids whose RESPONSE encode helper is referenced somewhere
    /// (§45/§34.1): pulls in the codec's use lines plus its generated encode
    /// helper.
    encode_codecs: BTreeSet<String>,
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
        // Directional-view contract (companion §5): a SINGLE-content
        // structured/codec body whose write view reconstructs losslessly
        // keeps the SHARED model on the trait signature, so its import rides
        // here; enum/multipart/stream bodies never convert.
        if operation.request_body_enum_name.is_none() {
            if let [content] = operation.request_contents.as_slice() {
                if let Some(view) = &content.view {
                    let convertible = content.codec.is_some()
                        || (!content.is_wildcard
                            && content.stream.is_none()
                            && matches!(
                                content.media_class,
                                MediaClass::JsonFamily | MediaClass::UrlEncodedForm
                            ));
                    if convertible && view.write_lossless {
                        self.model_types.insert(view.shared_type.clone());
                    }
                }
            }
        }
        for status in &operation.statuses {
            self.scan_status(status);
        }
        for parameter in &operation.parameters {
            self.scan_parameter(parameter);
        }
    }

    fn scan_request_content(&mut self, content: &PlannedContent) {
        if let Some(binding) = &content.codec {
            // §45 codec-claimed request entry: bounded collect then the
            // generated per-codec decode (D-impl-codec-plugins).
            self.decode_codecs.insert(binding.plugin_id.to_owned());
            self.note_payload(&binding.model_path, binding.model_from_views);
            self.import_body_validator(content);
            return;
        }
        if content.is_wildcard {
            self.needs_mime_of = true;
            return;
        }
        match content.media_class {
            MediaClass::JsonFamily => {
                self.note_payload(&content.model_expr, content.view.is_some());
                self.import_body_validator(content);
                self.needs_json_decode = true;
                self.needs_charset_check = true;
            }
            MediaClass::UrlEncodedForm => {
                // Bounded form decode per §16/D-impl-forms; axum's Form
                // extractor is never used (the router self-decodes).
                self.note_payload(&content.model_expr, content.view.is_some());
                self.import_body_validator(content);
                self.needs_form_decode = true;
                self.needs_charset_check = true;
            }
            MediaClass::PlainText => {
                self.import_body_validator(content);
                self.needs_text_decode = true;
                self.needs_charset_check = true;
            }
            MediaClass::Multipart => {
                // §17 Output B: one incremental pass buffers only bounded
                // scalar/JSON payloads; binary parts stay streaming.
                self.needs_multipart = true;
                if let Some(spec) = &content.multipart_spec {
                    for field in &spec.fields {
                        match &field.kind {
                            PlannedMultipartFieldKind::JsonPart(model) => {
                                self.model_types.extend(model_type_names(model));
                                self.needs_json_decode = true;
                            }
                            PlannedMultipartFieldKind::ScalarText(_) => {
                                // Even `String` parts decode through the
                                // strict-UTF-8 helper.
                                self.needs_multipart_scalar = true;
                                if let Some(name) = &field.scalar_validator {
                                    self.body_validator_fns.insert(name.clone());
                                }
                            }
                            PlannedMultipartFieldKind::BinaryPart => {}
                        }
                    }
                }
            }
            MediaClass::Binary | MediaClass::RawUnknown => {}
            // §6 request-direction streams: handlers receive a typed
            // `<Op><Framing>Input` wrapper decoding one record at a time
            // under `max_stream_record_bytes`.
            _ if content.stream.is_some() => {
                if let Some(framing) = content.stream.as_ref().map(|s| s.framing) {
                    self.request_framings.insert(framing);
                }
                if let Some(stream) = &content.stream {
                    self.note_payload(&stream.item_model_path, stream.item_from_views);
                }
            }
            // Planning rejects SSE/NDJSON/JSON-seq; they are later
            // deliverables and never reach us.
            _ => unreachable!(
                "planner emitted Phase 2 media class {:?}",
                content.media_class
            ),
        }
    }

    /// Companion §9: free validators live in super::models next to the
    /// model types; their imports follow the policy flag at emission time.
    fn import_body_validator(&mut self, content: &PlannedContent) {
        if let Some(PlannedBodyValidation::ScalarFn(name)) = &content.body_validation {
            self.body_validator_fns.insert(name.clone());
        }
    }

    /// Routes one payload type name into the right import bucket: shared
    /// models vs directional views (`super::views`, companion §5).
    fn note_payload(&mut self, expr: &str, from_views: bool) {
        if from_views {
            self.view_types.extend(model_type_names(expr));
        } else {
            self.model_types.extend(model_type_names(expr));
        }
    }

    fn scan_status(&mut self, status: &PlannedStatus) {
        if !status.headers.is_empty() {
            self.needs_typed_headers = true;
        }
        for content in effective_contents(status) {
            if content.is_wildcard {
                self.needs_any_response = true;
                continue;
            }
            match content.media_class {
                MediaClass::JsonFamily => {
                    self.note_payload(&content.model_expr, content.view.is_some());
                    self.needs_serialize_json = true;
                }
                MediaClass::PlainText => self.needs_encode_text = true,
                MediaClass::Binary | MediaClass::RawUnknown if content.codec.is_some() => {
                    // §45: codec-claimed response entries encode bounded
                    // through the generated per-codec helper.
                    let binding = content.codec.as_ref().expect("codec binding checked");
                    self.encode_codecs.insert(binding.plugin_id.to_owned());
                    let model_path = binding.model_path.clone();
                    let from_views = binding.model_from_views;
                    self.note_payload(&model_path, from_views);
                }
                MediaClass::Binary | MediaClass::RawUnknown => {
                    self.needs_stream_response = true;
                }
                // §18–§20 Output B: erased item-stream aliases encoded
                // per-item under `max_stream_record_bytes` (§40 contract).
                _ if content.stream.is_some() => {
                    if let Some(framing) = content.stream.as_ref().map(|s| s.framing) {
                        self.response_framings.insert(framing);
                    }
                    if let Some(stream) = &content.stream {
                        self.note_payload(&stream.item_model_path, stream.item_from_views);
                    }
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

fn emit_header(emitter: &mut Emitter, doc: &NormalizedDocument, flags: &Flags) {
    let mut docs = vec![
        "Axum server generated from the OpenAPI document (main spec §8 \
              Output B)."
            .to_owned(),
        String::new(),
        "Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form \
              extractor is never used — routes self-decode after the §28 \
             Content-Type dispatch), streaming raw payloads (§32), typed \
             documented response headers (§15: IntoResponse converts stored \
             domain values through the well-defined internal error path of \
             §48, firing the encode hook and emitting the fixed empty 500 on \
             failure), pre-handler protocol rejections outside the \
             documented enums (§39), identity-only inbound content coding \
             (§30.4), and the §28 Content-Type dispatch state machine. \
             Recorded decision for multi-content statuses WITH documented \
             headers: the typed fields hoist onto the status VARIANT beside \
             the content enum. The source document declares OpenAPI "
            .to_owned()
            + &doc.raw_version
            + ".",
    ];
    // Documented ONLY when this API consumes directional views, so
    // marker-free documents keep byte-identical output.
    if !flags.view_types.is_empty() {
        docs.push(String::new());
        docs.push(
            "Directional views (companion §5, main spec §50 test 50): request \
             bodies decode into `<M>Write` (required write-only fields are \
             mandatory there; required read-only fields are structurally \
             absent and surplus keys are ignored unless a schema declares \
             `additionalProperties: false`), response payloads carry \
             `<M>Read` (write-only fields never reach the wire), and decoded \
             request views run `validate_request()` before the handler. \
             Recorded trait contract: when `<M>Write` reconstructs the \
             shared model losslessly the router converts before invoking the \
             trait; otherwise the trait takes the view itself."
                .to_owned(),
        );
    }
    docs.push(
        "Generated deterministically byte-for-byte (main spec §50 test \
          39); do not edit by hand."
            .to_owned(),
    );
    emitter.inner_docs(0, &docs);
}

fn emit_imports(emitter: &mut Emitter, flags: &Flags, types: &TypesLocation) {
    let path_extractor = flags.needs_path_extractor;
    // Every import collects into ONE list that ends up crate-sorted the way
    // rustfmt's `reorder_imports` lays contiguous `use` items out; the
    // pre-existing push order already matches that sort, so default-config
    // documents stay byte-identical and codec use-lines simply slot in.
    let mut imports: Vec<String> = Vec::new();
    // Companion §9 free validators share the models import with the
    // model types — but only when the policy is ON and the calls exist.
    // rustfmt (2021 style) orders brace items LOWERCASE-INITIAL first, so
    // validator fn names precede type names; each run stays byte-sorted.
    let mut model_imports: Vec<&str> = flags.model_types.iter().map(String::as_str).collect();
    if flags.needs_request_schema_violation {
        model_imports.extend(flags.body_validator_fns.iter().map(String::as_str));
        model_imports.sort_unstable_by_key(|item| {
            (
                item.chars().next().is_some_and(char::is_uppercase),
                item.to_owned(),
            )
        });
        model_imports.dedup();
    }
    if !model_imports.is_empty() {
        imports.push(braced_use(
            &format!("use {}::", types.models_path()),
            &model_imports,
            &["super"],
        ));
    }
    // Directional views (companion §5): the views path sorts directly after
    // the models path under rustfmt's reorder_imports (both share their
    // first segment), so this slot keeps the block byte-stable.
    if !flags.view_types.is_empty() {
        let view_types: Vec<&str> = flags.view_types.iter().map(String::as_str).collect();
        imports.push(braced_use(
            &format!("use {}::", types.views_path()),
            &view_types,
            &["super"],
        ));
    }

    if flags.needs_into_response_trait {
        imports.push("use ::axum::response::IntoResponse;".to_owned());
    }
    if flags.needs_collect_body || flags.needs_collect_stream {
        imports.push(
            "use ::openapi_support::collect::{collect_body_limited, CollectLimitedError};"
                .to_owned(),
        );
    }
    if flags.needs_content_coding {
        imports.push(
            "use ::openapi_support::content_coding::ensure_identity_content_coding;".to_owned(),
        );
    }
    if flags.needs_serialize_json {
        imports.push("use ::openapi_support::encode::serialize_json_limited;".to_owned());
    }
    if flags.needs_form_decode {
        imports.push("use ::openapi_support::form::decode_form_limited;".to_owned());
    }
    if flags.has_operations {
        imports.push(
            "use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};"
                .to_owned(),
        );
        let mut stream_error_imports = Vec::new();
        if flags.request_framings.contains(&StreamFraming::JsonSeq) {
            imports.push("use ::openapi_support::jsonseq::decode_jsonseq;".to_owned());
            stream_error_imports.push("JsonSeqDecodeError");
        }
        if flags.response_framings.contains(&StreamFraming::JsonSeq) {
            imports.push("use ::openapi_support::jsonseq::encode_jsonseq_item;".to_owned());
        }
        imports.push("use ::openapi_support::limits::BodyLimits;".to_owned());
        if flags.needs_content_type_gate {
            imports.push(braced_use(
                "use ::openapi_support::mediatype::",
                &[
                    "is_wildcard_incoming",
                    "match_entry",
                    "parse_content_type",
                    "EntryMatch",
                    "ParsedMediaType",
                ],
                &[],
            ));
        }
        if flags.needs_multipart {
            imports.push(braced_use(
                "use ::openapi_support::multipart::",
                &[
                    "extract_boundary",
                    "stream_multipart",
                    "MultipartError",
                    "MultipartEvent",
                    "MultipartLimits",
                ],
                &[],
            ));
        }
        if flags.request_framings.contains(&StreamFraming::Ndjson) {
            imports.push("use ::openapi_support::ndjson::decode_ndjson;".to_owned());
            stream_error_imports.push("NdjsonDecodeError");
        }
        if flags.response_framings.contains(&StreamFraming::Ndjson) {
            imports.push("use ::openapi_support::ndjson::encode_ndjson_item;".to_owned());
        }
        push_params_import(&mut imports, flags);
        if flags.needs_peek {
            imports.push(
                "use ::openapi_support::peek::{detect_body_presence, BodyPresence};".to_owned(),
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
            || flags.needs_multipart
            || !flags.request_framings.is_empty()
            || !flags.decode_codecs.is_empty()
        {
            imports.push(
                "use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};".to_owned(),
            );
        } else {
            imports.push("use ::openapi_support::rejection::ProtocolRejection;".to_owned());
        }
        // rustfmt orders extern imports by crate name (openapi_support < std).
        if flags.request_framings.contains(&StreamFraming::Sse) {
            imports.push("use ::openapi_support::sse::decode_sse_json;".to_owned());
            stream_error_imports.push("SseDecodeError");
        }
        if flags.response_framings.contains(&StreamFraming::Sse) {
            imports.push("use ::openapi_support::sse::encode_sse_event;".to_owned());
        }
        if !stream_error_imports.is_empty() {
            // Decode-error enums live beside ServerStreamError (§40).
            stream_error_imports.sort_unstable();
            imports.push(braced_use(
                "use ::openapi_support::stream_errors::",
                &stream_error_imports,
                &[],
            ));
        }
        if !flags.response_framings.is_empty() {
            imports.push("use ::openapi_support::stream_errors::ServerStreamError;".to_owned());
        }
        if path_extractor {
            imports.push("use ::std::collections::HashMap;".to_owned());
        }
    }
    // §45 codec families: their use lines slot into rustfmt's canonical
    // crate-sorted position via the shared stable import sort.
    let mut codec_ids: Vec<String> = flags
        .decode_codecs
        .union(&flags.encode_codecs)
        .cloned()
        .collect();
    codec_ids.sort();
    if !codec_ids.is_empty() {
        let registry = codec_registry();
        for id in &codec_ids {
            if let Some(plugin) = registry.iter().find(|plugin| plugin.id() == id.as_str()) {
                imports.extend(plugin.emitted_use_lines());
            }
        }
    }
    // Sibling types keep the canonical push order above (`super::…` rides
    // the keyword bucket); an EXTERNAL base path joins the crate-name
    // bucket, so the same stable import sort re-slots every line exactly as
    // rustfmt would (main spec §45/§50).
    if !codec_ids.is_empty() || matches!(types, TypesLocation::External(_)) {
        imports.sort_by_key(|line| super::import_sort_key(line));
    }
    for import in &imports {
        emitter.block(0, import);
    }
}

/// One brace-import statement: collapsed when it fits within the rustfmt
/// maximum width, otherwise rustfmt's packed continuation form. The
/// `keyword_first` items (validator fn names inside the models brace group)
/// keep the lowercase-initial-first ordering rule of the original emitter.
fn braced_use(prefix: &str, items: &[&str], keyword_first: &[&str]) -> String {
    let _ = keyword_first;
    // Empirical rule (verified against the pinned toolchain): a braced `use`
    // tree stays on one line only up to [`RUSTFMT_MAX_WIDTH`] − 2 characters;
    // the canonical broken form packs the items onto one continuation line
    // when they fit under the same budget, else lists them one per line.
    const USE_BUDGET: usize = RUSTFMT_MAX_WIDTH - 2;
    if items.len() == 1 {
        // rustfmt drops redundant braces around a single use item.
        return format!("{prefix}{};", items[0]);
    }
    let joined = items.join(", ");
    if prefix.chars().count() + joined.chars().count() + 3 <= USE_BUDGET {
        return format!("{prefix}{{{joined}}};");
    }
    let mut text = format!("{prefix}{{\n");
    let packed = format!("{joined},");
    if 4 + packed.chars().count() <= USE_BUDGET {
        text.push_str(&format!("    {packed}\n"));
    } else {
        for item in items {
            text.push_str(&format!("    {item},\n"));
        }
    }
    text.push_str("};");
    text
}

fn push_params_import(imports: &mut Vec<String>, flags: &Flags) {
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
        imports.push(braced_use("use ::openapi_support::params::", &items, &[]));
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
        emit_request_body_enum(emitter, op_index, operation, layout, enum_name);
        first = false;
    }
    if let Some(name) = layout.wildcard_request(op_index) {
        if !first {
            emitter.blank();
        }
        emit_wildcard_request_struct(emitter, operation, name);
        first = false;
    }
    if let Some(name) = layout.multipart_input(op_index) {
        if !first {
            emitter.blank();
        }
        emit_multipart_types(emitter, op_index, operation, layout, name);
        first = false;
    }
    for (content_index, content) in operation.request_contents.iter().enumerate() {
        let Some(name) = layout.stream_input(op_index, content_index) else {
            continue;
        };
        if !first {
            emitter.blank();
        }
        emit_stream_input_struct(emitter, operation, content, name);
        first = false;
    }
    for (status_index, status) in operation.statuses.iter().enumerate() {
        for (content_index, content) in status.contents.iter().enumerate() {
            let Some(name) = layout.stream_alias(op_index, status_index, content_index) else {
                continue;
            };
            if !first {
                emitter.blank();
            }
            emit_stream_alias(emitter, operation, status, content, name);
            first = false;
        }
    }
    for (status_index, status) in operation.statuses.iter().enumerate() {
        if let Some(name) = layout.content_enum(op_index, status_index) {
            if !first {
                emitter.blank();
            }
            emit_content_enum(
                emitter,
                op_index,
                status_index,
                operation,
                status,
                layout,
                name,
            );
            first = false;
        }
        if let Some((name, shape)) = layout.wrapper(op_index, status_index) {
            if !first {
                emitter.blank();
            }
            emit_wrapper(
                emitter,
                op_index,
                status_index,
                operation,
                status,
                name,
                *shape,
                layout,
            );
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
/// streaming classes (§32); record-framed entries carry their erased
/// `<…>Stream` item-stream alias (§18–§20 Output B) on responses and the
/// typed `<Op><Framing>Input` wrapper on requests.
fn payload_type(content: &PlannedContent, stream_alias: Option<&str>) -> String {
    // §45 codec-claimed entries decode into the shared models.rs type.
    if let Some(binding) = &content.codec {
        return binding.model_path.clone();
    }
    match content.media_class {
        MediaClass::JsonFamily | MediaClass::UrlEncodedForm => content.model_expr.clone(),
        MediaClass::PlainText => "String".to_owned(),
        MediaClass::Binary | MediaClass::RawUnknown => "::axum::body::Body".to_owned(),
        _ if content.stream.is_some() && !content.is_wildcard => stream_alias
            .unwrap_or_else(|| panic!("stream alias registered for every exposed stream entry"))
            .to_owned(),
        _ => unreachable!(
            "planner emitted Phase 2 media class {:?}",
            content.media_class
        ),
    }
}

fn emit_request_body_enum(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    enum_name: &str,
) {
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
    for (content_index, content) in operation.request_contents.iter().enumerate() {
        if content.is_wildcard {
            emit_any_struct_variant(emitter, 1, &content.variant_name);
            continue;
        }
        let payload = match content.media_class {
            MediaClass::Multipart => layout.multipart_input(op_index).unwrap_or("()").to_owned(),
            _ if content.stream.is_some() => layout
                .stream_input(op_index, content_index)
                .expect("stream input registered")
                .to_owned(),
            _ => payload_type(content, None),
        };
        emitter.line(1, &format!("{}({payload}),", content.variant_name));
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
    op_index: usize,
    status_index: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    layout: &ServerLayout,
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
    if effective_contents(status)
        .iter()
        .all(|content| content.stream.is_none())
    {
        emitter.line(0, "#[derive(Debug)]");
    }
    emitter.line(0, &format!("pub enum {name} {{"));
    for (content_index, content) in effective_contents(status).iter().enumerate() {
        if content.is_wildcard {
            emit_any_struct_variant(emitter, 1, &content.variant_name);
            continue;
        }
        let alias = layout.stream_alias(op_index, status_index, content_index);
        emitter.line(
            1,
            &format!(
                "{}({}),",
                content.variant_name,
                payload_type(content, alias)
            ),
        );
    }
    emitter.line(0, "}");
}

#[allow(clippy::too_many_arguments)]
fn emit_wrapper(
    emitter: &mut Emitter,
    op_index: usize,
    status_index: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    name: &str,
    shape: WrapperShape,
    layout: &ServerLayout,
) {
    let payload_doc = match shape {
        WrapperShape::Stream => {
            "the body streams verbatim; typed documented-header fields ride \
             beside it where documented (main spec §15/§32)"
        }
        WrapperShape::Wildcard => {
            "`*/*` is not a concrete media type so the application supplies \
             the actual Content-Type (main spec §22); the body streams \
             verbatim"
        }
        WrapperShape::TypedHeaders => {
            "typed payload (main spec §15 Output B): required documented \
             headers as plain fields, optional ones as `Option<T>`, then the \
             body stored as a domain value for the bounded encoder"
        }
        WrapperShape::StreamHeaders => {
            "stream payload (§15/§40): required documented headers as plain \
             fields, optional ones as `Option<T>`, beside the erased \
             item-stream; the encoder commits status, Content-Type, and the \
             headers before streaming items one at a time"
        }
    };
    if shape == WrapperShape::TypedHeaders {
        emitter.docs(
            0,
            &[format!(
                "Payload for status {} of `{}`: {}.",
                crate::normalize::status_label(&status.key),
                operation.method,
                payload_doc
            )],
        );
    } else {
        emitter.docs(
            0,
            &[format!(
                "Payload for status {} of `{}` (main spec §32): {}.",
                crate::normalize::status_label(&status.key),
                operation.method,
                payload_doc
            )],
        );
    }
    if shape != WrapperShape::StreamHeaders {
        // StreamHeaders stores an erased item stream without `Debug`.
        emitter.line(0, "#[derive(Debug)]");
    }
    emitter.line(0, &format!("pub struct {name} {{"));
    emit_header_fields(emitter, status, 1, "pub ");
    match shape {
        // §22 Output B: the wildcard carries BOTH the application-supplied
        // Content-Type and the raw streaming body (`any_response` reads the
        // pair from this wrapper).
        WrapperShape::Wildcard => {
            emitter.line(1, "pub content_type: ::mime::Mime,");
            emitter.line(1, "pub body: ::axum::body::Body,");
        }
        // TypedHeaders wrappers store the DECODED domain value (§48's
        // sanctioned store-domain-values choice), not the raw body.
        WrapperShape::TypedHeaders => emitter.line(
            1,
            &format!(
                "pub body: {},",
                payload_type(
                    effective_contents(status).first().expect("single content"),
                    None
                )
            ),
        ),
        // §15/§40: typed documented headers beside the erased item-stream
        // alias; the encoder commits everything before attaching the body.
        WrapperShape::StreamHeaders => {
            let alias = layout.stream_alias(op_index, status_index, 0);
            emitter.line(
                1,
                &format!(
                    "pub body: {},",
                    payload_type(
                        effective_contents(status).first().expect("single content"),
                        alias
                    )
                ),
            );
        }
        WrapperShape::Stream => emitter.line(1, "pub body: ::axum::body::Body,"),
    }
    emitter.line(0, "}");
}

/// Typed documented-header fields of one status (main spec §15): required
/// headers become plain fields, optional ones `Option<T>`.
/// Typed documented-header fields of one status (main spec §15): required
/// headers become plain fields, optional ones `Option<T>`. `visibility` is
/// `pub` for wrapper STRUCTS; enum-variant fields share the enum's
/// visibility, so enum paths pass the empty prefix.
fn emit_header_fields(
    emitter: &mut Emitter,
    status: &PlannedStatus,
    indent: usize,
    visibility: &str,
) {
    for header in &status.headers {
        let field_type = if header.required {
            header.rust_type.clone()
        } else {
            format!("Option<{}>", header.rust_type)
        };
        emitter.docs(
            indent,
            &[format!(
                "Documented response header `{}` ({}).",
                header.wire_name,
                if header.required {
                    "required"
                } else {
                    "optional"
                }
            )],
        );
        emitter.line(
            indent,
            &format!("{visibility}{}: {field_type},", header.rust_name),
        );
    }
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
    if !has_stream_payload(operation) {
        // Erased item streams carry no `Debug`; derive it only when every
        // variant does.
        emitter.line(0, "#[derive(Debug)]");
    }
    emitter.line(0, &format!("pub enum {} {{", operation.response_enum_name));
    for (status_index, status) in operation.statuses.iter().enumerate() {
        emit_variant_doc(emitter, status);
        if let Some((wrapper, _shape)) = layout.wrapper(op_index, status_index) {
            emitter.line(1, &format!("{}({wrapper}),", status.enum_variant));
            continue;
        }
        let contents = effective_contents(status);
        if struct_variant_status(status) {
            // Ranges/default carry the wire status (§23–§24); documented
            // headers ride inside the struct variant (§15).
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emitter.line(2, "status: ::http::StatusCode,");
            emit_header_fields(emitter, status, 2, "");
            match contents.len() {
                0 => {}
                1 => {
                    let content = &contents[0];
                    if content.is_wildcard {
                        emitter.line(2, "content_type: ::mime::Mime,");
                        emitter.line(2, "body: ::axum::body::Body,");
                    } else {
                        let alias = layout.stream_alias(op_index, status_index, 0);
                        emitter.line(2, &format!("body: {},", payload_type(content, alias)));
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
        if !status.headers.is_empty() && contents.is_empty() {
            // Header-only documented response (e.g. 302 + Location): the
            // variant carries exactly the typed headers.
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emit_header_fields(emitter, status, 2, "");
            emitter.line(1, "},");
            continue;
        }
        if !status.headers.is_empty() && contents.len() >= 2 {
            // Multi-content with documented headers: the typed fields hoist
            // onto the STATUS VARIANT beside the content enum (recorded
            // decision; see module docs).
            let content_enum = layout
                .content_enum(op_index, status_index)
                .expect("registered");
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emit_header_fields(emitter, status, 2, "");
            emitter.line(2, &format!("content: {content_enum},"));
            emitter.line(1, "},");
            continue;
        }
        match contents.len() {
            0 => {
                emitter.line(1, &format!("{},", status.enum_variant));
            }
            1 => {
                let alias = layout.stream_alias(op_index, status_index, 0);
                emitter.line(
                    1,
                    &format!(
                        "{}({}),",
                        status.enum_variant,
                        payload_type(&contents[0], alias)
                    ),
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

/// Emits one erased `<Op><Status>[<Variant>]Stream` item-stream alias
/// (§18–§20 Output B, concrete-erased style): applications box their
/// producer into the alias and the §40 encoder consumes items one at a
/// time, so generic spread never leaks into trait signatures.
fn emit_stream_alias(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    content: &PlannedContent,
    name: &str,
) {
    let stream = content.stream.as_ref().expect("stream entry");
    let item = &stream.item_model_path;
    emitter.docs(
        0,
        &[format!(
            "Erased `{}` item stream for status {} of `{}` (main spec §{} \
             Output B / §40): failures after commit ride \
             `ServerStreamError` — no fabricated statuses.",
            stream.framing.as_snake(),
            crate::normalize::status_label(&status.key),
            operation.method,
            match stream.framing {
                StreamFraming::Sse => "18",
                StreamFraming::Ndjson => "19",
                StreamFraming::JsonSeq => "20",
            }
        )],
    );
    emitter.line(
        0,
        &format!("pub type {name} = ErasedItems<Result<{item}, ServerStreamError>>;"),
    );
}

/// True when any documented response payload of this operation is an erased
/// item-stream alias (no `Debug` implementation).
fn has_stream_payload(operation: &PlannedOperation) -> bool {
    operation.statuses.iter().any(|status| {
        !status.is_no_body_status
            && status
                .contents
                .iter()
                .any(|content| content.stream.is_some())
    })
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
    if operation
        .statuses
        .iter()
        .any(|status| !status.headers.is_empty())
    {
        // Typed-header arms build their wire list imperatively; clippy's
        // vec-init-then-push fires when every documented header is
        // required, so the encoder opts out locally.
        emitter.line(1, "#[allow(clippy::vec_init_then_push)]");
    }
    emitter.line(1, "pub fn into_response_with_limits(");
    emitter.line(2, "self,");
    // Arms that bound-encode payloads (textual/codec contents, per-item
    // stream encoding) consume `limits`; typed-header commits additionally
    // consume the overflow `hook`. Bare-status or pure-passthrough arms
    // touch neither, so each parameter underscores independently to keep
    // `-D warnings` clean (header-only HEAD outcomes use only the hook).
    let has_stream_status = operation.statuses.iter().any(|status| {
        !status.is_no_body_status
            && status
                .contents
                .iter()
                .any(|content| content.stream.is_some())
    });
    let encodes_payloads = operation.statuses.iter().any(|status| {
        effective_contents(status).iter().any(|content| {
            (matches!(
                content.media_class,
                MediaClass::JsonFamily | MediaClass::PlainText
            ) && !content.is_wildcard)
                || content.codec.is_some()
        }) || (!status.is_no_body_status && status.contents.iter().any(|c| c.stream.is_some()))
    });
    let hook_uses = operation.statuses.iter().any(|status| {
        !status.headers.is_empty() || {
            effective_contents(status).iter().any(|content| {
                (matches!(
                    content.media_class,
                    MediaClass::JsonFamily | MediaClass::PlainText
                ) && !content.is_wildcard)
                    || content.codec.is_some()
            }) || (!status.is_no_body_status && status.contents.iter().any(|c| c.stream.is_some()))
        }
    });
    let limits_name = if encodes_payloads {
        "limits"
    } else {
        "_limits"
    };
    let hook_name = if hook_uses { "hook" } else { "_hook" };
    emitter.line(2, &format!("{limits_name}: &BodyLimits,"));
    emitter.line(2, &format!("{hook_name}: &dyn EncodeOverflowHook,"));
    if has_stream_status {
        // §40: fired when a committed stream fails mid-production; the
        // encoder terminates the body abruptly afterward.
        emitter.line(
            2,
            "stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,",
        );
    }
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

    // §48 checked constructors on typed wrapper payloads: String-typed
    // headers can always fail `HeaderValue` conversion, so `new` validates
    // eagerly and returns `Err(InvalidResponseHeader)` instead of letting a
    // bad value surface only at encode time.
    let mut first_wrapper_ctor = true;
    for (status_index, _status) in operation.statuses.iter().enumerate() {
        if layout
            .wrapper(op_index, status_index)
            .map(|(_, shape)| *shape)
            != Some(WrapperShape::TypedHeaders)
        {
            continue;
        }
        if first_wrapper_ctor {
            emitter.blank();
            emitter.docs(
                0,
                &["Checked payload constructors validating every convertible \
                   documented header eagerly (main spec §15/§48)."
                    .to_owned()],
            );
            first_wrapper_ctor = false;
        } else {
            emitter.blank();
        }
        emit_wrapper_checked_ctor(emitter, operation, layout, op_index, status_index);
    }

    emitter.blank();
    emitter.line(
        0,
        &format!("impl ::axum::response::IntoResponse for {enum_name} {{"),
    );
    emitter.line(1, "fn into_response(self) -> ::axum::response::Response {");
    let has_stream_status = operation.statuses.iter().any(|status| {
        !status.is_no_body_status
            && status
                .contents
                .iter()
                .any(|content| content.stream.is_some())
    });
    if has_stream_status {
        emitter.line(2, "self.into_response_with_limits(");
        emitter.line(3, "&BodyLimits::process_default(),");
        emitter.line(3, "&NoOpEncodeOverflowHook,");
        emitter.line(
            3,
            "::std::sync::Arc::new(::openapi_support::hooks::NoOpStreamFailureHook),",
        );
        emitter.line(2, ")");
    } else {
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
    callee: String,
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
            callee: "encode_json_limited".to_owned(),
            args: [head, vec![format!("&{access}")], tail].concat(),
        },
        MediaClass::PlainText => EncodeCall {
            callee: "encode_text_limited".to_owned(),
            args: [head, vec![format!("&{access}")], tail].concat(),
        },
        // §45 codec-claimed entries encode through their generated per-codec
        // helper with the SAME argument shape (bounded, hook-observed).
        _ if content.codec.is_some() => EncodeCall {
            callee: format!(
                "{}_encode_limited",
                helper_prefix(
                    content
                        .codec
                        .as_ref()
                        .expect("codec binding checked")
                        .plugin_id
                )
            ),
            args: [head, vec![format!("&{access}")], tail].concat(),
        },
        // The caller passes the exact expression of the raw axum body; no
        // extra field access is appended here.
        MediaClass::Binary | MediaClass::RawUnknown => EncodeCall {
            callee: "stream_response".to_owned(),
            args: [head, vec![access.to_owned()]].concat(),
        },
        other => unreachable!("Phase 2 media class {other:?}"),
    }
}

/// Emits the [`StreamBodyEncoder`] constructor shared by every committed-
/// stream encoder arm (§40).
fn emit_stream_body_encoder_ctor(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Builds one [`StreamBodyEncoder`] over an application producer \
             (§40)."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[allow(clippy::too_many_arguments)]");
    emitter.line(0, "fn stream_body_encoder<T>(");
    emitter.line(1, "items: ErasedItems<Result<T, ServerStreamError>>,");
    emitter.line(1, "limit: usize,");
    emitter.line(
        1,
        "hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,",
    );
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(
        1,
        "encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,",
    );
    emitter.line(0, ") -> StreamBodyEncoder<T> {");
    emitter.line(1, "StreamBodyEncoder {");
    emitter.line(2, "items,");
    emitter.line(2, "limit,");
    emitter.line(2, "hook,");
    emitter.line(2, "operation_id,");
    emitter.line(2, "encode,");
    emitter.line(2, "finished: false,");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn any_call(status_arg: &str) -> EncodeCall {
    EncodeCall {
        callee: "any_response".to_owned(),
        args: vec![
            status_arg.to_owned(),
            "content_type".to_owned(),
            "body".to_owned(),
        ],
    }
}

/// Emits the §40 committed-stream body block: status + Content-Type literal
/// (+ typed documented headers when present) commit FIRST, then the body
/// pulls items one at a time, encoding each through the bounded per-item
/// encoder (`max_stream_record_bytes`). An item encode overflow or an
/// application error fires the stream-failure hook and TERMINATES the body
/// abruptly — no fabricated statuses, no in-band error frames, never partial
/// item bytes. `typed_headers` selects the §15 header-write step between the
/// Content-Type commit and the body attachment.
#[allow(clippy::too_many_arguments)]
fn emit_stream_body_block(
    emitter: &mut Emitter,
    indent: usize,
    status_expr: &str,
    items_expr: &str,
    content: &PlannedContent,
    op_id: &str,
    variant: &str,
    limits_arg: &str,
    typed_headers: bool,
) {
    let stream = content.stream.as_ref().expect("stream entry");
    let ct_literal = rust_string_literal(&content.media_type_literal);
    let encode_fn = match stream.framing {
        StreamFraming::Sse => "encode_sse_event",
        StreamFraming::Ndjson => "encode_ndjson_item",
        StreamFraming::JsonSeq => "encode_jsonseq_item",
    };
    let item = &stream.item_model_path;
    emitter.line(
        indent,
        &format!("let mut encoded = {status_expr}.into_response();"),
    );
    emitter.line(indent, "encoded.headers_mut().insert(");
    emitter.line(indent + 1, "::http::header::CONTENT_TYPE,");
    emitter.line(
        indent + 1,
        &format!("::http::HeaderValue::from_static({ct_literal}),"),
    );
    emitter.line(indent, ");");
    if typed_headers {
        emitter.line(indent, "let encoded = write_typed_headers(");
        emitter.line(indent + 1, "encoded,");
        emitter.line(indent + 1, "hook,");
        emitter.line(indent + 1, &format!("{},", rust_string_literal(op_id)));
        emitter.line(indent + 1, &format!("{},", rust_string_literal(variant)));
        emitter.line(indent + 1, "&typed_headers,");
        emitter.line(indent, ");");
    }
    // §40 step 2–3: pull, encode per-item, fire hook, terminate abruptly.
    emitter.line(
        indent,
        "*encoded.body_mut() = ::axum::body::Body::from_stream(stream_body_encoder(",
    );
    emitter.line(indent + 1, &format!("{items_expr},"));
    emitter.line(
        indent + 1,
        &format!("{limits_arg}.max_stream_record_bytes,"),
    );
    emitter.line(indent + 1, "::std::sync::Arc::clone(&stream_failure_hook),");
    emitter.line(indent + 1, &format!("{},", rust_string_literal(op_id)));
    emitter.line(indent + 1, &format!("{encode_fn}::<{item}>,"));
    emitter.line(indent, "));");
    emitter.line(indent, "encoded");
}

/// The erased item-stream encoder shared by every generated route (§40):
/// yields one encoded chunk per item; on `EncodeTooLarge` or an application
/// [`ServerStreamError`] it fires the configured
/// [`StreamFailureHook`](::openapi_support::hooks::StreamFailureHook) with
/// the operation id and returns the failure as a terminal body error so the
/// connection aborts (clients observe truncation distinct from clean EOF).
fn emit_stream_body_encoder(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[String::from(
            "Per-item encoder over one erased item-stream (main spec §40): \
             each item serializes under `max_stream_record_bytes`; overflow \
             or an application error fires the stream-failure hook and ends \
             the body abnormally — the committed status can never change.",
        )],
    );
    emitter.line(0, "struct StreamBodyEncoder<T> {");
    emitter.line(1, "items: ErasedItems<Result<T, ServerStreamError>>,");
    emitter.line(1, "limit: usize,");
    emitter.line(
        1,
        "hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,",
    );
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(
        1,
        "encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,",
    );
    emitter.line(1, "finished: bool,");
    emitter.line(0, "}");
    emitter.blank();
    emitter.line(
        0,
        "impl<T: serde::Serialize> ::futures_core::Stream for StreamBodyEncoder<T> {",
    );
    emitter.line(1, "type Item = Result<::bytes::Bytes, ServerStreamError>;");
    emitter.blank();
    emitter.line(1, "fn poll_next(");
    emitter.line(2, "self: ::std::pin::Pin<&mut Self>,");
    emitter.line(2, "cx: &mut ::core::task::Context<'_>,");
    emitter.line(1, ") -> ::core::task::Poll<Option<Self::Item>> {");
    emitter.line(2, "let this = self.get_mut();");
    emitter.line(2, "if this.finished {");
    emitter.line(3, "::core::task::Poll::Ready(None)");
    emitter.line(2, "} else {");
    emitter.line(
        3,
        "match ::futures_core::Stream::poll_next(this.items.as_mut(), cx) {",
    );
    emitter.line(
        4,
        "::core::task::Poll::Pending => ::core::task::Poll::Pending,",
    );
    emitter.line(4, "::core::task::Poll::Ready(None) => {");
    emitter.line(5, "this.finished = true;");
    emitter.line(5, "::core::task::Poll::Ready(None)");
    emitter.line(4, "}");
    // A block arm with STATEMENTS stays block form under rustfmt.
    emitter.line(4, "::core::task::Poll::Ready(Some(Ok(item))) => {");
    emitter.line(5, "let encoded = (this.encode)(&item, this.limit);");
    emitter.line(5, "match encoded {");
    emitter.line(
        6,
        "Ok(bytes) => ::core::task::Poll::Ready(Some(Ok(bytes))),",
    );
    emitter.line(6, "Err(error) => {");
    emitter.line(7, "this.finished = true;");
    emitter.line(7, "this.hook.on_stream_failure(this.operation_id, &error);");
    emitter.line(7, "let failure = ServerStreamError::new(error);");
    emitter.line(7, "::core::task::Poll::Ready(Some(Err(failure)))");
    emitter.line(6, "}");
    emitter.line(5, "}");
    emitter.line(4, "}");
    emitter.line(4, "::core::task::Poll::Ready(Some(Err(error))) => {");
    emitter.line(5, "this.finished = true;");
    emitter.line(5, "this.hook.on_stream_failure(this.operation_id, &error);");
    emitter.line(5, "::core::task::Poll::Ready(Some(Err(error)))");
    emitter.line(4, "}");
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(0, "}");
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

    // Streaming/wildcard/typed-header wrappers (§32/§22/§15). The arm binds
    // the wrapper value as `wrapper`, so field accesses must use that
    // binding, not the type.
    if let Some((_, shape)) = layout.wrapper(op_index, status_index) {
        let literal = rust_string_literal(&contents[0].media_type_literal);
        let call = match shape {
            WrapperShape::Stream => EncodeCall {
                callee: "stream_response".to_owned(),
                args: vec![constant.clone(), literal, "wrapper.body".to_owned()],
            },
            WrapperShape::Wildcard => EncodeCall {
                callee: "any_response".to_owned(),
                args: vec![
                    constant.clone(),
                    "wrapper.content_type".to_owned(),
                    "wrapper.body".to_owned(),
                ],
            },
            WrapperShape::TypedHeaders => structured_call(
                &contents[0],
                &constant,
                // structured_call adds the `&` itself.
                "wrapper.body",
                "limits",
                op_id,
                variant,
            ),
            // §15/§40: commit status, Content-Type, typed headers, then the
            // per-item encoded stream over the erased alias.
            WrapperShape::StreamHeaders => {
                emitter.line(3, &format!("Self::{variant}(wrapper) => {{"));
                emit_typed_pushes(emitter, 4, status, HeaderSource::Wrapper);
                emit_stream_body_block(
                    emitter,
                    4,
                    &constant,
                    "wrapper.body",
                    &contents[0],
                    op_id,
                    variant,
                    "limits",
                    true,
                );
                emitter.line(3, "}");
                return;
            }
        };
        if !status.headers.is_empty() {
            // §15 with the §48 recorded decision: IntoResponse converts the
            // stored domain values; a failing conversion fires the hook and
            // emits the fixed empty 500 (same machinery as §34.1).
            emitter.line(3, &format!("Self::{variant}(wrapper) => {{"));
            emit_typed_pushes(emitter, 4, status, HeaderSource::Wrapper);
            emit_let_call(emitter, 4, "encoded", &call);
            emit_header_write(emitter, 4, op_id, variant);
            emitter.line(3, "}");
            return;
        }
        emit_call_arm_expr(emitter, 3, &format!("Self::{variant}(wrapper)"), &call);
        return;
    }

    match contents.len() {
        // Unit statuses (§35) and header-only statuses (§15).
        0 => {
            let inline = format!("Self::{variant} => {constant}.into_response(),");
            if status.headers.is_empty() {
                if fits(3, &inline) {
                    emitter.line(3, &inline);
                } else {
                    emitter.line(3, &format!("Self::{variant} => {{"));
                    emitter.line(4, &format!("let status = {constant};"));
                    emitter.line(4, "status.into_response()");
                    emitter.line(3, "}");
                }
            } else {
                // Header-only variant: write the typed headers beside the
                // bare status. Multi-field struct PATTERNS in block arms go
                // vertical under rustfmt (canonical layout verified against
                // fixture 17); single-field patterns stay inline.
                let names: Vec<String> =
                    status.headers.iter().map(|h| h.rust_name.clone()).collect();
                if names.len() <= 1 {
                    emitter.line(
                        3,
                        &format!("Self::{variant} {{ {} }} => {{", names.join(", ")),
                    );
                } else {
                    emitter.line(3, &format!("Self::{variant} {{"));
                    for name in &names {
                        emitter.line(4, &format!("{name},"));
                    }
                    emitter.line(3, "} => {");
                }
                emit_typed_pushes(emitter, 4, status, HeaderSource::Local);
                emitter.line(4, &format!("let encoded = {constant}.into_response();"));
                emit_header_write(emitter, 4, op_id, variant);
                emitter.line(3, "}");
            }
        }
        1 => {
            // Record-framed stream payloads (§18–§20 Output B) commit per §40
            // and encode items one at a time.
            if contents[0].stream.is_some() {
                emitter.line(3, &format!("Self::{variant}(items) => {{"));
                emit_stream_body_block(
                    emitter,
                    4,
                    &constant,
                    "items",
                    &contents[0],
                    op_id,
                    variant,
                    "limits",
                    false,
                );
                emitter.line(3, "}");
                return;
            }
            // Remaining single-content statuses are wrapper statuses handled
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
            if !status.headers.is_empty() {
                // Headers hoist onto the status VARIANT (recorded decision):
                // bind them once, then append them to every negotiated
                // representation's response.
                let mut names: Vec<String> = status
                    .headers
                    .iter()
                    .map(|header| header.rust_name.clone())
                    .collect();
                names.push("content".to_owned());
                emit_struct_pattern(emitter, 3, &format!("Self::{variant}"), &names);
                emit_typed_pushes(emitter, 4, status, HeaderSource::Local);
                emitter.line(4, "match content {");
                emit_nested_content_arms(
                    emitter,
                    content_enum,
                    contents,
                    &constant,
                    "limits",
                    op_id,
                    variant,
                    5,
                    true,
                );
                emitter.line(4, "}");
                emitter.line(3, "}");
                return;
            }
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
                false,
            );
            emitter.line(3, "},");
        }
    }
}

/// Where typed header values live inside an encode arm: wrapper struct
/// fields (`wrapper.<name>`) or hoisted locals (`<name>`).
#[derive(Debug, Clone, Copy)]
enum HeaderSource {
    Wrapper,
    Local,
}

fn header_source_expr(source: HeaderSource, rust_name: &str) -> String {
    match source {
        HeaderSource::Wrapper => format!("wrapper.{rust_name}"),
        HeaderSource::Local => rust_name.to_owned(),
    }
}

/// Builds the `Vec<(&'static str, String)>` of documented header values:
/// strings clone directly; scalars format through Display. Optional fields
/// contribute only when present.
fn emit_typed_pushes(
    emitter: &mut Emitter,
    indent: usize,
    status: &PlannedStatus,
    source: HeaderSource,
) {
    emitter.line(
        indent,
        "let mut typed_headers = Vec::<(&'static str, String)>::new();",
    );
    for header in &status.headers {
        let wire = rust_string_literal(&header.wire_name.to_ascii_lowercase());
        let access = header_source_expr(source, &header.rust_name);
        let value_expr = if header.rust_type == "String" {
            format!("{access}.clone()")
        } else {
            format!("{access}.to_string()")
        };
        if header.required {
            let line = format!("typed_headers.push(({wire}, {value_expr}));");
            if fits(indent, &line) {
                emitter.line(indent, &line);
            } else {
                emitter.line(indent, "typed_headers.push((");
                emitter.line(indent + 1, &format!("{wire},"));
                emitter.line(indent + 1, &format!("{value_expr},"));
                emitter.line(indent, "));");
            }
        } else {
            emitter.line(
                indent,
                &format!("if let Some(value) = {}.as_ref() {{", access),
            );
            let line = format!("typed_headers.push(({wire}, value.to_owned()));");
            if header.rust_type == "String" && fits(indent + 1, &line) {
                emitter.line(indent + 1, &line);
            } else if header.rust_type == "String" {
                emitter.line(indent + 1, "typed_headers.push((");
                emitter.line(indent + 2, &format!("{wire},"));
                emitter.line(indent + 2, "value.clone(),");
                emitter.line(indent + 1, "));");
            } else if fits(indent + 1, &line.replace("to_owned", "to_string")) {
                emitter.line(
                    indent + 1,
                    &format!("typed_headers.push(({wire}, value.to_string()));"),
                );
            } else {
                emitter.line(indent + 1, "typed_headers.push((");
                emitter.line(indent + 2, &format!("{wire},"));
                emitter.line(indent + 2, "value.to_string(),");
                emitter.line(indent + 1, "));");
            }
            emitter.line(indent, "}");
        }
    }
}

/// Appends the collected typed headers to the encoded response; a failing
/// `HeaderValue` conversion takes the §34.1-style fallback path.
fn emit_header_write(emitter: &mut Emitter, indent: usize, op_id: &str, variant: &str) {
    let line = format!(
        "write_typed_headers(encoded, hook, {}, {}, &typed_headers)",
        rust_string_literal(op_id),
        rust_string_literal(variant)
    );
    // The argument list must also respect rustfmt's `fn_call_width` budget,
    // which forces vertical arguments even when the whole call would fit.
    let args_len = format!(
        "encoded, hook, {}, {}, &typed_headers",
        rust_string_literal(op_id),
        rust_string_literal(variant)
    )
    .chars()
    .count();
    if fits(indent, &line)
        && args_len <= FN_CALL_WIDTH
        && line.chars().count() + indent * 4 < RUSTFMT_MAX_WIDTH
    {
        emitter.line(indent, &line);
    } else {
        emitter.line(indent, "write_typed_headers(");
        emitter.line(indent + 1, "encoded,");
        emitter.line(indent + 1, "hook,");
        emitter.line(indent + 1, &format!("{},", rust_string_literal(op_id)));
        emitter.line(indent + 1, &format!("{},", rust_string_literal(variant)));
        emitter.line(indent + 1, "&typed_headers,");
        emitter.line(indent, ")");
    }
}

/// Nested content-enum arms: every representation encodes with its own
/// Content-Type literal (main spec §11/§41); wildcards stream with the
/// application-supplied mime (§22). When `with_headers` is set, each arm
/// binds its encoded response and appends the hoisted typed headers.
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
    with_headers: bool,
) {
    for content in contents {
        if content.is_wildcard {
            let pattern = format!(
                "{content_enum}::{} {{ content_type, body }}",
                content.variant_name
            );
            if with_headers {
                emit_header_write_arm(
                    emitter,
                    indent,
                    &pattern,
                    &any_call(status_arg),
                    op_id,
                    variant,
                );
            } else {
                emit_call_arm_expr(emitter, indent, &pattern, &any_call(status_arg));
            }
            continue;
        }
        // The arm binds the variant payload as `value`; streaming payloads
        // ARE the raw axum body, so `value` is passed through unchanged.
        // Record-framed streams commit per §40 and encode items lazily.
        if !content.is_wildcard && content.stream.is_some() {
            let pattern = format!("{content_enum}::{}(items)", content.variant_name);
            emitter.line(indent, &format!("{pattern} => {{"));
            emit_stream_body_block(
                emitter,
                indent + 1,
                status_arg,
                "items",
                content,
                op_id,
                variant,
                limits_arg,
                with_headers,
            );
            emitter.line(indent, "}");
            continue;
        }
        let call = structured_call(content, status_arg, "value", limits_arg, op_id, variant);
        let pattern = format!("{content_enum}::{}(value)", content.variant_name);
        if with_headers {
            emit_header_write_arm(emitter, indent, &pattern, &call, op_id, variant);
        } else {
            emit_call_arm_expr(emitter, indent, &pattern, &call);
        }
    }
}

/// One nested arm that binds `encoded` then appends the typed headers.
fn emit_header_write_arm(
    emitter: &mut Emitter,
    indent: usize,
    pattern: &str,
    call: &EncodeCall,
    op_id: &str,
    variant: &str,
) {
    emitter.line(indent, &format!("{pattern} => {{"));
    emit_let_call(emitter, indent + 1, "encoded", call);
    emit_header_write(emitter, indent + 1, op_id, variant);
    emitter.line(indent, "}"); // trailing comma handled by caller context
}

/// Emits `let <binding> = <call>;` with rustfmt-canonical wrapping: inline
/// when it fits, otherwise the argument list breaks vertically and the
/// semicolon rides the closing paren.
fn emit_let_call(emitter: &mut Emitter, indent: usize, binding: &str, call: &EncodeCall) {
    let joined = call.args.join(", ");
    let inline = format!("let {binding} = {}({joined});", call.callee);
    if fits(indent, &inline) {
        emitter.line(indent, &inline);
        return;
    }
    emitter.line(indent, &format!("let {binding} = {}(", call.callee));
    for arg in &call.args {
        emitter.line(indent + 1, &format!("{arg},"));
    }
    emitter.line(indent, ");");
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

    let mut field_names: Vec<String> = vec!["status".to_owned()];
    for header in &status.headers {
        field_names.push(header.rust_name.clone());
    }
    match contents.len() {
        0 => {}
        1 if contents[0].is_wildcard => {
            field_names.push("content_type".to_owned());
            field_names.push("body".to_owned());
        }
        _ => field_names.push("body".to_owned()),
    }
    emit_struct_pattern(emitter, 3, &format!("Self::{variant}"), &field_names);
    emitter.line(4, "debug_assert!(");
    emitter.line(5, &format!("{assertion},"));
    emitter.line(5, &format!("{},", rust_string_literal(&message)));
    emitter.line(4, ");");

    // Documented headers append to every encoded representation (§15).
    let has_headers = !status.headers.is_empty();
    if has_headers {
        emit_typed_pushes(emitter, 4, status, HeaderSource::Local);
    }

    match contents.len() {
        0 => {
            if has_headers {
                emitter.line(4, "let encoded = status.into_response();");
                emit_header_write(emitter, 4, op_id, variant);
            } else {
                emitter.line(4, "status.into_response()");
            }
        }
        1 => {
            let content = &contents[0];
            if !content.is_wildcard && content.stream.is_some() {
                // §40 committed-stream contract on range/default payloads.
                emit_stream_body_block(
                    emitter,
                    4,
                    "status",
                    "body",
                    content,
                    op_id,
                    variant,
                    "limits",
                    has_headers,
                );
            } else {
                let call = if content.is_wildcard {
                    any_call("status")
                } else {
                    structured_call(content, "status", "body", "limits", op_id, variant)
                };
                if has_headers {
                    emit_let_call(emitter, 4, "encoded", &call);
                    emit_header_write(emitter, 4, op_id, variant);
                } else {
                    emit_call_at(emitter, 4, &call);
                }
            }
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
                has_headers,
            );
            emitter.line(4, "}");
        }
    }
    emitter.line(3, "}");
}

/// Checked constructor validating the carried status (main spec §48).
/// Documented headers ride as plain domain-value parameters: per §48's
/// sanctioned alternative they are stored verbatim and validated at
/// encode time through the well-defined internal error path.
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

    let mut params = vec!["status: ::http::StatusCode".to_owned()];
    for header in &status.headers {
        let field_type = if header.required {
            header.rust_type.clone()
        } else {
            format!("Option<{}>", header.rust_type)
        };
        params.push(format!("{}: {field_type}", header.rust_name));
    }
    if !contents.is_empty() {
        params.push(format!(
            "body: {}",
            ctor_body_type(operation, layout, op_index, status_index)
        ));
    }

    let ctor_fields = |status: &PlannedStatus| -> Vec<String> {
        let mut names = vec!["status".to_owned()];
        for header in &status.headers {
            names.push(header.rust_name.clone());
        }
        let contents = effective_contents(status);
        match contents.len() {
            0 => {}
            1 if contents[0].is_wildcard => {
                names.push("content_type".to_owned());
                names.push("body".to_owned());
            }
            _ => names.push("body".to_owned()),
        }
        names
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
            emit_struct_literal(
                emitter,
                3,
                &format!("Ok(Self::{variant}"),
                &ctor_fields(status),
                ")",
            );
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
            emit_struct_literal(
                emitter,
                2,
                &format!("Ok(Self::{}", status.enum_variant),
                &ctor_fields(status),
                ")",
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
                let alias = layout.stream_alias(op_index, status_index, 0);
                payload_type(content, alias)
            }
        }
        _ => layout
            .content_enum(op_index, status_index)
            .expect("content enum registered for multi-content range/default")
            .to_owned(),
    }
}

/// Checked constructor on a §15 typed wrapper payload: validates every
/// String-typed header eagerly (main spec §48); scalar fields cannot fail
/// conversion and skip validation.
fn emit_wrapper_checked_ctor(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    op_index: usize,
    status_index: usize,
) {
    let status = &operation.statuses[status_index];
    let (wrapper, _) = layout
        .wrapper(op_index, status_index)
        .expect("typed wrapper registered");
    let contents = effective_contents(status);

    let mut params: Vec<String> = Vec::new();
    for header in &status.headers {
        let field_type = if header.required {
            header.rust_type.clone()
        } else {
            format!("Option<{}>", header.rust_type)
        };
        params.push(format!("{}: {field_type}", header.rust_name));
    }
    params.push(format!(
        "body: {}",
        match contents.len() {
            1 => payload_type(&contents[0], None),
            _ => layout
                .content_enum(op_index, status_index)
                .expect("registered")
                .to_owned(),
        }
    ));
    emit_ctor_signature_named(
        emitter,
        wrapper,
        "new",
        &params,
        "Result<Self, ::openapi_support::response_headers::InvalidResponseHeader>",
    );
    for header in &status.headers {
        if header.rust_type != "String" {
            continue;
        }
        if header.required {
            emitter.line(
                2,
                &format!(
                    "::openapi_support::response_headers::checked_value({}, &{})?;",
                    rust_string_literal(&header.wire_name.to_ascii_lowercase()),
                    header.rust_name
                ),
            );
        } else {
            emitter.line(
                2,
                &format!("if let Some(value) = {}.as_ref() {{", header.rust_name),
            );
            emitter.line(
                3,
                &format!(
                    "::openapi_support::response_headers::checked_value({}, value)?;",
                    rust_string_literal(&header.wire_name.to_ascii_lowercase())
                ),
            );
            emitter.line(2, "}");
        }
    }
    let mut fields: Vec<String> = status
        .headers
        .iter()
        .map(|header| header.rust_name.clone())
        .collect();
    fields.push("body".to_owned());
    emit_struct_literal(emitter, 2, "Ok(Self", &fields, ")");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Emits a struct-variant match pattern with rustfmt's observed canonical
/// layout: one line for at most two fields, vertical beyond that; always
/// closes with `} => {`.
fn emit_struct_pattern(emitter: &mut Emitter, indent: usize, head: &str, fields: &[String]) {
    let joined = fields.join(", ");
    let inline = format!("{head} {{ {joined} }} => {{");
    if fields.len() <= 2 && fits(indent, &inline) {
        emitter.line(indent, &inline);
        return;
    }
    emitter.line(indent, &format!("{head} {{"));
    for field in fields {
        emitter.line(indent + 1, &format!("{field},"));
    }
    emitter.line(indent, "} => {");
}

/// Emits a struct-literal expression with rustfmt's observed canonical
/// layout: one line for at most two short fields, vertical otherwise
/// (`struct_lit_width` keeps three-plus-field literals broken).
fn emit_struct_literal(
    emitter: &mut Emitter,
    indent: usize,
    head: &str,
    fields: &[String],
    tail: &str,
) {
    let joined = fields.join(", ");
    let inline = format!("{head} {{ {joined} }}{tail}");
    if fields.len() <= 2 && fits(indent, &inline) {
        emitter.line(indent, &inline);
        return;
    }
    emitter.line(indent, &format!("{head} {{"));
    for field in fields {
        emitter.line(indent + 1, &format!("{field},"));
    }
    emitter.line(indent, &format!("}}{tail}"));
}

/// Checked-constructor signature with an explicit return type, collapsing
/// when it fits.
fn emit_ctor_signature_named(
    emitter: &mut Emitter,
    self_type: &str,
    method: &str,
    params: &[String],
    ok_type: &str,
) {
    emitter.docs(
        0,
        &[format!("Checked constructor for [`{self_type}`] (§48).")],
    );
    emitter.line(0, &format!("impl {self_type} {{"));
    let joined = params.join(", ");
    let inline = format!("pub fn {method}({joined}) -> {ok_type} {{");
    if fits(1, &inline) {
        emitter.line(1, &inline);
    } else {
        emitter.line(1, &format!("pub fn {method}("));
        for param in params {
            emitter.line(2, &format!("{param},"));
        }
        emitter.line(1, &format!(") -> {ok_type} {{"));
    }
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
    if content.media_class == MediaClass::Multipart {
        let name = layout.multipart_input(op_index).expect("registered");
        return wrap(name.to_owned());
    }
    if let Some(name) = layout.stream_input(op_index, 0) {
        // §6 request-direction streams: handlers drain a typed item-stream
        // wrapper (D-impl-request-direction-streams).
        return wrap(name.to_owned());
    }
    // Directional-view contract (companion §5, recorded decision in the
    // module docs): a lossless write view hands the router the shared model
    // after conversion; otherwise the trait sees the decoded view itself.
    if let Some(view) = &content.view {
        if view.write_lossless {
            return wrap(view.shared_type.clone());
        }
        return wrap(content.model_expr.clone());
    }
    wrap(payload_type(content, None))
}

fn emit_state(emitter: &mut Emitter, api_trait: &str, has_streams: bool) {
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
    if has_streams {
        // Read only by committed-stream encoders (§40); present only when
        // some operation streams a record-framed response.
        emitter.line(
            1,
            "stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,",
        );
    }
    emitter.line(0, "}");
}

/// Emits one typed `<Op><Framing>Input` streaming request wrapper
/// (§6 request-direction streams; D-impl-request-direction-streams):
/// `next_item` decodes ONE record at a time under `max_stream_record_bytes`
/// and maps per-record decode failures onto the §39 table — oversized
/// records → 413 BodyTooLarge; malformed JSON, non-UTF-8 bytes, truncation
/// (the client aborting mid-record), transport failures, and a missing
/// record separator → 400 MalformedBody. Nothing aggregates the body.
fn emit_stream_input_struct(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    content: &PlannedContent,
    name: &str,
) {
    let stream = content.stream.as_ref().expect("stream entry");
    let item = &stream.item_model_path;
    let error_type = match stream.framing {
        StreamFraming::Sse => "SseDecodeError",
        StreamFraming::Ndjson => "NdjsonDecodeError",
        StreamFraming::JsonSeq => "JsonSeqDecodeError",
    };
    let decode_call = match stream.framing {
        StreamFraming::Sse => "decode_sse_json",
        StreamFraming::Ndjson => "decode_ndjson",
        StreamFraming::JsonSeq => "decode_jsonseq",
    };
    let framing_label = stream.framing.as_snake();
    emitter.docs(
        0,
        &[format!(
            "Streaming {framing_label} request input for `{}` (main spec §6): \
             `next_item` decodes one record at a time bounded by \
             `max_stream_record_bytes`; decode failures reject per §39 \
             (oversized → 413, malformed/truncated/non-UTF-8/aborted → 400) \
             and nothing aggregates the body.",
            operation.method
        )],
    );
    emitter.line(0, &format!("pub struct {name} {{"));
    emitter.line(
        1,
        &format!("stream: ErasedItems<Result<{item}, {error_type}>>,"),
    );
    emitter.line(0, "}");
    emitter.blank();
    emitter.line(0, &format!("impl {name} {{"));
    emitter.docs(
        1,
        &["Wraps the raw body chunk stream behind the incremental decoder.".to_owned()],
    );
    let new_head =
        "fn new(chunks: ::axum::body::BodyDataStream, limit: usize) -> Self {".to_owned();
    if fits(1, &new_head) {
        emitter.line(1, &new_head);
    } else {
        emitter.line(1, "fn new(");
        emitter.line(2, "chunks: ::axum::body::BodyDataStream,");
        emitter.line(2, "limit: usize,");
        emitter.line(1, ") -> Self {");
    }
    emitter.line(2, "Self {");
    emitter.line(
        3,
        &format!("stream: Box::pin({decode_call}::<{item}, _, _>(chunks, limit)),"),
    );
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.blank();
    emitter.docs(
        1,
        &["Next decoded item (`None` at the clean end-of-stream).".to_owned()],
    );
    emitter.line(1, "#[allow(clippy::missing_errors_doc)]");
    let signature = format!(
        "pub async fn next_item(&mut self) -> Result<Option<{item}>, ProtocolRejection> {{"
    );
    if fits(1, &signature) {
        emitter.line(1, &signature);
    } else {
        emitter.line(1, "pub async fn next_item(");
        emitter.line(2, "&mut self,");
        emitter.line(
            1,
            &format!(") -> Result<Option<{item}>, ProtocolRejection> {{"),
        );
    }
    emitter.line(2, "let next = ::std::future::poll_fn(|cx| {");
    emitter.line(
        3,
        "::futures_core::Stream::poll_next(self.stream.as_mut(), cx)",
    );
    emitter.line(2, "})");
    emitter.line(2, ".await;");
    emitter.line(2, "match next {");
    emitter.line(3, "Some(Ok(item)) => Ok(Some(item)),");
    emitter.line(3, "Some(Err(error)) => Err(match error {");
    emitter.line(4, &format!("{error_type}::RecordTooLarge {{ .. }} => {{"));
    emitter.line(5, "ProtocolRejection::new(RejectionKind::BodyTooLarge)");
    emitter.line(4, "}");
    emitter.line(
        4,
        &format!("_ => malformed_body(\"{framing_label} record failed to decode\"),"),
    );
    emitter.line(3, "}),");
    emitter.line(3, "None => Ok(None),");
    emitter.line(2, "}");
    emitter.line(1, "}");
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
    // §40: committed-stream operations thread the router's stream-failure
    // hook into their encoder so mid-production failures are observable
    // before the body terminates abruptly.
    let has_stream_status = operation.statuses.iter().any(|status| {
        !status.is_no_body_status
            && status
                .contents
                .iter()
                .any(|content| content.stream.is_some())
    });
    if has_stream_status {
        emitter.line(1, "Ok(response.into_response_with_limits(");
        emitter.line(2, "&limits,");
        emitter.line(2, "hook,");
        emitter.line(2, "::std::sync::Arc::clone(&__state.stream_failure_hook),");
        emitter.line(1, "))");
    } else {
        emitter.line(1, "Ok(response.into_response_with_limits(&limits, hook))");
    }
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
        /// Shared-model target when the router auto-converts the decoded
        /// `<M>Write` view before invoking the trait (companion §5 recorded
        /// decision: lossless reconstructions only).
        convert_to: Option<String>,
        validation: Option<PlannedBodyValidation>,
    },
    /// Single-content URL-encoded form: bounded collect then the support
    /// decoder (main spec §16); axum's `Form` extractor is never used.
    Form {
        model: String,
        /// See [`EntryPayload::Json::convert_to`].
        convert_to: Option<String>,
        validation: Option<PlannedBodyValidation>,
    },
    /// Single-content text: decodes to `String`.
    Text {
        validation: Option<PlannedBodyValidation>,
    },
    /// Single-content codec-claimed body (main spec §45): bounded collect
    /// then the generated per-codec decode helper; ANY codec failure maps
    /// onto MalformedBody 400 (module-level deviation note in
    /// [`crate::codegen::codecs`]).
    Codec {
        model: String,
        codec_id: &'static str,
        validation: Option<PlannedBodyValidation>,
    },
    /// Single-content streaming: the raw body passes through.
    RawBody,
    /// Single-content wildcard: the generated `<Op>RequestBody` struct.
    WildcardStruct {
        struct_name: String,
    },
    EnumJson {
        enum_name: String,
        variant: String,
        validation: Option<PlannedBodyValidation>,
    },
    EnumForm {
        enum_name: String,
        variant: String,
        validation: Option<PlannedBodyValidation>,
    },
    EnumText {
        enum_name: String,
        variant: String,
        validation: Option<PlannedBodyValidation>,
    },
    /// Codec-claimed variant of a `<Op>RequestBody` enum (main spec §45).
    EnumCodec {
        enum_name: String,
        variant: String,
        codec_id: &'static str,
        validation: Option<PlannedBodyValidation>,
    },
    EnumRaw {
        enum_name: String,
        variant: String,
    },
    EnumWildcard {
        enum_name: String,
        variant: String,
    },
    /// Single-content multipart (§17): the router runs one incremental pass
    /// and hands the application the owned streaming input struct. The
    /// optional enum pair wraps the value into a `<Op>RequestBody` variant.
    Multipart {
        collector: String,
        enum_variant: Option<(String, String)>,
    },
    /// Single-content record-framed stream (§6 request-direction streams):
    /// the router wraps the raw body in a typed `<Op><Framing>Input` whose
    /// `next_item` decodes one record at a time; decode failures reject per
    /// §39 during consumption (wire-arrival philosophy).
    Stream {
        input_name: String,
        enum_variant: Option<(String, String)>,
    },
}

impl EntryPayload {
    fn is_decodable(&self) -> bool {
        matches!(
            self,
            Self::Json { .. }
                | Self::Form { .. }
                | Self::Text { .. }
                | Self::Codec { .. }
                | Self::EnumJson { .. }
                | Self::EnumForm { .. }
                | Self::EnumText { .. }
                | Self::EnumCodec { .. }
        )
    }

    fn is_json(&self) -> bool {
        matches!(self, Self::Json { .. } | Self::EnumJson { .. })
    }

    fn is_form(&self) -> bool {
        matches!(self, Self::Form { .. } | Self::EnumForm { .. })
    }

    /// True when this payload decodes through a §45 codec helper: no charset
    /// gate applies (binary formats carry none) but the §28.3 empty-body rule
    /// still does.
    fn is_codec(&self) -> Option<&'static str> {
        match self {
            Self::Codec { codec_id, .. } | Self::EnumCodec { codec_id, .. } => Some(codec_id),
            _ => None,
        }
    }

    /// Companion §9 check attached to this payload, if any.
    fn validation(&self) -> Option<&PlannedBodyValidation> {
        match self {
            Self::Json { validation, .. }
            | Self::Form { validation, .. }
            | Self::Text { validation }
            | Self::Codec { validation, .. }
            | Self::EnumJson { validation, .. }
            | Self::EnumForm { validation, .. }
            | Self::EnumText { validation, .. }
            | Self::EnumCodec { validation, .. } => validation.as_ref(),
            _ => None,
        }
    }

    /// Shared-model target of the router's lossless auto-conversion
    /// (companion §5 recorded decision), if this payload converts.
    fn conversion_target(&self) -> Option<&str> {
        match self {
            Self::Json { convert_to, .. } | Self::Form { convert_to, .. } => convert_to.as_deref(),
            _ => None,
        }
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
    // Companion §9: validation calls ride the payload only when the policy
    // is on (validators stay emitted in models.rs either way).
    let validation = layout
        .runtime_validation
        .then(|| content.body_validation.clone())
        .flatten();
    // §45 codec claim: bounded collect + generated decode helper, mirroring
    // the JSON payload shape (D-impl-codec-plugins).
    if let Some(binding) = &content.codec {
        let codec_id = binding.plugin_id;
        return match enum_name {
            Some(enum_name) => EntryPayload::EnumCodec {
                enum_name,
                variant: content.variant_name.clone(),
                codec_id,
                validation,
            },
            None => EntryPayload::Codec {
                model: binding.model_path.clone(),
                codec_id,
                validation,
            },
        };
    }
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
                validation,
            },
            None => EntryPayload::Json {
                model: content.model_expr.clone(),
                convert_to: router_conversion_target(operation, content),
                validation,
            },
        },
        MediaClass::UrlEncodedForm => match enum_name {
            Some(enum_name) => EntryPayload::EnumForm {
                enum_name,
                variant: content.variant_name.clone(),
                validation,
            },
            None => EntryPayload::Form {
                model: content.model_expr.clone(),
                convert_to: router_conversion_target(operation, content),
                validation,
            },
        },
        MediaClass::PlainText => match enum_name {
            Some(enum_name) => EntryPayload::EnumText {
                enum_name,
                variant: content.variant_name.clone(),
                validation,
            },
            None => EntryPayload::Text { validation },
        },
        MediaClass::Binary | MediaClass::RawUnknown => match enum_name {
            Some(enum_name) => EntryPayload::EnumRaw {
                enum_name,
                variant: content.variant_name.clone(),
            },
            None => EntryPayload::RawBody,
        },
        MediaClass::Multipart => EntryPayload::Multipart {
            collector: format!("collect_{}_multipart", operation.method),
            enum_variant: enum_name.map(|enum_name| (enum_name, content.variant_name.clone())),
        },
        _ if content.stream.is_some() => EntryPayload::Stream {
            input_name: layout
                .stream_input(op_index, index)
                .expect("stream input registered")
                .to_owned(),
            enum_variant: enum_name.map(|enum_name| (enum_name, content.variant_name.clone())),
        },
        other => unreachable!("Phase 2 media class {other:?}"),
    }
}

fn route_is_buffered(operation: &PlannedOperation) -> bool {
    !operation.request_contents.is_empty()
        && operation.request_contents.iter().all(is_decodable_content)
}

/// Shared-model target of the router's lossless auto-conversion for a
/// single-content structured body (companion §5 recorded decision in the
/// module docs): only when the decoded `<M>Write` view reconstructs the
/// shared model without inventing values. Multi-content enums, multipart,
/// and streaming bodies never convert here.
fn router_conversion_target(
    operation: &PlannedOperation,
    content: &PlannedContent,
) -> Option<String> {
    if operation.request_body_enum_name.is_some() {
        return None;
    }
    let view = content.view.as_ref()?;
    view.write_lossless.then(|| view.shared_type.clone())
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
        let (arm_indent, close_indent) = emit_classify_match_head(emitter, &literals, 1);
        emit_absent_content_type_arm(emitter, operation, arm_indent);
        emit_unmatched_arm(emitter, arm_indent);
        for index in 0..operation.request_contents.len() {
            let payload = entry_payload(operation, layout, op_index, index);
            emit_entry_arm(emitter, &payload, operation, index, arm_indent, true);
        }
        emitter.line(
            arm_indent,
            "RequestEntryMatch::Entry(_) => unreachable!(\"request entry index out of range\"),",
        );
        emitter.line(close_indent, "};");
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
    let (arm_indent, close_indent) = emit_classify_match_head(emitter, &literals, 3);
    // The head helper emits the opening brace; arms follow at arm_indent.
    emitter.line(
        arm_indent,
        "RequestEntryMatch::AbsentContentType | RequestEntryMatch::Unmatched => {",
    );
    emitter.line(arm_indent + 1, "return Err(unsupported_media_type(");
    emitter.line(
        arm_indent + 2,
        "\"nonempty optional body arrived without a usable Content-Type\",",
    );
    emitter.line(arm_indent + 1, "));");
    emitter.line(arm_indent, "}");
    for index in 0..operation.request_contents.len() {
        let payload = entry_payload(operation, layout, op_index, index);
        emit_entry_arm(emitter, &payload, operation, index, arm_indent, false);
    }
    emitter.line(
        arm_indent,
        "RequestEntryMatch::Entry(_) => unreachable!(\"request entry index out of range\"),",
    );
    emitter.line(close_indent, "}");
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

/// Emits the `match classify_request_entry(...)` opener following rustfmt's
/// preference order and RETURNS the indents to use for the arms and the
/// closing `};`:
///
/// 1. whole opener plus brace on one line;
/// 2. opener on one line, brace dropping to its own line;
/// 3. only when a `let x = ` prefix exists: break after `=` so the match
///    (with its brace) sits one level deeper;
/// 4. otherwise break the slice entries vertically.
fn emit_classify_match_head(
    emitter: &mut Emitter,
    literals: &[String],
    indent: usize,
) -> (usize, usize) {
    let joined = literals.join(", ");
    let prefix = if indent == 1 {
        "let request_body = "
    } else {
        ""
    };
    // rustfmt's canonical layouts, most-preferred first: whole head on one
    // line; otherwise the argument list stays horizontal and only the match
    // brace drops; then (binding sites only) the break-after-`=` form; only
    // when even the head cannot fit do the slice entries break vertically.
    let head = format!("{prefix}match classify_request_entry(parsed.as_ref(), &[{joined}])");
    let open_brace = format!("{head} {{");
    if fits(indent, &open_brace) {
        emitter.line(indent, &open_brace);
        return (indent + 1, indent);
    }
    if fits(indent, &head) {
        emitter.line(indent, &head);
        emitter.line(indent, "{");
        return (indent + 1, indent);
    }
    if !prefix.is_empty() {
        let deep_open = format!("match classify_request_entry(parsed.as_ref(), &[{joined}]) {{");
        if fits(indent + 1, &deep_open) {
            emitter.line(indent, prefix.trim_end());
            emitter.line(indent + 1, &deep_open);
            return (indent + 2, indent + 1);
        }
    }
    emitter.line(
        indent,
        &format!("{prefix}match classify_request_entry(parsed.as_ref(), &["),
    );
    for literal in literals {
        emitter.line(indent + 1, &format!("{literal},"));
    }
    emitter.line(indent, "]) {");
    (indent + 1, indent)
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
    if let EntryPayload::Multipart {
        collector,
        enum_variant,
    } = payload
    {
        emitter.line(indent, &format!("RequestEntryMatch::Entry({index}) => {{"));
        let call = format!("{collector}(body, parsed.as_ref(), &limits).await?");
        // A bare `let request_body = X; request_body` trips
        // clippy::let_and_return on the MSRV toolchain (rustc 1.85), so the
        // binding is only emitted when a wrapper consumes it.
        let needs_binding = enum_variant.is_some() || !required;
        if needs_binding {
            let bind = format!("let request_body = {call}");
            if fits(indent + 1, &bind) {
                emitter.line(indent + 1, &bind);
            } else {
                emitter.line(indent + 1, "let request_body =");
                emitter.line(
                    indent + 2,
                    &format!("{collector}(body, parsed.as_ref(), &limits).await?;"),
                );
            }
        }
        match enum_variant {
            Some((enum_name, variant)) => {
                let inner = format!("{enum_name}::{variant}(request_body)");
                if required {
                    emitter.line(indent + 1, &inner);
                } else {
                    emitter.line(indent + 1, &format!("Some({inner})"));
                }
            }
            None => {
                if required {
                    // Tail expression: no binding was emitted above.
                    emitter.line(indent + 1, &call);
                } else {
                    emitter.line(indent + 1, "Some(request_body)");
                }
            }
        }
        emitter.line(indent, "}");
        return;
    }
    if let EntryPayload::Stream {
        input_name,
        enum_variant,
    } = payload
    {
        emitter.line(indent, &format!("RequestEntryMatch::Entry({index}) => {{"));
        // The typed input wrapper decodes lazily; its `next_item` maps
        // per-record failures onto §39 rejections during consumption.
        // rustfmt keeps the constructor argument list vertical (the one-line
        // form exceeds fn_call_width).
        let direct_tail = enum_variant.is_none() && required;
        let chunks = if required {
            "body.into_data_stream()"
        } else {
            // §28.2 peek-and-preserve: the peeked prefix decodes exactly once.
            "::axum::body::Body::from_stream(replay).into_data_stream()"
        };
        let inline = format!("{input_name}::new({chunks}, limits.max_stream_record_bytes)");
        if !direct_tail {
            emitter.line(indent + 1, "let value =");
        }
        if direct_tail && fits(indent + 1, &inline) {
            // Tail expression on one line, no semicolon.
            emitter.line(indent + 1, &inline);
        } else if direct_tail {
            emitter.line(indent + 1, &format!("{input_name}::new("));
            emitter.line(indent + 2, &format!("{chunks},"));
            emitter.line(indent + 2, "limits.max_stream_record_bytes,");
            emitter.line(indent + 1, ")");
        } else {
            emitter.line(indent + 1, &format!("{input_name}::new("));
            emitter.line(indent + 2, &format!("{chunks},"));
            emitter.line(indent + 2, "limits.max_stream_record_bytes,");
            emitter.line(indent + 1, ");");
        }
        match enum_variant {
            Some((enum_name, variant)) => {
                if required {
                    emitter.line(indent + 1, &format!("{enum_name}::{variant}(value)"));
                } else {
                    emitter.line(indent + 1, &format!("Some({enum_name}::{variant}(value))"));
                }
            }
            None if !required => {
                emitter.line(indent + 1, "Some(value)");
            }
            // Required single-content: the constructor IS the arm value.
            None => {}
        }
        emitter.line(indent, "}");
        return;
    }
    // Streaming/wildcard arms carry no statements, so rustfmt renders them
    // as expression arms; structured arms keep block form around their
    // charset check, bounded collection, and decode.
    if !payload.is_decodable() {
        emit_entry_yield_expr(emitter, indent, index, payload, required);
        return;
    }
    emitter.line(indent, &format!("RequestEntryMatch::Entry({index}) => {{"));
    // Set false when the decode expression itself becomes the arm value.
    let mut yield_value = true;
    if payload.is_decodable() {
        // §28.4 charset policy applies to the TEXTUAL classes only; binary
        // codec formats (§45) carry no charset parameters.
        if payload.is_codec().is_none() {
            emitter.line(indent + 1, "ensure_utf8_charset(parsed.as_ref())?;");
        }
        let limit_field = match payload {
            EntryPayload::Text { .. } | EntryPayload::EnumText { .. } => "text_body_bytes",
            _ => "structured_request_bytes",
        };
        if required {
            emitter.line(
                indent + 1,
                &format!("let bytes = body_bytes(body, limits.{limit_field}).await?;"),
            );
            // §28.3: an EMPTY body on a documented required body is missing,
            // never a default value — forms and codec bodies follow the JSON
            // rule.
            if payload.is_json() || payload.is_form() || payload.is_codec().is_some() {
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
            EntryPayload::Codec {
                model, codec_id, ..
            } => {
                let helper = format!("{}_decode_body", helper_prefix(codec_id));
                let bind = format!("let value: {model} = {helper}(&bytes)?;");
                if fits(indent + 1, &bind) {
                    emitter.line(indent + 1, &bind);
                } else {
                    emitter.line(indent + 1, &format!("let value: {model} ="));
                    emitter.line(indent + 2, &format!("{helper}(&bytes)?;"));
                }
            }
            EntryPayload::EnumCodec { codec_id, .. } => {
                let helper = format!("{}_decode_body", helper_prefix(codec_id));
                emitter.line(indent + 1, &format!("let value = {helper}(&bytes)?;"));
            }
            EntryPayload::Json { model, .. } => {
                let bind = format!("let value: {model} = decode_json_body(&bytes)?;");
                if fits(indent + 1, &bind) {
                    emitter.line(indent + 1, &bind);
                } else {
                    emitter.line(indent + 1, &format!("let value: {model} ="));
                    emitter.line(indent + 2, "decode_json_body(&bytes)?;");
                }
            }
            EntryPayload::Form { model, .. } => {
                let call = format!("decode_form_body(&bytes, limits.{limit_field})?;");
                let bind = format!("let value: {model} = {call}");
                if fits(indent + 1, &bind) {
                    emitter.line(indent + 1, &bind);
                } else {
                    emitter.line(indent + 1, &format!("let value: {model} ="));
                    emitter.line(indent + 2, &call);
                }
            }
            EntryPayload::EnumJson { .. } => {
                emitter.line(indent + 1, "let value = decode_json_body(&bytes)?;");
            }
            EntryPayload::EnumForm { .. } => {
                let line = format!("let value = decode_form_body(&bytes, limits.{limit_field})?;");
                emitter.line(indent + 1, &line);
            }
            // A required single-text entry with no companion §9 validator
            // has nothing between decode and yield: the `?` expression IS
            // the arm value, avoiding clippy::let_and_return in the router.
            EntryPayload::Text { validation: None } if required => {
                emitter.line(indent + 1, "decode_text_body(bytes)?");
                yield_value = false;
            }
            _ => {
                emitter.line(indent + 1, "let value = decode_text_body(bytes)?;");
            }
        }
        // Companion §9: bucket-2 checks run AFTER a successful decode, so a
        // violating body never reaches the handler (§39 rule 1; SchemaViolation
        // → 422).
        if let Some(validation) = payload.validation() {
            emit_request_validation_call(emitter, indent + 1, "body", validation);
        }
    }
    if yield_value {
        emit_entry_yield(emitter, indent + 1, payload, required);
    }
    emitter.line(indent, "}");
}

/// Emits one post-decode validation statement against the arm's `value`
/// binding: inherent `validate_request()` for composites, free models.rs
/// validators for constrained scalar aliases — routed through the shared
/// SchemaViolation 422 constructor with a location prefix. A single flat
/// call keeps rustfmt layout width-predictable.
fn emit_request_validation_call(
    emitter: &mut Emitter,
    indent: usize,
    location: &str,
    validation: &PlannedBodyValidation,
) {
    let inner = match validation {
        PlannedBodyValidation::Inherent => "value.validate_request()".to_owned(),
        PlannedBodyValidation::ScalarFn(name) => format!("{name}(&value)"),
    };
    let quoted = format!("\"{location}\"");
    emit_flat_call(emitter, indent, "require_valid_request", &[&quoted, &inner]);
}

/// Emits `NAME(ARG0, ARG1)?;` flat when it fits the rustfmt budget, else in
/// rustfmt's canonical vertical form (one argument per line).
fn emit_flat_call(emitter: &mut Emitter, indent: usize, name: &str, args: &[&str]) {
    let flat = format!("{name}({})?;", args.join(", "));
    if fits(indent, &flat) {
        emitter.line(indent, &flat);
        return;
    }
    emitter.line(indent, &format!("{name}("));
    for arg in args {
        emitter.line(indent + 1, &format!("{arg},"));
    }
    emitter.line(indent, ")?;");
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
        EntryPayload::Multipart { .. } | EntryPayload::Stream { .. } => {
            unreachable!("multipart/stream payloads are handled in emit_entry_arm")
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
        EntryPayload::EnumJson {
            enum_name, variant, ..
        }
        | EntryPayload::EnumForm {
            enum_name, variant, ..
        }
        | EntryPayload::EnumText {
            enum_name, variant, ..
        }
        | EntryPayload::EnumCodec {
            enum_name, variant, ..
        } => {
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
        EntryPayload::Json { .. }
        | EntryPayload::Form { .. }
        | EntryPayload::Text { .. }
        | EntryPayload::Codec { .. } => {
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
        EntryPayload::Json { .. }
        | EntryPayload::Form { .. }
        | EntryPayload::Text { .. }
        | EntryPayload::Codec { .. } => {
            // Optional bodies yield `Some(..)` so the outer presence match
            // separates Empty (None) from a decoded document (§28.2).
            // Directional-view contract (companion §5): a lossless write
            // view reconstructs the shared model as the arm's tail, so the
            // trait keeps its domain type; validation already ran on the
            // decoded view above.
            let inner = payload
                .conversion_target()
                .map(|shared| format!("{shared}::from(&value)"))
                .unwrap_or_else(|| "value".to_owned());
            if required {
                emitter.line(indent, &inner);
            } else {
                emitter.line(indent, &format!("Some({inner})"));
            }
        }
        EntryPayload::RawBody => {
            if required {
                emitter.line(indent, "body");
            } else {
                emitter.line(indent, &format!("Some({replay_body})"));
            }
        }
        EntryPayload::Multipart { .. } | EntryPayload::Stream { .. } => {
            unreachable!("multipart/stream payloads are handled in emit_entry_arm")
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
        EntryPayload::EnumJson {
            enum_name, variant, ..
        }
        | EntryPayload::EnumForm {
            enum_name, variant, ..
        }
        | EntryPayload::EnumText {
            enum_name, variant, ..
        }
        | EntryPayload::EnumCodec {
            enum_name, variant, ..
        } => {
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
// Multipart streaming input (main spec §17 Output B, §17.1)
// ----------------------------------------------------------------------

/// A planned multipart field with its collision-resolved struct identifier
/// (companion §10 numeric suffixing inside one input struct).
struct MultipartFieldIdents<'a> {
    field: &'a crate::codegen::plan::PlannedMultipartField,
    ident: String,
}

fn unique_multipart_field_name(base: &str, used: &mut BTreeMap<String, u32>) -> String {
    let counter = used.entry(base.to_owned()).or_insert(0);
    *counter += 1;
    if *counter == 1 {
        base.to_owned()
    } else {
        naming::sanitize_joined(&format!("{base}_{counter}"))
    }
}

fn resolve_multipart_field_idents(
    fields: &[crate::codegen::plan::PlannedMultipartField],
) -> Vec<MultipartFieldIdents<'_>> {
    let mut used: BTreeMap<String, u32> = BTreeMap::new();
    fields
        .iter()
        .map(|field| MultipartFieldIdents {
            field,
            ident: unique_multipart_field_name(&field.rust_name, &mut used),
        })
        .collect()
}

/// Server-side scalar/JSON part type with the companion §2.1 presence
/// matrix applied: required single-valued parts are plain; optional ones
/// ride `Option<T>`; repeated parts are `Vec<T>` in wire order.
fn multipart_input_field_type(
    kind: &PlannedMultipartFieldKind,
    repeated: bool,
    required: bool,
) -> String {
    let base = match kind {
        PlannedMultipartFieldKind::ScalarText(rust_type)
        | PlannedMultipartFieldKind::JsonPart(rust_type) => rust_type.clone(),
        PlannedMultipartFieldKind::BinaryPart => String::new(),
    };
    if repeated {
        format!("Vec<{base}>")
    } else if required {
        base
    } else {
        format!("Option<{base}>")
    }
}

/// Emits the per-operation streaming input struct plus one live streaming
/// part type per binary field (main spec §17 Output B). Required-part
/// enforcement is wire-arrival-based (§17.1/§38): scalar/JSON parts consumed
/// before the streaming handoff validated inside the collector; parts
/// arriving behind the live part decode onto `<Op>TrailingParts`, and
/// required names a clean end-of-message still lacks surface exactly one
/// terminal SchemaViolation from `next_chunk`.
fn emit_multipart_types(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
    input_name: &str,
) {
    let Some(spec) = operation
        .request_contents
        .iter()
        .find(|content| content.media_class == MediaClass::Multipart)
        .and_then(|content| content.multipart_spec.as_ref())
    else {
        return;
    };
    let resolved = resolve_multipart_field_idents(&spec.fields);
    let trailing_name = layout.multipart_trailing(op_index);

    for (field_index, _) in resolved.iter().enumerate() {
        let Some(part_name) = layout.multipart_part(op_index, field_index) else {
            continue;
        };
        // Plan time caps each multipart body at ONE binary part (§51.4), so
        // this emits at most one live part type per operation.
        emit_live_part_type(
            emitter,
            operation,
            input_name,
            part_name,
            &resolved,
            trailing_name,
            layout.runtime_validation,
        );
    }
    let has_binary = resolved
        .iter()
        .any(|entry| matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart));

    emitter.blank();
    emitter.docs(
        0,
        &[format!(
            "Streaming multipart input for `{}` (main spec §17 Output B): \
              scalar/JSON parts were bounded-buffered and decoded during the \
             router's single incremental pass up to the streaming handoff; \
              binary parts stay live streams over the request body.",
            operation.method
        )],
    );
    if has_binary {
        emitter.docs(
            0,
            &[String::from(
                "Required-part enforcement is wire-arrival-based (§17.1, \
                  §38): parts arriving BEFORE the first binary validate \
                  pre-handler in that pass; parts arriving behind the live \
                 stream decode onto its `trailing_parts`, and required names \
                  a clean end-of-message never delivers reject through the \
                  live part's terminal error instead of a pre-handler \
                  rejection.",
            )],
        );
    } else {
        emitter.docs(
            0,
            &[String::from(
                "Required-part enforcement ran entirely inside that pass, so \
                  missing required parts reject 422 BEFORE the application \
                  handler runs (§17.1).",
            )],
        );
    }
    emitter.line(0, &format!("pub struct {input_name} {{"));
    for (field_index, entry) in resolved.iter().enumerate() {
        emit_multipart_input_field(emitter, op_index, layout, field_index, entry, has_binary);
    }
    emitter.line(
        1,
        "unknown_log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,",
    );
    emitter.line(0, "}");
    emitter.blank();
    emitter.line(0, &format!("impl {input_name} {{"));
    emitter.docs(
        1,
        &[String::from(
            "Wire names of every unrecognized or late-arriving part observed \
             so far (§17.1 unknown-fields-ignore default): their payloads \
             stream past without buffering and never reject. Names behind a \
             streaming part appear once the application drains it through \
             `next_chunk`.",
        )],
    );
    emitter.line(1, "pub fn unknown_part_names(&self) -> Vec<String> {");
    emitter.line(2, "match self.unknown_log.lock() {");
    emitter.line(3, "Ok(guard) => guard.names.clone(),");
    emitter.line(3, "Err(poisoned) => poisoned.into_inner().names.clone(),");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// One declared field of the `<Op>MultipartInput` struct.
fn emit_multipart_input_field(
    emitter: &mut Emitter,
    op_index: usize,
    layout: &ServerLayout,
    field_index: usize,
    entry: &MultipartFieldIdents,
    has_binary: bool,
) {
    match &entry.field.kind {
        PlannedMultipartFieldKind::BinaryPart => {
            emit_multipart_field_doc(emitter, 1, entry);
            let part_name = layout
                .multipart_part(op_index, field_index)
                .expect("binary part registered");
            let field_type = if entry.field.required {
                part_name.to_owned()
            } else {
                format!("Option<{part_name}>")
            };
            emitter.line(1, &format!("pub {}: {field_type},", entry.ident));
        }
        kind @ (PlannedMultipartFieldKind::ScalarText(_)
        | PlannedMultipartFieldKind::JsonPart(_)) => {
            // With a live part in the body a required single-valued field
            // may still arrive behind the stream, so its value defers onto
            // `trailing_parts` and enforcement is wire-arrival-based.
            let deferred = has_binary && entry.field.required && !entry.field.repeated;
            if deferred {
                let cardinality = if entry.field.repeated {
                    "; repeated parts collect in wire order"
                } else {
                    ""
                };
                emitter.docs(
                    1,
                    &[format!(
                        "{} part `{}`{}: `Some` when it arrived before the \
                          streaming part; otherwise decoded onto the live \
                          part's `trailing_parts`.",
                        part_label(&entry.field.kind),
                        entry.field.wire_name,
                        cardinality
                    )],
                );
                let base = match kind {
                    PlannedMultipartFieldKind::ScalarText(rust_type)
                    | PlannedMultipartFieldKind::JsonPart(rust_type) => rust_type.clone(),
                    PlannedMultipartFieldKind::BinaryPart => String::new(),
                };
                emitter.line(1, &format!("pub {}: Option<{base}>,", entry.ident));
            } else {
                emit_multipart_field_doc(emitter, 1, entry);
                let field_type =
                    multipart_input_field_type(kind, entry.field.repeated, entry.field.required);
                emitter.line(1, &format!("pub {}: {field_type},", entry.ident));
            }
        }
    }
}

/// The human label for one multipart field's kind.
fn part_label(kind: &PlannedMultipartFieldKind) -> &'static str {
    match kind {
        PlannedMultipartFieldKind::ScalarText(_) => "Textual",
        PlannedMultipartFieldKind::JsonPart(_) => "JSON",
        PlannedMultipartFieldKind::BinaryPart => "Streaming binary",
    }
}

/// The standard one-line field doc for a multipart input/trailing field.
fn emit_multipart_field_doc(emitter: &mut Emitter, indent: usize, entry: &MultipartFieldIdents) {
    let cardinality = if entry.field.repeated {
        "; repeated parts collect in wire order"
    } else {
        ""
    };
    emitter.docs(
        indent,
        &[format!(
            "{} part `{}`{}.",
            part_label(&entry.field.kind),
            entry.field.wire_name,
            cardinality
        )],
    );
}

/// One live streaming part type plus its private tail-scan stage enum and
/// (when scalar/JSON fields exist) the operation's trailing-parts carrier.
#[allow(clippy::too_many_arguments)]
fn emit_live_part_type(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    input_name: &str,
    part_name: &str,
    resolved: &[MultipartFieldIdents],
    trailing_name: Option<&str>,
    validate_parts: bool,
) {
    // The single binary entry of this body (plan time caps bodies at one).
    let entry = resolved
        .iter()
        .find(|entry| matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart))
        .expect("live part requires a binary field");
    let tail_stage = format!("{part_name}TailStage");
    let buffered: Vec<&MultipartFieldIdents> = resolved
        .iter()
        .filter(|entry| {
            matches!(
                entry.field.kind,
                PlannedMultipartFieldKind::ScalarText(_) | PlannedMultipartFieldKind::JsonPart(_)
            )
        })
        .collect();

    emitter.docs(
        0,
        &[format!(
            "Live streaming view over binary part `{}` of `{}` (main \
              spec §17 Output B): `next_chunk` yields payload bytes with \
              backpressure as they arrive; nothing aggregates the part.",
            entry.field.wire_name, operation.method
        )],
    );
    emitter.docs(
        0,
        &[String::from(
            "Sequential semantics (§51.4): while this part is open the rest \
              of the message cannot advance. Trailing parts flow through \
              this type's tail scan instead of the pre-handler pass: \
              declared scalar/JSON parts arriving behind the stream decode \
             bounded onto `trailing_parts`, required wire names the clean \
              end-of-message still lacks surface exactly one terminal \
              SchemaViolation from `next_chunk` (§17.1 enforced on wire \
              arrival), duplicate single-valued reopenings reject, every \
              observed name is recorded for `unknown_part_names`, and all \
              remaining payloads drain without buffering.",
        )],
    );
    emitter.line(0, &format!("pub struct {part_name} {{"));
    emitter.line(1, "pub file_name: Option<String>,");
    emitter.line(1, "pub content_type: Option<::mime::Mime>,");
    if let Some(trailing) = trailing_name {
        emitter.docs(
            1,
            &[format!(
                "Scalar/JSON parts that arrived BEHIND this streaming part, \
                  decoded bounded as their boundaries closed (§38 \
                 application-owned tail); their pre-handoff siblings live on \
                  [`{input_name}`]."
            )],
        );
        emitter.line(1, &format!("pub trailing_parts: {trailing},"));
    }
    emitter.line(1, "events: MultipartEvents,");
    emitter.line(
        1,
        "log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,",
    );
    emitter.line(1, &format!("stage: {tail_stage},"));
    emitter.line(1, "scalar_limit: usize,");
    emitter.line(1, "buffer: Vec<u8>,");
    emitter.line(1, "finished: bool,");
    emitter.docs(
        1,
        &[String::from(
            "Required scalar/JSON wire names still unseen at the streaming \
              handoff (§17.1): satisfied by trailing arrivals, otherwise \
              reported once at the clean end-of-message.",
        )],
    );
    emitter.line(1, "pending_required: Vec<&'static str>,");
    emitter.docs(
        1,
        &[String::from(
            "Declared SINGLE-VALUED wire names already consumed anywhere \
              before now (pre-handoff or behind the stream): any reopening \
              violates §17.1.",
        )],
    );
    emitter.line(1, "seen_single_valued: Vec<String>,");
    emitter.line(0, "}");
    emitter.blank();
    emit_tail_stage_enum(emitter, part_name, &tail_stage, &buffered);
    if let Some(trailing) = trailing_name {
        emitter.blank();
        emit_trailing_parts_struct(emitter, input_name, part_name, trailing, &buffered);
    }
    emitter.blank();
    emitter.line(0, &format!("impl {part_name} {{"));
    emit_next_chunk(
        emitter,
        &tail_stage,
        &buffered,
        !buffered.is_empty(),
        validate_parts,
    );
    emitter.line(0, "}");
}

/// Private per-part tail-scan stages: `Idle` delivers payload chunks to the
/// application, `Drain` discards them, and one stage per scalar/JSON field
/// bounded-buffers that trailing part for decoding when its boundary closes.
fn emit_tail_stage_enum(
    emitter: &mut Emitter,
    part_name: &str,
    tail_stage: &str,
    buffered: &[&MultipartFieldIdents],
) {
    emitter.docs(
        0,
        &[format!(
            "Tail-scan stages of [`{part_name}`] (§51.4 sequential \
              semantics): `Idle` delivers payload chunks to the application; \
              `Drain` discards them; the remaining stages bounded-buffer one \
              trailing scalar/JSON part behind the stream."
        )],
    );
    emitter.line(0, "#[derive(Debug, Clone, Copy)]");
    emitter.line(0, &format!("enum {tail_stage} {{"));
    emitter.line(1, "Idle,");
    emitter.line(1, "Drain,");
    for entry in buffered {
        let variant = naming::ident(&entry.field.rust_name, NameStyle::Pascal);
        if entry.field.repeated {
            emitter.line(1, &format!("{variant}Element,"));
        } else {
            emitter.line(1, &format!("{variant},"));
        }
    }
    emitter.line(0, "}");
}

/// The `<Op>TrailingParts` carrier: every scalar/JSON field may still arrive
/// behind the streaming handoff, so singles ride `Option<T>` and repeated
/// fields collect `Vec<T>` in wire order.
fn emit_trailing_parts_struct(
    emitter: &mut Emitter,
    input_name: &str,
    part_name: &str,
    trailing_name: &str,
    buffered: &[&MultipartFieldIdents],
) {
    emitter.docs(
        0,
        &[format!(
            "Scalar/JSON parts observed BEHIND [`{part_name}`] (main spec \
              §17 Output B): decoded bounded as their boundaries closed. \
              Parts consumed BEFORE the streaming handoff live on \
              [`{input_name}`]; the split mirrors wire arrival (§38 \
              application-owned tail)."
        )],
    );
    emitter.line(0, "#[derive(Debug, Default)]");
    emitter.line(0, &format!("pub struct {trailing_name} {{"));
    for entry in buffered {
        emit_multipart_field_doc(emitter, 1, entry);
        let field_type = multipart_input_field_type(&entry.field.kind, entry.field.repeated, false);
        emitter.line(1, &format!("pub {}: {field_type},", entry.ident));
    }
    emitter.line(0, "}");
}

/// `next_chunk`: chunk delivery plus the wire-arrival-based tail scan.
fn emit_next_chunk(
    emitter: &mut Emitter,
    tail_stage: &str,
    buffered: &[&MultipartFieldIdents],
    has_buffered: bool,
    validate_parts: bool,
) {
    emitter.docs(
        1,
        &[String::from(
            "Advances to the next payload chunk of this part (`None` at its \
              clean end). Violations encountered while scanning trailing \
              parts surface here as protocol rejections because sequential \
              streaming cannot validate them any earlier; at the clean \
              end-of-message, required parts still pending produce exactly \
              one terminal SchemaViolation naming them (§17.1) and later \
              calls keep returning `None`.",
        )],
    );
    emitter.line(1, "#[allow(clippy::missing_errors_doc)]");
    let chunk_head =
        "pub async fn next_chunk(&mut self) -> Result<Option<::bytes::Bytes>, ProtocolRejection> {";
    if fits(1, chunk_head) {
        emitter.line(1, chunk_head);
    } else {
        emitter.line(1, "pub async fn next_chunk(");
        emitter.line(2, "&mut self,");
        emitter.line(
            1,
            ") -> Result<Option<::bytes::Bytes>, ProtocolRejection> {",
        );
    }
    emitter.line(2, "if self.finished {");
    emitter.line(3, "return Ok(None);");
    emitter.line(2, "}");
    emitter.line(2, "loop {");
    emitter.line(
        3,
        "let event = match next_multipart_event(&mut self.events).await {",
    );
    emitter.line(4, "Some(event) => event,");
    emitter.line(4, "None => {");
    emitter.line(5, "self.finished = true;");
    if has_buffered {
        emitter.line(5, "if !self.pending_required.is_empty() {");
        emitter.line(6, "let names = self.pending_required.join(\"`, `\");");
        emitter.line(6, "return Err(schema_violation(format!(");
        emitter.line(7, "\"missing required part(s) `{names}`\",");
        emitter.line(6, ")));");
        emitter.line(5, "}");
    }
    emitter.line(5, "return Ok(None);");
    emitter.line(4, "}");
    emitter.line(3, "};");
    emitter.line(3, "match event {");
    emitter.line(4, "Err(error) => return Err(multipart_rejection(&error)),");
    emitter.line(4, "Ok(MultipartEvent::PartBegin(headers)) => {");
    emitter.line(5, "if self.seen_single_valued.contains(&headers.name) {");
    emitter.line(6, "return Err(schema_violation(format!(");
    emitter.line(
        7,
        "\"duplicate single-valued part `{}` after the streaming part\",",
    );
    emitter.line(7, "headers.name");
    emitter.line(6, ")));");
    emitter.line(5, "}");
    emitter.line(5, "match headers.name.as_str() {");
    for entry in buffered {
        emit_tail_begin_arm(emitter, tail_stage, entry);
    }
    emitter.line(6, "other => {");
    emitter.line(7, "multipart_record_unknown(&self.log, other);");
    emitter.line(7, &format!("self.stage = {tail_stage}::Drain;"));
    emitter.line(6, "}");
    emitter.line(5, "}");
    emitter.line(4, "}");
    emitter.line(
        4,
        "Ok(MultipartEvent::PartChunk(chunk)) => match self.stage {",
    );
    emitter.line(5, &format!("{tail_stage}::Idle => return Ok(Some(chunk)),"));
    emitter.line(5, &format!("{tail_stage}::Drain => {{}}"));
    if has_buffered {
        let variants = buffered
            .iter()
            .map(|entry| tail_stage_variant_name(tail_stage, entry))
            .collect::<Vec<_>>();
        emit_or_pattern_head(emitter, 5, &variants, " => {");
        emitter.line(6, "self.buffer.extend_from_slice(&chunk);");
        emitter.line(6, "if self.buffer.len() > self.scalar_limit {");
        emitter.line(
            7,
            "return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));",
        );
        emitter.line(6, "}");
        emitter.line(5, "}");
    }
    emitter.line(4, "},");
    // A block holding only this match collapses onto the arrow (§50 test 40
    // keeps every emission rustfmt-canonical).
    emitter.line(4, "Ok(MultipartEvent::PartEnd) => match self.stage {");
    let idle_drain = vec![
        format!("{tail_stage}::Idle"),
        format!("{tail_stage}::Drain"),
    ];
    emit_or_pattern_head(emitter, 5, &idle_drain, " => {}");
    for entry in buffered {
        emit_tail_decode_arm(emitter, tail_stage, entry, validate_parts);
    }
    emitter.line(4, "},");
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "}");
}

/// Emits one post-decode validator call for a multipart scalar part
/// (companion §9): constrained scalar aliases validate right where the part
/// bytes become typed values, so a violating part rejects 422 before any
/// application code runs.
fn emit_part_validation_call(
    emitter: &mut Emitter,
    indent: usize,
    entry: &MultipartFieldIdents,
    value_expr: &str,
    validate_parts: bool,
) {
    let Some(name) = entry.field.scalar_validator.as_deref() else {
        return;
    };
    if !validate_parts {
        return;
    }
    let location_text = format!("part `{}`", entry.field.wire_name);
    let location = rust_string_literal(&location_text);
    let inner = format!("{name}({value_expr})");
    emit_flat_call(
        emitter,
        indent,
        "require_valid_request",
        &[&location, &inner],
    );
}

/// Stage variant name (no enum path) for one buffered trailing field.
fn tail_stage_variant_name(tail_stage: &str, entry: &MultipartFieldIdents) -> String {
    let variant = naming::ident(&entry.field.rust_name, NameStyle::Pascal);
    if entry.field.repeated {
        format!("{tail_stage}::{variant}Element")
    } else {
        format!("{tail_stage}::{variant}")
    }
}

/// Emits one match-arm pattern head from or-pattern alternatives: a single
/// line when it fits the rustfmt budget, otherwise rustfmt's canonical
/// broken form (`alt0`, then `| altN` lines, the last carrying `suffix`).
fn emit_or_pattern_head(
    emitter: &mut Emitter,
    indent: usize,
    alternatives: &[String],
    suffix: &str,
) {
    let single_line = format!("{}{suffix}", alternatives.join(" | "));
    if fits(indent, &single_line) {
        emitter.line(indent, &single_line);
        return;
    }
    let last = alternatives.len() - 1;
    for (index, alternative) in alternatives.iter().enumerate() {
        if index == 0 {
            emitter.line(indent, alternative);
        } else if index == last {
            emitter.line(indent, &format!("| {alternative}{suffix}"));
        } else {
            emitter.line(indent, &format!("| {alternative}"));
        }
    }
}

/// One `PartBegin` arm of the tail scan's declared-name dispatch: mark the
/// single-valued name consumed, clear it from the pending-required set, and
/// start bounded buffering toward its boundary decode.
fn emit_tail_begin_arm(emitter: &mut Emitter, tail_stage: &str, entry: &MultipartFieldIdents) {
    let wire = rust_string_literal(&entry.field.wire_name);
    emitter.line(6, &format!("{wire} => {{"));
    if !entry.field.repeated {
        emitter.line(
            7,
            &format!("self.seen_single_valued.push({wire}.to_owned());"),
        );
        if entry.field.required {
            emitter.line(
                7,
                &format!("self.pending_required.retain(|name| *name != {wire});"),
            );
        }
    }
    emitter.line(7, "self.buffer.clear();");
    emitter.line(
        7,
        &format!(
            "self.stage = {};",
            tail_stage_variant_name(tail_stage, entry)
        ),
    );
    emitter.line(6, "}");
}

/// One `PartEnd` decode arm of the tail scan: bounded-buffered bytes become
/// typed values on the trailing-parts carrier.
fn emit_tail_decode_arm(
    emitter: &mut Emitter,
    tail_stage: &str,
    entry: &MultipartFieldIdents,
    validate_parts: bool,
) {
    emitter.line(
        5,
        &format!("{} => {{", tail_stage_variant_name(tail_stage, entry)),
    );
    match &entry.field.kind {
        PlannedMultipartFieldKind::ScalarText(rust_type) => {
            let wire = rust_string_literal(&entry.field.wire_name);
            emitter.line(
                6,
                &format!(
                    "let value = multipart_scalar_text::<{rust_type}>({wire}, &self.buffer)?;"
                ),
            );
            emit_part_validation_call(emitter, 6, entry, "&value", validate_parts);
        }
        PlannedMultipartFieldKind::JsonPart(model) => {
            let bind = format!("let value: {model} = decode_json_body(&self.buffer)?;");
            if fits(6, &bind) {
                emitter.line(6, &bind);
            } else {
                emitter.line(6, &format!("let value: {model} ="));
                emitter.line(7, "decode_json_body(&self.buffer)?;");
            }
        }
        PlannedMultipartFieldKind::BinaryPart => unreachable!("binary parts never buffer"),
    }
    if entry.field.repeated {
        emitter.line(
            6,
            &format!("self.trailing_parts.{}.push(value);", entry.ident),
        );
    } else {
        emitter.line(
            6,
            &format!("self.trailing_parts.{} = Some(value);", entry.ident),
        );
    }
    emitter.line(6, &format!("self.stage = {tail_stage}::Idle;"));
    emitter.line(5, "}");
}

/// The router-side single-pass collector for one operation's
/// `multipart/form-data` body: streams the framed message exactly once,
/// buffering ONLY scalar/JSON payloads up to `multipart_scalar_part_bytes`,
/// enforcing required/duplicate/cardinality rules (§17.1), and handing the
/// application an input whose binary parts are live streams.
fn emit_multipart_collector(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &ServerLayout,
) {
    let Some(spec) = operation
        .request_contents
        .iter()
        .find(|content| content.media_class == MediaClass::Multipart)
        .and_then(|content| content.multipart_spec.as_ref())
    else {
        return;
    };
    let Some(input_name) = layout.multipart_input(op_index) else {
        return;
    };
    let resolved = resolve_multipart_field_idents(&spec.fields);
    let collector = format!("collect_{}_multipart", operation.method);

    emitter.blank();
    emitter.docs(
        0,
        &[format!(
            "Runs the §38 pre-handler pipeline for the `multipart/form-data` \
              body of `{}` (main spec §5.5/§17/§17.1): one incremental pass \
              up to the streaming handoff; scalar/JSON parts buffer only up \
              to `multipart_scalar_part_bytes`; duplicate-single-valued \
              parts reject before the trait runs, and required names the \
              pass never observed reject here unless they may still arrive \
              behind the live stream — those ride its `pending_required` set \
              (wire-arrival-based enforcement, §17.1/§38).",
            operation.method
        )],
    );
    emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
    emitter.line(0, "#[allow(clippy::too_many_lines)]");
    emitter.line(0, &format!("async fn {collector}("));
    emitter.line(1, "body: ::axum::body::Body,");
    emitter.line(1, "parsed: Option<&ParsedMediaType>,");
    emitter.line(1, "limits: &BodyLimits,");
    emitter.line(
        0,
        &format!(") -> Result<{input_name}, ProtocolRejection> {{"),
    );
    // Boundary extraction is never defaulted (§28.1); a Content-Type without
    // a usable boundary parameter is malformed framing.
    emitter.line(1, "let parsed = match parsed {");
    emitter.line(2, "Some(parsed) => parsed,");
    emitter.line(2, "None => {");
    emitter.line(
        3,
        "return Err(malformed_body(\"multipart body requires a Content-Type\"));",
    );
    emitter.line(2, "}");
    emitter.line(1, "};");
    emitter.line(1, "let boundary = match extract_boundary(parsed) {");
    emitter.line(2, "Ok(boundary) => boundary,");
    emitter.line(2, "Err(_) => {");
    emitter.line(3, "return Err(malformed_body(");
    emitter.line(4, "\"multipart Content-Type lacks a usable boundary\",");
    emitter.line(3, "));");
    emitter.line(2, "}");
    emitter.line(1, "};");
    emitter.line(
        1,
        "let mut events: MultipartEvents = Box::pin(stream_multipart(",
    );
    emitter.line(2, "body.into_data_stream(),");
    emitter.line(2, "boundary,");
    emitter.line(2, "MultipartLimits::from_body_limits(limits),");
    emitter.line(1, "));");
    emitter.line(1, "let unknown_log =");
    emitter.line(
        2,
        "::std::sync::Arc::new(::std::sync::Mutex::new(MultipartUnknownLog::default()));",
    );

    // Per-field collection state plus the buffering stage machine.
    emitter.blank();
    emitter.line(1, "#[derive(Clone, Copy)]");
    emitter.line(1, "enum Stage {");
    emitter.line(2, "Idle,");
    emitter.line(2, "Drain,");
    for entry in &resolved {
        if matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart) {
            continue;
        }
        let variant = naming::ident(&entry.field.rust_name, NameStyle::Pascal);
        if entry.field.repeated {
            emitter.line(2, &format!("{variant}Element,"));
        } else {
            emitter.line(2, &format!("{variant},"));
        }
    }
    emitter.line(1, "}");
    emitter.line(1, "let mut stage = Stage::Idle;");
    for (field_index, entry) in resolved.iter().enumerate() {
        match &entry.field.kind {
            PlannedMultipartFieldKind::BinaryPart => {
                let part_name = layout
                    .multipart_part(op_index, field_index)
                    .expect("binary part registered");
                emitter.line(
                    1,
                    &format!("let mut {}_part: Option<{part_name}> = None;", entry.ident),
                );
            }
            kind => {
                if entry.field.repeated {
                    let vec_type = multipart_input_field_type(kind, true, true);
                    emitter.line(
                        1,
                        &format!("let mut {}: {vec_type} = Vec::new();", entry.ident),
                    );
                } else if let PlannedMultipartFieldKind::ScalarText(rust_type)
                | PlannedMultipartFieldKind::JsonPart(rust_type) = kind
                {
                    emitter.line(
                        1,
                        &format!("let mut {}: Option<{rust_type}> = None;", entry.ident),
                    );
                }
            }
        }
        // Binary parts never track `seen_` here: their PartBegin arm hands
        // off immediately, so a same-name reopening always arrives BEHIND
        // the stream where the tail scan's `seen_single_valued` rejects it
        // (§17.1 wire-arrival enforcement).
        let needs_seen = !entry.field.repeated
            && !matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart);
        if needs_seen {
            emitter.line(1, &format!("let mut seen_{} = false;", entry.ident));
        }
    }
    emitter.line(1, "let mut buffer: Vec<u8> = Vec::new();");

    // The single incremental pass over the framed message.
    emitter.blank();
    emitter.line(1, "loop {");
    emitter.line(
        2,
        "let event = match next_multipart_event(&mut events).await {",
    );
    emitter.line(3, "Some(event) => event,");
    emitter.line(3, "None => break,");
    emitter.line(2, "};");
    emitter.line(2, "match event {");
    emitter.line(3, "Err(error) => return Err(multipart_rejection(&error)),");
    emitter.line(3, "Ok(MultipartEvent::PartBegin(headers)) => {");
    emitter.line(4, "stage = match headers.name.as_str() {");
    let trailing_name = layout.multipart_trailing(op_index);
    for (field_index, entry) in resolved.iter().enumerate() {
        emit_collector_field_arm(
            emitter,
            op_index,
            layout,
            field_index,
            entry,
            &resolved,
            trailing_name,
        );
    }
    emitter.line(5, "other => {");
    emitter.line(6, "multipart_record_unknown(&unknown_log, other);");
    emitter.line(6, "Stage::Drain");
    emitter.line(5, "}");
    emitter.line(4, "};");
    emitter.line(3, "}");
    emitter.line(3, "Ok(MultipartEvent::PartChunk(chunk)) => match stage {");
    emitter.line(4, "Stage::Idle | Stage::Drain => {}");
    emitter.line(4, "_ => {");
    emitter.line(5, "buffer.extend_from_slice(&chunk);");
    emitter.line(5, "if buffer.len() > limits.multipart_scalar_part_bytes {");
    emitter.line(
        6,
        "return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));",
    );
    emitter.line(5, "}");
    emitter.line(4, "}");
    emitter.line(3, "},");
    emitter.line(3, "Ok(MultipartEvent::PartEnd) => match stage {");
    emitter.line(4, "Stage::Idle | Stage::Drain => {}");
    for entry in &resolved {
        if matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart) {
            continue;
        }
        emit_collector_decode_arm(emitter, entry, layout.runtime_validation);
    }
    emitter.line(3, "},");
    emitter.line(2, "}");
    emitter.line(1, "}");

    // Required-part enforcement is wire-arrival-based (§17.1, §38): parts
    // this pass consumed validated inline above; names still outstanding at
    // the streaming handoff ride `pending_required` on the live part, which
    // reports one terminal SchemaViolation when the framing ends without
    // them. Without a handoff this pass saw the whole message.
    let binary_ident = resolved
        .iter()
        .find(|entry| matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart))
        .map(|entry| entry.ident.clone());
    if let Some(binary) = &binary_ident {
        emitter.line(1, &format!("let handed_off = {binary}_part.is_some();"));
    }
    for entry in &resolved {
        if !entry.field.required || entry.field.repeated {
            continue;
        }
        match &entry.field.kind {
            PlannedMultipartFieldKind::BinaryPart => {
                emitter.line(
                    1,
                    &format!("let {} = match {}_part {{", entry.ident, entry.ident),
                );
                emitter.line(2, "Some(part) => part,");
                emitter.line(2, "None => {");
                emitter.line(
                    3,
                    &format!(
                        "return Err(schema_violation(\"missing \
                         required part `{}`\"));",
                        entry.field.wire_name
                    ),
                );
                emitter.line(2, "}");
                emitter.line(1, "};");
            }
            PlannedMultipartFieldKind::ScalarText(_) | PlannedMultipartFieldKind::JsonPart(_) => {
                emitter.line(
                    1,
                    &format!("let {} = match {} {{", entry.ident, entry.ident),
                );
                if binary_ident.is_some() {
                    emitter.line(2, "Some(value) => Some(value),");
                    emitter.line(2, "None if handed_off => None,");
                } else {
                    emitter.line(2, "Some(value) => value,");
                }
                emitter.line(2, "None => {");
                emitter.line(
                    3,
                    &format!(
                        "return Err(schema_violation(\"missing \
                         required part `{}`\"));",
                        entry.field.wire_name
                    ),
                );
                emitter.line(2, "}");
                emitter.line(1, "};");
            }
        }
    }

    // Assemble the owned input in declaration order; optional/repeated
    // fields keep their collected shape.
    emitter.blank();
    emitter.line(1, &format!("Ok({input_name} {{"));
    for entry in &resolved {
        match &entry.field.kind {
            PlannedMultipartFieldKind::BinaryPart if !entry.field.required => {
                emitter.line(2, &format!("{}: {}_part,", entry.ident, entry.ident));
            }
            _ => {
                emitter.line(2, &format!("{},", entry.ident));
            }
        }
    }
    emitter.line(2, "unknown_log,");
    emitter.line(1, "})");
    emitter.line(0, "}");
}

/// One `PartBegin` arm of the collector's name dispatch.
fn emit_collector_field_arm(
    emitter: &mut Emitter,
    op_index: usize,
    layout: &ServerLayout,
    field_index: usize,
    entry: &MultipartFieldIdents,
    resolved: &[MultipartFieldIdents],
    trailing_name: Option<&str>,
) {
    let wire = rust_string_literal(&entry.field.wire_name);
    emitter.line(5, &format!("{wire} => {{"));
    let needs_seen =
        !entry.field.repeated && !matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart);
    if needs_seen {
        emitter.line(6, &format!("if seen_{} {{", entry.ident));
        emitter.line(7, "return Err(schema_violation(format!(");
        emitter.line(8, "\"duplicate single-valued part `{}`\",");
        emitter.line(8, "headers.name");
        emitter.line(7, ")));");
        emitter.line(6, "}");
        emitter.line(6, &format!("seen_{} = true;", entry.ident));
    }
    match &entry.field.kind {
        PlannedMultipartFieldKind::ScalarText(_) | PlannedMultipartFieldKind::JsonPart(_) => {
            emitter.line(6, "buffer.clear();");
            let variant = naming::ident(&entry.field.rust_name, NameStyle::Pascal);
            if entry.field.repeated {
                emitter.line(6, &format!("Stage::{variant}Element"));
            } else {
                emitter.line(6, &format!("Stage::{variant}"));
            }
        }
        PlannedMultipartFieldKind::BinaryPart => {
            let part_name = layout
                .multipart_part(op_index, field_index)
                .expect("binary part registered");
            let tail_stage = format!("{part_name}TailStage");
            // Wire-arrival-based enforcement (§17.1): every required
            // scalar/JSON name still unseen rides on the live part and is
            // reported at its clean end-of-message; already-consumed
            // single-valued names stay duplicate-protected behind it.
            emitter.line(
                6,
                "let mut pending_required: Vec<&'static str> = Vec::new();",
            );
            for other in resolved {
                if other.field.required
                    && !other.field.repeated
                    && matches!(
                        other.field.kind,
                        PlannedMultipartFieldKind::ScalarText(_)
                            | PlannedMultipartFieldKind::JsonPart(_)
                    )
                {
                    emitter.line(6, &format!("if {}.is_none() {{", other.ident));
                    emitter.line(
                        7,
                        &format!(
                            "pending_required.push({});",
                            rust_string_literal(&other.field.wire_name)
                        ),
                    );
                    emitter.line(6, "}");
                }
            }
            emitter.line(6, "let mut seen_single_valued: Vec<String> = Vec::new();");
            for other in resolved {
                // The live binary seeds its own name unconditionally below;
                // plan time guarantees it is the body's only binary part.
                if !other.field.repeated
                    && !matches!(other.field.kind, PlannedMultipartFieldKind::BinaryPart)
                {
                    emitter.line(6, &format!("if seen_{} {{", other.ident));
                    emitter.line(
                        7,
                        &format!(
                            "seen_single_valued.push({}.to_owned());",
                            rust_string_literal(&other.field.wire_name)
                        ),
                    );
                    emitter.line(6, "}");
                }
            }
            emitter.line(
                6,
                &format!(
                    "seen_single_valued.push({}.to_owned());",
                    rust_string_literal(&entry.field.wire_name)
                ),
            );
            emitter.line(
                6,
                &format!("{ident}_part = Some({part_name} {{", ident = entry.ident),
            );
            emitter.line(7, "file_name: headers.filename,");
            emitter.line(7, "content_type: headers.content_type,");
            if let Some(trailing) = trailing_name {
                emitter.line(7, &format!("trailing_parts: {trailing}::default(),"));
            }
            emitter.line(7, "events,");
            emitter.line(7, "log: ::std::sync::Arc::clone(&unknown_log),");
            emitter.line(7, &format!("stage: {tail_stage}::Idle,"));
            emitter.line(7, "scalar_limit: limits.multipart_scalar_part_bytes,");
            emitter.line(7, "buffer: Vec::new(),");
            emitter.line(7, "finished: false,");
            emitter.line(7, "pending_required,");
            emitter.line(7, "seen_single_valued,");
            emitter.line(6, "});");
            emitter.line(6, "break;");
        }
    }
    emitter.line(5, "}");
}

/// One `PartEnd` decode arm: bounded-buffered bytes become typed values.
fn emit_collector_decode_arm(
    emitter: &mut Emitter,
    entry: &MultipartFieldIdents,
    validate_parts: bool,
) {
    let variant = naming::ident(&entry.field.rust_name, NameStyle::Pascal);
    let stage = if entry.field.repeated {
        format!("Stage::{variant}Element")
    } else {
        format!("Stage::{variant}")
    };
    emitter.line(4, &format!("{stage} => {{"));
    match &entry.field.kind {
        PlannedMultipartFieldKind::ScalarText(rust_type) => {
            let wire = rust_string_literal(&entry.field.wire_name);
            emitter.line(
                5,
                &format!("let value = multipart_scalar_text::<{rust_type}>({wire}, &buffer)?;"),
            );
            emit_part_validation_call(emitter, 5, entry, "&value", validate_parts);
        }
        PlannedMultipartFieldKind::JsonPart(model) => {
            let bind = format!("let value: {model} = decode_json_body(&buffer)?;");
            if fits(5, &bind) {
                emitter.line(5, &bind);
            } else {
                emitter.line(5, &format!("let value: {model} ="));
                emitter.line(6, "decode_json_body(&buffer)?;");
            }
        }
        PlannedMultipartFieldKind::BinaryPart => unreachable!("binary parts never buffer"),
    }
    if entry.field.repeated {
        emitter.line(5, &format!("{}.push(value);", entry.ident));
    } else {
        emitter.line(5, &format!("{} = Some(value);", entry.ident));
    }
    emitter.line(5, "stage = Stage::Idle;");
    emitter.line(4, "}");
}

// ----------------------------------------------------------------------
// Router registration (axum 0.8 keeps `{param}` placeholders verbatim)
// ----------------------------------------------------------------------

fn emit_router(
    emitter: &mut Emitter,
    plan: &PlannedApi,
    api_trait: &str,
    has_stream_responses: bool,
) {
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
    if has_stream_responses {
        // §40: fired when a committed stream fails mid-production.
        emitter.line(
            1,
            "stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,",
        );
    } else {
        // Accepted for API uniformity across generated crates; unused when
        // no operation streams a record-framed response.
        emitter.line(
            1,
            "_stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,",
        );
    }
    emitter.line(0, ") -> ::axum::Router {");
    emitter.line(1, "let state = ServerState {");
    emitter.line(2, "api,");
    emitter.line(2, "limits,");
    emitter.line(2, "encode_overflow_hook,");
    if has_stream_responses {
        emitter.line(2, "stream_failure_hook,");
    }
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

    // §45 codec helpers precede everything else so router bodies reference
    // generated, rustfmt-canonical call targets instead of inline fragments.
    if !flags.decode_codecs.is_empty() || !flags.encode_codecs.is_empty() {
        emitter.blank();
        emit_codec_helpers(emitter, flags);
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
    if flags.needs_form_decode {
        emitter.blank();
        emit_decode_form_body(emitter);
    }
    if flags.needs_multipart {
        emitter.blank();
        emit_multipart_shared_helpers(emitter);
        emitter.blank();
        emit_multipart_rejection(emitter);
        emitter.blank();
        emit_multipart_schema_violation(emitter);
    }
    if flags.needs_multipart_scalar {
        emitter.blank();
        emit_multipart_scalar_text(emitter);
    }
    if flags.needs_request_schema_violation {
        emitter.blank();
        emit_request_schema_violation(emitter);
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
    if !flags.response_framings.is_empty() {
        emitter.blank();
        emit_stream_body_encoder(emitter);
        emitter.blank();
        emit_stream_body_encoder_ctor(emitter);
    }
    if flags.needs_any_response {
        emitter.blank();
        emit_any_response(emitter);
    }
    if flags.needs_typed_headers {
        emitter.blank();
        emit_write_typed_headers(emitter);
        emitter.blank();
        emit_header_encode_failure(emitter);
    }
}

/// Writes the collected typed documented headers onto an encoded response
/// (main spec §15): a value failing `HeaderValue` conversion discards the
/// partial response and takes the fixed §34.1-style fallback.
/// Emits the per-codec bounded decode/encode helpers (main spec §45) composed
/// from the plugin fragments, plus the XML fmt-sink adapter. One helper per
/// codec actually referenced on this side keeps `-D warnings` clean.
fn emit_codec_helpers(emitter: &mut Emitter, flags: &Flags) {
    let registry = codec_registry();
    let mut ids: Vec<&String> = flags.decode_codecs.union(&flags.encode_codecs).collect();
    ids.sort();
    let mut xml_sink_emitted = false;
    for id in ids {
        let Some(plugin) = registry.iter().find(|plugin| plugin.id() == id.as_str()) else {
            continue;
        };
        let prefix = helper_prefix(plugin.id());
        if plugin.id() == "xml" && flags.encode_codecs.contains(id.as_str()) && !xml_sink_emitted {
            emit_xml_fmt_sink(emitter);
            emitter.blank();
            xml_sink_emitted = true;
        }
        if flags.decode_codecs.contains(id.as_str()) {
            emitter.docs(
                0,
                &[
                    format!(
                        "Typed `{}` request-body decode from ALREADY-bounded bytes \
                         (main spec §45, D-impl-codec-plugins): the §28 Content-Type \
                         gate and the `structured_request_bytes` collection ran \
                         BEFORE this parse.",
                        plugin.id()
                    ),
                    "The schema/data distinction is not portable across codecs, so \
                     ALL decode failures — including missing-required-style errors — \
                     map onto MalformedBody 400 (documented deviation from §39 row 6)."
                        .to_owned(),
                ],
            );
            emitter.block(
                0,
                &format!(
                    "#[allow(clippy::missing_errors_doc)]\nfn {prefix}_decode_body<T>(bytes: \
                     &[u8]) -> Result<T, ProtocolRejection>"
                ),
            );
            emitter.line(0, "where");
            emitter.line(1, "T: serde::de::DeserializeOwned,");
            emitter.line(0, "{");
            emitter.block(1, &plugin.server_decode_expr("bytes", "T"));
            emitter.line(0, "}");
            emitter.blank();
        }
        if flags.encode_codecs.contains(id.as_str()) {
            emitter.docs(
                0,
                &[
                    format!(
                        "Bounded `{}` response encoding (main spec §45/§34/§41); the \
                         literal keeps distinct types such as application/xml separate \
                         from text/xml.",
                        plugin.id()
                    ),
                    "Bytes stream through the fail-fast counting writer: overflow \
                     discards partial output, fires the hook, and emits the fixed \
                     empty 500 (§34.1)."
                        .to_owned(),
                ],
            );
            emitter.line(0, &format!("fn {prefix}_encode_limited<T>("));
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
            emitter.block(1, &plugin.server_encode_stmts("value", "budget", "encoded"));
            emitter.line(1, "match encoded {");
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
    }
}

/// Adapts the byte-oriented counting writer to quick-xml's text-oriented
/// serializer target; XML output is UTF-8 text, so the conversion never loses
/// bytes.
fn emit_xml_fmt_sink(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "quick-xml serializes through `std::fmt::Write`; this sink forwards \
           the UTF-8 text into the byte-counting writer unchanged."
                .to_owned(),
        ],
    );
    emitter.line(0, "struct XmlFmtSink<'a, W>(&'a mut W);");
    emitter.blank();
    emitter.line(
        0,
        "impl<W: ::std::io::Write> ::std::fmt::Write for XmlFmtSink<'_, W> {",
    );
    emitter.line(
        1,
        "fn write_str(&mut self, text: &str) -> ::std::fmt::Result {",
    );
    // The chain exceeds rustfmt's chain_width, so the canonical form breaks
    // after the receiver.
    emitter.line(2, "self.0");
    emitter.line(3, ".write_all(text.as_bytes())");
    emitter.line(3, ".map_err(|_| ::std::fmt::Error)");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_write_typed_headers(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Appends typed documented response headers (main spec §15). A \
             value that cannot become a `HeaderValue` fires the encode hook \
             and emits the fixed empty 500 (§34.1 machinery; limit `0` is \
             the recorded sentinel for non-size encode failures such as \
             this one)."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn write_typed_headers(");
    emitter.line(1, "mut response: ::axum::response::Response,");
    emitter.line(1, "hook: &dyn EncodeOverflowHook,");
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(1, "variant: &'static str,");
    emitter.line(1, "headers: &[(&'static str, String)],");
    emitter.line(0, ") -> ::axum::response::Response {");
    emitter.line(1, "for (wire, value) in headers {");
    emitter.line(2, "match ::http::HeaderValue::try_from(value.as_str()) {");
    emitter.line(3, "Ok(header) => {");
    emitter.line(4, "response");
    emitter.line(5, ".headers_mut()");
    emitter.line(5, ".insert(::http::HeaderName::from_static(wire), header);");
    emitter.line(3, "}");
    emitter.line(3, "Err(_) => {");
    emitter.line(
        4,
        "return header_encode_failure(hook, operation_id, variant);",
    );
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(1, "response");
    emitter.line(0, "}");
}

/// §34.1 machinery applied to header-conversion failures: fire the hook,
/// then emit the protocol-safe fixed 500 with an empty body.
fn emit_header_encode_failure(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[
            "Fixed fallback for a documented header value that fails HTTP \
           header conversion at encode time (main spec §48's internal error \
           path): hook first, then the empty-bodied 500."
                .to_owned(),
        ],
    );
    emitter.line(0, "fn header_encode_failure(");
    emitter.line(1, "hook: &dyn EncodeOverflowHook,");
    emitter.line(1, "operation_id: &'static str,");
    emitter.line(1, "variant: &'static str,");
    emitter.line(0, ") -> ::axum::response::Response {");
    emitter.line(1, "hook.on_encode_overflow(operation_id, variant, 0);");
    emitter.line(
        1,
        "::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()",
    );
    emitter.line(0, "}");
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

/// Maps bounded form decode failures onto §39 kinds (main spec §16,
/// D-impl-charset-rejection, D-impl-runtime-validation-timing): the size
/// gate → 413 BodyTooLarge, syntax/charset failures → 400 MalformedBody,
/// data errors (missing fields, duplicates, wrong types) → 422
/// SchemaViolation. Axum's `Form` extractor is never used; the router
/// self-decodes from the bounded collect.
fn emit_decode_form_body(emitter: &mut Emitter) {
    emitter.line(
        0,
        "fn decode_form_body<T>(bytes: &[u8], limit: usize) -> Result<T, ProtocolRejection>",
    );
    emitter.line(0, "where");
    emitter.line(1, "T: serde::de::DeserializeOwned,");
    emitter.line(0, "{");
    emitter.line(1, "match decode_form_limited(bytes, limit) {");
    emitter.line(2, "Ok(value) => Ok(value),");
    emitter.line(
        2,
        "Err(::openapi_support::form::FormDecodeError::TooLarge { .. }) => {",
    );
    // Defensive: bounded collection already enforced this limit.
    emitter.line(
        3,
        "Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))",
    );
    emitter.line(2, "}");
    emitter.line(2, "Err(error) => {");
    emitter.line(3, "if error.is_syntax() {");
    emitter.line(4, "Err(malformed_body(\"malformed form body\"))");
    emitter.line(3, "} else {");
    emitter.line(
        4,
        "Err(ProtocolRejection::new(RejectionKind::SchemaViolation)",
    );
    emitter.line(
        5,
        ".with_detail(\"well-formed body failed schema validation\"))",
    );
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Shared machinery behind every generated multipart route (main spec
/// §5.5/§17): the boxed event stream shared between the router's validation
/// pass and the streaming parts, plus the unknown-part log and its
/// poison-safe recorder.
fn emit_multipart_shared_helpers(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[String::from(
            "Boxed multipart event stream shared between the router's \
             single-pass validation and the live streaming part handed to \
             the application (main spec §17).",
        )],
    );
    emitter.line(0, "type MultipartEvents = ::std::pin::Pin<");
    emitter.line(1, "Box<");
    emitter.line(
        2,
        "dyn ::futures_core::Stream<Item = Result<MultipartEvent, MultipartError>>",
    );
    emitter.line(3, "+ ::std::marker::Send,");
    emitter.line(1, ">,");
    emitter.line(0, ">;");
    emitter.blank();
    emitter.docs(
        0,
        &["Polls the next framing event without extension traits.".to_owned()],
    );
    emitter.line(0, "#[allow(clippy::needless_pass_by_ref_mut)]");
    emitter.line(0, "async fn next_multipart_event(");
    emitter.line(1, "events: &mut MultipartEvents,");
    emitter.line(0, ") -> Option<Result<MultipartEvent, MultipartError>> {");
    emitter.line(
        1,
        "::std::future::poll_fn(|cx| events.as_mut().poll_next(cx)).await",
    );
    emitter.line(0, "}");
    emitter.blank();
    emitter.docs(
        0,
        &[String::from(
            "Observability log of unrecognized/late part names (§17.1 \
              unknown-fields-ignore default); payloads are never retained.",
        )],
    );
    emitter.line(0, "#[derive(Default)]");
    emitter.line(0, "struct MultipartUnknownLog {");
    emitter.line(1, "names: Vec<String>,");
    emitter.line(0, "}");
    emitter.blank();
    emitter.docs(
        0,
        &["Records one observed name, surviving mutex poisoning.".to_owned()],
    );
    emitter.line(
        0,
        "fn multipart_record_unknown(log: &::std::sync::Mutex<MultipartUnknownLog>, name: &str) {",
    );
    emitter.line(1, "match log.lock() {");
    emitter.line(2, "Ok(mut guard) => guard.names.push(name.to_owned()),");
    emitter.line(
        2,
        "Err(poisoned) => poisoned.into_inner().names.push(name.to_owned()),",
    );
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Maps framing-engine failures onto the canonical §39 rows: cardinality
/// limits → BodyTooLarge 413; truncation and malformed framing →
/// MalformedBody 400.
fn emit_multipart_rejection(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[String::from(
            "Maps a multipart framing failure onto the §39 mapping table: \
             cardinality limits (part count, header budget, name lengths) \
             are bounded-collection rejections; truncation and malformed \
             framing are syntactic failures.",
        )],
    );
    emitter.line(
        0,
        "fn multipart_rejection(error: &MultipartError) -> ProtocolRejection {",
    );
    emitter.line(1, "match error {");
    emitter.line(2, "MultipartError::TooManyParts { .. }");
    emitter.line(2, "| MultipartError::PartHeaderTooLarge { .. }");
    emitter.line(2, "| MultipartError::FieldNameTooLong { .. }");
    emitter.line(2, "| MultipartError::FileNameTooLong { .. } => {");
    emitter.line(3, "ProtocolRejection::new(RejectionKind::BodyTooLarge)");
    emitter.line(2, "}");
    emitter.line(2, "MultipartError::Truncated => {");
    emitter.line(
        3,
        "malformed_body(\"multipart stream ended before the closing boundary\")",
    );
    emitter.line(2, "}");
    emitter.line(
        2,
        "MultipartError::MalformedFraming => malformed_body(\"malformed multipart framing\"),",
    );
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Runs one companion §9 body/part validator AFTER a successful decode: a
/// violation becomes SchemaViolation 422 with a location-prefixed detail;
/// the handler never observes it (§39 rules 1/3). Taking the WHOLE result
/// keeps every call site a single flat call.
fn emit_request_schema_violation(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[String::from(
            "Runs one companion §9 request-body/part validator after decode: \
             a violation rejects 422 SchemaViolation outside the documented \
             enum, with a location-prefixed diagnostic detail (§39 rows 6; \
             details stay off the wire per rule 3).",
        )],
    );
    emitter.line(0, "fn require_valid_request(");
    emitter.line(1, "location: &str,");
    emitter.line(
        1,
        "validation: ::std::result::Result<(), ::openapi_support::validation::Violation>,",
    );
    emitter.line(0, ") -> ::std::result::Result<(), ProtocolRejection> {");
    emitter.line(1, "validation.map_err(|violation| {");
    emitter.line(
        2,
        "ProtocolRejection::new(RejectionKind::SchemaViolation).with_detail(format!(",
    );
    emitter.line(
        3,
        "\"request body failed schema validation at `{location}`: {violation}\",",
    );
    emitter.line(2, "))");
    emitter.line(1, "})");
    emitter.line(0, "}");
}

/// Well-formed parts failing schema validation reject 422 (§17.1, §39 row 6).
fn emit_multipart_schema_violation(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[String::from(
            "Well-formed multipart content failing schema validation \
             (missing required part, duplicate single-valued part, bad \
             scalar value) → 422 (§17.1, §39 mapping row 6).",
        )],
    );
    emitter.line(
        0,
        "fn schema_violation(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {",
    );
    emitter.line(
        1,
        "ProtocolRejection::new(RejectionKind::SchemaViolation).with_detail(detail)",
    );
    emitter.line(0, "}");
}

/// Decodes one bounded textual scalar part: strict UTF-8 first (400), then
/// `FromStr` typing (well-formed bytes failing the schema → 422).
fn emit_multipart_scalar_text(emitter: &mut Emitter) {
    emitter.docs(
        0,
        &[String::from(
            "Decodes one bounded textual scalar part (§17.1): non-UTF-8 \
             bytes are MalformedBody 400; well-formed text failing its Rust \
             type is SchemaViolation 422.",
        )],
    );
    emitter.line(
        0,
        "fn multipart_scalar_text<T>(wire: &'static str, bytes: &[u8]) -> Result<T, ProtocolRejection>",
    );
    emitter.line(0, "where");
    emitter.line(1, "T: ::std::str::FromStr,");
    emitter.line(0, "{");
    emitter.line(1, "let text = std::str::from_utf8(bytes)");
    emitter.line(
        2,
        ".map_err(|_| malformed_body(\"multipart part is not valid UTF-8\"))?;",
    );
    emitter.line(1, "text.parse()");
    emitter.line(
        2,
        ".map_err(|_| schema_violation(format!(\"part `{wire}` failed schema validation\")))",
    );
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
    // The parse chain exceeds rustfmt's default chain_width, so the
    // canonical form breaks after the head expression.
    emitter.line(1, "format!(\"{}/{}\", parsed.ty, subtype)");
    emitter.line(2, ".parse()");
    emitter.line(2, ".unwrap_or(::mime::STAR_STAR)");
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
    emitter.line(1, "let header =");
    emitter.line(
        2,
        "::http::HeaderValue::try_from(declared).unwrap_or(::http::HeaderValue::from_static(\"*/*\"));",
    );
    // The insert chain exceeds rustfmt's default chain_width.
    emitter.line(1, "response");
    emitter.line(2, ".headers_mut()");
    emitter.line(2, ".insert(::http::header::CONTENT_TYPE, header);");
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
