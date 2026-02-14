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
    UUID_EXAMPLE, collect_empty_schema_names, collect_refs, for_each_operation,
    json_response_with_schema_ref, request_body_ref, schemas, schemas_mut, val_s,
};

/// Populate `summary` on operations that have a `description` but no `summary`.
///
/// Swagger UI displays `summary` in the collapsed endpoint list. Without it,
/// the full `description` is shown which can be verbose. This extracts the
/// first meaningful line of `description` as a concise `summary`.
pub fn populate_operation_summaries(doc: &mut Value) {
    for_each_operation(doc, |_path, _method, op_map| {
        let summary_key = Value::String("summary".to_string());

        // Skip if summary already present and non-empty
        let has_summary = op_map
            .get(&summary_key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        if has_summary {
            return;
        }

        let desc_key = Value::String("description".to_string());
        let Some(desc) = op_map.get(&desc_key).and_then(Value::as_str) else {
            return;
        };

        let summary = extract_first_line(desc);
        if !summary.is_empty() {
            op_map.insert(summary_key, Value::String(summary));
        }
    });
}

/// Extract the first non-empty, non-separator line from text.
///
/// Strips a trailing period for conciseness (consistent with tag summary style).
fn extract_first_line(text: &str) -> String {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().all(|c| c == '=') {
            continue;
        }
        return trimmed.strip_suffix('.').unwrap_or(trimmed).to_string();
    }
    String::new()
}

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

/// Strip `_UNSPECIFIED` / `unspecified` sentinel values from enum arrays.
///
/// Proto enums always include a `*_UNSPECIFIED = 0` sentinel. This function
/// removes those from:
/// - Path and query parameter schemas
/// - Component schema properties (both direct enums and array item enums)
pub fn strip_unspecified_from_query_enums(doc: &mut Value) {
    // Strip from path/query parameters
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

    // Strip from component schemas
    if let Some(schema_map) = schemas_mut(doc) {
        let schema_names: Vec<String> = schema_map
            .iter()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .collect();

        for name in &schema_names {
            let Some(props) = schema_map
                .get_mut(name.as_str())
                .and_then(Value::as_mapping_mut)
                .and_then(|s| s.get_mut("properties"))
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            let prop_names: Vec<String> = props
                .iter()
                .filter_map(|(k, _)| k.as_str().map(str::to_string))
                .collect();

            for prop_name in &prop_names {
                if let Some(prop) = props
                    .get_mut(prop_name.as_str())
                    .and_then(Value::as_mapping_mut)
                {
                    strip_unspecified_enum(prop);

                    if let Some(items) = prop.get_mut("items").and_then(Value::as_mapping_mut) {
                        strip_unspecified_enum(items);
                    }
                }
            }
        }
    }
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
        let Some(schema_map) = schemas_mut(doc) else {
            return;
        };

        for rewrite in rewrites {
            let Some(prop) = schema_map
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

/// Mark operations as deprecated in the `OpenAPI` spec.
///
/// Sets `deprecated: true` on matching operations, which renders as
/// strikethrough in Swagger UI. This is the standard `OpenAPI` mechanism
/// for indicating deprecated endpoints.
pub fn mark_deprecated_operations(doc: &mut Value, deprecated_ops: &[String]) {
    if deprecated_ops.is_empty() {
        return;
    }

    for_each_operation(doc, |_path, _method, op_map| {
        let op_id = op_map
            .get(Value::String("operationId".to_string()))
            .and_then(Value::as_str)
            .unwrap_or_default();

        if !deprecated_ops.iter().any(|id| id == op_id) {
            return;
        }

        op_map.insert(Value::String("deprecated".to_string()), Value::Bool(true));
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

    if let Some(schema_map) = schemas_mut(doc) {
        for name in &orphans {
            schema_map.remove(name.as_str());
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
    let schemas_snapshot: serde_yaml_ng::Mapping = schemas(doc).cloned().unwrap_or_default();

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

        let mut schema_with_examples = schema.clone();
        inject_property_examples(&mut schema_with_examples, example);
        media_type.insert(val_s("schema"), schema_with_examples);
    });

    // Remove schemas no longer referenced by any `$ref`.
    remove_orphaned_schemas(doc);
}

/// Inject example values from the generated example object into individual schema properties.
///
/// Moves examples from a flat object into per-property `example` annotations,
/// so Swagger UI displays them inline in the Schema tab alongside types and
/// constraints. Recurses into nested object properties.
fn inject_property_examples(schema: &mut Value, example: &Value) {
    let (Some(props), Some(example_map)) = (
        schema
            .as_mapping_mut()
            .and_then(|m| m.get_mut("properties"))
            .and_then(Value::as_mapping_mut),
        example.as_mapping(),
    ) else {
        return;
    };

    let prop_names: Vec<String> = props
        .iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect();

    for name in &prop_names {
        let (Some(prop), Some(ex_val)) = (
            props.get_mut(name.as_str()).and_then(Value::as_mapping_mut),
            example_map.get(name.as_str()),
        ) else {
            continue;
        };

        // Skip if property already has an example (e.g., UUID fields from validation injection)
        if prop.contains_key("example") {
            continue;
        }

        let is_nested_object = prop.get("properties").is_some();
        if is_nested_object {
            // Recurse into nested objects instead of setting a flat example
            let mut nested_schema = Value::Mapping(prop.clone());
            inject_property_examples(&mut nested_schema, ex_val);
            if let Value::Mapping(updated) = nested_schema {
                *prop = updated;
            }
        } else {
            prop.insert(val_s("example"), ex_val.clone());
        }
    }
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
    if lower.contains("password") {
        // Differentiate password examples so "currentPassword" vs "newPassword"
        // don't show identical values (confusing for API consumers).
        if lower.starts_with("new") {
            return Some(val_s("N3wP@ssw0rd!456"));
        }
        return Some(val_s("P@ssw0rd123!"));
    }
    // `secret` alone is ambiguous (TOTP secret, API secret, etc.) —
    // only match when combined with `password` (handled above).
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
    // `code` alone is ambiguous (OAuth authorization code, error code,
    // verification code, etc.). Match more specific names instead.
    if lower == "otp"
        || lower.contains("verification_code")
        || lower.contains("verificationcode")
        || lower.contains("mfa_code")
        || lower.contains("mfacode")
        || lower.contains("totp_code")
        || lower.contains("totpcode")
    {
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
    if lower == "locale" {
        return Some(val_s("en-US"));
    }
    if lower.contains("timezone") || lower.contains("time_zone") {
        return Some(val_s("America/New_York"));
    }
    if lower == "language" || lower == "lang" {
        return Some(val_s("en"));
    }
    if lower == "country" || lower == "ipcountry" || lower == "ip_country" {
        return Some(val_s("US"));
    }
    if lower.contains("idempotency") || lower.contains("request_id") || lower == "requestid" {
        return Some(val_s(UUID_EXAMPLE));
    }
    if lower == "description" {
        return Some(val_s("A brief description"));
    }
    if lower == "title" || lower == "subject" {
        return Some(val_s("Example Title"));
    }
    if lower.contains("hostname") || lower == "host" {
        return Some(val_s("api.example.com"));
    }
    if lower == "ip"
        || lower.contains("ip_address")
        || lower.contains("ipaddress")
        || lower.starts_with("ip_created")
        || lower.starts_with("ipcreated")
    {
        return Some(val_s("192.168.1.1"));
    }
    if lower.contains("user_agent") || lower.contains("useragent") {
        return Some(val_s("Mozilla/5.0 (compatible)"));
    }
    if lower.contains("content_type")
        || lower.contains("contenttype")
        || lower.contains("media_type")
        || lower.contains("mediatype")
    {
        return Some(val_s("application/json"));
    }
    if lower == "etag" {
        return Some(val_s("\"33a64df551425fcc55e4d42a148795d9f25f89d4\""));
    }
    // Device / session metadata heuristics
    if lower == "deviceid" || lower == "device_id" {
        return Some(val_s("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));
    }
    if lower == "devicename" || lower == "device_name" {
        return Some(val_s("iPhone 15 Pro"));
    }
    if lower == "devicetype" || lower == "device_type" {
        return Some(val_s("mobile"));
    }
    if lower == "installationid" || lower == "installation_id" {
        return Some(val_s(UUID_EXAMPLE));
    }
    None
}

/// Enrich component schema properties with heuristic-based example values.
///
/// Adds per-property `example` annotations to all schemas in
/// `components/schemas`, using field-name heuristics (email → `"user@example.com"`,
/// password → `"P@ssw0rd123!"`, etc.) and type-based defaults (enums, booleans,
/// integers, dates).
///
/// Only adds examples that provide real documentation value — generic string
/// fields without a matching heuristic are left without examples to avoid noise.
///
/// Skips properties that:
/// - Already have an `example` (e.g., UUID fields from Phase 8)
/// - Use `allOf`/`oneOf`/`$ref` (referenced schemas are enriched separately)
pub fn enrich_schema_examples(doc: &mut Value) {
    let schemas_snapshot: serde_yaml_ng::Mapping = schemas(doc).cloned().unwrap_or_default();

    let Some(schema_map) = schemas_mut(doc) else {
        return;
    };

    let names: Vec<String> = schema_map
        .iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect();

    for name in &names {
        let Some(props) = schema_map
            .get_mut(name.as_str())
            .and_then(Value::as_mapping_mut)
            .and_then(|s| s.get_mut("properties"))
            .and_then(Value::as_mapping_mut)
        else {
            continue;
        };

        let prop_names: Vec<String> = props
            .iter()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .collect();

        for prop_name in &prop_names {
            let Some(prop) = props
                .get_mut(prop_name.as_str())
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            // Skip if already has an example
            if prop.contains_key("example") {
                continue;
            }

            // Skip composite properties — referenced schemas get their own examples
            if prop.contains_key("allOf")
                || prop.contains_key("oneOf")
                || prop.contains_key("$ref")
                || prop.contains_key("properties")
            {
                continue;
            }

            if let Some(example) = meaningful_field_example(
                prop_name,
                &Value::Mapping(prop.clone()),
                &schemas_snapshot,
            ) {
                prop.insert(val_s("example"), example);
            }
        }
    }
}

/// Enrich inline request-body schemas with per-property examples.
///
/// Some operations (e.g., those with path parameters extracted from the request
/// message) end up with inline `type: object` request bodies rather than a
/// `$ref` to a named schema. [`enrich_schema_examples`] only touches named
/// component schemas, so this function fills the gap for inline bodies using
/// the same [`meaningful_field_example`] heuristics.
pub fn enrich_inline_request_body_examples(doc: &mut Value) {
    let schemas_snapshot: serde_yaml_ng::Mapping = schemas(doc).cloned().unwrap_or_default();

    for_each_operation(doc, |_path, _method, op_map| {
        // Navigate: requestBody → content → application/json → schema → properties
        let Some(props) = op_map
            .get_mut("requestBody")
            .and_then(Value::as_mapping_mut)
            .and_then(|rb| rb.get_mut("content"))
            .and_then(Value::as_mapping_mut)
            .and_then(|c| c.get_mut("application/json"))
            .and_then(Value::as_mapping_mut)
            .and_then(|mt| mt.get_mut("schema"))
            .and_then(Value::as_mapping_mut)
            // Only inline bodies (has `properties`), not $ref bodies
            .filter(|s| s.contains_key("properties") && !s.contains_key("$ref"))
            .and_then(|s| s.get_mut("properties"))
            .and_then(Value::as_mapping_mut)
        else {
            return;
        };

        let prop_names: Vec<String> = props
            .iter()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .collect();

        for prop_name in &prop_names {
            let Some(prop) = props
                .get_mut(prop_name.as_str())
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            if prop.contains_key("example") {
                continue;
            }

            if prop.contains_key("allOf")
                || prop.contains_key("oneOf")
                || prop.contains_key("$ref")
                || prop.contains_key("properties")
            {
                continue;
            }

            if let Some(example) = meaningful_field_example(
                prop_name,
                &Value::Mapping(prop.clone()),
                &schemas_snapshot,
            ) {
                prop.insert(val_s("example"), example);
            }
        }
    });
}

/// Generate a meaningful example for a field, returning `None` for generic defaults.
///
/// Unlike [`generate_field_example`] (which always returns a value, including a
/// generic `"string"` fallback), this only returns examples that add real
/// documentation value: name-based heuristics, enum values, UUIDs, dates,
/// booleans, integers, etc.
fn meaningful_field_example(
    name: &str,
    prop: &Value,
    schemas: &serde_yaml_ng::Mapping,
) -> Option<Value> {
    let map = prop.as_mapping();

    let field_type = map
        .and_then(|m| m.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let format = map
        .and_then(|m| m.get("format"))
        .and_then(Value::as_str)
        .unwrap_or("");

    // UUID
    if format == "uuid" {
        return Some(val_s(UUID_EXAMPLE));
    }

    // Date-time
    if format == "date-time" {
        return Some(val_s("2026-01-15T09:30:00Z"));
    }

    // Field mask
    if format == "field-mask" {
        return example_from_field_name(name).or_else(|| Some(val_s("name,email")));
    }

    // Enum — first non-unspecified value
    if let Some(enum_vals) = map.and_then(|m| m.get("enum")).and_then(Value::as_sequence) {
        return enum_vals
            .iter()
            .find(|v| v.as_str().is_none_or(|s| s != "unspecified"))
            .or_else(|| enum_vals.first())
            .cloned();
    }

    // Boolean
    if field_type == "boolean" {
        return Some(Value::Bool(true));
    }

    // Integer
    if field_type == "integer" {
        let min = map
            .and_then(|m| m.get("minimum"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        return Some(Value::Number(min.into()));
    }

    // Array — only produce examples when items have meaningful values
    // (enums, UUIDs, dates, etc.). Skip $ref arrays (referenced schema has
    // its own examples) and plain-string arrays (["string"] is noise).
    if field_type == "array" {
        if let Some(items) = map.and_then(|m| m.get("items")) {
            let is_ref = items
                .as_mapping()
                .is_some_and(|m| m.contains_key("$ref") || m.contains_key("allOf"));
            if is_ref {
                return None;
            }
            let item_example = meaningful_field_example("item", items, schemas)?;
            return Some(Value::Sequence(vec![item_example]));
        }
        return None;
    }

    // additionalProperties map
    if map.and_then(|m| m.get("additionalProperties")).is_some() {
        let mut obj = serde_yaml_ng::Mapping::new();
        obj.insert(val_s("key"), val_s("value"));
        return Some(Value::Mapping(obj));
    }

    // Name-based heuristics (only meaningful matches, None for unknowns)
    example_from_field_name(name)
}

/// Remove component schemas that are no longer referenced from outside
/// `components/schemas`.
///
/// Uses a reachability analysis: first collects "root" schemas referenced
/// directly from paths, responses, parameters, etc. Then transitively follows
/// `$ref` chains within `components/schemas` to find all reachable schemas.
/// Any schema not in this reachable set is an orphan — including
/// self-referential clusters like `google.rpc.Status` ↔ `google.protobuf.Any`
/// that have no external consumers.
pub fn remove_orphaned_schemas(doc: &mut Value) {
    let all_names: Vec<String> = schemas(doc)
        .map(|schema_map| {
            schema_map
                .keys()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if all_names.is_empty() {
        return;
    }

    // Step 1: collect $refs from everywhere EXCEPT components.schemas
    let external_refs = collect_external_schema_refs(doc);

    // Step 2: seed the reachable set with externally-referenced schema names
    let prefix = "#/components/schemas/";
    let mut reachable: std::collections::HashSet<String> = external_refs
        .iter()
        .filter_map(|r| r.strip_prefix(prefix).map(str::to_string))
        .collect();

    // Step 3: transitively follow $refs inside reachable schemas
    let schemas_snapshot: serde_yaml_ng::Mapping = schemas(doc).cloned().unwrap_or_default();
    let mut frontier: Vec<String> = reachable.iter().cloned().collect();
    while let Some(name) = frontier.pop() {
        if let Some(schema_val) = schemas_snapshot.get(name.as_str()) {
            let mut inner_refs = std::collections::HashSet::new();
            collect_refs(schema_val, &mut inner_refs);
            for r in inner_refs {
                if let Some(dep) = r.strip_prefix(prefix) {
                    if reachable.insert(dep.to_string()) {
                        frontier.push(dep.to_string());
                    }
                }
            }
        }
    }

    // Step 4: remove unreachable schemas
    let orphans: Vec<String> = all_names
        .into_iter()
        .filter(|name| !reachable.contains(name.as_str()))
        .collect();

    if !orphans.is_empty() {
        if let Some(schema_map) = schemas_mut(doc) {
            for name in &orphans {
                schema_map.remove(name.as_str());
            }
        }
    }
}

/// Collect `$ref` strings from every part of the document EXCEPT
/// `components.schemas`.
///
/// This lets [`remove_orphaned_schemas`] detect self-referential schema
/// clusters that have no external consumers (e.g., `google.rpc.Status` →
/// `google.protobuf.Any` where neither is used by any path or response).
fn collect_external_schema_refs(doc: &Value) -> std::collections::HashSet<String> {
    let mut refs = std::collections::HashSet::new();

    let Some(root) = doc.as_mapping() else {
        return refs;
    };

    for (key, value) in root {
        let key_str = key.as_str().unwrap_or_default();
        if key_str == "components" {
            // Walk components but skip the `schemas` sub-key
            if let Some(comp_map) = value.as_mapping() {
                for (comp_key, comp_val) in comp_map {
                    if comp_key.as_str() != Some("schemas") {
                        collect_refs(comp_val, &mut refs);
                    }
                }
            }
        } else {
            collect_refs(value, &mut refs);
        }
    }

    refs
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
        assert!(
            !vals
                .iter()
                .any(|v| v.as_str().unwrap().contains("UNSPECIFIED"))
        );
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
        let media_type =
            doc["paths"]["/v1/auth"]["post"]["requestBody"]["content"]["application/json"]
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

        // Examples should be on individual properties, not media-type level
        assert!(
            !media_type.contains_key("example"),
            "media-type-level example should not be present"
        );
        let email_prop = schema
            .get("properties")
            .unwrap()
            .as_mapping()
            .unwrap()
            .get("email")
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(
            email_prop.contains_key("example"),
            "property-level example should be present"
        );
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
        assert!(
            op.get("deprecated").is_none(),
            "unimplemented ops should not be marked deprecated"
        );
        assert!(
            op.get("description")
                .unwrap()
                .as_str()
                .unwrap()
                .starts_with("⚠️")
        );
        assert!(
            op.get("responses")
                .unwrap()
                .as_mapping()
                .unwrap()
                .contains_key("501")
        );
    }

    #[test]
    fn deprecated_operations_marked() {
        let yaml = r"
paths:
  /v1/old:
    get:
      operationId: OldService_GetOld
      responses:
        '200':
          description: OK
  /v1/new:
    get:
      operationId: NewService_GetNew
      responses:
        '200':
          description: OK
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        mark_deprecated_operations(&mut doc, &["OldService_GetOld".to_string()]);

        let old_op = doc["paths"]["/v1/old"]["get"].as_mapping().unwrap();
        assert!(old_op.get("deprecated").unwrap().as_bool().unwrap());

        let new_op = doc["paths"]["/v1/new"]["get"].as_mapping().unwrap();
        assert!(
            new_op.get("deprecated").is_none(),
            "non-deprecated op should not be marked"
        );
    }

    #[test]
    fn field_example_locale() {
        assert_eq!(
            example_from_field_name("locale").unwrap().as_str().unwrap(),
            "en-US"
        );
    }

    #[test]
    fn field_example_timezone() {
        assert_eq!(
            example_from_field_name("timezone")
                .unwrap()
                .as_str()
                .unwrap(),
            "America/New_York"
        );
        assert_eq!(
            example_from_field_name("time_zone")
                .unwrap()
                .as_str()
                .unwrap(),
            "America/New_York"
        );
    }

    #[test]
    fn field_example_language_and_country() {
        assert_eq!(
            example_from_field_name("language")
                .unwrap()
                .as_str()
                .unwrap(),
            "en"
        );
        assert_eq!(
            example_from_field_name("lang").unwrap().as_str().unwrap(),
            "en"
        );
        assert_eq!(
            example_from_field_name("country")
                .unwrap()
                .as_str()
                .unwrap(),
            "US"
        );
    }

    #[test]
    fn field_example_idempotency_key() {
        let val = example_from_field_name("idempotencyKey")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(val, UUID_EXAMPLE);
        let val2 = example_from_field_name("request_id")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(val2, UUID_EXAMPLE);
    }

    #[test]
    fn field_example_description_title() {
        assert_eq!(
            example_from_field_name("description")
                .unwrap()
                .as_str()
                .unwrap(),
            "A brief description"
        );
        assert_eq!(
            example_from_field_name("title").unwrap().as_str().unwrap(),
            "Example Title"
        );
        assert_eq!(
            example_from_field_name("subject")
                .unwrap()
                .as_str()
                .unwrap(),
            "Example Title"
        );
    }

    #[test]
    fn field_example_hostname_ip() {
        assert_eq!(
            example_from_field_name("hostname")
                .unwrap()
                .as_str()
                .unwrap(),
            "api.example.com"
        );
        assert_eq!(
            example_from_field_name("host").unwrap().as_str().unwrap(),
            "api.example.com"
        );
        assert_eq!(
            example_from_field_name("ip").unwrap().as_str().unwrap(),
            "192.168.1.1"
        );
        assert_eq!(
            example_from_field_name("ip_address")
                .unwrap()
                .as_str()
                .unwrap(),
            "192.168.1.1"
        );
    }

    #[test]
    fn field_example_user_agent_content_type() {
        assert_eq!(
            example_from_field_name("user_agent")
                .unwrap()
                .as_str()
                .unwrap(),
            "Mozilla/5.0 (compatible)"
        );
        assert_eq!(
            example_from_field_name("content_type")
                .unwrap()
                .as_str()
                .unwrap(),
            "application/json"
        );
        assert_eq!(
            example_from_field_name("mediaType")
                .unwrap()
                .as_str()
                .unwrap(),
            "application/json"
        );
    }

    #[test]
    fn field_example_etag() {
        let val = example_from_field_name("etag")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert!(val.starts_with('"') && val.ends_with('"'));
    }

    #[test]
    fn field_example_unknown_returns_none() {
        assert!(example_from_field_name("foobar").is_none());
        assert!(example_from_field_name("xyzzy").is_none());
    }

    #[test]
    fn schema_examples_enriched() {
        let yaml = r"
components:
  schemas:
    test.v1.Request:
      type: object
      properties:
        email:
          type: string
        password:
          type: string
        name:
          type: string
        status:
          type: string
          enum:
            - active
            - suspended
        active:
          type: boolean
        count:
          type: integer
          format: int32
        userId:
          type: string
          format: uuid
          example: 550e8400-e29b-41d4-a716-446655440000
        nested:
          allOf:
          - $ref: '#/components/schemas/test.v1.Nested'
        items:
          type: array
          items:
            type: string
        unknownField:
          type: string
    test.v1.Nested:
      type: object
      properties:
        value:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        enrich_schema_examples(&mut doc);

        let props = doc["components"]["schemas"]["test.v1.Request"]["properties"]
            .as_mapping()
            .unwrap();

        // Name-based heuristic: email
        assert_eq!(
            props["email"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_str()
                .unwrap(),
            "user@example.com"
        );

        // Name-based heuristic: password
        assert_eq!(
            props["password"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_str()
                .unwrap(),
            "P@ssw0rd123!"
        );

        // Name-based heuristic: name
        assert_eq!(
            props["name"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_str()
                .unwrap(),
            "John Doe"
        );

        // Enum: first value
        assert_eq!(
            props["status"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_str()
                .unwrap(),
            "active"
        );

        // Boolean
        assert!(
            props["active"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_bool()
                .unwrap()
        );

        // Integer
        assert_eq!(
            props["count"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_u64()
                .unwrap(),
            0
        );

        // Existing example preserved (not overwritten)
        assert_eq!(
            props["userId"]
                .as_mapping()
                .unwrap()
                .get("example")
                .unwrap()
                .as_str()
                .unwrap(),
            UUID_EXAMPLE
        );

        // allOf ref: no example added
        assert!(
            !props["nested"]
                .as_mapping()
                .unwrap()
                .contains_key("example"),
            "allOf properties should not get examples"
        );

        // Plain string array: no example (avoids ["string"] noise)
        assert!(
            props["items"]
                .as_mapping()
                .unwrap()
                .get("example")
                .is_none(),
            "plain string array should NOT get example"
        );

        // Unknown string field: no example (no heuristic match)
        assert!(
            !props["unknownField"]
                .as_mapping()
                .unwrap()
                .contains_key("example"),
            "unknown string fields should not get generic examples"
        );
    }

    #[test]
    fn operation_summaries_populated() {
        let yaml = r"
paths:
  /v1/users:
    get:
      operationId: UserService_ListUsers
      description: |
        List all users in the system.
        Supports filtering and pagination.
    post:
      operationId: UserService_CreateUser
      description: Create a new user.
  /v1/sessions:
    delete:
      operationId: AuthService_SignOut
      summary: Sign out
      description: |
        Invalidate the current session.
        Clears all tokens.
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        populate_operation_summaries(&mut doc);

        // Summary extracted from first line (trailing period stripped)
        let list_op = doc["paths"]["/v1/users"]["get"].as_mapping().unwrap();
        assert_eq!(
            list_op.get("summary").unwrap().as_str().unwrap(),
            "List all users in the system"
        );

        // Single-line description, period stripped
        let create_op = doc["paths"]["/v1/users"]["post"].as_mapping().unwrap();
        assert_eq!(
            create_op.get("summary").unwrap().as_str().unwrap(),
            "Create a new user"
        );

        // Existing summary preserved
        let signout_op = doc["paths"]["/v1/sessions"]["delete"].as_mapping().unwrap();
        assert_eq!(
            signout_op.get("summary").unwrap().as_str().unwrap(),
            "Sign out"
        );
    }

    #[test]
    fn operation_summary_skipped_without_description() {
        let yaml = r"
paths:
  /v1/health:
    get:
      operationId: HealthService_Check
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        populate_operation_summaries(&mut doc);

        let op = doc["paths"]["/v1/health"]["get"].as_mapping().unwrap();
        assert!(
            op.get("summary").is_none(),
            "summary should not be added when description is absent"
        );
    }

    #[test]
    fn unspecified_stripped_from_component_schemas() {
        let yaml = r"
components:
  schemas:
    test.v1.Response:
      type: object
      properties:
        status:
          type: string
          enum:
            - unspecified
            - active
            - suspended
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        strip_unspecified_from_query_enums(&mut doc);

        let vals = doc["components"]["schemas"]["test.v1.Response"]["properties"]["status"]["enum"]
            .as_sequence()
            .unwrap();
        assert_eq!(vals.len(), 2);
        assert!(
            !vals
                .iter()
                .any(|v| v.as_str().is_some_and(|s| s == "unspecified"))
        );
    }

    #[test]
    fn self_referential_cluster_removed() {
        let yaml = r#"
paths:
  /v1/test:
    get:
      operationId: TestService_Get
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/test.v1.Response'
components:
  schemas:
    test.v1.Response:
      type: object
      properties:
        name:
          type: string
    google.rpc.Status:
      type: object
      properties:
        code:
          type: integer
        details:
          type: array
          items:
            $ref: '#/components/schemas/google.protobuf.Any'
    google.protobuf.Any:
      type: object
      properties:
        '@type':
          type: string
      additionalProperties: true
"#;
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_orphaned_schemas(&mut doc);

        let schema_map = schemas(&doc).unwrap();
        assert!(
            schema_map.contains_key("test.v1.Response"),
            "externally-referenced schema should survive"
        );
        assert!(
            !schema_map.contains_key("google.rpc.Status"),
            "self-referential cluster member should be removed"
        );
        assert!(
            !schema_map.contains_key("google.protobuf.Any"),
            "self-referential cluster member should be removed"
        );
    }

    #[test]
    fn orphan_removal_preserves_cross_component_refs() {
        // A schema referenced from a response component (not paths) should survive.
        let yaml = r#"
paths:
  /v1/test:
    get:
      responses:
        '200':
          $ref: '#/components/responses/OkResponse'
components:
  responses:
    OkResponse:
      description: OK
      content:
        application/json:
          schema:
            $ref: '#/components/schemas/test.v1.Data'
  schemas:
    test.v1.Data:
      type: object
      properties:
        id:
          type: string
    test.v1.Orphan:
      type: object
      properties:
        unused:
          type: string
"#;
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_orphaned_schemas(&mut doc);

        let schema_map = schemas(&doc).unwrap();
        assert!(
            schema_map.contains_key("test.v1.Data"),
            "schema referenced from response component should survive"
        );
        assert!(
            !schema_map.contains_key("test.v1.Orphan"),
            "unreferenced schema should be removed"
        );
    }

    #[test]
    fn orphan_removal_preserves_transitive_schema_refs() {
        // Schema A is referenced from a path. Schema A references Schema B
        // via allOf. Schema B should NOT be removed as an orphan.
        let yaml = r#"
paths:
  /v1/test:
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/test.v1.Parent'
      responses:
        '200':
          description: OK
components:
  schemas:
    test.v1.Parent:
      type: object
      properties:
        child:
          allOf:
          - $ref: '#/components/schemas/test.v1.Child'
    test.v1.Child:
      type: object
      properties:
        name:
          type: string
    test.v1.Orphan:
      type: object
      properties:
        unused:
          type: string
"#;
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_orphaned_schemas(&mut doc);

        let schema_map = schemas(&doc).unwrap();
        assert!(
            schema_map.contains_key("test.v1.Parent"),
            "directly-referenced schema should survive"
        );
        assert!(
            schema_map.contains_key("test.v1.Child"),
            "transitively-referenced schema should survive"
        );
        assert!(
            !schema_map.contains_key("test.v1.Orphan"),
            "unreferenced schema should be removed"
        );
    }

    #[test]
    fn array_ref_items_skip_example_in_enrichment() {
        let yaml = r#"
components:
  schemas:
    test.v1.Parent:
      type: object
      properties:
        children:
          type: array
          items:
            $ref: '#/components/schemas/test.v1.Child'
        tags:
          type: array
          items:
            type: string
    test.v1.Child:
      type: object
      properties:
        name:
          type: string
"#;
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        enrich_schema_examples(&mut doc);

        let props = doc["components"]["schemas"]["test.v1.Parent"]["properties"]
            .as_mapping()
            .unwrap();

        // Array with $ref items: should NOT get an example (avoid ["string"] noise)
        assert!(
            !props["children"]
                .as_mapping()
                .unwrap()
                .contains_key("example"),
            "array with $ref items should not get example"
        );

        // Array with plain string items: should NOT get an example (avoids ["string"] noise)
        assert!(
            !props["tags"].as_mapping().unwrap().contains_key("example"),
            "plain string array should not get [\"string\"] example"
        );
    }

    #[test]
    fn code_field_is_ambiguous_returns_none() {
        // The bare `code` field name is ambiguous (OAuth code, error code,
        // verification code) — should not get a heuristic example.
        assert!(
            example_from_field_name("code").is_none(),
            "bare 'code' should not match a heuristic"
        );
    }

    #[test]
    fn secret_field_is_ambiguous_returns_none() {
        // `secret` alone is ambiguous (TOTP secret, API secret, etc.)
        assert!(
            example_from_field_name("secret").is_none(),
            "bare 'secret' should not match password heuristic"
        );
    }

    #[test]
    fn password_field_still_matches() {
        assert_eq!(
            example_from_field_name("password")
                .unwrap()
                .as_str()
                .unwrap(),
            "P@ssw0rd123!"
        );
        assert_eq!(
            example_from_field_name("newPassword")
                .unwrap()
                .as_str()
                .unwrap(),
            "N3wP@ssw0rd!456"
        );
        assert_eq!(
            example_from_field_name("currentPassword")
                .unwrap()
                .as_str()
                .unwrap(),
            "P@ssw0rd123!"
        );
    }

    #[test]
    fn device_field_heuristics() {
        assert_eq!(
            example_from_field_name("deviceId")
                .unwrap()
                .as_str()
                .unwrap(),
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
        );
        assert_eq!(
            example_from_field_name("device_id")
                .unwrap()
                .as_str()
                .unwrap(),
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
        );
        assert_eq!(
            example_from_field_name("deviceName")
                .unwrap()
                .as_str()
                .unwrap(),
            "iPhone 15 Pro"
        );
        assert_eq!(
            example_from_field_name("deviceType")
                .unwrap()
                .as_str()
                .unwrap(),
            "mobile"
        );
    }

    #[test]
    fn ip_country_heuristic() {
        assert_eq!(
            example_from_field_name("ipCountry")
                .unwrap()
                .as_str()
                .unwrap(),
            "US"
        );
        assert_eq!(
            example_from_field_name("ip_country")
                .unwrap()
                .as_str()
                .unwrap(),
            "US"
        );
    }

    #[test]
    fn ip_created_by_heuristic() {
        assert_eq!(
            example_from_field_name("ipCreatedBy")
                .unwrap()
                .as_str()
                .unwrap(),
            "192.168.1.1"
        );
        assert_eq!(
            example_from_field_name("ip_created_by")
                .unwrap()
                .as_str()
                .unwrap(),
            "192.168.1.1"
        );
    }

    #[test]
    fn installation_id_heuristic() {
        assert_eq!(
            example_from_field_name("installationId")
                .unwrap()
                .as_str()
                .unwrap(),
            UUID_EXAMPLE
        );
    }

    #[test]
    fn verification_code_specific_names() {
        assert_eq!(
            example_from_field_name("otp").unwrap().as_str().unwrap(),
            "123456"
        );
        assert_eq!(
            example_from_field_name("verificationCode")
                .unwrap()
                .as_str()
                .unwrap(),
            "123456"
        );
        assert_eq!(
            example_from_field_name("mfaCode")
                .unwrap()
                .as_str()
                .unwrap(),
            "123456"
        );
    }

    #[test]
    fn empty_inlined_bodies_removed_unconditionally() {
        let yaml = r"
paths:
  /v1/confirm:
    post:
      operationId: TestService_Confirm
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties: {}
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        remove_empty_inlined_request_bodies(&mut doc);

        let op = doc["paths"]["/v1/confirm"]["post"].as_mapping().unwrap();
        assert!(
            !op.contains_key("requestBody"),
            "empty inlined request body should be removed"
        );
    }

    #[test]
    fn plain_string_array_skips_example() {
        let yaml = r"
components:
  schemas:
    test.v1.Req:
      type: object
      properties:
        scopes:
          type: array
          items:
            type: string
        tags:
          type: array
          items:
            enum:
              - a
              - b
            type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        enrich_schema_examples(&mut doc);

        let scopes = &doc["components"]["schemas"]["test.v1.Req"]["properties"]["scopes"];
        assert!(
            scopes.as_mapping().unwrap().get("example").is_none(),
            "plain string array should NOT get [\"string\"] example"
        );

        let tags = &doc["components"]["schemas"]["test.v1.Req"]["properties"]["tags"];
        let tag_ex = tags["example"].as_sequence().unwrap();
        assert_eq!(tag_ex[0].as_str().unwrap(), "a");
    }

    #[test]
    fn inline_body_properties_enriched() {
        let yaml = r"
paths:
  /v1/users/{id}:
    patch:
      operationId: UserService_Update
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                name:
                  type: string
                  maxLength: 255
                email:
                  type: string
                  maxLength: 254
                password:
                  type: string
                  writeOnly: true
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        enrich_inline_request_body_examples(&mut doc);

        let schema = &doc["paths"]["/v1/users/{id}"]["patch"]["requestBody"]["content"]["application/json"]
            ["schema"]["properties"];

        assert_eq!(
            schema["name"]["example"].as_str().unwrap(),
            "John Doe",
            "name should get heuristic example"
        );
        assert_eq!(
            schema["email"]["example"].as_str().unwrap(),
            "user@example.com",
            "email should get heuristic example"
        );
        assert_eq!(
            schema["password"]["example"].as_str().unwrap(),
            "P@ssw0rd123!",
            "password should get heuristic example"
        );
    }

    #[test]
    fn inline_body_skips_ref_bodies() {
        let yaml = r#"
paths:
  /v1/auth/login:
    post:
      operationId: AuthService_Login
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/auth.v1.LoginRequest'
components:
  schemas:
    auth.v1.LoginRequest:
      type: object
      properties:
        email:
          type: string
"#;
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        enrich_inline_request_body_examples(&mut doc);

        // The $ref body should not be touched (it doesn't have inline properties)
        let body_schema = &doc["paths"]["/v1/auth/login"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"];
        assert!(
            body_schema.as_mapping().unwrap().contains_key("$ref"),
            "should remain a $ref, not be modified"
        );
        // Named schema should NOT have been enriched by this function
        let email_prop =
            &doc["components"]["schemas"]["auth.v1.LoginRequest"]["properties"]["email"];
        assert!(
            email_prop.as_mapping().unwrap().get("example").is_none(),
            "enrich_inline_request_body_examples should not touch named schemas"
        );
    }
}
