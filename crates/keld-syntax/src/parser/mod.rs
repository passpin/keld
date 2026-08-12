mod expression;
mod item;
mod statement;

use crate::green::{Event, build_tree};
use crate::{Keyword, Lexed, ParsedFile, Punct, SyntaxKind, TokenId, TokenKind};
use keld_source::{Diagnostic, DiagnosticCode, Span};

const PARSER_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0003");
const MAX_RECURSIVE_DEPTH: u16 = 256;

pub fn parse(lexed: Lexed) -> ParsedFile {
    let fallback_span = lexed
        .tokens
        .last()
        .map_or_else(fallback_span, |token| token.span);
    let events = {
        let mut parser = Parser::new(&lexed);
        parser.parse_source_file();
        parser.events
    };
    build_tree(lexed, events, fallback_span)
}

fn fallback_span() -> Span {
    Span::new(keld_source::SourceId(0), 0, 0).expect("empty fallback span is valid")
}

#[derive(Clone, Copy)]
struct Marker {
    position: usize,
}

#[derive(Clone, Copy)]
struct CompletedMarker {
    position: usize,
    kind: SyntaxKind,
}

struct Parser<'lexed> {
    lexed: &'lexed Lexed,
    position: usize,
    events: Vec<Event>,
    block_depth: u16,
    expression_depth: u16,
    pattern_depth: u16,
    type_depth: u16,
}

impl<'lexed> Parser<'lexed> {
    fn new(lexed: &'lexed Lexed) -> Self {
        Self {
            lexed,
            position: 0,
            events: Vec::new(),
            block_depth: 0,
            expression_depth: 0,
            pattern_depth: 0,
            type_depth: 0,
        }
    }

    fn parse_source_file(&mut self) {
        let marker = self.start();
        self.skip_terms();

        if self.at_keyword(Keyword::Module)
            || (self.at_keyword(Keyword::Unsafe) && self.nth_kind(1) == keyword(Keyword::Module))
        {
            self.parse_module_decl();
            self.require_term("after a module declaration");
            self.skip_terms();
        }

        while self.at_keyword(Keyword::Use) {
            self.parse_use_decl();
            self.require_term("after a use declaration");
            self.skip_terms();
        }

        while !self.at_eof() {
            if self.is_item_start() {
                self.parse_item();
                self.require_term("after an item");
            } else if self.at(TokenKind::Term) {
                self.bump();
            } else {
                self.error_expected(&["an item", "end of file"]);
                self.recover_top_level();
            }
            self.skip_terms();
        }

        self.bump_trivia();
        if self.at(TokenKind::Eof) {
            self.bump();
        }
        self.complete(marker, SyntaxKind::SourceFile);
    }

    fn start(&mut self) -> Marker {
        let position = self.events.len();
        self.events.push(Event::Start {
            kind: None,
            forward_parent: None,
        });
        Marker { position }
    }

    fn complete(&mut self, marker: Marker, kind: SyntaxKind) -> CompletedMarker {
        match &mut self.events[marker.position] {
            Event::Start {
                kind: event_kind, ..
            } => *event_kind = Some(kind),
            _ => debug_assert!(false, "marker must refer to a start event"),
        }
        self.events.push(Event::Finish);
        CompletedMarker {
            position: marker.position,
            kind,
        }
    }

    fn precede(&mut self, completed: CompletedMarker) -> Marker {
        let marker = self.start();
        let distance = marker.position - completed.position;
        match &mut self.events[completed.position] {
            Event::Start { forward_parent, .. } => *forward_parent = Some(distance),
            _ => debug_assert!(false, "completed marker must refer to a start event"),
        }
        marker
    }

    fn current_kind(&self) -> TokenKind {
        self.nth_kind(0)
    }

    fn nth_kind(&self, nth: usize) -> TokenKind {
        let mut remaining = nth;
        for token in &self.lexed.tokens[self.position..] {
            if is_trivia(token.kind) {
                continue;
            }
            if remaining == 0 {
                return token.kind;
            }
            remaining -= 1;
        }
        TokenKind::Eof
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current_kind() == kind
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        self.at(TokenKind::Keyword(keyword))
    }

    fn at_punct(&self, punct: Punct) -> bool {
        self.at(TokenKind::Punct(punct))
    }

    fn at_eof(&self) -> bool {
        self.at(TokenKind::Eof)
    }

    fn bump(&mut self) {
        self.bump_trivia();
        self.bump_raw();
    }

    fn bump_raw(&mut self) {
        if self.position >= self.lexed.tokens.len() {
            return;
        }
        let id = TokenId(u64::try_from(self.position).unwrap_or(u64::MAX));
        self.events.push(Event::Token(id));
        self.position += 1;
    }

    fn bump_trivia(&mut self) {
        while self
            .lexed
            .tokens
            .get(self.position)
            .is_some_and(|token| is_trivia(token.kind))
        {
            self.bump_raw();
        }
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        self.eat(TokenKind::Keyword(keyword))
    }

    fn eat_punct(&mut self, punct: Punct) -> bool {
        self.eat(TokenKind::Punct(punct))
    }

    fn expect(&mut self, kind: TokenKind, expected: &'static str) -> bool {
        if self.eat(kind) {
            true
        } else {
            self.error_expected(&[expected]);
            false
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword, expected: &'static str) -> bool {
        self.expect(TokenKind::Keyword(keyword), expected)
    }

    fn expect_punct(&mut self, punct: Punct, expected: &'static str) -> bool {
        self.expect(TokenKind::Punct(punct), expected)
    }

    fn expect_ident(&mut self, expected: &'static str) -> bool {
        self.expect(TokenKind::Ident, expected)
    }

    fn parse_name(&mut self, expected: &'static str) {
        self.bump_trivia();
        let marker = self.start();
        self.expect_ident(expected);
        self.complete(marker, SyntaxKind::Name);
    }

    fn parse_path(&mut self) {
        let marker = self.start();
        self.parse_name("a path name");
        while self.eat_punct(Punct::Dot) {
            self.parse_name("a path name after `.`");
        }
        self.complete(marker, SyntaxKind::Path);
    }

    fn skip_terms(&mut self) {
        while self.at(TokenKind::Term) {
            self.bump();
        }
    }

    fn require_term(&mut self, context: &'static str) {
        if !self.at(TokenKind::Term) && !self.at_eof() && !self.at_punct(Punct::RBrace) {
            self.error_message(format!("expected a statement terminator {context}"));
            return;
        }
        self.skip_terms();
    }

    fn error_expected(&mut self, expected: &[&str]) {
        let found = format_token(self.current_kind());
        self.error_message(format!(
            "unexpected {found}; expected {}",
            expected.join(" or ")
        ));
    }

    fn error_message(&mut self, message: String) {
        let span = self.current_span();
        self.events.push(Event::Error(Diagnostic::error(
            PARSER_DIAGNOSTIC,
            span,
            message,
        )));
    }

    fn current_span(&self) -> Span {
        self.lexed
            .tokens
            .iter()
            .skip(self.position)
            .find(|token| !is_trivia(token.kind))
            .or_else(|| self.lexed.tokens.last())
            .map_or_else(fallback_span, |token| token.span)
    }

    fn recover_top_level(&mut self) {
        let marker = self.start();
        let start = self.position;
        while !self.at_eof() && !self.at(TokenKind::Term) && !self.is_item_start() {
            self.bump();
        }
        if self.position == start && !self.at_eof() {
            self.bump();
        }
        self.complete(marker, SyntaxKind::Error);
    }

    fn recover_until(&mut self, punct: Punct) {
        let marker = self.start();
        let start = self.position;
        while !self.at_eof()
            && !self.at(TokenKind::Term)
            && !self.at_punct(Punct::Comma)
            && !self.at_punct(punct)
            && !self.at_punct(Punct::RBrace)
        {
            self.bump();
        }
        if self.position == start && !self.at_eof() && !self.at_punct(punct) {
            self.bump();
        }
        self.complete(marker, SyntaxKind::Error);
    }

    fn recover_balanced(&mut self, open: Punct, close: Punct) -> CompletedMarker {
        let marker = self.start();
        let mut depth = 0_u32;
        while !self.at_eof() {
            if self.at_punct(open) {
                depth = depth.saturating_add(1);
                self.bump();
            } else if self.at_punct(close) {
                self.bump();
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            } else {
                self.bump();
            }
        }
        self.complete(marker, SyntaxKind::Error)
    }

    fn depth_error(&mut self, construct: &str) {
        self.error_message(format!(
            "{construct} nesting exceeds the maximum depth of {MAX_RECURSIVE_DEPTH}"
        ));
    }
}

fn is_trivia(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
    )
}

const fn keyword(keyword: Keyword) -> TokenKind {
    TokenKind::Keyword(keyword)
}

fn format_token(kind: TokenKind) -> String {
    match kind {
        TokenKind::Ident => "identifier".to_owned(),
        TokenKind::Int => "integer literal".to_owned(),
        TokenKind::String => "string literal".to_owned(),
        TokenKind::Keyword(value) => format!("keyword `{value:?}`"),
        TokenKind::Punct(value) => format!("punctuation `{value:?}`"),
        TokenKind::Term => "statement terminator".to_owned(),
        TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment => {
            "trivia".to_owned()
        }
        TokenKind::Error => "invalid token".to_owned(),
        TokenKind::Eof => "end of file".to_owned(),
    }
}
