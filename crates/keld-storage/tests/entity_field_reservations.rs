use keld_storage::verify_text_for_test;

fn assert_conflict(source: &str) {
    let result = verify_text_for_test(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2005"),
        "expected KLD2005, got {:#?}",
        result.diagnostics
    );
}

fn assert_valid(source: &str) {
    let result = verify_text_for_test(source);
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn alias_locals_conflict_on_the_same_entity_field() {
    assert_conflict(
        "entity Holder {\nitems: List[Int]\n}\nfn change(left: List[Int], right: List[Int]) { left.push(1); return; }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); let alias = holder; change(holder.items, alias.items); return 0; }; }\n",
    );
}

#[test]
fn unrefined_same_type_entity_parameters_conservatively_conflict() {
    assert_conflict(
        "entity Holder {\nitems: List[Int]\n}\nfn change(left: List[Int], right: List[Int]) { left.push(1); return; }\nfn apply(left: Holder, right: Holder) { change(left.items, right.items); return; }\nfn main() -> Int { return 0; }\n",
    );
}

#[test]
fn proven_distinct_fresh_entities_keep_disjoint_field_loans() {
    assert_valid(
        "entity Holder {\nitems: List[Int]\n}\nfn change(left: List[Int], right: List[Int]) { left.push(1); return; }\nfn main() -> Int { lifecycle level { let left = Holder(items: List()); let right = Holder(items: List()); change(left.items, right.items); return 0; }; }\n",
    );
}

#[test]
fn read_only_loans_through_entity_aliases_are_compatible() {
    assert_valid(
        "entity Holder {\nitems: List[Int]\n}\nfn read(left: List[Int], right: List[Int]) -> Int { return left.length + right.length; }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); let alias = holder; return read(holder.items, alias.items); }; }\n",
    );
}

#[test]
fn different_fields_on_one_entity_remain_non_overlapping() {
    assert_valid(
        "entity Holder {\nleft: List[Int]\nright: List[Int]\n}\nfn change(left: List[Int], right: List[Int]) { left.push(1); return; }\nfn main() -> Int { lifecycle level { let holder = Holder(left: List(), right: List()); change(holder.left, holder.right); return 0; }; }\n",
    );
}
