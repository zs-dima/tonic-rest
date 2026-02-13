//! Proto descriptor parsing for extracting RPC metadata.
//!
//! Parses a `FileDescriptorSet` from raw bytes and extracts:
//! - **Streaming ops**: `(HTTP method, path)` pairs for server-streaming RPCs
//! - **Operation ID mapping**: `ServiceName_MethodName` for every annotated RPC
//! - **Validation constraints**: `validate.rules` → JSON Schema constraints
//! - **Enum rewrites**: prefix-stripped enum value mappings
//! - **Redirect paths**: endpoints returning 302 redirects
//! - **UUID schema**: auto-detected UUID wrapper type
//! - **Path param constraints**: per-endpoint path parameter metadata
//!
//! This keeps proto files as the **single source of truth** — the `OpenAPI`
//! post-processor auto-detects streaming endpoints and resolves operation IDs
//! instead of relying on hardcoded lists.

use std::collections::HashMap;

use prost::Message;

use crate::descriptor::{
    self, field_type, DescriptorProto, FieldDescriptorProto, FileDescriptorSet,
};
use crate::error;

/// A streaming operation: `(HTTP method, path)`.
///
/// Extracted from proto RPCs that are `server_streaming = true` and have
/// a `google.api.http` annotation.
#[derive(Debug, Clone)]
pub struct StreamingOp {
    /// HTTP method (e.g., `"get"`).
    pub method: String,
    /// URL path (e.g., `"/v1/users"`).
    pub path: String,
}

/// All RPC metadata extracted from proto descriptors.
///
/// Populated once via [`discover()`], then consumed by
/// [`PatchConfig`](crate::PatchConfig). Access extracted data through the
/// public accessor methods below.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProtoMetadata {
    /// Server-streaming RPCs with HTTP annotations.
    pub(crate) streaming_ops: Vec<StreamingOp>,

    /// All RPC operation IDs, keyed by short method name.
    pub(crate) operation_ids: Vec<OperationEntry>,

    /// Validation constraints extracted from `validate.rules` field options.
    pub(crate) field_constraints: Vec<SchemaConstraints>,

    /// Enum value rewrites for fields whose runtime serde strips prefixes.
    pub(crate) enum_rewrites: Vec<EnumRewrite>,

    /// HTTP paths for redirect endpoints (auto-detected from proto).
    pub(crate) redirect_paths: Vec<String>,

    /// gnostic schema name for UUID wrapper type (e.g., `core.v1.UUID`).
    pub(crate) uuid_schema: Option<String>,

    /// Field constraints for path parameters, keyed by HTTP path.
    pub(crate) path_param_constraints: Vec<PathParamInfo>,

    /// Raw → stripped enum value mapping for all prefix-stripped enums.
    pub(crate) enum_value_map: HashMap<String, String>,
}

impl ProtoMetadata {
    /// Server-streaming RPCs with HTTP annotations.
    #[must_use]
    pub fn streaming_ops(&self) -> &[StreamingOp] {
        &self.streaming_ops
    }

    /// All RPC operation IDs extracted from the descriptor set.
    #[must_use]
    pub fn operation_ids(&self) -> &[OperationEntry] {
        &self.operation_ids
    }

    /// Validation constraints from `validate.rules` field options.
    #[must_use]
    pub fn field_constraints(&self) -> &[SchemaConstraints] {
        &self.field_constraints
    }

    /// Enum value rewrites for prefix-stripped enums.
    #[must_use]
    pub fn enum_rewrites(&self) -> &[EnumRewrite] {
        &self.enum_rewrites
    }

    /// HTTP paths for redirect endpoints.
    #[must_use]
    pub fn redirect_paths(&self) -> &[String] {
        &self.redirect_paths
    }

    /// gnostic schema name for the auto-detected UUID wrapper type.
    #[must_use]
    pub fn uuid_schema(&self) -> Option<&str> {
        self.uuid_schema.as_deref()
    }

    /// Path parameter constraints, keyed by HTTP path template.
    #[must_use]
    pub fn path_param_constraints(&self) -> &[PathParamInfo] {
        &self.path_param_constraints
    }

    /// Raw → stripped enum value mapping for all prefix-stripped enums.
    #[must_use]
    pub fn enum_value_map(&self) -> &HashMap<String, String> {
        &self.enum_value_map
    }
}

/// Maps a short proto method name to its gnostic operation ID.
#[derive(Debug, Clone)]
pub struct OperationEntry {
    /// Short method name from proto (e.g., `Authenticate`).
    pub method_name: String,
    /// gnostic operation ID: `ServiceName_MethodName`.
    pub operation_id: String,
}

/// Validation constraints for all fields in one schema.
#[derive(Debug, Clone)]
pub struct SchemaConstraints {
    /// Schema name in gnostic format (e.g., `auth.v1.ClientInfo`).
    pub schema: String,
    /// Per-field constraints.
    pub fields: Vec<FieldConstraint>,
}

/// Enum value rewrite for a schema field whose runtime serde strips prefixes.
#[derive(Debug, Clone)]
pub struct EnumRewrite {
    /// Schema name in gnostic format (e.g., `operations.v1.HealthResponse`).
    pub schema: String,
    /// Field name in camelCase (e.g., `status`).
    pub field: String,
    /// Rewritten enum values matching runtime wire format (e.g., `["healthy", "unhealthy"]`).
    pub values: Vec<String>,
}

/// Path parameter constraint info for a specific HTTP endpoint.
#[derive(Debug, Clone)]
pub struct PathParamInfo {
    /// HTTP path template (e.g., `/v1/auth/sessions/{device_id}`).
    pub path: String,
    /// Path parameter constraints, one per template variable.
    pub params: Vec<PathParamConstraint>,
}

/// Constraint for a single path parameter.
#[derive(Debug, Clone)]
pub struct PathParamConstraint {
    /// Parameter name as it appears in the URL template (`snake_case`).
    pub name: String,
    /// Human-readable description from proto field comment (if available).
    pub description: Option<String>,
    /// Whether this parameter is a UUID type (references `core.v1.UUID`).
    pub is_uuid: bool,
    /// `minLength` (string) or `minimum` (integer).
    pub min: Option<u64>,
    /// `maxLength` (string) or `maximum` (integer).
    pub max: Option<u64>,
}

/// A single field's validation constraints, mapped to JSON Schema.
#[derive(Debug, Clone)]
pub struct FieldConstraint {
    /// Field name in camelCase (gnostic output format).
    pub field: String,
    /// `minLength` for strings, `minimum` for integers.
    pub min: Option<u64>,
    /// `maxLength` for strings, `maximum` for integers.
    pub max: Option<u64>,
    /// Regex pattern (from `validate.rules.string.pattern`).
    pub pattern: Option<String>,
    /// Enum of allowed string values (from `validate.rules.string.in`).
    pub enum_values: Vec<String>,
    /// Whether this is a `message.required` field → goes into schema `required` array.
    pub required: bool,
    /// Whether this is a UUID field (from `validate.rules.string.uuid = true`).
    pub is_uuid: bool,
    /// Proto field type: true if numeric (int32/uint32/uint64), false if string.
    pub is_numeric: bool,
    /// `minimum` for signed integers (int32). Mutually exclusive with `min`.
    /// When present, the JSON Schema should use this instead of `min`.
    pub signed_min: Option<i64>,
    /// `maximum` for signed integers (int32). Mutually exclusive with `max`.
    /// When present, the JSON Schema should use this instead of `max`.
    pub signed_max: Option<i64>,
}

/// Parse proto descriptor bytes and extract all RPC metadata.
///
/// Accepts raw `FileDescriptorSet` bytes (e.g., from `buf build --as-file-descriptor-set`
/// or `tonic_build`'s compiled descriptor). Returns metadata shared across all `OpenAPI`
/// patches.
///
/// # Errors
///
/// Returns an error if the descriptor bytes cannot be decoded.
pub fn discover(descriptor_bytes: &[u8]) -> error::Result<ProtoMetadata> {
    let fdset = FileDescriptorSet::decode(descriptor_bytes)?;

    let streaming_ops = extract_streaming_ops(&fdset);
    let operation_ids = extract_operation_ids(&fdset);
    let field_constraints = extract_field_constraints(&fdset);
    let (enum_rewrites, enum_value_map) = extract_enum_rewrites(&fdset);
    let redirect_paths = extract_redirect_paths(&fdset);
    let uuid_schema = detect_uuid_schema(&fdset);
    let path_param_constraints = extract_path_param_constraints(&fdset);

    Ok(ProtoMetadata {
        streaming_ops,
        operation_ids,
        field_constraints,
        enum_rewrites,
        redirect_paths,
        uuid_schema,
        path_param_constraints,
        enum_value_map,
    })
}

/// Resolve short method names to gnostic operation IDs using proto metadata.
///
/// Given `["Authenticate", "SignUp"]` and the proto descriptor mapping,
/// returns `["AuthService_Authenticate", "AuthService_SignUp"]`.
///
/// Supports both bare method names (`"Authenticate"`) and service-qualified
/// names (`"AuthService.Authenticate"`). Qualified names are matched first;
/// bare names fall back to unambiguous lookup (exactly one match).
///
/// # Errors
///
/// Returns an error if any method name is not found in the proto descriptors,
/// or if a bare method name matches multiple services (ambiguous).
pub fn resolve_operation_ids(
    metadata: &ProtoMetadata,
    method_names: &[&str],
) -> error::Result<Vec<String>> {
    method_names
        .iter()
        .map(|name| resolve_single_operation_id(metadata, name))
        .collect()
}

/// Resolve a single method name to its operation ID.
///
/// Checks for qualified `Service.Method` format first, then falls back
/// to bare method name with ambiguity detection.
fn resolve_single_operation_id(metadata: &ProtoMetadata, name: &str) -> error::Result<String> {
    // Check for qualified "Service.Method" format
    if let Some((service, method)) = name.split_once('.') {
        let qualified_id = format!("{service}_{method}");
        return metadata
            .operation_ids
            .iter()
            .find(|e| e.operation_id == qualified_id)
            .map(|e| e.operation_id.clone())
            .ok_or_else(|| error::Error::MethodNotFound {
                method: name.to_string(),
            });
    }

    // Bare method name: collect all matches
    let matches: Vec<&OperationEntry> = metadata
        .operation_ids
        .iter()
        .filter(|e| e.method_name == *name)
        .collect();

    match matches.len() {
        0 => Err(error::Error::MethodNotFound {
            method: name.to_string(),
        }),
        1 => Ok(matches[0].operation_id.clone()),
        _ => {
            let services: Vec<&str> = matches.iter().map(|e| e.operation_id.as_str()).collect();
            Err(error::Error::AmbiguousMethodName {
                method: name.to_string(),
                candidates: services.iter().map(ToString::to_string).collect(),
            })
        }
    }
}

/// Walk all services/methods and collect streaming ops with HTTP annotations.
fn extract_streaming_ops(fdset: &FileDescriptorSet) -> Vec<StreamingOp> {
    let mut ops = Vec::new();

    for file in &fdset.file {
        for service in &file.service {
            for method in &service.method {
                if !method.server_streaming.unwrap_or(false) {
                    continue;
                }

                let Some((http_method, path)) = descriptor::extract_http_pattern(method) else {
                    continue;
                };

                ops.push(StreamingOp {
                    method: http_method.to_string(),
                    path: path.to_string(),
                });
            }
        }
    }

    ops
}

/// Walk all services/methods and build `method_name → operation_id` mapping.
fn extract_operation_ids(fdset: &FileDescriptorSet) -> Vec<OperationEntry> {
    let mut entries = Vec::new();

    for file in &fdset.file {
        for service in &file.service {
            let service_name = service.name.as_deref().unwrap_or("");

            for method in &service.method {
                if method
                    .options
                    .as_ref()
                    .and_then(|o| o.http.as_ref())
                    .and_then(|h| h.pattern.as_ref())
                    .is_none()
                {
                    continue;
                }

                let method_name = method.name.as_deref().unwrap_or("");
                entries.push(OperationEntry {
                    method_name: method_name.to_string(),
                    operation_id: format!("{service_name}_{method_name}"),
                });
            }
        }
    }

    entries
}

/// Walk all messages and extract `validate.rules` as `SchemaConstraints`.
fn extract_field_constraints(fdset: &FileDescriptorSet) -> Vec<SchemaConstraints> {
    let mut result = Vec::new();

    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        collect_message_constraints(&mut result, package, &file.message_type);
    }

    result
}

/// Recursively collect constraints from messages (handles nested types).
fn collect_message_constraints(
    result: &mut Vec<SchemaConstraints>,
    parent_path: &str,
    messages: &[DescriptorProto],
) {
    for msg in messages {
        let msg_name = msg.name.as_deref().unwrap_or("");
        let schema = format!("{parent_path}.{msg_name}");

        let fields: Vec<FieldConstraint> =
            msg.field.iter().filter_map(field_to_constraint).collect();

        if !fields.is_empty() {
            result.push(SchemaConstraints {
                schema: schema.clone(),
                fields,
            });
        }

        collect_message_constraints(result, &schema, &msg.nested_type);
    }
}

/// Convert a single proto field's `validate.rules` to a `FieldConstraint`.
#[allow(
    clippy::too_many_lines,
    clippy::case_sensitive_file_extension_comparisons
)]
fn field_to_constraint(field: &FieldDescriptorProto) -> Option<FieldConstraint> {
    const JSON_SAFE_INT_MAX: u64 = 9_007_199_254_740_991;

    let rules = field.options.as_ref()?.rules.as_ref()?;
    let proto_name = field.name.as_deref().unwrap_or("");
    let camel_name = snake_to_lower_camel(proto_name);
    let field_type_id = field.r#type.unwrap_or(0);

    let msg_required = rules
        .message
        .as_ref()
        .and_then(|m| m.required)
        .unwrap_or(false);

    // String rules
    if let Some(sr) = &rules.string {
        let has_content = sr.min_len.is_some()
            || sr.max_len.is_some()
            || sr.pattern.is_some()
            || !sr.r#in.is_empty()
            || sr.uuid.unwrap_or(false);

        if has_content || msg_required {
            let implied_required = sr.min_len.unwrap_or(0) >= 1 || !sr.r#in.is_empty();
            return Some(FieldConstraint {
                field: camel_name,
                min: sr.min_len,
                max: sr.max_len,
                signed_min: None,
                signed_max: None,
                pattern: sr.pattern.clone(),
                enum_values: sr.r#in.clone(),
                required: msg_required || implied_required,
                is_uuid: sr.uuid.unwrap_or(false),
                is_numeric: false,
            });
        }
    }

    // Int32 rules — use i64 to avoid sign loss and overflow
    if let Some(ir) = &rules.int32 {
        let min = ir
            .gte
            .map(i64::from)
            .or(ir.gt.map(|v| i64::from(v).saturating_add(1)));
        let max = ir
            .lte
            .map(i64::from)
            .or(ir.lt.map(|v| i64::from(v).saturating_sub(1)));
        if min.is_some() || max.is_some() {
            return Some(FieldConstraint {
                field: camel_name,
                min: None,
                max: None,
                signed_min: min,
                signed_max: max,
                pattern: None,
                enum_values: Vec::new(),
                required: msg_required,
                is_uuid: false,
                is_numeric: true,
            });
        }
    }

    // UInt32 rules — use saturating_sub to avoid underflow when lt = 0
    if let Some(ur) = &rules.uint32 {
        let min = ur
            .gte
            .map(u64::from)
            .or(ur.gt.map(|v| u64::from(v).saturating_add(1)));
        let max = ur
            .lte
            .map(u64::from)
            .or(ur.lt.map(|v| u64::from(v).saturating_sub(1)));
        if min.is_some() || max.is_some() {
            return Some(FieldConstraint {
                field: camel_name,
                min,
                max,
                signed_min: None,
                signed_max: None,
                pattern: None,
                enum_values: Vec::new(),
                required: msg_required,
                is_uuid: false,
                is_numeric: true,
            });
        }
    }

    // UInt64 rules — propagate when within JSON safe integer range
    // Convert exclusive bounds to inclusive (gt → +1, lt → −1) like int32/uint32.
    if let Some(u64r) = &rules.uint64 {
        let min = u64r.gte.or(u64r.gt.map(|v| v.saturating_add(1)));
        let max = u64r.lte.or(u64r.lt.map(|v| v.saturating_sub(1)));
        let min_val = min.unwrap_or(0);
        let max_val = max;

        let fits_in_json = max_val.is_some_and(|m| m <= JSON_SAFE_INT_MAX);

        if fits_in_json || msg_required {
            return Some(FieldConstraint {
                field: camel_name,
                min: if fits_in_json && min_val > 0 {
                    Some(min_val)
                } else {
                    None
                },
                max: if fits_in_json { max_val } else { None },
                signed_min: None,
                signed_max: None,
                pattern: None,
                enum_values: Vec::new(),
                required: msg_required,
                is_uuid: false,
                is_numeric: fits_in_json,
            });
        }
    }

    // Enum rules (not_in typically used for "must not be UNSPECIFIED")
    if let Some(er) = &rules.r#enum {
        let enum_required = er.not_in.contains(&0);
        if enum_required || msg_required {
            return Some(FieldConstraint {
                field: camel_name,
                min: None,
                max: None,
                signed_min: None,
                signed_max: None,
                pattern: None,
                enum_values: Vec::new(),
                required: enum_required || msg_required,
                is_uuid: false,
                is_numeric: false,
            });
        }
    }

    // Message-only constraint (required without string/int rules)
    if msg_required {
        let is_uuid = field_type_id == field_type::MESSAGE
            && field
                .type_name
                .as_deref()
                .is_some_and(|t| t.ends_with(".UUID")); // proto type name, not file extension

        return Some(FieldConstraint {
            field: camel_name,
            min: None,
            max: None,
            signed_min: None,
            signed_max: None,
            pattern: None,
            enum_values: Vec::new(),
            required: true,
            is_uuid,
            is_numeric: false,
        });
    }

    None
}

/// Extract enum rewrites for schemas containing prefix-stripped enums.
fn extract_enum_rewrites(fdset: &FileDescriptorSet) -> (Vec<EnumRewrite>, HashMap<String, String>) {
    let mut prefix_enums: Vec<(String, String, Vec<String>)> = Vec::new();

    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        for enum_desc in &file.enum_type {
            let enum_name = enum_desc.name.as_deref().unwrap_or("");
            let fqn = format!(".{package}.{enum_name}");

            let values: Vec<&str> = enum_desc
                .value
                .iter()
                .filter_map(|v| v.name.as_deref())
                .collect();

            let Some(detected_prefix) = detect_enum_prefix(&values) else {
                continue;
            };

            if values.iter().all(|v| v.starts_with(&detected_prefix)) {
                let stripped: Vec<String> = values
                    .iter()
                    .map(|v| v[detected_prefix.len()..].to_lowercase())
                    .collect();
                prefix_enums.push((fqn, detected_prefix, stripped));
            }
        }
    }

    if prefix_enums.is_empty() {
        return (Vec::new(), HashMap::new());
    }

    // Build global raw → stripped value map
    let mut enum_value_map = HashMap::new();
    for file in &fdset.file {
        for enum_desc in &file.enum_type {
            let values: Vec<&str> = enum_desc
                .value
                .iter()
                .filter_map(|v| v.name.as_deref())
                .collect();

            let Some(detected_prefix) = detect_enum_prefix(&values) else {
                continue;
            };

            for raw in &values {
                if let Some(suffix) = raw.strip_prefix(detected_prefix.as_str()) {
                    enum_value_map.insert(raw.to_string(), suffix.to_lowercase());
                }
            }
        }
    }

    // Find all message fields referencing these enums (type 14 = TYPE_ENUM)
    let mut rewrites = Vec::new();

    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        collect_enum_rewrite_fields(&mut rewrites, package, &file.message_type, &prefix_enums);
    }

    (rewrites, enum_value_map)
}

/// Recursively scan messages for enum fields referencing prefix-stripped enums.
fn collect_enum_rewrite_fields(
    rewrites: &mut Vec<EnumRewrite>,
    parent_path: &str,
    messages: &[DescriptorProto],
    prefix_enums: &[(String, String, Vec<String>)],
) {
    for msg in messages {
        let msg_name = msg.name.as_deref().unwrap_or("");
        let schema = format!("{parent_path}.{msg_name}");

        for field in &msg.field {
            if field.r#type != Some(field_type::ENUM) {
                continue;
            }

            let Some(type_name) = field.type_name.as_deref() else {
                continue;
            };

            if let Some((_, _, stripped_values)) =
                prefix_enums.iter().find(|(fqn, _, _)| fqn == type_name)
            {
                let field_name = snake_to_lower_camel(field.name.as_deref().unwrap_or(""));

                rewrites.push(EnumRewrite {
                    schema: schema.clone(),
                    field: field_name,
                    values: stripped_values.clone(),
                });
            }
        }

        collect_enum_rewrite_fields(rewrites, &schema, &msg.nested_type, prefix_enums);
    }
}

/// Detect the common `UPPER_SNAKE_CASE_` prefix shared by all enum values.
///
/// Returns `None` if values don't share a common `_`-terminated prefix.
fn detect_enum_prefix(values: &[&str]) -> Option<String> {
    if values.is_empty() {
        return None;
    }

    let first = values[0];
    let common_len = first
        .char_indices()
        .find(|&(i, _)| values[1..].iter().any(|v| !v[..].starts_with(&first[..=i])))
        .map_or(first.len(), |(i, _)| i);

    let prefix = &first[..common_len];
    let last_underscore = prefix.rfind('_')?;
    let prefix = &first[..=last_underscore];

    if prefix.len() < 3 {
        return None;
    }

    Some(prefix.to_string())
}

/// Detect redirect endpoints by examining response message types.
fn extract_redirect_paths(fdset: &FileDescriptorSet) -> Vec<String> {
    let mut redirect_types: Vec<String> = Vec::new();
    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        collect_redirect_message_types(&mut redirect_types, package, &file.message_type);
    }

    if redirect_types.is_empty() {
        return Vec::new();
    }

    let mut paths = Vec::new();
    for file in &fdset.file {
        for service in &file.service {
            for method in &service.method {
                let output_type = method.output_type.as_deref().unwrap_or("");
                if !redirect_types.iter().any(|t| t == output_type) {
                    continue;
                }

                if let Some((_, path)) = descriptor::extract_http_pattern(method) {
                    paths.push(path.to_string());
                }
            }
        }
    }

    paths
}

/// Recursively find messages with a `redirect_url` field.
fn collect_redirect_message_types(
    result: &mut Vec<String>,
    package: &str,
    messages: &[DescriptorProto],
) {
    for msg in messages {
        let msg_name = msg.name.as_deref().unwrap_or("");
        let has_redirect_url = msg
            .field
            .iter()
            .any(|f| f.name.as_deref() == Some("redirect_url"));

        if has_redirect_url {
            result.push(format!(".{package}.{msg_name}"));
        }

        collect_redirect_message_types(result, package, &msg.nested_type);
    }
}

/// Detect the UUID wrapper schema name from proto descriptors.
fn detect_uuid_schema(fdset: &FileDescriptorSet) -> Option<String> {
    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        for msg in &file.message_type {
            let msg_name = msg.name.as_deref().unwrap_or("");

            if msg.field.len() != 1 {
                continue;
            }
            let field = &msg.field[0];
            if field.name.as_deref() != Some("value") || field.r#type != Some(field_type::STRING) {
                continue;
            }

            let has_uuid_pattern = field
                .options
                .as_ref()
                .and_then(|o| o.rules.as_ref())
                .and_then(|r| r.string.as_ref())
                .and_then(|s| s.pattern.as_deref())
                .is_some_and(|p| p.contains("0-9a-fA-F"));

            if has_uuid_pattern {
                return Some(format!("{package}.{msg_name}"));
            }
        }
    }
    None
}

/// Extract path parameter constraints from proto HTTP path templates.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // proto type names, not file paths
fn extract_path_param_constraints(fdset: &FileDescriptorSet) -> Vec<PathParamInfo> {
    let mut messages: HashMap<String, &[FieldDescriptorProto]> = HashMap::new();
    for file in &fdset.file {
        let package = file.package.as_deref().unwrap_or("");
        collect_message_fields(&mut messages, package, &file.message_type);
    }

    let mut result = Vec::new();

    for file in &fdset.file {
        for service in &file.service {
            for method in &service.method {
                let Some((_, path)) = descriptor::extract_http_pattern(method) else {
                    continue;
                };

                let param_names: Vec<&str> = path
                    .split('{')
                    .skip(1)
                    .filter_map(|s| s.split('}').next())
                    .collect();

                if param_names.is_empty() {
                    continue;
                }

                let input_type = method.input_type.as_deref().unwrap_or("");
                let fields = messages.get(input_type).copied().unwrap_or_default();

                let params: Vec<PathParamConstraint> = param_names
                    .iter()
                    .filter_map(|&param| {
                        let root_field = param.split('.').next().unwrap_or(param);
                        let field = fields
                            .iter()
                            .find(|f| f.name.as_deref() == Some(root_field))?;

                        let is_uuid = field.r#type == Some(field_type::MESSAGE)
                            && field
                                .type_name
                                .as_deref()
                                .is_some_and(|t| t.ends_with(".UUID")); // proto type name, not file extension

                        let (min, max) = field
                            .options
                            .as_ref()
                            .and_then(|o| o.rules.as_ref())
                            .and_then(|rules| rules.string.as_ref())
                            .map(|s| (s.min_len, s.max_len))
                            .unwrap_or_default();

                        Some(PathParamConstraint {
                            name: param
                                .split('.')
                                .enumerate()
                                .map(|(i, seg)| {
                                    if i == 0 {
                                        snake_to_lower_camel(seg)
                                    } else {
                                        seg.to_string()
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("."),
                            description: None,
                            is_uuid,
                            min,
                            max,
                        })
                    })
                    .collect();

                if !params.is_empty() {
                    let gnostic_path = convert_path_template_to_camel(path);
                    result.push(PathParamInfo {
                        path: gnostic_path,
                        params,
                    });
                }
            }
        }
    }

    result
}

/// Convert proto path template variables to gnostic's camelCase format.
fn convert_path_template_to_camel(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut rest = path;

    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let end = start + end;

        result.push_str(&rest[..=start]);
        let var = &rest[start + 1..end];

        if let Some((root, suffix)) = var.split_once('.') {
            result.push_str(&snake_to_lower_camel(root));
            result.push('.');
            result.push_str(suffix);
        } else {
            result.push_str(&snake_to_lower_camel(var));
        }

        result.push('}');
        rest = &rest[end + 1..];
    }

    result.push_str(rest);
    result
}

/// Build a lookup table: message FQN → field descriptors.
fn collect_message_fields<'a>(
    result: &mut HashMap<String, &'a [FieldDescriptorProto]>,
    parent_path: &str,
    messages: &'a [DescriptorProto],
) {
    for msg in messages {
        let msg_name = msg.name.as_deref().unwrap_or("");
        let fqn = format!(".{parent_path}.{msg_name}");
        result.insert(fqn.clone(), &msg.field);
        collect_message_fields(result, &fqn[1..], &msg.nested_type);
    }
}

/// Convert `snake_case` to `lowerCamelCase` (matches gnostic JSON field names).
pub(crate) fn snake_to_lower_camel(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use prost::Message as _;

    use crate::descriptor::*;

    use super::*;

    fn make_field(name: &str, ty: i32) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            r#type: Some(ty),
            type_name: None,
            options: None,
        }
    }

    fn make_service_with_http(
        service_name: &str,
        method_name: &str,
        pattern: HttpPattern,
        server_streaming: bool,
    ) -> ServiceDescriptorProto {
        ServiceDescriptorProto {
            name: Some(service_name.to_string()),
            method: vec![MethodDescriptorProto {
                name: Some(method_name.to_string()),
                input_type: Some(".test.v1.Request".to_string()),
                output_type: Some(".test.v1.Response".to_string()),
                options: Some(MethodOptions {
                    http: Some(HttpRule {
                        pattern: Some(pattern),
                        body: String::new(),
                    }),
                }),
                client_streaming: None,
                server_streaming: Some(server_streaming),
            }],
        }
    }

    fn make_fdset_with_services(services: Vec<ServiceDescriptorProto>) -> FileDescriptorSet {
        FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    field: vec![make_field("name", field_type::STRING)],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: services,
            }],
        }
    }

    #[test]
    fn discover_extracts_streaming_ops() {
        let fdset = make_fdset_with_services(vec![make_service_with_http(
            "TestService",
            "ListItems",
            HttpPattern::Get("/v1/items".to_string()),
            true,
        )]);
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.streaming_ops.len(), 1);
        assert_eq!(metadata.streaming_ops[0].method, "get");
        assert_eq!(metadata.streaming_ops[0].path, "/v1/items");
    }

    #[test]
    fn discover_skips_non_streaming() {
        let fdset = make_fdset_with_services(vec![make_service_with_http(
            "TestService",
            "GetItem",
            HttpPattern::Get("/v1/items/{id}".to_string()),
            false,
        )]);
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert!(metadata.streaming_ops.is_empty());
    }

    #[test]
    fn discover_extracts_operation_ids() {
        let fdset = make_fdset_with_services(vec![make_service_with_http(
            "ItemService",
            "CreateItem",
            HttpPattern::Post("/v1/items".to_string()),
            false,
        )]);
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.operation_ids.len(), 1);
        assert_eq!(metadata.operation_ids[0].method_name, "CreateItem");
        assert_eq!(
            metadata.operation_ids[0].operation_id,
            "ItemService_CreateItem"
        );
    }

    #[test]
    fn resolve_operation_ids_success() {
        let fdset = make_fdset_with_services(vec![make_service_with_http(
            "AuthService",
            "Authenticate",
            HttpPattern::Post("/v1/auth/authenticate".to_string()),
            false,
        )]);
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        let resolved = resolve_operation_ids(&metadata, &["Authenticate"]).unwrap();
        assert_eq!(resolved, vec!["AuthService_Authenticate"]);
    }

    #[test]
    fn resolve_operation_ids_missing() {
        let fdset = make_fdset_with_services(vec![]);
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        let result = resolve_operation_ids(&metadata, &["NonExistent"]);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_qualified_service_method() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    field: vec![make_field("name", field_type::STRING)],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![
                    make_service_with_http(
                        "AuthService",
                        "Delete",
                        HttpPattern::Delete("/v1/auth".to_string()),
                        false,
                    ),
                    make_service_with_http(
                        "UserService",
                        "Delete",
                        HttpPattern::Delete("/v1/users".to_string()),
                        false,
                    ),
                ],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        // Qualified name should resolve correctly
        let resolved = resolve_operation_ids(&metadata, &["AuthService.Delete"]).unwrap();
        assert_eq!(resolved, vec!["AuthService_Delete"]);

        let resolved = resolve_operation_ids(&metadata, &["UserService.Delete"]).unwrap();
        assert_eq!(resolved, vec!["UserService_Delete"]);
    }

    #[test]
    fn resolve_ambiguous_bare_name_errors() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    field: vec![make_field("name", field_type::STRING)],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![
                    make_service_with_http(
                        "AuthService",
                        "Delete",
                        HttpPattern::Delete("/v1/auth".to_string()),
                        false,
                    ),
                    make_service_with_http(
                        "UserService",
                        "Delete",
                        HttpPattern::Delete("/v1/users".to_string()),
                        false,
                    ),
                ],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        // Bare "Delete" is ambiguous — should error
        let result = resolve_operation_ids(&metadata, &["Delete"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("ambiguous"),
            "error should mention ambiguity: {err_msg}"
        );
        assert!(
            err_msg.contains("Service.Method"),
            "error should suggest qualified syntax: {err_msg}"
        );
    }

    #[test]
    fn snake_to_lower_camel_basic() {
        assert_eq!(snake_to_lower_camel("device_id"), "deviceId");
        assert_eq!(snake_to_lower_camel("user_id"), "userId");
        assert_eq!(snake_to_lower_camel("name"), "name");
        assert_eq!(snake_to_lower_camel("client_version"), "clientVersion");
    }

    #[test]
    fn convert_path_template_to_camel_works() {
        assert_eq!(
            convert_path_template_to_camel("/v1/sessions/{device_id}"),
            "/v1/sessions/{deviceId}"
        );
        assert_eq!(
            convert_path_template_to_camel("/v1/users/{user_id.value}"),
            "/v1/users/{userId.value}"
        );
        assert_eq!(convert_path_template_to_camel("/v1/items"), "/v1/items");
    }

    #[test]
    fn detect_enum_prefix_common() {
        let values = ["HEALTH_STATUS_HEALTHY", "HEALTH_STATUS_UNHEALTHY"];
        assert_eq!(
            detect_enum_prefix(&values),
            Some("HEALTH_STATUS_".to_string())
        );
    }

    #[test]
    fn detect_enum_prefix_none_for_no_common() {
        let values = ["FOO", "BAR"];
        assert_eq!(detect_enum_prefix(&values), None);
    }

    #[test]
    fn detect_enum_prefix_empty() {
        let values: &[&str] = &[];
        assert_eq!(detect_enum_prefix(values), None);
    }

    #[test]
    fn enum_rewrites_detected() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Response".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("status".to_string()),
                        r#type: Some(field_type::ENUM),
                        type_name: Some(".test.v1.Status".to_string()),
                        options: None,
                    }],
                    nested_type: vec![],
                }],
                enum_type: vec![EnumDescriptorProto {
                    name: Some("Status".to_string()),
                    value: vec![
                        EnumValueDescriptorProto {
                            name: Some("STATUS_UNSPECIFIED".to_string()),
                            number: Some(0),
                        },
                        EnumValueDescriptorProto {
                            name: Some("STATUS_ACTIVE".to_string()),
                            number: Some(1),
                        },
                    ],
                }],
                service: vec![],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.enum_rewrites.len(), 1);
        assert_eq!(metadata.enum_rewrites[0].schema, "test.v1.Response");
        assert_eq!(metadata.enum_rewrites[0].field, "status");
        assert_eq!(
            metadata.enum_rewrites[0].values,
            vec!["unspecified", "active"]
        );
    }

    #[test]
    fn redirect_paths_detected() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("RedirectResponse".to_string()),
                    field: vec![make_field("redirect_url", field_type::STRING)],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("TestService".to_string()),
                    method: vec![MethodDescriptorProto {
                        name: Some("DoRedirect".to_string()),
                        input_type: Some(".test.v1.Request".to_string()),
                        output_type: Some(".test.v1.RedirectResponse".to_string()),
                        options: Some(MethodOptions {
                            http: Some(HttpRule {
                                pattern: Some(HttpPattern::Get("/v1/redirect".to_string())),
                                body: String::new(),
                            }),
                        }),
                        client_streaming: None,
                        server_streaming: None,
                    }],
                }],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.redirect_paths, vec!["/v1/redirect"]);
    }

    #[test]
    fn nested_message_constraints_use_qualified_path() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Outer".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("name".to_string()),
                        r#type: Some(field_type::STRING),
                        type_name: None,
                        options: Some(FieldOptions {
                            rules: Some(FieldRules {
                                string: Some(StringRules {
                                    min_len: Some(1),
                                    max_len: Some(100),
                                    pattern: None,
                                    r#in: vec![],
                                    uuid: None,
                                }),
                                ..Default::default()
                            }),
                        }),
                    }],
                    nested_type: vec![DescriptorProto {
                        name: Some("Inner".to_string()),
                        field: vec![FieldDescriptorProto {
                            name: Some("value".to_string()),
                            r#type: Some(field_type::STRING),
                            type_name: None,
                            options: Some(FieldOptions {
                                rules: Some(FieldRules {
                                    string: Some(StringRules {
                                        min_len: Some(3),
                                        max_len: None,
                                        pattern: None,
                                        r#in: vec![],
                                        uuid: None,
                                    }),
                                    ..Default::default()
                                }),
                            }),
                        }],
                        nested_type: vec![],
                    }],
                }],
                enum_type: vec![],
                service: vec![],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        // Outer message should be "test.v1.Outer"
        let outer = metadata
            .field_constraints
            .iter()
            .find(|c| c.schema == "test.v1.Outer");
        assert!(outer.is_some(), "Outer constraint should exist");

        // Nested message should be "test.v1.Outer.Inner", NOT "test.v1.Inner"
        let inner = metadata
            .field_constraints
            .iter()
            .find(|c| c.schema == "test.v1.Outer.Inner");
        assert!(
            inner.is_some(),
            "Nested Inner constraint should use fully qualified path: {:?}",
            metadata
                .field_constraints
                .iter()
                .map(|c| &c.schema)
                .collect::<Vec<_>>()
        );

        // Ensure the old wrong path is NOT present
        let wrong = metadata
            .field_constraints
            .iter()
            .find(|c| c.schema == "test.v1.Inner");
        assert!(
            wrong.is_none(),
            "Should not have bare 'test.v1.Inner' schema"
        );
    }

    #[test]
    fn nested_message_fields_use_qualified_fqn() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Outer".to_string()),
                    field: vec![make_field("name", field_type::STRING)],
                    nested_type: vec![DescriptorProto {
                        name: Some("Inner".to_string()),
                        field: vec![make_field("value", field_type::STRING)],
                        nested_type: vec![],
                    }],
                }],
                enum_type: vec![],
                service: vec![ServiceDescriptorProto {
                    name: Some("Svc".to_string()),
                    method: vec![MethodDescriptorProto {
                        name: Some("Do".to_string()),
                        input_type: Some(".test.v1.Outer.Inner".to_string()),
                        output_type: Some(".test.v1.Outer".to_string()),
                        options: Some(MethodOptions {
                            http: Some(HttpRule {
                                pattern: Some(HttpPattern::Get("/v1/outer/{value}".to_string())),
                                body: String::new(),
                            }),
                        }),
                        client_streaming: None,
                        server_streaming: None,
                    }],
                }],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        // Path param lookup should resolve .test.v1.Outer.Inner (not .test.v1.Inner)
        assert!(
            !metadata.path_param_constraints.is_empty(),
            "should find path params via nested message FQN"
        );
    }

    #[test]
    fn nested_enum_rewrites_use_qualified_schema() {
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Outer".to_string()),
                    field: vec![],
                    nested_type: vec![DescriptorProto {
                        name: Some("Inner".to_string()),
                        field: vec![FieldDescriptorProto {
                            name: Some("status".to_string()),
                            r#type: Some(field_type::ENUM),
                            type_name: Some(".test.v1.Status".to_string()),
                            options: None,
                        }],
                        nested_type: vec![],
                    }],
                }],
                enum_type: vec![EnumDescriptorProto {
                    name: Some("Status".to_string()),
                    value: vec![
                        EnumValueDescriptorProto {
                            name: Some("STATUS_UNSPECIFIED".to_string()),
                            number: Some(0),
                        },
                        EnumValueDescriptorProto {
                            name: Some("STATUS_ACTIVE".to_string()),
                            number: Some(1),
                        },
                    ],
                }],
                service: vec![],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.enum_rewrites.len(), 1);
        // Should be qualified as "test.v1.Outer.Inner", not "test.v1.Inner"
        assert_eq!(
            metadata.enum_rewrites[0].schema, "test.v1.Outer.Inner",
            "nested enum rewrite should use fully qualified schema path"
        );
    }

    #[test]
    fn int32_boundary_values_no_overflow() {
        // Test gt = i32::MAX should not overflow
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("count".to_string()),
                        r#type: Some(field_type::INT32),
                        type_name: None,
                        options: Some(FieldOptions {
                            rules: Some(FieldRules {
                                int32: Some(Int32Rules {
                                    gte: Some(-100),
                                    lte: Some(100),
                                    gt: None,
                                    lt: None,
                                }),
                                ..Default::default()
                            }),
                        }),
                    }],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.field_constraints.len(), 1);
        let fc = &metadata.field_constraints[0].fields[0];
        assert_eq!(fc.signed_min, Some(-100));
        assert_eq!(fc.signed_max, Some(100));
        assert!(fc.is_numeric);
    }

    #[test]
    fn uint32_lt_zero_no_underflow() {
        // Test lt = 0 should not underflow (saturates to 0)
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("count".to_string()),
                        r#type: Some(field_type::UINT32),
                        type_name: None,
                        options: Some(FieldOptions {
                            rules: Some(FieldRules {
                                uint32: Some(UInt32Rules {
                                    lt: Some(0),
                                    lte: None,
                                    gt: None,
                                    gte: None,
                                }),
                                ..Default::default()
                            }),
                        }),
                    }],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![],
            }],
        };
        let bytes = fdset.encode_to_vec();
        // Should not panic from underflow
        let metadata = discover(&bytes).unwrap();
        assert_eq!(metadata.field_constraints.len(), 1);
        let fc = &metadata.field_constraints[0].fields[0];
        assert_eq!(fc.max, Some(0)); // saturated
    }

    #[test]
    fn uint64_exclusive_bounds_converted_to_inclusive() {
        // Proto: uint64 content_size = 3 [(validate.rules).uint64 = {gt: 0, lte: 10485760}];
        // gt: 0 → minimum: 1 (exclusive → inclusive), lte: 10485760 → maximum: 10485760
        let fdset = FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("test.proto".to_string()),
                package: Some("test.v1".to_string()),
                message_type: vec![DescriptorProto {
                    name: Some("Request".to_string()),
                    field: vec![FieldDescriptorProto {
                        name: Some("content_size".to_string()),
                        r#type: Some(field_type::UINT64),
                        type_name: None,
                        options: Some(FieldOptions {
                            rules: Some(FieldRules {
                                uint64: Some(UInt64Rules {
                                    gt: Some(0),
                                    gte: None,
                                    lt: None,
                                    lte: Some(10_485_760),
                                }),
                                ..Default::default()
                            }),
                        }),
                    }],
                    nested_type: vec![],
                }],
                enum_type: vec![],
                service: vec![],
            }],
        };
        let bytes = fdset.encode_to_vec();
        let metadata = discover(&bytes).unwrap();

        assert_eq!(metadata.field_constraints.len(), 1);
        let fc = &metadata.field_constraints[0].fields[0];
        assert_eq!(fc.field, "contentSize");
        assert_eq!(fc.min, Some(1), "gt:0 should become minimum:1");
        assert_eq!(fc.max, Some(10_485_760));
        assert!(fc.is_numeric);
    }
}
