use super::Parser;
use crate::{Keyword, Punct, SyntaxKind, TokenKind};

impl Parser<'_> {
    pub(super) fn parse_block(&mut self) {
        if self.block_depth >= super::MAX_RECURSIVE_DEPTH {
            self.depth_error("block");
            self.recover_balanced(Punct::LBrace, Punct::RBrace);
            return;
        }
        self.block_depth += 1;
        let marker = self.start();
        if !self.expect_punct(Punct::LBrace, "`{`") {
            self.complete(marker, SyntaxKind::Block);
            self.block_depth -= 1;
            return;
        }
        self.skip_terms();
        while !self.at_punct(Punct::RBrace) && !self.at_eof() {
            self.parse_statement();
            self.require_term("after a statement");
            self.skip_terms();
        }
        self.expect_punct(Punct::RBrace, "`}`");
        self.complete(marker, SyntaxKind::Block);
        self.block_depth -= 1;
    }

    fn parse_statement(&mut self) {
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Let | Keyword::Var) => self.parse_binding_statement(),
            TokenKind::Keyword(Keyword::Keep) => self.parse_keep_statement(),
            TokenKind::Keyword(Keyword::Retire) => self.parse_retire_statement(),
            TokenKind::Keyword(Keyword::Return) => self.parse_return_statement(),
            TokenKind::Keyword(Keyword::Break) => {
                self.parse_keyword_statement(Keyword::Break, SyntaxKind::BreakStmt);
            }
            TokenKind::Keyword(Keyword::Continue) => {
                self.parse_keyword_statement(Keyword::Continue, SyntaxKind::ContinueStmt);
            }
            TokenKind::Keyword(Keyword::Lifecycle) => self.parse_lifecycle_statement(),
            TokenKind::Keyword(Keyword::When) => self.parse_when_statement(),
            TokenKind::Keyword(Keyword::If) => self.parse_if_statement(),
            TokenKind::Keyword(Keyword::While) => self.parse_while_statement(),
            TokenKind::Keyword(Keyword::Try) => self.parse_try_statement(),
            _ if self.looks_like_assignment() => self.parse_assignment_statement(),
            _ => self.parse_expression_statement(),
        }
    }

    fn parse_binding_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_name("a binding name");
        if self.eat_punct(Punct::Colon) {
            self.parse_type();
        }
        if self.eat_punct(Punct::Eq) {
            self.parse_expression();
        }
        self.complete(marker, SyntaxKind::BindingStmt);
    }

    fn parse_keep_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_expression();
        self.expect_keyword(Keyword::In, "`in`");
        self.parse_name("a lifecycle name");
        self.complete(marker, SyntaxKind::KeepStmt);
    }

    fn parse_retire_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_expression();
        self.complete(marker, SyntaxKind::RetireStmt);
    }

    fn parse_return_statement(&mut self) {
        let marker = self.start();
        self.bump();
        if !self.at(TokenKind::Term) && !self.at_punct(Punct::RBrace) && !self.at_eof() {
            self.parse_expression();
        }
        self.complete(marker, SyntaxKind::ReturnStmt);
    }

    fn parse_keyword_statement(&mut self, keyword: Keyword, kind: SyntaxKind) {
        let marker = self.start();
        self.expect_keyword(keyword, "a statement keyword");
        self.complete(marker, kind);
    }

    fn parse_lifecycle_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_name("a lifecycle name");
        self.parse_block();
        self.complete(marker, SyntaxKind::LifecycleStmt);
    }

    fn parse_when_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_expression();
        self.expect_keyword(Keyword::As, "`as`");
        self.parse_name("a binding name");
        self.parse_block();
        if self.at_keyword_after_terms(Keyword::Else) {
            self.skip_terms();
            self.bump();
            self.parse_block();
        }
        self.complete(marker, SyntaxKind::WhenStmt);
    }

    fn parse_if_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_expression();
        self.parse_block();
        while self.at_keyword_after_terms(Keyword::Else) {
            self.skip_terms();
            self.bump();
            if self.eat_keyword(Keyword::If) {
                self.parse_expression();
                self.parse_block();
            } else {
                self.parse_block();
                break;
            }
        }
        self.complete(marker, SyntaxKind::IfStmt);
    }

    fn parse_while_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_expression();
        self.parse_block();
        self.complete(marker, SyntaxKind::WhileStmt);
    }

    fn parse_try_statement(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_block();
        if !self.at_keyword_after_terms(Keyword::Handle) {
            self.error_expected(&["a `handle` clause"]);
        }
        while self.at_keyword_after_terms(Keyword::Handle) {
            self.skip_terms();
            let clause = self.start();
            self.bump();
            self.parse_type();
            self.expect_keyword(Keyword::As, "`as`");
            self.parse_name("an error binding name");
            self.parse_block();
            self.complete(clause, SyntaxKind::HandleClause);
        }
        self.complete(marker, SyntaxKind::TryStmt);
    }

    fn parse_assignment_statement(&mut self) {
        let marker = self.start();
        self.parse_place();
        if matches!(
            self.current_kind(),
            TokenKind::Punct(
                Punct::Eq
                    | Punct::PlusEq
                    | Punct::MinusEq
                    | Punct::StarEq
                    | Punct::SlashEq
                    | Punct::PercentEq
            )
        ) {
            self.bump();
        } else {
            self.error_expected(&["an assignment operator"]);
        }
        self.parse_expression();
        self.complete(marker, SyntaxKind::AssignmentStmt);
    }

    fn parse_expression_statement(&mut self) {
        let marker = self.start();
        let before = self.position;
        self.parse_expression();
        if self.position == before && !self.at_eof() {
            self.bump();
        }
        self.complete(marker, SyntaxKind::ExprStmt);
    }

    pub(super) fn parse_place(&mut self) {
        let marker = self.start();
        self.parse_name("a place name");
        loop {
            if self.eat_punct(Punct::Dot) {
                self.parse_name("a field name");
            } else if self.eat_punct(Punct::LBracket) {
                self.parse_expression();
                self.expect_punct(Punct::RBracket, "`]`");
            } else {
                break;
            }
        }
        self.complete(marker, SyntaxKind::Place);
    }

    fn looks_like_assignment(&self) -> bool {
        if self.current_kind() != TokenKind::Ident {
            return false;
        }
        let mut significant = 0;
        let mut paren_depth = 0_u32;
        let mut bracket_depth = 0_u32;
        loop {
            let kind = self.nth_kind(significant);
            match kind {
                TokenKind::Punct(Punct::LParen) => paren_depth += 1,
                TokenKind::Punct(Punct::RParen) => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::Punct(Punct::LBracket) => bracket_depth += 1,
                TokenKind::Punct(Punct::RBracket) => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                }
                TokenKind::Punct(
                    Punct::Eq
                    | Punct::PlusEq
                    | Punct::MinusEq
                    | Punct::StarEq
                    | Punct::SlashEq
                    | Punct::PercentEq,
                ) if paren_depth == 0 && bracket_depth == 0 => return true,
                TokenKind::Term | TokenKind::Eof | TokenKind::Punct(Punct::RBrace)
                    if paren_depth == 0 && bracket_depth == 0 =>
                {
                    return false;
                }
                _ => {}
            }
            significant += 1;
        }
    }

    fn at_keyword_after_terms(&self, keyword: Keyword) -> bool {
        let mut significant = 0;
        while self.nth_kind(significant) == TokenKind::Term {
            significant += 1;
        }
        self.nth_kind(significant) == TokenKind::Keyword(keyword)
    }
}
