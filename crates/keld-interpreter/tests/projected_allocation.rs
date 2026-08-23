use keld_interpreter::Interpreter;
use keld_ir::{Instruction, Module};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use std::sync::Mutex;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
static ALLOCATION_TEST_LOCK: Mutex<()> = Mutex::new(());

fn lower_module(source: &str) -> Module {
    let verification = keld_storage::verify_text_for_test(source);
    let verified = verification
        .module
        .unwrap_or_else(|| panic!("source must verify: {:#?}", verification.diagnostics));
    keld_ir::lower(&verified)
}

fn allocation_counts(source: &str) -> (usize, usize) {
    let _guard = ALLOCATION_TEST_LOCK.lock().expect("allocation test lock");
    let module = lower_module(source);
    let mut interpreter = Interpreter::new(&module).expect("compiler IR validates");
    let mut region: Option<Region<'_, System>> = None;
    let mut operation_change = None;
    let mut hook = |instruction: &Instruction| {
        if let Some(region) = region.take() {
            let change = region.change();
            operation_change = Some((change.allocations, change.reallocations));
        }
        if matches!(instruction, Instruction::ListTryRemove { .. }) {
            assert!(
                operation_change.is_none() && region.is_none(),
                "source has multiple try_remove instructions"
            );
            region = Some(Region::new(GLOBAL));
        }
    };

    let result = interpreter
        .run_main_with_test_hook(&mut hook)
        .expect("projected try_remove executes");
    std::hint::black_box(result);
    operation_change.expect("try_remove instruction was followed by an instruction")
}

#[test]
fn projected_struct_field_try_remove_allocates_nothing_after_setup() {
    assert_eq!(
        allocation_counts(
            "struct Holder {\nitems: List[Int]\n}\nfn main() -> Int { let holder = Holder(items: List()); holder.items.push(7); let removed = holder.items.try_remove(0); return holder.items.length }\n",
        ),
        (0, 0)
    );
}

#[test]
fn projected_entity_field_try_remove_allocates_nothing_after_setup() {
    assert_eq!(
        allocation_counts(
            "entity Holder {\nitems: List[Int]\n}\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); holder.items.push(7); let removed = holder.items.try_remove(0); return holder.items.length; }; }\n",
        ),
        (0, 0)
    );
}
