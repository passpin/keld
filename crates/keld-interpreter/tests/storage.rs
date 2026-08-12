use keld_interpreter::{TestControls, Value, run_text_for_test, run_text_with_controls_for_test};

#[test]
fn read_loan_of_heap_text_does_not_allocate_or_copy() {
    let result = run_text_with_controls_for_test(
        "fn size(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { let value: Text = \"abcdefghijklmnopqrstuvwxyz\"; return size(value) }\n",
        TestControls::fail_structural_copy(1),
    )
    .expect("read loan does not allocate");
    assert_eq!(result.value, Value::Int(26));
}

#[test]
fn two_read_parameters_can_share_one_source_place() {
    let result = run_text_for_test(
        "fn sum(a: List[Int], b: List[Int]) -> Int { return a.length + b.length }\nfn main() -> Int { let items: List[Int] = List(); items.push(1); return sum(items, items) }\n",
    )
    .expect("overlapping reads execute");
    assert_eq!(result.value, Value::Int(2));
}

#[test]
fn empty_list_can_be_created_and_transferred_to_a_consuming_parameter() {
    let result = run_text_for_test(
        "fn consume(take items: List[Int]) { return }\nfn main() -> Int {\nlet items: List[Int] = List()\nconsume(take items)\nreturn 0\n}\n",
    )
    .expect("valid storage program must execute");

    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn empty_list_length_is_zero() {
    let result = run_text_for_test(
        "fn main() -> Int {\nlet items: List[Int] = List()\nreturn items.length\n}\n",
    )
    .expect("valid list length program must execute");

    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn list_push_increases_length() {
    let result = run_text_for_test(
        "fn main() -> Int {\nlet items: List[Int] = List()\nitems.push(7)\nreturn items.length\n}\n",
    )
    .expect("valid list push program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn list_copy_is_independent_from_the_source() {
    let result = run_text_for_test(
        "fn measure(items: List[Int]) -> Int {\nlet copied = items.copy()\ncopied.push(1)\nreturn items.length\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nreturn measure(items)\n}\n",
    )
    .expect("valid list copy program must execute");

    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn list_remove_returns_and_removes_the_element() {
    let result = run_text_for_test(
        "fn main() -> Int {\nlet items: List[Int] = List()\nitems.push(7)\nreturn items.remove(0)\n}\n",
    )
    .expect("valid list remove program must execute");

    assert_eq!(result.value, Value::Int(7));
}

#[test]
fn list_remove_reports_bounds_fault() {
    let fault = run_text_for_test(
        "fn main() -> Int {\nlet items: List[Int] = List()\nreturn items.remove(0)\n}\n",
    )
    .expect_err("removing from an empty list must fail");

    assert_eq!(fault.kind, keld_interpreter::RuntimeFaultKind::Bounds);
}

#[test]
fn text_literal_reports_byte_length() {
    let result = run_text_for_test("fn main() -> Int { return \"Keld\".byte_length }\n")
        .expect("valid Text program must execute");

    assert_eq!(result.value, Value::Int(4));
}

#[test]
fn empty_text_reports_is_empty() {
    let result =
        run_text_for_test("fn main() -> Int { if \"\".is_empty { return 1 } else { return 0 } }\n")
            .expect("valid empty Text program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn text_copy_and_take_keep_single_home_rules() {
    let result = run_text_for_test(
        "fn forward(take value: Text) -> Int {\nlet copied = value.copy()\nlet moved = take value\nreturn copied.byte_length + moved.byte_length\n}\nfn main() -> Int {\nlet value: Text = \"Keld\"\nreturn forward(take value)\n}\n",
    )
    .expect("valid Text transfer program must execute");

    assert_eq!(result.value, Value::Int(8));
}

#[test]
fn text_concat_is_owned_and_utf8_byte_exact() {
    let result = run_text_for_test(
        "fn main() -> Int {\nlet value = \"K\" + \"한\"\nreturn value.byte_length\n}\n",
    )
    .expect("valid Text concat program must execute");

    assert_eq!(result.value, Value::Int(4));
}

#[test]
fn text_equality_is_exact_and_non_normalizing() {
    let result = run_text_for_test(
        "fn main() -> Int { if \"e\u{301}\" == \"é\" { return 1 } else { return 0 } }\n",
    )
    .expect("valid Text equality program must execute");

    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn text_equality_matches_inline_and_heap_storage_by_bytes() {
    let result = run_text_for_test(
        "fn main() -> Int { if \"abcdefghijklmnopqrstuv\" == \"abcdefghijklmnopqrstuv\" { return 1 } else { return 0 } }\n",
    )
    .expect("valid long Text equality program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn nested_lists_transfer_copy_and_remove() {
    let result = run_text_for_test(
        "fn main() -> Int {\nlet inner: List[Int] = List()\ninner.push(7)\nlet outer: List[List[Int]] = List()\nouter.push(take inner)\nlet copied = outer.copy()\nlet removed = take copied\nreturn removed.remove(0).length\n}\n",
    )
    .expect("valid nested List program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn structural_loan_mutates_the_callers_list() {
    let result = run_text_for_test(
        "fn append(items: List[Int]) {\nitems.push(1)\nreturn\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nappend(items)\nreturn items.length\n}\n",
    )
    .expect("valid structural loan program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn consuming_parameter_returns_the_same_owned_list() {
    let result = run_text_for_test(
        "fn forward(take items: List[Int]) -> List[Int] {\nreturn take items\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nitems.push(1)\nlet result = forward(take items)\nreturn result.length\n}\n",
    )
    .expect("valid consuming return program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn managed_struct_field_can_be_constructed_and_read() {
    let result = run_text_for_test(
        "struct Holder { items: List[Int] }\nfn make(take items: List[Int]) -> Holder {\nreturn Holder(items: take items)\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nitems.push(1)\nlet holder = make(take items)\nreturn holder.items.length\n}\n",
    )
    .expect("valid managed struct program must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn managed_struct_field_can_be_loaned_to_a_function() {
    let result = run_text_for_test(
        "struct Holder { items: List[Int] }\nfn count(items: List[Int]) -> Int { return items.length }\nfn main() -> Int { let holder = Holder(items: List()); return count(holder.items) }\n",
    )
    .expect("managed field loan must execute");

    assert_eq!(result.value, Value::Int(0));
}

#[test]
fn managed_struct_field_can_be_structurally_loaned_to_a_function() {
    let result = run_text_for_test(
        "struct Holder { items: List[Int] }\nfn add(items: List[Int]) { items.push(1)\nreturn\n}\nfn main() -> Int { let holder = Holder(items: List())\nadd(holder.items)\nreturn holder.items.length\n}\n",
    )
    .expect("managed field structural loan must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn managed_entity_field_can_be_structurally_loaned_to_a_function() {
    let result = run_text_for_test(
        "entity Holder { items: List[Int] }\nfn add(items: List[Int]) { items.push(1)\nreturn\n}\nfn main() -> Int { lifecycle level { let holder = Holder(items: List())\nadd(holder.items)\nreturn holder.items.length\n} }\n",
    )
    .expect("managed entity field structural loan must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn direct_managed_entity_field_push_updates_the_field() {
    let result = run_text_for_test(
        "entity Holder { items: List[Int] }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List())\nholder.items.push(1)\nreturn holder.items.length\n} }\n",
    )
    .expect("direct entity field push must execute");

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn direct_managed_struct_field_remove_updates_the_field() {
    let result = run_text_for_test(
        "struct Holder { items: List[Int] }\nfn main() -> Int { let holder = Holder(items: List())\nholder.items.push(1)\nlet removed = holder.items.remove(0)\nreturn holder.items.length + removed\n}\n",
    )
    .expect("direct struct field remove must execute");

    assert_eq!(result.value, Value::Int(1));
}
