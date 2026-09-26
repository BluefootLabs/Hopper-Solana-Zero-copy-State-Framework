# Funded escrow and governance review — September 26, 2026

The escrow source replaces a state-only teaching example with a classic SPL
Token custody lifecycle. Local compiled v0/v3 tests run the actual program and
canonical token ELF, checking complete expected account states, refusal cases,
and rollback after successful transfers and vault closure. Host tests and clippy
cover the escrow and bounded multisig example fixes.

The Squads source review is bounded to the listed files. It does not reproduce
Squads test results or claim feature/performance parity. Duplicate member
configuration and threshold-preserving removal are fixed in Hopper's data
example; that example is not a complete governance executor.

Devnet receipts are added only after finalized verification. No registry
packages are changed by these non-published example and documentation edits.
