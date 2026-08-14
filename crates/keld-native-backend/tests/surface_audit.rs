use std::fs;

const INSTRUCTIONS: &[&str] = &[
    "ConstInt",
    "ConstBool",
    "ConstText",
    "ConstNoneLink",
    "Copy",
    "Take",
    "InstallHome",
    "MoveHome",
    "DropHome",
    "DropIfLive",
    "DropSlot",
    "CleanupTrackedScope",
    "ReplacePlace",
    "ReplaceField",
    "ListNew",
    "ListLength",
    "ListPush",
    "ListPushPlace",
    "ListRemove",
    "ListRemovePlace",
    "ListIndex",
    "ListGet",
    "ListReplace",
    "ListTryRemove",
    "ListClear",
    "ListReserve",
    "ListTryReserve",
    "TextByteLength",
    "TextIsEmpty",
    "TextConcat",
    "CheckedUnaryInt",
    "CheckedBinaryInt",
    "Not",
    "Compare",
    "Phi",
    "ConstructStruct",
    "ReadStructField",
    "BeginLifecycle",
    "EndLifecycle",
    "AllocateEntity",
    "EntityToLink",
    "OpenView",
    "ReadField",
    "WriteField",
    "CloseView",
    "KeepEntity",
    "RetireEntity",
    "Call",
];

const TERMINATORS: &[&str] = &[
    "Goto",
    "Branch",
    "ResolveLink",
    "Return",
    "Fault",
    "Unreachable",
];

#[test]
fn native_surface_is_explicit_and_exhaustive() {
    let ir_source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../keld-ir/src/instruction.rs"),
    )
    .expect("IR instruction source");
    let backend_source =
        fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("backend source");
    assert_eq!(INSTRUCTIONS.len(), 48);
    assert_eq!(TERMINATORS.len(), 6);
    for variant in INSTRUCTIONS {
        assert!(
            ir_source.contains(&format!("    {variant} {{")),
            "IR variant {variant}"
        );
        assert!(
            backend_source.contains(&format!("Instruction::{variant}")),
            "backend lowering arm for {variant}"
        );
    }
    for variant in TERMINATORS {
        assert!(
            ir_source.contains(&format!("    {variant}")),
            "IR terminator {variant}"
        );
        assert!(
            backend_source.contains(&format!("Terminator::{variant}")),
            "backend lowering arm for {variant}"
        );
    }
}
