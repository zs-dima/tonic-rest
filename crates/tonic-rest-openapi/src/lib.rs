#![allow(clippy::doc_markdown)] // README uses "OpenAPI" proper noun throughout
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! ## API Reference

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod config;
pub(crate) use tonic_rest_build::descriptor;
mod discover;
mod error;
mod patch;

/// Default `$ref` path for the REST error response schema.
///
/// Override via [`PatchConfig::error_schema_ref`] or [`ProjectConfig::error_schema_ref`]
/// when your proto package uses a different path (e.g., `"#/components/schemas/myapp.v1.Error"`).
pub const DEFAULT_ERROR_SCHEMA_REF: &str = "#/components/schemas/ErrorResponse";

pub use config::{PlainTextEndpoint, ProjectConfig, TransformConfig};
pub use discover::{discover, ProtoMetadata};
pub use error::{Error, Result};
pub use patch::{patch, PatchConfig};

/// Internal types for advanced use and testing.
///
/// **Not covered by semver guarantees.** These re-exports are `#[doc(hidden)]`
/// and may change in any release, including patch versions. They exist for
/// integration testing and advanced use cases only.
#[doc(hidden)]
pub mod internal {
    pub use crate::discover::{
        resolve_operation_ids, EnumRewrite, FieldConstraint, OperationEntry, PathParamConstraint,
        PathParamInfo, SchemaConstraints, StreamingOp,
    };

    use std::collections::HashMap;

    use crate::discover::ProtoMetadata;

    /// Builder extension for populating `ProtoMetadata` fields in tests.
    ///
    /// These bypass the normal `discover()` path for fixture-based testing.
    impl ProtoMetadata {
        /// Set streaming ops (test helper).
        pub fn set_streaming_ops(&mut self, ops: Vec<StreamingOp>) {
            self.streaming_ops = ops;
        }

        /// Set operation IDs (test helper).
        pub fn set_operation_ids(&mut self, ids: Vec<OperationEntry>) {
            self.operation_ids = ids;
        }

        /// Set field constraints (test helper).
        pub fn set_field_constraints(&mut self, constraints: Vec<SchemaConstraints>) {
            self.field_constraints = constraints;
        }

        /// Set enum rewrites (test helper).
        pub fn set_enum_rewrites(&mut self, rewrites: Vec<EnumRewrite>) {
            self.enum_rewrites = rewrites;
        }

        /// Set enum value map (test helper).
        pub fn set_enum_value_map(&mut self, map: HashMap<String, String>) {
            self.enum_value_map = map;
        }
    }
}
