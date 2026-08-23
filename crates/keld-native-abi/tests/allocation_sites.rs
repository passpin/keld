use keld_native_abi::{AllocationPhase, TEST_CONTROL_SCHEMA_VERSION, allocation_site_id};

#[test]
fn allocation_site_ids_are_frozen_and_phase_sensitive() {
    assert_eq!(TEST_CONTROL_SCHEMA_VERSION, 1);
    let base = 37;
    let text = allocation_site_id(base, AllocationPhase::Text, 0);
    let handle = allocation_site_id(base, AllocationPhase::Handle, 0);
    let nested = allocation_site_id(base, AllocationPhase::Copy, 3);
    assert_ne!(text, handle);
    assert_ne!(nested, allocation_site_id(base, AllocationPhase::Copy, 2));
    assert_eq!(text, allocation_site_id(base, AllocationPhase::Text, 0));
}
