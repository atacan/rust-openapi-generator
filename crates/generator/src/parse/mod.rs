//! Document loading: YAML parsing, `$ref` resolution, and version
//! normalization into the IR (companion §2–§3, DECISIONS.md D-§2/D-§3/D-§4.4).
//!
//! Entry point: [`load_document`].
//!
//! Sibling-keyword policy around `$ref` (companion §3), classified by document
//! version and position:
//!
//! 1. OAS 3.0, any position: `$ref` is a pure reference; every sibling key is
//!    ignored with a warning diagnostic.
//! 2. OAS 3.1+/3.2 Reference-Object positions (parameters, request bodies,
//!    responses, headers, path items): `summary`/`description` are recognized
//!    reference metadata; every other sibling warns and is ignored.
//! 3. OAS 3.1+/3.2 Schema-Object positions: `$ref` is not collapsed;
//!    `summary`/`description` become reference metadata and remaining sibling
//!    keywords are preserved as conjunction terms (`Ref.inline_constraints`)
//!    for the normalization package.
//!
//! Cycle policy (companion §3, DECISIONS.md D-impl-boxing): recursion
//! through properties produces `Indirection::Boxed` edges — refined after
//! loading by an SCC pass so ONLY edges closing a property-recursion cycle
//! stay boxed (acyclic property edges are direct) — arrays/maps leave edges
//! direct, and a cycle reached with no container break is an
//! `UnbrokenSelfContainment` error.

mod raw;
mod refs;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde_yaml::{Mapping, Value as Yaml};

use crate::diagnostics::{Diagnostic, Diagnostics, DocumentPath, Severity};
use crate::ir::document::{
    classify_media_type, ContentEntryIr, HeaderSpecIr, HttpMethod, IrDocument, OpenApiVersion,
    OperationIr, ParameterIr, ParameterLocation, ParameterStyle, PathEntry, RangeClass,
    RequestBodyIr, ResponseEntryIr, ResponseStatusKey, ServerIr, ServerVariable,
};
use crate::ir::schema::{
    AdditionalPropertiesPolicy, DiscriminatorIr, EnumValues, Indirection, PropertyIr, SchemaArena,
    SchemaEdge, SchemaId, SchemaKind, SchemaNode, SchemaRefName, UnsupportedReason, ValidationMeta,
};

use raw::{
    as_mapping, detect_version, is_ref_mapping, mapping_get, mapping_has, string_field,
    stringify_scalar, yaml_to_json,
};
use refs::{canonical_pointer_key, display_pointer, split_ref, tokenize_pointer, walk_pointer};

/// Guard against pathological `$ref` chains between reference objects
/// (parameters → parameters → …), which are not graph-addressable.
const MAX_ENTITY_REF_DEPTH: u32 = 32;

/// Headers, content, and reference metadata extracted from a Response
/// Object.
type ResponseParts = (Vec<(String, HeaderSpecIr)>, Vec<ContentEntryIr>, RefMeta);

/// Reference metadata recognized on OAS 3.1+/3.2 Reference Objects
/// surrounding an entity `$ref` and carried into the resolved entity
/// (companion §3).
#[derive(Debug, Default)]
struct RefMeta {
    summary: Option<String>,
    description: Option<String>,
}

impl RefMeta {
    /// Captures `summary`/`description` siblings from a Reference Object;
    /// OAS 3.0 documents ignore them with a warning instead (handled by
    /// [`Loader::note_entity_ref_siblings`]). Earlier hops of a `$ref`
    /// chain win over later ones.
    fn absorb(&mut self, version: OpenApiVersion, mapping: &Mapping) {
        if !version.is_at_least_3_1() {
            return;
        }
        if self.summary.is_none() {
            self.summary = mapping_get(mapping, "summary")
                .and_then(Yaml::as_str)
                .map(ToOwned::to_owned);
        }
        if self.description.is_none() {
            self.description = mapping_get(mapping, "description")
                .and_then(Yaml::as_str)
                .map(ToOwned::to_owned);
        }
    }

    /// Final metadata for the resolved entity: carried Reference-Object
    /// siblings win over the entity object's own top-level description;
    /// inline entities contribute no `summary` (companion §3).
    fn finish(self, value: &Yaml) -> (Option<String>, Option<String>) {
        let description = self
            .description
            .or_else(|| string_field(value, "description").map(ToOwned::to_owned));
        (self.summary, description)
    }
}

/// Loader configuration.
#[derive(Debug, Clone)]
pub struct LoadConfig {
    /// Maximum inline-schema nesting depth; exceeding it is an error
    /// diagnostic, never silent truncation (companion §3).
    pub max_inline_depth: u32,
    /// Whether relative external-file references are followed (D-§3: remote
    /// URL references are never fetched).
    pub external_files: bool,
}

impl Default for LoadConfig {
    fn default() -> Self {
        Self {
            max_inline_depth: 64,
            external_files: true,
        }
    }
}

/// Loads and resolves an OpenAPI document into the version-agnostic IR.
///
/// Returns the [`IrDocument`] when no error-severity diagnostic was recorded;
/// otherwise returns every diagnostic in document traversal order.
pub fn load_document(
    root_yaml: &str,
    root_dir: &Path,
    config: &LoadConfig,
) -> Result<IrDocument, Vec<Diagnostic>> {
    let mut loader = Loader::new(config.clone());
    match loader.open_root(root_yaml, root_dir) {
        Ok(()) => {
            let mut document = loader.run()?;
            apply_cycle_precise_indirection(&mut document.arena);
            Ok(document)
        }
        Err(()) => Err(loader.diags.into_vec()),
    }
}

/// Cycle-precise heap indirection (companion §3; DECISIONS.md
/// D-impl-boxing): recomputes every property edge's [`Indirection`] after
/// loading completes.
///
/// Strongly-connected components are computed over the restricted edge set
/// of property, composition, and `$ref` edges ONLY — array/tuple items,
/// schema-valued `additionalProperties`, `patternProperties`, and `contains`
/// are container positions that break recursion without boxing. A property
/// edge carries [`Indirection::Boxed`] exactly when its source and target
/// share an SCC (the edge closes a property-recursion cycle); every other
/// edge ends up [`Indirection::None`]. All external-file schemas are interned
/// into this single global arena, so one pass covers the whole document set.
fn apply_cycle_precise_indirection(arena: &mut SchemaArena) {
    let adjacency = recursion_adjacency(arena);
    let components = tarjan_components(&adjacency);
    for index in 0..arena.len() {
        let id = SchemaId(index as u32);
        let mut node = arena.get(id).clone();
        if rewrite_property_indirection(&mut node.kind, components[index], &components) {
            arena.complete(id, node);
        }
    }
}

/// Adjacency over the restricted (recursion-relevant) edge set: object
/// properties plus composition/`$ref` targets. Container edges
/// (`items`, `prefixItems`, schema-valued `additionalProperties`,
/// `patternProperties`, `contains`) are excluded — they break cycles and are
/// never boxed (companion §3).
fn recursion_adjacency(arena: &SchemaArena) -> Vec<Vec<u32>> {
    let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); arena.len()];
    for (id, node) in arena.iter() {
        let neighbors = &mut adjacency[id.0 as usize];
        match &node.kind {
            SchemaKind::Object { properties, .. } => {
                for property in properties {
                    neighbors.push(property.schema.target.0);
                }
            }
            SchemaKind::Ref {
                target,
                inline_constraints,
                ..
            } => {
                neighbors.push(target.0);
                for constraint in inline_constraints {
                    neighbors.push(constraint.target.0);
                }
            }
            SchemaKind::AllOf { members, .. }
            | SchemaKind::OneOf { members, .. }
            | SchemaKind::AnyOf { members, .. } => {
                for member in members {
                    neighbors.push(member.target.0);
                }
            }
            _ => {}
        }
    }
    adjacency
}

/// Iterative Tarjan strongly-connected components (no recursion blowups);
/// roots are visited in ascending arena-id order and neighbors in stored
/// order so the numbering is deterministic. Only component equality matters
/// downstream.
fn tarjan_components(adjacency: &[Vec<u32>]) -> Vec<usize> {
    let count = adjacency.len();
    let mut discovered = vec![u32::MAX; count];
    let mut lowlink = vec![0_u32; count];
    let mut on_stack = vec![false; count];
    let mut components = vec![usize::MAX; count];
    let mut stack: Vec<u32> = Vec::new();
    let mut frames: Vec<(u32, usize)> = Vec::new();
    let mut next_index = 0_u32;
    let mut next_component = 0_usize;

    for root in 0..count {
        if discovered[root] != u32::MAX {
            continue;
        }
        discovered[root] = next_index;
        lowlink[root] = next_index;
        next_index += 1;
        stack.push(root as u32);
        on_stack[root] = true;
        frames.push((root as u32, 0));
        while let Some(frame) = frames.last() {
            let node = frame.0 as usize;
            let position = frame.1;
            if position < adjacency[node].len() {
                frames.last_mut().expect("frame checked above").1 += 1;
                let successor = adjacency[node][position] as usize;
                if discovered[successor] == u32::MAX {
                    discovered[successor] = next_index;
                    lowlink[successor] = next_index;
                    next_index += 1;
                    stack.push(successor as u32);
                    on_stack[successor] = true;
                    frames.push((successor as u32, 0));
                } else if on_stack[successor] {
                    lowlink[node] = lowlink[node].min(discovered[successor]);
                }
            } else {
                frames.pop();
                if let Some(&(parent, _)) = frames.last() {
                    let parent = parent as usize;
                    lowlink[parent] = lowlink[parent].min(lowlink[node]);
                }
                if lowlink[node] == discovered[node] {
                    while let Some(top) = stack.pop() {
                        on_stack[top as usize] = false;
                        components[top as usize] = next_component;
                        if top as usize == node {
                            break;
                        }
                    }
                    next_component += 1;
                }
            }
        }
    }
    components
}

/// Rewrites one node's edge indirection flags in place: a property edge is
/// boxed iff its target shares the source's SCC under the restricted edge
/// set (D-impl-boxing); all other edges stay/become direct. Returns true
/// when the node changed.
fn rewrite_property_indirection(
    kind: &mut SchemaKind,
    source_component: usize,
    components: &[usize],
) -> bool {
    let SchemaKind::Object { properties, .. } = kind else {
        return false;
    };
    let mut changed = false;
    for property in properties {
        let boxed = components[property.schema.target.0 as usize] == source_component;
        let desired = if boxed {
            Indirection::Boxed
        } else {
            Indirection::None
        };
        if property.schema.indirection != desired {
            property.schema.indirection = desired;
            changed = true;
        }
    }
    changed
}

/// Where a schema-valued edge sits inside its parent; decides cycle
/// indirection policy (companion §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeContext {
    /// Object property: boxed at build time; the post-load SCC pass
    /// ([`apply_cycle_precise_indirection`], D-impl-boxing) later keeps
    /// `Boxed` only on edges closing a property-recursion cycle.
    Property,
    /// Array items / tuple items / additionalProperties / patternProperties /
    /// contains: containers break recursion without boxing.
    Container,
    /// allOf/oneOf/anyOf members and `$ref` sibling conjunction terms:
    /// composition positions provide no heap break of their own.
    Composition,
}

impl EdgeContext {
    fn base_indirection(self) -> Indirection {
        match self {
            // Build-time default; refined cycle-precisely after loading
            // (companion §3, D-impl-boxing).
            Self::Property => Indirection::Boxed,
            _ => Indirection::None,
        }
    }

    fn breaks_recursion(self) -> bool {
        matches!(self, Self::Property | Self::Container)
    }
}

/// Outcome of resolving a `$ref` target for schema positions.
#[derive(Debug, Clone, Copy)]
enum RefTarget {
    /// Expansion finished (or previously memoized).
    Ready(SchemaId),
    /// Expansion is on the resolution stack (cycle).
    InProgress(SchemaId),
}

#[derive(Debug)]
enum RefResolution {
    Target(RefTarget),
    /// Reference failed; points at a freshly interned `NotSupported` node.
    Fallback(SchemaId),
}

/// A failed `$ref` resolution with its diagnostic and fallback classification.
#[derive(Debug)]
struct RefFailure {
    code: &'static str,
    reason: UnsupportedReason,
    message: String,
}

impl RefFailure {
    fn new(code: &'static str, reason: UnsupportedReason, message: impl Into<String>) -> Self {
        Self {
            code,
            reason,
            message: message.into(),
        }
    }
}

/// One frame of the component-expansion resolution stack.
#[derive(Debug)]
struct StackFrame {
    doc: String,
    pointer_key: String,
    reserved: SchemaId,
    /// Cumulative container-break counter at push time; a re-entry is
    /// "unbroken" when the counter has not advanced since this snapshot.
    breaks_at_push: usize,
}

struct Loader {
    config: LoadConfig,
    diags: Diagnostics,
    version: OpenApiVersion,
    raw_version: String,
    root_doc: String,
    /// Canonical file path → parsed document root.
    files: BTreeMap<String, Yaml>,
    /// (canonical file, canonical pointer) → completed schema id.
    schema_memo: BTreeMap<(String, String), SchemaId>,
    stack: Vec<StackFrame>,
    /// Monotonic count of property/container descents; compared against
    /// [`StackFrame::breaks_at_push`] to classify cycles (companion §3).
    total_breaks: usize,
    arena: SchemaArena,
}

impl Loader {
    fn new(config: LoadConfig) -> Self {
        Self {
            config,
            diags: Diagnostics::new(),
            version: OpenApiVersion::V3_1,
            raw_version: String::new(),
            root_doc: String::new(),
            files: BTreeMap::new(),
            schema_memo: BTreeMap::new(),
            stack: Vec::new(),
            total_breaks: 0,
            arena: SchemaArena::new(),
        }
    }

    fn note(
        &mut self,
        severity: Severity,
        path: DocumentPath,
        code: &'static str,
        message: impl Into<String>,
    ) {
        self.diags.push(Diagnostic {
            severity,
            path,
            code,
            message: message.into(),
        });
    }

    fn note_error(&mut self, path: DocumentPath, code: &'static str, message: impl Into<String>) {
        self.note(Severity::Error, path, code, message);
    }

    fn note_warning(&mut self, path: DocumentPath, code: &'static str, message: impl Into<String>) {
        self.note(Severity::Warning, path, code, message);
    }

    fn schema_fallback(&mut self, reason: UnsupportedReason) -> SchemaId {
        self.arena
            .intern(SchemaNode::new(SchemaKind::NotSupported { reason }))
    }

    // ------------------------------------------------------------------
    // Root opening and file loading
    // ------------------------------------------------------------------

    fn open_root(&mut self, root_yaml: &str, root_dir: &Path) -> Result<(), ()> {
        let joined = Path::new(root_yaml);
        let path = if joined.is_absolute() {
            joined.to_path_buf()
        } else {
            root_dir.join(joined)
        };
        let canonical = match std::fs::canonicalize(normalize_lexical(&path)) {
            Ok(p) => p,
            Err(err) => {
                self.diags.error(
                    DocumentPath::root(),
                    "root_unreadable",
                    format!("cannot open root document `{}`: {err}", path.display()),
                );
                return Err(());
            }
        };
        let key = canonical.to_string_lossy().into_owned();
        let text = match std::fs::read_to_string(&canonical) {
            Ok(t) => t,
            Err(err) => {
                self.diags.error(
                    DocumentPath::root(),
                    "root_unreadable",
                    format!("cannot read `{key}`: {err}"),
                );
                return Err(());
            }
        };
        let parsed: Yaml = match serde_yaml::from_str(&text) {
            Ok(v) => v,
            Err(err) => {
                self.diags.error(
                    DocumentPath::root(),
                    "yaml_parse_error",
                    format!("cannot parse `{key}`: {err}"),
                );
                return Err(());
            }
        };
        let (version, raw_version) = match detect_version(&parsed) {
            Ok(v) => v,
            Err(message) => {
                self.diags
                    .error(DocumentPath::root(), "version_unsupported", message);
                return Err(());
            }
        };
        self.version = version;
        self.raw_version = raw_version;
        self.root_doc = key.clone();
        self.files.insert(key, parsed);
        Ok(())
    }

    /// Resolves a `$ref` string against the document that contains it.
    /// Returns the target document key and RFC 6901 tokens.
    fn resolve_reference(
        &mut self,
        reference: &str,
        referencing_doc: &str,
    ) -> Result<(String, Vec<String>), RefFailure> {
        let parts = split_ref(reference);
        let doc_key = if parts.file.is_empty() {
            referencing_doc.to_owned()
        } else {
            self.load_external_file(&parts.file, referencing_doc)?
        };
        let tokens = match tokenize_pointer(&parts.fragment) {
            Ok(tokens) => tokens,
            Err(refs::PointerParseError::AnchorStyle) => {
                return Err(RefFailure::new(
                    "ref_anchor_unsupported",
                    UnsupportedReason::AnchorRef,
                    format!(
                        "`{reference}` uses a plain-name fragment; only RFC 6901 pointers are \
                         supported ($anchor/$id rebasing is unsupported, D-§3)"
                    ),
                ));
            }
            Err(refs::PointerParseError::InvalidEscape) => {
                return Err(RefFailure::new(
                    "ref_invalid_pointer",
                    UnsupportedReason::Other("invalid JSON Pointer"),
                    format!("`{reference}` contains an invalid `~` escape"),
                ));
            }
        };
        Ok((doc_key, tokens))
    }

    fn load_external_file(
        &mut self,
        file_part: &str,
        referencing_doc: &str,
    ) -> Result<String, RefFailure> {
        if let Some(scheme) = refs::detect_scheme(file_part) {
            let remote = matches!(scheme.as_str(), "http" | "https");
            return Err(RefFailure::new(
                "ref_remote_url",
                if remote {
                    UnsupportedReason::RemoteRefUnfetched
                } else {
                    UnsupportedReason::Other("unsupported reference scheme")
                },
                format!(
                    "remote/URI-scheme reference `{scheme}:…` cannot be resolved; only local \
                     documents and relative external files are supported (D-§3)"
                ),
            ));
        }
        if file_part.starts_with('/') {
            return Err(RefFailure::new(
                "ref_absolute_path",
                UnsupportedReason::Other("absolute-path reference"),
                format!("absolute external path `{file_part}` is not a relative reference (D-§3)"),
            ));
        }
        if !self.config.external_files {
            return Err(RefFailure::new(
                "ref_external_disabled",
                UnsupportedReason::Other("external references disabled"),
                format!("external file `{file_part}` referenced but external_files is disabled"),
            ));
        }
        let referencing_path = Path::new(referencing_doc);
        let base_dir = referencing_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty());
        let joined = base_dir.map_or_else(|| PathBuf::from(file_part), |base| base.join(file_part));
        let normalized = normalize_lexical(&joined);
        let canonical = match std::fs::canonicalize(&normalized) {
            Ok(p) => p,
            Err(err) => {
                return Err(RefFailure::new(
                    "ref_external_missing",
                    UnsupportedReason::Other("external reference unavailable"),
                    format!(
                        "external file `{}` is missing or unreadable: {err}",
                        normalized.display()
                    ),
                ))
            }
        };
        let key = canonical.to_string_lossy().into_owned();
        if self.files.contains_key(&key) {
            return Ok(key);
        }
        let text = std::fs::read_to_string(&canonical).map_err(|err| {
            RefFailure::new(
                "ref_external_missing",
                UnsupportedReason::Other("external reference unavailable"),
                format!("external file `{key}` is unreadable: {err}"),
            )
        })?;
        let parsed: Yaml = serde_yaml::from_str(&text).map_err(|err| {
            RefFailure::new(
                "ref_external_missing",
                UnsupportedReason::Other("external reference unavailable"),
                format!("external file `{key}` is not valid YAML: {err}"),
            )
        })?;
        self.files.insert(key.clone(), parsed);
        Ok(key)
    }

    fn note_ref_failure(&mut self, path: &DocumentPath, reference: &str, failure: &RefFailure) {
        self.note_error(
            path.clone(),
            failure.code,
            format!("cannot resolve `{reference}`: {}", failure.message),
        );
    }

    /// Resolves a `$ref` for reference-object positions (parameters, request
    /// bodies, responses, headers, path items) and clones the target value.
    fn resolve_entity_value(
        &mut self,
        reference: &str,
        referencing_doc: &str,
        path: &DocumentPath,
    ) -> Option<Yaml> {
        match self.resolve_reference(reference, referencing_doc) {
            Ok((doc_key, tokens)) => {
                let walked = self
                    .files
                    .get(&doc_key)
                    .and_then(|root| walk_pointer(root, &tokens))
                    .cloned();
                match walked {
                    Some(value) => Some(value),
                    None => {
                        self.note_error(
                            path.clone(),
                            "ref_pointer_unresolved",
                            format!(
                                "pointer `{}` does not resolve in `{doc_key}`",
                                display_pointer(&tokens)
                            ),
                        );
                        None
                    }
                }
            }
            Err(failure) => {
                self.note_ref_failure(path, reference, &failure);
                None
            }
        }
    }

    // ------------------------------------------------------------------
    // Schema resolution machinery
    // ------------------------------------------------------------------

    /// Expands and interns the schema found at `(doc, tokens)`; previously
    /// completed targets reuse their memoized id, and re-entry into an
    /// in-progress target returns its reserved id so cycles stay detectable.
    fn intern_schema_at(
        &mut self,
        doc: &str,
        pointer_key: String,
        value: &Yaml,
        path: &DocumentPath,
    ) -> SchemaId {
        let memo_key = (doc.to_owned(), pointer_key.clone());
        if let Some(id) = self.schema_memo.get(&memo_key) {
            return *id;
        }
        if let Some(frame) = self
            .stack
            .iter()
            .find(|f| f.doc == memo_key.0 && f.pointer_key == memo_key.1)
        {
            return frame.reserved;
        }
        let reserved = self.arena.reserve();
        self.stack.push(StackFrame {
            doc: memo_key.0.clone(),
            pointer_key,
            reserved,
            breaks_at_push: self.total_breaks,
        });
        let node = self.build_node_value(value, doc, path, 0);
        self.stack.pop();
        self.arena.complete(reserved, node);
        self.schema_memo.insert(memo_key, reserved);
        reserved
    }

    /// Full `$ref` resolution for schema positions with memoization and
    /// cycle detection.
    fn resolve_schema_target(
        &mut self,
        reference: &str,
        referencing_doc: &str,
        path: &DocumentPath,
    ) -> RefResolution {
        let (doc_key, tokens) = match self.resolve_reference(reference, referencing_doc) {
            Ok(ok) => ok,
            Err(failure) => {
                self.note_ref_failure(path, reference, &failure);
                return RefResolution::Fallback(self.schema_fallback(failure.reason));
            }
        };
        let pointer_key = canonical_pointer_key(&tokens);
        if let Some(id) = self
            .schema_memo
            .get(&(doc_key.clone(), pointer_key.clone()))
        {
            return RefResolution::Target(RefTarget::Ready(*id));
        }
        if let Some(frame) = self
            .stack
            .iter()
            .find(|f| f.doc == doc_key && f.pointer_key == pointer_key)
        {
            return RefResolution::Target(RefTarget::InProgress(frame.reserved));
        }
        let target = match self
            .files
            .get(&doc_key)
            .and_then(|root| walk_pointer(root, &tokens))
            .cloned()
        {
            Some(value) => value,
            None => {
                self.note_error(
                    path.clone(),
                    "ref_pointer_unresolved",
                    format!(
                        "cannot resolve `{reference}`: pointer `{}` does not resolve in \
                         `{doc_key}`",
                        display_pointer(&tokens)
                    ),
                );
                return RefResolution::Fallback(
                    self.schema_fallback(UnsupportedReason::Other("unresolved $ref")),
                );
            }
        };
        RefResolution::Target(RefTarget::Ready(self.intern_schema_at(
            &doc_key,
            pointer_key,
            &target,
            path,
        )))
    }

    /// Builds (but does not intern) the schema node for a raw value.
    fn build_node_value(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
    ) -> SchemaNode {
        match value {
            Yaml::Bool(true) => SchemaNode::new(SchemaKind::AnyValue),
            Yaml::Bool(false) => SchemaNode::new(SchemaKind::NotSupported {
                reason: UnsupportedReason::Other("literal false schema"),
            }),
            Yaml::Mapping(mapping) => {
                if mapping_has(mapping, "$ref") {
                    self.build_ref_node(mapping, doc, path, depth)
                } else {
                    self.normalize_schema(mapping, doc, path, depth)
                }
            }
            _ => {
                self.note_warning(
                    path.clone(),
                    "schema_not_mapping",
                    "schema position holds a non-object value; falling back to raw/value",
                );
                SchemaNode::new(SchemaKind::NotSupported {
                    reason: UnsupportedReason::Other("non-object schema"),
                })
            }
        }
    }

    /// Depth-checked interning entry point for any schema position.
    fn build_schema_id(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
    ) -> SchemaId {
        if depth > self.config.max_inline_depth {
            self.note_error(
                path.clone(),
                "inline_depth_exceeded",
                format!(
                    "inline schema nesting exceeds max_inline_depth ({})",
                    self.config.max_inline_depth
                ),
            );
            return self.schema_fallback(UnsupportedReason::InlineExpansionDepthExceeded);
        }
        let node = self.build_node_value(value, doc, path, depth);
        self.arena.intern(node)
    }

    /// Creates an edge to a child schema at the given context; container
    /// descents advance the cycle-break counter before expansion.
    fn child_edge(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
        ctx: EdgeContext,
    ) -> SchemaEdge {
        if ctx.breaks_recursion() {
            self.total_breaks += 1;
        }
        let target = self.build_schema_id(value, doc, path, depth);
        SchemaEdge {
            target,
            indirection: ctx.base_indirection(),
        }
    }

    /// Handles a `$ref`-bearing mapping at a schema position (companion §3
    /// sibling rules) and returns the completed node.
    fn build_ref_node(
        &mut self,
        mapping: &Mapping,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
    ) -> SchemaNode {
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let Some(reference) = mapping_get(mapping, "$ref").and_then(Yaml::as_str) else {
            self.note_error(
                path.key("$ref"),
                "schema_invalid_ref",
                "`$ref` must be a string",
            );
            return SchemaNode::new(SchemaKind::NotSupported {
                reason: UnsupportedReason::Other("non-string $ref"),
            });
        };

        let mut summary = None;
        let mut description = None;
        if self.version.is_at_least_3_1() {
            summary = mapping_get(mapping, "summary")
                .and_then(Yaml::as_str)
                .map(ToOwned::to_owned);
            description = mapping_get(mapping, "description")
                .and_then(Yaml::as_str)
                .map(ToOwned::to_owned);
            // Remaining siblings are conjunction terms (case c).
            let mut synth = Mapping::new();
            for (key, value) in mapping {
                let Some(key_str) = key.as_str() else {
                    continue;
                };
                if matches!(key_str, "$ref" | "summary" | "description") {
                    continue;
                }
                synth.insert(key.clone(), value.clone());
            }
            if !synth.is_empty() {
                let synth_value = Yaml::Mapping(synth);
                let synth_id =
                    self.build_schema_id(&synth_value, doc, path, depth.saturating_add(1));
                let constraint = SchemaEdge {
                    target: synth_id,
                    indirection: Indirection::None,
                };
                return self.finish_ref_node(
                    reference,
                    doc,
                    path,
                    Some(constraint),
                    summary,
                    description,
                    diagnostics,
                );
            }
        } else {
            // OAS 3.0: every sibling key is ignored with a warning (case a).
            let ignored: Vec<&str> = mapping
                .keys()
                .filter_map(Yaml::as_str)
                .filter(|key| *key != "$ref")
                .collect();
            if !ignored.is_empty() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.clone(),
                    code: "sibling_ignored",
                    message: format!(
                        "OAS 3.0 nodes containing `$ref` are pure references; ignored sibling \
                         keys: {}",
                        ignored.join(", ")
                    ),
                });
            }
        }
        self.finish_ref_node(
            reference,
            doc,
            path,
            None,
            summary,
            description,
            diagnostics,
        )
    }

    /// Resolves the target of a classified `$ref` node and assembles it.
    #[allow(clippy::too_many_arguments)]
    fn finish_ref_node(
        &mut self,
        reference: &str,
        doc: &str,
        path: &DocumentPath,
        inline_constraint: Option<SchemaEdge>,
        summary: Option<String>,
        description: Option<String>,
        mut diagnostics: Vec<Diagnostic>,
    ) -> SchemaNode {
        let inline_constraints = inline_constraint.map_or_else(Vec::new, |edge| vec![edge]);
        match self.resolve_schema_target(reference, doc, path) {
            RefResolution::Fallback(fallback) => {
                let mut node = SchemaNode::new(SchemaKind::Ref {
                    target: fallback,
                    summary: None,
                    description: None,
                    inline_constraints: Vec::new(),
                });
                node.diagnostics = diagnostics;
                node
            }
            RefResolution::Target(RefTarget::Ready(target)) => {
                let mut node = SchemaNode::new(SchemaKind::Ref {
                    target,
                    summary,
                    description,
                    inline_constraints,
                });
                node.diagnostics = diagnostics;
                node
            }
            RefResolution::Target(RefTarget::InProgress(target)) => {
                // Re-entry: only legal when a property/container break
                // happened since the target's expansion started.
                let broken = match self
                    .stack
                    .iter()
                    .rev()
                    .find(|frame| frame.reserved == target)
                {
                    Some(frame) => self.total_breaks > frame.breaks_at_push,
                    None => true,
                };
                if !broken {
                    self.note_error(
                        path.clone(),
                        "ref_self_containment",
                        format!(
                            "`{reference}` participates in a cycle with no container break; a \
                             value would have to contain itself (companion §3)"
                        ),
                    );
                    diagnostics.clear();
                    return SchemaNode::new(SchemaKind::NotSupported {
                        reason: UnsupportedReason::UnbrokenSelfContainment,
                    });
                }
                let mut node = SchemaNode::new(SchemaKind::Ref {
                    target,
                    summary,
                    description,
                    inline_constraints,
                });
                node.diagnostics = diagnostics;
                node
            }
        }
    }

    // ------------------------------------------------------------------
    // Keyword normalization (companion §2, DECISIONS.md D-§2)
    // ------------------------------------------------------------------

    /// Normalizes a non-`$ref` schema mapping into a node.
    fn normalize_schema(
        &mut self,
        mapping: &Mapping,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
    ) -> SchemaNode {
        let mut spec = SchemaSpec::default();
        for (key_value, value) in mapping {
            let Some(key) = key_value.as_str() else {
                self.note_warning(
                    path.clone(),
                    "keyword_unknown",
                    format!(
                        "ignored non-string schema key `{}`",
                        stringify_scalar(key_value)
                    ),
                );
                continue;
            };
            self.absorb_keyword(&mut spec, key, value, doc, path, depth);
        }
        spec.assemble(path)
    }

    /// Processes one schema keyword into the working specification.
    #[allow(clippy::too_many_lines)]
    fn absorb_keyword(
        &mut self,
        spec: &mut SchemaSpec,
        key: &str,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
    ) {
        let child_depth = depth.saturating_add(1);
        match key {
            "type" => match value {
                Yaml::String(s) => spec.type_spec = Some(TypeSpec::Single(s.clone())),
                Yaml::Sequence(items) => {
                    let mut entries = Vec::with_capacity(items.len());
                    for item in items {
                        match item.as_str() {
                            Some(s) => entries.push(s.to_owned()),
                            None => {
                                spec.node_diagnostics.push(Diagnostic {
                                    severity: Severity::Error,
                                    path: path.key("type"),
                                    code: "type_array_invalid",
                                    message: format!(
                                        "`type` array entries must be strings, found `{}`",
                                        stringify_scalar(item)
                                    ),
                                });
                                spec.poisoned
                                    .get_or_insert(UnsupportedReason::Other("invalid type array"));
                            }
                        }
                    }
                    spec.type_spec = Some(TypeSpec::List(entries));
                }
                other => {
                    spec.node_diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        path: path.key("type"),
                        code: "type_array_invalid",
                        message: format!(
                            "`type` must be a string or array, found `{}`",
                            stringify_scalar(other)
                        ),
                    });
                    spec.poisoned
                        .get_or_insert(UnsupportedReason::Other("invalid type"));
                }
            },
            "nullable" => {
                if value.as_bool() == Some(true) {
                    spec.nullable = true;
                }
                if self.version.is_at_least_3_1() {
                    spec.node_diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        path: path.key("nullable"),
                        code: "nullable_in_31",
                        message: "`nullable` was removed in OAS 3.1; honored for compatibility"
                            .to_owned(),
                    });
                }
            }
            "readOnly" => spec.read_only = value.as_bool().unwrap_or(false),
            "writeOnly" => spec.write_only = value.as_bool().unwrap_or(false),
            "default" => match yaml_to_json(value) {
                Ok(json) => spec.default = Some(json),
                Err(err) => spec.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("default"),
                    code: "default_unrepresentable",
                    message: format!("`default` is not representable as JSON: {err}"),
                }),
            },
            "title" => spec.title = value.as_str().map(ToOwned::to_owned),
            "description" => spec.description = value.as_str().map(ToOwned::to_owned),
            // `deprecated` affects generated doc attributes only (codegen-time).
            "deprecated" => {}
            "format" => spec.format = value.as_str().map(ToOwned::to_owned),
            "pattern" => spec.validation.pattern = value.as_str().map(ToOwned::to_owned),
            "minLength" => spec.validation.min_length = number_u64(value),
            "maxLength" => spec.validation.max_length = number_u64(value),
            "minItems" => spec.validation.min_items = number_u64(value),
            "maxItems" => spec.validation.max_items = number_u64(value),
            "uniqueItems" => spec.validation.unique_items = value.as_bool().unwrap_or(false),
            "minProperties" => spec.validation.min_properties = number_u64(value),
            "maxProperties" => spec.validation.max_properties = number_u64(value),
            "contentEncoding" => {
                spec.validation.content_encoding = value.as_str().map(ToOwned::to_owned)
            }
            "contentMediaType" => {
                spec.validation.content_media_type = value.as_str().map(ToOwned::to_owned)
            }
            "minimum" | "maximum" | "multipleOf" => {
                if let Some(number) = number_f64(value) {
                    match key {
                        "minimum" => spec.validation.numeric.minimum = Some(number),
                        "maximum" => spec.validation.numeric.maximum = Some(number),
                        _ => spec.validation.numeric.multiple_of = Some(number),
                    }
                } else {
                    spec.constraint_type_warning(path, key, value);
                }
            }
            "exclusiveMinimum" | "exclusiveMaximum" => {
                let maximum = key == "exclusiveMaximum";
                match value.as_bool() {
                    Some(flag) => {
                        if self.version.is_at_least_3_1() {
                            spec.node_diagnostics.push(Diagnostic {
                                severity: Severity::Warning,
                                path: path.key(key),
                                code: "constraint_type_mismatch",
                                message: "boolean exclusive bound is the OAS 3.0 form".to_owned(),
                            });
                        }
                        if flag {
                            if maximum {
                                spec.exclusive_max_flag = true;
                            } else {
                                spec.exclusive_min_flag = true;
                            }
                        }
                    }
                    None => match number_f64(value) {
                        Some(number) => {
                            if !self.version.is_at_least_3_1() {
                                spec.node_diagnostics.push(Diagnostic {
                                    severity: Severity::Warning,
                                    path: path.key(key),
                                    code: "constraint_type_mismatch",
                                    message: "numeric exclusive bound is the OAS 3.1 form"
                                        .to_owned(),
                                });
                            }
                            if maximum {
                                spec.validation.numeric.exclusive_maximum = Some(number);
                            } else {
                                spec.validation.numeric.exclusive_minimum = Some(number);
                            }
                        }
                        None => spec.constraint_type_warning(path, key, value),
                    },
                }
            }
            "contains" => {
                let child_path = path.key("contains");
                spec.validation.contains = Some(self.child_edge(
                    value,
                    doc,
                    &child_path,
                    child_depth,
                    EdgeContext::Container,
                ));
            }
            "minContains" => spec.validation.min_contains = number_u64(value),
            "maxContains" => spec.validation.max_contains = number_u64(value),
            "patternProperties" => {
                if let Some(patterns) = as_mapping(value) {
                    for (pattern_value, schema_value) in patterns {
                        let pattern = pattern_value
                            .as_str()
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| stringify_scalar(pattern_value));
                        let child_path = path.key("patternProperties").key(&pattern);
                        let edge = self.child_edge(
                            schema_value,
                            doc,
                            &child_path,
                            child_depth,
                            EdgeContext::Container,
                        );
                        spec.validation.pattern_properties.push((pattern, edge));
                    }
                } else {
                    spec.constraint_type_warning(path, key, value);
                }
            }
            "example" => match yaml_to_json(value) {
                Ok(json) => spec.validation.examples.push(json),
                Err(err) => spec.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("example"),
                    code: "example_unrepresentable",
                    message: format!("`example` is not representable as JSON: {err}"),
                }),
            },
            "examples" => spec.absorb_examples(value, path, self.version),
            "enum" => match value.as_sequence() {
                Some(items) => {
                    let mut values = Vec::with_capacity(items.len());
                    for item in items {
                        match yaml_to_json(item) {
                            Ok(json) => values.push(json),
                            Err(err) => spec.node_diagnostics.push(Diagnostic {
                                severity: Severity::Warning,
                                path: path.key("enum"),
                                code: "example_unrepresentable",
                                message: format!("enum constant skipped: {err}"),
                            }),
                        }
                    }
                    if values.is_empty() {
                        spec.node_diagnostics.push(Diagnostic {
                            severity: Severity::Warning,
                            path: path.key("enum"),
                            code: "composition_empty",
                            message: "`enum` lists no constants; treating as unconstrained"
                                .to_owned(),
                        });
                    } else {
                        spec.enum_values = Some(values);
                    }
                }
                None => spec.constraint_type_warning(path, key, value),
            },
            "const" => match yaml_to_json(value) {
                Ok(json) => {
                    spec.const_present = true;
                    spec.const_value = Some(json);
                }
                Err(err) => spec.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("const"),
                    code: "example_unrepresentable",
                    message: format!("`const` is not representable as JSON: {err}"),
                }),
            },
            "properties" => {
                if let Some(properties) = as_mapping(value) {
                    for (wire_value, schema_value) in properties {
                        let wire_name = wire_value
                            .as_str()
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| stringify_scalar(wire_value));
                        let child_path = path.key("properties").key(&wire_name);
                        let edge = self.child_edge(
                            schema_value,
                            doc,
                            &child_path,
                            child_depth,
                            EdgeContext::Property,
                        );
                        spec.properties.push((wire_name, edge));
                    }
                } else {
                    spec.constraint_type_warning(path, key, value);
                }
            }
            "required" => match value.as_sequence() {
                Some(names) => {
                    for name in names {
                        match name.as_str() {
                            Some(s) => spec.required_names.push(s.to_owned()),
                            None => spec.constraint_type_warning(path, "required", name),
                        }
                    }
                }
                None => spec.constraint_type_warning(path, key, value),
            },
            "additionalProperties" => match value {
                Yaml::Bool(false) => spec.additional = Some(AdditionalPropertiesPolicy::Deny),
                Yaml::Bool(true) => spec.additional = Some(AdditionalPropertiesPolicy::Ignore),
                Yaml::Mapping(_) => {
                    let child_path = path.key("additionalProperties");
                    let edge = self.child_edge(
                        value,
                        doc,
                        &child_path,
                        child_depth,
                        EdgeContext::Container,
                    );
                    spec.additional = Some(AdditionalPropertiesPolicy::Schema(edge));
                }
                other => {
                    spec.constraint_type_warning(path, key, other);
                    spec.additional = Some(AdditionalPropertiesPolicy::Ignore);
                }
            },
            "items" => match value {
                Yaml::Sequence(items) if self.version.is_at_least_3_1() => {
                    // 2020-12 array form of `items` ≡ prefixItems.
                    for (index, item) in items.iter().enumerate() {
                        let child_path = path.key("items").index(index);
                        let edge = self.child_edge(
                            item,
                            doc,
                            &child_path,
                            child_depth,
                            EdgeContext::Container,
                        );
                        spec.items_list.push(edge);
                    }
                    spec.items_was_list = true;
                }
                Yaml::Sequence(_) => {
                    spec.node_diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        path: path.key("items"),
                        code: "keyword_unsupported",
                        message: "array-form `items` is not valid in OAS 3.0".to_owned(),
                    });
                    spec.poisoned
                        .get_or_insert(UnsupportedReason::Other("array-form items in 3.0"));
                }
                _ => {
                    let child_path = path.key("items");
                    spec.items_edge = Some(self.child_edge(
                        value,
                        doc,
                        &child_path,
                        child_depth,
                        EdgeContext::Container,
                    ));
                }
            },
            "prefixItems" => {
                if let Some(items) = value.as_sequence() {
                    for (index, item) in items.iter().enumerate() {
                        let child_path = path.key("prefixItems").index(index);
                        let edge = self.child_edge(
                            item,
                            doc,
                            &child_path,
                            child_depth,
                            EdgeContext::Container,
                        );
                        spec.prefix_items.push(edge);
                    }
                } else {
                    spec.constraint_type_warning(path, key, value);
                }
            }
            "allOf" | "oneOf" | "anyOf" => {
                let keyword: &'static str = match key {
                    "oneOf" => "oneOf",
                    "anyOf" => "anyOf",
                    _ => "allOf",
                };
                match value.as_sequence() {
                    Some(members) if !members.is_empty() => {
                        let mut edges = Vec::with_capacity(members.len());
                        for (index, member) in members.iter().enumerate() {
                            let child_path = path.key(keyword).index(index);
                            edges.push(self.child_edge(
                                member,
                                doc,
                                &child_path,
                                child_depth,
                                EdgeContext::Composition,
                            ));
                        }
                        spec.compositions.push((keyword, edges));
                    }
                    Some(_) => spec.node_diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        path: path.key(key),
                        code: "composition_empty",
                        message: format!("`{key}` lists no members; ignoring"),
                    }),
                    None => spec.constraint_type_warning(path, key, value),
                }
            }
            "discriminator" => spec.discriminator = self.parse_discriminator(value, path),
            "$id" => {
                if value.as_str().is_some_and(|s| !s.is_empty()) {
                    spec.record_unsupported(
                        UnsupportedReason::AnchorRef,
                        path.key("$id"),
                        "`$id` base-URI rebasing is unsupported (D-§3)".to_owned(),
                    );
                }
            }
            "$anchor" | "$dynamicAnchor" | "$dynamicRef" => spec.record_unsupported(
                UnsupportedReason::AnchorRef,
                path.key(key),
                format!("`{key}` anchors are unsupported (D-§3)"),
            ),
            "unevaluatedProperties" => {
                if value == &Yaml::Bool(false) {
                    spec.unevaluated_props_false = true;
                } else {
                    spec.record_unsupported(
                        UnsupportedReason::UnevaluatedKeywordsActive,
                        path.key(key),
                        "`unevaluatedProperties` alters matching beyond the v1 model (D-§2)"
                            .to_owned(),
                    );
                }
            }
            "unevaluatedItems" => spec.record_unsupported(
                UnsupportedReason::UnevaluatedKeywordsActive,
                path.key(key),
                "`unevaluatedItems` alters matching beyond the v1 model (D-§2)".to_owned(),
            ),
            "if" | "then" | "else" | "not" | "dependentSchemas" | "dependentRequired" => {
                spec.record_unsupported(
                    UnsupportedReason::Other("conditional applicator keyword"),
                    path.key(key),
                    format!("`{key}` is recorded with a generation diagnostic (D-§2 bucket 3)"),
                );
            }
            // Recognized but not consumed by this package.
            "$schema" | "$comment" | "$vocabulary" | "$defs" | "definitions" | "xml"
            | "externalDocs" => {}
            _ => {
                if key.starts_with("x-") {
                    // Vendor extensions are ignored except where explicitly
                    // consumed (x-rust-stream-item at media-type level).
                } else {
                    spec.node_diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        path: path.key(key),
                        code: "keyword_unknown",
                        message: format!("unknown schema keyword `{key}` ignored"),
                    });
                }
            }
        }
    }

    fn parse_discriminator(
        &mut self,
        value: &Yaml,
        path: &DocumentPath,
    ) -> Option<DiscriminatorIr> {
        let Some(mapping) = as_mapping(value) else {
            self.note_warning(
                path.key("discriminator"),
                "discriminator_invalid",
                "`discriminator` must be a mapping",
            );
            return None;
        };
        let Some(property_name) = mapping_get(mapping, "propertyName").and_then(Yaml::as_str)
        else {
            self.note_warning(
                path.key("discriminator"),
                "discriminator_invalid",
                "`discriminator.propertyName` must be a string",
            );
            return None;
        };
        let mut mapping_entries = Vec::new();
        if let Some(entries) = mapping_get(mapping, "mapping").and_then(Yaml::as_mapping) {
            for (raw, target) in entries {
                let Some(name) = raw.as_str() else { continue };
                let Some(target_ref) = target.as_str() else {
                    self.note_warning(
                        path.key("discriminator").key("mapping").key(name),
                        "discriminator_invalid",
                        "discriminator mapping values must be strings",
                    );
                    continue;
                };
                mapping_entries.push((name.to_owned(), self.resolve_schema_ref_name(target_ref)));
            }
        }
        let explicit = !mapping_entries.is_empty();
        Some(DiscriminatorIr {
            property_name: property_name.to_owned(),
            mapping: mapping_entries,
            explicit,
        })
    }

    /// Maps a discriminator mapping URI onto the referenced schema's declared
    /// component name when it addresses `components/schemas`; otherwise the
    /// raw string is kept. External files are not force-loaded here.
    fn resolve_schema_ref_name(&self, target: &str) -> SchemaRefName {
        let parts = split_ref(target);
        if let Ok(tokens) = tokenize_pointer(&parts.fragment) {
            if tokens.len() == 3 && tokens[0] == "components" && tokens[1] == "schemas" {
                return SchemaRefName(tokens[2].clone());
            }
        }
        SchemaRefName(target.to_owned())
    }

    // ------------------------------------------------------------------
    // Entity parsing (parameters, bodies, responses, headers, servers)
    // ------------------------------------------------------------------

    /// Warns about ignored siblings around a `$ref` in a Reference-Object
    /// position (companion §3 cases a/b).
    fn note_entity_ref_siblings(&mut self, mapping: &Mapping, path: &DocumentPath) {
        let mut ignored: Vec<&str> = Vec::new();
        for (key_value, _) in mapping {
            let Some(key) = key_value.as_str() else {
                continue;
            };
            if key == "$ref" {
                continue;
            }
            if self.version.is_at_least_3_1() && matches!(key, "summary" | "description") {
                // Recognized reference metadata; carried wherever the entity
                // IR exposes a slot (see module documentation for the gap).
                continue;
            }
            ignored.push(key);
        }
        if !ignored.is_empty() {
            self.note_warning(
                path.clone(),
                "sibling_ignored",
                format!(
                    "siblings of a reference-object `$ref` are ignored: {}",
                    ignored.join(", ")
                ),
            );
        }
    }

    fn parse_servers(&mut self, value: &Yaml, path: &DocumentPath) -> Vec<ServerIr> {
        let Some(entries) = value.as_sequence() else {
            self.note_warning(path.clone(), "server_invalid", "`servers` must be an array");
            return Vec::new();
        };
        let mut servers = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let entry_path = path.index(index);
            let Some(mapping) = as_mapping(entry) else {
                self.note_error(
                    entry_path,
                    "server_invalid",
                    "server entry must be a mapping",
                );
                continue;
            };
            let Some(url) = mapping_get(mapping, "url").and_then(Yaml::as_str) else {
                self.note_error(
                    entry_path.key("url"),
                    "server_invalid",
                    "server entry requires a string `url`",
                );
                continue;
            };
            let mut variables = Vec::new();
            if let Some(variables_value) =
                mapping_get(mapping, "variables").and_then(Yaml::as_mapping)
            {
                for (name_value, var_value) in variables_value {
                    let name = name_value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| stringify_scalar(name_value));
                    let var_path = entry_path.key("variables").key(&name);
                    let Some(default) = string_field(var_value, "default") else {
                        self.note_error(
                            var_path.key("default"),
                            "server_variable_invalid",
                            format!("server variable `{name}` requires a string `default`"),
                        );
                        continue;
                    };
                    let allowed_enum = mapping_get_opt(var_value, "enum")
                        .and_then(Yaml::as_sequence)
                        .map(|items| {
                            items
                                .iter()
                                .map(|item| {
                                    item.as_str()
                                        .map_or_else(|| stringify_scalar(item), ToOwned::to_owned)
                                })
                                .collect()
                        });
                    variables.push((
                        name,
                        ServerVariable {
                            default: default.to_owned(),
                            allowed_enum,
                        },
                    ));
                }
            }
            servers.push(ServerIr {
                url: url.to_owned(),
                variables,
            });
        }
        servers
    }

    fn parse_parameter_list(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
    ) -> Vec<ParameterIr> {
        let Some(entries) = value.as_sequence() else {
            self.note_warning(
                path.clone(),
                "parameter_invalid",
                "`parameters` must be an array",
            );
            return Vec::new();
        };
        let mut parameters = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let entry_path = path.index(index);
            if let Some(parameter) =
                self.parse_parameter(entry, doc, &entry_path, 0, RefMeta::default())
            {
                parameters.push(parameter);
            }
        }
        parameters
    }

    fn parse_parameter(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
        mut meta: RefMeta,
    ) -> Option<ParameterIr> {
        if is_ref_mapping(value) {
            if depth >= MAX_ENTITY_REF_DEPTH {
                self.note_error(
                    path.clone(),
                    "ref_self_containment",
                    "parameter `$ref` chain exceeds the depth limit",
                );
                return None;
            }
            let mapping = as_mapping(value)?;
            self.note_entity_ref_siblings(mapping, path);
            meta.absorb(self.version, mapping);
            let reference = string_field(value, "$ref")?.to_owned();
            let target = self.resolve_entity_value(&reference, doc, path)?;
            return self.parse_parameter(&target, doc, path, depth + 1, meta);
        }
        let Some(mapping) = as_mapping(value) else {
            self.note_warning(
                path.clone(),
                "parameter_invalid",
                "parameter must be a mapping",
            );
            return None;
        };
        let Some(name) = string_field(value, "name") else {
            self.note_error(
                path.key("name"),
                "parameter_invalid",
                "parameter requires a string `name`",
            );
            return None;
        };
        let Some(location_word) = string_field(value, "in") else {
            self.note_error(
                path.key("in"),
                "parameter_invalid",
                "parameter requires `in`",
            );
            return None;
        };
        let location = match location_word {
            "path" => ParameterLocation::Path,
            "query" => ParameterLocation::Query,
            "header" => ParameterLocation::Header,
            "cookie" => ParameterLocation::Cookie,
            "querystring" => {
                self.note_error(
                    path.key("in"),
                    "parameter_location_querystring",
                    "`in: querystring` (OAS 3.2) is rejected in v1 (companion §6)",
                );
                return None;
            }
            other => {
                self.note_error(
                    path.key("in"),
                    "parameter_location_invalid",
                    format!("unknown parameter location `{other}`"),
                );
                return None;
            }
        };
        let required = bool_field(value, "required").unwrap_or(false);
        let schema = if let Some(schema_value) = mapping_get(mapping, "schema") {
            self.build_schema_id(schema_value, doc, &path.key("schema"), 0)
        } else if mapping_has(mapping, "content") {
            self.note_warning(
                path.clone(),
                "parameter_content_unsupported",
                "content-style parameters fall back to raw/value representation",
            );
            self.schema_fallback(UnsupportedReason::Other("content-style parameter"))
        } else {
            self.note_warning(
                path.clone(),
                "parameter_schema_missing",
                "parameter declares neither `schema` nor `content`; treated as unconstrained",
            );
            self.arena.intern(SchemaNode::new(SchemaKind::AnyValue))
        };
        let default_style = match location {
            ParameterLocation::Path | ParameterLocation::Header => ParameterStyle::Simple,
            ParameterLocation::Query | ParameterLocation::Cookie => ParameterStyle::Form,
        };
        let style = string_field(value, "style")
            .and_then(parse_style)
            .unwrap_or(default_style);
        let explode = bool_field(value, "explode").unwrap_or(style == ParameterStyle::Form);
        let allow_reserved = bool_field(value, "allowReserved").unwrap_or(false);
        let (summary, description) = meta.finish(value);
        Some(ParameterIr {
            name: name.to_owned(),
            location,
            required,
            schema,
            style,
            explode,
            allow_reserved,
            summary,
            description,
        })
    }

    fn parse_request_body(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
        mut meta: RefMeta,
    ) -> Option<RequestBodyIr> {
        if is_ref_mapping(value) {
            if depth >= MAX_ENTITY_REF_DEPTH {
                self.note_error(
                    path.clone(),
                    "ref_self_containment",
                    "requestBody `$ref` chain exceeds the depth limit",
                );
                return None;
            }
            let mapping = as_mapping(value)?;
            self.note_entity_ref_siblings(mapping, path);
            meta.absorb(self.version, mapping);
            let reference = string_field(value, "$ref")?.to_owned();
            let target = self.resolve_entity_value(&reference, doc, path)?;
            return self.parse_request_body(&target, doc, path, depth + 1, meta);
        }
        let Some(mapping) = as_mapping(value) else {
            self.note_warning(
                path.clone(),
                "schema_not_mapping",
                "requestBody must be a mapping",
            );
            return None;
        };
        let required = bool_field(value, "required").unwrap_or(false);
        let content = match mapping_get(mapping, "content").and_then(Yaml::as_mapping) {
            Some(content) => self.parse_content_map(content, doc, &path.key("content")),
            None => {
                self.note_warning(
                    path.key("content"),
                    "request_body_content_missing",
                    "requestBody declares no `content`; requests will have no body",
                );
                Vec::new()
            }
        };
        let (summary, description) = meta.finish(value);
        Some(RequestBodyIr {
            required,
            content,
            summary,
            description,
        })
    }

    fn parse_content_map(
        &mut self,
        content: &Mapping,
        doc: &str,
        path: &DocumentPath,
    ) -> Vec<ContentEntryIr> {
        let mut entries = Vec::with_capacity(content.len());
        for (media_value, media_object) in content {
            let media_type = media_value
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| stringify_scalar(media_value));
            let entry_path = path.key(&media_type);
            let (media_class, is_wildcard) = classify_media_type(&media_type);
            let schema = match mapping_get_opt(media_object, "schema") {
                Some(schema_value) => {
                    self.build_schema_id(schema_value, doc, &entry_path.key("schema"), 0)
                }
                None => self.arena.intern(SchemaNode::new(SchemaKind::AnyValue)),
            };
            let stream_item_override =
                mapping_get_opt(media_object, "x-rust-stream-item").map(|override_value| {
                    self.build_schema_id(
                        override_value,
                        doc,
                        &entry_path.key("x-rust-stream-item"),
                        0,
                    )
                });
            let encoding = parse_encoding_map(media_object);
            // §44 override: `x-rust-body: stream` forces the raw streaming
            // representation for a bounded textual entry (D-impl-x-rust-body-
            // stream). Any other value stays an ignored vendor extension.
            let stream_override = mapping_get_opt(media_object, "x-rust-body")
                .and_then(|value| value.as_str())
                .is_some_and(|value| value.trim() == "stream");
            entries.push(ContentEntryIr {
                media_type,
                media_class,
                is_wildcard,
                stream_override,
                schema,
                stream_item_override,
                encoding,
            });
        }
        entries
    }

    fn parse_header(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
        mut meta: RefMeta,
    ) -> HeaderSpecIr {
        if is_ref_mapping(value) {
            if depth < MAX_ENTITY_REF_DEPTH {
                if let Some(mapping) = as_mapping(value) {
                    self.note_entity_ref_siblings(mapping, path);
                    meta.absorb(self.version, mapping);
                    if let Some(reference) = string_field(value, "$ref") {
                        if let Some(target) = self.resolve_entity_value(reference, doc, path) {
                            return self.parse_header(&target, doc, path, depth + 1, meta);
                        }
                    }
                }
            } else {
                self.note_error(
                    path.clone(),
                    "ref_self_containment",
                    "header `$ref` chain exceeds the depth limit",
                );
            }
            return HeaderSpecIr {
                required: false,
                schema: self.schema_fallback(UnsupportedReason::Other("unresolvable header $ref")),
                summary: meta.summary,
                description: meta.description,
            };
        }
        let required = bool_field(value, "required").unwrap_or(false);
        let schema = if let Some(schema_value) = mapping_get_opt(value, "schema") {
            self.build_schema_id(schema_value, doc, &path.key("schema"), 0)
        } else if mapping_has_opt(value, "content") {
            self.note_warning(
                path.clone(),
                "header_content_unsupported",
                "content-style headers fall back to raw/value representation",
            );
            self.schema_fallback(UnsupportedReason::Other("content-style header"))
        } else {
            self.note_warning(
                path.clone(),
                "header_schema_missing",
                "header declares neither `schema` nor `content`; treated as unconstrained",
            );
            self.arena.intern(SchemaNode::new(SchemaKind::AnyValue))
        };
        let (summary, description) = meta.finish(value);
        HeaderSpecIr {
            required,
            schema,
            summary,
            description,
        }
    }

    fn parse_response_value(
        &mut self,
        value: &Yaml,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
        mut meta: RefMeta,
    ) -> Option<ResponseParts> {
        if is_ref_mapping(value) {
            if depth >= MAX_ENTITY_REF_DEPTH {
                self.note_error(
                    path.clone(),
                    "ref_self_containment",
                    "response `$ref` chain exceeds the depth limit",
                );
                return None;
            }
            let mapping = as_mapping(value)?;
            self.note_entity_ref_siblings(mapping, path);
            meta.absorb(self.version, mapping);
            let reference = string_field(value, "$ref")?.to_owned();
            let target = self.resolve_entity_value(&reference, doc, path)?;
            return self.parse_response_value(&target, doc, path, depth + 1, meta);
        }
        let mapping = as_mapping(value)?;
        if string_field(value, "description").is_none() {
            self.note_warning(
                path.key("description"),
                "response_description_missing",
                "response declares no `description` (required by OAS)",
            );
        }
        let mut headers = Vec::new();
        if let Some(headers_value) = mapping_get(mapping, "headers").and_then(Yaml::as_mapping) {
            for (header_value, header_object) in headers_value {
                let wire_name = header_value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| stringify_scalar(header_value));
                let header = self.parse_header(
                    header_object,
                    doc,
                    &path.key("headers").key(&wire_name),
                    depth,
                    RefMeta::default(),
                );
                headers.push((wire_name, header));
            }
        }
        let content = match mapping_get(mapping, "content").and_then(Yaml::as_mapping) {
            Some(content) => self.parse_content_map(content, doc, &path.key("content")),
            None => Vec::new(),
        };
        // Carried Reference-Object siblings win over the resolved Response
        // Object's own description (companion §3).
        let (summary, description) = meta.finish(value);
        Some((
            headers,
            content,
            RefMeta {
                summary,
                description,
            },
        ))
    }

    fn parse_responses(
        &mut self,
        responses: &Mapping,
        doc: &str,
        path: &DocumentPath,
    ) -> Vec<ResponseEntryIr> {
        let mut entries = Vec::with_capacity(responses.len());
        for (status_value, response_value) in responses {
            let status_text = status_value
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| stringify_scalar(status_value));
            let entry_path = path.key(&status_text);
            match parse_status_key(&status_text) {
                Ok(status) => {
                    if let Some((headers, content, meta)) = self.parse_response_value(
                        response_value,
                        doc,
                        &entry_path,
                        0,
                        RefMeta::default(),
                    ) {
                        entries.push(ResponseEntryIr {
                            status,
                            headers,
                            content,
                            summary: meta.summary,
                            description: meta.description,
                        });
                    }
                }
                Err(StatusKeyError::Informational) => {
                    self.note_error(
                        entry_path,
                        "response_status_informational",
                        format!(
                            "informational status `{status_text}` models a transport-layer event, \
                             not an operation outcome (main spec §35)"
                        ),
                    );
                }
                Err(StatusKeyError::Invalid) => {
                    self.note_error(
                        entry_path,
                        "response_status_invalid",
                        format!(
                            "`{status_text}` is not `default`, an explicit 2xx–5xx code, or a \
                             `2XX`/`3XX`/`4XX`/`5XX` range"
                        ),
                    );
                }
            }
        }
        entries
    }

    // ------------------------------------------------------------------
    // Paths and operations
    // ------------------------------------------------------------------

    fn parse_path_entry(
        &mut self,
        value: &Yaml,
        path_template: &str,
        doc: &str,
        path: &DocumentPath,
        depth: u32,
    ) -> Option<PathEntry> {
        if is_ref_mapping(value) {
            if depth >= MAX_ENTITY_REF_DEPTH {
                self.note_error(
                    path.clone(),
                    "ref_self_containment",
                    "pathItem `$ref` chain exceeds the depth limit",
                );
                return None;
            }
            let mapping = as_mapping(value)?;
            self.note_entity_ref_siblings(mapping, path);
            let reference = string_field(value, "$ref")?.to_owned();
            let target = self.resolve_entity_value(&reference, doc, path)?;
            return self.parse_path_entry(&target, path_template, doc, path, depth + 1);
        }
        let mapping = as_mapping(value)?;
        let parameters = mapping_get(mapping, "parameters")
            .map(|parameters| self.parse_parameter_list(parameters, doc, &path.key("parameters")))
            .unwrap_or_default();
        let servers = mapping_get(mapping, "servers")
            .map(|servers| self.parse_servers(servers, &path.key("servers")));
        let mut operations = Vec::new();
        for (method_value, operation_value) in mapping {
            let Some(word) = method_value.as_str() else {
                continue;
            };
            if let Some(method) = HttpMethod::from_keyword(word) {
                let operation = self.parse_operation(operation_value, doc, &path.key(word));
                operations.push((method, operation));
            } else if !matches!(
                word,
                "parameters" | "servers" | "summary" | "description" | "$ref" | "$comment"
            ) && !word.starts_with("x-")
            {
                self.note_warning(
                    path.key(word),
                    "unknown_field",
                    format!("unknown path-item key `{word}` ignored"),
                );
            }
        }
        Some(PathEntry {
            path: path_template.to_owned(),
            servers,
            parameters,
            operations,
        })
    }

    fn parse_operation(&mut self, value: &Yaml, doc: &str, path: &DocumentPath) -> OperationIr {
        let empty = OperationIr {
            operation_id: None,
            tags: Vec::new(),
            parameters: Vec::new(),
            request_body: None,
            responses: Vec::new(),
            servers: None,
            deprecated: false,
        };
        let Some(mapping) = as_mapping(value) else {
            self.note_warning(
                path.clone(),
                "schema_not_mapping",
                "operation must be a mapping",
            );
            return empty;
        };
        let operation_id = string_field(value, "operationId").map(ToOwned::to_owned);
        let deprecated = bool_field(value, "deprecated").unwrap_or(false);
        let tags = mapping_get(mapping, "tags")
            .and_then(Yaml::as_sequence)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let parameters = mapping_get(mapping, "parameters")
            .map(|parameters| self.parse_parameter_list(parameters, doc, &path.key("parameters")))
            .unwrap_or_default();
        let request_body = match mapping_get(mapping, "requestBody") {
            None | Some(Yaml::Null) => None,
            Some(request_value) => self.parse_request_body(
                request_value,
                doc,
                &path.key("requestBody"),
                0,
                RefMeta::default(),
            ),
        };
        let responses = match mapping_get(mapping, "responses").and_then(Yaml::as_mapping) {
            Some(responses) => self.parse_responses(responses, doc, &path.key("responses")),
            None => {
                self.note_warning(
                    path.key("responses"),
                    "operation_responses_missing",
                    "operation declares no `responses` (required by OAS)",
                );
                Vec::new()
            }
        };
        let servers = mapping_get(mapping, "servers")
            .map(|servers| self.parse_servers(servers, &path.key("servers")));
        OperationIr {
            operation_id,
            tags,
            parameters,
            request_body,
            responses,
            servers,
            deprecated,
        }
    }

    // ------------------------------------------------------------------
    // Top-level traversal
    // ------------------------------------------------------------------

    fn preintern_components(&mut self, root: &Yaml) -> BTreeMap<String, SchemaId> {
        let mut schemas = BTreeMap::new();
        let Some(component_schemas) = as_mapping(root)
            .and_then(|m| mapping_get(m, "components"))
            .and_then(Yaml::as_mapping)
            .and_then(|c| mapping_get(c, "schemas"))
            .and_then(Yaml::as_mapping)
        else {
            return schemas;
        };
        for (name_value, definition) in component_schemas {
            let Some(name) = name_value.as_str() else {
                self.note_warning(
                    DocumentPath::root().key("components").key("schemas"),
                    "component_name_invalid",
                    format!(
                        "component schema name must be a string, found `{}`",
                        stringify_scalar(name_value)
                    ),
                );
                continue;
            };
            let path = DocumentPath::root()
                .key("components")
                .key("schemas")
                .key(name);
            let tokens = vec![
                "components".to_owned(),
                "schemas".to_owned(),
                name.to_owned(),
            ];
            let pointer_key = canonical_pointer_key(&tokens);
            let root_doc = self.root_doc.clone();
            let id = self.intern_schema_at(&root_doc, pointer_key, definition, &path);
            schemas.insert(name.to_owned(), id);
        }
        schemas
    }

    fn run(&mut self) -> Result<IrDocument, Vec<Diagnostic>> {
        let root = self
            .files
            .get(&self.root_doc)
            .cloned()
            .unwrap_or(Yaml::Null);
        let schemas = self.preintern_components(&root);

        let root_servers_path = DocumentPath::root().key("servers");
        let servers = as_mapping(&root)
            .and_then(|m| mapping_get(m, "servers"))
            .map(|value| self.parse_servers(value, &root_servers_path))
            .unwrap_or_default();

        // `info.title` is carried verbatim for emitted-manifest package
        // naming (main spec §3.1); everything else under `info` is ignored.
        let info_title = as_mapping(&root)
            .and_then(|m| mapping_get(m, "info"))
            .and_then(as_mapping)
            .and_then(|info| mapping_get(info, "title"))
            .and_then(Yaml::as_str)
            .map(ToOwned::to_owned);

        let mut paths = Vec::new();
        if let Some(paths_mapping) = as_mapping(&root)
            .and_then(|m| mapping_get(m, "paths"))
            .and_then(Yaml::as_mapping)
        {
            let root_doc = self.root_doc.clone();
            for (template_value, item_value) in paths_mapping {
                let template = template_value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| stringify_scalar(template_value));
                let entry_path = DocumentPath::root().key("paths").key(&template);
                if let Some(entry) =
                    self.parse_path_entry(item_value, &template, &root_doc, &entry_path, 0)
                {
                    paths.push(entry);
                }
            }
        }

        let diags = std::mem::take(&mut self.diags);
        let document = IrDocument {
            version: self.version,
            raw_version: self.raw_version.clone(),
            info_title,
            servers,
            paths,
            schemas,
            arena: std::mem::take(&mut self.arena),
        };
        diags.into_result(document)
    }
}

// ----------------------------------------------------------------------
// Free functions and small helpers
// ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusKeyError {
    Informational,
    Invalid,
}

/// Parses a response status key (main spec §23, §24, §35): `default`,
/// explicit 2xx–5xx codes, or `2XX`/`3XX`/`4XX`/`5XX` ranges. Informational
/// keys/ranges are rejected per the §35 consistency rule.
fn parse_status_key(key: &str) -> Result<ResponseStatusKey, StatusKeyError> {
    if key == "default" {
        return Ok(ResponseStatusKey::Default);
    }
    if let Ok(code) = key.parse::<u16>() {
        return match code {
            100..=199 => Err(StatusKeyError::Informational),
            200..=599 => Ok(ResponseStatusKey::Explicit(code)),
            _ => Err(StatusKeyError::Invalid),
        };
    }
    let bytes = key.as_bytes();
    if bytes.len() == 3
        && matches!(bytes[0], b'1'..=b'5')
        && bytes[1].eq_ignore_ascii_case(&b'X')
        && bytes[2].eq_ignore_ascii_case(&b'X')
    {
        // Informational ranges model transport events, never operation
        // outcomes (main spec §35 consistency rule).
        if bytes[0] == b'1' {
            return Err(StatusKeyError::Informational);
        }
        let range = match bytes[0] {
            b'2' => RangeClass::Success2xx,
            b'3' => RangeClass::Redirection3xx,
            b'4' => RangeClass::ClientError4xx,
            _ => RangeClass::ServerError5xx,
        };
        return Ok(ResponseStatusKey::RangeClass(range));
    }
    Err(StatusKeyError::Invalid)
}

fn parse_style(word: &str) -> Option<ParameterStyle> {
    Some(match word {
        "matrix" => ParameterStyle::Matrix,
        "label" => ParameterStyle::Label,
        "form" => ParameterStyle::Form,
        "simple" => ParameterStyle::Simple,
        "spaceDelimited" => ParameterStyle::SpaceDelimited,
        "pipeDelimited" => ParameterStyle::PipeDelimited,
        "deepObject" => ParameterStyle::DeepObject,
        _ => return None,
    })
}

fn number_f64(value: &Yaml) -> Option<f64> {
    value.as_f64()
}

fn number_u64(value: &Yaml) -> Option<u64> {
    value.as_u64()
}

fn bool_field(value: &Yaml, key: &str) -> Option<bool> {
    mapping_get_opt(value, key).and_then(Yaml::as_bool)
}

fn mapping_get_opt<'a>(value: &'a Yaml, key: &str) -> Option<&'a Yaml> {
    as_mapping(value).and_then(|m| mapping_get(m, key))
}

/// Reduces a Media Type Object's `encoding` map to its per-property
/// `contentType` declarations (main spec §17): declaration order, only
/// string-valued entries kept. Every other encoding keyword (`headers`,
/// `style`, …) is a later-phase concern and silently skipped here.
fn parse_encoding_map(media_object: &Yaml) -> Vec<(String, String)> {
    let Some(encoding) = mapping_get_opt(media_object, "encoding").and_then(as_mapping) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (property, encoding_value) in encoding {
        let property = property
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| stringify_scalar(property));
        if let Some(content_type) = string_field(encoding_value, "contentType") {
            out.push((property, content_type.to_owned()));
        }
    }
    out
}

fn mapping_has_opt(value: &Yaml, key: &str) -> bool {
    as_mapping(value).is_some_and(|m| mapping_has(m, key))
}

/// Lexically removes `.` and resolves `..` segments before canonicalizing.
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ----------------------------------------------------------------------
// Working schema specification collected across one mapping pass
// ----------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TypeSpec {
    Single(String),
    List(Vec<String>),
}

#[derive(Debug, Default)]
struct SchemaSpec {
    type_spec: Option<TypeSpec>,
    nullable: bool,
    read_only: bool,
    write_only: bool,
    default: Option<serde_json::Value>,
    title: Option<String>,
    description: Option<String>,
    format: Option<String>,
    properties: Vec<(String, SchemaEdge)>,
    required_names: Vec<String>,
    additional: Option<AdditionalPropertiesPolicy>,
    items_edge: Option<SchemaEdge>,
    items_was_list: bool,
    items_list: Vec<SchemaEdge>,
    prefix_items: Vec<SchemaEdge>,
    enum_values: Option<Vec<serde_json::Value>>,
    const_present: bool,
    const_value: Option<serde_json::Value>,
    compositions: Vec<(&'static str, Vec<SchemaEdge>)>,
    discriminator: Option<DiscriminatorIr>,
    validation: ValidationMeta,
    exclusive_min_flag: bool,
    exclusive_max_flag: bool,
    unevaluated_props_false: bool,
    unsupported: Vec<UnsupportedReason>,
    poisoned: Option<UnsupportedReason>,
    node_diagnostics: Vec<Diagnostic>,
}

impl SchemaSpec {
    fn record_unsupported(
        &mut self,
        reason: UnsupportedReason,
        path: DocumentPath,
        message: String,
    ) {
        self.unsupported.push(reason);
        self.node_diagnostics.push(Diagnostic {
            severity: Severity::Error,
            path,
            code: "keyword_unsupported",
            message,
        });
    }

    fn constraint_type_warning(&mut self, path: &DocumentPath, key: &str, value: &Yaml) {
        self.node_diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            path: path.key(key),
            code: "constraint_type_mismatch",
            message: format!(
                "`{key}` expects a compatible scalar, found `{}`; ignored",
                stringify_scalar(value)
            ),
        });
    }

    /// Plural `examples`: OAS 3.0 stores a mapping of named samples; OAS 3.1+
    /// follows JSON Schema with an array of values, unwrapping Example
    /// Objects that carry their sample under `value`.
    fn absorb_examples(&mut self, value: &Yaml, path: &DocumentPath, version: OpenApiVersion) {
        let examples_path = path.key("examples");
        let push_sample = |spec: &mut Self, sample: &Yaml| match yaml_to_json(sample) {
            Ok(json) => spec.validation.examples.push(json),
            Err(err) => spec.node_diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                path: examples_path.clone(),
                code: "example_unrepresentable",
                message: format!("example value skipped: {err}"),
            }),
        };
        match (version, value) {
            (OpenApiVersion::V3_0, Yaml::Mapping(named)) => {
                for (_, sample) in named {
                    push_sample(self, sample);
                }
            }
            (_, Yaml::Sequence(items)) => {
                for item in items {
                    let inner = item
                        .as_mapping()
                        .filter(|m| mapping_has(m, "value"))
                        .and_then(|m| mapping_get(m, "value"))
                        .unwrap_or(item);
                    push_sample(self, inner);
                }
            }
            _ => self.node_diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                path: examples_path,
                code: "examples_shape",
                message: "`examples` must be a mapping of samples in OAS 3.0 or an array of \
                          values in OAS 3.1+; ignored"
                    .to_owned(),
            }),
        }
    }

    fn assemble(mut self, path: &DocumentPath) -> SchemaNode {
        // Boolean exclusivity modifiers fold onto the bounds they modify.
        if self.exclusive_min_flag {
            match self.validation.numeric.minimum.take() {
                Some(bound) => self.validation.numeric.exclusive_minimum = Some(bound),
                None => self.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("exclusiveMinimum"),
                    code: "constraint_type_mismatch",
                    message: "boolean `exclusiveMinimum: true` without `minimum`".to_owned(),
                }),
            }
        }
        if self.exclusive_max_flag {
            match self.validation.numeric.maximum.take() {
                Some(bound) => self.validation.numeric.exclusive_maximum = Some(bound),
                None => self.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("exclusiveMaximum"),
                    code: "constraint_type_mismatch",
                    message: "boolean `exclusiveMaximum: true` without `maximum`".to_owned(),
                }),
            }
        }

        // Type normalization: single string, legacy forms, or 3.1 array.
        let mut nullable = self.nullable;
        let mut declared_type: Option<String> = None;
        let mut legacy_binary = false;
        match self.type_spec.take() {
            Some(TypeSpec::Single(word)) => {
                if matches!(word.as_str(), "binary" | "file") {
                    declared_type = Some("string".to_owned());
                    legacy_binary = true;
                } else {
                    declared_type = Some(word);
                }
            }
            Some(TypeSpec::List(entries)) => {
                if entries.is_empty() {
                    self.node_diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        path: path.key("type"),
                        code: "type_array_invalid",
                        message: "`type` array is empty; treating as unconstrained".to_owned(),
                    });
                }
                let non_null: Vec<&String> = entries
                    .iter()
                    .filter(|entry| entry != &&"null".to_owned())
                    .collect();
                if entries.iter().any(|entry| entry == "null") {
                    nullable = true;
                }
                match non_null.len() {
                    0 => {}
                    1 => declared_type = Some(non_null[0].clone()),
                    _ => {
                        self.node_diagnostics.push(Diagnostic {
                            severity: Severity::Error,
                            path: path.key("type"),
                            code: "type_array_mixed",
                            message: format!(
                                "multi-type array [{}] cannot be represented; falling back to \
                                 raw/value (D-§2)",
                                entries.join(", ")
                            ),
                        });
                        self.poisoned
                            .get_or_insert(UnsupportedReason::MixedTypeArray);
                    }
                }
            }
            None => {}
        }

        // Standalone `unevaluatedProperties: false` behaves exactly like
        // `additionalProperties: false` (DECISIONS.md D-§2 bucket 3).
        if self.unevaluated_props_false
            && self.unsupported.is_empty()
            && self.compositions.is_empty()
        {
            self.additional
                .get_or_insert(AdditionalPropertiesPolicy::Deny);
        }

        let kind = if let Some(poisoned) = self.poisoned {
            SchemaKind::NotSupported { reason: poisoned }
        } else if let Some(reason) = self.unsupported.first().copied() {
            SchemaKind::NotSupported { reason }
        } else if self.compositions.len() > 1 {
            self.node_diagnostics.push(Diagnostic {
                severity: Severity::Error,
                path: path.clone(),
                code: "keyword_unsupported",
                message: "multiple composition keywords on one schema cannot be represented; \
                          falling back to raw/value"
                    .to_owned(),
            });
            SchemaKind::NotSupported {
                reason: UnsupportedReason::Other("combined composition keywords"),
            }
        } else if let Some((keyword, members)) = self.compositions.pop() {
            if declared_type.is_some() {
                self.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("type"),
                    code: "type_conflict_shape",
                    message: "`type` alongside a composition keyword is folded into the \
                              composed schema"
                        .to_owned(),
                });
            }
            match keyword {
                "oneOf" => SchemaKind::OneOf {
                    members,
                    discriminator: self.discriminator.take(),
                },
                "anyOf" => SchemaKind::AnyOf {
                    members,
                    discriminator: self.discriminator.take(),
                },
                _ => SchemaKind::AllOf {
                    members,
                    discriminator: self.discriminator.take(),
                },
            }
        } else if self.enum_values.is_some() || self.const_present {
            let mut values = self.enum_values.take().unwrap_or_default();
            if self.const_present {
                values.push(self.const_value.take().unwrap_or(serde_json::Value::Null));
            }
            if values.iter().any(serde_json::Value::is_null) {
                values.retain(|value| !value.is_null());
                nullable = true;
            }
            if values.is_empty() {
                SchemaKind::AnyValue
            } else if values.iter().all(|v| v.is_string()) {
                SchemaKind::Enum {
                    values: EnumValues::Strings(
                        values
                            .into_iter()
                            .map(|v| v.as_str().unwrap().to_owned())
                            .collect(),
                    ),
                }
            } else if values
                .iter()
                .all(|v| v.as_i64().is_some() || v.as_u64().is_some_and(|u| u <= i64::MAX as u64))
            {
                SchemaKind::Enum {
                    values: EnumValues::Integers(
                        values
                            .into_iter()
                            .map(|v| v.as_i64().unwrap_or_else(|| v.as_u64().unwrap() as i64))
                            .collect(),
                    ),
                }
            } else {
                SchemaKind::Enum {
                    values: EnumValues::MixedFallback(values),
                }
            }
        } else if !self.prefix_items.is_empty() || self.items_was_list {
            let prefix = if self.prefix_items.is_empty() {
                std::mem::take(&mut self.items_list)
            } else {
                std::mem::take(&mut self.prefix_items)
            };
            let postfix = if self.items_was_list {
                None
            } else {
                self.items_edge.take()
            };
            SchemaKind::Tuple {
                prefix_items: prefix,
                items: postfix,
            }
        } else if let Some(items) = self.items_edge.take() {
            SchemaKind::Array { items }
        } else if declared_type.as_deref() == Some("null") {
            nullable = true;
            SchemaKind::AnyValue
        } else {
            self.object_or_scalar_kind(declared_type, legacy_binary, path)
        };

        self.validation.format = self.format.clone();

        let description = match (self.title.take(), self.description.take()) {
            (Some(title), Some(description)) => Some(format!("{title}\n\n{description}")),
            (Some(title), None) => Some(title),
            (None, description) => description,
        };

        if self.read_only && self.write_only {
            self.node_diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                path: path.clone(),
                code: "keyword_unknown",
                message: "schema declares both readOnly and writeOnly".to_owned(),
            });
        }

        SchemaNode {
            kind,
            nullable,
            read_only: self.read_only,
            write_only: self.write_only,
            default: self.default,
            validation: self.validation,
            description,
            diagnostics: self.node_diagnostics,
        }
    }

    fn object_or_scalar_kind(
        &mut self,
        declared_type: Option<String>,
        legacy_binary: bool,
        path: &DocumentPath,
    ) -> SchemaKind {
        let shape_object = !self.properties.is_empty()
            || self.additional.is_some()
            || self.validation.min_properties.is_some()
            || self.validation.max_properties.is_some()
            || !self.validation.pattern_properties.is_empty()
            || !self.required_names.is_empty();
        let required = self.required_names.clone();
        match declared_type.as_deref() {
            Some("object") if !shape_object => SchemaKind::FreeFormObject,
            Some("object") | None if shape_object => {
                let mut properties = Vec::with_capacity(self.properties.len());
                for (wire_name, schema) in std::mem::take(&mut self.properties) {
                    properties.push(PropertyIr {
                        required: required.iter().any(|name| name == &wire_name),
                        wire_name,
                        schema,
                    });
                }
                SchemaKind::Object {
                    properties,
                    additional: self
                        .additional
                        .take()
                        .unwrap_or(AdditionalPropertiesPolicy::Ignore),
                }
            }
            Some(_) if shape_object => {
                // Contradictory declaration (`type: string` + `properties`);
                // the object shape wins with a warning.
                self.node_diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    path: path.key("type"),
                    code: "type_conflict_shape",
                    message: format!(
                        "`type: {}` conflicts with object-shape keywords; treating as object",
                        declared_type.unwrap_or_default()
                    ),
                });
                let mut properties = Vec::with_capacity(self.properties.len());
                for (wire_name, schema) in std::mem::take(&mut self.properties) {
                    properties.push(PropertyIr {
                        required: required.iter().any(|name| name == &wire_name),
                        wire_name,
                        schema,
                    });
                }
                SchemaKind::Object {
                    properties,
                    additional: self
                        .additional
                        .take()
                        .unwrap_or(AdditionalPropertiesPolicy::Ignore),
                }
            }
            Some("string") => SchemaKind::String_ {
                binary: legacy_binary || self.format.as_deref() == Some("binary"),
                format: self.format.clone(),
            },
            Some("integer") => SchemaKind::Integer {
                format: self.format.clone(),
            },
            Some("number") => SchemaKind::Number {
                format: self.format.clone(),
            },
            Some("boolean") => SchemaKind::Boolean,
            _ => SchemaKind::AnyValue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_keys_parse_per_main_spec_35() {
        assert_eq!(parse_status_key("default"), Ok(ResponseStatusKey::Default));
        assert_eq!(
            parse_status_key("200"),
            Ok(ResponseStatusKey::Explicit(200))
        );
        assert_eq!(
            parse_status_key("204"),
            Ok(ResponseStatusKey::Explicit(204))
        );
        assert_eq!(
            parse_status_key("404"),
            Ok(ResponseStatusKey::Explicit(404))
        );
        assert_eq!(
            parse_status_key("599"),
            Ok(ResponseStatusKey::Explicit(599))
        );
        assert_eq!(
            parse_status_key("2XX"),
            Ok(ResponseStatusKey::RangeClass(RangeClass::Success2xx))
        );
        assert_eq!(
            parse_status_key("3xx"),
            Ok(ResponseStatusKey::RangeClass(RangeClass::Redirection3xx))
        );
        assert_eq!(
            parse_status_key("4XX"),
            Ok(ResponseStatusKey::RangeClass(RangeClass::ClientError4xx))
        );
        assert_eq!(
            parse_status_key("5XX"),
            Ok(ResponseStatusKey::RangeClass(RangeClass::ServerError5xx))
        );

        assert_eq!(parse_status_key("150"), Err(StatusKeyError::Informational));
        assert_eq!(parse_status_key("100"), Err(StatusKeyError::Informational));
        assert_eq!(parse_status_key("1XX"), Err(StatusKeyError::Informational));
        assert_eq!(parse_status_key("99"), Err(StatusKeyError::Invalid));
        assert_eq!(parse_status_key("600"), Err(StatusKeyError::Invalid));
        assert_eq!(parse_status_key("6XX"), Err(StatusKeyError::Invalid));
        assert_eq!(parse_status_key("2X"), Err(StatusKeyError::Invalid));
        assert_eq!(parse_status_key("ok"), Err(StatusKeyError::Invalid));
        assert_eq!(parse_status_key("-1"), Err(StatusKeyError::Invalid));
    }

    #[test]
    fn styles_parse_camel_case_keywords() {
        assert_eq!(parse_style("deepObject"), Some(ParameterStyle::DeepObject));
        assert_eq!(
            parse_style("spaceDelimited"),
            Some(ParameterStyle::SpaceDelimited)
        );
        assert_eq!(
            parse_style("pipeDelimited"),
            Some(ParameterStyle::PipeDelimited)
        );
        assert_eq!(parse_style("Simple"), None);
        assert_eq!(parse_style(""), None);
    }

    #[test]
    fn lexical_normalization_removes_dot_segments() {
        let base = Path::new("/docs/api");
        let joined = normalize_lexical(&base.join("../common/types.yaml"));
        assert_eq!(joined, PathBuf::from("/docs/common/types.yaml"));
        let dots = normalize_lexical(Path::new("./a/./b/../c.yaml"));
        assert_eq!(dots, PathBuf::from("a/c.yaml"));
    }
}
