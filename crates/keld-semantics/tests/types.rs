use keld_semantics::{DefinitionKind, TypeKind, analyze_text};
use std::fmt::Write;

#[test]
fn resolves_entity_names_as_refs_and_link_fields_as_links() {
    let analysis = analyze_text(
        "entity Enemy {\nhealth: Int\ntarget: link Enemy?\n}\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("program must type-check");
    let enemy = &module.definitions[0];

    assert_eq!(enemy.kind, DefinitionKind::Entity);
    assert!(matches!(
        module.types.kind(enemy.fields[0].ty),
        TypeKind::Int
    ));
    assert!(matches!(
        module.types.kind(enemy.fields[1].ty),
        TypeKind::Link {
            entity,
            optional: true
        } if *entity == enemy.id
    ));
}

#[test]
fn semantic_ids_follow_source_order_not_lookup_order() {
    let analysis = analyze_text(
        "struct Zebra {\nz: Int\n}\nstruct Alpha {\na: Int\n}\nfn zed() -> Int { return 1 }\nfn main() -> Int { return zed() }\n",
    );
    let module = analysis.module.expect("program must type-check");

    assert_eq!(module.definitions[0].id.0, 0);
    assert_eq!(module.definitions[0].name, "Zebra");
    assert_eq!(module.definitions[1].id.0, 1);
    assert_eq!(module.definitions[1].name, "Alpha");
    assert_eq!(module.functions[0].id.0, 0);
    assert_eq!(module.functions[1].id.0, 1);
}

#[test]
fn rejects_self_and_mutual_by_value_layout_cycles_but_links_break_cycles() {
    for text in [
        "struct Node {\nnext: Node\n}\nfn main() -> Int { return 0 }\n",
        "struct A {\nb: B\n}\nstruct B {\na: A\n}\nfn main() -> Int { return 0 }\n",
    ] {
        let analysis = analyze_text(text);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0110")
        );
    }

    let linked =
        analyze_text("entity Node {\nnext: link Node?\n}\nfn main() -> Int { return 0 }\n");
    assert!(linked.diagnostics.is_empty(), "{:#?}", linked.diagnostics);
}

#[test]
fn type_errors_use_stable_focused_codes() {
    let cases = [
        (
            "struct A {\nx: Missing\n}\nfn main() -> Int { return 0 }\n",
            "KLD0103",
        ),
        (
            "struct A {\nx: Int\nx: Int\n}\nfn main() -> Int { return 0 }\n",
            "KLD0102",
        ),
        (
            "fn main() -> Int { if 1 { return 1 } else { return 0 } }\n",
            "KLD0107",
        ),
        ("fn main() -> Int { return false }\n", "KLD0106"),
        ("fn main() -> Int { return missing }\n", "KLD0103"),
    ];

    for (text, code) in cases {
        let analysis = analyze_text(text);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == code),
            "{code}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn condition_type_error_is_focused_and_ordering_requires_ints() {
    let condition = analyze_text("fn main() -> Int { if 1 { return 1 } else { return 0 } }\n");
    assert_eq!(
        condition.diagnostics.len(),
        1,
        "{:#?}",
        condition.diagnostics
    );
    assert_eq!(condition.diagnostics[0].code.0, "KLD0107");

    let ordering =
        analyze_text("fn main() -> Int { if false < true { return 1 } else { return 0 } }\n");
    assert!(
        ordering
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD0106")
    );
}

#[test]
fn by_value_layout_depth_over_256_is_rejected_iteratively() {
    let mut text = String::new();
    for index in 0..257 {
        if index == 256 {
            writeln!(text, "struct S{index} {{\nvalue: Int\n}}").unwrap();
        } else {
            writeln!(text, "struct S{index} {{\nnext: S{}\n}}", index + 1).unwrap();
        }
    }
    text.push_str("fn main() -> Int { return 0 }\n");
    let analysis = analyze_text(&text);

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD0110")
    );
}

#[test]
fn bootstrap_assignment_exclusions_are_focused() {
    for text in [
        "fn main() -> Int { let x = 0; x = 1; return x }\n",
        "struct S {\nx: Int\n}\nfn main() -> Int { let s = S(x: 0); s.x = 1; return s.x }\n",
    ] {
        let analysis = analyze_text(text);
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.0 == "KLD0004" && diagnostic.primary.message.contains("assignment")
        }));
    }
}
