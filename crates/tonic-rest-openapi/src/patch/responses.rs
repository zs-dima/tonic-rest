//! Response-related transforms.
//!
//! - Empty responses → 204 No Content
//! - Redundant query param removal
//! - Plain text content types (configurable endpoints)
//! - Redirect endpoints → 302
//! - REST error schema injection
//! - Readiness probe 503

use serde_yaml_ng::Value;

use crate::config::PlainTextEndpoint;

use super::helpers::{
    for_each_operation, json_content_with_schema_ref, json_response_with_schema_ref,
    response_header, snake_to_lower_camel_dotted, val_s,
};

/// Convert `200 OK` with empty content to `204 No Content`.
pub fn patch_empty_responses(doc: &mut Value) {
    for_each_operation(doc, |_path, _method, op_map| {
        let Some(responses) = op_map.get_mut("responses").and_then(Value::as_mapping_mut) else {
            return;
        };

        let ok_key = Value::String("200".to_string());

        let is_empty_response = responses
            .get(&ok_key)
            .and_then(Value::as_mapping)
            .and_then(|r| r.get("content"))
            .and_then(Value::as_mapping)
            .is_some_and(serde_yaml_ng::Mapping::is_empty);

        if !is_empty_response {
            return;
        }

        responses.remove(&ok_key);

        let mut no_content = serde_yaml_ng::Mapping::new();
        no_content.insert(val_s("description"), val_s("No Content"));
        responses.insert(Value::String("204".to_string()), Value::Mapping(no_content));
    });
}

/// Remove query parameters that duplicate path parameters.
pub fn remove_redundant_query_params(doc: &mut Value) {
    for_each_operation(doc, |_path, _method, op_map| {
        let params_key = Value::String("parameters".to_string());
        let Some(params) = op_map.get_mut(&params_key).and_then(Value::as_sequence_mut) else {
            return;
        };

        let path_param_names: Vec<String> = params
            .iter()
            .filter_map(|p| {
                let m = p.as_mapping()?;
                let in_val = m.get("in")?.as_str()?;
                if in_val == "path" {
                    Some(m.get("name")?.as_str()?.to_string())
                } else {
                    None
                }
            })
            .collect();

        if path_param_names.is_empty() {
            return;
        }

        params.retain(|p| {
            let Some(m) = p.as_mapping() else {
                return true;
            };
            let Some(in_val) = m.get("in").and_then(Value::as_str) else {
                return true;
            };
            if in_val != "query" {
                return true;
            }
            let Some(name) = m.get("name").and_then(Value::as_str) else {
                return true;
            };

            !path_param_names
                .iter()
                .any(|path_name| snake_to_lower_camel_dotted(path_name) == name)
        });
    });
}

/// Patch plain-text endpoints to use `text/plain` instead of `application/json`.
///
/// Configured via [`PlainTextEndpoint`] entries in the project config.
pub fn patch_plain_text_endpoints(doc: &mut Value, endpoints: &[PlainTextEndpoint]) {
    if endpoints.is_empty() {
        return;
    }

    for_each_operation(doc, |path, _method, op_map| {
        let Some(endpoint) = endpoints.iter().find(|e| e.path == path) else {
            return;
        };

        let Some(content) = op_map
            .get_mut("responses")
            .and_then(Value::as_mapping_mut)
            .and_then(|r| r.get_mut("200"))
            .and_then(Value::as_mapping_mut)
            .and_then(|r| r.get_mut("content"))
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        let json_key = Value::String("application/json".to_string());
        if content.remove(&json_key).is_none() {
            return;
        }

        let mut schema = serde_yaml_ng::Mapping::new();
        schema.insert(val_s("type"), val_s("string"));

        let mut media_type = serde_yaml_ng::Mapping::new();
        media_type.insert(val_s("schema"), Value::Mapping(schema));
        if let Some(example) = &endpoint.example {
            media_type.insert(val_s("example"), val_s(example));
        }

        content.insert(val_s("text/plain"), Value::Mapping(media_type));
    });
}

/// Add response headers for the metrics endpoint.
///
/// Skipped if `metrics_path` is `None`.
pub fn patch_metrics_response_headers(doc: &mut Value, metrics_path: Option<&str>) {
    let Some(metrics_path) = metrics_path else {
        return;
    };

    for_each_operation(doc, |path, method, op_map| {
        if path != metrics_path || method != "get" {
            return;
        }

        let Some(response_200) = op_map
            .get_mut("responses")
            .and_then(Value::as_mapping_mut)
            .and_then(|r| r.get_mut("200"))
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        if !response_200.contains_key("headers") {
            response_200.insert(
                val_s("headers"),
                Value::Mapping(serde_yaml_ng::Mapping::new()),
            );
        }

        let Some(headers) = response_200
            .get_mut("headers")
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        headers.insert(
            val_s("Content-Type"),
            response_header(
                "Prometheus text exposition media type.",
                "text/plain; version=0.0.4; charset=utf-8",
            ),
        );
        headers.insert(
            val_s("Cache-Control"),
            response_header(
                "Caching policy for metrics responses.",
                "no-store, no-cache, max-age=0",
            ),
        );
    });
}

/// Add 503 response to readiness probe.
///
/// Skipped if `readiness_path` is `None`.
pub fn patch_readiness_probe_responses(doc: &mut Value, readiness_path: Option<&str>) {
    let Some(readiness_path) = readiness_path else {
        return;
    };

    for_each_operation(doc, |path, method, op_map| {
        if path != readiness_path || method != "get" {
            return;
        }

        let Some(responses) = op_map.get_mut("responses").and_then(Value::as_mapping_mut) else {
            return;
        };

        if responses.contains_key("503") {
            return;
        }

        let Some(schema_ref) = responses
            .get("200")
            .and_then(Value::as_mapping)
            .and_then(|r| r.get("content"))
            .and_then(Value::as_mapping)
            .and_then(|c| c.get("application/json"))
            .and_then(Value::as_mapping)
            .and_then(|m| m.get("schema"))
            .and_then(Value::as_mapping)
            .and_then(|s| s.get("$ref"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let schema_ref = schema_ref.to_string();

        responses.insert(
            val_s("503"),
            json_response_with_schema_ref("Service Unavailable", &schema_ref),
        );
    });
}

/// Patch redirect endpoints: convert `200` to `302` with `Location` header.
pub fn patch_redirect_endpoints(doc: &mut Value, redirect_paths: &[String]) {
    let Some(paths) = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("paths"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    let redirect_response: Value = serde_yaml_ng::from_str(
        r"
description: Redirect to frontend success or error page.
headers:
  Location:
    description: Frontend success or error page URL.
    required: true
    schema:
      type: string
      format: uri
",
    )
    .expect("static YAML must parse");

    let http_methods = ["get", "post", "put", "patch", "delete"];

    for path in redirect_paths {
        let Some(path_item) = paths.get_mut(path).and_then(Value::as_mapping_mut) else {
            continue;
        };

        for method in &http_methods {
            let Some(operation) = path_item.get_mut(*method).and_then(Value::as_mapping_mut) else {
                continue;
            };

            let Some(responses) = operation
                .get_mut("responses")
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            responses.remove("200");
            responses.insert(Value::String("302".to_string()), redirect_response.clone());
        }
    }
}

/// Ensure the REST error response schema exists under `components.schemas`.
pub fn ensure_rest_error_schema(doc: &mut Value, error_schema_ref: &str) {
    let schema_name = error_schema_ref.trim_start_matches("#/components/schemas/");

    let Some(root) = doc.as_mapping_mut() else {
        return;
    };

    if !root.contains_key("components") {
        root.insert(
            val_s("components"),
            Value::Mapping(serde_yaml_ng::Mapping::new()),
        );
    }

    let Some(components) = root.get_mut("components").and_then(Value::as_mapping_mut) else {
        return;
    };

    if !components.contains_key("schemas") {
        components.insert(
            val_s("schemas"),
            Value::Mapping(serde_yaml_ng::Mapping::new()),
        );
    }

    let Some(schemas) = components
        .get_mut("schemas")
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    if schemas.contains_key(schema_name) {
        return;
    }

    let schema: Value = serde_yaml_ng::from_str(
        r"
type: object
required:
  - error
properties:
  error:
    type: object
    required:
      - code
      - message
      - status
    properties:
      code:
        type: integer
        format: int32
        description: HTTP status code.
      message:
        type: string
        description: Human-readable error message.
      status:
        type: string
        description: gRPC status code name (e.g., INVALID_ARGUMENT).
description: REST error response envelope.
",
    )
    .expect("static YAML must parse");

    schemas.insert(val_s(schema_name), schema);
}

/// Rewrite operation `default` responses to use the REST error schema.
pub fn rewrite_default_error_responses(doc: &mut Value, error_schema_ref: &str) {
    for_each_operation(doc, |_path, _method, op_map| {
        let Some(default_response) = op_map
            .get_mut("responses")
            .and_then(Value::as_mapping_mut)
            .and_then(|r| r.get_mut("default"))
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        if !default_response.contains_key("description") {
            default_response.insert(val_s("description"), val_s("Default error response"));
        }

        default_response.insert(
            val_s("content"),
            json_content_with_schema_ref(error_schema_ref),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_response_becomes_204() {
        let yaml = r"
paths:
  /v1/signout:
    post:
      responses:
        '200':
          content: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        patch_empty_responses(&mut doc);

        let responses = doc["paths"]["/v1/signout"]["post"]["responses"]
            .as_mapping()
            .unwrap();
        assert!(!responses.contains_key("200"));
        assert!(responses.contains_key("204"));
    }

    #[test]
    fn redundant_query_params_removed() {
        let yaml = r"
paths:
  /v1/items/{itemId}:
    get:
      parameters:
        - name: itemId
          in: path
          schema:
            type: string
        - name: itemId
          in: query
          schema:
            type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_redundant_query_params(&mut doc);

        let params = doc["paths"]["/v1/items/{itemId}"]["get"]["parameters"]
            .as_sequence()
            .unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0]["in"].as_str().unwrap(), "path");
    }

    #[test]
    fn redirect_endpoint_patched() {
        let yaml = r"
paths:
  /v1/redirect:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/RedirectResponse'
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        patch_redirect_endpoints(&mut doc, &["/v1/redirect".to_string()]);

        let responses = doc["paths"]["/v1/redirect"]["get"]["responses"]
            .as_mapping()
            .unwrap();
        assert!(!responses.contains_key("200"));
        assert!(responses.contains_key("302"));
    }

    #[test]
    fn redirect_post_endpoint_patched() {
        let yaml = r"
paths:
  /v1/oauth/callback:
    post:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/RedirectResponse'
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        patch_redirect_endpoints(&mut doc, &["/v1/oauth/callback".to_string()]);

        let responses = doc["paths"]["/v1/oauth/callback"]["post"]["responses"]
            .as_mapping()
            .unwrap();
        assert!(
            !responses.contains_key("200"),
            "200 should be replaced by 302"
        );
        assert!(
            responses.contains_key("302"),
            "POST redirect should get 302"
        );
    }

    #[test]
    fn error_schema_created_if_missing() {
        let yaml = "paths: {}";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        ensure_rest_error_schema(&mut doc, "#/components/schemas/rest.v1.ErrorResponse");

        let schema = &doc["components"]["schemas"]["rest.v1.ErrorResponse"];
        assert!(schema.as_mapping().is_some());
    }
}
