//! CLI for `tonic-rest-openapi`.
//!
//! Standalone binary — no `cargo xtask`, no workspace coupling.
//!
//! # Subcommands
//!
//! ```text
//! # All-in-one: lint + generate + patch (recommended)
//! tonic-rest-openapi generate --config api/openapi/config.yaml
//!
//! # Or run steps individually:
//! tonic-rest-openapi patch \
//!   --descriptor descriptor.bin \
//!   --input openapi.yaml \
//!   --config api/openapi/config.yaml
//!
//! tonic-rest-openapi discover --descriptor descriptor.bin
//!
//! # Optional: inject Cargo.toml version into buf.gen.yaml
//! tonic-rest-openapi inject-version \
//!   --buf-gen buf.gen.yaml \
//!   --cargo-toml Cargo.toml \
//!   --output target/buf.gen.yaml
//! ```

#![forbid(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Parser;
use serde_yaml_ng::Value;
use tonic_rest_openapi::{PatchConfig, ProjectConfig};

/// `OpenAPI` 3.1 spec generator and patcher for Tonic gRPC services.
#[derive(Parser)]
#[command(name = "tonic-rest-openapi", version, about)]
enum Cli {
    /// Run the full `OpenAPI` generation pipeline: lint → generate → patch.
    ///
    /// Wraps `buf lint`, `buf generate`, `buf build`, and the patch pipeline
    /// into a single command. Requires the `buf` CLI to be installed.
    Generate(GenerateArgs),

    /// Apply transforms to a gnostic-generated `OpenAPI` YAML spec.
    ///
    /// Use this when you run `buf generate` and `buf build` separately, or
    /// when integrating into an existing build pipeline. For the all-in-one
    /// flow, use `generate`.
    Patch(PatchArgs),

    /// Print proto metadata extracted from a compiled descriptor set.
    Discover(DiscoverArgs),

    /// Inject a version string into a `buf.gen.yaml` plugin `opt` array.
    ///
    /// This is optional project-specific glue for syncing the `OpenAPI` spec
    /// version with Cargo.toml. The core workflow only needs `generate`
    /// (or `discover` + `patch`).
    InjectVersion(InjectVersionArgs),
}

#[derive(Parser)]
#[allow(clippy::struct_excessive_bools)]
struct PatchArgs {
    /// Path to the compiled proto `FileDescriptorSet` (binary).
    #[arg(short, long)]
    descriptor: PathBuf,

    /// Path to the input `OpenAPI` YAML file.
    #[arg(short, long)]
    input: PathBuf,

    /// Path to the output `OpenAPI` YAML file. Defaults to overwriting `--input`.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Path to a project config YAML file.
    ///
    /// Provides method lists, error schema ref, and transform toggles.
    /// CLI flags override values from the config file.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Comma-separated proto method names that return UNIMPLEMENTED.
    /// Overrides `unimplemented_methods` from the config file.
    #[arg(long, value_delimiter = ',')]
    unimplemented: Vec<String>,

    /// Comma-separated proto method names that require no authentication.
    /// Overrides `public_methods` from the config file.
    #[arg(long, value_delimiter = ',')]
    public: Vec<String>,

    /// `$ref` path for the REST error response schema.
    /// Overrides `error_schema_ref` from the config file.
    #[arg(long)]
    error_schema_ref: Option<String>,

    /// Skip the 3.0 → 3.1 upgrade transform.
    #[arg(long)]
    no_upgrade: bool,

    /// Skip SSE streaming annotation.
    #[arg(long)]
    no_sse: bool,

    /// Skip validation constraint injection.
    #[arg(long)]
    no_validation: bool,

    /// Skip security scheme addition.
    #[arg(long)]
    no_security: bool,

    /// Skip request body inlining.
    #[arg(long)]
    no_inline: bool,

    /// Skip UUID wrapper flattening.
    #[arg(long)]
    no_uuid_flatten: bool,
}

#[derive(Parser)]
struct DiscoverArgs {
    /// Path to the compiled proto `FileDescriptorSet` (binary).
    #[arg(short, long)]
    descriptor: PathBuf,
}

#[derive(Parser)]
struct InjectVersionArgs {
    /// Path to `buf.gen.yaml`.
    #[arg(long)]
    buf_gen: PathBuf,

    /// Version string to inject. Mutually exclusive with `--cargo-toml`.
    #[arg(long, conflicts_with = "cargo_toml")]
    version: Option<String>,

    /// Read version from this `Cargo.toml` instead of `--version`.
    #[arg(long, conflicts_with = "version")]
    cargo_toml: Option<PathBuf>,

    /// Write the modified YAML to this path instead of overwriting `--buf-gen`.
    ///
    /// Useful with `buf generate --template` to avoid modifying the original file.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct GenerateArgs {
    /// Path to `buf.gen.yaml` template.
    #[arg(long, default_value = "buf.gen.yaml")]
    buf_gen: PathBuf,

    /// Path to project config YAML for the patch pipeline.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Version string to inject into `buf.gen.yaml`.
    /// If omitted, reads from `--cargo-toml`.
    #[arg(long, conflicts_with = "cargo_toml")]
    version: Option<String>,

    /// Read version from this `Cargo.toml` (default: auto-detect in current dir).
    #[arg(long)]
    cargo_toml: Option<PathBuf>,

    /// Path to the `OpenAPI` spec (input for patching and final output).
    /// Must match the output path configured in `buf.gen.yaml`.
    #[arg(long, default_value = "api/openapi/v1/openapi.yaml")]
    spec: PathBuf,

    /// Directory for intermediate build artifacts (versioned `buf.gen.yaml`, descriptor).
    #[arg(long, default_value = "target")]
    work_dir: PathBuf,

    /// Skip the `buf lint` step.
    #[arg(long)]
    no_lint: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli {
        Cli::Generate(args) => run_generate(&args),
        Cli::Patch(args) => run_patch(&args),
        Cli::Discover(args) => run_discover(&args),
        Cli::InjectVersion(args) => run_inject_version(&args),
    }
}

fn run_generate(args: &GenerateArgs) -> anyhow::Result<()> {
    // Step 1: Lint
    if !args.no_lint {
        eprintln!("Linting proto files...");
        run_buf(&["lint"])?;
    }

    // Step 2: Resolve version
    let version = resolve_version(args.version.as_ref(), args.cargo_toml.as_ref())?;

    // Step 3: Inject version into buf.gen.yaml → work_dir copy
    fs::create_dir_all(&args.work_dir)
        .with_context(|| format!("Failed to create work dir: {}", args.work_dir.display()))?;

    let versioned_buf_gen = args.work_dir.join("buf.gen.yaml");
    eprintln!("Injecting version={version} into buf.gen.yaml...");
    let buf_gen_content = fs::read_to_string(&args.buf_gen)
        .with_context(|| format!("Failed to read {}", args.buf_gen.display()))?;
    let versioned_content = inject_version_yaml(&buf_gen_content, &version)?;
    fs::write(&versioned_buf_gen, versioned_content)
        .with_context(|| format!("Failed to write {}", versioned_buf_gen.display()))?;

    // Step 4: buf generate
    eprintln!("Generating OpenAPI spec...");
    run_buf(&[
        "generate",
        "--template",
        &versioned_buf_gen.to_string_lossy(),
    ])?;

    // Step 5: Build proto descriptor
    let descriptor_path = args.work_dir.join("proto-descriptor.bin");
    eprintln!("Building proto descriptor...");
    run_buf(&[
        "build",
        "--as-file-descriptor-set",
        "-o",
        &descriptor_path.to_string_lossy(),
    ])?;

    // Step 6: Discover + patch
    let project = match &args.config {
        Some(path) => {
            eprintln!("Loading config: {}", path.display());
            ProjectConfig::load(path)
                .with_context(|| format!("Failed to load config: {}", path.display()))?
        }
        None => ProjectConfig::default(),
    };

    let descriptor_bytes = fs::read(&descriptor_path)
        .with_context(|| format!("Failed to read descriptor: {}", descriptor_path.display()))?;
    let input_yaml = fs::read_to_string(&args.spec)
        .with_context(|| format!("Failed to read spec: {}", args.spec.display()))?;

    let metadata = tonic_rest_openapi::discover(&descriptor_bytes)
        .context("Failed to discover proto metadata")?;
    eprintln!(
        "Discovered {} operations, {} streaming",
        metadata.operation_ids().len(),
        metadata.streaming_ops().len(),
    );

    let config = PatchConfig::new(&metadata).with_project_config(&project);
    let output = tonic_rest_openapi::patch(&input_yaml, &config).context("Failed to patch spec")?;

    fs::write(&args.spec, &output)
        .with_context(|| format!("Failed to write spec: {}", args.spec.display()))?;
    eprintln!("OpenAPI 3.1 spec ready: {}", args.spec.display());

    Ok(())
}

/// Resolve version from explicit flag, Cargo.toml flag, or auto-detect.
fn resolve_version(
    explicit: Option<&String>,
    cargo_toml: Option<&PathBuf>,
) -> anyhow::Result<String> {
    match (explicit, cargo_toml) {
        (Some(v), _) => Ok(v.clone()),
        (_, Some(path)) => read_cargo_version(path),
        (None, None) => {
            let default_path = Path::new("Cargo.toml");
            if default_path.exists() {
                read_cargo_version(default_path)
            } else {
                bail!(
                    "No version specified. \
                     Use --version or --cargo-toml, \
                     or ensure Cargo.toml exists in the current directory."
                )
            }
        }
    }
}

/// Run a `buf` CLI command, forwarding stdout/stderr to the terminal.
fn run_buf(args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new("buf")
        .args(args)
        .status()
        .context(
            "Failed to run `buf` — is it installed? See https://buf.build/docs/installation",
        )?;

    if !status.success() {
        bail!("`buf {}` failed with {status}", args.join(" "));
    }
    Ok(())
}

/// Inject a version string into `buf.gen.yaml` content.
///
/// Finds all `opt` entries starting with `version=` and replaces the value.
fn inject_version_yaml(content: &str, version: &str) -> anyhow::Result<String> {
    let mut doc: Value =
        serde_yaml_ng::from_str(content).context("Failed to parse buf.gen.yaml")?;

    let plugins = doc
        .as_mapping_mut()
        .and_then(|m| m.get_mut("plugins"))
        .and_then(Value::as_sequence_mut)
        .context("buf.gen.yaml: missing 'plugins' array")?;

    let mut replaced = false;
    for plugin in plugins {
        let Some(opts) = plugin
            .as_mapping_mut()
            .and_then(|m| m.get_mut("opt"))
            .and_then(Value::as_sequence_mut)
        else {
            continue;
        };

        for opt in opts.iter_mut() {
            if opt.as_str().is_some_and(|s| s.starts_with("version=")) {
                *opt = Value::String(format!("version={version}"));
                replaced = true;
            }
        }
    }

    if !replaced {
        bail!("No `version=` option found in buf.gen.yaml plugins");
    }

    serde_yaml_ng::to_string(&doc).context("Failed to serialize buf.gen.yaml")
}

fn run_patch(args: &PatchArgs) -> anyhow::Result<()> {
    // Load project config (if provided), otherwise use defaults
    let project = match &args.config {
        Some(path) => {
            eprintln!("Loading config: {}", path.display());
            ProjectConfig::load(path)
                .with_context(|| format!("Failed to load config: {}", path.display()))?
        }
        None => ProjectConfig::default(),
    };

    // Read inputs
    let descriptor_bytes = fs::read(&args.descriptor)
        .with_context(|| format!("Failed to read descriptor: {}", args.descriptor.display()))?;

    let input_yaml = fs::read_to_string(&args.input)
        .with_context(|| format!("Failed to read input: {}", args.input.display()))?;

    // Discover proto metadata
    let metadata = tonic_rest_openapi::discover(&descriptor_bytes)
        .context("Failed to discover proto metadata")?;
    eprintln!(
        "Discovered {} operations, {} streaming",
        metadata.operation_ids().len(),
        metadata.streaming_ops().len(),
    );

    // Build PatchConfig: start from project config, then apply CLI overrides
    let config = PatchConfig::new(&metadata).with_project_config(&project);
    let config = apply_cli_overrides(config, args);

    // Patch
    let output = tonic_rest_openapi::patch(&input_yaml, &config).context("Failed to patch spec")?;

    // Write output
    let output_path = args.output.as_ref().unwrap_or(&args.input);
    fs::write(output_path, &output)
        .with_context(|| format!("Failed to write output: {}", output_path.display()))?;
    eprintln!("Wrote patched spec to {}", output_path.display());

    Ok(())
}

/// Apply CLI flags that override config file values.
fn apply_cli_overrides<'a>(mut config: PatchConfig<'a>, args: &PatchArgs) -> PatchConfig<'a> {
    // Method list overrides (CLI replaces config entirely if provided)
    if !args.unimplemented.is_empty() {
        let refs: Vec<&str> = args.unimplemented.iter().map(String::as_str).collect();
        config = config.unimplemented_methods(&refs);
    }
    if !args.public.is_empty() {
        let refs: Vec<&str> = args.public.iter().map(String::as_str).collect();
        config = config.public_methods(&refs);
    }

    // Scalar overrides
    if let Some(ref schema_ref) = args.error_schema_ref {
        config = config.error_schema_ref(schema_ref);
    }

    // Disable flags (one-directional: can only turn off via CLI)
    if args.no_upgrade {
        config = config.upgrade_to_3_1(false);
    }
    if args.no_sse {
        config = config.annotate_sse(false);
    }
    if args.no_validation {
        config = config.inject_validation(false);
    }
    if args.no_security {
        config = config.add_security(false);
    }
    if args.no_inline {
        config = config.inline_request_bodies(false);
    }
    if args.no_uuid_flatten {
        config = config.flatten_uuid_refs(false);
    }

    config
}

fn run_discover(args: &DiscoverArgs) -> anyhow::Result<()> {
    let descriptor_bytes = fs::read(&args.descriptor)
        .with_context(|| format!("Failed to read descriptor: {}", args.descriptor.display()))?;

    let metadata = tonic_rest_openapi::discover(&descriptor_bytes)
        .context("Failed to discover proto metadata")?;

    println!("=== Proto Metadata ===");
    println!();

    println!("Streaming operations: {}", metadata.streaming_ops().len());
    for op in metadata.streaming_ops() {
        println!("  {} {}", op.method.to_uppercase(), op.path);
    }

    println!();
    println!("Operation IDs: {}", metadata.operation_ids().len());
    for entry in metadata.operation_ids() {
        println!("  {} → {}", entry.method_name, entry.operation_id);
    }

    println!();
    println!(
        "Field constraints: {} schemas",
        metadata.field_constraints().len()
    );
    for sc in metadata.field_constraints() {
        println!("  {} ({} fields)", sc.schema, sc.fields.len());
    }

    println!();
    println!("Enum rewrites: {}", metadata.enum_rewrites().len());
    for rw in metadata.enum_rewrites() {
        println!("  {}.{} → {:?}", rw.schema, rw.field, rw.values);
    }

    println!();
    println!("Redirect paths: {:?}", metadata.redirect_paths());
    println!("UUID schema: {:?}", metadata.uuid_schema());

    Ok(())
}

fn run_inject_version(args: &InjectVersionArgs) -> anyhow::Result<()> {
    let version = match (&args.version, &args.cargo_toml) {
        (Some(v), _) => v.clone(),
        (_, Some(cargo_path)) => read_cargo_version(cargo_path)?,
        (None, None) => bail!("Either --version or --cargo-toml is required"),
    };

    eprintln!(
        "Injecting version={version} into {}",
        args.buf_gen.display()
    );

    let content = fs::read_to_string(&args.buf_gen)
        .with_context(|| format!("Failed to read {}", args.buf_gen.display()))?;

    let output = inject_version_yaml(&content, &version)?;

    let output_path = args.output.as_ref().unwrap_or(&args.buf_gen);
    fs::write(output_path, output)
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    eprintln!("Done");
    Ok(())
}

/// Read `version` from a Cargo.toml `[package]` or `[workspace.package]`.
fn read_cargo_version(path: &Path) -> anyhow::Result<String> {
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;

    let doc: toml::Table =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;

    if let Some(v) = doc
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(toml::Value::as_str)
    {
        return Ok(v.to_string());
    }

    if let Some(v) = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(toml::Value::as_str)
    {
        return Ok(v.to_string());
    }

    bail!("No version found in {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write content to a temporary file and return its path.
    fn write_temp_file(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tonic_rest_test_{name}"));
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn inject_version_replaces_existing() {
        let input = r"
version: v2
plugins:
  - remote: buf.build/community/google-gnostic-openapi
    out: api/openapi/v1
    opt:
      - version=0.0.0
      - naming=proto
";
        let result = inject_version_yaml(input, "1.2.3").unwrap();
        assert!(
            result.contains("version=1.2.3"),
            "version should be replaced"
        );
        assert!(
            !result.contains("version=0.0.0"),
            "old version should be gone"
        );
    }

    #[test]
    fn inject_version_multiple_plugins() {
        let input = r"
plugins:
  - remote: plugin-a
    opt:
      - version=0.0.0
  - remote: plugin-b
    opt:
      - version=0.0.0
";
        let result = inject_version_yaml(input, "2.0.0").unwrap();
        let count = result.matches("version=2.0.0").count();
        assert_eq!(count, 2, "both plugins should be updated");
    }

    #[test]
    fn inject_version_no_version_opt_errors() {
        let input = r"
plugins:
  - remote: plugin-a
    opt:
      - naming=proto
";
        let result = inject_version_yaml(input, "1.0.0");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No `version=`"));
    }

    #[test]
    fn inject_version_no_plugins_errors() {
        let input = "version: v2\n";
        let result = inject_version_yaml(input, "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn inject_version_invalid_yaml_errors() {
        let result = inject_version_yaml("{{invalid yaml", "1.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn read_cargo_version_package() {
        let path = write_temp_file(
            "cargo_pkg.toml",
            "[package]\nname = \"test\"\nversion = \"3.2.1\"\n",
        );
        let version = read_cargo_version(&path).unwrap();
        assert_eq!(version, "3.2.1");
    }

    #[test]
    fn read_cargo_version_workspace() {
        let path = write_temp_file(
            "cargo_ws.toml",
            "[workspace.package]\nversion = \"0.5.0\"\nedition = \"2021\"\n",
        );
        let version = read_cargo_version(&path).unwrap();
        assert_eq!(version, "0.5.0");
    }

    #[test]
    fn read_cargo_version_missing_errors() {
        let path = write_temp_file("cargo_no_ver.toml", "[package]\nname = \"test\"\n");
        let result = read_cargo_version(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No version"));
    }

    #[test]
    fn resolve_version_explicit() {
        let v = "1.0.0".to_string();
        let result = resolve_version(Some(&v), None).unwrap();
        assert_eq!(result, "1.0.0");
    }

    #[test]
    fn resolve_version_explicit_takes_precedence() {
        let v = "2.0.0".to_string();
        let cargo = PathBuf::from("nonexistent.toml");
        // explicit wins even if cargo_toml is provided
        let result = resolve_version(Some(&v), Some(&cargo)).unwrap();
        assert_eq!(result, "2.0.0");
    }

    #[test]
    fn resolve_version_from_cargo_toml() {
        let path = write_temp_file(
            "cargo_resolve.toml",
            "[package]\nname = \"test\"\nversion = \"4.0.0\"\n",
        );
        let result = resolve_version(None, Some(&path)).unwrap();
        assert_eq!(result, "4.0.0");
    }
}
