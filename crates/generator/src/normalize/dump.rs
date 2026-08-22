//! Deterministic debug dump of a [`NormalizedDocument`] (main spec §50
//! reproducibility rules): stable ordering, indented text, no timestamps
//! and no absolute paths. The golden harness compares this output
//! byte-for-byte against committed snapshots.

use std::fmt::Write as _;

use crate::ir::document::{MediaClass, ResponseStatusKey};
use crate::ir::schema::{
    AdditionalPropertiesPolicy, EnumValues, Indirection, SchemaEdge, SchemaId, SchemaKind,
    UnsupportedReason,
};

use super::composition::{
    ClosedEnumChoice, FallbackReason, IntersectedScalar, MergedObject, RawFallback, ResolvedKind,
};
use super::NormalizedDocument;

/// Renders the normalized document as a deterministic multi-line string.
#[must_use]
pub fn dump_normalized(doc: &NormalizedDocument) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "openapi {} ({})", doc.version_label(), doc.raw_version);

    out.push_str("root-servers:\n");
    for server in &doc.root_servers {
        write_server(&mut out, server, 2);
    }
    if doc.root_servers.is_empty() {
        out.push_str("  (none: operations default to `/`)\n");
    }

    out.push_str("schemas:\n");
    for schema in doc.schemas.values() {
        let _ = writeln!(
            out,
            "  {} -> type `{}` (#{}):",
            schema.component_name, schema.rust_type, schema.source.0
        );
        let resolved = doc.resolution(schema.source);
        dump_resolved(doc, &mut out, resolved, 4);
    }

    out.push_str("operations:\n");
    if doc.operations.is_empty() {
        out.push_str("  (none)\n");
    }
    for operation in &doc.operations {
        let _ = writeln!(out, "  {}:", operation.operation_key);
        let _ = writeln!(out, "    method-name: {}", operation.method_name);
        let _ = writeln!(out, "    response-enum: {}", operation.response_enum);
        if let Some(operation_id) = &operation.operation_id {
            let _ = writeln!(out, "    operation-id: {operation_id}");
        }
        if !operation.tags.is_empty() {
            let modules: Vec<String> = operation
                .tags
                .iter()
                .map(|tag| {
                    doc.names
                        .tag_modules
                        .get(tag)
                        .cloned()
                        .unwrap_or_else(|| tag.clone())
                })
                .collect();
            let _ = writeln!(out, "    tags: {}", modules.join(", "));
        }
        if operation.deprecated {
            out.push_str("    deprecated\n");
        }
        out.push_str("    effective-servers:\n");
        for server in &operation.effective_servers {
            write_server(&mut out, server, 6);
        }
        out.push_str("    parameters:\n");
        if operation.merged_parameters.is_empty() {
            out.push_str("      (none)\n");
        }
        for merged in &operation.merged_parameters {
            let parameter = &merged.parameter;
            let _ = writeln!(
                out,
                "      - in={} name=`{}` required={} style={:?} explode={} \
                 allow-reserved={} origin={:?}",
                location_label(parameter.location),
                parameter.name,
                parameter.required,
                parameter.style,
                parameter.explode,
                parameter.allow_reserved,
                merged.origin,
            );
        }
        match &operation.request_body {
            None => out.push_str("    request-body: (absent)\n"),
            Some(body) => {
                let _ = writeln!(out, "    request-body: required={}", body.required);
                dump_content(&mut out, &body.content, 6);
            }
        }
        out.push_str("    responses:\n");
        if operation.responses.is_empty() {
            out.push_str("      (none)\n");
        }
        for response in &operation.responses {
            let _ = writeln!(out, "      - status {}", status_key(response.status));
            for (wire, header) in &response.headers {
                let header_resolved = doc.resolution(header.schema);
                let _ = writeln!(
                    out,
                    "        header `{wire}` required={} nullable={} required-flag={}",
                    header.required, header_resolved.nullable, header.required
                );
            }
            dump_content(&mut out, &response.content, 8);
        }
    }

    out.push_str("arena:\n");
    let mut visited: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for (id, node) in doc.arena.iter() {
        dump_arena_node(doc, &mut out, id, node.kind.clone(), &mut visited, 2);
    }

    out.push_str("diagnostics:\n");
    if doc.diagnostics.is_empty() {
        out.push_str("  (none)\n");
    }
    for diagnostic in &doc.diagnostics {
        let _ = writeln!(
            out,
            "  - {} at {} [{}]: {}",
            diagnostic.severity, diagnostic.path, diagnostic.code, diagnostic.message
        );
    }
    out
}

impl NormalizedDocument {
    fn version_label(&self) -> &'static str {
        use crate::ir::document::OpenApiVersion;
        match self.version {
            OpenApiVersion::V3_0 => "3.0",
            OpenApiVersion::V3_1 => "3.1",
            OpenApiVersion::V3_2 => "3.2",
        }
    }
}

fn indent(out: &mut String, spaces: usize) {
    for _ in 0..spaces {
        out.push(' ');
    }
}

fn write_server(out: &mut String, server: &crate::ir::document::ServerIr, spaces: usize) {
    indent(out, spaces);
    let _ = writeln!(out, "- url `{}`", server.url);
    for (name, variable) in &server.variables {
        indent(out, spaces + 2);
        let allowed = variable
            .allowed_enum
            .as_ref()
            .map(|values| values.join("|"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "variable `{name}` default=`{}` enum=[{allowed}]",
            variable.default
        );
    }
}

fn location_label(location: crate::ir::document::ParameterLocation) -> &'static str {
    use crate::ir::document::ParameterLocation;
    match location {
        ParameterLocation::Path => "path",
        ParameterLocation::Query => "query",
        ParameterLocation::Header => "header",
        ParameterLocation::Cookie => "cookie",
    }
}

/// Renders a response status key in document spelling (`200`, `2XX`,
/// `default`).
#[must_use]
pub fn status_key(status: ResponseStatusKey) -> String {
    super::status_label(&status)
}

/// Renders one arena node's original kind plus its composition resolution.
fn dump_arena_node(
    doc: &NormalizedDocument,
    out: &mut String,
    id: SchemaId,
    kind: SchemaKind,
    visited: &mut std::collections::BTreeSet<u32>,
    spaces: usize,
) {
    indent(out, spaces);
    let fresh = visited.insert(id.0);
    let _ = writeln!(out, "#{} {}", id.0, render_kind(doc, &kind));
    if !fresh {
        return;
    }
    match &kind {
        SchemaKind::Object { properties, .. } => {
            for property in properties {
                indent(out, spaces + 2);
                let edge = &property.schema;
                let boxed = matches!(edge.indirection, Indirection::Boxed);
                let _ = writeln!(
                    out,
                    "property `{}` required={} -> #{}{}",
                    property.wire_name,
                    property.required,
                    edge.target.0,
                    if boxed { " (boxed)" } else { "" }
                );
            }
        }
        SchemaKind::Array { items }
        | SchemaKind::Tuple {
            items: Some(items), ..
        } => {
            indent(out, spaces + 2);
            let _ = writeln!(out, "items -> #{}", items.target.0);
        }
        SchemaKind::Tuple { prefix_items, .. } => {
            for (index, item) in prefix_items.iter().enumerate() {
                indent(out, spaces + 2);
                let _ = writeln!(out, "prefix[{index}] -> #{}", item.target.0);
            }
        }
        _ => {}
    }

    // The composition resolution rides under the arena entry.
    let Some(resolved) = doc.resolutions.get(id.0 as usize) else {
        return;
    };
    indent(out, spaces + 2);
    out.push_str("resolved:\n");
    dump_resolved(doc, out, resolved, spaces + 4);
}

/// Renders the resolved shape of a schema.
fn dump_resolved(
    doc: &NormalizedDocument,
    out: &mut String,
    resolved: &super::composition::ResolvedNode,
    spaces: usize,
) {
    indent(out, spaces);
    let _ = writeln!(out, "nullable={}", resolved.nullable);
    if let Some(discriminator) = &resolved.discriminator {
        indent(out, spaces);
        let explicit = if discriminator.explicit {
            "explicit"
        } else {
            "implicit"
        };
        let mapping: Vec<String> = discriminator
            .mapping
            .iter()
            .map(|(value, target)| format!("{value}=>{}", target.0))
            .collect();
        let _ = writeln!(
            out,
            "discriminator ({explicit}) property=`{}` mapping=[{}]",
            discriminator.property_name,
            mapping.join(", ")
        );
    }
    indent(out, spaces);
    match &resolved.kind {
        ResolvedKind::Plain => {
            writeln!(out, "plain").ok();
        }
        ResolvedKind::Alias(target) => {
            let _ = writeln!(out, "alias -> #{}", target.0);
        }
        ResolvedKind::MergedObject(MergedObject {
            properties,
            additional,
        }) => {
            let _ = writeln!(
                out,
                "merged-object additional={}",
                additional_label(additional)
            );
            for property in properties {
                indent(out, spaces + 2);
                let boxed = matches!(property.schema.indirection, Indirection::Boxed);
                let _ = writeln!(
                    out,
                    "property `{}` required={} -> #{}{}",
                    property.wire_name,
                    property.required,
                    property.schema.target.0,
                    if boxed { " (boxed)" } else { "" }
                );
            }
        }
        ResolvedKind::IntersectedScalar(IntersectedScalar { base_kind }) => {
            let _ = writeln!(
                out,
                "intersected-scalar base={}",
                render_kind(doc, base_kind)
            );
        }
        ResolvedKind::ClosedEnum(ClosedEnumChoice {
            branches,
            native_serde_candidate,
        }) => {
            let targets: Vec<String> = branches
                .iter()
                .map(|edge| format!("#{}", edge.target.0))
                .collect();
            let _ = writeln!(
                out,
                "closed-enum branches=[{}] native-serde-candidate={}",
                targets.join(", "),
                native_serde_candidate
            );
        }
        ResolvedKind::RawValueFallback(RawFallback {
            reason,
            native_serde_candidate,
        }) => {
            let _ = writeln!(
                out,
                "raw-value-fallback reason={} native-serde-candidate={}",
                fallback_reason(reason),
                native_serde_candidate
            );
        }
    }
    dump_validation(out, &resolved.validation, spaces);
}

fn fallback_reason(reason: &FallbackReason) -> &'static str {
    reason.label()
}

fn additional_label(additional: &AdditionalPropertiesPolicy) -> &'static str {
    match additional {
        AdditionalPropertiesPolicy::Deny => "deny",
        AdditionalPropertiesPolicy::Ignore => "ignore",
        AdditionalPropertiesPolicy::Schema(_) => "schema",
    }
}

/// Compact single-line rendering of a [`SchemaKind`].
fn render_kind(_doc: &NormalizedDocument, kind: &SchemaKind) -> String {
    let mut text = String::new();
    match kind {
        SchemaKind::AnyValue => text.push_str("any-value"),
        SchemaKind::FreeFormObject => text.push_str("free-form-object"),
        SchemaKind::Boolean => text.push_str("boolean"),
        SchemaKind::Integer { format } => {
            write!(text, "integer{}", format_suffix(format.as_deref())).ok();
        }
        SchemaKind::Number { format } => {
            write!(text, "number{}", format_suffix(format.as_deref())).ok();
        }
        SchemaKind::String_ { format, binary } => {
            let suffix = format_suffix(format.as_deref());
            if *binary {
                text.push_str(&format!("string{suffix} BINARY"));
            } else {
                text.push_str(&format!("string{suffix}"));
            }
        }
        SchemaKind::Array { items } => {
            write!(text, "array items->#{}", items.target.0).ok();
        }
        SchemaKind::Tuple {
            prefix_items,
            items,
        } => {
            let prefixes: Vec<String> = prefix_items
                .iter()
                .map(|edge| format!("#{}", edge.target.0))
                .collect();
            write!(text, "tuple prefix=[{}]", prefixes.join(",")).ok();
            if let Some(extra) = items {
                write!(text, " rest->#{}", extra.target.0).ok();
            }
        }
        SchemaKind::Object {
            properties,
            additional,
        } => {
            write!(
                text,
                "object props={} additional={}",
                properties.len(),
                additional_label(additional)
            )
            .ok();
        }
        SchemaKind::Enum { values } => match values {
            EnumValues::Strings(values) => {
                write!(text, "enum strings=[{}]", values.join("|")).ok();
            }
            EnumValues::Integers(values) => {
                let rendered: Vec<String> = values.iter().map(ToString::to_string).collect();
                write!(text, "enum integers=[{}]", rendered.join("|")).ok();
            }
            EnumValues::MixedFallback(_) => text.push_str("enum mixed"),
        },
        SchemaKind::Ref {
            target,
            inline_constraints,
            ..
        } => {
            write!(text, "ref->#{}", target.0).ok();
            if !inline_constraints.is_empty() {
                text.push_str(&format!(" +{} constraints", inline_constraints.len()));
            }
        }
        SchemaKind::AllOf { members, .. } => {
            write!(text, "all-of members={}", member_list(members)).ok();
        }
        SchemaKind::OneOf { members, .. } => {
            write!(text, "one-of members={}", member_list(members)).ok();
        }
        SchemaKind::AnyOf { members, .. } => {
            write!(text, "any-of members={}", member_list(members)).ok();
        }
        SchemaKind::NotSupported { reason } => {
            write!(text, "not-supported({})", unsupported_reason(reason)).ok();
        }
    }
    text
}

fn member_list(members: &[SchemaEdge]) -> String {
    members
        .iter()
        .map(|edge| format!("#{}", edge.target.0))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_suffix(format: Option<&str>) -> String {
    match format {
        Some(value) => format!(":{value}"),
        None => String::new(),
    }
}

fn unsupported_reason(reason: &UnsupportedReason) -> &'static str {
    match reason {
        UnsupportedReason::MixedTypeArray => "mixed-type-array",
        UnsupportedReason::UnevaluatedKeywordsActive => "unevaluated-keywords-active",
        UnsupportedReason::AnchorRef => "anchor-ref",
        UnsupportedReason::RemoteRefUnfetched => "remote-ref-unfetched",
        UnsupportedReason::UnbrokenSelfContainment => "unbroken-self-containment",
        UnsupportedReason::InlineExpansionDepthExceeded => "inline-depth-exceeded",
        UnsupportedReason::Other(_) => "other",
    }
}

/// Single-line summary of non-default validation metadata; the detailed
/// multi-line form follows separately.
fn validation_label(validation: &crate::ir::schema::ValidationMeta) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pattern) = &validation.pattern {
        parts.push(format!("pattern=/{pattern}/"));
    }
    if let Some(min_length) = validation.min_length {
        parts.push(format!("min-length={min_length}"));
    }
    if let Some(max_length) = validation.max_length {
        parts.push(format!("max-length={max_length}"));
    }
    let numeric = &validation.numeric;
    if let Some(minimum) = numeric.minimum {
        parts.push(format!("minimum={minimum:?}"));
    }
    if let Some(maximum) = numeric.maximum {
        parts.push(format!("maximum={maximum:?}"));
    }
    if let Some(exclusive_minimum) = numeric.exclusive_minimum {
        parts.push(format!("exclusive-minimum={exclusive_minimum:?}"));
    }
    if let Some(exclusive_maximum) = numeric.exclusive_maximum {
        parts.push(format!("exclusive-maximum={exclusive_maximum:?}"));
    }
    if let Some(multiple_of) = numeric.multiple_of {
        parts.push(format!("multiple-of={multiple_of:?}"));
    }
    if let Some(min_items) = validation.min_items {
        parts.push(format!("min-items={min_items}"));
    }
    if let Some(max_items) = validation.max_items {
        parts.push(format!("max-items={max_items}"));
    }
    if validation.unique_items {
        parts.push("unique-items".to_owned());
    }
    parts.join(" ")
}

/// Multi-line detailed validation block (kept after the kind line so the
/// dump stays greppable).
fn dump_validation(
    out: &mut String,
    validation: &crate::ir::schema::ValidationMeta,
    spaces: usize,
) {
    let label = validation_label(validation);
    if label.is_empty() {
        return;
    }
    indent(out, spaces + 2);
    let _ = writeln!(out, "validation: {label}");
}

fn dump_content(out: &mut String, content: &[crate::ir::document::ContentEntryIr], spaces: usize) {
    if content.is_empty() {
        return;
    }
    indent(out, spaces);
    out.push_str("content:\n");
    for entry in content {
        indent(out, spaces + 2);
        let wildcard = if entry.is_wildcard { " wildcard" } else { "" };
        let class = media_class_label(entry.media_class);
        let _ = writeln!(
            out,
            "- `{}` class={class}{wildcard} schema=#{}",
            entry.media_type, entry.schema.0
        );
        if let Some(override_id) = entry.stream_item_override {
            indent(out, spaces + 4);
            let _ = writeln!(out, "stream-item-override: #{}", override_id.0);
        }
    }
}

fn media_class_label(class: MediaClass) -> &'static str {
    match class {
        MediaClass::JsonFamily => "json",
        MediaClass::PlainText => "text",
        MediaClass::Binary => "binary",
        MediaClass::UrlEncodedForm => "form",
        MediaClass::Multipart => "multipart",
        MediaClass::EventStream => "sse",
        MediaClass::Ndjson => "ndjson",
        MediaClass::JsonSeq => "json-seq",
        MediaClass::RawUnknown => "raw",
    }
}
