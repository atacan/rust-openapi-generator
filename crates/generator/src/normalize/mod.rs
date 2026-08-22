//! Normalization layer: resolved IR → the normalized model consumed by
//! codegen (companion §4, §5, §8, §10; DECISIONS.md D-§6/D-§10/
//! D-impl-oneoffallback).
//!
//! [`normalize`] takes an [`IrDocument`] and produces a
//! [`NormalizedDocument`]: operations carry effective servers (companion §8
//! precedence) and merged parameters; component schemas gain deterministic
//! Rust names (companion §10); composition keywords are resolved into
//! [`composition::ResolvedKind`]s without destroying the original IR.
//!
//! Determinism rules: declaration order everywhere; `BTreeMap` only for
//! name-keyed tables; no timestamps, no absolute paths.

pub mod composition;
pub mod dump;
pub mod naming;

use std::collections::BTreeMap;

use crate::diagnostics::{Diagnostic, DocumentPath, Severity};
use crate::ir::document::{
    HttpMethod, IrDocument, ParameterIr, RequestBodyIr, ResponseEntryIr, ServerIr,
};
use crate::ir::schema::{SchemaArena, SchemaId};

use composition::{ResolvedNode, Resolver};
use naming::NameAssignments;

/// Fallback policy for unprovable `oneOf` disjointness (companion §4.2;
/// DECISIONS.md D-impl-oneoffallback: default = raw/value representation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneOfFallbackMode {
    /// Raw/value fallback carrying validation metadata + Warning diagnostic.
    RawValue,
    /// Hard generation error listing schema paths.
    Error,
}

/// Normalizer configuration.
#[derive(Debug, Clone)]
pub struct NormalizeConfig {
    pub oneof_fallback: OneOfFallbackMode,
    /// Inline-nesting hint carried for later packages; normalization itself
    /// relies on the loader's enforced depth cap (companion §3).
    pub max_inline_depth_hint: u32,
}

impl Default for NormalizeConfig {
    fn default() -> Self {
        Self {
            oneof_fallback: OneOfFallbackMode::RawValue,
            max_inline_depth_hint: 64,
        }
    }
}

/// A named component schema in the normalized model.
#[derive(Debug, Clone)]
pub struct NormalizedSchema {
    /// Component name verbatim (`components/schemas/{name}`).
    pub component_name: String,
    /// Arena id of the original interned node (kinds preserved).
    pub source: SchemaId,
    /// Assigned Rust type name (`PascalCase`, companion §10).
    pub rust_type: String,
}

/// Where a merged parameter was declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterOrigin {
    PathLevel,
    OperationLevel,
}

/// One parameter after merging path-level and operation-level lists:
/// path-level entries come first, and an operation-level parameter replaces
/// a same-named same-location path-level entry at its position (OAS
/// override semantics), keeping declaration order deterministic.
#[derive(Debug, Clone)]
pub struct MergedParameter {
    pub parameter: ParameterIr,
    pub origin: ParameterOrigin,
}

/// One operation with servers and parameters fully resolved (companion §8,
/// §6). Bodies/responses are carried from the IR unchanged.
#[derive(Debug, Clone)]
pub struct NormalizedOperation {
    pub method: HttpMethod,
    pub path_template: String,
    /// Stable operation key, e.g. `"get /widgets"`.
    pub operation_key: String,
    pub operation_id: Option<String>,
    /// snake_case method name from the naming pipeline (companion §10).
    pub method_name: String,
    /// Response enum type name (main spec §4 table).
    pub response_enum: String,
    /// Effective servers per companion §8: op > path > root; absent or
    /// empty root implies `/`. First entry is the default base URL.
    pub effective_servers: Vec<ServerIr>,
    pub merged_parameters: Vec<MergedParameter>,
    pub tags: Vec<String>,
    pub request_body: Option<RequestBodyIr>,
    pub responses: Vec<ResponseEntryIr>,
    pub deprecated: bool,
}

/// The normalized model: the loaded IR augmented with resolved composition
/// shapes, effective servers/parameters, and assigned names.
#[derive(Debug)]
pub struct NormalizedDocument {
    pub version: crate::ir::document::OpenApiVersion,
    pub raw_version: String,
    /// Root-level servers verbatim (may be empty → `/` defaults apply per
    /// operation through companion §8 precedence).
    pub root_servers: Vec<ServerIr>,
    /// Operations in document order (path declaration order, then method
    /// declaration order within each path item).
    pub operations: Vec<NormalizedOperation>,
    /// Named component schemas keyed by component name.
    pub schemas: BTreeMap<String, NormalizedSchema>,
    /// Original IR arena (untouched).
    pub arena: SchemaArena,
    /// Resolution slot per arena node, index-aligned by [`SchemaId`].
    pub resolutions: Vec<ResolvedNode>,
    /// Assigned names (companion §10).
    pub names: NameAssignments,
    /// Diagnostics produced while normalizing (fallback Warnings etc.),
    /// in resolution order.
    pub diagnostics: Vec<Diagnostic>,
}

impl NormalizedDocument {
    /// Resolution of the node addressed by `id`.
    #[must_use]
    pub fn resolution(&self, id: SchemaId) -> &ResolvedNode {
        &self.resolutions[id.0 as usize]
    }

    /// Chases alias chains to the effective resolution of `id`.
    #[must_use]
    pub fn resolve_alias(&self, id: SchemaId) -> SchemaId {
        let mut current = id;
        let mut guard = 0_usize;
        while let Some(resolved) = self.resolutions.get(current.0 as usize) {
            guard += 1;
            if guard > self.resolutions.len() {
                return current;
            }
            match resolved.kind {
                composition::ResolvedKind::Alias(target) => current = target,
                _ => return current,
            }
        }
        current
    }
}

/// Normalizes with default configuration ([`NormalizeConfig::default`]).
pub fn normalize(doc: IrDocument) -> Result<NormalizedDocument, Vec<Diagnostic>> {
    normalize_with_config(doc, &NormalizeConfig::default())
}

/// Normalizes a loaded document into the normalized model.
///
/// Returns every diagnostic when an Error-severity condition was recorded
/// (property conflicts are generation errors per companion §4.1; unproven
/// `oneOf` is an error only under [`OneOfFallbackMode::Error`]); otherwise
/// returns the model carrying any Warning diagnostics.
pub fn normalize_with_config(
    doc: IrDocument,
    config: &NormalizeConfig,
) -> Result<NormalizedDocument, Vec<Diagnostic>> {
    // Component ids were pre-interned before paths traversal, so ascending
    // arena id recovers declaration order (needed by the naming pipeline).
    let mut components_by_id: Vec<(u32, String)> = doc
        .schemas
        .iter()
        .map(|(name, id)| (id.0, name.clone()))
        .collect();
    components_by_id.sort_unstable();

    // 1. Composition resolution: roots first (named paths), then sweep.
    let mut roots: Vec<(SchemaId, DocumentPath)> = components_by_id
        .iter()
        .map(|(index, name)| {
            let crumbs = DocumentPath::root()
                .key("components")
                .key("schemas")
                .key(name.clone());
            (SchemaId(*index), crumbs)
        })
        .collect();
    roots.extend(operation_roots(&doc));

    let mut resolver = Resolver::new(&doc.arena, config.clone());
    resolver.set_component_names(
        components_by_id
            .iter()
            .map(|(i, n)| (*i, n.clone()))
            .collect(),
    );
    resolver.resolve_all(&roots);

    // 2. Operations: server precedence + parameter merging (§6, §8).
    let mut operations = Vec::new();
    for path_entry in &doc.paths {
        for (method, operation) in &path_entry.operations {
            let operation_key = format!("{} {}", method.as_keyword(), path_entry.path);
            let effective_servers = effective_servers(
                operation.servers.as_ref(),
                path_entry.servers.as_ref(),
                &doc.servers,
            );
            let merged_parameters = merge_parameters(&path_entry.parameters, &operation.parameters);
            operations.push(NormalizedOperation {
                method: *method,
                path_template: path_entry.path.clone(),
                operation_key: operation_key.clone(),
                operation_id: operation.operation_id.clone(),
                method_name: String::new(),
                response_enum: String::new(),
                effective_servers,
                merged_parameters,
                tags: operation.tags.clone(),
                request_body: operation.request_body.clone(),
                responses: operation.responses.clone(),
                deprecated: operation.deprecated,
            });
        }
    }

    // 3. Naming pipeline (companion §10): schemas in declaration order,
    // operations in document order.
    let names = assign_names(&components_by_id, &operations);
    for operation in &mut operations {
        if let Some((_, method)) = names
            .operation_methods
            .iter()
            .find(|(key, _)| key == &operation.operation_key)
        {
            operation.method_name = method.clone();
        }
        if let Some((_, enum_name)) = names
            .response_enums
            .iter()
            .find(|(key, _)| key == &operation.operation_key)
        {
            operation.response_enum = enum_name.clone();
        }
    }

    let schemas: BTreeMap<String, NormalizedSchema> = doc
        .schemas
        .iter()
        .map(|(name, id)| {
            let rust_type = names
                .schema_types
                .get(name)
                .cloned()
                .unwrap_or_else(|| naming::ident(name, naming::NameStyle::Pascal));
            (
                name.clone(),
                NormalizedSchema {
                    component_name: name.clone(),
                    source: *id,
                    rust_type,
                },
            )
        })
        .collect();

    let (resolutions, mut diagnostics) = resolver.finish();
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    diagnostics.shrink_to_fit();

    Ok(NormalizedDocument {
        version: doc.version,
        raw_version: doc.raw_version,
        root_servers: doc.servers,
        operations,
        schemas,
        arena: doc.arena,
        resolutions,
        names,
        diagnostics,
    })
}

/// Document paths of every schema referenced from operations (request
/// bodies, responses, headers, parameters) so their first resolution uses
/// meaningful breadcrumbs.
fn operation_roots(doc: &IrDocument) -> Vec<(SchemaId, DocumentPath)> {
    let mut roots = Vec::new();
    for path_entry in &doc.paths {
        let base = DocumentPath::root()
            .key("paths")
            .key(path_entry.path.clone());
        for param in &path_entry.parameters {
            roots.push((
                param.schema,
                base.key("parameters").key(param.name.clone()).key("schema"),
            ));
        }
        for (method, operation) in &path_entry.operations {
            let op_base = base.key(method.as_keyword());
            for param in &operation.parameters {
                roots.push((
                    param.schema,
                    op_base
                        .key("parameters")
                        .key(param.name.clone())
                        .key("schema"),
                ));
            }
            if let Some(body) = &operation.request_body {
                for content in &body.content {
                    roots.push((
                        content.schema,
                        op_base
                            .key("requestBody")
                            .key("content")
                            .key(content.media_type.clone())
                            .key("schema"),
                    ));
                }
            }
            for response in &operation.responses {
                let resp_base = op_base.key("responses").key(status_label(&response.status));
                for (_, header) in &response.headers {
                    roots.push((
                        header.schema,
                        resp_base.key("headers").key("_").key("schema"),
                    ));
                }
                for content in &response.content {
                    roots.push((
                        content.schema,
                        resp_base
                            .key("content")
                            .key(content.media_type.clone())
                            .key("schema"),
                    ));
                }
            }
        }
    }
    roots
}

/// Renders a response status key as its document spelling (`200`, `2XX`,
/// `default`) for stable paths and dumps.
#[must_use]
pub fn status_label(status: &crate::ir::document::ResponseStatusKey) -> String {
    use crate::ir::document::{RangeClass, ResponseStatusKey};
    match status {
        ResponseStatusKey::Explicit(code) => code.to_string(),
        ResponseStatusKey::RangeClass(range) => match range {
            RangeClass::Success2xx => "2XX".to_owned(),
            RangeClass::Redirection3xx => "3XX".to_owned(),
            RangeClass::ClientError4xx => "4XX".to_owned(),
            RangeClass::ServerError5xx => "5XX".to_owned(),
        },
        ResponseStatusKey::Default => "default".to_owned(),
    }
}

/// Companion §8 precedence: an operation-level array overrides the
/// path-level array, which overrides the root-level array; an absent or
/// empty root-level array implies `/`. Present-but-empty overrides fall
/// through to the next level (an empty array selects nothing).
fn effective_servers(
    operation_level: Option<&Vec<ServerIr>>,
    path_level: Option<&Vec<ServerIr>>,
    root_level: &[ServerIr],
) -> Vec<ServerIr> {
    let chosen = operation_level
        .filter(|servers| !servers.is_empty())
        .or_else(|| path_level.filter(|servers| !servers.is_empty()))
        .map_or(root_level, |servers| servers.as_slice());
    if chosen.is_empty() {
        vec![ServerIr {
            url: "/".to_owned(),
            variables: Vec::new(),
        }]
    } else {
        chosen.to_vec()
    }
}

/// Merges path-level and operation-level parameters: path-level entries
/// come first; an operation-level parameter with the same name AND location
/// replaces the path-level entry at its position (OAS override semantics).
fn merge_parameters(
    path_level: &[ParameterIr],
    operation_level: &[ParameterIr],
) -> Vec<MergedParameter> {
    let mut merged: Vec<MergedParameter> = path_level
        .iter()
        .map(|parameter| MergedParameter {
            parameter: parameter.clone(),
            origin: ParameterOrigin::PathLevel,
        })
        .collect();
    for parameter in operation_level {
        match merged.iter_mut().find(|entry| {
            entry.origin == ParameterOrigin::PathLevel
                && entry.parameter.name == parameter.name
                && entry.parameter.location == parameter.location
        }) {
            Some(entry) => {
                entry.parameter = parameter.clone();
                entry.origin = ParameterOrigin::OperationLevel;
            }
            None => merged.push(MergedParameter {
                parameter: parameter.clone(),
                origin: ParameterOrigin::OperationLevel,
            }),
        }
    }
    merged
}

/// Runs the naming pipeline over schemas, operations, and tags
/// (companion §10; DECISIONS.md D-§6/D-§10).
fn assign_names(
    components_in_order: &[(u32, String)],
    operations: &[NormalizedOperation],
) -> NameAssignments {
    let mut names = NameAssignments::default();

    // Schema types: declaration order decides which duplicate keeps the
    // clean name (first occurrence wins, numeric suffixes follow).
    let mut used_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, component) in components_in_order {
        let assigned = unique_name(component, naming::NameStyle::Pascal, &mut used_types);
        names.schema_types.insert(component.clone(), assigned);
    }

    // Methods and response enums per operation, in document order.
    let method_requests: Vec<String> = operations
        .iter()
        .map(|operation| operation.method_request())
        .collect();
    let methods = naming::assign_unique(&method_requests, naming::NameStyle::Snake);
    let enum_requests: Vec<String> = methods
        .iter()
        .map(|method| {
            format!(
                "{}Response",
                naming::ident(method, naming::NameStyle::Pascal)
            )
        })
        .collect();
    // Response enums share no table with schema types (different scopes),
    // but duplicates among themselves get suffixes deterministically.
    let mut used_enums: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (operation, request) in operations.iter().zip(enum_requests.iter()) {
        let assigned = unique_joined(request, &mut used_enums);
        names
            .response_enums
            .push((operation.operation_key.clone(), assigned));
    }
    for (operation, method) in operations.iter().zip(&methods) {
        names
            .operation_methods
            .push((operation.operation_key.clone(), method.clone()));
    }

    // Tags map to modules with the same sanitization (companion §10).
    let mut used_modules: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for operation in operations {
        for tag in &operation.tags {
            if !names.tag_modules.contains_key(tag) {
                let module = unique_name(tag, naming::NameStyle::Snake, &mut used_modules);
                names.tag_modules.insert(tag.clone(), module);
            }
        }
    }

    names
}

fn unique_name(
    raw: &str,
    style: naming::NameStyle,
    used: &mut std::collections::BTreeSet<String>,
) -> String {
    let base = naming::ident(raw, style);
    unique_joined(&base, used)
}

fn unique_joined(base: &str, used: &mut std::collections::BTreeSet<String>) -> String {
    let mut candidate = base.to_owned();
    let mut counter = 1_u32;
    while used.contains(&candidate) {
        counter += 1;
        candidate = naming::sanitize_joined(&format!("{base}_{counter}"));
    }
    used.insert(candidate.clone());
    candidate
}

impl NormalizedOperation {
    /// Method-name request derived from the operationId when present,
    /// otherwise from the HTTP method plus path-template words
    /// (companion §10 preserves operationId word boundaries).
    fn method_request(&self) -> String {
        match &self.operation_id {
            Some(operation_id) => operation_id.clone(),
            None => {
                let mut words = vec![self.method.as_keyword().to_owned()];
                words.extend(split_path_words(&self.path_template));
                words.join("_")
            }
        }
    }
}

/// Splits a path template into identifier words: `/objects/{id}` →
/// `["objects", "id"]`.
fn split_path_words(template: &str) -> Vec<String> {
    let cleaned = template.replace(['{', '}'], " ");
    naming::split_words(&cleaned)
        .into_iter()
        .filter(|word| !word.is_empty())
        .collect()
}
