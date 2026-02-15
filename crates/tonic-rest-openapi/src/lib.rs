#![allow(clippy::doc_markdown)] // README uses "OpenAPI" proper noun throughout
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! ## API Reference

#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "test-support")]
use std::collections::HashMap;

mod config;
pub(crate) use tonic_rest_core::descriptor;
mod discover;
mod error;
mod patch;

/// Default `$ref` path for the REST error response schema.
///
/// Override via [`PatchConfig::error_schema_ref`] or [`ProjectConfig::error_schema_ref`]
/// when your proto package uses a different path (e.g., `"#/components/schemas/myapp.v1.Error"`).
pub const DEFAULT_ERROR_SCHEMA_REF: &str = "#/components/schemas/ErrorResponse";

pub use config::{
    ContactInfo, ExternalDocsInfo, InfoOverrides, LicenseInfo, PlainTextEndpoint, ProjectConfig,
    ServerEntry, TransformConfig,
};
pub use discover::{
    EnumRewrite, FieldConstraint, OperationEntry, PathParamConstraint, PathParamInfo,
    ProtoMetadata, SchemaConstraints, StreamingOp, discover,
};
pub use error::{Error, Result};
pub use patch::{PatchConfig, patch};

/// Test-support utilities for constructing `ProtoMetadata` fixtures.
///
/// These setters bypass the normal [`discover()`] path, allowing tests to
/// populate individual fields without a real proto descriptor. Only available
/// when the `test-support` feature is enabled.
#[cfg(feature = "test-support")]
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
