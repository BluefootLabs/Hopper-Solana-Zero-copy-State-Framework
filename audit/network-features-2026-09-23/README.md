# Finalized feature-account observations

Public RPC observations captured on 2026-09-23. Each `*-raw.json` retains the
ordered requested keys and one finalized `getMultipleAccounts` response.
The corresponding cluster JSON records genesis, runtime version, source pin,
observation slot and interpreted feature state. `SHA256SUMS` covers all JSON.

Feature keys come from Agave `c17c5962f7e96fe72d37fefb2b6702b7447746a3`.
Active records require Feature-program ownership and the encoded activation
slot. Mainnet's 0460 address is System-owned and is recorded as unproven.
Neither its balance nor its presence proves activation.

These are endpoint observations, not authenticated ledger proofs, and report
only the captured slots. They do not establish future activation or runtime
behavior for a later deployment. No transaction was sent and no SOL was spent
to collect them. See the [source review](../../docs/SOURCE_REVIEW_2026-09-23.md)
for the feature matrix and engineering consequences.
