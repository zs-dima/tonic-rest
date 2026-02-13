//! Validation constraint transforms.
//!
//! - Inject `validate.rules` → JSON Schema constraints
//! - Flatten UUID wrapper `$ref` to inline `type: string, format: uuid`
//! - Flatten UUID path template variables (remove `.value` suffix)
//! - Simplify UUID query parameters from dot-notation
//! - Strip path-bound fields from request body schemas
//! - Enrich path parameters with proto constraints
//! - Annotate `writeOnly`/`readOnly` fields based on naming conventions
//! - Annotate `google.protobuf.Duration` fields with format and example

use serde_yaml_ng::Value;

use crate::discover::{PathParamInfo, SchemaConstraints};

use super::helpers::{
    UUID_EXAMPLE, UUID_PATTERN, for_each_operation, schemas_mut, snake_to_lower_camel_dotted,
    val_i64, val_n, val_s,
};

/// Flatten UUID wrapper references to inline `type: string, format: uuid`.
pub fn flatten_uuid_refs(doc: &mut Value, uuid_schema: Option<&str>) {
    let Some(uuid_schema_name) = uuid_schema else {
        return;
    };
    let uuid_ref = format!("#/components/schemas/{uuid_schema_name}");

    if let Some(schemas) = schemas_mut(doc) {
        let schema_names: Vec<String> = schemas
            .iter()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .collect();

        for name in &schema_names {
            if let Some(schema) = schemas
                .get_mut(name.as_str())
                .and_then(Value::as_mapping_mut)
            {
                flatten_uuid_in_properties(schema, &uuid_ref);
            }
        }

        schemas.remove(uuid_schema_name);
    }
}

/// Replace UUID `$ref` / `allOf` references in a schema's properties with inline string.
fn flatten_uuid_in_properties(schema: &mut serde_yaml_ng::Mapping, uuid_ref: &str) {
    let Some(props) = schema.get_mut("properties").and_then(Value::as_mapping_mut) else {
        return;
    };

    let prop_names: Vec<String> = props
        .iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect();

    for prop_name in &prop_names {
        let Some(prop) = props.get_mut(prop_name.as_str()) else {
            continue;
        };

        if !is_uuid_reference(prop, uuid_ref) {
            continue;
        }

        let description = prop
            .as_mapping()
            .and_then(|m| m.get("description"))
            .and_then(Value::as_str)
            .map(str::to_string);

        let mut replacement = serde_yaml_ng::Mapping::new();
        replacement.insert(val_s("type"), val_s("string"));
        replacement.insert(val_s("format"), val_s("uuid"));
        replacement.insert(val_s("pattern"), val_s(UUID_PATTERN));
        replacement.insert(val_s("example"), val_s(UUID_EXAMPLE));
        if let Some(desc) = description {
            replacement.insert(val_s("description"), val_s(&desc));
        }

        *prop = Value::Mapping(replacement);
    }
}

/// Check if a property value references the UUID schema.
fn is_uuid_reference(prop: &Value, uuid_ref: &str) -> bool {
    let Some(map) = prop.as_mapping() else {
        return false;
    };

    if map
        .get("$ref")
        .and_then(Value::as_str)
        .is_some_and(|r| r == uuid_ref)
    {
        return true;
    }

    if let Some(all_of) = map.get("allOf").and_then(Value::as_sequence) {
        return all_of.iter().any(|item| {
            item.as_mapping()
                .and_then(|m| m.get("$ref"))
                .and_then(Value::as_str)
                .is_some_and(|r| r == uuid_ref)
        });
    }

    false
}

/// Simplify UUID-typed query parameters from dot-notation to flat names.
pub fn simplify_uuid_query_params(doc: &mut Value) {
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

            let is_query = p
                .get("in")
                .and_then(Value::as_str)
                .is_some_and(|v| v == "query");

            if !is_query {
                continue;
            }

            let Some(name) = p.get("name").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };

            if let Some(base) = name.strip_suffix(".value") {
                p.insert(val_s("name"), val_s(base));
                p.insert(
                    val_s("description"),
                    val_s(&format!("UUID of the {}", base.replace("Id", ""))),
                );

                let mut schema = serde_yaml_ng::Mapping::new();
                schema.insert(val_s("type"), val_s("string"));
                schema.insert(val_s("format"), val_s("uuid"));
                schema.insert(val_s("pattern"), val_s(UUID_PATTERN));
                schema.insert(val_s("example"), val_s(UUID_EXAMPLE));
                p.insert(val_s("schema"), Value::Mapping(schema));
            }
        }
    });
}

/// Flatten UUID path template variables by stripping the `.value` suffix.
///
/// Rewrites path keys like `/v1/users/{userId.value}` to `/v1/users/{userId}`
/// and updates the corresponding path parameter `name` fields.
pub fn flatten_uuid_path_templates(doc: &mut Value) {
    let Some(paths) = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("paths"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    // Collect path keys that need rewriting
    let rewrites: Vec<(String, String)> = paths
        .iter()
        .filter_map(|(k, _)| {
            let path = k.as_str()?;
            if path.contains(".value}") {
                let new_path = path.replace(".value}", "}");
                Some((path.to_string(), new_path))
            } else {
                None
            }
        })
        .collect();

    for (old_path, new_path) in &rewrites {
        let old_key = Value::String(old_path.clone());
        if let Some(path_item) = paths.remove(&old_key) {
            let mut path_item = path_item;

            // Update parameter names inside each operation
            if let Some(path_map) = path_item.as_mapping_mut() {
                for (_, op_val) in path_map.iter_mut() {
                    let Some(op) = op_val.as_mapping_mut() else {
                        continue;
                    };
                    let Some(params) = op.get_mut("parameters").and_then(Value::as_sequence_mut)
                    else {
                        continue;
                    };
                    for param in params.iter_mut() {
                        let Some(p) = param.as_mapping_mut() else {
                            continue;
                        };
                        let is_path = p
                            .get("in")
                            .and_then(Value::as_str)
                            .is_some_and(|v| v == "path");
                        if !is_path {
                            continue;
                        }
                        if let Some(name) =
                            p.get("name").and_then(Value::as_str).map(str::to_string)
                        {
                            if let Some(base) = name.strip_suffix(".value") {
                                p.insert(val_s("name"), val_s(base));
                            }
                        }
                    }
                }
            }

            paths.insert(Value::String(new_path.clone()), path_item);
        }
    }
}

/// Check if a lowercased camelCase field name represents a write-only secret.
///
/// Uses suffix/exact matching instead of substring to avoid false positives:
/// - `"password"` → true (exact match — the field IS a password)
/// - `"currentpassword"` → true (suffix match — ends with "password")
/// - `"haspassword"` → false (boolean flag, not a secret value)
///
/// The heuristic: if the camelCase word before "password" is a verb/adjective
/// that modifies the password (new, current, old, client), it's still a secret.
/// If it's a boolean prefix (has, is, needs), it's a flag about the password,
/// not the password itself.
fn is_write_only_field(lower: &str) -> bool {
    const SECRETS: &[&str] = &["password", "secret", "credential"];
    const BOOL_PREFIXES: &[&str] = &["has", "is", "needs", "requires", "supports"];

    for secret in SECRETS {
        if lower == *secret {
            return true;
        }
        if let Some(prefix) = lower.strip_suffix(secret) {
            // Check if the prefix is a boolean indicator
            if BOOL_PREFIXES.contains(&prefix) {
                return false;
            }
            return true;
        }
    }
    false
}

/// Annotate schema fields with `writeOnly` or `readOnly` based on naming conventions.
///
/// Convention defaults:
/// - `writeOnly: true` — field names that ARE or END WITH `password`, `secret`, or `credential`
///   (e.g., `password`, `currentPassword`, `clientSecret` — but NOT `hasPassword`)
/// - `readOnly: true` — field names ending with `At` (e.g., `createdAt`, `updatedAt`)
///
/// Additional patterns from `extra_write_only` / `extra_read_only` are matched
/// as case-insensitive substrings.
pub fn annotate_field_access(
    doc: &mut Value,
    extra_write_only: &[String],
    extra_read_only: &[String],
) {
    let Some(schemas) = schemas_mut(doc) else {
        return;
    };

    let schema_names: Vec<String> = schemas
        .iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect();

    for name in &schema_names {
        let Some(props) = schemas
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

            let lower = prop_name.to_lowercase();

            // Suffix-match: `password` matches "password", "currentPassword",
            // "newPassword" but NOT "hasPassword" (boolean flag, not a secret).
            // For camelCase: check if the word appears as a suffix or the entire name.
            let is_write_only = is_write_only_field(&lower)
                || extra_write_only
                    .iter()
                    .any(|p| lower.contains(&p.to_lowercase()));

            let is_read_only = prop_name.ends_with("At")
                || prop_name.ends_with("_at")
                || extra_read_only
                    .iter()
                    .any(|p| lower.contains(&p.to_lowercase()));

            // Skip writeOnly on response schemas — fields like
            // `SetupMfaResponse.secret` must be returned to the client.
            let is_response_schema =
                name.contains("Response") || name.contains("Reply") || name.contains("Result");

            if is_write_only && !is_response_schema {
                prop.insert(val_s("writeOnly"), Value::Bool(true));
            } else if is_read_only {
                prop.insert(val_s("readOnly"), Value::Bool(true));
            }
        }
    }
}

/// Annotate `google.protobuf.Duration` fields with format and example.
///
/// Detects Duration fields by schema name pattern (`Duration` suffix) and
/// by property `pattern` matching the proto Duration regex. Adds
/// `example: "300s"` and enriches the description.
pub fn annotate_duration_fields(doc: &mut Value) {
    let Some(schemas) = schemas_mut(doc) else {
        return;
    };

    // Detect Duration schema names (e.g., "google.protobuf.Duration")
    // Match fully-qualified proto Duration names (e.g., "google.protobuf.Duration")
    // using `.Duration` suffix to avoid false positives on user schemas like
    // "SessionDuration". Plain "Duration" (no package prefix) is also accepted.
    let duration_schema_names: Vec<String> = schemas
        .iter()
        .filter_map(|(k, _)| {
            let name = k.as_str()?;
            if name == "Duration" || name.ends_with(".Duration") {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect();

    let duration_refs: Vec<String> = duration_schema_names
        .iter()
        .map(|n| format!("#/components/schemas/{n}"))
        .collect();

    // Rewrite the Duration schemas themselves to string type
    for dur_name in &duration_schema_names {
        if let Some(dur_schema) = schemas
            .get_mut(dur_name.as_str())
            .and_then(Value::as_mapping_mut)
        {
            dur_schema.remove("properties");
            dur_schema.insert(val_s("type"), val_s("string"));
            dur_schema.insert(val_s("example"), val_s("300s"));
            if !dur_schema.contains_key("description") {
                dur_schema.insert(
                    val_s("description"),
                    val_s("Duration in seconds with 's' suffix (e.g., \"300s\")."),
                );
            }
        }
    }

    // Walk all schemas and annotate Duration-typed properties
    let schema_names: Vec<String> = schemas
        .iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_string))
        .collect();

    for name in &schema_names {
        let Some(props) = schemas
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

            let is_duration_allof =
                prop.get("allOf")
                    .and_then(Value::as_sequence)
                    .is_some_and(|seq| {
                        seq.iter().any(|item| {
                            item.as_mapping()
                                .and_then(|m| m.get("$ref"))
                                .and_then(Value::as_str)
                                .is_some_and(|r| duration_refs.iter().any(|dr| dr == r))
                        })
                    });

            let is_duration_pattern = prop
                .get("pattern")
                .and_then(Value::as_str)
                .is_some_and(|p| p.contains("0-9") && p.contains('s'));

            if is_duration_allof || is_duration_pattern {
                // Replace allOf wrappers or pattern-based Duration fields
                // with inline string type. Simple $ref properties are left
                // untouched since the Duration schema itself is rewritten.
                prop.remove("$ref");
                prop.remove("allOf");
                prop.insert(val_s("type"), val_s("string"));
                prop.insert(val_s("example"), val_s("300s"));
                if !prop.contains_key("description") {
                    prop.insert(
                        val_s("description"),
                        val_s("Duration in seconds with 's' suffix (e.g., \"300s\")."),
                    );
                }
            }
        }
    }
}

/// Inject validation constraints into component schemas.
pub fn inject_validation_constraints(doc: &mut Value, constraints: &[SchemaConstraints]) {
    let Some(schemas) = schemas_mut(doc) else {
        return;
    };

    for sc in constraints {
        let Some(schema_map) = schemas
            .get_mut(sc.schema.as_str())
            .and_then(Value::as_mapping_mut)
        else {
            continue;
        };

        let Some(props) = schema_map
            .get_mut("properties")
            .and_then(Value::as_mapping_mut)
        else {
            continue;
        };

        let required_fields: Vec<&str> = sc
            .fields
            .iter()
            .filter(|f| f.required)
            .map(|f| f.field.as_str())
            .collect();

        for fc in &sc.fields {
            let Some(prop) = props
                .get_mut(fc.field.as_str())
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            if fc.is_numeric {
                prop.insert(val_s("type"), val_s("integer"));
                prop.remove("format");

                if let Some(v) = fc.signed_min {
                    prop.insert(val_s("minimum"), val_i64(v));
                } else if let Some(v) = fc.min {
                    prop.insert(val_s("minimum"), val_n(v));
                }
                if let Some(v) = fc.signed_max {
                    prop.insert(val_s("maximum"), val_i64(v));
                } else if let Some(v) = fc.max {
                    prop.insert(val_s("maximum"), val_n(v));
                }
            } else {
                if let Some(v) = fc.min {
                    prop.insert(val_s("minLength"), val_n(v));
                }
                if let Some(v) = fc.max {
                    prop.insert(val_s("maxLength"), val_n(v));
                }
            }

            if let Some(pattern) = &fc.pattern {
                prop.insert(val_s("pattern"), val_s(pattern));
            }

            if !fc.enum_values.is_empty() {
                let variants: Vec<Value> = fc.enum_values.iter().map(|s| val_s(s)).collect();
                prop.insert(val_s("enum"), Value::Sequence(variants));
            }

            if fc.is_uuid {
                prop.insert(val_s("format"), val_s("uuid"));
                prop.insert(val_s("pattern"), val_s(UUID_PATTERN));
                prop.insert(val_s("example"), val_s(UUID_EXAMPLE));
            }
        }

        if !required_fields.is_empty() {
            let values: Vec<Value> = required_fields.iter().map(|f| val_s(f)).collect();
            schema_map.insert(val_s("required"), Value::Sequence(values));
        }
    }
}

/// Strip path-bound fields from request body schemas.
///
/// Instead of mutating shared component schemas globally (which would break
/// other operations referencing the same schema), this inlines a modified
/// copy of the schema into each operation that has path parameters.
/// Operations without path parameters keep referencing the original schema.
#[allow(clippy::too_many_lines)]
pub fn strip_path_fields_from_body(doc: &mut Value) {
    // Phase 1: collect operation locations and their path fields + schema refs
    struct StripInfo {
        path: String,
        method: String,
        schema_ref: String,
        fields_to_remove: Vec<String>,
    }

    let mut strip_ops: Vec<StripInfo> = Vec::new();

    if let Some(paths) = doc
        .as_mapping()
        .and_then(|m| m.get("paths"))
        .and_then(Value::as_mapping)
    {
        for (path_key, path_item) in paths {
            let Some(path_str) = path_key.as_str() else {
                continue;
            };
            let Some(path_map) = path_item.as_mapping() else {
                continue;
            };

            for (method_key, operation) in path_map {
                let Some(method_str) = method_key.as_str() else {
                    continue;
                };
                let Some(op_map) = operation.as_mapping() else {
                    continue;
                };

                let path_fields: Vec<String> = op_map
                    .get("parameters")
                    .and_then(Value::as_sequence)
                    .into_iter()
                    .flatten()
                    .filter_map(|p| {
                        let m = p.as_mapping()?;
                        let in_val = m.get("in")?.as_str()?;
                        if in_val == "path" {
                            let name = m.get("name")?.as_str()?;
                            let parent = name.split('.').next()?;
                            Some(snake_to_lower_camel_dotted(parent))
                        } else {
                            None
                        }
                    })
                    .collect();

                if path_fields.is_empty() {
                    continue;
                }

                if let Some(schema_ref) = op_map
                    .get("requestBody")
                    .and_then(Value::as_mapping)
                    .and_then(|rb| rb.get("content"))
                    .and_then(Value::as_mapping)
                    .and_then(|c| c.get("application/json"))
                    .and_then(Value::as_mapping)
                    .and_then(|mt| mt.get("schema"))
                    .and_then(Value::as_mapping)
                    .and_then(|s| s.get("$ref"))
                    .and_then(Value::as_str)
                {
                    strip_ops.push(StripInfo {
                        path: path_str.to_string(),
                        method: method_str.to_string(),
                        schema_ref: schema_ref.to_string(),
                        fields_to_remove: path_fields,
                    });
                }
            }
        }
    }

    if strip_ops.is_empty() {
        return;
    }

    // Phase 2: for each operation, clone the referenced schema and strip fields
    // inline (replacing the $ref with the modified schema), preserving the
    // original component schema for other consumers.
    for info in &strip_ops {
        let schema_name = info.schema_ref.trim_start_matches("#/components/schemas/");

        // Clone the component schema
        let original_schema = doc
            .as_mapping()
            .and_then(|m| m.get("components"))
            .and_then(Value::as_mapping)
            .and_then(|m| m.get("schemas"))
            .and_then(Value::as_mapping)
            .and_then(|m| m.get(schema_name))
            .cloned();

        let Some(mut schema) = original_schema else {
            continue;
        };

        // Strip path-bound fields from the cloned schema
        if let Some(schema_map) = schema.as_mapping_mut() {
            if let Some(props) = schema_map
                .get_mut("properties")
                .and_then(Value::as_mapping_mut)
            {
                for field in &info.fields_to_remove {
                    props.remove(field.as_str());
                }
            }

            if let Some(required) = schema_map
                .get_mut("required")
                .and_then(Value::as_sequence_mut)
            {
                required.retain(|v| {
                    v.as_str()
                        .is_none_or(|s| !info.fields_to_remove.iter().any(|f| f == s))
                });
            }

            if schema_map
                .get("required")
                .and_then(Value::as_sequence)
                .is_some_and(Vec::is_empty)
            {
                schema_map.remove("required");
            }
        }

        // Replace the $ref with the inlined and stripped schema
        if let Some(schema_slot) = doc
            .as_mapping_mut()
            .and_then(|m| m.get_mut("paths"))
            .and_then(Value::as_mapping_mut)
            .and_then(|m| m.get_mut(info.path.as_str()))
            .and_then(Value::as_mapping_mut)
            .and_then(|m| m.get_mut(info.method.as_str()))
            .and_then(Value::as_mapping_mut)
            .and_then(|op| op.get_mut("requestBody"))
            .and_then(Value::as_mapping_mut)
            .and_then(|rb| rb.get_mut("content"))
            .and_then(Value::as_mapping_mut)
            .and_then(|c| c.get_mut("application/json"))
            .and_then(Value::as_mapping_mut)
            .and_then(|mt| mt.get_mut("schema"))
        {
            *schema_slot = schema;
        }
    }
}

/// Normalize a path for matching by stripping `.value` suffixes from template
/// variables, removing underscores, and lowercasing.
///
/// This handles two sources of inconsistency:
/// 1. UUID flattening strips `.value}` → `}` (may or may not have run yet)
/// 2. gnostic uses `{user_id.value}` (snake_case) for compound vars but
///    `{deviceId}` (camelCase) for simple vars, while proto discovery always
///    uses camelCase. Stripping underscores makes `user_id` match `userId`.
fn normalize_path_for_match(path: &str) -> String {
    path.replace(".value}", "}").replace('_', "").to_lowercase()
}

/// Normalize a parameter name for matching by stripping the `.value` suffix,
/// removing underscores, and lowercasing.
fn normalize_name_for_match(name: &str) -> String {
    name.strip_suffix(".value")
        .unwrap_or(name)
        .replace('_', "")
        .to_lowercase()
}

/// Enrich path parameters with constraints from proto field definitions.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // proto type names, not file paths
pub fn enrich_path_params(doc: &mut Value, path_params: &[PathParamInfo]) {
    for_each_operation(doc, |path, _method, op_map| {
        let Some(params) = op_map
            .get_mut("parameters")
            .and_then(Value::as_sequence_mut)
        else {
            return;
        };

        // Match against proto-discovered path, normalizing for:
        // 1. UUID path flattening: `{user_id.value}` → `{user_id}` (Phase 8)
        // 2. Case differences: gnostic inconsistently camelCases template vars
        //    (`{deviceId}` for simple fields, `{user_id.value}` for compound)
        //    while proto discovery always camelCases (`{userId.value}`).
        let path_normalized = normalize_path_for_match(path);
        let proto_info = path_params
            .iter()
            .find(|pp| normalize_path_for_match(&pp.path) == path_normalized);

        for param in params.iter_mut() {
            let Some(p) = param.as_mapping_mut() else {
                continue;
            };

            let is_path = p
                .get("in")
                .and_then(Value::as_str)
                .is_some_and(|v| v == "path");

            if !is_path {
                continue;
            }

            let name = p
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            // Match constraint by normalized name: strip `.value` suffix and
            // compare case-insensitively to handle camelCase/snake_case differences.
            let name_normalized = normalize_name_for_match(&name);
            let constraint = proto_info.and_then(|pp| {
                pp.params
                    .iter()
                    .find(|c| normalize_name_for_match(&c.name) == name_normalized)
            });

            // UUID wrapper path params
            if constraint.is_some_and(|c| c.is_uuid) {
                let mut schema = serde_yaml_ng::Mapping::new();
                schema.insert(val_s("type"), val_s("string"));
                schema.insert(val_s("format"), val_s("uuid"));
                schema.insert(val_s("pattern"), val_s(UUID_PATTERN));
                schema.insert(val_s("example"), val_s(UUID_EXAMPLE));
                p.insert(val_s("schema"), Value::Mapping(schema));
                p.insert(val_s("description"), val_s("Resource UUID"));
                continue;
            }

            // String constraints from proto
            if let Some(c) = constraint {
                if c.min.is_some() || c.max.is_some() {
                    let mut schema = serde_yaml_ng::Mapping::new();
                    schema.insert(val_s("type"), val_s("string"));
                    if let Some(min) = c.min {
                        schema.insert(val_s("minLength"), val_n(min));
                    }
                    if let Some(max) = c.max {
                        schema.insert(val_s("maxLength"), val_n(max));
                    }
                    p.insert(val_s("schema"), Value::Mapping(schema));
                }

                if let Some(desc) = &c.description {
                    p.insert(val_s("description"), val_s(desc));
                }
            }

            // Enum path params: strip UNSPECIFIED values
            if let Some(schema) = p.get_mut("schema").and_then(Value::as_mapping_mut) {
                if let Some(enum_vals) = schema.get_mut("enum").and_then(Value::as_sequence_mut) {
                    enum_vals.retain(|v| v.as_str().is_none_or(|s| !s.ends_with("_UNSPECIFIED")));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::discover::FieldConstraint;

    use super::*;

    #[test]
    fn uuid_ref_flattened() {
        let yaml = r"
components:
  schemas:
    core.v1.UUID:
      type: object
      properties:
        value:
          type: string
    test.v1.Request:
      type: object
      properties:
        userId:
          allOf:
            - $ref: '#/components/schemas/core.v1.UUID'
          description: User identifier
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        flatten_uuid_refs(&mut doc, Some("core.v1.UUID"));

        // UUID schema should be removed
        let schemas = doc["components"]["schemas"].as_mapping().unwrap();
        assert!(!schemas.contains_key("core.v1.UUID"));

        // Property should be flattened to string + uuid
        let prop = doc["components"]["schemas"]["test.v1.Request"]["properties"]["userId"]
            .as_mapping()
            .unwrap();
        assert_eq!(prop.get("type").unwrap().as_str().unwrap(), "string");
        assert_eq!(prop.get("format").unwrap().as_str().unwrap(), "uuid");
        assert_eq!(
            prop.get("description").unwrap().as_str().unwrap(),
            "User identifier"
        );
    }

    #[test]
    fn uuid_query_param_simplified() {
        let yaml = r"
paths:
  /v1/items:
    get:
      parameters:
        - name: userId.value
          in: query
          schema:
            type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        simplify_uuid_query_params(&mut doc);

        let param = doc["paths"]["/v1/items"]["get"]["parameters"][0]
            .as_mapping()
            .unwrap();
        assert_eq!(param.get("name").unwrap().as_str().unwrap(), "userId");
        let schema = param.get("schema").unwrap().as_mapping().unwrap();
        assert_eq!(schema.get("format").unwrap().as_str().unwrap(), "uuid");
        assert_eq!(
            schema.get("pattern").unwrap().as_str().unwrap(),
            UUID_PATTERN
        );
        assert_eq!(
            schema.get("example").unwrap().as_str().unwrap(),
            UUID_EXAMPLE
        );
    }

    #[test]
    fn validation_constraints_injected() {
        let yaml = r"
components:
  schemas:
    test.v1.Request:
      type: object
      properties:
        name:
          type: string
        email:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let constraints = vec![SchemaConstraints {
            schema: "test.v1.Request".to_string(),
            fields: vec![
                FieldConstraint {
                    field: "name".to_string(),
                    min: Some(1),
                    max: Some(100),
                    signed_min: None,
                    signed_max: None,
                    pattern: None,
                    enum_values: Vec::new(),
                    required: true,
                    is_uuid: false,
                    is_numeric: false,
                },
                FieldConstraint {
                    field: "email".to_string(),
                    min: Some(5),
                    max: Some(255),
                    signed_min: None,
                    signed_max: None,
                    pattern: None,
                    enum_values: Vec::new(),
                    required: true,
                    is_uuid: false,
                    is_numeric: false,
                },
            ],
        }];

        inject_validation_constraints(&mut doc, &constraints);

        let schema = doc["components"]["schemas"]["test.v1.Request"]
            .as_mapping()
            .unwrap();
        let name_prop = schema["properties"]["name"].as_mapping().unwrap();
        assert_eq!(name_prop.get("minLength").unwrap().as_u64().unwrap(), 1);
        assert_eq!(name_prop.get("maxLength").unwrap().as_u64().unwrap(), 100);

        let required = schema.get("required").unwrap().as_sequence().unwrap();
        assert!(required.contains(&val_s("name")));
        assert!(required.contains(&val_s("email")));
    }

    #[test]
    fn path_fields_stripped_from_body() {
        let yaml = r"
paths:
  /v1/items/{itemId}:
    put:
      parameters:
        - name: itemId
          in: path
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/test.v1.UpdateRequest'
components:
  schemas:
    test.v1.UpdateRequest:
      type: object
      properties:
        itemId:
          type: string
        name:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        strip_path_fields_from_body(&mut doc);

        // The operation's schema should be inlined with itemId stripped
        let inlined_schema =
            doc["paths"]["/v1/items/{itemId}"]["put"]["requestBody"]["content"]["application/json"]
                ["schema"]
                .as_mapping()
                .unwrap();
        let props = inlined_schema
            .get("properties")
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(
            !props.contains_key("itemId"),
            "path field should be stripped from inlined schema"
        );
        assert!(props.contains_key("name"), "non-path field should be kept");

        // The component schema should be UNCHANGED (not mutated globally)
        let component_props = doc["components"]["schemas"]["test.v1.UpdateRequest"]["properties"]
            .as_mapping()
            .unwrap();
        assert!(
            component_props.contains_key("itemId"),
            "component schema should still have itemId (operation-local stripping)"
        );
        assert!(component_props.contains_key("name"));
    }

    #[test]
    fn shared_schema_preserved_across_operations() {
        let yaml = r"
paths:
  /v1/items/{itemId}:
    put:
      parameters:
        - name: itemId
          in: path
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/test.v1.ItemRequest'
  /v1/items:
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/test.v1.ItemRequest'
components:
  schemas:
    test.v1.ItemRequest:
      type: object
      required:
        - itemId
        - name
      properties:
        itemId:
          type: string
        name:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        strip_path_fields_from_body(&mut doc);

        // PUT operation should have inlined schema with itemId stripped
        let put_schema =
            doc["paths"]["/v1/items/{itemId}"]["put"]["requestBody"]["content"]["application/json"]
                ["schema"]
                .as_mapping()
                .unwrap();
        let put_props = put_schema.get("properties").unwrap().as_mapping().unwrap();
        assert!(!put_props.contains_key("itemId"));
        assert!(put_props.contains_key("name"));

        // PUT required should not include itemId
        let put_required = put_schema.get("required").unwrap().as_sequence().unwrap();
        assert!(!put_required.contains(&val_s("itemId")));
        assert!(put_required.contains(&val_s("name")));

        // POST operation should still reference the original shared schema
        let post_schema = &doc["paths"]["/v1/items"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"];
        assert!(
            post_schema.as_mapping().unwrap().contains_key("$ref"),
            "POST should keep $ref to shared schema"
        );

        // Component schema should be untouched
        let component = doc["components"]["schemas"]["test.v1.ItemRequest"]
            .as_mapping()
            .unwrap();
        let component_props = component.get("properties").unwrap().as_mapping().unwrap();
        assert!(
            component_props.contains_key("itemId"),
            "component schema must be unchanged"
        );
        assert!(component_props.contains_key("name"));
    }

    #[test]
    fn uuid_path_template_flattened() {
        let yaml = r"
paths:
  /v1/users/{userId.value}:
    get:
      parameters:
        - name: userId.value
          in: path
          schema:
            type: string
      responses:
        '200':
          description: OK
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        flatten_uuid_path_templates(&mut doc);

        let paths = doc["paths"].as_mapping().unwrap();
        assert!(
            !paths.contains_key("/v1/users/{userId.value}"),
            "old path key should be removed"
        );
        assert!(
            paths.contains_key("/v1/users/{userId}"),
            "new path key should exist"
        );

        let param = doc["paths"]["/v1/users/{userId}"]["get"]["parameters"][0]
            .as_mapping()
            .unwrap();
        assert_eq!(param.get("name").unwrap().as_str().unwrap(), "userId");
    }

    /// After `flatten_uuid_path_templates` renames `{user_id.value}` → `{user_id}`,
    /// `enrich_path_params` must still match the proto-discovered constraint
    /// (which uses camelCase `userId.value`) and inject UUID metadata.
    #[test]
    fn uuid_path_param_enriched_after_flattening() {
        use crate::discover::{PathParamConstraint, PathParamInfo};

        // gnostic outputs snake_case for compound vars: `{user_id.value}`
        let yaml = r"
paths:
  /v1/users/{user_id.value}:
    get:
      parameters:
        - name: user_id.value
          in: path
          schema:
            type: string
      responses:
        '200':
          description: OK
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();

        // Proto discovery produces camelCase names via convert_path_template_to_camel
        let path_params = vec![PathParamInfo {
            path: "/v1/users/{userId.value}".to_string(),
            params: vec![PathParamConstraint {
                name: "userId.value".to_string(),
                description: Some("User unique identifier".to_string()),
                is_uuid: true,
                min: None,
                max: None,
            }],
        }];

        // Phase 8: flatten paths first (renames {user_id.value} → {user_id})
        flatten_uuid_path_templates(&mut doc);

        // Phase 10: enrich should still find and apply UUID metadata
        // despite path and name case differences
        enrich_path_params(&mut doc, &path_params);

        let param = doc["paths"]["/v1/users/{user_id}"]["get"]["parameters"][0]
            .as_mapping()
            .unwrap();
        assert_eq!(param.get("name").unwrap().as_str().unwrap(), "user_id");

        let schema = param.get("schema").unwrap().as_mapping().unwrap();
        assert_eq!(schema.get("type").unwrap().as_str().unwrap(), "string");
        assert_eq!(schema.get("format").unwrap().as_str().unwrap(), "uuid");
        assert_eq!(
            schema.get("pattern").unwrap().as_str().unwrap(),
            UUID_PATTERN,
        );
        assert_eq!(
            schema.get("example").unwrap().as_str().unwrap(),
            UUID_EXAMPLE,
        );
        assert_eq!(
            param.get("description").unwrap().as_str().unwrap(),
            "Resource UUID",
        );
    }

    #[test]
    fn field_access_annotation_conventions() {
        let yaml = r"
components:
  schemas:
    test.v1.User:
      type: object
      properties:
        password:
          type: string
        clientSecret:
          type: string
        createdAt:
          type: string
          format: date-time
        updatedAt:
          type: string
          format: date-time
        name:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        annotate_field_access(&mut doc, &[], &[]);

        let props = &doc["components"]["schemas"]["test.v1.User"]["properties"];
        assert!(props["password"]["writeOnly"].as_bool().unwrap());
        assert!(props["clientSecret"]["writeOnly"].as_bool().unwrap());
        assert!(props["createdAt"]["readOnly"].as_bool().unwrap());
        assert!(props["updatedAt"]["readOnly"].as_bool().unwrap());
        assert!(
            props["name"]
                .as_mapping()
                .unwrap()
                .get("writeOnly")
                .is_none()
        );
        assert!(
            props["name"]
                .as_mapping()
                .unwrap()
                .get("readOnly")
                .is_none()
        );
    }

    #[test]
    fn field_access_annotation_extra_patterns() {
        let yaml = r"
components:
  schemas:
    test.v1.Config:
      type: object
      properties:
        apiKey:
          type: string
        lastSyncAt:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        annotate_field_access(&mut doc, &["apiKey".to_string()], &["lastSync".to_string()]);

        let props = &doc["components"]["schemas"]["test.v1.Config"]["properties"];
        assert!(props["apiKey"]["writeOnly"].as_bool().unwrap());
        assert!(props["lastSyncAt"]["readOnly"].as_bool().unwrap());
    }

    #[test]
    fn write_only_skipped_on_response_schemas() {
        let yaml = r"
components:
  schemas:
    test.v1.SetupMfaResponse:
      type: object
      properties:
        secret:
          type: string
        provisioningUri:
          type: string
        expiresAt:
          type: string
          format: date-time
    test.v1.SetupMfaRequest:
      type: object
      properties:
        secret:
          type: string
        password:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        annotate_field_access(&mut doc, &[], &[]);

        // Response schema: `secret` must NOT be writeOnly (client must read it)
        let response_props =
            &doc["components"]["schemas"]["test.v1.SetupMfaResponse"]["properties"];
        assert!(
            response_props["secret"]
                .as_mapping()
                .unwrap()
                .get("writeOnly")
                .is_none(),
            "secret in Response schema should not be writeOnly"
        );
        assert!(response_props["expiresAt"]["readOnly"].as_bool().unwrap());

        // Request schema: `secret` and `password` should be writeOnly
        let request_props = &doc["components"]["schemas"]["test.v1.SetupMfaRequest"]["properties"];
        assert!(request_props["secret"]["writeOnly"].as_bool().unwrap());
        assert!(request_props["password"]["writeOnly"].as_bool().unwrap());
    }

    #[test]
    fn duration_fields_annotated() {
        let yaml = r"
components:
  schemas:
    google.protobuf.Duration:
      type: object
      properties:
        seconds:
          type: integer
    test.v1.Config:
      type: object
      properties:
        timeout:
          $ref: '#/components/schemas/google.protobuf.Duration'
        name:
          type: string
";
        let mut doc: Value = serde_yaml_ng::from_str(yaml).unwrap();
        annotate_duration_fields(&mut doc);

        let timeout = doc["components"]["schemas"]["test.v1.Config"]["properties"]["timeout"]
            .as_mapping()
            .unwrap();
        // Simple $ref is kept — the Duration schema itself is rewritten
        assert!(timeout.contains_key("$ref"));

        // Duration schema should be rewritten to string type
        let dur = doc["components"]["schemas"]["google.protobuf.Duration"]
            .as_mapping()
            .unwrap();
        assert_eq!(dur.get("type").unwrap().as_str().unwrap(), "string");
        assert_eq!(dur.get("example").unwrap().as_str().unwrap(), "300s");
        assert!(!dur.contains_key("properties"));

        // Non-duration field should be untouched
        let name = doc["components"]["schemas"]["test.v1.Config"]["properties"]["name"]
            .as_mapping()
            .unwrap();
        assert!(!name.contains_key("example"));
    }
}
