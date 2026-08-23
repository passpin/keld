use keld_lifecycle::verify_text_for_test;

#[test]
fn persistent_direct_reference_field_requires_a_link() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nentity Bad {\ntarget: Enemy\n}\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1002");
    assert!(
        result.diagnostics[0]
            .help
            .as_deref()
            .unwrap()
            .contains("link")
    );
}

#[test]
fn value_struct_direct_reference_field_also_requires_a_link() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nstruct Bad {\ntarget: Enemy\n}\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1002");
}

#[test]
fn unchecked_link_read_requires_when_resolution() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nentity World {\ntarget: link Enemy?\n}\nfn read(world: World) -> Int { return world.target.health }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1005");
    assert!(
        result.diagnostics[0]
            .help
            .as_deref()
            .unwrap()
            .contains("when")
    );
}

#[test]
fn resolved_link_reference_cannot_escape_its_when_scope() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn resolve(target: link Enemy?) -> Enemy { when target as enemy { return enemy } else { return Enemy(health: 0) } }\nfn main() -> Int { return 0 }\n",
    );

    assert_eq!(result.diagnostics[0].code.0, "KLD1002");
}

#[test]
fn every_entity_operation_receives_a_proof_annotation() {
    let result = verify_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn read(enemy: Enemy) -> Int { return enemy.health }\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 1); return read(enemy) } }\n",
    );
    let verified = result.module.expect("program must verify");

    assert!(!verified.proofs.is_empty());
    assert!(
        verified
            .proofs
            .iter()
            .all(|annotation| annotation.proof.0 < u32::MAX)
    );
}
