use keld_interpreter::{FieldId, Value, trace_text_for_test};

#[test]
fn locals_drop_in_reverse_successful_initialization_order() {
    let trace = trace_text_for_test(
        "fn main() -> Int { let first: Text = \"a\"; let second: Text = \"b\"; return 0 }\n",
    )
    .expect("program executes");
    assert_eq!(trace.text_markers(), vec![(1, b'b'), (1, b'a')]);
}

#[test]
fn moved_and_uninitialized_homes_are_skipped() {
    let trace = trace_text_for_test(
        "fn consume(take value: Text) { return }\nfn main() -> Int { let a: Text = \"a\"; var b: Text; consume(take a); return 0 }\n",
    )
    .expect("program executes");
    assert_eq!(trace.text_markers(), vec![(1, b'a')]);
}

#[test]
fn maybe_live_home_drops_only_on_the_initialized_path() {
    let live = trace_text_for_test(
        "fn main() -> Int { var value: Text; if true { value = \"x\"; }; return 0 }\n",
    )
    .expect("live path executes");
    let empty = trace_text_for_test(
        "fn main() -> Int { var value: Text; if false { value = \"x\"; }; return 0 }\n",
    )
    .expect("empty path executes");
    assert_eq!(live.text_markers(), vec![(1, b'x')]);
    assert!(empty.text_markers().is_empty());
}

#[test]
fn reinitialized_home_becomes_the_newest_cleanup() {
    let trace = trace_text_for_test(
        "fn consume(take value: Text) { return }\nfn main() -> Int { var a: Text = \"a\"; let b: Text = \"b\"; consume(take a); a = \"c\"; return 0 }\n",
    )
    .expect("program executes");
    assert_eq!(trace.text_markers(), vec![(1, b'a'), (1, b'c'), (1, b'b')]);
}

#[test]
fn owned_temporary_loan_drops_after_the_call_closes() {
    let trace = trace_text_for_test(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"abcdefghijklmnopqrstuvwxyz\") }\n",
    )
    .expect("program executes");
    assert_eq!(trace.result.value, Value::Int(26));
    assert_eq!(trace.text_markers(), vec![(26, b'a')]);
}

#[test]
fn fields_and_list_elements_drop_in_reverse_order_iteratively() {
    let trace = trace_text_for_test(
        "struct Pair {\nleft: Text\nright: Text\n}\nfn main() -> Int { let values: List[Pair] = List(); values.push(Pair(left: \"a\", right: \"b\")); values.push(Pair(left: \"c\", right: \"d\")); return 0 }\n",
    )
    .expect("program executes");
    assert_eq!(trace.list_indices(), vec![1, 0]);
    assert_eq!(
        trace.field_ids(),
        vec![FieldId(1), FieldId(0), FieldId(1), FieldId(0)]
    );
}
