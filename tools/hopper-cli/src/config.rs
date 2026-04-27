//! Project + global configuration for the `hopper` CLI.
//!
//! Two files, two scopes:
//!
//! - **`Hopper.toml`** (per-project, lives next to `Cargo.toml`) — declares
//!   the toolchain choice, testing framework, default backend, and the
//!   template the project was scaffolded from. Read by every `hopper`
//!   subcommand that needs to know "how should I build / test / deploy
//!   this project".
//! - **`~/.hopper/config.toml`** (global) — holds the wizard's last-used
//!   defaults so that `hopper init my-program` skips the prompts after
//!   the first interactive run, plus UI preferences (color, animation).
//!
//! Both files are TOML, both round-trip via serde, both have explicit
//! defaults. Either file being missing or malformed degrades gracefully:
//! we fall back to defaults rather than refusing to run, so a fresh
//! checkout of someone else's Hopper project is usable on first
//! invocation without setup.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Project config — Hopper.toml
// ---------------------------------------------------------------------------

/// Per-project configuration. Lives at `<project>/Hopper.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HopperToml {
    pub project: ProjectSection,
    #[serde(default)]
    pub toolchain: ToolchainSection,
    #[serde(default)]
    pub testing: TestingSection,
    #[serde(default)]
    pub backend: BackendSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSection {
    pub name: String,
    /// Free-form template name the project was scaffolded from. Not
    /// load-bearing for build/test; surfaced by `hopper doctor` so
    /// developers can tell at a glance what they started from.
    #[serde(default = "default_template")]
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainSection {
    /// `solana` (cargo build-sbf) or `upstream` (cargo +nightly build-bpf).
    /// Mirrors Quasar's split for porting parity.
    #[serde(default = "default_toolchain_kind")]
    pub kind: String,
}

impl Default for ToolchainSection {
    fn default() -> Self {
        Self {
            kind: default_toolchain_kind(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestingSection {
    /// `mollusk` (default) | `quasarsvm` | `solana-test-validator` | `none`.
    #[serde(default = "default_testing_framework")]
    pub framework: String,
}

impl Default for TestingSection {
    fn default() -> Self {
        Self {
            framework: default_testing_framework(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendSection {
    /// `hopper-native` (default) | `pinocchio` | `solana-program`.
    /// Forwarded into the cargo build feature flag.
    #[serde(default = "default_backend")]
    pub default: String,
}

impl Default for BackendSection {
    fn default() -> Self {
        Self {
            default: default_backend(),
        }
    }
}

fn default_template() -> String {
    "minimal".to_string()
}
fn default_toolchain_kind() -> String {
    "solana".to_string()
}
fn default_testing_framework() -> String {
    "mollusk".to_string()
}
fn default_backend() -> String {
    "hopper-native".to_string()
}

impl HopperToml {
    /// Construct a default-shaped config for a freshly scaffolded project.
    pub fn new(project_name: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            project: ProjectSection {
                name: project_name.into(),
                template: template.into(),
            },
            toolchain: ToolchainSection::default(),
            testing: TestingSection::default(),
            backend: BackendSection::default(),
        }
    }

    /// Load the project config from `<dir>/Hopper.toml`. Returns `Ok(None)`
    /// if the file doesn't exist; that's a soft signal that the project
    /// pre-dates the config-file convention. Callers should fall back to
    /// defaults rather than erroring out.
    pub fn load(project_dir: &Path) -> Result<Option<Self>, String> {
        let path = project_dir.join("Hopper.toml");
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let parsed: HopperToml = toml::from_str(&raw)
            .map_err(|err| format!("malformed {}: {err}", path.display()))?;
        Ok(Some(parsed))
    }

    /// Persist this config to `<dir>/Hopper.toml`. Creates the directory
    /// chain if missing.
    pub fn save(&self, project_dir: &Path) -> Result<(), String> {
        if !project_dir.exists() {
            fs::create_dir_all(project_dir).map_err(|err| {
                format!(
                    "failed to create project directory {}: {err}",
                    project_dir.display()
                )
            })?;
        }
        let path = project_dir.join("Hopper.toml");
        let serialised = toml::to_string_pretty(self)
            .map_err(|err| format!("failed to serialise Hopper.toml: {err}"))?;
        // Header comment makes it obvious to a human reader that this is
        // hopper's config and not something cargo wrote.
        let body = format!(
            "# Hopper project configuration. Generated by `hopper init`.\n# Edit by hand or via `hopper config set <key> <value>`.\n# Schema: https://hopperzero.dev/docs/hopper-toml\n\n{serialised}"
        );
        fs::write(&path, body)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))
    }
}

// ---------------------------------------------------------------------------
// Global config — ~/.hopper/config.toml
// ---------------------------------------------------------------------------

/// Cross-project user defaults. The wizard reads these to pre-fill its
/// prompts; subsequent prompts overwrite the file so the next run gets
/// faster every time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub defaults: GlobalDefaults,
    #[serde(default)]
    pub ui: GlobalUi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalDefaults {
    pub toolchain: String,
    pub testing: String,
    pub backend: String,
    pub template: String,
    /// `commit` (default) | `init` | `skip` — what to do with git on
    /// `hopper init`.
    pub git: String,
}

impl Default for GlobalDefaults {
    fn default() -> Self {
        Self {
            toolchain: default_toolchain_kind(),
            testing: default_testing_framework(),
            backend: default_backend(),
            template: default_template(),
            git: "commit".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalUi {
    pub color: bool,
    pub animation: bool,
}

impl Default for GlobalUi {
    fn default() -> Self {
        Self {
            color: true,
            animation: true,
        }
    }
}

impl GlobalConfig {
    /// Resolve the wizard-defaults path. Stored as `wizard.toml` to keep
    /// it distinct from `config.toml`, which the existing
    /// `hopper config` command tree owns for operational keys (cluster
    /// URL, payer keypair, default program ID, etc.). Both files live
    /// side by side in the same hopper config directory.
    pub fn path() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            return home.join(".hopper").join("wizard.toml");
        }
        if let Some(dir) = dirs::config_dir() {
            return dir.join("hopper").join("wizard.toml");
        }
        PathBuf::from(".hopper-wizard.toml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create global config dir {}: {err}",
                    parent.display()
                )
            })?;
        }
        let serialised = toml::to_string_pretty(self)
            .map_err(|err| format!("failed to serialise global config: {err}"))?;
        let body = format!(
            "# Hopper global config. Edit via `hopper config set <key> <value>`.\n# Stored at {}\n\n{serialised}",
            path.display()
        );
        fs::write(&path, body)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hopper_toml_round_trips() {
        let cfg = HopperToml::new("my-program", "nft-mint");
        let s = toml::to_string(&cfg).unwrap();
        let parsed: HopperToml = toml::from_str(&s).unwrap();
        assert_eq!(parsed.project.name, "my-program");
        assert_eq!(parsed.project.template, "nft-mint");
        assert_eq!(parsed.toolchain.kind, "solana");
        assert_eq!(parsed.testing.framework, "mollusk");
        assert_eq!(parsed.backend.default, "hopper-native");
    }

    #[test]
    fn missing_sections_default() {
        // A minimal Hopper.toml with only [project] should still parse,
        // backfilling the rest from defaults.
        let s = r#"
[project]
name = "skinny"
"#;
        let parsed: HopperToml = toml::from_str(s).unwrap();
        assert_eq!(parsed.project.template, "minimal");
        assert_eq!(parsed.toolchain.kind, "solana");
    }

    #[test]
    fn global_config_defaults_are_sane() {
        let cfg = GlobalConfig::default();
        assert_eq!(cfg.defaults.git, "commit");
        assert!(cfg.ui.color);
    }
}
