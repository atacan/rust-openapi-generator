//! Schema-level IR: the interned [`SchemaNode`] arena, edges, and keyword
//! metadata (companion §2–§5, DECISIONS.md D-§2 keyword buckets).
//!
//! All schemas discovered during loading — component schemas, inline
//! operation schemas, `$ref` targets — are interned into one
//! [`SchemaArena`] and addressed by [`SchemaId`]. Cycles are representable
//! because every edge is an id; the [`Indirection`] flag records where
//! heap indirection is required to break recursion (companion §3).

use serde_json::Value as JsonValue;

use crate::diagnostics::Diagnostic;

/// Address of a schema node inside the document's [`SchemaArena`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaId(pub u32);

/// Arena of all interned schema nodes for a document.
///
/// Interning order is deterministic: first encounter in document traversal
/// order (components/schemas declaration order first, then lazily resolved
/// references).
#[derive(Debug, Default)]
pub struct SchemaArena {
    nodes: Vec<SchemaNode>,
}

impl SchemaArena {
    /// Empty arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a completed node and returns its id.
    pub fn intern(&mut self, node: SchemaNode) -> SchemaId {
        let id =
            SchemaId(u32::try_from(self.nodes.len()).expect("schema arena exceeds u32::MAX nodes"));
        self.nodes.push(node);
        id
    }

    /// Number of interned nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when no nodes were interned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Node addressed by `id`.
    #[must_use]
    pub fn get(&self, id: SchemaId) -> &SchemaNode {
        &self.nodes[id.0 as usize]
    }

    /// Nodes in interning order.
    pub fn iter(&self) -> impl Iterator<Item = (SchemaId, &SchemaNode)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (SchemaId(i as u32), n))
    }

    /// Reserves a slot for an in-progress node so recursive references can
    /// point at its final id before construction finishes.
    pub(crate) fn reserve(&mut self) -> SchemaId {
        self.intern(SchemaNode {
            kind: SchemaKind::NotSupported {
                reason: UnsupportedReason::Other("placeholder"),
            },
            ..SchemaNode::default()
        })
    }

    /// Fills a previously reserved slot.
    pub(crate) fn complete(&mut self, id: SchemaId, node: SchemaNode) {
        self.nodes[id.0 as usize] = node;
    }
}

/// One schema in the arena.
///
/// `nullable` carries the value-nullability dimension; property presence is
/// recorded on [`PropertyIr::required`]. Together they form the companion
/// §2.1 presence/nullability matrix.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchemaNode {
    pub kind: SchemaKind,
    /// Value nullability: 3.0 `nullable: true` or 3.1 type array containing `"null"`.
    pub nullable: bool,
    pub read_only: bool,
    pub write_only: bool,
    pub default: Option<JsonValue>,
    /// Validation-only metadata (D-§2 bucket 2).
    pub validation: ValidationMeta,
    /// Carried for generated doc comments; `title` is prepended when present.
    pub description: Option<String>,
    /// Diagnostics recorded while building this node, in keyword order.
    ///
    /// These travel with the node into codegen (next work package); they do
    /// not fail loading on their own.
    pub diagnostics: Vec<Diagnostic>,
}

impl SchemaNode {
    /// Convenience constructor used by the loader.
    #[must_use]
    pub fn new(kind: SchemaKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }
}

/// Structural shape of a schema after version normalization.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum SchemaKind {
    /// `{}` / `true`: unconstrained value (D-§4.4 row 2).
    #[default]
    AnyValue,
    /// Unconstrained `type: object` → map representation (D-§4.4 row 1).
    FreeFormObject,
    Boolean,
    Integer {
        format: Option<String>,
    },
    Number {
        format: Option<String>,
    },
    /// String with optional format and binary payload marker
    /// (`format: binary`, deprecated 3.0 `type: binary` / `type: file`;
    /// companion §2 normalization table, main spec §5.3/§44).
    String_ {
        format: Option<String>,
        binary: bool,
    },
    Array {
        items: SchemaEdge,
    },
    /// 3.1 `prefixItems` (and 3.1 `items` written as an array), with the
    /// additional-items schema when present.
    Tuple {
        prefix_items: Vec<SchemaEdge>,
        items: Option<SchemaEdge>,
    },
    Object {
        properties: Vec<PropertyIr>,
        additional: AdditionalPropertiesPolicy,
    },
    /// `enum` / `const` constants (companion §4.3).
    Enum {
        values: EnumValues,
    },
    /// Resolved reference. In OAS 3.1+ schema positions sibling keywords are
    /// preserved as conjunction terms in `inline_constraints` (companion §3);
    /// in OAS 3.0 siblings are ignored with a warning instead.
    Ref {
        target: SchemaId,
        summary: Option<String>,
        description: Option<String>,
        inline_constraints: Vec<SchemaEdge>,
    },
    AllOf {
        members: Vec<SchemaEdge>,
        discriminator: Option<DiscriminatorIr>,
    },
    OneOf {
        members: Vec<SchemaEdge>,
        discriminator: Option<DiscriminatorIr>,
    },
    AnyOf {
        members: Vec<SchemaEdge>,
        discriminator: Option<DiscriminatorIr>,
    },
    /// Conservative fallback marker; codegen emits raw/value representations
    /// (DECISIONS.md D-impl-oneoffallback philosophy).
    NotSupported {
        reason: UnsupportedReason,
    },
}

/// Why a schema fell back to [`SchemaKind::NotSupported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    /// Multiple non-null entries in a 3.1 `type` array.
    MixedTypeArray,
    /// Active `unevaluatedProperties` / `unevaluatedItems` (D-§2 bucket 3).
    UnevaluatedKeywordsActive,
    /// `$anchor` / `$dynamicRef` / non-empty `$id` rebasing (D-§3).
    AnchorRef,
    /// Remote `http(s)` reference; fetching is never performed (D-§3).
    RemoteRefUnfetched,
    /// A cycle without an intervening container break (companion §3).
    UnbrokenSelfContainment,
    /// Inline nesting exceeded `LoadConfig::max_inline_depth`.
    InlineExpansionDepthExceeded,
    Other(&'static str),
}

/// Edge from one arena node to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaEdge {
    pub target: SchemaId,
    pub indirection: Indirection,
}

/// Heap-indirection policy for breaking recursion cycles (companion §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indirection {
    /// Recursion broken by arrays/maps or no recursion at all.
    None,
    /// Recursion through properties requires heap indirection (`Box<T>`).
    Boxed,
}

/// Object property; wire name preserved verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyIr {
    pub wire_name: String,
    pub schema: SchemaEdge,
    /// Presence dimension of the companion §2.1 matrix.
    pub required: bool,
}

/// `additionalProperties` policy (companion §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdditionalPropertiesPolicy {
    /// Explicit `false` → deny unknown fields.
    Deny,
    /// Absent or explicit `true` → unknown keys ignored (lossy model policy).
    Ignore,
    /// Schema-valued form.
    Schema(SchemaEdge),
}

/// Constant sets from `enum` / `const` (companion §4.3 proposed mapping).
#[derive(Debug, Clone, PartialEq)]
pub enum EnumValues {
    Strings(Vec<String>),
    Integers(Vec<i64>),
    MixedFallback(Vec<JsonValue>),
}

/// Discriminator metadata for inspect-select-validate routing
/// (companion §4.2 Decided).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscriminatorIr {
    pub property_name: String,
    /// Mapping values resolved to the referenced schema's declared component
    /// name when they address `components/schemas`; raw strings otherwise.
    pub mapping: Vec<(String, SchemaRefName)>,
    /// True when an explicit `mapping` object was present.
    pub explicit: bool,
}

/// The declared name of a referenced schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRefName(pub String);

/// Validation-only metadata retained per D-§2 bucket 2; enforced on server
/// requests per companion §9, lenient on client decode by default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationMeta {
    pub pattern: Option<String>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub numeric: NumericValidation,
    pub min_items: Option<u64>,
    pub max_items: Option<u64>,
    pub unique_items: bool,
    pub min_properties: Option<u64>,
    pub max_properties: Option<u64>,
    pub pattern_properties: Vec<(String, SchemaEdge)>,
    pub contains: Option<SchemaEdge>,
    pub min_contains: Option<u64>,
    pub max_contains: Option<u64>,
    pub content_encoding: Option<String>,
    pub content_media_type: Option<String>,
    /// Format kept as validation metadata unless the typed-format feature
    /// maps it to a concrete type later (companion §4.5). Also mirrored on
    /// typed kinds (`Integer { format }`, …) for that feature.
    pub format: Option<String>,
    /// Singular `example` plus plural `examples`, declaration order
    /// (companion §2 normalization table).
    pub examples: Vec<JsonValue>,
}

/// Numeric comparison constraints; boolean 3.0 exclusivity modifiers are
/// normalized onto the exclusive fields (companion §2 normalization table).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumericValidation {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub exclusive_minimum: Option<f64>,
    pub exclusive_maximum: Option<f64>,
    pub multiple_of: Option<f64>,
}
