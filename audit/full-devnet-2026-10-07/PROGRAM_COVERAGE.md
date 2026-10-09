# Program execution coverage

All 52 workspace `cdylib` packages built with fresh targets for SBF v0 and v3. Current devnet suites exercised 29 packages. The borrowed-argument program has the identical ELF to its earlier October 7 live run. The remaining 22 have no current devnet execution in this archive. A package row covers the named tests, not every handler or feature combination.

| Package | Compiled VM matrix | Devnet evidence |
| --- | --- | --- |
| `counter-hopper` | Outside this matrix | [Current named suite](live/suite-framework/receipt.json) |
| `counter-hopper-substrate` | Outside this matrix | [Current named suite](live/suite-framework/receipt.json) |
| `hello-hopper` | Outside this matrix | [Current named suite](live/suite-framework/receipt.json) |
| `hello-hopper-substrate` | Outside this matrix | [Current named suite](live/suite-framework/receipt.json) |
| `hopper-argus-guard` | Outside this matrix | Not executed here |
| `hopper-borrowed-args-fixture` | Executed | [Earlier identical ELF](../borrowed-dispatch-2026-10-07/devnet-manual/summary.json) |
| `hopper-bounded-multisig` | Executed | [Current named suite](live/suite-governance/receipt.json) |
| `hopper-byte-allowance` | Executed | [Current named suite](live/suite-byte-allowance-retry/receipt.json) |
| `hopper-canonical-pda-fixture` | Executed | Not executed here |
| `hopper-checked-cpi-fixture` | Executed | [Current named suite](live/suite-governance/receipt.json) |
| `hopper-cicada` | Outside this matrix | Not executed here |
| `hopper-cicada-canonical-route-fixture` | Outside this matrix | Not executed here |
| `hopper-cicada-route-fixture` | Outside this matrix | Not executed here |
| `hopper-compact-vault` | Outside this matrix | [Current named suite](deployments/hopper-compact-vault/receipt.json) |
| `hopper-confidential-lab` | Outside this matrix | [Current named suite](live/suite-confidential/receipt.json) |
| `hopper-counter` | Outside this matrix | Not executed here |
| `hopper-devnet-audit` | Outside this matrix | [Current named suite](deployments/hopper-devnet-audit/receipt.json) |
| `hopper-escrow` | Executed | [Current named suite](live/suite-token-escrow/receipt.json) |
| `hopper-external-oracle` | Outside this matrix | Not executed here |
| `hopper-function-lab` | Executed | [Current named suite](live/devnet-hopper-function-lab/summary.json) |
| `hopper-migration` | Outside this matrix | [Current named suite](deployments/hopper-migration/receipt.json) |
| `hopper-mint-plan-fixture` | Executed | [Current named suite](live/suite-mint-plan/receipt.json) |
| `hopper-native-lifecycle-fixture` | Executed | [Current named suite](live/suite-lifecycle/receipt.json) |
| `hopper-nft-mint` | Outside this matrix | Not executed here |
| `hopper-orderbook` | Executed | [Current named suite](deployments/hopper-orderbook/receipt.json) |
| `hopper-parity-vault` | Outside this matrix | Not executed here |
| `hopper-pda-boundaries-fixture` | Executed | Not executed here |
| `hopper-policy-vault` | Outside this matrix | Not executed here |
| `hopper-proc-vault` | Outside this matrix | Not executed here |
| `hopper-registry` | Outside this matrix | Not executed here |
| `hopper-return-provenance-fixture` | Executed | [Current named suite](live/suite-return-provenance/receipt.json) |
| `hopper-router` | Outside this matrix | Not executed here |
| `hopper-runtime-gate-fixture` | Executed | [Current named suite](live/suite-runtime-gate-retry/receipt.json) |
| `hopper-runtime-lab` | Outside this matrix | [Current named suite](live/suite-runtime-lab/receipt.json) |
| `hopper-runtime-lifecycle-fixture` | Executed | [Current named suite](live/suite-lifecycle/receipt.json) |
| `hopper-sentinel` | Outside this matrix | Not executed here |
| `hopper-showcase` | Outside this matrix | Not executed here |
| `hopper-sibling-introspection-fixture` | Executed | [Current named suite](live/suite-sibling/receipt.json) |
| `hopper-smoke` | Outside this matrix | Not executed here |
| `hopper-stablecoin-memo-pay` | Outside this matrix | Not executed here |
| `hopper-styx-ferry` | Outside this matrix | Not executed here |
| `hopper-tail-lab` | Outside this matrix | [Current named suite](live/suite-tail-lab/receipt.json) |
| `hopper-token-2022-ata` | Outside this matrix | Not executed here |
| `hopper-token-2022-transfer-hook` | Outside this matrix | Not executed here |
| `hopper-token-2022-vault` | Outside this matrix | [Current named suite](deployments/hopper-token-2022-vault/receipt.json) |
| `hopper-token-lab` | Outside this matrix | [Current named suite](live/suite-token-lab/receipt.json) |
| `hopper-token-outcomes-fixture` | Executed | [Current named suite](live/suite-token-outcomes/receipt.json) |
| `hopper-treasury` | Executed | [Current named suite](live/suite-governance/receipt.json) |
| `hopper-vault` | Executed | [Current named suite](live/suite-named-vault/receipt.json) |
| `hopper-virtual-state` | Outside this matrix | Not executed here |
| `hopper-xp-program-a` | Outside this matrix | [Current named suite](live/suite-cross-program/receipt.json) |
| `hopper-xp-program-b` | Outside this matrix | [Current named suite](live/suite-cross-program/receipt.json) |

The counter tutorial has no initialization handler; the external-oracle example expects an externally initialized account owned by its configured program. Build success does not establish a complete application lifecycle. Other unexecuted examples require their own account fixtures and semantic assertions.

Function-lab checks passed against independently computed values, but the two cluster consistency findings remain failures. The optional modular-exponentiation call was not executed on devnet.

`coverage.json` binds each row to the default v3 ELF hash and lists every selected live receipt. Diagnostic failures, interrupted attempts, and their recovery records remain separately archived; they are excluded from the completed-suite transaction count.
