#![allow(clippy::doc_markdown)] // README uses "OpenAPI" proper noun throughout
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! ## API Reference

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codegen;

/// Protobuf descriptor types re-exported from [`tonic_rest_core`].
///
/// **Deprecated**: depend on `tonic-rest-core` directly instead.
/// This re-export exists for backward compatibility and will be
/// removed in a future release.
#[doc(hidden)]
pub use tonic_rest_core::descriptor;
#[cfg(feature = "helpers")]
mod helpers;

pub use codegen::{generate, GenerateError, RestCodegenConfig};
#[cfg(feature = "helpers")]
pub use helpers::{
    configure_prost_serde, configure_prost_serde_with_options, dump_file_descriptor_set,
    try_configure_prost_serde, try_configure_prost_serde_with_options,
    try_dump_file_descriptor_set, ProstSerdeConfig,
};
