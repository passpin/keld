use keld_lifecycle::verify as verify_lifecycle;
use keld_storage::verify;

fn verify_source(source: &str) -> keld_storage::Verification {
    let flow = keld_flow::lower_text_for_test(source).expect("source must reach Flow");
    let lifecycle = verify_lifecycle(flow);
    let verified = lifecycle
        .module
        .expect("source must pass lifecycle verification");
    verify(verified)
}

#[test]
fn consuming_parameter_can_return_an_explicit_take() {
    let result = verify_source(
        "fn move_items(take items: List[Int]) -> List[Int] { return take items }\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    assert!(result.module.is_some());
}

#[test]
fn borrowed_parameter_cannot_be_returned_as_owned() {
    let result = verify_source(
        "fn borrow(items: List[Int]) -> List[Int] { return items }\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.0 == "KLD2003" }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn a_moved_home_cannot_be_taken_again() {
    let result = verify_source(
        "fn move_twice(take items: List[Int]) -> List[Int] { let first = take items; return take items }\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.0 == "KLD2002" }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn named_single_home_assignment_is_not_implicitly_a_copy() {
    let result = verify_source(
        "fn ambiguous(take items: List[Int]) -> List[Int] { let other = items; return take items }\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.0 == "KLD2001" }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn a_branch_move_produces_maybe_live_at_the_join() {
    let result = verify_source(
        "fn branch(flag: Bool, take items: List[Int]) -> List[Int] {\nif flag {\nlet moved = take items\n} else {\nlet untouched = 0\n}\nreturn take items\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.0 == "KLD2008" }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn explicit_copy_leaves_the_source_live() {
    let result = verify_source(
        "fn copy_items(items: List[Int]) -> List[Int] { let copied = items.copy(); return take copied }\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn moved_var_can_be_reinitialized_before_return() {
    let result = verify_source(
        "fn reinitialize() -> List[Int] {\nvar items: List[Int]\nitems = List()\nreturn take items\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn let_cannot_be_rebound_as_a_whole_value() {
    let result = keld_storage::verify_text_for_test(
        "fn invalid() {\nlet items: List[Int] = List()\nitems = List()\nreturn\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(!result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn list_push_requires_explicit_transfer_for_named_single_home_elements() {
    let result = keld_storage::verify_text_for_test(
        "fn invalid() {\nlet inner: List[Int] = List()\nlet outer: List[List[Int]] = List()\nouter.push(inner)\nreturn\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2001"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn consuming_call_requires_take_for_a_named_argument() {
    let result = keld_storage::verify_text_for_test(
        "fn consume(take items: List[Int]) { return }\nfn main() -> Int {\nlet items: List[Int] = List()\nconsume(items)\nreturn 0\n}\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2001"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn struct_construction_requires_transfer_for_managed_fields() {
    let result = keld_storage::verify_text_for_test(
        "struct Holder { items: List[Int] }\nfn invalid(take items: List[Int]) -> Holder { return Holder(items: items) }\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2001"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn managed_struct_field_read_is_not_an_implicit_copy() {
    let result = keld_storage::verify_text_for_test(
        "struct Holder { items: List[Int] }\nfn invalid(take items: List[Int]) {\nlet holder = Holder(items: take items)\nlet copy = holder.items\nreturn\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2004"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn entity_managed_field_assignment_requires_transfer() {
    let result = keld_storage::verify_text_for_test(
        "entity Holder { items: List[Int] }\nfn invalid(hold: Holder, items: List[Int]) {\nhold.items = items\nreturn\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2003"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn text_named_assignment_is_not_an_implicit_copy() {
    let result = keld_storage::verify_text_for_test(
        "fn invalid() {\nlet first: Text = \"keld\"\nlet second = first\nreturn\n}\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2001"),
        "{:#?}",
        result.diagnostics
    );
}
