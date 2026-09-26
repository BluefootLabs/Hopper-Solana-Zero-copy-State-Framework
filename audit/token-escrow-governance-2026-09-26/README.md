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

## Finalized devnet validation

The September 26, 2026 run finalized **33 transactions** with complete expected
account-state checks: 17 setup transactions and 16 escrow/donation transactions,
including ten expected refusals. The deployed 41,656-byte ELF matched the tested
v0 artifact before and after execution.

Program: `3ZhgEZzjCaoEHVUyYkHKWcYnqf4F1ND4vjRDDbb1JZdT`.
Source: `bd7e2a0f4e49d6b442fe46965feaf0fd8def4b45`.
ELF SHA-256: `9ae4f1aaf919e152d58789438c5b2859ba9f432862e65a64a77579407021751b`.

Make measured 7,548 and 7,873 CU for the two offers;
take with a surplus refund measured 8,731 CU;
cancel measured 4,773 CU. These are this fixture's
measurements, not a competitor comparison.

The first upload exhausted RPC retries before executable deployment. Its buffer
rent was recovered; a subsequent upload used a saved buffer key. Raw deployment
logs and all private keys are excluded from public evidence.

## Website verification

Website commit 4aea1a70494db675ae400180fd7086e208040a98 passed production build, including TypeScript checking.
Local and production HTTP checks each covered 50 pages, with no broken internal
routes or heading targets. Production checked 4367 internal links.
The deployment receipt binds the Vercel production deployment to that commit.
The final full-directory and scoped lint reruns made very little progress and
were stopped without diagnostics. Neither is counted as a passing gate in this
receipt. Build/TypeScript and page checks passed independently. Checks inspect
rendered HTML and content, not browser screenshots.
