# Runtime policy devnet evidence

Captured from clean source `fefc94bc898cf1ceaae55c7b5abf0fceebac3044` on public
devnet, finalized commitment. Program:
`BPNYrNXCPJwV3k8txPVYWjbqTswkLxRGF1d2DzcGTHAX`.

Deployment signature:
`2733e89xfNfZnsZBtmjGJzL4MuzfDAbiDutSuthfAF3JjxrGcbGAp6WDn1DuNdxzURWHffQMFjFi3EKNbrxGjive`.

ELF SHA-256:
`a55d562d7047cc119be552280ccce6b053a20d4ef98b47525ebbf1f99da93335`.
The runner compared the local ELF to finalized program dumps before and after.

The receipt and 12 finalized transaction responses cover two account creations,
four successful cases (narrow write, nested/out-of-order guards, selected cell,
and fresh no-policy state), and six deliberate refusals. Refusals return the
expected `Custom(0xD000)` or `Custom(0xD0FF)` and preserve complete observed
account snapshots. Cases also cover failed installation, leaked guards,
foreign lamport mutation, and foreign close refusal.

`SHA256SUMS` covers the public JSON files. Keys, private deployment diagnostics,
and binary copies are excluded. This first capture retained snapshot hashes
and the harness's verification results, rather than the full snapshot JSON;
the updated harness retains snapshots on subsequent runs. These receipts are
maintainer-generated observations, not independently authenticated pre/post
state proofs. They attest this fixture at this source, not later code or
Cicada's complete lifecycle.
