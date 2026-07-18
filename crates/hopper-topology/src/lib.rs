//! Hopper Loom: deterministic, profile-guided account-topology analysis.
//!
//! Loom is deliberately host-only and framework-neutral at its JSON boundary.
//! It models the scheduler conflicts caused by Solana account locks, explores
//! explicit vertical, horizontal, and hybrid placements, and emits a
//! reproducible Pareto frontier plus a narrowly scoped certificate.
//!
//! It does **not** claim that byte-disjoint writes inside one account execute
//! concurrently, predict validator throughput, rewrite program semantics, or
//! prove a migration correct. See the crate README for the complete boundary.

mod commitment;
mod model;
mod solver;

pub use commitment::{commit_profile, sha256_hex};
pub use model::*;
pub use solver::{route_bucket, solve, TopologyError};
