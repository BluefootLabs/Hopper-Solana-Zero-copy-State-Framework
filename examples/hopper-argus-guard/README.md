# Hopper Argus Guard

This example is a small Argus-style subsystem proof: an authority-owned risk
book with a hard exposure limit. It demonstrates the Hopper path a production
subsystem would use for risk accounting:

- versioned zero-copy state,
- checked arithmetic on wire integers,
- authority-bound account constraints,
- closure-based safe mutation through `with_mut`,
- path-qualified SBF allocator and panic macros.

Instructions:

- `0` initialize the risk book with a limit,
- `1` reserve exposure if the limit allows it,
- `2` release exposure after settlement.

This is not a claim about external Argus source provenance. It is the framework
proof artifact for the subsystem shape: bounded state mutation, explicit risk
limits, and audit-friendly account contracts.