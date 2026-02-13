//! Shared YAML manipulation helpers used across transform modules.

use serde_yaml_ng::Value;

/// UUID v4 regex pattern for OpenAPI schema `pattern` fields.
pub const UUID_PATTERN: &str =
    "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";

/// Example UUID v4 value for OpenAPI schema `example` fields.
pub const UUID_EXAMPLE: &str = "550e8400-e29b-41d4-a716-446655440000";

/// Shorthand for `Value::String`.
pub fn val_s(s: &str) -> Value {
    Value::String(s.to_string())
}

/// Shorthand for `Value::Number` (unsigned).
pub fn val_n(n: u64) -> Value {
    Value::Number(n.into())
}

/// Shorthand for `Value::Number` (signed).
pub fn val_i64(n: i64) -> Value {
    Value::Number(n.into())
}

/// Build `content` object for `application/json` with a schema `$ref`.
pub fn json_content_with_schema_ref(schema_ref: &str) -> Value {
    let mut schema = serde_yaml_ng::Mapping::new();
    schema.insert(val_s("$ref"), val_s(schema_ref));

    let mut media_type = serde_yaml_ng::Mapping::new();
    media_type.insert(val_s("schema"), Value::Mapping(schema));

    let mut content = serde_yaml_ng::Mapping::new();
    content.insert(val_s("application/json"), Value::Mapping(media_type));

    Value::Mapping(content)
}

/// Build a response object with description + `application/json` schema `$ref`.
pub fn json_response_with_schema_ref(description: &str, schema_ref: &str) -> Value {
    let mut response = serde_yaml_ng::Mapping::new();
    response.insert(val_s("description"), val_s(description));
    response.insert(val_s("content"), json_content_with_schema_ref(schema_ref));
    Value::Mapping(response)
}

/// Build a string-valued response header with a default value.
pub fn response_header(description: &str, default_value: &str) -> Value {
    let mut schema = serde_yaml_ng::Mapping::new();
    schema.insert(val_s("type"), val_s("string"));
    schema.insert(val_s("default"), val_s(default_value));

    let mut header = serde_yaml_ng::Mapping::new();
    header.insert(val_s("description"), val_s(description));
    header.insert(val_s("schema"), Value::Mapping(schema));

    Value::Mapping(header)
}

/// Known HTTP methods per the OpenAPI specification.
///
/// Path items can also contain `summary`, `description`, `parameters`, and
/// `servers` keys — we skip those so callbacks only receive actual operations.
const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Iterate over all operations in the spec, calling `f(path, method, operation_map)`.
///
/// Only iterates HTTP method keys (`get`, `post`, etc.), skipping path-level
/// metadata keys like `summary`, `parameters`, and `servers`.
pub fn for_each_operation(
    doc: &mut Value,
    mut f: impl FnMut(&str, &str, &mut serde_yaml_ng::Mapping),
) {
    let Some(paths) = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("paths"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    for (path_key, path_item) in paths.iter_mut() {
        let path_str = path_key.as_str().unwrap_or_default();
        let Some(path_map) = path_item.as_mapping_mut() else {
            continue;
        };

        for (method_key, operation) in path_map.iter_mut() {
            let method_str = method_key.as_str().unwrap_or_default();
            if !HTTP_METHODS.contains(&method_str) {
                continue;
            }
            let Some(op_map) = operation.as_mapping_mut() else {
                continue;
            };
            f(path_str, method_str, op_map);
        }
    }
}

/// Get the `$ref` from an operation's `requestBody`, if present.
pub fn request_body_ref(op: &serde_yaml_ng::Mapping) -> Option<&str> {
    op.get("requestBody")?
        .as_mapping()?
        .get("content")?
        .as_mapping()?
        .get("application/json")?
        .as_mapping()?
        .get("schema")?
        .as_mapping()?
        .get("$ref")?
        .as_str()
}

/// Collect names of schemas whose `properties` mapping is empty.
pub fn collect_empty_schema_names(doc: &Value) -> Vec<String> {
    let Some(schemas) = doc
        .as_mapping()
        .and_then(|m| m.get("components"))
        .and_then(Value::as_mapping)
        .and_then(|m| m.get("schemas"))
        .and_then(Value::as_mapping)
    else {
        return Vec::new();
    };

    schemas
        .iter()
        .filter_map(|(k, v)| {
            let name = k.as_str()?;
            let props = v.as_mapping()?.get("properties")?.as_mapping()?;
            props.is_empty().then(|| name.to_string())
        })
        .collect()
}

/// Convert `snake_case` dotted path to `lowerCamelCase` dotted path.
///
/// `user_id.value` → `userId.value`
pub fn snake_to_lower_camel_dotted(s: &str) -> String {
    s.split('.')
        .map(crate::discover::snake_to_lower_camel)
        .collect::<Vec<_>>()
        .join(".")
}

/// Recursively walk a YAML value tree and collect all `$ref` string values.
pub fn collect_refs(value: &Value, refs: &mut std::collections::HashSet<String>) {
    match value {
        Value::Mapping(map) => {
            for (k, v) in map {
                if k.as_str() == Some("$ref") {
                    if let Some(s) = v.as_str() {
                        refs.insert(s.to_string());
                    }
                }
                collect_refs(v, refs);
            }
        }
        Value::Sequence(seq) => {
            for item in seq {
                collect_refs(item, refs);
            }
        }
        _ => {}
    }
}
