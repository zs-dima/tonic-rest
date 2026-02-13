//! Build-time REST route generator.
//!
//! Reads the proto file descriptor set, extracts `google.api.http` annotations,
//! and generates Axum REST handler code that calls through the Tonic service traits.
//!
//! This keeps proto files as the single source of truth for both gRPC and REST APIs.
//!
//! # Architecture
//!
//! Generated code (route registration + handler wrappers) is thin — it delegates to
//! runtime utilities for request building, error conversion, and SSE stream mapping.
//! This separation keeps the generator simple and the utilities testable.

mod config;
mod emit;
mod extract;
mod types;

pub use config::{GenerateError, RestCodegenConfig};

use crate::descriptor::FileDescriptorSet;
use prost::Message as _;

/// Generate REST route code from a compiled proto file descriptor set.
///
/// Uses the provided [`RestCodegenConfig`] to determine which packages to
/// process and which methods are public. Returns Rust source code to be
/// written to `OUT_DIR/rest_routes.rs`.
///
/// When [`RestCodegenConfig::packages`] is empty, automatically discovers
/// packages from the descriptor set by scanning for services with
/// `google.api.http` annotations.
///
/// # Known Limitations
///
/// - **`additional_bindings`**: Proto `HttpRule.additional_bindings` (multiple
///   REST mappings per gRPC method) is not supported. Only the primary binding
///   is processed.
/// - **Partial body selectors**: Only `body: "*"` (full body) and `body: ""`
///   (no body) are supported. The `body: "field_name"` partial body binding
///   from the gRPC-HTTP transcoding spec is not implemented.
/// - **Repeated WKT fields**: `configure_prost_serde` does not wire serde
///   adapters for `repeated google.protobuf.Timestamp` or similar repeated
///   well-known type fields.
///
/// # Errors
///
/// Returns [`GenerateError`] if:
/// - `descriptor_bytes` is not a valid protobuf `FileDescriptorSet`
/// - A nested path param (e.g., `{user_id.value}`) is found but
///   [`RestCodegenConfig::wrapper_type`] is not configured
pub fn generate(
    descriptor_bytes: &[u8],
    config: &RestCodegenConfig,
) -> Result<String, GenerateError> {
    let fdset = FileDescriptorSet::decode(descriptor_bytes)?;

    // Resolve packages: use explicit mapping or auto-discover from descriptor
    let config = config.resolve(&fdset);

    let field_types = extract::collect_field_types(&fdset);
    let services = extract::extract_services(&fdset, &field_types, &config)?;
    Ok(emit::generate_code(&services, &config))
}

/// Convert `CamelCase` to `snake_case` (matches tonic-build output).
pub(crate) fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_uppercase() {
            if !result.is_empty() {
                // Insert underscore before uppercase when:
                // - preceded by lowercase (e.g., "List|U" → "list_u")
                // - preceded by uppercase followed by lowercase (e.g., "OA|u" → "o_au")
                let next_is_lower = chars.peek().is_some_and(|n| n.is_lowercase());
                let prev_is_lower = result.chars().last().is_some_and(char::is_lowercase);
                if prev_is_lower || next_is_lower {
                    result.push('_');
                }
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }

    result
}

impl RestCodegenConfig {
    /// Create a resolved copy of this config, auto-discovering packages if none are set.
    ///
    /// When `packages` is empty, scans the descriptor set for services with
    /// `google.api.http` annotations and infers Rust module paths from proto
    /// package names (dots → `::`, matching standard `prost-build` output).
    fn resolve(&self, fdset: &FileDescriptorSet) -> Self {
        let mut resolved = self.clone();
        if resolved.packages.is_empty() {
            resolved.packages = extract::discover_packages(fdset);
        }
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{
        field_type, DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto,
        FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet, HttpPattern, HttpRule,
        MethodDescriptorProto, MethodOptions, ServiceDescriptorProto,
    };
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    use super::extract::{collect_field_types, convert_to_axum_path, extract_path_params};
    use super::types::{FieldTypeInfo, ParamAssignment};

    /// Build a method descriptor with an HTTP annotation.
    fn make_method(
        name: &str,
        input: &str,
        output: &str,
        pattern: HttpPattern,
        body: &str,
        server_streaming: bool,
    ) -> MethodDescriptorProto {
        MethodDescriptorProto {
            name: Some(name.to_string()),
            input_type: Some(input.to_string()),
            output_type: Some(output.to_string()),
            options: Some(MethodOptions {
                http: Some(HttpRule {
                    pattern: Some(pattern),
                    body: body.to_string(),
                }),
            }),
            client_streaming: None,
            server_streaming: Some(server_streaming),
        }
    }

    /// Build a message descriptor with typed fields.
    fn make_message(name: &str, fields: &[(&str, i32, Option<&str>)]) -> DescriptorProto {
        DescriptorProto {
            name: Some(name.to_string()),
            field: fields
                .iter()
                .map(|(fname, ftype, type_name)| FieldDescriptorProto {
                    name: Some(fname.to_string()),
                    r#type: Some(*ftype),
                    type_name: type_name.map(ToString::to_string),
                    options: None,
                })
                .collect(),
            nested_type: vec![],
        }
    }

    /// Encode a `FileDescriptorSet` to bytes for `generate()`.
    fn encode_fdset(fdset: &FileDescriptorSet) -> Vec<u8> {
        fdset.encode_to_vec()
    }

    /// Compare generated code against a golden file.
    ///
    /// - If `UPDATE_GOLDEN=1` env var is set: overwrite the golden file.
    /// - If the golden file doesn't exist: create it and panic to force review.
    /// - Otherwise: assert equality.
    fn assert_golden(name: &str, actual: &str) {
        let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name);

        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
            std::fs::write(&golden_path, actual).unwrap();
            eprintln!("Updated golden file: {}", golden_path.display());
            return;
        }

        if !golden_path.exists() {
            std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
            std::fs::write(&golden_path, actual).unwrap();
            panic!(
                "Golden file created: {}. Inspect the output and re-run.",
                golden_path.display(),
            );
        }

        let expected = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
            panic!("Failed to read golden file {}: {e}", golden_path.display())
        });
        assert_eq!(
            actual,
            expected,
            "\n\nGolden file mismatch: {}\nSet UPDATE_GOLDEN=1 to update.\n",
            golden_path.display(),
        );
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("ListUsers"), "list_users");
        assert_eq!(to_snake_case("CreateUser"), "create_user");
        assert_eq!(to_snake_case("GetOAuthUrl"), "get_o_auth_url");
        assert_eq!(to_snake_case("VerifyMfa"), "verify_mfa");
        assert_eq!(to_snake_case("GetAvatarUploadUrl"), "get_avatar_upload_url");
        assert_eq!(to_snake_case("SetPassword"), "set_password");
        assert_eq!(to_snake_case("ListUsersInfo"), "list_users_info");
    }

    #[test]
    fn snake_case_edge_cases() {
        assert_eq!(to_snake_case(""), "");
        assert_eq!(to_snake_case("a"), "a");
        assert_eq!(to_snake_case("A"), "a");
        assert_eq!(to_snake_case("A_B"), "a_b");
        assert_eq!(to_snake_case("AB"), "a_b");
        assert_eq!(to_snake_case("ABc"), "a_bc");
        assert_eq!(to_snake_case("already_snake"), "already_snake");
        assert_eq!(to_snake_case("lowercase"), "lowercase");
        assert_eq!(to_snake_case("S"), "s");
    }

    #[test]
    fn test_proto_type_to_rust_default_root() {
        let config = RestCodegenConfig::new();
        assert_eq!(
            config.proto_type_to_rust(".users.v1.User"),
            "crate::users::User"
        );
        assert_eq!(
            config.proto_type_to_rust(".auth.v1.AuthResponse"),
            "crate::auth::AuthResponse"
        );
        assert_eq!(config.proto_type_to_rust(".google.protobuf.Empty"), "()");
        assert_eq!(
            config.proto_type_to_rust(".core.v1.Uuid"),
            "crate::core::Uuid"
        );
    }

    #[test]
    fn test_proto_type_to_rust_custom_root() {
        let config = RestCodegenConfig::new().proto_root("auth_proto");
        assert_eq!(
            config.proto_type_to_rust(".users.v1.User"),
            "auth_proto::users::User"
        );
        assert_eq!(
            config.proto_type_to_rust(".auth.v1.AuthResponse"),
            "auth_proto::auth::AuthResponse"
        );
        assert_eq!(config.proto_type_to_rust(".google.protobuf.Empty"), "()");
    }

    #[test]
    fn proto_type_to_rust_short_path() {
        let config = RestCodegenConfig::new();
        // Fewer than 3 segments → returned as-is
        assert_eq!(config.proto_type_to_rust("Foo"), "Foo");
        assert_eq!(config.proto_type_to_rust(".Foo"), ".Foo");
        assert_eq!(config.proto_type_to_rust("a.b"), "a.b");
    }

    #[test]
    fn extension_extractor_without_type() {
        let config = RestCodegenConfig::new();
        assert_eq!(config.extension_extractor_line(), "");
    }

    #[test]
    fn extension_extractor_with_type() {
        let config = RestCodegenConfig::new().extension_type("auth_core::AuthInfo");
        assert_eq!(
            config.extension_extractor_line(),
            "    ext: Option<Extension<auth_core::AuthInfo>>,\n",
        );
    }

    #[test]
    fn extension_request_lines_without_type() {
        let config = RestCodegenConfig::new().runtime_crate("tonic_rest");
        let lines = config.extension_and_request_lines("body");
        assert!(lines.contains("None"), "should pass None: {lines}");
        assert!(
            lines.contains("build_tonic_request::<_, ()>"),
            "should turbofish (): {lines}",
        );
        assert!(
            !lines.contains("Extension"),
            "should not mention Extension: {lines}"
        );
    }

    #[test]
    fn extension_request_lines_with_type() {
        let config = RestCodegenConfig::new()
            .runtime_crate("tonic_rest")
            .extension_type("auth_core::AuthInfo");
        let lines = config.extension_and_request_lines("query");
        assert!(
            lines.contains("ext.map(|Extension(v)| v)"),
            "should unwrap Extension: {lines}",
        );
        assert!(
            lines.contains("build_tonic_request(query"),
            "should use query var: {lines}",
        );
    }

    #[test]
    fn test_convert_to_axum_path() {
        assert_eq!(convert_to_axum_path("/v1/users"), "/v1/users");
        assert_eq!(
            convert_to_axum_path("/v1/users/{user_id.value}"),
            "/v1/users/{user_id_value}"
        );
        assert_eq!(
            convert_to_axum_path("/v1/users/{user_id.value}/password"),
            "/v1/users/{user_id_value}/password"
        );
        assert_eq!(
            convert_to_axum_path("/v1/auth/sessions/{device_id}"),
            "/v1/auth/sessions/{device_id}"
        );
    }

    #[test]
    fn axum_path_multiple_params() {
        assert_eq!(
            convert_to_axum_path("/v1/{org_id}/{user_id.value}/roles"),
            "/v1/{org_id}/{user_id_value}/roles",
        );
    }

    #[test]
    fn axum_path_no_params() {
        assert_eq!(convert_to_axum_path("/v1/health"), "/v1/health");
    }

    #[test]
    fn test_extract_path_params_nested() {
        let config = RestCodegenConfig::new().wrapper_type("crate::core::Uuid");
        let field_types = std::collections::HashMap::new();
        let params = extract_path_params(
            "/v1/users/{user_id.value}/password",
            ".users.v1.Foo",
            &field_types,
            &config,
        )
        .unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].axum_name, "user_id_value");
        assert!(matches!(
            params[0].assignment,
            ParamAssignment::UuidWrapper { .. }
        ));
    }

    #[test]
    fn test_extract_path_params_string_field() {
        let config = RestCodegenConfig::new();
        let mut msg_fields = std::collections::HashMap::new();
        msg_fields.insert(
            "device_id".to_string(),
            FieldTypeInfo {
                type_id: field_type::STRING,
                enum_type_name: None,
            },
        );
        let mut field_types = std::collections::HashMap::new();
        field_types.insert(".auth.v1.RevokeSessionRequest".to_string(), msg_fields);

        let params = extract_path_params(
            "/v1/auth/sessions/{device_id}",
            ".auth.v1.RevokeSessionRequest",
            &field_types,
            &config,
        )
        .unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].axum_name, "device_id");
        assert!(matches!(
            params[0].assignment,
            ParamAssignment::StringField { .. }
        ));
    }

    #[test]
    fn test_extract_path_params_enum_field() {
        let config = RestCodegenConfig::new();
        let mut msg_fields = std::collections::HashMap::new();
        msg_fields.insert(
            "provider".to_string(),
            FieldTypeInfo {
                type_id: field_type::ENUM,
                enum_type_name: Some(".auth.v1.OAuthProvider".to_string()),
            },
        );
        let mut field_types = std::collections::HashMap::new();
        field_types.insert(
            ".auth.v1.UnlinkOAuthProviderRequest".to_string(),
            msg_fields,
        );

        let params = extract_path_params(
            "/v1/auth/oauth/providers/{provider}",
            ".auth.v1.UnlinkOAuthProviderRequest",
            &field_types,
            &config,
        )
        .unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].axum_name, "provider");
        match &params[0].assignment {
            ParamAssignment::EnumField {
                field_name,
                enum_rust_type,
            } => {
                assert_eq!(field_name, "provider");
                assert_eq!(enum_rust_type, "crate::auth::OAuthProvider");
            }
            _ => panic!("Expected EnumField"),
        }
    }

    #[test]
    fn path_params_multiple() {
        let config = RestCodegenConfig::new().wrapper_type("crate::core::Uuid");
        let mut msg_fields = std::collections::HashMap::new();
        msg_fields.insert(
            "role".to_string(),
            FieldTypeInfo {
                type_id: field_type::STRING,
                enum_type_name: None,
            },
        );
        let mut field_types = std::collections::HashMap::new();
        field_types.insert(".test.v1.Req".to_string(), msg_fields);

        let params = extract_path_params(
            "/v1/users/{user_id.value}/roles/{role}",
            ".test.v1.Req",
            &field_types,
            &config,
        )
        .unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].axum_name, "user_id_value");
        assert!(matches!(
            params[0].assignment,
            ParamAssignment::UuidWrapper { .. }
        ));
        assert_eq!(params[1].axum_name, "role");
        assert!(matches!(
            params[1].assignment,
            ParamAssignment::StringField { .. }
        ));
    }

    #[test]
    fn path_params_no_params() {
        let config = RestCodegenConfig::new();
        let field_types = std::collections::HashMap::new();
        let params =
            extract_path_params("/v1/items", ".test.v1.Req", &field_types, &config).unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn path_params_unknown_field_defaults_to_string() {
        let config = RestCodegenConfig::new();
        let field_types = std::collections::HashMap::new(); // no field info
        let params =
            extract_path_params("/v1/items/{item_id}", ".test.v1.Req", &field_types, &config)
                .unwrap();
        assert_eq!(params.len(), 1);
        assert!(matches!(
            params[0].assignment,
            ParamAssignment::StringField { .. }
        ));
    }

    #[test]
    fn path_params_int32_field_produces_typed_param() {
        let config = RestCodegenConfig::new();
        let mut msg_fields = std::collections::HashMap::new();
        msg_fields.insert(
            "page".to_string(),
            FieldTypeInfo {
                type_id: field_type::INT32,
                enum_type_name: None,
            },
        );
        let mut field_types = std::collections::HashMap::new();
        field_types.insert(".test.v1.ListRequest".to_string(), msg_fields);

        let params = extract_path_params(
            "/v1/items/{page}",
            ".test.v1.ListRequest",
            &field_types,
            &config,
        )
        .unwrap();
        assert_eq!(params.len(), 1);
        match &params[0].assignment {
            ParamAssignment::TypedField {
                field_name,
                rust_type,
            } => {
                assert_eq!(field_name, "page");
                assert_eq!(*rust_type, "i32");
            }
            other => panic!("Expected TypedField, got {other:?}"),
        }
    }

    #[test]
    fn path_params_bool_field_produces_typed_param() {
        let config = RestCodegenConfig::new();
        let mut msg_fields = std::collections::HashMap::new();
        msg_fields.insert(
            "active".to_string(),
            FieldTypeInfo {
                type_id: field_type::BOOL,
                enum_type_name: None,
            },
        );
        let mut field_types = std::collections::HashMap::new();
        field_types.insert(".test.v1.Req".to_string(), msg_fields);

        let params =
            extract_path_params("/v1/items/{active}", ".test.v1.Req", &field_types, &config)
                .unwrap();
        assert_eq!(params.len(), 1);
        match &params[0].assignment {
            ParamAssignment::TypedField {
                field_name,
                rust_type,
            } => {
                assert_eq!(field_name, "active");
                assert_eq!(*rust_type, "bool");
            }
            other => panic!("Expected TypedField, got {other:?}"),
        }
    }

    #[test]
    fn config_default_values() {
        let config = RestCodegenConfig::new();
        assert!(config.packages.is_empty());
        assert!(config.public_methods.is_empty());
        assert_eq!(config.proto_root, "crate");
        assert_eq!(config.runtime_crate, "tonic_rest");
        assert!(config.wrapper_type.is_none());
        assert_eq!(config.sse_keep_alive_secs, 15);
        assert!(config.extension_type.is_none());
    }

    #[test]
    fn config_builder_chain() {
        let config = RestCodegenConfig::new()
            .package("auth.v1", "auth")
            .package("users.v1", "users")
            .proto_root("my_proto")
            .runtime_crate("my_runtime")
            .wrapper_type("my::Uuid")
            .sse_keep_alive_secs(30)
            .extension_type("my::Auth")
            .public_methods(&["Login", "SignUp"]);

        assert_eq!(config.packages.len(), 2);
        assert_eq!(config.rust_module("auth.v1"), Some("auth"));
        assert_eq!(config.rust_module("users.v1"), Some("users"));
        assert_eq!(config.rust_module("unknown"), None);
        assert_eq!(config.proto_root, "my_proto");
        assert_eq!(config.runtime_crate, "my_runtime");
        assert_eq!(config.wrapper_type.as_deref(), Some("my::Uuid"));
        assert_eq!(config.sse_keep_alive_secs, 30);
        assert_eq!(config.extension_type.as_deref(), Some("my::Auth"));
        assert!(config.public_methods.contains("Login"));
        assert!(config.public_methods.contains("SignUp"));
        assert!(!config.public_methods.contains("Delete"));
    }

    #[test]
    fn test_config_debug() {
        let config = RestCodegenConfig::new()
            .proto_root("crate")
            .runtime_crate("tonic_rest")
            .package("auth.v1", "auth");
        let debug = format!("{config:?}");
        assert!(debug.contains("proto_root"));
        assert!(debug.contains("runtime_crate"));
    }

    #[test]
    fn test_generate_returns_error_on_invalid_bytes() {
        let config = RestCodegenConfig::new();
        let result = generate(b"not a valid protobuf", &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("failed to decode"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn generate_error_display() {
        let err = GenerateError::Config("something broke".to_string());
        assert_eq!(err.to_string(), "something broke");

        let err = GenerateError::MissingWrapperType {
            param: "user_id.value".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("user_id.value"), "should contain param: {msg}");
        assert!(
            msg.contains("wrapper_type"),
            "should mention wrapper_type: {msg}",
        );
    }

    #[test]
    fn generate_error_is_std_error() {
        let err = GenerateError::Config("error".to_string());
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn generate_empty_descriptor() {
        let fdset = FileDescriptorSet { file: vec![] };
        let config = RestCodegenConfig::new().package("test.v1", "test");
        let code = generate(&encode_fdset(&fdset), &config).unwrap();
        // Should still produce valid code (header + empty public paths + empty router)
        assert!(code.contains("PUBLIC_REST_PATHS"));
        assert!(code.contains("fn all_rest_routes"));

        // Must be valid Rust syntax — previously generated invalid code
        // with empty type params and trailing comma
        syn::parse_file(&code).expect("empty-descriptor code should be valid Rust syntax");
    }

    #[test]
    fn generate_skips_unregistered_packages() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("other.proto".to_string()),
                package: Some("other.v1".to_string()),
                message_type: vec![make_message("Req", &[("name", field_type::STRING, None)])],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("OtherService".to_string()),
                    method: vec![make_method(
                        "DoStuff",
                        ".other.v1.Req",
                        ".other.v1.Req",
                        HttpPattern::Post("/v1/stuff".to_string()),
                        "*",
                        false,
                    )],
                }],
            }],
        };
        // Config only registers "test.v1", not "other.v1"
        let config = RestCodegenConfig::new().package("test.v1", "test");
        let code = generate(&encode_fdset(&fdset), &config).unwrap();
        assert!(!code.contains("other_service_rest_router"));
        assert!(!code.contains("rest_other_service"));
    }

    /// Basic CRUD service: POST (body), GET (query + path param), DELETE (path param, empty return).
    #[test]
    fn snapshot_basic_crud() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("item.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![
                    make_message("CreateItemRequest", &[("name", field_type::STRING, None)]),
                    make_message("GetItemRequest", &[("item_id", field_type::STRING, None)]),
                    make_message(
                        "DeleteItemRequest",
                        &[("item_id", field_type::STRING, None)],
                    ),
                    make_message("Item", &[("id", field_type::STRING, None)]),
                ],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("ItemService".to_string()),
                    method: vec![
                        make_method(
                            "CreateItem",
                            ".test.v1.CreateItemRequest",
                            ".test.v1.Item",
                            HttpPattern::Post("/v1/items".to_string()),
                            "*",
                            false,
                        ),
                        make_method(
                            "GetItem",
                            ".test.v1.GetItemRequest",
                            ".test.v1.Item",
                            HttpPattern::Get("/v1/items/{item_id}".to_string()),
                            "",
                            false,
                        ),
                        make_method(
                            "DeleteItem",
                            ".test.v1.DeleteItemRequest",
                            ".google.protobuf.Empty",
                            HttpPattern::Delete("/v1/items/{item_id}".to_string()),
                            "",
                            false,
                        ),
                    ],
                }],
            }],
        };

        let config = RestCodegenConfig::new()
            .package("test.v1", "test")
            .public_methods(&["CreateItem"]);

        let code = generate(&encode_fdset(&fdset), &config).unwrap();

        // Property checks
        assert!(code.contains("fn item_service_rest_router<S>"));
        assert!(code.contains("rest_item_service_create_item"));
        assert!(code.contains("rest_item_service_get_item"));
        assert!(code.contains("rest_item_service_delete_item"));
        assert!(code.contains("axum::routing::post("));
        assert!(code.contains("axum::routing::get("));
        assert!(code.contains("axum::routing::delete("));
        assert!(code.contains("StatusCode::NO_CONTENT"));
        assert!(code.contains("\"/v1/items\""));

        // Public paths
        assert!(
            code.contains("PUBLIC_REST_PATHS"),
            "missing PUBLIC_REST_PATHS",
        );

        // Golden file comparison
        assert_golden("basic_crud.rs", &code);

        // Syntax validation
        syn::parse_file(&code).expect("generated code should be valid Rust syntax");
    }

    /// Streaming SSE endpoint + UUID wrapper path param + auth type + custom keep-alive.
    #[test]
    fn snapshot_streaming_with_uuid_and_auth() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("events.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![
                    make_message("ListEventsRequest", &[]),
                    make_message("Event", &[("data", field_type::STRING, None)]),
                    make_message(
                        "UpdateUserRequest",
                        &[
                            ("user_id", 11, None), // TYPE_MESSAGE = 11
                            ("name", field_type::STRING, None),
                        ],
                    ),
                    make_message("User", &[("name", field_type::STRING, None)]),
                ],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("EventService".to_string()),
                    method: vec![
                        make_method(
                            "ListEvents",
                            ".test.v1.ListEventsRequest",
                            ".test.v1.Event",
                            HttpPattern::Get("/v1/events".to_string()),
                            "",
                            true, // server_streaming
                        ),
                        make_method(
                            "UpdateUser",
                            ".test.v1.UpdateUserRequest",
                            ".test.v1.User",
                            HttpPattern::Patch("/v1/users/{user_id.value}".to_string()),
                            "*",
                            false,
                        ),
                    ],
                }],
            }],
        };

        let config = RestCodegenConfig::new()
            .package("test.v1", "test")
            .runtime_crate("tonic_rest")
            .wrapper_type("crate::core::Uuid")
            .extension_type("crate::AuthInfo")
            .sse_keep_alive_secs(30);

        let code = generate(&encode_fdset(&fdset), &config).unwrap();

        // SSE handler properties
        assert!(code.contains("Sse<impl Stream<Item = Result<Event, Infallible>>>"));
        assert!(code.contains("KeepAlive::new()"));
        assert!(code.contains("Duration::from_secs(30)"));
        assert!(code.contains("sse_error_event"));

        // Auth type
        assert!(code.contains("Option<Extension<crate::AuthInfo>>"));

        // UUID wrapper path param
        assert!(code.contains("crate::core::Uuid"));
        assert!(code.contains("user_id_value"));
        assert!(code.contains("body.user_id = Some("));

        assert_golden("streaming_uuid_auth.rs", &code);
        syn::parse_file(&code).expect("generated code should be valid Rust syntax");
    }

    /// Enum path parameter with type resolution.
    #[test]
    fn snapshot_enum_path_param() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("providers.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![make_message(
                    "UnlinkRequest",
                    &[("provider", field_type::ENUM, Some(".test.v1.Provider"))],
                )],
                enum_type: vec![EnumDescriptorProto {
                    name: Some("Provider".to_string()),
                    value: vec![
                        EnumValueDescriptorProto {
                            name: Some("PROVIDER_UNSPECIFIED".to_string()),
                            number: Some(0),
                        },
                        EnumValueDescriptorProto {
                            name: Some("GOOGLE".to_string()),
                            number: Some(1),
                        },
                    ],
                }],
                service: vec![ServiceDescriptorProto {
                    name: Some("ProviderService".to_string()),
                    method: vec![make_method(
                        "Unlink",
                        ".test.v1.UnlinkRequest",
                        ".google.protobuf.Empty",
                        HttpPattern::Delete("/v1/providers/{provider}".to_string()),
                        "",
                        false,
                    )],
                }],
            }],
        };

        let config = RestCodegenConfig::new().package("test.v1", "test");
        let code = generate(&encode_fdset(&fdset), &config).unwrap();

        // Enum parsing
        assert!(code.contains("from_str_name("));
        assert!(code.contains("crate::test::Provider"));
        assert!(code.contains("to_ascii_uppercase()"));
        assert!(code.contains("invalid enum value for 'provider'"));

        assert_golden("enum_path_param.rs", &code);
        syn::parse_file(&code).expect("generated code should be valid Rust syntax");
    }

    /// Multiple services from different packages in a single descriptor.
    #[test]
    fn snapshot_multi_service() {
        let fdset = FileDescriptorSet {
            file: vec![
                FileDescriptorProto {
                    name: Some("auth.proto".to_string()),
                    package: Some("auth.v1".to_string()),
                    message_type: vec![
                        make_message("LoginRequest", &[("email", field_type::STRING, None)]),
                        make_message("LoginResponse", &[("token", field_type::STRING, None)]),
                    ],
                    enum_type: vec![],
                    service: vec![ServiceDescriptorProto {
                        name: Some("AuthService".to_string()),
                        method: vec![make_method(
                            "Login",
                            ".auth.v1.LoginRequest",
                            ".auth.v1.LoginResponse",
                            HttpPattern::Post("/v1/auth/login".to_string()),
                            "*",
                            false,
                        )],
                    }],
                },
                FileDescriptorProto {
                    name: Some("users.proto".to_string()),
                    package: Some("users.v1".to_string()),
                    message_type: vec![
                        make_message("ListUsersRequest", &[]),
                        make_message("User", &[("name", field_type::STRING, None)]),
                    ],
                    enum_type: vec![],
                    service: vec![ServiceDescriptorProto {
                        name: Some("UserService".to_string()),
                        method: vec![make_method(
                            "ListUsers",
                            ".users.v1.ListUsersRequest",
                            ".users.v1.User",
                            HttpPattern::Get("/v1/users".to_string()),
                            "",
                            true,
                        )],
                    }],
                },
            ],
        };

        let config = RestCodegenConfig::new()
            .package("auth.v1", "auth")
            .package("users.v1", "users")
            .public_methods(&["Login"]);

        let code = generate(&encode_fdset(&fdset), &config).unwrap();

        // Both services present
        assert!(code.contains("auth_service_rest_router"));
        assert!(code.contains("user_service_rest_router"));
        assert!(code.contains("rest_auth_service_login"));
        assert!(code.contains("rest_user_service_list_users"));

        // Combined router has both type params
        assert!(code.contains("fn all_rest_routes<S0, S1>"));

        // Public paths
        assert!(code.contains("\"/v1/auth/login\""));

        assert_golden("multi_service.rs", &code);
        syn::parse_file(&code).expect("generated code should be valid Rust syntax");
    }

    /// PUT endpoint with body and path param.
    #[test]
    fn snapshot_put_with_body_and_path() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("items.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![
                    make_message(
                        "ReplaceItemRequest",
                        &[
                            ("item_id", field_type::STRING, None),
                            ("name", field_type::STRING, None),
                        ],
                    ),
                    make_message("Item", &[("name", field_type::STRING, None)]),
                ],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("ItemService".to_string()),
                    method: vec![make_method(
                        "ReplaceItem",
                        ".test.v1.ReplaceItemRequest",
                        ".test.v1.Item",
                        HttpPattern::Put("/v1/items/{item_id}".to_string()),
                        "*",
                        false,
                    )],
                }],
            }],
        };

        let config = RestCodegenConfig::new().package("test.v1", "test");
        let code = generate(&encode_fdset(&fdset), &config).unwrap();

        assert!(code.contains("axum::routing::put("));
        assert!(code.contains("Path(item_id): Path<String>"));
        assert!(code.contains("Json(mut body)"));
        assert!(code.contains("body.item_id = item_id;"));

        assert_golden("put_body_path.rs", &code);
        syn::parse_file(&code).expect("generated code should be valid Rust syntax");
    }

    /// Nested message types are included in field type resolution.
    ///
    /// Before the `collect_message_fields` recursion fix, nested messages
    /// were not registered in the field-type map, causing path parameter
    /// type resolution to fall back to `String` instead of the real type.
    #[test]
    fn collect_field_types_includes_nested_messages() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("nested.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Outer".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("name".to_string()),
                        r#type: Some(field_type::STRING),
                        type_name: None,
                        options: None,
                    }],
                    nested_type: vec![DescriptorProto {
                        name: Some("Inner".to_string()),
                        field: vec![FieldDescriptorProto {
                            name: Some("item_id".to_string()),
                            r#type: Some(field_type::INT32),
                            type_name: None,
                            options: None,
                        }],
                        nested_type: vec![
                            // Doubly-nested
                            DescriptorProto {
                                name: Some("Deep".to_string()),
                                field: vec![FieldDescriptorProto {
                                    name: Some("x".to_string()),
                                    r#type: Some(field_type::STRING),
                                    type_name: None,
                                    options: None,
                                }],
                                nested_type: vec![],
                            },
                        ],
                    }],
                }],
                enum_type: vec![],
                service: vec![],
            }],
        };

        let types = collect_field_types(&fdset);

        // Top-level message
        let outer = types
            .get(".test.v1.Outer")
            .expect("Outer should be present");
        assert_eq!(outer["name"].type_id, field_type::STRING);

        // First-level nested message
        let inner = types
            .get(".test.v1.Outer.Inner")
            .expect("Outer.Inner should be present (recursion fix)");
        assert_eq!(inner["item_id"].type_id, field_type::INT32);

        // Doubly-nested message
        let deep = types
            .get(".test.v1.Outer.Inner.Deep")
            .expect("Outer.Inner.Deep should be present (deep recursion)");
        assert_eq!(deep["x"].type_id, field_type::STRING);
    }

    /// Service with no HTTP annotations should produce valid but minimal code.
    #[test]
    fn generate_service_without_http_annotations() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("no_http.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![make_message("Req", &[("name", field_type::STRING, None)])],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("NoHttpService".to_string()),
                    method: vec![MethodDescriptorProto {
                        name: Some("DoStuff".to_string()),
                        input_type: Some(".test.v1.Req".to_string()),
                        output_type: Some(".test.v1.Req".to_string()),
                        options: None, // No HTTP annotation
                        client_streaming: None,
                        server_streaming: None,
                    }],
                }],
            }],
        };

        let config = RestCodegenConfig::new().package("test.v1", "test");
        let code = generate(&encode_fdset(&fdset), &config).unwrap();
        // Service has no HTTP methods, so no router function should be generated
        assert!(!code.contains("no_http_service_rest_router"));
        // But the code should still be valid Rust
        syn::parse_file(&code).expect("code without HTTP methods should be valid Rust");
    }

    /// Nested path param without `wrapper_type` configured should return `MissingWrapperType`.
    #[test]
    fn generate_nested_path_without_wrapper_type_errors() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("nested.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![
                    make_message("UpdateReq", &[("user_id", 11, None)]),
                    make_message("User", &[("name", field_type::STRING, None)]),
                ],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("UserService".to_string()),
                    method: vec![make_method(
                        "UpdateUser",
                        ".test.v1.UpdateReq",
                        ".test.v1.User",
                        HttpPattern::Patch("/v1/users/{user_id.value}".to_string()),
                        "*",
                        false,
                    )],
                }],
            }],
        };

        // No wrapper_type configured
        let config = RestCodegenConfig::new().package("test.v1", "test");
        let result = generate(&encode_fdset(&fdset), &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, GenerateError::MissingWrapperType { .. }),
            "expected MissingWrapperType, got: {err}",
        );
        assert!(err.to_string().contains("user_id.value"));
    }

    /// Auto-discovery should find services with HTTP annotations even without explicit packages.
    #[test]
    fn auto_discover_packages() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("auto.proto".to_string()),
                package: Some("auto.v1".to_string()),
                message_type: vec![
                    make_message("PingRequest", &[]),
                    make_message("PingResponse", &[("ok", field_type::BOOL, None)]),
                ],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("HealthService".to_string()),
                    method: vec![make_method(
                        "Ping",
                        ".auto.v1.PingRequest",
                        ".auto.v1.PingResponse",
                        HttpPattern::Get("/v1/health/ping".to_string()),
                        "",
                        false,
                    )],
                }],
            }],
        };

        // No packages registered — auto-discovery should kick in
        let config = RestCodegenConfig::new();
        let code = generate(&encode_fdset(&fdset), &config).unwrap();
        assert!(
            code.contains("health_service_rest_router"),
            "auto-discovered service should produce a router",
        );
        syn::parse_file(&code).expect("auto-discovered code should be valid Rust");
    }
}
