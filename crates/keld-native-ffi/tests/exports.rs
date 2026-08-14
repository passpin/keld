use keld_native_abi::KeldFault;
use keld_native_ffi::{
    keld_rt_v1_abi_version, keld_rt_v1_context_destroy, keld_rt_v1_context_fault,
    keld_rt_v1_context_new, keld_rt_v1_context_status, keld_rt_v1_print_int,
};

#[test]
fn adapter_exports_only_versioned_symbols() {
    assert_eq!(keld_rt_v1_abi_version(), 1);
    assert_eq!(keld_rt_v1_print_int(7), 0);
    let context = keld_rt_v1_context_new();
    assert!(!context.is_null());
    assert_eq!(keld_rt_v1_context_status(context), 0);
    let mut fault = KeldFault::default();
    assert_eq!(keld_rt_v1_context_fault(context, &raw mut fault), 0);
    assert_eq!(fault, KeldFault::default());
    assert_eq!(keld_rt_v1_context_destroy(context), 0);
}
