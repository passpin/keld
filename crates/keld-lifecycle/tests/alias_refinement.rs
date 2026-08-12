use keld_lifecycle::verify_text_for_test;

#[test]
fn identity_inequality_refines_the_valid_branch() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn ok(a: Enemy, b: Enemy) -> Int retires a { if a != b { retire a; return b.health } else { retire a; return 0 } }\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn identity_equality_refines_to_must_alias() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn bad(a: Enemy, b: Enemy) -> Int retires a { if a == b { retire a; return b.health } else { retire a; return 0 } }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1001");
}

#[test]
fn copying_a_reference_preserves_must_alias_provenance() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn bad(enemy: Enemy) -> Int retires enemy { let alias = enemy; retire alias; return enemy.health }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1001");
}

#[test]
fn impossible_identity_edge_does_not_block_the_reachable_merge() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn remove(enemy: Enemy) retires enemy { let alias = enemy; if enemy == alias { let marker = 1 } else { let marker = 2 }; retire enemy }\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}
