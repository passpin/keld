use keld_runtime::{RuntimeTypeId, Store};

#[test]
fn retired_link_never_resolves_after_slot_reuse() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    let first = store.allocate(RuntimeTypeId(0), root, 10_u32).unwrap();
    let stale = store.link(first).unwrap();
    store.retire_with(first, |_| {}).unwrap();
    let second = store.allocate(RuntimeTypeId(0), root, 20_u32).unwrap();

    assert_eq!(first.slot_index(), second.slot_index());
    assert_ne!(first.generation(), second.generation());
    assert_eq!(store.resolve(stale), None);
    assert_eq!(store.read(second, |payload| *payload).unwrap(), 20);
}

#[test]
fn edits_are_closure_bounded_and_preserve_identity() {
    let mut store = Store::new().unwrap();
    let root = store.root_lifecycle();
    let entity = store.allocate(RuntimeTypeId(3), root, 4_i32).unwrap();

    store.edit(entity, |payload| *payload += 5).unwrap();
    assert_eq!(store.read(entity, |payload| *payload).unwrap(), 9);
    assert_eq!(store.resolve(store.link(entity).unwrap()), Some(entity));
}
