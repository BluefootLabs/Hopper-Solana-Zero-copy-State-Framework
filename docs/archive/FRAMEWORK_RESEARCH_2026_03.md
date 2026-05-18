# Zero-Copy Framework Research: Pinocchio, Star Frame, Quasar

> **Research date**: 2026-03-29  
> **Hopper baseline**: hopper-core v0.1 (pinocchio 0.10, no_std, no proc macros)  
> **Sources**: Full source code inspection of each framework  

> This document is a dated research snapshot captured before Hopper Native
> became the default public runtime surface. For current Hopper architecture,
> use `README.md`, `HOPPER_LANG.md`, `THE_HOPPER_MODEL.md`, and `ARCHITECTURE.md`.

---

## 1. Pinocchio (febo/pinocchio)

### 1.1 Account Validation Ergonomics

Pinocchio deliberately provides **zero validation framework**. It gives you the raw primitives and nothing more. This is a conscious design choice -- it's an SDK, not a framework.

**What Hopper already does better**: Hopper's validation system (tiered loading, check graph, fast u32 header compare) is already far ahead of what pinocchio offers natively.

**One pattern to note**: Pinocchio's `AccountView` method naming is extremely clean:

```rust
account.is_signer()      // bool
account.is_writable()    // bool
account.executable()     // bool
account.owned_by(&addr)  // bool
account.is_data_empty()  // bool
```

This boolean-return style lets callers compose with `&&` chains in a single `if` statement:

```rust
if !account.is_signer() || !account.is_writable() || !account.owned_by(program_id) {
    return Err(ProgramError::InvalidAccountData);
}
```

**Gap in Hopper**: Hopper's `check_*` functions return `ProgramResult`, which is good for error specificity but slightly less composable than booleans for complex multi-check predicates. Not necessarily a weakness -- just different.

### 1.2 Zero-Copy Overlay Safety

Pinocchio v0.10/v0.11 uses `solana-account-view` which wraps `*mut RuntimeAccount`. Key safety patterns:

1. **Borrow tracking via `borrow_state` byte**: The first byte of `RuntimeAccount` is reused for borrow tracking. `0xFF` = unborrowed, other values = duplicate index. `Ref`/`RefMut` guards enforce single-mutable-borrow.

2. **`Ref::map()` / `Ref::filter_map()` projections**: These are Pinocchio's equivalent of cell projection. You can narrow a `Ref<[u8]>` to a `Ref<T>` without re-borrowing:

    ```rust
    let data = account.try_borrow()?;  // Ref<[u8]>
    let vault = Ref::map(data, |d| &d[16..]);  // Ref<[u8]> (narrowed)
    ```

    **Actionable**: Hopper's `VerifiedAccount<T>` could benefit from a similar projection API -- currently the verified wrapper is monolithic. A `VerifiedAccount::map_field()` that projects to a sub-field while maintaining verification provenance would be useful.

3. **`account-resize` feature**: When enabled, stores original `data_len` in the padding bytes at parse time. All resizes are checked against `MAX_PERMITTED_DATA_INCREASE`. Costs +2 CU per account at entrypoint.

    **Actionable**: Hopper's `safe_realloc` could adopt the same +2 CU approach -- store original len at entry, validate resize bounds. Currently Hopper has `safe_realloc` but doesn't track cumulative delta.

### 1.3 CPI Patterns and CU Minimization

Pinocchio's CPI is **the gold standard** for minimal overhead:

**Struct-based CPI builders** (from `pinocchio-system`, `pinocchio-token`):
```rust
Transfer {
    from: payer,
    to: dest,
    lamports: amount,
}
.invoke_signed(&[signer_seeds])
```

Key optimized patterns:
- `InstructionAccount` has factory methods: `InstructionAccount::writable_signer(addr)`, `InstructionAccount::writable(addr)` -- single constructor per flag combination
- Instruction data is a fixed-size `[u8; N]` array, manually packed on the stack
- `invoke_signed()` accepts `&[Signer]` (Signer = `&[Seed]`, Seed = `&[u8]`)
- All CPI data is stack-allocated, zero heap
- `invoke_unchecked()` skips borrow validation for ~30 CU savings per CPI when caller knows borrows are safe

**How Hopper compares**: `HopperCpi<ACCTS, DATA>` is already const-generic and stack-allocated. But Pinocchio's struct-based approach is more ergonomic (named fields vs. chained `.add_account()` calls). 

**Actionable pattern**: Add struct-based CPI helpers alongside the generic builder. For common operations (system transfer, token transfer, etc.), struct syntax is cleaner:
```rust
// Instead of chained builder:
HopperCpi::<3, 9>::new(&token_program_id)
    .add_account(source, true, false)
    .add_account(dest, true, false)
    .add_account(authority, false, true)
    .set_data(&[3, ...])
    .invoke()?;

// Could offer:
hopper_solana::Transfer { from: source, to: dest, amount }.invoke()?;
```

### 1.4 Tooling / CLI

Pinocchio ships **zero tooling**. No CLI, no IDL, no scaffolding. This is deliberate -- it's a lightweight SDK layer.

**Actionable**: Not applicable -- there's nothing to borrow here.

---

## 2. Star Frame (star_frame)

### 2.1 "Designed Together" Coherence

Star Frame achieves coherence through an **elegant 4-phase lifecycle** with typed arguments at each phase:

```
Decode → Validate → Run → Cleanup
```

Each phase is a separate trait with its own generic argument type:
```rust
pub trait StarFrameInstruction: BorshDeserialize + InstructionArgs {
    type ReturnType: NoUninit;
    type Accounts<'decode, 'arg>: AccountSetDecode<'decode, Self::DecodeArg<'arg>>
        + AccountSetValidate<Self::ValidateArg<'arg>>
        + AccountSetCleanup<Self::CleanupArg<'arg>>;

    fn process(
        accounts: &mut Self::Accounts<'_, '_>,
        run_arg: Self::RunArg<'_>,
        ctx: &mut Context,
    ) -> Result<Self::ReturnType>;
}
```

The key innovation is `InstructionArgs::split_to_args()` -- a single instruction data struct gets decomposed into different typed inputs for each phase. This means:
- Decode phase gets decode-specific data (e.g., which accounts are optional)
- Validate phase gets validation parameters (e.g., constraint bounds)
- Run phase gets business logic data
- Cleanup phase gets cleanup configuration (e.g., who receives closed account rent)

**How Hopper compares**: Hopper has `Frame` with phased execution (Resolve → Validate → Borrow → Mutate → Emit), but the phases aren't typed. Any `Frame` method can be called at any time -- the phases are documented convention, not enforced.

**Actionable for Hopper**: The `PhasedFrame → ResolvedFrame → ValidatedFrame` type-state pattern in Hopper's `frame::phase` module already does this partially. Star Frame's key insight is that the instruction data itself should be decomposable per-phase with `split_to_args()`. Hopper could add:

```rust
pub trait InstructionData: Sized {
    type ValidateArgs<'a>;
    type ExecuteArgs<'a>;
    fn split(&mut self) -> (Self::ValidateArgs<'_>, Self::ExecuteArgs<'_>);
}
```

### 2.2 Typed Lifecycle / Phase Patterns

**The Modifier Stack**: Star Frame's real innovation is composable **type-level modifiers** that layer behavior:

```rust
// Composable: Init + Seeded + Signer + Mut -- all stack
Init<Seeded<Mut<Signer<Account<Vault>>>>>
```

Each modifier adds one capability:
- `Signer<T>` -- validates signer, provides `signer_seeds()`
- `Mut<T>` -- validates writable
- `Seeded<T, S, P>` -- validates PDA derivation, caches bump
- `Init<T>` -- handles account creation via CPI
- `ValidatedAccount<T>` -- adds custom validation hook

The derive macro `#[derive(AccountSet)]` generates the decode/validate/cleanup impls automatically. Each modifier responds to its specific validation argument:

```rust
#[validate(arg = Seeds<MySeedStruct>)]
#[cleanup(arg = CloseAccount<&recipient>)]
```

**Actionable for Hopper**: Hopper already has `VerifiedAccount<T>` and `VerifiedAccountMut<T>`, but they're flat -- not composable. A modifier-stack approach would let users compose:
```rust
type AuthorityVault = PdaVerified<WritableVerified<Account<Vault>>>;
```
where each wrapper adds its validation at the type level.

### 2.3 KeyFor<T> -- Typed Pubkey References

Star Frame's `KeyFor<T>` is a `#[repr(transparent)]` wrapper over `Pubkey` parameterized by account type:

```rust
pub struct KeyFor<T: ?Sized> {
    pubkey: Pubkey,
    _phantom: PhantomData<fn() -> T>,
}
```

This prevents you from accidentally storing a vault key where a mint key is expected -- the compiler catches it:
```rust
vault.mint_key = some_vault_key;  // ERROR: KeyFor<Vault> != KeyFor<Mint>
```

**Actionable for Hopper**: This is simple but powerful. Add a `TypedAddress<T>` newtype over `[u8; 32]`:
```rust
#[repr(transparent)]
pub struct TypedAddress<T: ?Sized> {
    bytes: [u8; 32],
    _phantom: PhantomData<fn() -> T>,
}
```
Wire-compatible with `Address`/`[u8; 32]` for zero-copy layouts.

### 2.4 UnitVal<T, Unit> -- Compile-Time Unit System

Star Frame ships a full compile-time dimensional analysis system:

```rust
let meters: UnitVal<f64, Meters> = UnitVal::new(4.0);
let seconds: UnitVal<f64, Seconds> = UnitVal::new(2.0);
let speed = meters / seconds;  // UnitVal<f64, MetersPerSecond>
let invalid = speed + seconds;  // COMPILE ERROR
```

This is over-engineered for most Solana programs, but the core idea -- **newtype wrappers that prevent misuse of numeric values** -- is highly relevant for DeFi. Token amounts, lamports, basis points, timestamps should all be different types.

**Actionable for Hopper**: In `hopper-core::math`, add lightweight typed amount wrappers:
```rust
pub struct Lamports(WireU64);
pub struct TokenAmount(WireU64);
pub struct BasisPoints(WireU16);
```
Not a full unit system, but prevents the most common arithmetic bugs.

### 2.5 Context Sysvar Caching

Star Frame's `Context` caches `Rent` and `Clock` using `Cell<Option<T>>`:
```rust
pub fn get_rent(&self) -> Result<Rent> {
    match self.rent_cache.get() {
        None => {
            let new_rent = Rent::get()?;
            self.rent_cache.set(Some(new_rent));
            Ok(new_rent)
        }
        Some(rent) => Ok(rent),
    }
}
```

This deduplicates sysvar reads across validation, execution, and cleanup phases.

**Actionable**: Hopper already has `CachedClock`, `CachedRent`, `SysvarContext` in its sysvar module. Verify these are used consistently across the Frame lifecycle -- Star Frame's pattern proves caching matters.

### 2.6 IDL / CLI

Star Frame has:
- A full **Codama-based IDL generation** system (`star_frame_idl` crate)
- A minimal CLI (`star_frame new` for project scaffolding)

The IDL system generates machine-readable program interfaces from the account and instruction types. It uses the `TypeToIdl`, `AccountSetToIdl`, `InstructionToIdl` traits.

**Current Hopper status**: Hopper has schema export through `hopper-schema` and `hopper-cli`, including Codama and Anchor-shaped outputs. The source-backed gap is narrower now: Anchor IDL export still leaves the `errors` array empty.

### 2.7 Weaknesses

- **Uses `std`**: Star Frame requires `std`, `Box<dyn>`, `Vec`, `borsh`. Not suitable for no_std/minimal environments.
- **Heavy proc macros**: `StarFrameProgram`, `InstructionSet`, `AccountSet`, `InstructionArgs`, `GetSeeds` etc. -- extensive proc macro surface.
- **Not CU-optimized**: Uses borsh deserialization, heap allocation, `Vec`-based seed construction. The CPI builder uses `Vec::with_capacity` for signers.

---

## 3. Quasar (blueshift-gg/quasar)

### 3.1 BUMP_OFFSET Optimization

**The implementation** (from `derive/src/account/fixed.rs`):

```rust
// In the #[account] proc macro:
let has_bump_u8 = fields_data.iter().any(|f| {
    f.ident.as_ref().is_some_and(|id| id == "bump")
        && matches!(&f.ty, syn::Type::Path(tp) if tp.path.is_ident("u8"))
});

let bump_offset_impl = if has_bump_u8 {
    quote! {
        const BUMP_OFFSET: Option<usize> = Some(
            #disc_len + core::mem::offset_of!(#zc_mod::#zc_name, bump)
        );
    }
} else {
    quote! {}
};
```

Then in `Discriminator` trait:
```rust
pub trait Discriminator {
    const DISCRIMINATOR: &'static [u8];
    const BUMP_OFFSET: Option<usize> = None;  // Default: no bump
}
```

During account parsing with `seeds`, the generated code reads the bump from account data at `BUMP_OFFSET` and calls `verify_program_address` (~200 CU) instead of `based_try_find_program_address` (~544 CU).

**How Hopper compares**: Hopper already has `BUMP_OFFSET` (inspired by Quasar) in `hopper_layout!`. The implementation is nearly identical -- const field scanning for a `bump` field, with `verify_pda_cached()` using the offset. ✅ **Already adopted**.

### 3.2 Batched Header Validation

Quasar reads the first 4 bytes of `RuntimeAccount` as a single u32 and compares against precomputed constants:

```rust
// In __internal:
pub const NODUP: u32 = 0xFF;
pub const NODUP_SIGNER: u32 = 0xFF | (1 << 8);
pub const NODUP_MUT: u32 = 0xFF | (1 << 16);
pub const NODUP_MUT_SIGNER: u32 = 0xFF | (1 << 8) | (1 << 16);
pub const NODUP_EXECUTABLE: u32 = 0xFF | (1 << 24);
```

The generated parse code does:
```rust
let actual_header = unsafe { *(raw_ptr as *const u32) };
if actual_header != EXPECTED_HEADER {
    return Err(decode_header_error(actual_header, EXPECTED_HEADER));
}
```

The `decode_header_error` function is `#[cold] #[inline(never)]` -- error decomposition only runs on the error path, keeping the hot path to a single compare.

**How Hopper compares**: Hopper already has this in `check::fast` module with identical constants (`HEADER_SIGNER`, `HEADER_WRITABLE`, etc.) and `check_account_fast()`. The implementation matches Quasar's approach. ✅ **Already adopted**.

### 3.3 Custom PDA Derivation via Raw Syscalls

Quasar's `based_try_find_program_address()` bypasses `sol_create_program_address` and calls `sol_sha256` + `sol_curve_validate_point` directly:

```rust
// ~544 CU per attempt (vs ~1500 CU for sol_try_find_program_address)
// For a typical PDA (bump 255): ~544 CU 
sol_sha256(input, len, hash_ptr);
let on_curve = sol_curve_validate_point(CURVE25519_EDWARDS, hash_ptr, null_mut());
if on_curve != 0 { /* valid PDA */ }
```

Key optimizations:
- Bump byte is a mutable pointer, only 1 byte changes per iteration
- `MaybeUninit<[&[u8]; 19]>` for the hash input array -- no zeroing
- u64 loop counter avoids per-iteration zero-extension on SBF
- `read_unaligned` for hash pointer manipulation

`verify_program_address()` does just the sha256 + comparison (~200 CU):
```rust
sol_sha256(input_with_bump, len, hash_ptr);
if keys_eq(&hash_as_address, expected) { Ok(()) } else { Err(InvalidSeeds) }
```

**How Hopper compares**: Hopper uses `Address::create_program_address()` for verification (~200 CU) which calls the `sol_create_program_address` syscall. This is already efficient for the verify path. For the find path, Hopper uses `Address::find_program_address()` which calls `sol_try_find_program_address` (~1500 CU).

**Actionable**: Add a `find_pda_fast()` function using Quasar's raw `sol_sha256` + `sol_curve_validate_point` approach for the ~544 CU find path. Most useful during initialization when bump isn't known yet. The verify path is already optimal.

### 3.4 keys_eq -- Optimized Address Comparison

```rust
pub fn keys_eq(a: &Address, b: &Address) -> bool {
    let a = a.as_array().as_ptr() as *const u64;
    let b = b.as_array().as_ptr() as *const u64;
    unsafe {
        core::ptr::read_unaligned(a) == core::ptr::read_unaligned(b)
            && core::ptr::read_unaligned(a.add(1)) == core::ptr::read_unaligned(b.add(1))
            && core::ptr::read_unaligned(a.add(2)) == core::ptr::read_unaligned(b.add(2))
            && core::ptr::read_unaligned(a.add(3)) == core::ptr::read_unaligned(b.add(3))
    }
}
```

Short-circuits on first mismatch. 4x u64 unaligned reads.

`is_system_program()` uses OR-fold (half the loads):
```rust
(read(a) | read(a+1) | read(a+2) | read(a+3)) == 0
```

**Actionable**: Add `keys_eq_fast()` and `is_zero_address()` to Hopper's check module. Currently Hopper uses `operator==` on `Address` which may not be short-circuiting:

```rust
// In hopper-core check module:
#[inline(always)]
pub fn keys_eq_fast(a: &Address, b: &Address) -> bool {
    let a = a.as_ref().as_ptr() as *const u64;
    let b = b.as_ref().as_ptr() as *const u64;
    unsafe {
        core::ptr::read_unaligned(a) == core::ptr::read_unaligned(b)
            && core::ptr::read_unaligned(a.add(1)) == core::ptr::read_unaligned(b.add(1))
            && core::ptr::read_unaligned(a.add(2)) == core::ptr::read_unaligned(b.add(2))
            && core::ptr::read_unaligned(a.add(3)) == core::ptr::read_unaligned(b.add(3))
    }
}
```

### 3.5 CPI Flag Extraction via Batched Read

Quasar's `cpi_account_from_view()` reads the 4-byte RuntimeAccount header, shifts right 8 to drop `borrow_state`, and writes the flags directly into the `CpiAccount` struct via `RawCpiBuilder` transmute:

```rust
let flags = (raw as *const u32).read_unaligned() >> 8;
let builder = RawCpiBuilder {
    address: &(*raw).address,
    lamports: &(*raw).lamports,
    data_len: (*raw).data_len,
    data: (raw as *const u8).add(RUNTIME_ACCOUNT_SIZE),
    owner: &(*raw).owner,
    rent_epoch: 0,
    flags: flags as u64,
};
core::mem::transmute(builder)
```

Compile-time assertions verify layout compatibility:
```rust
const _: () = assert!(size_of::<RawCpiBuilder>() == 56);
const _: () = assert!(size_of::<RawCpiBuilder>() == size_of::<CpiAccount>());
const _: () = assert!(offset_of!(RuntimeAccount, borrow_state) == 0);
const _: () = assert!(offset_of!(RuntimeAccount, is_signer) == 1);
```

**Actionable**: Hopper's `HopperCpi` could adopt this pattern for building `CpiAccount` arrays. Currently it does per-field assignment -- the batched extract + transmute saves a few instructions per account.

### 3.6 dispatch! Macro -- Direct SVM Buffer Parsing

Quasar's `dispatch!` macro parses accounts directly from the raw SVM input buffer pointer, never going through pinocchio's higher-level parsing:

```rust
dispatch!($ptr, $ix_data, 4, {
    [1, 0, 0, 0] => make(MakeAccounts),
    [2, 0, 0, 0] => take(TakeAccounts),
});
```

This:
1. Reads account count as `u64` at offset 0
2. Skips to first account entry at offset 8
3. Matches discriminator bytes (fixed `[u8; N]` array comparison)
4. Creates a `MaybeUninit<[AccountView; COUNT]>` buffer
5. Calls generated `parse_accounts()` which walks the raw buffer

The generated `parse_accounts()` function does per-account:
- Read `borrow_state` byte to detect duplicates
- If non-duplicate: read 4-byte header as u32, compare against expected
- Construct `AccountView` via `AccountView::new_unchecked(raw_ptr)`
- Advance pointer past account data + 10KiB realloc region + alignment

**How Hopper compares**: Hopper Native owns the entrypoint and already exposes `hopper_fast_entrypoint!` plus `hopper_lazy_entrypoint!`. The proc-macro `#[hopper::program]` path still dispatches through `ProgramResult`, so a raw-`u64` generated dispatch function remains a possible micro-optimization, but the low-level entrypoint surface is no longer just Pinocchio's standard eager path.

**Actionable for Hopper -- but with caution**: The entrypoint-level optimization is interesting but requires unsafe raw pointer walking. For Hopper, a more pragmatic approach would be a `lazy_dispatch!` macro that uses pinocchio's `lazy_program_entrypoint!` (available in pinocchio v0.11) to parse accounts on demand. This gets most of the CU savings without the raw pointer complexity.

### 3.7 RemainingAccounts -- Zero-Allocation Iterator

Quasar's `RemainingAccounts` uses a boundary pointer instead of a count:
```rust
pub struct RemainingAccounts<'a> {
    ptr: *mut u8,
    boundary: *const u8,
    declared: &'a [AccountView],
}
```

It lazily walks the SVM buffer, resolving duplicates by index into the declared accounts slice. No heap allocation, no upfront parsing.

**Current Hopper status**: `#[hopper::context]` now emits `ctx.remaining_accounts()` for strict duplicate-rejecting access, `ctx.remaining_accounts_passthrough()` for duplicate-preserving access, and `ctx.remaining_accounts_raw()` for the underlying slice. The runtime view is zero-allocation over the already-parsed `AccountView` tail and adds bounded helpers such as `signers::<N>()?` for multisig-style remaining accounts.

### 3.8 Interface<T> + ProgramInterface

```rust
pub trait ProgramInterface {
    fn matches(address: &Address) -> bool;
}

pub struct Interface<T: ProgramInterface> {
    view: AccountView,
    _marker: PhantomData<T>,
}
```

Used for Token vs Token-2022 compatible instructions.

**How Hopper compares**: Hopper has `check_owner_multi()` which iterates a slice of valid owners. Quasar's approach is compile-time via a trait impl -- slightly more efficient but less flexible.

### 3.9 Dynamic Fields with Offset Caching

For accounts with `String<P, N>` or `Vec<T, P, N>` fields, Quasar generates:
- Fixed fields first (enforced at compile time)
- Dynamic fields after, with inline length prefixes
- An `__off: [u32; N-1]` offset cache computed once at parse time
- `RawEncoded<PREFIX_BYTES>` for zero-copy CPI pass-through

**Current Hopper status**: Hopper now exposes `#[hopper::dynamic_account]` for
the compact Quasar-style case. It lowers inline `#[tail(string<N>)]` and
`#[tail(vec<Address, N>)]` fields into the fixed-body + `[u32 len][payload]`
tail model and generates borrowed views plus an owned editor. Offset caching is
still relevant for future indexed or segmented tail policies, where repeated
random access would otherwise require walking earlier fields.

### 3.10 CLI Toolchain

Quasar has a full CLI:
- `quasar init` -- scaffold project
- `quasar add` -- add instructions, state, errors
- `quasar build` -- compile on-chain program
- `quasar test` -- run test suite
- `quasar deploy` -- deploy to cluster
- `quasar idl` -- generate IDL
- `quasar profile` -- measure CU usage
- `quasar dump` -- dump sBPF assembly
- `quasar keys` -- manage program keypairs
- `quasar config` -- global settings

**Actionable**: `quasar profile` and `quasar dump` are the most interesting for Hopper. A CU profiling tool that measures per-instruction compute usage, and an sBPF disassembly tool for optimization analysis. These are framework-differentiating features.

---

## Summary: Actionable Patterns for Hopper

### Already Adopted ✅
1. **BUMP_OFFSET** (from Quasar) -- compile-time bump field detection, ~344 CU savings
2. **Batched u32 header validation** (from Quasar) -- single-compare account flags
3. **Multi-owner validation** (`check_owner_multi`) -- Token/Token-2022 interface pattern
4. **Stack-allocated const-generic CPI** -- `HopperCpi<A, D>`
5. **Compact dynamic account facade** -- `#[hopper::dynamic_account]` for bounded string and address-list tails
6. **Fast PDA find/verify** -- `find_and_verify_pda` uses the fast Hopper Native PDA path when a bump is not known; `verify_pda_cached` stays the cheapest stored-bump path
7. **Fast address comparisons** -- `keys_eq_fast()` and `is_zero_address()` are already in `hopper_core::check`
8. **Remaining accounts modes** -- generated strict/passthrough/raw accessors plus bounded signer parsing
9. **Typed Instructions sysvar view** -- `InstructionsSysvar` and `IntrospectedInstruction` wrap the raw parser helpers
10. **CLI profiling and dump surface** -- `hopper profile bench`, `hopper profile elf`, and `hopper dump`
11. **IDL/schema export** -- `hopper-schema` and `hopper-cli` emit Hopper, Codama, and Anchor-shaped schema/IDL outputs; Anchor IDL error messages are still not emitted

### High-Value Additions 🔴
12. **CPI flag extraction via batched read** -- `cpi_account_from_view()` pattern for HopperCpi
13. **Struct-based CPI helpers** -- named-field syntax for common operations (pinocchio pattern)
14. **Raw generated dispatch** -- a `process_instruction_raw` codegen path that returns `u64` directly for hot SBF entrypoints

### Medium-Value Additions 🟡
15. **InstructionData decomposition** -- `split_to_args()` pattern for phase-specific instruction data (Star Frame)
16. **Sysvar cache verification** -- ensure cached sysvars are used consistently through lifecycle helpers
17. **Offset caching for indexed/segmented dynamic views** -- compute all segment offsets in one pass (Quasar's `__off: [u32; N-1]`)
18. **Lock-file / upgrade-safety lint** -- compare generated manifests and lockfiles before deploy

### Tooling 🔧
19. **Profile artifact polish** -- make the current profiling outputs easier to publish and compare in PRs
20. **Anchor IDL error messages** -- source has `ErrorRegistry`, but the Anchor IDL exporters still write `errors: []`

### Explicitly NOT Worth Adopting ❌
- **Star Frame's `std` dependency** -- Hopper should stay `no_std`
- **Star Frame's proc macro weight** -- Hopper's declarative macros are a feature
- **Star Frame's borsh dependency** -- zero-copy means zero serialization
- **Star Frame's UnitVal dimension system** -- over-engineered for Solana (but lightweight typed amounts ARE worth adding)
- **Quasar's whole proc-macro framework model** -- Hopper keeps proc macros optional and lowers them into explicit runtime primitives
- **Quasar's entire framework model** -- Hopper should remain composable primitives, not a monolithic framework

---

## Framework Character Comparison

| Dimension | Pinocchio | Star Frame | Quasar | Hopper |
|-----------|-----------|------------|--------|--------|
| **Philosophy** | Raw SDK | Typed lifecycle | Anchor-comparable DX | Zero-copy primitives |
| **Proc macros** | None | Heavy | Heavy | Optional |
| **std required** | No | Yes | No | No |
| **CU optimization** | Extreme | Moderate | Extreme | Extreme |
| **Validation** | None | Phase-typed | Attribute-driven | Graph + fast checks |
| **Schema safety** | None | None | Discriminator | Layout fingerprint |
| **Dynamic fields** | None | UnsizedType | String/Vec with offset cache | `dynamic_account` + explicit tails/segments |
| **CLI** | None | Minimal | Full toolchain | Hopper CLI |
| **IDL** | None | Codama-based | Built-in | Hopper schema + Codama/Anchor export |

Hopper's differentiation: **layout fingerprinting + deterministic ABI + composable validation graph + optional proc macros over explicit primitives**. The main remaining gaps are profile artifact polish, raw generated dispatch, richer indexed dynamic-field DX, and Anchor IDL error-message export.
