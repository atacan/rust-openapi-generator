//! Percent-encoding helpers (companion §8).
//!
//! Path parameters percent-encode using the RFC 3986 unreserved set
//! (`ALPHA / DIGIT / "-" / "." / "_" / "~"`, openapi-semantics-spec §6/§8);
//! query components use the WHATWG `application/x-www-form-urlencoded`
//! serializer rules shared with form bodies, so a space serializes as `+`.

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes one path-segment value (companion §8).
///
/// Every RFC 3986 unreserved byte stays literal; every other byte — reserved
/// delimiters, space, and UTF-8 continuation bytes alike — becomes `%XY` with
/// uppercase hex digits.
#[must_use]
pub fn encode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(HEX_DIGITS[usize::from(byte >> 4)] as char);
                out.push(HEX_DIGITS[usize::from(byte & 0x0F)] as char);
            }
        }
    }
    out
}

/// Percent-encodes one query-component value in `application/x-www-form-urlencoded`
/// style: unreserved ASCII stays literal (including `*`), space becomes `+`,
/// and all remaining bytes become uppercase `%XY` escapes. This reuses the same
/// serializer rules as the bounded form encoder ([`crate::encode`]).
#[must_use]
pub fn encode_query_component(value: &str) -> String {
    let mut out = Vec::with_capacity(value.len());
    crate::encode::push_percent_encoded(&mut out, value);
    // The input is valid UTF-8 and only ASCII bytes are inserted, so the
    // output is valid UTF-8 by construction.
    String::from_utf8(out).expect("ASCII-only insertions preserve UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segment_keeps_rfc3986_unreserved_literal() {
        assert_eq!(encode_path_segment("abcXYZ0189-._~"), "abcXYZ0189-._~");
    }

    #[test]
    fn path_segment_encodes_reserved_and_space_with_uppercase_hex() {
        assert_eq!(encode_path_segment("a b"), "a%20b");
        assert_eq!(encode_path_segment("/?:#&="), "%2F%3F%3A%23%26%3D");
        assert_eq!(encode_path_segment("+*"), "%2B%2A");
    }

    #[test]
    fn path_segment_encodes_utf8_multibyte_bytes() {
        assert_eq!(encode_path_segment("é"), "%C3%A9");
        assert_eq!(encode_path_segment("€1"), "%E2%82%AC1");
    }

    #[test]
    fn query_component_uses_form_urlencoded_rules() {
        assert_eq!(encode_query_component("a b c"), "a+b+c");
        assert_eq!(encode_query_component("plain-0_*."), "plain-0_*.");
        // WHATWG rules escape `~`, unlike the RFC 3986 segment encoder.
        assert_eq!(encode_query_component("~"), "%7E");
        assert_eq!(encode_query_component("&=?#"), "%26%3D%3F%23");
        assert_eq!(encode_query_component("é"), "%C3%A9");
    }
}
