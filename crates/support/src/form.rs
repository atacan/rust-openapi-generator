//! `application/x-www-form-urlencoded` decoding (main spec §16, §28.3, §34).
//!
//! The decode side of bounded form bodies: strict UTF-8 (D-impl-charset-
//! rejection), WHATWG-style splitting with `+`→space percent decoding, and a
//! serde deserializer over the ordered pairs for FLAT structs only (generated
//! form models are flat; nested composites are a defensive error). The size
//! gate runs before any allocation-heavy work so an oversized body is
//! rejected without buffering beyond [`decode_form_limited`]'s limit.
//!
//! Strictness decisions (crate philosophy, documented per §16):
//!
//! - an empty body parses as ZERO pairs (browser convention); whether that is
//!   acceptable is decided by the caller — required-body routers reject empty
//!   bodies as §28.3 MalformedBody BEFORE parsing;
//! - an empty segment (`a=1&&b=2`, trailing `&`) is [`FormDecodeError::
//!   Malformed`], never silently skipped;
//! - a segment without `=` yields an EMPTY value (browser convention);
//! - repeated keys collect into sequence fields when the visitor asks for a
//!   seq; a scalar field receiving more than one value is
//!   [`FormDecodeError::DuplicateField`];
//! - unknown keys are offered to the visitor like any other field, so serde's
//!   default ignores them and `deny_unknown_fields` rejects them; the
//!   flatten × deny combination is NOT supported for forms v1.
//!
//! Error taxonomy for generated routers (§39): syntax failures
//! ([`FormDecodeError::is_syntax`]) map to MalformedBody 400, data failures
//! (missing fields, wrong types, duplicates) map to SchemaViolation 422, and
//! [`FormDecodeError::TooLarge`] maps to BodyTooLarge 413.

/// Failure modes of the bounded form decode path (§16, §39 mapping).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormDecodeError {
    /// Syntactically malformed framing: an empty segment or an invalid or
    /// truncated percent escape.
    #[error("malformed application/x-www-form-urlencoded body")]
    Malformed,
    /// The raw body (or a percent-decoded key/value) is not valid UTF-8
    /// (D-impl-charset-rejection: never replacement characters).
    #[error("form body is not valid UTF-8")]
    NotUtf8,
    /// A scalar field received more than one value; sequences must be asked
    /// for explicitly (§16 flat forms).
    #[error("duplicate form field `{0}`")]
    DuplicateField(String),
    /// Well-formed pairs failed schema validation (missing required field,
    /// wrong type, unsupported composite shape). Serde's own messages are
    /// carried verbatim for diagnostics.
    #[error("form data failed schema validation: {0}")]
    Schema(String),
    /// The encoded body exceeded the configured limit; checked BEFORE any
    /// parsing work (413 via the caller's BodyTooLarge rejection).
    #[error("form body exceeds limit of {limit} bytes")]
    TooLarge {
        /// The configured limit in bytes.
        limit: usize,
    },
}

impl FormDecodeError {
    /// True for pre-deserialization syntax failures, which map to
    /// MalformedBody 400; everything else is data-level (SchemaViolation
    /// 422) or the size gate ([`Self::TooLarge`], 413).
    #[must_use]
    pub fn is_syntax(&self) -> bool {
        matches!(self, Self::Malformed | Self::NotUtf8)
    }
}

impl serde::de::Error for FormDecodeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self::Schema(msg.to_string())
    }

    fn duplicate_field(field: &'static str) -> Self {
        Self::DuplicateField(field.to_owned())
    }
}

/// Parses a raw `application/x-www-form-urlencoded` body into ordered pairs.
///
/// Strict UTF-8 first (D-impl-charset-rejection), then split on `&`; every
/// segment MUST contain at least one byte (empty segments are malformed) and
/// may omit `=`, which yields an empty value. Keys and values are
/// percent-decoded with `+` meaning space. An entirely empty body parses as
/// zero pairs (see module docs).
///
/// # Errors
///
/// [`FormDecodeError::NotUtf8`] for non-UTF-8 input (raw or decoded),
/// [`FormDecodeError::Malformed`] for framing/escape defects.
pub fn parse_form_bytes(raw: &[u8]) -> Result<Vec<(String, String)>, FormDecodeError> {
    let text = std::str::from_utf8(raw).map_err(|_| FormDecodeError::NotUtf8)?;
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut pairs = Vec::new();
    for segment in text.split('&') {
        // Strict framing: an empty segment (`a=1&&b=2`, trailing `&`) is a
        // defect, never silently skipped (§16).
        if segment.is_empty() {
            return Err(FormDecodeError::Malformed);
        }
        let (encoded_key, encoded_value) = match segment.split_once('=') {
            Some((key, value)) => (key, value),
            // Browser convention: a bare token carries an empty value.
            None => (segment, ""),
        };
        let key = decode_component(encoded_key)?;
        let value = decode_component(encoded_value)?;
        pairs.push((key, value));
    }
    Ok(pairs)
}

/// Percent-decodes one form component: `+` becomes a space, `%XY` (two hex
/// digits) becomes its byte, and every other byte passes through. The result
/// must be valid UTF-8.
fn decode_component(encoded: &str) -> Result<String, FormDecodeError> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while !rest.is_empty() {
        match rest[0] {
            b'+' => {
                bytes.push(b' ');
                rest = &rest[1..];
            }
            b'%' => {
                let hex = rest.get(1..3).ok_or(FormDecodeError::Malformed)?;
                let high = hex_digit(hex[0]).ok_or(FormDecodeError::Malformed)?;
                let low = hex_digit(hex[1]).ok_or(FormDecodeError::Malformed)?;
                bytes.push((high << 4) | low);
                rest = &rest[3..];
            }
            byte => {
                bytes.push(byte);
                rest = &rest[1..];
            }
        }
    }
    String::from_utf8(bytes).map_err(|_| FormDecodeError::NotUtf8)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decodes a bounded form body into `T` (main spec §16): the size gate runs
/// FIRST so an oversized body is rejected before any parsing or grouping
/// work, then [`parse_form_bytes`] plus [`deserialize_form_pairs`].
///
/// # Errors
///
/// [`FormDecodeError::TooLarge`] when `raw.len() > limit`; otherwise the
/// parse/deserialize errors of the inner steps.
pub fn decode_form_limited<T>(raw: &[u8], limit: usize) -> Result<T, FormDecodeError>
where
    T: serde::de::DeserializeOwned,
{
    if raw.len() > limit {
        return Err(FormDecodeError::TooLarge { limit });
    }
    let pairs = parse_form_bytes(raw)?;
    deserialize_form_pairs(&pairs)
}

/// Deserializes `T` from ordered form pairs (flat structs only).
///
/// Pairs are grouped by key preserving first-occurrence order; repeated keys
/// feed sequence fields, and a scalar field meeting more than one value is
/// [`FormDecodeError::DuplicateField`]. Missing required fields and type
/// mismatches surface as [`FormDecodeError::Schema`] through serde's own
/// reporting.
///
/// # Errors
///
/// Any [`FormDecodeError`] produced by the visitor; see the module docs for
/// the router mapping.
pub fn deserialize_form_pairs<T>(pairs: &[(String, String)]) -> Result<T, FormDecodeError>
where
    T: serde::de::DeserializeOwned,
{
    // Group by key in first-occurrence order; forms are bounded by the
    // caller's limit, so the linear rescan stays cheap and allocation-free
    // apart from the owned strings themselves.
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in pairs {
        if let Some((_, values)) = groups.iter_mut().find(|(name, _)| name == key) {
            values.push(value.clone());
        } else {
            groups.push((key.clone(), vec![value.clone()]));
        }
    }
    T::deserialize(FormPairs { groups })
}

/// Top-level deserializer over the grouped pairs: flat struct/map access
/// only; every other entry shape is a defensive schema error because
/// generated form models are flat (§16).
struct FormPairs {
    groups: Vec<(String, Vec<String>)>,
}

impl<'de> serde::Deserializer<'de> for FormPairs {
    type Error = FormDecodeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_map(PairAccess {
            groups: self.groups,
            cursor: 0,
        })
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct enum identifier ignored_any
    }
}

/// Map access walking the grouped pairs in first-occurrence order; also
/// serves as the VALUE-position deserializer for the pair under the cursor
/// (serde seeds receive `&mut PairAccess`).
struct PairAccess {
    groups: Vec<(String, Vec<String>)>,
    cursor: usize,
}

impl<'de> serde::de::MapAccess<'de> for PairAccess {
    type Error = FormDecodeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: serde::de::DeserializeSeed<'de>,
    {
        let Some(group) = self.groups.get(self.cursor) else {
            return Ok(None);
        };
        seed.deserialize(FieldKey(group.0.clone())).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        let Some((name, values)) = self.groups.get(self.cursor) else {
            return Err(FormDecodeError::Malformed);
        };
        let parts = FieldValues {
            name: name.clone(),
            values: values.clone(),
        };
        self.cursor += 1;
        seed.deserialize(parts)
    }
}

/// Key-position payload: serde's derived field identifiers arrive here via
/// `deserialize_identifier`.
struct FieldKey(String);

impl<'de> serde::Deserializer<'de> for FieldKey {
    type Error = FormDecodeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_string(self.0)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct enum identifier ignored_any
    }
}

/// Value-position payload for one grouped key: exactly one value behaves as
/// the scalar itself, more than one feeds a sequence visitor and errors on
/// any scalar request ([`FormDecodeError::DuplicateField`]).
struct FieldValues {
    name: String,
    values: Vec<String>,
}

impl FieldValues {
    fn single(&self) -> Result<&str, FormDecodeError> {
        match self.values.as_slice() {
            [only] => Ok(only.as_str()),
            _ => Err(FormDecodeError::DuplicateField(self.name.clone())),
        }
    }
}

impl<'de> serde::Deserializer<'de> for FieldValues {
    type Error = FormDecodeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_string(self.values.first().cloned().unwrap_or_default())
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value = parse_bool(self.single()?)?;
        visitor.visit_bool(value)
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: i8 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_i8(value)
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: i16 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_i16(value)
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: i32 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_i32(value)
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: i64 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_i64(value)
    }

    fn deserialize_i128<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: i128 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_i128(value)
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: u8 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_u8(value)
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: u16 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_u16(value)
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: u32 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_u32(value)
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: u64 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_u64(value)
    }

    fn deserialize_u128<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: u128 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_u128(value)
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: f32 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_f32(value)
    }

    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let value: f64 = self.single()?.parse().map_err(invalid_number)?;
        visitor.visit_f64(value)
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let text = self.single()?;
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(single), None) => visitor.visit_char(single),
            _ => Err(FormDecodeError::Schema(format!("invalid char `{text}`"))),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_string(self.single()?.to_owned())
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_seq(RepeatedValues {
            name: self.name,
            values: self.values,
            front: 0,
        })
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_map<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        Err(FormDecodeError::Schema(
            "nested composites are not representable in flat forms".to_owned(),
        ))
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.deserialize_map(_visitor)
    }

    serde::forward_to_deserialize_any! {
        bytes byte_buf identifier enum
        tuple tuple_struct unit_struct
    }
}

/// Sequence access over the repeated values of ONE key (repeated keys
/// collect into `Vec` fields, §16).
struct RepeatedValues {
    name: String,
    values: Vec<String>,
    front: usize,
}

impl<'de> serde::de::SeqAccess<'de> for RepeatedValues {
    type Error = FormDecodeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: serde::de::DeserializeSeed<'de>,
    {
        let Some(value) = self.values.get(self.front) else {
            return Ok(None);
        };
        self.front += 1;
        seed.deserialize(FieldValues {
            name: self.name.clone(),
            values: vec![value.clone()],
        })
        .map(Some)
    }
}

fn parse_bool(text: &str) -> Result<bool, FormDecodeError> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(FormDecodeError::Schema(format!(
            "invalid boolean `{other}`"
        ))),
    }
}

fn invalid_number(error: impl std::fmt::Display) -> FormDecodeError {
    FormDecodeError::Schema(format!("invalid number: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq, Deserialize)]
    struct LoginForm {
        username: String,
        password: String,
        remember_me: Option<bool>,
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Tagged {
        tags: Vec<String>,
        count: u32,
    }

    fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, FormDecodeError> {
        let pairs = parse_form_bytes(body.as_bytes())?;
        deserialize_form_pairs(&pairs)
    }

    #[test]
    fn parses_ordered_pairs_with_plus_and_percent_rules() {
        let pairs = parse_form_bytes(b"a=1&name=hello+world&sym=%26%3D%3F&uni=caf%C3%A9")
            .expect("well-formed");
        assert_eq!(
            pairs,
            vec![
                ("a".to_owned(), "1".to_owned()),
                ("name".to_owned(), "hello world".to_owned()),
                ("sym".to_owned(), "&=?".to_owned()),
                ("uni".to_owned(), "café".to_owned()),
            ]
        );
    }

    #[test]
    fn missing_equals_yields_empty_value_but_empty_segments_are_malformed() {
        let pairs = parse_form_bytes(b"flag&x=").expect("bare token allowed");
        assert_eq!(pairs[0], ("flag".to_owned(), String::new()));
        assert_eq!(pairs[1], ("x".to_owned(), String::new()));

        assert_eq!(
            parse_form_bytes(b"a=1&&b=2"),
            Err(FormDecodeError::Malformed)
        );
        assert_eq!(parse_form_bytes(b"a=1&"), Err(FormDecodeError::Malformed));
    }

    #[test]
    fn empty_body_parses_as_zero_pairs_and_non_utf8_is_rejected() {
        assert!(parse_form_bytes(b"").expect("empty").is_empty());
        assert_eq!(
            parse_form_bytes(&[0xFF, 0xFE]),
            Err(FormDecodeError::NotUtf8)
        );
    }

    #[test]
    fn truncated_and_bad_percent_escapes_are_malformed() {
        assert_eq!(parse_form_bytes(b"%4"), Err(FormDecodeError::Malformed));
        assert_eq!(parse_form_bytes(b"%G1"), Err(FormDecodeError::Malformed));
        assert_eq!(parse_form_bytes(b"k=a%2"), Err(FormDecodeError::Malformed));
        // Decoded bytes must reassemble into UTF-8 (D-impl-charset-rejection).
        assert_eq!(parse_form_bytes(b"k=%FF"), Err(FormDecodeError::NotUtf8));
    }

    #[test]
    fn decodes_flat_structs_with_optional_fields() {
        let decoded: LoginForm =
            decode("username=ada&password=s3cret&remember_me=true").expect("all fields present");
        assert_eq!(
            decoded,
            LoginForm {
                username: "ada".to_owned(),
                password: "s3cret".to_owned(),
                remember_me: Some(true),
            }
        );

        let minimal: LoginForm = decode("username=ada&password=x").expect("optional absent");
        assert_eq!(minimal.remember_me, None);

        let error = decode::<LoginForm>("username=ada").expect_err("missing password");
        assert!(
            !error.is_syntax(),
            "missing fields are data errors: {error}"
        );
    }

    #[test]
    fn invalid_types_are_data_errors_not_syntax_errors() {
        let error = decode::<LoginForm>("username=a&password=p&remember_me=maybe")
            .expect_err("bad boolean");
        assert!(!error.is_syntax(), "{error}");

        let syntax = decode::<LoginForm>("user%name=%2").expect_err("malformed escape");
        assert!(syntax.is_syntax(), "{syntax}");
    }

    #[test]
    fn duplicate_scalars_error_but_sequences_collect_repeated_keys() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct Scalar {
            v: u32,
        }
        let error = decode::<Scalar>("v=1&v=2").expect_err("duplicate scalar");
        assert_eq!(error, FormDecodeError::DuplicateField("v".to_owned()));

        let tagged: Tagged = decode("tags=a&count=1&tags=b&tags=c").expect("repeated keys");
        assert_eq!(
            tagged,
            Tagged {
                tags: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
                count: 1,
            }
        );
    }

    #[test]
    fn unknown_keys_are_ignored_by_default() {
        let decoded: LoginForm =
            decode("username=u&password=p&extra=noise&remember_me=false").expect("decoded");
        assert_eq!(decoded.username, "u");
        assert_eq!(decoded.remember_me, Some(false));
    }

    #[test]
    fn maps_decode_like_urlencoded_scalar_maps() {
        let decoded: BTreeMap<String, String> = decode("a=1&b=hello+world").expect("scalar map");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded["b"], "hello world");

        let nested = decode::<LoginForm>("").expect_err("empty pairs miss required fields");
        assert!(!nested.is_syntax());
    }

    #[test]
    fn size_gate_runs_before_parsing_work() {
        // Over the limit even though the content would parse fine.
        let error =
            decode_form_limited::<LoginForm>(b"username=a&password=b", 10).expect_err("over limit");
        assert_eq!(
            error,
            FormDecodeError::TooLarge { limit: 10 },
            "the size gate must win over parse results"
        );

        let decoded: LoginForm =
            decode_form_limited(b"username=a&password=b", 1024).expect("within limit");
        assert_eq!(decoded.username, "a");

        // Exactly at the limit succeeds.
        let body = b"username=a";
        let parsed: BTreeMap<String, String> =
            decode_form_limited(body, body.len()).expect("exactly at limit");
        assert_eq!(parsed["username"], "a");
    }

    #[test]
    fn round_trips_against_the_bounded_encoder() {
        use crate::encode::serialize_form_limited;

        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct Flat {
            name: String,
            count: u32,
            tags: Vec<String>,
            note: Option<String>,
        }

        let value = Flat {
            name: "gear box".to_owned(),
            count: 7,
            tags: vec!["a".to_owned(), "b".to_owned()],
            note: Some("café".to_owned()),
        };
        let bytes = serialize_form_limited(&value, 512).expect("encode under limit");
        let decoded: Flat = decode_form_limited(&bytes, 512).expect("decode");
        assert_eq!(decoded, value);
    }
}
