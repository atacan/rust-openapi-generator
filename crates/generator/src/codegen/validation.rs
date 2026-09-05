//! Runtime-validation planning shared by the models and server emitters
//! (companion §9, DECISIONS.md D-impl-runtime-validation-timing Phase 2
//! half).
//!
//! [`Analysis`] answers two questions deterministically for every arena
//! node:
//!
//! 1. does the resolved shape need an emitted `validate_request()` method
//!    ([`Analysis::has_validator`]) — true when any bucket-2 constraint in
//!    its own or a descendant's metadata survives normalization;
//! 2. is it a constrained scalar alias with a free validation function
//!    ([`Analysis::scalar_alias`]) — the `Slug` case.
//!
//! The same predicate drives router wiring: the server emitter consults
//! [`Analysis::named_has_validator`] through the assigned type names so both
//! emitters agree on exactly which types carry validators (byte-for-byte
//! deterministic output, main spec §50 test 39).

use std::collections::BTreeMap;

use crate::ir::schema::{NumericValidation, SchemaId, SchemaKind, ValidationMeta};
use crate::normalize::composition::{IntersectedScalar, ResolvedKind};
use crate::normalize::naming::{self, NameStyle};
use crate::normalize::{NormalizedDocument, NormalizedSchema};

/// Parameter kind of a constrained scalar alias's free validator function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarParamKind {
    /// `&str` parameter validated with string constraints + formats.
    Str,
    /// Integer target (`i32`/`i64`); bounds ride through `f64`.
    Int,
    /// `f64` target.
    Float,
}

/// Free validator function of one constrained scalar alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScalarAlias {
    /// `validate_<snake>_request`, derived from the component's Rust name.
    pub fn_name: String,
    /// Rust type name of the alias (`Slug`), for doc comments and lookups.
    pub type_name: String,
    pub kind: ScalarParamKind,
}

/// Per-document runtime-validation facts (memoized, declaration-order
/// deterministic).
#[derive(Debug)]
pub(crate) struct Analysis {
    /// Arena-indexed verdicts after alias chasing.
    has_validator: Vec<Verdict>,
    scalar_aliases: BTreeMap<u32, ScalarAlias>,
}

/// The v1 recognized string formats (`validate_format_string` enforces
/// exactly this set; unknown names stay metadata-only, documented).
/// Single source of truth shared with the models emitter's
/// `known_format` so the "does this branch need validating?" verdict and
/// the "should this type get an impl?" emission always agree (issue #8:
/// an unrecognized `format` such as `regex` must not mark a shape as
/// validated when no check will be emitted for it).
pub(crate) fn is_recognized_format(format: Option<&str>) -> bool {
    matches!(
        format,
        Some("date-time" | "date" | "time" | "email" | "hostname" | "uri" | "uuid")
    )
}

/// True when the metadata carries at least one constraint this phase
/// enforces at runtime on server requests (D-§2 bucket 2 minus the
/// documented v1 skips: patternProperties/contentEncoding/contentMediaType/
/// examples stay metadata-only).
///
/// `format` counts ONLY when it names a recognized string format (see
/// [`is_recognized_format`]) on a string shape (`validate_format_string`);
/// integer/number formats such as `int32` are type-shaping — already
/// enforced structurally at Serde decode — and unrecognized string formats
/// emit no check, so neither must ever make a node LOOK validated when the
/// models emitter will not emit checks for it (issue #8).
///
/// Takes an explicit verdict on whether the declared `format` produces a
/// runtime check in this shape context.
pub(crate) fn enforceable_with(format_checked: bool, validation: &ValidationMeta) -> bool {
    let numeric: &NumericValidation = &validation.numeric;
    validation.pattern.is_some()
        || validation.min_length.is_some()
        || validation.max_length.is_some()
        || *numeric != NumericValidation::default()
        || validation.min_items.is_some()
        || validation.max_items.is_some()
        || validation.unique_items
        || validation.min_properties.is_some()
        || validation.max_properties.is_some()
        || validation.contains.is_some()
        || (format_checked && is_recognized_format(validation.format.as_deref()))
}

/// Whether `format` enforces at runtime for one resolved shape: string shapes
/// only (companion §9 v1 policy; see [`enforceable_with`]).
fn format_checked(doc: &NormalizedDocument, effective: SchemaId) -> bool {
    match doc.resolution(effective).kind.clone() {
        ResolvedKind::IntersectedScalar(scalar) => {
            matches!(scalar.base_kind, SchemaKind::String_ { binary: false, .. })
        }
        ResolvedKind::Plain => matches!(
            doc.arena.get(effective).kind,
            SchemaKind::String_ { binary: false, .. }
        ),
        _ => false,
    }
}

/// Node-kind-aware [`enforceable`] used everywhere a resolved id is at hand.
pub(crate) fn node_enforceable(doc: &NormalizedDocument, effective: SchemaId) -> bool {
    enforceable_with(
        format_checked(doc, effective),
        &doc.resolution(effective).validation,
    )
}

/// Computes the analysis once per document; emitters share the verdicts.
#[must_use]
pub(crate) fn analyze(doc: &NormalizedDocument) -> Analysis {
    let mut state = Vec::with_capacity(doc.resolutions.len());
    for _ in 0..doc.resolutions.len() {
        state.push(Verdict::Unknown);
    }

    let mut scalar_aliases = BTreeMap::new();
    for schema in doc.schemas.values() {
        if let Some(kind) = scalar_alias_kind(doc, schema) {
            let fn_name = format!("validate_{}_request", snake_of(&schema.rust_type));
            scalar_aliases.insert(
                doc.resolve_alias(schema.source).0,
                ScalarAlias {
                    fn_name,
                    type_name: schema.rust_type.clone(),
                    kind,
                },
            );
        }
    }
    let mut analysis = Analysis {
        has_validator: state,
        scalar_aliases,
    };
    for id in 0..doc.resolutions.len() {
        let effective = doc.resolve_alias(SchemaId(id as u32));
        let _ = compute(doc, &mut analysis.has_validator, effective);
    }
    analysis
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Unknown,
    InProgress,
    Yes,
    No,
}

fn compute(doc: &NormalizedDocument, memo: &mut [Verdict], effective: SchemaId) -> bool {
    match memo[effective.0 as usize] {
        Verdict::Yes => return true,
        Verdict::No => return false,
        Verdict::InProgress => return false,
        Verdict::Unknown => {}
    }
    memo[effective.0 as usize] = Verdict::InProgress;
    let verdict = node_verdict(doc, effective, memo);
    memo[effective.0 as usize] = if verdict { Verdict::Yes } else { Verdict::No };
    verdict
}

/// Cycle-aware fixpoint: a node needs a validator when its own metadata is
/// enforceable, any child subtree carries checks, or any child is itself a
/// validated composite. The `InProgress → false` answer is the conservative
/// reading of a constraint-free cycle (nothing in it enforces anything).
fn node_verdict(doc: &NormalizedDocument, effective: SchemaId, memo: &mut [Verdict]) -> bool {
    if node_enforceable(doc, effective) {
        return true;
    }
    match doc.resolution(effective).kind.clone() {
        ResolvedKind::MergedObject(merged) => merged
            .properties
            .iter()
            .any(|property| property_checks(doc, property.schema.target, memo)),
        ResolvedKind::ClosedEnum(choice) => choice
            .branches
            .iter()
            .any(|branch| branch_checks(doc, branch.target, memo)),
        ResolvedKind::Plain => match doc.arena.get(effective).kind.clone() {
            SchemaKind::Object { properties, .. } => properties
                .iter()
                .any(|property| property_checks(doc, property.schema.target, memo)),
            _ => false,
        },
        ResolvedKind::IntersectedScalar(_) | ResolvedKind::RawValueFallback(_) => false,
        ResolvedKind::Alias(_) => unreachable!("aliases chased before verdicts"),
    }
}

/// One object property: its own constraints, its subtree's constraints
/// (arrays of constrained scalars), or a referenced composite's validator.
fn property_checks(doc: &NormalizedDocument, target: SchemaId, memo: &mut [Verdict]) -> bool {
    let effective = doc.resolve_alias(target);
    if node_enforceable(doc, effective) {
        return true;
    }
    match doc.resolution(effective).kind.clone() {
        ResolvedKind::IntersectedScalar(scalar) => scalar_subtree(doc, &scalar, memo),
        ResolvedKind::MergedObject(_) | ResolvedKind::ClosedEnum(_) => {
            compute(doc, memo, effective)
        }
        ResolvedKind::RawValueFallback(_) => false,
        ResolvedKind::Alias(_) => unreachable!("aliases chased before verdicts"),
        ResolvedKind::Plain => match doc.arena.get(effective).kind.clone() {
            SchemaKind::Array { items } => subtree_edge(doc, items.target, memo),
            SchemaKind::Tuple { prefix_items, .. } => prefix_items
                .iter()
                .any(|edge| subtree_edge(doc, edge.target, memo)),
            SchemaKind::Object { .. } => compute(doc, memo, effective),
            SchemaKind::Enum { .. } => false,
            _ => false,
        },
    }
}

fn scalar_subtree(
    doc: &NormalizedDocument,
    scalar: &IntersectedScalar,
    memo: &mut [Verdict],
) -> bool {
    match scalar.base_kind {
        SchemaKind::Array { items } => subtree_edge(doc, items.target, memo),
        SchemaKind::Object { ref properties, .. } => properties
            .iter()
            .any(|property| property_checks(doc, property.schema.target, memo)),
        _ => false,
    }
}

fn branch_checks(doc: &NormalizedDocument, target: SchemaId, memo: &mut [Verdict]) -> bool {
    let effective = doc.resolve_alias(target);
    node_enforceable(doc, effective)
        || property_checks(doc, target, memo)
        || compute(doc, memo, effective)
}

fn subtree_edge(doc: &NormalizedDocument, target: SchemaId, memo: &mut [Verdict]) -> bool {
    property_checks(doc, target, memo)
}

/// Scalar-alias classification for named components: a resolved scalar shape
/// (plain or intersected) carrying at least one enforceable constraint.
fn scalar_alias_kind(
    doc: &NormalizedDocument,
    schema: &NormalizedSchema,
) -> Option<ScalarParamKind> {
    let effective = doc.resolve_alias(schema.source);
    let base = match doc.resolution(effective).kind.clone() {
        ResolvedKind::IntersectedScalar(scalar) => Some(scalar.base_kind),
        ResolvedKind::Plain => Some(doc.arena.get(effective).kind.clone()),
        _ => None,
    }?;
    let format_checked = matches!(base, SchemaKind::String_ { binary: false, .. });
    if !enforceable_with(format_checked, &doc.resolution(effective).validation) {
        return None;
    }
    let kind = match base {
        SchemaKind::String_ { binary: false, .. } => ScalarParamKind::Str,
        SchemaKind::Integer { .. } => ScalarParamKind::Int,
        SchemaKind::Number { .. } => ScalarParamKind::Float,
        _ => return None,
    };
    Some(kind)
}

fn snake_of(rust_type: &str) -> String {
    naming::ident(rust_type, NameStyle::Snake)
}

impl Analysis {
    /// True when the resolved shape at `id` emits a `validate_request`
    /// method (or, for scalar aliases, callers should use the free
    /// function instead).
    pub(crate) fn has_validator(&self, id: SchemaId) -> bool {
        self.has_validator[id.0 as usize] == Verdict::Yes
    }

    /// Constrained scalar alias at `id`, if the shape is one.
    pub(crate) fn scalar_alias(&self, id: SchemaId) -> Option<&ScalarAlias> {
        self.scalar_aliases.get(&id.0)
    }
}
