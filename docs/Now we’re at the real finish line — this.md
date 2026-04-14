a lot of correct subsystems living too close together

That is why performance is not landing where you expected.

You have:

too much inside hopper-core
too much conceptual richness too close to the hot path
still not enough separation between:
substrate
runtime semantics
macro/codegen DX
advanced execution / tooling layers

So the path forward is not to delete the edge.

It is:

reduce hot-path drag
split core from advanced
flatten the runtime story
make the repo communicate one execution model
The biggest repo-level finding

The Hopper repo shape is the single biggest issue now.

Hopper current shape

From the latest zip, your repo has:

crates/hopper-anchor
crates/hopper-associated-token
crates/hopper-core
and likely the other Hopper crates further down
benchmark infrastructure
docs
tooling

But hopper-core itself is carrying far too much:

ABI
account system
accounts context/meta/program/signer
checks and graphs
collections
CPI
diff
dispatch
event
frame
invariant
migrate
policy
receipt
segment map
state
sysvar
time
and likely more

That means “core” is currently doing the job of:

substrate support
runtime support
advanced execution
optional lifecycle features
tooling-facing metadata helpers

That is too much.

What Pinocchio teaches here

Pinocchio separates:

SDK core
derive
CLI
companion program crates

It does not let its hot-path identity drown in platform breadth.

What Quasar teaches here

Quasar separates:

CLI
derive
examples
framework/runtime pieces

Again: the repo shape itself communicates that the hot path is not the same thing as the whole platform.

Why Hopper likely missed the performance target

Based on the latest structure, here are the most likely causes.

1. Too much “core”

When one crate contains:

account access
advanced validation graphs
lifecycle systems
receipts
policies
migration support
explainability
segmented account support
collections
you increase both:
compile-time complexity
conceptual coupling
the risk that hot-path code gets routed through abstractions that were really meant for advanced use
2. Runtime still not flattened enough

Earlier we already saw hopper-runtime/src/account.rs holding a backend-shaped identity. If that persists in spirit, it means:

the access path still isn’t fully “owned”
wrapper layers can still add friction
optimizing the core path remains harder than it should be
3. Segment innovation is real, but may still be implemented too high

Segment-level access is your biggest real innovation.
But if the segment system is not lowered aggressively enough into:

static offsets
direct pointer ops
tiny borrow checks
then it becomes “correct but heavier” instead of “correct and obviously worth it”
4. Borrow safety may still be richer than the hot path can afford

This is a subtle one.
Hopper wants runtime-enforced safety.
That’s good.
But if the borrow layer becomes:

too wrapper-heavy
too projection-heavy
too host-driven in semantics
too generalized
then it costs too much mentally and maybe in CU as well
5. Macro/codegen story is still split

You now clearly need proc-macro DX, yes.
But if the repo still centers a giant declarative macro buffet while also trying to become Quasar-simple, the codegen story feels split and the emitted code may still reflect older abstraction layers.

What to fix now, path by path

This is the concrete audit outcome.

1. crates/hopper-core — biggest refactor target
Verdict

This crate is too broad.

What to do

Do not gut it.
Do split its role conceptually and probably physically.

What belongs in true core

Keep only the pieces that truly belong to the shared hot-path-adjacent base:

abi/*
account/header.rs
account/pod.rs
account/segment.rs
maybe account/overlay.rs if it is actually central
field_map.rs
segment_map.rs
maybe a very small subset of state/*
minimal check/guards.rs or check/fast.rs if they are truly universal
maybe tiny invariant primitives if zero-cost and central
What should move out of “core identity”

These are not bad. They are just too rich to sit at the center.

frame/*
receipt.rs
policy.rs
check/graph.rs
migrate/*
accounts/explain.rs
accounts/migrating.rs
richer collections not needed in the default story
any virtual/lifecycle/platform-ish support
Recommendation

Split these into either:

hopper-advanced
hopper-observe
hopper-lifecycle
or at least feature-gate them heavily and reclassify them in docs
Why

This alone will help the repo communicate one hot-path story and may reduce accidental internal drag.

2. crates/hopper-native — strengthen and trust it more
Verdict

This is still the right place to win.

What to do

Make hopper-native the unquestioned owner of:

pointer path
raw account view
entrypoint
raw CPI invoke
PDA derivation substrate
Exact files
src/account_view.rs
src/entrypoint.rs
src/cpi.rs
src/pda.rs
src/raw_input.rs
src/raw_account.rs
Priority

High.

Fix target

A safe generated Hopper accessor should lower almost directly into hopper-native.

3. crates/hopper-runtime/src/account.rs — still critical
Verdict

This file is still one of the main bottlenecks to clarity and probably performance.

What to do

Make this the one public access model.

Converge on:

load<T>()
load_mut<T>()
segment_ref<T>(SEG)
segment_mut<T>(SEG)
unsafe raw_ref<T>()
unsafe raw_mut<T>()
Remove drift

Do not let this file keep feeling like:

many access philosophies
backend-oriented wrapper identity
a place where advanced systems leak into the ordinary path
Goal

One public access model. Same pointer path under both safe and raw access.

4. crates/hopper-runtime/src/borrow.rs and borrow_registry.rs
Verdict

Real differentiator, but also real risk.

What to do

Keep:

segment-level conflict detection
fixed-size registry
read/write overlap logic

Make sure:

fixed array
no heap
no dynamic “graph engine”
no runtime names
no rich wrapper gymnastics in hot path
Important

The registry should be:

concrete
tiny
obvious
Goal

If Hopper is slower than expected, this is one of the first places I’d simplify before touching the segment model itself.

5. crates/hopper-runtime/src/context.rs
Verdict

Should become extremely boring.

What to do

Keep it as:

accounts owner
borrow registry owner
thin base for generated accessors

Do not let it own:

execution modifiers
policy engines
tooling semantics
6. Macro story — needs explicit split now
Hopper current issue

You still have a big declarative macro story in hopper-macros, but the product direction clearly wants Quasar-level DX.

Correct move

Support both:

declarative low-level macros remain
new proc-macro companion becomes the top DX surface
What to do

Create / solidify:

hopper-macros-proc

With:

#[hopper::state]
#[hopper::context]
maybe #[hopper::program]
Rule

These macros generate:

constants
accessors
impls
metadata

They do not own runtime logic.

Why

This gives you Quasar parity while keeping Pinocchio-style transparency available.

7. CLI — you need to narrow it before you broaden it
Quasar lesson

Quasar feels finished partly because its CLI is clearly its own layer:

cli/src/build.rs
deploy.rs
idl.rs
lint.rs
test.rs
etc.

It feels productized.

Hopper lesson

Your CLI should not try to be everything yet.
It should first be the trust-building surface.

Keep and polish first
hopper build
hopper compile --emit rust
hopper inspect
hopper explain
Why

Those commands prove:

performance transparency
inspectability
safety visibility

That matters more than a long command list right now.

8. Manager — keep, but narrow its mission
Verdict

Manager is useful, but it must not become a second semantic engine.

What to do

Use it to prove:

layouts are inspectable
segments are visualizable
borrows/conflicts are explainable
receipts/schema are visible if enabled

That is enough for now.

9. Companion crates — yes, add them

To your question:

won’t we need hopper-token etc like Pinocchio does since we will be replacing solana-program?

Yes.

Absolutely.

Pinocchio’s companion crates matter because they make the ecosystem feel complete.
If Hopper is owning the full flow, it should own these too.

Add soon after core alignment
crates/hopper-system
crates/hopper-token
Then later
crates/hopper-token-2022
crates/hopper-associated-token
Rule

These should start thin:

builder/helper crates
explicit
no giant abstraction layer

They are important for parity, but not before core alignment.

How Hopper compares right now
Against Pinocchio

Hopper is already richer and safer in concept.
But Pinocchio still wins structurally on:

substrate sharpness
repo layering
companion completeness
confidence that nothing hidden is in the hot path
Against Quasar

Hopper can absolutely beat Quasar if it gets the top-layer DX right.
But Quasar still wins structurally today on:

clean codegen separation
obvious product shape
lower conceptual clutter at first glance

So Hopper’s challenge is no longer “invent enough.”
It is:

be as sharp as Pinocchio
feel as easy as Quasar
keep the Hopper-only segment/safety/introspection edge
Final verdict
Are you building the right thing?

Yes.

Is the repo currently aligned enough to hit the performance goal?

Not yet.

Why not?

Because Hopper is still carrying too much richness too close to its core, and the repo does not yet communicate a single flattened access/runtime story the way Pinocchio and Quasar do.

What to do now
slim the hot path
split advanced from core
finalize proc-macro DX as thin codegen
add Hopper-owned companion crates after core alignment
use CLI to prove transparency and safety

# Hopper v30 Strict Refactor Checklist

## Purpose

This checklist aligns Hopper toward the actual goal:

- **Pinocchio-level runtime shape**
- **Quasar-level ergonomics**
- **Hopper-level safety, segmentation, and introspection**

The goal is **not** to gut Hopper.

The goal is to:

- keep the edge
- remove drift
- reduce hot-path drag
- make one access model obvious
- reclassify advanced systems without deleting them

---

## Non-negotiable principles

### Runtime
- No heap in hot path
- No runtime string lookups in access path
- Safe and raw access must lower to the same pointer math
- Macros generate structure, not runtime behavior
- Segment access must compile to constants and offsets

### Public model
Stop presenting Hopper as:
- raw mode
- segmented mode
- pipeline mode
- macro mode

Replace that with:
- **one access model**
- **explicit unsafe raw access**
- **optional advanced guarantees**
- **compile-time generated ergonomics**
- **inspectable compiled output**

### North star
A Hopper developer should primarily feel like they are writing:

```rust
let balance = ctx.vault.balance_mut()?;

or explicitly:

let vault = unsafe { ctx.vault.raw_mut::<Vault>() };

Everything else is:

optional guarantee
optional metadata
optional tooling
optional observability

Not a separate framework mode.

Master rules
Runtime rules
No heap in hot path
No runtime string lookups in access path
No duplicate execution models
Safe and raw access must lower to the same pointer math
Macros generate structure, not runtime behavior
Public API rules

Stop publicly framing Hopper as:

raw mode
segmented mode
pipeline mode
macro mode

Replace that with:

one access model
explicit unsafe raw access
optional advanced guarantees
inspectable compiled output
Public phrase

Access + Guarantees

Pass 0 — Repo shape and crate boundaries

Before touching implementation details, lock the boundary story.

Crates that should stay central
hopper-native
hopper-runtime
hopper-core
hopper-macros
hopper-macros-proc
hopper-schema
hopper-cli
hopper-manager
Crates to add after core alignment
hopper-system
hopper-token
later hopper-token-2022
later hopper-associated-token
Boundary rules
hopper-native

Substrate only:

entrypoint
raw input
raw account view
pointer math
raw CPI
PDA derivation
hopper-runtime

Semantics only:

typed access
borrow registry
safe/unsafe access API
context
layout-aware loads
runtime validation
hopper-core

Minimal shared primitives only:

ABI
tiny account/layout helpers
field/segment metadata
small universal contracts
hopper-macros

Low-level declarative support only

hopper-macros-proc

Top-level DX only:

#[hopper::state]
#[hopper::context]
maybe #[hopper::program]
hopper-schema

Generated mirror of runtime truth

hopper-cli

Trust-building tooling:

build
emit rust
inspect
explain
hopper-manager

Inspector/visualizer only

Pass 1 — Lock the hot path

These files come first.

1. crates/hopper-native/src/account_view.rs
Goal

Make this the single substrate truth for memory access.

Change goals
Normalize around one pointer path
Keep this file tiny and direct
Ensure safe and raw access share the same pointer math base
Required primitives
segment_ref<T>(offset, size)
segment_mut<T>(offset, size)
unsafe raw_ref<T>()
unsafe raw_mut<T>()
Requirements
Add #[inline(always)] on hot-path accessors
No names, strings, or maps in these methods
No schema logic
No policy logic
No manager/CLI logic
Done when

Every higher-level access path can be traced back to these four primitives.

2. crates/hopper-runtime/src/account.rs
Goal

Make this the one public access story.

Public surface to converge on
Safe full account
load<T>()
load_mut<T>()
Safe segment
segment_ref<T>(SEG)
segment_mut<T>(SEG)
Explicit raw
unsafe raw_ref<T>()
unsafe raw_mut<T>()
Change goals
Collapse all competing access APIs into this one model
Remove any “multiple mode” feeling
Make sure the safe path and the raw path both lower into the same native pointer logic
Remove or refactor
Alternate overlapping access APIs
Backend-first identity feel
Dynamic-lookup based access
Anything that makes access look philosophically split
Done when

A developer can learn this file and understand Hopper’s entire access story.

3. crates/hopper-runtime/src/borrow_registry.rs
Goal

Make borrow safety real, cheap, and predictable.

Required structure

Use:

fixed-size array
no heap
no Vec
small borrow entry struct
Borrow entry should encode
account key
offset
size
access kind (read/write)
Conflict rules
read + read = okay
any overlapping write = reject
overlap only matters for the same account key
Change goals
Tighten the Solana-target branch if it is too no-op-ish
Keep host-side behavior from setting unrealistic expectations for on-chain behavior
Done when

Borrow safety is:

real enough to matter
cheap enough not to kill performance
simple enough to reason about
4. crates/hopper-runtime/src/borrow.rs
Goal

Keep safety wrappers only where they actually help.

Change goals
Keep wrappers minimal
Keep projection helpers only if they lower cleanly into direct access
Inline everything important
Do not
Turn this into a second abstraction stack
Let it dominate the segment access path
Done when

Borrow support feels invisible in the normal access path.

5. crates/hopper-runtime/src/context.rs
Goal

Make context boring and stable.

Context should own
accounts
borrow registry
thin indexed access
Change goals
Keep this as the target for macro-generated accessors
Remove any extra execution model logic from here
Do not
Put pipeline engine logic here
Put manager logic here
Put schema logic here
Create a second public framework surface here
Done when

Context feels like a container, not a framework.

Pass 2 — Lock segments and layout

This preserves Hopper’s real edge.

6. crates/hopper-core/src/account/segment.rs
Goal

Make this the canonical segment primitive.

Keep
offset
size
Change goals
Keep this file tiny and explicit
Treat it as a memory primitive, not a framework concept
Done when

This file feels like a raw contract for memory slices.

7. crates/hopper-core/src/segment_map.rs
Goal

Make segment metadata static and compile-time-first.

Change goals
Use associated constants or static tables
Make ordering deterministic
Ensure runtime consumes constants, not names
Do not
Use runtime string lookup in hot path
Make this reflective or dynamic
Done when

Hot path uses constants.
Tooling uses names.

8. crates/hopper-core/src/field_map.rs
Goal

Keep field metadata aligned with segment metadata.

Change goals
Ensure FieldInfo and segment info come from the same codegen source
Keep this primarily for:
schema
CLI
manager
inspection
Done when

Field maps and segment maps cannot drift from each other.

9. crates/hopper-runtime/src/layout.rs
Goal

Keep layout authoritative.

Keep
LayoutContract
version/discriminator/layout id rules
compatibility helpers
Change goals
Ensure segment/layout metadata are driven by compile-time generated constants
Keep runtime/layout as the authoritative truth
Do not
Let schema or manager concepts own runtime rules
Let this become a tooling kitchen sink
Done when

Runtime truth clearly lives here.

Pass 3 — Narrow what “core” means

This is one of the biggest structural issues right now.

10. crates/hopper-core/Cargo.toml + crates/hopper-core/src/lib.rs
Goal

Make hopper-core mean only the hot-path-adjacent shared foundation.

Keep in true core

Only things that are genuinely hot-path-adjacent and universally shared:

ABI primitives
minimal account/header/pod/segment/overlay if central
field/segment maps
maybe tiny invariant or fast-check primitives
maybe tiny state helpers
Reclassify out of core identity

These are valuable, but too rich to sit at the center:

frame/*
receipt.rs
policy.rs
check/graph.rs
migrate/*
accounts/explain.rs
accounts/migrating.rs
richer collections not needed in launch story
any virtual/lifecycle/platform-ish support
Done when

“Core” means the hot-path-adjacent foundation, not the whole Hopper platform.

11. crates/hopper-core/src/frame/*
Goal

Reclassify as advanced execution features.

Action
Feature-gate if needed
Do not let this define the basic Hopper authoring story
Done when

Normal Hopper users can ignore frame entirely.

12. crates/hopper-core/src/receipt.rs
Goal

Keep receipts, but move them out of the launch-critical center.

Action
Treat as opt-in observability / advanced feature
Done when

Receipts feel like strength, not required knowledge.

13. crates/hopper-core/src/policy.rs
Goal

Keep policies optional.

Action
Do not let policies define the primary access model
Done when

Policies are additive guarantees, not the first thing users meet.

14. crates/hopper-core/src/check/graph.rs
Goal

Reclassify as advanced validation.

Action
Move graph-based checks out of the default mental model
Done when

You can launch Hopper without making graph validation central.

15. crates/hopper-core/src/migrate/*
Goal

Keep as lifecycle tooling.

Action
Reclassify as advanced
Do not let migrations dominate the core story
Done when

Migration remains supported but not identity-defining.

16. crates/hopper-core/src/accounts/explain.rs
Goal

Clarify whether this belongs in core or tooling.

Action
If primarily for CLI/manager, move conceptually toward tooling consumption
If kept in core, ensure it is a very light reusable introspection helper
Done when

Explainability is visible without muddying execution semantics.

17. crates/hopper-core/src/accounts/migrating.rs
Goal

Treat as advanced lifecycle support.

Done when

It no longer shapes the first-time Hopper story.

Pass 4 — Align macro and codegen story

Yes, Hopper needs macro-based codegen.
That is the correct Solana move.

The rule is:

Macros generate structure, not runtime behavior.

18. crates/hopper-macros/src/lib.rs
Goal

Keep only low-level declarative support here.

Keep
Low-level helpers
Compile-time checks
Useful low-level layout support
Change goals
Stop treating this crate as the whole authoring story
Separate old declarative Hopper from the new public DX layer
Done when

This crate feels like support infrastructure, not the main public API.

19. Add crates/hopper-macros-proc
Add
#[hopper::state]
#[hopper::context]
maybe #[hopper::program]
#[hopper::state] should generate
segment constants
static segment tables
field map
layout impls
schema export hooks
#[hopper::context] should generate
account index bindings
accessor methods like vault_balance_mut()
direct static segment constant usage
#[hopper::program] should generate
thin dispatch glue only
Important

Macros must not own:

runtime logic
validation execution semantics
hidden control flow
Done when

Generated Rust is reviewable and clearly lowers to native/runtime primitives.

20. Generated accessors
Required hot-path shape
#[inline(always)]
pub fn vault_balance_mut(&mut self) -> Result<&mut u64, ProgramError> {
    const SEG: Segment = Segment { offset: 0, size: 8 };
    self.ctx.accounts[0].segment_mut(&mut self.ctx.borrows, SEG.offset, SEG.size)
}
Rules
No string lookup
No dynamic map lookup
No rich runtime dispatch
Done when

Pinocchio-minded developers can trust emitted code immediately.

Pass 5 — CLI / Manager / companion crates
21. tools/hopper-cli/*
Goal

Make CLI a trust engine.

Keep and polish first
hopper build
hopper compile --emit rust
hopper inspect
hopper explain
hopper compile --emit rust

This is non-negotiable.
It is the trust-builder.

hopper inspect

Show:

fields
segments
offsets
sizes
versions/layout ids if useful
hopper explain

Show:

reads
writes
segment touches
borrow conflicts / guarantees
Hide / postpone
orchestration-heavy commands
unstable manager-style flows
commands whose semantics outpace runtime truth
Done when

CLI makes Hopper:

more transparent than Pinocchio
more explainable than Quasar
22. hopper-manager/*
Goal

Make manager an inspector, not an engine.

Keep
account visualization
segment visualization
borrow/access graph ideas
schema-driven inspection
Do not
Let manager define runtime semantics
Let it become a second framework model
Done when

Manager proves Hopper is inspectable and safer.

23. Add crates/hopper-system
Add after core alignment

Start thin:

transfer
create account
assign
maybe close/realloc helpers later
Rule

Explicit builder/helper surface.
No giant abstraction layer.

24. Add crates/hopper-token
Add after core alignment

Start thin:

transfer
mint_to
burn
approve
close account
Rule

Use Hopper-owned types and runtime surfaces.

25. Later crates
hopper-token-2022
hopper-associated-token
maybe hopper-memo
maybe hopper-sysvar

Only after the core is obviously aligned.

Docs and public model patch

Do this in parallel so the repo stops sounding more fragmented than the code.

Stop saying publicly
raw mode
segmented mode
pipeline mode
macro mode
Start saying
one access model
explicit unsafe raw access
optional advanced guarantees
compile-time generated ergonomics
inspectable compiled output
Core public phrase

Access + Guarantees

Start-now order

If you are starting immediately after reading this, do it in exactly this order:

crates/hopper-native/src/account_view.rs
crates/hopper-runtime/src/account.rs
crates/hopper-runtime/src/borrow_registry.rs
crates/hopper-runtime/src/borrow.rs
crates/hopper-runtime/src/context.rs
crates/hopper-core/src/account/segment.rs
crates/hopper-core/src/segment_map.rs
crates/hopper-core/src/field_map.rs
crates/hopper-runtime/src/layout.rs
crates/hopper-core/Cargo.toml + src/lib.rs
crates/hopper-macros/src/lib.rs
add crates/hopper-macros-proc
generate accessor/constants-only DX path
crates/hopper-core/src/frame/*
crates/hopper-core/src/receipt.rs
crates/hopper-core/src/policy.rs
crates/hopper-core/src/check/graph.rs
crates/hopper-core/src/migrate/*
crates/hopper-core/src/accounts/explain.rs
crates/hopper-core/src/accounts/migrating.rs
tools/hopper-cli/*
hopper-manager/*
crates/hopper-system
crates/hopper-token
Final reminder

Do not gut the repo.

Do not flatten Hopper into Quasar-with-safety.

Do:

preserve the edge
reduce drift
make one access model obvious
push advanced systems into optional guarantees and tooling

That is the path to making Pinocchio and Quasar feel obsolete.