use keld_semantics::analyze_text;

#[test]
fn supported_bootstrap_program_reaches_typed_hir() {
    let analysis = analyze_text(
        r"entity Enemy {
health: Int
target: link Enemy?
}
fn main() -> Int {
lifecycle level {
let enemy = Enemy(health: 7, target: none)
return enemy.health
}
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(analysis.module.is_some());
}

#[test]
fn every_deferred_construct_has_one_focused_feature_diagnostic() {
    let cases = [
        ("use game.io\nfn main() -> Int { return 0 }\n", "use"),
        ("enum E { A }\nfn main() -> Int { return 0 }\n", "enum"),
        ("fn main() -> Int { match 0 { _ => 0 } }\n", "match"),
        (
            "struct Box[T] { value: T }\nfn main() -> Int { return 0 }\n",
            "generic",
        ),
        ("fn main() -> Int raises Error { return 0 }\n", "raises"),
        (
            "fn main() -> Int { try { return 0 } handle Error as error { return 1 } }\n",
            "try",
        ),
        (
            "unsafe module app\nfn main() -> Int { return 0 }\n",
            "unsafe",
        ),
        (
            "extern \"c\" fn foreign() -> Int\nfn main() -> Int { return 0 }\n",
            "extern",
        ),
    ];

    for (text, feature) in cases {
        let analysis = analyze_text(text);
        let feature_diagnostics = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.0 == "KLD0004")
            .collect::<Vec<_>>();
        assert_eq!(
            feature_diagnostics.len(),
            1,
            "{feature}: {:#?}",
            analysis.diagnostics
        );
        assert!(
            feature_diagnostics[0].primary.message.contains(feature),
            "{feature}: {:#?}",
            feature_diagnostics[0]
        );
    }
}

#[test]
fn outer_unsupported_construct_suppresses_child_feature_cascades() {
    let analysis = analyze_text(
        "fn main() -> Int { try { try { return 0 } handle Error as inner { return 0 } } handle Error as outer { return 0 } }\n",
    );
    let feature_diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.0 == "KLD0004")
        .collect::<Vec<_>>();

    assert_eq!(feature_diagnostics.len(), 1, "{:#?}", analysis.diagnostics);
    assert!(feature_diagnostics[0].primary.message.contains("try"));
}

#[test]
fn direct_and_mutual_recursion_are_deferred() {
    for text in [
        "fn loop() -> Int { return loop() }\nfn main() -> Int { return 0 }\n",
        "fn a() -> Int { return b() }\nfn b() -> Int { return a() }\nfn main() -> Int { return 0 }\n",
    ] {
        let analysis = analyze_text(text);
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.0 == "KLD0004" && diagnostic.primary.message.contains("recursive")
        }));
    }
}
