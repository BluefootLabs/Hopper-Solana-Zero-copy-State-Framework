# hopper-memo

Hopper-owned CPI helper for the SPL Memo program.

The SPL Memo program (`MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`) records
arbitrary UTF-8 byte payloads in transaction logs and asserts that a list of
accounts have signed. It is the canonical primitive for on-chain
metadata stamping, orderbook IDs, off-chain reference numbers, audit notes, and
arbitrary protocol tags without spinning up program-owned state.

## Quick start

```rust,ignore
use hopper_memo::Memo;

// `user` is the signing account's `&AccountView`.
Memo {
    signers: &[user],
    memo: b"order=42",
    program_id: None,
}
.invoke()?;
```

For PDA-signed memos, pass the seed list to `invoke_signed`:

```rust,ignore
use hopper::cpi::{Seed, Signer};

let bump_seed = [bump];
let seeds = [Seed::from(b"vault"), Seed::from(&bump_seed)];

Memo {
    signers: &[vault_pda],
    memo: b"deposit",
    program_id: None,
}
.invoke_signed(&[Signer::from(&seeds)])?;
```

A single invocation accepts at most `MAX_MEMO_SIGNERS` (16) signer accounts;
more returns `ProgramError::InvalidArgument`.

## Programs

| Program | Address |
|---|---|
| Memo v2 (default) | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` |
| Memo v1 (legacy) | `Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo` |

`Memo` defaults to v2; pass `program_id: Some(&hopper_memo::v1::MEMO_V1_PROGRAM_ID)`
to invoke v1.

## Compatibility

The API covers Memo v1 and v2 and can be used alongside other Hopper SPL CPI
builders.

Docs: <https://docs.rs/crate/hopper-memo>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
