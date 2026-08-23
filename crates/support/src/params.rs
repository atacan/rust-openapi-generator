//! OpenAPI parameter serialization and deserialization (companion §6).
//!
//! Implements the full OAS `style` × `explode` matrix derived from RFC 6570
//! (companion §6 "Decided"): form + explode is the default for query and
//! cookie parameters, `simple` for path and header; `label`/`matrix` extend
//! path, `spaceDelimited`/`pipeDelimited`/`deepObject` extend query, and
//! `allowReserved` is honored for query values. Cookie parameters travel as
//! ordinary `Cookie` header material in both directions — no cookie jar
//! (companion §6). Unknown style/location combinations are generation errors
//! upstream; this runtime accepts any combination it is handed through the
//! documented fallbacks below.
//!
//! # Wire conventions fixed by this module
//!
//! * **Structural characters are never escaped; atomic pieces always are.**
//!   Encoders insert delimiters (`,` `.` `;` `=` space `|`) raw and
//!   percent-escape each atomic piece; decoders split on the raw delimiters
//!   first and percent-decode the pieces afterwards.
//! * **Query space handling.** Form style reuses the WHATWG
//!   `application/x-www-form-urlencoded` serializer from
//!   `crate::percent::encode_query_component`: space serializes as `+`.
//!   Every other query encoding — `spaceDelimited`, `pipeDelimited`,
//!   `deepObject`, and any style when `allowReserved` is set — uses RFC 3986
//!   strict escaping where space serializes as `%20` and a literal `+` is
//!   escaped like any other reserved byte. The decoder mirrors this exactly:
//!   `+` reads as space only for form style with `allowReserved = false`.
//! * **`allowReserved`** leaves the RFC 3986 reserved set
//!   `:/?#[]@!$&'()*+,;=` literal in query values while still escaping
//!   spaces, controls, and non-ASCII bytes. Delimiters staying literal means
//!   values that themselves contain the style's delimiter no longer
//!   round-trip unambiguously — inherent to the OAS feature and honored here
//!   as a documented lossy corner.
//! * **Shape fidelity.** The wire cannot always express container shape: a
//!   one-element exploded array is byte-identical to the bare scalar, an
//!   empty exploded composite emits nothing at all, and a non-exploded
//!   object list is indistinguishable from an even-length array.
//!   [`decode_query`], [`decode_header_value`], and [`decode_path_segment`]
//!   reconstruct the richest plausible shape; the `*_shaped` variants take
//!   the schema's expected [`ParamShape`] and invert every encoding exactly.
//!   The exploded-form object shape is nameless on the wire, so generic
//!   decoding of it additionally requires pairs pre-filtered to the
//!   parameter (generated code splits by known property names anyway).
//! * **Decoded scalars are text.** The wire carries no type information, so
//!   decoders produce [`ParamValue::Text`]; generated code parses numbers
//!   and booleans against the schema it already knows.
//! * **Composite leaves flatten.** An array or object nested inside another
//!   composite renders through [`ParamValue::to_text`] (comma-joined items /
//!   `prop,val` alternation), mirroring the non-exploded `simple` shape.
//!   `deepObject` is the exception mandated by companion §6: values below
//!   one level are an [`ParamEncodeError::UnsupportedShape`] error.
//! * **Headers are verbatim.** Header values are not percent-encoded
//!   (matching the OAS `simple`/delimited header examples), so a value
//!   containing its style's delimiter is ambiguous on the wire.
//! * **Cookies are form-encoded** regardless of `spec.style` (OAS cookie
//!   semantics are form); decode them with [`parse_cookie_header`] followed
//!   by [`decode_query`] using a [`ParamStyle::Form`] spec with
//!   `allow_reserved = false`. `allow_reserved` is query-only per OAS and is
//!   ignored for cookies, paths, and headers.
//! * **Determinism.** Object entries keep declaration order end to end;
//!   where a single value is expected but the wire repeats a key, the last
//!   occurrence wins (WHATWG convention).

use crate::percent;

/// Schema-typed parameter value (companion §6).
///
/// Generated converters turn schema types into these variants; the wire
/// renderers below are the single place that knows OAS style rules.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// Free-form text (also the decode product for every scalar).
    Text(String),
    /// Signed integer.
    Int(i64),
    /// Floating-point number; rendered with standard `Display` (so `1.0`
    /// prints as `1` and non-finite values print as `NaN`/`inf` — such
    /// values never round-trip and must be rejected by generated validation).
    Float(f64),
    /// Boolean, rendered `true`/`false`.
    Bool(bool),
    /// Sequence; declaration order preserved.
    Array(Vec<ParamValue>),
    /// Mapping with declaration order preserved (companion §6 determinism).
    Object(Vec<(String, ParamValue)>),
}

impl ParamValue {
    /// Flat text rendering: scalars print directly (numbers via standard
    /// `Display`, bools as `true`/`false`), arrays join items with `,`, and
    /// objects alternate `prop,val`. Composite values nested inside other
    /// composites flatten through this method.
    #[must_use]
    pub fn to_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Array(items) => {
                let pieces: Vec<String> = items.iter().map(Self::to_text).collect();
                pieces.join(",")
            }
            Self::Object(entries) => {
                let mut pieces = Vec::with_capacity(entries.len() * 2);
                for (prop, value) in entries {
                    pieces.push(prop.clone());
                    pieces.push(value.to_text());
                }
                pieces.join(",")
            }
        }
    }
}

impl From<&str> for ParamValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for ParamValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<i64> for ParamValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for ParamValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<bool> for ParamValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// OAS parameter location (`in`, companion §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamLocation {
    /// Path template parameter.
    Path,
    /// URL query parameter.
    Query,
    /// Header parameter.
    Header,
    /// Cookie parameter traveling inside the `Cookie` header (companion §6).
    Cookie,
}

/// OAS parameter style (companion §6), mirroring the OpenAPI `style` values.
///
/// Defined here rather than reused from the generator IR: the support crate
/// is independent of the generator and accepts any style/location
/// combination the generator passes (invalid combinations are rejected at
/// generation time, not here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamStyle {
    /// Path/header default; comma-joined.
    Simple,
    /// RFC 6570 label expansion: `.value` (path).
    Label,
    /// RFC 6570 matrix expansion: `;name=value` (path).
    Matrix,
    /// Query/cookie default; form-urlencoded semantics.
    Form,
    /// Items joined with a single space (query; `%20`, never `+`).
    SpaceDelimited,
    /// Items joined with `|` (query).
    PipeDelimited,
    /// Bracketed one-level property keys `name[prop]=val` (query only).
    DeepObject,
}

/// Static description of one parameter: its wire name and serialization
/// knobs (companion §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// Wire name of the parameter (template/header/cookie name).
    pub name: String,
    /// OAS style controlling the wire shape.
    pub style: ParamStyle,
    /// OAS `explode` flag.
    pub explode: bool,
    /// OAS `allowReserved`: keep RFC 3986 reserved characters literal in
    /// query values. Ignored outside the query location (see module docs).
    pub allow_reserved: bool,
}

impl ParamSpec {
    /// Builds a spec from its parts.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        style: ParamStyle,
        explode: bool,
        allow_reserved: bool,
    ) -> Self {
        Self {
            name: name.into(),
            style,
            explode,
            allow_reserved,
        }
    }
}

/// Expected container shape of a parameter, taken from the schema.
///
/// Wire formats cannot always self-describe shape (see the module-level
/// "Shape fidelity" note), so the `*_shaped` decode entry points accept the
/// schema's expectation and invert encodings exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamShape {
    /// Single scalar value.
    Scalar,
    /// Sequence of scalars.
    Array,
    /// Mapping of scalar properties.
    Object,
}

/// Encode-side failure: the value's shape cannot be expressed in the
/// requested style (companion §6 restricts `deepObject` to one level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unsupported value shape for parameter serialization: {0}")]
pub enum ParamEncodeError {
    /// The value shape is not representable in this style.
    UnsupportedShape(&'static str),
}

/// Decode-side failure (server rejections surface as `400`-class
/// rejections; client decoding surfaces a decode error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParamDecodeError {
    /// The raw wire value violates the style's grammar.
    #[error("malformed parameter value: {0}")]
    Malformed(&'static str),
    /// The value shape cannot exist in the requested style.
    #[error("unsupported value shape for parameter serialization: {0}")]
    UnsupportedShape(&'static str),
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Renders a path-template parameter (companion §6): label `.value`, matrix
/// `;name=value`, simple `value`. Atomic pieces percent-encode with the
/// RFC 3986 unreserved set; structural delimiters stay raw.
///
/// Styles that OAS does not define for path (`form`, delimited, deepObject)
/// render with `simple` shapes — the runtime-side fallback for combinations
/// the generator is expected to reject.
#[must_use]
pub fn encode_path(spec: &ParamSpec, value: &ParamValue) -> String {
    match spec.style {
        ParamStyle::Label => encode_label(spec, value),
        ParamStyle::Matrix => encode_matrix(spec, value),
        ParamStyle::Simple
        | ParamStyle::Form
        | ParamStyle::SpaceDelimited
        | ParamStyle::PipeDelimited
        | ParamStyle::DeepObject => render_composite(spec, value, ",", path_escape),
    }
}

/// Renders one query parameter as wire-ready `key`/`value` pairs, appended
/// in order by the caller. Pair keys and values are percent-encoded per the
/// module conventions (form style uses `+` for space; all other styles use
/// `%20`). A non-exploded empty composite emits one presence-marker pair
/// `name=`; an exploded empty composite emits nothing.
///
/// Styles that OAS does not define for query (`simple`, `label`, `matrix`)
/// render with form shapes as the runtime-side fallback.
///
/// # Errors
///
/// [`ParamEncodeError::UnsupportedShape`] when `deepObject` receives a
/// non-object value or an object whose values are not scalars (companion §6
/// fixes `deepObject` at exactly one level).
pub fn encode_query_pairs(
    spec: &ParamSpec,
    value: &ParamValue,
) -> Result<Vec<(String, String)>, ParamEncodeError> {
    let policy = QueryEscape::for_spec(spec);
    let key = escaped_piece(policy, &spec.name);

    if spec.style == ParamStyle::DeepObject {
        let mut pairs = Vec::new();
        encode_deep_object(&key, policy, value, &mut pairs)?;
        return Ok(pairs);
    }

    // Non-exploded composite join character: the style's delimiter, or a
    // comma for form/simple/label/matrix fallbacks.
    let joiner = query_joiner(spec.style);
    let mut pairs = Vec::new();
    match value {
        ParamValue::Array(items) if spec.explode => {
            for item in items {
                pairs.push((key.clone(), escaped_piece(policy, &item.to_text())));
            }
        }
        ParamValue::Array(items) => {
            pairs.push((key, join_pieces(policy, items, joiner)));
        }
        ParamValue::Object(entries) if spec.explode => {
            for (prop, item) in entries {
                pairs.push((
                    escaped_piece(policy, prop),
                    escaped_piece(policy, &item.to_text()),
                ));
            }
        }
        ParamValue::Object(entries) => {
            let mut pieces = Vec::with_capacity(entries.len() * 2);
            for (prop, item) in entries {
                pieces.push(escaped_piece(policy, prop));
                pieces.push(escaped_piece(policy, &item.to_text()));
            }
            pairs.push((key, pieces.join(joiner)));
        }
        scalar => pairs.push((key, escaped_piece(policy, &scalar.to_text()))),
    }
    Ok(pairs)
}

/// Renders a header parameter value (companion §6): `simple` (default) or
/// the space/pipe delimited variants. Values are verbatim — headers are not
/// percent-encoded (see module docs). `explode` affects only objects
/// (`prop=val` joined by the delimiter) because header arrays have no
/// repetition channel. Non-header styles render with `simple` shapes.
#[must_use]
pub fn encode_header_value(spec: &ParamSpec, value: &ParamValue) -> String {
    render_composite(spec, value, header_joiner(spec.style), str::to_owned)
}

/// Renders a cookie parameter as `name=value` segment(s) joined with `"; "`
/// (companion §6: cookie parameters travel inside the `Cookie` header, no
/// jar). Form semantics apply regardless of `spec.style`; values percent-
/// encode with the WHATWG form rules (space `+`, `;`/`,` escaped), so
/// segments compose safely via [`build_cookie_header`] or concatenation and
/// decode through [`parse_cookie_header`] plus [`decode_query`].
#[must_use]
pub fn encode_cookie_value(spec: &ParamSpec, value: &ParamValue) -> String {
    let escape = percent::encode_query_component;
    let key = escape(&spec.name);
    let mut segments = Vec::new();
    match value {
        ParamValue::Array(items) if spec.explode => {
            for item in items {
                segments.push(format!("{key}={}", escape(&item.to_text())));
            }
        }
        ParamValue::Array(items) => {
            let joined = items
                .iter()
                .map(|item| escape(&item.to_text()))
                .collect::<Vec<_>>()
                .join(",");
            segments.push(format!("{key}={joined}"));
        }
        ParamValue::Object(entries) if spec.explode => {
            for (prop, item) in entries {
                segments.push(format!("{}={}", escape(prop), escape(&item.to_text())));
            }
        }
        ParamValue::Object(entries) => {
            let mut pieces = Vec::with_capacity(entries.len() * 2);
            for (prop, item) in entries {
                pieces.push(escape(prop));
                pieces.push(escape(&item.to_text()));
            }
            segments.push(format!("{key}={}", pieces.join(",")));
        }
        scalar => segments.push(format!("{key}={}", escape(&scalar.to_text()))),
    }
    segments.join("; ")
}

/// Joins prepared `k=v` pairs into one `Cookie` header value
/// (companion §6): `"k1=v1; k2=v2"`.
#[must_use]
pub fn build_cookie_header(pairs: &[(String, String)]) -> String {
    let segments: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    segments.join("; ")
}

/// Leniently splits a raw `Cookie` header into pairs (companion §6): split
/// on `;`, trim whitespace, and treat a segment without `=` as a name with
/// an empty value. Empty segments are skipped.
#[must_use]
pub fn parse_cookie_header(raw: &str) -> Vec<(String, String)> {
    raw.split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment.split_once('=') {
            Some((key, value)) => (key.trim().to_owned(), value.trim().to_owned()),
            None => (segment.to_owned(), String::new()),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Encoding internals
// ---------------------------------------------------------------------------

/// RFC 3986 reserved set kept literal by `allowReserved` (companion §6):
/// gen-delims plus sub-delims.
const RFC3986_RESERVED: &[u8] = b":/?#[]@!$&'()*+,;=";

/// Query percent-encoding policy (module docs, "Query space handling").
#[derive(Debug, Clone, Copy)]
enum QueryEscape {
    /// WHATWG form rules: space `+`, `*` literal (form style only).
    FormPlus,
    /// RFC 3986 strict: space `%20`; reserved bytes literal only when
    /// `allowReserved` is set.
    Strict(bool),
}

impl QueryEscape {
    fn for_spec(spec: &ParamSpec) -> Self {
        match spec.style {
            // Form style keeps the WHATWG urlencoded convention shared with
            // form bodies (percent.rs); everything else is RFC 3986 strict,
            // honoring allowReserved by leaving reserved bytes literal.
            ParamStyle::Form if !spec.allow_reserved => Self::FormPlus,
            _ => Self::Strict(spec.allow_reserved),
        }
    }

    fn push_escaped(self, out: &mut String, piece: &str) {
        match self {
            Self::FormPlus => out.push_str(&percent::encode_query_component(piece)),
            Self::Strict(allow_reserved) => {
                const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                for &byte in piece.as_bytes() {
                    match byte {
                        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                            out.push(byte as char);
                        }
                        _ if allow_reserved && RFC3986_RESERVED.contains(&byte) => {
                            out.push(byte as char);
                        }
                        _ => {
                            out.push('%');
                            out.push(HEX_DIGITS[usize::from(byte >> 4)] as char);
                            out.push(HEX_DIGITS[usize::from(byte & 0x0F)] as char);
                        }
                    }
                }
            }
        }
    }
}

fn escaped_piece(policy: QueryEscape, text: &str) -> String {
    let mut out = String::new();
    policy.push_escaped(&mut out, text);
    out
}

fn join_pieces(policy: QueryEscape, items: &[ParamValue], joiner: &str) -> String {
    let pieces: Vec<String> = items
        .iter()
        .map(|item| escaped_piece(policy, &item.to_text()))
        .collect();
    pieces.join(joiner)
}

fn query_joiner(style: ParamStyle) -> &'static str {
    match style {
        // Query context never carries raw spaces: the delimiter itself
        // serializes as `%20` (module docs, "Query space handling"). Values
        // containing a space are therefore ambiguous on the wire — same
        // documented lossy corner as allowReserved delimiters.
        ParamStyle::SpaceDelimited => "%20",
        ParamStyle::PipeDelimited => "|",
        _ => ",",
    }
}

fn header_joiner(style: ParamStyle) -> &'static str {
    match style {
        ParamStyle::SpaceDelimited => " ",
        ParamStyle::PipeDelimited => "|",
        _ => ",",
    }
}

/// Path pieces escape with the RFC 3986 unreserved set (companion §8), so
/// every structural delimiter of label/matrix/simple is escaped inside
/// values and stays raw only where the renderer places it.
fn path_escape(piece: &str) -> String {
    percent::encode_path_segment(piece)
}

/// Shared renderer for `simple`-shaped composites (path simple and all
/// header styles): arrays join with the delimiter, non-exploded objects
/// alternate `prop,val`, exploded objects emit `prop=val` entries, scalars
/// stand alone. `escape` applies to every atomic piece.
fn render_composite(
    spec: &ParamSpec,
    value: &ParamValue,
    joiner: &str,
    escape: impl Fn(&str) -> String,
) -> String {
    match value {
        ParamValue::Array(items) => {
            let pieces: Vec<String> = items.iter().map(|item| escape(&item.to_text())).collect();
            pieces.join(joiner)
        }
        ParamValue::Object(entries) if spec.explode => {
            let pieces: Vec<String> = entries
                .iter()
                .map(|(prop, item)| format!("{}={}", escape(prop), escape(&item.to_text())))
                .collect();
            pieces.join(joiner)
        }
        ParamValue::Object(entries) => {
            // Non-exploded: flat prop,val alternation joined pairwise.
            let mut flat = Vec::with_capacity(entries.len() * 2);
            for (prop, item) in entries {
                flat.push(escape(prop));
                flat.push(escape(&item.to_text()));
            }
            flat.join(joiner)
        }
        scalar => escape(&scalar.to_text()),
    }
}

/// RFC 6570 label expansion (companion §6): leading `.`; exploded arrays
/// join with `.`, exploded objects emit `prop=val` entries joined with `.`,
/// non-exploded composites stay comma-joined inside the prefix.
fn encode_label(spec: &ParamSpec, value: &ParamValue) -> String {
    let mut out = String::from(".");
    if spec.explode {
        match value {
            ParamValue::Array(items) => {
                let pieces: Vec<String> = items
                    .iter()
                    .map(|item| path_escape(&item.to_text()))
                    .collect();
                out.push_str(&pieces.join("."));
            }
            ParamValue::Object(entries) => {
                let pieces: Vec<String> = entries
                    .iter()
                    .map(|(prop, item)| {
                        format!("{}={}", path_escape(prop), path_escape(&item.to_text()))
                    })
                    .collect();
                out.push_str(&pieces.join("."));
            }
            scalar => out.push_str(&path_escape(&scalar.to_text())),
        }
    } else {
        out.push_str(&render_composite(spec, value, ",", path_escape));
    }
    out
}

/// RFC 6570 matrix expansion (companion §6): `;name=value`. Exploded arrays
/// repeat `;name=item`; exploded objects drop the name and emit one
/// `;prop=val` per entry; non-exploded composites comma-join inside a
/// single `name=`. A non-exploded empty composite keeps its presence marker
/// (`;name=`); an exploded empty composite emits nothing.
fn encode_matrix(spec: &ParamSpec, value: &ParamValue) -> String {
    let name = path_escape(&spec.name);
    match value {
        ParamValue::Array(items) if spec.explode => {
            let mut out = String::new();
            for item in items {
                out.push(';');
                out.push_str(&name);
                out.push('=');
                out.push_str(&path_escape(&item.to_text()));
            }
            out
        }
        ParamValue::Array(items) => {
            let pieces: Vec<String> = items
                .iter()
                .map(|item| path_escape(&item.to_text()))
                .collect();
            format!(";{name}={}", pieces.join(","))
        }
        ParamValue::Object(entries) if spec.explode => {
            let mut out = String::new();
            for (prop, item) in entries {
                out.push(';');
                out.push_str(&path_escape(prop));
                out.push('=');
                out.push_str(&path_escape(&item.to_text()));
            }
            out
        }
        ParamValue::Object(entries) => {
            let mut flat = Vec::with_capacity(entries.len() * 2);
            for (prop, item) in entries {
                flat.push(path_escape(prop));
                flat.push(path_escape(&item.to_text()));
            }
            format!(";{name}={}", flat.join(","))
        }
        scalar => format!(";{name}={}", path_escape(&scalar.to_text())),
    }
}

/// deepObject (query only, companion §6): `name[prop]=val` per property,
/// exactly one level. Deeper nesting is an error, not silent truncation.
fn encode_deep_object(
    key: &str,
    policy: QueryEscape,
    value: &ParamValue,
    pairs: &mut Vec<(String, String)>,
) -> Result<(), ParamEncodeError> {
    let entries = match value {
        ParamValue::Object(entries) => entries,
        _ => {
            return Err(ParamEncodeError::UnsupportedShape(
                "deepObject style requires an Object parameter value",
            ))
        }
    };
    for (prop, item) in entries {
        let text = match item {
            // Companion §6: nesting beyond one level is unsupported.
            ParamValue::Array(_) | ParamValue::Object(_) => {
                return Err(ParamEncodeError::UnsupportedShape(
                    "deepObject values must be scalars (one level)",
                ))
            }
            scalar => scalar.to_text(),
        };
        let prop_key = escaped_piece(policy, prop);
        pairs.push((format!("{key}[{prop_key}]"), escaped_piece(policy, &text)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Percent-decodes one atomic piece. When `plus_as_space` is set (form style
/// with `allowReserved = false` only — see module docs) `+` reads as space;
/// everywhere else `+` is literal because strict encoders always escape it.
fn percent_decode(raw: &str, plus_as_space: bool) -> Result<String, ParamDecodeError> {
    fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .copied()
                    .and_then(hex_value)
                    .ok_or(ParamDecodeError::Malformed("invalid %XY percent escape"))?;
                let low = bytes
                    .get(index + 2)
                    .copied()
                    .and_then(hex_value)
                    .ok_or(ParamDecodeError::Malformed("invalid %XY percent escape"))?;
                out.push((high << 4) | low);
                index += 3;
            }
            b'+' if plus_as_space => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out)
        .map_err(|_| ParamDecodeError::Malformed("percent-decoded bytes are not valid UTF-8"))
}

/// Whether `+` decodes to space for this spec: form style keeps the WHATWG
/// convention only when `allowReserved` is off (module docs).
fn plus_is_space(spec: &ParamSpec) -> bool {
    spec.style == ParamStyle::Form && !spec.allow_reserved
}

/// Decodes one query parameter from raw (still percent-encoded) wire pairs.
///
/// Shape is inferred from the wire: repeated `name=` pairs become an array,
/// a single pair a scalar. Missing input (no pair carries the name, or no
/// `name[` keys for deepObject) yields `Ok(None)` rather than an empty
/// value. The exploded-form OBJECT wire shape is nameless and therefore
/// never inferred — decode it with [`decode_query_shaped`] and
/// [`ParamShape::Object`] over pairs pre-filtered to this parameter.
pub fn decode_query<'a>(
    spec: &ParamSpec,
    raw_pairs: impl Iterator<Item = (&'a str, &'a str)> + Clone,
) -> Result<Option<ParamValue>, ParamDecodeError> {
    let pairs: Vec<(&str, &str)> = raw_pairs.collect();
    if pairs.is_empty() {
        return Ok(None);
    }
    if spec.style == ParamStyle::DeepObject {
        return decode_query_shaped(spec, pairs.into_iter(), ParamShape::Object);
    }

    let plus = plus_is_space(spec);
    let named: Vec<&str> = pairs
        .iter()
        .filter(|(key, _)| query_key_matches(key, &spec.name, plus))
        .map(|(_, value)| *value)
        .collect();
    // Zero named pairs means the parameter is absent. The exploded-object
    // wire shape is nameless, so it can never be fabricated safely from
    // foreign pairs here — use `decode_query_shaped(.., ParamShape::Object)`
    // over pre-filtered pairs for that case (module docs).
    if named.is_empty() {
        return Ok(None);
    }
    let shape = if spec.explode {
        match named.len() {
            1 => ParamShape::Scalar,
            _ => ParamShape::Array,
        }
    } else if named[named.len() - 1].contains(query_joiner(spec.style)) {
        ParamShape::Array
    } else {
        ParamShape::Scalar
    };
    decode_query_shaped(spec, pairs.into_iter(), shape)
}

/// Decodes one query parameter given the schema's expected shape; this
/// inverts [`encode_query_pairs`] exactly for every style × explode × shape
/// combination (module docs, "Shape fidelity").
///
/// # Errors
///
/// [`ParamDecodeError::Malformed`] for grammar violations (bad percent
/// escapes, odd-length object lists, unexpected repetitions).
pub fn decode_query_shaped<'a>(
    spec: &ParamSpec,
    raw_pairs: impl Iterator<Item = (&'a str, &'a str)>,
    shape: ParamShape,
) -> Result<Option<ParamValue>, ParamDecodeError> {
    let pairs: Vec<(&str, &str)> = raw_pairs.collect();
    if spec.style == ParamStyle::DeepObject {
        return decode_deep_object(spec, &pairs);
    }

    let plus = plus_is_space(spec);
    let named: Vec<&str> = pairs
        .iter()
        .filter(|(key, _)| query_key_matches(key, &spec.name, plus))
        .map(|(_, value)| *value)
        .collect();

    match shape {
        ParamShape::Scalar => {
            let last = match named.last() {
                Some(value) => *value,
                None => return Ok(None),
            };
            // Repeated values are the exploded-array encoding; seeing them
            // under a scalar schema is a contract violation, not leniency.
            if spec.explode && named.len() > 1 {
                return Err(ParamDecodeError::Malformed(
                    "repeated query pairs for an exploded scalar parameter",
                ));
            }
            Ok(Some(ParamValue::Text(percent_decode(last, plus)?)))
        }
        ParamShape::Array => {
            if named.is_empty() {
                return Ok(None);
            }
            if !spec.explode {
                // Last occurrence wins for duplicated keys (WHATWG rule).
                return split_composite(named[named.len() - 1], query_joiner(spec.style), plus)
                    .map(|items| Some(ParamValue::Array(items)));
            }
            let mut items = Vec::with_capacity(named.len());
            for raw in named {
                items.push(ParamValue::Text(percent_decode(raw, plus)?));
            }
            Ok(Some(ParamValue::Array(items)))
        }
        ParamShape::Object => {
            if spec.explode {
                // The wire carries bare prop=val pairs without the parameter
                // name; every passed pair belongs to this parameter (callers
                // must pre-filter, see `decode_query`).
                if pairs.is_empty() {
                    return Ok(None);
                }
                return object_from_pairs(pairs.into_iter(), plus).map(Some);
            }
            let last = match named.last() {
                Some(value) => *value,
                None => return Ok(None),
            };
            let pieces = split_pieces(last, query_joiner(spec.style));
            if pieces.len() == 1 && pieces[0].is_empty() {
                return Ok(Some(ParamValue::Object(Vec::new())));
            }
            if pieces.len() % 2 != 0 {
                return Err(ParamDecodeError::Malformed(
                    "odd number of elements for a non-exploded object",
                ));
            }
            let mut entries = Vec::with_capacity(pieces.len() / 2);
            for chunk in pieces.chunks(2) {
                let prop = percent_decode(chunk[0], plus)?;
                let value = percent_decode(chunk[1], plus)?;
                upsert_entry(&mut entries, prop, ParamValue::Text(value));
            }
            Ok(Some(ParamValue::Object(entries)))
        }
    }
}

/// deepObject decode: keys are `name[prop]`, one level only (companion §6).
/// Keys not matching this parameter's prefix are ignored (other parameters
/// share the query string); deeper bracket nesting is a malformed error.
fn decode_deep_object(
    spec: &ParamSpec,
    pairs: &[(&str, &str)],
) -> Result<Option<ParamValue>, ParamDecodeError> {
    // deepObject always uses strict escaping (space `%20`), so `+` is
    // literal in both keys and values regardless of allowReserved.
    let policy = QueryEscape::Strict(spec.allow_reserved);
    let mut prefix = escaped_piece(policy, &spec.name);
    prefix.push('[');

    let mut entries: Vec<(String, ParamValue)> = Vec::new();
    for (key, value) in pairs {
        let rest = match key.strip_prefix(prefix.as_str()) {
            Some(rest) => rest,
            None => continue,
        };
        let prop_raw = match rest.strip_suffix(']') {
            Some(prop_raw) => prop_raw,
            None => {
                return Err(ParamDecodeError::Malformed(
                    "deepObject key is missing its closing ']'",
                ))
            }
        };
        if prop_raw.is_empty() {
            return Err(ParamDecodeError::Malformed(
                "deepObject key has an empty property name",
            ));
        }
        if prop_raw.contains('[') || prop_raw.contains(']') {
            return Err(ParamDecodeError::Malformed(
                "deepObject nesting beyond one level is unsupported",
            ));
        }
        let prop = percent_decode(prop_raw, false)?;
        let decoded = ParamValue::Text(percent_decode(value, false)?);
        upsert_entry(&mut entries, prop, decoded);
    }
    if entries.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ParamValue::Object(entries)))
    }
}

/// Decodes a header parameter value with inferred shape (see
/// [`decode_query`]): two or more delimiter-separated pieces become an
/// array — or an object when every piece carries `=` and the spec explodes.
/// Use [`decode_header_value_shaped`] when the schema shape is known.
/// Header material is verbatim; pieces are trimmed leniently, never
/// percent-decoded.
///
/// Missing input (empty or whitespace-only) yields `Ok(None)`.
pub fn decode_header_value(
    spec: &ParamSpec,
    raw: &str,
) -> Result<Option<ParamValue>, ParamDecodeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let joiner = header_joiner(spec.style);
    let pieces: Vec<&str> = split_pieces(trimmed, joiner)
        .iter()
        .map(|piece| piece.trim())
        .collect();
    let shape = match pieces.len() {
        0 | 1 => ParamShape::Scalar,
        _ if spec.explode && pieces.iter().all(|piece| piece.contains('=')) => ParamShape::Object,
        _ => ParamShape::Array,
    };
    decode_header_value_shaped(spec, raw, shape)
}

/// Decodes a header parameter given the schema's expected shape; inverts
/// [`encode_header_value`] exactly. Verbatim text (no percent-decoding,
/// module docs); `explode` affects objects only (`prop=val` entries),
/// mirroring the encoder.
pub fn decode_header_value_shaped(
    spec: &ParamSpec,
    raw: &str,
    shape: ParamShape,
) -> Result<Option<ParamValue>, ParamDecodeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let joiner = header_joiner(spec.style);
    let pieces: Vec<String> = split_pieces(trimmed, joiner)
        .iter()
        .map(|piece| piece.trim().to_owned())
        .collect();
    match shape {
        ParamShape::Scalar => Ok(Some(ParamValue::Text(trimmed.to_owned()))),
        ParamShape::Array => Ok(Some(ParamValue::Array(
            pieces.into_iter().map(ParamValue::Text).collect(),
        ))),
        ParamShape::Object => {
            if spec.explode {
                let mut entries = Vec::with_capacity(pieces.len());
                for piece in pieces {
                    let (prop, value) = piece.split_once('=').ok_or(
                        ParamDecodeError::Malformed("exploded header object entry is missing '='"),
                    )?;
                    upsert_entry(
                        &mut entries,
                        prop.to_owned(),
                        ParamValue::Text(value.to_owned()),
                    );
                }
                Ok(Some(ParamValue::Object(entries)))
            } else {
                composite_object(pieces).map(Some)
            }
        }
    }
}

/// Decodes a path-template parameter from one raw segment with inferred
/// shape (see [`decode_query`]): label/matrix prefixes self-describe the
/// style's structure; simple segments infer arrays from commas and objects
/// from `=`-bearing exploded entries when possible. Use
/// [`decode_path_segment_shaped`] when the schema shape is known.
///
/// Missing input (empty segment) yields `Ok(None)`.
pub fn decode_path_segment(
    spec: &ParamSpec,
    raw: &str,
) -> Result<Option<ParamValue>, ParamDecodeError> {
    let body = strip_path_prefix(spec, raw)?;
    let Some(body) = body else {
        return Ok(None);
    };

    let (joiner, explode_entries): (&str, bool) = match spec.style {
        ParamStyle::Matrix => {
            let parts = matrix_parts(body)?;
            if parts.len() > 1 {
                // Multiple parts: exploded array (all named) or exploded
                // object (bare props); the structure self-describes.
                return decode_path_segment_shaped(spec, raw, infer_matrix_shape(spec, &parts)?);
            }
            // Single part: decide between scalar and non-exploded composite.
            let (_, value_raw) = matrix_key_value(parts[0])?;
            if value_raw.contains(',') {
                (",", false)
            } else {
                return Ok(Some(ParamValue::Text(percent_decode(value_raw, false)?)));
            }
        }
        ParamStyle::Label => (if spec.explode { "." } else { "," }, spec.explode),
        _ => (",", spec.explode),
    };

    let pieces = split_pieces(body, joiner);
    let shape = match pieces.len() {
        0 | 1 => ParamShape::Scalar,
        _ if explode_entries && pieces.iter().all(|piece| piece.contains('=')) => {
            ParamShape::Object
        }
        _ => ParamShape::Array,
    };
    decode_path_segment_shaped(spec, raw, shape)
}

/// Decodes a path-template parameter given the schema's expected shape;
/// inverts [`encode_path`] exactly for label/matrix/simple. Structural
/// delimiters split first, then each atomic piece percent-decodes (RFC 3986
/// unreserved set was used on encode). Non-path styles fall back to simple
/// shapes, mirroring the encoder.
pub fn decode_path_segment_shaped(
    spec: &ParamSpec,
    raw: &str,
    shape: ParamShape,
) -> Result<Option<ParamValue>, ParamDecodeError> {
    let body = match strip_path_prefix(spec, raw)? {
        Some(body) => body,
        None => return Ok(None),
    };
    match spec.style {
        ParamStyle::Matrix => decode_matrix_body(spec, body, shape).map(Some),
        ParamStyle::Label => {
            // Exploded labels separate array items and object entries with
            // dots; non-exploded composites stay comma-joined (companion §6).
            let joiner = if spec.explode { "." } else { "," };
            decode_simple_body(spec, body, joiner, shape).map(Some)
        }
        _ => decode_simple_body(spec, body, ",", shape).map(Some),
    }
}

// ---------------------------------------------------------------------------
// Decoding internals
// ---------------------------------------------------------------------------

/// Whether a raw wire key names this parameter. Undecodable keys never
/// match (foreign parameters must not fail this parameter's decode); values
/// selected through a match get full validation later.
fn query_key_matches(raw_key: &str, name: &str, plus_as_space: bool) -> bool {
    percent_decode(raw_key, plus_as_space).is_ok_and(|decoded| decoded == name)
}

fn upsert_entry(entries: &mut Vec<(String, ParamValue)>, key: String, value: ParamValue) {
    match entries.iter_mut().find(|(existing, _)| *existing == key) {
        Some(slot) => slot.1 = value,
        None => entries.push((key, value)),
    }
}

/// Splits raw wire text on a structural delimiter before percent-decoding.
fn split_pieces<'a>(raw: &'a str, joiner: &str) -> Vec<&'a str> {
    raw.split(joiner).collect()
}

/// Non-exploded composite decode: single empty piece is the empty-array
/// presence marker; otherwise every piece percent-decodes into a Text item.
fn split_composite(
    raw: &str,
    joiner: &str,
    plus: bool,
) -> Result<Vec<ParamValue>, ParamDecodeError> {
    let pieces = split_pieces(raw, joiner);
    if pieces.len() == 1 && pieces[0].is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::with_capacity(pieces.len());
    for piece in pieces {
        items.push(ParamValue::Text(percent_decode(piece, plus)?));
    }
    Ok(items)
}

/// Builds an Object from bare prop/val wire pairs, last occurrence winning.
fn object_from_pairs<'a>(
    pairs: impl Iterator<Item = (&'a str, &'a str)>,
    plus: bool,
) -> Result<ParamValue, ParamDecodeError> {
    let mut entries: Vec<(String, ParamValue)> = Vec::new();
    for (key, value) in pairs {
        let prop = percent_decode(key, plus)?;
        let decoded = ParamValue::Text(percent_decode(value, plus)?);
        upsert_entry(&mut entries, prop, decoded);
    }
    Ok(ParamValue::Object(entries))
}

/// Alternating-piece object construction shared by non-exploded header and
/// simple/label path bodies. `decode` applies per medium (percent-decoding
/// on paths, verbatim on headers).
fn alternating_object<F>(pieces: &[&str], decode: F) -> Result<ParamValue, ParamDecodeError>
where
    F: Fn(&str) -> Result<String, ParamDecodeError>,
{
    if pieces.len() % 2 != 0 {
        return Err(ParamDecodeError::Malformed(
            "odd number of elements for a non-exploded object",
        ));
    }
    let mut entries = Vec::with_capacity(pieces.len() / 2);
    for chunk in pieces.chunks(2) {
        let prop = decode(chunk[0])?;
        let value = decode(chunk[1])?;
        upsert_entry(&mut entries, prop, ParamValue::Text(value));
    }
    Ok(ParamValue::Object(entries))
}

/// Header non-exploded objects are flat prop,val alternations with
/// verbatim (never percent-decoded) text.
fn composite_object(pieces: Vec<String>) -> Result<ParamValue, ParamDecodeError> {
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    alternating_object(&refs, |piece| Ok(piece.to_owned()))
}

/// Strips the style's structural prefix from a raw path segment.
///
/// Returns `Ok(None)` for empty input (missing parameter). Label segments
/// must start with `.` and matrix segments with `;` — anything else is
/// malformed rather than silently reinterpreted (companion §6 grammar).
fn strip_path_prefix<'a>(
    spec: &ParamSpec,
    raw: &'a str,
) -> Result<Option<&'a str>, ParamDecodeError> {
    if raw.is_empty() {
        return Ok(None);
    }
    match spec.style {
        ParamStyle::Label => raw
            .strip_prefix('.')
            .map(Some)
            .ok_or(ParamDecodeError::Malformed(
                "label parameter segment is missing its leading '.'",
            )),
        ParamStyle::Matrix => raw
            .strip_prefix(';')
            .map(Some)
            .ok_or(ParamDecodeError::Malformed(
                "matrix parameter segment is missing its leading ';'",
            )),
        _ => Ok(Some(raw)),
    }
}

/// Splits a matrix body into its `;`-separated parts, dropping the empty
/// segment produced by the leading `;`.
fn matrix_parts(body: &str) -> Result<Vec<&str>, ParamDecodeError> {
    let parts: Vec<&str> = body.split(';').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return Err(ParamDecodeError::Malformed(
            "matrix parameter has no ;name=value part",
        ));
    }
    Ok(parts)
}

/// Splits one matrix part into (raw key, raw value).
fn matrix_key_value(part: &str) -> Result<(&str, &str), ParamDecodeError> {
    part.split_once('=')
        .ok_or(ParamDecodeError::Malformed("matrix part is missing '='"))
}

/// Matrix part key comparison: keys were percent-escaped on encode, so the
/// raw key decodes before comparing against the spec name.
fn matrix_part_is_named(part: &str, name: &str) -> Result<bool, ParamDecodeError> {
    let (key_raw, _) = matrix_key_value(part)?;
    Ok(percent_decode(key_raw, false)? == name)
}

/// Shape inference over multiple matrix parts (used by
/// [`decode_path_segment`]): parts all carrying the parameter name are an
/// exploded array; any other `k=v` population is an exploded object.
fn infer_matrix_shape(spec: &ParamSpec, parts: &[&str]) -> Result<ParamShape, ParamDecodeError> {
    let mut all_named = true;
    for part in parts {
        if !matrix_part_is_named(part, &spec.name)? {
            all_named = false;
            break;
        }
    }
    Ok(if all_named {
        ParamShape::Array
    } else {
        ParamShape::Object
    })
}

/// Matrix body decode against a known shape (companion §6):
///
/// * Scalar — exactly one part `;name=value`;
/// * Array — exploded parts repeat `;name=item`; non-exploded is a single
///   comma-joined `;name=a,b,c` (single empty value = empty array);
/// * Object — exploded parts are bare `;prop=val` entries; non-exploded is
///   one `;name=prop,val,...` alternation.
fn decode_matrix_body(
    spec: &ParamSpec,
    body: &str,
    shape: ParamShape,
) -> Result<ParamValue, ParamDecodeError> {
    let parts = matrix_parts(body)?;
    match shape {
        ParamShape::Scalar => {
            if parts.len() != 1 {
                return Err(ParamDecodeError::Malformed(
                    "multiple matrix parts for a scalar parameter",
                ));
            }
            let (key_raw, value_raw) = matrix_key_value(parts[0])?;
            if percent_decode(key_raw, false)? != spec.name {
                return Err(ParamDecodeError::Malformed(
                    "matrix part carries a different parameter name",
                ));
            }
            Ok(ParamValue::Text(percent_decode(value_raw, false)?))
        }
        ParamShape::Array => {
            let mut items = Vec::with_capacity(parts.len());
            if spec.explode {
                for part in parts {
                    let (key_raw, value_raw) = matrix_key_value(part)?;
                    if percent_decode(key_raw, false)? != spec.name {
                        return Err(ParamDecodeError::Malformed(
                            "exploded matrix array part carries a different name",
                        ));
                    }
                    items.push(ParamValue::Text(percent_decode(value_raw, false)?));
                }
            } else {
                if parts.len() != 1 {
                    return Err(ParamDecodeError::Malformed(
                        "non-exploded matrix array must be one ;name=value part",
                    ));
                }
                let (key_raw, value_raw) = matrix_key_value(parts[0])?;
                if percent_decode(key_raw, false)? != spec.name {
                    return Err(ParamDecodeError::Malformed(
                        "matrix part carries a different parameter name",
                    ));
                }
                items = split_composite(value_raw, ",", false)?;
            }
            Ok(ParamValue::Array(items))
        }
        ParamShape::Object => {
            if spec.explode {
                // Bare prop=val parts; the parameter name never appears.
                let mut entries = Vec::with_capacity(parts.len());
                for part in parts {
                    let (prop_raw, value_raw) = matrix_key_value(part)?;
                    let prop = percent_decode(prop_raw, false)?;
                    let value = percent_decode(value_raw, false)?;
                    upsert_entry(&mut entries, prop, ParamValue::Text(value));
                }
                Ok(ParamValue::Object(entries))
            } else {
                if parts.len() != 1 {
                    return Err(ParamDecodeError::Malformed(
                        "non-exploded matrix object must be one ;name=value part",
                    ));
                }
                let (key_raw, value_raw) = matrix_key_value(parts[0])?;
                if percent_decode(key_raw, false)? != spec.name {
                    return Err(ParamDecodeError::Malformed(
                        "matrix part carries a different parameter name",
                    ));
                }
                let pieces = split_pieces(value_raw, ",");
                if pieces.len() == 1 && pieces[0].is_empty() {
                    return Ok(ParamValue::Object(Vec::new()));
                }
                alternating_object(&pieces, |piece| percent_decode(piece, false))
            }
        }
    }
}

/// Simple/label body decode against a known shape. Structural splitting
/// happens before percent-decoding so atomic pieces restore verbatim.
fn decode_simple_body(
    spec: &ParamSpec,
    body: &str,
    joiner: &str,
    shape: ParamShape,
) -> Result<ParamValue, ParamDecodeError> {
    match shape {
        ParamShape::Scalar => Ok(ParamValue::Text(percent_decode(body, false)?)),
        ParamShape::Array => Ok(ParamValue::Array(split_composite(body, joiner, false)?)),
        ParamShape::Object => {
            let pieces = split_pieces(body, joiner);
            if pieces.len() == 1 && pieces[0].is_empty() {
                return Ok(ParamValue::Object(Vec::new()));
            }
            if spec.explode {
                let mut entries = Vec::with_capacity(pieces.len());
                for piece in pieces {
                    let (prop_raw, value_raw) = piece.split_once('=').ok_or(
                        ParamDecodeError::Malformed("exploded composite entry is missing '='"),
                    )?;
                    let prop = percent_decode(prop_raw, false)?;
                    let value = percent_decode(value_raw, false)?;
                    upsert_entry(&mut entries, prop, ParamValue::Text(value));
                }
                Ok(ParamValue::Object(entries))
            } else {
                alternating_object(&pieces, |piece| percent_decode(piece, false))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, style: ParamStyle, explode: bool, allow_reserved: bool) -> ParamSpec {
        ParamSpec::new(name, style, explode, allow_reserved)
    }

    fn text(value: &str) -> ParamValue {
        ParamValue::Text(value.to_owned())
    }

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn array(items: &[&str]) -> ParamValue {
        ParamValue::Array(items.iter().map(|item| text(item)).collect())
    }

    fn role_object() -> ParamValue {
        ParamValue::Object(vec![
            ("role".to_owned(), text("admin")),
            ("firstName".to_owned(), text("Alex")),
        ])
    }

    #[test]
    fn to_text_renders_scalars_and_flattens_composites() {
        assert_eq!(text("hi").to_text(), "hi");
        assert_eq!(ParamValue::Int(-42).to_text(), "-42");
        assert_eq!(ParamValue::Float(1.5).to_text(), "1.5");
        // Standard Display: integral floats print without a fraction.
        assert_eq!(ParamValue::Float(1.0).to_text(), "1");
        assert_eq!(ParamValue::Bool(true).to_text(), "true");
        assert_eq!(array(&["a", "b"]).to_text(), "a,b");
        assert_eq!(role_object().to_text(), "role,admin,firstName,Alex");
        // Composite leaves flatten through to_text (module docs).
        let nested = ParamValue::Array(vec![array(&["x"]), ParamValue::Int(3)]);
        assert_eq!(nested.to_text(), "x,3");
    }

    #[test]
    fn query_form_matches_oas_style_table() {
        let color = |explode| spec("color", ParamStyle::Form, explode, false);

        assert_eq!(
            encode_query_pairs(&color(true), &text("blue")).expect("scalar"),
            owned(&[("color", "blue")])
        );
        assert_eq!(
            encode_query_pairs(&color(true), &array(&["blue", "black", "brown"])).expect("array"),
            owned(&[("color", "blue"), ("color", "black"), ("color", "brown")])
        );
        // Exploded object emits bare prop=val pairs (OAS form+explode).
        assert_eq!(
            encode_query_pairs(&color(true), &role_object()).expect("object"),
            owned(&[("role", "admin"), ("firstName", "Alex")])
        );

        assert_eq!(
            encode_query_pairs(&color(false), &text("blue")).expect("scalar"),
            owned(&[("color", "blue")])
        );
        assert_eq!(
            encode_query_pairs(&color(false), &array(&["blue", "black", "brown"])).expect("array"),
            owned(&[("color", "blue,black,brown")])
        );
        assert_eq!(
            encode_query_pairs(&color(false), &role_object()).expect("object"),
            owned(&[("color", "role,admin,firstName,Alex")])
        );

        // WHATWG form rules: space is `+`, reserved bytes are escaped.
        let pairs = encode_query_pairs(&color(false), &text("a b&c")).expect("escaped");
        assert_eq!(pairs, owned(&[("color", "a+b%26c")]));
    }

    #[test]
    fn query_space_delimited_encodes_spaces_as_percent20_not_plus() {
        let space = |explode| spec("color", ParamStyle::SpaceDelimited, explode, false);

        // Strict RFC 3986 escaping for this style: %20, never `+`.
        assert_eq!(
            encode_query_pairs(&space(false), &array(&["blue", "black", "brown"])).expect("joined"),
            owned(&[("color", "blue%20black%20brown")])
        );
        assert_eq!(
            encode_query_pairs(&space(false), &text("a b")).expect("space value"),
            owned(&[("color", "a%20b")])
        );
        // Exploded delimited behaves like repeated form pairs.
        assert_eq!(
            encode_query_pairs(&space(true), &array(&["blue", "black"])).expect("exploded"),
            owned(&[("color", "blue"), ("color", "black")])
        );
        // Object shapes mirror the form conventions.
        assert_eq!(
            encode_query_pairs(&space(false), &role_object()).expect("object"),
            owned(&[("color", "role%20admin%20firstName%20Alex")])
        );
    }

    #[test]
    fn query_pipe_delimited_non_exploded_joins_with_pipe() {
        let pipe = spec("color", ParamStyle::PipeDelimited, false, false);
        assert_eq!(
            encode_query_pairs(&pipe, &array(&["blue", "black", "brown"])).expect("joined"),
            owned(&[("color", "blue|black|brown")])
        );
        // A literal pipe inside a value is escaped (strict policy).
        assert_eq!(
            encode_query_pairs(&pipe, &text("a|b")).expect("escaped"),
            owned(&[("color", "a%7Cb")])
        );
    }

    #[test]
    fn query_deep_object_emits_one_level_bracket_keys() {
        let deep = spec("color", ParamStyle::DeepObject, true, false);
        let rgb = ParamValue::Object(vec![
            ("R".to_owned(), ParamValue::Int(100)),
            ("G".to_owned(), ParamValue::Int(200)),
            ("B".to_owned(), ParamValue::Int(150)),
        ]);
        assert_eq!(
            encode_query_pairs(&deep, &rgb).expect("rgb"),
            owned(&[
                ("color[R]", "100"),
                ("color[G]", "200"),
                ("color[B]", "150")
            ])
        );
        // Values use strict escaping (space %20) even though the style is
        // not form; brackets around the property stay structural.
        let spaced = ParamValue::Object(vec![("a key".to_owned(), text("v w"))]);
        assert_eq!(
            encode_query_pairs(&deep, &spaced).expect("spaced"),
            owned(&[("color[a%20key]", "v%20w")])
        );
        // explode is irrelevant for deepObject (always property-per-pair).
        let no_explode = spec("color", ParamStyle::DeepObject, false, false);
        assert_eq!(
            encode_query_pairs(&no_explode, &rgb).expect("no explode flag"),
            encode_query_pairs(&deep, &rgb).expect("explode flag")
        );
    }

    #[test]
    fn encode_deep_object_rejects_non_objects_and_nested_values() {
        let deep = spec("color", ParamStyle::DeepObject, true, false);

        let error = encode_query_pairs(&deep, &text("blue")).expect_err("scalar is not an object");
        assert_eq!(
            error,
            ParamEncodeError::UnsupportedShape(
                "deepObject style requires an Object parameter value"
            )
        );

        let nested_array = ParamValue::Object(vec![("R".to_owned(), array(&["x"]))]);
        let error =
            encode_query_pairs(&deep, &nested_array).expect_err("array value below one level");
        assert_eq!(
            error,
            ParamEncodeError::UnsupportedShape("deepObject values must be scalars (one level)")
        );

        let nested_object = ParamValue::Object(vec![(
            "R".to_owned(),
            ParamValue::Object(vec![("deep".to_owned(), text("x"))]),
        )]);
        assert!(encode_query_pairs(&deep, &nested_object).is_err());
    }
}

#[cfg(test)]
mod path_header_cookie_tests {
    use super::*;

    fn text(value: &str) -> ParamValue {
        ParamValue::Text(value.to_owned())
    }

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn array(items: &[&str]) -> ParamValue {
        ParamValue::Array(items.iter().map(|item| text(item)).collect())
    }

    fn role_object() -> ParamValue {
        ParamValue::Object(vec![
            ("role".to_owned(), text("admin")),
            ("firstName".to_owned(), text("Alex")),
        ])
    }

    #[test]
    fn path_simple_label_matrix_match_oas_style_table() {
        let simple = |explode| ParamSpec::new("id", ParamStyle::Simple, explode, false);
        let label = |explode| ParamSpec::new("id", ParamStyle::Label, explode, false);
        let matrix = |explode| ParamSpec::new("color", ParamStyle::Matrix, explode, false);

        // simple: comma-joined; explode never changes arrays.
        assert_eq!(encode_path(&simple(false), &ParamValue::Int(5)), "5");
        assert_eq!(
            encode_path(&simple(true), &array(&["3", "4", "5"])),
            "3,4,5"
        );
        assert_eq!(
            encode_path(&simple(false), &role_object()),
            "role,admin,firstName,Alex"
        );
        assert_eq!(
            encode_path(&simple(true), &role_object()),
            "role=admin,firstName=Alex"
        );

        // label: leading dot; exploded composites use dot separators.
        assert_eq!(encode_path(&label(false), &ParamValue::Int(5)), ".5");
        assert_eq!(
            encode_path(&label(false), &array(&["3", "4", "5"])),
            ".3,4,5"
        );
        assert_eq!(
            encode_path(&label(false), &role_object()),
            ".role,admin,firstName,Alex"
        );
        assert_eq!(encode_path(&label(true), &ParamValue::Int(5)), ".5");
        assert_eq!(
            encode_path(&label(true), &array(&["3", "4", "5"])),
            ".3.4.5"
        );
        assert_eq!(
            encode_path(&label(true), &role_object()),
            ".role=admin.firstName=Alex"
        );

        // matrix: ;name=value; exploded arrays repeat the name, exploded
        // objects drop it.
        assert_eq!(encode_path(&matrix(false), &text("blue")), ";color=blue");
        assert_eq!(
            encode_path(&matrix(false), &array(&["3", "4", "5"])),
            ";color=3,4,5"
        );
        assert_eq!(
            encode_path(&matrix(false), &role_object()),
            ";color=role,admin,firstName,Alex"
        );
        assert_eq!(encode_path(&matrix(true), &text("blue")), ";color=blue");
        assert_eq!(
            encode_path(&matrix(true), &array(&["3", "4", "5"])),
            ";color=3;color=4;color=5"
        );
        assert_eq!(
            encode_path(&matrix(true), &role_object()),
            ";role=admin;firstName=Alex"
        );

        // Atomic pieces escape structural characters (RFC 3986 unreserved
        // set); delimiters stay raw only where the renderer places them.
        let dotted = ParamSpec::new("v", ParamStyle::Simple, false, false);
        assert_eq!(encode_path(&dotted, &text("a,b=c")), "a%2Cb%3Dc");
    }

    #[test]
    fn non_path_styles_fall_back_to_documented_shapes() {
        // Runtime-side fallback: non-path styles render with simple shapes.
        let form = ParamSpec::new("x", ParamStyle::Form, true, false);
        assert_eq!(encode_path(&form, &array(&["a", "b"])), "a,b");
        // Non-query styles in query render with form shapes.
        let simple = ParamSpec::new("x", ParamStyle::Simple, false, false);
        assert_eq!(
            crate::params::encode_query_pairs(&simple, &array(&["a", "b"]))
                .expect("fallback pairs"),
            owned(&[("x", "a,b")])
        );
    }

    #[test]
    fn header_simple_and_delimited_styles_render_verbatim() {
        let simple = |explode| ParamSpec::new("h", ParamStyle::Simple, explode, false);

        assert_eq!(
            encode_header_value(&simple(false), &ParamValue::Int(5)),
            "5"
        );
        assert_eq!(
            encode_header_value(&simple(false), &array(&["3", "4", "5"])),
            "3,4,5"
        );
        assert_eq!(
            encode_header_value(&simple(false), &array(&["3", "4", "5"])),
            encode_header_value(&simple(true), &array(&["3", "4", "5"])),
            "explode does not change header arrays (no repetition channel)"
        );
        assert_eq!(
            encode_header_value(&simple(false), &role_object()),
            "role,admin,firstName,Alex"
        );
        assert_eq!(
            encode_header_value(&simple(true), &role_object()),
            "role=admin,firstName=Alex"
        );

        let spaced = ParamSpec::new("h", ParamStyle::SpaceDelimited, false, false);
        assert_eq!(
            encode_header_value(&spaced, &array(&["3", "4", "5"])),
            "3 4 5"
        );
        let piped = ParamSpec::new("h", ParamStyle::PipeDelimited, false, false);
        assert_eq!(
            encode_header_value(&piped, &array(&["3", "4", "5"])),
            "3|4|5"
        );
    }

    #[test]
    fn cookie_values_form_encode_into_semicolon_joined_segments() {
        let session = |explode| ParamSpec::new("id", ParamStyle::Form, explode, false);

        assert_eq!(
            encode_cookie_value(&session(false), &ParamValue::Int(45)),
            "id=45"
        );
        assert_eq!(
            encode_cookie_value(&session(true), &array(&["3", "4", "5"])),
            "id=3; id=4; id=5"
        );
        assert_eq!(
            encode_cookie_value(&session(false), &array(&["3", "4", "5"])),
            "id=3,4,5"
        );
        assert_eq!(
            encode_cookie_value(&session(true), &role_object()),
            "role=admin; firstName=Alex"
        );
        assert_eq!(
            encode_cookie_value(&session(false), &role_object()),
            "id=role,admin,firstName,Alex"
        );
        // Form escaping keeps `;` and `,` out of the cookie grammar, so a
        // value containing them stays unambiguous after splitting.
        assert_eq!(
            encode_cookie_value(&session(false), &text("a;b,c d")),
            "id=a%3Bb%2Cc+d"
        );
        // spec.style is irrelevant: cookies always take form semantics.
        let weird = ParamSpec::new("id", ParamStyle::Matrix, true, true);
        assert_eq!(
            encode_cookie_value(&weird, &array(&["3", "4"])),
            encode_cookie_value(&session(true), &array(&["3", "4"]))
        );
    }

    #[test]
    fn cookie_header_helpers_build_and_leniently_parse() {
        let header = build_cookie_header(&[
            ("id".to_owned(), "45".to_owned()),
            ("session".to_owned(), "abc".to_owned()),
        ]);
        assert_eq!(header, "id=45; session=abc");

        assert_eq!(
            parse_cookie_header("id=45; session=abc"),
            [
                ("id".to_owned(), "45".to_owned()),
                ("session".to_owned(), "abc".to_owned())
            ]
        );
        // Lenient: whitespace tolerance, empty segments, `=`-less names.
        assert_eq!(
            parse_cookie_header("  a = 1 ;; flag"),
            [
                ("a".to_owned(), "1".to_owned()),
                ("flag".to_owned(), String::new())
            ]
        );
        assert_eq!(parse_cookie_header(""), Vec::new());
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn text(value: &str) -> ParamValue {
        ParamValue::Text(value.to_owned())
    }

    fn array(items: &[&str]) -> ParamValue {
        ParamValue::Array(items.iter().map(|item| text(item)).collect())
    }

    fn pairs_of<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Iterator<Item = (&'a str, &'a str)> + Clone {
        pairs.iter().copied()
    }

    fn assert_malformed(result: Result<Option<ParamValue>, ParamDecodeError>) {
        assert!(
            matches!(result, Err(ParamDecodeError::Malformed(_))),
            "{result:?}"
        );
    }

    #[test]
    fn decode_missing_input_returns_none() {
        let form = ParamSpec::new("color", ParamStyle::Form, true, false);
        // No pair carries the name and none is available for an exploded
        // object interpretation.
        assert_eq!(
            decode_query(&form, pairs_of(&[("other", "x")])).expect("absent"),
            None,
            "foreign-only pairs are treated as absent for name-carrying shapes"
        );
        let deep = ParamSpec::new("color", ParamStyle::DeepObject, true, false);
        assert_eq!(
            decode_query(&deep, pairs_of(&[("other[y]", "1")])).expect("foreign only"),
            None,
            "deepObject ignores foreign keys"
        );
        let header = ParamSpec::new("h", ParamStyle::Simple, false, false);
        assert_eq!(
            decode_header_value(&header, "").expect("empty header"),
            None
        );
        assert_eq!(
            decode_header_value(&header, "   ").expect("blank header"),
            None
        );
        for style in [ParamStyle::Simple, ParamStyle::Label, ParamStyle::Matrix] {
            let path = ParamSpec::new("p", style, false, false);
            assert_eq!(decode_path_segment(&path, "").expect("empty segment"), None);
        }
        // Empty exploded composites emit nothing on the wire, so they
        // decode as absent (documented vanish).
        let empty = encode_query_pairs(&form, &array(&[])).expect("exploded empty");
        assert!(empty.is_empty());
        let raw: Vec<(String, String)> = Vec::new();
        assert_eq!(
            decode_query_shaped(
                &form,
                raw.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                ParamShape::Array
            )
            .expect("no pairs"),
            None
        );
    }

    #[test]
    fn decode_malformed_inputs_report_errors() {
        // Structural prefixes are mandatory.
        let label = ParamSpec::new("p", ParamStyle::Label, false, false);
        assert_malformed(decode_path_segment(&label, "nodot"));
        let matrix = ParamSpec::new("p", ParamStyle::Matrix, false, false);
        assert_malformed(decode_path_segment(&matrix, "name=x"));
        assert_malformed(decode_path_segment(&matrix, ";"));

        // Odd alternations cannot be an object.
        let form = ParamSpec::new("q", ParamStyle::Form, false, false);
        assert_malformed(decode_query_shaped(
            &form,
            pairs_of(&[("q", "a,b,c")]),
            ParamShape::Object,
        ));

        // Bad percent escapes and non-UTF-8 bytes.
        assert_malformed(decode_query_shaped(
            &form,
            pairs_of(&[("q", "%ZZ")]),
            ParamShape::Scalar,
        ));
        assert_malformed(decode_query_shaped(
            &form,
            pairs_of(&[("q", "%FF")]),
            ParamShape::Scalar,
        ));
        assert_malformed(decode_query_shaped(
            &form,
            pairs_of(&[("q", "%2")]),
            ParamShape::Scalar,
        ));

        // deepObject grammar violations.
        let deep = ParamSpec::new("c", ParamStyle::DeepObject, true, false);
        assert_malformed(decode_query(&deep, pairs_of(&[("c[x", "1")])));
        assert_malformed(decode_query(&deep, pairs_of(&[("c[", "1")])));
        assert_malformed(decode_query(&deep, pairs_of(&[("c[][x]", "1")])));
        assert_malformed(decode_query(&deep, pairs_of(&[("c[a][b]", "1")])));
        assert_malformed(decode_query(&deep, pairs_of(&[("c[]", "1")])));

        // Repetition is the exploded-array encoding; seeing it under an
        // exploded scalar schema is a contract violation. Non-exploded
        // duplicates take the last value instead (WHATWG rule).
        let exploded = ParamSpec::new("q", ParamStyle::Form, true, false);
        assert_malformed(decode_query_shaped(
            &exploded,
            pairs_of(&[("q", "1"), ("q", "2")]),
            ParamShape::Scalar,
        ));
        assert_eq!(
            decode_query_shaped(
                &form,
                pairs_of(&[("q", "1"), ("q", "2")]),
                ParamShape::Scalar
            )
            .expect("last wins"),
            Some(text("2"))
        );

        // Matrix cardinality mismatches.
        assert_malformed(decode_path_segment_shaped(
            &matrix,
            ";p=1;p=2",
            ParamShape::Scalar,
        ));
        assert_malformed(decode_path_segment_shaped(
            &matrix,
            ";p=1;other=2",
            ParamShape::Array,
        ));

        // Exploded composite entries must carry '='.
        let label_exploded = ParamSpec::new("p", ParamStyle::Label, true, false);
        assert_malformed(decode_path_segment_shaped(
            &label_exploded,
            ".role=admin.broken",
            ParamShape::Object,
        ));
        let header = ParamSpec::new("h", ParamStyle::Simple, true, false);
        assert_malformed(decode_header_value_shaped(
            &header,
            "broken, k=v",
            ParamShape::Object,
        ));
    }

    #[test]
    fn allow_reserved_preserves_rfc3986_reserved_characters() {
        // Reserved set `:/?#[]@!$&'()*+,;=` stays literal; spaces still
        // escape (as %20 in the strict policy), controls/non-ASCII too.
        let reserved_on = ParamSpec::new("q", ParamStyle::Form, false, true);
        let value = text("b c/d?e=f&g,h:i");
        assert_eq!(
            encode_query_pairs(&reserved_on, &value).expect("allowReserved"),
            owned(&[("q", "b%20c/d?e=f&g,h:i")])
        );
        // Same value with full form encoding escapes every reserved byte.
        let reserved_off = ParamSpec::new("q", ParamStyle::Form, false, false);
        assert_eq!(
            encode_query_pairs(&reserved_off, &value).expect("form"),
            owned(&[("q", "b+c%2Fd%3Fe%3Df%26g%2Ch%3Ai")])
        );

        // Round trip through the strict policy: `+` is literal because the
        // encoder always escaped it.
        let plus_value = text("a+b c");
        let encoded = encode_query_pairs(&reserved_on, &plus_value).expect("plus value");
        assert_eq!(encoded, owned(&[("q", "a+b%20c")]));
        let decoded = decode_query_shaped(
            &reserved_on,
            encoded.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            ParamShape::Scalar,
        )
        .expect("round trip");
        assert_eq!(decoded, Some(plus_value));

        // Delimited styles honor allowReserved too (strict policy base).
        let pipe_reserved = ParamSpec::new("q", ParamStyle::PipeDelimited, false, true);
        assert_eq!(
            encode_query_pairs(&pipe_reserved, &text("a:b")).expect("pipe reserved"),
            owned(&[("q", "a:b")])
        );
    }

    #[test]
    fn allow_reserved_arrays_with_embedded_delimiters_are_lossy() {
        // Documented OAS caveat: with delimiters left literal, values that
        // contain the delimiter are ambiguous. The decoder splits on them.
        let spec = ParamSpec::new("q", ParamStyle::Form, false, true);
        let value = ParamValue::Array(vec![text("a,b"), text("c")]);
        let encoded = encode_query_pairs(&spec, &value).expect("encoded");
        assert_eq!(encoded, owned(&[("q", "a,b,c")]));
        let decoded = decode_query_shaped(
            &spec,
            encoded.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            ParamShape::Array,
        )
        .expect("decodable but lossy");
        assert_eq!(decoded, Some(array(&["a", "b", "c"])));
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;

    fn text(value: &str) -> ParamValue {
        ParamValue::Text(value.to_owned())
    }

    fn array(items: &[&str]) -> ParamValue {
        ParamValue::Array(items.iter().map(|item| text(item)).collect())
    }

    fn role_object() -> ParamValue {
        ParamValue::Object(vec![
            ("role".to_owned(), text("admin")),
            ("firstName".to_owned(), text("Alex")),
        ])
    }

    fn shape_of(value: &ParamValue) -> ParamShape {
        match value {
            ParamValue::Array(_) => ParamShape::Array,
            ParamValue::Object(_) => ParamShape::Object,
            _ => ParamShape::Scalar,
        }
    }

    /// The wire carries no type information, so decoded scalars come back
    /// as `Text`; compare against this canonical form (module docs).
    fn canonical(value: &ParamValue) -> ParamValue {
        match value {
            ParamValue::Array(items) => ParamValue::Array(items.iter().map(canonical).collect()),
            ParamValue::Object(entries) => ParamValue::Object(
                entries
                    .iter()
                    .map(|(key, item)| (key.clone(), canonical(item)))
                    .collect(),
            ),
            scalar => ParamValue::Text(scalar.to_text()),
        }
    }

    /// Values chosen to avoid the documented wire ambiguities (no
    /// delimiters inside values, no single-item arrays).
    fn matrix_samples() -> Vec<ParamValue> {
        vec![
            text("blue"),
            ParamValue::Int(-42),
            ParamValue::Bool(true),
            ParamValue::Float(1.5),
            array(&["blue", "black", "brown"]),
            role_object(),
        ]
    }

    fn assert_query_round_trip(spec: &ParamSpec, value: &ParamValue) {
        let encoded = encode_query_pairs(spec, value)
            .unwrap_or_else(|error| panic!("query encodable {spec:?} {value:?}: {error}"));
        let decoded = decode_query_shaped(
            spec,
            encoded.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            shape_of(value),
        )
        .unwrap_or_else(|error| panic!("query decodable {spec:?}: {error}"));
        assert_eq!(
            decoded.as_ref(),
            Some(&canonical(value)),
            "query round trip {spec:?}"
        );
    }

    fn assert_path_round_trip(spec: &ParamSpec, value: &ParamValue) {
        let encoded = encode_path(spec, value);
        let decoded = decode_path_segment_shaped(spec, &encoded, shape_of(value))
            .unwrap_or_else(|error| panic!("path decodable {encoded:?}: {error}"));
        assert_eq!(
            decoded.as_ref(),
            Some(&canonical(value)),
            "path round trip {spec:?}"
        );
    }

    fn assert_header_round_trip(spec: &ParamSpec, value: &ParamValue) {
        let encoded = encode_header_value(spec, value);
        let decoded = decode_header_value_shaped(spec, &encoded, shape_of(value))
            .unwrap_or_else(|error| panic!("header decodable {encoded:?}: {error}"));
        assert_eq!(
            decoded.as_ref(),
            Some(&canonical(value)),
            "header round trip {spec:?}"
        );
    }

    #[test]
    fn query_round_trip_matrix_across_styles_explode_and_shapes() {
        for style in [
            ParamStyle::Form,
            ParamStyle::SpaceDelimited,
            ParamStyle::PipeDelimited,
        ] {
            for explode in [false, true] {
                for allow_reserved in [false, true] {
                    let spec = ParamSpec::new("color", style, explode, allow_reserved);
                    for value in matrix_samples() {
                        assert_query_round_trip(&spec, &value);
                    }
                }
            }
        }

        // deepObject round trips objects only (companion §6).
        for allow_reserved in [false, true] {
            let spec = ParamSpec::new("color", ParamStyle::DeepObject, false, allow_reserved);
            assert_query_round_trip(&spec, &role_object());
        }

        // Non-exploded empty composites keep a presence marker and invert.
        for style in [ParamStyle::Form, ParamStyle::SpaceDelimited] {
            let spec = ParamSpec::new("color", style, false, false);
            assert_query_round_trip(&spec, &array(&[]));
            assert_query_round_trip(&spec, &ParamValue::Object(Vec::new()));
        }
    }

    #[test]
    fn path_round_trip_matrix_across_styles_explode_and_shapes() {
        for style in [ParamStyle::Simple, ParamStyle::Label, ParamStyle::Matrix] {
            for explode in [false, true] {
                let spec = ParamSpec::new("color", style, explode, false);
                for value in matrix_samples() {
                    assert_path_round_trip(&spec, &value);
                }
                // Non-path styles fall back to simple shapes; decode mirrors.
                let form_in_path = ParamSpec::new("color", ParamStyle::Form, explode, false);
                assert_path_round_trip(&form_in_path, &array(&["a", "b"]));
            }
        }
    }

    #[test]
    fn header_round_trip_matrix_across_styles_and_shapes() {
        for style in [
            ParamStyle::Simple,
            ParamStyle::SpaceDelimited,
            ParamStyle::PipeDelimited,
        ] {
            for explode in [false, true] {
                let spec = ParamSpec::new("h", style, explode, false);
                // Header values are verbatim: samples must not contain the
                // style's delimiter (documented ambiguity).
                for value in matrix_samples() {
                    assert_header_round_trip(&spec, &value);
                }
            }
        }
    }

    #[test]
    fn cookie_round_trip_through_header_helpers() {
        for explode in [false, true] {
            for value in matrix_samples() {
                let spec = ParamSpec::new("id", ParamStyle::Form, explode, false);
                let header_value = encode_cookie_value(&spec, &value);
                let pairs = parse_cookie_header(&header_value);
                let raw: Vec<(&str, &str)> = pairs
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                let decoded = decode_query_shaped(
                    // Cookie decoding recipe: form spec, allowReserved off.
                    &spec,
                    raw.iter().copied(),
                    shape_of(&value),
                )
                .expect("cookie decodable");
                assert_eq!(
                    decoded.as_ref(),
                    Some(&canonical(&value)),
                    "cookie round trip {value:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod inference_tests {
    use super::*;

    fn text(value: &str) -> ParamValue {
        ParamValue::Text(value.to_owned())
    }

    fn array(items: &[&str]) -> ParamValue {
        ParamValue::Array(items.iter().map(|item| text(item)).collect())
    }

    fn pairs_of<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Iterator<Item = (&'a str, &'a str)> + Clone {
        pairs.iter().copied()
    }

    #[test]
    fn decode_query_infers_shapes_from_wire() {
        let form = ParamSpec::new("color", ParamStyle::Form, true, false);
        // Repeated pairs read as an array.
        assert_eq!(
            decode_query(&form, pairs_of(&[("color", "blue"), ("color", "black")]))
                .expect("repeated"),
            Some(array(&["blue", "black"]))
        );
        // A single pair reads as a scalar (one-item arrays conflate with
        // scalars; use the shaped API when the schema knows).
        assert_eq!(
            decode_query(&form, pairs_of(&[("color", "blue")])).expect("single"),
            Some(text("blue"))
        );
        // The exploded-object wire shape is nameless, so the generic entry
        // point reports absence instead of guessing from foreign pairs.
        assert_eq!(
            decode_query(&form, pairs_of(&[("role", "admin"), ("firstName", "Alex")]))
                .expect("nameless shape is absent"),
            None
        );
        // Shaped decoding over this parameter's pre-filtered pairs
        // reconstructs the object (module docs, "Shape fidelity").
        assert_eq!(
            decode_query_shaped(
                &form,
                pairs_of(&[("role", "admin"), ("firstName", "Alex")]),
                ParamShape::Object
            )
            .expect("shaped exploded object"),
            Some(ParamValue::Object(vec![
                ("role".to_owned(), text("admin")),
                ("firstName".to_owned(), text("Alex"))
            ]))
        );

        let plain = ParamSpec::new("color", ParamStyle::Form, false, false);
        // Comma-bearing values infer arrays.
        assert_eq!(
            decode_query(&plain, pairs_of(&[("color", "1,2,3")])).expect("csv"),
            Some(array(&["1", "2", "3"]))
        );
        // Non-exploded objects are wire-indistinguishable from even-length
        // arrays: inference yields the array, shaped decoding is required.
        assert_eq!(
            decode_query(&plain, pairs_of(&[("color", "role,admin")])).expect("ambiguous"),
            Some(array(&["role", "admin"]))
        );
        assert_eq!(
            decode_query_shaped(
                &plain,
                pairs_of(&[("color", "role,admin")]),
                ParamShape::Object
            )
            .expect("shaped object"),
            Some(ParamValue::Object(vec![("role".to_owned(), text("admin"))]))
        );

        // deepObject self-describes as an object.
        let deep = ParamSpec::new("color", ParamStyle::DeepObject, true, false);
        assert_eq!(
            decode_query(
                &deep,
                pairs_of(&[("other", "z"), ("color[R]", "100"), ("color[G]", "200")])
            )
            .expect("deepObject"),
            Some(ParamValue::Object(vec![
                ("R".to_owned(), text("100")),
                ("G".to_owned(), text("200"))
            ]))
        );
    }

    #[test]
    fn decode_path_infers_structures_from_prefixes() {
        let label = ParamSpec::new("id", ParamStyle::Label, true, false);
        assert_eq!(
            decode_path_segment(&label, ".3.4.5").expect("label array"),
            Some(array(&["3", "4", "5"]))
        );
        assert_eq!(
            decode_path_segment(&label, ".5").expect("label scalar"),
            Some(text("5"))
        );

        let matrix = ParamSpec::new("color", ParamStyle::Matrix, true, false);
        assert_eq!(
            decode_path_segment(&matrix, ";color=3;color=4").expect("matrix array"),
            Some(array(&["3", "4"]))
        );
        assert_eq!(
            decode_path_segment(&matrix, ";role=admin;firstName=Alex").expect("matrix object"),
            Some(ParamValue::Object(vec![
                ("role".to_owned(), text("admin")),
                ("firstName".to_owned(), text("Alex"))
            ]))
        );
        let matrix_plain = ParamSpec::new("color", ParamStyle::Matrix, false, false);
        assert_eq!(
            decode_path_segment(&matrix_plain, ";color=3,4").expect("matrix non-exploded array"),
            Some(array(&["3", "4"]))
        );
        // Wrong name in a single part still decodes as that scalar text —
        // name checking belongs to shaped decoding.
        assert_eq!(
            decode_path_segment_shaped(&matrix, ";size=7", ParamShape::Scalar)
                .expect_err("name mismatch"),
            ParamDecodeError::Malformed("matrix part carries a different parameter name")
        );

        let simple = ParamSpec::new("id", ParamStyle::Simple, false, false);
        assert_eq!(
            decode_path_segment(&simple, "3,4,5").expect("simple array"),
            Some(array(&["3", "4", "5"]))
        );
        assert_eq!(
            decode_path_segment(&simple, "plain").expect("simple scalar"),
            Some(text("plain"))
        );
        // Percent-encoded pieces decode after structural splitting.
        assert_eq!(
            decode_path_segment(&simple, "a%2Cb,c").expect("escaped piece"),
            Some(array(&["a,b", "c"]))
        );
    }

    #[test]
    fn decode_header_infers_delimited_shapes() {
        let simple = ParamSpec::new("h", ParamStyle::Simple, false, false);
        assert_eq!(
            decode_header_value(&simple, "3, 4, 5").expect("header array"),
            Some(array(&["3", "4", "5"]))
        );
        assert_eq!(
            decode_header_value(&simple, "5").expect("header scalar"),
            Some(text("5"))
        );
        // Exploded entries carrying '=' infer an object.
        let exploded = ParamSpec::new("h", ParamStyle::Simple, true, false);
        assert_eq!(
            decode_header_value(&exploded, "role=admin, firstName=Alex").expect("header object"),
            Some(ParamValue::Object(vec![
                ("role".to_owned(), text("admin")),
                ("firstName".to_owned(), text("Alex"))
            ]))
        );
        let pipe = ParamSpec::new("h", ParamStyle::PipeDelimited, false, false);
        assert_eq!(
            decode_header_value(&pipe, "3|4|5").expect("pipe header"),
            Some(array(&["3", "4", "5"]))
        );
    }
}
