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
fn different_constant_indices_are_disjoint() {
    let result = verify_text_for_test(
        "fn change(left: List[Int], right: List[Int]) {\nleft.push(1)\nreturn\n}\nfn main() -> Int {\nlet matrix: List[List[Int]] = List()\nchange(matrix[0], matrix[1])\nreturn 0\n}\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn same_constant_indices_conflict() {
    assert_code(
        "fn change(left: List[Int], right: List[Int]) {\nleft.push(1)\nreturn\n}\nfn main() -> Int {\nlet matrix: List[List[Int]] = List()\nchange(matrix[0], matrix[0])\nreturn 0\n}\n",
        "KLD2005",
    );
}

#[test]
fn distinct_struct_fields_are_disjoint() {
    let result = verify_text_for_test(
        "struct Holder {\nleft: List[Int]\nright: List[Int]\n}\nfn change(left: List[Int], right: List[Int]) {\nleft.push(1)\nreturn\n}\nfn main() -> Int {\nlet holder = Holder(left: List(), right: List())\nchange(holder.left, holder.right)\nreturn 0\n}\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn same_struct_field_conflicts() {
    assert_code(
        "struct Holder {\nleft: List[Int]\nright: List[Int]\n}\nfn change(left: List[Int], right: List[Int]) {\nleft.push(1)\nreturn\n}\nfn main() -> Int {\nlet holder = Holder(left: List(), right: List())\nchange(holder.left, holder.left)\nreturn 0\n}\n",
        "KLD2005",
    );
}

#[test]
fn value_indices_are_conservatively_overlapping() {
    assert_code(
        "fn change(left: List[Int], right: List[Int]) {\nleft.push(1)\nreturn\n}\nfn main() -> Int {\nlet matrix: List[List[Int]] = List()\nlet index = 0\nchange(matrix[index], matrix[index])\nreturn 0\n}\n",
        "KLD2005",
    );
}
