# September 24 counter and hello refresh

Source: `dfd140053a7827c0d651cbe3bb926070ce68f15c` (clean).
`builds.json` records the commands, pinned v1.54/v0 builder and Pina release
profile. The existing framework-comparison driver's `measure` and rendering
functions were run with the freshly built development-profile host verifier;
its executable checksum is recorded. Program ELFs use the release recipe.
Rebuilt Pinocchio reference values match the pinned published values exactly.
Peer Pina, Quasar, and Anchor rows remain Pina's pinned published measurements,
not executions of the September 24 source heads.

Hopper's macro counter moved from 358 to 349 CU and 8,376 to 8,312 ELF bytes.
Initialize remains 1,549 CU; hello and substrate rows are unchanged. Quasar's
published increment remains lower at 330 CU. Account contracts and layouts
differ as described in RESULTS.md. The previous table is retained separately.
