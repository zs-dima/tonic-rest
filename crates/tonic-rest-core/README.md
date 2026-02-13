# tonic-rest-core

[![Crates.io](https://img.shields.io/crates/v/tonic-rest-core.svg)](https://crates.io/crates/tonic-rest-core)
[![docs.rs](https://img.shields.io/docsrs/tonic-rest-core)](https://docs.rs/tonic-rest-core)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.82-blue.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)

Shared protobuf descriptor types for the [tonic-rest](https://github.com/zs-dima/tonic-rest) ecosystem.

This is an **internal crate** — it provides the custom `prost::Message` types
that preserve the `google.api.http` extension (field 72295728) which standard
`prost_types::MethodOptions` drops during decoding.

Both [`tonic-rest-build`](https://crates.io/crates/tonic-rest-build) and
[`tonic-rest-openapi`](https://crates.io/crates/tonic-rest-openapi) depend on
this crate to share a single set of descriptor types. You should not need to
depend on it directly — use the higher-level crates instead.

## License

[MIT](LICENSE)
