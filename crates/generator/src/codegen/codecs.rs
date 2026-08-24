//! Optional media-codec plugin families (main spec §5.9, §45;
//! DECISIONS.md D-impl-codec-plugins, D-impl-override-precedence).
//!
//! Typed codecs are OPT-IN: without a configured codec, formats such as XML,
//! CBOR, or MessagePack fall back to raw streaming bodies (§5.9) and are never
//! guessed into an eager representation. When a plugin claims an entry during
//! planning (§45), BOTH directions become typed:
//!
//! - decode runs AFTER a bounded collection under the purpose-specific
//!   structured limit (`structured_response_bytes` / `error_response_bytes`
//!   client-side, `structured_request_bytes` server-side), so memory stays
//!   bounded by construction (D-impl-codec-plugins);
//! - encoding serializes through the fail-fast
//!   [`::openapi_support::encode::CountingWriter`] via
//!   [`serialize_with_writer_limited`], mirroring the §34 JSON invariants.
//!
//! Built-in plugins serialize through serde-compatible codecs, so claimed
//! entries reuse the SHARED models.rs types and their Serde derives verbatim
//! (no per-codec model duplication). Runtime crates are dependencies of
//! EMITTED manifests only for enabled codecs — `openapi-support` stays
//! dependency-light (companion §4.5); emitted-manifest generation consumes
//! [`MediaCodecPlugin::manifest_dependency`] through
//! [`manifest_dependency_for`] (main spec §3.1,
//! [`super::manifest::generate_manifest`]).
//!
//! # Error-mapping contract (documented deviation)
//!
//! The JSON emitter distinguishes serde syntax errors from data errors
//! (MalformedBody 400 vs SchemaViolation 422). That distinction is NOT
//! portable across third-party codecs: quick-xml/ciborium/rmp-serde collapse
//! missing-required and shape errors into one opaque error type. Codec decode
//! failures therefore map onto `MalformedBody` 400 SERVER-side wholesale —
//! including missing-required-style errors we cannot detect portably — while
//! CLIENT-side every failure surfaces as `ClientError::Decode { content_type,
//! source }`. Companion §9 validators still run on successfully decoded
//! values, exactly as for the JSON family. This leniency is a recorded
//! deviation from §39 row 6 for codec families.
//!
//! # Emitter composition
//!
//! The four expression/statement methods return SELF-CONTAINED fragments:
//!
//! - `client_decode_expr(bytes_expr, model)` evaluates to
//!   `Result<{model}, ClientError>` and may bind nothing; it references the
//!   ambient bindings named by `bytes_expr`, plus `content_type: Option<mime>`.
//! - `server_decode_expr(bytes_expr, model)` evaluates to
//!   `Result<{model}, ProtocolRejection>` referencing the generated
//!   `malformed_body` helper.
//! - `*_encode_stmts(value_expr, limit_expr, out_var)` are statements whose
//!   LAST line binds `out_var` to
//!   `Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>`;
//!   overflow POLICY stays emitter-owned because it differs per side
//!   (§34.2 pre-send client error vs §34.1 hook + fixed 500 fallback).
//!
//! Emitters compose these fragments ONCE PER CODEC into module-level helpers
//! (`<id>_encode_limited` / `<id>_decode_*`) so call sites reuse the exact
//! bounded-encode/decode layout machinery of the JSON family and stay
//! rustfmt-canonical.

/// One planned codec claim over a media-type entry: carries everything the
/// emitters need besides the plugin behavior itself. `model_path` follows the
/// SAME models.rs resolution as the JSON family (shared Serde derives),
/// directionally cut to `<M>Write`/`<M>Read` when the resolved component
/// carries directional views (companion §5).
#[derive(Debug, Clone)]
pub struct CodecBinding {
    /// Id of the claiming plugin ([`MediaCodecPlugin::id`]).
    pub plugin_id: &'static str,
    /// Rust type path into `super::models` (or `super::views` when
    /// [`Self::model_from_views`] is set), resolved like JsonFamily.
    pub model_path: String,
    /// Fully qualified runtime crate path referenced by emitted code
    /// (e.g. `"::quick_xml"`).
    pub runtime_crate: &'static str,
    /// Human-readable note about required crate features, surfaced on the
    /// binding for manifest/doc generation.
    pub feature_note: &'static str,
    /// True when [`Self::model_path`] names a `super::views` type.
    pub model_from_views: bool,
}

/// Compile-time codec plugin consulted during planning (§45).
///
/// `handles` conventions per built-in plugin (exact literals plus structured
/// suffixes, all matched case-insensitively against the parameter-stripped
/// base literal):
///
/// - **xml**: `application/xml`, `text/xml`, any `+xml` suffix.
/// - **cbor**: `application/cbor`.
/// - **msgpack**: `application/msgpack`, `application/x-msgpack`, any
///   `+msgpack` suffix.
pub trait MediaCodecPlugin {
    /// Stable registry id (also the generated helper prefix).
    fn id(&self) -> &'static str;
    /// True when this plugin claims the given base media-type literal.
    fn handles(&self, media_type_literal: &str) -> bool;
    /// Fully qualified runtime crate path of this codec.
    fn runtime_crate(&self) -> &'static str;
    /// Required feature/crate note carried on [`CodecBinding`].
    fn feature_note(&self) -> &'static str;
    /// `[workspace.dependencies]`-style dependency fragment for an EMITTED
    /// manifest (caret requirement, no path deps; main spec §3.1). Consumed by
    /// future generated-crate emission; today it rides on the plan metadata.
    fn manifest_dependency(&self) -> &'static str;

    /// Client-side response decode fragment: expression of type
    /// `Result<{model}, ClientError>` mapping ANY codec error onto
    /// `ClientError::Decode`.
    fn client_decode_expr(&self, bytes_expr: &str, model: &str) -> String;
    /// Client-side request encode statements binding `out_var` to
    /// `Result<Bytes, EncodeTooLarge>`; overflow handling stays at the call
    /// site (§34.2 pre-send error).
    fn client_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String;
    /// Server-side request decode fragment: expression of type
    /// `Result<{model}, ProtocolRejection>` mapping ANY codec error onto
    /// MalformedBody 400 (see the module-level error-mapping contract).
    fn server_decode_expr(&self, bytes_expr: &str, model: &str) -> String;
    /// Server-side response encode statements binding `out_var` to
    /// `Result<Bytes, EncodeTooLarge>`; overflow handling stays with the
    /// caller (§34.1 hook + fixed empty 500).
    fn server_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String;
    /// Full `use` lines the generated file needs whenever this plugin's
    /// helpers are present.
    fn emitted_use_lines(&self) -> Vec<String>;
}

// ----------------------------------------------------------------------
// Shared fragment builders (kept byte-stable across plugins)
// ----------------------------------------------------------------------

/// Client decode fragment shared by every serde-compatible codec: ANY codec
/// error becomes `ClientError::Decode` carrying the negotiated content type.
/// `call` is the codec's bare decode fn name pulled in through
/// [`MediaCodecPlugin::emitted_use_lines`].
fn client_decode_via(call: &str, bytes_expr: &str) -> String {
    format!(
        "{call}({bytes}).map_err(|error| ClientError::Decode {{\n\
         \x20   content_type: content_type.clone(),\n\
         \x20   source: Box::new(error),\n\
         }})",
        bytes = bytes_expr
    )
}

/// Server decode fragment shared by every serde-compatible codec: ANY codec
/// error becomes MalformedBody 400 (module docs: documented deviation).
fn server_decode_via(call: &str, bytes_expr: &str) -> String {
    // Single line: at the helper-body indent the whole expression stays
    // within the rustfmt maximum width, and short chains must NOT be
    // pre-broken or rustfmt --check would rewrite them.
    format!(
        "{call}({bytes}).map_err(|_| malformed_body(\"malformed body\"))",
        bytes = bytes_expr
    )
}

/// Encode statements shared by every codec: serialize through the fail-fast
/// CountingWriter via [`::openapi_support::encode::
/// serialize_with_writer_limited`], binding `out_var` to the Result so the
/// emitter keeps §34.1/§34.2 policy ownership.
fn encode_stmts_via(write_body: &str, limit_expr: &str, out_var: &str) -> String {
    format!(
        "let {out} = ::openapi_support::encode::serialize_with_writer_limited({limit}, |writer| {{\n{body}\n}});",
        out = out_var,
        limit = limit_expr,
        body = write_body
    )
}

// ----------------------------------------------------------------------
// XML (quick-xml, `serialize` feature)
// ----------------------------------------------------------------------

/// XML codec over `quick_xml`: decodes with `de::from_reader`, encodes with
/// `se::Serializer` writing through the counting writer under a fixed `root`
/// element (decoding ignores the root name, so round trips stay symmetric).
struct XmlCodec;

impl MediaCodecPlugin for XmlCodec {
    fn id(&self) -> &'static str {
        "xml"
    }

    fn handles(&self, media_type_literal: &str) -> bool {
        let lowered = media_type_literal.to_ascii_lowercase();
        lowered == "application/xml" || lowered == "text/xml" || lowered.ends_with("+xml")
    }

    fn runtime_crate(&self) -> &'static str {
        "::quick_xml"
    }

    fn feature_note(&self) -> &'static str {
        "requires the `serialize` feature of quick-xml"
    }

    fn manifest_dependency(&self) -> &'static str {
        "quick-xml = { version = \"0.41\", features = [\"serialize\"] }"
    }

    fn client_decode_expr(&self, bytes_expr: &str, _model: &str) -> String {
        client_decode_via("from_reader", bytes_expr)
    }

    fn client_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String {
        encode_stmts_via(&xml_write_body(value_expr), limit_expr, out_var)
    }

    fn server_decode_expr(&self, bytes_expr: &str, _model: &str) -> String {
        server_decode_via("from_reader", bytes_expr)
    }

    fn server_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String {
        encode_stmts_via(&xml_write_body(value_expr), limit_expr, out_var)
    }

    fn emitted_use_lines(&self) -> Vec<String> {
        vec!["use ::quick_xml::de::from_reader;".to_owned()]
    }
}

/// quick-xml 0.41 serializer shape: `with_root` validates the tag name and
/// returns the serializer (consumed by value), and `Serialize::serialize`
/// reports its buffering verdict, which we discard.
fn xml_write_body(value_expr: &str) -> String {
    // Fragment lines arrive pre-indented one level past the enclosing
    // statement. The serializer `let` stays on ONE line because, once the
    // helper body indent is applied, it fits within the rustfmt maximum
    // width; the trailing `.map_err` chain breaks instead.
    let lines = [
        "    let mut sink = XmlFmtSink(writer);",
        "    let serializer = ::quick_xml::se::Serializer::with_root(&mut sink, Some(\"root\"))",
        "        .map_err(::std::io::Error::other)?;",
        "    ::serde::Serialize::serialize(VALUE, serializer)",
        "        .map(|_verdict| ())",
        "        .map_err(::std::io::Error::other)",
    ];
    lines.join("\n").replace("VALUE", value_expr)
}

// ----------------------------------------------------------------------
// CBOR (ciborium)
// ----------------------------------------------------------------------

/// CBOR codec over `ciborium`: `de::from_reader` / `ser::into_writer`.
struct CborCodec;

impl MediaCodecPlugin for CborCodec {
    fn id(&self) -> &'static str {
        "cbor"
    }

    fn handles(&self, media_type_literal: &str) -> bool {
        media_type_literal.eq_ignore_ascii_case("application/cbor")
    }

    fn runtime_crate(&self) -> &'static str {
        "::ciborium"
    }

    fn feature_note(&self) -> &'static str {
        "no non-default features required"
    }

    fn manifest_dependency(&self) -> &'static str {
        "ciborium = \"0.2\""
    }

    fn client_decode_expr(&self, bytes_expr: &str, _model: &str) -> String {
        client_decode_via("from_reader", bytes_expr)
    }

    fn client_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String {
        let write_body = format!(
            "    ::ciborium::ser::into_writer({value}, writer).map_err(::std::io::Error::other)",
            value = value_expr
        );
        encode_stmts_via(&write_body, limit_expr, out_var)
    }

    fn server_decode_expr(&self, bytes_expr: &str, _model: &str) -> String {
        server_decode_via("from_reader", bytes_expr)
    }

    fn server_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String {
        let write_body = format!(
            "    ::ciborium::ser::into_writer({value}, writer).map_err(::std::io::Error::other)",
            value = value_expr
        );
        encode_stmts_via(&write_body, limit_expr, out_var)
    }

    fn emitted_use_lines(&self) -> Vec<String> {
        vec!["use ::ciborium::de::from_reader;".to_owned()]
    }
}

// ----------------------------------------------------------------------
// MessagePack (rmp-serde)
// ----------------------------------------------------------------------

/// MessagePack codec over `rmp-serde`. Encoding NEVER uses `to_vec`, which
/// buffers unboundedly (main spec §49): items stream through the fail-fast
/// counting writer instead (`rmp_serde::encode::write`).
struct MessagePackCodec;

impl MediaCodecPlugin for MessagePackCodec {
    fn id(&self) -> &'static str {
        "msgpack"
    }

    fn handles(&self, media_type_literal: &str) -> bool {
        let lowered = media_type_literal.to_ascii_lowercase();
        lowered == "application/msgpack"
            || lowered == "application/x-msgpack"
            || lowered.ends_with("+msgpack")
    }

    fn runtime_crate(&self) -> &'static str {
        "::rmp_serde"
    }

    fn feature_note(&self) -> &'static str {
        "no non-default features required"
    }

    fn manifest_dependency(&self) -> &'static str {
        "rmp-serde = \"1.3\""
    }

    fn client_decode_expr(&self, bytes_expr: &str, _model: &str) -> String {
        client_decode_via("from_slice", bytes_expr)
    }

    fn client_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String {
        // `to_vec` buffers unboundedly (§49): write through the counter.
        let write_body = format!(
            "    ::rmp_serde::encode::write(writer, {value}).map_err(::std::io::Error::other)",
            value = value_expr
        );
        encode_stmts_via(&write_body, limit_expr, out_var)
    }

    fn server_decode_expr(&self, bytes_expr: &str, _model: &str) -> String {
        server_decode_via("from_slice", bytes_expr)
    }

    fn server_encode_stmts(&self, value_expr: &str, limit_expr: &str, out_var: &str) -> String {
        let write_body = format!(
            "    ::rmp_serde::encode::write(writer, {value}).map_err(::std::io::Error::other)",
            value = value_expr
        );
        encode_stmts_via(&write_body, limit_expr, out_var)
    }

    fn emitted_use_lines(&self) -> Vec<String> {
        vec!["use ::rmp_serde::from_slice;".to_owned()]
    }
}

/// Built-in v1 registry (declaration order = claim precedence): XML, CBOR,
/// MessagePack. Protobuf/Avro stay deferred (D-impl-codec-plugins: they need
/// external schema compilers this generator does not invoke). Plugins here are
/// consulted ONLY for ids enabled in the generator configuration.
pub fn default_registry() -> Vec<Box<dyn MediaCodecPlugin>> {
    vec![
        Box::new(XmlCodec),
        Box::new(CborCodec),
        Box::new(MessagePackCodec),
    ]
}

/// Manifest dependency fragment for one enabled plugin id, looked up through
/// the default registry (main spec §3.1 metadata carrier).
pub fn manifest_dependency_for(plugin_id: &str) -> Option<&'static str> {
    default_registry()
        .iter()
        .find(|plugin| plugin.id() == plugin_id)
        .map(|plugin| plugin.manifest_dependency())
}

/// Generated helper names derived from a plugin id: `<prefix>_encode_limited`
/// / `<prefix>_decode_typed` (client) and `<prefix>_decode_body` +
/// `<prefix>_encode_limited` (server). Kept in ONE place so emitters agree.
pub(crate) fn helper_prefix(plugin_id: &str) -> String {
    plugin_id.replace(['-', '.'], "_")
}
