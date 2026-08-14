use keld_native_runtime::{NativeValueError, RuntimeContext};

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
