# Self-audit: what Hopper checks about your program

Most Solana bugs that lose money are boring. A signer check that was
dropped in a refactor. An account that became writable. A bound that went
from 10 to 100 because someone needed it for a test. None of those change
how the program looks from the outside, and none of them fail a build.

Hopper's position is that the framework already knows the answers. The
macros see every account, constraint, seed, byte range, and rule you
declare. So Hopper writes them down, compares them between releases, and
fails the build or the release when something got looser.

This page is the map. Every item names the command or attribute, when it
runs, and what it does not cover.

## At compile time

These fail `cargo build`. No tool to install, no step to remember.

| Check | What it refuses |
|---|---|
| Layout field types | A `bool`, `u64`, `char`, reference, or enum in a zero-copy layout. The error names the wire type to use (`WireBool`, `WireU64`, `EnumByte<E>`, `OptionByte<T>`). |
| Seed lists | A literal seed longer than 32 bytes, or a `seeds = [..]` list with no room left for the bump. Both fail every derivation at run time. |
| Value rules | A `#[check(..)]` rule no value can satisfy, such as `value >= 10 && value < 10`. |
| Error codes | A user error code inside the ranges Hopper reserves for its own refusals (`0xB000..=0xEFFF`). |
| Creation constraints | `init` without `payer` or `space`, `seeds` without `bump`, `realloc` without a payer or a zero policy, `bump = stored` on an account that is being created. |
| Event size | A CPI event larger than the emit buffer. |
| Stack frames | `hopper build` fails when the SBF builder reports a frame over 4,096 bytes. `cargo build-sbf` prints that line and exits 0. |

## At bind, on chain

These run inside your program before the handler does.

- **One account in two mutable roles.** Passing the same account as `from`
  and `to` is refused with `ERR_ALIASED_MUTABLE_ACCOUNTS` unless the
  context declares the alias with `dup`.
- **Surplus accounts.** The count-exact entrypoint refuses a transaction
  that passes more accounts than the instruction declares.
- **Stored values.** A layout with `#[check(..)]` rules has its stored
  values checked for every existing account the context binds. A layout
  without rules costs nothing.
- **Write boundaries.** Under `strict_writes`, a mutable borrow outside the
  declared byte ranges is refused before the borrow is returned.

## The manifest

`hopper::program_manifest!` builds one static from the same constants the
runtime enforces, so the published description cannot drift from the code:

```text
hopper compile --emit manifest --package my-program
```

The manifest carries every instruction with its discriminator, accounts,
signer and writable flags, PDA seeds, `has_one` relations, expected owners
and addresses, lifecycle (init, realloc, close), write ranges, lamport
permissions, layouts with field offsets and fingerprints, and the
`fieldRules` table: each `#[check]` rule as written plus the integer bounds
it decides.

`hopper explain program`, `explain context`, and `explain instruction`
read it back in plain language.

## The upgrade gate

This is the one to put in CI.

```text
grillo authority-diff released.manifest.json candidate.manifest.json
```

It compares what two releases declare and classifies every difference:

| Finding | Meaning | Exit code |
|---|---|---:|
| `WIDENED` | The new release permits something the old one refused | 2 |
| `REVIEW` | A change that cannot be ordered, such as different PDA seeds | 3 |
| `NARROWED` | The new release is stricter | 0 |
| `INFO` | No authority change, for example a rename | 0 |

What counts as a widening: a dropped signer, a new writable account, a
write range that reaches another field, a removed PDA or `has_one`
binding, a new lamport permission, a new instruction, and a value rule
that was removed or loosened (`field_rule_removed`, `field_rule_widened`).

A loosened bound is worth a second look. If `tier <= 10` becomes
`tier <= 100`, account state your old program could never produce is now
accepted by the new one. A tightened rule is reported too, as `NARROWED`,
with a note: accounts whose stored value breaks the new rule stop binding.
That is an availability question you want answered before you deploy.

An intended widening is approved by checking in the reviewed report:

```text
grillo authority-diff old.json new.json --out reviewed.json
grillo authority-diff old.json new.json --approve reviewed.json
```

The approval names both manifest digests, so it cannot be replayed onto a
different pair of releases. `hopper verify --release` with
`--authority-baseline` runs the same gate and binds each manifest to its
ELF.

## After execution

`grillo verify` takes a manifest and an evidence bundle (pre and post
snapshots plus the touch map the program emitted) and recomputes

```text
changed ⊆ acquired ⊆ authorized
```

Bytes that changed must have been acquired through a tracked write, and
bytes that were acquired must be inside the declared ranges. Exit codes:
`0` pass, `2` violation, `3` inconclusive, `1` malformed input.

`hopper tx explain <signature>` decodes a transaction's touch map into
field names.

## Layout changes

```text
hopper compat old-layout.json new-layout.json
hopper diff   old-layout.json new-layout.json
hopper plan   old-layout.json new-layout.json
```

`compat` gives the verdict (identical, wire compatible, append safe,
migration required, incompatible), `diff` lists each field, and `plan`
lists the migration steps with byte counts.

## Source lints

```text
hopper lint                  # account relationship graph and diagnostics
hopper lint zc               # zero-copy footguns in typed contexts
hopper lint --deny-escapes   # raw and unchecked escapes as errors
```

## Auditing Hopper itself

A zero-copy framework is `unsafe` code with a nice API on top. If you are
going to trust it with funds, someone has to read that `unsafe` code, and
the first week of any audit goes to finding it and working out what each
piece claims. Hopper does that week for you and keeps the result current.

```text
python scripts/audit-map.py            # write the map
python scripts/audit-map.py --verify   # the gate
python scripts/audit-map.py --show <id>
```

[audit/UNSAFE_MAP.md](../audit/UNSAFE_MAP.md) lists every `unsafe` block,
function, impl, and trait in the framework outside test code: 851 sites at
the time of writing. For each one `audit/unsafe-map.json` records:

- the file, the line, and the enclosing function (or the macro whose
  expansion carries the site);
- the justification written next to it, and what kind it is: written for
  that site, shared with a neighbour, or the boilerplate sentence that
  says someone looked without saying why the code is sound;
- the tests, proofs, and fuzz targets that call the enclosing function by
  name, or, when none does, the tested function a call chain reaches it
  through, or that the site only compiles for the VM;
- a SHA-256 of the site's code.

`--verify` is a ratchet. It fails when the committed map is stale, when a
site has no reasoning of its own, and when a site that runs on the host is
not reached from any test. A new `unsafe` block lands with its argument
written down and a test that runs it, or the build is red.

Getting there was the audit. The first run found 167 boilerplate sites and
44 with no argument; every one was read and rewritten. The next pass read
the 61 host sites no test reached and wrote a test for each. Both passes
found real defects, all in the changelog: safe readers that looked at
account bytes underneath a live exclusive borrow, a header accessor that
handed out an untracked reference, `project_hopper` reading six bytes
inside the 16-byte header, a segment registry entry that could point at
the header and the lock flags, mint readers that took any nonzero option
tag as present, and a cross-program interface form whose documented usage
could never match.

Sites are named `path::function#n`, not by line number, so an edit
somewhere else in the file does not rename them.

### The review ledger

An audit is a snapshot. The code keeps moving. The ledger ties each
sign-off to the exact code that was read:

```text
python scripts/audit-map.py --sign <id> --reviewer <name> --note <text>
python scripts/audit-map.py --check
```

`--sign` records the reviewer, the date, and the hash of the site.
`--check` lists every signed site whose code has changed since, every
signed site that no longer exists, and how many sites nobody has signed.
It exits 2 when a signed site changed. A release cannot quietly carry a
sign-off for code that was edited after the reviewer read it.

### The public API lock

The unsafe map covers what the code does. The API lock covers what it
promises:

```text
python scripts/api-lock.py                       # write audit/api/<crate>.txt
python scripts/api-lock.py --verify              # the gate
python scripts/api-lock.py --against-published   # the release plan
```

`audit/api/` holds one file per published crate with the signature of
every public item, rendered from rustdoc JSON: functions, fields, variants
with their discriminants, constant values, trait items, impls, and macro
rules. A change to the public surface is a diff in review, and `--verify`
fails CI when the lock and the code disagree.

`--against-published` renders the version on crates.io the same way and
says what the next release has to be called. A changed signature, a
changed constant, a removed item, a new variant on an exhaustive enum, or
a new required trait item is a break; a macro that only gained rules, a
function that became `const`, and new items are not. A crate whose own
signatures name a dependency that breaks needs a new minor version as
well, and the plan follows that through the whole graph. `--strict` fails
when a manifest version is lower than the plan requires.
The next release fails planning when registry metadata cannot be fetched or
decoded. Only an actual registry 404 is treated as an unpublished package.

cargo-semver-checks 0.50 passes the tree that changed
`AccountView::layout_id` from `Option<&[u8; 8]>` to `Option<[u8; 8]>` and
`DataFingerprint::capture` from `Self` to a `Result`: it has no lint for a
changed return type. The lock reports both, and two more the changelog
had missed: `MAX_HASH_SEGMENTS` went from 16 to 20,000, and
`MintExtension`, an enum callers could match exhaustively, gained seven
variants. It names 0.5.0 for every crate that exposes the runtime's types.

## The framework checks itself the same way

- `scripts/check-unsafe-safety-comments.py` fails when an `unsafe` block
  has no `SAFETY` comment next to it.
- `scripts/check-doc-citations.py` fails when the unsafe inventory cites a
  test, file, or proof harness that does not exist.
  [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md) lists every `unsafe` entry
  point with its contract and the test that exercises it.
- Every token builder is compared byte for byte and account for account
  with the canonical `spl-token-2022-interface` constructors.
- The comparison bench fails when a Hopper row's compute units or binary
  size grew against the results committed at HEAD.
- `hopper audit-check` verifies the hashes of the audit evidence, the age
  of each quality-gate attestation, and the list of open blockers.
  The next release also rejects manifests without required evidence or required
  gates, blank or duplicate identifiers, empty gate commands, and malformed
  dates. Failed or stale gates do not count as current. These are manifest
  integrity checks; the command does not execute the declared test commands.
  Schema 2 requires execution receipts for required gates, verifies their
  command, completion date, successful exit code, and log digest, and checks
  that the recorded source inventory still matches the checkout. The inventory
  includes Markdown, which can be a compiler/test input through `include_str!`,
  alongside Rust, configuration, scripts, and diagnostic snapshots. Adding,
  editing, or removing these inputs invalidates the receipt; the recorder also
  rejects a command that changes them while running. Historical `audit/` bundles
  are excluded from this inventory and checked separately as evidence.
- `scripts/verify-evidence.py` checks every evidence bundle under `audit/`
  against its `SHA256SUMS` in the checkout, so a devnet run's receipts,
  transaction logs, and program dumps can be checked from a clone;
  `.gitattributes` stores the bundles byte for byte. Fifteen early bundles
  hashed program dumps they never archived; the script names those files,
  whose hashes stay in their bundles, and fails on any other gap.
- Kani proofs cover the loader-input parser, and five fuzz targets cover
  the parsers and overlays (`fuzz/fuzz_targets`).

## What none of this proves

The gate compares declarations. It does not read bytecode and it does not
prove a handler does what its manifest says. `grillo verify` checks the
evidence it is given and does not authenticate where that evidence came
from. Raw and unchecked access paths are outside the tracked write model,
which is why the lint can turn them into errors.

## Record a reproducible quality gate

Unreleased readiness schema 2 requires schema 1 execution receipts. These bind
a local check to the Rust, TOML, Python,
YAML, JSON, Markdown, compiler-diagnostic snapshots, and root lockfile inputs present
before and after it ran.
Tracked and untracked non-ignored inputs are included; historical `audit/`
artifacts are excluded. Added, removed, or edited inputs invalidate the receipt.
Source digests normalize CRLF to LF so Git's Windows checkout conversion does
not create false drift. Logs and receipts use exact bytes. Text evidence may
explicitly set `normalizeLineEndings: true`; binary evidence never should.
Other input formats and external environment inputs need their own evidence entries.

```sh
python scripts/record-quality-gate.py --out target/fmt.json -- cargo fmt --all -- --check
```

The runner executes the argument vector without a shell, records combined
output in `target/fmt.log`, and refuses to overwrite existing evidence. A source
change during execution produces an unsuccessful receipt. Add the receipt's
relative path and SHA-256 as the gate's `receipt` object. When archiving a run,
copy its log, update the receipt's log path, and hash the finalized receipt.
`audit-check --strict` then verifies the receipt and current inputs without
rerunning its command. Schema 1 remains readable for older dossiers.

These receipts detect drift and inconsistent declarations. They are not signed
CI attestations and cannot authenticate who ran a command. Runtime fixtures,
advisory database revisions, compiler versions, and external services remain
separate provenance requirements. See [release readiness](RELEASE_READINESS.md).

Hopper has not had an independent security audit. These checks shrink
what a reviewer has to read. They do not replace the reviewer.
