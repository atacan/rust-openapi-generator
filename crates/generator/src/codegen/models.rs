//! Shared-schema model emission (main spec §2.6, §3): one deterministic
//! `models.rs` per normalized document carrying every named component schema
//! in declaration order plus generated definitions for nested anonymous
//! objects and enumerations emitted before their parents.
//!
//! Property presence/nullability follows the companion §2.1 matrix
//! cell-for-cell; bucket-2 validation constraints ride as documentation until
//! Phase 2 runtime enforcement (DECISIONS.md D-impl-codegen-emission,
//! D-impl-flatten-map-deterministic, D-impl-runtime-validation-timing).
//! Output is byte-deterministic: declaration-order iteration only, no
//! timestamps and no paths anywhere (main spec §50 test 39).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as JsonValue;

use crate::ir::schema::{
    AdditionalPropertiesPolicy, DiscriminatorIr, EnumValues, Indirection, PropertyIr, SchemaEdge,
    SchemaId, SchemaKind, UnsupportedReason, ValidationMeta,
};
use crate::normalize::composition::{
    ClosedEnumChoice, FallbackReason, IntersectedScalar, MergedObject, RawFallback, ResolvedKind,
};
use crate::normalize::naming::{self, NameStyle};
use crate::normalize::{NormalizedDocument, NormalizedSchema};

use super::validation::{
    analyze, enforceable_with, is_recognized_format, Analysis, ScalarParamKind,
};
use super::Emitter;

/// Cell attribute for required + nullable properties (companion §2.1 row 2):
/// explicit `null` yields `None`, a missing key is a schema violation; no
/// `default` attr because presence stays mandatory.
const REQUIRED_NULLABLE_ATTR: &str =
    "#[serde(deserialize_with = \"openapi_support::optional::presence::deserialize_required_nullable\")]";

/// Cell attribute for optional + non-nullable properties (row 3): a missing
/// key is [`openapi_support::optional::OptionalField::Absent`], an explicit
/// `null` is a decode error.
const OPTIONAL_NON_NULLABLE_ATTR: &str =
    "#[serde(default, skip_serializing_if = \"openapi_support::optional::is_absent\")]";

/// Cell attribute for optional + nullable properties (row 4): absence and
/// `null` both yield `None`.
const OPTIONAL_NULLABLE_ATTR: &str = "#[serde(default)]";

/// Derive list for every struct and derive-based enum (main spec §2.6 shape).
const DERIVES: &str = "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]";

const NULLABLE_NOTE: &str =
    "Nullable: instances may be JSON `null`; reference sites wrap this type in `Option<T>`.";

const BINARY_WARNING: &str =
    "Warning: `format: binary` marks a raw payload; Binary media classes stream bytes and \
     never reach shared models in Phase 1 (main spec §5.3), so this is modeled as `String`.";

/// Renders ONE `models.rs`: crate-doc comment, support-crate/serde imports
/// where needed, then every named schema in declaration order.
#[must_use]
pub fn generate_models(doc: &NormalizedDocument) -> String {
    // Components in document position: ascending arena ids recover
    // declaration order (components were pre-interned before path traversal;
    // see normalize_with_config).
    let mut components: Vec<&NormalizedSchema> = doc.schemas.values().collect();
    components.sort_by_key(|schema| schema.source.0);

    // Fallback-shaped components emit their dedicated `<TypeName>Fallback`
    // newtype as the public type; every reference site must use that name.
    let names: Vec<String> = components
        .iter()
        .map(|schema| {
            let is_fallback = match doc.resolution(schema.source).kind.clone() {
                ResolvedKind::RawValueFallback(_) => true,
                ResolvedKind::Plain => matches!(
                    doc.arena.get(schema.source).kind,
                    SchemaKind::AnyValue | SchemaKind::NotSupported { .. }
                ),
                _ => false,
            };
            if is_fallback {
                format!("{}Fallback", schema.rust_type)
            } else {
                schema.rust_type.clone()
            }
        })
        .collect();

    let named: BTreeMap<u32, String> = components
        .iter()
        .zip(names.iter())
        .map(|(schema, name)| (schema.source.0, name.clone()))
        .collect();

    // Type names are global to the module: seed with every assigned name so a
    // generated nested name can never collide with a later component (and
    // vice versa). Issue #11 synthetic body types are reserved BEFORE any
    // definition is built, so nested anonymous names suffix around the
    // operation-based body names rather than stealing them.
    let mut used_names: BTreeSet<String> = named.values().cloned().collect();
    for (_, enum_name) in &doc.names.response_enums {
        used_names.insert(enum_name.clone());
    }
    for body_name in doc.names.synthetic_body_types.values() {
        used_names.insert(body_name.clone());
    }

    let mut generator = Generator {
        doc,
        named,
        anonymous: BTreeMap::new(),
        used_names,
        needs_optional_field: false,
        needs_btree_map: false,
        needs_serde: false,
        analysis: analyze(doc),
        validation: ValidationStatements::new(doc),
        defs: Vec::new(),
    };
    for (schema, name) in components.iter().zip(names.iter()) {
        generator.define_component(schema, name);
    }
    // Issue #11: top-level definitions for anonymous composite bodies under
    // their operation-based names (`<Op>RequestBody` / `<Op>ResponseBody`).
    // Nested anonymous schemas discovered inside are named `<Body><Field>`
    // through the same mechanism as component children. Arena ids are
    // site-unique, so a body id can never already sit in the anonymous table
    // (the guard is defensive only).
    let bodies: Vec<(u32, String)> = doc
        .names
        .synthetic_body_types
        .iter()
        .map(|(id, name)| (*id, name.clone()))
        .collect();
    for (id, name) in &bodies {
        if generator.anonymous.contains_key(id) {
            continue;
        }
        generator.define_node(SchemaId(*id), name);
    }

    render(&generator)
}

// ----------------------------------------------------------------------
// Definitions collected during discovery (children before parents)
// ----------------------------------------------------------------------

enum Def {
    Struct(StructDef),
    StringsEnum(EnumDef),
    IntegersEnum(IntegersEnumDef),
    MixedEnum(EnumDef),
    ChoiceEnum(ChoiceEnumDef),
    TypeAlias(AliasDef),
    FallbackNewtype(FallbackDef),
}

struct StructDef {
    name: String,
    docs: Vec<String>,
    deny_unknown_fields: bool,
    fields: Vec<Field>,
    /// Rendered statements of `validate_request` (fully pre-indented lines);
    /// empty when the model carries no runtime-enforceable constraints.
    validation_body: Vec<String>,
}

struct EnumDef {
    name: String,
    docs: Vec<String>,
    /// Field-less string variants; rename present when it differs from the
    /// wire constant.
    variants: Vec<Variant>,
}

struct IntegersEnumDef {
    name: String,
    docs: Vec<String>,
    /// Variant name ↔ wire discriminant, declaration order.
    variants: Vec<(String, i64)>,
    constants: Vec<i64>,
}

struct ChoiceEnumDef {
    name: String,
    docs: Vec<String>,
    /// Newtype variants over branch types, branch declaration order.
    variants: Vec<(String, String)>,
    /// Per-variant validator call when the branch payload type carries a
    /// `validate_request` (`Some(option_wrapped)`); `None` otherwise.
    variant_validations: Vec<Option<bool>>,
}

struct AliasDef {
    name: String,
    docs: Vec<String>,
    target: String,
    /// Free `validate_<snake>_request` for a constrained scalar alias
    /// (companion §9); `None` for unconstrained aliases and non-scalar
    /// targets (free-form maps, tuples).
    validator: Option<AliasValidator>,
}

/// Rendered free validator of one constrained scalar alias.
struct AliasValidator {
    fn_name: String,
    param_ty: &'static str,
    docs: Vec<String>,
    body: Vec<String>,
}

struct FallbackDef {
    name: String,
    docs: Vec<String>,
}

struct Variant {
    rust_name: String,
    rename: Option<String>,
    payload: Option<&'static str>,
}

struct Field {
    docs: Vec<String>,
    attrs: Vec<String>,
    name: String,
    ty: String,
}

// ----------------------------------------------------------------------
// Runtime-validation emission fragments (companion §9, D-§2 bucket 2)
// ----------------------------------------------------------------------
//
// Every builder renders statements for an EXPLICIT indentation level
// (four-space steps; a `validate_request` method body sits at level two,
// free alias validators at level one) because rustfmt's chain layout
// depends on absolute column position — wrapper blocks pass deeper levels
// down instead of re-prefixing pre-rendered text. Layout mirrors rustfmt's
// rules so generated output stays rustfmt-clean (main spec §50 test 40):
// 1. call plus annotation fit in [`RUSTFMT_MAX_WIDTH`] → one line;
// 2. otherwise the call fits alone → its own line, `.map_err` link one
//    level deeper;
// 3. otherwise the argument list breaks vertically with the closing paren
//    AND the `.map_err` link at the statement's own indent.

/// Max text width of generated code, matching rustfmt's default.
const RUSTFMT_MAX_WIDTH: usize = 100;

/// Statement pad inside an inherent `validate_request` method body
/// (indentation level two).
pub(crate) const METHOD_PAD: &str = "        ";

/// One support-crate validation call with optional field annotation
/// (companion §9): `field` names the generated location so routers surface
/// `Violation::Field` paths through SchemaViolation 422 details.
pub(crate) struct CheckCall {
    /// Callee expression WITHOUT the opening paren.
    pub(crate) callee: String,
    /// Fully rendered arguments (may themselves be nested blocks).
    pub(crate) args: Vec<String>,
    /// Field label for `Violation::at_field`; `None` leaves bare errors
    /// (undecidable-context predicates feeding `is_ok()` only).
    pub(crate) field: Option<String>,
}

/// Annotated composite-recursion statement
/// (`::openapi_support::validation::located("field", EXPR.validate_request())?;`).
/// A single flat call keeps emitted code free of multi-method chains, whose
/// rustfmt layout is context-dependent; [`CheckCall::render`] owns the width
/// decisions.
pub(crate) fn annotated_call(expr: &str, field: Option<&str>, level: usize) -> Vec<String> {
    let Some(field) = field else {
        return vec![format!("{}{};", "    ".repeat(level), expr)];
    };
    CheckCall {
        callee: "::openapi_support::validation::located".to_owned(),
        args: vec![format!("\"{}\"", escape_string(field)), expr.to_owned()],
        field: None,
    }
    .render(level)
}

impl CheckCall {
    pub(crate) fn render(&self, level: usize) -> Vec<String> {
        let pad = "    ".repeat(level);
        let inner = format!("{pad}    ");
        let annotation = self.field.as_deref().map(|field| {
            format!(
                ".map_err(|error| error.at_field(\"{}\"))",
                escape_string(field)
            )
        });
        let flat_call = format!("{}({})", self.callee, self.args.join(", "));
        let terminator = "?;";
        match &annotation {
            Some(annotation) => {
                let one_line = format!("{pad}{flat_call}{annotation}{terminator}");
                if one_line.chars().count() <= RUSTFMT_MAX_WIDTH {
                    return vec![one_line];
                }
                let flat_head = format!("{pad}{flat_call}");
                if flat_head.chars().count() <= RUSTFMT_MAX_WIDTH {
                    return vec![flat_head, format!("{pad}    {annotation}{terminator}")];
                }
            }
            None => {
                let one_line = format!("{pad}{flat_call}{terminator}");
                if one_line.chars().count() <= RUSTFMT_MAX_WIDTH {
                    return vec![one_line];
                }
            }
        }
        // Vertical form; multi-line arguments carry their INNER relative
        // layout (four-space steps), prefixed here to the argument column.
        let mut lines = vec![format!("{}{}(", pad, self.callee)];
        for arg in &self.args {
            let mut segments = arg.split('\n').map(ToOwned::to_owned).collect::<Vec<_>>();
            let last = segments.len() - 1;
            for (index, segment) in segments.iter_mut().enumerate() {
                let line = format!("{inner}{segment}");
                *segment = if index == last {
                    format!("{line},")
                } else {
                    line
                };
            }
            lines.extend(segments);
        }
        match &annotation {
            None => lines.push(format!("{pad}){terminator}")),
            Some(annotation) => {
                lines.push(format!("{pad})"));
                lines.push(format!("{pad}{annotation}{terminator}"));
            }
        }
        lines
    }
}

/// `validate_string` + `validate_format_string` against one accessor
/// expression (already borrow-shaped, e.g. `&self.code` or `value`).
pub(crate) fn string_check_lines(
    accessor: &str,
    validation: &ValidationMeta,
    field: Option<&str>,
    level: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    if validation.pattern.is_some()
        || validation.min_length.is_some()
        || validation.max_length.is_some()
    {
        let pattern = validation
            .pattern
            .as_deref()
            .map(|pattern| format!("Some(\"{}\")", escape_string(pattern)))
            .unwrap_or_else(|| "None".to_owned());
        lines.extend(
            CheckCall {
                callee: "::openapi_support::validation::validate_string".to_owned(),
                args: vec![
                    accessor.to_owned(),
                    format!(
                        "&::openapi_support::validation::StringConstraints {{\n    \
                         pattern: {pattern},\n    min_length: {},\n    max_length: {},\n}}",
                        option_u64(validation.min_length),
                        option_u64(validation.max_length)
                    ),
                ],
                field: field.map(ToOwned::to_owned),
            }
            .render(level),
        );
    }
    if let Some(format) = known_format(validation.format.as_deref()) {
        lines.extend(
            CheckCall {
                callee: "::openapi_support::validation::validate_format_string".to_owned(),
                args: vec![accessor.to_owned(), format!("\"{format}\"")],
                field: field.map(ToOwned::to_owned),
            }
            .render(level),
        );
    }
    lines
}

/// `validate_number` against one value expression.
pub(crate) fn number_check_lines(
    value_expr: &str,
    numeric: &crate::ir::schema::NumericValidation,
    field: Option<&str>,
    level: usize,
) -> Vec<String> {
    let min = match numeric.exclusive_minimum {
        Some(bound) => Some((bound, true)),
        None => numeric.minimum.map(|bound| (bound, false)),
    };
    let max = match numeric.exclusive_maximum {
        Some(bound) => Some((bound, true)),
        None => numeric.maximum.map(|bound| (bound, false)),
    };
    let bound_lit = |bound: Option<(f64, bool)>| match bound {
        Some((value, exclusive)) => format!("Some(({}, {}))", float_literal(value), exclusive),
        None => "None".to_owned(),
    };
    let multiple_of = numeric
        .multiple_of
        .map(|divisor| format!("Some({})", float_literal(divisor)))
        .unwrap_or_else(|| "None".to_owned());
    CheckCall {
        callee: "::openapi_support::validation::validate_number".to_owned(),
        args: vec![
            value_expr.to_owned(),
            bound_lit(min),
            bound_lit(max),
            multiple_of,
        ],
        field: field.map(ToOwned::to_owned),
    }
    .render(level)
}

pub(crate) fn option_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "None".to_owned(), |value| format!("Some({value})"))
}

/// Deterministic float literal with an explicit fractional part.
fn float_literal(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

/// The v1 recognized formats; unknown names stay metadata-only (documented).
/// Delegates to the shared [`is_recognized_format`] verdict so emission and
/// the validation analysis use the same predicate (issue #8).
fn known_format(format: Option<&str>) -> Option<&str> {
    match format {
        Some(name) if is_recognized_format(Some(name)) => Some(name),
        _ => None,
    }
}

// ----------------------------------------------------------------------
// Discovery pass
// ----------------------------------------------------------------------

struct Generator<'a> {
    doc: &'a NormalizedDocument,
    /// Named component arena id → Rust type name.
    named: BTreeMap<u32, String>,
    /// Anonymous effective node id → generated definition name.
    anonymous: BTreeMap<u32, String>,
    used_names: BTreeSet<String>,
    needs_optional_field: bool,
    needs_btree_map: bool,
    needs_serde: bool,
    /// Shared runtime-validation verdicts (companion §9); consulted for
    /// nested recursion and alias free functions.
    analysis: Analysis,
    /// Shared check-statement builders (companion §9), reused verbatim by
    /// the directional view emitter for `<M>Write` validators.
    validation: ValidationStatements<'a>,
    defs: Vec<Def>,
}

impl Generator<'_> {
    fn chase(&self, id: SchemaId) -> SchemaId {
        self.doc.resolve_alias(id)
    }

    fn resolution_of(&self, id: SchemaId) -> &ResolvedKind {
        &self.doc.resolution(id).kind
    }

    fn nullable_of(&self, id: SchemaId) -> bool {
        self.doc.resolution(id).nullable
    }

    fn validation_of(&self, id: SchemaId) -> ValidationMeta {
        self.doc.resolution(id).validation.clone()
    }

    fn fresh_type_name(&mut self, base: &str) -> String {
        unique_in(&mut self.used_names, base)
    }

    /// Registers an anonymous composite definition under a fresh name derived
    /// from `hint`, returning the name. The name is reserved before building
    /// so shared or cyclic references terminate deterministically.
    fn register_anonymous(&mut self, effective: SchemaId, hint: &str) -> String {
        if let Some(name) = self.anonymous.get(&effective.0) {
            return name.clone();
        }
        let name = self.fresh_type_name(hint);
        self.anonymous.insert(effective.0, name.clone());
        self.define_node(effective, &name);
        name
    }

    /// Defines the item for one named component. Nested anonymous definitions
    /// discovered along the way are pushed first, so every parent appears
    /// after its children in [`Generator::defs`].
    fn define_component(&mut self, schema: &NormalizedSchema, component_name: &str) {
        if matches!(self.resolution_of(schema.source), ResolvedKind::Alias(_)) {
            // Transparent `$ref` passthrough: a pure type alias.
            let effective = self.chase(schema.source);
            let mut docs =
                description_lines(&self.doc.arena.get(schema.source).description.clone());
            if docs.is_empty() {
                docs = description_lines(&self.doc.arena.get(effective).description.clone());
            }
            let target = self.reference_type(effective, component_name);
            let validator = self.alias_validator(component_name, effective);
            self.defs.push(Def::TypeAlias(AliasDef {
                name: component_name.to_owned(),
                docs,
                target,
                validator,
            }));
            return;
        }
        self.define_node(schema.source, component_name);
    }

    /// Builds the definition for one alias-chased arena node under `name`,
    /// pushing nested definitions before its own.
    fn define_node(&mut self, effective: SchemaId, name: &str) {
        match self.resolution_of(effective).clone() {
            ResolvedKind::Alias(_) => {} // callers chase aliases first; defensive only
            ResolvedKind::MergedObject(MergedObject {
                properties,
                additional,
            }) => {
                let validation = self.validation_of(effective);
                let docs = self.definition_docs(effective, &[], &validation);
                self.needs_serde = true;
                let def = self.build_struct(name, docs, &properties, additional, &validation);
                self.defs.push(def);
            }
            ResolvedKind::IntersectedScalar(scalar) => {
                self.define_intersected_scalar(effective, name, &scalar);
            }
            ResolvedKind::ClosedEnum(choice) => {
                let proven_note = "Proven mutually exclusive branches (companion §4.2): \
                                   exclusivity was proven statically, so derive-based untagged \
                                   decoding preserves exactly-one validation."
                    .to_owned();
                let mut docs =
                    self.definition_docs(effective, &[proven_note], &ValidationMeta::default());
                if let Some(discriminator) = self.doc.resolution(effective).discriminator.clone() {
                    docs.extend(discriminator_docs(&discriminator));
                }
                self.needs_serde = true;
                let def = self.build_choice_enum(name, docs, &choice);
                self.defs.push(def);
            }
            ResolvedKind::RawValueFallback(RawFallback { reason, .. }) => {
                let docs = self.definition_docs(
                    effective,
                    &fallback_reason_docs(reason),
                    &ValidationMeta::default(),
                );
                self.push_fallback(name, docs);
            }
            ResolvedKind::Plain => match self.doc.arena.get(effective).kind.clone() {
                SchemaKind::Object {
                    properties,
                    additional,
                } => {
                    let validation = self.validation_of(effective);
                    let docs = self.definition_docs(effective, &[], &validation);
                    self.needs_serde = true;
                    let def = self.build_struct(name, docs, &properties, additional, &validation);
                    self.defs.push(def);
                }
                SchemaKind::Enum { values } => self.define_enum(effective, name, values),
                SchemaKind::AnyValue | SchemaKind::NotSupported { .. } => {
                    let extra = match self.doc.arena.get(effective).kind {
                        SchemaKind::AnyValue => vec![
                            "Unconstrained schema (D-§4.4 row 2): represented as raw JSON."
                                .to_owned(),
                        ],
                        SchemaKind::NotSupported { reason } => unsupported_reason_docs(&reason),
                        _ => Vec::new(),
                    };
                    let docs = self.definition_docs(effective, &extra, &ValidationMeta::default());
                    self.push_fallback(name, docs);
                }
                SchemaKind::FreeFormObject => {
                    let docs = self.definition_docs(effective, &[], &ValidationMeta::default());
                    self.needs_serde = true;
                    self.defs.push(Def::TypeAlias(AliasDef {
                        name: name.to_owned(),
                        docs,
                        target: "serde_json::Map<String, serde_json::Value>".to_owned(),
                        validator: None,
                    }));
                }
                other_kind => {
                    let hint = name.to_owned();
                    let docs = self.definition_docs(effective, &[], &ValidationMeta::default());
                    self.needs_serde = true;
                    let target = self.scalar_target(&other_kind, &hint);
                    let validator = self.alias_validator(name, effective);
                    self.defs.push(Def::TypeAlias(AliasDef {
                        name: name.to_owned(),
                        docs,
                        target,
                        validator,
                    }));
                }
            },
        }
    }

    /// Intersected scalars render as ONE validated type carrying every check
    /// as documentation (companion §4.1, main spec §50 test 51); intersected
    /// homogeneous enums still need a nominal enum definition. Constrained
    /// scalar intersections additionally emit their free validator.
    fn define_intersected_scalar(
        &mut self,
        effective: SchemaId,
        name: &str,
        scalar: &IntersectedScalar,
    ) {
        let validation = self.validation_of(effective);
        let docs = self.definition_docs(effective, &[], &validation);
        match scalar.base_kind.clone() {
            SchemaKind::Enum { values } => self.define_enum(effective, name, values),
            other_kind => {
                let hint = name.to_owned();
                self.needs_serde = true;
                let target = self.scalar_target(&other_kind, &hint);
                let validator = self.alias_validator(name, effective);
                self.defs.push(Def::TypeAlias(AliasDef {
                    name: name.to_owned(),
                    docs,
                    target,
                    validator,
                }));
            }
        }
    }

    fn define_enum(&mut self, effective: SchemaId, name: &str, values: EnumValues) {
        let docs = self.definition_docs(effective, &[], &ValidationMeta::default());
        match values {
            EnumValues::Strings(constants) => {
                let mut used_variants = BTreeSet::new();
                let variants = constants
                    .iter()
                    .map(|constant| {
                        let rust_name = unique_in(
                            &mut used_variants,
                            &naming::ident(constant, NameStyle::Pascal),
                        );
                        Variant {
                            rename: (rust_name != *constant).then(|| constant.clone()),
                            rust_name,
                            payload: None,
                        }
                    })
                    .collect();
                self.needs_serde = true;
                self.defs.push(Def::StringsEnum(EnumDef {
                    name: name.to_owned(),
                    docs,
                    variants,
                }));
            }
            EnumValues::Integers(constants) => {
                let mut used_variants = BTreeSet::new();
                let variants = constants
                    .iter()
                    .map(|constant| {
                        (
                            unique_in(&mut used_variants, &integer_label(*constant)),
                            *constant,
                        )
                    })
                    .collect();
                self.needs_serde = true;
                self.defs.push(Def::IntegersEnum(IntegersEnumDef {
                    name: name.to_owned(),
                    docs,
                    variants,
                    constants,
                }));
            }
            EnumValues::MixedFallback(constants) => {
                let note = "Mixed-type enumeration (companion §4.3): typed variants for scalar \
                            constants plus a trailing `Other` catch-all matched last under \
                            `#[serde(untagged)]`; null and non-scalar constants fold into \
                            `Other`, whose identity Phase 2 validators enforce."
                    .to_owned();
                let mut docs = docs;
                docs.push(note);
                let variants = self.mixed_variants(&constants);
                self.needs_serde = true;
                self.defs.push(Def::MixedEnum(EnumDef {
                    name: name.to_owned(),
                    docs,
                    variants,
                }));
            }
        }
    }

    fn mixed_variants(&mut self, constants: &[JsonValue]) -> Vec<Variant> {
        let mut used_variants = BTreeSet::new();
        let mut variants = Vec::with_capacity(constants.len() + 1);
        for constant in constants {
            let variant = match constant {
                JsonValue::String(text) => Variant {
                    rust_name: unique_in(
                        &mut used_variants,
                        &naming::ident(text, NameStyle::Pascal),
                    ),
                    rename: None,
                    payload: Some("String"),
                },
                JsonValue::Number(number) => {
                    let label = number
                        .as_i64()
                        .map_or_else(|| format!("F{}", number), integer_label);
                    let payload = if number.is_i64() || number.is_u64() {
                        "i64"
                    } else {
                        "f64"
                    };
                    Variant {
                        rust_name: unique_in(&mut used_variants, &label),
                        rename: None,
                        payload: Some(payload),
                    }
                }
                JsonValue::Bool(flag) => Variant {
                    rust_name: unique_in(&mut used_variants, if *flag { "True" } else { "False" }),
                    rename: None,
                    payload: Some("bool"),
                },
                JsonValue::Null => Variant {
                    rust_name: unique_in(&mut used_variants, "Null"),
                    rename: None,
                    payload: None,
                },
                JsonValue::Array(_) | JsonValue::Object(_) => continue,
            };
            variants.push(variant);
        }
        variants.push(Variant {
            rust_name: unique_in(&mut used_variants, "Other"),
            rename: None,
            payload: Some("serde_json::Value"),
        });
        variants
    }

    /// Shared doc-comment assembly for named definitions: description,
    /// nullable note, binary-string warning, constraint metadata, default.
    fn definition_docs(
        &self,
        effective: SchemaId,
        extra: &[String],
        validation: &ValidationMeta,
    ) -> Vec<String> {
        let node = self.doc.arena.get(effective);
        let resolved = self.doc.resolution(effective);
        let mut lines = description_lines(&node.description.clone());
        lines.extend_from_slice(extra);
        if resolved.nullable {
            lines.push(NULLABLE_NOTE.to_owned());
        }
        if is_binary_string(self.doc, effective) {
            lines.push(BINARY_WARNING.to_owned());
        }
        lines.extend(validation_lines(validation));
        if let Some(default) = &node.default {
            let json = serde_json::to_string(default).unwrap_or_else(|_| "null".to_owned());
            lines.push(format!("Default: `{json}`."));
        }
        lines
    }

    fn push_fallback(&mut self, name: &str, docs: Vec<String>) {
        self.needs_serde = true;
        self.defs.push(Def::FallbackNewtype(FallbackDef {
            name: name.to_owned(),
            docs,
        }));
    }

    /// One struct from object properties plus the additionalProperties policy
    /// (companion §4.4): Deny → `deny_unknown_fields`, Ignore → nothing,
    /// Schema(edge) → trailing flattened `BTreeMap` (D-impl-flatten-map-
    /// deterministic). Deny and flatten can never co-occur: the policy is a
    /// single enum value and normalization treats deny ∩ schema-valued as
    /// unrepresentable. When the object (or its subtree) carries bucket-2
    /// constraints, the rendered `validate_request` body rides along.
    fn build_struct(
        &mut self,
        name: &str,
        docs: Vec<String>,
        properties: &[PropertyIr],
        additional: AdditionalPropertiesPolicy,
        object_validation: &ValidationMeta,
    ) -> Def {
        debug_assert!(
            !matches!(additional, AdditionalPropertiesPolicy::Deny)
                || !matches!(additional, AdditionalPropertiesPolicy::Schema(_)),
            "deny and schema-valued additionalProperties cannot co-occur"
        );
        let deny_unknown_fields = matches!(additional, AdditionalPropertiesPolicy::Deny);

        let struct_name = name.to_owned();
        let counting = object_validation.min_properties.is_some()
            || object_validation.max_properties.is_some();
        let mut fields = Vec::with_capacity(properties.len() + 1);
        let mut used_field_names = BTreeSet::new();
        let mut validation_body = Vec::new();
        for property in properties {
            let built = self.build_field(&struct_name, property, &mut used_field_names, counting);
            validation_body.extend(built.checks);
            fields.push(built.field);
        }
        let additional_ident = if let AdditionalPropertiesPolicy::Schema(edge) = additional {
            self.needs_btree_map = true;
            let value_type = self.edge_type(edge, &format!("{name}Additional"));
            let ident = unique_in(&mut used_field_names, "additional");
            fields.push(Field {
                docs: Vec::new(),
                attrs: vec!["#[serde(flatten)]".to_owned()],
                name: ident.clone(),
                ty: format!("BTreeMap<String, {value_type}>"),
            });
            if counting {
                // v1 counts schema-valued map entries toward the property
                // count; the VALUE schemas stay metadata-only (documented).
                validation_body.push(format!("{METHOD_PAD}property_count += self.{ident}.len();"));
            }
            Some(ident)
        } else {
            None
        };
        if counting {
            validation_body.extend(
                CheckCall {
                    callee: "::openapi_support::validation::validate_object_props".to_owned(),
                    args: vec![
                        String::from("property_count"),
                        option_u64(object_validation.min_properties),
                        option_u64(object_validation.max_properties),
                    ],
                    field: None,
                }
                .render(2),
            );
        }
        let _ = additional_ident;

        Def::Struct(StructDef {
            name: struct_name,
            docs,
            deny_unknown_fields,
            fields,
            validation_body: wrap_property_count_intro(validation_body, counting),
        })
    }

    /// One property through the companion §2.1 matrix cell-for-cell:
    /// requiredness from [`PropertyIr::required`], nullability from the
    /// referenced node's resolution — plus the runtime-validation checks of
    /// companion §9 when the referenced shape carries any.
    fn build_field(
        &mut self,
        parent: &str,
        property: &PropertyIr,
        used_field_names: &mut BTreeSet<String>,
        counting: bool,
    ) -> BuiltField {
        let effective = self.chase(property.schema.target);
        let nullable = self.nullable_of(effective);
        let hint = format!(
            "{parent}{}",
            naming::ident(&property.wire_name, NameStyle::Pascal)
        );
        let base = self.edge_type(property.schema, &hint);
        let boxed = matches!(property.schema.indirection, Indirection::Boxed);

        let (ty, cell_attr) = match (property.required, nullable) {
            (true, false) => (base, None),
            (true, true) => (format!("Option<{base}>"), Some(REQUIRED_NULLABLE_ATTR)),
            (false, false) => {
                self.needs_optional_field = true;
                (
                    format!("OptionalField<{base}>"),
                    Some(OPTIONAL_NON_NULLABLE_ATTR),
                )
            }
            (false, true) => (format!("Option<{base}>"), Some(OPTIONAL_NULLABLE_ATTR)),
        };

        // snake_case through the naming pipeline; intra-struct collisions get
        // numeric suffixes by declaration order (companion §10).
        let mut attrs = Vec::new();
        let field_name = {
            let base = naming::ident(&property.wire_name, NameStyle::Snake);
            let mut candidate = base.clone();
            let mut counter = 1_u32;
            while !used_field_names.insert(candidate.clone()) {
                counter += 1;
                candidate = naming::sanitize_joined(&format!("{base}_{counter}"));
            }
            candidate
        };
        if field_name != property.wire_name {
            attrs.push(format!(
                "#[serde(rename = \"{}\")]",
                escape_string(&property.wire_name)
            ));
        }
        if let Some(attr) = cell_attr {
            attrs.push(attr.to_owned());
        }

        let node = self.doc.arena.get(effective);
        let mut docs = description_lines(&node.description.clone());
        if is_binary_string(self.doc, effective) {
            docs.push(BINARY_WARNING.to_owned());
        }
        docs.extend(validation_lines(&self.validation_of(effective)));
        if let Some(default) = &node.default {
            let json = serde_json::to_string(default).unwrap_or_else(|_| "null".to_owned());
            docs.push(format!("Default: `{json}`."));
        }

        // Runtime checks (companion §9): wrapped cells validate only their
        // present-inner value; plain cells validate directly.
        let wrapper = Wrapper::from_cell(property.required, nullable);
        let validation = self.validation_of(effective);
        let mut checks =
            self.field_check_lines(&field_name, effective, &validation, boxed, wrapper);
        if counting {
            let count_expr = match (property.required, nullable) {
                (true, false) => String::from("1_usize;"),
                (_, true) => format!("usize::from(self.{field_name}.is_some());"),
                (false, false) => {
                    format!("usize::from(matches!(self.{field_name}, OptionalField::Present(_)));")
                }
            };
            checks.push(format!("{METHOD_PAD}property_count += {count_expr}"));
        }

        BuiltField {
            field: Field {
                docs,
                attrs,
                name: field_name,
                ty,
            },
            checks,
        }
    }

    /// All statements for one object field: delegates to the shared
    /// [`ValidationStatements`] builders (companion §9), which both emitters
    /// use verbatim.
    fn field_check_lines(
        &self,
        field_name: &str,
        effective: SchemaId,
        validation: &ValidationMeta,
        boxed: bool,
        wrapper: Wrapper,
    ) -> Vec<String> {
        self.validation
            .field_check_lines(field_name, effective, validation, boxed, wrapper)
    }

    /// Free validator function for a constrained scalar alias component
    /// (the `Slug` case); `None` for everything else.
    fn alias_validator(&self, component_name: &str, effective: SchemaId) -> Option<AliasValidator> {
        let alias = self.analysis.scalar_alias(effective)?;
        let validation = self.doc.resolution(effective).validation.clone();
        // Free function bodies sit at indentation level one; the whole-alias
        // value carries no field label (routers add the body context).
        let mut body = match alias.kind {
            ScalarParamKind::Str => string_check_lines("value", &validation, None, 1),
            ScalarParamKind::Int => {
                number_check_lines("*value as f64", &validation.numeric, None, 1)
            }
            ScalarParamKind::Float => number_check_lines("*value", &validation.numeric, None, 1),
        };
        body.push("    Ok(())".to_owned());
        let param_ty = match alias.kind {
            ScalarParamKind::Str => "&str",
            ScalarParamKind::Int => "&i64",
            ScalarParamKind::Float => "&f64",
        };
        Some(AliasValidator {
            fn_name: alias.fn_name.clone(),
            param_ty,
            docs: vec![format!(
                "Server-side request validation (companion §9) for the \
                 constrained scalar alias `{component_name}`: bucket-2 \
                 constraints enforced on server requests; client encoding \
                 stays lenient."
            )],
            body,
        })
    }
}

/// Shared runtime-validation statement builders (companion §9): the exact
/// bucket-2 check emission used by `models.rs`, exposed to the directional
/// view emitter so `<M>Write` views carry byte-identical validators keyed
/// by their surviving field lists (main spec §50 test 50 continuity).
pub(crate) struct ValidationStatements<'a> {
    doc: &'a NormalizedDocument,
    analysis: Analysis,
}

impl<'a> ValidationStatements<'a> {
    pub(crate) fn new(doc: &'a NormalizedDocument) -> Self {
        Self {
            doc,
            analysis: analyze(doc),
        }
    }

    fn chase(&self, id: SchemaId) -> SchemaId {
        self.doc.resolve_alias(id)
    }

    fn resolution_of(&self, id: SchemaId) -> &ResolvedKind {
        &self.doc.resolution(id).kind
    }

    fn validation_of(&self, id: SchemaId) -> ValidationMeta {
        self.doc.resolution(id).validation.clone()
    }

    /// All statements for one object field: scalar constraints, array
    /// cardinality/uniqueness/items/contains, recursion into validated
    /// composites — each guarded by the presence wrapper when the matrix
    /// cell wraps the value. Direct statements render at method-body level
    /// two; wrapped statements at level three inside their `if let` head.
    pub(crate) fn field_check_lines(
        &self,
        field_name: &str,
        effective: SchemaId,
        validation: &ValidationMeta,
        boxed: bool,
        wrapper: Wrapper,
    ) -> Vec<String> {
        const LEVEL: usize = 2;
        let direct_string = format!("&self.{field_name}");
        let direct_numeric = format!("(self.{field_name}) as f64");
        let direct_float = format!("self.{field_name}");
        // Array helpers take owned-method receivers (`self.f.len()`), never
        // a leading reference (companion §9 emission contract).
        let direct_array = format!("self.{field_name}");

        let base_kind = self.effective_base_kind(effective);
        let mut lines: Vec<String> = Vec::new();
        let numeric_declared =
            validation.numeric != crate::ir::schema::NumericValidation::default();
        // Rejection details name the generated Rust field (companion §9).
        let label = field_name.to_owned();
        let some_label = Some(label.as_str());
        match (wrapper, base_kind.as_ref()) {
            (Wrapper::Direct, Some(SchemaKind::String_ { binary: false, .. })) => lines.extend(
                string_check_lines(&direct_string, validation, some_label, LEVEL),
            ),
            (Wrapper::Direct, Some(SchemaKind::Integer { .. })) if numeric_declared => {
                lines.extend(number_check_lines(
                    &direct_numeric,
                    &validation.numeric,
                    some_label,
                    LEVEL,
                ));
            }
            (Wrapper::Direct, Some(SchemaKind::Number { .. })) if numeric_declared => {
                lines.extend(number_check_lines(
                    &direct_float,
                    &validation.numeric,
                    some_label,
                    LEVEL,
                ));
            }
            (Wrapper::Direct, Some(SchemaKind::Array { items })) => {
                lines.extend(self.array_check_lines(
                    &direct_array,
                    *items,
                    validation,
                    some_label,
                    LEVEL,
                ));
            }
            (Wrapper::OptionalField, Some(SchemaKind::String_ { binary: false, .. })) => {
                lines.extend(string_check_lines("value", validation, some_label, 3));
            }
            (Wrapper::Option | Wrapper::OptionalField, Some(SchemaKind::Integer { .. }))
                if numeric_declared =>
            {
                lines.extend(number_check_lines(
                    "*value as f64",
                    &validation.numeric,
                    some_label,
                    3,
                ));
            }
            (Wrapper::Option | Wrapper::OptionalField, Some(SchemaKind::Number { .. }))
                if numeric_declared =>
            {
                lines.extend(number_check_lines(
                    "*value",
                    &validation.numeric,
                    some_label,
                    3,
                ));
            }
            (Wrapper::OptionalField, Some(SchemaKind::Array { items })) => {
                lines.extend(self.array_check_lines("value", *items, validation, some_label, 3));
            }
            _ => {}
        }
        // Recurse into validated composite children (Box derefs implicitly).
        if self.is_validated_composite(effective) {
            let call_target = match wrapper {
                Wrapper::Direct if boxed => format!("(*self.{field_name})."),
                Wrapper::Direct => format!("self.{field_name}."),
                Wrapper::Option | Wrapper::OptionalField => "value.".to_owned(),
            };
            let level = if wrapper == Wrapper::Direct { LEVEL } else { 3 };
            lines.extend(annotated_call(
                &format!("{call_target}validate_request()"),
                some_label,
                level,
            ));
        }
        if lines.is_empty() || wrapper == Wrapper::Direct {
            return lines;
        }
        let head = match wrapper {
            Wrapper::Direct => unreachable!("handled above"),
            Wrapper::Option => format!("if let Some(value) = self.{field_name}.as_ref() {{"),
            Wrapper::OptionalField => {
                format!("if let OptionalField::Present(value) = &self.{field_name} {{")
            }
        };
        let mut guarded = vec![format!("{METHOD_PAD}{head}")];
        guarded.extend(lines);
        guarded.push(format!("{METHOD_PAD}}}"));
        guarded
    }

    /// Array-field statements: length bounds, uniqueness (string/number
    /// elements only, documented v1 limit), per-item checks, and the
    /// `contains` match-count block when representable.
    fn array_check_lines(
        &self,
        accessor: &str,
        items: SchemaEdge,
        validation: &ValidationMeta,
        field: Option<&str>,
        level: usize,
    ) -> Vec<String> {
        // Normalize the receiver: helpers take method-call form
        // (`self.f.len()`), the item loop borrows via `.iter()` so both
        // direct (`self.f`, behind `&self`) and already-borrowed (`value`,
        // bound as `&Vec<T>`) receivers iterate as `&T` without `&&Vec<T>`.
        let base = accessor.strip_prefix('&').unwrap_or(accessor);
        let mut lines = Vec::new();
        if validation.min_items.is_some() || validation.max_items.is_some() {
            lines.extend(
                CheckCall {
                    callee: "::openapi_support::validation::validate_array_len".to_owned(),
                    args: vec![
                        format!("{base}.len()"),
                        format!(
                            "&::openapi_support::validation::ArrayConstraints {{\n    \
                             min_items: {},\n    max_items: {},\n}}",
                            option_u64(validation.min_items),
                            option_u64(validation.max_items)
                        ),
                    ],
                    field: field.map(ToOwned::to_owned),
                }
                .render(level),
            );
        }

        let element = self.chase(items.target);
        let element_kind = self.effective_base_kind(element);
        let unique_kind = match element_kind.as_ref() {
            Some(SchemaKind::String_ { binary: false, .. }) => UniqueKind::Strings,
            Some(SchemaKind::Integer { .. } | SchemaKind::Number { .. }) => UniqueKind::Numbers,
            _ => UniqueKind::None,
        };
        match unique_kind {
            UniqueKind::None => {}
            UniqueKind::Strings => {
                lines.extend(
                    CheckCall {
                        callee: "::openapi_support::validation::require_unique_strings".to_owned(),
                        args: vec![format!("{base}.iter()")],
                        field: field.map(ToOwned::to_owned),
                    }
                    .render(level),
                );
            }
            UniqueKind::Numbers => {
                let map = match element_kind.as_ref() {
                    Some(SchemaKind::Number { .. }) => ".iter().copied()",
                    _ => ".iter().map(|value| *value as f64)",
                };
                lines.extend(
                    CheckCall {
                        callee: "::openapi_support::validation::require_unique_numbers".to_owned(),
                        args: vec![format!("{base}{map}")],
                        field: field.map(ToOwned::to_owned),
                    }
                    .render(level),
                );
            }
        }

        // Per-item statements: the item's OWN metadata (works through alias
        // chasing even when the element type renders inline). Rejection
        // details label elements `<field>[*]`.
        let item_label = field.map(|field| format!("{field}[*]"));
        let item_lines = self.element_item_lines(element, item_label.as_deref(), level + 1);
        let loop_head = format!("for item in {base}.iter() {{");
        if !item_lines.is_empty() {
            lines.push(format!("{}{loop_head}", "    ".repeat(level)));
            lines.extend(item_lines);
            lines.push(format!("{}}}", "    ".repeat(level)));
        }

        if let Some(contains_edge) = validation.contains {
            if let Some(count_block) =
                self.contains_block(&loop_head, element_kind.as_ref(), contains_edge, level)
            {
                lines.extend(count_block);
            }
        }
        lines
    }

    /// Item-level check statements for one array ELEMENT schema (`item` is
    /// the loop binding).
    fn element_item_lines(
        &self,
        element: SchemaId,
        field: Option<&str>,
        level: usize,
    ) -> Vec<String> {
        let validation = self.validation_of(element);
        match self.effective_base_kind(element).as_ref() {
            Some(SchemaKind::String_ { binary: false, .. }) => {
                string_check_lines("item", &validation, field, level)
            }
            Some(SchemaKind::Integer { .. }) => {
                number_check_lines("*item as f64", &validation.numeric, field, level)
            }
            Some(SchemaKind::Number { .. }) => {
                number_check_lines("*item", &validation.numeric, field, level)
            }
            _ => {
                if self.is_validated_composite(element) {
                    annotated_call("item.validate_request()", field, level)
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The `contains` block: counts elements satisfying the CONTAINS
    /// schema's own scalar constraints (v1: only same-family scalar contains
    /// schemas are decidable; anything else skips, documented).
    fn contains_block(
        &self,
        loop_head: &str,
        element_kind: Option<&SchemaKind>,
        contains_edge: SchemaEdge,
        level: usize,
    ) -> Option<Vec<String>> {
        let contains_effective = self.chase(contains_edge.target);
        let validation = self.validation_of(contains_effective);
        let contains_kind = self.effective_base_kind(contains_effective)?;
        // `format` enforces at runtime on string shapes only (companion §9
        // v1 policy); numeric formats are type-shaping at decode.
        let format_checked = matches!(contains_kind, SchemaKind::String_ { binary: false, .. });
        if !enforceable_with(format_checked, &validation) {
            return None;
        }
        let same_family = matches!(
            (&contains_kind, element_kind),
            (
                SchemaKind::String_ { binary: false, .. },
                Some(SchemaKind::String_ { binary: false, .. }),
            ) | (SchemaKind::Integer { .. }, Some(SchemaKind::Integer { .. }),)
                | (SchemaKind::Number { .. }, Some(SchemaKind::Number { .. }),)
        );
        if !same_family {
            return None;
        }
        let pad = "    ".repeat(level);
        let inner = format!("{pad}    ");
        let inner2 = format!("{inner}    ");
        // Predicate errors stay UNANNOTATED: the verdict feeds is_ok() and
        // the aggregate count reports through validate_contains_count.
        let (param_ty, mut predicate) = match &contains_kind {
            SchemaKind::Integer { .. } => (
                "&i64",
                number_check_lines("*item as f64", &validation.numeric, None, level + 2),
            ),
            SchemaKind::Number { .. } => (
                "&f64",
                number_check_lines("*item", &validation.numeric, None, level + 2),
            ),
            _ => (
                "&String",
                string_check_lines("item", &validation, None, level + 2),
            ),
        };
        // The last statement becomes the closure's tail expression so the
        // call site can branch on `is_ok()`.
        if let Some(last) = predicate.last_mut() {
            *last = last.trim_end().trim_end_matches(';').to_owned();
        }
        // minContains defaults to 1 when the document omitted it.
        let min = option_u64(validation.min_contains.or(Some(1)));
        let max = option_u64(validation.max_contains);
        let mut lines = Vec::new();
        lines.push(format!("{pad}{{"));
        lines.push(format!("{inner}let item_matches = |item: {param_ty}| {{"));
        lines.extend(predicate);
        lines.push(format!("{inner}}};"));
        lines.push(format!("{inner}let mut matched = 0_usize;"));
        lines.push(format!("{inner}{loop_head}"));
        lines.push(format!("{inner2}if item_matches(item).is_ok() {{"));
        lines.push(format!("{inner2}matched += 1;"));
        lines.push(format!("{inner2}}}"));
        lines.push(format!("{inner}}}"));
        lines.extend(
            CheckCall {
                callee: "::openapi_support::validation::validate_contains_count".to_owned(),
                args: vec!["matched".to_owned(), min, max],
                field: None,
            }
            .render(level + 1),
        );
        lines.push(format!("{pad}}}"));
        Some(lines)
    }

    /// Base [`SchemaKind`] after composition resolution; composite verdicts
    /// come from the resolution itself, not this view.
    pub(crate) fn effective_base_kind(&self, effective: SchemaId) -> Option<SchemaKind> {
        match self.resolution_of(effective).clone() {
            ResolvedKind::IntersectedScalar(scalar) => Some(scalar.base_kind),
            ResolvedKind::RawValueFallback(_) | ResolvedKind::Alias(_) => None,
            ResolvedKind::MergedObject(_) | ResolvedKind::ClosedEnum(_) => None,
            ResolvedKind::Plain => Some(self.doc.arena.get(effective).kind.clone()),
        }
    }

    /// Composite shapes that emit a `validate_request` method.
    pub(crate) fn is_validated_composite(&self, effective: SchemaId) -> bool {
        let composite = matches!(
            self.resolution_of(effective),
            ResolvedKind::MergedObject(_) | ResolvedKind::ClosedEnum(_)
        ) || matches!(self.resolution_of(effective), ResolvedKind::Plain)
            && matches!(
                self.doc.arena.get(effective).kind,
                SchemaKind::Object { .. }
            );
        composite && self.analysis.has_validator(effective)
    }
}

/// Presence wrapper of one field in the generated struct, deciding both the
/// accessor expressions and the `if let` guard of its checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wrapper {
    /// Required + non-nullable: validate the field directly.
    Direct,
    /// Required + nullable or optional + nullable (`Option<T>`).
    Option,
    /// Optional + non-nullable (`OptionalField<T>`).
    OptionalField,
}

impl Wrapper {
    pub(crate) fn from_cell(required: bool, nullable: bool) -> Self {
        match (required, nullable) {
            (true, false) => Self::Direct,
            (false, false) => Self::OptionalField,
            (_, true) => Self::Option,
        }
    }
}

/// A built struct field plus its runtime-validation statements.
struct BuiltField {
    field: Field,
    checks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UniqueKind {
    None,
    Strings,
    Numbers,
}

/// Emits the `let mut property_count = 0_usize;` prologue ahead of the
/// accumulated `+=` contributions when the object declares property-count
/// bounds.
pub(crate) fn wrap_property_count_intro(body: Vec<String>, counting: bool) -> Vec<String> {
    if !counting || body.is_empty() {
        return body;
    }
    let mut out = vec![format!("{METHOD_PAD}let mut property_count = 0_usize;")];
    out.extend(body);
    out
}

// ----------------------------------------------------------------------
// Type expressions
// ----------------------------------------------------------------------

impl Generator<'_> {
    /// Type expression for one edge: referenced type path with heap
    /// indirection applied (`Box<T>` when the cycle-precise pass flagged the
    /// property edge, D-impl-boxing). Nullability wrapping belongs to the
    /// matrix cells at property sites.
    fn edge_type(&mut self, edge: SchemaEdge, hint: &str) -> String {
        let inner = self.reference_type(edge.target, hint);
        match edge.indirection {
            Indirection::Boxed => format!("Box<{inner}>"),
            Indirection::None => inner,
        }
    }

    /// Rust type naming the (alias-chased) node `id`: named components use
    /// their assigned names; anonymous composites that need nominal types get
    /// registered on first encounter; everything else renders inline.
    fn reference_type(&mut self, id: SchemaId, hint: &str) -> String {
        let effective = self.chase(id);
        if let Some(name) = self.named.get(&effective.0) {
            return name.clone();
        }
        if let Some(name) = self.anonymous.get(&effective.0) {
            return name.clone();
        }
        let needs_definition = match self.resolution_of(effective) {
            ResolvedKind::MergedObject(_) | ResolvedKind::ClosedEnum(_) => true,
            ResolvedKind::IntersectedScalar(scalar) => {
                matches!(scalar.base_kind, SchemaKind::Enum { .. })
            }
            // Raw/value fallbacks at anonymous sites stay bare serde_json::Value.
            ResolvedKind::RawValueFallback(_) | ResolvedKind::Alias(_) => false,
            ResolvedKind::Plain => matches!(
                self.doc.arena.get(effective).kind,
                SchemaKind::Object { .. } | SchemaKind::Enum { .. }
            ),
        };
        if !needs_definition {
            return self.inline_target(effective, hint);
        }
        self.register_anonymous(effective, hint)
    }

    fn inline_target(&mut self, effective: SchemaId, hint: &str) -> String {
        let kind = match self.resolution_of(effective).clone() {
            ResolvedKind::IntersectedScalar(scalar) => scalar.base_kind,
            _ => self.doc.arena.get(effective).kind.clone(),
        };
        self.scalar_target(&kind, hint)
    }

    /// Inline-expressible targets: primitives, arrays, tuples, free-form maps,
    /// raw JSON.
    fn scalar_target(&mut self, kind: &SchemaKind, hint: &str) -> String {
        match kind {
            SchemaKind::Boolean => "bool".to_owned(),
            SchemaKind::Integer { format } => match format.as_deref() {
                Some("int32") => "i32".to_owned(),
                _ => "i64".to_owned(),
            },
            SchemaKind::Number { .. } => "f64".to_owned(),
            SchemaKind::String_ { .. } => "String".to_owned(),
            SchemaKind::FreeFormObject => "serde_json::Map<String, serde_json::Value>".to_owned(),
            SchemaKind::Array { items } => {
                let element = self.edge_type(*items, &format!("{hint}Item"));
                format!("Vec<{element}>")
            }
            SchemaKind::Tuple { prefix_items, .. } => {
                let elements: Vec<String> = prefix_items
                    .iter()
                    .enumerate()
                    .map(|(index, edge)| self.edge_type(*edge, &format!("{hint}{index}")))
                    .collect();
                format!("({})", elements.join(", "))
            }
            // Unreachable through resolved graphs; defensive raw JSON.
            _ => "serde_json::Value".to_owned(),
        }
    }
}

// ----------------------------------------------------------------------
// Choice enums (proven oneOf/anyOf)
// ----------------------------------------------------------------------

impl Generator<'_> {
    /// Newtype variants over branch types; variant names come from branch
    /// component names with numeric suffixes on collision (companion §10).
    fn build_choice_enum(
        &mut self,
        name: &str,
        docs: Vec<String>,
        choice: &ClosedEnumChoice,
    ) -> Def {
        let mut used_variants = BTreeSet::new();
        let mut variants = Vec::with_capacity(choice.branches.len());
        let mut variant_validations = Vec::with_capacity(choice.branches.len());
        for (index, branch) in choice.branches.iter().enumerate() {
            let effective = self.chase(branch.target);
            let fallback_hint = format!("{name}Branch{}", index + 1);
            let base_variant_name = self
                .named
                .get(&effective.0)
                .cloned()
                .unwrap_or_else(|| fallback_hint.clone());
            let variant_name = unique_in(&mut used_variants, &base_variant_name);
            let nullable = self.nullable_of(effective);
            let payload_base = self.edge_type(*branch, &fallback_hint);
            let payload = if nullable {
                format!("Option<{payload_base}>")
            } else {
                payload_base
            };
            // Validate through branch payloads that carry validators
            // (recursion; raw/value fallbacks never do, documented).
            let validation = if self.analysis.has_validator(effective) {
                Some(nullable)
            } else {
                None
            };
            variants.push((variant_name, payload));
            variant_validations.push(validation);
        }
        Def::ChoiceEnum(ChoiceEnumDef {
            name: name.to_owned(),
            docs,
            variants,
            variant_validations,
        })
    }
}

fn integer_label(constant: i64) -> String {
    if constant >= 0 {
        format!("V{constant}")
    } else {
        format!("Neg{}", constant.abs())
    }
}

/// First occurrence keeps the clean name; later collisions get `_2`, `_3`, …
/// ordered by document position (companion §10).
fn unique_in(used: &mut BTreeSet<String>, base: &str) -> String {
    let sanitized = naming::sanitize_joined(base);
    let mut candidate = sanitized.clone();
    let mut counter = 1_u32;
    while !used.insert(candidate.clone()) {
        counter += 1;
        candidate = naming::sanitize_joined(&format!("{sanitized}_{counter}"));
    }
    candidate
}

/// Escapes a wire name for a double-quoted Rust string literal.
fn escape_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if control.is_control() => {
                out.push_str(&format!("\\u{{{:x}}}", control as u32))
            }
            other => out.push(other),
        }
    }
    out
}

// ----------------------------------------------------------------------
// Doc-comment helpers (deterministic fixed ordering)
// ----------------------------------------------------------------------

/// Splits a schema description into doc lines without trailing whitespace.
fn description_lines(description: &Option<String>) -> Vec<String> {
    let Some(text) = description else {
        return Vec::new();
    };
    text.split('\n')
        .map(|line| line.trim_end_matches('\r').trim_end().to_owned())
        .collect()
}

/// True when the node resolves to a binary-marked string shape.
fn is_binary_string(doc: &NormalizedDocument, effective: SchemaId) -> bool {
    match doc.resolution(effective).kind.clone() {
        ResolvedKind::IntersectedScalar(scalar) => {
            matches!(scalar.base_kind, SchemaKind::String_ { binary: true, .. })
        }
        _ => matches!(
            doc.arena.get(effective).kind,
            SchemaKind::String_ { binary: true, .. }
        ),
    }
}

/// Bucket-2 validation metadata rendered as documentation (D-§2 bucket 2;
/// enforced on server requests at runtime since the Phase 2 validation
/// package, D-impl-runtime-validation-timing).
pub fn validation_lines(validation: &ValidationMeta) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pattern) = &validation.pattern {
        parts.push(format!("pattern `{pattern}`"));
    }
    if let Some(min_length) = validation.min_length {
        parts.push(format!("minLength >= {min_length}"));
    }
    if let Some(max_length) = validation.max_length {
        parts.push(format!("maxLength <= {max_length}"));
    }
    let numeric = &validation.numeric;
    if let Some(minimum) = numeric.minimum {
        parts.push(format!("minimum >= {minimum}"));
    }
    if let Some(maximum) = numeric.maximum {
        parts.push(format!("maximum <= {maximum}"));
    }
    if let Some(exclusive_minimum) = numeric.exclusive_minimum {
        parts.push(format!("exclusiveMinimum > {exclusive_minimum}"));
    }
    if let Some(exclusive_maximum) = numeric.exclusive_maximum {
        parts.push(format!("exclusiveMaximum < {exclusive_maximum}"));
    }
    if let Some(multiple_of) = numeric.multiple_of {
        parts.push(format!("multipleOf {multiple_of}"));
    }
    if let Some(min_items) = validation.min_items {
        parts.push(format!("minItems >= {min_items}"));
    }
    if let Some(max_items) = validation.max_items {
        parts.push(format!("maxItems <= {max_items}"));
    }
    if validation.unique_items {
        parts.push("uniqueItems".to_owned());
    }
    if let Some(min_properties) = validation.min_properties {
        parts.push(format!("minProperties >= {min_properties}"));
    }
    if let Some(max_properties) = validation.max_properties {
        parts.push(format!("maxProperties <= {max_properties}"));
    }
    if !validation.pattern_properties.is_empty() {
        parts.push(format!(
            "patternProperties x{}",
            validation.pattern_properties.len()
        ));
    }
    if validation.contains.is_some() {
        let mut contains = "contains".to_owned();
        if let Some(min_contains) = validation.min_contains {
            contains.push_str(&format!(" (min {min_contains}"));
            if let Some(max_contains) = validation.max_contains {
                contains.push_str(&format!(", max {max_contains}"));
            }
            contains.push(')');
        } else if let Some(max_contains) = validation.max_contains {
            contains.push_str(&format!(" (max {max_contains})"));
        }
        parts.push(contains);
    }
    if let Some(encoding) = &validation.content_encoding {
        parts.push(format!("contentEncoding `{encoding}`"));
    }
    if let Some(media_type) = &validation.content_media_type {
        parts.push(format!("contentMediaType `{media_type}`"));
    }
    if let Some(format) = &validation.format {
        parts.push(format!("format `{format}`"));
    }
    if !validation.examples.is_empty() {
        parts.push(format!("examples x{}", validation.examples.len()));
    }
    if parts.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "Constraints (enforced by generated routers on server requests, \
         companion §9; lenient on client decode): {}.",
        parts.join("; ")
    )]
}

/// Discriminator metadata as a doc comment; inspect-select-validate decode
/// routing is a later package (companion §4.2).
fn discriminator_docs(discriminator: &DiscriminatorIr) -> Vec<String> {
    let mut lines = vec![format!(
        "Discriminator (routing hint only; inspect-select-validate decode arrives in a later \
         package): property `{}`.",
        discriminator.property_name
    )];
    if !discriminator.mapping.is_empty() {
        let entries: Vec<String> = discriminator
            .mapping
            .iter()
            .map(|(value, target)| format!("{value} -> {}", target.0))
            .collect();
        lines.push(format!("Mapping: {}.", entries.join(", ")));
    }
    lines
}

/// Why a composition fell back to raw/value (companion §4.1/§4.2,
/// D-impl-oneoffallback).
fn fallback_reason_docs(reason: FallbackReason) -> Vec<String> {
    let why = match reason {
        FallbackReason::UnrepresentableAllOf => {
            "the allOf members cannot be represented losslessly as one typed model \
             (unrepresentable-all-of)"
        }
        FallbackReason::UnprovenOneOf => {
            "mutual exclusivity of the oneOf branches could not be proven statically \
             (unproven-one-of)"
        }
        FallbackReason::UnprovenAnyOf => {
            "mutual exclusivity of the anyOf branches could not be proven statically and \
             choose-one enums are forbidden without proof (unproven-any-of)"
        }
    };
    vec![format!(
        "Raw/value fallback carrying retained validation metadata (companion §4.2, \
         DECISIONS.md D-impl-oneoffallback): {why}; exactly-one semantics stay exact at \
         the JSON level."
    )]
}

fn unsupported_reason_docs(reason: &UnsupportedReason) -> Vec<String> {
    let why = match reason {
        UnsupportedReason::MixedTypeArray => "multiple non-null entries in a 3.1 `type` array",
        UnsupportedReason::UnevaluatedKeywordsActive => {
            "active unevaluatedProperties/unevaluatedItems"
        }
        UnsupportedReason::AnchorRef => "$anchor/$dynamicRef or non-empty $id rebasing",
        UnsupportedReason::RemoteRefUnfetched => "remote http(s) reference (never fetched)",
        UnsupportedReason::UnbrokenSelfContainment => {
            "cycle without an intervening container break"
        }
        UnsupportedReason::InlineExpansionDepthExceeded => "inline nesting exceeded the depth cap",
        UnsupportedReason::Other(detail) => detail,
    };
    vec![format!(
        "Raw/value fallback: {why}; represented as `serde_json::Value`."
    )]
}

// ----------------------------------------------------------------------
// Emission pass
// ----------------------------------------------------------------------

fn render(generator: &Generator<'_>) -> String {
    let mut emitter = Emitter::new();
    // rustfmt drops blank lines directly after an inner doc-comment block, so
    // imports follow the crate docs immediately.
    emit_crate_docs(&mut emitter);

    if generator.needs_btree_map {
        emitter.line(0, "use std::collections::BTreeMap;");
    }
    if generator.needs_optional_field || generator.needs_serde {
        if generator.needs_btree_map {
            emitter.blank();
        }
        if generator.needs_optional_field {
            emitter.line(0, "use openapi_support::optional::OptionalField;");
        }
        if generator.needs_serde {
            emitter.line(0, "use serde::{Deserialize, Serialize};");
        }
    }
    // One blank line separates the header/imports from definitions. Avoid a
    // trailing blank line when a document has no components/schemas.
    if !generator.defs.is_empty() {
        emitter.blank();
    }

    for (index, def) in generator.defs.iter().enumerate() {
        if index > 0 {
            emitter.blank();
        }
        emit_def(&mut emitter, def);
    }
    emitter.finish()
}

fn emit_crate_docs(emitter: &mut Emitter) {
    let docs = [
        "Shared schema models generated from the OpenAPI document (main spec §2.6): one",
        "module reused by both client and server operation codecs.",
        "",
        "Every named `components/schemas` entry appears below in document declaration",
        "order; nested anonymous objects and enumerations become generated definitions",
        "emitted before their parents (`<Parent><FieldPascal>` plus numeric collision",
        "suffixes, companion §10).",
        "",
        "Property presence/nullability follows companion §2.1 cell-for-cell; bucket-2",
        "validation constraints ride as documentation and as emitted `validate_request`",
        "methods (companion §9; D-impl-runtime-validation-timing Phase 2 half). This file",
        "is generated deterministically byte-for-byte (main spec §50 test 39); do not edit",
        "by hand.",
    ];
    let owned: Vec<String> = docs.iter().map(|line| (*line).to_owned()).collect();
    emitter.inner_docs(0, &owned);
}

fn emit_def(emitter: &mut Emitter, def: &Def) {
    match def {
        Def::Struct(struct_def) => {
            emit_struct(emitter, struct_def);
            if !struct_def.validation_body.is_empty() {
                emitter.blank();
                emit_validate_request_impl(emitter, &struct_def.name, &struct_def.validation_body);
            }
        }
        Def::StringsEnum(enum_def) => emit_enum(emitter, enum_def, false),
        Def::MixedEnum(enum_def) => emit_enum(emitter, enum_def, true),
        Def::IntegersEnum(integers) => emit_integers_enum(emitter, integers),
        Def::ChoiceEnum(choice) => {
            emit_choice_enum(emitter, choice);
            if choice.variant_validations.iter().any(Option::is_some) {
                emitter.blank();
                emit_choice_enum_validator(emitter, choice);
            }
        }
        Def::TypeAlias(alias) => {
            emit_alias(emitter, alias);
            if let Some(validator) = &alias.validator {
                emitter.blank();
                emit_alias_validator(emitter, validator);
            }
        }
        Def::FallbackNewtype(fallback) => emit_fallback(emitter, fallback),
    }
}

/// The shared `validate_request` inherent method (companion §9): structural
/// checks stay in Serde decode; these enforce the D-§2 bucket-2 constraints.
pub(crate) fn emit_validate_request_impl(emitter: &mut Emitter, name: &str, body: &[String]) {
    emitter.line(0, &format!("impl {name} {{"));
    emitter.docs(
        1,
        &[
            String::from(
                "Server-side request validation (companion §9): structural \
                 checks stay in Serde decode; these enforce the D-§2",
            ),
            String::from("bucket-2 constraints. Client decoding stays lenient."),
        ],
    );
    emitter.line(1, "pub fn validate_request(");
    emitter.line(2, "&self,");
    emitter.line(
        1,
        ") -> ::std::result::Result<(), ::openapi_support::validation::Violation> {",
    );
    for line in body {
        emitter.line(0, line);
    }
    emitter.line(2, "Ok(())");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Validator over a proven-exclusive choice enum: recurses into branch
/// payloads that carry their own validators (raw/value fallback branches
/// never appear here — they are separate `<Type>Fallback` newtypes).
fn emit_choice_enum_validator(emitter: &mut Emitter, def: &ChoiceEnumDef) {
    emitter.line(0, &format!("impl {} {{", def.name));
    emitter.docs(
        1,
        &[
            String::from(
                "Server-side request validation (companion §9): each branch \
                 validates its payload when that branch type carries",
            ),
            String::from("constraints; nullable branches validate only when present."),
        ],
    );
    emitter.line(1, "pub fn validate_request(");
    emitter.line(2, "&self,");
    emitter.line(
        1,
        ") -> ::std::result::Result<(), ::openapi_support::validation::Violation> {",
    );
    emitter.line(2, "match self {");
    for ((variant_name, _payload), validation) in def.variants.iter().zip(&def.variant_validations)
    {
        match *validation {
            None => emitter.line(3, &format!("Self::{variant_name}(_) => {{}}")),
            Some(false) => {
                emitter.line(3, &format!("Self::{variant_name}(inner) => {{"));
                for line in annotated_call("inner.validate_request()", Some(variant_name), 4) {
                    emitter.line(0, &line);
                }
                emitter.line(3, "}");
            }
            Some(true) => {
                emitter.line(3, &format!("Self::{variant_name}(Some(inner)) => {{"));
                for line in annotated_call("inner.validate_request()", Some(variant_name), 4) {
                    emitter.line(0, &line);
                }
                emitter.line(3, "}");
                emitter.line(3, &format!("Self::{variant_name}(None) => {{}}"));
            }
        }
    }
    emitter.line(2, "}");
    emitter.line(2, "Ok(())");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// The free validator of one constrained scalar alias.
fn emit_alias_validator(emitter: &mut Emitter, validator: &AliasValidator) {
    emitter.docs(0, &validator.docs);
    emitter.line(0, &format!("pub fn {}(", validator.fn_name));
    emitter.line(1, &format!("value: {},", validator.param_ty));
    emitter.line(
        0,
        ") -> ::std::result::Result<(), ::openapi_support::validation::Violation> {",
    );
    for line in &validator.body {
        emitter.line(0, line);
    }
    emitter.line(0, "}");
}

fn emit_struct(emitter: &mut Emitter, def: &StructDef) {
    emitter.docs(0, &def.docs);
    emitter.line(0, DERIVES);
    if def.deny_unknown_fields {
        emitter.line(0, "#[serde(deny_unknown_fields)]");
    }
    if def.fields.is_empty() {
        let line = format!("pub struct {} {{}}", def.name);
        emitter.line(0, &line);
        return;
    }
    let header = format!("pub struct {} {{", def.name);
    emitter.line(0, &header);
    for field in &def.fields {
        emitter.docs(1, &field.docs);
        for attr in &field.attrs {
            emit_serde_attr(emitter, 1, attr);
        }
        let line = format!("pub {}: {},", field.name, field.ty);
        emitter.line(1, &line);
    }
    emitter.line(0, "}");
}

/// Emits one `#[serde(...)]` attribute in rustfmt-canonical shape: short
/// attributes stay on one line, over-wide ones wrap their arguments onto
/// continuation lines (no trailing comma, matching rustfmt's attribute
/// layout) so generated output stays rustfmt-clean (main spec §50 test 40).
fn emit_serde_attr(emitter: &mut Emitter, indent: usize, attr: &str) {
    if indent * 4 + attr.chars().count() <= RUSTFMT_MAX_WIDTH {
        emitter.line(indent, attr);
        return;
    }
    const HEAD: &str = "#[serde(";
    let Some(inner) = attr
        .strip_prefix(HEAD)
        .and_then(|rest| rest.strip_suffix(")]"))
    else {
        emitter.line(indent, attr);
        return;
    };
    emitter.line(indent, HEAD);
    for argument in split_attr_arguments(inner) {
        emitter.line(indent + 1, &argument);
    }
    emitter.line(indent, ")]");
}

/// Splits serde attribute arguments on top-level commas (quote-aware).
fn split_attr_arguments(inner: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for character in inner.chars() {
        match character {
            '\\' if in_quotes => {
                escaped = true;
                current.push(character);
            }
            '"' if !escaped => {
                in_quotes = !in_quotes;
                current.push(character);
            }
            ',' if !in_quotes => {
                arguments.push(current.trim_start().to_owned());
                current.clear();
            }
            _ => {
                escaped = false;
                current.push(character);
            }
        }
    }
    if !current.is_empty() {
        arguments.push(current.trim_start().to_owned());
    }
    arguments
}

fn emit_enum(emitter: &mut Emitter, def: &EnumDef, untagged: bool) {
    emitter.docs(0, &def.docs);
    emitter.line(0, DERIVES);
    if untagged {
        emitter.line(0, "#[serde(untagged)]");
    }
    let header = format!("pub enum {} {{", def.name);
    emitter.line(0, &header);
    for variant in &def.variants {
        if let Some(rename) = &variant.rename {
            let attr = format!("#[serde(rename = \"{}\")]", escape_string(rename));
            emitter.line(1, &attr);
        }
        let line = match variant.payload {
            Some(payload) => format!("{}({}),", variant.rust_name, payload),
            None => format!("{},", variant.rust_name),
        };
        emitter.line(1, &line);
    }
    emitter.line(0, "}");
}

fn emit_choice_enum(emitter: &mut Emitter, def: &ChoiceEnumDef) {
    emitter.docs(0, &def.docs);
    emitter.line(0, DERIVES);
    emitter.line(0, "#[serde(untagged)]");
    let header = format!("pub enum {} {{", def.name);
    emitter.line(0, &header);
    for (variant_name, payload) in &def.variants {
        let line = format!("{variant_name}({payload}),");
        emitter.line(1, &line);
    }
    emitter.line(0, "}");
}

fn emit_integers_enum(emitter: &mut Emitter, def: &IntegersEnumDef) {
    emitter.docs(0, &def.docs);
    emitter.line(0, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
    let header = format!("pub enum {} {{", def.name);
    emitter.line(0, &header);
    for (variant_name, constant) in &def.variants {
        let line = format!("{variant_name} = {constant},");
        emitter.line(1, &line);
    }
    emitter.line(0, "}");
    emitter.blank();

    let impl_header = format!("impl {} {{", def.name);
    emitter.line(0, &impl_header);
    let values_doc = ["Wire discriminants accepted by this enumeration.".to_owned()];
    emitter.docs(1, &values_doc);
    let constants: Vec<String> = def.constants.iter().map(ToString::to_string).collect();
    let values_line = format!(
        "pub const VALUES: &'static [i64] = &[{}];",
        constants.join(", ")
    );
    emitter.line(1, &values_line);
    emitter.line(0, "}");
    emitter.blank();

    let serialize_header = format!("impl serde::Serialize for {} {{", def.name);
    emitter.line(0, &serialize_header);
    emitter.line(
        1,
        "fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>",
    );
    emitter.line(1, "where");
    emitter.line(2, "S: serde::Serializer,");
    emitter.line(1, "{");
    emitter.line(
        2,
        "// Bare JSON numbers; derived unit variants would emit strings.",
    );
    emitter.line(2, "serializer.serialize_i64(*self as i64)");
    emitter.line(1, "}");
    emitter.line(0, "}");
    emitter.blank();

    let deserialize_header = format!("impl<'de> serde::Deserialize<'de> for {} {{", def.name);
    emitter.line(0, &deserialize_header);
    emitter.line(
        1,
        "fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>",
    );
    emitter.line(1, "where");
    emitter.line(2, "D: serde::Deserializer<'de>,");
    emitter.line(1, "{");
    emitter.line(
        2,
        "let raw = <i64 as serde::Deserialize>::deserialize(deserializer)?;",
    );
    emitter.line(2, "match raw {");
    for (variant_name, constant) in &def.variants {
        let arm = format!("{constant} => Ok(Self::{variant_name}),");
        emitter.line(3, &arm);
    }
    // The enum name and every expected discriminant are inlined into the
    // literal so the generated `format!` only captures the runtime value.
    let expected: Vec<String> = def.constants.iter().map(ToString::to_string).collect();
    let message = format!(
        "\"unknown discriminant {{other}} for enum `{}`, expected one of [{}]\"",
        def.name,
        expected.join(", ")
    );
    emitter.line(3, "other => Err(serde::de::Error::custom(format!(");
    emitter.line(4, &message);
    emitter.line(3, "))),");
    emitter.line(2, "}");
    emitter.line(1, "}");
    emitter.line(0, "}");
}

fn emit_alias(emitter: &mut Emitter, def: &AliasDef) {
    emitter.docs(0, &def.docs);
    let line = format!("pub type {} = {};", def.name, def.target);
    emitter.line(0, &line);
}

fn emit_fallback(emitter: &mut Emitter, def: &FallbackDef) {
    emitter.docs(0, &def.docs);
    emitter.line(0, DERIVES);
    let line = format!("pub struct {}(pub serde_json::Value);", def.name);
    emitter.line(0, &line);
}
