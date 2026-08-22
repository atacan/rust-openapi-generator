//! Bounded serialization with fail-fast enforcement (main spec §34).

use std::io;
use std::io::Write as _;

use bytes::Bytes;
use serde::ser;
use serde::Serialize;

/// Raised when encoded output would exceed its configured byte limit.
///
/// Also returned defensively for value shapes bounded form serialization never
/// produces (nested composites in value positions, units, bytes, non-finite
/// floats), since partial output must never escape (sections 34.1, 34.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("encoded output exceeds limit of {limit} bytes")]
pub struct EncodeTooLarge {
    /// The configured limit in bytes.
    pub limit: usize,
}

impl serde::ser::Error for EncodeTooLarge {
    fn custom<T>(_msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        Self { limit: usize::MAX }
    }
}

/// Writer forwarding to an inner writer while counting accepted bytes,
/// refusing any write whose end would land strictly above `limit`.
///
/// A refused write forwards neither wholly nor partially, so the inner buffer
/// never exceeds `limit`; a write ending exactly at `limit` succeeds.
pub struct CountingWriter<W> {
    inner: W,
    limit: usize,
    written: usize,
}

impl<W> CountingWriter<W> {
    pub fn new(inner: W, limit: usize) -> Self {
        Self {
            inner,
            limit,
            written: 0,
        }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }

    pub fn bytes_written(&self) -> usize {
        self.written
    }
}

impl<W: io::Write> io::Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let overflow = EncodeTooLarge { limit: self.limit };
        let total = self
            .written
            .checked_add(buf.len())
            .ok_or_else(|| io::Error::other(overflow))?;
        if total > self.limit {
            return Err(io::Error::other(EncodeTooLarge { limit: self.limit }));
        }
        self.inner.write_all(buf)?;
        self.written = total;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Serializes `value` as JSON, enforcing `limit` during serialization.
///
/// The limit fails fast through [`CountingWriter`] rather than buffering the
/// document first (section 34); on error no partial output escapes. Any
/// serializer failure surfaces as [`EncodeTooLarge`].
pub fn serialize_json_limited<T>(value: &T, limit: usize) -> Result<Bytes, EncodeTooLarge>
where
    T: Serialize,
{
    let mut writer = CountingWriter::new(
        Vec::with_capacity(limit.min(INITIAL_ENCODE_CAPACITY)),
        limit,
    );
    {
        let mut serializer = serde_json::Serializer::new(&mut writer);
        Serialize::serialize(value, &mut serializer).map_err(|_| EncodeTooLarge { limit })?;
    }
    Ok(Bytes::from(writer.into_inner()))
}

/// Serializes `value` as an `application/x-www-form-urlencoded` document,
/// enforcing `limit` during serialization per DECISIONS.md D-impl-forms.
///
/// Supported shapes: structs, maps with scalar values, sequences of scalars
/// (repeated key), `Option<T>` (`None` omits the pair entirely), and scalars
/// (bool/int/float/String/str/char). Keys and values encode identically:
/// UTF-8 percent-encoding of reserved bytes, space as `+`, floats via standard
/// Display, non-finite floats rejected. Unsupported shapes abort with
/// [`EncodeTooLarge`] as a defensive fallback.
pub fn serialize_form_limited<T>(value: &T, limit: usize) -> Result<Bytes, EncodeTooLarge>
where
    T: Serialize,
{
    let mut form = FormSerializer {
        writer: CountingWriter::new(
            Vec::with_capacity(limit.min(INITIAL_ENCODE_CAPACITY)),
            limit,
        ),
        limit,
        wrote_any_pair: false,
        staged_key: None,
        context: FormContext::Root,
    };
    Serialize::serialize(value, &mut form).map_err(|_| EncodeTooLarge { limit })?;
    Ok(Bytes::from(form.writer.into_inner()))
}

const INITIAL_ENCODE_CAPACITY: usize = 8 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormContext {
    Root,
    Field,
    SeqElement,
}

struct FormSerializer<W> {
    writer: CountingWriter<W>,
    limit: usize,
    wrote_any_pair: bool,
    staged_key: Option<Vec<u8>>,
    context: FormContext,
}

impl<W: io::Write> FormSerializer<W> {
    fn defensive(limit: usize) -> EncodeTooLarge {
        EncodeTooLarge { limit }
    }

    fn emit(&mut self, data: &[u8]) -> Result<(), EncodeTooLarge> {
        self.writer
            .write_all(data)
            .map_err(|_| Self::defensive(self.limit))
    }

    fn emit_pair(&mut self, key: &[u8], value: &[u8]) -> Result<(), EncodeTooLarge> {
        if self.wrote_any_pair {
            self.emit(b"&")?;
        }
        self.wrote_any_pair = true;
        self.emit(key)?;
        self.emit(b"=")?;
        self.emit(value)
    }

    fn take_staged_key(&mut self) -> Result<Vec<u8>, EncodeTooLarge> {
        self.staged_key
            .take()
            .ok_or_else(|| Self::defensive(self.limit))
    }

    fn stage_encoded_key(&mut self, key: Vec<u8>) {
        self.staged_key = Some(key);
        self.context = FormContext::Field;
    }

    fn stage_element(&mut self, key: &[u8]) {
        self.staged_key = Some(key.to_vec());
        self.context = FormContext::SeqElement;
    }

    fn stage_str_key(&mut self, key: &str) {
        let mut encoded = Vec::new();
        push_percent_encoded(&mut encoded, key);
        self.stage_encoded_key(encoded);
    }

    fn emit_str_value(&mut self, value: &str) -> Result<(), EncodeTooLarge> {
        let key = self.take_staged_key()?;
        self.context = FormContext::Root;
        let mut encoded = Vec::new();
        push_percent_encoded(&mut encoded, value);
        self.emit_pair(&key, &encoded)
    }

    fn emit_display_value<T: std::fmt::Display>(&mut self, value: T) -> Result<(), EncodeTooLarge> {
        let key = self.take_staged_key()?;
        self.context = FormContext::Root;
        let mut encoded = Vec::new();
        push_display(&mut encoded, value);
        self.emit_pair(&key, &encoded)
    }
}

impl<'a, W: io::Write> ser::Serializer for &'a mut FormSerializer<W> {
    type Ok = ();
    type Error = EncodeTooLarge;
    type SerializeSeq = FormSeq<'a, W>;
    type SerializeTuple = ser::Impossible<(), EncodeTooLarge>;
    type SerializeTupleStruct = ser::Impossible<(), EncodeTooLarge>;
    type SerializeTupleVariant = ser::Impossible<(), EncodeTooLarge>;
    type SerializeMap = FormMap<'a, W>;
    type SerializeStruct = FormStruct<'a, W>;
    type SerializeStructVariant = ser::Impossible<(), EncodeTooLarge>;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_i128(self, value: i128) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_u128(self, value: u128) -> Result<Self::Ok, Self::Error> {
        self.emit_display_value(value)
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            self.emit_display_value(value)
        } else {
            Err(FormSerializer::<W>::defensive(self.limit))
        }
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            self.emit_display_value(value)
        } else {
            Err(FormSerializer::<W>::defensive(self.limit))
        }
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        let mut buffer = [0u8; 4];
        self.emit_str_value(value.encode_utf8(&mut buffer))
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.emit_str_value(value)
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.staged_key = None;
        self.context = FormContext::Root;
        Ok(())
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        match self.staged_key.take() {
            Some(key) if self.context == FormContext::Field => {
                self.context = FormContext::SeqElement;
                Ok(FormSeq {
                    serializer: self,
                    key,
                })
            }
            _ => Err(FormSerializer::<W>::defensive(self.limit)),
        }
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        if self.context != FormContext::Root || self.staged_key.is_some() {
            return Err(FormSerializer::<W>::defensive(self.limit));
        }
        Ok(FormMap { serializer: self })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        if self.context != FormContext::Root || self.staged_key.is_some() {
            return Err(FormSerializer::<W>::defensive(self.limit));
        }
        Ok(FormStruct { serializer: self })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(FormSerializer::<W>::defensive(self.limit))
    }
}

struct FormSeq<'a, W> {
    serializer: &'a mut FormSerializer<W>,
    key: Vec<u8>,
}

impl<W: io::Write> ser::SerializeSeq for FormSeq<'_, W> {
    type Ok = ();
    type Error = EncodeTooLarge;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.serializer.stage_element(&self.key);
        value.serialize(&mut *self.serializer)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct FormMap<'a, W> {
    serializer: &'a mut FormSerializer<W>,
}

impl<W: io::Write> ser::SerializeMap for FormMap<'_, W> {
    type Ok = ();
    type Error = EncodeTooLarge;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        key.serialize(FormKeyStager {
            serializer: &mut *self.serializer,
        })
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut *self.serializer)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct FormStruct<'a, W> {
    serializer: &'a mut FormSerializer<W>,
}

impl<W: io::Write> ser::SerializeStruct for FormStruct<'_, W> {
    type Ok = ();
    type Error = EncodeTooLarge;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.serializer.stage_str_key(key);
        value.serialize(&mut *self.serializer)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct FormKeyStager<'a, W> {
    serializer: &'a mut FormSerializer<W>,
}

impl<W: io::Write> FormKeyStager<'_, W> {
    fn stage_display<T: std::fmt::Display>(self, value: T) -> Result<(), EncodeTooLarge> {
        let mut encoded = Vec::new();
        push_display(&mut encoded, value);
        self.serializer.stage_encoded_key(encoded);
        Ok(())
    }

    fn stage_str(self, value: &str) -> Result<(), EncodeTooLarge> {
        self.serializer.stage_str_key(value);
        Ok(())
    }

    fn reject<T>(self) -> Result<T, EncodeTooLarge> {
        Err(EncodeTooLarge {
            limit: self.serializer.limit,
        })
    }
}

impl<'a, W: io::Write> ser::Serializer for FormKeyStager<'a, W> {
    type Ok = ();
    type Error = EncodeTooLarge;
    type SerializeSeq = ser::Impossible<(), EncodeTooLarge>;
    type SerializeTuple = ser::Impossible<(), EncodeTooLarge>;
    type SerializeTupleStruct = ser::Impossible<(), EncodeTooLarge>;
    type SerializeTupleVariant = ser::Impossible<(), EncodeTooLarge>;
    type SerializeMap = ser::Impossible<(), EncodeTooLarge>;
    type SerializeStruct = ser::Impossible<(), EncodeTooLarge>;
    type SerializeStructVariant = ser::Impossible<(), EncodeTooLarge>;

    fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_i128(self, value: i128) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_u128(self, value: u128) -> Result<Self::Ok, Self::Error> {
        self.stage_display(value)
    }

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            self.stage_display(value)
        } else {
            self.reject()
        }
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            self.stage_display(value)
        } else {
            self.reject()
        }
    }

    fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
        let mut buffer = [0u8; 4];
        self.stage_str(value.encode_utf8(&mut buffer))
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.stage_str(value)
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
        self.reject()
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.reject()
    }

    fn serialize_some<T>(self, _value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.reject()
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.reject()
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.reject()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.reject()
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.reject()
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.reject()
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.reject()
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.reject()
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.reject()
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.reject()
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.reject()
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.reject()
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.reject()
    }
}

/// WHATWG application/x-www-form-urlencoded serializer rules.
fn push_percent_encoded(out: &mut Vec<u8>, value: &str) {
    const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in value.as_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(byte),
            b' ' => out.push(b'+'),
            _ => {
                out.push(b'%');
                out.push(HEX_DIGITS[usize::from(byte >> 4)]);
                out.push(HEX_DIGITS[usize::from(byte & 0x0F)]);
            }
        }
    }
}

fn push_display<T: std::fmt::Display>(out: &mut Vec<u8>, value: T) {
    out.extend_from_slice(format!("{value}").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Widget {
        name: String,
        count: u32,
        tags: Vec<String>,
        note: Option<String>,
    }

    fn oversized_widget(rows: usize) -> Widget {
        Widget {
            name: "w".repeat(rows),
            count: 0,
            tags: Vec::new(),
            note: None,
        }
    }

    #[test]
    fn json_round_trips_a_representative_struct_under_limit() {
        let widget = Widget {
            name: "gear".to_owned(),
            count: 7,
            tags: vec!["a".to_owned(), "b".to_owned()],
            note: Some("hi".to_owned()),
        };
        let bytes =
            serialize_json_limited(&widget, 1024).expect("representative struct under limit");
        assert_eq!(
            bytes.as_ref(),
            br#"{"name":"gear","count":7,"tags":["a","b"],"note":"hi"}"# as &[u8]
        );
        let decoded: Widget = serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(decoded, widget);
    }

    #[test]
    fn json_exactly_at_limit_succeeds_and_empty_over_limit_fails() {
        assert_eq!(
            serialize_json_limited(&(), 4)
                .expect("null fits a 4-byte budget")
                .as_ref(),
            b"null"
        );
        let error = serialize_json_limited(&(), 3).expect_err("null misses a 3-byte budget");
        assert_eq!(error, EncodeTooLarge { limit: 3 });
    }

    #[test]
    fn json_oversized_value_errors_and_writer_stays_bounded() {
        let widget = oversized_widget(64);
        let error = serialize_json_limited(&widget, 32).expect_err("over limit");
        assert_eq!(error, EncodeTooLarge { limit: 32 });

        let mut writer = CountingWriter::new(Vec::<u8>::new(), 32);
        {
            let mut serializer = serde_json::Serializer::new(&mut writer);
            assert!(Serialize::serialize(&widget, &mut serializer).is_err());
        }
        assert!(writer.bytes_written() <= 32);
        assert!(writer.into_inner().len() <= 32);
    }

    #[test]
    fn json_fail_fast_holds_for_values_encoding_to_about_100x_the_limit() {
        let widget = oversized_widget(2048);
        let limit = 128;

        let result = serialize_json_limited(&widget, limit);
        assert!(matches!(result, Err(EncodeTooLarge { limit: 128 })));

        let mut writer = CountingWriter::new(Vec::<u8>::new(), limit);
        {
            let mut serializer = serde_json::Serializer::new(&mut writer);
            assert!(Serialize::serialize(&widget, &mut serializer).is_err());
        }
        assert!(writer.bytes_written() <= limit);
        assert!(writer.into_inner().len() <= limit);
    }

    #[derive(Serialize)]
    struct FlatForm {
        absent: Option<String>,
        present: Option<&'static str>,
        items: Vec<&'static str>,
        flag: bool,
        count: i32,
        ratio: f64,
        greeting: &'static str,
        unicode: &'static str,
    }

    fn flat_form() -> FlatForm {
        FlatForm {
            absent: None,
            present: Some("yes"),
            items: vec!["a", "b"],
            flag: true,
            count: -42,
            ratio: 1.5,
            greeting: "hello world",
            unicode: "café",
        }
    }

    #[test]
    fn form_encodes_struct_fields_per_urlencoded_rules() {
        let bytes = serialize_form_limited(&flat_form(), 512).expect("flat form under limit");
        assert_eq!(
            bytes.as_ref(),
            b"present=yes&items=a&items=b&flag=true&count=-42&ratio=1.5&greeting=hello+world&unicode=caf%C3%A9"
        );
    }

    #[test]
    fn form_encodes_scalar_maps_deterministically() {
        let mut map = BTreeMap::new();
        map.insert("b key", "v&v");
        map.insert("a", "1");
        let bytes = serialize_form_limited(&map, 256).expect("scalar map under limit");
        assert_eq!(bytes.as_ref(), b"a=1&b+key=v%26v");

        let empty: BTreeMap<&str, &str> = BTreeMap::new();
        assert_eq!(
            serialize_form_limited(&empty, 16).expect("empty map yields an empty body"),
            Bytes::new()
        );
    }

    #[test]
    fn form_handles_nested_options_and_rejects_unsupported_shapes_defensively() {
        #[derive(Serialize)]
        struct NestedOptions {
            flat_some: Option<Option<&'static str>>,
            hidden: Option<Option<&'static str>>,
        }

        let bytes = serialize_form_limited(
            &NestedOptions {
                flat_some: Some(Some("deep")),
                hidden: Some(None),
            },
            64,
        )
        .expect("nested options flatten");
        assert_eq!(bytes.as_ref(), b"flat_some=deep");

        #[derive(Serialize)]
        struct Inner {
            x: u32,
        }

        #[derive(Serialize)]
        struct Outer {
            inner: Inner,
        }

        assert!(
            serialize_form_limited(
                &Outer {
                    inner: Inner { x: 1 }
                },
                64
            )
            .is_err(),
            "nested structs abort defensively"
        );

        #[derive(Serialize)]
        struct NonFinite {
            ratio: f64,
        }

        assert!(serialize_form_limited(&NonFinite { ratio: f64::NAN }, 64).is_err());
    }

    #[test]
    fn form_oversize_fails_fast_with_writer_staying_bounded() {
        let error = serialize_form_limited(&flat_form(), 8).expect_err("over limit");
        assert_eq!(error, EncodeTooLarge { limit: 8 });

        let mut form = FormSerializer {
            writer: CountingWriter::new(Vec::<u8>::new(), 8),
            limit: 8,
            wrote_any_pair: false,
            staged_key: None,
            context: FormContext::Root,
        };
        assert!(Serialize::serialize(&flat_form(), &mut form).is_err());
        assert!(form.writer.bytes_written() <= 8);
        assert!(form.writer.into_inner().len() <= 8);
    }
}
