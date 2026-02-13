//! Cleanup, normalization, and request-body inlining transforms.
//!
//! - Tag description simplification
//! - Enum value rewrites (prefix stripping)
//! - Unimplemented operation markers
//! - Empty request body removal
//! - Unused schema removal
//! - `format: enum` noise removal
//! - Request body inlining with example generation

use std::collections::HashMap;

use serde_yaml_ng::Value;

use crate::discover::ProtoMetadata;

use super::helpers::{
    collect_empty_schema_names, collect_refs, for_each_operation, json_response_with_schema_ref,
    request_body_ref, val_s, UUID_EXAMPLE,
};

/// Simplify tag descriptions for Swagger UI rendering.
///
/// Proto service comments often contain `=====` separator lines,
/// checklists, and flow diagrams that clutter the Swagger UI tag header.
/// Keeps only the first meaningful line as a short summary.
pub fn clean_tag_descriptions(doc: &mut Value) {
    let Some(tags) = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("tags"))
        .and_then(Value::as_sequence_mut)
    else {
        return;
    };

    for tag in tags.iter_mut() {
        let Some(tag_map) = tag.as_mapping_mut() else {
            continue;
        };

        let desc_key = Value::String("description".to_string());
        let Some(desc) = tag_map.get(&desc_key).and_then(Value::as_str) else {
            continue;
        };

        let summary = extract_tag_summary(desc);
        tag_map.insert(desc_key, Value::String(summary));
    }
}

/// Extract the first meaningful line from a raw proto service comment.
///
/// Skips `=====` separator lines and blank lines.
fn extract_tag_summary(raw: &str) -> String {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().all(|c| c == '=') {
            continue;
        }
        return trimmed.to_string();
    }
    raw.trim().to_string()
}

/// Strip `_UNSPECIFIED` / `unspecified` sentinel values from parameter enum arrays.
///
/// Proto enums always include a `*_UNSPECIFIED = 0` sentinel. This function
/// removes those from path and query parameter schemas since they are never
/// valid API values.
pub fn strip_unspecified_from_query_enums(doc: &mut Value) {
    for_each_operation(doc, |_path, _method, op_map| {
        let Some(params) = op_map
            .get_mut("parameters")
            .and_then(Value::as_sequence_mut)
        else {
            return;
        };

        for param in params.iter_mut() {
            let Some(p) = param.as_mapping_mut() else {
                continue;
            };

            let is_param = p
                .get("in")
                .and_then(Value::as_str)
                .is_some_and(|v| v == "query" || v == "path");

            if !is_param {
                continue;
            }

            if let Some(schema) = p.get_mut("schema").and_then(Value::as_mapping_mut) {
                strip_unspecified_enum(schema);

                // Array items (e.g., `statuses` is `type: array, items: {enum: [...]}`)
                if let Some(items) = schema.get_mut("items").and_then(Value::as_mapping_mut) {
                    strip_unspecified_enum(items);
                }
            }
        }
    });
}

/// Remove unspecified sentinel values from a schema's enum array.
fn strip_unspecified_enum(schema: &mut serde_yaml_ng::Mapping) {
    if let Some(enum_vals) = schema.get_mut("enum").and_then(Value::as_sequence_mut) {
        enum_vals.retain(|v| {
            v.as_str().is_none_or(|s| {
                s != "unspecified" && !s.ends_with("_UNSPECIFIED") && !s.ends_with("_unspecified")
            })
        });
    }
}

/// Rewrite enum values in component schemas from raw proto names to clean names.
///
/// Uses [`ProtoMetadata::enum_rewrites`] for targeted property rewrites and
/// [`ProtoMetadata::enum_value_map`] for global inline enum rewrites.
pub fn rewrite_enum_values(doc: &mut Value, metadata: &ProtoMetadata) {
    let rewrites = &metadata.enum_rewrites;
    if !rewrites.is_empty() {
        let Some(schemas) = doc
            .as_mapping_mut()
            .and_then(|m| m.get_mut("components"))
            .and_then(Value::as_mapping_mut)
            .and_then(|m| m.get_mut("schemas"))
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        for rewrite in rewrites {
            let Some(prop) = schemas
                .get_mut(rewrite.schema.as_str())
                .and_then(Value::as_mapping_mut)
                .and_then(|s| s.get_mut("properties"))
                .and_then(Value::as_mapping_mut)
                .and_then(|p| p.get_mut(rewrite.field.as_str()))
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            // Direct enum on the property (scalar field)
            if let Some(enum_vals) = prop.get_mut("enum").and_then(Value::as_sequence_mut) {
                *enum_vals = rewrite.values.iter().map(|s| val_s(s)).collect();
            }

            // Enum inside `items` (repeated/array field)
            if let Some(enum_vals) = prop
                .get_mut("items")
                .and_then(Value::as_mapping_mut)
                .and_then(|items| items.get_mut("enum"))
                .and_then(Value::as_sequence_mut)
            {
                *enum_vals = rewrite.values.iter().map(|s| val_s(s)).collect();
            }
        }
    }

    // Also rewrite inline enums in path/query parameters.
    rewrite_inline_enums(doc, &metadata.enum_value_map);
}

/// Rewrite inline enum values in path and query parameters.
fn rewrite_inline_enums(doc: &mut Value, value_map: &HashMap<String, String>) {
    if value_map.is_empty() {
        return;
    }
    rewrite_inline_enums_recursive(doc, value_map);
}

/// Recursively walk the YAML tree and rewrite enum arrays using the raw→stripped map.
fn rewrite_inline_enums_recursive(value: &mut Value, map: &HashMap<String, String>) {
    match value {
        Value::Mapping(m) => {
            if let Some(enum_vals) = m.get_mut("enum").and_then(Value::as_sequence_mut) {
                for val in enum_vals.iter_mut() {
                    if let Some(raw) = val.as_str() {
                        if let Some(stripped) = map.get(raw) {
                            *val = Value::String(stripped.clone());
                        }
                    }
                }
            }
            for (_, v) in m.iter_mut() {
                rewrite_inline_enums_recursive(v, map);
            }
        }
        Value::Sequence(seq) => {
            for item in seq.iter_mut() {
                rewrite_inline_enums_recursive(item, map);
            }
        }
        _ => {}
    }
}

/// Mark operations that currently return `UNIMPLEMENTED` with availability metadata.
///
/// Adds `x-not-implemented: true` and prepends a notice to the description.
/// Also adds a `501 Not Implemented` response entry.
pub fn mark_unimplemented_operations(
    doc: &mut Value,
    unimplemented_ops: &[String],
    error_schema_ref: &str,
) {
    for_each_operation(doc, |_path, _method, op_map| {
        let op_id = op_map
            .get(Value::String("operationId".to_string()))
            .and_then(Value::as_str)
            .unwrap_or_default();

        if !unimplemented_ops.iter().any(|id| id == op_id) {
            return;
        }

        op_map.insert(
            Value::String("x-not-implemented".to_string()),
            Value::Bool(true),
        );

        let desc_key = Value::String("description".to_string());
        let existing = op_map
            .get(&desc_key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        if !existing.starts_with("⚠️") {
            op_map.insert(
                desc_key,
                Value::String(format!(
                    "⚠️ **Not yet implemented** — returns gRPC UNIMPLEMENTED.\n\n{existing}"
                )),
            );
        }

        if let Some(responses) = op_map.get_mut("responses").and_then(Value::as_mapping_mut) {
            if !responses.contains_key("501") {
                responses.insert(
                    val_s("501"),
                    json_response_with_schema_ref("Not Implemented", error_schema_ref),
                );
            }
        }
    });
}

/// Remove `requestBody` from operations whose request schema has no properties.
pub fn remove_empty_request_bodies(doc: &mut Value) {
    let empty_schemas = collect_empty_schema_names(doc);

    for_each_operation(doc, |_path, _method, op| {
        let is_empty = request_body_ref(op)
            .is_some_and(|r| empty_schemas.iter().any(|s| r.ends_with(s.as_str())));
        if is_empty {
            op.remove("requestBody");
        }
    });
}

/// Remove `requestBody` entries where the inlined schema has no properties.
///
/// After path field stripping and request body inlining, some operations
/// end up with an empty inline schema. Remove them for cleaner output.
pub fn remove_empty_inlined_request_bodies(doc: &mut Value) {
    for_each_operation(doc, |_path, _method, op| {
        let is_empty = op
            .get("requestBody")
            .and_then(Value::as_mapping)
            .and_then(|rb| rb.get("content"))
            .and_then(Value::as_mapping)
            .and_then(|c| c.get("application/json"))
            .and_then(Value::as_mapping)
            .and_then(|mt| mt.get("schema"))
            .and_then(Value::as_mapping)
            .and_then(|s| s.get("properties"))
            .and_then(Value::as_mapping)
            .is_some_and(serde_yaml_ng::Mapping::is_empty);

        if is_empty {
            op.remove("requestBody");
        }
    });
}

/// Remove empty-property schemas from components that are no longer referenced.
pub fn remove_unused_empty_schemas(doc: &mut Value) {
    let empty = collect_empty_schema_names(doc);
    if empty.is_empty() {
        return;
    }

    let mut referenced = std::collections::HashSet::new();
    collect_refs(doc, &mut referenced);

    let orphans: Vec<String> = empty
        .into_iter()
        .filter(|name| {
            let ref_str = format!("#/components/schemas/{name}");
            !referenced.contains(&ref_str)
        })
        .collect();

    if let Some(schemas) = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("components"))
        .and_then(Value::as_mapping_mut)
        .and_then(|m| m.get_mut("schemas"))
        .and_then(Value::as_mapping_mut)
    {
        for name in &orphans {
            schemas.remove(name.as_str());
        }
    }
}

/// Remove nonstandard `format: enum` from all schema properties.
///
/// gnostic adds `format: enum` to every enum-typed field. This is not a valid
/// JSON Schema / `OpenAPI` 3.1 format value.
pub fn remove_format_enum(doc: &mut Value) {
    strip_format_enum_recursive(doc);
}

/// Recursively walk the YAML tree and remove `format: enum` entries.
fn strip_format_enum_recursive(value: &mut Value) {
    match value {
        Value::Mapping(map) => {
            let format_key = Value::String("format".to_string());
            let is_format_enum = map
                .get(&format_key)
                .and_then(Value::as_str)
                .is_some_and(|v| v == "enum");

            if is_format_enum {
                map.remove(&format_key);
            }

            for (_, v) in map.iter_mut() {
                strip_format_enum_recursive(v);
            }
        }
        Value::Sequence(seq) => {
            for item in seq.iter_mut() {
                strip_format_enum_recursive(item);
            }
        }
        _ => {}
    }
}

/// Inline request body schemas directly into operations for better Swagger UI.
///
/// Replaces `$ref` to component schemas with the full inline schema,
/// generates heuristic-based examples, resolves nested references, and removes
/// orphaned schemas. Example values are best-effort guesses from field names
/// and types — review and adjust them in the output if needed.
pub fn inline_request_bodies(doc: &mut Value) {
    // Clone component schemas for read-only lookup during mutation.
    let schemas_snapshot: serde_yaml_ng::Mapping = doc
        .as_mapping()
        .and_then(|m| m.get("components"))
        .and_then(Value::as_mapping)
        .and_then(|m| m.get("schemas"))
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();

    // Collect inline data for each operation, keyed by (path, method) to avoid
    // collisions when multiple operations share the same operationId or have empty IDs.
    let mut inlines: Vec<(String, String, Value, Value, Option<String>)> = Vec::new();

    for_each_operation(doc, |path, method, op_map| {
        let Some(ref_str) = request_body_ref(op_map).map(str::to_string) else {
            return;
        };
        let schema_name = ref_str.trim_start_matches("#/components/schemas/");
        let Some(schema) = schemas_snapshot
            .get(schema_name)
            .and_then(Value::as_mapping)
        else {
            return;
        };

        let mut body_schema = schema.clone();
        let desc = body_schema.remove("description").and_then(|v| match v {
            Value::String(s) => Some(s),
            _ => None,
        });

        resolve_nested_refs(&mut body_schema, &schemas_snapshot);
        let example = generate_schema_example(&body_schema, &schemas_snapshot);

        inlines.push((
            path.to_string(),
            method.to_string(),
            Value::Mapping(body_schema),
            example,
            desc,
        ));
    });

    // Apply collected inlines to operations.
    for_each_operation(doc, |path, method, op_map| {
        let Some((_, _, schema, example, desc)) = inlines
            .iter()
            .find(|(p, m, _, _, _)| p == path && m == method)
        else {
            return;
        };

        let Some(rb) = op_map
            .get_mut("requestBody")
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        if let Some(d) = desc {
            rb.insert(val_s("description"), val_s(d));
        }

        let Some(media_type) = rb
            .get_mut("content")
            .and_then(Value::as_mapping_mut)
            .and_then(|c| c.get_mut("application/json"))
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        media_type.insert(val_s("schema"), schema.clone());

        if let Value::Mapping(m) = example {
            if !m.is_empty() {
                media_type.insert(val_s("example"), Value::Mapping(m.clone()));
            }
        }
    });

    // Remove schemas no longer referenced by any `$ref`.
    remove_orphaned_schemas(doc);
}

/// Resolve `allOf: [{$ref: ...}]` in schema properties to inline objects.
fn resolve_nested_refs(schema: &mut serde_yaml_ng::Mapping, schemas: &serde_yaml_ng::Mapping) {
    let Some(props) = schema.get_mut("properties").and_then(Value::as_mapping_mut) else {
        return;
    };

    let prop_names: Vec<String> = props
        .iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect();

    for name in &prop_names {
        let Some(prop) = props.get_mut(name.as_str()).and_then(Value::as_mapping_mut) else {
            continue;
        };

        let ref_name = prop
            .get("allOf")
            .and_then(Value::as_sequence)
            .and_then(|seq| seq.first())
            .and_then(Value::as_mapping)
            .and_then(|m| m.get("$ref"))
            .and_then(Value::as_str)
            .map(|r| r.trim_start_matches("#/components/schemas/").to_string());

        let Some(ref_name) = ref_name else {
            continue;
        };

        let Some(resolved) = schemas.get(ref_name.as_str()).and_then(Value::as_mapping) else {
            continue;
        };

        let desc = prop.get("description").cloned();
        *prop = resolved.clone();
        if let Some(d) = desc {
            prop.insert(val_s("description"), d);
        }
    }
}

/// Generate an example object from a schema's properties.
fn generate_schema_example(
    schema: &serde_yaml_ng::Mapping,
    schemas: &serde_yaml_ng::Mapping,
) -> Value {
    let Some(props) = schema.get("properties").and_then(Value::as_mapping) else {
        return Value::Mapping(serde_yaml_ng::Mapping::new());
    };

    let mut example = serde_yaml_ng::Mapping::new();
    for (key, val) in props {
        let name = key.as_str().unwrap_or_default();
        example.insert(key.clone(), generate_field_example(name, val, schemas));
    }
    Value::Mapping(example)
}

/// Generate an example value for a single field based on its name, type, and constraints.
fn generate_field_example(name: &str, prop: &Value, schemas: &serde_yaml_ng::Mapping) -> Value {
    let map = prop.as_mapping();

    let field_type = map
        .and_then(|m| m.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let format = map
        .and_then(|m| m.get("format"))
        .and_then(Value::as_str)
        .unwrap_or("");

    // UUID fields
    if format == "uuid" {
        return val_s(UUID_EXAMPLE);
    }

    // Field mask → comma-separated field names
    if format == "field-mask" {
        return example_from_field_name(name).unwrap_or_else(|| val_s("name,email"));
    }

    // Enum fields → first non-unspecified value, fallback to first
    if let Some(enum_vals) = map.and_then(|m| m.get("enum")).and_then(Value::as_sequence) {
        let best = enum_vals
            .iter()
            .find(|v| v.as_str().is_none_or(|s| s != "unspecified"))
            .or_else(|| enum_vals.first());
        if let Some(val) = best {
            return val.clone();
        }
    }

    // Nested object (has `properties` — already inlined)
    if let Some(inner) = map
        .filter(|_| field_type == "object")
        .filter(|m| m.get("properties").is_some())
    {
        return generate_schema_example(inner, schemas);
    }

    // Nested allOf reference (not yet inlined)
    if let Some(ref_name) = map
        .and_then(|m| m.get("allOf"))
        .and_then(Value::as_sequence)
        .and_then(|seq| seq.first())
        .and_then(Value::as_mapping)
        .and_then(|m| m.get("$ref"))
        .and_then(Value::as_str)
    {
        let schema_name = ref_name.trim_start_matches("#/components/schemas/");
        if let Some(resolved) = schemas.get(schema_name).and_then(Value::as_mapping) {
            return generate_schema_example(resolved, schemas);
        }
    }

    // additionalProperties map
    if map.and_then(|m| m.get("additionalProperties")).is_some() {
        let mut obj = serde_yaml_ng::Mapping::new();
        obj.insert(val_s("key"), val_s("value"));
        return Value::Mapping(obj);
    }

    // Array fields
    if field_type == "array" {
        if let Some(items) = map.and_then(|m| m.get("items")) {
            let item_example = generate_field_example("item", items, schemas);
            return Value::Sequence(vec![item_example]);
        }
        return Value::Sequence(vec![]);
    }

    // Name-based heuristics for strings
    if let Some(v) = example_from_field_name(name) {
        return v;
    }

    // Type defaults
    if field_type == "boolean" {
        return Value::Bool(true);
    }
    if field_type == "integer" {
        let min = map
            .and_then(|m| m.get("minimum"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        return Value::Number(min.into());
    }
    if format == "date-time" {
        return val_s("2026-01-15T09:30:00Z");
    }

    val_s("string")
}

/// Generate an example value based on field name heuristics.
///
/// Uses common naming conventions to produce realistic example values.
/// Only universal patterns are matched (email, password, name, URL, etc.).
/// For domain-specific field naming, post-process the generated spec or
/// override examples in your config.
fn example_from_field_name(name: &str) -> Option<Value> {
    let lower = name.to_lowercase();
    if lower.contains("password") || lower.contains("secret") {
        return Some(val_s("P@ssw0rd123!"));
    }
    if lower == "identifier" || lower.contains("email") {
        return Some(val_s("user@example.com"));
    }
    if lower.contains("phone") {
        return Some(val_s("+1234567890"));
    }
    if lower == "name" || lower.contains("displayname") || lower.contains("display_name") {
        return Some(val_s("John Doe"));
    }
    if lower.contains("token") {
        return Some(val_s("eyJhbGciOiJIUzI1NiIs..."));
    }
    if lower == "code" {
        return Some(val_s("123456"));
    }
    if lower == "query" || lower == "search" {
        return Some(val_s("search term"));
    }
    if lower.contains("url") || lower.contains("uri") {
        return Some(val_s("https://example.com"));
    }
    if lower.contains("version") {
        return Some(val_s("1.0.0"));
    }
    if lower.contains("pagesize") || lower.contains("page_size") || lower.contains("limit") {
        return Some(Value::Number(20.into()));
    }
    if lower.contains("pagetoken") || lower.contains("page_token") || lower.contains("cursor") {
        return Some(val_s("eyJpZCI6MTAwfQ=="));
    }
    None
}

/// Remove component schemas that are no longer referenced by any `$ref`.
///
/// Runs in a loop to handle cascading orphans.
fn remove_orphaned_schemas(doc: &mut Value) {
    loop {
        let all_names: Vec<String> = doc
            .as_mapping()
            .and_then(|m| m.get("components"))
            .and_then(Value::as_mapping)
            .and_then(|m| m.get("schemas"))
            .and_then(Value::as_mapping)
            .map(|schemas| {
                schemas
                    .keys()
                    .filter_map(|k| k.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        if all_names.is_empty() {
            break;
        }

        let mut referenced = std::collections::HashSet::new();
        collect_refs(doc, &mut referenced);

        let orphans: Vec<String> = all_names
            .into_iter()
            .filter(|name| {
                let ref_str = format!("#/components/schemas/{name}");
                !referenced.contains(&ref_str)
            })
            .collect();

        if orphans.is_empty() {
            break;
        }

        if let Some(schemas) = doc
            .as_mapping_mut()
            .and_then(|m| m.get_mut("components"))
            .and_then(Value::as_mapping_mut)
            .and_then(|m| m.get_mut("schemas"))
            .and_then(Value::as_mapping_mut)
        {
            for name in &orphans {
                schemas.remove(name.as_str());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_descriptions_simplified() {
        let yaml = r"
tags:
  - name: AuthService
    description: |
      ====================================
      Authentication service for users.
      Handles sign-up, login, and sessions.
      OWASP checklist items...
  - name: UserService
    description: User management.
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        clean_tag_descriptions(&mut doc);

        let tags = doc["tags"].as_sequence().unwrap();
        assert_eq!(
            tags[0]["description"].as_str().unwrap(),
            "Authentication service for users."
        );
        assert_eq!(tags[1]["description"].as_str().unwrap(), "User management.");
    }

    #[test]
    fn unspecified_stripped_from_query() {
        let yaml = r"
paths:
  /v1/users:
    get:
      parameters:
        - name: status
          in: query
          schema:
            type: string
            enum:
              - USER_STATUS_UNSPECIFIED
              - USER_STATUS_ACTIVE
              - USER_STATUS_SUSPENDED
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        strip_unspecified_from_query_enums(&mut doc);

        let vals = doc["paths"]["/v1/users"]["get"]["parameters"][0]["schema"]["enum"]
            .as_sequence()
            .unwrap();
        assert_eq!(vals.len(), 2);
        assert!(!vals
            .iter()
            .any(|v| v.as_str().unwrap().contains("UNSPECIFIED")));
    }

    #[test]
    fn format_enum_removed() {
        let yaml = r"
components:
  schemas:
    test.v1.Request:
      type: object
      properties:
        status:
          type: string
          format: enum
          enum:
            - active
            - suspended
        name:
          type: string
          format: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_format_enum(&mut doc);

        let status = doc["components"]["schemas"]["test.v1.Request"]["properties"]["status"]
            .as_mapping()
            .unwrap();
        assert!(!status.contains_key("format"));

        // Non-enum formats should remain
        let name = doc["components"]["schemas"]["test.v1.Request"]["properties"]["name"]
            .as_mapping()
            .unwrap();
        assert_eq!(name.get("format").unwrap().as_str().unwrap(), "string");
    }

    #[test]
    fn empty_request_bodies_removed() {
        let yaml = r"
paths:
  /v1/sessions:
    delete:
      operationId: AuthService_SignOut
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/auth.v1.SignOutRequest'
components:
  schemas:
    auth.v1.SignOutRequest:
      type: object
      properties: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_empty_request_bodies(&mut doc);

        let op = doc["paths"]["/v1/sessions"]["delete"].as_mapping().unwrap();
        assert!(!op.contains_key("requestBody"));
    }

    #[test]
    fn request_body_inlining_works() {
        let yaml = r"
paths:
  /v1/auth:
    post:
      operationId: AuthService_Authenticate
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/auth.v1.AuthRequest'
components:
  schemas:
    auth.v1.AuthRequest:
      type: object
      description: Authentication request body.
      properties:
        email:
          type: string
        password:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        inline_request_bodies(&mut doc);

        // Schema ref should be replaced with inline properties
        let media_type = doc["paths"]["/v1/auth"]["post"]["requestBody"]["content"]
            ["application/json"]
            .as_mapping()
            .unwrap();
        let schema = media_type.get("schema").unwrap().as_mapping().unwrap();
        assert!(schema.contains_key("properties"));
        assert!(!schema.contains_key("$ref"));

        // Description should be moved to requestBody
        let rb = doc["paths"]["/v1/auth"]["post"]["requestBody"]
            .as_mapping()
            .unwrap();
        assert_eq!(
            rb.get("description").unwrap().as_str().unwrap(),
            "Authentication request body."
        );

        // Example should be generated
        assert!(media_type.contains_key("example"));
    }

    #[test]
    fn unimplemented_operations_marked() {
        let yaml = r"
paths:
  /v1/mfa/setup:
    post:
      operationId: AuthService_SetupMfa
      description: Set up MFA.
      responses:
        '200':
          description: OK
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let error_ref = "#/components/schemas/ErrorResponse";
        mark_unimplemented_operations(&mut doc, &["AuthService_SetupMfa".to_string()], error_ref);

        let op = doc["paths"]["/v1/mfa/setup"]["post"].as_mapping().unwrap();
        assert!(op.get("x-not-implemented").unwrap().as_bool().unwrap());
        assert!(op
            .get("description")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("⚠️"));
        assert!(op
            .get("responses")
            .unwrap()
            .as_mapping()
            .unwrap()
            .contains_key("501"));
    }
}
