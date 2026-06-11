# `hopper build`

Build a Hopper program. Defaults to the SBF target (`cargo build-sbf`); pass
`--host` to build for the host triple instead (useful for fast unit-test loops
where you do not need the on-chain artifact).

## Usage

```
hopper build [--host | --sbf] [--watch] [-p <package>] [<cargo args>...]
```

## Flags

| Flag | Meaning |
|---|---|
| `--sbf` | Build to SBF (default). Emits `target/deploy/<crate>.so`. |
| `--host` | Build for the host triple via plain `cargo build`. |
| `--watch` | Rebuild on file change. |
| `-p <package>` | Build a specific workspace member. |

Any unrecognized arguments are forwarded to `cargo` / `cargo build-sbf`.

## Behavior

On the SBF path, `hopper build` snapshots the `.so` size in
`target/deploy/` before and after the build and prints the delta, so a size
regression is visible on every build (mirroring the size line other zero-copy
toolchains print).

## Examples

```bash
# SBF build of the counter example
hopper build -p hopper-counter

# Fast host build for unit tests
hopper build --host -p hopper-migration

# Watch + rebuild
hopper build --watch -p hopper-escrow
```
