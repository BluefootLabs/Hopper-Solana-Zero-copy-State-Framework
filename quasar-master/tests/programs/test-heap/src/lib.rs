#![no_std]
#![allow(dead_code)]

use quasar_lang::prelude::*;

mod instructions;
use instructions::*;
pub mod events;
declare_id!("33333333333333333333333333333333333333333333");

#[program]
mod quasar_test_heap {
    use super::*;

    /// Non-heap instruction: pure computation, no allocation.
    #[instruction(discriminator = 0)]
    pub fn no_heap_ok(ctx: Ctx<NoHeapOk>) -> Result<(), ProgramError> {
        ctx.accounts.handler()
    }

    /// Heap-enabled instruction: uses alloc.
    #[instruction(discriminator = 1, heap)]
    pub fn heap_vec_ok(ctx: Ctx<HeapVecOk>) -> Result<(), ProgramError> {
        ctx.accounts.handler()
    }

    /// Non-heap instruction that attempts allocation.
    /// In release builds (no debug feature): cursor is set past end of heap,
    /// alloc returns null, handle_alloc_error triggers panic_handler, program
    /// aborts. In debug builds: cursor is initialized normally, alloc
    /// succeeds.
    #[instruction(discriminator = 2)]
    pub fn no_heap_alloc_attempt(ctx: Ctx<NoHeapAllocAttempt>) -> Result<(), ProgramError> {
        ctx.accounts.handler()
    }

    /// Event emission in a program with any_heap=true.
    /// Verifies that __handle_event dispatch works when the entrypoint
    /// skips heap cursor initialization (deferred to per-arm init).
    #[instruction(discriminator = 3)]
    pub fn emit_event_ok(ctx: Ctx<EmitEventOk>) -> Result<(), ProgramError> {
        ctx.accounts.handler()
    }
}
