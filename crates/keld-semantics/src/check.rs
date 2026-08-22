use crate::analyze::{Analyzer, direct_child};
mod list;
use crate::symbols::{FunctionSignature, RetirementSignature};
use crate::{
    BindingMutability, CompareOp, DefId, DefinitionKind, FunctionEffects, FunctionId, HirBinaryOp,
    HirBlock, HirExpr, HirExprKind, HirFunction, HirIf, HirLifecycle, HirLifecycleId, HirPlace,
    HirStmt, HirStmtKind, HirUnaryOp, HirWhen, HirWhile, LocalId, ParameterIndex, TypeId,
    TypeKind, TypeStore,
};
use keld_numeric::{
    IntBinaryOp, IntUnaryOp, ParsedIntLiteral, eval_binary, eval_unary, parse_int_literal,
};
use keld_source::{DiagnosticCode, Span};
use keld_syntax::{Keyword, Punct, SyntaxKind, SyntaxNode, TokenKind};
use std::collections::{BTreeMap, BTreeSet};

const UNKNOWN_NAME_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0103");
const ARGUMENT_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0104");
const FIELD_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0105");
const TYPE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0106");
const CONDITION_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0107");
const FEATURE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0004");
const RETURN_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0111");
const LOOP_CONTROL_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0112");
const CONSTANT_FAULT_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0120");
const INTEGER_RANGE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0121");

impl Analyzer<'_, '_> {
    pub(crate) fn check_function_bodies(&mut self) {
        let signatures = self.signatures.clone();
        for signature in &signatures {
            let function = BodyChecker::new(self, signature).check();
            self.functions.push(function);
        }
    }
}

#[derive(Clone, Default)]
struct Environment {
    locals: BTreeMap<String, (LocalId, TypeId)>,
    lifecycles: BTreeMap<String, HirLifecycleId>,
}

struct CheckedExpr {
    hir: HirExpr,
    constant_int: Option<i64>,
}

struct BodyChecker<'analyzer, 'source, 'syntax> {
    analyzer: &'analyzer mut Analyzer<'source, 'syntax>,
    signature: &'analyzer FunctionSignature<'syntax>,
    environment: Environment,
    next_local: u32,
    next_lifecycle: u32,
    local_types: Vec<TypeId>,
    local_mutability: Vec<BindingMutability>,
    suppressed_constant_faults: u16,
    loop_depth: usize,
}

impl<'analyzer, 'source, 'syntax> BodyChecker<'analyzer, 'source, 'syntax> {
    fn new(
        analyzer: &'analyzer mut Analyzer<'source, 'syntax>,
        signature: &'analyzer FunctionSignature<'syntax>,
    ) -> Self {
        let mut environment = Environment::default();
        let mut local_types = Vec::new();
        for (index, parameter) in signature.parameters.iter().enumerate() {
            let local = LocalId(u32::try_from(index).unwrap_or(u32::MAX));
            environment
                .locals
                .insert(parameter.name.clone(), (local, parameter.ty));
            local_types.push(parameter.ty);
        }
        Self {
            analyzer,
            signature,
            environment,
            next_local: u32::try_from(signature.parameters.len()).unwrap_or(u32::MAX),
            next_lifecycle: 0,
            local_types,
            local_mutability: vec![BindingMutability::Let; signature.parameters.len()],
            suppressed_constant_faults: 0,
            loop_depth: 0,
        }
    }

    fn check(mut self) -> HirFunction {
        let effects = self.resolve_effects();
        let body_node = direct_child(self.signature.node, SyntaxKind::Block);
        let mut environment = self.environment.clone();
        let mut body = body_node.map_or_else(
            || HirBlock {
                statements: Vec::new(),
                span: self.signature.node.span,
            },
            |node| self.check_block(node, &mut environment),
        );
        let definitely_returns = block_definitely_returns(&body);
        if self.signature.return_type != TypeStore::UNIT && !definitely_returns {
            let span = direct_child(self.signature.node, SyntaxKind::ReturnClause)
                .map_or(self.signature.node.span, |clause| clause.span);
            self.error(
                RETURN_DIAGNOSTIC,
                span,
                "non-Unit function can fall through without returning a value".to_owned(),
            );
        } else if self.signature.return_type == TypeStore::UNIT && !definitely_returns {
            body.statements.push(HirStmt {
                kind: HirStmtKind::Return(None),
                span: body.span,
            });
        }

        HirFunction {
            id: self.signature.id,
            name: self.signature.name.clone(),
            parameters: self
                .signature
                .parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| {
                    (
                        LocalId(u32::try_from(index).unwrap_or(u32::MAX)),
                        parameter.ty,
                    )
                })
                .collect(),
            parameter_modes: self
                .signature
                .parameters
                .iter()
                .map(|parameter| parameter.mode)
                .collect(),
            parameter_names: self
                .signature
                .parameters
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
            return_type: self.signature.return_type,
            effects,
            body,
            span: self.signature.node.span,
            local_types: self.local_types,
            local_mutability: self.local_mutability,
        }
    }

    fn resolve_effects(&mut self) -> FunctionEffects {
        let mut effects = FunctionEffects::default();
        for retirement in &self.signature.retirements {
            match retirement {
                RetirementSignature::Parameter(name, span) => {
                    if let Some((local, ty)) = self.environment.locals.get(name).copied() {
                        if matches!(self.analyzer.types.kind(ty), TypeKind::EntityRef(_)) {
                            effects.retires.push(local);
                            effects.retires_spans.push(*span);
                        } else {
                            self.error(
                                TYPE_DIAGNOSTIC,
                                *span,
                                format!("retirement target `{name}` is not an entity reference"),
                            );
                        }
                    } else {
                        self.error(
                            UNKNOWN_NAME_DIAGNOSTIC,
                            *span,
                            format!("unknown retirement parameter `{name}`"),
                        );
                    }
                }
                RetirementSignature::Any(definition, span) => {
                    effects.retires_any.push(*definition);
                    effects.retires_any_spans.push(*span);
                }
            }
        }
        effects
    }

    fn check_block(&mut self, node: &SyntaxNode, environment: &mut Environment) -> HirBlock {
        let mut statements = Vec::new();
        for statement in node.child_nodes().filter(|child| is_statement(child.kind)) {
            if let Some(checked) = self.check_statement(statement, environment) {
                statements.push(checked);
            }
        }
        HirBlock {
            statements,
            span: node.span,
        }
    }

    fn check_statement(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
    ) -> Option<HirStmt> {
        let kind = match node.kind {
            SyntaxKind::BindingStmt => self.check_binding(node, environment),
            SyntaxKind::AssignmentStmt => self.check_assignment(node, environment),
            SyntaxKind::ReturnStmt => Some(self.check_return(node, environment)),
            SyntaxKind::ExprStmt => expression_child(node).map(|expression| {
                let checked = self.check_expression(expression, None, environment);
                if checked.hir.ty != TypeStore::UNIT && checked.hir.ty != TypeStore::ERROR {
                    self.error(
                        TYPE_DIAGNOSTIC,
                        expression.span,
                        "expression statements must have type Unit".to_owned(),
                    );
                }
                HirStmtKind::Expr(checked.hir)
            }),
            SyntaxKind::IfStmt => Some(self.check_if(node, environment)),
            SyntaxKind::WhileStmt => Some(self.check_while(node, environment)),
            SyntaxKind::BreakStmt => {
                if self.loop_depth == 0 {
                    self.error(
                        LOOP_CONTROL_DIAGNOSTIC,
                        node.span,
                        "`break` is only valid inside a while loop".to_owned(),
                    );
                }
                Some(HirStmtKind::Break)
            }
            SyntaxKind::ContinueStmt => {
                if self.loop_depth == 0 {
                    self.error(
                        LOOP_CONTROL_DIAGNOSTIC,
                        node.span,
                        "`continue` is only valid inside a while loop".to_owned(),
                    );
                }
                Some(HirStmtKind::Continue)
            }
            SyntaxKind::WhenStmt => Some(self.check_when(node, environment)),
            SyntaxKind::LifecycleStmt => Some(self.check_lifecycle(node, environment)),
            SyntaxKind::KeepStmt => self.check_keep(node, environment),
            SyntaxKind::RetireStmt => expression_child(node).map(|expression| {
                let checked = self.check_expression(expression, None, environment);
                if !matches!(
                    self.analyzer.types.kind(checked.hir.ty),
                    TypeKind::EntityRef(_)
                ) && checked.hir.ty != TypeStore::ERROR
                {
                    self.error(
                        TYPE_DIAGNOSTIC,
                        expression.span,
                        "`retire` requires an entity reference".to_owned(),
                    );
                }
                HirStmtKind::Retire(checked.hir)
            }),
            _ => None,
        }?;
        Some(HirStmt {
            kind,
            span: node.span,
        })
    }

    fn check_binding(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
    ) -> Option<HirStmtKind> {
        let name_node = direct_child(node, SyntaxKind::Name)?;
        let name = self.analyzer.context.node_text(name_node)?.to_owned();
        let explicit = direct_child(node, SyntaxKind::Type)
            .map(|type_node| self.analyzer.resolve_type(type_node));
        let expression = expression_child(node);
        let is_var = self.analyzer.context.has_direct_keyword(node, Keyword::Var);
        let checked =
            expression.map(|expression| self.check_expression(expression, explicit, environment));
        let Some(ty) = explicit.or_else(|| checked.as_ref().map(|checked| checked.hir.ty)) else {
            self.error(
                TYPE_DIAGNOSTIC,
                node.span,
                "a binding without an initializer requires an explicit type".to_owned(),
            );
            return None;
        };
        if !is_var && checked.is_none() {
            self.error(
                TYPE_DIAGNOSTIC,
                node.span,
                "`let` requires an initializer".to_owned(),
            );
            return None;
        }
        if checked
            .as_ref()
            .is_some_and(|checked| self.analyzer.types.kind(checked.hir.ty) == &TypeKind::Error)
        {
            return None;
        }
        if environment.locals.contains_key(&name) {
            self.error(
                DiagnosticCode("KLD0102"),
                name_node.span,
                format!("duplicate local `{name}`"),
            );
        }
        let local = self.allocate_local(
            ty,
            if is_var {
                BindingMutability::Var
            } else {
                BindingMutability::Let
            },
        );
        environment.locals.insert(name, (local, ty));
        if is_var {
            Some(HirStmtKind::Var {
                local,
                initializer: checked.map(|checked| checked.hir),
            })
        } else {
            Some(HirStmtKind::Let {
                local,
                initializer: checked.expect("let initializer was checked").hir,
            })
        }
    }

    fn check_assignment(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
    ) -> Option<HirStmtKind> {
        let place_node = direct_child(node, SyntaxKind::Place)?;
        let (target, target_ty) = self.check_place(place_node, environment)?;
        let value_node = expression_child(node)?;
        let value = self.check_expression(value_node, Some(target_ty), environment);
        let operator = self.direct_punct(node);
        match operator {
            Some(Punct::Eq) => Some(HirStmtKind::Assign {
                target,
                value: value.hir,
            }),
            Some(punct) => {
                if target
                    .projections
                    .iter()
                    .any(|projection| matches!(projection, crate::HirProjection::Index(_)))
                {
                    self.error(
                        FEATURE_DIAGNOSTIC,
                        node.span,
                        "indexed compound assignment is not supported".to_owned(),
                    );
                    None
                } else {
                    int_binary_operator(punct).map(|op| HirStmtKind::CompoundAssign {
                        target,
                        op,
                        value: value.hir,
                    })
                }
            }
            None => None,
        }
    }

    fn check_return(&mut self, node: &SyntaxNode, environment: &mut Environment) -> HirStmtKind {
        let expression = expression_child(node);
        if self.signature.return_type == TypeStore::UNIT {
            if expression.is_some() {
                self.error(
                    TYPE_DIAGNOSTIC,
                    node.span,
                    "Unit function cannot return a value".to_owned(),
                );
            }
            return HirStmtKind::Return(None);
        }
        let Some(expression) = expression else {
            self.error(
                TYPE_DIAGNOSTIC,
                node.span,
                "non-Unit function must return a value".to_owned(),
            );
            return HirStmtKind::Return(None);
        };
        let checked =
            self.check_expression(expression, Some(self.signature.return_type), environment);
        HirStmtKind::Return(Some(checked.hir))
    }

    fn check_while(&mut self, node: &SyntaxNode, environment: &Environment) -> HirStmtKind {
        let condition = if let Some(expression) = expression_child(node) {
            let checked = self.check_expression(expression, None, &mut environment.clone());
            if checked.hir.ty != TypeStore::BOOL && checked.hir.ty != TypeStore::ERROR {
                self.error(
                    CONDITION_DIAGNOSTIC,
                    expression.span,
                    "while condition must have type Bool".to_owned(),
                );
            }
            checked.hir
        } else {
            Self::error_expression(node.span)
        };

        let body = if let Some(block) = direct_child(node, SyntaxKind::Block) {
            self.loop_depth = self.loop_depth.saturating_add(1);
            let body = self.check_block(block, &mut environment.clone());
            self.loop_depth = self.loop_depth.saturating_sub(1);
            body
        } else {
            HirBlock {
                statements: Vec::new(),
                span: node.span,
            }
        };

        HirStmtKind::While(HirWhile { condition, body })
    }

    fn check_if(&mut self, node: &SyntaxNode, environment: &Environment) -> HirStmtKind {
        let expressions = node
            .child_nodes()
            .filter(|child| is_expression(child.kind))
            .collect::<Vec<_>>();
        let blocks = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Block)
            .collect::<Vec<_>>();
        HirStmtKind::If(self.check_if_chain(node, &expressions, &blocks, 0, environment))
    }

    fn check_if_chain(
        &mut self,
        node: &SyntaxNode,
        expressions: &[&SyntaxNode],
        blocks: &[&SyntaxNode],
        index: usize,
        environment: &Environment,
    ) -> HirIf {
        let condition = if let Some(expression) = expressions.get(index) {
            let checked = self.check_expression(expression, None, &mut environment.clone());
            if checked.hir.ty != TypeStore::BOOL && checked.hir.ty != TypeStore::ERROR {
                self.error(
                    CONDITION_DIAGNOSTIC,
                    expression.span,
                    "if condition must have type Bool".to_owned(),
                );
            }
            checked.hir
        } else {
            Self::error_expression(node.span)
        };
        let then_block = blocks.get(index).map_or_else(
            || HirBlock {
                statements: Vec::new(),
                span: node.span,
            },
            |block| self.check_block(block, &mut environment.clone()),
        );
        let else_block = if index + 1 < expressions.len() {
            let nested = self.check_if_chain(node, expressions, blocks, index + 1, environment);
            Some(HirBlock {
                statements: vec![HirStmt {
                    kind: HirStmtKind::If(nested),
                    span: node.span,
                }],
                span: node.span,
            })
        } else {
            blocks
                .get(index + 1)
                .map(|block| self.check_block(block, &mut environment.clone()))
        };
        HirIf {
            condition,
            then_block,
            else_block,
        }
    }

    fn check_when(&mut self, node: &SyntaxNode, environment: &Environment) -> HirStmtKind {
        let expression = expression_child(node);
        let link = if let Some(expression) = expression {
            self.check_expression(expression, None, &mut environment.clone())
                .hir
        } else {
            Self::error_expression(node.span)
        };
        let entity = if let TypeKind::Link { entity, .. } = *self.analyzer.types.kind(link.ty) {
            entity
        } else {
            self.error(
                TYPE_DIAGNOSTIC,
                link.span,
                "`when` requires a link or optional value".to_owned(),
            );
            DefId(u32::MAX)
        };
        let binding_node = direct_child(node, SyntaxKind::Name);
        let binding_name = binding_node
            .and_then(|name| self.analyzer.context.node_text(name))
            .unwrap_or("<error>")
            .to_owned();
        let binding_type = self.analyzer.types.intern(TypeKind::EntityRef(entity));
        let binding = self.allocate_local(binding_type, BindingMutability::Let);
        let blocks = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Block)
            .collect::<Vec<_>>();
        let mut live_environment = environment.clone();
        live_environment
            .locals
            .insert(binding_name, (binding, binding_type));
        let live = blocks.first().map_or_else(
            || HirBlock {
                statements: Vec::new(),
                span: node.span,
            },
            |block| self.check_block(block, &mut live_environment),
        );
        let absent = blocks
            .get(1)
            .map(|block| self.check_block(block, &mut environment.clone()));
        HirStmtKind::When(HirWhen {
            link,
            binding,
            live,
            absent,
        })
    }

    fn check_lifecycle(&mut self, node: &SyntaxNode, environment: &Environment) -> HirStmtKind {
        let name = direct_child(node, SyntaxKind::Name)
            .and_then(|name| self.analyzer.context.node_text(name))
            .unwrap_or("<error>")
            .to_owned();
        let id = HirLifecycleId(self.next_lifecycle);
        self.next_lifecycle = self.next_lifecycle.saturating_add(1);
        let mut nested = environment.clone();
        nested.lifecycles.insert(name.clone(), id);
        let body = direct_child(node, SyntaxKind::Block).map_or_else(
            || HirBlock {
                statements: Vec::new(),
                span: node.span,
            },
            |block| self.check_block(block, &mut nested),
        );
        HirStmtKind::Lifecycle(HirLifecycle { id, name, body })
    }

    fn check_keep(&mut self, node: &SyntaxNode, environment: &Environment) -> Option<HirStmtKind> {
        let expression = expression_child(node)?;
        let entity = self.check_expression(expression, None, &mut environment.clone());
        if !matches!(
            self.analyzer.types.kind(entity.hir.ty),
            TypeKind::EntityRef(_)
        ) && entity.hir.ty != TypeStore::ERROR
        {
            self.error(
                TYPE_DIAGNOSTIC,
                expression.span,
                "`keep` requires an entity reference".to_owned(),
            );
        }
        let name = direct_child(node, SyntaxKind::Name)
            .and_then(|name| self.analyzer.context.node_text(name))?;
        let Some(lifecycle) = environment.lifecycles.get(name).copied() else {
            self.error(
                UNKNOWN_NAME_DIAGNOSTIC,
                node.span,
                format!("unknown active lifecycle `{name}`"),
            );
            return None;
        };
        Some(HirStmtKind::Keep {
            entity: entity.hir,
            lifecycle,
        })
    }

    fn check_expression(
        &mut self,
        node: &SyntaxNode,
        expected: Option<TypeId>,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let checked = match node.kind {
            SyntaxKind::LiteralExpr => self.check_literal(node, expected),
            SyntaxKind::NameExpr => self.check_name(node, environment),
            SyntaxKind::ParenthesizedExpr => {
                if let Some(inner) = expression_child(node) {
                    self.check_expression(inner, expected, environment)
                } else {
                    CheckedExpr {
                        hir: Self::error_expression(node.span),
                        constant_int: None,
                    }
                }
            }
            SyntaxKind::UnaryExpr => self.check_unary(node, environment),
            SyntaxKind::TakeExpr => self.check_take(node, environment),
            SyntaxKind::CallExpr => self.check_call(node, expected, environment),
            SyntaxKind::FieldExpr => self.check_field(node, environment),
            SyntaxKind::IndexExpr => self.check_list_index(node, environment),
            SyntaxKind::LogicalOrExpr
            | SyntaxKind::LogicalAndExpr
            | SyntaxKind::EqualityExpr
            | SyntaxKind::ComparisonExpr
            | SyntaxKind::ShiftExpr
            | SyntaxKind::AdditiveExpr
            | SyntaxKind::MultiplicativeExpr => self.check_binary(node, environment),
            _ => CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            },
        };
        if let Some(expected) = expected {
            self.coerce(checked, expected)
        } else {
            checked
        }
    }

    fn check_literal(&mut self, node: &SyntaxNode, expected: Option<TypeId>) -> CheckedExpr {
        match self.analyzer.context.direct_token_kind(node) {
            Some(TokenKind::Int) => {
                let text = self.analyzer.context.node_text(node).unwrap_or("");
                match parse_int_literal(text) {
                    ParsedIntLiteral::Value(value) => CheckedExpr {
                        hir: HirExpr {
                            ty: TypeStore::INT,
                            span: node.span,
                            kind: HirExprKind::Int(value),
                        },
                        constant_int: Some(value),
                    },
                    ParsedIntLiteral::IntMinMagnitude | ParsedIntLiteral::OutOfRange => {
                        self.error(
                            INTEGER_RANGE_DIAGNOSTIC,
                            node.span,
                            "integer literal is outside the Int range".to_owned(),
                        );
                        CheckedExpr {
                            hir: Self::error_expression(node.span),
                            constant_int: None,
                        }
                    }
                }
            }
            Some(TokenKind::Keyword(Keyword::True | Keyword::False)) => {
                let value = self.analyzer.context.node_text(node) == Some("true");
                CheckedExpr {
                    hir: HirExpr {
                        ty: TypeStore::BOOL,
                        span: node.span,
                        kind: HirExprKind::Bool(value),
                    },
                    constant_int: None,
                }
            }
            Some(TokenKind::Keyword(Keyword::None)) => {
                let ty = expected.filter(|ty| {
                    matches!(
                        self.analyzer.types.kind(*ty),
                        TypeKind::Link { optional: true, .. }
                    )
                });
                if ty.is_none() {
                    self.error(
                        TYPE_DIAGNOSTIC,
                        node.span,
                        "`none` requires an optional link context".to_owned(),
                    );
                }
                CheckedExpr {
                    hir: HirExpr {
                        ty: ty.unwrap_or(TypeStore::ERROR),
                        span: node.span,
                        kind: HirExprKind::None,
                    },
                    constant_int: None,
                }
            }
            Some(TokenKind::String) => {
                let text = self.analyzer.context.node_text(node).unwrap_or("");
                let value = decode_string_literal(text).unwrap_or_default();
                let ty = self.analyzer.types.intern(TypeKind::Text);
                CheckedExpr {
                    hir: HirExpr {
                        ty,
                        span: node.span,
                        kind: HirExprKind::TextLiteral(value),
                    },
                    constant_int: None,
                }
            }
            _ => CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            },
        }
    }

    fn check_take(&mut self, node: &SyntaxNode, environment: &mut Environment) -> CheckedExpr {
        let Some(place_node) = direct_child(node, SyntaxKind::Place) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let names = place_node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Name)
            .collect::<Vec<_>>();
        if place_node.child_nodes().count() != 1 {
            self.error(
                DiagnosticCode("KLD2004"),
                node.span,
                "`take` does not support projected places".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        }
        if names.len() != 1 {
            self.error(
                DiagnosticCode("KLD2009"),
                node.span,
                "`take` requires a named single-home local".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        }
        let name = self
            .analyzer
            .context
            .node_text(names[0])
            .unwrap_or("<error>");
        let Some((base, ty)) = environment.locals.get(name).copied() else {
            self.error(
                UNKNOWN_NAME_DIAGNOSTIC,
                names[0].span,
                format!("unknown value `{name}`"),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        if self.analyzer.types.storage_class(ty) != crate::StorageClass::SingleHome {
            self.error(
                DiagnosticCode("KLD2009"),
                node.span,
                "`take` requires a single-home value".to_owned(),
            );
        }
        CheckedExpr {
            hir: HirExpr {
                ty,
                span: node.span,
                kind: HirExprKind::Take(HirPlace {
                    base,
                    projections: Vec::new(),
                    span: place_node.span,
                }),
            },
            constant_int: None,
        }
    }

    fn check_name(&mut self, node: &SyntaxNode, environment: &Environment) -> CheckedExpr {
        let name = direct_child(node, SyntaxKind::Name)
            .and_then(|name| self.analyzer.context.node_text(name))
            .unwrap_or("<error>");
        if let Some((local, ty)) = environment.locals.get(name).copied() {
            return CheckedExpr {
                hir: HirExpr {
                    ty,
                    span: node.span,
                    kind: HirExprKind::Local(local),
                },
                constant_int: None,
            };
        }
        self.error(
            UNKNOWN_NAME_DIAGNOSTIC,
            node.span,
            format!("unknown value `{name}`"),
        );
        CheckedExpr {
            hir: Self::error_expression(node.span),
            constant_int: None,
        }
    }

    fn check_unary(&mut self, node: &SyntaxNode, environment: &mut Environment) -> CheckedExpr {
        let operand_node = expression_child(node);
        if self.analyzer.context.has_direct_punct(node, Punct::Minus)
            && operand_node.is_some_and(|operand| self.is_min_magnitude(operand))
        {
            return CheckedExpr {
                hir: HirExpr {
                    ty: TypeStore::INT,
                    span: node.span,
                    kind: HirExprKind::Int(i64::MIN),
                },
                constant_int: Some(i64::MIN),
            };
        }
        let Some(operand_node) = operand_node else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        if self.analyzer.context.has_direct_punct(node, Punct::Bang) {
            let operand = self.check_expression(operand_node, Some(TypeStore::BOOL), environment);
            return CheckedExpr {
                hir: HirExpr {
                    ty: TypeStore::BOOL,
                    span: node.span,
                    kind: HirExprKind::Unary {
                        op: HirUnaryOp::Not,
                        value: Box::new(operand.hir),
                    },
                },
                constant_int: None,
            };
        }
        let operand = self.check_expression(operand_node, Some(TypeStore::INT), environment);
        let constant_int = operand.constant_int.and_then(|value| {
            eval_unary(IntUnaryOp::Neg, value)
                .map_err(|fault| self.constant_fault(node.span, fault))
                .ok()
        });
        CheckedExpr {
            hir: HirExpr {
                ty: TypeStore::INT,
                span: node.span,
                kind: HirExprKind::Unary {
                    op: HirUnaryOp::Int(IntUnaryOp::Neg),
                    value: Box::new(operand.hir),
                },
            },
            constant_int,
        }
    }

    fn check_binary(&mut self, node: &SyntaxNode, environment: &mut Environment) -> CheckedExpr {
        let operands = node
            .child_nodes()
            .filter(|child| is_expression(child.kind))
            .collect::<Vec<_>>();
        if operands.len() < 2 {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        }
        let punct = self.direct_punct(node);
        if matches!(punct, Some(Punct::AndAnd | Punct::OrOr)) {
            return self.check_boolean_binary(node, operands[0], operands[1], punct, environment);
        }
        if matches!(punct, Some(Punct::EqEq | Punct::BangEq)) {
            return self.check_equality(node, operands[0], operands[1], punct, environment);
        }
        if matches!(
            punct,
            Some(Punct::Less | Punct::LessEq | Punct::Greater | Punct::GreaterEq)
        ) {
            let lhs = self.check_expression(operands[0], Some(TypeStore::INT), environment);
            let rhs = self.check_expression(operands[1], Some(TypeStore::INT), environment);
            let op = comparison_operator(punct.unwrap_or(Punct::Less));
            return CheckedExpr {
                hir: HirExpr {
                    ty: TypeStore::BOOL,
                    span: node.span,
                    kind: HirExprKind::Binary {
                        op: HirBinaryOp::Compare(op),
                        lhs: Box::new(lhs.hir),
                        rhs: Box::new(rhs.hir),
                    },
                },
                constant_int: None,
            };
        }

        if punct == Some(Punct::Plus) {
            let lhs = self.check_expression(operands[0], None, environment);
            if matches!(self.analyzer.types.kind(lhs.hir.ty), TypeKind::Text) {
                let rhs = self.check_expression(operands[1], Some(lhs.hir.ty), environment);
                return CheckedExpr {
                    hir: HirExpr {
                        ty: lhs.hir.ty,
                        span: node.span,
                        kind: HirExprKind::Binary {
                            op: HirBinaryOp::TextConcat,
                            lhs: Box::new(lhs.hir),
                            rhs: Box::new(rhs.hir),
                        },
                    },
                    constant_int: None,
                };
            }
        }

        let lhs = self.check_expression(operands[0], Some(TypeStore::INT), environment);
        let rhs = self.check_expression(operands[1], Some(TypeStore::INT), environment);
        let op = punct
            .and_then(int_binary_operator)
            .unwrap_or(IntBinaryOp::Add);
        let constant_int = lhs
            .constant_int
            .zip(rhs.constant_int)
            .and_then(|(lhs, rhs)| {
                eval_binary(op, lhs, rhs)
                    .map_err(|fault| self.constant_fault(node.span, fault))
                    .ok()
            });
        CheckedExpr {
            hir: HirExpr {
                ty: TypeStore::INT,
                span: node.span,
                kind: HirExprKind::Binary {
                    op: HirBinaryOp::Int(op),
                    lhs: Box::new(lhs.hir),
                    rhs: Box::new(rhs.hir),
                },
            },
            constant_int,
        }
    }

    fn check_boolean_binary(
        &mut self,
        node: &SyntaxNode,
        lhs_node: &SyntaxNode,
        rhs_node: &SyntaxNode,
        punct: Option<Punct>,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let lhs = self.check_expression(lhs_node, Some(TypeStore::BOOL), environment);
        let short_circuits = matches!(
            (&lhs.hir.kind, punct),
            (HirExprKind::Bool(false), Some(Punct::AndAnd))
                | (HirExprKind::Bool(true), Some(Punct::OrOr))
        );
        if short_circuits {
            self.suppressed_constant_faults = self.suppressed_constant_faults.saturating_add(1);
        }
        let rhs = self.check_expression(rhs_node, Some(TypeStore::BOOL), environment);
        if short_circuits {
            self.suppressed_constant_faults = self.suppressed_constant_faults.saturating_sub(1);
        }
        let op = if punct == Some(Punct::AndAnd) {
            HirBinaryOp::And
        } else {
            HirBinaryOp::Or
        };
        CheckedExpr {
            hir: HirExpr {
                ty: TypeStore::BOOL,
                span: node.span,
                kind: HirExprKind::Binary {
                    op,
                    lhs: Box::new(lhs.hir),
                    rhs: Box::new(rhs.hir),
                },
            },
            constant_int: None,
        }
    }

    fn check_equality(
        &mut self,
        node: &SyntaxNode,
        lhs_node: &SyntaxNode,
        rhs_node: &SyntaxNode,
        punct: Option<Punct>,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let lhs = self.check_expression(lhs_node, None, environment);
        let rhs = self.check_expression(rhs_node, Some(lhs.hir.ty), environment);
        match self.analyzer.types.kind(lhs.hir.ty) {
            TypeKind::Int
            | TypeKind::Bool
            | TypeKind::Text
            | TypeKind::EntityRef(_)
            | TypeKind::Error => {}
            TypeKind::Link { .. } => self.error(
                FEATURE_DIAGNOSTIC,
                node.span,
                "link equality is not supported; resolve the link with `when`".to_owned(),
            ),
            _ => self.error(
                TYPE_DIAGNOSTIC,
                node.span,
                "equality requires Int, Bool, Text, or direct entity identity operands".to_owned(),
            ),
        }
        let op = comparison_operator(punct.unwrap_or(Punct::EqEq));
        CheckedExpr {
            hir: HirExpr {
                ty: TypeStore::BOOL,
                span: node.span,
                kind: HirExprKind::Binary {
                    op: HirBinaryOp::Compare(op),
                    lhs: Box::new(lhs.hir),
                    rhs: Box::new(rhs.hir),
                },
            },
            constant_int: None,
        }
    }

    fn check_call(
        &mut self,
        node: &SyntaxNode,
        expected: Option<TypeId>,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some(callee) = node.child_nodes().find(|child| is_expression(child.kind)) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        if let Some(method) = method_name(callee, &self.analyzer.context) {
            return match method {
                "copy" => self.check_copy_call(node, callee, environment),
                "push" => self.check_list_push_call(node, callee, environment),
                "remove" => self.check_list_remove_call(node, callee, environment),
                "get" => self.check_list_get_call(node, callee, environment),
                "try_remove" => self.check_list_try_remove_call(node, callee, environment),
                "clear" => self.check_list_clear_call(node, callee, environment),
                "reserve" => self.check_list_reserve_call(node, callee, environment),
                "try_reserve" => self.check_list_try_reserve_call(node, callee, environment),
                _ => {
                    self.error(
                        UNKNOWN_NAME_DIAGNOSTIC,
                        callee.span,
                        format!("unknown method `{method}`"),
                    );
                    CheckedExpr {
                        hir: Self::error_expression(node.span),
                        constant_int: None,
                    }
                }
            };
        }
        let Some(name) = expression_name(callee, &self.analyzer.context).map(str::to_owned) else {
            self.error(
                FEATURE_DIAGNOSTIC,
                callee.span,
                "only named functions and constructors can be called".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        if name == "List" {
            return self.check_list_constructor(node, expected);
        }
        let arguments = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Argument)
            .collect::<Vec<_>>();
        if let Some(function) = self.analyzer.function_names.get(&name).copied() {
            return self.check_function_call(node, function, &arguments, environment);
        }
        if let Some(definition) = self.analyzer.definition_names.get(&name).copied() {
            return self.check_constructor(node, definition, &arguments, expected, environment);
        }
        self.error(
            UNKNOWN_NAME_DIAGNOSTIC,
            callee.span,
            format!("unknown function or constructor `{name}`"),
        );
        CheckedExpr {
            hir: Self::error_expression(node.span),
            constant_int: None,
        }
    }

    fn check_list_constructor(
        &mut self,
        node: &SyntaxNode,
        expected: Option<TypeId>,
    ) -> CheckedExpr {
        if node
            .child_nodes()
            .any(|child| child.kind == SyntaxKind::Argument)
        {
            self.error(
                ARGUMENT_DIAGNOSTIC,
                node.span,
                "`List()` does not accept arguments".to_owned(),
            );
        }
        let Some(ty) = expected else {
            self.error(
                TYPE_DIAGNOSTIC,
                node.span,
                "`List()` requires an expected `List[T]` type".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        if !matches!(self.analyzer.types.kind(ty), TypeKind::List(_)) {
            self.error(
                TYPE_DIAGNOSTIC,
                node.span,
                "`List()` requires an expected `List[T]` type".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        }
        CheckedExpr {
            hir: HirExpr {
                ty,
                span: node.span,
                kind: HirExprKind::ListNew,
            },
            constant_int: None,
        }
    }

    fn check_copy_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some(base_node) = callee.child_nodes().find(|child| is_expression(child.kind)) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        if node
            .child_nodes()
            .any(|child| child.kind == SyntaxKind::Argument)
        {
            self.error(
                ARGUMENT_DIAGNOSTIC,
                node.span,
                "`.copy()` does not accept arguments".to_owned(),
            );
        }
        let base = self.check_expression(base_node, None, environment);
        if !self.analyzer.types.is_structurally_duplicable(base.hir.ty) {
            self.error(
                DiagnosticCode("KLD2006"),
                base.hir.span,
                "this value is not structurally duplicable".to_owned(),
            );
        }
        CheckedExpr {
            hir: HirExpr {
                ty: base.hir.ty,
                span: node.span,
                kind: HirExprKind::Copy(Box::new(base.hir)),
            },
            constant_int: None,
        }
    }

    fn check_list_push_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some(base_node) = callee.child_nodes().find(|child| is_expression(child.kind)) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let base = self.check_expression(base_node, None, environment);
        let Some(element) = (match self.analyzer.types.kind(base.hir.ty) {
            TypeKind::List(element) => Some(*element),
            _ => None,
        }) else {
            self.error(
                TYPE_DIAGNOSTIC,
                base.hir.span,
                "`.push()` requires a List receiver".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let arguments = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Argument)
            .collect::<Vec<_>>();
        if arguments.len() != 1 {
            self.error(
                ARGUMENT_DIAGNOSTIC,
                node.span,
                "`.push()` requires exactly one argument".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        }
        let value = expression_child(arguments[0])
            .map(|value| self.check_expression(value, Some(element), environment))
            .unwrap_or(CheckedExpr {
                hir: Self::error_expression(arguments[0].span),
                constant_int: None,
            });
        CheckedExpr {
            hir: HirExpr {
                ty: TypeStore::UNIT,
                span: node.span,
                kind: HirExprKind::ListPush {
                    list: Box::new(base.hir),
                    value: Box::new(value.hir),
                },
            },
            constant_int: None,
        }
    }

    fn check_list_remove_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some(base_node) = callee.child_nodes().find(|child| is_expression(child.kind)) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let base = self.check_expression(base_node, None, environment);
        let Some(element) = (match self.analyzer.types.kind(base.hir.ty) {
            TypeKind::List(element) => Some(*element),
            _ => None,
        }) else {
            self.error(
                TYPE_DIAGNOSTIC,
                base.hir.span,
                "`.remove()` requires a List receiver".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let arguments = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Argument)
            .collect::<Vec<_>>();
        if arguments.len() != 1 {
            self.error(
                ARGUMENT_DIAGNOSTIC,
                node.span,
                "`.remove()` requires exactly one index argument".to_owned(),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        }
        let index = expression_child(arguments[0])
            .map(|index| self.check_expression(index, Some(TypeStore::INT), environment))
            .unwrap_or(CheckedExpr {
                hir: Self::error_expression(arguments[0].span),
                constant_int: None,
            });
        CheckedExpr {
            hir: HirExpr {
                ty: element,
                span: node.span,
                kind: HirExprKind::ListRemove {
                    list: Box::new(base.hir),
                    index: Box::new(index.hir),
                },
            },
            constant_int: None,
        }
    }

    fn check_function_call(
        &mut self,
        node: &SyntaxNode,
        function: FunctionId,
        arguments: &[&SyntaxNode],
        environment: &mut Environment,
    ) -> CheckedExpr {
        let signature = self.analyzer.signatures[function.0 as usize].clone();
        let mut used = BTreeSet::new();
        let mut next_positional = 0_usize;
        let mut checked_arguments = Vec::new();
        for argument in arguments {
            let named = direct_child(argument, SyntaxKind::Name)
                .and_then(|name| self.analyzer.context.node_text(name));
            let index = named.map_or_else(
                || {
                    let index = next_positional;
                    next_positional += 1;
                    index
                },
                |name| {
                    signature
                        .parameters
                        .iter()
                        .position(|parameter| parameter.name == name)
                        .unwrap_or(usize::MAX)
                },
            );
            let Some(parameter) = signature.parameters.get(index) else {
                self.error(
                    ARGUMENT_DIAGNOSTIC,
                    argument.span,
                    "unknown or excess function argument".to_owned(),
                );
                continue;
            };
            if !used.insert(index) {
                self.error(
                    ARGUMENT_DIAGNOSTIC,
                    argument.span,
                    format!("duplicate argument `{}`", parameter.name),
                );
                continue;
            }
            if let Some(expression) = expression_child(argument) {
                let checked = self.check_expression(expression, Some(parameter.ty), environment);
                checked_arguments.push((
                    ParameterIndex(u32::try_from(index).unwrap_or(u32::MAX)),
                    checked.hir,
                ));
            }
        }
        if used.len() != signature.parameters.len() {
            self.error(
                ARGUMENT_DIAGNOSTIC,
                node.span,
                "call does not provide every parameter exactly once".to_owned(),
            );
        }
        CheckedExpr {
            hir: HirExpr {
                ty: signature.return_type,
                span: node.span,
                kind: HirExprKind::Call {
                    function,
                    arguments: checked_arguments,
                },
            },
            constant_int: None,
        }
    }

    fn check_constructor(
        &mut self,
        node: &SyntaxNode,
        definition: DefId,
        arguments: &[&SyntaxNode],
        _expected: Option<TypeId>,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let definition_data = self.analyzer.definitions[definition.0 as usize].clone();
        let mut used = BTreeSet::new();
        let mut next_positional = 0_usize;
        let mut fields = Vec::new();
        for argument in arguments {
            let named = direct_child(argument, SyntaxKind::Name)
                .and_then(|name| self.analyzer.context.node_text(name));
            let index = named.map_or_else(
                || {
                    let index = next_positional;
                    next_positional += 1;
                    index
                },
                |name| {
                    definition_data
                        .fields
                        .iter()
                        .position(|field| field.name == name)
                        .unwrap_or(usize::MAX)
                },
            );
            let Some(field) = definition_data.fields.get(index) else {
                self.error(
                    ARGUMENT_DIAGNOSTIC,
                    argument.span,
                    "unknown or excess constructor field".to_owned(),
                );
                continue;
            };
            if !used.insert(index) {
                self.error(
                    ARGUMENT_DIAGNOSTIC,
                    argument.span,
                    format!("duplicate field initializer `{}`", field.name),
                );
                continue;
            }
            if let Some(expression) = expression_child(argument) {
                let checked = self.check_expression(expression, Some(field.ty), environment);
                fields.push((field.id, checked.hir));
            }
        }
        if used.len() != definition_data.fields.len() {
            self.error(
                ARGUMENT_DIAGNOSTIC,
                node.span,
                "constructor does not initialize every field exactly once".to_owned(),
            );
        }
        let (ty, kind) = match definition_data.kind {
            DefinitionKind::Struct => (
                self.analyzer.types.intern(TypeKind::Struct(definition)),
                HirExprKind::ConstructStruct { definition, fields },
            ),
            DefinitionKind::Entity => (
                self.analyzer.types.intern(TypeKind::EntityRef(definition)),
                HirExprKind::ConstructEntity { definition, fields },
            ),
        };
        CheckedExpr {
            hir: HirExpr {
                ty,
                span: node.span,
                kind,
            },
            constant_int: None,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn check_field(&mut self, node: &SyntaxNode, environment: &mut Environment) -> CheckedExpr {
        let children = node.child_nodes().collect::<Vec<_>>();
        let Some(base_node) = children
            .iter()
            .copied()
            .find(|child| is_expression(child.kind))
        else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let field_name = children
            .iter()
            .copied()
            .find(|child| child.kind == SyntaxKind::Name)
            .and_then(|name| self.analyzer.context.node_text(name))
            .unwrap_or("<error>")
            .to_owned();
        if field_name == "byte_length" || field_name == "is_empty" {
            let base = self.check_expression(base_node, None, environment);
            if !matches!(self.analyzer.types.kind(base.hir.ty), TypeKind::Text) {
                self.error(
                    TYPE_DIAGNOSTIC,
                    base.hir.span,
                    "Text property requires a Text receiver".to_owned(),
                );
                return CheckedExpr {
                    hir: Self::error_expression(node.span),
                    constant_int: None,
                };
            }
            return CheckedExpr {
                hir: HirExpr {
                    ty: if field_name == "byte_length" {
                        TypeStore::INT
                    } else {
                        TypeStore::BOOL
                    },
                    span: node.span,
                    kind: if field_name == "byte_length" {
                        HirExprKind::TextByteLength(Box::new(base.hir))
                    } else {
                        HirExprKind::TextIsEmpty(Box::new(base.hir))
                    },
                },
                constant_int: None,
            };
        }
        if expression_name(base_node, &self.analyzer.context) == Some("Int") {
            let value = match field_name.as_str() {
                "MIN" => Some(i64::MIN),
                "MAX" => Some(i64::MAX),
                _ => None,
            };
            if let Some(value) = value {
                return CheckedExpr {
                    hir: HirExpr {
                        ty: TypeStore::INT,
                        span: node.span,
                        kind: HirExprKind::Int(value),
                    },
                    constant_int: Some(value),
                };
            }
        }
        let base = self.check_expression(base_node, None, environment);
        if matches!(self.analyzer.types.kind(base.hir.ty), TypeKind::List(_))
            && field_name == "length"
        {
            return CheckedExpr {
                hir: HirExpr {
                    ty: TypeStore::INT,
                    span: node.span,
                    kind: HirExprKind::ListLength(Box::new(base.hir)),
                },
                constant_int: None,
            };
        }
        let definition = match *self.analyzer.types.kind(base.hir.ty) {
            TypeKind::Struct(definition) | TypeKind::EntityRef(definition) => definition,
            TypeKind::Link { entity, .. } => entity,
            _ => {
                self.error(
                    FIELD_DIAGNOSTIC,
                    node.span,
                    "field access requires a struct, entity, or link".to_owned(),
                );
                return CheckedExpr {
                    hir: Self::error_expression(node.span),
                    constant_int: None,
                };
            }
        };
        let Some(field) = self.analyzer.definitions[definition.0 as usize]
            .fields
            .iter()
            .find(|field| field.name == field_name)
            .cloned()
        else {
            self.error(
                FIELD_DIAGNOSTIC,
                node.span,
                format!("unknown field `{field_name}`"),
            );
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let kind = if matches!(self.analyzer.types.kind(base.hir.ty), TypeKind::Link { .. }) {
            HirExprKind::UncheckedLinkField {
                link: Box::new(base.hir),
                field: field.id,
            }
        } else {
            HirExprKind::Field {
                base: Box::new(base.hir),
                field: field.id,
            }
        };
        CheckedExpr {
            hir: HirExpr {
                ty: field.ty,
                span: node.span,
                kind,
            },
            constant_int: None,
        }
    }

    fn coerce(&mut self, checked: CheckedExpr, expected: TypeId) -> CheckedExpr {
        if checked.hir.ty == expected
            || checked.hir.ty == TypeStore::ERROR
            || expected == TypeStore::ERROR
        {
            return checked;
        }
        if let (TypeKind::EntityRef(actual), TypeKind::Link { entity, .. }) = (
            self.analyzer.types.kind(checked.hir.ty),
            self.analyzer.types.kind(expected),
        ) && actual == entity
        {
            return CheckedExpr {
                hir: HirExpr {
                    ty: expected,
                    span: checked.hir.span,
                    kind: HirExprKind::EntityToLink(Box::new(checked.hir)),
                },
                constant_int: None,
            };
        }
        self.error(
            TYPE_DIAGNOSTIC,
            checked.hir.span,
            format!(
                "type mismatch: expected {:?}, found {:?}",
                self.analyzer.types.kind(expected),
                self.analyzer.types.kind(checked.hir.ty)
            ),
        );
        checked
    }

    fn is_min_magnitude(&self, node: &SyntaxNode) -> bool {
        let mut current = node;
        while current.kind == SyntaxKind::ParenthesizedExpr {
            let Some(inner) = expression_child(current) else {
                return false;
            };
            current = inner;
        }
        current.kind == SyntaxKind::LiteralExpr
            && self.analyzer.context.direct_token_kind(current) == Some(TokenKind::Int)
            && self
                .analyzer
                .context
                .node_text(current)
                .is_some_and(|text| parse_int_literal(text) == ParsedIntLiteral::IntMinMagnitude)
    }

    fn allocate_local(&mut self, ty: TypeId, mutability: BindingMutability) -> LocalId {
        let local = LocalId(self.next_local);
        self.next_local = self.next_local.saturating_add(1);
        self.local_types.push(ty);
        self.local_mutability.push(mutability);
        local
    }

    fn token_kind(&self, id: keld_syntax::TokenId) -> Option<TokenKind> {
        let index = usize::try_from(id.0).ok()?;
        self.analyzer
            .context
            .lexed
            .tokens
            .get(index)
            .map(|token| token.kind)
    }

    fn direct_punct(&self, node: &SyntaxNode) -> Option<Punct> {
        node.direct_token_ids()
            .find_map(|id| match self.token_kind(id) {
                Some(TokenKind::Punct(punct)) => Some(punct),
                _ => None,
            })
    }

    fn constant_fault(&mut self, span: Span, fault: keld_numeric::NumericFault) {
        if self.suppressed_constant_faults > 0 {
            return;
        }
        self.error(
            CONSTANT_FAULT_DIAGNOSTIC,
            span,
            format!("constant Int expression faults with {fault:?}"),
        );
    }

    fn error_expression(span: Span) -> HirExpr {
        HirExpr {
            ty: TypeStore::ERROR,
            span,
            kind: HirExprKind::Int(0),
        }
    }

    fn error(&mut self, code: DiagnosticCode, span: Span, message: String) {
        self.analyzer.error(code, span, message);
    }
}

pub(crate) fn called_functions(function: &HirFunction) -> Vec<FunctionId> {
    let mut calls = BTreeSet::new();
    collect_block_calls(&function.body, &mut calls);
    calls.into_iter().collect()
}

fn collect_block_calls(block: &HirBlock, calls: &mut BTreeSet<FunctionId>) {
    for statement in &block.statements {
        match &statement.kind {
            HirStmtKind::Let { initializer, .. } => collect_expr_calls(initializer, calls),
            HirStmtKind::Var { initializer, .. } => {
                if let Some(initializer) = initializer {
                    collect_expr_calls(initializer, calls);
                }
            }
            HirStmtKind::Assign { value, .. } | HirStmtKind::CompoundAssign { value, .. } => {
                collect_expr_calls(value, calls);
            }
            HirStmtKind::Expr(expression)
            | HirStmtKind::Retire(expression)
            | HirStmtKind::Keep {
                entity: expression, ..
            } => collect_expr_calls(expression, calls),
            HirStmtKind::Return(expression) => {
                if let Some(expression) = expression {
                    collect_expr_calls(expression, calls);
                }
            }
            HirStmtKind::If(value) => {
                collect_expr_calls(&value.condition, calls);
                collect_block_calls(&value.then_block, calls);
                if let Some(block) = &value.else_block {
                    collect_block_calls(block, calls);
                }
            }
            HirStmtKind::While(value) => {
                collect_expr_calls(&value.condition, calls);
                collect_block_calls(&value.body, calls);
            }
            HirStmtKind::Break | HirStmtKind::Continue => {}
            HirStmtKind::When(value) => {
                collect_expr_calls(&value.link, calls);
                collect_block_calls(&value.live, calls);
                if let Some(block) = &value.absent {
                    collect_block_calls(block, calls);
                }
            }
            HirStmtKind::Lifecycle(value) => collect_block_calls(&value.body, calls),
        }
    }
}

fn collect_expr_calls(expression: &HirExpr, calls: &mut BTreeSet<FunctionId>) {
    match &expression.kind {
        HirExprKind::Int(_)
        | HirExprKind::Bool(_)
        | HirExprKind::TextLiteral(_)
        | HirExprKind::None
        | HirExprKind::Local(_)
        | HirExprKind::Take(_)
        | HirExprKind::ListNew => {}
        HirExprKind::Call {
            function,
            arguments,
        } => {
            calls.insert(*function);
            for (_, argument) in arguments {
                collect_expr_calls(argument, calls);
            }
        }
        HirExprKind::Unary { value, .. } | HirExprKind::EntityToLink(value) => {
            collect_expr_calls(value, calls);
        }
        HirExprKind::Binary { lhs, rhs, .. } => {
            collect_expr_calls(lhs, calls);
            collect_expr_calls(rhs, calls);
        }
        HirExprKind::Field { base, .. } => collect_expr_calls(base, calls),
        HirExprKind::UncheckedLinkField { link, .. } => collect_expr_calls(link, calls),
        HirExprKind::ConstructStruct { fields, .. }
        | HirExprKind::ConstructEntity { fields, .. } => {
            for (_, value) in fields {
                collect_expr_calls(value, calls);
            }
        }
        HirExprKind::Copy(value)
        | HirExprKind::TextByteLength(value)
        | HirExprKind::TextIsEmpty(value)
        | HirExprKind::ListLength(value)
        | HirExprKind::ListClear(value) => collect_expr_calls(value, calls),
        HirExprKind::ListIndex { list, index }
        | HirExprKind::ListGet { list, index }
        | HirExprKind::ListTryRemove { list, index }
        | HirExprKind::ListRemove { list, index } => {
            collect_expr_calls(list, calls);
            collect_expr_calls(index, calls);
        }
        HirExprKind::ListReserve { list, additional }
        | HirExprKind::ListTryReserve { list, additional } => {
            collect_expr_calls(list, calls);
            collect_expr_calls(additional, calls);
        }
        HirExprKind::ListPush { list, value } => {
            collect_expr_calls(list, calls);
            collect_expr_calls(value, calls);
        }
    }
}

fn is_statement(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::BindingStmt
            | SyntaxKind::AssignmentStmt
            | SyntaxKind::KeepStmt
            | SyntaxKind::RetireStmt
            | SyntaxKind::ReturnStmt
            | SyntaxKind::BreakStmt
            | SyntaxKind::ContinueStmt
            | SyntaxKind::ExprStmt
            | SyntaxKind::LifecycleStmt
            | SyntaxKind::WhenStmt
            | SyntaxKind::IfStmt
            | SyntaxKind::WhileStmt
            | SyntaxKind::TryStmt
    )
}

fn is_expression(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LogicalOrExpr
            | SyntaxKind::LogicalAndExpr
            | SyntaxKind::EqualityExpr
            | SyntaxKind::ComparisonExpr
            | SyntaxKind::ShiftExpr
            | SyntaxKind::AdditiveExpr
            | SyntaxKind::MultiplicativeExpr
            | SyntaxKind::UnaryExpr
            | SyntaxKind::TakeExpr
            | SyntaxKind::CallExpr
            | SyntaxKind::FieldExpr
            | SyntaxKind::IndexExpr
            | SyntaxKind::ParenthesizedExpr
            | SyntaxKind::MatchExpr
            | SyntaxKind::LiteralExpr
            | SyntaxKind::NameExpr
            | SyntaxKind::Error
    )
}

fn expression_child(node: &SyntaxNode) -> Option<&SyntaxNode> {
    node.child_nodes().find(|child| is_expression(child.kind))
}

fn expression_name<'a>(
    node: &SyntaxNode,
    context: &'a crate::analyze::SyntaxContext<'_, '_>,
) -> Option<&'a str> {
    if node.kind != SyntaxKind::NameExpr {
        return None;
    }
    direct_child(node, SyntaxKind::Name).and_then(|name| context.node_text(name))
}

fn method_name<'a>(
    node: &SyntaxNode,
    context: &'a crate::analyze::SyntaxContext<'_, '_>,
) -> Option<&'a str> {
    if node.kind != SyntaxKind::FieldExpr {
        return None;
    }
    node.child_nodes()
        .find(|child| child.kind == SyntaxKind::Name)
        .and_then(|name| context.node_text(name))
}

fn int_binary_operator(punct: Punct) -> Option<IntBinaryOp> {
    Some(match punct {
        Punct::Plus | Punct::PlusEq => IntBinaryOp::Add,
        Punct::Minus | Punct::MinusEq => IntBinaryOp::Sub,
        Punct::Star | Punct::StarEq => IntBinaryOp::Mul,
        Punct::Slash | Punct::SlashEq => IntBinaryOp::Div,
        Punct::Percent | Punct::PercentEq => IntBinaryOp::Rem,
        Punct::Shl => IntBinaryOp::Shl,
        Punct::Shr => IntBinaryOp::Shr,
        _ => return None,
    })
}

fn comparison_operator(punct: Punct) -> CompareOp {
    match punct {
        Punct::EqEq => CompareOp::Eq,
        Punct::BangEq => CompareOp::NotEq,
        Punct::Less => CompareOp::Less,
        Punct::LessEq => CompareOp::LessEq,
        Punct::Greater => CompareOp::Greater,
        Punct::GreaterEq => CompareOp::GreaterEq,
        _ => unreachable!("only comparison punctuation reaches comparison lowering"),
    }
}

fn decode_string_literal(text: &str) -> Option<String> {
    let body = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut output = String::new();
    let mut chars = body.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match chars.next()? {
            '0' => output.push('\0'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            '\\' => output.push('\\'),
            '"' => output.push('"'),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut digits = String::new();
                loop {
                    let digit = chars.next()?;
                    if digit == '}' {
                        break;
                    }
                    digits.push(digit);
                    if digits.len() > 6 {
                        return None;
                    }
                }
                let scalar = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)?;
                output.push(scalar);
            }
            _ => return None,
        }
    }
    Some(output)
}

fn block_definitely_returns(block: &HirBlock) -> bool {
    block
        .statements
        .iter()
        .any(statement_definitely_returns)
}

fn statement_definitely_returns(statement: &HirStmt) -> bool {
    match &statement.kind {
        HirStmtKind::Return(_) => true,
        HirStmtKind::Lifecycle(value) => block_definitely_returns(&value.body),
        HirStmtKind::If(value) => {
            block_definitely_returns(&value.then_block)
                && value
                    .else_block
                    .as_ref()
                    .is_some_and(block_definitely_returns)
        }
        HirStmtKind::When(value) => {
            block_definitely_returns(&value.live)
                && value.absent.as_ref().is_some_and(block_definitely_returns)
        }
        HirStmtKind::While(value) => {
            matches!(value.condition.kind, HirExprKind::Bool(true))
                && !block_has_break_for_current_loop(&value.body)
        }
        _ => false,
    }
}

fn block_has_break_for_current_loop(block: &HirBlock) -> bool {
    block.statements.iter().any(|statement| match &statement.kind {
        HirStmtKind::Break => true,
        HirStmtKind::While(_) => false,
        HirStmtKind::If(value) => {
            block_has_break_for_current_loop(&value.then_block)
                || value
                    .else_block
                    .as_ref()
                    .is_some_and(block_has_break_for_current_loop)
        }
        HirStmtKind::When(value) => {
            block_has_break_for_current_loop(&value.live)
                || value
                    .absent
                    .as_ref()
                    .is_some_and(block_has_break_for_current_loop)
        }
        HirStmtKind::Lifecycle(value) => block_has_break_for_current_loop(&value.body),
        _ => false,
    })
}
