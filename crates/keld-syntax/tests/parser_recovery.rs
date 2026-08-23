use keld_source::{SourceId, SourceText};
use keld_syntax::{SyntaxKind, lex, parse};

fn parse_text(text: &str) -> (SourceText, keld_syntax::ParsedFile) {
    let source = SourceText::from_str(SourceId(12), text).expect("valid test source");
    let parsed = parse(lex(&source));
    (source, parsed)
}

fn assert_parser_error(text: &str) {
    let (source, parsed) = parse_text(text);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD0003"),
        "{text}\n{:#?}",
        parsed.diagnostics
    );
    assert_eq!(parsed.reconstruct(&source), source.text());
    let actual_ids = parsed.root.token_ids().map(|id| id.0).collect::<Vec<_>>();
    let expected_ids = (0..u64::try_from(parsed.lexed.tokens.len()).unwrap()).collect::<Vec<_>>();
    assert_eq!(
        actual_ids, expected_ids,
        "each token must occur exactly once"
    );
}

#[test]
fn recovers_at_term_and_keeps_the_following_function() {
    let (source, parsed) = parse_text("fn broken( { return 0 }\nfn main() -> Int { return 1 }\n");

    assert!(!parsed.diagnostics.is_empty());
    assert_eq!(parsed.ast().functions().count(), 2);
    assert_eq!(parsed.reconstruct(&source), source.text());
}

#[test]
fn rejects_comparison_and_equality_chaining_without_losing_tokens() {
    assert_parser_error("fn main() -> Bool { return 1 < 2 < 3 }\n");
    assert_parser_error("fn main() -> Bool { return 1 == 1 != false }\n");
}

#[test]
fn rejects_positional_arguments_after_named_arguments() {
    assert_parser_error("fn main() -> Int { return make(first: 1, 2) }\n");
}

#[test]
fn recovers_delimited_lists_at_comma_or_closing_delimiter() {
    let (_, parsed) = parse_text(
        "fn broken(a Int, b: Int) -> Int { return make(first: 1 second: 2) }\nfn main() -> Int { return 0 }\n",
    );

    assert!(parsed.diagnostics.len() >= 2, "{:#?}", parsed.diagnostics);
    assert_eq!(parsed.ast().functions().count(), 2);
}

#[test]
fn missing_closing_delimiters_are_diagnosed_without_panicking() {
    for text in [
        "fn main() -> Int { return (1 + 2 }\n",
        "fn main() -> Int { return values[0 }\n",
        "fn main(value: List[Int) -> Int { return 0 }\n",
        "struct Point { x: Int\n",
    ] {
        assert_parser_error(text);
    }
}

#[test]
fn depth_257_is_recovered_as_an_error_node() {
    let expression = format!("{}1{}", "(".repeat(257), ")".repeat(257));
    let text = format!("fn main() -> Int {{ return {expression} }}\n");
    let (source, parsed) = parse_text(&text);

    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD0003")
    );
    assert!(
        parsed
            .root
            .descendant_kinds()
            .any(|kind| *kind == SyntaxKind::Error)
    );
    assert_eq!(parsed.reconstruct(&source), source.text());
}

#[test]
fn every_recursive_grammar_family_rejects_depth_257() {
    let unary = format!("{}1", "-".repeat(257));
    assert_parser_error(&format!("fn main() -> Int {{ return {unary} }}\n"));

    let nested_type = format!("{}Int{}", "List[".repeat(256), "]".repeat(256));
    assert_parser_error(&format!(
        "fn main(value: {nested_type}) -> Int {{ return 0 }}\n"
    ));

    let nested_pattern = format!("{}_{suffix}", "Pair(".repeat(256), suffix = ")".repeat(256));
    assert_parser_error(&format!(
        "fn main() -> Int {{ match 0 {{ {nested_pattern} => 0 }} }}\n"
    ));

    let nested_blocks = format!(
        "{}return{}",
        "lifecycle level {".repeat(256),
        "}".repeat(256)
    );
    assert_parser_error(&format!("fn main() {{ {nested_blocks} }}\n"));
}

#[test]
fn arbitrary_error_tokens_never_prevent_forward_progress() {
    let (source, parsed) = parse_text("λ λ λ\nfn main() -> Int { return @@@ }\n");

    assert!(!parsed.diagnostics.is_empty());
    assert_eq!(parsed.ast().functions().count(), 1);
    assert_eq!(parsed.reconstruct(&source), source.text());
}
