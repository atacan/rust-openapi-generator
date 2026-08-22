//! Media-type parsing and `Content-Type` dispatch matching (main spec §28).
//!
//! §28 precedence: parse the media type without treating parameters such as
//! `charset=utf-8` as a different base type, then prefer exact OpenAPI content
//! entries, then structured suffix families (`+json`), then media type ranges,
//! then `*/*`. A syntactically unparseable value is never ignored or defaulted
//! (§28.1); it surfaces as [`MalformedContentType`].

/// Failure mode of [`parse_content_type`] (main spec §28.1).
///
/// The value was syntactically unparseable: server callers reject with `400`
/// and client callers surface a decode error — never defaulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("malformed Content-Type value")]
pub struct MalformedContentType;

/// Parsed media type (main spec §28 step 1).
///
/// `ty`, `subtype`, and `suffix` are normalized to ASCII lowercase for
/// matching; parameter names are lowercased while parameter values keep their
/// original case (the `charset` value is interpreted by callers per §28.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMediaType {
    /// Type token before `/`, lowercased.
    pub ty: String,
    /// Subtype token after `/` and before any `+suffix`, lowercased.
    pub subtype: String,
    /// Structured suffix after the last `+` (for example `json`), lowercased.
    pub suffix: Option<String>,
    /// Parameters in declaration order; values keep case verbatim.
    pub parameters: Vec<(String, String)>,
}

impl ParsedMediaType {
    fn base_subtype(&self) -> String {
        match &self.suffix {
            Some(suffix) => format!("{}+{}", self.subtype, suffix),
            None => self.subtype.clone(),
        }
    }
}

/// Outcome of matching a parsed media type against one OpenAPI content entry,
/// ranked exactly as the §28 precedence list: [`EntryMatch::Exact`] beats
/// [`EntryMatch::SuffixFamily`] beats [`EntryMatch::RangeMatch`] beats
/// [`EntryMatch::Wildcard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryMatch {
    /// Base type/subtype equals the entry (entry parameters ignored).
    Exact,
    /// Entry is a `+json` declaration and the parsed type belongs to the JSON
    /// suffix family (same suffix or plain `json`) with no exact match.
    SuffixFamily,
    /// Entry is a `type/*` range covering the parsed type.
    RangeMatch,
    /// Entry is `*/*`.
    Wildcard,
}

/// Parses a raw `Content-Type` header value (main spec §28 step 1, §28.1).
///
/// Strips surrounding whitespace, splits type/subtype at `/`, and parses
/// `key=value` parameters where quoted values (with backslash escapes) are
/// allowed. Anything unparseable — no slash, empty tokens, invalid characters,
/// unterminated quotes, or dangling/empty parameter segments — is an error;
/// it is never silently dropped or defaulted (§28.1).
#[allow(clippy::result_unit_err)]
pub fn parse_content_type(raw: &str) -> Result<ParsedMediaType, MalformedContentType> {
    let raw = raw.trim();
    let (ty_raw, rest) = raw.split_once('/').ok_or(MalformedContentType)?;
    let ty = parse_token(ty_raw.trim())?;

    // Everything after the first `;` is the parameter section; quoted values
    // that contain `;` are handled by the parameter scanner below.
    let (subtype_raw, params_raw) = match rest.split_once(';') {
        Some((subtype, params)) => (subtype, Some(params)),
        None => (rest, None),
    };
    let subtype_full = parse_token(subtype_raw.trim())?;
    let (subtype, suffix) = split_suffix(subtype_full)?;

    let mut parameters = Vec::new();
    if let Some(params) = params_raw {
        let mut rest = params;
        loop {
            let (name, value, next) = parse_one_parameter(rest)?;
            parameters.push((name.to_ascii_lowercase(), value));
            let Some(next) = next else { break };
            if next.is_empty() {
                // A trailing `;` leaves an empty parameter segment: malformed.
                return Err(MalformedContentType);
            }
            rest = next;
        }
    }

    Ok(ParsedMediaType {
        ty: ty.to_ascii_lowercase(),
        subtype: subtype.to_ascii_lowercase(),
        suffix: suffix.map(|suffix| suffix.to_ascii_lowercase()),
        parameters,
    })
}

fn split_suffix(subtype: &str) -> Result<(&str, Option<&str>), MalformedContentType> {
    match subtype.rsplit_once('+') {
        None => Ok((subtype, None)),
        Some((base, suffix)) if base.is_empty() || suffix.is_empty() => Err(MalformedContentType),
        Some((base, suffix)) => Ok((base, Some(suffix))),
    }
}

/// RFC 9110 `tchar`: the only characters valid in type/subtype/parameter-name
/// tokens.
fn is_tchar(byte: u8) -> bool {
    matches!(
        byte,
        b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

fn parse_token(token: &str) -> Result<&str, MalformedContentType> {
    if !token.is_empty() && token.bytes().all(is_tchar) {
        return Ok(token);
    }
    Err(MalformedContentType)
}

/// Parses one leading `name=value` parameter. Returns the remainder after its
/// terminating `;` (whitespace-stripped), or `None` when the parameter ends
/// the input cleanly. Empty segments (`;;`, a trailing `;`) and missing values
/// are malformed per §28.1 strictness.
fn parse_one_parameter(
    input: &str,
) -> Result<(String, String, Option<&str>), MalformedContentType> {
    let input = input.trim_start();
    let eq = input.find('=').ok_or(MalformedContentType)?;
    if input[..eq].contains(';') {
        return Err(MalformedContentType);
    }
    let name = parse_token(input[..eq].trim())?;
    let value_input = &input[eq + 1..];

    if let Some(quoted) = value_input.strip_prefix('"') {
        let mut value = String::new();
        let mut offset = 0usize;
        loop {
            let rest = &quoted[offset..];
            let Some(byte) = rest.as_bytes().first() else {
                return Err(MalformedContentType);
            };
            match byte {
                b'"' => {
                    offset += 1;
                    break;
                }
                b'\\' => {
                    // Quoted-pair: the escaped character is taken literally.
                    let Some(ch) = rest[1..].chars().next() else {
                        return Err(MalformedContentType);
                    };
                    value.push(ch);
                    offset += 1 + ch.len_utf8();
                }
                _ => {
                    let ch = rest.chars().next().expect("nonempty byte prefix");
                    value.push(ch);
                    offset += ch.len_utf8();
                }
            }
        }
        // After the closing quote only whitespace may precede the next `;`.
        let after = quoted[offset..].trim_start();
        match after.strip_prefix(';') {
            Some(next) => return Ok((name.to_owned(), value, Some(next.trim_start()))),
            None if after.is_empty() => return Ok((name.to_owned(), value, None)),
            None => return Err(MalformedContentType),
        }
    }

    let end = value_input.find(';').unwrap_or(value_input.len());
    let value = value_input[..end].trim();
    if !value.is_empty() && value.bytes().all(is_tchar) {
        let next = value_input[end..].strip_prefix(';').map(str::trim_start);
        return Ok((name.to_owned(), value.to_owned(), next));
    }
    Err(MalformedContentType)
}

/// Matches a parsed incoming media type against one OpenAPI content entry key,
/// implementing the §28 precedence rules for a single pair.
///
/// Rules:
/// - exact string equality on base type/subtype (entry parameters ignored) →
///   [`EntryMatch::Exact`] (§28 example: `application/problem+json;
///   charset=utf-8` matches `application/problem+json` exactly);
/// - an entry ending in `+json` matches a parsed type carrying the same suffix
///   or a plain `json` subtype → [`EntryMatch::SuffixFamily`]; callers rank via
///   the enum order because this function evaluates one pair at a time;
/// - `type/*` entries → [`EntryMatch::RangeMatch`]; `*/*` →
///   [`EntryMatch::Wildcard`];
/// - wildcard INCOMING types (`*/*`, `*/*`-typed) never match concrete entries
///   here: server callers reject them separately per §28.5, so this returns
///   [`None`] whenever [`is_wildcard_incoming`] holds.
///
/// Malformed entry keys simply do not match ([`None`]); they are not errors.
#[must_use]
pub fn match_entry(parsed: &ParsedMediaType, entry: &str) -> Option<EntryMatch> {
    if is_wildcard_incoming(parsed) {
        return None;
    }
    // The entry base precedes the first `;`; entry parameters are ignored.
    let base = entry.split(';').next()?.trim();
    let (entry_ty, entry_subtype) = base.split_once('/')?;
    let entry_ty = entry_ty.trim().to_ascii_lowercase();
    let entry_subtype = entry_subtype.trim().to_ascii_lowercase();
    if entry_ty.is_empty() || entry_subtype.is_empty() {
        return None;
    }

    if entry_ty == "*" && entry_subtype == "*" {
        return Some(EntryMatch::Wildcard);
    }
    if entry_subtype == "*" {
        return (entry_ty == parsed.ty).then_some(EntryMatch::RangeMatch);
    }

    if parsed.ty == entry_ty && parsed.base_subtype() == entry_subtype {
        return Some(EntryMatch::Exact);
    }
    if entry_subtype.ends_with("+json")
        && (parsed.suffix.as_deref() == Some("json") || parsed.subtype == "json")
    {
        return Some(EntryMatch::SuffixFamily);
    }
    None
}

/// True when the incoming type is a wildcard (`*/*` or `*/subtype`), which per
/// §28.5 must not select among documented request entries on the server.
#[must_use]
pub fn is_wildcard_incoming(parsed: &ParsedMediaType) -> bool {
    parsed.ty == "*" || parsed.subtype == "*"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(raw: &str) -> ParsedMediaType {
        parse_content_type(raw).expect("well-formed content type")
    }

    #[test]
    fn parses_type_subtype_suffix_and_parameters() {
        let parsed = parsed("Application/Problem+JSON; Charset=UTF-8; q=1");
        assert_eq!(parsed.ty, "application");
        assert_eq!(parsed.subtype, "problem");
        assert_eq!(parsed.suffix.as_deref(), Some("json"));
        assert_eq!(
            parsed.parameters,
            vec![
                ("charset".to_owned(), "UTF-8".to_owned()),
                ("q".to_owned(), "1".to_owned())
            ]
        );
    }

    #[test]
    fn quoted_parameters_preserve_semicolons_escapes_and_case() {
        let parsed = parsed(r#"application/json;charset="a;b\"c""#);
        assert_eq!(
            parsed.parameters,
            vec![("charset".to_owned(), "a;b\"c".to_owned())]
        );
    }

    #[test]
    fn whitespace_around_tokens_is_stripped() {
        let parsed = parsed("  application / json ; charset = utf-8 ");
        assert_eq!(parsed.ty, "application");
        assert_eq!(parsed.subtype, "json");
        assert_eq!(
            parsed.parameters,
            vec![("charset".to_owned(), "utf-8".to_owned())]
        );
    }

    #[test]
    fn malformed_values_are_rejected_never_defaulted() {
        for raw in [
            "",
            "application",
            "/json",
            "application/",
            "application/+json",
            "application/json+",
            "appli cation/json",
            "application/jso n",
            "application/json;",
            "application/json;;charset=utf-8",
            "application/json;charset=",
            "application/json;charset=\"unterminated",
            "application/json;charset=\"x\"junk",
            "application/json;charset",
            "application/js@on",
        ] {
            assert!(
                parse_content_type(raw).is_err(),
                "expected {raw:?} to be rejected"
            );
        }
    }

    #[test]
    fn exact_entry_matches_ignoring_entry_and_incoming_parameters() {
        let problem = parsed("application/problem+json; charset=utf-8");
        assert_eq!(
            match_entry(&problem, "application/problem+json"),
            Some(EntryMatch::Exact)
        );
        assert_eq!(
            match_entry(&problem, "application/problem+json; q=0.9"),
            Some(EntryMatch::Exact)
        );
        // Case-insensitive on both sides.
        assert_eq!(
            match_entry(
                &parsed("APPLICATION/PROBLEM+JSON"),
                "application/problem+json"
            ),
            Some(EntryMatch::Exact)
        );
    }

    #[test]
    fn suffix_family_matches_json_entries_without_exact_base() {
        let json = parsed("application/json");
        assert_eq!(
            match_entry(&json, "application/vnd.foo+json"),
            Some(EntryMatch::SuffixFamily)
        );
        let other_json = parsed("application/vnd.bar+json; charset=utf-8");
        assert_eq!(
            match_entry(&other_json, "application/vnd.foo+json"),
            Some(EntryMatch::SuffixFamily)
        );
        // Exact outranks suffix family when bases coincide.
        assert_eq!(
            match_entry(&other_json, "application/vnd.bar+json"),
            Some(EntryMatch::Exact)
        );
        // Non-JSON families never trigger family logic.
        assert_eq!(
            match_entry(&parsed("text/xml"), "application/vnd.foo+xml"),
            None
        );
        assert_eq!(
            match_entry(&parsed("text/plain"), "application/vnd.foo+json"),
            None
        );
    }

    #[test]
    fn ranges_and_wildcards_rank_below_concrete_entries() {
        let text = parsed("text/plain;charset=utf-8");
        assert_eq!(match_entry(&text, "text/*"), Some(EntryMatch::RangeMatch));
        assert_eq!(match_entry(&text, "application/*"), None);
        assert_eq!(match_entry(&text, "*/*"), Some(EntryMatch::Wildcard));
        assert_eq!(match_entry(&text, "text/html"), None);
    }

    #[test]
    fn wildcard_incoming_types_never_match_any_entry_here() {
        for raw in ["*/*", "*/plain", "application/*"] {
            let incoming = parsed(raw);
            assert!(is_wildcard_incoming(&incoming));
            assert_eq!(match_entry(&incoming, "application/json"), None);
            assert_eq!(match_entry(&incoming, "*/*"), None);
        }
        assert!(!is_wildcard_incoming(&parsed("application/json")));
    }

    #[test]
    fn malformed_entry_keys_do_not_match() {
        let json = parsed("application/json");
        assert_eq!(match_entry(&json, ""), None);
        assert_eq!(match_entry(&json, "application"), None);
        assert_eq!(match_entry(&json, "/"), None);
    }
}
