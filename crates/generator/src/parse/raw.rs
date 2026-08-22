//! Generic ordered YAML tree access.
//!
//! Documents are parsed into `serde_yaml::Value`, whose `Mapping` is an
//! insertion-ordered map; traversal therefore follows document declaration
//! order, which is the determinism requirement for diagnostics and interning
//! (main spec §50). Helpers here also convert YAML values to
//! `serde_json::Value` for IR fields that store raw JSON (`default`,
//! `examples`, enum constants).

use serde_yaml::{Mapping, Value as Yaml};

use crate::ir::document::OpenApiVersion;

/// Returns the value as a mapping, if it is one.
#[must_use]
pub(crate) fn as_mapping(value: &Yaml) -> Option<&Mapping> {
    value.as_mapping()
}

/// Fetches a string-keyed entry from a mapping.
#[must_use]
pub(crate) fn mapping_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Yaml> {
    mapping.get(Yaml::String(key.to_owned()))
}

/// True when the mapping has the given key.
#[must_use]
pub(crate) fn mapping_has(mapping: &Mapping, key: &str) -> bool {
    mapping.contains_key(Yaml::String(key.to_owned()))
}

/// True when the value is a mapping containing `$ref`.
#[must_use]
pub(crate) fn is_ref_mapping(value: &Yaml) -> bool {
    as_mapping(value).is_some_and(|m| mapping_has(m, "$ref"))
}

/// Extracts a string-valued keyword.
#[must_use]
pub(crate) fn string_field<'a>(value: &'a Yaml, key: &str) -> Option<&'a str> {
    as_mapping(value)
        .and_then(|m| mapping_get(m, key))
        .and_then(Yaml::as_str)
}

/// Renders a non-string YAML scalar deterministically (used for JSON object
/// keys when a YAML document uses numeric/boolean keys).
#[must_use]
pub(crate) fn stringify_scalar(value: &Yaml) -> String {
    match value {
        Yaml::Null => "null".to_owned(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(n) => n.to_string(),
        other => format!("{other:?}").to_owned(),
    }
}

/// Converts a YAML value into a JSON value for IR storage.
///
/// YAML tags are unwrapped (the tagged inner value is used); non-finite
/// floats are rejected because JSON cannot represent them; non-string object
/// keys are rendered with [`stringify_scalar`].
pub(crate) fn yaml_to_json(value: &Yaml) -> Result<serde_json::Value, String> {
    Ok(match value {
        Yaml::Null => serde_json::Value::Null,
        Yaml::Bool(b) => serde_json::Value::Bool(*b),
        Yaml::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::from(i)
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::from(u)
            } else {
                let f = n
                    .as_f64()
                    .ok_or_else(|| format!("unrepresentable number {n}"))?;
                if !f.is_finite() {
                    return Err(format!("non-finite number {n}"));
                }
                serde_json::Value::from(f)
            }
        }
        Yaml::String(s) => serde_json::Value::String(s.clone()),
        Yaml::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(yaml_to_json(item)?);
            }
            serde_json::Value::Array(out)
        }
        Yaml::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = k
                    .as_str()
                    .map_or_else(|| stringify_scalar(k), ToOwned::to_owned);
                obj.insert(key, yaml_to_json(v)?);
            }
            serde_json::Value::Object(obj)
        }
        Yaml::Tagged(tagged) => yaml_to_json(&tagged.value)?,
    })
}

/// Reads the root `openapi` field and maps it onto a supported release
/// family. Anything else is an error listing the supported versions
/// (companion §2).
pub(crate) fn detect_version(root: &Yaml) -> Result<(OpenApiVersion, String), String> {
    let Some(mapping) = root.as_mapping() else {
        return Err("document root must be a mapping".to_owned());
    };
    let Some(raw) = mapping_get(mapping, "openapi").and_then(Yaml::as_str) else {
        return Err("missing required string field `openapi`".to_owned());
    };
    let segments: Vec<&str> = raw.split('.').collect();
    let version = match segments.as_slice() {
        ["3", minor, ..] => match *minor {
            "0" => OpenApiVersion::V3_0,
            "1" => OpenApiVersion::V3_1,
            "2" => OpenApiVersion::V3_2,
            other => {
                return Err(format!(
                    "unsupported OpenAPI version `{raw}`; supported: 3.0.x, 3.1.x, 3.2.x \
                     (got minor `{other}`)"
                ))
            }
        },
        _ => {
            return Err(format!(
                "unsupported OpenAPI version `{raw}`; supported: 3.0.x, 3.1.x, 3.2.x"
            ))
        }
    };
    Ok((version, raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Yaml {
        serde_yaml::from_str(yaml).expect("test yaml parses")
    }

    #[test]
    fn version_detection_accepts_supported_releases() {
        for (raw, expected) in [
            ("3.0.3", OpenApiVersion::V3_0),
            ("3.0", OpenApiVersion::V3_0),
            ("3.1.0", OpenApiVersion::V3_1),
            ("3.1.1", OpenApiVersion::V3_1),
            ("3.2.0", OpenApiVersion::V3_2),
        ] {
            let doc = parse(&format!("openapi: \"{raw}\""));
            let (version, parsed_raw) = detect_version(&doc).unwrap();
            assert_eq!(version, expected, "{raw}");
            assert_eq!(parsed_raw, raw);
        }
    }

    #[test]
    fn version_detection_rejects_unsupported_releases() {
        for raw in ["2.0", "4.0.0", "3.9.9", "1.2"] {
            let doc = parse(&format!("openapi: \"{raw}\""));
            let err = detect_version(&doc).unwrap_err();
            assert!(err.contains("supported"), "{err}");
        }
        let missing = parse("info: {}");
        assert!(detect_version(&missing).is_err());
        let scalar = parse("- just\n- a list");
        assert!(detect_version(&scalar).is_err());
    }

    #[test]
    fn mapping_preserves_declaration_order() {
        let doc = parse("zebra: 1\nalpha: 2\nmid: 3");
        let m = as_mapping(&doc).unwrap();
        let keys: Vec<String> = m
            .iter()
            .map(|(k, _)| k.as_str().unwrap_or("?").to_owned())
            .collect();
        assert_eq!(keys, ["zebra", "alpha", "mid"]);
    }

    #[test]
    fn yaml_to_json_converts_scalars_sequences_and_tags() {
        let v = parse("a: [1, two]\nb: true");
        let json = yaml_to_json(&v).unwrap();
        assert_eq!(json, serde_json::json!({"a": [1, "two"], "b": true}));
        assert!(json["a"][0].is_i64());

        let tagged = parse("!Custom hello");
        let json = yaml_to_json(&tagged).unwrap();
        assert_eq!(json, serde_json::Value::String("hello".into()));
    }

    #[test]
    fn yaml_to_json_rejects_non_finite_numbers() {
        let v = parse("x: .inf");
        assert!(yaml_to_json(&v).is_err());
    }

    #[test]
    fn ref_detection_on_mappings_only() {
        assert!(is_ref_mapping(&parse("$ref: '#/x'")));
        assert!(!is_ref_mapping(&parse("$REF: '#/x'")));
        assert!(!is_ref_mapping(&parse("[1,2]")));
    }
}
