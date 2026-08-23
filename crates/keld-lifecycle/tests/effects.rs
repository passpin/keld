use keld_lifecycle::{ReturnProvenance, verify_text_for_test};

#[test]
fn exact_declared_retirement_matches_inferred_behavior() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn remove(enemy: Enemy) retires enemy { retire enemy }\nfn main() -> Int { return 0 }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    assert_eq!(
        result.module.unwrap().summaries[0].retires_parameters,
        vec![0]
    );
}

#[test]
fn undeclared_and_redundant_retirement_effects_are_rejected() {
    for text in [
        "entity Enemy {\nhealth: Int\n}\nfn remove(enemy: Enemy) { retire enemy }\nfn main() -> Int { return 0 }\n",
        "entity Enemy {\nhealth: Int\n}\nfn inspect(enemy: Enemy) retires enemy { let health = enemy.health }\nfn main() -> Int { return 0 }\n",
    ] {
        let result = verify_text_for_test(text);
        assert_eq!(result.diagnostics[0].code.0, "KLD1009");
        assert!(result.diagnostics[0].help.is_some());
    }
}

#[test]
fn broad_retirement_propagates_through_calls() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nentity World {\ntarget: link Enemy?\n}\nfn sweep(world: World) retires any Enemy { when world.target as enemy { retire enemy } }\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 1); let world = World(target: enemy); sweep(world); return enemy.health } }\n",
    );

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD1008")
    );
}

#[test]
fn parameter_and_fresh_return_provenance_are_summarized() {
    let parameter = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn identity(enemy: Enemy) -> Enemy { return enemy }\nfn main() -> Int { return 0 }\n",
    );
    assert!(matches!(
        parameter.module.unwrap().summaries[0].return_provenance,
        ReturnProvenance::EntitySources {
            ref parameters,
            fresh_in_caller_lifecycle: false
        } if parameters == &[0]
    ));

    let fresh = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn create() -> Enemy { return Enemy(health: 1) }\nfn main() -> Int { lifecycle level { let enemy = create(); return enemy.health } }\n",
    );
    assert!(matches!(
        fresh.module.unwrap().summaries[0].return_provenance,
        ReturnProvenance::EntitySources {
            ref parameters,
            fresh_in_caller_lifecycle: true
        } if parameters.is_empty()
    ));
}

#[test]
fn entrypoint_applies_broad_liveness_barriers_without_declaring_public_effects() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nentity World {\ntarget: link Enemy?\n}\nfn sweep(world: World) retires any Enemy { when world.target as enemy { retire enemy } }\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 1); let world = World(target: enemy); sweep(world); when world.target as live { return live.health } else { return 0 } } }\n",
    );

    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}
