use keld_source::{SourceId, SourceText};
use keld_syntax::{Keyword, Punct, TokenKind, lex};

fn source(text: &str) -> SourceText {
    SourceText::from_str(SourceId(7), text).expect("test source must be valid")
}

fn core_kinds(text: &str) -> Vec<TokenKind> {
    let source = source(text);
    lex(&source)
        .tokens
        .into_iter()
        .filter_map(|token| match token.kind {
            TokenKind::Whitespace
            | TokenKind::LineComment
            | TokenKind::BlockComment
            | TokenKind::Term
            | TokenKind::Eof => None,
            kind => Some(kind),
        })
        .collect()
}

fn significant_kinds(text: &str) -> Vec<TokenKind> {
    let source = source(text);
    lex(&source).significant_kinds()
}

fn term_count(text: &str) -> usize {
    significant_kinds(text)
        .into_iter()
        .filter(|kind| *kind == TokenKind::Term)
        .count()
}

#[test]
fn lexing_is_lossless_and_every_source_byte_has_one_owner() {
    let source = source("let values = List[Text](\"a\\n\") // note\nvalues.copy();\n");
    let lexed = lex(&source);

    assert!(lexed.diagnostics.is_empty());
    assert_eq!(lexed.reconstruct(&source), source.text());

    let mut covered_until = 0;
    for token in lexed.tokens.iter().filter(|token| !token.synthetic) {
        assert_eq!(token.span.start().0, covered_until);
        covered_until = token.span.end().0;
    }
    assert_eq!(covered_until as usize, source.text().len());
}

#[test]
fn all_reserved_words_are_keywords() {
    let cases = [
        ("any", Keyword::Any),
        ("as", Keyword::As),
        ("break", Keyword::Break),
        ("continue", Keyword::Continue),
        ("else", Keyword::Else),
        ("entity", Keyword::Entity),
        ("enum", Keyword::Enum),
        ("extern", Keyword::Extern),
        ("false", Keyword::False),
        ("fn", Keyword::Fn),
        ("handle", Keyword::Handle),
        ("if", Keyword::If),
        ("in", Keyword::In),
        ("keep", Keyword::Keep),
        ("let", Keyword::Let),
        ("lifecycle", Keyword::Lifecycle),
        ("link", Keyword::Link),
        ("match", Keyword::Match),
        ("module", Keyword::Module),
        ("none", Keyword::None),
        ("pub", Keyword::Pub),
        ("raises", Keyword::Raises),
        ("retire", Keyword::Retire),
        ("retires", Keyword::Retires),
        ("return", Keyword::Return),
        ("struct", Keyword::Struct),
        ("take", Keyword::Take),
        ("true", Keyword::True),
        ("try", Keyword::Try),
        ("unsafe", Keyword::Unsafe),
        ("use", Keyword::Use),
        ("var", Keyword::Var),
        ("when", Keyword::When),
        ("while", Keyword::While),
        ("async", Keyword::Async),
        ("await", Keyword::Await),
        ("dynamic", Keyword::Dynamic),
        ("impl", Keyword::Impl),
        ("interface", Keyword::Interface),
        ("resource", Keyword::Resource),
        ("shared", Keyword::Shared),
    ];

    for (spelling, keyword) in cases {
        assert_eq!(
            core_kinds(spelling),
            vec![TokenKind::Keyword(keyword)],
            "{spelling}"
        );
    }
}

#[test]
fn identifier_and_wildcard_boundaries_match_the_grammar() {
    assert_eq!(
        core_kinds("a Z a0 a_b _x _0 __ _"),
        vec![
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Punct(Punct::Underscore),
        ]
    );
}

#[test]
fn punctuation_uses_longest_matching() {
    let text =
        "( ) { } [ ] , . : ? _ = => -> + - * / % += -= *= /= %= ! != == < <= > >= << >> && ||";
    assert_eq!(
        core_kinds(text),
        vec![
            Punct::LParen,
            Punct::RParen,
            Punct::LBrace,
            Punct::RBrace,
            Punct::LBracket,
            Punct::RBracket,
            Punct::Comma,
            Punct::Dot,
            Punct::Colon,
            Punct::Question,
            Punct::Underscore,
            Punct::Eq,
            Punct::FatArrow,
            Punct::Arrow,
            Punct::Plus,
            Punct::Minus,
            Punct::Star,
            Punct::Slash,
            Punct::Percent,
            Punct::PlusEq,
            Punct::MinusEq,
            Punct::StarEq,
            Punct::SlashEq,
            Punct::PercentEq,
            Punct::Bang,
            Punct::BangEq,
            Punct::EqEq,
            Punct::Less,
            Punct::LessEq,
            Punct::Greater,
            Punct::GreaterEq,
            Punct::Shl,
            Punct::Shr,
            Punct::AndAnd,
            Punct::OrOr,
        ]
        .into_iter()
        .map(TokenKind::Punct)
        .collect::<Vec<_>>()
    );

    assert_eq!(
        significant_kinds("a;b"),
        vec![
            TokenKind::Ident,
            TokenKind::Term,
            TokenKind::Ident,
            TokenKind::Term
        ]
    );
}

#[test]
fn strings_and_comments_preserve_normalized_source() {
    let text = "\"\\0\\n\\r\\t\\\\\\\"\\u{41} 한국어\" // 줄\n/* 비중첩 /* 그대로 */ tail */";
    let source = source(text);
    let lexed = lex(&source);

    assert!(lexed.diagnostics.is_empty());
    assert_eq!(lexed.reconstruct(&source), text);
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == TokenKind::String)
    );
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == TokenKind::LineComment)
    );
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == TokenKind::BlockComment)
    );
}

#[test]
fn virtual_terminators_follow_depth_and_previous_token_rules() {
    assert_eq!(
        significant_kinds("a\n\n"),
        vec![TokenKind::Ident, TokenKind::Term, TokenKind::Term]
    );
    assert_eq!(
        significant_kinds("a +\nb"),
        vec![
            TokenKind::Ident,
            TokenKind::Punct(Punct::Plus),
            TokenKind::Ident,
            TokenKind::Term,
        ]
    );
    assert_eq!(
        significant_kinds("(a\nb)"),
        vec![
            TokenKind::Punct(Punct::LParen),
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Punct(Punct::RParen),
            TokenKind::Term,
        ]
    );
    assert_eq!(
        significant_kinds("{a}"),
        vec![
            TokenKind::Punct(Punct::LBrace),
            TokenKind::Ident,
            TokenKind::Term,
            TokenKind::Punct(Punct::RBrace),
            TokenKind::Term,
        ]
    );
    assert_eq!(
        significant_kinds("a/* first\nsecond\n*/b"),
        vec![
            TokenKind::Ident,
            TokenKind::Term,
            TokenKind::Term,
            TokenKind::Ident,
            TokenKind::Term,
        ]
    );
    assert_eq!(
        significant_kinds("(a/* first\nsecond\n*/b)"),
        vec![
            TokenKind::Punct(Punct::LParen),
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::Punct(Punct::RParen),
            TokenKind::Term,
        ]
    );
}

#[test]
fn every_statement_ending_token_qualifies_a_newline() {
    let cases = [
        "name\n",
        "1_000\n",
        "\"text\"\n",
        "true\n",
        "false\n",
        "none\n",
        "break\n",
        "continue\n",
        "return\n",
        ")\n",
        "]\n",
        "}\n",
        "?\n",
        "name // comment\n",
    ];

    for text in cases {
        assert_eq!(term_count(text), 1, "{text:?}");
    }
}

#[test]
fn every_non_ending_operator_suppresses_a_newline() {
    let prefixes = [
        "a +", "a -", "a *", "a /", "a %", "a <<", "a >>", "a <", "a <=", "a >", "a >=", "a ==",
        "a !=", "a &&", "a ||", "a,", "a.", "a:", "a =", "a =>", "a ->", "(", "[", "{",
    ];

    for prefix in prefixes {
        let text = format!("{prefix}\nx");
        assert_eq!(term_count(&text), 1, "{text:?}");
    }
}

#[test]
fn explicit_semicolon_is_the_only_non_synthetic_term() {
    let source = source("a;\n");
    let lexed = lex(&source);
    let terms = lexed
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Term)
        .collect::<Vec<_>>();

    assert_eq!(terms.len(), 1);
    assert!(!terms[0].synthetic);
    assert_eq!(source.slice(terms[0].span), Some(";"));
}

#[test]
fn lexical_errors_are_diagnostic_and_lossless() {
    let cases = [
        "1__0",
        "1_",
        "\"bad\\q\"",
        "\"bad\\u{}\"",
        "\"bad\\u{D800}\"",
        "\"unterminated",
        "/* unterminated",
        "lambda λ",
    ];

    for text in cases {
        let source = source(text);
        let lexed = lex(&source);
        assert!(
            lexed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.0 == "KLD0002"),
            "missing diagnostic for {text:?}"
        );
        assert_eq!(lexed.reconstruct(&source), text, "{text:?}");
    }
}
