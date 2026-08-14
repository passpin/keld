use keld_native_runtime::{NativeValueError, RuntimeContext};

#[test]
fn projected_places_resolve_and_replace_nested_values_synchronously() {
    let mut context = RuntimeContext::new().expect("context");
    let inner = context.list_new().expect("inner list");
    context
        .list_push(inner, RuntimeContext::int_value(4), false)
        .expect("first element");
    context
        .list_push(inner, RuntimeContext::int_value(8), false)
        .expect("second element");
    let root = context
        .struct_new(1, &[inner], &[true])
        .expect("root struct");
    let steps = [
        keld_native_abi::KeldPlaceStep {
            kind: 0,
            reserved: 0,
            value: 0,
        },
        keld_native_abi::KeldPlaceStep {
            kind: 1,
            reserved: 0,
            value: 1,
        },
    ];
    assert_eq!(
        context.place_resolve(root, None, &steps),
        Ok(RuntimeContext::int_value(8))
    );
    let (displaced, managed) = context
        .place_replace(root, None, &steps, RuntimeContext::int_value(9), false)
        .expect("replace projected value");
    assert_eq!(displaced, RuntimeContext::int_value(8));
    assert!(!managed);
    assert_eq!(
        context.place_resolve(root, None, &steps),
        Ok(RuntimeContext::int_value(9))
    );
    context.drop_managed(root).expect("drop root");
}

#[test]
fn generational_managed_struct_copy_and_drop_reject_stale_handles() {
    let mut context = RuntimeContext::new().expect("context");
    let text = context.text_new(b"hello").expect("text");
    let scalar = RuntimeContext::int_value(7);
    let structure = context
        .struct_new(4, &[scalar, text], &[false, true])
        .expect("struct");
    let copy = context.copy_managed(structure).expect("deep copy");
    let (copied_text, managed) = context.struct_field(copy, 1).expect("field");
    assert!(managed);
    assert_ne!(copied_text.words[0], text.words[0]);
    context.drop_managed(structure).expect("drop original");
    context.drop_managed(copy).expect("drop copy");
    assert_eq!(
        context.drop_managed(structure),
        Err(NativeValueError::InvalidHandle)
    );
}

#[test]
fn take_value_clears_only_the_source_envelope() {
    let mut source = RuntimeContext::int_value(-9);
    let moved = RuntimeContext::take_value(&mut source);
    assert_eq!(moved.words[0], (-9_i64).cast_unsigned());
    assert_eq!(source, keld_native_abi::KeldValue::default());
}

#[test]
fn list_mutations_preserve_bounds_and_deep_copy_elements() {
    let mut context = RuntimeContext::new().expect("context");
    let list = context.list_new().expect("list");
    let text = context.text_new(b"x").expect("text");
    context.list_push(list, text, true).expect("push");
    assert_eq!(context.list_length(list), Ok(1));
    let copied = context.copy_managed(list).expect("copy list");
    let (copied_text, managed) = context.list_get(copied, 0).expect("copied element");
    assert!(managed);
    assert_ne!(copied_text.words[0], text.words[0]);
    assert_eq!(context.list_get(list, 1), Err(NativeValueError::Bounds));
    let (removed, removed_managed) = context.list_remove(list, 0).expect("remove");
    assert!(removed_managed);
    context.drop_managed(removed).expect("removed text");
    context.drop_managed(list).expect("drop list");
    context.drop_managed(copied).expect("drop copied list");
}

#[test]
fn try_reserve_returns_false_for_negative_or_impossible_capacity() {
    let mut context = RuntimeContext::new().expect("context");
    let list = context.list_new().expect("list");
    assert_eq!(context.list_try_reserve(list, -1), Ok(false));
    assert_eq!(context.list_try_reserve(list, i64::MAX), Ok(false));
    assert_eq!(context.list_length(list), Ok(0));
    context.drop_managed(list).expect("drop list");
}

#[test]
fn reserve_classifies_impossible_byte_capacity_as_capacity_fault() {
    let mut context = RuntimeContext::new().expect("context");
    let list = context.list_new().expect("list");
    assert_eq!(
        context.list_reserve(list, i64::MAX),
        Err(NativeValueError::Capacity)
    );
    context.drop_managed(list).expect("drop list");
}

#[test]
fn reserve_validates_stale_handles_before_capacity_classification() {
    let mut context = RuntimeContext::new().expect("context");
    let list = context.list_new().expect("list");
    context.drop_managed(list).expect("drop list");
    assert_eq!(
        context.list_reserve(list, -1),
        Err(NativeValueError::InvalidHandle)
    );
    assert_eq!(
        context.list_try_reserve(list, -1),
        Err(NativeValueError::InvalidHandle)
    );
}
