# Offline effect verification with Grillo

Grillo recomputes whether an observed state transition stayed inside a declared
mutation contract. It is a host-side verifier in the Hopper workspace, not a
Solana program and not a third-party audit product.

```text
changed bytes ⊆ acquired write bytes ⊆ authorized bytes
```

- **Authorized** comes from the instruction's manifest contract.
- **Acquired** comes from the instruction touch map.
- **Changed** comes from caller-supplied pre/post account snapshots.

That separation matters. Hopper enforces declared ranges before tracked mutable
borrows. Grillo takes the resulting evidence and recomputes containment in a
separate process. It does not trust a precomputed PASS flag.

## What ships

The workspace contains a stable v0.1 command surface and an experimental v0.2
library surface:

| Profile | Current surface | Verdict boundary |
|---|---|---|
| Effect ABI v0.1 | Stable `grillo` CLI plus embeddable Rust core | Data-byte containment and declared lamport mutation over supplied snapshots and a touch map |
| Effect ABI v0.2 | Experimental strict Rust contract, binder, and verifier APIs | Complete account-transition dimensions, deployment/artifact binding, remaining-account grammar, and bounded CPI envelopes |

`grillo-manifest` and `grillo-verifier` 0.1.0 are published on crates.io;
their registry archive checksums were verified on 2026-09-24 UTC. Install the
CLI with its explicit feature:

```sh
cargo install grillo-verifier --version 0.1.0 --features cli --locked
grillo commit hopper.manifest.json
grillo verify hopper.manifest.json bundle.json
```

From a source checkout, use `cargo run -p grillo-verifier --features cli
--bin grillo --` before the same arguments.

`commit` prints the mutation-contract commitment for each instruction. `verify`
parses the manifest and evidence bundle, resolves any argument-parametric ranges,
and returns one of three outcomes:

- `PASS` and exit code `0`: the supplied complete scope satisfies the contract;
- `VIOLATION` and exit code `2`: evidence identifies an untracked write,
  unauthorized acquisition, or unauthorized lamport delta; or
- `INCONCLUSIVE` and exit code `3`: the touch map or claimed scope is incomplete.

Malformed input exits `1`. Partial evidence never becomes a PASS.

## Evidence boundary

The v0.1 command consumes a caller-supplied JSON bundle containing the
instruction name, optional argument payload, touch-map bytes, and pre/post data
for each account in scope. Optional lamport pairs let it check the lamport
dimension when the contract declares that dimension complete.

```json
{
  "instruction": "execute_intent",
  "argumentPayload": "0300",
  "touchMap": "7a0100...",
  "accounts": [
    { "index": 2, "pre": "...", "post": "..." }
  ]
}
```

Grillo currently performs no RPC request, signature verification, deterministic
replay, or producer authentication. It does not prove that a bundle came from a
particular transaction or deployment. A transaction-wide snapshot is also not
an invocation boundary: sibling instructions can write and later reverse bytes.
Describe a PASS as scoped to the exact supplied invocation-entry/exit evidence.

Effect ABI v0.2 can bind a contract to a program ID, loader ID, executable
digest, manifest commitment, concrete accounts, and nested calls. Its frame
provenance remains an untrusted label: binding the fields cryptographically does
not authenticate who observed them or whether they came from the ledger.

## No deployment rent

Grillo has no SBF entrypoint or deployable `cdylib`. It runs off chain, so its
on-chain deployment rent is **0 SOL / not applicable**. Local CPU, storage, RPC,
archive, or replay-provider costs depend on the evidence pipeline around it.
Cicada's program deployment cost is a separate loader-v3 calculation.

## The next useful step

Ledger-bound Grillo should accept a transaction signature and construct trusted
invocation evidence through deterministic replay, a Geyser/archive sidecar, or
an account-history provider. Standard transaction metadata does not contain
arbitrary invocation-entry/exit account bytes, so an RPC signature alone is not
enough.

For programs without an instrumented touch map, the strongest defensible check
is `changed ⊆ authorized`. It is weaker than Hopper's three-set check, but it can
extend effect checking to programs that publish a compatible third-party
contract.

## Upgrade and proof integrations

The shipped `hopper verify --authority-baseline` gate compares manifest
declarations: signer and writable roles, strict write ranges, parametric cells,
lamport permissions, remaining-account limits, and represented account
constraints including PDA and CPI-program bindings. It classifies changes as
widened, narrowed, review, or informational; it does not inspect handler bytecode
or infer behavior omitted from the manifest.

```sh
hopper verify --release candidate.manifest.json candidate.so \
  --authority-baseline released.manifest.json \
  --baseline-so released.so \
  --authority-report authority-diff.json
```

Release mode binds the candidate and baseline manifests to their respective
ELF interface commitments. Deployed baseline programs and candidate loader
buffers/programs are also supported. An unapproved widening exits `2`; an
unordered change needing review exits `3`. An approval report must match the
exact manifest pair and listed findings. These are declaration-review results,
not authenticated execution replay or a proof of bytecode equivalence.

The same contract can also drive formal proof adapters.
[QEDGen/qedsvm](https://github.com/QEDGen/qedsvm/blob/99bd5ede85374adc7fc5c835c2432ecf4e123fd1/docs/COVERAGE.md)
can prove frame properties for selected compiled paths, while
[CVLR](https://docs.certora.com/en/latest/docs/solana/usage.html) can discharge
user-written Solana rules. Those engines have explicit coverage boundaries:
selected paths, loop settings, CPI handling, and supported syscalls matter. A
future Hopper proof certificate should therefore publish which handlers and
paths were covered, the verifier version and settings, and every unsupported
case. Runtime authority, runtime evidence, and formal path proofs are
complementary; none should be presented as the other.

Program Metadata currently standardizes records such as IDL and security text;
it does not document a standard effect record. Hopper already publishes the full manifest through `hopper publish-manifest`
under the custom `hopper-manifest` seed. This is a Hopper convention, not a
reserved standard effect namespace. Verify its interface commitment against
the intended executable; publication alone does not authenticate runtime evidence.

## Network boundary

Solana schedules transactions with whole-account locks. Grillo verifies mutation
semantics after execution; Hopper byte ranges do not create sub-account runtime
parallelism or a fee discount.

Read the exact experimental specifications in
[Effect ABI v0.1](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/EFFECT_ABI_V0_1.md)
and
[Effect ABI v0.2](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/EFFECT_ABI_V0_2.md).

## Evidence shape checks, 2026-09-22

Supply exactly one pre/post snapshot pair per positional account index.
The v0.1 JSON bundle verifier rejects repeated indices as malformed input,
including identical copies. The typed library returns INCONCLUSIVE for that
shape, so conflicting snapshots cannot receive a scoped PASS and duplicates
cannot inflate changed-byte counts. Changed ranges are returned in account
and offset order regardless of snapshot input order.

This validates evidence structure. It does not authenticate who supplied the
snapshots or bind them to a ledger transaction. The next integration target is
an invocation-level replay producer with artifact identity and complete
entry/exit snapshots, followed by the experimental v0.2 verifier.
