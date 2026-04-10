#[cfg(feature = "hopper-native-backend")]
mod native;
#[cfg(feature = "pinocchio-backend")]
mod pinocchio;
#[cfg(feature = "solana-program-backend")]
mod solana_program;

#[cfg(feature = "hopper-native-backend")]
#[doc(hidden)]
pub use native::{
    BackendAccountView,
    BackendAddress,
    BackendProgramResult,
    bridge_to_runtime,
    process_entrypoint,
};

#[cfg(feature = "pinocchio-backend")]
#[doc(hidden)]
pub use pinocchio::{
    BackendAccountView,
    BackendAddress,
    BackendProgramResult,
    bridge_to_runtime,
    process_entrypoint,
};

#[cfg(feature = "solana-program-backend")]
#[doc(hidden)]
pub use solana_program::{
    BackendAccountView,
    BackendAddress,
    BackendProgramResult,
    bridge_to_runtime,
    process_entrypoint,
};

#[cfg(feature = "hopper-native-backend")]
pub(crate) use native::*;
#[cfg(feature = "pinocchio-backend")]
pub(crate) use pinocchio::*;
#[cfg(feature = "solana-program-backend")]
pub(crate) use solana_program::*;
