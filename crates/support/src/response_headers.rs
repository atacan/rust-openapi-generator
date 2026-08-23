//! Typed response-header conversion (main spec §15, §48).
//!
//! Generated server wrappers store domain values; converting them into
//! [`http::HeaderValue`] can fail for String-typed fields, so §48 prefers
//! checked constructors. This module hosts the shared error and the checked
//! conversion used both by generated `<Op><Status>::new` constructors (eager
//! validation) and by the generated `IntoResponse` path, whose failure maps
//! to the fixed empty 500 fallback with its hook firing (§34.1 machinery).

/// A documented header value failed HTTP header conversion (main spec §48).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("response header `{name}` value failed HTTP header conversion")]
pub struct InvalidResponseHeader {
    /// Verbatim wire name of the offending header.
    pub name: &'static str,
}

/// Converts one header value eagerly (§48 checked-constructor territory).
///
/// # Errors
///
/// [`InvalidResponseHeader`] naming the rejected header when `value` is not
/// a legal visible-ASCII header value.
pub fn checked_value(
    name: &'static str,
    value: &str,
) -> Result<http::HeaderValue, InvalidResponseHeader> {
    http::HeaderValue::try_from(value).map_err(|_| InvalidResponseHeader { name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_values_convert_and_invalid_ones_name_the_header() {
        assert_eq!(
            checked_value("Location", "/widgets/w1").expect("valid"),
            http::HeaderValue::from_static("/widgets/w1")
        );

        let error = checked_value("Location", "bad\u{7F}value").expect_err("control byte");
        assert_eq!(error.name, "Location");
        assert!(matches!(error, InvalidResponseHeader { name: "Location" }));
        assert!(error.to_string().contains("Location"));
    }
}
