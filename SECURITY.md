# Security Policy

Hopper is a zero-copy framework for Solana programs that handle real assets.
Reporting security issues responsibly matters.

## Reporting a vulnerability

**Please do not open public GitHub issues for security findings.** Instead:

- Email **security@bluefootlabs.com** with a description, reproduction
  steps, and any proof-of-concept code.
- Or DM [@moonmanquark](https://x.com/moonmanquark) on X for an initial
  contact and we'll move to a secure channel.

We aim to acknowledge receipt within 48 hours and provide a triage
verdict within 5 business days.

## Scope

- All crates under `crates/` published to crates.io.
- The CLI tooling under `tools/hopper-cli/`.
- The example programs in `examples/` (we treat findings here as
  documentation issues unless the example is explicitly named as a
  reference / known-good pattern).

## Out of scope

- Findings against deprecated branches or pre-`0.1.0` versions.
- Issues caused by user code that violates a documented `# Safety`
  invariant. Hopper's unsafe inventory is at
  [`docs/UNSAFE_INVARIANTS.md`](docs/UNSAFE_INVARIANTS.md); calling a
  `pub unsafe fn` without upholding its preconditions is the caller's
  responsibility.

## Disclosure timeline

For confirmed findings:

1. We work with the reporter on a fix.
2. The fix lands behind a private feature flag if disruptive, or
   directly into `main` if not.
3. We coordinate a public disclosure date with the reporter — usually
   30 days from the patch landing, longer if the finding affects
   downstream protocols that need time to upgrade.
4. CVE assignment for any finding meaningful enough to warrant one.

Reporters who follow this process get full credit in the disclosure
note and the [`CHANGELOG.md`](CHANGELOG.md) entry.

## Hardening status

The full audit posture is documented in [`AUDIT.md`](AUDIT.md). Hopper's
security model rests on:

- **No `unsafe` without a documented invariant.** The unsafe inventory
  is tracked in [`docs/UNSAFE_INVARIANTS.md`](docs/UNSAFE_INVARIANTS.md).
- **Layout fingerprints.** Every account carries an 8-byte SHA-256
  layout ID in its header so cross-program reads cannot be tricked
  into the wrong shape.
- **Segment-level borrow tracking.** Byte-range-level aliasing
  enforcement via `hopper_runtime::segment_borrow::SegmentBorrowRegistry`.
- **Trapping duplicate-marker handler.** The loader-input parser
  refuses forward-reference and self-loop duplicates rather than
  silently falling through to account zero (a pre-audit footgun, now
  closed).
- **Three-tier memory access.** Safe overlay → Pod → unchecked raw,
  with the `unsafe` keyword visible at every escape hatch.

Thanks for helping keep Hopper safe.
