use crate::{Keyword, Punct, Token, TokenKind};
use keld_source::{Diagnostic, DiagnosticCode, SourceText, Span, sort_diagnostics};

const LEXICAL_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0002");

pub struct Lexed {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    #[must_use]
    pub fn reconstruct(&self, source: &SourceText) -> String {
        let mut reconstructed = String::with_capacity(source.text().len());
        for token in self.tokens.iter().filter(|token| !token.synthetic) {
            if let Some(text) = source.slice(token.span) {
                reconstructed.push_str(text);
            }
        }
        reconstructed
    }

    #[must_use]
    pub fn significant_kinds(&self) -> Vec<TokenKind> {
        self.tokens
            .iter()
            .filter_map(|token| match token.kind {
                TokenKind::Whitespace
                | TokenKind::LineComment
                | TokenKind::BlockComment
                | TokenKind::Eof => None,
                kind => Some(kind),
            })
            .collect()
    }
}

#[must_use]
pub fn lex(source: &SourceText) -> Lexed {
    Lexer::new(source).run()
}

struct Lexer<'source> {
    source: &'source SourceText,
    text: &'source str,
    offset: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    parenthesis_depth: u32,
    bracket_depth: u32,
    previous_can_end_statement: bool,
    last_significant_is_term: bool,
}

impl<'source> Lexer<'source> {
    fn new(source: &'source SourceText) -> Self {
        Self {
            source,
            text: source.text(),
            offset: 0,
            tokens: Vec::new(),
            diagnostics: Vec::new(),
            parenthesis_depth: 0,
            bracket_depth: 0,
            previous_can_end_statement: false,
            last_significant_is_term: false,
        }
    }

    fn run(mut self) -> Lexed {
        while self.offset < self.text.len() {
            let byte = self.text.as_bytes()[self.offset];
            match byte {
                b' ' | b'\t' | b'\n' => self.scan_whitespace(),
                b'a'..=b'z' | b'A'..=b'Z' => self.scan_identifier(),
                b'_' if self.next_byte().is_some_and(is_identifier_continue) => {
                    self.scan_identifier();
                }
                b'0'..=b'9' => self.scan_integer(),
                b'"' => self.scan_string(),
                b'/' if self.starts_with("//") => self.scan_line_comment(),
                b'/' if self.starts_with("/*") => self.scan_block_comment(),
                b'}' => {
                    self.insert_term_before_boundary(self.offset);
                    self.scan_punctuation();
                }
                _ if punctuation(self.remaining()).is_some() => self.scan_punctuation(),
                _ => self.scan_unknown_character(),
            }
        }

        self.insert_term_before_boundary(self.offset);
        self.emit(TokenKind::Eof, self.offset, self.offset, true);
        sort_diagnostics(&mut self.diagnostics);
        Lexed {
            tokens: self.tokens,
            diagnostics: self.diagnostics,
        }
    }

    fn scan_whitespace(&mut self) {
        let start = self.offset;
        let mut newlines = 0;
        while let Some(byte) = self.text.as_bytes().get(self.offset).copied() {
            match byte {
                b' ' | b'\t' => self.offset += 1,
                b'\n' => {
                    self.offset += 1;
                    newlines += 1;
                }
                _ => break,
            }
        }
        self.emit(TokenKind::Whitespace, start, self.offset, false);
        self.insert_newline_terms(newlines, self.offset);
    }

    fn scan_identifier(&mut self) {
        let start = self.offset;
        self.offset += 1;
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .copied()
            .is_some_and(is_identifier_continue)
        {
            self.offset += 1;
        }

        let identifier = &self.text[start..self.offset];
        let kind =
            Keyword::from_identifier(identifier).map_or(TokenKind::Ident, TokenKind::Keyword);
        self.emit_language(kind, start, self.offset);
    }

    fn scan_integer(&mut self) {
        let start = self.offset;
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
        {
            self.offset += 1;
        }

        let bytes = &self.text.as_bytes()[start..self.offset];
        let valid = bytes.iter().enumerate().all(|(index, byte)| {
            *byte != b'_'
                || (index > 0
                    && index + 1 < bytes.len()
                    && bytes[index - 1].is_ascii_digit()
                    && bytes[index + 1].is_ascii_digit())
        });
        if !valid {
            self.error(
                start,
                self.offset,
                "integer underscores must occur between digits",
            );
        }
        self.emit_language(TokenKind::Int, start, self.offset);
    }

    fn scan_string(&mut self) {
        let start = self.offset;
        self.offset += 1;
        let mut terminated = false;

        while self.offset < self.text.len() {
            match self.text.as_bytes()[self.offset] {
                b'"' => {
                    self.offset += 1;
                    terminated = true;
                    break;
                }
                b'\n' => break,
                b'\\' => self.scan_escape(),
                _ => self.advance_character(),
            }
        }

        if !terminated {
            self.error(start, self.offset, "unterminated string literal");
        }
        self.emit_language(TokenKind::String, start, self.offset);
    }

    fn scan_escape(&mut self) {
        let start = self.offset;
        self.offset += 1;
        let Some(byte) = self.text.as_bytes().get(self.offset).copied() else {
            self.error(start, self.offset, "incomplete string escape");
            return;
        };

        match byte {
            b'0' | b'n' | b'r' | b't' | b'\\' | b'"' => self.offset += 1,
            b'u' => self.scan_unicode_escape(start),
            b'\n' => self.error(start, self.offset, "incomplete string escape"),
            _ => {
                self.advance_character();
                self.error(start, self.offset, "unknown string escape");
            }
        }
    }

    fn scan_unicode_escape(&mut self, escape_start: usize) {
        self.offset += 1;
        if self.text.as_bytes().get(self.offset) != Some(&b'{') {
            self.error(
                escape_start,
                self.offset,
                "Unicode escape requires an opening brace",
            );
            return;
        }
        self.offset += 1;
        let digits_start = self.offset;
        while let Some(byte) = self.text.as_bytes().get(self.offset).copied() {
            if byte == b'}' || byte == b'\n' || byte == b'"' {
                break;
            }
            self.advance_character();
        }

        let digits_end = self.offset;
        let closed = self.text.as_bytes().get(self.offset) == Some(&b'}');
        if closed {
            self.offset += 1;
        }
        let digits = &self.text[digits_start..digits_end];
        let value = u32::from_str_radix(digits, 16).ok();
        let valid = closed
            && (1..=6).contains(&digits.len())
            && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
            && value.and_then(char::from_u32).is_some();
        if !valid {
            self.error(escape_start, self.offset, "invalid Unicode scalar escape");
        }
    }

    fn scan_line_comment(&mut self) {
        let start = self.offset;
        self.offset += 2;
        while self.offset < self.text.len() && self.text.as_bytes()[self.offset] != b'\n' {
            self.advance_character();
        }
        self.emit(TokenKind::LineComment, start, self.offset, false);
    }

    fn scan_block_comment(&mut self) {
        let start = self.offset;
        self.offset += 2;
        let mut newlines = 0;
        let mut terminated = false;
        while self.offset < self.text.len() {
            if self.starts_with("*/") {
                self.offset += 2;
                terminated = true;
                break;
            }
            if self.text.as_bytes()[self.offset] == b'\n' {
                newlines += 1;
                self.offset += 1;
            } else {
                self.advance_character();
            }
        }
        if !terminated {
            self.error(start, self.offset, "unterminated block comment");
        }
        self.emit(TokenKind::BlockComment, start, self.offset, false);
        self.insert_newline_terms(newlines, self.offset);
    }

    fn scan_punctuation(&mut self) {
        let start = self.offset;
        let (punct, length) = punctuation(self.remaining())
            .expect("scan_punctuation is called only for recognized punctuation");
        self.offset += length;

        if punct == Punct::Semicolon {
            self.emit(TokenKind::Term, start, self.offset, false);
            self.previous_can_end_statement = false;
            self.last_significant_is_term = true;
            return;
        }

        match punct {
            Punct::LParen => self.parenthesis_depth = self.parenthesis_depth.saturating_add(1),
            Punct::RParen => self.parenthesis_depth = self.parenthesis_depth.saturating_sub(1),
            Punct::LBracket => self.bracket_depth = self.bracket_depth.saturating_add(1),
            Punct::RBracket => self.bracket_depth = self.bracket_depth.saturating_sub(1),
            _ => {}
        }
        self.emit_language(TokenKind::Punct(punct), start, self.offset);
    }

    fn scan_unknown_character(&mut self) {
        let start = self.offset;
        self.advance_character();
        self.error(start, self.offset, "unknown character");
        self.emit_language(TokenKind::Error, start, self.offset);
    }

    fn insert_newline_terms(&mut self, count: usize, position: usize) {
        if self.parenthesis_depth == 0 && self.bracket_depth == 0 && self.previous_can_end_statement
        {
            for _ in 0..count {
                self.emit(TokenKind::Term, position, position, true);
            }
            if count > 0 {
                self.last_significant_is_term = true;
            }
        }
    }

    fn insert_term_before_boundary(&mut self, position: usize) {
        if self.previous_can_end_statement && !self.last_significant_is_term {
            self.emit(TokenKind::Term, position, position, true);
            self.last_significant_is_term = true;
        }
    }

    fn emit_language(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.previous_can_end_statement = can_end_statement(kind);
        self.last_significant_is_term = false;
        self.emit(kind, start, end, false);
    }

    fn emit(&mut self, kind: TokenKind, start: usize, end: usize, synthetic: bool) {
        let start = u32::try_from(start).expect("source offsets fit in u32");
        let end = u32::try_from(end).expect("source offsets fit in u32");
        let span = Span::new(self.source.id(), start, end).expect("lexer spans are ordered");
        self.tokens.push(Token {
            kind,
            span,
            synthetic,
        });
    }

    fn error(&mut self, start: usize, end: usize, message: &str) {
        let start = u32::try_from(start).expect("source offsets fit in u32");
        let end = u32::try_from(end).expect("source offsets fit in u32");
        let span = Span::new(self.source.id(), start, end).expect("diagnostic spans are ordered");
        self.diagnostics
            .push(Diagnostic::error(LEXICAL_DIAGNOSTIC, span, message));
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.remaining().starts_with(prefix)
    }

    fn remaining(&self) -> &str {
        &self.text[self.offset..]
    }

    fn next_byte(&self) -> Option<u8> {
        self.text.as_bytes().get(self.offset + 1).copied()
    }

    fn advance_character(&mut self) {
        let character = self
            .remaining()
            .chars()
            .next()
            .expect("advance_character requires a remaining character");
        self.offset += character.len_utf8();
    }
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn can_end_statement(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident
            | TokenKind::Int
            | TokenKind::String
            | TokenKind::Keyword(
                Keyword::True
                    | Keyword::False
                    | Keyword::None
                    | Keyword::Break
                    | Keyword::Continue
                    | Keyword::Return
            )
            | TokenKind::Punct(Punct::RParen | Punct::RBracket | Punct::RBrace | Punct::Question)
    )
}

fn punctuation(text: &str) -> Option<(Punct, usize)> {
    let matched = if text.starts_with("=>") {
        (Punct::FatArrow, 2)
    } else if text.starts_with("->") {
        (Punct::Arrow, 2)
    } else if text.starts_with("+=") {
        (Punct::PlusEq, 2)
    } else if text.starts_with("-=") {
        (Punct::MinusEq, 2)
    } else if text.starts_with("*=") {
        (Punct::StarEq, 2)
    } else if text.starts_with("/=") {
        (Punct::SlashEq, 2)
    } else if text.starts_with("%=") {
        (Punct::PercentEq, 2)
    } else if text.starts_with("!=") {
        (Punct::BangEq, 2)
    } else if text.starts_with("==") {
        (Punct::EqEq, 2)
    } else if text.starts_with("<=") {
        (Punct::LessEq, 2)
    } else if text.starts_with(">=") {
        (Punct::GreaterEq, 2)
    } else if text.starts_with("<<") {
        (Punct::Shl, 2)
    } else if text.starts_with(">>") {
        (Punct::Shr, 2)
    } else if text.starts_with("&&") {
        (Punct::AndAnd, 2)
    } else if text.starts_with("||") {
        (Punct::OrOr, 2)
    } else {
        let punct = match text.as_bytes().first().copied()? {
            b'(' => Punct::LParen,
            b')' => Punct::RParen,
            b'{' => Punct::LBrace,
            b'}' => Punct::RBrace,
            b'[' => Punct::LBracket,
            b']' => Punct::RBracket,
            b',' => Punct::Comma,
            b'.' => Punct::Dot,
            b':' => Punct::Colon,
            b'?' => Punct::Question,
            b';' => Punct::Semicolon,
            b'_' => Punct::Underscore,
            b'=' => Punct::Eq,
            b'+' => Punct::Plus,
            b'-' => Punct::Minus,
            b'*' => Punct::Star,
            b'/' => Punct::Slash,
            b'%' => Punct::Percent,
            b'!' => Punct::Bang,
            b'<' => Punct::Less,
            b'>' => Punct::Greater,
            _ => return None,
        };
        (punct, 1)
    };
    Some(matched)
}
