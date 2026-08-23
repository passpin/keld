use keld_interpreter::{Interpreter, RuntimeList, Value};
use keld_ir::{Instruction, IrType, Module};
use keld_semantics::FunctionId;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use std::sync::Mutex;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static ALLOCATION_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn successful_try_remove_and_optional_wrap_allocate_nothing() {
    let _guard = ALLOCATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut list = RuntimeList::from_values(vec![Value::Int(7)]);
    let module = Module {
        definitions: Vec::new(),
        functions: Vec::new(),
        main: FunctionId(0),
    };
    let region = Region::new(GLOBAL);

    let removed = list.try_remove(0).expect("element exists");
    let wrapped = removed
        .into_optional_some(&module, &IrType::Int)
        .expect("removed value matches the element type");
    std::hint::black_box(&wrapped);
    let change = region.change();

    assert_eq!(change.allocations, 0, "{change:#?}");
    assert_eq!(change.reallocations, 0, "{change:#?}");
}

#[derive(Clone, Copy)]
enum MeasuredInstruction {
    ListIndex,
    ListGet,
}

fn allocation_counts(source: &str, measured: MeasuredInstruction) -> (usize, usize) {
    let verification = keld_storage::verify_text_for_test(source);
    let verified = verification
        .module
        .unwrap_or_else(|| panic!("source must verify: {:#?}", verification.diagnostics));
    let module = keld_ir::lower(&verified);
    let mut interpreter = Interpreter::new(&module).expect("compiler IR validates");
    let mut region: Option<Region<'_, System>> = None;
    let mut operation_change = None;
    let mut hook = |instruction: &Instruction| {
        if let Some(region) = region.take() {
            let change = region.change();
            operation_change = Some((change.allocations, change.reallocations));
        }
        let is_measured = matches!(
            (measured, instruction),
            (
                MeasuredInstruction::ListIndex,
                Instruction::ListIndex { .. }
            ) | (MeasuredInstruction::ListGet, Instruction::ListGet { .. })
        );
        if is_measured {
            assert!(
                operation_change.is_none() && region.is_none(),
                "source has multiple measured instructions"
            );
            region = Some(Region::new(GLOBAL));
        }
    };

    let result = interpreter
        .run_main_with_test_hook(&mut hook)
        .expect("measured List operation executes");
    std::hint::black_box(result);
    operation_change.expect("measured instruction was followed by an instruction")
}

#[test]
fn list_get_optional_wrap_adds_no_allocation_to_list_index_copy() {
    let _guard = ALLOCATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let index = allocation_counts(
        "fn main() -> Int { let items: List[Int] = List(); items.push(7); let value = items[0]; return value; }\n",
        MeasuredInstruction::ListIndex,
    );
    let get = allocation_counts(
        "fn main() -> Int { let items: List[Int] = List(); items.push(7); let value = items.get(0); return 0; }\n",
        MeasuredInstruction::ListGet,
    );

    assert_eq!(get, index, "ListGet {get:?} versus ListIndex {index:?}");
}
