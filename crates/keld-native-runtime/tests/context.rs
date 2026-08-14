use keld_native_abi::{FaultKind, KeldFault, RuntimeStatus};
use keld_native_runtime::RuntimeContext;

#[test]
fn context_records_only_the_first_failure() {
    let mut context = RuntimeContext::new().expect("context setup");
    assert_eq!(context.status(), RuntimeStatus::Ok);
    context.record_language_fault(FaultKind::Bounds, 9);
    context.record_language_fault(FaultKind::Allocation, 10);
    assert_eq!(context.status(), RuntimeStatus::LanguageFault);
    assert_eq!(
        context.first_failure(),
        Some(KeldFault {
            kind: 6,
            location: 9
        })
    );
}

#[test]
fn context_exposes_the_root_lifecycle_as_a_fixed_record() {
    let context = RuntimeContext::new().expect("context setup");
    let root = context.root_lifecycle();
    assert_ne!(root.brand, 0);
    assert_eq!(root.index, 0);
    assert_eq!(root.reserved, 0);
}
