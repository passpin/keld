#[test]
fn text_content_loop_ir_is_valid() {
    let source = r#"
fn main() -> Int {
    let a = "abcdefghijklmnopqrstuvwxyz"
    let b = "!"
    let expected = "abcdefghijklmnopqrstuvwxyz!"
    var i = 0
    var count = 0
    while i < 1000000 {
        if (a + b) == expected {
            count += 1
        } else {
            count += 7
        }
        i += 1
    }
    return count + i
}
"#;
    let flow = keld_flow::lower_text_for_test(source).expect("flow lowering");
    let lifecycle = keld_lifecycle::verify(flow)
        .module
        .expect("lifecycle verification");
    let storage = keld_storage::verify(lifecycle)
        .module
        .expect("storage verification");
    let module = keld_ir::lower(&storage);
    println!("TEXTIR_MODULE {module:#?}");
    let diagnostics = keld_ir::validate(&module);
    println!("TEXTIR_DIAGNOSTICS {diagnostics:#?}");
    assert!(
        diagnostics.is_empty(),
        "generated Text-content IR is invalid"
    );
}
