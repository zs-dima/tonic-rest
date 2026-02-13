//! Shared protobuf descriptor types for the tonic-rest ecosystem.
//!
//! This crate provides custom [`prost::Message`] types that preserve the
//! `google.api.http` extension (field 72295728) which standard
//! `prost_types::MethodOptions` drops during decoding.
//!
//! Both `tonic-rest-build` (build-time codegen) and `tonic-rest-openapi`
//! (`OpenAPI` spec generation) depend on these shared types. You should not
//! need to depend on this crate directly — use the higher-level crates instead.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod descriptor;
