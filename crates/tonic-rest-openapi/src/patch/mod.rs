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
    /// Proto metadata extracted via [`crate::discover`].
    metadata: &'a ProtoMetadata,

    /// Raw proto method names — resolved to operation IDs at [`patch()`] time.
    unimplemented_method_names: Vec<String>,

    /// Raw proto method names — resolved to operation IDs at [`patch()`] time.
    public_method_names: Vec<String>,

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
}

impl<'a> PatchConfig<'a> {
    /// Create a new config with all transforms enabled and default settings.
    #[must_use]
    pub fn new(metadata: &'a ProtoMetadata) -> Self {
        Self {
            metadata,
            unimplemented_method_names: Vec::new(),
            public_method_names: Vec::new(),
            error_schema_ref: crate::DEFAULT_ERROR_SCHEMA_REF.to_string(),
            plain_text_endpoints: Vec::new(),
            metrics_path: None,
            readiness_path: None,
            transforms: crate::config::TransformConfig::default(),
            bearer_description: None,
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
        self.transforms = crate::config::TransformConfig {
            upgrade_to_3_1: project.transforms.upgrade_to_3_1,
            annotate_sse: project.transforms.annotate_sse,
            inject_validation: project.transforms.inject_validation,
            add_security: project.transforms.add_security,
            inline_request_bodies: project.transforms.inline_request_bodies,
            flatten_uuid_refs: project.transforms.flatten_uuid_refs,
            normalize_line_endings: project.transforms.normalize_line_endings,
        };

        if !project.unimplemented_methods.is_empty() {
            self.unimplemented_method_names
                .clone_from(&project.unimplemented_methods);
        }
        if !project.public_methods.is_empty() {
            self.public_method_names.clone_from(&project.public_methods);
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

    /// Set the `$ref` path for the REST error response schema.
    #[must_use]
    pub fn error_schema_ref(mut self, ref_path: &str) -> Self {
        self.error_schema_ref = ref_path.to_string();
        self
    }

    /// Enable or disable the 3.0 → 3.1 upgrade transform.
    #[must_use]
    pub fn upgrade_to_3_1(mut self, enabled: bool) -> Self {
        self.transforms.upgrade_to_3_1 = enabled;
        self
    }

    /// Enable or disable SSE streaming annotation.
    #[must_use]
    pub fn annotate_sse(mut self, enabled: bool) -> Self {
        self.transforms.annotate_sse = enabled;
        self
    }

    /// Enable or disable validation constraint injection.
    #[must_use]
    pub fn inject_validation(mut self, enabled: bool) -> Self {
        self.transforms.inject_validation = enabled;
        self
    }

    /// Enable or disable security scheme addition.
    #[must_use]
    pub fn add_security(mut self, enabled: bool) -> Self {
        self.transforms.add_security = enabled;
        self
    }

    /// Enable or disable request body inlining.
    #[must_use]
    pub fn inline_request_bodies(mut self, enabled: bool) -> Self {
        self.transforms.inline_request_bodies = enabled;
        self
    }

    /// Enable or disable UUID wrapper flattening.
    #[must_use]
    pub fn flatten_uuid_refs(mut self, enabled: bool) -> Self {
        self.transforms.flatten_uuid_refs = enabled;
        self
    }

    /// Enable or disable CRLF → LF normalization.
    #[must_use]
    pub fn normalize_line_endings(mut self, enabled: bool) -> Self {
        self.transforms.normalize_line_endings = enabled;
        self
    }

    /// Skip the 3.0 → 3.1 upgrade transform.
    #[must_use]
    pub fn skip_upgrade(self) -> Self {
        self.upgrade_to_3_1(false)
    }

    /// Skip SSE streaming annotation.
    #[must_use]
    pub fn skip_sse(self) -> Self {
        self.annotate_sse(false)
    }

    /// Skip validation constraint injection.
    #[must_use]
    pub fn skip_validation(self) -> Self {
        self.inject_validation(false)
    }

    /// Skip security scheme addition.
    #[must_use]
    pub fn skip_security(self) -> Self {
        self.add_security(false)
    }

    /// Skip request body inlining.
    #[must_use]
    pub fn skip_inline_request_bodies(self) -> Self {
        self.inline_request_bodies(false)
    }

    /// Skip UUID wrapper flattening.
    #[must_use]
    pub fn skip_uuid_flattening(self) -> Self {
        self.flatten_uuid_refs(false)
    }

    /// Skip CRLF → LF normalization.
    #[must_use]
    pub fn skip_line_ending_normalization(self) -> Self {
        self.normalize_line_endings(false)
    }

    /// Set a custom description for the Bearer auth scheme.
    ///
    /// When `None`, defaults to `"Bearer authentication token"`.
    #[must_use]
    pub fn bearer_description(mut self, description: &str) -> Self {
        self.bearer_description = Some(description.to_string());
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
    fn resolved_ops(&self) -> error::Result<(Vec<String>, Vec<String>)> {
        let unimplemented = self.resolve_method_list(&self.unimplemented_method_names)?;
        let public = self.resolve_method_list(&self.public_method_names)?;
        Ok((unimplemented, public))
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
/// The 12-phase pipeline has ordering dependencies:
/// - **Phases 1–3** (structural, streaming, responses): run first to establish
///   the base spec structure; later phases depend on correct response entries.
/// - **Phase 4** (enum rewrites): must run before inlining (phase 11) so that
///   inlined schemas contain the rewritten enum values.
/// - **Phase 5** (unimplemented markers): must run after response fixes (phase 3)
///   so that `501` responses are added to specs with correct error schema refs.
/// - **Phase 6** (security): must run after operation ID resolution; independent
///   of validation.
/// - **Phase 7** (cleanup): removes empty bodies before constraint injection
///   to avoid injecting constraints into schemas about to be removed.
/// - **Phase 8** (UUID flattening): must run before validation (phase 9) so
///   that flattened UUID fields get correct format/pattern constraints.
/// - **Phase 9** (validation): injects constraints into component schemas.
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
    let (unimplemented_ops, public_ops) = config.resolved_ops()?;

    // Phase 1: Structural transforms (3.0 → 3.1)
    if config.transforms.upgrade_to_3_1 {
        oas31::upgrade_version(&mut doc);
        oas31::convert_nullable(&mut doc);
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

    // Phase 4: Enum value rewrites
    cleanup::strip_unspecified_from_query_enums(&mut doc);
    cleanup::rewrite_enum_values(&mut doc, config.metadata);

    // Phase 5: Unimplemented operation markers
    if !unimplemented_ops.is_empty() {
        cleanup::mark_unimplemented_operations(
            &mut doc,
            &unimplemented_ops,
            &config.error_schema_ref,
        );
    }

    // Phase 6: Security
    if config.transforms.add_security {
        security::add_security_schemes(&mut doc, &public_ops, config.bearer_description.as_deref());
    }

    // Phase 7: Cleanup (tags, empty bodies, format noise)
    cleanup::clean_tag_descriptions(&mut doc);
    cleanup::remove_empty_request_bodies(&mut doc);
    cleanup::remove_unused_empty_schemas(&mut doc);
    cleanup::remove_format_enum(&mut doc);

    // Phase 8: UUID flattening
    if config.transforms.flatten_uuid_refs {
        validation::flatten_uuid_refs(&mut doc, config.metadata.uuid_schema.as_deref());
    }
    validation::simplify_uuid_query_params(&mut doc);

    // Phase 9: Validation constraint injection
    if config.transforms.inject_validation {
        validation::inject_validation_constraints(&mut doc, &config.metadata.field_constraints);
    }

    // Phase 10: Path field stripping (must run after constraint injection)
    validation::strip_path_fields_from_body(&mut doc);
    validation::enrich_path_params(&mut doc, &config.metadata.path_param_constraints);

    // Phase 11: Request body inlining
    if config.transforms.inline_request_bodies {
        cleanup::inline_request_bodies(&mut doc);
        cleanup::remove_empty_inlined_request_bodies(&mut doc);
    }

    // Phase 12: Final normalization
    if config.transforms.normalize_line_endings {
        oas31::normalize_line_endings(&mut doc);
    }

    serde_yaml_ng::to_string(&doc).map_err(error::Error::from)
}
