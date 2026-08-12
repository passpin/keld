use keld_semantics::{DefinitionKind, TypeKind, analyze_text};
use std::fmt::Write;

#[test]
fn classifies_managed_types_and_structural_duplicability() {
    let mut types = keld_semantics::TypeStore::new();
    let text = types.intern(TypeKind::Text);
    let list_text = types.intern(TypeKind::List(text));
    let pair = types.intern(TypeKind::Struct(keld_semantics::DefId(0)));
    types.register_struct_fields(
        keld_semantics::DefId(0),
        [text, keld_semantics::TypeStore::INT],
    );

    assert!(matches!(types.kind(text), TypeKind::Text));
    assert!(matches!(types.kind(list_text), TypeKind::List(id) if *id == text));
    assert_eq!(
        types.storage_class(text),
        keld_semantics::StorageClass::SingleHome
    );
    assert_eq!(
        types.storage_class(list_text),
        keld_semantics::StorageClass::SingleHome
    );
    assert!(types.is_structurally_duplicable(text));
    assert!(types.is_structurally_duplicable(list_text));
    assert_eq!(
        types.storage_class(pair),
        keld_semantics::StorageClass::SingleHome
    );
}

#[test]
fn parses_text_list_and_general_optional_types() {
    let analysis = analyze_text(
        "struct Pair { left: Text\nright: List[Int]\nmaybe: List[Text]?\n}\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("managed types must be accepted");
    let fields = &module.definitions[0].fields;

    assert!(matches!(module.types.kind(fields[0].ty), TypeKind::Text));
    assert!(
        matches!(module.types.kind(fields[1].ty), TypeKind::List(element) if *module.types.kind(*element) == TypeKind::Int)
    );
    assert!(
        matches!(module.types.kind(fields[2].ty), TypeKind::Optional(inner) if matches!(module.types.kind(*inner), TypeKind::List(_)))
    );
}

#[test]
fn records_consuming_parameters_and_uninitialized_vars() {
    let analysis = analyze_text(
        "fn consume(take items: List[Int]) { return }\nfn main() -> Int { var items: List[Int]\nreturn 0 }\n",
    );
    let module = analysis.module.expect("storage metadata must type-check");
    assert_eq!(
        module.functions[0].parameter_modes[0],
        keld_semantics::ParameterMode::Take
    );
    assert_eq!(
        module.functions[1].local_mutability[0],
        keld_semantics::BindingMutability::Var
    );
    assert!(matches!(
        module.functions[1].body.statements[0].kind,
        keld_semantics::HirStmtKind::Var {
            initializer: None,
            ..
        }
    ));
}

#[test]
fn types_take_as_an_explicit_owned_expression() {
    let analysis = analyze_text(
        "fn move_items(take items: List[Int]) -> List[Int] { return take items }\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("take must type-check");
    assert!(
        matches!(
            module.functions[0].body.statements[0].kind,
            keld_semantics::HirStmtKind::Return(Some(keld_semantics::HirExpr {
                kind: keld_semantics::HirExprKind::Take(_),
                ..
            }))
        ),
        "{:#?}",
        module.functions[0].body
    );
}

#[test]
fn types_copy_as_an_explicit_structural_operation() {
    let analysis = analyze_text(
        "fn copy_items(items: List[Int]) -> List[Int] { return items.copy() }\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("copy must type-check");
    assert!(matches!(
        module.functions[0].body.statements[0].kind,
        keld_semantics::HirStmtKind::Return(Some(keld_semantics::HirExpr {
            kind: keld_semantics::HirExprKind::Copy(_),
            ..
        }))
    ));
}

#[test]
fn types_empty_list_constructor_from_expected_type() {
    let analysis =
        analyze_text("fn make() -> List[Int] { return List() }\nfn main() -> Int { return 0 }\n");
    let module = analysis.module.expect("List() must use its expected type");
    assert!(matches!(
        module.functions[0].body.statements[0].kind,
        keld_semantics::HirStmtKind::Return(Some(keld_semantics::HirExpr {
            kind: keld_semantics::HirExprKind::ListNew,
            ..
        }))
    ));
}

#[test]
fn types_list_length_as_a_checked_property() {
    let analysis = analyze_text(
        "fn length(items: List[Int]) -> Int { return items.length }\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("List.length must type-check");
    assert!(matches!(
        module.functions[0].body.statements[0].kind,
        keld_semantics::HirStmtKind::Return(Some(keld_semantics::HirExpr {
            kind: keld_semantics::HirExprKind::ListLength(_),
            ..
        }))
    ));
}

#[test]
fn types_list_push_with_an_element_type() {
    let analysis = analyze_text(
        "fn add(items: List[Int]) { items.push(1)\nreturn }\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("List.push must type-check");
    assert!(matches!(
        module.functions[0].body.statements[0].kind,
        keld_semantics::HirStmtKind::Expr(keld_semantics::HirExpr {
            kind: keld_semantics::HirExprKind::ListPush { .. },
            ty: keld_semantics::TypeStore::UNIT,
            ..
        })
    ));
}

#[test]
fn types_list_remove_returns_the_element_type() {
    let analysis = analyze_text(
        "fn remove(items: List[Int]) -> Int { return items.remove(0) }\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("List.remove must type-check");
    assert!(matches!(
        module.functions[0].body.statements[0].kind,
        keld_semantics::HirStmtKind::Return(Some(keld_semantics::HirExpr {
            kind: keld_semantics::HirExprKind::ListRemove { .. },
            ty: keld_semantics::TypeStore::INT,
            ..
        }))
    ));
}

#[test]
fn types_text_literal_and_byte_length() {
    let analysis = analyze_text(
        "fn length() -> Int { return \"Keld\".byte_length }\nfn main() -> Int { return 0 }\n",
    );
    let module = analysis.module.expect("Text literal must type-check");
    assert!(matches!(
        module.functions[0].body.statements[0].kind,
        keld_semantics::HirStmtKind::Return(Some(keld_semantics::HirExpr {
            kind: keld_semantics::HirExprKind::TextByteLength(_),
            ty: keld_semantics::TypeStore::INT,
            ..
        }))
    ));
}

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
    let immutable = analyze_text("fn main() -> Int { let x = 0; x = 1; return x }\n");
    assert!(
        immutable
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD2010")
    );
    let struct_field = analyze_text(
        "struct S {\nx: Int\n}\nfn main() -> Int { let s = S(x: 0); s.x = 1; return s.x }\n",
    );
    assert!(struct_field.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.0 == "KLD0004" && diagnostic.primary.message.contains("assignment")
    }));
}
