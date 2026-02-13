//! Runtime types and utilities for generated REST route handlers.
//!
//! This module provides the shared types that generated Axum handlers reference:
//! - [`RestError`] — Error type that converts [`tonic::Status`] to HTTP responses
//! - [`build_tonic_request`] — Bridges Axum requests to [`tonic::Request`]
//! - [`sse_error_event`] — Formats gRPC errors as SSE events
//! - [`grpc_to_http_status`] — Maps gRPC status codes to HTTP status codes
//! - [`grpc_code_name`] — Returns the canonical `SCREAMING_SNAKE_CASE` name for a gRPC code

mod error;
mod request;
mod sse;
mod status_map;

pub use error::RestError;
pub use request::{
    build_tonic_request, build_tonic_request_simple, build_tonic_request_with_headers,
    cloudflare_header_names, forwarded_header_names, CLOUDFLARE_HEADERS, FORWARDED_HEADERS,
};
pub use sse::sse_error_event;
pub use status_map::{grpc_code_name, grpc_to_http_status};
