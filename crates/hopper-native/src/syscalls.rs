//! Raw Solana syscall declarations.
//!
//! These are the extern "C" functions provided by the Solana BPF runtime.
//! Only available when compiling for `target_os = "solana"`.

#[cfg(target_os = "solana")]
extern "C" {
    /// Log a UTF-8 message.
    pub fn sol_log_(message: *const u8, len: u64);

    /// Log a 64-bit value.
    pub fn sol_log_64_(arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64);

    /// Log the current compute unit consumption.
    pub fn sol_log_compute_units_();

    /// Log structured data segments (for events).
    pub fn sol_log_data(data: *const u8, data_len: u64);

    /// Invoke a cross-program instruction (C ABI).
    pub fn sol_invoke_signed_c(
        instruction_addr: *const u8,
        account_infos_addr: *const u8,
        account_infos_len: u64,
        signers_seeds_addr: *const u8,
        signers_seeds_len: u64,
    ) -> u64;

    /// Create a program-derived address.
    pub fn sol_create_program_address(
        seeds_addr: *const u8,
        seeds_len: u64,
        program_id_addr: *const u8,
        address_addr: *mut u8,
    ) -> u64;

    /// Find a program-derived address with bump seed.
    pub fn sol_try_find_program_address(
        seeds_addr: *const u8,
        seeds_len: u64,
        program_id_addr: *const u8,
        address_addr: *mut u8,
        bump_seed_addr: *mut u8,
    ) -> u64;

    /// SHA-256 hash.
    pub fn sol_sha256(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64;

    /// Validate whether a point lies on the selected curve.
    pub fn sol_curve_validate_point(
        curve_id: u64,
        point_addr: *const u8,
        result_point_addr: *mut u8,
    ) -> u64;

    /// Keccak-256 hash.
    pub fn sol_keccak256(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64;

    /// Set return data for the current instruction.
    pub fn sol_set_return_data(data: *const u8, length: u64);

    /// Get return data from the previous CPI.
    pub fn sol_get_return_data(data: *mut u8, length: u64, program_id: *mut u8) -> u64;

    /// Get the current clock sysvar.
    pub fn sol_get_clock_sysvar(addr: *mut u8) -> u64;

    /// Get the current rent sysvar.
    pub fn sol_get_rent_sysvar(addr: *mut u8) -> u64;

    /// Get epoch schedule sysvar.
    pub fn sol_get_epoch_schedule_sysvar(addr: *mut u8) -> u64;

    /// Abort program execution.
    pub fn sol_panic_(file: *const u8, len: u64, line: u64, column: u64) -> !;

    // ── Memory operations (SVM-optimized) ─────────────────────────

    /// Copy `n` bytes from `src` to `dst` (non-overlapping).
    pub fn sol_memcpy_(dst: *mut u8, src: *const u8, n: u64);

    /// Copy `n` bytes from `src` to `dst` (overlapping safe).
    pub fn sol_memmove_(dst: *mut u8, src: *const u8, n: u64);

    /// Compare `n` bytes. Sets `*result` to <0, 0, or >0.
    pub fn sol_memcmp_(s1: *const u8, s2: *const u8, n: u64, result: *mut i32);

    /// Fill `n` bytes with `c`.
    pub fn sol_memset_(s: *mut u8, c: u8, n: u64);

    // ── Instruction introspection ────────────────────────────────

    /// Get the current instruction stack height.
    pub fn sol_get_stack_height() -> u64;

    /// Get a previously processed sibling instruction.
    pub fn sol_get_processed_sibling_instruction(
        index: u64,
        meta: *mut u8,
        program_id: *mut u8,
        data: *mut u8,
        accounts: *mut u8,
    ) -> u64;

    /// Get the last restart slot sysvar.
    pub fn sol_get_last_restart_slot(addr: *mut u8) -> u64;
}
