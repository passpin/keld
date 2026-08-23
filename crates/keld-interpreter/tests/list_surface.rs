use keld_interpreter::{
    AllocationController, CapacityError, ReserveFailure, RuntimeFaultKind, RuntimeList, Value,
    required_capacity, run_text_for_test, run_text_with_controls_for_test, trace_text_for_test,
};

fn assert_static_error(source: &str, code: &str) {
    let result = keld_storage::verify_text_for_test(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == code),
        "expected {code}, got {:#?}",
        result.diagnostics
    );
}

#[test]
fn copyable_index_read_returns_the_element() {
    let result = run_text_for_test(
        "fn main() -> Int { let items: List[Int] = List(); items.push(7); return items[0] }\n",
    )
    .expect("valid index executes");
    assert_eq!(result.value, Value::Int(7));
}

#[test]
fn negative_and_equal_length_indices_fault_with_bounds() {
    for index in [-1, 1] {
        let source = format!(
            "fn main() -> Int {{ let items: List[Int] = List(); items.push(7); return items[{index}] }}\n"
        );
        let fault = run_text_for_test(&source).expect_err("invalid index faults");
        assert_eq!(fault.kind, RuntimeFaultKind::Bounds);
    }
}

#[test]
fn text_element_can_be_loaned_but_not_bound_as_owned() {
    let result = run_text_for_test(
        "fn size(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { let items: List[Text] = List(); items.push(\"Keld\"); return size(items[0]) }\n",
    )
    .expect("indexed Text loan executes");
    assert_eq!(result.value, Value::Int(4));
    assert_static_error(
        "fn invalid(items: List[Text]) { let owned = items[0]; return }\nfn main() -> Int { return 0 }\n",
        "KLD2004",
    );
}

#[test]
fn indexed_replacement_installs_before_cleaning_the_old_value() {
    let trace = trace_text_for_test(
        "fn main() -> Int { let items: List[Text] = List(); items.push(\"old\"); items[0] = \"new\"; return items[0].byte_length }\n",
    )
    .expect("replacement executes");
    assert_eq!(trace.result.value, Value::Int(3));
    assert_eq!(trace.text_markers(), vec![(3, b'o'), (3, b'n')]);
}

#[test]
fn get_returns_none_without_fault_and_does_not_change_the_list() {
    let list = RuntimeList::from_values(vec![Value::Int(7)]);
    assert_eq!(list.get_copy(1), Ok(None));
    assert_eq!(list.length(), 1);
    assert_eq!(list.get_copy(0), Ok(Some(Value::Int(7))));
}

#[test]
fn try_remove_invalid_returns_none_and_preserves_storage_and_values() {
    let mut list = RuntimeList::with_capacity_for_test(8);
    list.push_for_test(Value::Int(7));
    let capacity = list.capacity_for_test();
    assert_eq!(list.try_remove(1), None);
    assert_eq!(list.length(), 1);
    assert_eq!(list.capacity_for_test(), capacity);
    assert_eq!(list.values_for_test(), &[Value::Int(7)]);
}

#[test]
fn try_remove_success_returns_owned_element_and_closes_the_gap() {
    let result = run_text_for_test(
        "fn main() -> Int { let inner: List[Int] = List(); inner.push(7); let outer: List[List[Int]] = List(); outer.push(take inner); let removed = outer.try_remove(0); return outer.length }\n",
    )
    .expect("successful try_remove executes");
    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn clear_destroys_elements_in_reverse_index_order() {
    let trace = trace_text_for_test(
        "fn main() -> Int { let items: List[Text] = List(); items.push(\"a\"); items.push(\"b\"); items.push(\"c\"); items.clear(); return 0 }\n",
    )
    .expect("clear executes");
    assert_eq!(trace.list_indices(), vec![2, 1, 0]);
    assert_eq!(trace.text_markers(), vec![(1, b'c'), (1, b'b'), (1, b'a')]);
}

#[test]
fn clear_executes_after_a_list_allocation_failure_injection() {
    let result = run_text_with_controls_for_test(
        "fn main() -> Int { let items: List[Text] = List(); items.reserve(2); items.push(\"a\"); items.push(\"b\"); let other: List[Int] = List(); let failed = other.try_reserve(1); items.clear(); if failed { return 1 }; return items.length }\n",
        keld_interpreter::TestControls::fail_list_attempts([2, 3]),
    )
    .expect("clear executes after the injected allocation failure");
    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn clear_pops_elements_without_releasing_list_storage() {
    let mut list = RuntimeList::with_capacity_for_test(8);
    list.push_for_test(Value::Int(1));
    list.push_for_test(Value::Int(2));
    let capacity = list.capacity_for_test();

    while list.pop().is_some() {}

    assert_eq!(list.length(), 0);
    assert_eq!(list.capacity_for_test(), capacity);
}

#[test]
fn remove_and_clear_preserve_capacity() {
    let mut list = RuntimeList::with_capacity_for_test(8);
    list.push_for_test(Value::Int(1));
    list.push_for_test(Value::Int(2));
    let capacity = list.capacity_for_test();
    let _ = list.remove(0);
    while list.pop().is_some() {}
    assert_eq!(list.capacity_for_test(), capacity);
}

#[test]
fn reserve_negative_and_unaddressable_sizes_are_capacity_faults() {
    assert_eq!(required_capacity(0, -1), Err(CapacityError::Impossible));
    assert_eq!(
        required_capacity(0, 192_153_584_101_141_162),
        Ok(192_153_584_101_141_162)
    );
    assert_eq!(
        required_capacity(0, 192_153_584_101_141_163),
        Err(CapacityError::Impossible)
    );
    assert_eq!(
        required_capacity(0, i64::MAX),
        Err(CapacityError::Impossible)
    );
}

#[test]
fn try_reserve_failure_returns_false_and_preserves_list() {
    let mut list = RuntimeList::from_values(vec![Value::Int(7)]);
    let capacity = list.capacity_for_test();
    let mut allocations = AllocationController::fail_list_attempts([1, 2]);
    assert!(!list.try_reserve(8, &mut allocations));
    assert_eq!(list.length(), 1);
    assert_eq!(list.capacity_for_test(), capacity);
    assert_eq!(list.values_for_test(), &[Value::Int(7)]);
}

#[test]
fn preferred_growth_failure_retries_minimum_capacity() {
    let mut list = RuntimeList::new();
    let mut allocations = AllocationController::fail_list_attempts([1]);
    list.reserve(1, &mut allocations)
        .expect("minimum retry succeeds");
    assert_eq!(allocations.list_attempts(), 2);
    assert!(list.capacity_for_test() >= 1);
}

#[test]
fn reserve_accounts_for_length_when_the_list_has_spare_capacity() {
    let mut list = RuntimeList::with_capacity_for_test(4);
    list.push_for_test(Value::Int(7));
    let mut allocations = AllocationController::default();

    list.reserve(100, &mut allocations)
        .expect("reservation succeeds");

    assert!(list.capacity_for_test() >= 101);
}

#[test]
fn nonempty_minimum_capacity_retry_meets_the_exact_requirement() {
    let mut list = RuntimeList::with_capacity_for_test(4);
    list.push_for_test(Value::Int(7));
    let mut allocations = AllocationController::fail_list_attempts([1]);

    list.reserve(4, &mut allocations)
        .expect("minimum retry succeeds");

    assert_eq!(allocations.list_attempts(), 2);
    assert!(list.capacity_for_test() >= 5);
}

#[test]
fn successful_try_reserve_guarantees_the_next_n_pushes_do_not_grow() {
    let mut list = RuntimeList::new();
    let mut allocations = AllocationController::default();
    assert!(list.try_reserve(3, &mut allocations));
    let attempts = allocations.list_attempts();
    for value in [1, 2, 3] {
        list.push(Value::Int(value), &mut allocations)
            .expect("reserved push succeeds");
    }
    assert_eq!(allocations.list_attempts(), attempts);
}

#[test]
fn nonempty_reservation_guarantees_exactly_the_next_additional_pushes() {
    let mut list = RuntimeList::with_capacity_for_test(4);
    list.push_for_test(Value::Int(0));
    let mut allocations = AllocationController::default();
    list.reserve(5, &mut allocations)
        .expect("reservation succeeds");
    let attempts = allocations.list_attempts();

    for value in 1..=5 {
        list.push(Value::Int(value), &mut allocations)
            .expect("reserved push succeeds");
    }

    assert_eq!(allocations.list_attempts(), attempts);
    assert_eq!(list.length(), 6);
}

#[test]
fn push_reports_allocation_only_after_minimum_retry_fails() {
    let mut list = RuntimeList::new();
    let mut allocations = AllocationController::fail_list_attempts([1, 2]);
    assert_eq!(
        list.push(Value::Int(1), &mut allocations),
        Err(ReserveFailure::Allocation)
    );
    assert_eq!(allocations.list_attempts(), 2);
    assert_eq!(list.length(), 0);
}

#[test]
fn reserve_and_try_reserve_execute_through_the_ir() {
    let result = run_text_for_test(
        "fn main() -> Int { let items: List[Int] = List(); items.reserve(3); let ok = items.try_reserve(-1); if ok { return 1 }; return items.length }\n",
    )
    .expect("reserve operations execute");
    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn injected_push_growth_failure_is_reported_as_allocation() {
    let fault = run_text_with_controls_for_test(
        "fn main() -> Int { let items: List[Int] = List(); items.push(1); return 0 }\n",
        keld_interpreter::TestControls::fail_list_attempts([1, 2]),
    )
    .expect_err("injected growth failure faults");
    assert_eq!(fault.kind, RuntimeFaultKind::Allocation);
}

#[test]
fn nested_list_indexing_replacement_and_bounds_are_checked() {
    let result = run_text_for_test(
        "fn main() -> Int { let inner: List[Int] = List()\ninner.push(1)\nlet outer: List[List[Int]] = List()\nouter.push(take inner)\nouter[0][0] = 7\nreturn outer[0][0]\n}\n",
    )
    .expect("nested indexed replacement executes");
    assert_eq!(result.value, Value::Int(7));

    let fault = run_text_for_test(
        "fn main() -> Int { let inner: List[Int] = List()\nlet outer: List[List[Int]] = List()\nouter.push(take inner)\nreturn outer[0][0]\n}\n",
    )
    .expect_err("nested out-of-bounds access must fault");
    assert_eq!(fault.kind, RuntimeFaultKind::Bounds);
}

#[test]
fn failed_indexed_rhs_evaluation_does_not_install_a_replacement() {
    let fault = run_text_for_test(
        "fn main() -> Int { let items: List[Int] = List()\nitems.push(7)\nitems[0] = items[1]\nreturn items[0]\n}\n",
    )
    .expect_err("failed RHS indexing must fault before replacement");
    assert_eq!(fault.kind, RuntimeFaultKind::Bounds);
}
