use keld_interpreter::{Interpreter, InterpreterFailure, RuntimeFaultKind, TestControls};
use keld_ir::{AllocationPhase, AllocationSchedule, Instruction};
use keld_source::Span;

fn compile(source: &str) -> keld_ir::Module {
    let flow = keld_flow::lower_text_for_test(source).expect("source flow");
    let verified = keld_lifecycle::verify(flow)
        .module
        .expect("lifecycle verification");
    let storage = keld_storage::verify(verified)
        .module
        .expect("storage verification");
    keld_ir::lower(&storage)
}

#[test]
fn frozen_site_schedule_controls_interpreter_at_the_same_site_as_native() {
    let module = compile(
        "fn main() -> Int { let value: Text = \"heap-text-for-schedule!\"; return value.byte_length; }\n",
    );
    let schedule = AllocationSchedule::from_module(&module);
    let main = module
        .functions
        .iter()
        .find(|function| function.id == module.main)
        .expect("main function");
    let base = schedule
        .base_id(main.id, main.entry, 0)
        .expect("entry allocation coordinate");
    let text_site = schedule.site_id(base, AllocationPhase::Text, 0);
    let mut interpreter = Interpreter::with_controls_for_test(
        &module,
        TestControls::fail_allocation_schedule([(text_site, AllocationPhase::Text, 1)]),
    )
    .expect("validated executable IR");
    let run = interpreter.run_main();
    let failure = run.expect_err("the frozen Text site must fail");
    assert!(matches!(
        &failure,
        InterpreterFailure::Runtime(fault) if fault.kind == RuntimeFaultKind::Allocation
    ));
    let expected_span = match &main.blocks[0].instructions[0] {
        Instruction::ConstText { span, .. } => *span,
        instruction => panic!("unexpected first instruction: {instruction:?}"),
    };
    assert_eq!(failure_span(&failure), expected_span);
    assert_eq!(
        interpreter
            .allocation_observations_for_test()
            .first()
            .map(|observation| observation.site_id),
        Some(schedule.site_id(base, AllocationPhase::Context, 0))
    );
}

fn failure_span(failure: &InterpreterFailure) -> Span {
    match failure {
        InterpreterFailure::Runtime(fault) => fault.span,
        InterpreterFailure::Internal(error) => panic!("unexpected internal failure: {error}"),
    }
}
