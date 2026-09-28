# sbpf-linker on the Hopper comparison fixtures, 2026-09-28

Status: evaluated, **not adopted**. Every artifact the linker produced from
Hopper's fixtures loads and then faults at run time. `cargo build-sbf` stays
the only build path, and Hopper's own sBPFv3 counter is smaller than the
linker's anyway.

## Why it was worth measuring

Pina reported blueshift's `sbpf-linker` 0.2.1 at 15,512 bytes against
25,032 for `cargo build-sbf --lto` on its counter, 38% smaller, verified in
Mollusk on the `sbpf-solana-solana` target, and silently broken on
`bpfel-unknown-none`. A 38% size cut would be worth a second build path.

## What was run

`scripts/eval-sbpf-linker.sh` inside `rust:bookworm` (the linker ships only
Linux and macOS binaries):

- Agave v3.1.6 release tarball, which bootstraps platform-tools v1.52 and
  the `1.89.0-sbpf-solana-v1.52` toolchain;
- `sbpf-linker` v0.2.2, `x86_64-unknown-linux-musl` release binary;
- the two macro fixtures from `bench/framework-comparison/programs/`, built
  three ways with the bench's release profile (`lto = fat` where stated,
  `codegen-units = 1`, `opt-level = 3`, `overflow-checks = false`):
  `cargo build-sbf --lto`, and `cargo build --release --target
  sbpf-solana-solana` with `-C linker=sbpf-linker -C linker-flavor=ld
  -C linker-plugin-lto -C panic=abort -C relocation-model=static`, with
  fat LTO and without.

rustc's GNU-ld code path passes the linker flags its command line rejects
(`--flavor`, `--version-script`, `--no-undefined-version`, `--threads`,
`-z`, `--as-needed`, `-Bstatic`). The script puts a shim in front of the
linker that keeps the inputs, the output, and the library paths, exports
`entrypoint` explicitly, and drops the rest. Without the shim the link does
not start.

Each artifact then went through the same Mollusk verifier the comparison
bench uses (`bench/framework-comparison/verifier`): hello must log
`Hello, Solana!`; the counter must initialize its PDA and increment it.

## Result

| Fixture | Build | Bytes | sBPF | Verifier |
|---|---|---:|---|---|
| hello (macro) | `cargo build-sbf --lto`, platform-tools v1.52 | 1,776 | v0 | pass |
| hello (macro) | `cargo build-sbf --lto --arch v3`, v1.54 (bench row) | 1,072 | v3 | pass |
| hello (macro) | sbpf-linker | 984 | v3 | **fails**: access violation reading 1 byte at `0x100` |
| hello (macro) | sbpf-linker, fat LTO | 840 | v3 | **fails**: same fault |
| counter (macro) | `cargo build-sbf --lto`, platform-tools v1.52 | 8,688 | v0 | pass |
| counter (macro) | `cargo build-sbf --lto --arch v3`, v1.54 (bench row) | 7,352 | v3 | pass |
| counter (macro) | sbpf-linker | 7,944 | v3 | **fails**: access violation reading 32 bytes at `0x0` |
| counter (macro) | sbpf-linker, fat LTO | 7,944 | v3 | **fails**: same fault |

The faults are reads of constants (the log string, a 32-byte address) at
addresses that hold nothing: the read-only data is not where the code
expects it. That is the failure Pina documented for `bpfel-unknown-none`,
reproduced here from `sbpf-solana-solana` objects. The loader accepts all
four files, so a build-success check or a deploy would not have caught it;
the functional gate did.

## Reading

- The sizes of the linker's artifacts are not comparable to anything: a
  program that cannot read its constants is small partly because of what
  is missing or misplaced.
- Where the comparison is fair, the standard toolchain already wins on the
  larger program: Hopper's sBPFv3 counter from `cargo build-sbf --arch v3`
  is 7,352 bytes, 592 fewer than the linker's 7,944.
- The linker is built for upstream-BPF objects from a nightly `rustc`
  (`cargo +nightly build-bpf` in its template), where `target_os` is not
  `solana`. Hopper's syscall and entrypoint paths are gated on
  `target_os = "solana"`, so that route would compile different code, the
  defect Pina found on the same target.

## Decision

Keep `cargo build-sbf` (and `--arch v3` ahead of SIMD-0500) as the only
build path. Re-evaluate when the linker accepts rustc's link line for the
Solana target without a shim and a fixture passes the verifier; the script
makes that a one-command check. Any alternative build path stays behind
the rule this evaluation exercised: an artifact counts only after the
Mollusk verifier has run it.
