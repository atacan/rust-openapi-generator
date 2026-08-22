//! Composition resolution: `allOf`/`oneOf`/`anyOf` → [`ResolvedKind`]
//! (companion §4, DECISIONS.md D-impl-oneoffallback).
//!
//! Resolution is depth-first over the arena with memoization keyed by arena
//! id; original [`SchemaKind`]s are never mutated — every node gains a
//! parallel [`ResolvedNode`] instead. `$ref` nodes without sibling
//! conjunction terms resolve as transparent aliases; OAS 3.1 sibling schema
//! keywords participate as additional allOf members (conjunction semantics,
//! companion §3).
//!
//! Verdicts:
//!
//! - **allOf**: all-object members merge field-wise (identical constraints
//!   collapse, `required` unions, conflicts are generation errors);
//!   compatible scalars intersect into one validated type; anything else
//!   falls back to raw/value with a Warning (default policy mirroring
//!   D-impl-oneoffallback). `serde(flatten)` is never used for non-object
//!   members.
//! - **oneOf/anyOf**: choose-one enums require static mutual-exclusivity
//!   proof (companion §4.2); otherwise raw/value fallback (Warning), or an
//!   Error for `oneOf` when configured. The discriminator is recorded but
//!   never changes a verdict (inspect-select-validate is decode routing
//!   only).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as JsonValue;

use crate::diagnostics::{Diagnostic, DocumentPath, Severity};
use crate::ir::schema::{
    AdditionalPropertiesPolicy, DiscriminatorIr, EnumValues, Indirection, NumericValidation,
    PropertyIr, SchemaArena, SchemaEdge, SchemaId, SchemaKind, ValidationMeta,
};

use super::NormalizeConfig;

/// Why a schema fell back to [`ResolvedKind::RawValueFallback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// Mixed or unrepresentable `allOf` members (companion §4.1 last bullet).
    UnrepresentableAllOf,
    /// Mutual exclusivity of `oneOf` branches could not be proven
    /// statically (companion §4.2).
    UnprovenOneOf,
    /// Mutual exclusivity of `anyOf` branches could not be proven;
    /// choose-one enums are forbidden here (companion §4.2 MUST NOT).
    UnprovenAnyOf,
}

impl FallbackReason {
    /// Stable label used by the deterministic dump.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::UnrepresentableAllOf => "unrepresentable-all-of",
            Self::UnprovenOneOf => "unproven-one-of",
            Self::UnprovenAnyOf => "unproven-any-of",
        }
    }
}

/// Final shape of one schema after composition resolution (a new enum
/// alongside [`SchemaKind`]; original kinds are preserved).
#[derive(Debug, Clone)]
pub enum ResolvedKind {
    /// No composition applies at this node; the original [`SchemaKind`]
    /// stands unchanged.
    Plain,
    /// `$ref` without sibling conjunction terms: transparent alias to the
    /// referenced node's resolution.
    Alias(SchemaId),
    /// allOf members merged field-wise into one object (companion §4.1).
    MergedObject(MergedObject),
    /// Compatible scalar/array members intersected onto ONE type carrying
    /// every validation check (main spec §50 test 51).
    IntersectedScalar(IntersectedScalar),
    /// Proven mutually exclusive branches → a choose-one enum is allowed.
    ClosedEnum(ClosedEnumChoice),
    /// Conservative raw/value fallback carrying retained validation
    /// metadata (D-impl-oneoffallback default policy).
    RawValueFallback(RawFallback),
}

/// Field-wise merged allOf object (companion §4.1): identical constraints
/// collapse, `required` unions, property order follows first appearance.
#[derive(Debug, Clone)]
pub struct MergedObject {
    pub properties: Vec<PropertyIr>,
    pub additional: AdditionalPropertiesPolicy,
}

/// Intersected compatible scalars: one unified base type; the combined
/// validation metadata rides on [`ResolvedNode::validation`].
#[derive(Debug, Clone)]
pub struct IntersectedScalar {
    /// Unified base type (`Boolean`, `Integer`, `Number`, `String_`,
    /// homogeneous `Enum`, or `Array` with identical item schemas).
    pub base_kind: SchemaKind,
}

/// Proven-exclusive branch set of a `oneOf`/`anyOf`.
#[derive(Debug, Clone)]
pub struct ClosedEnumChoice {
    /// Branch edges in declaration order.
    pub branches: Vec<SchemaEdge>,
    /// True only when wire-shape equivalence AND exclusivity are both
    /// provable for native serde internally-tagged collapse (companion
    /// §4.2 Decided); codegen makes the final call later.
    pub native_serde_candidate: bool,
}

/// Raw/value fallback record.
#[derive(Debug, Clone)]
pub struct RawFallback {
    pub reason: FallbackReason,
    /// Always false on fallback paths: proof of exclusivity would have made
    /// the node a [`ResolvedKind::ClosedEnum`] instead.
    pub native_serde_candidate: bool,
}

/// Resolution result for one arena node.
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub kind: ResolvedKind,
    /// Value nullability after resolution: unconstrained schemas count as
    /// nullable; typed shapes carry 3.0 `nullable` / 3.1 `"null"` typing
    /// (companion §2 normalization table). allOf intersects (all members
    /// must admit null).
    pub nullable: bool,
    /// Validation metadata retained through composition (D-§2 bucket 2).
    pub validation: ValidationMeta,
    /// Discriminator metadata, recorded either way (routing hint only,
    /// companion §4.2).
    pub discriminator: Option<DiscriminatorIr>,
    /// Diagnostics produced while resolving THIS node, in member order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Top-level JSON type of a branch, used by proof standard (c)
/// (companion §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonType {
    Object,
    String,
    Integer,
    Number,
    Boolean,
    Array,
}

fn json_type_of_kind(kind: &SchemaKind) -> Option<JsonType> {
    Some(match kind {
        SchemaKind::FreeFormObject | SchemaKind::Object { .. } => JsonType::Object,
        SchemaKind::String_ { .. } => JsonType::String,
        SchemaKind::Integer { .. } => JsonType::Integer,
        SchemaKind::Number { .. } => JsonType::Number,
        SchemaKind::Boolean => JsonType::Boolean,
        SchemaKind::Array { .. } | SchemaKind::Tuple { .. } => JsonType::Array,
        SchemaKind::Enum { values } => match values {
            EnumValues::Strings(_) => JsonType::String,
            EnumValues::Integers(_) => JsonType::Integer,
            EnumValues::MixedFallback(_) => return None,
        },
        SchemaKind::AnyValue
        | SchemaKind::Ref { .. }
        | SchemaKind::AllOf { .. }
        | SchemaKind::OneOf { .. }
        | SchemaKind::AnyOf { .. }
        | SchemaKind::NotSupported { .. } => return None,
    })
}

/// Outcome of resolving one composed node.
enum CompositionOutcome {
    /// A plain/alias verdict whose metadata comes from the original node.
    Passthrough(ResolvedKind),
    /// A fully computed resolved node (boxed to keep the enum small).
    Computed(Box<ResolvedNode>),
}

/// Depth-first resolver over one document arena. Slots are index-aligned
/// with the arena: `slots[id]` holds the resolution of arena node `id`.
pub(crate) struct Resolver<'a> {
    arena: &'a SchemaArena,
    arena_len: usize,
    slots: Vec<Option<ResolvedNode>>,
    in_progress: BTreeSet<u32>,
    /// Declared component names by arena id (used by discriminator proof a).
    component_names: BTreeMap<u32, String>,
    pub(crate) diags: Vec<Diagnostic>,
    config: NormalizeConfig,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(arena: &'a SchemaArena, config: NormalizeConfig) -> Self {
        let arena_len = arena.iter().count();
        Self {
            arena,
            arena_len,
            slots: Vec::new(),
            in_progress: BTreeSet::new(),
            component_names: BTreeMap::new(),
            diags: Vec::new(),
            config,
        }
    }

    /// Registers declared component names for discriminator proof (a).
    pub(crate) fn set_component_names(&mut self, names: BTreeMap<u32, String>) {
        self.component_names = names;
    }

    fn slot_mut(&mut self, id: SchemaId) -> &mut Option<ResolvedNode> {
        let needed = id.0 as usize + 1;
        if self.slots.len() < needed {
            self.slots.resize_with(needed, || None);
        }
        &mut self.slots[id.0 as usize]
    }

    pub(crate) fn slot(&self, id: SchemaId) -> Option<&ResolvedNode> {
        self.slots.get(id.0 as usize).and_then(|s| s.as_ref())
    }

    /// Resolves the roots first (so diagnostics use meaningful document
    /// paths), then sweeps any remaining nodes in id order.
    pub(crate) fn resolve_all(&mut self, roots: &[(SchemaId, DocumentPath)]) {
        for (id, crumbs) in roots {
            self.resolve(*id, crumbs);
        }
        for index in 0..self.arena_len {
            let crumbs = DocumentPath::root().key("arena").key(index.to_string());
            self.resolve(SchemaId(index as u32), &crumbs);
        }
    }

    /// Consumes the resolver into per-arena-id resolutions
    /// (index-aligned with the arena) plus collected diagnostics in
    /// resolution order.
    pub(crate) fn finish(self) -> (Vec<ResolvedNode>, Vec<Diagnostic>) {
        let slots = self
            .slots
            .into_iter()
            .map(|slot| slot.expect("resolve_all leaves no slot empty"))
            .collect();
        (slots, self.diags)
    }

    /// Memoizing depth-first resolution of one node.
    pub(crate) fn resolve(&mut self, id: SchemaId, crumbs: &DocumentPath) {
        if self.slot(id).is_some() || self.in_progress.contains(&id.0) {
            return;
        }
        self.in_progress.insert(id.0);
        let node = self.arena.get(id);
        let outcome = match &node.kind {
            SchemaKind::Ref {
                target,
                inline_constraints,
                ..
            } if inline_constraints.is_empty() => {
                self.resolve(*target, crumbs);
                CompositionOutcome::Passthrough(ResolvedKind::Alias(*target))
            }
            SchemaKind::Ref {
                target,
                inline_constraints,
                ..
            } => {
                // OAS 3.1 schema-position `$ref` with sibling keywords:
                // conjunction semantics — target plus terms act as extra
                // allOf members (companion §3 case c).
                let mut members = vec![SchemaEdge {
                    target: *target,
                    indirection: Indirection::None,
                }];
                members.extend(inline_constraints.iter().copied());
                self.resolve_all_of_like(members, None, &crumbs.key("$ref-conjunction"))
            }
            SchemaKind::AllOf {
                members,
                discriminator,
            } => self.resolve_all_of_like(members.clone(), discriminator.clone(), crumbs),
            SchemaKind::OneOf {
                members,
                discriminator,
            } => self.resolve_choice(
                ChoiceKeyword::OneOf,
                members.clone(),
                discriminator.clone(),
                crumbs,
            ),
            SchemaKind::AnyOf {
                members,
                discriminator,
            } => self.resolve_choice(
                ChoiceKeyword::AnyOf,
                members.clone(),
                discriminator.clone(),
                crumbs,
            ),
            _ => CompositionOutcome::Passthrough(ResolvedKind::Plain),
        };

        let resolved = match outcome {
            CompositionOutcome::Passthrough(kind) => {
                let nullable = match &kind {
                    ResolvedKind::Alias(target) => self.allows_null(*target),
                    _ => matches!(node.kind, SchemaKind::AnyValue) || node.nullable,
                };
                ResolvedNode {
                    kind,
                    nullable,
                    validation: node.validation.clone(),
                    discriminator: node_discriminator(&node.kind),
                    diagnostics: Vec::new(),
                }
            }
            CompositionOutcome::Computed(resolved) => *resolved,
        };
        *self.slot_mut(id) = Some(resolved);
        self.in_progress.remove(&id.0);
    }

    fn allows_null(&self, id: SchemaId) -> bool {
        match self.slot(id) {
            Some(resolved) => resolved.nullable,
            None => false,
        }
    }

    /// Chases alias chains to the effective node, collecting sibling
    /// conjunction terms from any `$ref` nodes along the way (companion §3:
    /// OAS 3.1 schema-position siblings are evaluated together with the
    /// referenced schema). Alias cycles cannot survive loading
    /// (`UnbrokenSelfContainment`), but the guard keeps this total.
    fn chase(&self, start: SchemaId) -> (SchemaId, Vec<SchemaEdge>) {
        let mut current = start;
        let mut terms: Vec<SchemaEdge> = Vec::new();
        let mut guard = 0_usize;
        loop {
            guard += 1;
            if guard > self.arena_len {
                return (current, terms);
            }
            match &self.arena.get(current).kind {
                SchemaKind::Ref {
                    target,
                    inline_constraints,
                    ..
                } => {
                    terms.extend(inline_constraints.iter().copied());
                    current = *target;
                }
                _ => return (current, terms),
            }
        }
    }

    fn chase_id(&self, start: SchemaId) -> SchemaId {
        self.chase(start).0
    }
}

/// Folds a conjunction term's validation metadata into a classified
/// member (companion §3 case c). Validation-only terms never change the
/// member's type family; irreconcilable folds degrade to [`MemberClass::Opaque`].
fn fold_term_validation(class: MemberClass, term: &crate::ir::schema::SchemaNode) -> MemberClass {
    match class {
        MemberClass::Object {
            properties,
            additional,
            validation,
            nullable,
        } => match intersect_validation(&validation, &term.validation) {
            Ok(merged) => MemberClass::Object {
                properties,
                additional,
                validation: merged,
                nullable: nullable && (matches!(term.kind, SchemaKind::AnyValue) || term.nullable),
            },
            Err(_) => MemberClass::Opaque,
        },
        MemberClass::Scalar(mut shape) => {
            match intersect_validation(&shape.validation, &term.validation) {
                Ok(merged) => {
                    shape.validation = merged;
                    shape.nullable = shape.nullable
                        && (matches!(term.kind, SchemaKind::AnyValue) || term.nullable);
                    MemberClass::Scalar(shape)
                }
                Err(_) => MemberClass::Opaque,
            }
        }
        // An unconstrained member carrying sibling keywords is not
        // representable as a merge participant; conservative degradation.
        MemberClass::Neutral => {
            if term.validation == crate::ir::schema::ValidationMeta::default() {
                MemberClass::Neutral
            } else {
                MemberClass::Opaque
            }
        }
        MemberClass::Opaque => MemberClass::Opaque,
    }
}

/// Discriminator attached to an original composition node, if any.
fn node_discriminator(kind: &SchemaKind) -> Option<DiscriminatorIr> {
    match kind {
        SchemaKind::AllOf { discriminator, .. }
        | SchemaKind::OneOf { discriminator, .. }
        | SchemaKind::AnyOf { discriminator, .. } => discriminator.clone(),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChoiceKeyword {
    OneOf,
    AnyOf,
}

/// Constant set carried by an enum-family kind, if any.
fn scalar_enum_values(kind: &SchemaKind) -> Option<EnumValues> {
    match kind {
        SchemaKind::Enum { values } => Some(values.clone()),
        _ => None,
    }
}

impl ChoiceKeyword {
    fn as_str(self) -> &'static str {
        match self {
            Self::OneOf => "oneOf",
            Self::AnyOf => "anyOf",
        }
    }
}

// ----------------------------------------------------------------------
// allOf: intersection-first (companion §4.1)
// ----------------------------------------------------------------------

/// Classification of one effective allOf member after alias chasing.
#[derive(Debug, Clone)]
enum MemberClass {
    Object {
        properties: Vec<PropertyIr>,
        additional: Option<AdditionalPropertiesPolicy>,
        validation: ValidationMeta,
        nullable: bool,
    },
    Scalar(ScalarShape),
    /// Unconstrained `{}` member: imposes nothing, skipped when merging.
    Neutral,
    /// Nested compositions, tuples, mixed enums, unsupported nodes.
    Opaque,
}

#[derive(Debug, Clone)]
struct ScalarShape {
    family: Family,
    validation: ValidationMeta,
    nullable: bool,
    /// `format: binary` payload marker (main spec §5.3).
    binary: bool,
    /// Item edge for array-family shapes.
    items_edge: Option<SchemaEdge>,
    /// Constant set for enum-family shapes.
    enum_values: Option<EnumValues>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Boolean,
    Integer,
    Number,
    String_,
    Array,
    EnumStrings,
    EnumIntegers,
}

fn family_of_kind(kind: &SchemaKind) -> Option<Family> {
    Some(match kind {
        SchemaKind::Boolean => Family::Boolean,
        SchemaKind::Integer { .. } => Family::Integer,
        SchemaKind::Number { .. } => Family::Number,
        SchemaKind::String_ { .. } => Family::String_,
        SchemaKind::Array { items } if matches!(items.indirection, Indirection::None) => {
            Family::Array
        }
        SchemaKind::Enum { values } => match values {
            EnumValues::Strings(_) => Family::EnumStrings,
            EnumValues::Integers(_) => Family::EnumIntegers,
            EnumValues::MixedFallback(_) => return None,
        },
        _ => return None,
    })
}

/// Unifies two scalar families; `None` marks an incompatible pair.
/// `Integer` ⊂ `Number` promotes to `Number` (format agreement is checked
/// through the shared validation intersection).
fn unify_family(a: Family, b: Family) -> Option<Family> {
    use Family::*;
    Some(match (a, b) {
        (x, y) if x == y => x,
        (Integer, Number) | (Number, Integer) => Number,
        _ => return None,
    })
}

impl<'a> Resolver<'a> {
    fn resolve_all_of_like(
        &mut self,
        members: Vec<SchemaEdge>,
        discriminator: Option<DiscriminatorIr>,
        crumbs: &DocumentPath,
    ) -> CompositionOutcome {
        for (index, edge) in members.iter().enumerate() {
            let member_crumbs = crumbs.key("allOf").index(index);
            self.resolve(edge.target, &member_crumbs);
        }

        let mut classes = Vec::with_capacity(members.len());
        for (index, edge) in members.iter().enumerate() {
            let member_crumbs = crumbs.key("allOf").index(index);
            classes.push(self.classify_member(edge.target, &member_crumbs));
        }

        // A single-member allOf is a pure passthrough.
        if classes.len() == 1 {
            return CompositionOutcome::Passthrough(ResolvedKind::Alias(members[0].target));
        }

        let non_neutral_count = classes
            .iter()
            .filter(|c| !matches!(c, MemberClass::Neutral))
            .count();
        if non_neutral_count == 0 {
            // Every member was `{}`; nothing constrains anything.
            return CompositionOutcome::Passthrough(ResolvedKind::Plain);
        }
        if non_neutral_count == 1 {
            // One constraining member plus neutral `{}`s: the constraint
            // stands alone (intersection with the universe).
            for (index, class) in classes.iter().enumerate() {
                if !matches!(class, MemberClass::Neutral) {
                    return CompositionOutcome::Passthrough(ResolvedKind::Alias(
                        members[index].target,
                    ));
                }
            }
        }

        let all_objects = classes
            .iter()
            .all(|c| matches!(c, MemberClass::Object { .. } | MemberClass::Neutral));
        let any_object = classes
            .iter()
            .any(|c| matches!(c, MemberClass::Object { .. }));
        let all_scalars = !any_object
            && classes
                .iter()
                .all(|c| matches!(c, MemberClass::Scalar(_) | MemberClass::Neutral));

        if all_objects {
            self.merge_objects(&classes, discriminator, crumbs)
        } else if all_scalars {
            self.intersect_scalars(&classes, discriminator, crumbs)
        } else {
            self.fallback(
                FallbackReason::UnrepresentableAllOf,
                discriminator,
                ValidationMeta::default(),
                "allof_unrepresentable",
                format!(
                    "`allOf` mixes object and non-object members at {}; falling back to \
                     raw/value (serde(flatten) is never used for non-object members, \
                     companion §4.1)",
                    render_crumbs(crumbs)
                ),
                crumbs,
            )
        }
    }

    /// Classifies one member by chasing aliases (collecting conjunction
    /// terms) to the effective node and consulting its already-computed
    /// resolution for nested compositions. Term validations fold into the
    /// member's validation metadata; irreconcilable folds degrade the
    /// member to [`MemberClass::Opaque`] which drives the conservative
    /// fallback path.
    fn classify_member(&mut self, edge_target: SchemaId, crumbs: &DocumentPath) -> MemberClass {
        let (chased, terms) = self.chase(edge_target);
        for term in &terms {
            self.resolve(term.target, crumbs);
        }
        let mut class = self.classify_effective(chased);
        for term in &terms {
            class = fold_term_validation(class, self.arena.get(term.target));
        }
        class
    }

    /// Classification of the effective node after alias chasing.
    fn classify_effective(&self, chased: SchemaId) -> MemberClass {
        let resolved = match self.slot(chased) {
            Some(resolved) => resolved.clone(),
            None => return MemberClass::Opaque,
        };
        let node = self.arena.get(chased);
        match &resolved.kind {
            ResolvedKind::MergedObject(merged) => MemberClass::Object {
                properties: merged.properties.clone(),
                additional: Some(merged.additional),
                validation: resolved.validation.clone(),
                nullable: resolved.nullable,
            },
            ResolvedKind::IntersectedScalar(scalar) => {
                let family = match family_of_kind(&scalar.base_kind) {
                    Some(family) => family,
                    None => return MemberClass::Opaque,
                };
                let binary = matches!(&scalar.base_kind, SchemaKind::String_ { binary: true, .. });
                let items_edge = match &scalar.base_kind {
                    SchemaKind::Array { items } => Some(*items),
                    _ => None,
                };
                MemberClass::Scalar(ScalarShape {
                    family,
                    validation: resolved.validation.clone(),
                    nullable: resolved.nullable,
                    binary,
                    items_edge,
                    enum_values: scalar_enum_values(&scalar.base_kind),
                })
            }
            ResolvedKind::ClosedEnum(_) | ResolvedKind::RawValueFallback(_) => MemberClass::Opaque,
            ResolvedKind::Alias(_) | ResolvedKind::Plain => match &node.kind {
                SchemaKind::AnyValue => MemberClass::Neutral,
                SchemaKind::Object {
                    properties,
                    additional,
                } => MemberClass::Object {
                    properties: properties.clone(),
                    additional: Some(*additional),
                    validation: node.validation.clone(),
                    nullable: node.nullable,
                },
                SchemaKind::FreeFormObject => MemberClass::Object {
                    properties: Vec::new(),
                    additional: Some(AdditionalPropertiesPolicy::Ignore),
                    validation: node.validation.clone(),
                    nullable: node.nullable,
                },
                kind => match family_of_kind(kind) {
                    Some(family) => {
                        let binary = matches!(kind, SchemaKind::String_ { binary: true, .. });
                        let items_edge = match kind {
                            SchemaKind::Array { items } => Some(*items),
                            _ => None,
                        };
                        MemberClass::Scalar(ScalarShape {
                            family,
                            validation: node.validation.clone(),
                            nullable: node.nullable,
                            binary,
                            items_edge,
                            enum_values: scalar_enum_values(kind),
                        })
                    }
                    None => MemberClass::Opaque,
                },
            },
        }
    }

    /// Field-wise object merge: identical constraints collapse, `required`
    /// unions, conflicting property constraints are generation errors
    /// listing schema paths (companion §4.1).
    fn merge_objects(
        &mut self,
        classes: &[MemberClass],
        discriminator: Option<DiscriminatorIr>,
        crumbs: &DocumentPath,
    ) -> CompositionOutcome {
        let mut properties: Vec<PropertyIr> = Vec::new();
        let mut additional: Option<Option<AdditionalPropertiesPolicy>> = Some(None);
        let mut combined = ValidationMeta::default();
        let mut nullable = true;
        let mut conflict: Option<(String, String)> = None;
        let mut unrepresentable = false;

        for class in classes {
            let MemberClass::Object {
                properties: props,
                additional: add,
                validation,
                nullable: member_nullable,
            } = class
            else {
                continue;
            };
            nullable &= *member_nullable;
            for prop in props {
                if let Some(existing) = properties
                    .iter_mut()
                    .find(|p| p.wire_name == prop.wire_name)
                {
                    existing.required |= prop.required;
                    let same = self.nodes_equivalent(existing.schema.target, prop.schema.target);
                    if !same {
                        conflict
                            .get_or_insert_with(|| (prop.wire_name.clone(), render_crumbs(crumbs)));
                    }
                } else {
                    properties.push(prop.clone());
                }
            }
            match combine_additional(flatten_accumulator(additional.take()), *add) {
                Ok(combined_additional) => additional = Some(combined_additional),
                Err(()) => unrepresentable = true,
            }
            match intersect_validation(&combined, validation) {
                Ok(merged) => combined = merged,
                Err(field) => {
                    unrepresentable = true;
                    let _ = field;
                }
            }
        }

        if let Some((wire_name, path)) = conflict {
            // Generation error per companion §4.1; the resolution slot still
            // needs a value, but normalize will surface the Error severity.
            self.diags.push(Diagnostic {
                severity: Severity::Error,
                path: crumbs.clone(),
                code: "allof_property_conflict",
                message: format!(
                    "conflicting constraints for property `{wire_name}` between merged \
                     allOf members at {path} (differing type/kind/nullability/validation); \
                     this is a generation error listing schema paths (companion §4.1)"
                ),
            });
            return CompositionOutcome::Computed(Box::new(ResolvedNode {
                kind: ResolvedKind::RawValueFallback(RawFallback {
                    reason: FallbackReason::UnrepresentableAllOf,
                    native_serde_candidate: false,
                }),
                nullable,
                validation: ValidationMeta::default(),
                discriminator,
                diagnostics: Vec::new(),
            }));
        }
        if unrepresentable {
            return self.fallback(
                FallbackReason::UnrepresentableAllOf,
                discriminator,
                ValidationMeta::default(),
                "allof_unrepresentable",
                format!(
                    "merged allOf members at {} declare irreconcilable \
                     additionalProperties or object-level constraints; falling back to \
                     raw/value",
                    render_crumbs(crumbs)
                ),
                crumbs,
            );
        }

        CompositionOutcome::Computed(Box::new(ResolvedNode {
            kind: ResolvedKind::MergedObject(MergedObject {
                properties,
                additional: additional
                    .unwrap_or(Some(AdditionalPropertiesPolicy::Ignore))
                    .unwrap_or(AdditionalPropertiesPolicy::Ignore),
            }),
            nullable,
            validation: combined,
            discriminator,
            diagnostics: Vec::new(),
        }))
    }

    /// Scalar intersection: compatible families unify onto one validated
    /// type carrying every constraint (main spec §50 test 51).
    fn intersect_scalars(
        &mut self,
        classes: &[MemberClass],
        discriminator: Option<DiscriminatorIr>,
        crumbs: &DocumentPath,
    ) -> CompositionOutcome {
        let shapes: Vec<ScalarShape> = classes
            .iter()
            .filter_map(|c| match c {
                MemberClass::Scalar(shape) => Some(shape.clone()),
                _ => None,
            })
            .collect();

        let mut family = shapes[0].family;
        for shape in &shapes[1..] {
            match unify_family(family, shape.family) {
                Some(unified) => family = unified,
                None => {
                    return self.scalar_fallback(discriminator, crumbs, "incompatible base types");
                }
            }
        }

        // Arrays unify only onto identical item schemas (conservative).
        if family == Family::Array {
            let items_ok = shapes.iter().all(|shape| shape.items_edge.is_some())
                && shapes.iter().all(|shape| {
                    let first = shapes[0].items_edge.expect("checked above");
                    self.nodes_equivalent(
                        first.target,
                        shape.items_edge.expect("checked above").target,
                    )
                });
            if !items_ok {
                return self.scalar_fallback(discriminator, crumbs, "differing array item types");
            }
        }

        let mut combined = ValidationMeta::default();
        let mut nullable = true;
        for shape in &shapes {
            nullable &= shape.nullable;
            match intersect_validation(&combined, &shape.validation) {
                Ok(merged) => combined = merged,
                Err(field) => {
                    let _ = field;
                    return self.scalar_fallback(
                        discriminator,
                        crumbs,
                        "conflicting validation constraints that cannot be represented \
                         losslessly",
                    );
                }
            }
        }

        let base_kind = if family == Family::Array {
            SchemaKind::Array {
                items: shapes[0]
                    .items_edge
                    .expect("array family carries its item edge"),
            }
        } else {
            match family {
                Family::Boolean => SchemaKind::Boolean,
                Family::Integer => SchemaKind::Integer {
                    format: combined.format.clone(),
                },
                Family::Number => SchemaKind::Number {
                    format: combined.format.clone(),
                },
                Family::String_ => SchemaKind::String_ {
                    format: combined.format.clone(),
                    binary: shapes.iter().any(|shape| shape.binary),
                },
                Family::Array => unreachable!("handled above"),
                Family::EnumStrings | Family::EnumIntegers => {
                    let shared = intersect_enum_values(&shapes, family);
                    match shared {
                        Some(values) if enum_values_len(&values) > 0 => SchemaKind::Enum { values },
                        _ => {
                            return self.scalar_fallback(
                                discriminator,
                                crumbs,
                                "enum members share no constants",
                            );
                        }
                    }
                }
            }
        };

        CompositionOutcome::Computed(Box::new(ResolvedNode {
            kind: ResolvedKind::IntersectedScalar(IntersectedScalar { base_kind }),
            nullable,
            validation: combined,
            discriminator,
            diagnostics: Vec::new(),
        }))
    }

    fn scalar_fallback(
        &mut self,
        discriminator: Option<DiscriminatorIr>,
        crumbs: &DocumentPath,
        why: &str,
    ) -> CompositionOutcome {
        self.fallback(
            FallbackReason::UnrepresentableAllOf,
            discriminator,
            ValidationMeta::default(),
            "allof_unrepresentable",
            format!(
                "allOf scalar members at {} cannot be intersected into one validated \
                 type ({why}); falling back to raw/value (companion §4.1)",
                render_crumbs(crumbs)
            ),
            crumbs,
        )
    }

    /// Records a raw/value fallback with a Warning diagnostic (default
    /// policy mirroring D-impl-oneoffallback conservatism).
    #[allow(clippy::too_many_arguments)]
    fn fallback(
        &mut self,
        reason: FallbackReason,
        discriminator: Option<DiscriminatorIr>,
        validation: ValidationMeta,
        code: &'static str,
        message: String,
        crumbs: &DocumentPath,
    ) -> CompositionOutcome {
        self.diags.push(Diagnostic {
            severity: Severity::Warning,
            path: crumbs.clone(),
            code,
            message,
        });
        CompositionOutcome::Computed(Box::new(ResolvedNode {
            kind: ResolvedKind::RawValueFallback(RawFallback {
                reason,
                native_serde_candidate: false,
            }),
            nullable: false,
            validation,
            discriminator,
            diagnostics: Vec::new(),
        }))
    }
}

// ----------------------------------------------------------------------
// oneOf / anyOf: exactly-one semantics with static exclusivity proofs
// (companion §4.2)
// ----------------------------------------------------------------------

/// Single-valued constant found on a branch's top-level property.
#[derive(Debug, Clone)]
struct ConstantFact {
    value: JsonValue,
    required: bool,
    /// True when the property schema is exactly the single constant with no
    /// additional validation keywords (needed by the native serde check).
    validation_free: bool,
}

/// Facts gathered per branch for the exclusivity proofs.
#[derive(Debug, Clone)]
struct BranchFacts {
    json_type: Option<JsonType>,
    allows_null: bool,
    /// Property name → constant fact (top-level only).
    consts: BTreeMap<String, ConstantFact>,
}

impl<'a> Resolver<'a> {
    fn resolve_choice(
        &mut self,
        keyword: ChoiceKeyword,
        members: Vec<SchemaEdge>,
        discriminator: Option<DiscriminatorIr>,
        crumbs: &DocumentPath,
    ) -> CompositionOutcome {
        for (index, edge) in members.iter().enumerate() {
            let member_crumbs = crumbs.key(keyword.as_str()).index(index);
            self.resolve(edge.target, &member_crumbs);
        }

        let branches: Vec<BranchFacts> = members
            .iter()
            .map(|edge| self.branch_facts(edge.target))
            .collect();

        let proven = prove_exclusive(&branches, &members, discriminator.as_ref(), self);
        let native = proven && native_serde_candidate(&branches, discriminator.as_ref());

        if proven {
            return CompositionOutcome::Computed(Box::new(ResolvedNode {
                kind: ResolvedKind::ClosedEnum(ClosedEnumChoice {
                    branches: members,
                    native_serde_candidate: native,
                }),
                nullable: false,
                validation: ValidationMeta::default(),
                discriminator,
                diagnostics: Vec::new(),
            }));
        }

        let code = match keyword {
            ChoiceKeyword::OneOf => "oneof_unprovable",
            ChoiceKeyword::AnyOf => "anyof_unprovable",
        };
        let severity = match keyword {
            // anyOf always falls back (companion §4.2 MUST NOT clause);
            // oneOf honors the configured mode.
            ChoiceKeyword::OneOf => match self.config.oneof_fallback {
                super::OneOfFallbackMode::RawValue => Severity::Warning,
                super::OneOfFallbackMode::Error => Severity::Error,
            },
            ChoiceKeyword::AnyOf => Severity::Warning,
        };
        let reason = match keyword {
            ChoiceKeyword::OneOf => FallbackReason::UnprovenOneOf,
            ChoiceKeyword::AnyOf => FallbackReason::UnprovenAnyOf,
        };
        self.diags.push(Diagnostic {
            severity,
            path: crumbs.clone(),
            code,
            message: format!(
                "mutual exclusivity of {} branches at {} cannot be proven statically; \
                 falling back to raw/value carrying validation metadata \
                 (companion §4.2, D-impl-oneoffallback)",
                keyword.as_str(),
                render_crumbs(crumbs)
            ),
        });
        CompositionOutcome::Computed(Box::new(ResolvedNode {
            kind: ResolvedKind::RawValueFallback(RawFallback {
                reason,
                native_serde_candidate: false,
            }),
            nullable: false,
            validation: ValidationMeta::default(),
            discriminator,
            diagnostics: Vec::new(),
        }))
    }

    fn branch_facts(&self, edge_target: SchemaId) -> BranchFacts {
        let chased = self.chase_id(edge_target);
        let node = self.arena.get(chased);
        let resolved_kind = self.slot(chased).map(|r| r.kind.clone());
        let json_type = match (&resolved_kind, &node.kind) {
            (
                Some(ResolvedKind::MergedObject(_))
                | Some(ResolvedKind::ClosedEnum(_))
                | Some(ResolvedKind::RawValueFallback(_)),
                _,
            ) => None,
            (Some(ResolvedKind::IntersectedScalar(scalar)), _) => {
                json_type_of_kind(&scalar.base_kind)
            }
            _ => json_type_of_kind(&node.kind),
        };
        BranchFacts {
            json_type,
            allows_null: self.allows_null(chased),
            consts: top_level_consts(self, chased),
        }
    }
}

/// Extracts single-constant properties (`const` or one-value `enum`) from a
/// branch's top-level object shape; both plain objects and merged allOf
/// objects contribute.
fn top_level_consts(
    resolver: &Resolver<'_>,
    branch_id: SchemaId,
) -> BTreeMap<String, ConstantFact> {
    let mut out = BTreeMap::new();
    let properties: Vec<PropertyIr> = match resolver.slot(branch_id).map(|r| r.kind.clone()) {
        Some(ResolvedKind::MergedObject(merged)) => merged.properties,
        Some(ResolvedKind::Alias(target)) => {
            return top_level_consts(resolver, target);
        }
        _ => match &resolver.arena.get(branch_id).kind {
            SchemaKind::Object { properties, .. } => properties.clone(),
            _ => Vec::new(),
        },
    };
    for prop in properties {
        let prop_target = resolver.chase_id(prop.schema.target);
        let prop_node = resolver.arena.get(prop_target);
        let constant = match &prop_node.kind {
            SchemaKind::Enum { values } => match values {
                EnumValues::Strings(items) if items.len() == 1 => {
                    Some(JsonValue::String(items[0].clone()))
                }
                EnumValues::Integers(items) if items.len() == 1 => Some(JsonValue::from(items[0])),
                _ => None,
            },
            _ => None,
        };
        if let Some(value) = constant {
            let validation_free = prop_node.validation == ValidationMeta::default();
            out.insert(
                prop.wire_name.clone(),
                ConstantFact {
                    value,
                    required: prop.required,
                    validation_free,
                },
            );
        }
    }
    out
}

/// Proof standard over branch pairs (companion §4.2): (a) explicit
/// discriminator mapping assigning disjoint constants to distinct schemas
/// where every mapped branch itself constrains the tag property, (b)
/// contradictory single-valued REQUIRED constants on a shared same-named
/// property, (c) pairwise-differing top-level JSON types. Integer/Number are
/// treated as overlapping (`5` validates against both); at most one branch
/// may admit `null`. The discriminator NEVER changes the verdict on its own:
/// (a) only fires when the mapping genuinely assigns disjoint constants AND
/// every branch schema carries its own matching required tag constant, so a
/// branch that does not constrain the tag can never validate another
/// branch's documents.
fn prove_exclusive(
    branches: &[BranchFacts],
    members: &[SchemaEdge],
    discriminator: Option<&DiscriminatorIr>,
    resolver: &Resolver<'_>,
) -> bool {
    if branches.is_empty() || members.is_empty() {
        return false;
    }
    if branches.iter().filter(|b| b.allows_null).count() > 1 {
        return false;
    }
    proof_discriminator(branches, members, discriminator, resolver)
        || (0..branches.len()).all(|i| {
            (0..branches.len())
                .filter(|j| *j != i)
                .all(|j| const_contradiction(&branches[i], &branches[j]))
        })
        || types_disjoint(branches)
}

/// Proof (b): two branches cannot both validate when a shared required
/// property carries different single constants on each.
fn const_contradiction(a: &BranchFacts, b: &BranchFacts) -> bool {
    for (name, fact_a) in &a.consts {
        if !fact_a.required {
            continue;
        }
        if let Some(fact_b) = b.consts.get(name) {
            if fact_b.required && fact_a.value != fact_b.value {
                return true;
            }
        }
    }
    false
}

/// Proof (c): pairwise-differing top-level JSON types across every branch;
/// integer/number overlap blocks the proof.
fn types_disjoint(branches: &[BranchFacts]) -> bool {
    if branches.iter().any(|b| b.json_type.is_none()) {
        return false;
    }
    let has_integer = branches
        .iter()
        .any(|b| b.json_type == Some(JsonType::Integer));
    let has_number = branches
        .iter()
        .any(|b| b.json_type == Some(JsonType::Number));
    if has_integer && has_number {
        return false;
    }
    for i in 0..branches.len() {
        for j in (i + 1)..branches.len() {
            if branches[i].json_type == branches[j].json_type {
                return false;
            }
        }
    }
    true
}

/// Proof (a): an explicit mapping covers every branch exactly once with
/// pairwise-distinct constant values, AND every mapped-to branch schema
/// itself requires the discriminator property carrying exactly its mapping
/// constant (`const` or a one-value `enum`, string or integer form; resolved
/// through `Ref` chains exactly like the [`BranchFacts`] consts used by
/// proof (b), companion §4.2). A mapping entry alone is NOT sound: a mapped
/// branch that does not constrain the tag property could still validate
/// documents carrying another branch's tag value, violating exactly-one.
/// Branches must resolve to declared component schemas to match mapping
/// entries. `branches` and `members` are index-aligned.
fn proof_discriminator(
    branches: &[BranchFacts],
    members: &[SchemaEdge],
    discriminator: Option<&DiscriminatorIr>,
    resolver: &Resolver<'_>,
) -> bool {
    let Some(disc) = discriminator else {
        return false;
    };
    if !disc.explicit || disc.mapping.is_empty() || branches.len() != members.len() {
        return false;
    }
    let mut assigned_values: Vec<Option<String>> = Vec::with_capacity(members.len());
    for edge in members {
        let chased = resolver.chase_id(edge.target);
        let component = resolver.component_names.get(&chased.0);
        let value = component.and_then(|name| {
            disc.mapping
                .iter()
                .find(|(_, target)| &target.0 == name)
                .map(|(value, _)| value.clone())
        });
        assigned_values.push(value);
    }
    if assigned_values.iter().any(Option::is_none) {
        return false;
    }
    let flat: Vec<&String> = assigned_values.iter().flatten().collect();
    let distinct: BTreeSet<&String> = flat.iter().copied().collect();
    if distinct.len() != flat.len() {
        return false;
    }
    // Every branch must itself require the tag property with the exact
    // single-valued constant of its mapping entry; otherwise exactly-one
    // validation could not be reduced to validating only the selected
    // branch (companion §4.2 Decided).
    branches
        .iter()
        .zip(assigned_values.iter())
        .all(|(branch, value)| {
            let Some(value) = value else {
                return false;
            };
            match branch.consts.get(&disc.property_name) {
                Some(fact) => fact.required && mapping_value_matches(&fact.value, value),
                None => false,
            }
        })
}

/// True when a branch's single-valued tag constant agrees with a mapping
/// entry: string constants compare verbatim; integer constants compare via
/// the decimal spelling of the mapping key (`enum: [2]` ↔ mapping `2`).
fn mapping_value_matches(constant: &JsonValue, mapping_value: &str) -> bool {
    if constant == &JsonValue::String(mapping_value.to_owned()) {
        return true;
    }
    constant
        .as_i64()
        .is_some_and(|int| int.to_string() == mapping_value)
}

/// Native serde internally-tagged candidate flag (companion §4.2 Decided):
/// exclusivity proven AND every branch is object-typed AND carries the tag
/// property as its expected required single constant without extra tag
/// constraints. Codegen decides later; this is computed honestly either way.
fn native_serde_candidate(
    branches: &[BranchFacts],
    discriminator: Option<&DiscriminatorIr>,
) -> bool {
    let Some(disc) = discriminator else {
        return false;
    };
    branches.iter().all(|branch| {
        branch.json_type == Some(JsonType::Object)
            && branch
                .consts
                .get(&disc.property_name)
                .is_some_and(|fact| fact.required && fact.validation_free)
    })
}

/// Intersects homogeneous enum constant sets across allOf members,
/// preserving first-member order.
fn intersect_enum_values(shapes: &[ScalarShape], family: Family) -> Option<EnumValues> {
    let first = shapes[0].enum_values.clone()?;
    let shared: Vec<String> = match (&first, family) {
        (EnumValues::Strings(values), Family::EnumStrings) => values.clone(),
        (EnumValues::Integers(values), Family::EnumIntegers) => {
            let mut shared_ints = values.clone();
            for shape in &shapes[1..] {
                if let Some(EnumValues::Integers(values)) = &shape.enum_values {
                    shared_ints.retain(|v| values.contains(v));
                } else {
                    return None;
                }
            }
            return Some(EnumValues::Integers(shared_ints));
        }
        _ => return None,
    };
    let mut shared_strings = shared;
    for shape in &shapes[1..] {
        if let Some(EnumValues::Strings(values)) = &shape.enum_values {
            shared_strings.retain(|v| values.contains(v));
        } else {
            return None;
        }
    }
    Some(EnumValues::Strings(shared_strings))
}

fn enum_values_len(values: &EnumValues) -> usize {
    match values {
        EnumValues::Strings(items) => items.len(),
        EnumValues::Integers(items) => items.len(),
        EnumValues::MixedFallback(items) => items.len(),
    }
}

// ----------------------------------------------------------------------
// Structural equivalence of arena nodes (merge conflict detection)
// ----------------------------------------------------------------------

impl<'a> Resolver<'a> {
    /// Structural equality of two schema nodes after normalization:
    /// kind (recursively), nullability, directionality, default, and
    /// validation must agree; descriptions and load diagnostics are
    /// cosmetic and ignored. Cycle-safe via an assumed-equal pair memo
    /// (coinductive equality).
    fn nodes_equivalent(&self, a: SchemaId, b: SchemaId) -> bool {
        let mut memo = BTreeSet::new();
        nodes_equivalent_inner(self.arena, a, b, &mut memo)
    }
}

fn edges_equivalent(
    arena: &SchemaArena,
    a: &crate::ir::schema::SchemaEdge,
    b: &crate::ir::schema::SchemaEdge,
    memo: &mut BTreeSet<(u32, u32)>,
) -> bool {
    a.indirection == b.indirection && nodes_equivalent_inner(arena, a.target, b.target, memo)
}

fn nodes_equivalent_inner(
    arena: &SchemaArena,
    a: SchemaId,
    b: SchemaId,
    memo: &mut BTreeSet<(u32, u32)>,
) -> bool {
    if a == b {
        return true;
    }
    let key = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
    if !memo.insert(key) {
        // Already assumed equal higher up the recursion (cycle).
        return true;
    }
    let node_a = arena.get(a);
    let node_b = arena.get(b);
    if node_a.nullable != node_b.nullable
        || node_a.read_only != node_b.read_only
        || node_a.write_only != node_b.write_only
        || node_a.default != node_b.default
        || node_a.validation != node_b.validation
    {
        return false;
    }
    kinds_equivalent(arena, &node_a.kind, &node_b.kind, memo)
}

fn kinds_equivalent(
    arena: &SchemaArena,
    a: &SchemaKind,
    b: &SchemaKind,
    memo: &mut BTreeSet<(u32, u32)>,
) -> bool {
    match (a, b) {
        (SchemaKind::AnyValue, SchemaKind::AnyValue) => true,
        (SchemaKind::FreeFormObject, SchemaKind::FreeFormObject) => true,
        (SchemaKind::Boolean, SchemaKind::Boolean) => true,
        (SchemaKind::Integer { format: fa }, SchemaKind::Integer { format: fb }) => fa == fb,
        (SchemaKind::Number { format: fa }, SchemaKind::Number { format: fb }) => fa == fb,
        (
            SchemaKind::String_ {
                format: fa,
                binary: ba,
            },
            SchemaKind::String_ {
                format: fb,
                binary: bb,
            },
        ) => fa == fb && ba == bb,
        (SchemaKind::Array { items: ia }, SchemaKind::Array { items: ib }) => {
            edges_equivalent(arena, ia, ib, memo)
        }
        (
            SchemaKind::Tuple {
                prefix_items: pa,
                items: xa,
            },
            SchemaKind::Tuple {
                prefix_items: pb,
                items: xb,
            },
        ) => {
            pa.len() == pb.len()
                && pa
                    .iter()
                    .zip(pb.iter())
                    .all(|(x, y)| edges_equivalent(arena, x, y, memo))
                && match (xa, xb) {
                    (Some(xa), Some(xb)) => edges_equivalent(arena, xa, xb, memo),
                    (None, None) => true,
                    _ => false,
                }
        }
        (
            SchemaKind::Object {
                properties: pa,
                additional: aa,
            },
            SchemaKind::Object {
                properties: pb,
                additional: ab,
            },
        ) => {
            additional_equivalent(arena, aa, ab, memo)
                && pa.len() == pb.len()
                && pa.iter().zip(pb.iter()).all(|(x, y)| {
                    x.wire_name == y.wire_name
                        && x.required == y.required
                        && edges_equivalent(arena, &x.schema, &y.schema, memo)
                })
        }
        (SchemaKind::Enum { values: va }, SchemaKind::Enum { values: vb }) => va == vb,
        (
            SchemaKind::Ref {
                target: ta,
                inline_constraints: ca,
                ..
            },
            SchemaKind::Ref {
                target: tb,
                inline_constraints: cb,
                ..
            },
        ) => {
            ta == tb
                && ca.len() == cb.len()
                && ca
                    .iter()
                    .zip(cb.iter())
                    .all(|(x, y)| edges_equivalent(arena, x, y, memo))
        }
        (
            SchemaKind::AllOf {
                members: ma,
                discriminator: da,
            },
            SchemaKind::AllOf {
                members: mb,
                discriminator: db,
            },
        )
        | (
            SchemaKind::OneOf {
                members: ma,
                discriminator: da,
            },
            SchemaKind::OneOf {
                members: mb,
                discriminator: db,
            },
        )
        | (
            SchemaKind::AnyOf {
                members: ma,
                discriminator: da,
            },
            SchemaKind::AnyOf {
                members: mb,
                discriminator: db,
            },
        ) => {
            da == db
                && ma.len() == mb.len()
                && ma
                    .iter()
                    .zip(mb.iter())
                    .all(|(x, y)| edges_equivalent(arena, x, y, memo))
        }
        (SchemaKind::NotSupported { reason: ra }, SchemaKind::NotSupported { reason: rb }) => {
            reason_eq(ra, rb)
        }
        _ => false,
    }
}

fn reason_eq(
    a: &crate::ir::schema::UnsupportedReason,
    b: &crate::ir::schema::UnsupportedReason,
) -> bool {
    use crate::ir::schema::UnsupportedReason::*;
    match (a, b) {
        (MixedTypeArray, MixedTypeArray)
        | (UnevaluatedKeywordsActive, UnevaluatedKeywordsActive)
        | (AnchorRef, AnchorRef)
        | (RemoteRefUnfetched, RemoteRefUnfetched)
        | (UnbrokenSelfContainment, UnbrokenSelfContainment)
        | (InlineExpansionDepthExceeded, InlineExpansionDepthExceeded) => true,
        (Other(x), Other(y)) => std::ptr::eq(*x, *y),
        _ => false,
    }
}

fn additional_equivalent(
    arena: &SchemaArena,
    a: &AdditionalPropertiesPolicy,
    b: &AdditionalPropertiesPolicy,
    memo: &mut BTreeSet<(u32, u32)>,
) -> bool {
    match (a, b) {
        (AdditionalPropertiesPolicy::Deny, AdditionalPropertiesPolicy::Deny)
        | (AdditionalPropertiesPolicy::Ignore, AdditionalPropertiesPolicy::Ignore) => true,
        (AdditionalPropertiesPolicy::Schema(x), AdditionalPropertiesPolicy::Schema(y)) => {
            edges_equivalent(arena, x, y, memo)
        }
        _ => false,
    }
}

// ----------------------------------------------------------------------
// additionalProperties unification and validation-metadata intersection
// ----------------------------------------------------------------------

/// Flattens the merge accumulator: the inner `None` (no policy declared
/// yet) behaves like an absent keyword.
fn flatten_accumulator(
    accumulator: Option<Option<AdditionalPropertiesPolicy>>,
) -> Option<AdditionalPropertiesPolicy> {
    accumulator.flatten()
}

/// Combines two declared `additionalProperties` policies under intersection
/// semantics: identical policies collapse; deny ∩ ignore = deny; mixing a
/// schema-valued form with anything else is unrepresentable.
fn combine_additional(
    current: Option<AdditionalPropertiesPolicy>,
    incoming: Option<AdditionalPropertiesPolicy>,
) -> Result<Option<AdditionalPropertiesPolicy>, ()> {
    use AdditionalPropertiesPolicy::{Deny, Ignore, Schema};
    let combined = match (current, incoming) {
        // Absent keywords on either side impose no constraint.
        (None, None) => return Ok(None),
        (None, Some(other)) | (Some(other), None) => other,
        (Some(Deny), Some(Ignore)) | (Some(Ignore), Some(Deny)) => Deny,
        (Some(Schema(_)), Some(_)) | (Some(_), Some(Schema(_))) => return Err(()),
        (Some(same), Some(_)) => same,
    };
    Ok(Some(combined))
}

/// Intersects two validation metadata sets (companion §4.1: scalar members
/// combine checks onto ONE validated type). Bounds merge tightly
/// (min-of-maxes / max-of-mins); conjunctions that cannot be represented —
/// two differing patterns, two differing formats, two multipleOfs — fail
/// with the conflicting field name.
fn intersect_validation(
    acc: &ValidationMeta,
    incoming: &ValidationMeta,
) -> Result<ValidationMeta, &'static str> {
    let mut out = acc.clone();

    fn single_or_equal<T: PartialEq + Clone>(
        acc: &Option<T>,
        inc: &Option<T>,
    ) -> Result<Option<T>, &'static str> {
        Ok(match (acc, inc) {
            (None, other) => other.clone(),
            (some, None) => some.clone(),
            (Some(x), Some(y)) if x == y => Some(x.clone()),
            _ => return Err("conflicting constraint"),
        })
    }

    out.pattern = single_or_equal(&acc.pattern, &incoming.pattern)?;
    out.format = single_or_equal(&acc.format, &incoming.format)?;
    out.content_encoding = single_or_equal(&acc.content_encoding, &incoming.content_encoding)?;
    out.content_media_type =
        single_or_equal(&acc.content_media_type, &incoming.content_media_type)?;

    out.min_length = opt_max(acc.min_length, incoming.min_length);
    out.max_length = opt_min(acc.max_length, incoming.max_length);
    out.min_items = opt_max(acc.min_items, incoming.min_items);
    out.max_items = opt_min(acc.max_items, incoming.max_items);
    out.unique_items = acc.unique_items || incoming.unique_items;
    out.min_properties = opt_max(acc.min_properties, incoming.min_properties);
    out.max_properties = opt_min(acc.max_properties, incoming.max_properties);

    // patternProperties/contains: only one side may specify (conservative).
    if !incoming.pattern_properties.is_empty() {
        if !acc.pattern_properties.is_empty() {
            return Err("patternProperties");
        }
        out.pattern_properties = incoming.pattern_properties.clone();
    }
    if incoming.contains.is_some() {
        if acc.contains.is_some() {
            return Err("contains");
        }
        out.contains = incoming.contains;
        out.min_contains = incoming.min_contains.or(acc.min_contains);
        out.max_contains = incoming.max_contains.or(acc.max_contains);
    } else if incoming.min_contains.is_some() || incoming.max_contains.is_some() {
        out.min_contains = opt_max(acc.min_contains, incoming.min_contains);
        out.max_contains = opt_min(acc.max_contains, incoming.max_contains);
    }

    out.numeric = intersect_numeric(&acc.numeric, &incoming.numeric)?;

    let mut examples = acc.examples.clone();
    examples.extend(incoming.examples.iter().cloned());
    out.examples = examples;

    Ok(out)
}

fn intersect_numeric(
    acc: &NumericValidation,
    inc: &NumericValidation,
) -> Result<NumericValidation, &'static str> {
    let lower = merge_bound(
        (acc.minimum, acc.exclusive_minimum),
        (inc.minimum, inc.exclusive_minimum),
        BoundSide::Lower,
    )?;
    let upper = merge_bound(
        (acc.maximum, acc.exclusive_maximum),
        (inc.maximum, inc.exclusive_maximum),
        BoundSide::Upper,
    )?;
    Ok(NumericValidation {
        minimum: lower.0,
        exclusive_minimum: lower.1,
        maximum: upper.0,
        exclusive_maximum: upper.1,
        multiple_of: match (acc.multiple_of, inc.multiple_of) {
            (None, other) | (other, None) => other,
            (Some(x), Some(y)) if x == y => Some(x),
            _ => return Err("multipleOf"),
        },
    })
}

#[derive(Clone, Copy)]
enum BoundSide {
    Lower,
    Upper,
}

/// Merges (inclusive, exclusive) bound pairs: the tighter value wins; ties
/// prefer the exclusive form (x ≥ v ∧ x > v ≡ x > v).
#[allow(clippy::type_complexity)]
fn merge_bound(
    a: (Option<f64>, Option<f64>),
    b: (Option<f64>, Option<f64>),
    side: BoundSide,
) -> Result<(Option<f64>, Option<f64>), &'static str> {
    let candidates = [
        a.0.map(|v| (v, false)),
        a.1.map(|v| (v, true)),
        b.0.map(|v| (v, false)),
        b.1.map(|v| (v, true)),
    ]
    .into_iter()
    .flatten();
    let mut best: Option<(f64, bool)> = None;
    for (value, exclusive) in candidates {
        best = Some(match best {
            None => (value, exclusive),
            Some((bv, be)) => {
                let better = match side {
                    BoundSide::Lower => value > bv || (value == bv && exclusive),
                    BoundSide::Upper => value < bv || (value == bv && exclusive),
                };
                if better {
                    (value, exclusive)
                } else {
                    (bv, be)
                }
            }
        });
    }
    Ok(match best {
        Some((value, true)) => (None, Some(value)),
        Some((value, false)) => (Some(value), None),
        None => (None, None),
    })
}

fn opt_max(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn opt_min(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Renders a document path for diagnostic messages.
fn render_crumbs(crumbs: &DocumentPath) -> String {
    crumbs.to_string()
}
