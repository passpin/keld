from pathlib import Path

path = Path("crates/keld-flow/src/lower.rs")
text = path.read_text()


def replace(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match, found {count}: {old[:120]!r}")
    text = text.replace(old, new, 1)


replace(
    '''    CompareOp, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirFunction, HirIf, HirLifecycle,
    HirPlace, HirProjection, HirStmt, HirStmtKind, HirUnaryOp, HirWhen, LocalId, TypeId, TypeKind,
    TypeStore, TypedModule,
''',
    '''    CompareOp, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirFunction, HirIf, HirLifecycle,
    HirPlace, HirProjection, HirStmt, HirStmtKind, HirUnaryOp, HirWhen, HirWhile, LocalId, TypeId,
    TypeKind, TypeStore, TypedModule,
''',
)

replace(
    '''struct FunctionBuilder<'module> {
''',
    '''#[derive(Clone, Copy)]
struct LoopTarget {
    condition: BlockId,
    exit: BlockId,
    outer_storage_scope: StorageScopeId,
    lifecycle_depth: usize,
}

struct FunctionBuilder<'module> {
''',
)

replace(
    '''    current_storage_scope: StorageScopeId,
    active_lifecycles: Vec<LifecycleId>,
    next_allocation_site: u32,
''',
    '''    current_storage_scope: StorageScopeId,
    active_lifecycles: Vec<LifecycleId>,
    loop_targets: Vec<LoopTarget>,
    next_allocation_site: u32,
''',
)

replace(
    '''            current_storage_scope: StorageScopeId(0),
            active_lifecycles: Vec::new(),
            next_allocation_site: 0,
''',
    '''            current_storage_scope: StorageScopeId(0),
            active_lifecycles: Vec::new(),
            loop_targets: Vec::new(),
            next_allocation_site: 0,
''',
)

replace(
    '''            HirStmtKind::Expr(expression) => {
                self.lower_expression(expression);
            }
            HirStmtKind::If(value) => self.lower_if(value),
            HirStmtKind::When(value) => self.lower_when(value),
''',
    '''            HirStmtKind::Expr(expression) => {
                self.lower_expression(expression);
            }
            HirStmtKind::If(value) => self.lower_if(value),
            HirStmtKind::While(value) => self.lower_while(value),
            HirStmtKind::Break => self.lower_loop_control(true),
            HirStmtKind::Continue => self.lower_loop_control(false),
            HirStmtKind::When(value) => self.lower_when(value),
''',
)

replace(
    '''    fn lower_if(&mut self, value: &HirIf) {
''',
    '''    fn lower_while(&mut self, value: &HirWhile) {
        let outer_storage_scope = self.current_storage_scope;
        let condition_storage_scope = self.new_storage_scope(outer_storage_scope);
        let body_storage_scope = self.new_storage_scope(outer_storage_scope);

        let condition = self.new_block();
        let condition_branch = self.new_block();
        let body = self.new_block();
        let exit = self.new_block();
        self.blocks[condition.0 as usize].storage_scope = condition_storage_scope;
        self.blocks[condition_branch.0 as usize].storage_scope = outer_storage_scope;
        self.blocks[body.0 as usize].storage_scope = body_storage_scope;
        self.blocks[exit.0 as usize].storage_scope = outer_storage_scope;

        self.terminate(Terminator::Goto(condition));

        self.current = condition;
        self.current_storage_scope = condition_storage_scope;
        let condition_value = self.lower_expression(&value.condition);
        if self.current_is_open() {
            self.terminate(Terminator::ExitScopes {
                storage_scopes: vec![condition_storage_scope],
                lifecycles: Vec::new(),
                next: ExitTarget::Goto(condition_branch),
            });
        }

        self.current = condition_branch;
        self.current_storage_scope = outer_storage_scope;
        if let Some(condition_value) = condition_value {
            self.terminate(Terminator::Branch {
                condition: condition_value,
                then_block: body,
                else_block: exit,
            });
        } else {
            self.terminate(Terminator::Goto(exit));
        }

        self.loop_targets.push(LoopTarget {
            condition,
            exit,
            outer_storage_scope,
            lifecycle_depth: self.active_lifecycles.len(),
        });
        self.current = body;
        self.current_storage_scope = body_storage_scope;
        self.lower_block(&value.body);
        if self.current_is_open() {
            self.terminate(Terminator::ExitScopes {
                storage_scopes: self.storage_scopes_until(outer_storage_scope),
                lifecycles: Vec::new(),
                next: ExitTarget::Goto(condition),
            });
        }
        self.loop_targets.pop();

        self.current = exit;
        self.current_storage_scope = outer_storage_scope;
    }

    fn lower_loop_control(&mut self, is_break: bool) {
        let Some(target) = self.loop_targets.last().copied() else {
            return;
        };
        let storage_scopes = self.storage_scopes_until(target.outer_storage_scope);
        let lifecycles = self.active_lifecycles[target.lifecycle_depth..]
            .iter()
            .rev()
            .copied()
            .collect();
        self.terminate(Terminator::ExitScopes {
            storage_scopes,
            lifecycles,
            next: ExitTarget::Goto(if is_break { target.exit } else { target.condition }),
        });
    }

    fn lower_if(&mut self, value: &HirIf) {
''',
)

path.write_text(text)
print("implemented structured Flow loop lowering")
