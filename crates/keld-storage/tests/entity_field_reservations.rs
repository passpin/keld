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

#[test]
fn nested_entity_field_replacement_conflicts_with_an_outer_reservation() {
    assert_conflict(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect(items: List[Int], marker: Int) { return; }\nfn replace(holder: Holder) -> Int { holder.items = List(); return 0; }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); inspect(holder.items, replace(holder)); return 0; }; }\n",
    );
}

#[test]
fn nested_entity_retirement_conflicts_with_an_outer_reservation() {
    assert_conflict(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect(items: List[Int], marker: Int) { return; }\nfn retire_holder(holder: Holder) -> Int retires holder { retire holder; return 0; }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); inspect(holder.items, retire_holder(holder)); return 0; }; }\n",
    );
}

#[test]
fn nested_mutation_of_a_proven_distinct_entity_is_valid() {
    assert_valid(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect(items: List[Int], marker: Int) { return; }\nfn replace(holder: Holder) -> Int { holder.items = List(); return 0; }\nfn main() -> Int { lifecycle level { let left = Holder(items: List()); let right = Holder(items: List()); inspect(left.items, replace(right)); return 0; }; }\n",
    );
}

#[test]
fn broad_retirement_conflicts_with_every_same_type_entity_field() {
    assert_conflict(
        "entity Holder {\nitems: List[Int]\n}\nentity Registry {\ntarget: link Holder?\n}\nfn inspect(items: List[Int], marker: Int) { return; }\nfn sweep(registry: Registry) -> Int retires any Holder { when registry.target as holder { retire holder; }; return 0; }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); let registry = Registry(target: holder); inspect(holder.items, sweep(registry)); return 0; }; }\n",
    );
}

#[test]
fn nested_read_of_the_same_entity_field_is_compatible() {
    assert_valid(
        "entity Holder {\nitems: List[Int]\n}\nfn inspect(items: List[Int], marker: Int) { return; }\nfn read(holder: Holder) -> Int { return holder.items.length; }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); inspect(holder.items, read(holder)); return 0; }; }\n",
    );
}
