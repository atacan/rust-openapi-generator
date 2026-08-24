//! Reqwest client emission (main spec §8 Output A shape, §26–§29, §30.1,
//! §32, §34.2, §36; DECISIONS.md D-impl-clienterror-location,
//! D-impl-typed-headers-phase2, D-impl-relative-servers,
//! D-impl-singlefile-layout).
//!
//! Consumes the shared [`crate::codegen::plan`] and renders ONE deterministic
//! `client.rs` module: a [`Client`]/[`ClientBuilder`] pair plus per-operation
//! response enums, nested content enums, streaming status wrappers, request
//! body enums, and bounded/streaming method bodies. Generated code references
//! only `::openapi_support`, `::reqwest`, `::http`, `::mime`, `::bytes`,
//! `::futures_core`, `serde_json`, and `super::models`.
//!
//! Companion §8 server handling: operation-level `servers` override
//! path-level override root-level, and the first entry of each effective
//! array is that operation's default base. Every DISTINCT effective default
//! URL becomes its own stored base — `base_url` holds the primary (the first
//! operation's first effective server, backwards-compatible construction)
//! and further bases live in `base_url_<key>` fields with deterministic
//! snake_case keys. Recorded DECISION (D-impl-relative-servers follow-up,
//! documented in the emitted module docs): an explicit
//! [`ClientBuilder::base_url`] replaces ONLY the primary base; secondary
//! bases are overridden individually through
//! [`ClientBuilder::secondary_base_url`], so a relative secondary still
//! requires its own absolute value.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::document::{
    HttpMethod, MediaClass, ParameterLocation, ParameterStyle, RangeClass, ResponseStatusKey,
};
use crate::normalize::naming;
use crate::normalize::NormalizedDocument;

use super::plan::{
    PlannedApi, PlannedContent, PlannedMultipart, PlannedMultipartField, PlannedMultipartFieldKind,
    PlannedOperation, PlannedParameter, PlannedStatus,
};
use super::Emitter;

const RUSTFMT_MAX_WIDTH: usize = 100;

/// Renders ONE generated `client.rs` for the planned API (main spec §3,
/// D-impl-singlefile-layout).
#[must_use]
pub fn generate_client(doc: &NormalizedDocument, plan: &PlannedApi) -> String {
    let mut flags = Flags::default();
    let mut used_names = reserved_names(doc);
    let layout = Layout::new(plan, &mut used_names);

    for operation in &plan.operations {
        flags.scan_operation(operation);
    }

    let mut emitter = Emitter::new();
    emit_header(&mut emitter, doc, &flags);
    let bases = BaseSet::new(plan);
    emit_client(&mut emitter, &bases);
    emit_builder(&mut emitter, &bases);
    for (op_index, operation) in plan.operations.iter().enumerate() {
        emit_operation_definitions(&mut emitter, op_index, operation, &layout);
    }
    emit_methods(&mut emitter, plan, &bases, &layout, &mut flags);
    emit_module_helpers(&mut emitter, &flags, bases.has_any_variables());
    emitter.finish()
}

/// Names already taken in the module scope: every assigned schema type
/// (including `<Type>Fallback` shapes) and every response enum. Generated
/// nested names never collide with them after suffixing.
fn reserved_names(doc: &NormalizedDocument) -> BTreeSet<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    for schema in doc.schemas.values() {
        used.insert(schema.rust_type.clone());
        used.insert(format!("{}Fallback", schema.rust_type));
    }
    for (_, enum_name) in &doc.names.response_enums {
        used.insert(enum_name.clone());
    }
    used
}

/// Deterministic collision-free type name in the client module scope.
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

/// Module-level type-name registry for one API: content enums and streaming
/// wrappers get `<Op><Status>[Content]` names with numeric collision
/// suffixes ordered by document position (companion §10). Multipart request
/// bodies register their `<Op>Request` input struct (§17 Output A).
#[derive(Debug, Default)]
struct Layout {
    /// (operation document index, status index) → generated type name.
    content_enums: BTreeMap<(usize, usize), String>,
    wrappers: BTreeMap<(usize, usize), String>,
    /// Operation document index → `<Op>Request` multipart input struct.
    request_structs: BTreeMap<usize, String>,
}

impl Layout {
    fn new(plan: &PlannedApi, used: &mut BTreeSet<String>) -> Self {
        let mut layout = Self::default();
        for (op_index, operation) in plan.operations.iter().enumerate() {
            if operation
                .request_contents
                .iter()
                .any(|content| content.media_class == MediaClass::Multipart)
            {
                let base = format!("{}Request", operation.pascal);
                let name = fresh_name(used, base);
                layout.request_structs.insert(op_index, name);
            }
            for (status_index, status) in operation.statuses.iter().enumerate() {
                if status.contents.len() >= 2 {
                    let base = format!("{}{}Content", operation.pascal, status_name_part(status));
                    let name = fresh_name(used, base);
                    layout.content_enums.insert((op_index, status_index), name);
                }
                if let Some(name) = wrapper_name(status) {
                    let name = fresh_name(used, format!("{}{}", operation.pascal, name));
                    layout.wrappers.insert((op_index, status_index), name);
                }
            }
        }
        layout
    }

    fn request_struct(&self, op_index: usize) -> Option<&str> {
        self.request_structs.get(&op_index).map(String::as_str)
    }
}

/// Statuses whose payload owns a wrapper struct named `<Op><Status>`
/// (main spec §32/§15): streaming single-content statuses carry the raw
/// response; decodable single-content statuses WITH documented headers
/// become §15 Output A wrappers (`{ typed header fields..., body }`).
/// No-body statuses never wrap (§35); range/default statuses with headers
/// keep their struct variants (which carry the wire status, §23–§24).
fn wrapper_name(status: &PlannedStatus) -> Option<String> {
    if status.is_no_body_status {
        return None;
    }
    let is_range = !matches!(status.key, ResponseStatusKey::Explicit(_));
    let [content] = status.contents.as_slice() else {
        return None;
    };
    if content.is_wildcard
        || matches!(
            content.media_class,
            MediaClass::Binary | MediaClass::RawUnknown
        )
    {
        // Streaming/wildcard payloads keep the Phase 1 wrapper shape unless
        // a range/default needs its inline struct variant for headers.
        if status.headers.is_empty() || !is_range {
            return Some(status_name_part(status));
        }
        return None;
    }
    if !status.headers.is_empty() && !is_range {
        return Some(status_name_part(status));
    }
    None
}

/// Status portion of derived type names per the §4 table
/// (`GetArtifact200Content`, `GetObject200`): explicit codes contribute their
/// digits; ranges and `default` use the full variant name.
fn status_name_part(status: &PlannedStatus) -> String {
    match status.key {
        ResponseStatusKey::Explicit(code) => code.to_string(),
        _ => status.enum_variant.clone(),
    }
}

/// Emission flags gathered in one deterministic scan so the import block and
/// module helpers contain exactly what the bodies reference (warning-free
/// under `-D warnings`).
#[derive(Debug, Default)]
struct Flags {
    model_types: BTreeSet<String>,
    needs_body_limit_direction: bool,
    needs_serialize_json: bool,
    needs_serialize_form: bool,
    needs_collect: bool,
    needs_content_type_helpers: bool,
    needs_charset_check: bool,
    needs_empty_json_body: bool,
    needs_json_decode: bool,
    needs_text_decode: bool,
    needs_encode_overflow: bool,
    needs_negotiation_rank: bool,
    needs_encode_path: bool,
    needs_query_pairs: bool,
    needs_header_value: bool,
    needs_cookie_value: bool,
    needs_param_spec: bool,
    needs_param_style: bool,
    /// Typed documented response headers exist somewhere (main spec §15):
    /// pulls in the shared header-value decode helper plus its source-error
    /// type.
    needs_response_header_parse: bool,
    /// At least one REQUIRED documented header exists → the required
    /// variant of the parse helper is emitted.
    needs_required_header_helper: bool,
    /// At least one OPTIONAL documented header exists → the optional
    /// variant of the parse helper is emitted.
    needs_optional_header_helper: bool,
    /// A multipart request body exists somewhere (main spec §17): pulls in
    /// the `reqwest::multipart` form builder plus the per-part mime helper.
    needs_multipart: bool,
}

impl Flags {
    fn scan_operation(&mut self, operation: &PlannedOperation) {
        for content in &operation.request_contents {
            self.scan_request_content(content);
        }
        for status in &operation.statuses {
            self.scan_status(status);
            if status.contents.len() >= 2 {
                self.needs_negotiation_rank = true;
            }
        }
        for parameter in &operation.parameters {
            self.needs_param_spec = true;
            self.needs_param_style = true;
            match parameter.location {
                ParameterLocation::Path => self.needs_encode_path = true,
                ParameterLocation::Query => self.needs_query_pairs = true,
                ParameterLocation::Header => self.needs_header_value = true,
                ParameterLocation::Cookie => self.needs_cookie_value = true,
            }
        }
    }

    fn scan_request_content(&mut self, content: &PlannedContent) {
        match content.media_class {
            MediaClass::JsonFamily => {
                self.needs_serialize_json = true;
                self.needs_body_limit_direction = true;
                self.model_types
                    .extend(model_type_names(&content.model_expr));
            }
            MediaClass::UrlEncodedForm => {
                // Bounded form serialization per §34/D-impl-forms; Reqwest's
                // `.form()` convenience is never used.
                self.needs_serialize_form = true;
                self.needs_body_limit_direction = true;
                self.model_types
                    .extend(model_type_names(&content.model_expr));
            }
            MediaClass::Multipart => {
                // §17 Output A: owned input struct; JSON parts serialize
                // bounded BEFORE any wire traffic (§34.2); binary parts stay
                // streaming (`reqwest::Body`).
                self.needs_multipart = true;
                self.needs_body_limit_direction = true;
                if let Some(spec) = &content.multipart_spec {
                    for field in &spec.fields {
                        if let PlannedMultipartFieldKind::JsonPart(model) = &field.kind {
                            self.needs_serialize_json = true;
                            self.needs_encode_overflow = true;
                            let cleaned = model.strip_prefix("Option<").unwrap_or(model);
                            let cleaned = cleaned.strip_suffix(">").unwrap_or(cleaned);
                            self.model_types.extend(model_type_names(cleaned));
                        }
                    }
                }
            }
            MediaClass::PlainText => self.needs_body_limit_direction = true,
            MediaClass::Binary | MediaClass::RawUnknown => {}
            // Planning rejects the rest; unreachable here.
            _ => {}
        }
    }

    fn scan_status(&mut self, status: &PlannedStatus) {
        if !status.headers.is_empty() {
            self.needs_response_header_parse = true;
            for header in &status.headers {
                if header.required {
                    self.needs_required_header_helper = true;
                } else {
                    self.needs_optional_header_helper = true;
                }
            }
        }
        if status.is_no_body_status || status.contents.is_empty() {
            return;
        }
        let textual = status.contents.iter().any(|content| {
            matches!(
                content.media_class,
                MediaClass::JsonFamily | MediaClass::PlainText | MediaClass::UrlEncodedForm
            )
        });
        let negotiated = status.contents.len() >= 2;
        if textual || negotiated {
            self.needs_content_type_helpers = true;
        }
        if textual {
            self.needs_charset_check = true;
            self.needs_collect = true;
        }
        for content in &status.contents {
            match content.media_class {
                MediaClass::JsonFamily => {
                    self.model_types
                        .extend(model_type_names(&content.model_expr));
                    self.needs_collect = true;
                    self.needs_empty_json_body = true;
                }
                MediaClass::PlainText => self.needs_collect = true,
                MediaClass::Binary | MediaClass::RawUnknown => {}
                _ => {}
            }
        }
    }
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
// Module header, imports, Client/ClientBuilder
// ----------------------------------------------------------------------

fn emit_header(emitter: &mut Emitter, doc: &NormalizedDocument, flags: &Flags) {
    emitter.docs(
        0,
        &[
            "Reqwest client generated from the OpenAPI document (main spec §8 \
             Output A)."
                .to_owned(),
            String::new(),
            "Bounded JSON/form bodies (§34), streaming raw payloads (§32), \
             exhaustive documented-status enums (§2.4), typed documented \
             response headers (§15), redirects off by default (§30.1), and \
             the authoritative `ClientError` (§36). Recorded decision for \
             multi-content statuses WITH documented headers: the typed fields \
             hoist onto the status VARIANT beside the content enum. The \
             source document declares OpenAPI "
                .to_owned()
                + &doc.raw_version
                + ".",
            String::new(),
            "Servers (companion §8): operation-level `servers` override \
             path-level, path-level overrides root-level, and within each \
             effective array the first entry is that operation's default \
             base. Every DISTINCT effective default URL becomes its own \
             stored base: `base_url` is the primary (the first operation's \
             first effective server); further bases live in \
             `base_url_<key>` fields whose keys are documented under \
             `ClientBuilder::secondary_base_url`. Recorded decision: an \
             explicit `base_url` replaces ONLY the primary base; each other \
             base needs its own `secondary_base_url` override, so a relative \
             secondary still requires an absolute value there \
             (D-impl-relative-servers)."
                .to_owned(),
            "Generated deterministically byte-for-byte (main spec §50 test 39); \
             do not edit by hand."
                .to_owned(),
        ],
    );

    let mut imports: Vec<String> = Vec::new();
    if !flags.model_types.is_empty() {
        if flags.model_types.len() == 1 {
            let only = flags.model_types.iter().next().expect("one model type");
            imports.push(format!("use super::models::{only};"));
        } else {
            let models: Vec<String> = flags.model_types.iter().cloned().collect();
            let joined = models.join(", ");
            imports.push(format!("use super::models::{{{joined}}};"));
        }
    }
    if flags.needs_body_limit_direction {
        imports.push(
            "use ::openapi_support::client_error::{BodyLimitDirection, ClientError};".to_owned(),
        );
    } else {
        imports.push("use ::openapi_support::client_error::ClientError;".to_owned());
    }
    if flags.needs_collect {
        imports.push("use ::openapi_support::collect::collect_reqwest_limited;".to_owned());
    }
    if flags.needs_serialize_form {
        imports.push("use ::openapi_support::encode::serialize_form_limited;".to_owned());
    }
    if flags.needs_serialize_json {
        imports.push("use ::openapi_support::encode::serialize_json_limited;".to_owned());
    }
    imports.push("use ::openapi_support::limits::BodyLimits;".to_owned());
    if flags.needs_content_type_helpers {
        if flags.needs_negotiation_rank {
            imports.push(
                "use ::openapi_support::mediatype::{match_entry, ParsedMediaType};".to_owned(),
            );
        } else {
            imports.push("use ::openapi_support::mediatype::ParsedMediaType;".to_owned());
        }
    }

    let mut params: Vec<String> = Vec::new();
    if flags.needs_cookie_value {
        params.push("encode_cookie_value".to_owned());
    }
    if flags.needs_header_value {
        params.push("encode_header_value".to_owned());
    }
    if flags.needs_encode_path {
        params.push("encode_path".to_owned());
    }
    if flags.needs_query_pairs {
        params.push("encode_query_pairs".to_owned());
    }
    if flags.needs_param_spec {
        params.push("ParamSpec".to_owned());
    }
    if flags.needs_param_style {
        params.push("ParamStyle".to_owned());
    }
    if flags.needs_param_spec {
        params.push("ParamValue".to_owned());
    }
    if !params.is_empty() {
        // rustfmt keeps short brace lists on one line.
        let joined = params.join(", ");
        imports.push(format!("use ::openapi_support::params::{{{joined}}};"));
    }
    if flags.needs_multipart {
        imports.push("use ::reqwest::multipart::{Form, Part};".to_owned());
    }

    for import in &imports {
        emitter.line(0, import);
    }
}

fn emit_client(emitter: &mut Emitter, bases: &BaseSet) {
    emitter.blank();
    emitter.docs(
        0,
        &[
            "Client carrying one resolved base per distinct effective default \
             server (companion §8): `base_url` is the PRIMARY base (the first \
             operation's first effective server); every further distinct URL \
             gets its own `base_url_<key>` field, and each generated method \
             sends through its operation's own base."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[derive(Clone)]");
    emitter.line(0, "pub struct Client {");
    emitter.line(1, "http: ::reqwest::Client,");
    emitter.line(1, "base_url: String,");
    emitter.line(1, "limits: BodyLimits,");
    for entry in bases.secondaries() {
        emitter.line(1, &format!("{}: String,", entry.field));
    }
    emitter.line(0, "}");
}

fn emit_builder(emitter: &mut Emitter, bases: &BaseSet) {
    let primary = bases.primary();
    emitter.blank();
    emitter.docs(
        0,
        &[
            "Builder for `Client` (main spec §30.1): redirects disabled unless \
             opted in through `follow_redirects`; relative default servers \
             require explicit overrides (D-impl-relative-servers). Recorded \
             decision (companion §8): an explicit `base_url` replaces ONLY \
             the primary base; every additional base is overridden per key \
             through `secondary_base_url`."
                .to_owned(),
        ],
    );
    emitter.line(0, "pub struct ClientBuilder {");
    emitter.line(1, "http: ::reqwest::ClientBuilder,");
    emitter.line(1, "base_url: Option<String>,");
    emitter.line(1, "limits: BodyLimits,");
    emitter.line(1, "default_server_url: String,");
    emitter.line(
        1,
        "default_server_variables: Vec<(String, String, Option<Vec<String>>)>,",
    );
    if bases.has_secondaries() {
        emitter.line(
            1,
            "secondary_base_urls: ::std::collections::BTreeMap<String, String>,",
        );
    }
    for entry in bases.secondaries() {
        if entry.variables.is_empty() {
            continue;
        }
        emitter.line(
            1,
            &format!(
                "{}_server_variables: Vec<(String, String, Option<Vec<String>>)>,",
                entry.key()
            ),
        );
    }
    emitter.line(
        1,
        "server_variables: ::std::collections::BTreeMap<String, String>,",
    );
    emitter.line(0, "}");
    emitter.blank();

    emitter.line(0, "impl Default for ClientBuilder {");
    emitter.line(1, "fn default() -> Self {");
    emitter.line(2, "Self::new()");
    emitter.line(1, "}");
    emitter.line(0, "}");
    emitter.blank();

    emitter.line(0, "impl ClientBuilder {");
    emitter.docs(
        1,
        &[
            "Process-default transport: no redirects (§30.1) and process-default \
              body limits (§33)."
                .to_owned(),
        ],
    );
    emitter.line(1, "#[must_use]");
    emitter.line(1, "pub fn new() -> Self {");
    let url_line = format!(
        "let default_server_url = {}.to_owned();",
        rust_string_literal(&primary.url)
    );
    if fits(2, &url_line) {
        emitter.line(2, &url_line);
    } else {
        emitter.line(2, "let default_server_url =");
        emitter.line(
            3,
            &format!("{}.to_owned();", rust_string_literal(&primary.url)),
        );
    }
    if primary.variables.is_empty() {
        emitter.line(2, "let default_server_variables = Vec::new();");
    } else {
        emit_variables_binding(emitter, 2, "default_server_variables", &primary.variables);
    }
    for entry in bases.secondaries() {
        if entry.variables.is_empty() {
            continue;
        }
        emit_variables_binding(
            emitter,
            2,
            &format!("{}_server_variables", entry.key()),
            &entry.variables,
        );
    }
    emitter.line(2, "Self {");
    emitter.line(
        3,
        "http: ::reqwest::Client::builder().redirect(::reqwest::redirect::Policy::none()),",
    );
    emitter.line(3, "base_url: None,");
    emitter.line(3, "limits: BodyLimits::process_default(),");
    emitter.line(3, "default_server_url,");
    emitter.line(3, "default_server_variables,");
    if bases.has_secondaries() {
        emitter.line(
            3,
            "secondary_base_urls: ::std::collections::BTreeMap::new(),",
        );
        for entry in bases.secondaries() {
            if !entry.variables.is_empty() {
                emitter.line(3, &format!("{}_server_variables,", entry.key()));
            }
        }
    }
    emitter.line(3, "server_variables: ::std::collections::BTreeMap::new(),");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.blank();

    emitter.docs(
        1,
        &[
            "Overrides the resolved PRIMARY base URL only (recorded companion \
              §8 decision); required before `build` when the primary default \
              server is not absolute. Secondary bases are overridden through \
              `secondary_base_url`."
                .to_owned(),
        ],
    );
    emitter.line(
        1,
        "pub fn base_url(mut self, value: impl Into<String>) -> Self {",
    );
    emitter.line(2, "self.base_url = Some(value.into());");
    emitter.line(2, "self");
    emitter.line(1, "}");
    emitter.blank();

    emitter.line(
        1,
        "/// Replaces the process-default body limits (main spec §33).",
    );
    emitter.line(1, "pub fn limits(mut self, limits: BodyLimits) -> Self {");
    emitter.line(2, "self.limits = limits;");
    emitter.line(2, "self");
    emitter.line(1, "}");
    emitter.blank();

    emitter.docs(
        1,
        &[
            "Opts into redirect following (§30.1); generated decoding never \
             buffers bodies to enable replay."
                .to_owned(),
        ],
    );
    emitter.line(
        1,
        "pub fn follow_redirects(mut self, policy: ::reqwest::redirect::Policy) -> Self {",
    );
    emitter.line(2, "self.http = self.http.redirect(policy);");
    emitter.line(2, "self");
    emitter.line(1, "}");
    if bases.has_secondaries() {
        emitter.blank();
        let mut docs = vec![
            "Overrides ONE secondary base URL by its documented key (companion \
              §8: every distinct effective default server generates its own \
             base)."
                .to_owned(),
            "An explicit `base_url` never affects these bases — it replaces \
             only the primary (recorded decision); a relative secondary URL \
             therefore REQUIRES an absolute value here \
             (D-impl-relative-servers)."
                .to_owned(),
            "Keys are deterministic snake_case derivations of each server URL; \
             declared keys for this client:"
                .to_owned(),
        ];
        for entry in bases.secondaries() {
            docs.push(format!("- `{}`: `{}`", entry.key(), entry.url));
        }
        emitter.docs(1, &docs);
        emitter.line(
            1,
            "pub fn secondary_base_url(mut self, key: &str, value: impl Into<String>) -> Self {",
        );
        emitter.line(2, "self.secondary_base_urls");
        emitter.line(3, ".insert(key.to_owned(), value.into());");
        emitter.line(2, "self");
        emitter.line(1, "}");
    }

    emit_variable_builders(emitter, bases);
    emit_build(emitter, bases);
    emitter.line(0, "}");
}

fn emit_variable_builders(emitter: &mut Emitter, bases: &BaseSet) {
    if !bases.has_any_variables() {
        return;
    }
    emitter.blank();
    let mut used_methods: BTreeSet<String> = BTreeSet::new();
    for (wire_name, variable) in bases.variable_registry() {
        let method = bases.variable_method_name(&wire_name, &mut used_methods);

        let mut doc_line = format!(
            "Server variable `{{{wire_name}}}` (declared default `{}`",
            variable.default
        );
        match &variable.allowed_enum {
            Some(allowed) if !allowed.is_empty() => {
                doc_line.push_str("; allowed values: ");
                doc_line.push_str(&allowed.join(", "));
                doc_line.push(')');
            }
            _ => doc_line.push(')'),
        }
        emitter.docs(1, &[doc_line]);
        emitter.docs(
            1,
            &[
                "One builder method per variable name controls EVERY base that \
              declares it (companion §8); enum validation against the \
             declared allowed values happens at `build` time."
                    .to_owned(),
            ],
        );
        let signature = format!("pub fn {method}(mut self, value: impl Into<String>) -> Self {{");
        emitter.line(1, &signature);
        // Two lines: rustfmt breaks chains beyond its default chain width.
        emitter.line(2, "self.server_variables");
        emitter.line(
            3,
            &format!(
                ".insert({}.to_owned(), value.into());",
                rust_string_literal(&wire_name)
            ),
        );
        emitter.line(2, "self");
        emitter.line(1, "}");
    }
}

fn emit_build(emitter: &mut Emitter, bases: &BaseSet) {
    emitter.blank();
    emitter.docs(
        1,
        &[
            "Builds the client (main spec §30.1, companion §8): every distinct \
              base resolves independently — builder overrides or declared \
             defaults, validated against their enums — and a non-absolute \
             base without its own override is `ClientError::InvalidUrl` \
             (D-impl-relative-servers)."
                .to_owned(),
        ],
    );
    emitter.line(1, "pub fn build(self) -> Result<Client, ClientError> {");
    emitter.line(2, "let base_url = match self.base_url {");
    emitter.line(3, "Some(explicit) => explicit,");
    emitter.line(3, "None => substitute_server_variables(");
    emitter.line(4, "&self.default_server_url,");
    emitter.line(4, "&self.default_server_variables,");
    emitter.line(4, "&self.server_variables,");
    emitter.line(3, ")?,");
    emitter.line(2, "};");
    emitter.line(2, "let trimmed = base_url.trim_end_matches('/');");
    emitter.line(2, "if !is_absolute_url(trimmed) {");
    emitter.line(3, "return Err(ClientError::InvalidUrl(format!(");
    emitter.line(
        4,
        "\"base URL `{trimmed}` is not absolute; call `base_url` because no \\
         absolute default server exists\"",
    );
    emitter.line(3, ")));");
    emitter.line(2, "}");
    for entry in bases.secondaries() {
        let key = entry.key();
        let lookup = format!(
            "let {key}_override = self.secondary_base_urls.get({}).cloned();",
            rust_string_literal(key)
        );
        if fits(2, &lookup) {
            emitter.line(2, &lookup);
        } else {
            emitter.line(2, &format!("let {key}_override = self"));
            emitter.line(3, ".secondary_base_urls");
            emitter.line(3, &format!(".get({})", rust_string_literal(key)));
            emitter.line(3, ".cloned();");
        }
        let resolve_head = format!("let url_{key} = match {key}_override {{");
        if fits(2, &resolve_head) {
            emitter.line(2, &resolve_head);
        } else {
            emitter.line(2, &format!("let url_{key} = match {key}_override"));
            emitter.line(2, "{");
        }
        emitter.line(3, "Some(explicit) => explicit,");
        if entry.variables.is_empty() {
            let call = format!(
                "None => substitute_server_variables({}, &[], &self.server_variables)?,",
                rust_string_literal(&entry.url)
            );
            if fits(3, &call) {
                emitter.line(3, &call);
            } else {
                emitter.line(3, "None => substitute_server_variables(");
                emitter.line(4, &format!("{},", rust_string_literal(&entry.url)));
                emitter.line(4, "&[],");
                emitter.line(4, "&self.server_variables,");
                emitter.line(3, ")?,");
            }
        } else {
            emitter.line(3, "None => substitute_server_variables(");
            emitter.line(4, &format!("{},", rust_string_literal(&entry.url)));
            emitter.line(4, &format!("&{key}_server_variables,"));
            emitter.line(4, "&self.server_variables,");
            emitter.line(3, ")?,");
        }
        emitter.line(2, "};");
        let trimmed_line = format!("let trimmed_{key} = url_{key}.trim_end_matches('/');");
        if fits(2, &trimmed_line) {
            emitter.line(2, &trimmed_line);
        } else {
            emitter.line(2, &format!("let trimmed_{key} ="));
            emitter.line(3, &format!("url_{key}.trim_end_matches('/');"));
        }
        emitter.line(2, &format!("if !is_absolute_url(trimmed_{key}) {{"));
        emitter.line(3, "return Err(ClientError::InvalidUrl(format!(");
        emitter.line(
            4,
            &format!(
                "\"secondary base `{key}` URL `{{trimmed_{key}}}` is not absolute; \\
                 call `secondary_base_url` with an absolute value\""
            ),
        );
        emitter.line(3, ")));");
        emitter.line(2, "}");
    }
    emitter.line(
        2,
        "let http = self.http.build().map_err(ClientError::Transport)?;",
    );
    emitter.line(2, "Ok(Client {");
    emitter.line(3, "http,");
    emitter.line(3, "base_url: trimmed.to_owned(),");
    emitter.line(3, "limits: self.limits,");
    for entry in bases.secondaries() {
        emitter.line(
            3,
            &format!("{}: trimmed_{}.to_owned(),", entry.field, entry.key()),
        );
    }
    emitter.line(2, "})");
    emitter.line(1, "}");
}

/// Every DISTINCT effective default-server URL across planned operations,
/// first appearance in document order (companion §8): entry 0 is the PRIMARY
/// base — the first operation's first effective server, kept
/// backwards-compatible through `Client::base_url`/`ClientBuilder::base_url`
/// — and every later entry becomes an additional stored base referenced as
/// `base_url_<key>` with a deterministic snake_case key derived from its URL
/// (recorded decision: `base_url` overrides only the primary; secondaries
/// are overridden per key via `secondary_base_url`).
#[derive(Debug)]
struct BaseSet {
    entries: Vec<BaseEntry>,
    /// Operation document index → entry index.
    assignment: Vec<usize>,
}

/// One distinct base: the raw URL template plus the union of declared
/// server variables across every operation assigned here (declaration order,
/// deduplicated by wire name).
#[derive(Debug)]
struct BaseEntry {
    url: String,
    /// Deterministic snake_case key; `None` only on the primary entry.
    key: Option<String>,
    /// Full field name on `Client`: `base_url` or `base_url_<key>`.
    field: String,
    variables: Vec<(String, crate::ir::document::ServerVariable)>,
}

impl BaseSet {
    fn new(plan: &PlannedApi) -> Self {
        let mut entries: Vec<BaseEntry> = Vec::new();
        let mut assignment = Vec::with_capacity(plan.operations.len());
        for operation in &plan.operations {
            // The normalizer guarantees at least one effective server (`/`
            // fallback, companion §8 rule 3).
            let url = operation
                .servers
                .first()
                .map_or_else(|| "/".to_owned(), |server| server.url.clone());
            let index = match entries.iter().position(|entry| entry.url == url) {
                Some(index) => index,
                None => {
                    entries.push(BaseEntry {
                        url,
                        key: None,
                        field: String::new(),
                        variables: Vec::new(),
                    });
                    entries.len() - 1
                }
            };
            assignment.push(index);
            let entry = &mut entries[index];
            if let Some(server) = operation.servers.first() {
                for (name, variable) in &server.variables {
                    if !entry.variables.iter().any(|(seen, _)| seen == name) {
                        entry.variables.push((name.clone(), variable.clone()));
                    }
                }
            }
        }
        if entries.is_empty() {
            entries.push(BaseEntry {
                url: "/".to_owned(),
                key: None,
                field: "base_url".to_owned(),
                variables: Vec::new(),
            });
        }
        // The primary keeps the plain field; secondaries get deterministic
        // keys with numeric suffixes on collisions (first appearance wins,
        // companion §10).
        entries[0].field = "base_url".to_owned();
        let mut used_keys: BTreeSet<String> = BTreeSet::new();
        for entry in entries.iter_mut().skip(1) {
            let key = fresh_name(&mut used_keys, base_key(&entry.url));
            entry.key = Some(key.clone());
            entry.field = format!("base_url_{key}");
        }
        Self {
            entries,
            assignment,
        }
    }

    fn primary(&self) -> &BaseEntry {
        &self.entries[0]
    }

    /// All entries after the primary, document order.
    fn secondaries(&self) -> &[BaseEntry] {
        &self.entries[1..]
    }

    fn has_secondaries(&self) -> bool {
        self.entries.len() > 1
    }

    fn has_any_variables(&self) -> bool {
        self.entries.iter().any(|entry| !entry.variables.is_empty())
    }

    /// Base-field name serving one operation's default server.
    fn field_for(&self, op_index: usize) -> &str {
        let entry_index = self.assignment.get(op_index).copied().unwrap_or(0);
        self.entries[entry_index].field.as_str()
    }

    /// First-appearance registry of declared variables across all bases,
    /// deduplicated by wire name: one builder method controls a variable
    /// shared by several bases (companion §8).
    fn variable_registry(&self) -> Vec<(String, crate::ir::document::ServerVariable)> {
        let mut registry: Vec<(String, crate::ir::document::ServerVariable)> = Vec::new();
        for entry in &self.entries {
            for (name, variable) in &entry.variables {
                if !registry.iter().any(|(seen, _)| seen == name) {
                    registry.push((name.clone(), variable.clone()));
                }
            }
        }
        registry
    }

    fn variable_method_name(&self, wire_name: &str, used: &mut BTreeSet<String>) -> String {
        const RESERVED: &[&str] = &[
            "new",
            "default",
            "base_url",
            "secondary_base_url",
            "limits",
            "follow_redirects",
            "build",
        ];
        used.extend(RESERVED.iter().map(|name| (*name).to_owned()));
        let base = naming::ident(wire_name, naming::NameStyle::Snake);
        let mut candidate = base.clone();
        let mut counter = 1_u32;
        while !used.insert(candidate.clone()) {
            counter += 1;
            candidate = naming::sanitize_joined(&format!("{base}_{counter}"));
        }
        candidate
    }
}

impl BaseEntry {
    /// Builder binding/local prefix (`storage_server_variables`,
    /// `url_storage`, …); only meaningful on secondary entries.
    fn key(&self) -> &str {
        self.key.as_deref().unwrap_or("primary")
    }
}

/// Deterministic snake_case key for one server URL: non-alphanumerics act as
/// word separators (`/storage` → `storage`,
/// `https://api.example.com/v1` → `https_api_example_com_v1`).
fn base_key(url: &str) -> String {
    let cleaned: String = url
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect();
    naming::ident(cleaned.trim(), naming::NameStyle::Snake)
}

fn emit_variables_binding(
    emitter: &mut Emitter,
    indent: usize,
    binding_name: &str,
    variables: &[(String, crate::ir::document::ServerVariable)],
) {
    // Layout matches rustfmt's canonical rendering of this construction.
    emitter.line(
        indent,
        &format!("let {binding_name} = vec![server_variable("),
    );
    for (name, variable) in variables {
        emitter.line(indent + 1, &format!("{},", rust_string_literal(name)));
        emitter.line(
            indent + 1,
            &format!("{},", rust_string_literal(&variable.default)),
        );
        match &variable.allowed_enum {
            Some(allowed) if !allowed.is_empty() => {
                let items: Vec<String> = allowed
                    .iter()
                    .map(|value| rust_string_literal(value))
                    .collect();
                let joined = items.join(", ");
                emitter.line(indent + 1, &format!("&[{joined}],"));
            }
            _ => {
                emitter.line(indent + 1, "&[],");
            }
        }
    }
    emitter.line(indent, ")];");
}

// ----------------------------------------------------------------------
// Per-operation type definitions
// ----------------------------------------------------------------------

fn emit_operation_definitions(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &Layout,
) {
    emitter.blank();
    let mut first = true;
    if operation.request_body_enum_name.is_some() {
        emit_request_body_enum(emitter, op_index, operation, layout);
        first = false;
    }
    if let Some(name) = layout.request_struct(op_index) {
        if !first {
            emitter.blank();
        }
        let spec = operation
            .request_contents
            .iter()
            .find(|content| content.media_class == MediaClass::Multipart)
            .and_then(|content| content.multipart_spec.as_ref());
        emit_multipart_request_struct(emitter, operation, name, spec);
        first = false;
    }
    for (status_index, status) in operation.statuses.iter().enumerate() {
        if let Some(name) = layout.content_enums.get(&(op_index, status_index)) {
            if !first {
                emitter.blank();
            }
            emit_content_enum(emitter, operation, status, name);
            first = false;
        }
        if let Some(name) = layout.wrappers.get(&(op_index, status_index)) {
            if !first {
                emitter.blank();
            }
            emit_wrapper(emitter, operation, status, name);
            first = false;
        }
    }
    if !first {
        emitter.blank();
    }
    emit_response_enum(emitter, op_index, operation, layout);
}

fn emit_request_body_enum(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &Layout,
) {
    let Some(enum_name) = &operation.request_body_enum_name else {
        return;
    };
    emitter.docs(
        0,
        &[format!(
            "Request payloads for `{}` (main spec §12/§43): owning variants \
                 (D-§51.3); streaming variants attach `reqwest::Body` verbatim.",
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub enum {enum_name} {{"));
    for content in &operation.request_contents {
        let payload = match content.media_class {
            MediaClass::Multipart => layout.request_struct(op_index).unwrap_or("()").to_owned(),
            _ => request_payload_type(content),
        };
        emitter.line(1, &format!("{}({}),", content.variant_name, payload));
    }
    emitter.line(0, "}");
}

/// Client-side payload type of one request media entry (§6 summary table).
fn request_payload_type(content: &PlannedContent) -> String {
    match content.media_class {
        MediaClass::JsonFamily | MediaClass::UrlEncodedForm => content.model_expr.clone(),
        MediaClass::PlainText => "String".to_owned(),
        MediaClass::Binary | MediaClass::RawUnknown => "::reqwest::Body".to_owned(),
        // Planning rejects the rest before emission.
        _ => "()".to_owned(),
    }
}

/// A planned multipart field plus its collision-resolved struct identifiers
/// (companion §10 numeric suffixing inside one input struct).
struct MultipartFieldIdents<'a> {
    field: &'a PlannedMultipartField,
    ident: String,
    file_ident: String,
    mime_ident: String,
}

/// Field identifier with numeric suffixing on collisions (companion §10).
fn unique_field_name(base: &str, used: &mut BTreeMap<String, u32>) -> String {
    let counter = used.entry(base.to_owned()).or_insert(0);
    *counter += 1;
    if *counter == 1 {
        base.to_owned()
    } else {
        naming::sanitize_joined(&format!("{base}_{counter}"))
    }
}

fn resolve_multipart_idents(fields: &[PlannedMultipartField]) -> Vec<MultipartFieldIdents<'_>> {
    let mut used_fields: BTreeMap<String, u32> = BTreeMap::new();
    fields
        .iter()
        .map(|field| {
            let ident = unique_field_name(&field.rust_name, &mut used_fields);
            let (file_ident, mime_ident) = match &field.kind {
                PlannedMultipartFieldKind::BinaryPart => (
                    unique_field_name(&format!("{ident}_name"), &mut used_fields),
                    unique_field_name(&format!("{ident}_content_type"), &mut used_fields),
                ),
                _ => (String::new(), String::new()),
            };
            MultipartFieldIdents {
                field,
                ident,
                file_ident,
                mime_ident,
            }
        })
        .collect()
}

/// The `<Op>Request` multipart input struct (main spec §17 Output A):
/// scalar/JSON parts are owned values; every binary part stays a streaming
/// `::reqwest::Body` (never `Vec<u8>`), carrying optional upload filename and
/// content type beside it.
fn emit_multipart_request_struct(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    name: &str,
    spec: Option<&PlannedMultipart>,
) {
    emitter.docs(
        0,
        &[format!(
            "Multipart input for `{}` (main spec §17 Output A): scalar/JSON \
                 parts are owned values; binary parts stay streaming \
                 (`::reqwest::Body`, never buffered by generated code).",
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub struct {name} {{"));
    let resolved = resolve_multipart_idents(spec.map(|s| s.fields.as_slice()).unwrap_or(&[]));
    for entry in &resolved {
        match &entry.field.kind {
            PlannedMultipartFieldKind::ScalarText(rust_type) => {
                emit_part_field_doc(emitter, 1, entry.field, "owned textual");
                let field_type = wrap_option_unless_required(rust_type, entry.field);
                emitter.line(1, &format!("pub {}: {field_type},", entry.ident));
            }
            PlannedMultipartFieldKind::JsonPart(model) => {
                emit_part_field_doc(emitter, 1, entry.field, "JSON");
                let field_type = wrap_option_unless_required(model, entry.field);
                emitter.line(1, &format!("pub {}: {field_type},", entry.ident));
            }
            PlannedMultipartFieldKind::BinaryPart => {
                emit_part_field_doc(emitter, 1, entry.field, "streaming binary");
                emitter.line(1, &format!("pub {}: ::reqwest::Body,", entry.ident));
                emitter.docs(
                    1,
                    &[format!(
                        "Upload filename reported for part `{}`, when set.",
                        entry.field.wire_name
                    )],
                );
                emitter.line(1, &format!("pub {}: Option<String>,", entry.file_ident));
                emitter.docs(
                    1,
                    &[format!(
                        "Content type for part `{}`, when set.",
                        entry.field.wire_name
                    )],
                );
                emitter.line(
                    1,
                    &format!("pub {}: Option<::mime::Mime>,", entry.mime_ident),
                );
            }
        }
    }
    emitter.line(0, "}");

    // One §17 from_file constructor per binary part: the named part streams
    // straight off the opened file through tokio-util's ReaderStream; other
    // binary parts start empty.
    for target in resolved
        .iter()
        .filter(|entry| matches!(entry.field.kind, PlannedMultipartFieldKind::BinaryPart))
    {
        let ctor = format!("from_{}", target.ident);
        emit_from_file_ctor(emitter, name, &ctor, &resolved, target);
    }
}

fn emit_part_field_doc(
    emitter: &mut Emitter,
    indent: usize,
    field: &PlannedMultipartField,
    kind_label: &str,
) {
    let cardinality = if field.repeated {
        "; repeated parts collect in wire order"
    } else {
        ""
    };
    emitter.docs(
        indent,
        &[format!(
            "{kind_label} part `{}`{}.",
            field.wire_name, cardinality
        )],
    );
}

/// Required single-valued scalar/JSON parts are plain; optional ones ride
/// `Option<T>`; repeated parts are `Vec<T>` collecting in wire order.
fn wrap_option_unless_required(rust_type: &str, field: &PlannedMultipartField) -> String {
    if field.repeated {
        format!("Vec<{rust_type}>")
    } else if field.required {
        rust_type.to_owned()
    } else {
        format!("Option<{rust_type}>")
    }
}

/// Emits one §17 `from_file` constructor for one binary part: parameters are
/// every scalar/JSON field in declaration order, then the path; other binary
/// parts initialize to an empty streaming body.
fn emit_from_file_ctor(
    emitter: &mut Emitter,
    struct_name: &str,
    ctor: &str,
    resolved: &[MultipartFieldIdents],
    target: &MultipartFieldIdents,
) {
    emitter.blank();
    emitter.line(0, &format!("impl {struct_name} {{"));
    emitter.docs(
        1,
        &[format!(
            "Opens `path` as the streaming payload of part `{}` (main spec \
              §17): bytes flow through tokio-util's ReaderStream without \
             whole-file buffering; other binary parts start empty.",
            target.field.wire_name
        )],
    );
    emitter.docs(
        1,
        &["Errors propagate `std::io::Error` from opening the file.".to_owned()],
    );
    emitter.line(1, "#[allow(clippy::missing_errors_doc)]");
    emitter.line(1, &format!("pub async fn {ctor}("));
    for entry in resolved {
        if let PlannedMultipartFieldKind::ScalarText(rust_type)
        | PlannedMultipartFieldKind::JsonPart(rust_type) = &entry.field.kind
        {
            let param_type = wrap_option_unless_required(rust_type, entry.field);
            emitter.line(2, &format!("{}: {param_type},", entry.ident));
        }
    }
    emitter.line(2, "path: impl AsRef<::std::path::Path>,");
    emitter.line(1, ") -> Result<Self, ::std::io::Error> {");
    emitter.line(
        2,
        "let file = ::tokio::fs::File::open(path.as_ref()).await?;",
    );
    emitter.line(2, "let stream = ::tokio_util::io::ReaderStream::new(file);");
    emitter.line(2, "Ok(Self {");
    for entry in resolved {
        match &entry.field.kind {
            PlannedMultipartFieldKind::ScalarText(_) | PlannedMultipartFieldKind::JsonPart(_) => {
                emitter.line(3, &format!("{},", entry.ident));
            }
            PlannedMultipartFieldKind::BinaryPart => {
                if entry.field.wire_name == target.field.wire_name {
                    emitter.line(
                        3,
                        &format!("{}: ::reqwest::Body::wrap_stream(stream),", entry.ident),
                    );
                    emitter.line(3, &format!("{}: path", entry.file_ident));
                    emitter.line(4, ".as_ref()");
                    emitter.line(4, ".file_name()");
                    emitter.line(4, ".map(|value| value.to_string_lossy().into_owned()),");
                } else {
                    emitter.line(
                        3,
                        &format!("{}: ::bytes::Bytes::new().into(),", entry.ident),
                    );
                    emitter.line(3, &format!("{}: None,", entry.file_ident));
                }
                emitter.line(3, &format!("{}: None,", entry.mime_ident));
            }
        }
    }
    emitter.line(2, "})");
    emitter.line(1, "}");
    emitter.line(0, "}");
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
            "Documented representations for status {} of `{}` (main spec §11): \
                 the client negotiates via Content-Type (§28).",
            crate::normalize::status_label(&status.key),
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub enum {name} {{"));
    for content in &status.contents {
        emitter.line(
            1,
            &format!(
                "{}({}),",
                content.variant_name,
                response_payload_type(content)
            ),
        );
    }
    emitter.line(0, "}");
}

/// Client-side payload type of one response media entry (§7 summary table):
/// bounded models/`String` for structured classes, owned raw
/// `reqwest::Response` for streaming classes (§32).
fn response_payload_type(content: &PlannedContent) -> String {
    match content.media_class {
        MediaClass::JsonFamily => content.model_expr.clone(),
        MediaClass::PlainText => "String".to_owned(),
        MediaClass::Binary | MediaClass::RawUnknown => "::reqwest::Response".to_owned(),
        _ => "()".to_owned(),
    }
}

fn emit_wrapper(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    name: &str,
) {
    let [content] = status.contents.as_slice() else {
        unreachable!("wrapper statuses always have exactly one content entry");
    };
    let streaming = matches!(
        content.media_class,
        MediaClass::Binary | MediaClass::RawUnknown
    );
    let mut docs = Vec::new();
    if streaming {
        let suffix = if status.headers.is_empty() {
            String::new()
        } else {
            " plus its typed documented headers (§15, superseding \
             D-impl-typed-headers-phase2)"
                .to_owned()
        };
        docs.push(format!(
            "Streaming payload for status {} of `{}` (main spec §32): owns \
             the response{}.",
            crate::normalize::status_label(&status.key),
            operation.method,
            suffix
        ));
    } else {
        // §15 Output A wrapper: typed documented headers beside the body.
        docs.push(format!(
            "Typed payload for status {} of `{}` (main spec §15 Output A): \
             required headers as plain fields, optional headers as \
             `Option<T>`, then the decoded body.",
            crate::normalize::status_label(&status.key),
            operation.method
        ));
    }
    emitter.docs(0, &docs);
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub struct {name} {{"));
    emit_header_fields(emitter, status, 1);
    if streaming {
        emitter.line(1, "pub response: ::reqwest::Response,");
    } else {
        emitter.line(1, &format!("pub body: {},", response_payload_type(content)));
    }
    emitter.line(0, "}");
    if !streaming {
        // Structured §15 wrappers expose the decoded payload directly; no
        // raw-response conveniences apply.
        return;
    }
    emitter.blank();
    emitter.line(0, &format!("impl {name} {{"));
    emitter.docs(
        1,
        &["Consumes the wrapper into the raw chunk stream (main spec §32).".to_owned()],
    );
    // NOTE: deliberately no `#[must_use]` — the returned opaque
    // `impl Stream` is already `must_use`, and a bare attribute here trips
    // clippy::double_must_use under `-D warnings`.
    emitter.line(1, "pub fn into_bytes_stream(");
    emitter.line(2, "self,");
    emitter.line(
        1,
        ") -> impl ::futures_core::Stream<Item = ::reqwest::Result<::bytes::Bytes>> {",
    );
    emitter.line(2, "self.response.bytes_stream()");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Typed documented-header fields of one status (main spec §15): required
/// headers become plain fields, optional ones `Option<T>`.
fn emit_header_fields(emitter: &mut Emitter, status: &PlannedStatus, indent: usize) {
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
        emitter.line(indent, &format!("pub {}: {field_type},", header.rust_name));
    }
}

fn emit_response_enum(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    layout: &Layout,
) {
    emitter.docs(
        0,
        &[format!(
            "Documented outcomes for `{}` (main spec §8/§13): exhaustive match \
                 required; deliberately not `#[non_exhaustive]` (§47).",
            operation.method
        )],
    );
    emitter.line(0, "#[derive(Debug)]");
    emitter.line(0, &format!("pub enum {} {{", operation.response_enum_name));
    for (status_index, status) in operation.statuses.iter().enumerate() {
        emit_variant_doc(emitter, status);
        if layout.wrappers.contains_key(&(op_index, status_index)) {
            let wrapper = layout
                .wrappers
                .get(&(op_index, status_index))
                .expect("wrapper registered");
            emitter.line(1, &format!("{}({wrapper}),", status.enum_variant));
            continue;
        }
        if struct_variant_status(status) {
            // Ranges/default carry the wire status (main spec §23–§24);
            // documented headers ride inside the struct variant (§15).
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emitter.line(2, "status: ::http::StatusCode,");
            emit_header_fields(emitter, status, 2);
            match status.contents.len() {
                0 => {}
                _ => {
                    let body_type = match status.contents.len() {
                        1 => response_payload_type(&status.contents[0]),
                        _ => layout
                            .content_enums
                            .get(&(op_index, status_index))
                            .expect("content enum registered")
                            .clone(),
                    };
                    emitter.line(2, &format!("body: {body_type},"));
                }
            }
            emitter.line(1, "},");
            continue;
        }
        if !status.headers.is_empty() && status.contents.is_empty() {
            // Header-only documented response (e.g. 302 + Location): the
            // variant carries exactly the typed headers.
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emit_header_fields(emitter, status, 2);
            emitter.line(1, "},");
            continue;
        }
        if !status.headers.is_empty() && status.contents.len() >= 2 {
            // Multi-content with documented headers: the typed fields hoist
            // onto the STATUS VARIANT beside the content enum (recorded
            // decision; see module docs).
            let content_enum = layout
                .content_enums
                .get(&(op_index, status_index))
                .expect("content enum registered");
            emitter.line(1, &format!("{} {{", status.enum_variant));
            emit_header_fields(emitter, status, 2);
            emitter.line(2, &format!("content: {content_enum},"));
            emitter.line(1, "},");
            continue;
        }
        match status.contents.len() {
            0 => {
                emitter.line(1, &format!("{},", status.enum_variant));
            }
            1 => {
                let payload = response_payload_type(&status.contents[0]);
                emitter.line(1, &format!("{}({payload}),", status.enum_variant));
            }
            _ => {
                let content_enum = layout
                    .content_enums
                    .get(&(op_index, status_index))
                    .expect("content enum registered");
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
// Operation methods
// ----------------------------------------------------------------------

fn emit_methods(
    emitter: &mut Emitter,
    plan: &PlannedApi,
    bases: &BaseSet,
    layout: &Layout,
    flags: &mut Flags,
) {
    emitter.blank();
    emitter.line(0, "impl Client {");
    for (op_index, operation) in plan.operations.iter().enumerate() {
        if op_index > 0 {
            emitter.blank();
        }
        // Each operation sends through ITS effective default base
        // (companion §8); ops sharing the primary URL keep referencing it.
        let base_field = bases.field_for(op_index).to_owned();
        emit_operation_method(emitter, op_index, operation, &base_field, layout, flags);
    }
    emitter.line(0, "}");
}

/// One argument entry of the generated method signature.
struct SignatureArgument {
    name: String,
    rust_type: String,
}

fn signature_arguments(
    operation: &PlannedOperation,
    layout: &Layout,
    op_index: usize,
) -> Vec<SignatureArgument> {
    let mut arguments: Vec<SignatureArgument> = operation
        .parameters
        .iter()
        .map(|parameter| {
            let base = match parameter.rust_type.as_str() {
                "String" => "&str".to_owned(),
                other if other.starts_with("Vec<") => {
                    format!(
                        "&[{}]",
                        other.trim_start_matches("Vec<").trim_end_matches('>')
                    )
                }
                other => other.to_owned(),
            };
            let rust_type = if parameter.required {
                base
            } else {
                format!("Option<{base}>")
            };
            SignatureArgument {
                name: parameter.rust_name.clone(),
                rust_type,
            }
        })
        .collect();
    if operation.request_body_enum_name.is_some() || !operation.request_contents.is_empty() {
        let body_type = match (
            &operation.request_body_enum_name,
            operation.request_contents.first(),
        ) {
            (Some(enum_name), _) => {
                if operation.request_body_required {
                    enum_name.clone()
                } else {
                    format!("Option<{enum_name}>")
                }
            }
            (None, Some(content)) => {
                let base = request_parameter_type(content, layout, op_index);
                if operation.request_body_required {
                    base
                } else {
                    format!("Option<{base}>")
                }
            }
            (None, None) => unreachable!("body flag set without content"),
        };
        arguments.push(SignatureArgument {
            name: "body".to_owned(),
            rust_type: body_type,
        });
    }
    arguments
}

/// Direct request-parameter type for single-content operations (§6 table):
/// `&T` for JSON and forms (D-§51.3 convenience), the owned `<Op>Request`
/// input struct for multipart (§17 Output A), `&str` for text, owned
/// `reqwest::Body` for streaming payloads.
fn request_parameter_type(content: &PlannedContent, layout: &Layout, op_index: usize) -> String {
    match content.media_class {
        MediaClass::JsonFamily | MediaClass::UrlEncodedForm => {
            format!("&{}", content.model_expr)
        }
        MediaClass::Multipart => layout.request_struct(op_index).unwrap_or("()").to_owned(),
        MediaClass::PlainText => "&str".to_owned(),
        MediaClass::Binary | MediaClass::RawUnknown => "::reqwest::Body".to_owned(),
        _ => "()".to_owned(),
    }
}

fn emit_operation_method(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    base_field: &str,
    layout: &Layout,
    flags: &mut Flags,
) {
    let mut doc_lines = vec![format!(
        "`{}` `{}`.",
        operation.http.as_keyword().to_ascii_uppercase(),
        operation.path_template
    )];
    if let Some(operation_id) = &operation.operation_id {
        doc_lines.push(format!("Operation `{operation_id}`."));
    }
    emitter.docs(1, &doc_lines);
    if operation.deprecated {
        emitter.line(1, "#[deprecated]");
    }

    let arguments = signature_arguments(operation, layout, op_index);
    let args_inline: Vec<String> = std::iter::once("&self".to_owned())
        .chain(
            arguments
                .iter()
                .map(|argument| format!("{}: {}", argument.name, argument.rust_type)),
        )
        .collect();
    let return_type = format!("Result<{}, ClientError>", operation.response_enum_name);
    let inline = format!(
        "pub async fn {}({}) -> {return_type} {{",
        operation.method,
        args_inline.join(", ")
    );
    if fits(1, &inline) {
        emitter.line(1, &inline);
    } else {
        emitter.line(1, &format!("pub async fn {}(", operation.method));
        emitter.line(2, "&self,");
        for argument in &arguments {
            emitter.line(2, &format!("{}: {},", argument.name, argument.rust_type));
        }
        emitter.line(1, &format!(") -> {return_type} {{"));
    }

    emit_url_building(emitter, operation, base_field, flags);
    emit_request_construction(emitter, operation, flags);

    emitter.line(2, "match response.status() {");
    let has_default = operation
        .statuses
        .last()
        .is_some_and(|status| status.key == ResponseStatusKey::Default);
    for (status_index, _) in operation.statuses.iter().enumerate() {
        emit_status_arm(emitter, op_index, operation, status_index, layout, flags);
    }
    if !has_default {
        emitter.line(
            3,
            "other => Err(ClientError::UndocumentedStatus { status: other }),",
        );
    }
    emitter.line(2, "}");
    emitter.line(1, "}");
}

// ----------------------------------------------------------------------
// URL building (companion §8: unreserved-set encoding, declaration order)
// ----------------------------------------------------------------------

/// One parsed piece of a path template.
enum TemplatePart {
    /// Literal slash-separated text; percent-encoded at GENERATION time.
    Literal(String),
    /// `{name}` slot substituted through the §6 style encoders at runtime.
    Parameter(String),
}

fn parse_template(template: &str) -> Vec<TemplatePart> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            parts.push(TemplatePart::Literal(rest[..open].to_owned()));
        }
        let after = &rest[open + 1..];
        let close = after
            .find('}')
            .unwrap_or_else(|| panic!("unterminated path parameter in `{template}`"));
        parts.push(TemplatePart::Parameter(after[..close].to_owned()));
        rest = &after[close + 1..];
    }
    if !rest.is_empty() {
        parts.push(TemplatePart::Literal(rest.to_owned()));
    }
    parts
}

fn emit_url_building(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    base_field: &str,
    flags: &mut Flags,
) {
    // Companion §8: the operation's own effective default base (primary
    // `base_url` or its `base_url_<key>` field).
    emitter.line(2, &format!("let mut url = self.{base_field}.clone();"));
    for part in parse_template(&operation.path_template) {
        match part {
            TemplatePart::Literal(text) => {
                let encoded = encode_literal_path(&text);
                if !encoded.is_empty() {
                    let line = format!("url.push_str({});", rust_string_literal(&encoded));
                    emitter.line(2, &line);
                }
            }
            TemplatePart::Parameter(name) => {
                let parameter = operation
                    .parameters
                    .iter()
                    .find(|candidate| {
                        candidate.location == ParameterLocation::Path && candidate.wire_name == name
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "path template `{}` references undeclared parameter `{name}`",
                            operation.path_template
                        )
                    });
                flags.needs_encode_path = true;
                emit_param_spec(emitter, 2, parameter);
                emitter.line(
                    2,
                    &format!(
                        "let value = {};",
                        param_value_expr(parameter, parameter.rust_name.clone())
                    ),
                );
                emitter.line(2, "url.push_str(&encode_path(&spec, &value));");
            }
        }
    }

    // Query pairs append in declaration order with `?` then `&` separators;
    // empty pair lists skip the query string entirely (companion §8).
    let query_parameters: Vec<&PlannedParameter> = operation
        .parameters
        .iter()
        .filter(|parameter| parameter.location == ParameterLocation::Query)
        .collect();
    if query_parameters.is_empty() {
        return;
    }
    emitter.line(
        2,
        "let mut query_pairs: Vec<(String, String)> = Vec::new();",
    );
    for parameter in query_parameters {
        flags.needs_query_pairs = true;
        emit_param_spec(emitter, 2, parameter);
        if parameter.required {
            emitter.line(
                2,
                &format!(
                    "let value = {};",
                    param_value_expr(parameter, parameter.rust_name.clone())
                ),
            );
            emitter.line(2, "query_pairs.extend(");
            emit_query_extend_body(emitter, 3, &parameter.wire_name);
            emitter.line(2, ");");
        } else {
            emitter.line(2, &format!("if let Some(raw) = {} {{", parameter.rust_name));
            emitter.line(
                3,
                &format!(
                    "let value = {};",
                    param_value_expr(parameter, "raw".to_owned())
                ),
            );
            emitter.line(3, "query_pairs.extend(");
            emit_query_extend_body(emitter, 4, &parameter.wire_name);
            emitter.line(3, ");");
            emitter.line(2, "}");
        }
    }
    emitter.line(2, "if !query_pairs.is_empty() {");
    emitter.line(3, "url.push('?');");
    emitter.line(
        3,
        "for (index, (name, value)) in query_pairs.iter().enumerate() {",
    );
    emitter.line(4, "if index > 0 {");
    emitter.line(5, "url.push('&');");
    emitter.line(4, "}");
    emitter.line(4, "url.push_str(name);");
    emitter.line(4, "url.push('=');");
    emitter.line(4, "url.push_str(value);");
    emitter.line(3, "}");
    emitter.line(2, "}");
}

fn emit_query_extend_body(emitter: &mut Emitter, indent: usize, wire_name: &str) {
    emitter.line(indent, "encode_query_pairs(&spec, &value)");
    emitter.line(indent, ".map_err(|error| ClientError::InvalidUrl(format!(");
    emitter.line(
        indent + 1,
        &format!(
            "\"parameter `{}` serialization failed: {{error}}\"",
            wire_name
        ),
    );
    emitter.line(indent, "))),");
    emitter.line(indent, "?,");
}

fn emit_param_spec(emitter: &mut Emitter, indent: usize, parameter: &PlannedParameter) {
    let style = param_style_name(parameter.style);
    let line = format!(
        "let spec = ParamSpec::new({}, ParamStyle::{style}, {}, {});",
        rust_string_literal(&parameter.wire_name),
        parameter.explode,
        parameter.allow_reserved
    );
    emitter.line(indent, &line);
}

/// Encodes literal template text segment-by-segment at generation time so the
/// emitted string is final (deterministic, no runtime cost).
fn encode_literal_path(text: &str) -> String {
    text.split('/')
        .map(|piece| {
            if piece.is_empty() {
                String::new()
            } else {
                encode_unreserved(piece)
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// RFC 3986 unreserved set percent-encoding (companion §8), applied to
/// literal path-template pieces at generation time.
fn encode_unreserved(value: &str) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            _ => {
                out.push('%');
                out.push(HEX_DIGITS[usize::from(byte >> 4)] as char);
                out.push(HEX_DIGITS[usize::from(byte & 0x0F)] as char);
            }
        }
    }
    out
}

// ----------------------------------------------------------------------
// Request construction: headers, cookies, body (§26, §29, §34.2)
// ----------------------------------------------------------------------

/// Emits from body preparation through `let response = …send().await?`.
fn emit_request_construction(
    emitter: &mut Emitter,
    operation: &PlannedOperation,
    flags: &mut Flags,
) {
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

    // Single-content JSON/text bodies serialize BEFORE the send so an encode
    // overflow returns `BodyTooLarge` without any wire traffic (§34.2).
    match operation.request_contents.as_slice() {
        [content] if content.media_class == MediaClass::JsonFamily => {
            flags.needs_serialize_json = true;
            flags.needs_body_limit_direction = true;
            if operation.request_body_required {
                flags.needs_encode_overflow = true;
                emit_bounded_encode(emitter, 2, "body", "serialize_json_limited");
            }
        }
        [content] if content.media_class == MediaClass::UrlEncodedForm => {
            flags.needs_serialize_form = true;
            flags.needs_body_limit_direction = true;
            if operation.request_body_required {
                flags.needs_encode_overflow = true;
                emit_bounded_encode(emitter, 2, "body", "serialize_form_limited");
            }
        }
        [content] if content.media_class == MediaClass::PlainText => {
            flags.needs_body_limit_direction = true;
            if operation.request_body_required {
                flags.needs_encode_overflow = true;
                emit_text_len_check(emitter, 2, "body");
            }
        }
        _ => {}
    }

    let multi_body = operation.request_body_enum_name.is_some();
    let optional_single =
        !multi_body && !operation.request_body_required && !operation.request_contents.is_empty();
    let is_multipart = operation
        .request_contents
        .first()
        .is_some_and(|content| content.media_class == MediaClass::Multipart);
    let needs_rebinding = multi_body
        || optional_single
        || is_multipart
        || !header_parameters.is_empty()
        || !cookie_parameters.is_empty();

    // §30.1: redirects are off by default so documented 3xx statuses reach
    // the exhaustive enum; opt-in following never buffers bodies for replay.
    if needs_rebinding {
        let request_head = format!(
            "let mut request = self.http.request(::http::Method::{}, &url);",
            http_method_const(operation.http)
        );
        if fits(2, &request_head) {
            emitter.line(2, &request_head);
        } else {
            emitter.line(2, "let mut request =");
            emitter.line(
                3,
                &format!(
                    "self.http.request(::http::Method::{}, &url);",
                    http_method_const(operation.http)
                ),
            );
        }
        emit_header_params(emitter, &header_parameters, flags);
        emit_cookie_params(emitter, &cookie_parameters);
        emit_body_assignment(emitter, operation, flags);
        if !operation.accept_header_value.is_empty() {
            emitter.line(
                2,
                &format!(
                    "request = request.header(::http::header::ACCEPT, {});",
                    rust_string_literal(&operation.accept_header_value)
                ),
            );
        }
        emitter.line(2, "let response = request.send().await?;");
    } else {
        emitter.line(
            2,
            "// \u{a7}30.1: redirects are off by default so documented 3xx statuses reach \
             the exhaustive enum; opt-in following never buffers bodies for replay.",
        );
        emitter.line(2, "let response = self");
        emitter.line(3, ".http");
        emitter.line(
            3,
            &format!(
                ".request(::http::Method::{}, &url)",
                http_method_const(operation.http)
            ),
        );
        if !operation.request_contents.is_empty() {
            let content = &operation.request_contents[0];
            emit_chain_header(
                emitter,
                3,
                "::http::header::CONTENT_TYPE",
                &content.media_type_literal,
            );
        }
        if !operation.accept_header_value.is_empty() {
            emit_chain_header(
                emitter,
                3,
                "::http::header::ACCEPT",
                &operation.accept_header_value,
            );
        }
        if let [content] = operation.request_contents.as_slice() {
            let payload_expr = match content.media_class {
                MediaClass::JsonFamily | MediaClass::UrlEncodedForm => "payload",
                MediaClass::PlainText => "body.to_owned()",
                _ => "body",
            };
            emitter.line(3, &format!(".body({payload_expr})"));
        }
        emitter.line(3, ".send()");
        emitter.line(3, ".await?;");
    }
}

fn emit_header_params(emitter: &mut Emitter, parameters: &[&PlannedParameter], flags: &mut Flags) {
    for parameter in parameters {
        flags.needs_header_value = true;
        emit_param_spec(emitter, 2, parameter);
        let header_call = |emitter: &mut Emitter, indent: usize| {
            emitter.line(
                indent,
                &format!(
                    "request = request.header({}, encode_header_value(&spec, &value));",
                    rust_string_literal(&parameter.wire_name)
                ),
            );
        };
        if parameter.required {
            emitter.line(
                2,
                &format!(
                    "let value = {};",
                    param_value_expr(parameter, parameter.rust_name.clone())
                ),
            );
            header_call(emitter, 2);
        } else {
            emitter.line(2, &format!("if let Some(raw) = {} {{", parameter.rust_name));
            emitter.line(
                3,
                &format!(
                    "let value = {};",
                    param_value_expr(parameter, "raw".to_owned())
                ),
            );
            header_call(emitter, 3);
            emitter.line(2, "}");
        }
    }
}

fn emit_cookie_params(emitter: &mut Emitter, parameters: &[&PlannedParameter]) {
    if parameters.is_empty() {
        return;
    }
    emitter.line(2, "let mut cookie_segments: Vec<String> = Vec::new();");
    for parameter in parameters {
        emit_param_spec(emitter, 2, parameter);
        if parameter.required {
            emitter.line(
                2,
                &format!(
                    "let value = {};",
                    param_value_expr(parameter, parameter.rust_name.clone())
                ),
            );
            emitter.line(
                2,
                "cookie_segments.push(encode_cookie_value(&spec, &value));",
            );
        } else {
            emitter.line(2, &format!("if let Some(raw) = {} {{", parameter.rust_name));
            emitter.line(
                3,
                &format!(
                    "let value = {};",
                    param_value_expr(parameter, "raw".to_owned())
                ),
            );
            emitter.line(
                3,
                "cookie_segments.push(encode_cookie_value(&spec, &value));",
            );
            emitter.line(2, "}");
        }
    }
    emitter.line(2, "if !cookie_segments.is_empty() {");
    emitter.line(
        3,
        "request = request.header(::http::header::COOKIE, cookie_segments.join(\"; \"));",
    );
    emitter.line(2, "}");
}

/// Body assignment for rebound requests: single payloads (required or
/// optional), multi-content enums (§12/§26/§43), and multipart form
/// builders (§17).
fn emit_body_assignment(emitter: &mut Emitter, operation: &PlannedOperation, flags: &mut Flags) {
    if let Some(enum_name) = &operation.request_body_enum_name {
        match (
            operation.request_body_required,
            operation.request_contents.as_slice(),
        ) {
            (_, []) => {}
            (true, contents) => {
                emitter.line(2, "request = match body {");
                for content in contents {
                    emit_request_enum_arm(emitter, enum_name, content, flags, 3);
                }
                emitter.line(2, "};");
            }
            (false, contents) => {
                emitter.line(2, "if let Some(body) = body {");
                emitter.line(3, "request = match body {");
                for content in contents {
                    emit_request_enum_arm(emitter, enum_name, content, flags, 4);
                }
                emitter.line(3, "};");
                emitter.line(2, "}");
            }
        }
        return;
    }

    let Some(content) = operation.request_contents.first() else {
        return;
    };
    if content.media_class == MediaClass::Multipart {
        let spec = content.multipart_spec.as_ref();
        if operation.request_body_required {
            emit_multipart_form_build(emitter, 2, "body", spec, flags);
            emitter.line(2, "request = request.multipart(form);");
        } else {
            emitter.line(2, "if let Some(body) = body {");
            emit_multipart_form_build(emitter, 3, "body", spec, flags);
            emitter.line(3, "request = request.multipart(form);");
            emitter.line(2, "}");
        }
        return;
    }
    if operation.request_body_required {
        emit_single_request_body(emitter, 2, content, true, flags);
    } else {
        emitter.line(2, "if let Some(body) = body {");
        emit_single_request_body(emitter, 3, content, false, flags);
        emitter.line(2, "}");
    }
}

/// Single-payload assignment on the rebound request builder. Required JSON
/// and form bodies were already bounded-encoded before `request` existed
/// (`payload` binding); optional bodies serialize HERE so an encode overflow
/// still returns before any wire traffic (§34.2).
fn emit_single_request_body(
    emitter: &mut Emitter,
    indent: usize,
    content: &PlannedContent,
    required: bool,
    flags: &mut Flags,
) {
    match content.media_class {
        MediaClass::JsonFamily if !required => {
            flags.needs_serialize_json = true;
            flags.needs_body_limit_direction = true;
            flags.needs_encode_overflow = true;
            emit_bounded_encode(emitter, indent, "body", "serialize_json_limited");
        }
        MediaClass::UrlEncodedForm if !required => {
            flags.needs_serialize_form = true;
            flags.needs_body_limit_direction = true;
            flags.needs_encode_overflow = true;
            emit_bounded_encode(emitter, indent, "body", "serialize_form_limited");
        }
        _ => {}
    }
    emitter.line(indent, "request = request");
    match content.media_class {
        MediaClass::JsonFamily | MediaClass::UrlEncodedForm => {
            emitter.line(
                indent + 1,
                &format!(
                    ".header(::http::header::CONTENT_TYPE, {})",
                    rust_string_literal(&content.media_type_literal)
                ),
            );
            emitter.line(indent + 1, ".body(payload);");
        }
        MediaClass::PlainText => {
            emitter.line(
                indent + 1,
                ".header(::http::header::CONTENT_TYPE, \"text/plain\")",
            );
            emitter.line(indent + 1, ".body(body.to_owned());");
        }
        _ => {
            emitter.line(
                indent + 1,
                &format!(
                    ".header(::http::header::CONTENT_TYPE, {})",
                    rust_string_literal(&content.media_type_literal)
                ),
            );
            emitter.line(indent + 1, ".body(body);");
        }
    }
}

/// One match arm of a `<Op>RequestBody` dispatch, rebinding the
/// `request` builder per variant.
fn emit_request_enum_arm(
    emitter: &mut Emitter,
    enum_name: &str,
    content: &PlannedContent,
    flags: &mut Flags,
    indent: usize,
) {
    let variant = format!("{enum_name}::{}", content.variant_name);
    match content.media_class {
        MediaClass::JsonFamily => {
            flags.needs_serialize_json = true;
            flags.needs_body_limit_direction = true;
            flags.needs_encode_overflow = true;
            emitter.line(indent, &format!("{variant}(value) => {{"));
            emit_bounded_encode(emitter, indent + 1, "&value", "serialize_json_limited");
            emitter.line(
                indent + 1,
                &format!(
                    "request.header(::http::header::CONTENT_TYPE, {})",
                    rust_string_literal(&content.media_type_literal)
                ),
            );
            emitter.line(indent + 1, ".body(payload)");
            emitter.line(indent, "}");
        }
        MediaClass::UrlEncodedForm => {
            // §16/§34: bounded form serialization; `.form()` never used.
            flags.needs_serialize_form = true;
            flags.needs_body_limit_direction = true;
            flags.needs_encode_overflow = true;
            emitter.line(indent, &format!("{variant}(value) => {{"));
            emit_bounded_encode(emitter, indent + 1, "&value", "serialize_form_limited");
            emitter.line(
                indent + 1,
                &format!(
                    "request.header(::http::header::CONTENT_TYPE, {})",
                    rust_string_literal(&content.media_type_literal)
                ),
            );
            emitter.line(indent + 1, ".body(payload)");
            emitter.line(indent, "}");
        }
        MediaClass::PlainText => {
            flags.needs_body_limit_direction = true;
            flags.needs_encode_overflow = true;
            emitter.line(indent, &format!("{variant}(text) => {{"));
            emit_text_len_check(emitter, indent + 1, "text");
            emitter.line(
                indent + 1,
                &format!(
                    "request.header(::http::header::CONTENT_TYPE, {})",
                    rust_string_literal(&content.media_type_literal)
                ),
            );
            emitter.line(indent + 1, ".body(text)");
            emitter.line(indent, "}");
        }
        MediaClass::Multipart => {
            // §17: the form builder carries the multipart Content-Type with
            // its boundary itself; a static header would break framing.
            flags.needs_multipart = true;
            emitter.line(indent, &format!("{variant}(value) => {{"));
            let spec = content.multipart_spec.as_ref();
            emit_multipart_form_build(emitter, indent + 1, "value", spec, flags);
            emitter.line(indent + 1, "request.multipart(form)");
            emitter.line(indent, "}");
        }
        _ => {
            emitter.line(indent, &format!("{variant}(body) => {{"));
            emitter.line(indent + 1, "request");
            emitter.line(
                indent + 2,
                &format!(
                    ".header(::http::header::CONTENT_TYPE, {})",
                    rust_string_literal(&content.media_type_literal)
                ),
            );
            emitter.line(indent + 2, ".body(body)");
            emitter.line(indent, "}");
        }
    }
}

// ----------------------------------------------------------------------
// ----------------------------------------------------------------------
// Multipart form building (main spec §17 Output A)
// ----------------------------------------------------------------------

/// Builds a `::reqwest::multipart::Form` from the `<Op>Request` value:
/// scalar parts via `Part::text`, JSON parts serialized through the bounded
/// `serialize_json_limited` FIRST (§34.2: overflow returns before any wire
/// traffic) and attached with their declared (or `application/json`) media
/// type, binary parts as unbuffered `Part::stream` payloads with optional
/// filename/mime. Repeated scalar/JSON parts append one part per value in
/// declaration order. The caller attaches with `request.multipart(form)`,
/// which also writes the boundary-bearing Content-Type header.
fn emit_multipart_form_build(
    emitter: &mut Emitter,
    indent: usize,
    value_expr: &str,
    spec: Option<&PlannedMultipart>,
    flags: &mut Flags,
) {
    flags.needs_multipart = true;
    emitter.line(indent, "let mut form = Form::new();");
    let Some(spec) = spec else {
        return;
    };
    for entry in &resolve_multipart_idents(&spec.fields) {
        let field = entry.field;
        let wire = rust_string_literal(&field.wire_name);
        match &field.kind {
            PlannedMultipartFieldKind::ScalarText(rust_type) => {
                if field.repeated {
                    emitter.line(
                        indent,
                        &format!("for value in &{}.{} {{", value_expr, entry.ident),
                    );
                    emitter.line(
                        indent + 1,
                        &format!(
                            "form = form.part({wire}, Part::text({}));",
                            scalar_text_expr(rust_type, "value")
                        ),
                    );
                    emitter.line(indent, "}");
                } else if field.required {
                    let expr =
                        scalar_text_expr(rust_type, &format!("{}.{}", value_expr, entry.ident));
                    emitter.line(
                        indent,
                        &format!("form = form.part({wire}, Part::text({expr}));"),
                    );
                } else {
                    emitter.line(
                        indent,
                        &format!("if let Some(value) = &{}.{} {{", value_expr, entry.ident),
                    );
                    emitter.line(
                        indent + 1,
                        &format!(
                            "form = form.part({wire}, Part::text({}));",
                            scalar_text_expr(rust_type, "value")
                        ),
                    );
                    emitter.line(indent, "}");
                }
            }
            PlannedMultipartFieldKind::JsonPart(_) => {
                flags.needs_serialize_json = true;
                let mime_literal = field
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "application/json".to_owned());
                if field.repeated {
                    emitter.line(
                        indent,
                        &format!("for element in &{}.{} {{", value_expr, entry.ident),
                    );
                    emit_bounded_encode(emitter, indent + 1, "element", "serialize_json_limited");
                    emit_form_part_json(emitter, indent + 1, &wire, &mime_literal);
                    emitter.line(indent, "}");
                } else {
                    let expr = format!("&{}.{}", value_expr, entry.ident);
                    emit_bounded_encode(emitter, indent, &expr, "serialize_json_limited");
                    emit_form_part_json(emitter, indent, &wire, &mime_literal);
                }
            }
            PlannedMultipartFieldKind::BinaryPart => {
                emitter.line(indent, &format!("form = form.part({wire}, {{"));
                emitter.line(
                    indent + 1,
                    &format!(
                        "let mut part = Part::stream({}.{});",
                        value_expr, entry.ident
                    ),
                );
                emitter.line(
                    indent + 1,
                    &format!(
                        "if let Some(value) = {}.{} {{",
                        value_expr, entry.file_ident
                    ),
                );
                emitter.line(indent + 2, "part = part.file_name(value.clone());");
                emitter.line(indent + 1, "}");
                emitter.line(
                    indent + 1,
                    &format!(
                        "if let Some(value) = {}.{} {{",
                        value_expr, entry.mime_ident
                    ),
                );
                emitter.line(indent + 2, "part = part_with_mime(part, value.as_ref())?;");
                emitter.line(indent + 1, "}");
                emitter.line(indent + 1, "part");
                emitter.line(indent, "});");
            }
        }
    }
}

/// Text conversion for one scalar part value (`String` clones; typed
/// scalars render through `Display`).
fn scalar_text_expr(rust_type: &str, expr: &str) -> String {
    if rust_type == "String" {
        format!("{expr}.clone()")
    } else {
        format!("{expr}.to_string()")
    }
}

/// Appends one already-serialized JSON payload as a mime-typed part.
fn emit_form_part_json(emitter: &mut Emitter, indent: usize, wire: &str, mime_literal: &str) {
    let mime = rust_string_literal(mime_literal);
    let inner = format!("part_with_mime(Part::bytes(Vec::from(&payload[..])), {mime})?");
    let outer_args = format!("{wire}, {inner}");
    let inline = format!("form = form.part({outer_args});");
    if fits(indent, &inline) && outer_args.chars().count() <= FN_CALL_WIDTH {
        emitter.line(indent, &inline);
        return;
    }
    emitter.line(indent, "form = form.part(");
    emitter.line(indent + 1, &format!("{wire},"));
    if fits(indent + 1, &format!("{inner},")) {
        emitter.line(indent + 1, &format!("{inner},"));
    } else {
        emitter.line(indent + 2, "part_with_mime(");
        emitter.line(indent + 3, "Part::bytes(Vec::from(&payload[..])),");
        emitter.line(indent + 3, &format!("{mime},"));
        emitter.line(indent + 2, ")?,");
    }
    emitter.line(indent, ");");
}

// ----------------------------------------------------------------------
// Status decode arms (§23–§28, §35)
// ----------------------------------------------------------------------

/// Payload expression carried by a variant result.
#[derive(Clone)]
enum BodyExpr {
    /// No payload (unit variants and body-less struct variants).
    None,
    /// A decoded value binding (`value`/`content`) or wrapper construction.
    Value(String),
    /// The streaming wrapper for this status.
    Wrapper,
}

fn emit_status_arm(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    status_index: usize,
    layout: &Layout,
    flags: &mut Flags,
) {
    let status = &operation.statuses[status_index];
    let pattern = arm_pattern(status);
    let has_headers = !status.headers.is_empty();

    // §35: no-body statuses and HEAD never read or validate the body, even
    // when the document lists content entries for them. (Planning refuses
    // headers on no-body statuses, so nothing is dropped here.)
    if status.is_no_body_status || operation.http == HttpMethod::Head {
        emit_simple_arm(
            emitter,
            3,
            &pattern,
            op_index,
            operation,
            status,
            status_index,
            layout,
            BodyExpr::None,
        );
        return;
    }

    // Example 1 pattern: structured budget on 2xx-class statuses, error
    // budget otherwise.
    let limit_expr = if status.is_success_class {
        "self.limits.structured_response_bytes"
    } else {
        "self.limits.error_response_bytes"
    };

    if let [content] = status.contents.as_slice() {
        if matches!(
            content.media_class,
            MediaClass::Binary | MediaClass::RawUnknown
        ) {
            if has_headers {
                // Typed headers are read BEFORE handing the raw response to
                // the wrapper (main spec §15).
                emitter.line(3, &format!("{pattern} {{"));
                emit_header_binds(emitter, 4, status);
                emit_typed_wrapper_result(
                    emitter,
                    4,
                    operation,
                    status,
                    status_index,
                    layout,
                    op_index,
                    "response,",
                );
                emitter.line(3, "}");
                return;
            }
            emit_simple_arm(
                emitter,
                3,
                &pattern,
                op_index,
                operation,
                status,
                status_index,
                layout,
                BodyExpr::Wrapper,
            );
            return;
        }
    }

    if status.contents.is_empty() {
        if has_headers {
            // Header-only documented response (§15): parse the headers and
            // construct the struct variant; no body is read.
            emitter.line(3, &format!("{pattern} {{"));
            emit_header_binds(emitter, 4, status);
            let mut fields: Vec<String> = Vec::new();
            if struct_variant_status(status) {
                fields.push("status".to_owned());
            }
            for header in &status.headers {
                fields.push(header.rust_name.clone());
            }
            let head = format!(
                "Ok({}::{} {{",
                operation.response_enum_name, status.enum_variant
            );
            // rustfmt keeps at most two-field struct literals on one line
            // (struct_lit_width heuristic), vertical beyond that.
            let joined = fields.join(", ");
            if fields.len() <= 2 {
                let inline = format!("{head} {joined} }})");
                if fits(4, &inline) {
                    emitter.line(4, &inline);
                    emitter.line(3, "}");
                    return;
                }
            }
            emitter.line(4, &head);
            for field in &fields {
                emitter.line(5, &format!("{field},"));
            }
            emitter.line(4, "})");
            emitter.line(3, "}");
            return;
        }
        emit_simple_arm(
            emitter,
            3,
            &pattern,
            op_index,
            operation,
            status,
            status_index,
            layout,
            BodyExpr::None,
        );
        return;
    }

    emitter.line(3, &format!("{pattern} {{"));
    // §15 Output A: documented headers parse BEFORE the body is consumed.
    if has_headers {
        emit_header_binds(emitter, 4, status);
    }
    match status.contents.len() {
        1 => {
            emit_content_type_gate(emitter, &status.contents, flags);
            emit_collect_and_decode(emitter, 4, &status.contents[0], flags, limit_expr);
            emit_result_line(
                emitter,
                4,
                op_index,
                operation,
                status,
                status_index,
                layout,
                BodyExpr::Value("value".to_owned()),
            );
        }
        _ => {
            emit_negotiated_arm_body(
                emitter,
                op_index,
                operation,
                status,
                status_index,
                layout,
                flags,
                limit_expr,
            );
        }
    }
    emitter.line(3, "}");
}

/// Emits one `let <rust_name> = parse_{required,optional}_header::<T>(
/// &response, "<wire>")?;` binding per documented header (main spec §15).
/// Header names are plan-validated field names, so `from_static` is safe.
fn emit_header_binds(emitter: &mut Emitter, indent: usize, status: &PlannedStatus) {
    for header in &status.headers {
        let helper = if header.required {
            "parse_required_header"
        } else {
            "parse_optional_header"
        };
        let wire = rust_string_literal(&header.wire_name.to_ascii_lowercase());
        let line = format!(
            "let {} = {helper}::<{}>(&response, {wire})?;",
            header.rust_name, header.rust_type
        );
        let head = format!("let {} =", header.rust_name);
        let deep_call = format!("{helper}::<{}>(&response, {wire})?;", header.rust_type);
        if fits(indent, &line) {
            emitter.line(indent, &line);
        } else if fits(indent + 1, &deep_call) {
            // rustfmt breaks after `=` first, keeping the argument list
            // horizontal on the continuation line.
            emitter.line(indent, &head);
            emitter.line(indent + 1, &deep_call);
        } else {
            emitter.line(
                indent,
                &format!(
                    "let {} = {helper}::<{}>(&response,",
                    header.rust_name, header.rust_type
                ),
            );
            emitter.line(indent + 1, &format!("{wire})?;"));
        }
    }
}

/// Emits the `Ok(<Enum>::<Variant>(<Wrapper> {{ ... }}))` construction for a
/// status whose typed wrapper carries documented headers beside its payload;
/// `inner_line` is the trailing field (`response,` for streaming payloads or
/// `body: value,` for decoded ones).
#[allow(clippy::too_many_arguments)]
fn emit_typed_wrapper_result(
    emitter: &mut Emitter,
    indent: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    status_index: usize,
    layout: &Layout,
    op_index: usize,
    inner_line: &str,
) {
    let wrapper = layout
        .wrappers
        .get(&(op_index, status_index))
        .unwrap_or_else(|| panic!("wrapper missing for {}", status.enum_variant));
    emitter.line(
        indent,
        &format!(
            "Ok({}::{}({wrapper} {{",
            operation.response_enum_name, status.enum_variant
        ),
    );
    emit_wrapper_fields(emitter, indent + 1, status);
    emitter.line(indent + 1, inner_line);
    emitter.line(indent, "}))");
}

/// The typed-header fields of a wrapper literal (already-bound locals).
fn emit_wrapper_fields(emitter: &mut Emitter, indent: usize, status: &PlannedStatus) {
    for header in &status.headers {
        emitter.line(indent, &format!("{},", header.rust_name));
    }
}

/// Expression-form match arm (`PATTERN => RESULT,`) for bodies that are a
/// single expression; rustfmt collapses such arms out of block form.
#[allow(clippy::too_many_arguments)]
fn emit_simple_arm(
    emitter: &mut Emitter,
    indent: usize,
    pattern: &str,
    op_index: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    status_index: usize,
    layout: &Layout,
    body: BodyExpr,
) {
    let mut probe = Emitter::new();
    emit_result_line(
        &mut probe,
        0,
        op_index,
        operation,
        status,
        status_index,
        layout,
        body.clone(),
    );
    let rendered = probe.finish();
    let single = rendered.trim_end().to_owned();
    if !single.contains('\n') {
        let inline = format!("{pattern} {single},");
        if fits(indent, &inline) {
            emitter.line(indent, &inline);
            return;
        }
    }
    emitter.line(indent, &format!("{pattern} {{"));
    emitter.line(indent + 1, &single);
    emitter.line(indent, "}");
}

/// Match-arm pattern for one planned status: constants for standard codes,
/// guards otherwise; ranges bind `status` through their half-open guard;
/// `Default` is the trailing catch-all.
fn arm_pattern(status: &PlannedStatus) -> String {
    match status.key {
        ResponseStatusKey::Explicit(code) => match status_code_const(code) {
            Some(const_name) => format!("::http::StatusCode::{const_name} =>"),
            None => format!("status if status.as_u16() == {code} =>"),
        },
        ResponseStatusKey::RangeClass(range) => {
            let (low, high) = range_bounds(range);
            format!("status if ({low}..{high}).contains(&status.as_u16()) =>")
        }
        ResponseStatusKey::Default => "status =>".to_owned(),
    }
}

fn range_bounds(range: RangeClass) -> (u16, u16) {
    match range {
        RangeClass::Success2xx => (200, 300),
        RangeClass::Redirection3xx => (300, 400),
        RangeClass::ClientError4xx => (400, 500),
        RangeClass::ServerError5xx => (500, 600),
    }
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

/// Range and default statuses render as struct variants carrying the wire
/// status (main spec §23–§24); explicit statuses keep plain shapes.
fn struct_variant_status(status: &PlannedStatus) -> bool {
    !matches!(status.key, ResponseStatusKey::Explicit(_))
}

/// Emits the result expression of one arm with rustfmt-canonical wrapping:
/// single line when it fits within 100 columns, vertical layout otherwise.
#[allow(clippy::too_many_arguments)]
fn emit_result_line(
    emitter: &mut Emitter,
    indent: usize,
    op_index: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    status_index: usize,
    layout: &Layout,
    body: BodyExpr,
) {
    let enum_name = &operation.response_enum_name;
    let variant = &status.enum_variant;
    let wrapper = layout.wrappers.get(&(op_index, status_index));
    let has_headers = !status.headers.is_empty();

    if struct_variant_status(status) {
        match (&body, wrapper) {
            (BodyExpr::None, _) => {
                let inline = format!("Ok({enum_name}::{variant} {{ status }})");
                if fits(indent, &inline) {
                    emitter.line(indent, &inline);
                } else {
                    emitter.line(indent, &format!("Ok({enum_name}::{variant} {{"));
                    emitter.line(indent + 1, "status,");
                    emitter.line(indent, "})");
                }
            }
            (BodyExpr::Value(value), _) => {
                emitter.line(indent, &format!("Ok({enum_name}::{variant} {{"));
                emitter.line(indent + 1, "status,");
                if has_headers {
                    emit_wrapper_fields(emitter, indent + 1, status);
                }
                emitter.line(indent + 1, &format!("body: {value},"));
                emitter.line(indent, "})");
            }
            (BodyExpr::Wrapper, Some(wrapper_name)) => {
                emitter.line(indent, &format!("Ok({enum_name}::{variant} {{"));
                emitter.line(indent + 1, "status,");
                if has_headers {
                    // Headers were parsed before the raw response moved into
                    // the wrapper (main spec §15).
                    emit_wrapper_fields(emitter, indent + 1, status);
                    emitter.line(indent + 1, &format!("body: {wrapper_name} {{ response }},"));
                } else {
                    emitter.line(indent + 1, &format!("body: {wrapper_name} {{ response }},"));
                }
                emitter.line(indent, "})");
            }
            (BodyExpr::Wrapper, None) => unreachable!("wrapper expression without registration"),
        }
        return;
    }

    // Explicit statuses with a §15 typed wrapper: headers hoist into the
    // wrapper beside the decoded payload.
    if has_headers && matches!(body, BodyExpr::Value(_)) {
        if wrapper.is_some() {
            emit_typed_wrapper_result(
                emitter,
                indent,
                operation,
                status,
                status_index,
                layout,
                op_index,
                "body: value,",
            );
            return;
        }
        unreachable!("typed single-content status without a registered wrapper");
    }

    let payload = match &body {
        BodyExpr::None => String::new(),
        BodyExpr::Value(value) => value.clone(),
        BodyExpr::Wrapper => format!(
            "{} {{ response }}",
            wrapper.unwrap_or_else(|| panic!("wrapper missing for {variant}"))
        ),
    };
    let inline = if payload.is_empty() {
        format!("Ok({enum_name}::{variant})")
    } else {
        format!("Ok({enum_name}::{variant}({payload}))")
    };
    if fits(indent, &inline) {
        emitter.line(indent, &inline);
        return;
    }
    if payload.is_empty() {
        emitter.line(indent, "Ok(");
        emitter.line(indent + 1, &format!("{enum_name}::{variant}"));
        emitter.line(indent, ")");
    } else {
        emitter.line(indent, "Ok(");
        emitter.line(indent + 1, &format!("{enum_name}::{variant}({payload}),"));
        emitter.line(indent, ")");
    }
}

fn fits(indent: usize, text: &str) -> bool {
    indent * 4 + text.chars().count() <= RUSTFMT_MAX_WIDTH
}

/// Content-Type gate shared by every decodable arm (§28 steps 1–2):
/// duplicate headers are ambiguous, missing yields the
/// `UnexpectedContentType{actual: None}` error, malformed surfaces as
/// `MalformedContentType`; produces a `content_type` binding.
fn emit_content_type_gate(emitter: &mut Emitter, contents: &[PlannedContent], flags: &mut Flags) {
    flags.needs_content_type_helpers = true;
    let expected: Vec<String> = contents
        .iter()
        .map(|content| {
            format!(
                "{}.to_owned()",
                rust_string_literal(&content.media_type_literal)
            )
        })
        .collect();
    let joined = expected.join(", ");
    let expected_line = format!("expected: vec![{joined}],");
    emitter.line(4, "let parsed = parse_response_content_type(&response)?;");
    emitter.line(4, "let Some(parsed) = parsed else {");
    emitter.line(5, "return Err(ClientError::UnexpectedContentType {");
    if fits(6, &expected_line) {
        emitter.line(6, &expected_line);
    } else {
        emitter.line(6, "expected: vec![");
        for item in &expected {
            emitter.line(7, &format!("{item},"));
        }
        emitter.line(6, "],");
    }
    emitter.line(6, "actual: None,");
    emitter.line(5, "});");
    emitter.line(4, "};");
    emitter.line(4, "let content_type = mime_of(&parsed)?;");
}

/// Bounded JSON/form request serialization; an encode overflow returns
/// `BodyTooLarge` without sending anything (§34.2). `serializer` selects
/// `serialize_json_limited` or `serialize_form_limited`.
fn emit_bounded_encode(emitter: &mut Emitter, indent: usize, value_expr: &str, serializer: &str) {
    let call = format!("{serializer}({value_expr}, self.limits.structured_encode_bytes)");
    let head = format!("let payload = match {call} {{");
    let mut arm_indent = indent + 1;
    let mut close_indent = indent;
    if fits(indent, &head) {
        emitter.line(indent, &head);
    } else if fits(indent + 1, &format!("match {call} {{")) {
        // rustfmt prefers breaking after `=` so the whole match head stays
        // horizontal on its own continuation line.
        emitter.line(indent, "let payload =");
        emitter.line(indent + 1, &format!("match {call} {{"));
        arm_indent = indent + 2;
        close_indent = indent + 1;
    } else {
        emitter.line(indent, &format!("let payload = match {serializer}("));
        emitter.line(indent + 1, &format!("{value_expr},"));
        emitter.line(indent + 1, "self.limits.structured_encode_bytes,");
        emitter.line(indent, ") {");
    }
    emitter.line(arm_indent, "Ok(payload) => payload,");
    let err_line =
        "Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),";
    if fits(arm_indent, err_line) {
        emitter.line(arm_indent, err_line);
    } else {
        emitter.line(arm_indent, "Err(_) => return Err(encode_overflow_error(");
        emitter.line(arm_indent + 1, "self.limits.structured_encode_bytes,");
        emitter.line(arm_indent, ")),");
    }
    emitter.line(close_indent, "};");
}

/// Plain-text request length check against the encode budget (§34.2).
fn emit_text_len_check(emitter: &mut Emitter, indent: usize, value_expr: &str) {
    emitter.line(
        indent,
        &format!("if {value_expr}.len() > self.limits.structured_encode_bytes {{"),
    );
    emitter.line(
        indent + 1,
        "return Err(encode_overflow_error(self.limits.structured_encode_bytes));",
    );
    emitter.line(indent, "}");
}

/// Bounded collection plus typed decoding for one structured/text entry
/// (Example 1 pattern); produces a `value` binding.
fn emit_collect_and_decode(
    emitter: &mut Emitter,
    indent: usize,
    content: &PlannedContent,
    flags: &mut Flags,
    limit_expr: &str,
) {
    let inner = indent + 1;
    let inner2 = indent + 2;
    flags.needs_collect = true;
    if matches!(
        content.media_class,
        MediaClass::JsonFamily | MediaClass::PlainText
    ) {
        flags.needs_charset_check = true;
        emitter.line(indent, "ensure_utf8_charset(&parsed)?;");
    }
    emitter.line(indent, &format!("let limit = {limit_expr};"));
    emitter.line(
        indent,
        "let bytes = collect_reqwest_limited(response, limit).await?;",
    );
    if content.media_class == MediaClass::JsonFamily {
        flags.needs_empty_json_body = true;
        flags.needs_json_decode = true;
        emitter.line(indent, "if bytes.is_empty() {");
        emitter.line(inner, "return Err(ClientError::Decode {");
        emitter.line(inner2, "content_type: Some(content_type),");
        emitter.line(inner2, "source: Box::new(EmptyJsonBody),");
        emitter.line(inner, "});");
        emitter.line(indent, "}");
        let model = &content.model_expr;
        let decode_line = format!("let value: {model} = json_decode(&bytes, Some(content_type))?;");
        if fits(indent, &decode_line) {
            emitter.line(indent, &decode_line);
        } else {
            emitter.line(indent, &format!("let value: {model} ="));
            emitter.line(inner, "json_decode(&bytes, Some(content_type))?;");
        }
    } else {
        flags.needs_text_decode = true;
        emitter.line(
            indent,
            "let value = text_decode(bytes, Some(content_type))?;",
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_negotiated_arm_body(
    emitter: &mut Emitter,
    op_index: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    status_index: usize,
    layout: &Layout,
    flags: &mut Flags,
    limit_expr: &str,
) {
    let content_enum = layout
        .content_enums
        .get(&(op_index, status_index))
        .expect("content enum registered for multi-content status");
    let expected: Vec<String> = status
        .contents
        .iter()
        .map(|content| {
            format!(
                "{}.to_owned()",
                rust_string_literal(&content.media_type_literal)
            )
        })
        .collect();
    let joined = expected.join(", ");

    emit_content_type_gate(emitter, &status.contents.clone(), flags);

    emitter.line(4, "let mut best_rank: Option<u8> = None;");
    emitter.line(4, "let mut best_index: usize = 0;");
    for (index, content) in status.contents.iter().enumerate() {
        let literal = rust_string_literal(&content.media_type_literal);
        let match_line = format!("if let Some(rank) = match_entry(&parsed, {literal}) {{");
        emitter.line(4, &match_line);
        emitter.line(5, "let rank = negotiation_rank(rank);");
        emitter.line(5, "if best_rank.is_none_or(|seen| rank < seen) {");
        emitter.line(6, "best_rank = Some(rank);");
        emitter.line(6, &format!("best_index = {index};"));
        emitter.line(5, "}");
        emitter.line(4, "}");
    }

    emitter.line(
        4,
        "let selected = best_rank.is_some().then_some(best_index);",
    );
    emitter.line(4, "match selected {");
    for (index, content) in status.contents.iter().enumerate() {
        emitter.line(5, &format!("Some({index}) => {{"));
        match content.media_class {
            MediaClass::JsonFamily | MediaClass::PlainText => {
                emit_collect_and_decode(emitter, 6, content, flags, limit_expr);
                emit_negotiated_result(
                    emitter,
                    6,
                    operation,
                    status,
                    content_enum,
                    &format!("{}(value)", content.variant_name),
                );
            }
            _ => {
                emit_negotiated_result(
                    emitter,
                    6,
                    operation,
                    status,
                    content_enum,
                    &format!("{}(response)", content.variant_name),
                );
            }
        }
        emitter.line(5, "}");
    }
    emitter.line(5, "_ => Err(ClientError::UnexpectedContentType {");
    if fits(6, &format!("expected: vec![{joined}],")) {
        emitter.line(6, &format!("expected: vec![{joined}],"));
    } else {
        emitter.line(6, "expected: vec![");
        for item in &expected {
            emitter.line(7, &format!("{item},"));
        }
        emitter.line(6, "],");
    }
    emitter.line(6, "actual: Some(mime_of(&parsed)?),");
    emitter.line(5, "}),");
    emitter.line(4, "}");
}

/// Result expression of one negotiated branch: tuple-variant construction
/// for explicit statuses, struct-variant fields (`status`, `body`) for
/// ranges/default. Documented headers hoist onto the status VARIANT beside
/// the content enum (recorded decision; see module docs). Wrapping follows
/// rustfmt-canonical layout.
fn emit_negotiated_result(
    emitter: &mut Emitter,
    indent: usize,
    operation: &PlannedOperation,
    status: &PlannedStatus,
    content_enum: &str,
    payload_expr: &str,
) {
    // Bind the nested-content value first so construction stays one call
    // deep (predictable rustfmt layout).
    emitter.line(
        indent,
        &format!("let payload = {content_enum}::{payload_expr};"),
    );
    let enum_name = &operation.response_enum_name;
    let variant = &status.enum_variant;
    if struct_variant_status(status) {
        // rustfmt keeps struct literals vertical beyond `struct_lit_width`.
        emitter.line(indent, &format!("Ok({enum_name}::{variant} {{"));
        emitter.line(indent + 1, "status,");
        emit_wrapper_fields_opt(emitter, indent + 1, status);
        emitter.line(indent + 1, "body: payload,");
        emitter.line(indent, "})");
        return;
    }
    if !status.headers.is_empty() {
        emitter.line(indent, &format!("Ok({enum_name}::{variant} {{"));
        emit_wrapper_fields_opt(emitter, indent + 1, status);
        emitter.line(indent + 1, "content: payload,");
        emitter.line(indent, "})");
        return;
    }
    emitter.line(indent, &format!("Ok({enum_name}::{variant}(payload))"));
}

/// Header-field locals inside a result literal; empty when none documented.
fn emit_wrapper_fields_opt(emitter: &mut Emitter, indent: usize, status: &PlannedStatus) {
    if !status.headers.is_empty() {
        emit_wrapper_fields(emitter, indent, status);
    }
}

// ----------------------------------------------------------------------
// Module helpers emitted into the generated file
// ----------------------------------------------------------------------

fn emit_module_helpers(emitter: &mut Emitter, flags: &Flags, has_variables: bool) {
    if flags.needs_multipart {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "Attaches a declared media type to one multipart part (main \
                  spec §17). The literal was planned from the document's \
                 `encoding.contentType`; a value the MIME parser refuses is \
                 a malformed content type, never silently defaulted."
                    .to_owned(),
            ],
        );
        emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
        let sig =
            "fn part_with_mime(part: Part, mime_literal: &str) -> Result<Part, ClientError> {";
        if fits(0, sig) {
            emitter.line(0, sig);
        } else {
            emitter.line(0, "fn part_with_mime(");
            emitter.line(1, "part: Part,");
            emitter.line(1, "mime_literal: &str,");
            emitter.line(0, ") -> Result<Part, ClientError> {");
        }
        emitter.line(1, "part.mime_str(mime_literal).map_err(|_| {");
        emitter.line(
            2,
            "ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)",
        );
        emitter.line(1, "})");
        emitter.line(0, "}");
    }
    if flags.needs_content_type_helpers {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "Reads and parses the response `Content-Type` (§28 steps 1–2): \
                 duplicate headers are ambiguous decode errors (§28.1), a \
                 missing header yields `None`, malformed values surface as \
                 `MalformedContentType`."
                    .to_owned(),
            ],
        );
        emitter.line(0, "fn parse_response_content_type(");
        emitter.line(1, "response: &::reqwest::Response,");
        emitter.line(0, ") -> Result<Option<ParsedMediaType>, ClientError> {");
        emitter.line(1, "let values: Vec<&::http::HeaderValue> = response");
        emitter.line(2, ".headers()");
        emitter.line(2, ".get_all(::http::header::CONTENT_TYPE)");
        emitter.line(2, ".iter()");
        emitter.line(2, ".collect();");
        emitter.line(1, "if values.len() > 1 {");
        emitter.line(2, "return Err(ClientError::Decode {");
        emitter.line(3, "content_type: None,");
        emitter.line(3, "source: Box::new(DuplicateContentType),");
        emitter.line(2, "});");
        emitter.line(1, "}");
        emitter.line(1, "let Some(raw) = values.first() else {");
        emitter.line(2, "return Ok(None);");
        emitter.line(1, "};");
        emitter.line(1, "let text = raw.to_str().map_err(|_| {");
        emitter.line(
            2,
            "ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)",
        );
        emitter.line(1, "})?;");
        emitter.line(
            1,
            "Ok(Some(::openapi_support::mediatype::parse_content_type(",
        );
        emitter.line(2, "text,");
        emitter.line(1, ")?))");
        emitter.line(0, "}");

        emitter.blank();
        emit_error_type(
            emitter,
            "DuplicateContentType",
            None,
            "\"duplicate Content-Type headers on one message\"",
            "duplicate Content-Type headers are an ambiguous message (§28.1); \
             generated code never picks one arbitrarily",
        );

        emitter.blank();
        emitter.docs(
            0,
            &["Builds the `mime::Mime` carried by [`ClientError`] fields.".to_owned()],
        );
        emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
        emitter.line(
            0,
            "fn mime_of(parsed: &ParsedMediaType) -> Result<::mime::Mime, ClientError> {",
        );
        emitter.line(1, "let subtype = match &parsed.suffix {");
        emitter.line(
            2,
            "Some(suffix) => format!(\"{}+{}\", parsed.subtype, suffix),",
        );
        emitter.line(2, "None => parsed.subtype.clone(),");
        emitter.line(1, "};");
        emitter.line(1, "let text = format!(\"{}/{}\", parsed.ty, subtype);");
        emitter.line(1, "text.parse().map_err(|_| {");
        emitter.line(
            2,
            "ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)",
        );
        emitter.line(1, "})");
        emitter.line(0, "}");
    }

    if flags.needs_charset_check {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "§28.4 charset policy (D-impl-charset-rejection): textual media \
                 decode as UTF-8; any other declared charset is a decode error \
                 instead of replacement-character corruption."
                    .to_owned(),
            ],
        );
        emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
        emitter.line(
            0,
            "fn ensure_utf8_charset(parsed: &ParsedMediaType) -> Result<(), ClientError> {",
        );
        emitter.line(
            1,
            "if let Some((_, value)) = parsed.parameters.iter().find(|(name, _)| name == \"charset\") {",
        );
        emitter.line(2, "let lowered = value.to_ascii_lowercase();");
        emitter.line(2, "if lowered != \"utf-8\" && lowered != \"utf8\" {");
        emitter.line(3, "return Err(ClientError::Decode {");
        emitter.line(4, "content_type: None,");
        emitter.line(4, "source: Box::new(UnsupportedCharset(value.clone())),");
        emitter.line(3, "});");
        emitter.line(2, "}");
        emitter.line(1, "}");
        emitter.line(1, "Ok(())");
        emitter.line(0, "}");

        emitter.blank();
        emit_error_type(
            emitter,
            "UnsupportedCharset",
            Some("String"),
            "\"charset `{}` is outside the UTF-8 family\"",
            "declared charset is outside the UTF-8 family (§28.4); generated \
             clients surface this as `ClientError::Decode` \
             (D-impl-charset-rejection)",
        );
    }

    if flags.needs_empty_json_body {
        emitter.blank();
        emit_error_type(
            emitter,
            "EmptyJsonBody",
            None,
            "\"documented JSON status arrived with an empty body\"",
            "a documented JSON status arrived with an empty body; empty input is \
             never decoded as a default value (§28.3)",
        );
    }

    if flags.needs_negotiation_rank {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "§28 dispatch ranking: Exact beats suffix family beats range \
                 match beats wildcard."
                    .to_owned(),
            ],
        );
        emitter.line(0, "#[must_use]");
        emitter.line(
            0,
            "fn negotiation_rank(matched: ::openapi_support::mediatype::EntryMatch) -> u8 {",
        );
        emitter.line(1, "match matched {");
        emitter.line(2, "::openapi_support::mediatype::EntryMatch::Exact => 0,");
        emitter.line(
            2,
            "::openapi_support::mediatype::EntryMatch::SuffixFamily => 1,",
        );
        emitter.line(
            2,
            "::openapi_support::mediatype::EntryMatch::RangeMatch => 2,",
        );
        emitter.line(
            2,
            "::openapi_support::mediatype::EntryMatch::Wildcard => 3,",
        );
        emitter.line(1, "}");
        emitter.line(0, "}");
    }

    if flags.needs_json_decode {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "Maps bounded JSON decode failures onto [`ClientError::Decode`] \
                 (§36); `content_type` is carried for diagnostics."
                    .to_owned(),
            ],
        );
        emitter.line(
            0,
            "fn json_decode<T>(bytes: &[u8], content_type: Option<::mime::Mime>) -> Result<T, ClientError>",
        );
        emitter.line(0, "where");
        emitter.line(1, "T: serde::de::DeserializeOwned,");
        emitter.line(0, "{");
        emitter.line(
            1,
            "serde_json::from_slice(bytes).map_err(|error| ClientError::Decode {",
        );
        emitter.line(2, "content_type,");
        emitter.line(2, "source: Box::new(error),");
        emitter.line(1, "})");
        emitter.line(0, "}");
    }

    if flags.needs_text_decode {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "UTF-8 validation for bounded plain-text bodies (§28.4): invalid \
               bytes are decode errors, never replacement characters."
                    .to_owned(),
            ],
        );
        emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
        emitter.line(0, "fn text_decode(");
        emitter.line(1, "bytes: ::bytes::Bytes,");
        emitter.line(1, "content_type: Option<::mime::Mime>,");
        emitter.line(0, ") -> Result<String, ClientError> {");
        emitter.line(
            1,
            "String::from_utf8(bytes.to_vec()).map_err(|error| ClientError::Decode {",
        );
        emitter.line(2, "content_type,");
        emitter.line(2, "source: Box::new(error),");
        emitter.line(1, "})");
        emitter.line(0, "}");
    }

    if flags.needs_encode_overflow {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "Client-side encode overflow (§34.2): returned BEFORE anything is \
               sent."
                    .to_owned(),
            ],
        );
        emitter.line(0, "#[must_use]");
        emitter.line(0, "fn encode_overflow_error(limit: usize) -> ClientError {");
        emitter.line(1, "ClientError::BodyTooLarge {");
        emitter.line(2, "direction: BodyLimitDirection::Encode,");
        emitter.line(2, "limit,");
        emitter.line(1, "}");
        emitter.line(0, "}");
    }

    if flags.needs_response_header_parse {
        emitter.blank();
        emitter.docs(
            0,
            &[
                "Typed documented response headers (main spec §15): required \
                 headers missing from the response are protocol errors \
                 (`MissingRequiredHeader`), values failing their Rust type are \
                 `InvalidHeader`; both surface BEFORE the body is consumed. A \
                 repeated documented header reads its first occurrence."
                    .to_owned(),
            ],
        );
        if flags.needs_required_header_helper {
            emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
            emitter.line(0, "fn parse_required_header<T>(");
            emitter.line(1, "response: &::reqwest::Response,");
            emitter.line(1, "wire: &'static str,");
            emitter.line(0, ") -> Result<T, ClientError>");
            emitter.line(0, "where");
            emitter.line(1, "T: ::std::str::FromStr,");
            emitter.line(1, "T::Err: ::std::error::Error + Send + Sync + 'static,");
            emitter.line(0, "{");
            emitter.line(1, "let name = ::http::HeaderName::from_static(wire);");
            emitter.line(1, "let Some(raw) = response.headers().get(&name) else {");
            emitter.line(
                2,
                "return Err(ClientError::MissingRequiredHeader { name });",
            );
            emitter.line(1, "};");
            emitter.line(1, "parse_header_value(name, raw)");
            emitter.line(0, "}");
            emitter.blank();
        }
        if flags.needs_optional_header_helper {
            emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
            emitter.line(0, "fn parse_optional_header<T>(");
            emitter.line(1, "response: &::reqwest::Response,");
            emitter.line(1, "wire: &'static str,");
            emitter.line(0, ") -> Result<Option<T>, ClientError>");
            emitter.line(0, "where");
            emitter.line(1, "T: ::std::str::FromStr,");
            emitter.line(1, "T::Err: ::std::error::Error + Send + Sync + 'static,");
            emitter.line(0, "{");
            emitter.line(1, "let name = ::http::HeaderName::from_static(wire);");
            emitter.line(1, "match response.headers().get(&name) {");
            emitter.line(2, "Some(raw) => parse_header_value(name, raw).map(Some),");
            emitter.line(2, "None => Ok(None),");
            emitter.line(1, "}");
            emitter.line(0, "}");
            emitter.blank();
        }
        emitter.docs(
            0,
            &["Decodes one raw header value into its typed representation.".to_owned()],
        );
        emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
        emitter.line(0, "fn parse_header_value<T>(");
        emitter.line(1, "name: ::http::HeaderName,");
        emitter.line(1, "raw: &::http::HeaderValue,");
        emitter.line(0, ") -> Result<T, ClientError>");
        emitter.line(0, "where");
        emitter.line(1, "T: ::std::str::FromStr,");
        emitter.line(1, "T::Err: ::std::error::Error + Send + Sync + 'static,");
        emitter.line(0, "{");
        emitter.line(
            1,
            "let text = raw.to_str().map_err(|_| ClientError::InvalidHeader {",
        );
        emitter.line(2, "name: name.clone(),");
        emitter.line(2, "source: Box::new(NonUtf8HeaderValue),");
        emitter.line(1, "})?;");
        emitter.line(
            1,
            "text.parse().map_err(|source| ClientError::InvalidHeader {",
        );
        emitter.line(2, "name,");
        emitter.line(2, "source: Box::new(source),");
        emitter.line(1, "})");
        emitter.line(0, "}");

        emitter.blank();
        emit_error_type(
            emitter,
            "NonUtf8HeaderValue",
            None,
            "\"documented response header value is not valid UTF-8\"",
            "a documented response header carried non-UTF-8 bytes; generated \
             clients surface this as `ClientError::InvalidHeader`",
        );
    }

    emitter.blank();
    emitter.docs(
        0,
        &[
            "Substitutes server variables with builder overrides or declared \
             defaults, validating enum membership at build time (companion §8)."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[allow(clippy::missing_errors_doc)]");
    emitter.line(0, "fn substitute_server_variables(");
    emitter.line(1, "url: &str,");
    emitter.line(1, "variables: &[(String, String, Option<Vec<String>>)],");
    emitter.line(
        1,
        "overrides: &::std::collections::BTreeMap<String, String>,",
    );
    emitter.line(0, ") -> Result<String, ClientError> {");
    emitter.line(1, "let mut resolved = url.to_owned();");
    emitter.line(1, "for (name, default, allowed) in variables {");
    emitter.line(2, "let value = if let Some(value) = overrides.get(name) {");
    emitter.line(3, "value.clone()");
    emitter.line(2, "} else {");
    emitter.line(3, "default.clone()");
    emitter.line(2, "};");
    emitter.line(2, "if let Some(allowed) = allowed {");
    emitter.line(3, "if !allowed.contains(&value) {");
    emitter.line(4, "return Err(ClientError::InvalidUrl(format!(");
    emitter.line(
        5,
        "\"server variable `{name}` value `{value}` is not one of {allowed:?}\"",
    );
    emitter.line(4, ")));");
    emitter.line(3, "}");
    emitter.line(2, "}");
    emitter.line(2, "let placeholder = format!(\"{{{name}}}\");");
    emitter.line(2, "if !resolved.contains(&placeholder) {");
    emitter.line(3, "return Err(ClientError::InvalidUrl(format!(");
    emitter.line(
        4,
        "\"server variable `{name}` has no placeholder in `{url}`\"",
    );
    emitter.line(3, ")));");
    emitter.line(2, "}");
    emitter.line(2, "resolved = resolved.replace(&placeholder, &value);");
    emitter.line(1, "}");
    emitter.line(1, "if resolved.contains('{') || resolved.contains('}') {");
    emitter.line(2, "return Err(ClientError::InvalidUrl(format!(");
    emitter.line(
        3,
        "\"unresolved server variable placeholder in `{resolved}`\"",
    );
    emitter.line(2, ")));");
    emitter.line(1, "}");
    emitter.line(1, "Ok(resolved)");
    emitter.line(0, "}");

    if has_variables {
        emitter.blank();
        emitter.docs(
            0,
            &["One declared server variable in builder-ready form.".to_owned()],
        );
        emitter.line(0, "#[must_use]");
        emitter.line(0, "fn server_variable(");
        emitter.line(1, "name: &str,");
        emitter.line(1, "default: &str,");
        emitter.line(1, "allowed: &[&str],");
        emitter.line(0, ") -> (String, String, Option<Vec<String>>) {");
        emitter.line(1, "let allowed = if allowed.is_empty() {");
        emitter.line(2, "None");
        emitter.line(1, "} else {");
        emitter.line(
            2,
            "Some(allowed.iter().map(|value| (*value).to_owned()).collect())",
        );
        emitter.line(1, "};");
        emitter.line(1, "(name.to_owned(), default.to_owned(), allowed)");
        emitter.line(0, "}");
    }

    emitter.blank();
    emitter.docs(
        0,
        &[
            "Absolute-URL gate for the resolved base (D-impl-relative-servers): \
             scheme + `://` + non-empty remainder."
                .to_owned(),
        ],
    );
    emitter.line(0, "#[must_use]");
    emitter.line(0, "fn is_absolute_url(url: &str) -> bool {");
    emitter.line(
        1,
        "let Some((scheme, rest)) = url.split_once(\"://\") else {",
    );
    emitter.line(2, "return false;");
    emitter.line(1, "};");
    emitter.line(1, "!scheme.is_empty()");
    emitter.line(2, "&& scheme.chars().all(|character| {");
    emitter.line(
        3,
        "character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')",
    );
    emitter.line(2, "})");
    emitter.line(2, "&& !rest.is_empty()");
    emitter.line(0, "}");
}

/// One tiny source-error payload carried inside `ClientError::Decode`
/// `source`; generated code invents no `ClientError` variants beyond §36 —
/// these only implement `std::error::Error`.
fn emit_error_type(
    emitter: &mut Emitter,
    name: &str,
    field: Option<&str>,
    display_format: &str,
    doc: &str,
) {
    emitter.docs(0, &[doc.to_owned()]);
    emitter.line(0, "#[derive(Debug)]");
    match field {
        Some(field_type) => emitter.line(0, &format!("struct {name}({field_type});")),
        None => emitter.line(0, &format!("struct {name};")),
    }
    emitter.blank();
    emitter.line(0, &format!("impl std::fmt::Display for {name} {{"));
    emitter.line(
        1,
        "fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {",
    );
    match field {
        Some(_) => emitter.line(2, &format!("write!(f, {display_format}, self.0)")),
        None => emitter.line(2, &format!("f.write_str({display_format})")),
    }
    emitter.line(1, "}");
    emitter.line(0, "}");
    emitter.blank();
    emitter.line(0, &format!("impl std::error::Error for {name} {{}}"));
}

/// One `.header(PATH, VALUE)` element of a request-builder chain, laid out
/// the way rustfmt renders calls whose argument list exceeds
/// `fn_call_width` (60 columns): arguments break vertically.
fn emit_chain_header(emitter: &mut Emitter, indent: usize, path: &str, value: &str) {
    let value_literal = rust_string_literal(value);
    let horizontal = format!(".header({path}, {value_literal})");
    if path.len() + value_literal.len() + ", ".len() + "()".len() <= FN_CALL_WIDTH {
        emitter.line(indent, &horizontal);
        return;
    }
    emitter.line(indent, ".header(");
    emitter.line(indent + 1, &format!("{path},"));
    emitter.line(indent + 1, &format!("{value_literal},"));
    emitter.line(indent, ")");
}

/// rustfmt's default maximum width for a function/macro call argument list.
const FN_CALL_WIDTH: usize = 60;

// ----------------------------------------------------------------------
// Small emission utilities
// ----------------------------------------------------------------------

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

/// Uppercase HTTP verb for `::http::Method::{CONST}`.
fn http_method_const(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Put => "PUT",
        HttpMethod::Post => "POST",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Options => "OPTIONS",
        HttpMethod::Head => "HEAD",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Trace => "TRACE",
    }
}

/// IR style → support-crate style (identical variant names, mapped
/// exhaustively for safety).
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

/// Runtime conversion of one typed parameter into a `ParamValue` for the §6
/// encoders; `value_expr` is the local binding or `raw` inside option matches.
fn param_value_expr(parameter: &PlannedParameter, value_expr: String) -> String {
    match parameter.rust_type.as_str() {
        "String" => format!("ParamValue::Text({value_expr}.to_owned())"),
        "i32" => format!("ParamValue::Int(i64::from({value_expr}))"),
        "i64" => format!("ParamValue::Int({value_expr})"),
        "f64" => format!("ParamValue::Float({value_expr})"),
        "bool" => format!("ParamValue::Bool({value_expr})"),
        "Vec<String>" => format!(
            "ParamValue::Array({value_expr}.iter().map(|item| ParamValue::Text(item.clone())).collect::<Vec<_>>())"
        ),
        "Vec<i32>" => format!(
            "ParamValue::Array({value_expr}.iter().map(|item| ParamValue::Int(i64::from(*item))).collect::<Vec<_>>())"
        ),
        "Vec<i64>" => format!(
            "ParamValue::Array({value_expr}.iter().map(|item| ParamValue::Int(*item)).collect::<Vec<_>>())"
        ),
        "Vec<f64>" => format!(
            "ParamValue::Array({value_expr}.iter().map(|item| ParamValue::Float(*item)).collect::<Vec<_>>())"
        ),
        "Vec<bool>" => format!(
            "ParamValue::Array({value_expr}.iter().map(|item| ParamValue::Bool(*item)).collect::<Vec<_>>())"
        ),
        other => unreachable!("plan rejected unsupported parameter schema `{other}`"),
    }
}
