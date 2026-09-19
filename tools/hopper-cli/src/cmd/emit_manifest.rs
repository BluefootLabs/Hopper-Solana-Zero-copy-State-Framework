//! `hopper compile --emit manifest --package <name>`, generate
//! `hopper.manifest.json` FROM SOURCE.
//!
//! Every other `--emit` target consumes an existing manifest JSON; this
//! is the producer. It runs the manifest printer test that
//! `hopper::program_manifest!` emits into the program crate
//! (`cargo test --lib -- __hopper_print_manifest --nocapture`), reads the
//! JSON printed between two marker lines, and writes it next to the
//! package as `hopper.manifest.json`. The published schema is therefore
//! rendered from the SAME statics the runtime enforces
//! (`SCHEMA_METADATA`, `STRICT_WRITES`, `WRITE_RANGES`, ...) and cannot
//! drift from the code. This is the trick `anchor idl build` uses. A test
//! binary compiles the crate source directly, so it works for a program
//! crate whose `crate-type` is `["cdylib"]` alone; an earlier scratch
//! harness that linked the crate as a library could not.
//!
//! The target package must invoke `hopper::program_manifest! { program =
//! <mod>, layouts = [...], events = [...] }` once at the crate root (see
//! `examples/hopper-counter`): `#[program]` / `#[account]` /
//! `#[hopper::event]` emit all the deep metadata, so the block only names
//! the module and the layout/event types.

use std::path::{Path, PathBuf};
use std::process::Command;

use hopper_schema::codama::{MANIFEST_EXPORT_BEGIN, MANIFEST_EXPORT_END};

/// Test filter passed to the program crate's test binary. Substring
/// matching keeps it correct whether the macro was invoked at the crate
/// root or inside a module.
const PRINTER_TEST_FILTER: &str = "__hopper_print_manifest";

/// Locate a package's directory via `cargo metadata` (resolved, so
/// path/rename indirection is handled).
fn locate_package_dir(package: &str, cwd: &Path) -> Result<PathBuf, String> {
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
    packages
        .iter()
        .find_map(|p| {
            if p.get("name").and_then(serde_json::Value::as_str) == Some(package) {
                p.get("manifest_path")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|m| PathBuf::from(m).parent().map(Path::to_path_buf))
            } else {
                None
            }
        })
        .ok_or_else(|| format!("package `{package}` not found in this workspace (cargo metadata)"))
}

/// Pull the manifest JSON out of the test binary's stdout. `None` when the
/// markers are absent, which means the crate has no printer test (no
/// `program_manifest!` invocation) or the filter matched nothing.
fn extract_manifest_json(stdout: &str) -> Option<String> {
    let begin = stdout.find(MANIFEST_EXPORT_BEGIN)? + MANIFEST_EXPORT_BEGIN.len();
    let end = begin + stdout[begin..].find(MANIFEST_EXPORT_END)?;
    Some(stdout[begin..end].trim().to_string())
}

/// Generate `hopper.manifest.json` from the package's `program_manifest!`
/// block. Returns the path written.
pub fn emit_manifest_from_source(
    package: &str,
    out: Option<&Path>,
    cwd: &Path,
) -> Result<PathBuf, String> {
    let pkg_dir = locate_package_dir(package, cwd)?;

    eprintln!(
        "running the manifest printer for `{package}` (cargo test --lib; the first run builds \
         the package)..."
    );
    let output = Command::new("cargo")
        .args(["test", "--quiet", "--lib", "--manifest-path"])
        .arg(pkg_dir.join("Cargo.toml"))
        .args(["--", PRINTER_TEST_FILTER, "--nocapture"])
        .output()
        .map_err(|e| format!("could not run cargo test for the manifest printer: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "the manifest printer for `{package}` failed to build or run:\n{stderr}"
        ));
    }
    let json = extract_manifest_json(&stdout).ok_or_else(|| {
        format!(
            "`{package}` printed no manifest. Add one block at the crate root, \
             `hopper::program_manifest! {{ program = <your_program_mod>, layouts = [...], \
             events = [...] }}` (see examples/hopper-counter/src/lib.rs), and re-run.\n\n\
             cargo test output:\n{}\n{}",
            stdout.trim(),
            stderr.trim()
        )
    })?;
    if !json.starts_with('{') || !json.ends_with('}') {
        return Err(format!(
            "the manifest printer output does not look like a JSON manifest:\n{}",
            &json[..json.len().min(400)]
        ));
    }
    let mut json = json;
    json.push('\n');

    let out_path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| pkg_dir.join("hopper.manifest.json"));
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(&out_path, &json).map_err(|e| format!("writing {}: {e}", out_path.display()))?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_json_is_read_back_between_the_markers() {
        let stdout = format!(
            "\nrunning 1 test\n\n{MANIFEST_EXPORT_BEGIN}\n{{\n  \"name\": \"p\"\n}}\n\
             {MANIFEST_EXPORT_END}\ntest __hopper_manifest_export::__hopper_print_manifest ... ok\n"
        );
        assert_eq!(
            extract_manifest_json(&stdout).as_deref(),
            Some("{\n  \"name\": \"p\"\n}")
        );
        assert_eq!(extract_manifest_json("running 0 tests\n"), None);
        assert_eq!(
            extract_manifest_json(&format!("{MANIFEST_EXPORT_BEGIN}\n{{ truncated")),
            None
        );
    }
}
