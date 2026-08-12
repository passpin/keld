use super::{BodyChecker, CheckedExpr, Environment, expression_child, is_expression};
use crate::{HirExpr, HirExprKind, HirPlace, HirProjection, TypeId, TypeKind, TypeStore};
use keld_source::DiagnosticCode;
use keld_syntax::{SyntaxKind, SyntaxNode};

impl BodyChecker<'_, '_, '_> {
    pub(super) fn check_place(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
    ) -> Option<(HirPlace, TypeId)> {
        let mut children = node.child_nodes();
        let base_node = children
            .next()
            .filter(|child| child.kind == SyntaxKind::Name)?;
        let base_name = self.analyzer.context.node_text(base_node)?;
        let Some((base, base_ty)) = environment.locals.get(base_name).copied() else {
            self.error(
                super::UNKNOWN_NAME_DIAGNOSTIC,
                base_node.span,
                format!("unknown local `{base_name}`"),
            );
            return None;
        };
        let mut current_ty = base_ty;
        let mut projections = Vec::new();
        for child in children {
            if child.kind == SyntaxKind::Name {
                let field_name = self.analyzer.context.node_text(child).unwrap_or("<error>");
                let (TypeKind::Struct(definition) | TypeKind::EntityRef(definition)) =
                    *self.analyzer.types.kind(current_ty)
                else {
                    self.error(
                        super::FEATURE_DIAGNOSTIC,
                        child.span,
                        "field projection requires a struct or entity reference".to_owned(),
                    );
                    return None;
                };
                let Some(field) = self.analyzer.definitions[definition.0 as usize]
                    .fields
                    .iter()
                    .find(|field| field.name == field_name)
                    .cloned()
                else {
                    self.error(
                        super::FIELD_DIAGNOSTIC,
                        child.span,
                        format!("unknown field `{field_name}`"),
                    );
                    return None;
                };
                current_ty = field.ty;
                projections.push(HirProjection::Field(field.id));
            } else if is_expression(child.kind) {
                let index = self.check_expression(child, Some(TypeStore::INT), environment);
                let TypeKind::List(element) = *self.analyzer.types.kind(current_ty) else {
                    self.error(
                        super::TYPE_DIAGNOSTIC,
                        child.span,
                        "index projection requires a List receiver".to_owned(),
                    );
                    return None;
                };
                current_ty = element;
                projections.push(HirProjection::Index(index.hir));
            }
        }
        let requires_var = projections.is_empty()
            || (matches!(projections.first(), Some(HirProjection::Field(_)))
                && matches!(self.analyzer.types.kind(base_ty), TypeKind::Struct(_)));
        if requires_var && self.local_mutability[base.0 as usize] != crate::BindingMutability::Var {
            self.error(
                DiagnosticCode("KLD2010"),
                node.span,
                "cannot replace through an immutable local".to_owned(),
            );
        }
        Some((
            HirPlace {
                base,
                projections,
                span: node.span,
            },
            current_ty,
        ))
    }

    pub(super) fn check_list_index(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let expressions = node
            .child_nodes()
            .filter(|child| is_expression(child.kind))
            .collect::<Vec<_>>();
        let (Some(list_node), Some(index_node)) = (expressions.first(), expressions.get(1)) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        let list = self.check_expression(list_node, None, environment);
        let index = self.check_expression(index_node, Some(TypeStore::INT), environment);
        let Some(element) = list_element(self, list.hir.ty, list.hir.span) else {
            return CheckedExpr {
                hir: Self::error_expression(node.span),
                constant_int: None,
            };
        };
        CheckedExpr {
            hir: HirExpr {
                ty: element,
                span: node.span,
                kind: HirExprKind::ListIndex {
                    list: Box::new(list.hir),
                    index: Box::new(index.hir),
                },
            },
            constant_int: index.constant_int,
        }
    }

    pub(super) fn check_list_get_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some((base, element)) = self.check_list_receiver(callee, environment) else {
            return Self::error_checked(node.span);
        };
        if self.analyzer.types.storage_class(element) != crate::StorageClass::ImplicitCopy {
            self.error(
                super::TYPE_DIAGNOSTIC,
                base.hir.span,
                "`.get()` requires an implicitly copyable element type".to_owned(),
            );
        }
        let Some(index) = self.check_one_index_argument(node, environment, "`.get()`") else {
            return Self::error_checked(node.span);
        };
        let ty = self.analyzer.types.intern(TypeKind::Optional(element));
        CheckedExpr {
            hir: HirExpr {
                ty,
                span: node.span,
                kind: HirExprKind::ListGet {
                    list: Box::new(base.hir),
                    index: Box::new(index.hir),
                },
            },
            constant_int: None,
        }
    }

    pub(super) fn check_list_try_remove_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some((base, element)) = self.check_list_receiver(callee, environment) else {
            return Self::error_checked(node.span);
        };
        let Some(index) = self.check_one_index_argument(node, environment, "`.try_remove()`")
        else {
            return Self::error_checked(node.span);
        };
        let ty = self.analyzer.types.intern(TypeKind::Optional(element));
        CheckedExpr {
            hir: HirExpr {
                ty,
                span: node.span,
                kind: HirExprKind::ListTryRemove {
                    list: Box::new(base.hir),
                    index: Box::new(index.hir),
                },
            },
            constant_int: None,
        }
    }

    pub(super) fn check_list_clear_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        let Some((base, _)) = self.check_list_receiver(callee, environment) else {
            return Self::error_checked(node.span);
        };
        if node
            .child_nodes()
            .any(|child| child.kind == SyntaxKind::Argument)
        {
            self.error(
                super::ARGUMENT_DIAGNOSTIC,
                node.span,
                "`.clear()` does not accept arguments".to_owned(),
            );
        }
        CheckedExpr {
            hir: HirExpr {
                ty: TypeStore::UNIT,
                span: node.span,
                kind: HirExprKind::ListClear(Box::new(base.hir)),
            },
            constant_int: None,
        }
    }

    pub(super) fn check_list_reserve_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        self.check_list_capacity_call(node, callee, environment, false)
    }

    pub(super) fn check_list_try_reserve_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> CheckedExpr {
        self.check_list_capacity_call(node, callee, environment, true)
    }

    fn check_list_capacity_call(
        &mut self,
        node: &SyntaxNode,
        callee: &SyntaxNode,
        environment: &mut Environment,
        fallible: bool,
    ) -> CheckedExpr {
        let method = if fallible {
            "`.try_reserve()`"
        } else {
            "`.reserve()`"
        };
        let Some((base, _)) = self.check_list_receiver(callee, environment) else {
            return Self::error_checked(node.span);
        };
        let Some(additional) = self.check_one_argument(node, environment, method) else {
            return Self::error_checked(node.span);
        };
        CheckedExpr {
            hir: HirExpr {
                ty: if fallible {
                    TypeStore::BOOL
                } else {
                    TypeStore::UNIT
                },
                span: node.span,
                kind: if fallible {
                    HirExprKind::ListTryReserve {
                        list: Box::new(base.hir),
                        additional: Box::new(additional.hir),
                    }
                } else {
                    HirExprKind::ListReserve {
                        list: Box::new(base.hir),
                        additional: Box::new(additional.hir),
                    }
                },
            },
            constant_int: None,
        }
    }

    fn check_list_receiver(
        &mut self,
        callee: &SyntaxNode,
        environment: &mut Environment,
    ) -> Option<(CheckedExpr, TypeId)> {
        let base_node = callee
            .child_nodes()
            .find(|child| is_expression(child.kind))?;
        let base = self.check_expression(base_node, None, environment);
        let element = list_element(self, base.hir.ty, base.hir.span)?;
        Some((base, element))
    }

    fn check_one_index_argument(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
        method: &str,
    ) -> Option<CheckedExpr> {
        let arguments = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Argument)
            .collect::<Vec<_>>();
        if arguments.len() != 1 {
            self.error(
                super::ARGUMENT_DIAGNOSTIC,
                node.span,
                format!("{method} requires exactly one index argument"),
            );
            return None;
        }
        let value = expression_child(arguments[0])?;
        Some(self.check_expression(value, Some(TypeStore::INT), environment))
    }

    fn check_one_argument(
        &mut self,
        node: &SyntaxNode,
        environment: &mut Environment,
        method: &str,
    ) -> Option<CheckedExpr> {
        let arguments = node
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::Argument)
            .collect::<Vec<_>>();
        if arguments.len() != 1 {
            self.error(
                super::ARGUMENT_DIAGNOSTIC,
                node.span,
                format!("{method} requires exactly one argument"),
            );
            return None;
        }
        let value = expression_child(arguments[0])?;
        Some(self.check_expression(value, Some(TypeStore::INT), environment))
    }

    fn error_checked(span: keld_source::Span) -> CheckedExpr {
        CheckedExpr {
            hir: Self::error_expression(span),
            constant_int: None,
        }
    }
}

fn list_element(
    checker: &mut BodyChecker<'_, '_, '_>,
    ty: TypeId,
    span: keld_source::Span,
) -> Option<TypeId> {
    if let TypeKind::List(element) = *checker.analyzer.types.kind(ty) {
        Some(element)
    } else {
        checker.error(
            super::TYPE_DIAGNOSTIC,
            span,
            "List operation requires a List receiver".to_owned(),
        );
        None
    }
}
