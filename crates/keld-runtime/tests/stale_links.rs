use keld_runtime::{RuntimeTypeId, Store, StoreError, StoreInvariantError};

#[test]
fn identities_and_lifecycles_cannot_cross_store_brands() {
    let mut first = Store::new().unwrap();
    let mut second = Store::new().unwrap();
    let entity = first
        .allocate(RuntimeTypeId(0), first.root_lifecycle(), 1_u32)
        .unwrap();

    assert_eq!(
        second.read(entity, |payload| *payload),
        Err(StoreError::InvalidOperation(
            StoreInvariantError::ForeignIdentity
        ))
    );
    assert_eq!(
        second.allocate(RuntimeTypeId(0), first.root_lifecycle(), 2_u32),
        Err(StoreError::InvalidOperation(
            StoreInvariantError::InvalidLifecycle
        ))
    );
}
