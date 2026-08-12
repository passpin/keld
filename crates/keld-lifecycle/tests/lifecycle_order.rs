use keld_lifecycle::verify_text_for_test;

#[test]
fn keep_moves_a_known_inner_entity_to_an_active_ancestor() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn main() -> Int { lifecycle outer { lifecycle inner { let enemy = Enemy(health: 1); keep enemy in outer; return enemy.health } } }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn keep_rejects_dynamic_parameter_lifecycle() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn bad(enemy: Enemy) -> Int { lifecycle outer { keep enemy in outer; return enemy.health } }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1004");
    assert!(result.diagnostics[0].help.is_some());
}

#[test]
fn keep_requires_a_strict_ancestor_not_the_same_lifecycle() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn main() -> Int { lifecycle outer { let enemy = Enemy(health: 1); keep enemy in outer; return enemy.health } }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1004");
}
