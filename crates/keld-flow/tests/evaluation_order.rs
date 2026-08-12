use keld_flow::{FlowOp, lower_text_for_test};

#[test]
fn call_arguments_are_materialized_left_to_right() {
    let flow = lower_text_for_test(
        "fn pick(a: Int, b: Int) -> Int { return b }\nfn main() -> Int { return pick(1 + 2, 3 * 4) }\n",
    )
    .unwrap();
    let ops = flow
        .function_named("main")
        .unwrap()
        .linear_ops()
        .into_iter()
        .filter(|operation| {
            !matches!(
                operation,
                FlowOp::BeginCall { .. } | FlowOp::ReserveArgument { .. }
            )
        })
        .collect::<Vec<_>>();

    assert!(matches!(ops[0], FlowOp::ConstInt { value: 1, .. }));
    assert!(matches!(ops[1], FlowOp::ConstInt { value: 2, .. }));
    assert!(matches!(ops[2], FlowOp::BinaryInt { .. }));
    assert!(matches!(ops[3], FlowOp::ConstInt { value: 3, .. }));
    assert!(matches!(ops[4], FlowOp::ConstInt { value: 4, .. }));
    assert!(matches!(ops[5], FlowOp::BinaryInt { .. }));
    assert!(matches!(ops[6], FlowOp::Call { .. }));
}

#[test]
fn constructor_field_ids_do_not_reorder_source_evaluation() {
    let flow = lower_text_for_test(
        "struct Pair {\nfirst: Int\nsecond: Int\n}\nfn mark(x: Int) -> Int { return x }\nfn main() -> Int { let p = Pair(second: mark(2), first: mark(1)); return p.first }\n",
    )
    .unwrap();
    let ops = flow.function_named("main").unwrap().linear_ops();
    let calls = ops
        .iter()
        .filter_map(|operation| match operation {
            FlowOp::Call { arguments, .. } => Some(arguments[0].1),
            _ => None,
        })
        .collect::<Vec<_>>();
    let construction = ops
        .iter()
        .find_map(|operation| match operation {
            FlowOp::ConstructStruct { fields, .. } => Some(fields),
            _ => None,
        })
        .unwrap();

    assert_eq!(calls.len(), 2);
    assert_eq!(construction[0].0.0, 1);
    assert_eq!(construction[1].0.0, 0);
}

#[test]
fn assignment_materializes_identity_then_rhs_then_writes() {
    let flow = lower_text_for_test(
        "entity Counter {\nvalue: Int\n}\nfn replacement() -> Int { return 9 }\nfn main() -> Int { lifecycle level { let counter = Counter(value: 0); counter.value = replacement(); return counter.value } }\n",
    )
    .unwrap();
    let ops = flow.function_named("main").unwrap().linear_ops();
    let target_copy = ops
        .iter()
        .position(|operation| matches!(operation, FlowOp::CopyLocal { .. }))
        .unwrap();
    let call = ops
        .iter()
        .position(|operation| matches!(operation, FlowOp::Call { .. }))
        .unwrap();
    let write = ops
        .iter()
        .position(|operation| matches!(operation, FlowOp::WriteEntityField { .. }))
        .unwrap();

    assert!(target_copy < call && call < write);
}

#[test]
fn compound_assignment_reads_old_value_before_rhs() {
    let flow = lower_text_for_test(
        "entity Counter {\nvalue: Int\n}\nfn replacement() -> Int { return 9 }\nfn main() -> Int { lifecycle level { let counter = Counter(value: 0); counter.value += replacement(); return counter.value } }\n",
    )
    .unwrap();
    let ops = flow.function_named("main").unwrap().linear_ops();
    let read = ops
        .iter()
        .position(|operation| matches!(operation, FlowOp::ReadEntityField { .. }))
        .unwrap();
    let call = ops
        .iter()
        .position(|operation| matches!(operation, FlowOp::Call { .. }))
        .unwrap();
    let calculation = ops
        .iter()
        .rposition(|operation| matches!(operation, FlowOp::BinaryInt { .. }))
        .unwrap();
    let write = ops
        .iter()
        .position(|operation| matches!(operation, FlowOp::WriteEntityField { .. }))
        .unwrap();

    assert!(read < call && call < calculation && calculation < write);
}

#[test]
fn boolean_operators_use_control_flow_and_one_phi() {
    let flow = lower_text_for_test(
        "fn truth() -> Bool { return true }\nfn main() -> Int { let both = false && truth(); let either = true || truth(); if both || either { return 1 } else { return 0 } }\n",
    )
    .unwrap();
    let function = flow.function_named("main").unwrap();

    assert_eq!(
        function
            .linear_ops()
            .iter()
            .filter(|operation| matches!(operation, FlowOp::Phi { .. }))
            .count(),
        3
    );
    assert!(function.blocks.len() >= 10);
}

#[test]
fn take_lowers_to_an_explicit_local_transfer() {
    let flow = lower_text_for_test(
        "fn take_items(take items: List[Int]) -> List[Int] { return take items }\nfn main() -> Int { return 0 }\n",
    )
    .unwrap();
    let ops = flow.function_named("take_items").unwrap().linear_ops();

    assert!(matches!(ops[0], FlowOp::TakeLocal { .. }));
}
