//! # Hopper Manager
//!
//! Schema-driven inspector library for Hopper programs.
//!
//! The manager is an **inspector, not an engine**. It consumes the canonical
//! runtime/layout/schema truth published by Hopper programs and returns
//! human-readable reports. It never invents its own semantics, every byte,
//! offset, and label comes from `hopper_schema::ProgramManifest` or the raw
//! account bytes themselves.
//!
//! ## Design
//!
//! This crate exposes **pure functions** that take a `ProgramManifest` plus
//! some input and return a `String` or a structured report. They do no I/O,
//! no argv parsing, no RPC calls, and no `process::exit`. Those concerns
//! belong to the caller (typically `hopper-cli` or a custom tool).
//!
//! ## Modules
//!
//! - [`inspect`], identify accounts, decode headers and fields
//! - [`summary`], render layouts, policies, events, fingerprint tables
//! - [`analyze`], compatibility verdicts, semantic diffs, migration plans
//!
//! ## Example
//!
//! ```ignore
//! use hopper_manager as mgr;
//!
//! let report = mgr::inspect::identify(&manifest, &raw_bytes)?;
//! println!("{}", report);
//! ```

pub mod analyze;
pub mod inspect;
pub mod summary;

pub use analyze::{compatibility_report, field_diff_report};
pub use inspect::{decode_account, header_report, identify_account, segment_map_report};
pub use summary::{
    events_report, fingerprints_report, instruction_report, layouts_report, policies_report,
    program_summary,
};

/// One-stop human-readable overview of a program manifest.
///
/// Equivalent to the CLI's `hopper manager summary`, the default
/// `Display` impl on `ProgramManifest`. Exposed here so downstream tools
/// can get the same formatting without reaching into schema internals.
#[inline]
pub fn overview(manifest: &hopper_schema::ProgramManifest) -> String {
    program_summary(manifest)
}
