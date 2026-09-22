# Runtime policy devnet rerun

Clean source: `ad1e209daacc5fbee5fa1a9da58b6df561d6badd`.
Public devnet program: `BPNYrNXCPJwV3k8txPVYWjbqTswkLxRGF1d2DzcGTHAX`.
Freshly rebuilt ELF SHA-256:
`a55d562d7047cc119be552280ccce6b053a20d4ef98b47525ebbf1f99da93335`.

The ELF is identical to the first deployed fixture. The runner also compared
it to finalized on-chain dumps before and after these transactions. All 12
transactions finalized: two account creations, four successful cases, and
six exact policy refusals. Each refusal preserved the complete observed
account state. The first no-policy write ran after the leaked-guard refusal,
checking that policy state does not persist into the next VM invocation.

`receipt.json` records signatures, slots, CU, errors, and canonical snapshot
hashes. Each `case-*.snapshots.json` includes the exact account JSON captured
before and after the corresponding transaction. To reproduce each hash, use
SHA-256 over UTF-8 `json.dumps(snapshot, sort_keys=True, separators=(",", ":"))`.
`SHA256SUMS` covers all 23 public JSON files. No keys, private diagnostics,
or binary copies are archived.

These are maintainer-generated observations from public finalized RPC, not
independently authenticated pre/post state proofs. The scope is this runtime
fixture, not Cicada's complete lifecycle or a framework-wide security audit.
