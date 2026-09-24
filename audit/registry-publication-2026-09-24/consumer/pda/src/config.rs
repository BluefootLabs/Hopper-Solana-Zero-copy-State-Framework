//! Shared wire layout for the SBF program and its host-side fixture builder.
#[derive(Clone, Copy)]
#[hopper::state(disc = 1, version = 1)]
#[repr(C)]
pub struct Config {
    #[bump]
    pub value: u8,
}
