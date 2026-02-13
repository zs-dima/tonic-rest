# tonic-rest-openapi

[![Crates.io](https://img.shields.io/crates/v/tonic-rest-openapi.svg)](https://crates.io/crates/tonic-rest-openapi)
[![docs.rs](https://img.shields.io/docsrs/tonic-rest-openapi)](https://docs.rs/tonic-rest-openapi)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV](https://img.shields.io/badge/MSRV-1.82-blue.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)

OpenAPI 3.1 spec generation and patching from protobuf descriptors for Tonic gRPC services.

Part of the [tonic-rest](https://github.com/zs-dima/tonic-rest) ecosystem — define your API once in proto files, get gRPC, REST, and OpenAPI 3.1.

Reads compiled protobuf `FileDescriptorSet` bytes and a gnostic-generated OpenAPI YAML spec,
then applies a configurable pipeline of transforms to produce a clean OpenAPI 3.1 spec that
matches the runtime REST behavior.

## Key Features

- **Proto as single source of truth** — OpenAPI spec derived from the same proto files that drive gRPC and REST
- **12-phase transform pipeline** — produces a clean OpenAPI 3.1 spec with security, validation constraints, and SSE annotations
- **Google error model** — injects structured error schemas matching runtime `RestError` JSON responses
- **Security built-in** — auto-generates Bearer JWT security scheme with public endpoint overrides
- **Library + CLI** — use programmatically in your build pipeline or as a standalone CI tool

## Pipeline

The 12-phase transform pipeline:

| Phase | Transform                                                                     |
| ----- | ----------------------------------------------------------------------------- |
| 1     | Structural (3.0 → 3.1 upgrade, server/info injection)                         |
| 2     | SSE streaming annotations + `Last-Event-ID` header                            |
| 3     | Response fixes (empty→204, plain text, redirects, error schemas, 201 Created) |
| 4     | Enum value rewrites (strip UNSPECIFIED, normalize values)                     |
| 5     | Unimplemented (501) and deprecated operation markers                          |
| 6     | Security (Bearer JWT, public endpoint overrides)                              |
| 7     | Cleanup (tags, empty bodies, unused schemas, `format: enum` removal)          |
| 8     | UUID wrapper flattening (path templates, `$ref` inlining, query params)       |
| 9     | Validation constraints + field access annotation + Duration rewriting         |
| 10    | Path field stripping + path parameter enrichment                              |
| 11    | Request body inlining + orphan removal                                        |
| 12    | CRLF → LF normalization                                                       |

Each phase can be individually enabled/disabled via `PatchConfig` or `ProjectConfig`.

## Usage

### As a Library

```rust,ignore
use tonic_rest_openapi::{PatchConfig, ProjectConfig, discover, patch};

// From a config file
let project = ProjectConfig::load(Path::new("api/openapi/config.yaml"))?;
let metadata = discover(&descriptor_bytes)?;
let config = PatchConfig::new(&metadata).with_project_config(&project);
let patched_yaml = patch(&input_yaml, &config)?;
```

Or configure programmatically:

```rust,ignore
let config = PatchConfig::new(&metadata)
    .unimplemented_methods(&["SetupMfa", "DisableMfa"])
    .public_methods(&["Login", "SignUp"])
    .bearer_description("JWT access token")
    .error_schema_ref("#/components/schemas/ErrorResponse");

let patched_yaml = patch(&input_yaml, &config)?;
```

### As a CLI

```bash
# Full pipeline: lint → generate → patch
tonic-rest-openapi generate --config api/openapi/config.yaml --cargo-toml Cargo.toml

# Standalone patch
tonic-rest-openapi patch --config api/openapi/config.yaml --input spec.yaml --output patched.yaml

# Discover proto metadata
tonic-rest-openapi discover --descriptor file_descriptor_set.bin
```

Enable the `cli` feature for the binary:

```toml
[dependencies]
tonic-rest-openapi = { version = "0.1", features = ["cli"] }
```

## Feature Flags

| Feature | Default | Description                                                                                                   |
| ------- | ------- | ------------------------------------------------------------------------------------------------------------- |
| `cli`   | off     | CLI binary with `generate`, `patch`, `discover`, `inject-version` subcommands (adds `clap`, `toml`, `anyhow`) |

## Project Config File

```yaml
# api/openapi/config.yaml
error_schema_ref: "#/components/schemas/ErrorResponse"

unimplemented_methods:
  - SetupMfa

public_methods:
  - Login
  - SignUp

plain_text_endpoints:
  - path: /health/live
    example: "OK"

metrics_path: /metrics
readiness_path: /health/ready

transforms:
  upgrade_to_3_1: true
  annotate_sse: true
  inject_validation: true
  add_security: true
```

For a complete end-to-end example with proto files, `build.rs`, REST handlers, and OpenAPI generation,
see [auth-service-rs](https://github.com/zs-dima/auth-service-rs).

## Companion Crates

| Crate                                                         | Purpose                 | Cargo section          |
| ------------------------------------------------------------- | ----------------------- | ---------------------- |
| [tonic-rest-core](https://crates.io/crates/tonic-rest-core)   | Shared descriptor types | internal               |
| [tonic-rest](https://crates.io/crates/tonic-rest)             | Runtime types           | `[dependencies]`       |
| [tonic-rest-build](https://crates.io/crates/tonic-rest-build) | Build-time codegen      | `[build-dependencies]` |
| **tonic-rest-openapi** (this)                                 | OpenAPI 3.1 generation  | CLI / CI               |


## Dependencies

This crate uses [`serde_yaml_ng`](https://crates.io/crates/serde_yaml_ng) for YAML parsing
and serialization. `serde_yaml_ng` is a maintained fork of the archived `serde_yaml` crate.
While it is the best available option today, be aware that its ecosystem adoption is narrower
than `serde_json`. If a more widely-adopted YAML serde library emerges in the future,
migration may be warranted.

## Compatibility

| tonic-rest-openapi | tonic-rest-core | prost | MSRV |
| ------------------ | --------------- | ----- | ---- |
| 0.1.x              | 0.1             | 0.14  | 1.82 |

## License

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)

