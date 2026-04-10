//! Raw escape hatch for Hopper Native.
//!
//! Direct access to syscalls, unchecked CPI, and memory primitives.
//! Only use in audited paths where the higher-level APIs are insufficient.

#[allow(unused_imports)]
pub use crate::syscalls::*;
pub use crate::mem::*;

#[cfg(feature = "cpi")]
pub use crate::cpi::{invoke_unchecked, invoke_signed_unchecked};
