//! Shared-types location configuration for client/server emission
//! (DECISIONS.md D-impl-selective-artifacts).
//!
//! The generated `client.rs`/`server.rs` modules reference the shared schema
//! surface — `models.rs` plus its directional read/write views (`views.rs`,
//! companion §5) — through two import prefixes. When all artifacts are
//! generated into one crate those modules are SIBLINGS of the emitter's
//! module, so imports render as `super::models` / `super::views`
//! (D-impl-singlefile-layout). When a workspace splits the shared types into
//! their own crate (or module tree), the emitters instead reference the
//! EXTERNAL base path supplied by the user:
//!
//! ```text
//! TypesLocation::Sibling            External("api_types")
//!   models → super::models           models → api_types::models
//!   views  → super::views            views  → api_types::views
//! ```
//!
//! The emitters receive this decision explicitly ([`CodegenConfig`]) and
//! render the import lines themselves; output is never post-processed by
//! textual replacement. [`validate_rust_path`] keeps obviously invalid paths
//! out without pulling in a parser dependency (D-impl-codegen-emission).

/// Where the shared types surface (`models` + `views`) lives relative to the
/// emitted client/server modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypesLocation {
    /// Same-parent sibling modules: `super::models` / `super::views`. This is
    /// the default and preserves every existing byte of generated output.
    Sibling,
    /// An externally generated namespace base; the `models` and `views`
    /// modules live UNDER this Rust path (e.g. `api_types`,
    /// `crate::generated::types`, `company_api::v2`).
    External(String),
}

impl TypesLocation {
    /// Full import prefix for the shared models module
    /// (`super::models` or `<base>::models`).
    #[must_use]
    pub fn models_path(&self) -> String {
        match self {
            Self::Sibling => "super::models".to_owned(),
            Self::External(base) => format!("{base}::models"),
        }
    }

    /// Full import prefix for the directional views module
    /// (`super::views` or `<base>::views`).
    #[must_use]
    pub fn views_path(&self) -> String {
        match self {
            Self::Sibling => "super::views".to_owned(),
            Self::External(base) => format!("{base}::views"),
        }
    }

    /// Constructs an external location from a user-supplied Rust path,
    /// rejecting syntactically invalid input with a human-readable reason.
    ///
    /// # Errors
    ///
    /// Returns the validation reason when `base` is not a plain Rust
    /// module/crate path (see [`validate_rust_path`]).
    pub fn external(base: &str) -> Result<Self, String> {
        validate_rust_path(base)?;
        Ok(Self::External(base.to_owned()))
    }
}

/// Emission configuration shared by the configurable client/server entry
/// points ([`crate::codegen::client::generate_client_with_config`],
/// [`crate::codegen::server::generate_server_with_config`]).
///
/// The default routes to [`TypesLocation::Sibling`], so existing callers
/// (snapshot harnesses, the conformance build, committed examples) keep
/// byte-identical output without naming the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenConfig {
    /// Where the shared types surface lives.
    pub types_location: TypesLocation,
}

impl Default for CodegenConfig {
    fn default() -> Self {
        Self {
            types_location: TypesLocation::Sibling,
        }
    }
}

/// Validates a user-supplied Rust path for `--types-path`.
///
/// The grammar mirrors the paths generated `use` statements can actually
/// spell (main spec §3), without a parser dependency:
///
/// - an optional LEADING `::` marks an external/absolute path; such a path
///   must not contain the path keywords at all (`::crate::foo` is not Rust);
/// - otherwise the FIRST segment may be the relative qualifier `crate`,
///   `self`, or `super`;
/// - after `self` or `super`, further `super`s are permitted
///   (`super::super::shared`, `self::super::shared`); every other keyword
///   position is rejected (`crate::self`, `foo::super`);
/// - every remaining segment must be a plain identifier that is not a Rust
///   keyword, or a RAW identifier (`r#type`) — and raw identifiers cannot
///   escape `crate`, `self`, `super`, or `Self`.
///
/// Deliberately small and deterministic: it exists to catch obvious mistakes
/// — kebab-case package names, filesystem separators, dangling separators,
/// keyword abuse — before they become confusing compile errors.
///
/// # Errors
///
/// Returns a human-readable reason when `path` cannot appear in generated
/// `use` statements.
pub fn validate_rust_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("the path is empty".to_owned());
    }
    let absolute = trimmed.starts_with("::");
    let body = trimmed.strip_prefix("::").unwrap_or(trimmed);
    let segments: Vec<&str> = body.split("::").collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(format!(
            "`{trimmed}` has an empty segment (dangling or repeated `::`)"
        ));
    }
    // Whether the PREVIOUS segment permits a following `super` (Rust allows
    // exactly this one keyword chain: `self::super`, `super::super`).
    let mut super_chain_allowed = false;
    for (index, segment) in segments.iter().enumerate() {
        let (raw, name) = match segment.strip_prefix("r#") {
            Some(name) => (true, name),
            None => (false, *segment),
        };
        if !raw && matches!(name, "crate" | "self" | "super") {
            if absolute {
                return Err(format!(
                    "`{trimmed}` is an external/absolute path and cannot \
                     contain the path keyword `{name}`"
                ));
            }
            let legal = match name {
                "crate" | "self" => index == 0,
                _ => index == 0 || super_chain_allowed,
            };
            if !legal {
                return Err(match name {
                    "super" => format!(
                        "`super` inside `{trimmed}` may follow only `self` or \
                         another `super`"
                    ),
                    _ => format!(
                        "`{name}` may appear only as the FIRST segment of \
                         `{trimmed}`"
                    ),
                });
            }
            super_chain_allowed = matches!(name, "self" | "super");
            continue;
        }
        if raw && matches!(name, "crate" | "self" | "super" | "Self") {
            return Err(format!(
                "`{segment}` cannot escape the reserved keyword `{name}` \
                 with a raw identifier in `{trimmed}`"
            ));
        }
        if !is_identifier(name) {
            return Err(format!(
                "`{segment}` is not a valid Rust identifier in `{trimmed}`"
            ));
        }
        if !raw && is_keyword(name) {
            return Err(format!(
                "`{name}` is a Rust keyword and cannot be used as a path \
                 segment in `{trimmed}`"
            ));
        }
        super_chain_allowed = false;
    }
    Ok(())
}

/// ASCII identifier shape: starts with `_` or a letter, continues with
/// letters/digits/underscores.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|rest| rest.is_ascii_alphanumeric() || rest == '_')
}

/// Strict + reserved keywords that cannot appear as a plain path segment.
/// Includes the path keywords themselves so ABSOLUTE paths (`::crate`) and
/// raw-escape checks reject them uniformly; relative positions are handled
/// by the grammar above before this list is consulted.
fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "Self"
            | "as"
            | "break"
            | "const"
            | "continue"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "gen"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_paths_render_super() {
        let location = TypesLocation::Sibling;
        assert_eq!(location.models_path(), "super::models");
        assert_eq!(location.views_path(), "super::views");
    }

    #[test]
    fn external_paths_render_base_modules() {
        let location = TypesLocation::external("api_types").unwrap();
        assert_eq!(location.models_path(), "api_types::models");
        assert_eq!(location.views_path(), "api_types::views");

        let nested = TypesLocation::external("crate::generated::types").unwrap();
        assert_eq!(nested.models_path(), "crate::generated::types::models");

        let leading = TypesLocation::external("::company::api").unwrap();
        assert_eq!(leading.views_path(), "::company::api::views");
    }

    #[test]
    fn validator_accepts_normal_paths() {
        for path in [
            "api_types",
            "crate::types",
            "crate::generated::types",
            "self::types",
            "super::shared",
            "super::super::shared",
            "self::super::shared",
            "::absolutecrate",
            "::api_types",
            "::company_api::v2",
            "company_api::v2",
            "r#type",
            "foo::r#type",
            "crate",
            "A1_b2::x",
        ] {
            assert!(validate_rust_path(path).is_ok(), "{path} must validate");
        }
    }

    #[test]
    fn validator_rejects_obviously_invalid_paths() {
        for path in [
            "",
            "   ",
            "api-types",
            "foo/bar",
            "foo::",
            "::foo::",
            "a::::b",
            "1abc",
            "foo bar",
            "foo::type",
            "Self",
            ":",
            ":foo",
            "foo:bar",
            "r#",
            "foo::r#",
            "r#1abc",
        ] {
            assert!(
                validate_rust_path(path).is_err(),
                "`{path}` must be rejected"
            );
        }
    }

    #[test]
    fn validator_rejects_misplaced_path_keywords() {
        // `crate`/`self` only ever start a relative path; `super` may chain
        // only after `self`/`super`.
        for path in [
            "crate::self",
            "crate::crate",
            "crate::super",
            "self::crate",
            "self::self",
            "super::crate",
            "super::self",
            "shared::super::types",
            "foo::super::bar",
            "foo::crate",
            "foo::self",
            "crate::types::super",
        ] {
            assert!(
                validate_rust_path(path).is_err(),
                "`{path}` must be rejected"
            );
        }
    }

    #[test]
    fn validator_rejects_keywords_after_leading_colons() {
        for path in ["::crate::foo", "::self::foo", "::super::foo", "::crate"] {
            assert!(
                validate_rust_path(path).is_err(),
                "`{path}` must be rejected"
            );
        }
    }

    #[test]
    fn validator_rejects_raw_keyword_escapes() {
        for path in ["r#self", "r#super", "r#crate", "r#Self", "foo::r#self"] {
            assert!(
                validate_rust_path(path).is_err(),
                "`{path}` must be rejected"
            );
        }
    }

    #[test]
    fn validator_accepts_raw_non_keyword_identifiers_in_every_position() {
        for path in ["r#type", "foo::r#type", "crate::r#loop", "::r#async"] {
            assert!(validate_rust_path(path).is_ok(), "{path} must validate");
        }
    }

    #[test]
    fn codegen_config_defaults_to_sibling() {
        let config = CodegenConfig::default();
        assert_eq!(config.types_location, TypesLocation::Sibling);
    }
}
