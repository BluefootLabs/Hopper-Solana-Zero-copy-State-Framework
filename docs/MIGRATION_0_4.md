# Moving to Hopper 0.4

Upgrade `hopper-lang` and the CLI together. The facade pins its matching
`hopper-derive` version because generated code uses facade/runtime APIs from
that release. The 0.4 line prevents existing 0.3 dependency ranges from selecting
these new expansions.

```toml
hopper = { package = "hopper-lang", version = "=0.4.0", features = ["proc-macros"] }
```

Existing generated wire formats, discriminator bytes, layout fingerprints,
positional constructors, and borrow APIs remain unchanged. Review these source
and validation changes when upgrading:

- State macros generate a `<State>Fields` companion and `from_fields` /
  `set_fields` methods. Rename handwritten items that collide with those names.
  See [named initialization](NAMED_INITIALIZATION.md) for fixed-head and lifecycle
  limits.
- Handwritten body-only implementations of the legacy `HopperLayout` trait
  should set `OVERLAY_OFFSET = 16`. Its default remains zero for structs that
  include the account header. Generated state types set this automatically.
- Incorrect `FixedLayout::SIZE` implementations now fail during code generation
  on checked casts and collections. Leave the default or use the actual Rust
  type size. Overriding `_SIZE_IS_HONEST` does not bypass the check. `cargo check`
  alone does not exercise all generic compile-time assertions; run a build.
- Runtime typed loaders independently bound the actual type, even with custom
  validators and reported-size methods. Compact initialization rejects tail
  prefixes overlapping the head or overflowing their offset calculation.
- Typed segment descriptors must report the correct element size, count no
  greater than capacity, and an allocated region within the account. Previously
  accepted malformed metadata is rejected.
- The SOL vault example's deposit account list now includes the System Program.
  Clients of that example must supply it. This corrects its CPI account list;
  it does not automatically change other programs' instruction interfaces.

`VerifiedAccount` remains a size-checked overlay wrapper; its public constructors
do not establish ownership or authorization. Use the appropriate account loader.
The host harness remains a direct invocation tool: use compiled SBF and devnet
tests for actual transaction rollback and custody rules.
