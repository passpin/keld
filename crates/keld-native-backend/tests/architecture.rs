#[test]
fn backend_dependency_boundary_excludes_semantic_policy_crates() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "keld-flow",
        "keld-lifecycle",
        "keld-storage",
        "keld-semantics",
        "keld-interpreter",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "backend depends on {forbidden}"
        );
    }
    for required in [
        "keld-ir",
        "keld-native-abi",
        "keld-native-llvm",
        "keld-source",
    ] {
        assert!(manifest.contains(required), "backend is missing {required}");
    }
}

#[test]
fn backend_source_does_not_reconstruct_policy() {
    let source = include_str!("../src/lib.rs");
    for forbidden in [
        "keld_flow",
        "keld_lifecycle",
        "keld_storage",
        "keld_semantics",
        "keld_interpreter",
    ] {
        assert!(!source.contains(forbidden), "backend imports {forbidden}");
    }
}
