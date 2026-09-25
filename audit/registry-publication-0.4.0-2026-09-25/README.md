# Hopper 0.4.0 registry publication

26 framework/CLI packages were published at 0.4.0 from clean source
a4ed5fdd4b291592cfc1240e5ba20936b96e27c3. Three independent support packages remain unchanged at
0.1.0. All 29 registry checksums and downloaded archive VCS records were verified.

Fresh registry-only named-vault and byte-allowance consumers compile to exactly
the gated v0 ELF bytes and pass five compiled tests. The named-vault artifact is
also the one verified before/after 19 finalized devnet transactions. The
byte-allowance artifact's tests in this release are compiled SBF; its earlier
0.3.2 devnet evidence remains separately dated.

API rustdoc builds passed for hopper-lang, hopper-runtime, hopper-systems and
hopper-derive. The build reported an existing unresolved LayoutManifest link
in hopper-systems; this is a documentation-link warning, not a build failure.
Hosted docs.rs availability is recorded separately from local rustdoc.

Files capture publication, downloads, source lineage, registry dependency
locks, ELF identity, and compiled execution. SHA256SUMS seals exact public bytes.

The repository subsequently corrects that link as a qualified type name.
The warning-denied rustdoc rerun passes. This comment-only fix does not alter
the immutable 0.4.0 registry archive; its exact scope is recorded in
validation/postpublication-doc-fix.json. Hosted 0.4.0 docs.rs URLs still returned
404 at the captured check, so hosted API-documentation availability is pending.
