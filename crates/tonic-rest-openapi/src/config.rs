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
/// [`PatchConfig`](crate::PatchConfig) via [`ProjectConfig::apply`].
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    /// `$ref` path for the REST error response schema.
    pub error_schema_ref: String,

    /// Proto method short names for endpoints returning `UNIMPLEMENTED`.
    pub unimplemented_methods: Vec<String>,

    /// Proto method short names for public (no-auth) endpoints.
    pub public_methods: Vec<String>,

    /// Endpoints that should use `text/plain` instead of `application/json`.
    pub plain_text_endpoints: Vec<PlainTextEndpoint>,

    /// Metrics endpoint path for response header enrichment (e.g., `/metrics`).
    pub metrics_path: Option<String>,

    /// Readiness probe path for adding 503 response (e.g., `/health/ready`).
    pub readiness_path: Option<String>,

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

/// Individual transform on/off switches (all default to `true`).
#[derive(Debug, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransformConfig {
    /// Upgrade `OpenAPI` 3.0 → 3.1.
    pub upgrade_to_3_1: bool,

    /// Annotate SSE streaming operations.
    pub annotate_sse: bool,

    /// Inject proto validation constraints into JSON Schema.
    pub inject_validation: bool,

    /// Add bearer auth security schemes.
    pub add_security: bool,

    /// Inline request body schemas for better Swagger UI rendering.
    pub inline_request_bodies: bool,

    /// Flatten UUID wrapper `$ref` to inline `type: string, format: uuid`.
    pub flatten_uuid_refs: bool,

    /// Normalize CRLF → LF in string values.
    pub normalize_line_endings: bool,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            error_schema_ref: crate::DEFAULT_ERROR_SCHEMA_REF.to_string(),
            unimplemented_methods: Vec::new(),
            public_methods: Vec::new(),
            plain_text_endpoints: Vec::new(),
            metrics_path: None,
            readiness_path: None,
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
        assert!(config.plain_text_endpoints.is_empty());
        assert!(config.metrics_path.is_none());
        assert!(config.readiness_path.is_none());
        assert!(config.transforms.upgrade_to_3_1);
        assert!(config.transforms.annotate_sse);
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
plain_text_endpoints:
  - path: /health/live
    example: "OK"
  - path: /metrics
metrics_path: /metrics
readiness_path: /health/ready
transforms:
  add_security: false
"##;
        let config: ProjectConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.error_schema_ref, "#/components/schemas/MyError");
        assert_eq!(config.unimplemented_methods, vec!["SetupMfa", "DisableMfa"]);
        assert_eq!(config.public_methods, vec!["Authenticate"]);
        assert_eq!(config.plain_text_endpoints.len(), 2);
        assert_eq!(config.plain_text_endpoints[0].path, "/health/live");
        assert_eq!(
            config.plain_text_endpoints[0].example.as_deref(),
            Some("OK")
        );
        assert!(config.plain_text_endpoints[1].example.is_none());
        assert_eq!(config.metrics_path.as_deref(), Some("/metrics"));
        assert_eq!(config.readiness_path.as_deref(), Some("/health/ready"));
        assert!(!config.transforms.add_security);
        // Other transforms keep defaults
        assert!(config.transforms.upgrade_to_3_1);
        assert!(config.transforms.inline_request_bodies);
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
