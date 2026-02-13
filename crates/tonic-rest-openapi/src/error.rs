//! Typed error enum for the `tonic-rest-openapi` library API.
//!
//! Library consumers can match on specific variants. The CLI (`main.rs`)
//! converts these to `anyhow::Error` at the binary boundary for richer
//! context messages.

/// Errors produced by `tonic-rest-openapi` library operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// File I/O failure (reading config, descriptor, or spec files).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// YAML parsing or serialization failure.
    #[error(transparent)]
    Yaml(#[from] serde_yaml_ng::Error),

    /// Proto `FileDescriptorSet` decoding failure.
    #[error("failed to decode proto descriptor: {0}")]
    ProtoDecode(#[from] prost::DecodeError),

    /// A proto method name was not found in the descriptor set.
    ///
    /// Check spelling or verify the method has a `google.api.http` annotation.
    #[error(
        "method '{method}' not found in proto descriptors; \
         check spelling or verify it has a google.api.http annotation"
    )]
    MethodNotFound {
        /// The unresolved method name.
        method: String,
    },

    /// A bare method name matches multiple services.
    ///
    /// Use qualified `Service.Method` syntax to disambiguate
    /// (e.g., `"AuthService.Delete"` instead of `"Delete"`).
    #[error(
        "ambiguous method name '{method}' matches multiple services: {candidates:?}; \
         use qualified 'Service.Method' syntax to disambiguate"
    )]
    AmbiguousMethodName {
        /// The ambiguous bare method name.
        method: String,
        /// All matching operation IDs.
        candidates: Vec<String>,
    },
}

/// Convenience alias used throughout the library's public API.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time assertion that `Error` is `Send + Sync`.
    /// Required for use in async contexts and across thread boundaries.
    const _: () = {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Error>();
    };
}
