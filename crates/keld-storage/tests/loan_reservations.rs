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
fn two_read_loans_of_the_same_list_are_valid() {
    let result = verify_source(
        "fn read(left: List[Int], right: List[Int]) {\nlet n = left.length\nreturn\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nread(items, items)\nreturn 0\n}\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn structural_loan_cannot_overlap_a_second_loan_of_the_same_list() {
    let result = verify_source(
        "fn change(left: List[Int], right: List[Int]) {\nleft.push(1)\nreturn\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nchange(items, items)\nreturn 0\n}\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2005"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn pending_read_reservation_blocks_later_structural_argument_evaluation() {
    let result = verify_source(
        "fn inspect(items: List[Int], count: Int) { return }\nfn main() -> Int {\nlet items: List[Int] = List()\nitems.push(1)\ninspect(items, items.remove(0))\nreturn 0\n}\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2005"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn outer_reservation_survives_nested_argument_evaluation() {
    let result = verify_source(
        "fn mutate(items: List[Int]) -> Int {\nitems.push(1)\nreturn items.length\n}\nfn inspect(items: List[Int], count: Int) {\nreturn\n}\nfn main() -> Int {\nlet items: List[Int] = List()\ninspect(items, mutate(items))\nreturn 0\n}\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2005"),
        "{:#?}",
        result.diagnostics
    );
}
