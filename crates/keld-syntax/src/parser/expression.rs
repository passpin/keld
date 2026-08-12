use super::{CompletedMarker, Parser};
use crate::{Keyword, Punct, SyntaxKind, TokenKind};

#[derive(Clone, Copy)]
struct BinaryOperator {
    left_binding_power: u8,
    right_binding_power: u8,
    kind: SyntaxKind,
    non_associative: bool,
}

impl Parser<'_> {
    pub(super) fn parse_expression(&mut self) -> CompletedMarker {
        self.parse_binary_expression(1)
    }

    fn parse_binary_expression(&mut self, minimum_binding_power: u8) -> CompletedMarker {
        let mut left = self.parse_prefix_expression();
        while let Some(operator) = binary_operator(self.current_kind()) {
            if operator.left_binding_power < minimum_binding_power {
                break;
            }
            if operator.non_associative && left.kind == operator.kind {
                self.error_message(
                    "comparison and equality operators cannot be chained".to_owned(),
                );
            }
            let marker = self.precede(left);
            self.bump();
            self.parse_binary_expression(operator.right_binding_power);
            left = self.complete(marker, operator.kind);
        }
        left
    }

    fn parse_prefix_expression(&mut self) -> CompletedMarker {
        if matches!(
            self.current_kind(),
            TokenKind::Punct(Punct::Bang | Punct::Minus)
        ) {
            if self.expression_depth >= super::MAX_RECURSIVE_DEPTH {
                self.depth_error("unary expression");
                return self.recover_expression();
            }
            self.expression_depth += 1;
            let marker = self.start();
            self.bump();
            self.parse_prefix_expression();
            let completed = self.complete(marker, SyntaxKind::UnaryExpr);
            self.expression_depth -= 1;
            return completed;
        }

        if self.at_keyword(Keyword::Take) {
            let marker = self.start();
            self.bump();
            self.parse_place();
            return self.complete(marker, SyntaxKind::TakeExpr);
        }

        let primary = self.parse_primary_expression();
        self.parse_postfix_expression(primary)
    }

    fn parse_primary_expression(&mut self) -> CompletedMarker {
        match self.current_kind() {
            TokenKind::Int
            | TokenKind::String
            | TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::None) => {
                let marker = self.start();
                self.bump();
                self.complete(marker, SyntaxKind::LiteralExpr)
            }
            TokenKind::Ident => {
                let marker = self.start();
                self.parse_name("a value name");
                self.complete(marker, SyntaxKind::NameExpr)
            }
            TokenKind::Punct(Punct::LParen) => self.parse_parenthesized_expression(),
            TokenKind::Keyword(Keyword::Match) => self.parse_match_expression(),
            _ => {
                self.error_expected(&["an expression"]);
                let marker = self.start();
                if !is_expression_boundary(self.current_kind()) {
                    self.bump();
                }
                self.complete(marker, SyntaxKind::Error)
            }
        }
    }

    fn parse_parenthesized_expression(&mut self) -> CompletedMarker {
        if self.expression_depth >= super::MAX_RECURSIVE_DEPTH {
            self.depth_error("parenthesized expression");
            return self.recover_balanced(Punct::LParen, Punct::RParen);
        }
        self.expression_depth += 1;
        let marker = self.start();
        self.bump();
        self.parse_expression();
        self.expect_punct(Punct::RParen, "`)`");
        let completed = self.complete(marker, SyntaxKind::ParenthesizedExpr);
        self.expression_depth -= 1;
        completed
    }

    fn parse_postfix_expression(&mut self, mut left: CompletedMarker) -> CompletedMarker {
        loop {
            if self.at_punct(Punct::LParen) {
                let marker = self.precede(left);
                self.parse_argument_list();
                left = self.complete(marker, SyntaxKind::CallExpr);
            } else if self.at_punct(Punct::Dot) {
                let marker = self.precede(left);
                self.bump();
                self.parse_name("a field name");
                left = self.complete(marker, SyntaxKind::FieldExpr);
            } else if self.at_punct(Punct::LBracket) {
                let marker = self.precede(left);
                self.bump();
                self.parse_nested_expression(Punct::RBracket);
                self.expect_punct(Punct::RBracket, "`]`");
                left = self.complete(marker, SyntaxKind::IndexExpr);
            } else {
                break;
            }
        }
        left
    }

    fn parse_argument_list(&mut self) {
        self.bump();
        let mut saw_named = false;
        if !self.at_punct(Punct::RParen) {
            loop {
                let argument = self.start();
                let named = self.current_kind() == TokenKind::Ident
                    && self.nth_kind(1) == TokenKind::Punct(Punct::Colon);
                if named {
                    saw_named = true;
                    self.parse_name("an argument name");
                    self.bump();
                } else if saw_named {
                    self.error_message(
                        "a positional argument cannot follow a named argument".to_owned(),
                    );
                }
                self.parse_nested_expression(Punct::RParen);
                self.complete(argument, SyntaxKind::Argument);
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
                if self.at_punct(Punct::RParen) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen, "`)`");
    }

    fn parse_nested_expression(&mut self, closing: Punct) -> CompletedMarker {
        if self.expression_depth >= super::MAX_RECURSIVE_DEPTH {
            self.depth_error("expression");
            let marker = self.start();
            while !self.at_eof()
                && !self.at(TokenKind::Term)
                && !self.at_punct(Punct::Comma)
                && !self.at_punct(closing)
            {
                self.bump();
            }
            return self.complete(marker, SyntaxKind::Error);
        }
        self.expression_depth += 1;
        let expression = self.parse_expression();
        self.expression_depth -= 1;
        expression
    }

    fn parse_match_expression(&mut self) -> CompletedMarker {
        if self.expression_depth >= super::MAX_RECURSIVE_DEPTH {
            self.depth_error("match expression");
            return self.recover_expression();
        }
        self.expression_depth += 1;
        let marker = self.start();
        self.bump();
        self.parse_expression();
        if !self.expect_punct(Punct::LBrace, "`{`") {
            let completed = self.complete(marker, SyntaxKind::MatchExpr);
            self.expression_depth -= 1;
            return completed;
        }
        self.skip_terms();
        let mut arms = 0_usize;
        while !self.at_punct(Punct::RBrace) && !self.at_eof() {
            let arm = self.start();
            self.parse_pattern();
            self.expect_punct(Punct::FatArrow, "`=>`");
            if self.at_punct(Punct::LBrace) {
                self.parse_block();
            } else {
                self.parse_expression();
            }
            self.complete(arm, SyntaxKind::MatchArm);
            arms += 1;
            self.require_term("after a match arm");
            self.skip_terms();
        }
        if arms == 0 {
            self.error_message("a match expression requires at least one arm".to_owned());
        }
        self.expect_punct(Punct::RBrace, "`}`");
        let completed = self.complete(marker, SyntaxKind::MatchExpr);
        self.expression_depth -= 1;
        completed
    }

    fn parse_pattern(&mut self) -> CompletedMarker {
        if self.pattern_depth >= super::MAX_RECURSIVE_DEPTH {
            self.depth_error("pattern");
            if self.at_punct(Punct::LParen) {
                return self.recover_balanced(Punct::LParen, Punct::RParen);
            }
            return self.recover_expression();
        }
        self.pattern_depth += 1;
        let marker = self.start();
        match self.current_kind() {
            TokenKind::Punct(Punct::Underscore)
            | TokenKind::Int
            | TokenKind::String
            | TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::None) => self.bump(),
            TokenKind::Ident => {
                self.parse_path();
                if self.eat_punct(Punct::LParen) {
                    if !self.at_punct(Punct::RParen) {
                        loop {
                            self.parse_pattern();
                            if !self.eat_punct(Punct::Comma) {
                                break;
                            }
                            if self.at_punct(Punct::RParen) {
                                break;
                            }
                        }
                    }
                    self.expect_punct(Punct::RParen, "`)`");
                }
            }
            _ => {
                self.error_expected(&["a pattern"]);
                if !is_expression_boundary(self.current_kind()) {
                    self.bump();
                }
            }
        }
        let completed = self.complete(marker, SyntaxKind::Pattern);
        self.pattern_depth -= 1;
        completed
    }

    fn recover_expression(&mut self) -> CompletedMarker {
        let marker = self.start();
        let start = self.position;
        while !is_expression_boundary(self.current_kind()) {
            self.bump();
        }
        if self.position == start && !self.at_eof() {
            self.bump();
        }
        self.complete(marker, SyntaxKind::Error)
    }
}

fn binary_operator(kind: TokenKind) -> Option<BinaryOperator> {
    let (left_binding_power, syntax_kind, non_associative) = match kind {
        TokenKind::Punct(Punct::OrOr) => (1, SyntaxKind::LogicalOrExpr, false),
        TokenKind::Punct(Punct::AndAnd) => (2, SyntaxKind::LogicalAndExpr, false),
        TokenKind::Punct(Punct::EqEq | Punct::BangEq) => (3, SyntaxKind::EqualityExpr, true),
        TokenKind::Punct(Punct::Less | Punct::LessEq | Punct::Greater | Punct::GreaterEq) => {
            (4, SyntaxKind::ComparisonExpr, true)
        }
        TokenKind::Punct(Punct::Shl | Punct::Shr) => (5, SyntaxKind::ShiftExpr, false),
        TokenKind::Punct(Punct::Plus | Punct::Minus) => (6, SyntaxKind::AdditiveExpr, false),
        TokenKind::Punct(Punct::Star | Punct::Slash | Punct::Percent) => {
            (7, SyntaxKind::MultiplicativeExpr, false)
        }
        _ => return None,
    };
    Some(BinaryOperator {
        left_binding_power,
        right_binding_power: left_binding_power + 1,
        kind: syntax_kind,
        non_associative,
    })
}

fn is_expression_boundary(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Term
            | TokenKind::Eof
            | TokenKind::Punct(
                Punct::Comma
                    | Punct::Colon
                    | Punct::FatArrow
                    | Punct::RParen
                    | Punct::RBracket
                    | Punct::RBrace
            )
            | TokenKind::Keyword(Keyword::As | Keyword::In)
    )
}
