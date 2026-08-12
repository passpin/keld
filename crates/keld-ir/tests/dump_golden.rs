use keld_ir::lower;
use keld_lifecycle::verify_text_for_test;

#[test]
fn executable_ir_dump_is_byte_stable_and_uses_numeric_ids() {
    let verified = verify_text_for_test(
        "fn add(a: Int, b: Int) -> Int { return a + b }\nfn main() -> Int { return add(1, 2) }\n",
    )
    .module
    .expect("program must verify");
    let module = lower(&verified);

    let first = module.dump();
    let second = module.dump();
    assert_eq!(first, second);
    assert!(first.contains("function f0"));
    assert!(first.contains("block b0"));
    assert!(first.contains("@s0:"));
    assert!(!first.contains("0x"));
}
