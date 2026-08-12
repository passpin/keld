use keld_interpreter::{Value, run_text_for_test};

#[test]
fn stale_link_resolution_uses_the_absent_edge() {
    let result = run_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 4); let saved: link Enemy? = enemy; retire enemy; when saved as live { return 1 } else { return 4 } } }\n",
    )
    .unwrap();

    assert_eq!(result.value, Value::Int(4));
}

#[test]
fn kept_entity_survives_its_original_lifecycle() {
    let result = run_text_for_test(
        "entity Enemy {\nhealth: Int\n}\nentity Anchor {\ntarget: link Enemy?\n}\nfn main() -> Int { lifecycle game { let anchor = Anchor(target: none); lifecycle level { let enemy = Enemy(health: 30); anchor.target = enemy; keep enemy in game }; when anchor.target as survivor { return survivor.health } else { return 0 } } }\n",
    )
    .unwrap();

    assert_eq!(result.value, Value::Int(30));
}
