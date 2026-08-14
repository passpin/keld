use keld_interpreter::{
    Interpreter, InterpreterFailure, RuntimeFaultKind, TestControls, Value, run_text_for_test,
    run_text_with_controls_for_test,
};
use keld_ir::{
    Function, Instruction, IrBlock, IrBlockId, IrDefinition, IrDefinitionKind, IrType, Module,
    Register, RegisterStorage, Terminator,
};
use keld_semantics::{DefId, FieldId, FunctionId};
use keld_source::{SourceId, Span};

const HEAP_TEXT: &str = "abcdefghijklmnopqrstuvwxyz";

#[test]
fn heap_text_constant_allocation_failure_is_reported_as_allocation_fault() {
    let fault = run_text_with_controls_for_test(
        &format!(
            "fn main() -> Int {{ let value: Text = \"{HEAP_TEXT}\"; return value.byte_length; }}\n"
        ),
        TestControls::fail_text_attempts([1]),
    )
    .expect_err("injected heap Text constant allocation must fault");

    assert_eq!(fault.kind, RuntimeFaultKind::Allocation);
}

#[test]
fn heap_text_copy_allocation_failure_is_reported_as_allocation_fault() {
    let fault = run_text_with_controls_for_test(
        &format!(
            "fn main() -> Int {{ let value: Text = \"{HEAP_TEXT}\"; let copied = value.copy(); return copied.byte_length; }}\n"
        ),
        TestControls::fail_text_attempts([2]),
    )
    .expect_err("injected heap Text copy allocation must fault");

    assert_eq!(fault.kind, RuntimeFaultKind::Allocation);
}

#[test]
fn successful_heap_text_constant_and_copy_preserve_bytes_and_length() {
    let result = run_text_for_test(&format!(
        "fn main() -> Int {{ let value: Text = \"{HEAP_TEXT}\"; let copied = value.copy(); if copied == value {{ return copied.byte_length; }} else {{ return 0; }} }}\n"
    ))
    .expect("heap Text constant and structural copy execute");

    assert_eq!(result.value, Value::Int(26));
}

#[test]
fn projected_text_loan_and_call_place_allocations_report_allocation_fault() {
    let source = format!(
        "struct Holder {{ value: Text }}\nfn size(value: Text) -> Int {{ return value.byte_length; }}\nfn main() -> Int {{ let holder = Holder(value: \"{HEAP_TEXT}\"); return size(holder.value); }}\n"
    );

    for attempt in [1, 2, 3] {
        let fault =
            run_text_with_controls_for_test(&source, TestControls::fail_place_attempts([attempt]))
                .expect_err("projected/call place allocation must fault");
        assert_eq!(
            fault.kind,
            RuntimeFaultKind::Allocation,
            "attempt {attempt}"
        );
    }

    let result = run_text_with_controls_for_test(&source, TestControls::fail_place_attempts([4]))
        .expect("all required place allocations are tracked");
    assert_eq!(result.value, Value::Int(26));
}

#[test]
fn nested_indexed_loan_place_allocation_failure_is_reported() {
    let source = format!(
        "fn size(value: Text) -> Int {{ return value.byte_length; }}\nfn main() -> Int {{ let inner: List[Text] = List(); inner.push(\"{HEAP_TEXT}\"); let outer: List[List[Text]] = List(); outer.push(take inner); return size(outer[0][0]); }}\n"
    );
    let fault = run_text_with_controls_for_test(&source, TestControls::fail_place_attempts([2]))
        .expect_err("nested indexed place allocation must fault");

    assert_eq!(fault.kind, RuntimeFaultKind::Allocation);
}

#[test]
fn projected_replacement_place_failure_is_reported_before_commit() {
    let fault = run_text_with_controls_for_test(
        "struct Holder { value: Text }\nfn main() -> Int { var holder = Holder(value: \"old\"); holder.value = \"new\"; return 0; }\n",
        TestControls::fail_place_attempts([1]),
    )
    .expect_err("projected replacement metadata allocation must fault");

    assert_eq!(fault.kind, RuntimeFaultKind::Allocation);
}

#[test]
fn loan_phi_place_copy_allocation_failure_is_reported() {
    let module = loan_phi_module();
    let mut interpreter =
        Interpreter::with_controls_for_test(&module, TestControls::fail_place_attempts([2]))
            .expect("manual loan-Phi IR validates");
    let failure = interpreter
        .run_main()
        .expect_err("Phi place copy allocation must fault");

    assert!(matches!(
        failure,
        InterpreterFailure::Runtime(fault) if fault.kind == RuntimeFaultKind::Allocation
    ));

    let mut interpreter =
        Interpreter::with_controls_for_test(&module, TestControls::fail_place_attempts([3]))
            .expect("manual loan-Phi IR validates");
    let result = interpreter
        .run_main()
        .expect("read-only use of the Phi loan does not copy its place");
    assert_eq!(result.value, Value::Int(26));
}

#[allow(clippy::too_many_lines)]
fn loan_phi_module() -> Module {
    let span = Span::new(SourceId(0), 0, 1).unwrap();
    let definition = DefId(0);
    let home = RegisterStorage::Home {
        scope: keld_flow::StorageScopeId(0),
        conditional: false,
    };
    Module {
        definitions: vec![IrDefinition {
            id: definition,
            kind: IrDefinitionKind::Struct,
            fields: vec![(FieldId(0), IrType::Text)],
        }],
        functions: vec![Function {
            id: FunctionId(0),
            span,
            parameters: vec![],
            parameter_modes: vec![],
            parameter_effects: vec![],
            current_lifecycle: Register(0),
            register_types: vec![
                IrType::Lifecycle,
                IrType::Text,
                IrType::Struct(definition),
                IrType::Bool,
                IrType::Text,
                IrType::Text,
                IrType::Text,
                IrType::Int,
            ],
            register_storage: vec![
                RegisterStorage::Trivial,
                home.clone(),
                home,
                RegisterStorage::Trivial,
                RegisterStorage::Loan,
                RegisterStorage::Loan,
                RegisterStorage::Loan,
                RegisterStorage::Trivial,
            ],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![
                IrBlock {
                    id: IrBlockId(0),
                    instructions: vec![
                        Instruction::ConstText {
                            dst: Register(1),
                            value: HEAP_TEXT.to_owned(),
                            span,
                        },
                        Instruction::ConstructStruct {
                            dst: Register(2),
                            definition,
                            fields: vec![(FieldId(0), Register(1))],
                            span,
                        },
                        Instruction::ConstBool {
                            dst: Register(3),
                            value: true,
                            span,
                        },
                    ],
                    terminator: Terminator::Branch {
                        condition: Register(3),
                        then_block: IrBlockId(1),
                        else_block: IrBlockId(2),
                    },
                },
                IrBlock {
                    id: IrBlockId(1),
                    instructions: vec![Instruction::ReadStructField {
                        dst: Register(4),
                        base: Register(2),
                        field: FieldId(0),
                        span,
                    }],
                    terminator: Terminator::Goto(IrBlockId(3)),
                },
                IrBlock {
                    id: IrBlockId(2),
                    instructions: vec![Instruction::ReadStructField {
                        dst: Register(5),
                        base: Register(2),
                        field: FieldId(0),
                        span,
                    }],
                    terminator: Terminator::Goto(IrBlockId(3)),
                },
                IrBlock {
                    id: IrBlockId(3),
                    instructions: vec![
                        Instruction::Phi {
                            dst: Register(6),
                            inputs: vec![(IrBlockId(1), Register(4)), (IrBlockId(2), Register(5))],
                            span,
                        },
                        Instruction::TextByteLength {
                            dst: Register(7),
                            text: Register(6),
                            span,
                        },
                        Instruction::DropHome {
                            home: Register(2),
                            span,
                        },
                    ],
                    terminator: Terminator::Return(Some(Register(7))),
                },
            ],
            entry: IrBlockId(0),
        }],
        main: FunctionId(0),
    }
}
