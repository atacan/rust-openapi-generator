//! Structured diagnostics for document loading and normalization.
//!
//! Diagnostics are produced in deterministic document-traversal order
//! (reproducibility requirement of main spec §50). `DocumentPath` renders a
//! location inside the source document as an RFC 6901 JSON-pointer fragment,
//! e.g. `#/paths/~1widgets/get/responses/200` (`~0`/`~1` escaping).

use std::fmt;

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Non-fatal: recorded for the generator, loading continues.
    Warning,
    /// Fatal when surfaced through [`Diagnostics::into_result`].
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

/// One step inside a [`DocumentPath`]: an object key or an array index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentPathSegment {
    Key(String),
    Index(usize),
}

/// Location inside a source document, rendered as an RFC 6901 pointer
/// fragment with `~0`/`~1` escaping (companion §3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentPath(Vec<DocumentPathSegment>);

impl DocumentPath {
    /// Path of the document root (`#`).
    #[must_use]
    pub fn root() -> Self {
        Self(Vec::new())
    }

    /// Appends an object-key segment.
    pub fn push_key(&mut self, key: impl Into<String>) {
        self.0.push(DocumentPathSegment::Key(key.into()));
    }

    /// Appends an array-index segment.
    pub fn push_index(&mut self, index: usize) {
        self.0.push(DocumentPathSegment::Index(index));
    }

    /// Returns a copy extended by an object-key segment.
    #[must_use]
    pub fn key(&self, key: impl Into<String>) -> Self {
        let mut next = self.clone();
        next.push_key(key);
        next
    }

    /// Returns a copy extended by an array-index segment.
    #[must_use]
    pub fn index(&self, index: usize) -> Self {
        let mut next = self.clone();
        next.push_index(index);
        next
    }

    /// True when the path points at the document root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Segments in traversal order.
    #[must_use]
    pub fn segments(&self) -> &[DocumentPathSegment] {
        &self.0
    }
}

impl fmt::Display for DocumentPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("#")?;
        for segment in &self.0 {
            f.write_str("/")?;
            match segment {
                DocumentPathSegment::Key(key) => f.write_str(&escape_pointer_token(key))?,
                DocumentPathSegment::Index(index) => write!(f, "{index}")?,
            }
        }
        Ok(())
    }
}

/// RFC 6901 token escaping: `~` becomes `~0`, `/` becomes `~1`.
#[must_use]
pub fn escape_pointer_token(token: &str) -> String {
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

/// A single diagnostic record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub path: DocumentPath,
    /// Stable machine-readable identifier, e.g. `"ref_remote_url"`.
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {} [{}]: {}",
            self.severity, self.path, self.code, self.message
        )
    }
}

/// Ordered diagnostic collector; order is document traversal order.
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    entries: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a diagnostic preserving traversal order.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.entries.push(diagnostic);
    }

    /// Appends an error-severity diagnostic.
    pub fn error(&mut self, path: DocumentPath, code: &'static str, message: impl Into<String>) {
        self.push(Diagnostic {
            severity: Severity::Error,
            path,
            code,
            message: message.into(),
        });
    }

    /// Appends a warning-severity diagnostic.
    pub fn warning(&mut self, path: DocumentPath, code: &'static str, message: impl Into<String>) {
        self.push(Diagnostic {
            severity: Severity::Warning,
            path,
            code,
            message: message.into(),
        });
    }

    /// All diagnostics in traversal order.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.entries.iter()
    }

    /// Number of collected diagnostics.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when at least one error-severity diagnostic was recorded.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.entries.iter().any(|d| d.severity == Severity::Error)
    }

    /// Yields the value when no errors were recorded, otherwise every
    /// diagnostic (warnings included). Warnings on the success path are not
    /// carried by the value itself; inspect them before calling this.
    pub fn into_result<T>(self, value: T) -> Result<T, Vec<Diagnostic>> {
        if self.has_errors() {
            Err(self.entries)
        } else {
            Ok(value)
        }
    }

    /// Consumes the collector into the ordered diagnostic list.
    #[must_use]
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_round_trips_rfc6901_specials() {
        assert_eq!(escape_pointer_token("a/b"), "a~1b");
        assert_eq!(escape_pointer_token("a~b"), "a~0b");
        assert_eq!(escape_pointer_token("~01"), "~001");
        assert_eq!(escape_pointer_token("plain"), "plain");
    }

    #[test]
    fn document_path_renders_spec_example() {
        let mut path = DocumentPath::root();
        path.push_key("paths");
        path.push_key("/widgets");
        path.push_key("get");
        path.push_key("responses");
        path.push_key("200");
        assert_eq!(path.to_string(), "#/paths/~1widgets/get/responses/200");
    }

    #[test]
    fn root_path_is_hash_only() {
        assert_eq!(DocumentPath::root().to_string(), "#");
        assert!(DocumentPath::root().is_root());
    }

    #[test]
    fn diagnostics_preserve_push_order_and_error_detection() {
        let mut diags = Diagnostics::new();
        diags.warning(DocumentPath::root().key("a"), "w_code", "first");
        diags.error(DocumentPath::root().key("b"), "e_code", "second");
        diags.warning(DocumentPath::root().key("c"), "w2", "third");

        assert!(diags.has_errors());
        assert_eq!(diags.len(), 3);
        let codes: Vec<&str> = diags.iter().map(|d| d.code).collect();
        assert_eq!(codes, ["w_code", "e_code", "w2"]);

        let err = diags.into_result(42_u8).unwrap_err();
        assert_eq!(err.len(), 3);
        assert_eq!(err[0].code, "w_code");
    }

    #[test]
    fn into_result_ok_when_only_warnings() {
        let mut diags = Diagnostics::new();
        diags.warning(DocumentPath::root(), "w", "only warning");
        assert!(!diags.has_errors());
        assert_eq!(diags.into_result("value").unwrap(), "value");
    }
}
