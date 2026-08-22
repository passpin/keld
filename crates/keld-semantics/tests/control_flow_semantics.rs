use keld_semantics::{HirExprKind, HirStmtKind, TypeStore, analyze_text};

#[test]
fn while_break_and_continue_are_not_feature_gated() {
    let analysis = analyze_text(
        r#"fn main() -> Int {
var i = 0
while i < 4 {
i += 1
if i == 2 { continue }
if i == 3 { break }
}
return i
}
"#,
    );

    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.0 != "KLD0004"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn while_condition_must_be_bool() {
    let analysis = analyze_text("fn main() -> Int { while 1 { break } return 0 }\n");

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD0107"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn loop_control_outside_loop_is_rejected() {
    for keyword in ["break", "continue"] {
        let analysis = analyze_text(&format!("fn main() -> Int {{ {keyword}; return 0 }}\n"));
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0112"),
            "{keyword}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn while_is_preserved_in_typed_hir() {
    let analysis = analyze_text(
        "fn main() -> Int { var i = 0; while i < 2 { i += 1 }; return i }\n",
    );
    assert!(analysis.diagnostics.is_empty(), "{:#?}", analysis.diagnostics);
    let module = analysis.module.expect("loop program must reach typed HIR");
    let while_statement = &module.functions[0].body.statements[1];
    let HirStmtKind::While(while_) = &while_statement.kind else {
        panic!("expected while HIR, got {:#?}", while_statement.kind);
    };
    assert_eq!(while_.condition.ty, TypeStore::BOOL);
    assert!(matches!(&while_.condition.kind, HirExprKind::Binary { .. }));
    assert!(matches!(
        &while_.body.statements[0].kind,
        HirStmtKind::CompoundAssign { .. }
    ));
}

#[test]
fn literal_true_loop_without_direct_break_is_non_fallthrough() {
    let analysis = analyze_text("fn main() -> Int { while true { } }\n");
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.0 != "KLD0111"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn break_in_nested_loop_does_not_make_outer_true_loop_fallthrough() {
    let analysis = analyze_text("fn main() -> Int { while true { while true { break } } }\n");
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.0 != "KLD0111"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn direct_break_makes_true_loop_may_fallthrough() {
    let analysis = analyze_text("fn main() -> Int { while true { break } }\n");
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD0111"),
        "{:#?}",
        analysis.diagnostics
    );
}
