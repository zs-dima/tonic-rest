# Changelog

All notable changes to the `tonic-rest` crates will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-02-13

### Added

- **tonic-rest**: Runtime types for REST + SSE endpoints generated from protobuf `google.api.http` annotations
  - `RestError` — converts `tonic::Status` to HTTP JSON error responses (Google API error model)
  - `build_tonic_request` — bridges Axum HTTP requests to `tonic::Request` with header forwarding
  - `sse_error_event` / SSE streaming support for server-side streaming RPCs
  - `grpc_to_http_status` / `grpc_code_name` — maps all 17 gRPC codes to HTTP status codes
  - Serde adapters for `Timestamp`, `Duration`, `FieldMask` (behind `serde` feature)
  - `define_enum_serde!` macro for proto3 enum fields

- **tonic-rest-build**: Build-time REST codegen from protobuf descriptors
  - `generate()` — reads `FileDescriptorSet`, extracts `google.api.http` annotations, emits Axum handlers
  - `RestCodegenConfig` — package mapping, extension types, public methods, SSE keep-alive
  - `dump_file_descriptor_set` / `configure_prost_serde` helpers (behind `helpers` feature)

- **tonic-rest-openapi**: OpenAPI 3.1 spec generation and patching
  - 12-phase transform pipeline (OAS 3.0→3.1 upgrade, SSE, security, validation, cleanup)
  - `PatchConfig` / `ProjectConfig` for programmatic and file-based configuration
  - CLI binary with `generate`, `patch`, `discover`, `inject-version` subcommands (behind `cli` feature)

[0.1.0]: https://github.com/zs-dima/tonic-rest/releases/tag/v0.1.0
