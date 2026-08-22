from pathlib import Path


def replace(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one match, found {count}: {old[:80]!r}")
    file.write_text(text.replace(old, new, 1))
    print(f"updated {path}")


replace(
    "crates/keld-semantics/src/features.rs",
    '''        SyntaxKind::TypeParameterList => Some("generic"),
        SyntaxKind::WhileStmt => Some("while"),
        SyntaxKind::BreakStmt => Some("break"),
        SyntaxKind::ContinueStmt => Some("continue"),
        SyntaxKind::MatchExpr => Some("match"),
''',
    '''        SyntaxKind::TypeParameterList => Some("generic"),
        SyntaxKind::MatchExpr => Some("match"),
''',
)

replace(
    "crates/keld-semantics/src/hir.rs",
    '''#[derive(Clone, Debug)]
pub struct HirIf {
    pub condition: HirExpr,
    pub then_block: HirBlock,
    pub else_block: Option<HirBlock>,
}

#[derive(Clone, Debug)]
pub struct HirWhen {
''',
    '''#[derive(Clone, Debug)]
pub struct HirIf {
    pub condition: HirExpr,
    pub then_block: HirBlock,
    pub else_block: Option<HirBlock>,
}

#[derive(Clone, Debug)]
pub struct HirWhile {
    pub condition: HirExpr,
    pub body: HirBlock,
}

#[derive(Clone, Debug)]
pub struct HirWhen {
''',
)

replace(
    "crates/keld-semantics/src/hir.rs",
    '''    Expr(HirExpr),
    If(HirIf),
    When(HirWhen),
''',
    '''    Expr(HirExpr),
    If(HirIf),
    While(HirWhile),
    Break,
    Continue,
    When(HirWhen),
''',
)

check = "crates/keld-semantics/src/check.rs"
replace(
    check,
    '''    HirBlock, HirExpr, HirExprKind, HirFunction, HirIf, HirLifecycle, HirLifecycleId, HirPlace,
    HirStmt, HirStmtKind, HirUnaryOp, HirWhen, LocalId, ParameterIndex, TypeId, TypeKind,
    TypeStore,
''',
    '''    HirBlock, HirExpr, HirExprKind, HirFunction, HirIf, HirLifecycle, HirLifecycleId, HirPlace,
    HirStmt, HirStmtKind, HirUnaryOp, HirWhen, HirWhile, LocalId, ParameterIndex, TypeId,
    TypeKind, TypeStore,
''',
)

replace(
    check,
    '''const FEATURE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0004");
const RETURN_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0111");
const CONSTANT_FAULT_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0120");
''',
    '''const FEATURE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0004");
const RETURN_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0111");
const LOOP_CONTROL_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0112");
const CONSTANT_FAULT_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0120");
''',
)

replace(
    check,
    '''    local_types: Vec<TypeId>,
    local_mutability: Vec<BindingMutability>,
    suppressed_constant_faults: u16,
}
''',
    '''    local_types: Vec<TypeId>,
    local_mutability: Vec<BindingMutability>,
    suppressed_constant_faults: u16,
    loop_depth: usize,
}
''',
)

replace(
    check,
    '''            local_types,
            local_mutability: vec![BindingMutability::Let; signature.parameters.len()],
            suppressed_constant_faults: 0,
        }
''',
    '''            local_types,
            local_mutability: vec![BindingMutability::Let; signature.parameters.len()],
            suppressed_constant_faults: 0,
            loop_depth: 0,
        }
''',
)

replace(
    check,
    "        let definitely_returns = body_node.is_some_and(block_definitely_returns);\n",
    "        let definitely_returns = block_definitely_returns(&body);\n",
)

replace(
    check,
    '''            SyntaxKind::IfStmt => Some(self.check_if(node, environment)),
            SyntaxKind::WhenStmt => Some(self.check_when(node, environment)),
''',
    '''            SyntaxKind::IfStmt => Some(self.check_if(node, environment)),
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
''',
)

replace(
    check,
    '''        HirStmtKind::Return(Some(checked.hir))
    }

    fn check_if(&mut self, node: &SyntaxNode, environment: &Environment) -> HirStmtKind {
''',
    '''        HirStmtKind::Return(Some(checked.hir))
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
''',
)

replace(
    check,
    '''            HirStmtKind::If(value) => {
                collect_expr_calls(&value.condition, calls);
                collect_block_calls(&value.then_block, calls);
                if let Some(block) = &value.else_block {
                    collect_block_calls(block, calls);
                }
            }
            HirStmtKind::When(value) => {
''',
    '''            HirStmtKind::If(value) => {
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
''',
)

replace(
    check,
    '''fn block_definitely_returns(block: &SyntaxNode) -> bool {
    block
        .child_nodes()
        .filter(|child| is_statement(child.kind))
        .any(statement_definitely_returns)
}

fn statement_definitely_returns(statement: &SyntaxNode) -> bool {
    match statement.kind {
        SyntaxKind::ReturnStmt => true,
        SyntaxKind::LifecycleStmt => {
            direct_child(statement, SyntaxKind::Block).is_some_and(block_definitely_returns)
        }
        SyntaxKind::IfStmt => {
            let expressions = statement
                .child_nodes()
                .filter(|child| is_expression(child.kind))
                .count();
            let blocks = statement
                .child_nodes()
                .filter(|child| child.kind == SyntaxKind::Block)
                .collect::<Vec<_>>();
            blocks.len() > expressions && blocks.iter().all(|block| block_definitely_returns(block))
        }
        SyntaxKind::WhenStmt => {
            let blocks = statement
                .child_nodes()
                .filter(|child| child.kind == SyntaxKind::Block)
                .collect::<Vec<_>>();
            blocks.len() == 2 && blocks.iter().all(|block| block_definitely_returns(block))
        }
        _ => false,
    }
}
''',
    '''fn block_definitely_returns(block: &HirBlock) -> bool {
    block.statements.iter().any(statement_definitely_returns)
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
            matches!(&value.condition.kind, HirExprKind::Bool(true))
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
''',
)

replace(
    "crates/keld-semantics/tests/feature_gate.rs",
    '''        ("enum E { A }\\nfn main() -> Int { return 0 }\\n", "enum"),
        (
            "fn main() -> Int { while true { break }; return 0 }\\n",
            "while",
        ),
        ("fn main() -> Int { break; return 0 }\\n", "break"),
        ("fn main() -> Int { continue; return 0 }\\n", "continue"),
        ("fn main() -> Int { match 0 { _ => 0 } }\\n", "match"),
''',
    '''        ("enum E { A }\\nfn main() -> Int { return 0 }\\n", "enum"),
        ("fn main() -> Int { match 0 { _ => 0 } }\\n", "match"),
''',
)

replace(
    "crates/keld-semantics/tests/feature_gate.rs",
    '''fn outer_unsupported_construct_suppresses_child_feature_cascades() {
    let analysis = analyze_text("fn main() -> Int { while true { var x = 0; break }; return 0 }\\n");
    let feature_diagnostics = analysis
''',
    '''fn outer_unsupported_construct_suppresses_child_feature_cascades() {
    let analysis = analyze_text(
        "fn main() -> Int { try { try { return 0 } handle Error as inner { return 0 } } handle Error as outer { return 0 } }\\n",
    );
    let feature_diagnostics = analysis
''',
)

replace(
    "crates/keld-semantics/tests/feature_gate.rs",
    '    assert!(feature_diagnostics[0].primary.message.contains("while"));\n',
    '    assert!(feature_diagnostics[0].primary.message.contains("try"));\n',
)

replace(
    "crates/keld-semantics/tests/control_flow_semantics.rs",
    '''    assert!(matches!(while_.condition.kind, HirExprKind::Binary { .. }));
    assert!(matches!(
        while_.body.statements[0].kind,
''',
    '''    assert!(matches!(&while_.condition.kind, HirExprKind::Binary { .. }));
    assert!(matches!(
        &while_.body.statements[0].kind,
''',
)
