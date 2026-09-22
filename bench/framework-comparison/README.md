# Framework comparison

Hopper rows for pina's cross-framework benchmark
(`benchmarks/framework-comparison` in https://github.com/pina-rs/pina), built
and measured under pina's own rules so the numbers are comparable with the
table pina publishes for Pina, Pinocchio, Quasar, and Anchor v2.

- `programs/hello/{hopper,hopper-substrate}`: the hello-world fixture, on the
  `#[program]` macro path and on the raw substrate entrypoint.
- `programs/counter/{hopper,hopper-substrate}`: the PDA counter fixture
  (`b"counter" + authority`, `initialize` takes the bump, `increment` re-derives
  the PDA and does a checked add). The substrate row uses a compact 10-byte
  account, byte-for-byte pina's layout; the macro row carries Hopper's 16-byte
  header (25-byte account) and says so.
- `verifier/`: a like-for-like port of pina's Mollusk verifier. Same command
  line, same account states, same assertions, one run per instruction.
- `reference/`: pina's pinocchio fixtures, vendored so the verifier can be
  proven against pina's published pinocchio numbers on this toolchain before
  any Hopper number is trusted. See `reference/NOTICE.md`.
- `results/`: the generated `RESULTS.md` and `results.json`.

## Run it

```sh
py -3.12 scripts/bench-framework-comparison.py
```

The script builds every fixture with `cargo build-sbf --lto` under pina's
release profile (`lto = "fat"`, `codegen-units = 1`, `opt-level = 3`,
`overflow-checks = false`, injected through the same `CARGO_PROFILE_RELEASE_*`
variables pina's driver sets), builds the verifier, measures each artifact,
and rewrites `results/`. It exits non-zero if any fixture fails its functional
checks; a fast number from a broken program is not a result.

For a source-bound capture, commit the source and use `--require-clean` with
an output directory under `target`, then review/copy the generated reports.
The JSON records source commit, clean-tree status, lockfile SHA-256 and each
measured ELF's SHA-256. The driver refuses source changes during capture.
`--skip-build` explicitly records reused artifacts without asserting their
source identity and cannot be combined with `--require-clean`.

## Adding the rows upstream

pina's driver takes one entry per framework in `frameworksFor()`; the
fixtures here are written so they drop in unchanged apart from the
dependency line. Copy `programs/<case>/hopper-substrate` to
`benchmarks/framework-comparison/programs/<case>/hopper`, replace the
workspace dependency with a pinned git revision of `hopper`, commit its
`Cargo.lock`, and register:

```ts
{
  directory: "hopper",
  label: "Hopper",
  crateName: `${program}_hopper`,
  data: { hello: "", initialize: "00", increment: "01" },
  initializeTakesBump: true,
  counterAccount: BUMP_THEN_COUNT,
}
```

Program ids, instruction bytes, and the account layout are already pina's.
