#![allow(clippy::doc_markdown)] // README uses "OpenAPI" proper noun throughout
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! ## API Reference

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codegen;
#[doc(hidden)]
/// Internal protobuf descriptor types shared with `tonic-rest-openapi`.
///
/// **Not covered by semver guarantees.** These types are `#[doc(hidden)]` and
/// may change in any release, including patch versions. Do not depend on them
/// directly — use the public API surface of `tonic-rest-build` instead.
pub mod descriptor;
#[cfg(feature = "helpers")]
mod helpers;

pub use codegen::{generate, GenerateError, RestCodegenConfig};
#[cfg(feature = "helpers")]
pub use helpers::{
    configure_prost_serde, configure_prost_serde_with_options, dump_file_descriptor_set,
    try_configure_prost_serde, try_configure_prost_serde_with_options,
    try_dump_file_descriptor_set, ProstSerdeConfig,
};
