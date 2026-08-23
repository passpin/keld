use keld_storage::verify_text_for_test;

#[test]
fn storage_ignores_operations_on_an_impossible_identity_edge() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn inspect(enemy: Enemy) -> Int { let alias = enemy; if enemy == alias { return enemy.health } else { let values: List[Int] = List(); values.push(1); return values.length } }\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}
