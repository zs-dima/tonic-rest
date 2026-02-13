# tonic-rest

[![Crates.io](https://img.shields.io/crates/v/tonic-rest.svg)](https://crates.io/crates/tonic-rest)
[![docs.rs](https://img.shields.io/docsrs/tonic-rest)](https://docs.rs/tonic-rest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.82-blue.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)

Runtime types for REST + SSE endpoints generated from protobuf `google.api.http` annotations.

This crate provides the shared types that generated Axum handlers reference at runtime.
The companion crate [`tonic-rest-build`](https://crates.io/crates/tonic-rest-build)
generates the handler code at build time.

## Types

- **`RestError`** — Converts `tonic::Status` to HTTP JSON error responses following the [Google API error model](https://cloud.google.com/apis/design/errors)
- **`build_tonic_request`** — Bridges Axum HTTP requests to `tonic::Request`, forwarding headers and extensions (e.g., auth info)
- **`sse_error_event`** — Formats gRPC errors as SSE events
- **`grpc_to_http_status`** / **`grpc_code_name`** — Maps all 17 gRPC codes to HTTP status codes and canonical names

## Serde Adapters

Behind the `serde` feature (enabled by default), provides `#[serde(with)]` adapters
for prost well-known types:

| Adapter                         | Type                              | Wire format                                       |
| ------------------------------- | --------------------------------- | ------------------------------------------------- |
| `timestamp` / `opt_timestamp`   | `Timestamp` / `Option<Timestamp>` | RFC 3339 (`"2025-01-15T09:30:00Z"`)               |
| `duration` / `opt_duration`     | `Duration` / `Option<Duration>`   | Seconds with suffix (`"300s"`)                    |
| `field_mask` / `opt_field_mask` | `FieldMask` / `Option<FieldMask>` | Comma-separated camelCase (`"displayName,email"`) |

And the `define_enum_serde!` macro for proto3 enum fields (which are `i32` in prost):

```rust,ignore
tonic_rest::define_enum_serde!(user_role, crate::UserRole);
// With prefix stripping:
tonic_rest::define_enum_serde!(health_status, crate::HealthStatus, "HEALTH_STATUS_");
```

## Feature Flags

| Feature | Default | Description                                                                                  |
| ------- | ------- | -------------------------------------------------------------------------------------------- |
| `serde` | **on**  | WKT serde adapters + `define_enum_serde!` macro (adds `prost-types`, `chrono`, `serde` deps) |

## Quick Start

```toml
[dependencies]
tonic-rest = "0.1"

[build-dependencies]
tonic-rest-build = "0.1"
```

For a complete end-to-end example, see [auth-service-rs](https://github.com/zs-dima/auth-service-rs).

## Companion Crates

| Crate                                                             | Purpose                | Cargo section          |
| ----------------------------------------------------------------- | ---------------------- | ---------------------- |
| **tonic-rest** (this)                                             | Runtime types          | `[dependencies]`       |
| [tonic-rest-build](https://crates.io/crates/tonic-rest-build)     | Build-time codegen     | `[build-dependencies]` |
| [tonic-rest-openapi](https://crates.io/crates/tonic-rest-openapi) | OpenAPI 3.1 generation | CLI / CI               |

## Compatibility

| tonic-rest | tonic | axum | prost-types | MSRV |
| ---------- | ----- | ---- | ----------- | ---- |
| 0.1.x      | 0.14  | 0.8  | 0.14        | 1.82 |


