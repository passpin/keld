use keld_semantics::{HirExprKind, HirStmtKind, analyze_text};

#[test]
fn requires_exactly_one_zero_parameter_int_main_without_retirement_effects() {
    let cases = [
        "fn helper() -> Int { return 0 }\n",
        "fn main(value: Int) -> Int { return value }\n",
        "fn main() { return }\n",
        "entity E {\nx: Int\n}\nfn main() -> Int retires any E { return 0 }\n",
        "fn main() -> Int { return 0 }\nfn main() -> Int { return 1 }\n",
    ];

    for text in cases {
        let analysis = analyze_text(text);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0109"),
            "{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn constant_faults_and_int_range_errors_are_compile_time_diagnostics() {
    for text in [
        "fn main() -> Int { return 1 / 0 }\n",
        "fn main() -> Int { return 9223372036854775807 + 1 }\n",
        "fn main() -> Int { return 1 << 64 }\n",
    ] {
        let analysis = analyze_text(text);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0120")
        );
    }

    for text in [
        "fn main() -> Int { return 9223372036854775808 }\n",
        "fn main() -> Int { return 9223372036854775809 }\n",
    ] {
        let analysis = analyze_text(text);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0121")
        );
    }
}

#[test]
fn constant_fault_detection_respects_short_circuit_evaluation() {
    let analysis = analyze_text(
        "fn safe() -> Bool { return false && (1 / 0 == 0) }\nfn main() -> Int { return 0 }\n",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn unary_min_magnitude_and_int_constants_lower_to_exact_hir_values() {
    for (expression, expected) in [
        ("-9223372036854775808", i64::MIN),
        ("Int.MIN", i64::MIN),
        ("Int.MAX", i64::MAX),
    ] {
        let analysis = analyze_text(&format!("fn main() -> Int {{ return {expression} }}\n"));
        let module = analysis.module.expect("constant must be accepted");
        let return_statement = &module.functions[module.main.0 as usize].body.statements[0];
        let HirStmtKind::Return(Some(value)) = &return_statement.kind else {
            panic!("expected return HIR")
        };
        assert!(matches!(value.kind, HirExprKind::Int(actual) if actual == expected));
    }
}

#[test]
fn non_unit_functions_must_return_on_every_structured_path() {
    for text in [
        "fn main() -> Int { let x = 1 }\n",
        "fn main() -> Int { if true { return 1 } }\n",
        "entity E {\nx: Int\n}\nfn main() -> Int { lifecycle level { let e = E(x: 1) } }\n",
    ] {
        let analysis = analyze_text(text);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0111")
        );
    }

    let complete = analyze_text("fn main() -> Int { if true { return 1 } else { return 0 } }\n");
    assert!(
        complete.diagnostics.is_empty(),
        "{:#?}",
        complete.diagnostics
    );
}

#[test]
fn constructor_and_call_arguments_keep_source_evaluation_order() {
    let analysis = analyze_text(
        "struct Pair {\nfirst: Int\nsecond: Int\n}\nfn pick(a: Int, b: Int) -> Int { return b }\nfn main() -> Int { let pair = Pair(second: pick(1, 2), first: 0); return pair.second }\n",
    );
    let module = analysis.module.expect("program must type-check");
    let main = &module.functions[module.main.0 as usize];
    let HirStmtKind::Let { initializer, .. } = &main.body.statements[0].kind else {
        panic!("expected let HIR")
    };
    let HirExprKind::ConstructStruct { fields, .. } = &initializer.kind else {
        panic!("expected struct construction")
    };

    assert_eq!(fields[0].0.0, 1);
    assert_eq!(fields[1].0.0, 0);
}

#[test]
fn else_if_conditions_are_preserved_as_nested_hir() {
    let analysis = analyze_text(
        "fn main() -> Int { if false { return 0 } else if true { return 1 } else { return 2 } }\n",
    );
    let module = analysis.module.expect("program must type-check");
    let HirStmtKind::If(outer) = &module.functions[module.main.0 as usize].body.statements[0].kind
    else {
        panic!("expected outer if")
    };
    let nested_block = outer.else_block.as_ref().expect("else-if block");
    let HirStmtKind::If(nested) = &nested_block.statements[0].kind else {
        panic!("expected nested if")
    };

    assert!(matches!(nested.condition.kind, HirExprKind::Bool(true)));
    assert!(nested.else_block.is_some());
}

#[test]
fn unchecked_link_field_access_remains_marked_for_proof_analysis() {
    let analysis = analyze_text(
        "entity Enemy {\nhealth: Int\n}\nstruct World {\ntarget: link Enemy?\n}\nfn main() -> Int { let world = World(target: none); return world.target.health }\n",
    );
    let module = analysis
        .module
        .expect("typing retains the unchecked access");
    let HirStmtKind::Return(Some(value)) =
        &module.functions[module.main.0 as usize].body.statements[1].kind
    else {
        panic!("expected return")
    };

    assert!(matches!(value.kind, HirExprKind::UncheckedLinkField { .. }));
}
