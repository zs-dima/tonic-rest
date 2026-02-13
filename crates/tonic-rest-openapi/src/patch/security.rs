//! Security scheme transforms.
//!
//! Adds Bearer JWT security scheme and per-operation overrides
//! for public (no-auth) endpoints.

use serde_yaml_ng::Value;

use super::helpers::{for_each_operation, val_s};

/// Add `securitySchemes` and per-operation `security` requirements.
///
/// Sets Bearer JWT as the global default, then overrides public endpoints
/// with empty security (`security: []`).
///
/// Merges into existing `securitySchemes` rather than replacing, so
/// user-defined schemes (e.g., `apiKey`, `oauth2`) are preserved.
pub fn add_security_schemes(
    doc: &mut Value,
    public_ops: &[String],
    bearer_description: Option<&str>,
) {
    let description = bearer_description.unwrap_or("Bearer authentication token");

    // Add securitySchemes to components
    let components = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("components"))
        .and_then(Value::as_mapping_mut);

    if let Some(components) = components {
        // Build the bearerAuth scheme value programmatically to avoid
        // YAML injection from user-provided description text.
        let mut bearer_scheme = serde_yaml_ng::Mapping::new();
        bearer_scheme.insert(val_s("type"), val_s("http"));
        bearer_scheme.insert(val_s("scheme"), val_s("bearer"));
        bearer_scheme.insert(val_s("bearerFormat"), val_s("JWT"));
        bearer_scheme.insert(val_s("description"), val_s(description));

        // Merge into existing securitySchemes (preserve other schemes)
        let schemes = components
            .entry(val_s("securitySchemes"))
            .or_insert_with(|| Value::Mapping(serde_yaml_ng::Mapping::new()));

        if let Some(schemes_map) = schemes.as_mapping_mut() {
            schemes_map.insert(val_s("bearerAuth"), Value::Mapping(bearer_scheme));
        }
    }

    // Append bearerAuth to top-level security (preserve existing requirements)
    if let Some(root) = doc.as_mapping_mut() {
        let bearer_requirement: Value = serde_yaml_ng::from_str(
            r"
- bearerAuth: []
",
        )
        .expect("static YAML must parse");

        let security = root
            .entry(val_s("security"))
            .or_insert_with(|| Value::Sequence(vec![]));

        if let Some(seq) = security.as_sequence_mut() {
            // Only add if not already present
            let already_has_bearer = seq.iter().any(|item| {
                item.as_mapping()
                    .is_some_and(|m| m.contains_key("bearerAuth"))
            });
            if !already_has_bearer {
                if let Some(bearer_seq) = bearer_requirement.as_sequence() {
                    seq.extend(bearer_seq.iter().cloned());
                }
            }
        }
    }

    // Override public operations with empty security
    for_each_operation(doc, |_path, _method, op_map| {
        let op_id = op_map
            .get(Value::String("operationId".to_string()))
            .and_then(Value::as_str)
            .unwrap_or_default();

        if public_ops.iter().any(|id| id == op_id) {
            op_map.insert(
                Value::String("security".to_string()),
                Value::Sequence(vec![]),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_bearer_auth_scheme() {
        let yaml = r"
components:
  schemas: {}
paths:
  /v1/auth/login:
    post:
      operationId: AuthService_Authenticate
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        add_security_schemes(&mut doc, &["AuthService_Authenticate".to_string()], None);

        // Global security should be set
        assert!(doc["security"].as_sequence().is_some());

        // Security schemes should exist
        assert!(
            doc["components"]["securitySchemes"]["bearerAuth"]
                .as_mapping()
                .is_some()
        );

        // Public endpoint should have empty security
        let op = doc["paths"]["/v1/auth/login"]["post"].as_mapping().unwrap();
        let security = op.get("security").unwrap().as_sequence().unwrap();
        assert!(security.is_empty());
    }

    #[test]
    fn preserves_existing_security_schemes() {
        let yaml = r"
components:
  schemas: {}
  securitySchemes:
    apiKey:
      type: apiKey
      name: X-API-Key
      in: header
paths: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        add_security_schemes(&mut doc, &[], None);

        let schemes = doc["components"]["securitySchemes"].as_mapping().unwrap();

        // Both the original apiKey and new bearerAuth should exist
        assert!(
            schemes.contains_key("apiKey"),
            "existing apiKey scheme should be preserved"
        );
        assert!(
            schemes.contains_key("bearerAuth"),
            "bearerAuth should be added"
        );
    }

    #[test]
    fn appends_to_existing_global_security() {
        let yaml = r"
components:
  schemas: {}
security:
  - apiKey: []
paths: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        add_security_schemes(&mut doc, &[], None);

        let security = doc["security"].as_sequence().unwrap();
        assert_eq!(security.len(), 2, "should have both apiKey and bearerAuth");

        let has_api_key = security
            .iter()
            .any(|item| item.as_mapping().is_some_and(|m| m.contains_key("apiKey")));
        let has_bearer = security.iter().any(|item| {
            item.as_mapping()
                .is_some_and(|m| m.contains_key("bearerAuth"))
        });
        assert!(has_api_key, "apiKey should be preserved");
        assert!(has_bearer, "bearerAuth should be appended");
    }

    #[test]
    fn description_with_special_yaml_chars_does_not_panic() {
        let yaml = r"
components:
  schemas: {}
paths: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();

        // These characters would break YAML string interpolation
        let tricky_description = "Use: colons\nnewlines # comments {braces}";
        add_security_schemes(&mut doc, &[], Some(tricky_description));

        let desc = doc["components"]["securitySchemes"]["bearerAuth"]["description"]
            .as_str()
            .unwrap();
        assert_eq!(desc, tricky_description);
    }

    #[test]
    fn idempotent_bearer_not_duplicated() {
        let yaml = r"
components:
  schemas: {}
paths: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();

        add_security_schemes(&mut doc, &[], None);
        add_security_schemes(&mut doc, &[], None);

        let security = doc["security"].as_sequence().unwrap();
        let bearer_count = security
            .iter()
            .filter(|item| {
                item.as_mapping()
                    .is_some_and(|m| m.contains_key("bearerAuth"))
            })
            .count();
        assert_eq!(bearer_count, 1, "bearerAuth should not be duplicated");
    }
}
