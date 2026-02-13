# tonic-rest-openapi

[![Crates.io](https://img.shields.io/crates/v/tonic-rest-openapi.svg)](https://crates.io/crates/tonic-rest-openapi)
[![docs.rs](https://img.shields.io/docsrs/tonic-rest-openapi)](https://docs.rs/tonic-rest-openapi)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.82-blue.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)

OpenAPI 3.1 spec generation and patching from protobuf descriptors for Tonic gRPC services.

Reads compiled protobuf `FileDescriptorSet` bytes and a gnostic-generated OpenAPI YAML spec,
then applies a configurable pipeline of transforms to produce a clean OpenAPI 3.1 spec that
matches the runtime REST behavior.

## Pipeline

The 12-phase transform pipeline:

| Phase | Transform                                                 |
| ----- | --------------------------------------------------------- |
| 1     | OpenAPI 3.0 → 3.1 (version + nullable conversion)         |
| 2     | SSE streaming annotations                                 |
| 3     | Response fixes (empty→204, redirects, error schemas)      |
| 4     | Enum value rewrites (strip UNSPECIFIED, normalize values) |
| 5     | Unimplemented operation markers (501)                     |
| 6     | Security (Bearer JWT, public endpoint overrides)          |
| 7     | Cleanup (tags, empty bodies, unused schemas)              |
| 8     | UUID wrapper flattening (inline string/uuid)              |
| 9     | Validation constraint injection (from proto rules)        |
| 10    | Path parameter enrichment                                 |
| 11    | Request body inlining + orphan removal                    |
| 12    | CRLF → LF normalization                                   |

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

| Crate                                                         | Purpose                | Cargo section          |
| ------------------------------------------------------------- | ---------------------- | ---------------------- |
| [tonic-rest](https://crates.io/crates/tonic-rest)             | Runtime types          | `[dependencies]`       |
| [tonic-rest-build](https://crates.io/crates/tonic-rest-build) | Build-time codegen     | `[build-dependencies]` |
| **tonic-rest-openapi** (this)                                 | OpenAPI 3.1 generation | CLI / CI               |

> **Note:** `tonic-rest-openapi` depends on `tonic-rest-build` (as a regular `[dependency]`,
> not a `[build-dependency]`) for shared proto descriptor types. This means adding
> `tonic-rest-openapi` to your project will pull in `tonic-rest-build` as a transitive
> runtime dependency. This is intentional — both crates share the same custom `prost::Message`
> types for parsing `google.api.http` annotations. If you only need OpenAPI generation in CI
> (not in your application binary), consider using the CLI binary or a separate workspace crate.

## Dependencies

This crate uses [`serde_yaml_ng`](https://crates.io/crates/serde_yaml_ng) for YAML parsing
and serialization. `serde_yaml_ng` is a maintained fork of the archived `serde_yaml` crate.
While it is the best available option today, be aware that its ecosystem adoption is narrower
than `serde_json`. If a more widely-adopted YAML serde library emerges in the future,
migration may be warranted.

## Compatibility

| tonic-rest-openapi | tonic-rest-build | prost | MSRV |
| ------------------ | ---------------- | ----- | ---- |
| 0.1.x              | 0.1              | 0.14  | 1.82 |

