//! Deterministic naming pipeline (companion §10; DECISIONS.md D-§6/D-§10).
//!
//! - identifiers split on `_`, `-`, `.`, space, `$`, `/` and camelCase
//!   boundaries, preserving digits attached to preceding words
//!   (`getArtifactV2` → `get_artifact_v2`);
//! - types are `PascalCase`, methods/fields `snake_case`;
//! - Rust keywords (strict + reserved) and empty identifiers get a trailing
//!   `_` (never raw identifiers);
//! - collisions inside one assignment table get numeric suffixes `_2`,
//!   `_3`, … ordered by document position — the first occurrence keeps the
//!   clean name (never hash-based, main spec §50 reproducibility).

use std::collections::{BTreeMap, BTreeSet};

/// Characters that always separate identifier words (companion §10).
const SEPARATORS: &[char] = &['_', '-', '.', ' ', '$', '/'];

/// Strict and reserved Rust keywords across editions 2015–2024.
///
/// Primitive type names (`u8`, `str`, `bool`, …) are deliberately absent:
/// they are not keywords and must not be suffixed (companion §10).
const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "become",
    "box",
    "break",
    "const",
    "continue",
    "crate",
    "do",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "macro",
    "macro_rules",
    "match",
    "mod",
    "move",
    "mut",
    "override",
    "priv",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "unsafe",
    "unsized",
    "use",
    "virtual",
    "where",
    "while",
    "yield",
];

/// Casing style applied to a sanitized identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameStyle {
    /// Types: `PascalCase`.
    Pascal,
    /// Methods, fields, modules: `snake_case`.
    Snake,
}

/// Splits a raw document identifier into words.
///
/// Boundaries: separator characters, lower→upper camelCase edges, acronym
/// ends (`HTTPServer` → `HTTP` + `Server`), and digit→letter edges.
/// Digits attach to the preceding word (`V2` stays together).
#[must_use]
pub fn split_words(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if SEPARATORS.contains(&ch) {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        let prev = if index == 0 {
            None
        } else {
            Some(chars[index - 1])
        };
        let next = chars.get(index + 1).copied();
        let boundary = match prev {
            None => false,
            Some(p) => {
                let lower_or_digit_then_upper =
                    (p.is_ascii_lowercase() || p.is_ascii_digit()) && ch.is_ascii_uppercase();
                let acronym_end = p.is_ascii_uppercase()
                    && ch.is_ascii_uppercase()
                    && next.is_some_and(|n| n.is_ascii_lowercase());
                let digit_then_letter = p.is_ascii_digit() && ch.is_ascii_alphabetic();
                lower_or_digit_then_upper || acronym_end || digit_then_letter
            }
        };
        if boundary && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Applies keyword/empty/leading-digit rules to an already-cased string.
#[must_use]
pub fn sanitize_joined(joined: &str) -> String {
    if joined.is_empty() || KEYWORDS.contains(&joined) {
        return format!("{joined}_");
    }
    if joined.starts_with(|c: char| c.is_ascii_digit()) {
        return format!("_{joined}");
    }
    joined.to_owned()
}

/// Joins words into a `PascalCase` or `snake_case` identifier.
#[must_use]
pub fn join_words(words: &[String], style: NameStyle) -> String {
    let joined = match style {
        NameStyle::Snake => words
            .iter()
            .map(|w| w.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
        NameStyle::Pascal => words
            .iter()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => {
                        first.to_ascii_uppercase().to_string()
                            + &chars.as_str().to_ascii_lowercase()
                    }
                    None => String::new(),
                }
            })
            .collect::<String>(),
    };
    sanitize_joined(&joined)
}

/// Full pipeline for one raw identifier: split + case + keyword rules.
#[must_use]
pub fn ident(raw: &str, style: NameStyle) -> String {
    join_words(&split_words(raw), style)
}

/// Assigns collision-free names preserving request order: the first
/// occurrence keeps the clean name, later duplicates get `_2`, `_3`, …
/// ordered by document position (companion §10).
pub fn assign_unique(requests: &[String], style: NameStyle) -> Vec<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    requests
        .iter()
        .map(|raw| {
            let base = ident(raw, style);
            let mut candidate = base.clone();
            let mut counter = 1_u32;
            while used.contains(&candidate) {
                counter += 1;
                candidate = sanitize_joined(&format!("{base}_{counter}"));
            }
            used.insert(candidate.clone());
            candidate
        })
        .collect()
}

/// Assigned Rust names derived from one OpenAPI document (companion §10).
///
/// Every table is a pure function of document content plus declaration
/// order. Name-keyed tables use [`BTreeMap`]; order-sensitive tables are
/// vectors keyed by stable operation keys (`"get /widgets"`).
#[derive(Debug, Default, Clone)]
pub struct NameAssignments {
    /// Component schema name → Rust type name (`PascalCase`).
    pub schema_types: BTreeMap<String, String>,
    /// Operation key (`"get /widgets"`) → snake_case method name.
    pub operation_methods: Vec<(String, String)>,
    /// Operation key → response enum type name (e.g. `GetArtifactResponse`).
    pub response_enums: Vec<(String, String)>,
    /// Effective anonymous body arena id → generated `models.rs` type name
    /// (issue #11): inline composite request/response bodies that need a
    /// nominal type are named after their operation —
    /// `<Op>RequestBody` / `<Op>ResponseBody` — with the same numeric
    /// collision suffixes ordered by document position. The `ResponseBody`
    /// suffix (never the bare `Response`) keeps the issue #9 reservation:
    /// `<Operation>Response` stays owned by the generated response enum.
    pub synthetic_body_types: BTreeMap<u32, String>,
    /// Raw tag → snake_case module name (same sanitization rules).
    pub tag_modules: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_and_separator_boundaries() {
        assert_eq!(
            split_words("getArtifactV2"),
            ["get", "Artifact", "V2"].map(ToOwned::to_owned)
        );
        assert_eq!(ident("getArtifactV2", NameStyle::Snake), "get_artifact_v2");
        assert_eq!(ident("createWidget", NameStyle::Snake), "create_widget");
        assert_eq!(
            ident("GetArtifactResponse", NameStyle::Snake),
            "get_artifact_response"
        );
        assert_eq!(ident("X-Request-Id", NameStyle::Snake), "x_request_id");
        assert_eq!(ident("a.b$c/d e", NameStyle::Snake), "a_b_c_d_e");
        assert_eq!(ident("foo_bar", NameStyle::Snake), "foo_bar");
    }

    #[test]
    fn acronym_runs_split_before_title_word() {
        assert_eq!(ident("HTTPServer", NameStyle::Snake), "http_server");
        assert_eq!(ident("HTTPServer", NameStyle::Pascal), "HttpServer");
        assert_eq!(
            split_words("HTTPServer"),
            ["HTTP", "Server"].map(ToOwned::to_owned)
        );
    }

    #[test]
    fn digits_attach_to_preceding_words() {
        // The digit run stays attached to the preceding word; the letter
        // after a digit starts a new word.
        assert_eq!(
            split_words("artifact2x"),
            ["artifact2", "x"].map(ToOwned::to_owned)
        );
        assert_eq!(ident("artifact2x", NameStyle::Snake), "artifact2_x");
        // A digit after a letter stays attached: V2 is one word.
        assert_eq!(split_words("V2"), ["V2"].map(ToOwned::to_owned));
        assert_eq!(split_words("line1"), ["line1"].map(ToOwned::to_owned));
        assert_eq!(ident("getArtifactV2", NameStyle::Pascal), "GetArtifactV2");
    }

    #[test]
    fn keywords_and_empty_identifiers_get_trailing_underscore() {
        assert_eq!(ident("type", NameStyle::Snake), "type_");
        assert_eq!(ident("match", NameStyle::Pascal), "Match");
        assert_eq!(ident("self", NameStyle::Snake), "self_");
        assert_eq!(ident("Self", NameStyle::Pascal), "Self_");
        assert_eq!(ident("crate", NameStyle::Snake), "crate_");
        assert_eq!(ident("gen", NameStyle::Snake), "gen_");
        assert_eq!(ident("async", NameStyle::Snake), "async_");
        // Primitives are NOT keywords.
        assert_eq!(ident("u8", NameStyle::Snake), "u8");
        assert_eq!(ident("str", NameStyle::Snake), "str");
        assert_eq!(join_words(&[], NameStyle::Snake), "_");
        assert_eq!(ident("---", NameStyle::Snake), "_");
        // Leading digits are not valid Rust identifiers.
        assert_eq!(sanitize_joined("2fa"), "_2fa");
    }

    #[test]
    fn collisions_get_position_ordered_numeric_suffixes() {
        let requests = vec![
            "FooBar".to_owned(),
            "foo_bar".to_owned(),
            "unrelated".to_owned(),
        ];
        assert_eq!(
            assign_unique(&requests, NameStyle::Pascal),
            ["FooBar", "FooBar_2", "Unrelated"]
        );
        // Repeated collisions keep counting deterministically.
        let again = vec!["type".to_owned(), "type".to_owned()];
        assert_eq!(
            assign_unique(&again, NameStyle::Snake),
            ["type_", "type__2"]
        );
    }
}
