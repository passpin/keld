use keld_storage::verify_text_for_test;

fn assert_code(source: &str, code: &str) {
    let result = verify_text_for_test(source);
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
fn indexed_replacement_rejects_structural_rhs_access() {
    assert_code(
        "fn main() -> Int {\nlet items: List[Int] = List()\nlet index = 0\nitems[index] = items.remove(0)\nreturn 0\n}\n",
        "KLD2007",
    );
}

#[test]
fn indexed_replacement_allows_read_only_rhs_access() {
    let result = verify_text_for_test(
        "fn main() -> Int {\nlet items: List[Int] = List()\nlet index = 0\nlet other = 0\nitems[index] = items[other]\nreturn 0\n}\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn indexed_replacement_rejects_take_rhs_access() {
    assert_code(
        "fn consume(take items: List[Int]) -> Int {\nreturn 0\n}\nfn main() -> Int {\nlet items: List[Int] = List()\nlet index = 0\nitems[index] = consume(take items)\nreturn 0\n}\n",
        "KLD2007",
    );
}
