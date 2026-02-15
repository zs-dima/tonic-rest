//! `OpenAPI` spec patching pipeline.
//!
//! Applies a configurable sequence of transforms to a gnostic-generated
//! `OpenAPI` YAML spec, producing a clean `OpenAPI` 3.1 spec that matches
//! the runtime REST behavior.
//!
//! Transforms are grouped into logical modules:
//! - [`oas31`] — `OpenAPI` 3.0 → 3.1 structural changes
//! - [`streaming`] — SSE streaming annotations
//! - [`responses`] — Response status codes, redirects, plain text, error schemas
//! - [`security`] — Bearer auth schemes and per-operation overrides
//! - [`validation`] — Proto validation constraints → JSON Schema
//! - [`cleanup`] — Tag cleanup, orphan removal, formatting normalization

mod cleanup;
mod helpers;
mod oas31;
mod responses;
mod security;
mod streaming;
mod validation;

use serde_yaml_ng::Value;

use crate::config::PlainTextEndpoint;
use crate::config::{InfoOverrides, ServerEntry};
use crate::discover::ProtoMetadata;
use crate::error;

/// Configuration for the `OpenAPI` patch pipeline.
///
/// Controls which transforms run and their parameters. Construct with
/// [`PatchConfig::new`] and configure via [`with_project_config`](Self::with_project_config)
/// (file-based) or individual builder methods (programmatic).
///
/// # Example
///
/// ```ignore
/// let config = PatchConfig::new(&metadata)
///     .unimplemented_methods(&["SetupMfa", "DisableMfa"])
///     .public_methods(&["Login", "SignUp"])
///     .error_schema_ref("#/components/schemas/ErrorResponse");
/// ```
#[derive(Debug)]
pub struct PatchConfig<'a> {
    /// Proto metadata extracted via [`crate::discover()`].
    metadata: &'a ProtoMetadata,

    /// Raw proto method names — resolved to operation IDs at [`patch()`] time.
    unimplemented_method_names: Vec<String>,

    /// Raw proto method names — resolved to operation IDs at [`patch()`] time.
    public_method_names: Vec<String>,

    /// Raw proto method names — resolved to operation IDs at [`patch()`] time.
    deprecated_method_names: Vec<String>,

    /// `$ref` path for the REST error response schema.
    error_schema_ref: String,

    /// Endpoints that should use `text/plain` instead of `application/json`.
    plain_text_endpoints: Vec<PlainTextEndpoint>,

    /// Metrics endpoint path for response header enrichment (e.g., `/metrics`).
    metrics_path: Option<String>,

    /// Readiness probe path for adding 503 response (e.g., `/health/ready`).
    readiness_path: Option<String>,

    /// Transform toggles (all default to `true`).
    transforms: crate::config::TransformConfig,

    /// Custom description for the Bearer auth scheme in `OpenAPI`.
    ///
    /// Defaults to `"Bearer authentication token"` when `None`.
    bearer_description: Option<String>,

    /// Server entries for the `servers` block.
    servers: Vec<ServerEntry>,

    /// `OpenAPI` `info` block overrides.
    info: InfoOverrides,

    /// Additional field name patterns to mark as `writeOnly`.
    write_only_fields: Vec<String>,

    /// Additional field name patterns to mark as `readOnly`.
    read_only_fields: Vec<String>,
}

impl<'a> PatchConfig<'a> {
    /// Create a new config with all transforms enabled and default settings.
    #[must_use]
    pub fn new(metadata: &'a ProtoMetadata) -> Self {
        Self {
            metadata,
            unimplemented_method_names: Vec::new(),
            public_method_names: Vec::new(),
            deprecated_method_names: Vec::new(),
            error_schema_ref: crate::DEFAULT_ERROR_SCHEMA_REF.to_string(),
            plain_text_endpoints: Vec::new(),
            metrics_path: None,
            readiness_path: None,
            transforms: crate::config::TransformConfig::default(),
            bearer_description: None,
            servers: Vec::new(),
            info: InfoOverrides::default(),
            write_only_fields: Vec::new(),
            read_only_fields: Vec::new(),
        }
    }

    /// Apply settings from a [`ProjectConfig`](crate::ProjectConfig).
    ///
    /// Copies method lists, error schema ref, transform toggles, and endpoint
    /// settings from the config into this builder. Builder methods called after
    /// this will override config values.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let project = ProjectConfig::load(Path::new("config.yaml"))?;
    /// let config = PatchConfig::new(&metadata).with_project_config(&project);
    /// ```
    #[must_use]
    pub fn with_project_config(mut self, project: &crate::ProjectConfig) -> Self {
        self.error_schema_ref.clone_from(&project.error_schema_ref);
        self.plain_text_endpoints
            .clone_from(&project.plain_text_endpoints);
        self.metrics_path.clone_from(&project.metrics_path);
        self.readiness_path.clone_from(&project.readiness_path);
        self.servers.clone_from(&project.servers);
        self.info = project.info.clone();
        self.write_only_fields
            .clone_from(&project.write_only_fields);
        self.read_only_fields.clone_from(&project.read_only_fields);
        self.transforms = project.transforms;

        if !project.unimplemented_methods.is_empty() {
            self.unimplemented_method_names
                .clone_from(&project.unimplemented_methods);
        }
        if !project.public_methods.is_empty() {
            self.public_method_names.clone_from(&project.public_methods);
        }
        if !project.deprecated_methods.is_empty() {
            self.deprecated_method_names
                .clone_from(&project.deprecated_methods);
        }

        self
    }

    /// Set proto method names of endpoints that return `UNIMPLEMENTED`.
    ///
    /// Method names are resolved to gnostic operation IDs at [`patch()`] time.
    /// Invalid names will produce an error when `patch()` is called.
    #[must_use]
    pub fn unimplemented_methods(mut self, methods: &[&str]) -> Self {
        self.unimplemented_method_names = methods.iter().map(ToString::to_string).collect();
        self
    }

    /// Set proto method names of endpoints that do not require authentication.
    ///
    /// Method names are resolved to gnostic operation IDs at [`patch()`] time.
    /// Invalid names will produce an error when `patch()` is called.
    #[must_use]
    pub fn public_methods(mut self, methods: &[&str]) -> Self {
        self.public_method_names = methods.iter().map(ToString::to_string).collect();
        self
    }

    /// Set proto method names of deprecated endpoints.
    ///
    /// Method names are resolved to gnostic operation IDs at [`patch()`] time.
    /// These operations will receive `deprecated: true` in the output spec.
    #[must_use]
    pub fn deprecated_methods(mut self, methods: &[&str]) -> Self {
        self.deprecated_method_names = methods.iter().map(ToString::to_string).collect();
        self
    }

    /// Set the `$ref` path for the REST error response schema.
    #[must_use]
    pub fn error_schema_ref(mut self, ref_path: &str) -> Self {
        self.error_schema_ref = ref_path.to_string();
        self
    }

    /// Enable or disable the 3.0 → 3.1 upgrade transform.
    #[must_use]
    pub const fn upgrade_to_3_1(mut self, enabled: bool) -> Self {
        self.transforms.upgrade_to_3_1 = enabled;
        self
    }

    /// Enable or disable SSE streaming annotation.
    #[must_use]
    pub const fn annotate_sse(mut self, enabled: bool) -> Self {
        self.transforms.annotate_sse = enabled;
        self
    }

    /// Enable or disable validation constraint injection.
    #[must_use]
    pub const fn inject_validation(mut self, enabled: bool) -> Self {
        self.transforms.inject_validation = enabled;
        self
    }

    /// Enable or disable security scheme addition.
    #[must_use]
    pub const fn add_security(mut self, enabled: bool) -> Self {
        self.transforms.add_security = enabled;
        self
    }

    /// Enable or disable request body inlining.
    #[must_use]
    pub const fn inline_request_bodies(mut self, enabled: bool) -> Self {
        self.transforms.inline_request_bodies = enabled;
        self
    }

    /// Enable or disable UUID wrapper flattening.
    #[must_use]
    pub const fn flatten_uuid_refs(mut self, enabled: bool) -> Self {
        self.transforms.flatten_uuid_refs = enabled;
        self
    }

    /// Enable or disable CRLF → LF normalization.
    #[must_use]
    pub const fn normalize_line_endings(mut self, enabled: bool) -> Self {
        self.transforms.normalize_line_endings = enabled;
        self
    }

    /// Enable or disable server/info injection.
    #[must_use]
    pub const fn inject_servers(mut self, enabled: bool) -> Self {
        self.transforms.inject_servers = enabled;
        self
    }

    /// Enable or disable `200` → `201 Created` rewrite.
    #[must_use]
    pub const fn rewrite_create_responses(mut self, enabled: bool) -> Self {
        self.transforms.rewrite_create_responses = enabled;
        self
    }

    /// Enable or disable `writeOnly`/`readOnly` field annotation.
    #[must_use]
    pub const fn annotate_field_access(mut self, enabled: bool) -> Self {
        self.transforms.annotate_field_access = enabled;
        self
    }

    /// Skip the 3.0 → 3.1 upgrade transform.
    #[must_use]
    pub const fn skip_upgrade(self) -> Self {
        self.upgrade_to_3_1(false)
    }

    /// Skip SSE streaming annotation.
    #[must_use]
    pub const fn skip_sse(self) -> Self {
        self.annotate_sse(false)
    }

    /// Skip validation constraint injection.
    #[must_use]
    pub const fn skip_validation(self) -> Self {
        self.inject_validation(false)
    }

    /// Skip security scheme addition.
    #[must_use]
    pub const fn skip_security(self) -> Self {
        self.add_security(false)
    }

    /// Skip request body inlining.
    #[must_use]
    pub const fn skip_inline_request_bodies(self) -> Self {
        self.inline_request_bodies(false)
    }

    /// Skip UUID wrapper flattening.
    #[must_use]
    pub const fn skip_uuid_flattening(self) -> Self {
        self.flatten_uuid_refs(false)
    }

    /// Skip CRLF → LF normalization.
    #[must_use]
    pub const fn skip_line_ending_normalization(self) -> Self {
        self.normalize_line_endings(false)
    }

    /// Skip server/info injection.
    #[must_use]
    pub const fn skip_servers(self) -> Self {
        self.inject_servers(false)
    }

    /// Skip `200` → `201 Created` rewrite.
    #[must_use]
    pub const fn skip_create_response_rewrite(self) -> Self {
        self.rewrite_create_responses(false)
    }

    /// Skip `writeOnly`/`readOnly` field annotation.
    #[must_use]
    pub const fn skip_field_access_annotation(self) -> Self {
        self.annotate_field_access(false)
    }

    /// Set a custom description for the Bearer auth scheme.
    ///
    /// When `None`, defaults to `"Bearer authentication token"`.
    #[must_use]
    pub fn bearer_description(mut self, description: &str) -> Self {
        self.bearer_description = Some(description.to_string());
        self
    }

    /// Set server entries for the `servers` block.
    #[must_use]
    pub fn servers(mut self, servers: &[ServerEntry]) -> Self {
        self.servers = servers.to_vec();
        self
    }

    /// Set `OpenAPI` `info` block overrides.
    #[must_use]
    pub fn info(mut self, info: InfoOverrides) -> Self {
        self.info = info;
        self
    }

    /// Set additional field name patterns to mark as `writeOnly`.
    #[must_use]
    pub fn write_only_fields(mut self, fields: &[&str]) -> Self {
        self.write_only_fields = fields.iter().map(ToString::to_string).collect();
        self
    }

    /// Set additional field name patterns to mark as `readOnly`.
    #[must_use]
    pub fn read_only_fields(mut self, fields: &[&str]) -> Self {
        self.read_only_fields = fields.iter().map(ToString::to_string).collect();
        self
    }

    /// Set endpoints that should use `text/plain` content type.
    #[must_use]
    pub fn plain_text_endpoints(mut self, endpoints: &[PlainTextEndpoint]) -> Self {
        self.plain_text_endpoints = endpoints.to_vec();
        self
    }

    /// Set the metrics endpoint path for response header enrichment.
    #[must_use]
    pub fn metrics_path(mut self, path: &str) -> Self {
        self.metrics_path = Some(path.to_string());
        self
    }

    /// Set the readiness probe path for 503 response addition.
    #[must_use]
    pub fn readiness_path(mut self, path: &str) -> Self {
        self.readiness_path = Some(path.to_string());
        self
    }

    /// Resolve deferred method names to operation IDs.
    fn resolved_ops(&self) -> error::Result<(Vec<String>, Vec<String>, Vec<String>)> {
        let unimplemented = self.resolve_method_list(&self.unimplemented_method_names)?;
        let public = self.resolve_method_list(&self.public_method_names)?;
        let deprecated = self.resolve_method_list(&self.deprecated_method_names)?;
        Ok((unimplemented, public, deprecated))
    }

    /// Resolve a list of method names to gnostic operation IDs.
    fn resolve_method_list(&self, names: &[String]) -> error::Result<Vec<String>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        crate::discover::resolve_operation_ids(self.metadata, &refs)
    }
}

/// Apply the configured transform pipeline to an `OpenAPI` YAML spec.
///
/// Parses the input YAML, applies all enabled transforms in the correct order,
/// and returns the patched YAML string.
///
/// # Phase Ordering
///
/// The pipeline has ordering dependencies:
/// - **Phase 1** (structural): 3.0 → 3.1 upgrade, server/info injection.
/// - **Phase 2** (streaming): SSE annotations, `Last-Event-ID` header.
/// - **Phase 3** (responses): status codes, plain text, redirects, error
///   schemas, `201 Created` rewrite.
/// - **Phase 4** (enum rewrites): must run before inlining (phase 11) so that
///   inlined schemas contain the rewritten enum values.
/// - **Phase 5** (markers): unimplemented (`501`) and deprecated flags; must
///   run after response fixes (phase 3).
/// - **Phase 6** (security): bearer auth schemes; independent of validation.
/// - **Phase 7** (cleanup): removes empty bodies before constraint injection.
/// - **Phase 8** (UUID flattening): path template `.value` stripping, `$ref`
///   flattening, query param simplification; must run before validation.
/// - **Phase 9** (validation): constraint injection, `writeOnly`/`readOnly`
///   annotation, `Duration` field rewriting.
/// - **Phase 10** (path field stripping): must run after constraint injection
///   (phase 9) since it clones schemas before removing path fields.
/// - **Phase 11** (inlining): must run after path stripping (phase 10) to
///   correctly detect emptied bodies; runs last among content transforms.
/// - **Phase 12** (normalization): always runs last as a final cleanup pass.
///
/// # Errors
///
/// Returns an error if the input YAML cannot be parsed, processing fails,
/// or any deferred method name (from [`PatchConfig::unimplemented_methods`]
/// or [`PatchConfig::public_methods`]) cannot be resolved against proto metadata.
pub fn patch(input_yaml: &str, config: &PatchConfig<'_>) -> error::Result<String> {
    let mut doc: Value = serde_yaml_ng::from_str(input_yaml)?;

    // Resolve deferred method names to operation IDs
    let (unimplemented_ops, public_ops, deprecated_ops) = config.resolved_ops()?;

    // Phase 1: Structural transforms (3.0 → 3.1)
    if config.transforms.upgrade_to_3_1 {
        oas31::upgrade_version(&mut doc);
        oas31::convert_nullable(&mut doc);
    }
    if config.transforms.inject_servers {
        oas31::inject_servers_and_info(&mut doc, &config.servers, &config.info);
    }

    // Phase 2: Streaming annotations
    if config.transforms.annotate_sse {
        streaming::annotate_sse(&mut doc, &config.metadata.streaming_ops);
    }

    // Phase 3: Response fixes
    responses::patch_empty_responses(&mut doc);
    responses::remove_redundant_query_params(&mut doc);
    responses::patch_plain_text_endpoints(&mut doc, &config.plain_text_endpoints);
    responses::patch_metrics_response_headers(&mut doc, config.metrics_path.as_deref());
    responses::patch_readiness_probe_responses(&mut doc, config.readiness_path.as_deref());
    responses::patch_redirect_endpoints(&mut doc, &config.metadata.redirect_paths);
    responses::ensure_rest_error_schema(&mut doc, &config.error_schema_ref);
    responses::rewrite_default_error_responses(&mut doc, &config.error_schema_ref);
    if config.transforms.rewrite_create_responses {
        responses::rewrite_create_responses(&mut doc);
    }

    // Phase 4: Enum value rewrites
    // Rewrite first (prefix-stripping), then strip unspecified sentinels.
    // Order matters: rewrite_enum_values replaces enum arrays wholesale on
    // component schemas (including the lowercased "unspecified" value), so
    // stripping must run after to remove them from all locations.
    cleanup::rewrite_enum_values(&mut doc, config.metadata);
    cleanup::strip_unspecified_from_query_enums(&mut doc);

    // Phase 5: Unimplemented operation markers
    if !unimplemented_ops.is_empty() {
        cleanup::mark_unimplemented_operations(
            &mut doc,
            &unimplemented_ops,
            &config.error_schema_ref,
        );
    }

    if !deprecated_ops.is_empty() {
        cleanup::mark_deprecated_operations(&mut doc, &deprecated_ops);
    }

    // Phase 6: Security
    if config.transforms.add_security {
        security::add_security_schemes(&mut doc, &public_ops, config.bearer_description.as_deref());
    }

    // Phase 7: Cleanup (tags, summaries, empty bodies, format noise)
    cleanup::clean_tag_descriptions(&mut doc);
    cleanup::populate_operation_summaries(&mut doc);
    cleanup::remove_empty_request_bodies(&mut doc);
    cleanup::remove_unused_empty_schemas(&mut doc);
    cleanup::remove_format_enum(&mut doc);

    // Phase 8: UUID flattening
    validation::flatten_uuid_path_templates(&mut doc);
    if config.transforms.flatten_uuid_refs {
        validation::flatten_uuid_refs(&mut doc, config.metadata.uuid_schema.as_deref());
    }
    validation::simplify_uuid_query_params(&mut doc);

    // Phase 9: Validation constraint injection
    if config.transforms.inject_validation {
        validation::inject_validation_constraints(&mut doc, &config.metadata.field_constraints);
    }
    if config.transforms.annotate_field_access {
        validation::annotate_field_access(
            &mut doc,
            &config.write_only_fields,
            &config.read_only_fields,
        );
    }
    validation::annotate_duration_fields(&mut doc);

    // Phase 10: Path field stripping (must run after constraint injection)
    validation::strip_path_fields_from_body(&mut doc);
    validation::enrich_path_params(&mut doc, &config.metadata.path_param_constraints);

    // Phase 11: Request body handling
    //
    // When inlining is enabled, request body schemas are inlined into
    // operations with per-property examples and the originals are removed
    // as orphans. When disabled, component schemas are enriched with
    // per-property examples in-place so they remain visible in the
    // Schemas section of Swagger UI.
    //
    // Empty body removal and orphan cleanup always run regardless of the
    // inlining mode — path-field stripping (phase 10) can leave empty
    // bodies, and self-referential schema clusters (e.g., google.rpc.Status)
    // should always be pruned.
    if config.transforms.inline_request_bodies {
        cleanup::inline_request_bodies(&mut doc);
    } else {
        cleanup::enrich_schema_examples(&mut doc);
    }
    cleanup::enrich_inline_request_body_examples(&mut doc);
    cleanup::remove_empty_inlined_request_bodies(&mut doc);
    cleanup::remove_orphaned_schemas(&mut doc);

    // Phase 12: Final normalization
    if config.transforms.normalize_line_endings {
        oas31::normalize_line_endings(&mut doc);
    }

    serde_yaml_ng::to_string(&doc).map_err(error::Error::from)
}
