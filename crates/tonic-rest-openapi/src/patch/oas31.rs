//! `OpenAPI` 3.0 → 3.1 structural transforms.
//!
//! - Version bump: `openapi: "3.0.3"` → `"3.1.0"`
//! - Nullable conversion: `nullable: true` → `type: [original, "null"]`
//! - Server/info injection
//! - Line ending normalization: CRLF → LF

use serde_yaml_ng::Value;

use crate::config::{InfoOverrides, ServerEntry};

use super::helpers::val_s;

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

/// Inject `servers` block and enrich `info` with contact, license, and external docs.
///
/// If no servers are configured, a default `http://localhost:8080` entry is added.
/// Info overrides are merged into the existing `info` block without replacing
/// fields already present (e.g., `title` and `version` from gnostic).
pub fn inject_servers_and_info(doc: &mut Value, servers: &[ServerEntry], info: &InfoOverrides) {
    let Some(root) = doc.as_mapping_mut() else {
        return;
    };

    // --- servers ---
    let server_entries: Vec<Value> = if servers.is_empty() {
        let mut entry = serde_yaml_ng::Mapping::new();
        entry.insert(val_s("url"), val_s("http://localhost:8080"));
        entry.insert(val_s("description"), val_s("Local development"));
        vec![Value::Mapping(entry)]
    } else {
        servers
            .iter()
            .map(|s| {
                let mut entry = serde_yaml_ng::Mapping::new();
                entry.insert(val_s("url"), val_s(&s.url));
                if let Some(desc) = &s.description {
                    entry.insert(val_s("description"), val_s(desc));
                }
                Value::Mapping(entry)
            })
            .collect()
    };
    root.insert(val_s("servers"), Value::Sequence(server_entries));

    // --- info enrichment ---
    if !root.contains_key("info") {
        root.insert(val_s("info"), Value::Mapping(serde_yaml_ng::Mapping::new()));
    }
    let Some(info_map) = root.get_mut("info").and_then(Value::as_mapping_mut) else {
        return;
    };

    if let Some(tos) = &info.terms_of_service {
        info_map.insert(val_s("termsOfService"), val_s(tos));
    }

    if let Some(contact) = &info.contact {
        let mut c = serde_yaml_ng::Mapping::new();
        if let Some(name) = &contact.name {
            c.insert(val_s("name"), val_s(name));
        }
        if let Some(email) = &contact.email {
            c.insert(val_s("email"), val_s(email));
        }
        if let Some(url) = &contact.url {
            c.insert(val_s("url"), val_s(url));
        }
        info_map.insert(val_s("contact"), Value::Mapping(c));
    }

    if let Some(license) = &info.license {
        let mut l = serde_yaml_ng::Mapping::new();
        l.insert(val_s("name"), val_s(&license.name));
        if let Some(url) = &license.url {
            l.insert(val_s("url"), val_s(url));
        }
        info_map.insert(val_s("license"), Value::Mapping(l));
    }

    if let Some(ext) = &info.external_docs {
        let mut ed = serde_yaml_ng::Mapping::new();
        ed.insert(val_s("url"), val_s(&ext.url));
        if let Some(desc) = &ext.description {
            ed.insert(val_s("description"), val_s(desc));
        }
        root.insert(val_s("externalDocs"), Value::Mapping(ed));
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

    #[test]
    fn inject_default_servers() {
        let mut doc: Value = serde_yaml_ng::from_str("info:\n  title: Test\npaths: {}").unwrap();
        inject_servers_and_info(&mut doc, &[], &InfoOverrides::default());

        let servers = doc["servers"].as_sequence().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["url"].as_str().unwrap(), "http://localhost:8080");
        assert_eq!(
            servers[0]["description"].as_str().unwrap(),
            "Local development"
        );
    }

    #[test]
    fn inject_custom_servers_and_info() {
        use crate::config::{ContactInfo, LicenseInfo};

        let mut doc: Value = serde_yaml_ng::from_str("info:\n  title: Test\npaths: {}").unwrap();
        let servers = vec![
            ServerEntry {
                url: "https://api.example.com".to_string(),
                description: Some("Production".to_string()),
            },
            ServerEntry {
                url: "http://localhost:8080".to_string(),
                description: Some("Development".to_string()),
            },
        ];
        let info = InfoOverrides {
            contact: Some(ContactInfo {
                name: Some("API Team".to_string()),
                email: Some("api@example.com".to_string()),
                url: None,
            }),
            license: Some(LicenseInfo {
                name: "MIT".to_string(),
                url: Some("https://opensource.org/licenses/MIT".to_string()),
            }),
            external_docs: None,
            terms_of_service: Some("https://example.com/tos".to_string()),
        };

        inject_servers_and_info(&mut doc, &servers, &info);

        let srv = doc["servers"].as_sequence().unwrap();
        assert_eq!(srv.len(), 2);
        assert_eq!(srv[0]["url"].as_str().unwrap(), "https://api.example.com");

        let info_map = doc["info"].as_mapping().unwrap();
        assert_eq!(
            info_map.get("termsOfService").unwrap().as_str().unwrap(),
            "https://example.com/tos"
        );
        assert_eq!(info_map["contact"]["name"].as_str().unwrap(), "API Team");
        assert_eq!(info_map["license"]["name"].as_str().unwrap(), "MIT");

        // title should be preserved
        assert_eq!(info_map.get("title").unwrap().as_str().unwrap(), "Test");
    }
}
