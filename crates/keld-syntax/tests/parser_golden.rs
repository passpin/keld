use keld_source::{SourceId, SourceText};
use keld_syntax::{SyntaxKind, SyntaxNode, lex, parse};

fn parse_text(text: &str) -> (SourceText, keld_syntax::ParsedFile) {
    let source = SourceText::from_str(SourceId(11), text).expect("valid test source");
    let parsed = parse(lex(&source));
    (source, parsed)
}

fn direct_node_kinds(node: &SyntaxNode) -> Vec<SyntaxKind> {
    node.child_nodes().map(|child| child.kind).collect()
}

fn assert_accepted(text: &str, expected_roots: &[SyntaxKind]) {
    let (source, parsed) = parse_text(text);
    assert!(
        parsed.diagnostics.is_empty(),
        "{text}\n{:#?}",
        parsed.diagnostics
    );
    assert_eq!(direct_node_kinds(&parsed.root), expected_roots, "{text}");
    assert_eq!(parsed.reconstruct(&source), source.text(), "{text}");

    let actual_ids = parsed.root.token_ids().map(|id| id.0).collect::<Vec<_>>();
    let expected_ids = (0..u64::try_from(parsed.lexed.tokens.len()).unwrap()).collect::<Vec<_>>();
    assert_eq!(
        actual_ids, expected_ids,
        "each token must occur exactly once"
    );
}

#[test]
fn parses_shift_below_addition_and_above_comparison() {
    let (_, parsed) = parse_text("fn main() -> Int { return 1 + 2 << 3 < 40 }\n");

    assert!(parsed.diagnostics.is_empty());
    assert_eq!(
        parsed
            .root
            .descendant_kinds()
            .filter(|kind| **kind == SyntaxKind::ShiftExpr)
            .count(),
        1
    );
    assert_eq!(
        parsed
            .root
            .descendant_kinds()
            .filter(|kind| **kind == SyntaxKind::ComparisonExpr)
            .count(),
        1
    );
    assert_eq!(
        parsed
            .root
            .descendant_kinds()
            .filter(|kind| **kind == SyntaxKind::AdditiveExpr)
            .count(),
        1
    );
}

#[test]
fn accepts_every_top_level_form_and_type_clause() {
    assert_accepted(
        r#"unsafe module game.world
use game.math
use game.render.Color
pub struct Pair[A, B] {
first: A
second: B
}
entity Enemy {
target: link Enemy?
payload: Pair[Int, Int]?
}
enum Choice[T] {
None
One(T)
Pair(T, T)
}
pub fn choose(take value: Pair[Int, Int], enemy: link Enemy?) -> Pair[Int, Int] raises IoError, ParseError retires enemy, any Enemy {
return value
}
extern "c" fn foreign(value: Int) -> Int raises IoError
"#,
        &[
            SyntaxKind::ModuleDecl,
            SyntaxKind::UseDecl,
            SyntaxKind::UseDecl,
            SyntaxKind::StructDecl,
            SyntaxKind::EntityDecl,
            SyntaxKind::EnumDecl,
            SyntaxKind::FunctionDecl,
            SyntaxKind::ExternFunctionDecl,
        ],
    );
}

#[test]
fn accepts_every_statement_family() {
    assert_accepted(
        r"fn statements(flag: Bool, target: link Enemy) -> Int {
let x: Int = 1
var y: Int
y = x
y += 1
keep target in level
retire target
lifecycle level {
let nested = 0
}
when target as live {
return live.health
} else {
return 0
}
if flag {
return 1
} else if false {
return 2
} else {
return 3
}
while flag {
break
continue
}
try {
fallible()
} handle IoError as error {
observe(error)
} handle ParseError as error {
observe(error)
}
return
}
",
        &[SyntaxKind::FunctionDecl],
    );
}

#[test]
fn accepts_every_expression_and_pattern_family() {
    assert_accepted(
        r"fn expressions(a: Int, b: Int, value: Pair) -> Int {
let arithmetic = -a * b / 2 % 3 + 4 - 5 << 1 >> 1
let logic = !false && a == b || a != b
let ordering = a < b
let postfix = make(first: a, second: b).field[0](a)
let moved = take value.field[0]
match postfix {
_ => 0
none => 1
true => 2
Pair(1, nested) => nested
game.Pair(a, b) => { return a }
}
}
",
        &[SyntaxKind::FunctionDecl],
    );
}

#[test]
fn virtual_terms_before_else_and_handle_are_accepted() {
    assert_accepted(
        "fn main() -> Int { if true { return 1 }\nelse { return 0 } }\n",
        &[SyntaxKind::FunctionDecl],
    );
    assert_accepted(
        "fn main() -> Int { try { return 1 }\nhandle Error as error { return 0 } }\n",
        &[SyntaxKind::FunctionDecl],
    );
}

#[test]
fn typed_ast_wrappers_project_the_immutable_tree() {
    let (source, parsed) = parse_text(
        "module app\nuse app.io\nfn main(value: Int) -> Int retires any Enemy { return value }\n",
    );
    assert!(parsed.diagnostics.is_empty());

    let file = parsed.ast();
    assert!(file.module().is_some());
    assert_eq!(file.uses().count(), 1);
    assert_eq!(file.items().count(), 1);

    let function = file.functions().next().unwrap();
    let name = function.name();
    let name_index = usize::try_from(name.token_id().0).unwrap();
    let name_span = parsed.lexed.tokens[name_index].span;
    assert_eq!(source.slice(name_span), Some("main"));
    assert_eq!(function.parameters().count(), 1);
    assert!(function.return_type().is_some());
    assert_eq!(function.retirement_targets().count(), 1);
    assert_eq!(function.body().syntax().kind, SyntaxKind::Block);
}

#[test]
fn accepts_the_maximum_supported_recursive_depth() {
    let expression = format!("{}1{}", "(".repeat(256), ")".repeat(256));
    let text = format!("fn main() -> Int {{ return {expression} }}\n");
    let (source, parsed) = parse_text(&text);

    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    assert_eq!(parsed.reconstruct(&source), source.text());
}

#[test]
fn accepts_maximum_unary_type_pattern_and_block_depths() {
    let unary = format!("{}1", "-".repeat(256));
    let (_, parsed) = parse_text(&format!("fn main() -> Int {{ return {unary} }}\n"));
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);

    let nested_type = format!("{}Int{}", "List[".repeat(255), "]".repeat(255));
    let (_, parsed) = parse_text(&format!(
        "fn main(value: {nested_type}) -> Int {{ return 0 }}\n"
    ));
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);

    let nested_pattern = format!("{}_{suffix}", "Pair(".repeat(255), suffix = ")".repeat(255));
    let (_, parsed) = parse_text(&format!(
        "fn main() -> Int {{ match 0 {{ {nested_pattern} => 0 }} }}\n"
    ));
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);

    let nested_blocks = format!(
        "{}return{}",
        "lifecycle level {".repeat(255),
        "}".repeat(255)
    );
    let (_, parsed) = parse_text(&format!("fn main() {{ {nested_blocks} }}\n"));
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
}
