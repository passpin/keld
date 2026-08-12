use super::{Parser, keyword};
use crate::{Keyword, Punct, SyntaxKind, TokenKind};

impl Parser<'_> {
    pub(super) fn is_item_start(&self) -> bool {
        let first = self.current_kind();
        let second = self.nth_kind(1);
        matches!(
            first,
            TokenKind::Keyword(
                Keyword::Struct | Keyword::Entity | Keyword::Enum | Keyword::Fn | Keyword::Extern
            )
        ) || (first == keyword(Keyword::Pub)
            && matches!(
                second,
                TokenKind::Keyword(
                    Keyword::Struct
                        | Keyword::Entity
                        | Keyword::Enum
                        | Keyword::Fn
                        | Keyword::Extern
                )
            ))
    }

    pub(super) fn parse_module_decl(&mut self) {
        let marker = self.start();
        self.eat_keyword(Keyword::Unsafe);
        self.expect_keyword(Keyword::Module, "`module`");
        self.parse_path();
        self.complete(marker, SyntaxKind::ModuleDecl);
    }

    pub(super) fn parse_use_decl(&mut self) {
        let marker = self.start();
        self.bump();
        self.parse_path();
        self.complete(marker, SyntaxKind::UseDecl);
    }

    pub(super) fn parse_item(&mut self) {
        let marker = self.start();
        self.eat_keyword(Keyword::Pub);
        let kind = match self.current_kind() {
            TokenKind::Keyword(Keyword::Struct) => {
                self.parse_record_header();
                self.parse_field_block();
                SyntaxKind::StructDecl
            }
            TokenKind::Keyword(Keyword::Entity) => {
                self.parse_record_header();
                self.parse_field_block();
                SyntaxKind::EntityDecl
            }
            TokenKind::Keyword(Keyword::Enum) => {
                self.parse_record_header();
                self.parse_variant_block();
                SyntaxKind::EnumDecl
            }
            TokenKind::Keyword(Keyword::Fn) => {
                self.parse_function_tail(false);
                SyntaxKind::FunctionDecl
            }
            TokenKind::Keyword(Keyword::Extern) => {
                self.parse_function_tail(true);
                SyntaxKind::ExternFunctionDecl
            }
            _ => {
                self.error_expected(&["a declaration keyword"]);
                self.recover_top_level();
                SyntaxKind::Error
            }
        };
        self.complete(marker, kind);
    }

    fn parse_record_header(&mut self) {
        self.bump();
        self.parse_name("a declaration name");
        if self.at_punct(Punct::LBracket) {
            self.parse_type_parameter_list();
        }
    }

    fn parse_type_parameter_list(&mut self) {
        let marker = self.start();
        self.bump();
        if !self.at_punct(Punct::RBracket) {
            loop {
                self.parse_name("a type parameter");
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
                if self.at_punct(Punct::RBracket) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RBracket, "`]`");
        self.complete(marker, SyntaxKind::TypeParameterList);
    }

    fn parse_field_block(&mut self) {
        let marker = self.start();
        if !self.expect_punct(Punct::LBrace, "`{`") {
            self.complete(marker, SyntaxKind::FieldBlock);
            return;
        }
        self.skip_terms();
        while !self.at_punct(Punct::RBrace) && !self.at_eof() {
            let field = self.start();
            self.parse_name("a field name");
            self.expect_punct(Punct::Colon, "`:`");
            self.parse_type();
            self.complete(field, SyntaxKind::FieldDecl);
            self.require_term("after a field");
            self.skip_terms();
        }
        self.expect_punct(Punct::RBrace, "`}`");
        self.complete(marker, SyntaxKind::FieldBlock);
    }

    fn parse_variant_block(&mut self) {
        let marker = self.start();
        if !self.expect_punct(Punct::LBrace, "`{`") {
            self.complete(marker, SyntaxKind::VariantBlock);
            return;
        }
        self.skip_terms();
        while !self.at_punct(Punct::RBrace) && !self.at_eof() {
            let variant = self.start();
            self.parse_name("a variant name");
            if self.eat_punct(Punct::LParen) {
                if !self.at_punct(Punct::RParen) {
                    loop {
                        self.parse_type();
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
            self.complete(variant, SyntaxKind::VariantDecl);
            self.require_term("after a variant");
            self.skip_terms();
        }
        self.expect_punct(Punct::RBrace, "`}`");
        self.complete(marker, SyntaxKind::VariantBlock);
    }

    fn parse_function_tail(&mut self, external: bool) {
        if external {
            self.expect_keyword(Keyword::Extern, "`extern`");
            self.expect(TokenKind::String, "an ABI string");
        }
        self.expect_keyword(Keyword::Fn, "`fn`");
        self.parse_name("a function name");
        if self.at_punct(Punct::LBracket) {
            self.parse_type_parameter_list();
        }
        self.parse_parameter_list();
        if self.at_punct(Punct::Arrow) {
            let clause = self.start();
            self.bump();
            self.parse_type();
            self.complete(clause, SyntaxKind::ReturnClause);
        }
        if self.at_keyword(Keyword::Raises) {
            self.parse_raises_clause();
        }
        if !external && self.at_keyword(Keyword::Retires) {
            self.parse_retires_clause();
        }
        if !external {
            self.parse_block();
        }
    }

    fn parse_parameter_list(&mut self) {
        let marker = self.start();
        if !self.expect_punct(Punct::LParen, "`(`") {
            self.recover_until(Punct::RParen);
        }
        if !self.at_punct(Punct::RParen) && !self.at_eof() {
            loop {
                let parameter = self.start();
                self.eat_keyword(Keyword::Take);
                self.parse_name("a parameter name");
                if !self.expect_punct(Punct::Colon, "`:`") {
                    self.recover_until(Punct::RParen);
                }
                if !self.at_punct(Punct::Comma) && !self.at_punct(Punct::RParen) {
                    self.parse_type();
                }
                self.complete(parameter, SyntaxKind::Parameter);
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
                if self.at_punct(Punct::RParen) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen, "`)`");
        self.complete(marker, SyntaxKind::ParameterList);
    }

    fn parse_raises_clause(&mut self) {
        let marker = self.start();
        self.bump();
        loop {
            self.parse_type();
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.complete(marker, SyntaxKind::RaisesClause);
    }

    fn parse_retires_clause(&mut self) {
        let marker = self.start();
        self.bump();
        loop {
            let target = self.start();
            if self.eat_keyword(Keyword::Any) {
                self.parse_type();
            } else {
                self.parse_name("a parameter name or `any Type`");
            }
            self.complete(target, SyntaxKind::RetirementTarget);
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.complete(marker, SyntaxKind::RetiresClause);
    }

    pub(super) fn parse_type(&mut self) {
        if self.type_depth >= super::MAX_RECURSIVE_DEPTH {
            self.depth_error("type");
            if self.at_punct(Punct::LBracket) {
                self.recover_balanced(Punct::LBracket, Punct::RBracket);
            } else {
                self.recover_until(Punct::RBracket);
            }
            return;
        }
        self.type_depth += 1;
        let marker = self.start();
        self.eat_keyword(Keyword::Link);
        self.parse_path();
        if self.at_punct(Punct::LBracket) {
            self.parse_type_argument_list();
        }
        self.eat_punct(Punct::Question);
        self.complete(marker, SyntaxKind::Type);
        self.type_depth -= 1;
    }

    fn parse_type_argument_list(&mut self) {
        let marker = self.start();
        self.bump();
        if !self.at_punct(Punct::RBracket) {
            loop {
                self.parse_type();
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
                if self.at_punct(Punct::RBracket) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RBracket, "`]`");
        self.complete(marker, SyntaxKind::TypeArgumentList);
    }
}
