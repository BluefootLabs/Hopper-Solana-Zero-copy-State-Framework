# Runtime lab

The parts of Hopper that only mean something on the VM, one instruction
each: the heap allocator against the heap the runtime really maps, the
panic handler in both builds, and the SlotHashes lookup against a sysvar
with skipped slots. Every instruction answers through return data, so a
test checks the values that came back.

| Tag | Instruction | What it shows |
|---|---|---|
| 0 | `allocate(bytes)` | One block of `bytes`, filled, with a marker on every page read back. Past 32 KiB it needs the transaction to request a heap frame. Returns the heap used and the pages touched. |
| 1 | `grow(kib)` | A vector grown one KiB at a time. It is the most recent allocation, so every growth happens in place and the heap used equals its capacity. |
| 2 | `checkpoint_loop(bytes, rounds)` | Allocate and drop `bytes`, `rounds` times, releasing the heap to a checkpoint each round. Returns the heap used after the loop and the peak. |
| 3 | `panic_now(code)` | Panics with a message carrying `code`. |
| 4 | `slot_hash_back(back)` | The hash of the slot `back` slots behind the newest SlotHashes entry, the reason when there is none, and the number of sysvar reads. |
| 5 | `slot_hash_at(slot)` | The same for an absolute slot. |

The program declares a 256 KiB heap with
`hopper::default_allocator!(heap = 256 * 1024)`. A transaction that
allocates more than 32 KiB carries `ComputeBudgetInstruction::RequestHeapFrame`;
`hopper tx send --heap-frame 262144` adds it.

## Run it

From this directory:

```text
cargo build-sbf
cargo build-sbf --features report-panics --sbf-out-dir ../../target/deploy-report-panics
cargo test --test lab -- --nocapture
```

A test whose artifact is missing prints `SKIPPED` and passes.

## What the tests measure

| Case | Result |
|---|---|
| 32 KiB heap, no frame requested | 12 KiB less the cursor word usable; one byte more fails when the write leaves the mapped heap |
| 256 KiB frame requested | a 241,656-byte block (all of it above the scratch region), 2,250 CU for the instruction |
| Vector grown to 200 KiB a KiB at a time | 200 KiB of heap used, equal to its capacity; 21,280 CU |
| 100 rounds of 8 KiB through a 12 KiB heap | nothing held at the end, peak one round; 14,419 CU |
| Slot hash 3 slots back | 1 sysvar read, 535 CU for the instruction |
| Slot hash 20 to 250 slots back | 2 reads, about 915 to 958 CU |
| Every slot of a 512-entry sysvar with skips | never more than 4 reads |
| Panic, default build | aborts, logs nothing from the program, 90 CU |
| Panic, `report-panics` build | `Program log: runtime lab panic, code 7` and `Panicked in <file> at <line>:<column>`, 559 CU |

The `report-panics` feature turns on Hopper's `panic-location` and
`panic-message` features; the default build is the silent handler that
production programs ship.
