//! Project-level `OpenAPI` configuration loaded from YAML.
//!
//! Externalizes project-specific knobs (method lists, error schema, transform
//! toggles, endpoint paths) so they live next to the proto/OpenAPI files
//! instead of being hardcoded in Rust source.
//!
//! # File format
//!
//! ```yaml
//! # api/openapi/config.yaml
//! error_schema_ref: "#/components/schemas/ErrorResponse"
//!
//! # Proto method names that return UNIMPLEMENTED at runtime.
//! unimplemented_methods:
//!   - SetupMfa
//!   - DisableMfa
//!
//! # Proto method names that require no authentication.
//! public_methods:
//!   - Login
//!   - SignUp
//!
//! # Endpoints that should use text/plain instead of application/json.
//! plain_text_endpoints:
//!   - path: /health/live
//!     example: "OK"
//!   - path: /metrics
//!
//! # Metrics endpoint for response header enrichment.
//! metrics_path: /metrics
//!
//! # Readiness probe path for 503 response addition.
//! readiness_path: /health/ready
//!
//! # Transform toggles (all default to true).
//! transforms:
//!   upgrade_to_3_1: true
//!   annotate_sse: true
//! ```

use std::path::Path;

use serde::Deserialize;

/// Project-level `OpenAPI` generation config.
///
/// Loaded from a YAML file via [`ProjectConfig::load`], then applied to a
/// [`PatchConfig`](crate::PatchConfig) via [`PatchConfig::with_project_config`](crate::PatchConfig::with_project_config).
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// `$ref` path for the REST error response schema.
    pub error_schema_ref: String,

    /// Proto method short names for endpoints returning `UNIMPLEMENTED`.
    pub unimplemented_methods: Vec<String>,

    /// Proto method short names for public (no-auth) endpoints.
    pub public_methods: Vec<String>,

    /// Proto method short names for deprecated endpoints.
    pub deprecated_methods: Vec<String>,

    /// Endpoints that should use `text/plain` instead of `application/json`.
    pub plain_text_endpoints: Vec<PlainTextEndpoint>,

    /// Metrics endpoint path for response header enrichment (e.g., `/metrics`).
    pub metrics_path: Option<String>,

    /// Readiness probe path for adding 503 response (e.g., `/health/ready`).
    pub readiness_path: Option<String>,

    /// Server entries for the `servers` block.
    pub servers: Vec<ServerEntry>,

    /// `OpenAPI` `info` block overrides (contact, license, external docs).
    pub info: InfoOverrides,

    /// Additional field name patterns to mark as `writeOnly`.
    pub write_only_fields: Vec<String>,

    /// Additional field name patterns to mark as `readOnly`.
    pub read_only_fields: Vec<String>,

    /// Transform toggles.
    pub transforms: TransformConfig,
}

/// An endpoint that returns plain text instead of JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct PlainTextEndpoint {
    /// HTTP path (e.g., `/health/live`).
    pub path: String,
    /// Optional example response body (e.g., `"OK"`).
    pub example: Option<String>,
}

/// A server entry for the `OpenAPI` `servers` block.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerEntry {
    /// Server URL (e.g., `http://localhost:8080`).
    pub url: String,
    /// Optional human-readable server description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Overrides for the `OpenAPI` `info` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct InfoOverrides {
    /// API contact information.
    pub contact: Option<ContactInfo>,
    /// API license information.
    pub license: Option<LicenseInfo>,
    /// Link to external documentation.
    pub external_docs: Option<ExternalDocsInfo>,
    /// URL to the Terms of Service.
    pub terms_of_service: Option<String>,
}

/// Contact information for the `OpenAPI` `info.contact` block.
#[derive(Debug, Clone, Deserialize)]
pub struct ContactInfo {
    /// Contact name.
    pub name: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact URL.
    pub url: Option<String>,
}

/// License information for the `OpenAPI` `info.license` block.
#[derive(Debug, Clone, Deserialize)]
pub struct LicenseInfo {
    /// License name (e.g., `"MIT"`).
    pub name: String,
    /// URL to the full license text.
    pub url: Option<String>,
}

/// External documentation link for `externalDocs`.
#[derive(Debug, Clone, Deserialize)]
pub struct ExternalDocsInfo {
    /// URL to the external documentation.
    pub url: String,
    /// Short description of the external docs.
    pub description: Option<String>,
}

/// Individual transform on/off switches (all default to `true`).
///
/// Controls which phases of the 12-phase pipeline run. Each toggle maps to
/// one or more pipeline phases. See [`patch()`](crate::patch) for phase ordering.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransformConfig {
    /// Upgrade `OpenAPI` 3.0 → 3.1 (phase 1).
    ///
    /// Converts `openapi: "3.0.x"` to `"3.1.0"`, rewrites `nullable: true` to
    /// `type: ["string", "null"]`, and applies other 3.1 structural changes.
    pub upgrade_to_3_1: bool,

    /// Annotate SSE streaming operations (phase 2).
    ///
    /// Adds `text/event-stream` response content type, `Last-Event-ID` header,
    /// and streaming-specific descriptions to server-streaming RPCs.
    pub annotate_sse: bool,

    /// Inject proto validation constraints into JSON Schema (phase 9).
    ///
    /// Maps `validate.rules` from proto field options to `minLength`, `maxLength`,
    /// `pattern`, `minimum`, `maximum`, `required`, and `enum` constraints.
    pub inject_validation: bool,

    /// Add bearer auth security schemes (phase 6).
    ///
    /// Injects a `bearerAuth` security scheme and applies it globally,
    /// with overrides for public endpoints.
    pub add_security: bool,

    /// Inline request body schemas for better Swagger UI rendering (phase 11).
    ///
    /// Replaces `$ref` request bodies with inline schemas containing property
    /// examples, improving the "Try it out" experience in Swagger UI.
    pub inline_request_bodies: bool,

    /// Flatten UUID wrapper `$ref` to inline `type: string, format: uuid` (phase 8).
    ///
    /// Simplifies single-field UUID wrapper messages by inlining the string type
    /// with `format: uuid` and `pattern` validation.
    pub flatten_uuid_refs: bool,

    /// Normalize CRLF → LF in string values (phase 12).
    ///
    /// Ensures consistent line endings in the output spec, preventing
    /// platform-dependent diffs.
    pub normalize_line_endings: bool,

    /// Inject `servers` and `info` overrides into the spec (phase 1).
    ///
    /// Merges configured server URLs and info block overrides (contact,
    /// license, terms of service) into the spec.
    pub inject_servers: bool,

    /// Rewrite `200` → `201 Created` for create/signup endpoints (phase 3).
    ///
    /// Detects operations named `Create*`, `SignUp*`, or `Register*` and
    /// changes their success response from 200 to 201.
    pub rewrite_create_responses: bool,

    /// Annotate fields with `writeOnly`/`readOnly` based on naming conventions (phase 9).
    ///
    /// Fields matching patterns like `password`, `secret`, `token` are marked
    /// `writeOnly`. Fields like `created_at`, `updated_at` are marked `readOnly`.
    /// Additional patterns can be configured via `write_only_fields` / `read_only_fields`.
    pub annotate_field_access: bool,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            error_schema_ref: crate::DEFAULT_ERROR_SCHEMA_REF.to_string(),
            unimplemented_methods: Vec::new(),
            public_methods: Vec::new(),
            deprecated_methods: Vec::new(),
            plain_text_endpoints: Vec::new(),
            metrics_path: None,
            readiness_path: None,
            servers: Vec::new(),
            info: InfoOverrides::default(),
            write_only_fields: Vec::new(),
            read_only_fields: Vec::new(),
            transforms: TransformConfig::default(),
        }
    }
}

impl Default for TransformConfig {
    fn default() -> Self {
        Self {
            upgrade_to_3_1: true,
            annotate_sse: true,
            inject_validation: true,
            add_security: true,
            inline_request_bodies: true,
            flatten_uuid_refs: true,
            normalize_line_endings: true,
            inject_servers: true,
            rewrite_create_responses: true,
            annotate_field_access: true,
        }
    }
}

impl ProjectConfig {
    /// Load config from a YAML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &Path) -> crate::error::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_yaml_ng::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_defaults() {
        let config: ProjectConfig = serde_yaml_ng::from_str("{}").unwrap();
        assert!(config.unimplemented_methods.is_empty());
        assert!(config.public_methods.is_empty());
        assert!(config.deprecated_methods.is_empty());
        assert!(config.plain_text_endpoints.is_empty());
        assert!(config.metrics_path.is_none());
        assert!(config.readiness_path.is_none());
        assert!(config.servers.is_empty());
        assert!(config.info.contact.is_none());
        assert!(config.info.license.is_none());
        assert!(config.write_only_fields.is_empty());
        assert!(config.read_only_fields.is_empty());
        assert!(config.transforms.upgrade_to_3_1);
        assert!(config.transforms.annotate_sse);
        assert!(config.transforms.inject_servers);
        assert!(config.transforms.rewrite_create_responses);
        assert!(config.transforms.annotate_field_access);
    }

    #[test]
    fn deserialize_full() {
        let yaml = r##"
error_schema_ref: "#/components/schemas/MyError"
unimplemented_methods:
  - SetupMfa
  - DisableMfa
public_methods:
  - Authenticate
deprecated_methods:
  - OldEndpoint
plain_text_endpoints:
  - path: /health/live
    example: "OK"
  - path: /metrics
metrics_path: /metrics
readiness_path: /health/ready
servers:
  - url: https://api.example.com
    description: Production
  - url: http://localhost:8080
    description: Local dev
info:
  contact:
    name: API Team
    email: api@example.com
  license:
    name: MIT
    url: https://opensource.org/licenses/MIT
  external_docs:
    url: https://docs.example.com
    description: Full documentation
  terms_of_service: https://example.com/tos
write_only_fields:
  - apiKey
read_only_fields:
  - lastSyncAt
transforms:
  add_security: false
  inject_servers: false
"##;
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.error_schema_ref, "#/components/schemas/MyError");
        assert_eq!(config.unimplemented_methods, vec!["SetupMfa", "DisableMfa"]);
        assert_eq!(config.public_methods, vec!["Authenticate"]);
        assert_eq!(config.deprecated_methods, vec!["OldEndpoint"]);
        assert_eq!(config.plain_text_endpoints.len(), 2);
        assert_eq!(config.plain_text_endpoints[0].path, "/health/live");
        assert_eq!(
            config.plain_text_endpoints[0].example.as_deref(),
            Some("OK")
        );
        assert!(config.plain_text_endpoints[1].example.is_none());
        assert_eq!(config.metrics_path.as_deref(), Some("/metrics"));
        assert_eq!(config.readiness_path.as_deref(), Some("/health/ready"));
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.servers[0].url, "https://api.example.com");
        assert_eq!(config.servers[0].description.as_deref(), Some("Production"));
        assert!(config.info.contact.is_some());
        assert_eq!(
            config.info.contact.as_ref().unwrap().name.as_deref(),
            Some("API Team")
        );
        assert!(config.info.license.is_some());
        assert_eq!(config.info.license.as_ref().unwrap().name, "MIT");
        assert!(config.info.external_docs.is_some());
        assert_eq!(
            config.info.terms_of_service.as_deref(),
            Some("https://example.com/tos")
        );
        assert_eq!(config.write_only_fields, vec!["apiKey"]);
        assert_eq!(config.read_only_fields, vec!["lastSyncAt"]);
        assert!(!config.transforms.add_security);
        assert!(!config.transforms.inject_servers);
        // Other transforms keep defaults
        assert!(config.transforms.upgrade_to_3_1);
        assert!(config.transforms.inline_request_bodies);
        assert!(config.transforms.rewrite_create_responses);
        assert!(config.transforms.annotate_field_access);
    }

    #[test]
    fn load_from_file() {
        let dir = std::env::temp_dir().join("tonic-rest-openapi-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-config.yaml");
        std::fs::write(
            &path,
            "public_methods:\n  - Login\nmetrics_path: /metrics\n",
        )
        .unwrap();

        let config = ProjectConfig::load(&path).unwrap();
        assert_eq!(config.public_methods, vec!["Login"]);
        assert_eq!(config.metrics_path.as_deref(), Some("/metrics"));
        // Defaults still apply
        assert!(config.transforms.upgrade_to_3_1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let result = ProjectConfig::load(Path::new("/nonexistent/config.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn load_invalid_yaml_returns_error() {
        let dir = std::env::temp_dir().join("tonic-rest-openapi-test-invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.yaml");
        std::fs::write(&path, "public_methods: [[[invalid").unwrap();

        let result = ProjectConfig::load(&path);
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
