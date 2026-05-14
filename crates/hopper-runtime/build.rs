fn main() {
    const BACKENDS: [(&str, &str); 3] = [
        (
            "CARGO_FEATURE_HOPPER_NATIVE_BACKEND",
            "hopper-native-backend",
        ),
        (
            "CARGO_FEATURE_LEGACY_PINOCCHIO_COMPAT",
            "legacy-pinocchio-compat",
        ),
        (
            "CARGO_FEATURE_SOLANA_PROGRAM_BACKEND",
            "solana-program-backend",
        ),
    ];

    let enabled: Vec<&'static str> = BACKENDS
        .iter()
        .filter_map(|(env, name)| std::env::var_os(env).map(|_| *name))
        .collect();

    if enabled.len() == 1 {
        return;
    }

    let enabled = if enabled.is_empty() {
        "none".to_string()
    } else {
        enabled.join(", ")
    };

    eprintln!(
        "Enable exactly one Hopper backend for hopper-runtime: \
         hopper-native-backend, legacy-pinocchio-compat, or solana-program-backend. \
         Enabled: {enabled}"
    );
    std::process::exit(1);
}
