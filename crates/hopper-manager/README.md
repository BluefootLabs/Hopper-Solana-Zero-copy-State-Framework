# hopper-manager

Schema-driven program inspector library for [Hopper](https://hopperzero.dev).
Backs the `hopper manager` CLI command family.

## What it does

Given a `ProgramManifest` and raw account bytes, this crate identifies the
account type by `(disc, layout_id)`, decodes every field by name, renders
the segment registry for segmented accounts, and produces a structured
report ready for CLI rendering or web display.

Every public function is pure (no I/O, no syscalls), so the same logic
powers the `hopper manager` CLI subcommands and any embedded admin
dashboard or web explorer that wants the same view.

License: Apache-2.0.
