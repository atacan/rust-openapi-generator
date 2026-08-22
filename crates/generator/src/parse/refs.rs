//! `$ref` string parsing and RFC 6901 JSON Pointer walking (companion §3,
//! DECISIONS.md D-§3).
//!
//! Only RFC 6901 pointer fragments against the local document or relative
//! external files are resolved. Plain-name/anchor fragments (`#Foo`,
//! `$anchor`, `$dynamicRef`) and non-empty `$id` rebasing are rejected by the
//! loader; percent-decoding of URI components is not performed in v1.

use serde_yaml::Value as Yaml;

/// A `$ref` string split at its first `#`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefParts {
    /// Everything before `#` (may be empty for same-document references);
    /// the whole string when no fragment is present.
    pub file: String,
    /// Fragment after `#` (possibly empty).
    pub fragment: String,
}

#[must_use]
pub(crate) fn split_ref(reference: &str) -> RefParts {
    match reference.split_once('#') {
        Some((file, fragment)) => RefParts {
            file: file.to_owned(),
            fragment: fragment.to_owned(),
        },
        None => RefParts {
            file: reference.to_owned(),
            fragment: String::new(),
        },
    }
}

/// Why a pointer fragment could not be tokenized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerParseError {
    /// Fragment does not start with `/` and is not empty: an anchor-style
    /// plain-name fragment (D-§3 rejects these).
    AnchorStyle,
    /// A `~` escape not followed by `0` or `1`.
    InvalidEscape,
}

/// Tokenizes a JSON Pointer fragment per RFC 6901, unescaping `~1` → `/`
/// then `~0` → `~`. The empty fragment addresses the whole document.
pub(crate) fn tokenize_pointer(fragment: &str) -> Result<Vec<String>, PointerParseError> {
    if fragment.is_empty() {
        return Ok(Vec::new());
    }
    let Some(body) = fragment.strip_prefix('/') else {
        return Err(PointerParseError::AnchorStyle);
    };
    let mut tokens = Vec::new();
    for raw in body.split('/') {
        tokens.push(unescape_token(raw).ok_or(PointerParseError::InvalidEscape)?);
    }
    Ok(tokens)
}

fn unescape_token(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            _ => return None,
        }
    }
    Some(out)
}

/// Escapes a token for display/memo keys: `~` → `~0`, `/` → `~1`.
#[must_use]
pub(crate) fn escape_pointer_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for ch in token.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            other => out.push(other),
        }
    }
    out
}

/// Canonical memo/stack key for a token sequence.
#[must_use]
pub(crate) fn canonical_pointer_key(tokens: &[String]) -> String {
    if tokens.is_empty() {
        String::new()
    } else {
        let mut key = String::new();
        for token in tokens {
            key.push('/');
            key.push_str(&escape_pointer_token(token));
        }
        key
    }
}

/// Renders tokens back into a displayable fragment.
#[must_use]
pub(crate) fn display_pointer(tokens: &[String]) -> String {
    if tokens.is_empty() {
        "#".to_owned()
    } else {
        format!("#{}", canonical_pointer_key(tokens))
    }
}

/// Walks a parsed document following RFC 6901 tokens; `None` when any step
/// is missing (unknown key, out-of-bounds index, non-container node).
#[must_use]
pub(crate) fn walk_pointer<'a>(root: &'a Yaml, tokens: &[String]) -> Option<&'a Yaml> {
    let mut current = root;
    for token in tokens {
        current = match current {
            Yaml::Mapping(mapping) => mapping.get(Yaml::String(token.clone()))?,
            Yaml::Sequence(sequence) => {
                let index: usize = token.parse().ok()?;
                sequence.get(index)?
            }
            _ => return None,
        };
    }
    Some(current)
}

/// Detects a URI scheme prefix on a reference's file part. Returns the
/// scheme (lowercased) when the part before the first `:` is a plausible
/// scheme and precedes any `/`; e.g. `"https://…"` → `"https"`.
#[must_use]
pub(crate) fn detect_scheme(file_part: &str) -> Option<String> {
    let colon = file_part.find(':')?;
    let candidate = &file_part[..colon];
    if candidate.is_empty()
        || !candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    {
        return None;
    }
    // Avoid treating Windows drive letters as schemes ("C:/x.yaml").
    if candidate.len() == 1 && file_part[colon + 1..].starts_with('/') {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value as Y;

    #[test]
    fn tokenize_round_trips_escaping() {
        let tokens = tokenize_pointer("/components/schemas/Foo~1Bar").unwrap();
        assert_eq!(tokens, ["components", "schemas", "Foo/Bar"]);
        assert_eq!(
            canonical_pointer_key(&tokens),
            "/components/schemas/Foo~1Bar"
        );

        let tilde = tokenize_pointer("/a~0b").unwrap();
        assert_eq!(tilde, ["a~b"]);
        assert_eq!(canonical_pointer_key(&tilde), "/a~0b");
    }

    #[test]
    fn tokenize_rejects_anchor_style_and_bad_escapes() {
        assert_eq!(
            tokenize_pointer("MyAnchor"),
            Err(PointerParseError::AnchorStyle)
        );
        assert_eq!(
            tokenize_pointer("/a~2b"),
            Err(PointerParseError::InvalidEscape)
        );
        assert_eq!(
            tokenize_pointer("/ab~"),
            Err(PointerParseError::InvalidEscape)
        );
    }

    #[test]
    fn empty_fragment_addresses_whole_document() {
        assert_eq!(tokenize_pointer("").unwrap(), Vec::<String>::new());
        assert_eq!(display_pointer(&[]), "#");
    }

    #[test]
    fn walk_traverses_maps_sequences_and_escapes() {
        let doc = Y::Mapping({
            let mut m = serde_yaml::Mapping::new();
            m.insert(
                Y::String("paths".into()),
                Y::Mapping({
                    let mut inner = serde_yaml::Mapping::new();
                    inner.insert(Y::String("a/b".into()), Y::String("found".into()));
                    inner
                }),
            );
            m.insert(
                Y::String("list".into()),
                Y::Sequence(vec![Y::Null, Y::Bool(true)]),
            );
            m
        });
        assert_eq!(
            walk_pointer(&doc, &["paths".into(), "a/b".into()]).unwrap(),
            &Y::String("found".into())
        );
        assert_eq!(
            walk_pointer(&doc, &["list".into(), "1".into()]).unwrap(),
            &Y::Bool(true)
        );
        assert!(walk_pointer(&doc, &["missing".into()]).is_none());
        assert!(walk_pointer(&doc, &["list".into(), "9".into()]).is_none());
        assert!(walk_pointer(&doc, &["list".into(), "x".into()]).is_none());
        assert_eq!(walk_pointer(&doc, &[]), Some(&doc));
    }

    #[test]
    fn split_ref_handles_all_shapes() {
        let parts = split_ref("#/components/schemas/A");
        assert_eq!(
            (parts.file.as_str(), parts.fragment.as_str()),
            ("", "/components/schemas/A")
        );
        let parts = split_ref("common.yaml#/components/schemas/A");
        assert_eq!(
            (parts.file.as_str(), parts.fragment.as_str()),
            ("common.yaml", "/components/schemas/A")
        );
        let parts = split_ref("common.yaml");
        assert_eq!(
            (parts.file.as_str(), parts.fragment.as_str()),
            ("common.yaml", "")
        );
    }

    #[test]
    fn scheme_detection_names_remote_and_custom_schemes() {
        assert_eq!(
            detect_scheme("https://example.com/x.yaml").as_deref(),
            Some("https")
        );
        assert_eq!(
            detect_scheme("http://example.com/x.yaml").as_deref(),
            Some("http")
        );
        assert_eq!(detect_scheme("ftp://host/f.yaml").as_deref(), Some("ftp"));
        assert_eq!(detect_scheme("../common/types.yaml"), None);
        assert_eq!(detect_scheme("types.yaml"), None);
        assert_eq!(detect_scheme("./local.yaml"), None);
    }

    #[test]
    fn windows_drive_letter_is_not_a_scheme() {
        assert_eq!(detect_scheme("C:/docs/openapi.yaml"), None);
    }
}
