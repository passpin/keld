use keld_ir::lower;
use keld_storage::verify_text_for_test;

#[test]
fn executable_ir_dump_is_byte_stable_and_uses_numeric_ids() {
    let verified = verify_text_for_test(
        "fn size(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { let value: Text = \"Keld\"; return size(value) }\n",
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
    assert!(first.contains("install-home"));
    assert!(first.contains("drop-home"));
    assert!(!first.contains("0x"));
}
