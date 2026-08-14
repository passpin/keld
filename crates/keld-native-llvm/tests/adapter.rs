use keld_native_llvm::{LLVM_SYS_VERSION, initialize_x86_target};

#[test]
fn adapter_is_versioned_and_has_a_safe_entrypoint() {
    assert_eq!(LLVM_SYS_VERSION, "221.0.1");
    initialize_x86_target();
}
