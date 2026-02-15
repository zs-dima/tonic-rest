//! Internal types used during codegen — not part of the public API.

use std::collections::HashMap;

/// Parsed service info from proto descriptors.
#[derive(Debug)]
pub struct ServiceRoute {
    /// Rust module name for the service package (e.g., "auth", "users")
    pub package_mod: String,
    /// Proto service name (e.g., `AuthService`, `UserService`)
    pub service_name: String,
    /// Individual method routes
    pub methods: Vec<MethodRoute>,
}

#[derive(Debug)]
pub struct MethodRoute {
    /// Proto method name (e.g., `ListUsers`)
    pub proto_name: String,
    /// Method name in `snake_case` (e.g., `list_users`)
    pub rust_name: String,
    /// HTTP method (get, post, put, patch, delete)
    pub http_method: String,
    /// URL path from proto (e.g., `/v1/users/{user_id.value}`)
    pub path: String,
    /// Axum-compatible path (e.g., `/v1/users/{user_id_value}`)
    pub axum_path: String,
    /// Whether request body is used ("*" = full body)
    pub has_body: bool,
    /// Whether the method returns a stream
    pub server_streaming: bool,
    /// Rust input type path
    pub input_type: String,
    /// Rust output type path
    pub output_type: String,
    /// Whether the output is google.protobuf.Empty
    pub returns_empty: bool,
    /// Path parameters extracted from URL pattern
    pub path_params: Vec<PathParam>,
}

/// A path parameter extracted from the URL pattern.
#[derive(Debug)]
pub struct PathParam {
    /// Axum param name (e.g., `user_id_value`)
    pub axum_name: String,
    /// How to assign this param to the request body
    pub assignment: ParamAssignment,
}

/// How a path parameter maps to a proto request field.
#[derive(Debug)]
pub enum ParamAssignment {
    /// Nested UUID wrapper: `{user_id.value}` → `body.user_id = Some(Uuid { value })`
    UuidWrapper { parent_field: String },
    /// Simple string field: `{device_id}` → `body.device_id = device_id`
    StringField { field_name: String },
    /// Typed numeric/bool field: `{page}` → parsed by Axum's `Path<i32>` extractor
    TypedField {
        field_name: String,
        /// Rust type for the path extractor (e.g., `i32`, `u32`, `i64`, `u64`, `bool`)
        rust_type: &'static str,
    },
    /// Enum field (i32 in prost): `{provider}` → parse via `EnumType::from_str_name()`, 400 on invalid
    EnumField {
        field_name: String,
        /// Rust type path for the enum (e.g., `crate::auth::OAuthProvider`)
        enum_rust_type: String,
    },
}

/// Per-field type info: proto type id + optional fully-qualified enum type name.
#[derive(Debug, Clone)]
pub struct FieldTypeInfo {
    pub type_id: i32,
    /// For enum fields: the FQN (e.g., `.auth.v1.OAuthProvider`)
    pub enum_type_name: Option<String>,
}

/// Map of fully-qualified message name → field name → field type info.
pub type MessageFieldTypes = HashMap<String, HashMap<String, FieldTypeInfo>>;
