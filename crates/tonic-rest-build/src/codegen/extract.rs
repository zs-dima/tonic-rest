//! Service and method extraction from proto descriptors.

use std::collections::HashMap;

use tonic_rest_core::descriptor::{self, field_type, FileDescriptorSet, MethodDescriptorProto};

use super::config::{GenerateError, RestCodegenConfig};
use super::types::{
    FieldTypeInfo, MessageFieldTypes, MethodRoute, ParamAssignment, PathParam, ServiceRoute,
};

/// Auto-discover packages from a descriptor set by finding services with HTTP annotations.
pub(crate) fn discover_packages(fdset: &FileDescriptorSet) -> HashMap<String, String> {
    let mut packages = HashMap::new();

    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        if package.is_empty() {
            continue;
        }

        let has_http_methods = file.service.iter().any(|svc| {
            svc.method
                .iter()
                .any(|method| descriptor::extract_http_pattern(method).is_some())
        });

        if has_http_methods {
            packages
                .entry(package.to_string())
                .or_insert_with(|| infer_rust_module(package));
        }
    }

    packages
}

/// Infer a Rust module path from a proto package name.
///
/// Converts dots to `::` to match standard `prost-build` module generation:
/// - `auth.v1` → `auth::v1`
/// - `my_service` → `my_service`
/// - `org.service.v2` → `org::service::v2`
fn infer_rust_module(package: &str) -> String {
    package.replace('.', "::")
}

/// Build a lookup table: fully-qualified message name → { `field_name` → field type info }.
pub(crate) fn collect_field_types(fdset: &FileDescriptorSet) -> MessageFieldTypes {
    let mut map = HashMap::new();

    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        for msg in &file.message_type {
            collect_message_fields(&mut map, &format!(".{package}"), msg);
        }
    }

    map
}

/// Recursively collect field types from a message and its nested types.
fn collect_message_fields(
    map: &mut MessageFieldTypes,
    parent_path: &str,
    msg: &descriptor::DescriptorProto,
) {
    let msg_name = msg.name.as_deref().unwrap_or("");
    let fqn = format!("{parent_path}.{msg_name}");

    let mut fields = HashMap::new();
    for field in &msg.field {
        if let (Some(name), Some(ty)) = (field.name.as_deref(), field.r#type) {
            fields.insert(
                name.to_string(),
                FieldTypeInfo {
                    type_id: ty,
                    enum_type_name: if ty == field_type::ENUM {
                        field.type_name.clone()
                    } else {
                        None
                    },
                },
            );
        }
    }
    map.insert(fqn.clone(), fields);

    // Recurse into nested message types
    for nested in &msg.nested_type {
        collect_message_fields(map, &fqn, nested);
    }
}

pub(crate) fn extract_services(
    fdset: &FileDescriptorSet,
    field_types: &MessageFieldTypes,
    config: &RestCodegenConfig,
) -> Result<Vec<ServiceRoute>, GenerateError> {
    let mut result = Vec::new();

    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");

        // Only process packages registered in the config
        let Some(package_mod) = config.rust_module(package) else {
            continue;
        };

        for service in &file.service {
            let service_name = service.name.as_deref().unwrap_or("").to_string();
            let mut methods = Vec::new();

            for method in &service.method {
                if let Some(route) = extract_method_route(method, field_types, config)? {
                    methods.push(route);
                }
            }

            if !methods.is_empty() {
                result.push(ServiceRoute {
                    package_mod: package_mod.to_string(),
                    service_name,
                    methods,
                });
            }
        }
    }

    Ok(result)
}

fn extract_method_route(
    method: &MethodDescriptorProto,
    field_types: &MessageFieldTypes,
    config: &RestCodegenConfig,
) -> Result<Option<MethodRoute>, GenerateError> {
    let Some((http_method, path)) = descriptor::extract_http_pattern(method) else {
        return Ok(None);
    };
    let body = method
        .options
        .as_ref()
        .and_then(|o| o.http.as_ref())
        .map_or("", |h| h.body.as_str());

    let proto_name = method.name.as_deref().unwrap_or("").to_string();

    // Only `body: "*"` (whole message) is supported. Partial body selectors
    // (e.g., `body: "user"`) require sub-message deserialization that the
    // codegen does not implement — reject early with a clear error.
    if !body.is_empty() && body != "*" {
        return Err(GenerateError::UnsupportedBodySelector {
            method: proto_name,
            body: body.to_string(),
        });
    }
    let rust_name = super::to_snake_case(&proto_name);
    let server_streaming = method.server_streaming.unwrap_or(false);

    let input_fqn = method.input_type.as_deref().unwrap_or("");
    let input_type = config.proto_type_to_rust(input_fqn);
    let raw_output = method.output_type.as_deref().unwrap_or("");
    let returns_empty = raw_output == ".google.protobuf.Empty";
    let output_type = config.proto_type_to_rust(raw_output);

    let has_body = !body.is_empty();
    let path_params = extract_path_params(path, input_fqn, field_types, config)?;
    let axum_path = convert_to_axum_path(path);

    Ok(Some(MethodRoute {
        proto_name,
        rust_name,
        http_method: http_method.to_string(),
        path: path.to_string(),
        axum_path,
        has_body,
        server_streaming,
        input_type,
        output_type,
        returns_empty,
        path_params,
    }))
}

pub(super) fn extract_path_params(
    path: &str,
    input_fqn: &str,
    field_types: &MessageFieldTypes,
    config: &RestCodegenConfig,
) -> Result<Vec<PathParam>, GenerateError> {
    let mut params = Vec::new();
    let msg_fields = field_types.get(input_fqn);
    let mut rest = path;

    while let Some(start) = rest.find('{') {
        if let Some(end) = rest[start..].find('}') {
            let field_path = &rest[start + 1..start + end];
            let axum_name = field_path.replace('.', "_");
            let is_nested = field_path.contains('.');

            let assignment = if is_nested {
                // Nested field: `user_id.value` → UUID wrapper pattern
                if config.wrapper_type.is_none() {
                    return Err(GenerateError::MissingWrapperType {
                        param: field_path.to_string(),
                    });
                }
                let parent = field_path.split('.').next().unwrap_or(field_path);
                ParamAssignment::UuidWrapper {
                    parent_field: parent.to_string(),
                }
            } else {
                // Simple field: look up type from message descriptor
                let field_info = msg_fields.and_then(|f| f.get(field_path));
                let type_id = field_info.map_or(field_type::STRING, |fi| fi.type_id);

                if type_id == field_type::ENUM {
                    // Resolve FQN enum type to Rust path
                    let enum_rust_type = field_info
                        .and_then(|fi| fi.enum_type_name.as_deref())
                        .map_or_else(|| "i32".to_string(), |fqn| config.proto_type_to_rust(fqn));
                    ParamAssignment::EnumField {
                        field_name: field_path.to_string(),
                        enum_rust_type,
                    }
                } else if let Some(rust_type) = proto_type_to_rust_scalar(type_id) {
                    // Typed scalar: let Axum's Path<T> extractor handle parsing
                    ParamAssignment::TypedField {
                        field_name: field_path.to_string(),
                        rust_type,
                    }
                } else {
                    ParamAssignment::StringField {
                        field_name: field_path.to_string(),
                    }
                }
            };

            params.push(PathParam {
                axum_name,
                assignment,
            });
            rest = &rest[start + end + 1..];
        } else {
            break;
        }
    }

    Ok(params)
}

/// Map proto field type IDs to Rust scalar types for path parameter extraction.
///
/// Returns `None` for `STRING` (uses `String` as default) and unsupported types.
fn proto_type_to_rust_scalar(type_id: i32) -> Option<&'static str> {
    match type_id {
        field_type::INT32 => Some("i32"),
        field_type::INT64 => Some("i64"),
        field_type::UINT32 => Some("u32"),
        field_type::UINT64 => Some("u64"),
        field_type::BOOL => Some("bool"),
        _ => None,
    }
}

pub(super) fn convert_to_axum_path(path: &str) -> String {
    let mut result = String::new();
    let mut rest = path;

    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find('}') {
            let field_path = &rest[start + 1..start + end];
            let axum_name = field_path.replace('.', "_");
            result.push('{');
            result.push_str(&axum_name);
            result.push('}');
            rest = &rest[start + end + 1..];
        } else {
            break;
        }
    }
    result.push_str(rest);
    result
}
