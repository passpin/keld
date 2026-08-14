use keld_native_ffi::keld_rt_v1_abi_version;

#[test]
fn adapter_exports_only_versioned_symbols() {
    assert_eq!(keld_rt_v1_abi_version(), 1);
}
