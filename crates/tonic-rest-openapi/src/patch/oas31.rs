//! `OpenAPI` 3.0 → 3.1 structural transforms.
//!
//! - Version bump: `openapi: "3.0.3"` → `"3.1.0"`
//! - Nullable conversion: `nullable: true` → `type: [original, "null"]`
//! - Line ending normalization: CRLF → LF

use serde_yaml_ng::Value;

/// Set `openapi: "3.1.0"`.
pub fn upgrade_version(doc: &mut Value) {
    if let Value::Mapping(map) = doc {
        map.insert(
            Value::String("openapi".to_string()),
            Value::String("3.1.0".to_string()),
        );
    }
}

/// Convert `nullable: true` → `type: [original, "null"]` (JSON Schema 2020-12).
/// Remove `nullable: false` (no-op in 3.1).
pub fn convert_nullable(value: &mut Value) {
    match value {
        Value::Mapping(map) => {
            let nullable_key = Value::String("nullable".to_string());
            let type_key = Value::String("type".to_string());

            let is_nullable = map
                .get(&nullable_key)
                .is_some_and(|v| *v == Value::Bool(true));

            if map.contains_key(&nullable_key) {
                if is_nullable {
                    if let Some(type_val) = map.get(&type_key).cloned() {
                        map.insert(
                            type_key,
                            Value::Sequence(vec![type_val, Value::String("null".to_string())]),
                        );
                    }
                }
                map.remove(&nullable_key);
            }

            for (_, v) in map.iter_mut() {
                convert_nullable(v);
            }
        }
        Value::Sequence(seq) => {
            for item in seq.iter_mut() {
                convert_nullable(item);
            }
        }
        _ => {}
    }
}

/// Normalize CRLF → LF in all string values within the YAML document.
pub fn normalize_line_endings(value: &mut Value) {
    match value {
        Value::String(s) => {
            if s.contains("\r\n") {
                *s = s.replace("\r\n", "\n");
            }
        }
        Value::Mapping(map) => {
            for (_, v) in map.iter_mut() {
                normalize_line_endings(v);
            }
        }
        Value::Sequence(seq) => {
            for item in seq.iter_mut() {
                normalize_line_endings(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_version_sets_3_1() {
        let mut doc: Value = serde_yaml_ng::from_str("openapi: '3.0.3'\ninfo: {}").unwrap();
        upgrade_version(&mut doc);
        let version = doc
            .as_mapping()
            .unwrap()
            .get("openapi")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(version, "3.1.0");
    }

    #[test]
    fn convert_nullable_true_to_type_array() {
        let mut doc: Value = serde_yaml_ng::from_str("type: string\nnullable: true").unwrap();
        convert_nullable(&mut doc);

        let map = doc.as_mapping().unwrap();
        assert!(!map.contains_key(Value::String("nullable".to_string())));
        let type_val = map.get("type").unwrap();
        let seq = type_val.as_sequence().unwrap();
        assert_eq!(seq.len(), 2);
        assert_eq!(seq[0].as_str().unwrap(), "string");
        assert_eq!(seq[1].as_str().unwrap(), "null");
    }

    #[test]
    fn convert_nullable_false_removed() {
        let mut doc: Value = serde_yaml_ng::from_str("type: string\nnullable: false").unwrap();
        convert_nullable(&mut doc);

        let map = doc.as_mapping().unwrap();
        assert!(!map.contains_key(Value::String("nullable".to_string())));
        assert_eq!(map.get("type").unwrap().as_str().unwrap(), "string");
    }

    #[test]
    fn normalize_crlf_to_lf() {
        let mut doc = Value::String("line1\r\nline2\r\n".to_string());
        normalize_line_endings(&mut doc);
        assert_eq!(doc.as_str().unwrap(), "line1\nline2\n");
    }
}
