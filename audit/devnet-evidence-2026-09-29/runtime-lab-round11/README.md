# Devnet evidence, 2026-09-29: the runtime lab, round eleven

Public devnet (`https://api.devnet.solana.com`), signer `4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and upgrade
authority). The program is `examples/hopper-runtime-lab`, built twice with
`cargo build-sbf` in a clean worktree at commit `25ed970`:

- the default build, 26,136 bytes, SHA-256 `0986c5ee45ddfa941ca11abe388284c9708b232ef8fc6fb1079e04fa5dac7e11`,
  deployed fresh as `FzojENn2sf6z3tRZRq5iwzvZbZPVPdMX5UzvmvxaG7uH` at slot 505,682,067;
- the `report-panics` build, 29,088 bytes, SHA-256
  `372f3e5c615c31db60aca9499e110dae778539064ff0df8afbfbdee398c734be`, deployed fresh as
  `9nDHLtQrknMvPcex3XJiH5C1LWBYnK6p3RnhTF4NfoTc` at slot 505,682,150.

Both on-chain dumps equalled the local ELFs before the first transaction and
after the last one (`before-*-onchain.so`, `after-*-onchain.so`).

What this round proves, on a live validator:

- The program declares a 256 KiB heap. With `hopper tx send --heap-frame 262144`
  it allocates 200 KiB and the whole frame above Hopper's scratch region
  (241,656 bytes); one byte more is refused by the allocator itself
  (`Custom(6800)`). Without the frame the same 200 KiB fails when the
  write leaves the 32 KiB the runtime mapped.
- A vector grown to 200 KiB one KiB at a time uses exactly 200 KiB of heap,
  and a loop of 100 rounds of 8 KiB runs through the 12 KiB default heap with
  a checkpoint, holding nothing at the end.
- `slot_hash_lookup` against the live SlotHashes sysvar: each returned hash
  equals the sysvar's entry for that slot, a slot older than the sysvar is
  `TooOld`, and a future slot is `Ahead`.
- A panic in the default build logs nothing from the program. In the
  reporting build the runtime logs the message and the location:
  `Program 9nDHLtQrknMvPcex3XJiH5C1LWBYnK6p3RnhTF4NfoTc failed: SBF program Panicked in examples\hopper-runtime-lab\src\lib.rs at 168:9`.

One accounting detail worth knowing. The two failed allocations are both
access violations, charged differently: `allocate-past-default-heap` faulted
on a direct one-byte store outside any mapped region (`Access violation writing 1 bytes at address 0x300008000 (in unallocated region)`) and the
transaction meta reports 200,000 compute units, the whole budget, while
the program's own log line reads 195; `allocate-200k-without-frame` faulted inside the
memset syscall (`Access violation writing 204799 bytes at address 0x300005008 (in heap region)`) and
was charged 960.

Not run: `slot-hash-skipped` (no skipped slot in the last 200).

15 transactions, each sent with `hopper tx send` and re-fetched at `finalized`.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| `allocate-default-heap` | `VUTqnDTsQ2DYV6EhM6EvhGYhMGfhGFAnEkpxqv8YGJpjZETr76bkX2YsEhLYCDGDkypLD6ELWiP2iYFiPLtZAxx` | 505,682,441 | ok | 381 |
| `allocate-past-default-heap` | `5LYd4RRwebobMgpkRSzATnQ3kpK5JdaUk8xNWYwSx8JrLBfMVAWPAGLbBNKAbfsekprvy4VNKXWz4k4j6QeE95TW` | 505,682,456 | refused: `ProgramFailedToComplete` (instruction 0) | 200,000 |
| `allocate-200k-with-frame` | `2e9xMxDmNcpB6XCgF52gcsSiZoe4s3dPjugFFJxSqDE8GdbP7L73c7LDiJsQK7RQ1KLBiEfffQomZi1qgqeCyDFb` | 505,682,466 | ok | 2,109 |
| `allocate-200k-without-frame` | `rH1StZHrMXomttFeDQT8nrbvnupmBQUCGk8JSKqcMDD3dGHUPVfAs1bKNDDVKVqp5g97FUAw4FLECRqpAVLcn4X` | 505,682,478 | refused: `ProgramFailedToComplete` (instruction 0) | 960 |
| `allocate-whole-frame` | `31dFFjh7WoFL7NeFRhU2sa5JsD5JBU7U8rue6oUT35NThNXNcVDZwzufefyuzhg5iWdwbwp3usdzqEqcqbPWNWW6` | 505,682,504 | ok | 2,400 |
| `allocate-past-declared-heap` | `3GNumXTQd13QAVVtwFuAuHCjzmUyY57VxM1Jqi8LeBVvona7VRQ642EVuQuWL8LX6YxEmL4QbLxBqq9dhCLbpCLK` | 505,682,516 | refused: `Custom(6800)` (instruction 1) | 340 |
| `grow-to-200k` | `5oZvy2zUgQxTE9zxu9sjTFjRjbEHaLrhB5bwvokoDXrXrNHJYhuR1TYQHJSBwbxCXBBtsTBJhXddaFcuNgZBhm8Z` | 505,682,533 | ok | 21,430 |
| `checkpoint-loop` | `2SB6h3vyHp7v6Hj7HU8ZaF4ngcHDYD5osaTzgKnzojH5za7L1YTwYmWumq7fQnECXBLbYDdEYwshy9iju1ouW4Y7` | 505,682,546 | ok | 14,419 |
| `slot-hash-1-back` | `3dmKjB17pNo3xJNavmcWByAq7xJdzBonL2NEvgbkYLXMFTpsvYhk7LzgNg9QC8t8sLLvXvGxL7sfWf1YJXScWndn` | 505,682,560 | ok | 529 |
| `slot-hash-100-back` | `9MhP1z1AmjGf8jJ2o6kPD7YkRf8Gs79BmNh8FPYYGK5Sbd3C8Ttt5osFQP9AZUVdSBV4TNaw9p4EYuEwcShXcxV` | 505,682,574 | ok | 988 |
| `slot-hash-300-back` | `5yJZnPfzAfXQYGT9mFQSsT2jH4C5kRqZoVVdHm7TFK4GDfG1wo3vCKRbMC8kE3Ypmr9LF1vdua7QWN6nAk9FvRFK` | 505,682,590 | ok | 988 |
| `slot-hash-too-old` | `ctsdD2q6S8SaqfZfHGUktbNi6rH8UEnSHVcxFPhQ77FZB7F3yZ3VgKw8dbKkMQ14EiXfK9pN3KWLX2TY45UwFhY` | 505,682,607 | ok | 1,074 |
| `slot-hash-ahead` | `2Ft9Hh7ZXjsKjFkNx9wwEwnCz4wooVZv1mcxBET6wZwr7ZxijrL59ozTF87ETeYQL1dZMQdBAETuWXrCrmGaohdT` | 505,682,616 | ok | 488 |
| `panic-silent` | `5WzeRay87GSKRZSLMMcmLWBQcrESbC2ooeaiFcQkES9nT9DRQGdP2RTNUwZPcqQ76qi2f3PjZxK9hW6wyDdpZc8P` | 505,682,626 | refused: `ProgramFailedToComplete` (instruction 0) | 90 |
| `panic-reported` | `23T2LN7zVKwYpRCAoLhVQAjpgAcbywK1Rrsa4QomaqMMoUVDXJT7zkhSsGrcjx57EyZcVqZpR6NR8TFARK7khXW8` | 505,682,641 | refused: `ProgramFailedToComplete` (instruction 0) | 559 |

`receipt.json` is the runner's own record; `SHA256SUMS` lists every file here.
