//! Document-level IR: version, servers, paths/operations, request bodies,
//! responses, and media-type classification (main spec §5, §18.1, §23, §24,
//! §35; companion §6, §8).

use std::collections::BTreeMap;

use crate::ir::schema::{SchemaArena, SchemaId};

/// A loaded OpenAPI document after `$ref` resolution and version
/// normalization (companion §2). This package ends here; composition
/// merging, naming, and codegen are later work packages.
#[derive(Debug)]
pub struct IrDocument {
    /// Parsed release family.
    pub version: OpenApiVersion,
    /// Original `openapi` string from the document.
    pub raw_version: String,
    /// Root-level servers in declaration order.
    pub servers: Vec<ServerIr>,
    /// Path entries in declaration order.
    pub paths: Vec<PathEntry>,
    /// `components/schemas` names mapped to arena ids.
    pub schemas: BTreeMap<String, SchemaId>,
    /// All interned schema nodes.
    pub arena: SchemaArena,
}

/// Supported release families (companion §2: 3.0.x, 3.1.x, 3.2.x).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenApiVersion {
    V3_0,
    V3_1,
    V3_2,
}

impl OpenApiVersion {
    #[must_use]
    pub fn is_at_least_3_1(self) -> bool {
        matches!(self, Self::V3_1 | Self::V3_2)
    }
}

/// Server object; URLs kept verbatim (relative resolution is a codegen
/// concern per companion §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIr {
    pub url: String,
    /// Variables in declaration order.
    pub variables: Vec<(String, ServerVariable)>,
}

/// Server variable with declared default and optional allowed values
/// (companion §8: builder parameters with defaults and enum validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerVariable {
    pub default: String,
    pub allowed_enum: Option<Vec<String>>,
}

/// One entry of the root `paths` object.
#[derive(Debug, Clone)]
pub struct PathEntry {
    /// Path template verbatim, e.g. `/widgets/{id}`.
    pub path: String,
    /// Path-level `servers`, overriding root-level (companion §8.2);
    /// `None` when absent.
    pub servers: Option<Vec<ServerIr>>,
    /// Path-level parameters; merged into operations during normalization
    /// by a later work package — levels are recorded separately here.
    pub parameters: Vec<ParameterIr>,
    /// Operations in document declaration order.
    pub operations: Vec<(HttpMethod, OperationIr)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Put,
    Post,
    Delete,
    Options,
    Head,
    Patch,
    Trace,
}

impl HttpMethod {
    /// Lowercase keyword used in the source document.
    #[must_use]
    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Put => "put",
            Self::Post => "post",
            Self::Delete => "delete",
            Self::Options => "options",
            Self::Head => "head",
            Self::Patch => "patch",
            Self::Trace => "trace",
        }
    }

    /// Parses an operation keyword; `None` for anything else.
    #[must_use]
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Some(match keyword {
            "get" => Self::Get,
            "put" => Self::Put,
            "post" => Self::Post,
            "delete" => Self::Delete,
            "options" => Self::Options,
            "head" => Self::Head,
            "patch" => Self::Patch,
            "trace" => Self::Trace,
            _ => return None,
        })
    }
}

/// One operation.
#[derive(Debug, Clone)]
pub struct OperationIr {
    /// Raw `operationId` before any naming pipeline runs (companion §10).
    pub operation_id: Option<String>,
    /// Raw `tags`; sanitized to module names by the naming pipeline
    /// (companion §10).
    pub tags: Vec<String>,
    /// Operation-level parameters only; path-level ones live on
    /// [`PathEntry::parameters`] and are merged during normalization.
    pub parameters: Vec<ParameterIr>,
    pub request_body: Option<RequestBodyIr>,
    /// Responses in declaration order; 1xx keys/ranges are rejected at parse
    /// time (main spec §35 consistency rule) and never interned.
    pub responses: Vec<ResponseEntryIr>,
    /// Operation-level servers overriding path/root (companion §8.1).
    pub servers: Option<Vec<ServerIr>>,
    pub deprecated: bool,
}

/// Parameter after `$ref` resolution.
///
/// OAS style/explode/allow-reserved defaults are applied at load time so the
/// IR is total; downstream packages validate location/style combinations
/// (companion §6).
#[derive(Debug, Clone)]
pub struct ParameterIr {
    pub name: String,
    pub location: ParameterLocation,
    pub required: bool,
    pub schema: SchemaId,
    pub style: ParameterStyle,
    pub explode: bool,
    pub allow_reserved: bool,
    /// `summary` sibling carried from an OAS 3.1+/3.2 Reference Object
    /// wrapping this parameter's `$ref` (companion §3); inline parameters
    /// leave it `None`.
    pub summary: Option<String>,
    /// `description` sibling of the same Reference Object or, when absent,
    /// the resolved Parameter Object's own description (companion §3).
    pub description: Option<String>,
}

/// Wire location of a parameter. The 3.2 `querystring` location is rejected
/// at parse time with an error diagnostic (companion §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterLocation {
    Path,
    Query,
    Header,
    Cookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterStyle {
    Matrix,
    Label,
    Form,
    Simple,
    SpaceDelimited,
    PipeDelimited,
    DeepObject,
}

/// Request body; absent bodies are `None` on [`OperationIr::request_body`].
#[derive(Debug, Clone)]
pub struct RequestBodyIr {
    pub required: bool,
    /// Media type entries in declaration order.
    pub content: Vec<ContentEntryIr>,
    /// `summary` sibling carried from an OAS 3.1+/3.2 Reference Object
    /// wrapping this body's `$ref` (companion §3); inline bodies leave it
    /// `None`.
    pub summary: Option<String>,
    /// `description` sibling of the same Reference Object or, when absent,
    /// the resolved Request Body Object's own description (companion §3).
    pub description: Option<String>,
}

/// One media-type entry of a request body or response content map.
#[derive(Debug, Clone)]
pub struct ContentEntryIr {
    /// Media range key verbatim (including parameters such as `;charset=…`).
    pub media_type: String,
    pub media_class: MediaClass,
    /// True for ranges like `text/*`, `application/*`, `*/*` which must be
    /// served as raw streaming bodies (main spec §5.10).
    pub is_wildcard: bool,
    pub schema: SchemaId,
    /// `x-rust-stream-item` override: when present it names the streamed
    /// item type while `schema` describes the envelope (main spec §18.1).
    pub stream_item_override: Option<SchemaId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaClass {
    /// `application/json`, `*+json` suffixes (§5.1).
    JsonFamily,
    /// Known plain-text types (§5.2).
    PlainText,
    Binary,
    UrlEncodedForm,
    Multipart,
    EventStream,
    Ndjson,
    JsonSeq,
    /// Anything unclassified, including wildcard ranges (§5.9, §5.10).
    RawUnknown,
}

/// One response keyed by status.
#[derive(Debug, Clone)]
pub struct ResponseEntryIr {
    pub status: ResponseStatusKey,
    /// Typed headers; wire names preserved verbatim (companion §6/D-§10),
    /// declaration order.
    pub headers: Vec<(String, HeaderSpecIr)>,
    pub content: Vec<ContentEntryIr>,
    /// `summary` sibling carried from an OAS 3.1+/3.2 Reference Object
    /// wrapping this response's `$ref` (companion §3); inline responses
    /// leave it `None`.
    pub summary: Option<String>,
    /// `description` sibling of the same Reference Object or, when absent,
    /// the resolved Response Object's own description (companion §3).
    pub description: Option<String>,
}

/// Response key after parsing (main spec §23, §24, §35): explicit codes,
/// range classes, or `default`. Informational statuses are rejected at parse
/// time and never reach the IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseStatusKey {
    Explicit(u16),
    RangeClass(RangeClass),
    Default,
}

/// Status range classes accepted in response keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeClass {
    Success2xx,
    Redirection3xx,
    ClientError4xx,
    ServerError5xx,
}

/// Typed response header (Header Object resolved).
#[derive(Debug, Clone)]
pub struct HeaderSpecIr {
    pub required: bool,
    pub schema: SchemaId,
    /// `summary` sibling carried from an OAS 3.1+/3.2 Reference Object
    /// wrapping this header's `$ref` (companion §3); inline headers leave
    /// it `None`.
    pub summary: Option<String>,
    /// `description` sibling of the same Reference Object or, when absent,
    /// the resolved Header Object's own description (companion §3).
    pub description: Option<String>,
}

/// Classifies a media-range key per main spec §5. Returns the semantic class
/// and whether the key is a wildcard range (`*/*`, `type/*`). Matching is on
/// the type/subtype only; parameters (`;charset=…`) are stripped.
///
/// Order of precedence: JSON family (exact + `+json` suffix), SSE, NDJSON,
/// JSON sequences, URL-encoded forms, multipart, known plain text, binary
/// families, then everything else as raw unknown.
#[must_use]
pub fn classify_media_type(raw: &str) -> (MediaClass, bool) {
    let base = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let Some((ty, sub)) = base.split_once('/') else {
        return (MediaClass::RawUnknown, false);
    };
    if ty == "*" || sub == "*" {
        // Wildcard ranges are raw streaming unless a concrete entry matched
        // first at runtime (main spec §5.10); multipart/* keeps its class
        // because §5.5/§6 route every multipart/* generically to multipart
        // streaming.
        if ty == "multipart" {
            return (MediaClass::Multipart, true);
        }
        return (MediaClass::RawUnknown, true);
    }
    let class = if sub.ends_with("+json") || (ty == "application" && sub == "json") {
        MediaClass::JsonFamily
    } else if ty == "text" && sub == "event-stream" {
        MediaClass::EventStream
    } else if ty == "application" && matches!(sub, "x-ndjson" | "ndjson" | "jsonl") {
        MediaClass::Ndjson
    } else if ty == "application" && sub == "json-seq" {
        MediaClass::JsonSeq
    } else if ty == "application" && sub == "x-www-form-urlencoded" {
        MediaClass::UrlEncodedForm
    } else if ty == "multipart" {
        MediaClass::Multipart
    } else if (ty == "text" && matches!(sub, "plain" | "html" | "csv" | "markdown"))
        || (ty == "application" && sub == "sql")
    {
        MediaClass::PlainText
    } else if (ty == "application" && matches!(sub, "octet-stream" | "pdf" | "zip" | "gzip"))
        || matches!(ty, "image" | "audio" | "video" | "font")
    {
        MediaClass::Binary
    } else {
        MediaClass::RawUnknown
    };
    (class, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_of(raw: &str) -> MediaClass {
        classify_media_type(raw).0
    }

    #[test]
    fn json_family_includes_suffix_and_problem() {
        assert_eq!(class_of("application/json"), MediaClass::JsonFamily);
        assert_eq!(class_of("application/problem+json"), MediaClass::JsonFamily);
        assert_eq!(
            class_of("application/vnd.acme.widget+json"),
            MediaClass::JsonFamily
        );
        assert_eq!(
            class_of("application/json;charset=utf-8"),
            MediaClass::JsonFamily
        );
    }

    #[test]
    fn stream_and_form_families() {
        assert_eq!(class_of("text/event-stream"), MediaClass::EventStream);
        assert_eq!(class_of("application/x-ndjson"), MediaClass::Ndjson);
        assert_eq!(class_of("application/ndjson"), MediaClass::Ndjson);
        assert_eq!(class_of("application/jsonl"), MediaClass::Ndjson);
        assert_eq!(class_of("application/json-seq"), MediaClass::JsonSeq);
        assert_eq!(
            class_of("application/x-www-form-urlencoded"),
            MediaClass::UrlEncodedForm
        );
        assert_eq!(class_of("multipart/form-data"), MediaClass::Multipart);
        assert_eq!(class_of("multipart/mixed"), MediaClass::Multipart);
    }

    #[test]
    fn plain_text_binary_and_unknown() {
        assert_eq!(class_of("text/plain"), MediaClass::PlainText);
        assert_eq!(class_of("text/markdown"), MediaClass::PlainText);
        assert_eq!(class_of("application/sql"), MediaClass::PlainText);
        assert_eq!(class_of("application/octet-stream"), MediaClass::Binary);
        assert_eq!(class_of("image/png"), MediaClass::Binary);
        assert_eq!(class_of("video/mp4"), MediaClass::Binary);
        assert_eq!(class_of("font/woff2"), MediaClass::Binary);
        assert_eq!(class_of("application/cbor"), MediaClass::RawUnknown);
        assert_eq!(class_of("application/xml"), MediaClass::RawUnknown);
    }

    #[test]
    fn wildcards_flagged_raw() {
        let (class, wildcard) = classify_media_type("*/*");
        assert_eq!((class, wildcard), (MediaClass::RawUnknown, true));
        let (class, wildcard) = classify_media_type("text/*");
        assert_eq!((class, wildcard), (MediaClass::RawUnknown, true));
        let (class, wildcard) = classify_media_type("application/*");
        assert_eq!((class, wildcard), (MediaClass::RawUnknown, true));
        let (class, wildcard) = classify_media_type("multipart/*");
        assert_eq!((class, wildcard), (MediaClass::Multipart, true));
    }

    #[test]
    fn malformed_media_type_is_raw_unknown() {
        assert_eq!(
            classify_media_type("noslash"),
            (MediaClass::RawUnknown, false)
        );
    }

    #[test]
    fn http_method_keywords_round_trip() {
        for keyword in [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ] {
            let method = HttpMethod::from_keyword(keyword).unwrap();
            assert_eq!(method.as_keyword(), keyword);
        }
        assert!(HttpMethod::from_keyword("query").is_none());
    }
}
