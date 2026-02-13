//! Configuration for REST route code generation.

use std::collections::{HashMap, HashSet};

/// Error returned by [`generate`](super::generate).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GenerateError {
    /// Proto `FileDescriptorSet` decoding failure.
    #[error("failed to decode FileDescriptorSet: {0}")]
    ProtoDecode(#[from] prost::DecodeError),

    /// A nested path param (e.g., `{user_id.value}`) was found but
    /// [`RestCodegenConfig::wrapper_type`] is not configured.
    #[error(
        "nested path param '{{{param}}}' requires wrapper_type to be configured. \
         Call .wrapper_type(\"path::to::Uuid\") on RestCodegenConfig."
    )]
    MissingWrapperType {
        /// The nested path parameter that triggered the error (e.g., `user_id.value`).
        param: String,
    },

    /// Generic configuration error.
    #[error("{0}")]
    Config(String),
}

/// Configuration for REST route code generation.
///
/// Decouples the generator from any specific service — all project-specific
/// knowledge (which packages to process, which methods are public) is passed
/// in rather than hardcoded.
///
/// # Auto-Discovery
///
/// When no packages are registered, [`generate`](super::generate) automatically discovers all
/// services with `google.api.http` annotations in the descriptor set, inferring
/// Rust module paths from proto package names (dots → `::`, e.g., `auth.v1` →
/// `auth::v1`). This matches standard `prost-build` module generation.
///
/// # Examples
///
/// Minimal — auto-discovers packages from descriptor set:
///
/// ```ignore
/// let config = RestCodegenConfig::new();
/// let code = tonic_rest_build::generate(&descriptor_bytes, &config)?;
/// ```
///
/// Explicit package mapping (e.g., when using `pub use v1::*;` re-exports):
///
/// ```ignore
/// let config = RestCodegenConfig::new()
///     .package("auth.v1", "auth")
///     .package("users.v1", "users")
///     .wrapper_type("crate::core::Uuid")
///     .extension_type("my_app::AuthInfo")
///     .public_methods(&["Login", "SignUp"]);
///
/// let code = tonic_rest_build::generate(&descriptor_bytes, &config)?;
/// ```
#[derive(Clone, Debug)]
pub struct RestCodegenConfig {
    /// Proto package → Rust module mapping.
    ///
    /// When empty, packages are auto-discovered from the descriptor set:
    /// any service with `google.api.http` annotations is included, and the
    /// Rust module path is inferred from the proto package name (dots → `::`,
    /// e.g., `auth.v1` → `auth::v1`).
    ///
    /// When set explicitly, only listed packages are processed:
    /// - Key: proto package name (e.g., `"auth.v1"`)
    /// - Value: Rust module path (e.g., `"auth"` or `"auth::v1"`)
    pub(crate) packages: HashMap<String, String>,

    /// Proto method names whose REST paths should bypass authentication.
    ///
    /// These are emitted as `PUBLIC_REST_PATHS` in the generated code.
    pub(crate) public_methods: HashSet<String>,

    /// Root module for proto-generated types (default: `"crate"`).
    ///
    /// Used to convert `.auth.v1.User` → `{proto_root}::auth::User`.
    pub(crate) proto_root: String,

    /// Path to the runtime crate/module (default: `"tonic_rest"`).
    ///
    /// Generated handlers reference `{runtime_crate}::RestError`, etc.
    /// Set to `"crate::rest"` if the runtime types live in-crate.
    pub(crate) runtime_crate: String,

    /// Rust type path for single-field wrapper messages (e.g., `"crate::core::Uuid"`).
    ///
    /// When set, nested path params like `{user_id.value}` generate:
    /// `body.user_id = Some({wrapper_type} { value })`. This is commonly
    /// used for UUID wrapper types in protobuf.
    /// When `None`, nested params with `.` in the path will produce a
    /// [`GenerateError`].
    pub(crate) wrapper_type: Option<String>,

    /// SSE keep-alive interval in seconds (default: 15).
    pub(crate) sse_keep_alive_secs: u64,

    /// Concrete extension type extracted from Axum request extensions.
    ///
    /// When set, generated handlers use `Option<Extension<{extension_type}>>` to
    /// extract the value from request extensions and pass it to `build_tonic_request`.
    /// This is typically used for auth info (e.g., `"my_app::AuthInfo"`).
    /// When `None`, handlers skip extension extraction and pass `None::<()>` directly.
    pub(crate) extension_type: Option<String>,

    /// Extra HTTP headers to forward from REST requests to gRPC metadata.
    ///
    /// When set, generated handlers combine [`FORWARDED_HEADERS`] with these
    /// and call `build_tonic_request_with_headers` instead of `build_tonic_request`.
    /// Use this for vendor-specific headers (e.g., `["cf-connecting-ip"]` for Cloudflare).
    pub(crate) extra_forwarded_headers: Vec<String>,
}

impl Default for RestCodegenConfig {
    fn default() -> Self {
        Self {
            packages: HashMap::new(),
            public_methods: HashSet::new(),
            proto_root: "crate".to_string(),
            runtime_crate: "tonic_rest".to_string(),
            wrapper_type: None,
            sse_keep_alive_secs: 15,
            extension_type: None,
            extra_forwarded_headers: Vec::new(),
        }
    }
}

impl RestCodegenConfig {
    /// Create a new config with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a proto package for REST route generation.
    ///
    /// When at least one package is registered, only registered packages are
    /// processed (auto-discovery is disabled).
    ///
    /// # Example
    /// ```ignore
    /// config.package("auth.v1", "auth")
    ///       .package("users.v1", "users");
    /// ```
    #[must_use]
    pub fn package(mut self, proto_package: &str, rust_module: &str) -> Self {
        self.packages
            .insert(proto_package.to_string(), rust_module.to_string());
        self
    }

    /// Set proto method names whose REST paths bypass authentication.
    ///
    /// Method names should be in `PascalCase` as defined in proto (e.g., `"Authenticate"`).
    #[must_use]
    pub fn public_methods(mut self, methods: &[&str]) -> Self {
        self.public_methods = methods.iter().map(ToString::to_string).collect();
        self
    }

    /// Set the root module path for proto-generated types.
    ///
    /// Default: `"crate"` — converts `.auth.v1.User` → `crate::auth::User`.
    #[must_use]
    pub fn proto_root(mut self, root: &str) -> Self {
        self.proto_root = root.to_string();
        self
    }

    /// Set the runtime crate/module path for generated handler imports.
    ///
    /// Default: `"tonic_rest"` — generates `tonic_rest::RestError`, etc.
    /// Set to `"crate::rest"` if the runtime types live alongside the generated code.
    #[must_use]
    pub fn runtime_crate(mut self, path: &str) -> Self {
        self.runtime_crate = path.to_string();
        self
    }

    /// Set the Rust type path for single-field wrapper messages.
    ///
    /// Required when proto paths contain nested params like `{user_id.value}`.
    /// Commonly used for UUID wrapper types. Without this, [`generate`](super::generate)
    /// returns a [`GenerateError`] for nested path params.
    #[must_use]
    pub fn wrapper_type(mut self, type_path: &str) -> Self {
        self.wrapper_type = Some(type_path.to_string());
        self
    }

    /// Set the SSE keep-alive interval in seconds (default: 15).
    ///
    /// Values less than 1 are clamped to 1 to prevent continuous keep-alive spam.
    #[must_use]
    pub fn sse_keep_alive_secs(mut self, secs: u64) -> Self {
        self.sse_keep_alive_secs = secs.max(1);
        self
    }

    /// Set the extension type extracted from Axum request extensions.
    ///
    /// When set, generated handlers use `Option<Extension<T>>` to extract
    /// the value and forward it to `build_tonic_request`. Typically used
    /// for auth info (e.g., `"my_app::AuthInfo"`).
    /// When `None`, handlers skip extension extraction entirely.
    ///
    /// # Example
    /// ```ignore
    /// config.extension_type("my_app::AuthInfo")
    /// ```
    #[must_use]
    pub fn extension_type(mut self, type_path: &str) -> Self {
        self.extension_type = Some(type_path.to_string());
        self
    }

    /// Add extra HTTP headers to forward from REST requests to gRPC metadata.
    ///
    /// These are combined with the default [`FORWARDED_HEADERS`] at startup.
    /// Use for vendor-specific headers like Cloudflare's `cf-connecting-ip`.
    ///
    /// # Example
    /// ```ignore
    /// // Forward Cloudflare client IP header
    /// config.extra_forwarded_headers(&["cf-connecting-ip"])
    /// ```
    #[must_use]
    pub fn extra_forwarded_headers(mut self, headers: &[&str]) -> Self {
        self.extra_forwarded_headers = headers.iter().map(ToString::to_string).collect();
        self
    }

    /// Resolve a proto package name to its Rust module name.
    pub(crate) fn rust_module(&self, proto_package: &str) -> Option<&str> {
        self.packages.get(proto_package).map(String::as_str)
    }

    /// Return the extension extractor line for the handler signature, or empty
    /// string if no extension type is configured.
    ///
    /// With `extension_type("Foo")`: `"    ext: Option<Extension<Foo>>,\n"`
    /// Without:                      `""`
    pub(crate) fn extension_extractor_line(&self) -> String {
        match &self.extension_type {
            Some(ty) => format!("    ext: Option<Extension<{ty}>>,\n"),
            None => String::new(),
        }
    }

    /// Return the extension binding + `build_tonic_request` call for the handler body.
    ///
    /// When `extra_forwarded_headers` is empty, uses `build_tonic_request`
    /// (which forwards the default header set). When extra headers are
    /// configured, uses `build_tonic_request_with_headers` with the
    /// generated `ALL_FORWARDED_HEADERS` constant.
    pub(crate) fn extension_and_request_lines(&self, body_var: &str) -> String {
        let rt = &self.runtime_crate;
        let build_fn = if self.extra_forwarded_headers.is_empty() {
            match &self.extension_type {
                Some(_) => format!("{rt}::build_tonic_request({body_var}, &headers, ext)",),
                None => format!("{rt}::build_tonic_request::<_, ()>({body_var}, &headers, None)",),
            }
        } else {
            match &self.extension_type {
                Some(_) => format!(
                    "{rt}::build_tonic_request_with_headers({body_var}, &headers, ext, ALL_FORWARDED_HEADERS)",
                ),
                None => format!(
                    "{rt}::build_tonic_request_with_headers::<_, ()>({body_var}, &headers, None, ALL_FORWARDED_HEADERS)",
                ),
            }
        };

        match &self.extension_type {
            Some(_) => format!(
                "    let ext = ext.map(|Extension(v)| v);\n\
                 \x20   let req = {build_fn};\n",
            ),
            None => format!("    let req = {build_fn};\n",),
        }
    }

    /// Convert a fully-qualified proto type to a Rust type path.
    ///
    /// Uses the resolved packages map for accurate module resolution:
    /// - `.auth.v1.User` → `{proto_root}::auth::User` (with `.package("auth.v1", "auth")`)
    /// - `.auth.v1.User` → `{proto_root}::auth::v1::User` (auto-discovered)
    /// - `.google.protobuf.Empty` → `()`
    ///
    /// Falls back to first-segment heuristic for types whose package is not
    /// in the resolved map (e.g., cross-package references).
    pub(crate) fn proto_type_to_rust(&self, proto_fqn: &str) -> String {
        if proto_fqn == ".google.protobuf.Empty" {
            return "()".to_string();
        }

        let trimmed = proto_fqn.trim_start_matches('.');

        // Find the longest matching package prefix in the packages map
        let mut best: Option<(&str, &str)> = None;
        for (package, module) in &self.packages {
            if let Some(rest) = trimmed.strip_prefix(package.as_str()) {
                if rest.starts_with('.') && best.is_none_or(|(p, _)| package.len() > p.len()) {
                    best = Some((package.as_str(), module.as_str()));
                }
            }
        }

        if let Some((package, module)) = best {
            let type_name = &trimmed[package.len() + 1..];
            format!("{}::{module}::{type_name}", self.proto_root)
        } else {
            // Fallback: use first segment as module name
            let parts: Vec<&str> = trimmed.split('.').collect();
            if parts.len() >= 3 {
                let package = parts[0];
                let type_name = parts[parts.len() - 1];
                format!("{}::{package}::{type_name}", self.proto_root)
            } else {
                proto_fqn.to_string()
            }
        }
    }
}
