//! Presence/nullability matrix support (companion §2.1).
//!
//! Generated structs map each property cell of the §2.1 matrix explicitly:
//!
//! | Presence | Nullability | Representation |
//! |---|---|---|
//! | required | non-nullable | plain `T` |
//! | required | nullable | `Option<T>` through [`presence::deserialize_required_nullable`] |
//! | optional | non-nullable | [`OptionalField<T>`] |
//! | optional | nullable | plain `Option<T>` with `#[serde(default)]` |

use std::fmt;
use std::marker::PhantomData;

/// Wrapper for the optional + non-nullable cell (companion §2.1).
///
/// A missing key deserializes as [`OptionalField::Absent`] (paired with
/// `#[serde(default)]`); an explicit JSON `null` is a decode error, because
/// the contract declares the property non-nullable.
#[derive(Default)]
pub enum OptionalField<T> {
    /// Property key was absent from the document.
    #[default]
    Absent,
    /// Property carried a value.
    Present(T),
}

impl<T> OptionalField<T> {
    /// Returns the wrapped value if present.
    #[must_use]
    pub fn into_inner(self) -> Option<T> {
        match self {
            Self::Absent => None,
            Self::Present(value) => Some(value),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for OptionalField<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("Absent"),
            Self::Present(value) => f.debug_tuple("Present").field(value).finish(),
        }
    }
}

impl<T: Clone> Clone for OptionalField<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Absent => Self::Absent,
            Self::Present(value) => Self::Present(value.clone()),
        }
    }
}

impl<T: Copy> Copy for OptionalField<T> {}

impl<T: PartialEq> PartialEq for OptionalField<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Absent, Self::Absent) => true,
            (Self::Present(lhs), Self::Present(rhs)) => lhs == rhs,
            _ => false,
        }
    }
}

impl<T: Eq> Eq for OptionalField<T> {}

/// `skip_serializing_if` predicate omitting [`OptionalField::Absent`] fields
/// entirely from generated output.
#[must_use]
pub fn is_absent<T>(field: &OptionalField<T>) -> bool {
    matches!(field, OptionalField::Absent)
}

impl<'de, T> serde::Deserialize<'de> for OptionalField<T>
where
    T: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor<T>(PhantomData<T>);

        impl<'de, T> serde::de::Visitor<'de> for Visitor<T>
        where
            T: serde::Deserialize<'de>,
        {
            type Value = OptionalField<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a value for an optional non-nullable field")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Err(E::custom(EXPLICIT_NULL_DISALLOWED))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Err(E::custom(EXPLICIT_NULL_DISALLOWED))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                T::deserialize(deserializer).map(Self::Value::Present)
            }
        }

        deserializer.deserialize_option(Visitor(PhantomData))
    }
}

const EXPLICIT_NULL_DISALLOWED: &str =
    "explicit null is not allowed for optional non-nullable field";

impl<T> serde::Serialize for OptionalField<T>
where
    T: serde::Serialize,
{
    /// Writes only the wrapped value; generated structs pair this type with
    /// `skip_serializing_if = "openapi_support::optional::is_absent"` so an
    /// absent field never reaches output at all.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Absent => serializer.serialize_none(),
            Self::Present(value) => value.serialize(serializer),
        }
    }
}

/// Presence-aware decoding adapters for required properties (companion §2.1).
pub mod presence {
    use serde::Deserialize as _;

    /// Decodes the required + nullable cell (companion §2.1): JSON `null`
    /// yields `Ok(None)`, any present value yields `Ok(Some(value))`, and a
    /// **missing** key is a schema violation.
    ///
    /// Used through `#[serde(deserialize_with = "...")]` on an `Option<T>`
    /// field *without* `#[serde(default)]`, serde itself raises the
    /// missing-field error before the adapter runs, which satisfies the
    /// contract; do not combine it with `#[serde(default)]`.
    pub fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: serde::Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    const NULL_ERROR_MESSAGE: &str = "explicit null is not allowed for optional non-nullable field";

    // a. required + non-nullable: plain field.
    #[derive(Debug, Deserialize)]
    struct RequiredNonNullable {
        id: u32,
    }

    #[test]
    fn required_non_nullable_fails_on_missing_key_and_explicit_null() {
        let error =
            serde_json::from_str::<RequiredNonNullable>("{}").expect_err("missing required key");
        assert!(
            error.to_string().starts_with("missing field `id`"),
            "{error}"
        );

        let error = serde_json::from_str::<RequiredNonNullable>(r#"{"id": null}"#)
            .expect_err("null violates non-nullability");
        assert!(error.to_string().contains("invalid type: null"), "{error}");

        let decoded: RequiredNonNullable = serde_json::from_str(r#"{"id": 7}"#).expect("value");
        assert_eq!(decoded.id, 7);
    }

    // b. required + nullable: presence-aware adapter.
    #[derive(Debug, Deserialize)]
    struct RequiredNullable {
        #[serde(deserialize_with = "presence::deserialize_required_nullable")]
        nickname: Option<String>,
    }

    #[test]
    fn required_nullable_null_yields_none_and_missing_key_fails() {
        let decoded: RequiredNullable =
            serde_json::from_str(r#"{"nickname": null}"#).expect("explicit null decodes to None");
        assert_eq!(decoded.nickname, None);

        let decoded: RequiredNullable =
            serde_json::from_str(r#"{"nickname": "ace"}"#).expect("present value decodes to Some");
        assert_eq!(decoded.nickname.as_deref(), Some("ace"));

        let error =
            serde_json::from_str::<RequiredNullable>("{}").expect_err("missing key is a violation");
        assert!(
            error.to_string().starts_with("missing field `nickname`"),
            "{error}"
        );
    }

    // c. optional + non-nullable: OptionalField.
    #[derive(Debug, Deserialize)]
    struct OptionalNonNullable {
        #[serde(default)]
        tag: OptionalField<String>,
    }

    #[test]
    fn optional_non_nullable_missing_key_is_absent_and_null_errors() {
        let decoded: OptionalNonNullable = serde_json::from_str("{}").expect("absent key");
        assert_eq!(decoded.tag, OptionalField::Absent);

        let decoded: OptionalNonNullable = serde_json::from_str(r#"{"tag": "x"}"#).expect("value");
        assert_eq!(decoded.tag, OptionalField::Present(String::from("x")));

        let error = serde_json::from_str::<OptionalNonNullable>(r#"{"tag": null}"#)
            .expect_err("explicit null rejected");
        assert!(error.to_string().starts_with(NULL_ERROR_MESSAGE), "{error}");
    }

    // d. optional + nullable: plain Option<T> with default.
    #[derive(Debug, Deserialize)]
    struct OptionalNullable {
        #[serde(default)]
        note: Option<String>,
    }

    #[test]
    fn optional_nullable_conflates_absent_and_null() {
        let decoded: OptionalNullable = serde_json::from_str("{}").expect("absent key");
        assert_eq!(decoded.note, None);

        let decoded: OptionalNullable = serde_json::from_str(r#"{"note": null}"#).expect("null");
        assert_eq!(decoded.note, None);

        let decoded: OptionalNullable = serde_json::from_str(r#"{"note": "hi"}"#).expect("value");
        assert_eq!(decoded.note.as_deref(), Some("hi"));
    }

    // e. serialization round trip.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct RoundTrip {
        always: u8,
        #[serde(
            default,
            skip_serializing_if = "is_absent",
            deserialize_with = "optional_field_or_default"
        )]
        maybe: OptionalField<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nullable: Option<String>,
    }

    fn optional_field_or_default<'de, D, T>(deserializer: D) -> Result<OptionalField<T>, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: serde::Deserialize<'de>,
    {
        Ok(OptionalField::deserialize(deserializer).unwrap_or(OptionalField::Absent))
    }

    #[test]
    fn absent_field_is_skipped_from_output_and_present_field_round_trips() {
        let absent = RoundTrip {
            always: 1,
            maybe: OptionalField::Absent,
            nullable: None,
        };
        let json = serde_json::to_string(&absent).expect("serialize");
        assert_eq!(json, r#"{"always":1}"#);

        let present = RoundTrip {
            always: 2,
            maybe: OptionalField::Present(String::from("m")),
            nullable: Some(String::from("n")),
        };
        let json = serde_json::to_string(&present).expect("serialize");
        assert_eq!(json, r#"{"always":2,"maybe":"m","nullable":"n"}"#);

        let decoded: RoundTrip = serde_json::from_str(&json).expect("round trip");
        assert_eq!(decoded, present);
    }

    #[test]
    fn wrapper_basics_default_into_inner_and_predicate() {
        assert_eq!(
            OptionalField::<u8>::default(),
            OptionalField::Absent,
            "Default is Absent"
        );
        assert_eq!(OptionalField::Present(3_u8).into_inner(), Some(3));
        assert_eq!(OptionalField::<u8>::Absent.into_inner(), None);
        assert!(is_absent(&OptionalField::<u8>::Absent));
        assert!(!is_absent(&OptionalField::Present(0_u8)));
        assert!(format!("{:?}", OptionalField::Present("v")).contains("\"v\""));

        #[allow(clippy::clone_on_copy)]
        let copied: OptionalField<u8> = OptionalField::Present(9);
        assert_eq!(copied.clone(), copied, "Copy types stay clonable");
    }
}
