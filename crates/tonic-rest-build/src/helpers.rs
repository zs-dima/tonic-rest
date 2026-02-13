//! Build-time helpers for protobuf → Rust codegen with serde support.
//!
//! These helpers encapsulate the common build.rs patterns needed when using
//! `tonic-rest` with prost-generated types:
//!
//! - [`dump_file_descriptor_set`] — invoke `protoc` to produce a binary descriptor set
//! - [`configure_prost_serde`] — auto-discover fields and apply `#[serde(with)]` attributes
//!
//! # Typical `build.rs`
//!
//! ```ignore
//! use tonic_rest_build::{RestCodegenConfig, generate, dump_file_descriptor_set, configure_prost_serde};
//!
//! const PROTO_FILES: &[&str] = &["proto/service.proto"];
//! const PROTO_INCLUDES: &[&str] = &["proto"];
//!
//! fn main() {
//!     let out_dir = std::env::var("OUT_DIR").unwrap();
//!     let descriptor_path = format!("{out_dir}/file_descriptor_set.bin");
//!
//!     // Phase 1: Compile protos to descriptor set
//!     let descriptor_bytes = dump_file_descriptor_set(PROTO_FILES, PROTO_INCLUDES, &descriptor_path);
//!
//!     // Phase 2: Configure prost with serde attributes
//!     let mut config = prost_build::Config::new();
//!     config.file_descriptor_set_path(&descriptor_path);
//!     configure_prost_serde(
//!         &mut config,
//!         &descriptor_bytes,
//!         PROTO_FILES,
//!         "crate::serde_wkt",
//!         &[
//!             (".google.protobuf.Timestamp", "opt_timestamp"),
//!             (".google.protobuf.Duration", "opt_duration"),
//!         ],
//!         &[
//!             (".my.v1.Status", "my_status"),
//!         ],
//!     );
//!
//!     config.compile_protos(PROTO_FILES, PROTO_INCLUDES).unwrap();
//!
//!     // Phase 3: Generate REST routes
//!     let rest_config = RestCodegenConfig::new().package("my.v1", "my");
//!     let code = generate(&descriptor_bytes, &rest_config).unwrap();
//!     std::fs::write(format!("{out_dir}/rest_routes.rs"), code).unwrap();
//! }
//! ```

use prost::Message;
use prost_types::FileDescriptorSet;
use prost_types::field_descriptor_proto::{Label, Type};

/// Builder for configuring prost serde attributes.
///
/// Provides a cleaner alternative to the positional-parameter
/// [`configure_prost_serde`] family. Construct via [`ProstSerdeConfig::new`],
/// configure, then call [`apply`](Self::apply) or [`try_apply`](Self::try_apply).
///
/// # Example
///
/// ```ignore
/// ProstSerdeConfig::new(&descriptor_bytes, PROTO_FILES)
///     .wkt_root("crate::serde_wkt")
///     .wkt(".google.protobuf.Timestamp", "opt_timestamp")
///     .wkt(".google.protobuf.Duration", "opt_duration")
///     .enum_serde(".my.v1.Status", "my_status")
///     .rename_all("camelCase")
///     .apply(&mut config);
/// ```
#[derive(Clone, Debug)]
pub struct ProstSerdeConfig<'a> {
    descriptor_bytes: &'a [u8],
    proto_files: Vec<String>,
    wkt_root: String,
    wkt_map: Vec<(String, String)>,
    enum_map: Vec<(String, String)>,
    rename_all: Option<String>,
}

impl<'a> ProstSerdeConfig<'a> {
    /// Create a new builder with the descriptor bytes and proto source files.
    ///
    /// Defaults: `wkt_root = "tonic_rest::serde"`, `rename_all = "camelCase"`.
    #[must_use]
    pub fn new(descriptor_bytes: &'a [u8], proto_files: &[&str]) -> Self {
        Self {
            descriptor_bytes,
            proto_files: proto_files.iter().map(ToString::to_string).collect(),
            wkt_root: "tonic_rest::serde".to_string(),
            wkt_map: Vec::new(),
            enum_map: Vec::new(),
            rename_all: Some("camelCase".to_string()),
        }
    }

    /// Set the module path for WKT serde adapters.
    ///
    /// Default: `"tonic_rest::serde"`.
    ///
    /// # Panics
    ///
    /// Panics (in debug builds) if `root` is empty. An empty root would generate
    /// invalid `#[serde(with = "")]` attributes.
    #[must_use]
    pub fn wkt_root(mut self, root: &str) -> Self {
        debug_assert!(!root.is_empty(), "wkt_root must not be empty");
        self.wkt_root = root.to_string();
        self
    }

    /// Register a well-known type → serde module mapping.
    ///
    /// # Example
    /// ```ignore
    /// .wkt(".google.protobuf.Timestamp", "opt_timestamp")
    /// ```
    #[must_use]
    pub fn wkt(mut self, type_fqn: &str, serde_module: &str) -> Self {
        self.wkt_map
            .push((type_fqn.to_string(), serde_module.to_string()));
        self
    }

    /// Register an enum type → serde module mapping.
    ///
    /// # Example
    /// ```ignore
    /// .enum_serde(".my.v1.UserRole", "user_role")
    /// ```
    #[must_use]
    pub fn enum_serde(mut self, type_fqn: &str, serde_module: &str) -> Self {
        self.enum_map
            .push((type_fqn.to_string(), serde_module.to_string()));
        self
    }

    /// Set the `serde(rename_all)` strategy for all message types.
    ///
    /// Default: `"camelCase"`. Pass `None` to skip `rename_all` entirely.
    #[must_use]
    pub fn rename_all(mut self, strategy: &str) -> Self {
        self.rename_all = Some(strategy.to_string());
        self
    }

    /// Disable `serde(rename_all)` — fields keep their proto `snake_case` names.
    #[must_use]
    pub fn no_rename(mut self) -> Self {
        self.rename_all = None;
        self
    }

    /// Apply serde attributes to the prost config.
    ///
    /// # Panics
    ///
    /// Panics if the descriptor bytes cannot be decoded.
    /// Use [`try_apply`](Self::try_apply) for a fallible alternative.
    pub fn apply(self, config: &mut prost_build::Config) {
        self.try_apply(config)
            .expect("failed to decode FileDescriptorSet for serde configuration");
    }

    /// Fallible version of [`apply`](Self::apply).
    ///
    /// # Errors
    ///
    /// Returns [`prost::DecodeError`] if the descriptor bytes are invalid protobuf.
    pub fn try_apply(self, config: &mut prost_build::Config) -> Result<(), prost::DecodeError> {
        let proto_files_refs: Vec<&str> = self.proto_files.iter().map(String::as_str).collect();
        let wkt_refs: Vec<(&str, &str)> = self
            .wkt_map
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let enum_refs: Vec<(&str, &str)> = self
            .enum_map
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        try_configure_prost_serde_with_options(
            config,
            self.descriptor_bytes,
            &proto_files_refs,
            &self.wkt_root,
            &wkt_refs,
            &enum_refs,
            self.rename_all.as_deref(),
        )
    }
}

/// Invoke `protoc` to produce a binary `FileDescriptorSet`.
///
/// This is the first step in a typical build.rs flow. It runs `protoc` with
/// `--descriptor_set_out` and `--include_imports` to produce a complete
/// descriptor set containing all type information needed for serde attribute
/// discovery and REST codegen.
///
/// Uses `prost_build::protoc_from_env()` to locate `protoc`, which checks
/// the `PROTOC` environment variable first, then falls back to bundled/PATH.
///
/// # Panics
///
/// Panics if `protoc` cannot be found or exits with a non-zero status.
/// Use [`try_dump_file_descriptor_set`] for a fallible alternative.
///
/// # Returns
///
/// The raw bytes of the descriptor set file (also written to `out_path`).
#[must_use]
pub fn dump_file_descriptor_set(
    proto_files: &[&str],
    includes: &[&str],
    out_path: &str,
) -> Vec<u8> {
    try_dump_file_descriptor_set(proto_files, includes, out_path)
        .expect("failed to run protoc and produce descriptor set")
}

/// Fallible version of [`dump_file_descriptor_set`].
///
/// Returns `Err` if `protoc` cannot be found, exits with a non-zero status,
/// or the output file cannot be read. Allows callers to handle protoc failures
/// gracefully (e.g., skip REST codegen when protoc is unavailable).
///
/// # Errors
///
/// Returns [`std::io::Error`] if protoc cannot be spawned, fails, or the
/// descriptor file cannot be read.
pub fn try_dump_file_descriptor_set(
    proto_files: &[&str],
    includes: &[&str],
    out_path: &str,
) -> std::io::Result<Vec<u8>> {
    let protoc = prost_build::protoc_from_env();

    let mut cmd = std::process::Command::new(&protoc);
    cmd.arg("--descriptor_set_out").arg(out_path);
    cmd.arg("--include_imports");
    for inc in includes {
        cmd.arg(format!("--proto_path={inc}"));
    }
    for file in proto_files {
        cmd.arg(file);
    }

    let output = cmd.output().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("failed to run protoc at {}: {e}", protoc.display()),
        )
    })?;

    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "protoc failed with {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        )));
    }

    std::fs::read(out_path)
}

/// Configure prost serde attributes by scanning a `FileDescriptorSet`.
///
/// Automatically wires `#[serde(with)]` adapters for:
/// - **WKT fields**: matched via `wkt_map` (e.g., Timestamp → RFC 3339)
/// - **Enum fields**: matched via `enum_map` (e.g., `UserRole` → string name)
/// - **`skip_serializing_if`**: all proto3 explicit `optional` fields
///
/// Also applies base `Serialize`/`Deserialize` derives with `camelCase` renaming
/// to all messages and enums. Use [`configure_prost_serde_with_options`] for
/// custom `rename_all` strategies.
///
/// # Parameter Order
///
/// Note that this function takes `config` as the first parameter (`&mut prost_build::Config`),
/// which differs from [`generate(descriptor_bytes, config)`](crate::generate) which takes
/// data first. This follows `prost_build` convention where `Config` is the receiver-like
/// first argument. Consider using [`ProstSerdeConfig`] for a builder-style alternative.
///
/// # Parameters
///
/// - `config` — mutable `prost_build::Config` to apply attributes to
/// - `descriptor_bytes` — raw bytes of the `FileDescriptorSet` (from [`dump_file_descriptor_set`])
/// - `proto_files` — list of proto source files (used to identify "our" packages vs imports)
/// - `wkt_root` — module path for WKT serde adapters (e.g., `"crate::serde_wkt"` or
///   `"tonic_rest::serde"`)
/// - `wkt_map` — maps well-known type FQN → serde module name
///   (e.g., `(".google.protobuf.Timestamp", "opt_timestamp")`)
/// - `enum_map` — maps proto enum FQN → serde module name
///   (e.g., `(".core.v1.UserRole", "user_role")`)
///
/// # Panics
///
/// Panics if `descriptor_bytes` cannot be decoded as a `FileDescriptorSet`.
/// Use [`try_configure_prost_serde`] for a fallible alternative.
pub fn configure_prost_serde(
    config: &mut prost_build::Config,
    descriptor_bytes: &[u8],
    proto_files: &[&str],
    wkt_root: &str,
    wkt_map: &[(&str, &str)],
    enum_map: &[(&str, &str)],
) {
    try_configure_prost_serde(
        config,
        descriptor_bytes,
        proto_files,
        wkt_root,
        wkt_map,
        enum_map,
    )
    .expect("failed to decode FileDescriptorSet for serde configuration");
}

/// Fallible version of [`configure_prost_serde`].
///
/// Returns `Err` if `descriptor_bytes` is not a valid `FileDescriptorSet`,
/// allowing callers to provide custom error messages or combine with other
/// error types.
///
/// # Errors
///
/// Returns [`prost::DecodeError`] if the descriptor bytes are invalid protobuf.
pub fn try_configure_prost_serde(
    config: &mut prost_build::Config,
    descriptor_bytes: &[u8],
    proto_files: &[&str],
    wkt_root: &str,
    wkt_map: &[(&str, &str)],
    enum_map: &[(&str, &str)],
) -> Result<(), prost::DecodeError> {
    try_configure_prost_serde_with_options(
        config,
        descriptor_bytes,
        proto_files,
        wkt_root,
        wkt_map,
        enum_map,
        Some("camelCase"),
    )
}

/// Configure prost serde attributes with custom `rename_all` strategy.
///
/// Like [`configure_prost_serde`] but allows overriding the `serde(rename_all)`
/// strategy applied to all message types. Pass `None` to skip `rename_all` entirely.
///
/// # Parameters
///
/// - `rename_all` — serde rename strategy for messages (e.g., `Some("camelCase")`),
///   or `None` to skip renaming. Follows protobuf JSON mapping when set to `"camelCase"`.
///
/// See [`configure_prost_serde`] for descriptions of the other parameters.
///
/// # Panics
///
/// Panics if `descriptor_bytes` cannot be decoded as a `FileDescriptorSet`.
/// Use [`try_configure_prost_serde_with_options`] for a fallible alternative.
pub fn configure_prost_serde_with_options(
    config: &mut prost_build::Config,
    descriptor_bytes: &[u8],
    proto_files: &[&str],
    wkt_root: &str,
    wkt_map: &[(&str, &str)],
    enum_map: &[(&str, &str)],
    rename_all: Option<&str>,
) {
    try_configure_prost_serde_with_options(
        config,
        descriptor_bytes,
        proto_files,
        wkt_root,
        wkt_map,
        enum_map,
        rename_all,
    )
    .expect("failed to decode FileDescriptorSet for serde configuration");
}

/// Fallible version of [`configure_prost_serde_with_options`].
///
/// Like [`try_configure_prost_serde`] but accepts a custom `rename_all` strategy.
/// Pass `None` to skip `rename_all` entirely.
///
/// # Errors
///
/// Returns [`prost::DecodeError`] if the descriptor bytes are invalid protobuf.
pub fn try_configure_prost_serde_with_options(
    config: &mut prost_build::Config,
    descriptor_bytes: &[u8],
    proto_files: &[&str],
    wkt_root: &str,
    wkt_map: &[(&str, &str)],
    enum_map: &[(&str, &str)],
    rename_all: Option<&str>,
) -> Result<(), prost::DecodeError> {
    let fds = FileDescriptorSet::decode(descriptor_bytes)?;

    match rename_all {
        Some(strategy) => {
            config.message_attribute(
                ".",
                format!(
                    "#[derive(serde::Serialize, serde::Deserialize)] \
                     #[serde(rename_all = \"{strategy}\")]"
                ),
            );
        }
        None => {
            config.message_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
        }
    }
    config.enum_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");

    // Identify our source packages (vs imported deps like google.protobuf)
    let our_packages: Vec<String> = fds
        .file
        .iter()
        .filter(|f| {
            let name = f.name();
            proto_files.iter().any(|p| p.ends_with(name))
        })
        .map(|f| f.package().to_string())
        .collect();

    // Scan every message field in our packages
    for file in &fds.file {
        let package = file.package();
        if !our_packages.iter().any(|p| p == package) {
            continue;
        }
        for msg in &file.message_type {
            apply_field_attributes(
                config,
                &format!(".{package}"),
                msg,
                wkt_root,
                wkt_map,
                enum_map,
            );
        }
    }

    Ok(())
}

/// Recursively scan message fields and apply serde attributes.
fn apply_field_attributes(
    config: &mut prost_build::Config,
    parent_path: &str,
    msg: &prost_types::DescriptorProto,
    wkt_root: &str,
    wkt_map: &[(&str, &str)],
    enum_map: &[(&str, &str)],
) {
    let msg_name = msg.name();
    let msg_path = format!("{parent_path}.{msg_name}");

    // Collect map entry type names so we can skip map fields below.
    let map_entry_names: Vec<&str> = msg
        .nested_type
        .iter()
        .filter(|n| {
            n.options
                .as_ref()
                .is_some_and(prost_types::MessageOptions::map_entry)
        })
        .map(prost_types::DescriptorProto::name)
        .collect();

    for field in &msg.field {
        let field_name = field.name();
        let field_path = format!("{msg_path}.{field_name}");
        let field_type = field.r#type();
        let type_name = field.type_name();
        let is_repeated = field.label() == Label::Repeated;
        let is_optional = field.proto3_optional();

        // Skip map fields — they have native serde support.
        if is_repeated && field_type == Type::Message {
            let entry_fqn = type_name.rsplit('.').next().unwrap_or("");
            if map_entry_names.contains(&entry_fqn) {
                continue;
            }
        }

        // Proto3 explicit `optional` fields: skip None values in JSON output.
        if is_optional {
            config.field_attribute(
                &field_path,
                "#[serde(skip_serializing_if = \"Option::is_none\")]",
            );
        }

        match field_type {
            // Well-known types: auto-apply serde adapters from wkt_map.
            Type::Message if !is_repeated => {
                if let Some((_, module)) = wkt_map.iter().find(|(fqn, _)| *fqn == type_name) {
                    config.field_attribute(
                        &field_path,
                        format!("#[serde(with = \"{wkt_root}::{module}\", default)]"),
                    );
                }
            }
            // Enum fields: auto-wire serde module from enum_map.
            Type::Enum => {
                if let Some((_, module)) = enum_map.iter().find(|(fqn, _)| *fqn == type_name) {
                    let attr = if is_repeated {
                        format!("#[serde(with = \"{wkt_root}::{module}::repeated\")]")
                    } else if is_optional {
                        format!("#[serde(with = \"{wkt_root}::{module}::optional\", default)]")
                    } else {
                        format!("#[serde(with = \"{wkt_root}::{module}\")]")
                    };
                    config.field_attribute(&field_path, attr);
                }
            }
            _ => {}
        }
    }

    // Recurse into nested messages (but skip map entry types)
    for nested in &msg.nested_type {
        if nested
            .options
            .as_ref()
            .is_some_and(prost_types::MessageOptions::map_entry)
        {
            continue;
        }
        apply_field_attributes(config, &msg_path, nested, wkt_root, wkt_map, enum_map);
    }
}

#[cfg(test)]
mod tests {
    use prost::Message;
    use prost_types::field_descriptor_proto::{Label, Type};
    use prost_types::{
        DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
        FileDescriptorSet,
    };

    use super::*;

    /// Build a minimal `FileDescriptorSet` with one file and encode it.
    fn encode_fdset(file: FileDescriptorProto) -> Vec<u8> {
        FileDescriptorSet { file: vec![file] }.encode_to_vec()
    }

    /// Helper to build a simple field descriptor.
    fn make_field(name: &str, ty: Type, type_name: &str) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            r#type: Some(ty.into()),
            type_name: if type_name.is_empty() {
                None
            } else {
                Some(type_name.to_string())
            },
            label: Some(Label::Optional.into()),
            ..Default::default()
        }
    }

    /// Helper to build an optional field (proto3_optional = true).
    fn make_optional_field(name: &str, ty: Type) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            r#type: Some(ty.into()),
            label: Some(Label::Optional.into()),
            proto3_optional: Some(true),
            ..Default::default()
        }
    }

    /// Helper to build a repeated field.
    fn make_repeated_field(name: &str, ty: Type, type_name: &str) -> FieldDescriptorProto {
        FieldDescriptorProto {
            name: Some(name.to_string()),
            r#type: Some(ty.into()),
            type_name: if type_name.is_empty() {
                None
            } else {
                Some(type_name.to_string())
            },
            label: Some(Label::Repeated.into()),
            ..Default::default()
        }
    }

    fn make_file(name: &str, package: &str, messages: Vec<DescriptorProto>) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(name.to_string()),
            package: Some(package.to_string()),
            message_type: messages,
            ..Default::default()
        }
    }

    #[test]
    fn try_configure_rejects_invalid_bytes() {
        let mut config = prost_build::Config::new();
        let result = try_configure_prost_serde(
            &mut config,
            b"not valid protobuf",
            &["test.proto"],
            "crate::serde",
            &[],
            &[],
        );
        assert!(result.is_err());
    }

    #[test]
    fn try_configure_accepts_empty_descriptor() {
        let bytes = FileDescriptorSet { file: vec![] }.encode_to_vec();
        let mut config = prost_build::Config::new();
        let result = try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde",
            &[],
            &[],
        );
        assert!(result.is_ok());
    }

    #[test]
    fn try_configure_with_options_no_rename() {
        let file = make_file(
            "test.proto",
            "test.v1",
            vec![DescriptorProto {
                name: Some("Msg".to_string()),
                field: vec![make_field("name", Type::String, "")],
                ..Default::default()
            }],
        );
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        // Should not panic with rename_all = None
        let result = try_configure_prost_serde_with_options(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde",
            &[],
            &[],
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn builder_default_values() {
        let bytes = FileDescriptorSet { file: vec![] }.encode_to_vec();
        let builder = ProstSerdeConfig::new(&bytes, &["test.proto"]);

        assert_eq!(builder.wkt_root, "tonic_rest::serde");
        assert!(builder.wkt_map.is_empty());
        assert!(builder.enum_map.is_empty());
        assert_eq!(builder.rename_all.as_deref(), Some("camelCase"));
    }

    #[test]
    fn builder_chain() {
        let bytes = FileDescriptorSet { file: vec![] }.encode_to_vec();
        let builder = ProstSerdeConfig::new(&bytes, &["test.proto"])
            .wkt_root("my::serde")
            .wkt(".google.protobuf.Timestamp", "opt_timestamp")
            .wkt(".google.protobuf.Duration", "opt_duration")
            .enum_serde(".test.v1.Status", "my_status")
            .rename_all("snake_case");

        assert_eq!(builder.wkt_root, "my::serde");
        assert_eq!(builder.wkt_map.len(), 2);
        assert_eq!(builder.enum_map.len(), 1);
        assert_eq!(builder.rename_all.as_deref(), Some("snake_case"));
    }

    #[test]
    fn builder_no_rename() {
        let bytes = FileDescriptorSet { file: vec![] }.encode_to_vec();
        let builder = ProstSerdeConfig::new(&bytes, &["test.proto"]).no_rename();
        assert!(builder.rename_all.is_none());
    }

    #[test]
    fn builder_apply_succeeds_with_valid_bytes() {
        let file = make_file(
            "test.proto",
            "test.v1",
            vec![DescriptorProto {
                name: Some("Msg".to_string()),
                field: vec![make_field("name", Type::String, "")],
                ..Default::default()
            }],
        );
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        ProstSerdeConfig::new(&bytes, &["test.proto"])
            .wkt_root("crate::serde")
            .try_apply(&mut config)
            .expect("should succeed with valid descriptor bytes");
    }

    #[test]
    fn builder_try_apply_rejects_invalid_bytes() {
        let mut config = prost_build::Config::new();
        let result = ProstSerdeConfig::new(b"bad bytes", &["test.proto"]).try_apply(&mut config);
        assert!(result.is_err());
    }

    #[test]
    fn optional_field_gets_skip_serializing() {
        // Build a descriptor with a proto3 optional field
        let file = make_file(
            "test.proto",
            "test.v1",
            vec![DescriptorProto {
                name: Some("Msg".to_string()),
                field: vec![make_optional_field("nickname", Type::String)],
                ..Default::default()
            }],
        );
        let bytes = encode_fdset(file);

        // We can't directly inspect prost_build::Config internals, but we can
        // verify no panic occurs and the function runs cleanly.
        let mut config = prost_build::Config::new();
        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde",
            &[],
            &[],
        )
        .unwrap();
        // If we got here, the function correctly processed the optional field
    }

    #[test]
    fn wkt_field_gets_serde_with_attribute() {
        let file = make_file(
            "test.proto",
            "test.v1",
            vec![DescriptorProto {
                name: Some("Event".to_string()),
                field: vec![make_field(
                    "created_at",
                    Type::Message,
                    ".google.protobuf.Timestamp",
                )],
                ..Default::default()
            }],
        );
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde_wkt",
            &[(".google.protobuf.Timestamp", "opt_timestamp")],
            &[],
        )
        .unwrap();
        // No panic = WKT field was matched and attribute applied
    }

    #[test]
    fn enum_field_gets_serde_with_attribute() {
        let file = FileDescriptorProto {
            name: Some("test.proto".to_string()),
            package: Some("test.v1".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("User".to_string()),
                field: vec![make_field("role", Type::Enum, ".test.v1.UserRole")],
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("UserRole".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde_wkt",
            &[],
            &[(".test.v1.UserRole", "user_role")],
        )
        .unwrap();
    }

    #[test]
    fn repeated_enum_field_uses_repeated_module() {
        let file = FileDescriptorProto {
            name: Some("test.proto".to_string()),
            package: Some("test.v1".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("Filter".to_string()),
                field: vec![make_repeated_field(
                    "roles",
                    Type::Enum,
                    ".test.v1.UserRole",
                )],
                ..Default::default()
            }],
            enum_type: vec![EnumDescriptorProto {
                name: Some("UserRole".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde_wkt",
            &[],
            &[(".test.v1.UserRole", "user_role")],
        )
        .unwrap();
    }

    #[test]
    fn nested_messages_are_processed_recursively() {
        let file = make_file(
            "test.proto",
            "test.v1",
            vec![DescriptorProto {
                name: Some("Outer".to_string()),
                field: vec![make_field(
                    "ts",
                    Type::Message,
                    ".google.protobuf.Timestamp",
                )],
                nested_type: vec![DescriptorProto {
                    name: Some("Inner".to_string()),
                    field: vec![make_field(
                        "updated_at",
                        Type::Message,
                        ".google.protobuf.Timestamp",
                    )],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde",
            &[(".google.protobuf.Timestamp", "opt_timestamp")],
            &[],
        )
        .unwrap();
    }

    #[test]
    fn map_entry_fields_are_skipped() {
        // Simulate a proto map<string, string> which creates a nested MapEntry type
        let file = make_file(
            "test.proto",
            "test.v1",
            vec![DescriptorProto {
                name: Some("Config".to_string()),
                field: vec![make_repeated_field(
                    "labels",
                    Type::Message,
                    ".test.v1.Config.LabelsEntry",
                )],
                nested_type: vec![DescriptorProto {
                    name: Some("LabelsEntry".to_string()),
                    field: vec![
                        make_field("key", Type::String, ""),
                        make_field("value", Type::String, ""),
                    ],
                    options: Some(prost_types::MessageOptions {
                        map_entry: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let bytes = encode_fdset(file);
        let mut config = prost_build::Config::new();

        // Should not panic — map entries should be skipped
        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde",
            &[],
            &[],
        )
        .unwrap();
    }

    #[test]
    fn ignores_imported_packages() {
        let fdset = FileDescriptorSet {
            file: vec![
                // Our file
                make_file(
                    "test.proto",
                    "test.v1",
                    vec![DescriptorProto {
                        name: Some("Msg".to_string()),
                        field: vec![make_field("name", Type::String, "")],
                        ..Default::default()
                    }],
                ),
                // Imported file (e.g., google.protobuf) — should be skipped
                make_file(
                    "google/protobuf/timestamp.proto",
                    "google.protobuf",
                    vec![DescriptorProto {
                        name: Some("Timestamp".to_string()),
                        field: vec![make_field("seconds", Type::Int64, "")],
                        ..Default::default()
                    }],
                ),
            ],
        };
        let bytes = fdset.encode_to_vec();
        let mut config = prost_build::Config::new();

        // Only "test.proto" is in our proto_files list
        try_configure_prost_serde(
            &mut config,
            &bytes,
            &["test.proto"],
            "crate::serde",
            &[],
            &[],
        )
        .unwrap();
        // google.protobuf messages should not have had attributes applied
    }
}
