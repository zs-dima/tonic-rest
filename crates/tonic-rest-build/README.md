# tonic-rest-build

[![Crates.io](https://img.shields.io/crates/v/tonic-rest-build.svg)](https://crates.io/crates/tonic-rest-build)
[![docs.rs](https://img.shields.io/docsrs/tonic-rest-build)](https://docs.rs/tonic-rest-build)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.82-blue.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)

Build-time REST codegen from protobuf `google.api.http` annotations for Tonic + Axum.

Part of the [tonic-rest](https://github.com/zs-dima/tonic-rest) ecosystem — define your API once in proto files, get gRPC, REST, and OpenAPI 3.1.

Reads a compiled proto `FileDescriptorSet`, extracts `google.api.http` annotations,
and generates Axum route handler code that calls through Tonic service traits — keeping
proto files as the single source of truth for both gRPC and REST APIs.

## Key Features

- **Proto as single source of truth** — one definition drives gRPC, REST endpoints, and OpenAPI docs
- **Build-time codegen** — Axum handlers generated from `FileDescriptorSet` at compile time; zero runtime overhead or reflection
- **Standard annotations** — uses [`google.api.http`](https://cloud.google.com/endpoints/docs/grpc/transcoding) bindings, not a proprietary DSL
- **Zero-config auto-discovery** — scans the descriptor set for any service with HTTP annotations; no manual package listing required
- **SSE for server streaming** — streaming RPCs are automatically exposed as Server-Sent Events endpoints
- **Serde auto-wiring** — `configure_prost_serde` discovers WKT fields and applies `#[serde(with)]` attributes automatically

## Quick Start

```toml
[dependencies]
tonic-rest = "0.1"

[build-dependencies]
tonic-rest-build = "0.1"
prost-build = "0.14"
```

Zero-config `build.rs` — auto-discovers packages from the descriptor set:

```rust,ignore
use tonic_rest_build::{RestCodegenConfig, generate, dump_file_descriptor_set};

const PROTO_FILES: &[&str] = &["proto/service.proto"];
const PROTO_INCLUDES: &[&str] = &["proto"];

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let descriptor_path = format!("{out_dir}/file_descriptor_set.bin");

    // Phase 1: Compile protos → descriptor set
    let descriptor_bytes = dump_file_descriptor_set(PROTO_FILES, PROTO_INCLUDES, &descriptor_path);

    // Phase 2: Compile protos → Rust (prost/tonic)
    let mut config = prost_build::Config::new();
    config.file_descriptor_set_path(&descriptor_path);
    config.compile_protos(PROTO_FILES, PROTO_INCLUDES).unwrap();

    // Phase 3: Generate REST routes
    let rest_config = RestCodegenConfig::new();
    let code = generate(&descriptor_bytes, &rest_config).unwrap();
    std::fs::write(format!("{out_dir}/rest_routes.rs"), code).unwrap();
}
```

## Configuration

Explicit package mapping (e.g., when using `pub use v1::*;` re-exports):

```rust,ignore
let config = RestCodegenConfig::new()
    .package("auth.v1", "auth")
    .package("users.v1", "users")
    .wrapper_type("crate::core::Uuid")
    .extension_type("my_app::AuthInfo")
    .public_methods(&["Login", "SignUp"])
    .sse_keep_alive_secs(30);
```

### `RestCodegenConfig` Options

| Method                            | Default        | Description                                        |
| --------------------------------- | -------------- | -------------------------------------------------- |
| `.package(proto, rust)`           | auto-discover  | Proto package → Rust module mapping                |
| `.extension_type(path)`           | `None`         | Extension type for Axum `Extension<T>` extraction  |
| `.public_methods(list)`           | empty          | Methods whose paths skip auth middleware           |
| `.wrapper_type(path)`             | `None`         | Rust type for single-field wrapper messages (UUID) |
| `.proto_root(path)`               | `"crate"`      | Root module for proto types                        |
| `.runtime_crate(path)`            | `"tonic_rest"` | Path to runtime types                              |
| `.sse_keep_alive_secs(n)`         | `15`           | SSE keep-alive interval                            |
| `.extra_forwarded_headers(&[..])` | empty          | Extra HTTP headers to forward to gRPC metadata     |

## Feature Flags

| Feature   | Default | Description                                                                             |
| --------- | ------- | --------------------------------------------------------------------------------------- |
| `helpers` | **on**  | `dump_file_descriptor_set` and `configure_prost_serde` helpers (adds `prost-build` dep) |

## Serde Attribute Helper

Auto-discover proto fields and apply `#[serde(with)]` attributes:

```rust,ignore
use tonic_rest_build::configure_prost_serde;

configure_prost_serde(
    &mut config,
    &descriptor_bytes,
    PROTO_FILES,
    "crate::serde_wkt",
    &[(".google.protobuf.Timestamp", "opt_timestamp")],
    &[(".my.v1.UserRole", "user_role")],
);
```

## Runtime Dependencies

The generated handler code references types from these crates — ensure they are
in your `[dependencies]`:

| Crate        | Used for                                              |
| ------------ | ----------------------------------------------------- |
| `tonic-rest` | `RestError`, `build_tonic_request`, `sse_error_event` |
| `tonic`      | `tonic::Status`, `tonic::Request`, service traits     |
| `axum`       | Router, extractors, `Json`, `Query`, SSE              |
| `futures`    | `Stream`, `StreamExt` (streaming endpoints only)      |
| `serde_json` | `Json` extractor/response                             |

## Generated Code

For each service with HTTP annotations:

- `{service}_rest_router(service: Arc<S>) -> Router` — route registration
- Per-method handler functions with proper extractors
- `PUBLIC_REST_PATHS: &[&str]` — paths that bypass authentication middleware
- `all_rest_routes(...)` — combined router for all services

### Handler Variants

| HTTP Method     | Body           | Response                 |
| --------------- | -------------- | ------------------------ |
| POST/PUT/PATCH  | `Json<T>`      | `Json<Response>`         |
| GET             | `Query<T>`     | `Json<Response>`         |
| DELETE          | `T::default()` | `StatusCode::NO_CONTENT` |
| GET (streaming) | `Query<T>`     | `Sse<impl Stream>`       |

## Planned

- **`additional_bindings`**: Proto `HttpRule.additional_bindings` (multiple REST mappings per
  gRPC method) is not supported. Only the primary HTTP binding is processed.
- **Partial body selectors**: Only `body: "*"` (full body) and `body: ""` (no body) are
  supported. The `body: "field_name"` partial body binding from the gRPC-HTTP transcoding spec
  is not implemented.
- **Repeated WKT fields**: `configure_prost_serde` does not wire serde adapters for
  lists of well-known types (e.g. `repeated google.protobuf.Timestamp`). Single fields of these
  types work correctly.

For a complete end-to-end example with proto files, `build.rs`, REST handlers, and OpenAPI generation,
see [auth-service-rs](https://github.com/zs-dima/auth-service-rs).

## Companion Crates

| Crate                                                             | Purpose                | Cargo section          |
| ----------------------------------------------------------------- | ---------------------- | ---------------------- |
| [tonic-rest](https://crates.io/crates/tonic-rest)                 | Runtime types          | `[dependencies]`       |
| **tonic-rest-build** (this)                                       | Build-time codegen     | `[build-dependencies]` |
| [tonic-rest-openapi](https://crates.io/crates/tonic-rest-openapi) | OpenAPI 3.1 generation | CLI / CI               |

## Compatibility

| tonic-rest-build | prost / prost-build | tonic | axum | MSRV |
| ---------------- | ------------------- | ----- | ---- | ---- |
| 0.1.x            | 0.14                | 0.14  | 0.8  | 1.82 |

## License

[MIT](LICENSE)

