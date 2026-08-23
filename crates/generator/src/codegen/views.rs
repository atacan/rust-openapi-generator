//! Directional view types for read-only/write-only schemas (companion §5):
//! one deterministic `views.rs` per normalized document.
//!
//! Companion §5 contract implemented here:
//!
//! - Shared models keep every field ([`super::models`]); directionality is
//!   enforced by `<Name>Write` / `<Name>Read` view types emitted for every
//!   object schema carrying at least one `readOnly` OR `writeOnly` property.
//!   Models without either marker receive no view types (identity case).
//! - The **write view** (client request encode / server request decode) omits
//!   `readOnly` properties entirely; the **read view** (server response
//!   encode / client response decode) omits `writeOnly` properties entirely.
//! - Requiredness is directional: required + `writeOnly` is required only in
//!   the write view, required + `readOnly` only in the read view, and
//!   directionless required fields are required in both. Off-direction fields
//!   are omitted outright, so every surviving field applies the companion
//!   §2.1 presence/nullability matrix exactly as [`super::models`] does.
//! - Conversions are intentionally asymmetric. The projection
//!   `From<&Name> for NameWrite` / `for NameRead` always exists and clones
//!   (or copies) kept fields into owned views — the single chosen shape,
//!   borrow-based and deterministic. The reverse reconstruction
//!   `From<&NameWrite> for Name` / `From<&NameRead> for Name` exists only
//!   when lossless: every field missing from that view must be optional in
//!   the shared model (`Option` or `OptionalField`). Otherwise NO conversion
//!   is generated — values are never fabricated (§5: a read view of
//!   `Widget { password: String }` cannot fabricate the password).
//!
//! Field types mirror `super::models`: `Box` edges, matrix-cell wrappers,
//! serde renames, nested anonymous definitions before their users. Output is
//! byte-deterministic (declaration order only, no timestamps, no paths, main
//! spec §50 test 39).
//!
//! Deviation note: §5 also allows a declared schema `default` to make an
//! omitted required field reconstructible. Phase 1 models render defaults as
//! documentation only, so a required non-nullable field counts as NOT lossless
//! even when it carries a default; nothing is invented at any point.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as JsonValue;

use crate::ir::schema::{
    AdditionalPropertiesPolicy, DiscriminatorIr, EnumValues, Indirection, PropertyIr, SchemaEdge,
    SchemaId, SchemaKind, ValidationMeta,
};
use crate::normalize::composition::{ClosedEnumChoice, MergedObject, ResolvedKind};
use crate::normalize::naming::{self, NameStyle};
use crate::normalize::{NormalizedDocument, NormalizedSchema};

use super::models::validation_lines;
use super::Emitter;

/// Derive list shared with `super::models` (main spec §2.6 shape).
const DERIVES: &str = "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]";

/// Cell attribute for required + nullable surviving fields (companion §2.1
/// row 2), identical to `super::models`.
const REQUIRED_NULLABLE_ATTR: &str =
    "#[serde(deserialize_with = \"openapi_support::optional::presence::deserialize_required_nullable\")]";

/// Cell attribute for optional + non-nullable surviving fields (row 3).
const OPTIONAL_NON_NULLABLE_ATTR: &str =
    "#[serde(default, skip_serializing_if = \"openapi_support::optional::is_absent\")]";

/// Cell attribute for optional + nullable surviving fields (row 4).
const OPTIONAL_NULLABLE_ATTR: &str = "#[serde(default)]";

const NULLABLE_NOTE: &str =
    "Nullable: instances may be JSON `null`; reference sites wrap this type in `Option<T>`.";

const BINARY_WARNING: &str =
    "Warning: `format: binary` marks a raw payload; Binary media classes stream bytes and \
     never reach shared models in Phase 1 (main spec §5.3), so this is modeled as `String`.";

/// Reconstruction expression for a view-omitted field sitting in the
/// optional + non-nullable cell of the shared model (companion §5 lossless
/// rule): nothing is invented, the field stays absent.
const ABSENT_EXPR: &str = "openapi_support::optional::OptionalField::Absent";

const RUSTFMT_MAX_WIDTH: usize = 100;

/// Renders ONE `views.rs`: crate-doc header, granular imports where needed,
/// then per qualifying component (declaration order) its write/read structs,
/// each followed by its conversion impls. Anonymous nested definitions are
/// pushed before their users, mirroring [`super::models`].
#[must_use]
pub fn generate_views(doc: &NormalizedDocument) -> String {
    // Declaration order via ascending arena ids (components were pre-interned;
    // same recovery trick as generate_models).
    let mut components: Vec<&NormalizedSchema> = doc.schemas.values().collect();
    components.sort_by_key(|schema| schema.source.0);

    // Seed the module-wide namespace exactly like models.rs (fallback newtype
    // suffix included) so generated anonymous names can never collide across
    // the two modules' public items.
    let mut named: BTreeMap<u32, String> = BTreeMap::new();
    let mut used_names: BTreeSet<String> = BTreeSet::new();
    for schema in &components {
        let name = public_component_name(doc, schema);
        used_names.insert(name.clone());
        named.insert(schema.source.0, name);
    }
    for (_, enum_name) in &doc.names.response_enums {
        used_names.insert(enum_name.clone());
    }

    let mut generator = Generator {
        doc,
        named,
        anonymous: BTreeMap::new(),
        used_names,
        model_imports: BTreeSet::new(),
        needs_optional_field: false,
        needs_btree_map: false,
        needs_serde: false,
        blocks: Vec::new(),
    };
    for schema in &components {
        generator.define_component_views(schema);
    }

    render(&generator)
}

/// Public type name of one component including the fallback newtype suffix,
/// mirroring the naming rule in [`super::models::generate_models`].
fn public_component_name(doc: &NormalizedDocument, schema: &NormalizedSchema) -> String {
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
}

// ----------------------------------------------------------------------
// Planning types
// ----------------------------------------------------------------------

/// One emitted top-level item; children always precede parents.
enum Block {
    Struct(StructDef),
    StringsEnum(EnumDef),
    IntegersEnum(IntegersEnumDef),
    MixedEnum(EnumDef),
    ChoiceEnum(ChoiceEnumDef),
    TypeAlias(AliasDef),
    Conversion(ConversionDef),
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
    variants: Vec<Variant>,
}

struct IntegersEnumDef {
    name: String,
    docs: Vec<String>,
    variants: Vec<(String, i64)>,
    constants: Vec<i64>,
}

struct ChoiceEnumDef {
    name: String,
    docs: Vec<String>,
    variants: Vec<(String, String)>,
}

struct AliasDef {
    name: String,
    docs: Vec<String>,
    target: String,
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

/// Borrow-based conversion between the shared model and one directional
/// view; assignments are emitted verbatim inside `Self { .. }`.
struct ConversionDef {
    docs: Vec<String>,
    from_ref: String,
    to_type: String,
    assignments: Vec<(String, String)>,
}

/// A shared-model property resolved once and reused by both directions:
/// directionality flags come from the chased target node, `base_type`
/// mirrors `super::models` field typing, and `field_name` is computed once
/// over the full declaration-order list so both views and both conversions
/// agree byte-for-byte.
struct ResolvedProperty {
    wire_name: String,
    required: bool,
    read_only: bool,
    write_only: bool,
    nullable: bool,
    base_type: String,
    /// The unboxed base value is `Copy`, so kept-field expressions move
    /// instead of cloning. Cell wrappers stay `Copy` exactly when their
    /// payload is, so one flag serves every representation.
    copy_value: bool,
    field_name: String,
    docs: Vec<String>,
}

/// Transport direction of a view (companion §5 bullet list).
#[derive(Clone, Copy)]
enum Direction {
    /// Client request encode / server request decode: `readOnly` omitted.
    Write,
    /// Server response encode / client response decode: `writeOnly` omitted.
    Read,
}

impl Direction {
    fn keeps(self, property: &ResolvedProperty) -> bool {
        match self {
            Self::Write => !property.read_only,
            Self::Read => !property.write_only,
        }
    }

    /// Marker whose presence drops a property from this view's wire shape.
    fn dropped_label(self) -> &'static str {
        match self {
            Self::Write => "readOnly",
            Self::Read => "writeOnly",
        }
    }

    /// Rust field names dropped by this direction, declaration order.
    fn dropped_fields(self, all: &[ResolvedProperty]) -> Vec<&str> {
        all.iter()
            .filter(|property| !self.keeps(property))
            .map(|property| property.field_name.as_str())
            .collect()
    }

    /// Fixed doc lines opening one view struct, after the shared-model
    /// description lines.
    fn header_docs(self, component: &str, all: &[ResolvedProperty]) -> Vec<String> {
        let mut lines = vec![format!(
            "Directional {} view of `{component}` (companion §5): {} wire shape.",
            match self {
                Self::Write => "write",
                Self::Read => "read",
            },
            self.purpose(),
        )];
        let dropped = self.dropped_fields(all);
        if dropped.is_empty() {
            lines.push("Every property survives in this direction.".to_owned());
        } else {
            lines.push(format!(
                "{} properties are omitted here: {}.",
                self.dropped_label(),
                dropped.join(", ")
            ));
        }
        lines
    }

    fn purpose(self) -> &'static str {
        match self {
            Self::Write => "client request encode and server request decode",
            Self::Read => "server response encode and client response decode",
        }
    }
}

// ----------------------------------------------------------------------
// Discovery pass
// ----------------------------------------------------------------------

struct Generator<'a> {
    doc: &'a NormalizedDocument,
    /// Named component arena id → public Rust type name.
    named: BTreeMap<u32, String>,
    /// Anonymous effective node id → generated definition name.
    anonymous: BTreeMap<u32, String>,
    used_names: BTreeSet<String>,
    /// Shared-model type names referenced from this module (granular sorted
    /// `use super::models::…` imports).
    model_imports: BTreeSet<String>,
    needs_optional_field: bool,
    needs_btree_map: bool,
    needs_serde: bool,
    blocks: Vec<Block>,
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

    /// Views for one named component — or nothing for alias passthroughs,
    /// non-object shapes, and the identity case without any directional
    /// marker (companion §5).
    fn define_component_views(&mut self, schema: &NormalizedSchema) {
        if matches!(self.resolution_of(schema.source), ResolvedKind::Alias(_)) {
            // Transparent `$ref` passthrough: the effective component carries
            // the views under its own name.
            return;
        }
        let effective = self.chase(schema.source);
        let (properties, additional) = match self.resolution_of(effective).clone() {
            ResolvedKind::MergedObject(MergedObject {
                properties,
                additional,
            }) => (properties, additional),
            ResolvedKind::Plain => match self.doc.arena.get(effective).kind.clone() {
                SchemaKind::Object {
                    properties,
                    additional,
                } => (properties, additional),
                _ => return,
            },
            _ => return,
        };

        // Cheap direction probe BEFORE any resolution: registering anonymous
        // children or model imports for components that end up with no views
        // would leak side effects into the module (stray definitions and
        // unused imports under `-D warnings`).
        let has_direction = properties.iter().any(|property| {
            let node = self.doc.arena.get(self.chase(property.schema.target));
            node.read_only || node.write_only
        });
        if !has_direction {
            return;
        }

        let resolved = self.resolve_properties(schema.rust_type.as_str(), &properties);
        if !resolved
            .iter()
            .any(|property| property.read_only || property.write_only)
        {
            return;
        }

        // View names reserve their slots in the module namespace before any
        // anonymous registration, so later collisions suffix deterministically
        // (companion §10).
        let component = schema.rust_type.as_str();
        let write_name = self.fresh_type_name(&format!("{component}Write"));
        let read_name = self.fresh_type_name(&format!("{component}Read"));
        // The conversion impls below reference the SHARED model type by bare
        // name, so it must ride the granular `use super::models::…` imports.
        self.model_imports.insert(component.to_owned());

        let node = self.doc.arena.get(effective);
        let mut write_docs = description_lines(&node.description.clone());
        write_docs.extend(Direction::Write.header_docs(component, &resolved));
        let mut read_docs = description_lines(&node.description.clone());
        read_docs.extend(Direction::Read.header_docs(component, &resolved));

        let write_struct = self.build_view_struct(
            &write_name,
            write_docs,
            Direction::Write,
            &resolved,
            additional,
        );
        let read_struct = self.build_view_struct(
            &read_name,
            read_docs,
            Direction::Read,
            &resolved,
            additional,
        );

        let projection_write = self.projection(component, &write_name, Direction::Write, &resolved);
        let reconstruction_write =
            self.reconstruction(component, &write_name, Direction::Write, &resolved);
        let projection_read = self.projection(component, &read_name, Direction::Read, &resolved);
        let reconstruction_read =
            self.reconstruction(component, &read_name, Direction::Read, &resolved);

        // Anonymous children were registered during resolution and already sit
        // in `blocks`; each view struct is followed immediately by its
        // conversions for deterministic, reviewable ordering.
        self.blocks.push(Block::Struct(write_struct));
        self.blocks.push(Block::Conversion(projection_write));
        if let Some(back) = reconstruction_write {
            self.blocks.push(Block::Conversion(back));
        }
        self.blocks.push(Block::Struct(read_struct));
        self.blocks.push(Block::Conversion(projection_read));
        if let Some(back) = reconstruction_read {
            self.blocks.push(Block::Conversion(back));
        }
    }

    /// Resolves every property once in declaration order: chased direction
    /// flags, nullability, the shared type expression (registering anonymous
    /// nominal composites on first encounter, children before parents), and
    /// the collision-resolved Rust field name.
    fn resolve_properties(
        &mut self,
        component: &str,
        properties: &[PropertyIr],
    ) -> Vec<ResolvedProperty> {
        let mut resolved = Vec::with_capacity(properties.len());
        let mut used_field_names: BTreeSet<String> = BTreeSet::new();
        for property in properties {
            let effective = self.chase(property.schema.target);
            let node = self.doc.arena.get(effective);
            let hint = format!(
                "{component}{}",
                naming::ident(&property.wire_name, NameStyle::Pascal)
            );
            let base_type = self.edge_type(property.schema, &hint);
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
            let mut docs = description_lines(&node.description.clone());
            if is_binary_string(self.doc, effective) {
                docs.push(BINARY_WARNING.to_owned());
            }
            docs.extend(validation_lines(&self.validation_of(effective)));
            if let Some(default) = &node.default {
                let json = serde_json::to_string(default).unwrap_or_else(|_| "null".to_owned());
                docs.push(format!("Default: `{json}`."));
            }
            resolved.push(ResolvedProperty {
                wire_name: property.wire_name.clone(),
                required: property.required,
                read_only: node.read_only,
                write_only: node.write_only,
                nullable: self.nullable_of(effective),
                copy_value: property.schema.indirection == Indirection::None
                    && base_is_copy(
                        self.resolution_of(effective),
                        &self.doc.arena.get(effective).kind,
                    ),
                base_type,
                field_name,
                docs,
            });
        }
        resolved
    }

    /// One directional view struct from the surviving properties plus the
    /// unchanged additionalProperties policy (companion §4.4).
    fn build_view_struct(
        &mut self,
        name: &str,
        docs: Vec<String>,
        direction: Direction,
        all: &[ResolvedProperty],
        additional: AdditionalPropertiesPolicy,
    ) -> StructDef {
        let deny_unknown_fields = matches!(additional, AdditionalPropertiesPolicy::Deny);
        self.needs_serde = true;

        let struct_name = name.to_owned();
        let mut fields = Vec::new();
        let mut used_field_names: BTreeSet<String> = BTreeSet::new();
        for property in all.iter().filter(|property| direction.keeps(property)) {
            fields.push(self.build_view_field(property, &mut used_field_names));
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

        StructDef {
            name: struct_name,
            docs,
            deny_unknown_fields,
            fields,
        }
    }

    /// Surviving field through the companion §2.1 matrix cell-for-cell with
    /// its directional requiredness; serde renames match `super::models`.
    fn build_view_field(
        &mut self,
        property: &ResolvedProperty,
        used_field_names: &mut BTreeSet<String>,
    ) -> Field {
        let (ty, cell_attr) = self.matrix_cell(property);
        let mut attrs = Vec::new();
        if property.field_name != property.wire_name {
            attrs.push(format!(
                "#[serde(rename = \"{}\")]",
                escape_string(&property.wire_name)
            ));
        }
        if let Some(attr) = cell_attr {
            attrs.push(attr.to_owned());
        }
        Field {
            docs: property.docs.clone(),
            attrs,
            name: unique_in(used_field_names, &property.field_name),
            ty,
        }
    }

    /// Companion §2.1 cell for one property: (required, nullable) → type
    /// wrapper plus serde attribute; identical to `super::models`.
    fn matrix_cell(&mut self, property: &ResolvedProperty) -> (String, Option<&'static str>) {
        match (property.required, property.nullable) {
            (true, false) => (property.base_type.clone(), None),
            (true, true) => (
                format!("Option<{}>", property.base_type),
                Some(REQUIRED_NULLABLE_ATTR),
            ),
            (false, false) => {
                self.needs_optional_field = true;
                (
                    format!("OptionalField<{}>", property.base_type),
                    Some(OPTIONAL_NON_NULLABLE_ATTR),
                )
            }
            (false, true) => (
                format!("Option<{}>", property.base_type),
                Some(OPTIONAL_NULLABLE_ATTR),
            ),
        }
    }

    /// Shared-model → view projection (companion §5, always generated):
    /// `From<&Name>` moving/cloning every kept field into the owned view.
    fn projection(
        &mut self,
        component: &str,
        view_name: &str,
        direction: Direction,
        all: &[ResolvedProperty],
    ) -> ConversionDef {
        let mut docs = vec![format!(
            "Projects the shared model into the {view_name} (companion §5, \
             {purpose}): kept fields clone or copy from the borrowed model; \
             this projection always exists.",
            purpose = direction.purpose(),
        )];
        let dropped = direction.dropped_fields(all);
        if dropped.is_empty() {
            docs.push("Every shared property survives in this direction.".to_owned());
        } else {
            docs.push(format!(
                "Omitted here ({}): {}.",
                direction.dropped_label(),
                dropped.join(", ")
            ));
        }
        ConversionDef {
            docs,
            from_ref: format!("&{component}"),
            to_type: view_name.to_owned(),
            assignments: all
                .iter()
                .filter(|property| direction.keeps(property))
                .map(|property| (property.field_name.clone(), kept_expression(property)))
                .collect(),
        }
    }

    /// View → shared reconstruction (companion §5): generated ONLY when every
    /// field missing from the view is optional in the shared model, so the
    /// conversion invents no values; otherwise `None` — no conversion at all.
    fn reconstruction(
        &mut self,
        component: &str,
        view_name: &str,
        direction: Direction,
        all: &[ResolvedProperty],
    ) -> Option<ConversionDef> {
        let mut assignments: Vec<(String, String)> = Vec::with_capacity(all.len());
        let mut filled: Vec<(String, bool)> = Vec::new();
        for property in all {
            if direction.keeps(property) {
                assignments.push((property.field_name.clone(), kept_expression(property)));
                continue;
            }
            // Missing-in-view field: lossless only when the shared-model cell
            // can express absence (`Option` / `OptionalField`). A required
            // non-nullable field carries no reconstructible value — never
            // invent one (companion §5).
            match (property.required, property.nullable) {
                (true, false) => return None,
                (false, false) => {
                    filled.push((property.field_name.clone(), false));
                    assignments.push((property.field_name.clone(), ABSENT_EXPR.to_owned()));
                }
                (_, true) => {
                    filled.push((property.field_name.clone(), true));
                    assignments.push((property.field_name.clone(), "None".to_owned()));
                }
            }
        }
        let mut docs = vec![
            "Losslessly reconstructs the shared model from the view (companion \
             §5): every field omitted from the view is optional in the shared \
             model, so the conversion invents no values."
                .to_owned(),
        ];
        if filled.is_empty() {
            docs.push("Nothing is omitted in this direction.".to_owned());
        } else {
            let fills: Vec<String> = filled
                .iter()
                .map(|(field, nullable)| {
                    if *nullable {
                        format!("{field} to `None`")
                    } else {
                        format!("{field} to absent")
                    }
                })
                .collect();
            docs.push(format!("Missing fields default: {}.", fills.join(", ")));
        }
        Some(ConversionDef {
            docs,
            from_ref: format!("&{view_name}"),
            to_type: component.to_owned(),
            assignments,
        })
    }

    /// Registers an anonymous composite under a fresh name derived from
    /// `hint`; re-encounters reuse the registration.
    fn register_anonymous(&mut self, effective: SchemaId, hint: &str) -> String {
        if let Some(name) = self.anonymous.get(&effective.0) {
            return name.clone();
        }
        let name = self.fresh_type_name(hint);
        self.anonymous.insert(effective.0, name.clone());
        self.define_node(effective, &name);
        name
    }

    /// Builds one anonymous composite definition (children pushed first),
    /// mirroring the reachable subset of [`super::models`] discovery.
    fn define_node(&mut self, effective: SchemaId, name: &str) {
        match self.resolution_of(effective).clone() {
            // Callers only route registrable kinds here; defensive no-ops.
            ResolvedKind::Alias(_) | ResolvedKind::RawValueFallback(_) => {}
            ResolvedKind::MergedObject(MergedObject {
                properties,
                additional,
            }) => {
                let validation = self.validation_of(effective);
                let docs = self.definition_docs(effective, &[], &validation);
                self.needs_serde = true;
                let def = self.build_anon_struct(name, docs, &properties, additional);
                self.blocks.push(Block::Struct(def));
            }
            ResolvedKind::IntersectedScalar(scalar) => {
                let validation = self.validation_of(effective);
                let docs = self.definition_docs(effective, &[], &validation);
                match scalar.base_kind.clone() {
                    SchemaKind::Enum { values } => self.define_enum(effective, name, values),
                    other_kind => {
                        self.needs_serde = true;
                        let target = self.scalar_target(&other_kind, name);
                        self.blocks.push(Block::TypeAlias(AliasDef {
                            name: name.to_owned(),
                            docs,
                            target,
                        }));
                    }
                }
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
                self.blocks.push(Block::ChoiceEnum(def));
            }
            ResolvedKind::Plain => match self.doc.arena.get(effective).kind.clone() {
                SchemaKind::Object {
                    properties,
                    additional,
                } => {
                    let validation = self.validation_of(effective);
                    let docs = self.definition_docs(effective, &[], &validation);
                    self.needs_serde = true;
                    let def = self.build_anon_struct(name, docs, &properties, additional);
                    self.blocks.push(Block::Struct(def));
                }
                SchemaKind::Enum { values } => self.define_enum(effective, name, values),
                _ => {}
            },
        }
    }

    fn define_enum(&mut self, effective: SchemaId, name: &str, values: EnumValues) {
        let docs = self.definition_docs(effective, &[], &ValidationMeta::default());
        match values {
            EnumValues::Strings(constants) => {
                let mut used_variants: BTreeSet<String> = BTreeSet::new();
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
                self.blocks.push(Block::StringsEnum(EnumDef {
                    name: name.to_owned(),
                    docs,
                    variants,
                }));
            }
            EnumValues::Integers(constants) => {
                let mut used_variants: BTreeSet<String> = BTreeSet::new();
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
                self.blocks.push(Block::IntegersEnum(IntegersEnumDef {
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
                self.blocks.push(Block::MixedEnum(EnumDef {
                    name: name.to_owned(),
                    docs,
                    variants,
                }));
            }
        }
    }

    fn mixed_variants(&mut self, constants: &[JsonValue]) -> Vec<Variant> {
        let mut used_variants: BTreeSet<String> = BTreeSet::new();
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
                        .map_or_else(|| format!("F{number}"), integer_label);
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

    /// Doc-comment assembly for anonymous definitions: description,
    /// nullable note, binary warning, constraints, default.
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

    /// Anonymous struct: identical policy handling to `super::models`
    /// (companion §4.4); fields keep EVERY property through the full §2.1
    /// matrix regardless of transport direction, because they describe the
    /// referenced shape itself rather than a directional cut.
    fn build_anon_struct(
        &mut self,
        name: &str,
        docs: Vec<String>,
        properties: &[PropertyIr],
        additional: AdditionalPropertiesPolicy,
    ) -> StructDef {
        debug_assert!(
            !matches!(additional, AdditionalPropertiesPolicy::Deny)
                || !matches!(additional, AdditionalPropertiesPolicy::Schema(_)),
            "deny and schema-valued additionalProperties cannot co-occur"
        );
        let deny_unknown_fields = matches!(additional, AdditionalPropertiesPolicy::Deny);

        let struct_name = name.to_owned();
        let mut fields = Vec::with_capacity(properties.len() + 1);
        let mut used_field_names: BTreeSet<String> = BTreeSet::new();
        for property in properties {
            let effective = self.chase(property.schema.target);
            let node = self.doc.arena.get(effective);
            let hint = format!(
                "{name}{}",
                naming::ident(&property.wire_name, NameStyle::Pascal)
            );
            let resolved = ResolvedProperty {
                wire_name: property.wire_name.clone(),
                required: property.required,
                read_only: node.read_only,
                write_only: node.write_only,
                nullable: self.nullable_of(effective),
                copy_value: false,
                base_type: self.edge_type(property.schema, &hint),
                field_name: naming::ident(&property.wire_name, NameStyle::Snake),
                docs: Vec::new(),
            };
            let (ty, cell_attr) = self.matrix_cell(&resolved);

            let field_name = {
                let base = resolved.field_name.clone();
                let mut candidate = base.clone();
                let mut counter = 1_u32;
                while !used_field_names.insert(candidate.clone()) {
                    counter += 1;
                    candidate = naming::sanitize_joined(&format!("{base}_{counter}"));
                }
                candidate
            };
            let mut attrs = Vec::new();
            if field_name != property.wire_name {
                attrs.push(format!(
                    "#[serde(rename = \"{}\")]",
                    escape_string(&property.wire_name)
                ));
            }
            if let Some(attr) = cell_attr {
                attrs.push(attr.to_owned());
            }
            let mut field_docs = description_lines(&node.description.clone());
            if is_binary_string(self.doc, effective) {
                field_docs.push(BINARY_WARNING.to_owned());
            }
            field_docs.extend(validation_lines(&self.validation_of(effective)));
            if let Some(default) = &node.default {
                let json = serde_json::to_string(default).unwrap_or_else(|_| "null".to_owned());
                field_docs.push(format!("Default: `{json}`."));
            }
            fields.push(Field {
                docs: field_docs,
                attrs,
                name: field_name,
                ty,
            });
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

        StructDef {
            name: struct_name,
            docs,
            deny_unknown_fields,
            fields,
        }
    }

    /// Newtype choice enum over proven-exclusive branches (companion §4.2).
    fn build_choice_enum(
        &mut self,
        name: &str,
        docs: Vec<String>,
        choice: &ClosedEnumChoice,
    ) -> ChoiceEnumDef {
        let mut used_variants: BTreeSet<String> = BTreeSet::new();
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
        ChoiceEnumDef {
            name: name.to_owned(),
            docs,
            variants,
        }
    }

    // ----------------------------------------------------------------------
    // Type expressions (mirror super::models)
    // ----------------------------------------------------------------------

    /// Type expression for one edge: referenced type path with heap
    /// indirection applied (`Box<T>` when the cycle-precise pass flagged the
    /// property edge); nullability wrapping belongs to the matrix cells.
    fn edge_type(&mut self, edge: SchemaEdge, hint: &str) -> String {
        let inner = self.reference_type(edge.target, hint);
        match edge.indirection {
            Indirection::Boxed => format!("Box<{inner}>"),
            Indirection::None => inner,
        }
    }

    /// Rust type naming the (alias-chased) node `id`: named components use
    /// their assigned names (imported granularly from `super::models`);
    /// anonymous composites that need nominal types get registered on first
    /// encounter; everything else renders inline.
    fn reference_type(&mut self, id: SchemaId, hint: &str) -> String {
        let effective = self.chase(id);
        if let Some(name) = self.named.get(&effective.0) {
            self.model_imports.insert(name.clone());
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
            _ => "serde_json::Value".to_owned(),
        }
    }
}

/// Kept-field expression inside a conversion body: move when the value is
/// `Copy` (avoids `clippy::clone_on_copy`), clone otherwise.
fn kept_expression(property: &ResolvedProperty) -> String {
    if property.copy_value {
        format!("value.{}", property.field_name)
    } else {
        format!("value.{}.clone()", property.field_name)
    }
}

// ----------------------------------------------------------------------
// Helpers shared with the models emitter (local copies)
// ----------------------------------------------------------------------

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

/// Base-value Copy-ness: primitives and integer-discriminant enums are
/// `Copy`; everything else needs `.clone()`.
fn base_is_copy(resolution: &ResolvedKind, kind: &SchemaKind) -> bool {
    match resolution {
        ResolvedKind::IntersectedScalar(scalar) => copy_kind(&scalar.base_kind),
        ResolvedKind::Plain => copy_kind(kind),
        _ => false,
    }
}

fn copy_kind(kind: &SchemaKind) -> bool {
    matches!(
        kind,
        SchemaKind::Boolean
            | SchemaKind::Integer { .. }
            | SchemaKind::Number { .. }
            | SchemaKind::Enum {
                values: EnumValues::Integers(_)
            }
    )
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

/// Discriminator metadata as a doc comment (companion §4.2).
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

/// Splits a schema description into doc lines without trailing whitespace.
fn description_lines(description: &Option<String>) -> Vec<String> {
    let Some(text) = description else {
        return Vec::new();
    };
    text.split('\n')
        .map(|line| line.trim_end_matches('\r').trim_end().to_owned())
        .collect()
}

// ----------------------------------------------------------------------
// Emission pass
// ----------------------------------------------------------------------

fn render(generator: &Generator<'_>) -> String {
    let mut emitter = Emitter::new();
    if generator.blocks.is_empty() {
        // Identity document: no readOnly/writeOnly anywhere. A plain comment
        // keeps the file valid Rust (doc comments would need a following
        // item) while explaining the absence (companion §5).
        emitter.line(
            0,
            "// No directional view types: this document declares no \
             readOnly/writeOnly properties (companion §5).",
        );
        return emitter.finish();
    }
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
    if !generator.model_imports.is_empty() {
        emitter.blank();
        for name in &generator.model_imports {
            let line = format!("use super::models::{name};");
            emitter.line(0, &line);
        }
    }
    emitter.blank();

    for (index, block) in generator.blocks.iter().enumerate() {
        if index > 0 {
            emitter.blank();
        }
        emit_block(&mut emitter, block);
    }
    emitter.finish()
}

fn emit_crate_docs(emitter: &mut Emitter) {
    let docs = [
        "Directional view types generated from the OpenAPI document (companion §5):",
        "`Write` views carry the client-request-encode / server-request-decode wire",
        "shape with `readOnly` properties omitted; `Read` views carry the server-",
        "response-encode / client-response-decode wire shape with `writeOnly`",
        "properties treated as absent.",
        "",
        "Requiredness is directional (companion §5): required `writeOnly` fields are",
        "required only in `Write`, required `readOnly` fields only in `Read`, and",
        "directionless required fields stay required in both. Every surviving field",
        "applies the companion §2.1 presence/nullability matrix identically to",
        "`super::models`.",
        "",
        "Conversions are intentionally asymmetric (companion §5): projections",
        "`From<&SharedModel> for *View` always exist; reconstructions",
        "`From<&*View> for SharedModel` exist only when every view-omitted field is",
        "optional in the shared model, so no value is ever fabricated. Models",
        "without `readOnly`/`writeOnly` properties receive no view types.",
        "",
        "This file is generated deterministically byte-for-byte (main spec §50 test",
        "39); do not edit by hand.",
    ];
    let owned: Vec<String> = docs.iter().map(|line| (*line).to_owned()).collect();
    emitter.docs(0, &owned);
}

fn emit_block(emitter: &mut Emitter, block: &Block) {
    match block {
        Block::Struct(struct_def) => emit_struct(emitter, struct_def),
        Block::StringsEnum(enum_def) => emit_enum(emitter, enum_def, false),
        Block::MixedEnum(enum_def) => emit_enum(emitter, enum_def, true),
        Block::IntegersEnum(integers) => emit_integers_enum(emitter, integers),
        Block::ChoiceEnum(choice) => emit_choice_enum(emitter, choice),
        Block::TypeAlias(alias) => emit_alias(emitter, alias),
        Block::Conversion(conversion) => emit_conversion(emitter, conversion),
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

fn emit_conversion(emitter: &mut Emitter, def: &ConversionDef) {
    emitter.docs(0, &def.docs);
    let header = format!("impl From<{}> for {} {{", def.from_ref, def.to_type);
    emitter.line(0, &header);
    let signature = format!("fn from(value: {}) -> Self {{", def.from_ref);
    emitter.line(1, &signature);
    if def.assignments.is_empty() {
        emitter.line(2, "Self {}");
    } else {
        emitter.line(2, "Self {");
        for (field, expression) in &def.assignments {
            let line = format!("{field}: {expression},");
            emitter.line(3, &line);
        }
        emitter.line(2, "}");
    }
    emitter.line(1, "}");
    emitter.line(0, "}");
}

/// Emits one `#[serde(...)]` attribute in rustfmt-canonical shape (same rule
/// as `super::models`): short attributes stay on one line, over-wide ones
/// wrap their arguments onto continuation lines.
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
