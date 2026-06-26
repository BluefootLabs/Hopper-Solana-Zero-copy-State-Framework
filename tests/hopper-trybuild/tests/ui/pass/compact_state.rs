use hopper::prelude::*;

// A compact layout: `[disc:u8][body]`, no 16-byte universal header.
#[derive(Clone, Copy)]
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Vault {
    pub authority: [u8; 32],
    pub balance: WireU64,
}

fn main() {
    // Compile-time-only checks on the macro-emitted compact consts.
    const _: usize = Vault::BODY_SIZE; // 40
    const _: usize = Vault::COMPACT_LEN; // 41
    const _: usize = Vault::MIN_SIZE; // 41
    const _: usize = Vault::INIT_SPACE; // 41
    const _: u8 = Vault::DISC; // 1
    // Absolute offsets fold in the single discriminator byte, not HEADER_LEN.
    const _: u32 = Vault::AUTHORITY_ABS_OFFSET; // 1
    const _: u32 = Vault::BALANCE_ABS_OFFSET; // 33

    // The macro also emits a registry-row builder for the Tier-2 manifest.
    let _entry = Vault::registry_entry();

    // The unified one-source-of-truth descriptor: the loader, the registry
    // row, and the offsets all read from `LayoutDescriptor::DESCRIPTOR`.
    use hopper::manifest::LayoutDescriptor;
    const _: u8 = <Vault as LayoutDescriptor>::DESCRIPTOR.disc; // 1
    let _entry2 = <Vault as LayoutDescriptor>::registry_entry();
}
