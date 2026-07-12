//! `hopper compile --emit manifest --package <name>` — generate
//! `hopper.manifest.json` FROM SOURCE.
//!
//! Every other `--emit` target consumes an existing manifest JSON; this
//! is the producer. It compiles a tiny generated harness that depends
//! on the target package, prints
//! `hopper_schema::codama::ManifestJson(&<pkg>::PROGRAM_MANIFEST)`, and
//! writes the result next to the package as `hopper.manifest.json` —
//! so the published schema is rendered from the SAME statics the
//! runtime enforces (`SCHEMA_METADATA`, `STRICT_WRITES`,
//! `WRITE_RANGES`, ...) and cannot drift from the code. This is the
//! same build-a-harness trick `anchor idl build` uses, without the
//! Node/JS leg.
//!
//! The target package must export a `pub static PROGRAM_MANIFEST:
//! hopper_schema::ProgramManifest` (see `examples/hopper-counter` for
//! the hand-assembled shape; `#[program]` is slated to emit it
//! automatically, at which point every Hopper program gets this for
//! free).

use std::path::{Path, PathBuf};
use std::process::Command;

/// `my-program` → `my_program` (the crate ident Cargo exposes).
fn crate_ident(package: &str) -> String {
    package.replace('-', "_")
}

/// Render the scratch harness's Cargo.toml. The empty `[workspace]`
/// table detaches it from any enclosing workspace; both path deps
/// resolve to the caller's actual source trees, so the harness compiles
/// against exactly the code being described.
fn scratch_cargo_toml(package: &str, package_dir: &Path, hopper_lang_dir: &Path) -> String {
    format!(
        "[package]\n\
         name = \"hopper-manifest-export\"\n\
         version = \"0.0.0\"\n\
         edition = \"2021\"\n\
         publish = false\n\
         \n\
         [dependencies]\n\
         {package} = {{ path = {pkg_path:?} }}\n\
         hopper = {{ path = {hopper_path:?}, package = \"hopper-lang\", default-features = false }}\n\
         \n\
         [workspace]\n",
        package = package,
        pkg_path = package_dir.display().to_string(),
        hopper_path = hopper_lang_dir.display().to_string(),
    )
}

/// Render the scratch harness's main.rs.
fn scratch_main_rs(package: &str) -> String {
    format!(
        "fn main() {{\n    print!(\"{{}}\", hopper::hopper_schema::codama::ManifestJson(&{ident}::PROGRAM_MANIFEST));\n}}\n",
        ident = crate_ident(package)
    )
}

/// Locate a package's directory and the hopper-lang framework root via
/// `cargo metadata` (resolved, so path/rename indirection is handled).
fn locate_dirs(package: &str, cwd: &Path) -> Result<(PathBuf, PathBuf), String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cargo metadata produced invalid JSON: {e}"))?;
    let packages = meta
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or("cargo metadata: no packages array")?;
    let dir_of = |name: &str| -> Option<PathBuf> {
        packages.iter().find_map(|p| {
            if p.get("name").and_then(serde_json::Value::as_str) == Some(name) {
                p.get("manifest_path")
                    .and_then(serde_json::Value::as_str)
                    .map(|m| PathBuf::from(m).parent().map(Path::to_path_buf))
                    .flatten()
            } else {
                None
            }
        })
    };
    let pkg_dir = dir_of(package).ok_or_else(|| {
        format!("package `{package}` not found in this workspace (cargo metadata)")
    })?;
    let hopper_dir = dir_of("hopper-lang").ok_or(
        "hopper-lang not found in the dependency graph; --emit manifest must run inside a \
         workspace whose programs depend on the Hopper framework",
    )?;
    Ok((pkg_dir, hopper_dir))
}

/// Generate `hopper.manifest.json` from the package's exported
/// `PROGRAM_MANIFEST`. Returns the path written.
pub fn emit_manifest_from_source(
    package: &str,
    out: Option<&Path>,
    cwd: &Path,
) -> Result<PathBuf, String> {
    let (pkg_dir, hopper_dir) = locate_dirs(package, cwd)?;

    let scratch = std::env::temp_dir().join(format!("hopper-manifest-export-{package}"));
    let src = scratch.join("src");
    std::fs::create_dir_all(&src).map_err(|e| format!("scratch dir: {e}"))?;
    std::fs::write(
        scratch.join("Cargo.toml"),
        scratch_cargo_toml(package, &pkg_dir, &hopper_dir),
    )
    .map_err(|e| format!("scratch Cargo.toml: {e}"))?;
    std::fs::write(src.join("main.rs"), scratch_main_rs(package))
        .map_err(|e| format!("scratch main.rs: {e}"))?;

    eprintln!("compiling manifest harness for `{package}` (first run builds the deps)...");
    let output = Command::new("cargo")
        .args(["run", "--quiet"])
        .arg("--manifest-path")
        .arg(scratch.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("could not run the manifest harness: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("PROGRAM_MANIFEST") {
            return Err(format!(
                "`{package}` does not export `pub static PROGRAM_MANIFEST: \
                 hopper_schema::ProgramManifest`. Add one (see \
                 examples/hopper-counter/src/lib.rs for the shape built from the \
                 macro-generated consts) and re-run.\n\nharness build output:\n{stderr}"
            ));
        }
        return Err(format!("manifest harness failed to build/run:\n{stderr}"));
    }
    let json = String::from_utf8(output.stdout)
        .map_err(|e| format!("harness printed non-UTF8 output: {e}"))?;
    if !json.trim_start().starts_with('{') {
        return Err(format!(
            "harness output does not look like a JSON manifest:\n{}",
            &json[..json.len().min(400)]
        ));
    }

    let out_path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| pkg_dir.join("hopper.manifest.json"));
    std::fs::write(&out_path, &json).map_err(|e| format!("writing {}: {e}", out_path.display()))?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_conversion_swaps_hyphens() {
        assert_eq!(crate_ident("hopper-counter"), "hopper_counter");
        assert_eq!(crate_ident("plain"), "plain");
    }

    #[test]
    fn scratch_harness_files_reference_the_package_and_printer() {
        let toml = scratch_cargo_toml(
            "hopper-counter",
            Path::new("D:/x/examples/hopper-counter"),
            Path::new("D:/x"),
        );
        assert!(toml.contains("hopper-counter = { path ="));
        assert!(toml.contains("package = \"hopper-lang\""));
        assert!(
            toml.contains("[workspace]"),
            "must detach from enclosing workspaces"
        );

        let main = scratch_main_rs("hopper-counter");
        assert!(main.contains("hopper_counter::PROGRAM_MANIFEST"));
        assert!(main.contains("ManifestJson"));
    }
}
