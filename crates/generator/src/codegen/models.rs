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
    // vice versa).
    let mut used_names: BTreeSet<String> = named.values().cloned().collect();
    for (_, enum_name) in &doc.names.response_enums {
        used_names.insert(enum_name.clone());
    }

    let mut generator = Generator {
        doc,
        named,
        anonymous: BTreeMap::new(),
        used_names,
        needs_optional_field: false,
        needs_btree_map: false,
        needs_serde: false,
        defs: Vec::new(),
    };
    for (schema, name) in components.iter().zip(names.iter()) {
        generator.define_component(schema, name);
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
}

struct AliasDef {
    name: String,
    docs: Vec<String>,
    target: String,
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
    defs: Vec<Def>,
}

impl<'a> Generator<'a> {
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
            self.defs.push(Def::TypeAlias(AliasDef {
                name: component_name.to_owned(),
                docs,
                target,
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
                let def = self.build_struct(name, docs, &properties, additional);
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
                    let def = self.build_struct(name, docs, &properties, additional);
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
                    }));
                }
                other_kind => {
                    let hint = name.to_owned();
                    let docs = self.definition_docs(effective, &[], &ValidationMeta::default());
                    self.needs_serde = true;
                    let target = self.scalar_target(&other_kind, &hint);
                    self.defs.push(Def::TypeAlias(AliasDef {
                        name: name.to_owned(),
                        docs,
                        target,
                    }));
                }
            },
        }
    }

    /// Intersected scalars render as ONE validated type carrying every check
    /// as documentation (companion §4.1, main spec §50 test 51); intersected
    /// homogeneous enums still need a nominal enum definition.
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
                self.defs.push(Def::TypeAlias(AliasDef {
                    name: name.to_owned(),
                    docs,
                    target,
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
    /// unrepresentable.
    fn build_struct(
        &mut self,
        name: &str,
        docs: Vec<String>,
        properties: &[PropertyIr],
        additional: AdditionalPropertiesPolicy,
    ) -> Def {
        debug_assert!(
            !matches!(additional, AdditionalPropertiesPolicy::Deny)
                || !matches!(additional, AdditionalPropertiesPolicy::Schema(_)),
            "deny and schema-valued additionalProperties cannot co-occur"
        );
        let deny_unknown_fields = matches!(additional, AdditionalPropertiesPolicy::Deny);

        let struct_name = name.to_owned();
        let mut fields = Vec::with_capacity(properties.len() + 1);
        let mut used_field_names = BTreeSet::new();
        for property in properties {
            fields.push(self.build_field(&struct_name, property, &mut used_field_names));
        }
        if let AdditionalPropertiesPolicy::Schema(edge) = additional {
            self.needs_btree_map = true;
            let value_type = self.edge_type(edge, &format!("{name}Additional"));
            fields.push(Field {
                docs: Vec::new(),
                attrs: vec!["#[serde(flatten)]".to_owned()],
                name: unique_in(&mut used_field_names, "additional"),
                ty: format!("BTreeMap<String, {value_type}>"),
            });
        }

        Def::Struct(StructDef {
            name: struct_name,
            docs,
            deny_unknown_fields,
            fields,
        })
    }

    /// One property through the companion §2.1 matrix cell-for-cell:
    /// requiredness from [`PropertyIr::required`], nullability from the
    /// referenced node's resolution.
    fn build_field(
        &mut self,
        parent: &str,
        property: &PropertyIr,
        used_field_names: &mut BTreeSet<String>,
    ) -> Field {
        let effective = self.chase(property.schema.target);
        let nullable = self.nullable_of(effective);
        let hint = format!(
            "{parent}{}",
            naming::ident(&property.wire_name, NameStyle::Pascal)
        );
        let base = self.edge_type(property.schema, &hint);

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

        Field {
            docs,
            attrs,
            name: field_name,
            ty,
        }
    }
}

// ----------------------------------------------------------------------
// Type expressions
// ----------------------------------------------------------------------

impl<'a> Generator<'a> {
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

impl<'a> Generator<'a> {
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
        for (index, branch) in choice.branches.iter().enumerate() {
            let effective = self.chase(branch.target);
            let fallback_hint = format!("{name}Branch{}", index + 1);
            let base_variant_name = self
                .named
                .get(&effective.0)
                .cloned()
                .unwrap_or_else(|| fallback_hint.clone());
            let variant_name = unique_in(&mut used_variants, &base_variant_name);
            let payload_base = self.edge_type(*branch, &fallback_hint);
            let payload = if self.nullable_of(effective) {
                format!("Option<{payload_base}>")
            } else {
                payload_base
            };
            variants.push((variant_name, payload));
        }
        Def::ChoiceEnum(ChoiceEnumDef {
            name: name.to_owned(),
            docs,
            variants,
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
/// enforced at runtime starting Phase 2, D-impl-runtime-validation-timing).
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
        "Constraints (runtime enforcement starts in Phase 2, \
         DECISIONS.md D-impl-runtime-validation-timing): {}.",
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
    // One blank line separates the header from the definitions.
    emitter.blank();

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
        "validation constraints ride as documentation until Phase 2 runtime enforcement",
        "(DECISIONS.md D-impl-runtime-validation-timing). This file is generated",
        "deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.",
    ];
    let owned: Vec<String> = docs.iter().map(|line| (*line).to_owned()).collect();
    emitter.docs(0, &owned);
}

fn emit_def(emitter: &mut Emitter, def: &Def) {
    match def {
        Def::Struct(struct_def) => emit_struct(emitter, struct_def),
        Def::StringsEnum(enum_def) => emit_enum(emitter, enum_def, false),
        Def::MixedEnum(enum_def) => emit_enum(emitter, enum_def, true),
        Def::IntegersEnum(integers) => emit_integers_enum(emitter, integers),
        Def::ChoiceEnum(choice) => emit_choice_enum(emitter, choice),
        Def::TypeAlias(alias) => emit_alias(emitter, alias),
        Def::FallbackNewtype(fallback) => emit_fallback(emitter, fallback),
    }
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

const RUSTFMT_MAX_WIDTH: usize = 100;

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
