//! Raw escape hatch for Hopper Native.
//!
//! Direct access to syscalls, unchecked CPI, and memory primitives.
//! Only use in audited paths where the higher-level APIs are insufficient.

pub use crate::mem::*;
#[allow(unused_imports)]
pub use crate::syscalls::*;

#[cfg(feature = "cpi")]
pub use crate::cpi::{invoke_signed_unchecked, invoke_unchecked};
