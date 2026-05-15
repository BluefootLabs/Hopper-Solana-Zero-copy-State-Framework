fn legacy_surface_terms() -> Vec<String> {
    vec![
        ["#", "[hopper", "::context]"].concat(),
        ["#", "[signer]"].concat(),
        ["ctx: ", "Context<"].concat(),
        ["Account", "View"].concat(),
        ["_load", "_mut("].concat(),
        ["_load", "("].concat(),
    ]
}

#[test]
fn public_docs_and_templates_stay_on_first_touch_surface() {
    let files = [
        ("README.md", include_str!("../README.md")),
        (
            "docs/FIRST_FIVE_MINUTES.md",
            include_str!("../docs/FIRST_FIVE_MINUTES.md"),
        ),
        (
            "docs/GETTING_STARTED_SERIOUS.md",
            include_str!("../docs/GETTING_STARTED_SERIOUS.md"),
        ),
        (
            "docs/WRITING_HOPPER_PROGRAMS.md",
            include_str!("../docs/WRITING_HOPPER_PROGRAMS.md"),
        ),
        (
            "docs/MIGRATION_FROM_QUASAR.md",
            include_str!("../docs/MIGRATION_FROM_QUASAR.md"),
        ),
        (
            "docs/PORT_QUASAR_IN_20_MINUTES.md",
            include_str!("../docs/PORT_QUASAR_IN_20_MINUTES.md"),
        ),
        (
            "docs/DYNAMIC_TAILS_FROM_QUASAR.md",
            include_str!("../docs/DYNAMIC_TAILS_FROM_QUASAR.md"),
        ),
        ("docs/WHY_HOPPER.md", include_str!("../docs/WHY_HOPPER.md")),
        (
            "docs/TOKEN_2022_GUIDE.md",
            include_str!("../docs/TOKEN_2022_GUIDE.md"),
        ),
        (
            "docs/HOPPER_VS_ANCHOR_QUASAR_PINOCCHIO.md",
            include_str!("../docs/HOPPER_VS_ANCHOR_QUASAR_PINOCCHIO.md"),
        ),
        (
            "tools/hopper-cli/src/cmd/add.rs",
            include_str!("../tools/hopper-cli/src/cmd/add.rs"),
        ),
        (
            "examples/hopper-counter/src/lib.rs",
            include_str!("../examples/hopper-counter/src/lib.rs"),
        ),
        (
            "examples/hopper-vault/src/lib.rs",
            include_str!("../examples/hopper-vault/src/lib.rs"),
        ),
        (
            "examples/hopper-escrow/src/lib.rs",
            include_str!("../examples/hopper-escrow/src/lib.rs"),
        ),
    ];

    let terms = legacy_surface_terms();
    let mut violations = Vec::new();

    for (path, contents) in files {
        for term in &terms {
            if contents.contains(term) {
                violations.push(format!("{path} contains {term:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "public docs/templates should lead with Ctx<T>, ctx.accounts.*, and typed wrappers:\n{}",
        violations.join("\n")
    );
}
