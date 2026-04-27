# hopper-memo

Hopper-owned CPI helper for the SPL Memo program.

The SPL Memo program (`MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`) records
arbitrary UTF-8 byte payloads in transaction logs and asserts that a list of
accounts have signed. It is the canonical primitive for on-chain
metadata stamping (orderbook IDs, off-chain reference numbers, audit notes,
arbitrary protocol tags) without spinning up program-owned state.

## Quick start

```rust,ignore
use hopper_memo::Memo;

Memo {
    signers: &[user.account_view()],
    memo: b"order=42",
}
.invoke()?;
```

For PDA-signed memos, pass the seed list to `invoke_signed`:

```rust,ignore
Memo {
    signers: &[vault_pda.account_view()],
    memo: b"deposit",
}
.invoke_signed(&[Signer::from(&[b"vault", &[bump]][..])])?;
```

## Programs

| Program | Address |
|---|---|
| Memo v2 (default) | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` |
| Memo v1 (legacy) | `Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo` |

`Memo` defaults to v2; pass `program_id: Some(&hopper_memo::v1::MEMO_V1_PROGRAM_ID)`
to invoke v1.

## Compatibility

Pinocchio parity: `pinocchio-memo`. Quasar omits a memo helper.
