use keld_native_runtime::RuntimeContext;

#[test]
fn text_concat_preserves_inputs_and_contents() {
    let mut context = RuntimeContext::new().expect("context");
    let lhs = context
        .text_new(b"abcdefghijklmnopqrstuvwxyz")
        .expect("lhs text");
    let rhs = context.text_new(b"!").expect("rhs text");

    let concatenated = context.text_concat(lhs, rhs).expect("concat");

    assert_eq!(
        context.text_bytes(concatenated),
        Ok(&b"abcdefghijklmnopqrstuvwxyz!"[..])
    );
    assert_eq!(
        context.text_bytes(lhs),
        Ok(&b"abcdefghijklmnopqrstuvwxyz"[..])
    );
    assert_eq!(context.text_bytes(rhs), Ok(&b"!"[..]));

    context.drop_managed(lhs).expect("drop lhs");
    context.drop_managed(rhs).expect("drop rhs");
    context
        .drop_managed(concatenated)
        .expect("drop concatenated text");
}

#[test]
fn text_concat_does_not_snapshot_input_buffers() {
    let source = include_str!("../src/lib.rs");
    let start = source
        .find("    pub fn text_concat(\n")
        .expect("text_concat function");
    let tail = &source[start..];
    let end = tail
        .find("    pub fn text_bytes(")
        .expect("text_bytes follows text_concat");
    let implementation = &tail[..end];

    assert!(
        !implementation.contains(".to_vec()"),
        "text_concat must copy directly into its result buffer instead of allocating input snapshots"
    );
}
