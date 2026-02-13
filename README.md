# tonic-rest

[![Crates.io](https://img.shields.io/crates/v/tonic-rest.svg)](https://crates.io/crates/tonic-rest)
[![docs.rs](https://img.shields.io/docsrs/tonic-rest)](https://docs.rs/tonic-rest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.82-blue.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)

Define your API once in proto files — get gRPC, REST, and OpenAPI 3.1.

```text
                    ┌──────────────────┐
                    │   .proto files   │
                    │ google.api.http  │
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
      ┌──────────────┐ ┌──────────┐ ┌─────────────┐
      │  Tonic gRPC  │ │ Axum REST│ │ OpenAPI 3.1 │
      │   handlers   │ │ handlers │ │    spec     │
      └──────────────┘ └──────────┘ └─────────────┘
```

`tonic-rest` reads standard `google.api.http` proto annotations and generates type-safe Axum REST handlers alongside your existing Tonic gRPC services — all at build time, with zero runtime reflection.

## Key Features

- **Proto as single source of truth** — one definition drives gRPC, REST endpoints, and OpenAPI docs
- **Build-time codegen** — Axum handlers are generated from `FileDescriptorSet` at compile time; no runtime overhead or reflection
- **Standard annotations** — uses [`google.api.http`](https://cloud.google.com/endpoints/docs/grpc/transcoding) bindings, not a proprietary DSL
- **Zero-config auto-discovery** — scans the descriptor set for any service with HTTP annotations; no manual package listing required
- **SSE for server streaming** — server-streaming RPCs are automatically exposed as Server-Sent Events endpoints
- **OpenAPI 3.1 pipeline** — 12-phase transform pipeline produces a clean spec with security, validation constraints, and proper SSE annotations
- **Google error model** — gRPC errors map to structured JSON responses following the [Google API error model](https://cloud.google.com/apis/design/errors)
- **Serde adapters** — ready-made `#[serde(with)]` modules for `Timestamp`, `Duration`, `FieldMask`, and proto3 enums

## Crates

| Crate                                            | Purpose                                                              | Cargo section          |
| ------------------------------------------------ | -------------------------------------------------------------------- | ---------------------- |
| [tonic-rest](crates/tonic-rest/)                 | Runtime types (error mapping, request bridging, SSE, serde adapters) | `[dependencies]`       |
| [tonic-rest-build](crates/tonic-rest-build/)     | Build-time codegen (proto → Axum handlers)                           | `[build-dependencies]` |
| [tonic-rest-openapi](crates/tonic-rest-openapi/) | OpenAPI 3.1 spec generation and patching (library + CLI)             | CLI / CI               |

## Quick Start

```toml
[dependencies]
tonic-rest = "0.1"

[build-dependencies]
tonic-rest-build = "0.1"
```

### `build.rs`

```rust,ignore
use tonic_rest_build::{RestCodegenConfig, generate, dump_file_descriptor_set};

const PROTO_FILES: &[&str] = &["proto/service.proto"];
const PROTO_INCLUDES: &[&str] = &["proto"];

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let descriptor_path = format!("{out_dir}/file_descriptor_set.bin");

    // Compile protos → descriptor set
    let descriptor_bytes = dump_file_descriptor_set(PROTO_FILES, PROTO_INCLUDES, &descriptor_path);

    // Compile protos → Rust (prost/tonic) as usual
    let mut config = prost_build::Config::new();
    config.file_descriptor_set_path(&descriptor_path);
    config.compile_protos(PROTO_FILES, PROTO_INCLUDES).unwrap();

    // Generate REST routes — auto-discovers services with HTTP annotations
    let rest_config = RestCodegenConfig::new();
    let code = generate(&descriptor_bytes, &rest_config).unwrap();
    std::fs::write(format!("{out_dir}/rest_routes.rs"), code).unwrap();
}
```

### OpenAPI Generation (CLI)

```bash
cargo install tonic-rest-openapi --features cli

tonic-rest-openapi generate --config api/openapi/config.yaml --cargo-toml Cargo.toml
```

## Example Project

For a complete end-to-end example with proto files, `build.rs`, REST handlers, and OpenAPI generation,
see [auth-service-rs](https://github.com/zs-dima/auth-service-rs).

## Compatibility

| tonic-rest | tonic | axum | prost | MSRV |
| ---------- | ----- | ---- | ----- | ---- |
| 0.1.x      | 0.14  | 0.8  | 0.14  | 1.82 |

## License

[MIT](LICENSE)
