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

/// Validates a user-supplied Rust path for `--types-path`: an optional
/// leading `::`, then `::`-separated segments where the first may be
/// `crate`/`self`/`super` and every other segment must be a (possibly raw)
/// identifier that is not a Rust keyword.
///
/// Deliberately small and deterministic (no parser dependency): it exists to
/// catch obvious mistakes — kebab-case package names, filesystem separators,
/// dangling separators — before they become confusing compile errors.
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
    let body = trimmed.strip_prefix("::").unwrap_or(trimmed);
    let segments: Vec<&str> = body.split("::").collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(format!(
            "`{trimmed}` has an empty segment (dangling or repeated `::`)"
        ));
    }
    for (index, segment) in segments.iter().enumerate() {
        match *segment {
            "crate" | "self" | "super" if index == 0 => continue,
            "crate" | "self" | "super" => {
                return Err(format!(
                    "`{segment}` may only appear as the FIRST segment of \
                     `{trimmed}`"
                ))
            }
            other => {
                let (raw, name) = match other.strip_prefix("r#") {
                    Some(name) => (true, name),
                    None => (false, other),
                };
                if !is_identifier(name) {
                    return Err(format!(
                        "`{other}` is not a valid Rust identifier in `{trimmed}`"
                    ));
                }
                if !raw && is_keyword(name) {
                    return Err(format!(
                        "`{name}` is a Rust keyword and cannot be used as a \
                         path segment in `{trimmed}`"
                    ));
                }
            }
        }
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
fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "break"
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
            "::absolutecrate",
            "company_api::v2",
            "r#type",
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
            "foo::type",
            "crate::self",
            "self::crate",
            "foo bar",
        ] {
            assert!(
                validate_rust_path(path).is_err(),
                "`{path}` must be rejected"
            );
        }
    }

    #[test]
    fn codegen_config_defaults_to_sibling() {
        let config = CodegenConfig::default();
        assert_eq!(config.types_location, TypesLocation::Sibling);
    }
}
