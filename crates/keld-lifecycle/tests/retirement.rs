use keld_lifecycle::verify_text_for_test;

#[test]
fn retiring_a_must_alias_makes_both_names_retired() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn bad(enemy: Enemy) -> Int retires enemy { let alias = enemy; retire enemy; return alias.health }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1001");
    assert!(result.diagnostics[0].help.is_some());
}

#[test]
fn retiring_one_may_alias_invalidates_the_other_parameter() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn bad(a: Enemy, b: Enemy) -> Int retires a { retire a; return b.health }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1008");
    assert!(result.diagnostics[0].help.is_some());
}

#[test]
fn retirement_on_only_one_path_rejects_the_joined_use() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn bad(enemy: Enemy, flag: Bool) -> Int retires enemy { if flag { retire enemy }; return enemy.health }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1003");
    assert!(result.diagnostics[0].help.is_some());
}

#[test]
fn distinct_allocations_survive_exact_retirement() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn main() -> Int { lifecycle level { let first = Enemy(health: 1); let second = Enemy(health: 2); retire first; return second.health } }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}
