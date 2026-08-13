use keld_interpreter::{RuntimeList, Value};
use keld_ir::{IrType, Module};
use keld_semantics::FunctionId;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
fn successful_try_remove_and_optional_wrap_allocate_nothing() {
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
