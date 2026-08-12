use keld_interpreter::{
    RuntimeFaultKind, RuntimeList, Value, run_text_for_test, trace_text_for_test,
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
