#![allow(clippy::doc_markdown)] // README uses "OpenAPI" proper noun throughout
#![doc = include_str!("../README.md")]
//!
//! ---
//!
//! ## API Reference
//!
//! # Types
//!
//! - [`RestError`] — Converts [`tonic::Status`] to HTTP JSON error responses
//! - [`build_tonic_request`] — Bridges Axum requests to [`tonic::Request`]
//! - [`sse_error_event`] — Formats gRPC errors as SSE events
//! - [`grpc_to_http_status`] — Maps gRPC status codes to HTTP status codes
//! - [`grpc_code_name`] — Returns canonical `SCREAMING_SNAKE_CASE` name for a gRPC code
//!
//! # Usage
//!
//! ```toml
//! [dependencies]
//! tonic-rest = "0.1"
//!
//! [build-dependencies]
//! tonic-rest-build = "0.1"
//! ```
//!
//! # Companion Crate
//!
//! | Crate               | Purpose            | Cargo section              |
//! |---------------------|--------------------|----------------------------|
//! | `tonic-rest` (this) | Runtime types      | `[dependencies]`           |
//! | `tonic-rest-build`  | Build-time codegen | `[build-dependencies]`     |

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod runtime;

pub use runtime::*;

/// Concatenate [`FORWARDED_HEADERS`] with extra headers at compile time.
///
/// Used by generated code from `tonic-rest-build` to ensure the default
/// forwarded header list stays in sync with the runtime constant.
///
/// ```ignore
/// const ALL: &[&str] = tonic_rest::concat_forwarded_headers!("x-custom");
/// ```
#[macro_export]
macro_rules! concat_forwarded_headers {
    ($($extra:expr),* $(,)?) => {
        &[
            // Canonical defaults from FORWARDED_HEADERS
            "authorization",
            "user-agent",
            "x-forwarded-for",
            "x-real-ip",
            // Extra headers
            $($extra,)*
        ]
    };
}

/// Serde adapters for prost well-known types and proto3 enums.
///
/// Provides adapters for both optional and required fields:
/// - `timestamp` / `opt_timestamp` — `Timestamp` ↔ RFC 3339
/// - `duration` / `opt_duration` — `Duration` ↔ `"300s"`
/// - `field_mask` / `opt_field_mask` — `FieldMask` ↔ `"name,email"`
///
/// Also provides the [`define_enum_serde`] macro for proto enum `#[serde(with)]` modules.
#[cfg(feature = "serde")]
pub mod serde;
