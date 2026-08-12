use keld_runtime::{RuntimeTypeId, Store, StoreError, StoreInvariantError};

#[test]
fn lifecycle_cleanup_is_reverse_adoption_order() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    store.allocate(RuntimeTypeId(0), root, 1).unwrap();
    let child = store.begin_lifecycle(root).unwrap();
    let kept = store.allocate(RuntimeTypeId(0), child, 2).unwrap();
    store.keep(kept, root).unwrap();
    store.allocate(RuntimeTypeId(0), root, 3).unwrap();
    let mut cleaned = Vec::new();

    store
        .end_lifecycle_with(child, |payload| cleaned.push(payload))
        .unwrap();
    store.finish_with(|payload| cleaned.push(payload)).unwrap();
    assert_eq!(cleaned, vec![3, 2, 1]);
}

#[test]
fn parent_cannot_end_before_its_active_child() {
    let mut store = Store::<u32>::new().unwrap();
    let root = store.root_lifecycle();
    let parent = store.begin_lifecycle(root).unwrap();
    let _child = store.begin_lifecycle(parent).unwrap();

    assert_eq!(
        store.end_lifecycle_with(parent, drop),
        Err(StoreError::InvalidOperation(
            StoreInvariantError::ActiveChild
        ))
    );
}

#[test]
fn lifecycle_end_stales_links_and_finish_runs_once() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    let child = store.begin_lifecycle(root).unwrap();
    let entity = store.allocate(RuntimeTypeId(0), child, 5_u32).unwrap();
    let link = store.link(entity).unwrap();

    store.end_lifecycle_with(child, drop).unwrap();
    assert_eq!(store.resolve(link), None);
    store.finish_with(drop).unwrap();
    assert_eq!(
        store.finish_with(drop),
        Err(StoreError::InvalidOperation(
            StoreInvariantError::AlreadyFinished
        ))
    );
}

#[test]
fn keep_rejects_the_same_or_descendant_lifecycle() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    let child = store.begin_lifecycle(root).unwrap();
    let entity = store.allocate(RuntimeTypeId(0), child, 1_u32).unwrap();

    assert_eq!(
        store.keep(entity, child),
        Err(StoreError::InvalidOperation(
            StoreInvariantError::NonAncestorKeep
        ))
    );
}
