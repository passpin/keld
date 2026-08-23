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

/// Every current executable-IR variant is tied to a differential fixture.
/// `source:` entries are compiled from Keld source once and then run through
/// the interpreter/native O0/O2 harness; `ir:` entries name the focused
/// validated-IR builders in the differential harness for variants that the
/// source lowering deliberately normalizes away (direct List forms, explicit
/// Fault, Phi, and explicit cleanup edges).
const SOURCE_IR_DIFFERENTIAL_FIXTURES: &[(&str, &[&str], &[&str])] = &[
    (
        "source:not_surface",
        &["ConstBool", "Not"],
        &["Branch", "Goto", "Return"],
    ),
    (
        "source:numeric_edges",
        &["ConstInt", "CheckedUnaryInt", "CheckedBinaryInt", "Compare"],
        &["Unreachable"],
    ),
    (
        "source:text_surface",
        &[
            "ConstText",
            "TextByteLength",
            "TextIsEmpty",
            "TextConcat",
            "Compare",
        ],
        &[],
    ),
    (
        "source:struct_surface",
        &[
            "ConstructStruct",
            "ReadStructField",
            "InstallHome",
            "MoveHome",
            "DropHome",
            "DropSlot",
        ],
        &[],
    ),
    (
        "source:nested_projected_surface",
        &[
            "ListIndex",
            "ListPushPlace",
            "ListRemovePlace",
            "ListReplace",
        ],
        &[],
    ),
    (
        "source:list_surface",
        &[
            "ListNew",
            "ListLength",
            "ListGet",
            "ListTryRemove",
            "ListClear",
            "ListReserve",
            "ListTryReserve",
        ],
        &[],
    ),
    (
        "source:entity_surface",
        &[
            "Copy",
            "BeginLifecycle",
            "EndLifecycle",
            "AllocateEntity",
            "EntityToLink",
            "OpenView",
            "ReadField",
            "WriteField",
            "CloseView",
            "ReplaceField",
            "KeepEntity",
            "RetireEntity",
            "ConstNoneLink",
        ],
        &["ResolveLink"],
    ),
    ("source:call_surface", &["Call"], &[]),
    ("source:replace_place_surface", &["ReplacePlace"], &[]),
    ("source:replace_field_surface", &["ReplaceField"], &[]),
    (
        "ir:differential::take_cleanup_ir",
        &["Take", "DropIfLive"],
        &[],
    ),
    (
        "ir:differential::cleanup_scope_ir",
        &["CleanupTrackedScope"],
        &[],
    ),
    (
        "ir:differential::direct_list_ir",
        &["ListPush", "ListRemove"],
        &[],
    ),
    ("ir:differential::fault_ir", &[], &["Fault"]),
    ("ir:differential::phi_ir", &["Phi"], &[]),
];

const KNOWN_DIFFERENTIAL_FIXTURES: &[&str] = &[
    "source:not_surface",
    "source:numeric_edges",
    "source:text_surface",
    "source:struct_surface",
    "source:nested_projected_surface",
    "source:list_surface",
    "source:entity_surface",
    "source:call_surface",
    "source:take_surface",
    "source:replace_field_surface",
    "source:replace_place_surface",
    "source:normal_managed_list_parameter",
    "source:normal_managed_struct_parameter",
    "ir:differential::take_cleanup_ir",
    "ir:differential::cleanup_scope_ir",
    "ir:differential::direct_list_ir",
    "ir:differential::fault_ir",
    "ir:differential::phi_ir",
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

#[test]
fn every_ir_variant_has_a_named_source_or_ir_differential_fixture() {
    let mut instruction_fixtures = std::collections::BTreeMap::<&str, &str>::new();
    let mut terminator_fixtures = std::collections::BTreeMap::<&str, &str>::new();
    for (fixture, instructions, terminators) in SOURCE_IR_DIFFERENTIAL_FIXTURES {
        assert!(fixture.starts_with("source:") || fixture.starts_with("ir:"));
        assert!(
            KNOWN_DIFFERENTIAL_FIXTURES.contains(fixture),
            "unknown differential fixture {fixture}"
        );
        for instruction in *instructions {
            assert!(
                INSTRUCTIONS.contains(instruction),
                "unknown instruction {instruction}"
            );
            instruction_fixtures.entry(instruction).or_insert(fixture);
        }
        for terminator in *terminators {
            assert!(
                TERMINATORS.contains(terminator),
                "unknown terminator {terminator}"
            );
            terminator_fixtures.entry(terminator).or_insert(fixture);
        }
    }
    for instruction in INSTRUCTIONS {
        assert!(
            instruction_fixtures.contains_key(instruction),
            "missing differential fixture for {instruction}"
        );
    }
    for terminator in TERMINATORS {
        assert!(
            terminator_fixtures.contains_key(terminator),
            "missing differential fixture for {terminator}"
        );
    }
}
