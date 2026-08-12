use crate::{
    AllocationSite, BlockId, ExitTarget, FlowBlock, FlowFunction, FlowModule, FlowOp, LifecycleId,
    Place, StorageScopeId, Terminator, ValueId,
};
use keld_semantics::{
    CompareOp, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirFunction, HirIf, HirLifecycle,
    HirStmt, HirStmtKind, HirUnaryOp, HirWhen, LocalId, TypeId, TypeKind, TypeStore, TypedModule,
};
use keld_source::Diagnostic;

#[must_use]
pub fn lower(module: &TypedModule) -> FlowModule {
    FlowModule {
        definitions: module.definitions.clone(),
        types: module.types.clone(),
        functions: module
            .functions
            .iter()
            .map(|function| FunctionBuilder::new(module, function).lower())
            .collect(),
        main: module.main,
    }
}

/// Runs the supported text frontend and lowers its typed module.
///
/// # Errors
///
/// Returns all sorted frontend diagnostics when source analysis fails.
pub fn lower_text_for_test(text: &str) -> Result<FlowModule, Vec<Diagnostic>> {
    let analysis = keld_semantics::analyze_text(text);
    analysis
        .module
        .map_or_else(|| Err(analysis.diagnostics), |module| Ok(lower(&module)))
}

struct FunctionBuilder<'module> {
    module: &'module TypedModule,
    function: &'module HirFunction,
    blocks: Vec<FlowBlock>,
    sealed: Vec<bool>,
    current: BlockId,
    value_types: Vec<TypeId>,
    lifecycle_parents: Vec<Option<LifecycleId>>,
    storage_scope_parents: Vec<Option<StorageScopeId>>,
    local_scopes: Vec<StorageScopeId>,
    current_storage_scope: StorageScopeId,
    active_lifecycles: Vec<LifecycleId>,
    next_allocation_site: u32,
    next_call: u32,
}

impl<'module> FunctionBuilder<'module> {
    fn new(module: &'module TypedModule, function: &'module HirFunction) -> Self {
        Self {
            module,
            function,
            blocks: vec![FlowBlock {
                id: BlockId(0),
                storage_scope: StorageScopeId(0),
                operations: Vec::new(),
                terminator: Terminator::Unreachable,
            }],
            sealed: vec![false],
            current: BlockId(0),
            value_types: Vec::new(),
            lifecycle_parents: vec![None],
            storage_scope_parents: vec![None],
            local_scopes: vec![StorageScopeId(0); function.local_types.len()],
            current_storage_scope: StorageScopeId(0),
            active_lifecycles: Vec::new(),
            next_allocation_site: 0,
            next_call: 0,
        }
    }

    fn lower(mut self) -> FlowFunction {
        self.lower_block(&self.function.body);
        if self.current_is_open() {
            self.exit_to(ExitTarget::Return(None));
        }
        FlowFunction {
            id: self.function.id,
            name: self.function.name.clone(),
            span: self.function.span,
            parameters: self
                .function
                .parameters
                .iter()
                .map(|(local, _)| *local)
                .collect(),
            parameter_modes: self.function.parameter_modes.clone(),
            local_types: self.function.local_types.clone(),
            local_mutability: self.function.local_mutability.clone(),
            return_type: self.function.return_type,
            effects: self.function.effects.clone(),
            current_lifecycle: LifecycleId(0),
            value_types: self.value_types,
            lifecycle_parents: self.lifecycle_parents,
            storage_scope_parents: self.storage_scope_parents,
            local_scopes: self.local_scopes,
            blocks: self.blocks,
            entry: BlockId(0),
        }
    }

    fn lower_block(&mut self, block: &HirBlock) {
        for statement in &block.statements {
            if !self.current_is_open() {
                break;
            }
            self.lower_statement(statement);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_statement(&mut self, statement: &HirStmt) {
        match &statement.kind {
            HirStmtKind::Let { local, initializer } => {
                self.assign_local_scope(*local);
                if let Some(value) = self.lower_expression(initializer) {
                    self.emit(FlowOp::StoreLocal {
                        local: *local,
                        value,
                        span: statement.span,
                    });
                }
            }
            HirStmtKind::Var { local, initializer } => {
                self.assign_local_scope(*local);
                if let Some(initializer) = initializer
                    && let Some(value) = self.lower_expression(initializer)
                {
                    self.emit(FlowOp::StoreLocal {
                        local: *local,
                        value,
                        span: statement.span,
                    });
                }
            }
            HirStmtKind::Assign { target, value } => {
                if target.fields.is_empty() {
                    if let Some(value) = self.lower_expression(value) {
                        self.emit(FlowOp::StoreLocal {
                            local: target.base,
                            value,
                            span: statement.span,
                        });
                    }
                    return;
                }
                let entity = self.copy_local(target.base, statement.span);
                if let Some(value) = self.lower_expression(value)
                    && let Some(field) = target.fields.first()
                {
                    self.emit(FlowOp::WriteEntityField {
                        entity,
                        field: *field,
                        value,
                        span: statement.span,
                    });
                }
            }
            HirStmtKind::CompoundAssign { target, op, value } => {
                let entity = self.copy_local(target.base, statement.span);
                let Some(field) = target.fields.first().copied() else {
                    return;
                };
                let old = self.new_value(TypeStore::INT);
                self.emit(FlowOp::ReadEntityField {
                    dst: old,
                    entity,
                    field,
                    span: statement.span,
                });
                if let Some(rhs) = self.lower_expression(value) {
                    let result = self.new_value(TypeStore::INT);
                    self.emit(FlowOp::BinaryInt {
                        dst: result,
                        op: *op,
                        lhs: old,
                        rhs,
                        span: statement.span,
                    });
                    self.emit(FlowOp::WriteEntityField {
                        entity,
                        field,
                        value: result,
                        span: statement.span,
                    });
                }
            }
            HirStmtKind::Expr(expression) => {
                self.lower_expression(expression);
            }
            HirStmtKind::If(value) => self.lower_if(value),
            HirStmtKind::When(value) => self.lower_when(value),
            HirStmtKind::Lifecycle(value) => self.lower_lifecycle(value),
            HirStmtKind::Keep { entity, lifecycle } => {
                if let Some(entity) = self.lower_expression(entity) {
                    self.emit(FlowOp::Keep {
                        entity,
                        target: flow_lifecycle(*lifecycle),
                        span: statement.span,
                    });
                }
            }
            HirStmtKind::Retire(entity) => {
                if let Some(entity) = self.lower_expression(entity) {
                    self.emit(FlowOp::Retire {
                        entity,
                        span: statement.span,
                    });
                }
            }
            HirStmtKind::Return(value) => {
                let value = value
                    .as_ref()
                    .and_then(|expression| self.lower_expression(expression));
                self.exit_to(ExitTarget::Return(value));
            }
        }
    }

    fn lower_if(&mut self, value: &HirIf) {
        let then_block = self.new_block();
        let else_block = self.new_block();
        let merge_block = self.new_block();

        if let Some((lhs, rhs, equality)) = identity_condition(&value.condition, &self.module.types)
        {
            let lhs = self.lower_expression(lhs);
            let rhs = self.lower_expression(rhs);
            if let (Some(lhs), Some(rhs)) = (lhs, rhs) {
                let (equal, not_equal) = if equality {
                    (then_block, else_block)
                } else {
                    (else_block, then_block)
                };
                self.terminate(Terminator::BranchIdentity {
                    lhs,
                    rhs,
                    equal,
                    not_equal,
                });
            }
        } else if let Some(condition) = self.lower_expression(&value.condition) {
            self.terminate(Terminator::Branch {
                condition,
                then_block,
                else_block,
            });
        }

        let then_reaches_merge =
            self.lower_branch(&value.then_block, then_block, merge_block, None);
        let else_reaches_merge = if let Some(block) = &value.else_block {
            self.lower_branch(block, else_block, merge_block, None)
        } else {
            self.current = else_block;
            self.terminate(Terminator::Goto(merge_block));
            true
        };
        self.current = merge_block;
        if !then_reaches_merge && !else_reaches_merge {
            self.sealed[merge_block.0 as usize] = true;
        }
    }

    fn lower_when(&mut self, value: &HirWhen) {
        let Some(link) = self.lower_expression(&value.link) else {
            return;
        };
        let live = self.new_block();
        let absent = self.new_block();
        let merge = self.new_block();
        self.terminate(Terminator::ResolveLink {
            link,
            bind_local: value.binding,
            live,
            absent,
            span: value.link.span,
        });
        let live_reaches = self.lower_branch(&value.live, live, merge, Some(value.binding));
        let absent_reaches = if let Some(block) = &value.absent {
            self.lower_branch(block, absent, merge, None)
        } else {
            self.current = absent;
            self.terminate(Terminator::Goto(merge));
            true
        };
        self.current = merge;
        if !live_reaches && !absent_reaches {
            self.sealed[merge.0 as usize] = true;
        }
    }

    fn lower_lifecycle(&mut self, value: &HirLifecycle) {
        let lifecycle = flow_lifecycle(value.id);
        let parent = self.current_lifecycle();
        let lifecycle_index = lifecycle.0 as usize;
        if self.lifecycle_parents.len() <= lifecycle_index {
            self.lifecycle_parents.resize(lifecycle_index + 1, None);
        }
        self.lifecycle_parents[lifecycle_index] = Some(parent);
        self.emit(FlowOp::BeginLifecycle {
            lifecycle,
            parent,
            span: value.body.span,
        });
        let parent_storage_scope = self.current_storage_scope;
        let body_storage_scope = self.new_storage_scope(parent_storage_scope);
        self.active_lifecycles.push(lifecycle);
        self.current_storage_scope = body_storage_scope;
        self.lower_block(&value.body);
        let after = self.new_block();
        self.blocks[after.0 as usize].storage_scope = parent_storage_scope;
        if self.current_is_open() {
            self.terminate(Terminator::ExitScopes {
                storage_scopes: self.storage_scopes_until(parent_storage_scope),
                lifecycles: vec![lifecycle],
                next: ExitTarget::Goto(after),
            });
        } else {
            self.sealed[after.0 as usize] = true;
        }
        self.active_lifecycles.pop();
        self.current_storage_scope = parent_storage_scope;
        self.current = after;
    }

    fn lower_branch(
        &mut self,
        block: &HirBlock,
        entry: BlockId,
        merge: BlockId,
        scope_local: Option<LocalId>,
    ) -> bool {
        let parent_storage_scope = self.current_storage_scope;
        let branch_storage_scope = self.new_storage_scope(parent_storage_scope);
        self.blocks[entry.0 as usize].storage_scope = branch_storage_scope;
        if let Some(local) = scope_local {
            self.local_scopes[local.0 as usize] = branch_storage_scope;
        }
        self.current = entry;
        self.current_storage_scope = branch_storage_scope;
        self.lower_block(block);
        if self.current_is_open() {
            self.terminate(Terminator::ExitScopes {
                storage_scopes: vec![branch_storage_scope],
                lifecycles: Vec::new(),
                next: ExitTarget::Goto(merge),
            });
            self.current_storage_scope = parent_storage_scope;
            true
        } else {
            self.current_storage_scope = parent_storage_scope;
            false
        }
    }

    #[allow(clippy::too_many_lines)]
    fn lower_expression(&mut self, expression: &HirExpr) -> Option<ValueId> {
        match &expression.kind {
            HirExprKind::Int(value) => Some(self.lower_int(expression, *value)),
            HirExprKind::Bool(value) => Some(self.lower_bool(expression, *value)),
            HirExprKind::None => self.lower_none(expression),
            HirExprKind::Local(local) => Some(self.copy_local(*local, expression.span)),
            HirExprKind::Take(place) => {
                let local = place.base;
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::TakeLocal {
                    dst,
                    local,
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::Copy(value) => {
                let source = self.lower_expression(value)?;
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::CopyStorage {
                    dst,
                    source,
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::ListNew => {
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::ListNew {
                    dst,
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::TextLiteral(value) => {
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::ConstText {
                    dst,
                    value: value.clone(),
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::ListLength(list) => {
                if let HirExprKind::Local(local) = &list.kind {
                    let dst = self.new_value(expression.ty);
                    self.emit(FlowOp::ListLengthLocal {
                        dst,
                        local: *local,
                        span: expression.span,
                    });
                    return Some(dst);
                }
                let list = self.lower_expression(list)?;
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::ListLength {
                    dst,
                    list,
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::ListPush { list, value } => {
                if let HirExprKind::Local(local) = &list.kind {
                    let value = self.lower_expression(value)?;
                    self.emit(FlowOp::ListPushLocal {
                        local: *local,
                        value,
                        span: expression.span,
                    });
                    return Some(self.new_value(TypeStore::UNIT));
                }
                if let HirExprKind::Field { .. } = &list.kind {
                    let place = place_for_argument(list)?;
                    let list_value = self.lower_expression(list)?;
                    let value = self.lower_expression(value)?;
                    self.emit(FlowOp::ListPushPlace {
                        list: list_value,
                        place,
                        value,
                        span: expression.span,
                    });
                    return Some(self.new_value(TypeStore::UNIT));
                }
                let list = self.lower_expression(list)?;
                let value = self.lower_expression(value)?;
                self.emit(FlowOp::ListPush {
                    list,
                    value,
                    span: expression.span,
                });
                Some(self.new_value(TypeStore::UNIT))
            }
            HirExprKind::ListRemove { list, index } => {
                let index = self.lower_expression(index)?;
                let dst = self.new_value(expression.ty);
                if let HirExprKind::Local(local) = &list.kind {
                    self.emit(FlowOp::ListRemoveLocal {
                        dst,
                        local: *local,
                        index,
                        span: expression.span,
                    });
                } else if let HirExprKind::Field { .. } = &list.kind {
                    let place = place_for_argument(list)?;
                    let list_value = self.lower_expression(list)?;
                    self.emit(FlowOp::ListRemovePlace {
                        dst,
                        list: list_value,
                        place,
                        index,
                        span: expression.span,
                    });
                } else {
                    let list = self.lower_expression(list)?;
                    self.emit(FlowOp::ListRemove {
                        dst,
                        list,
                        index,
                        span: expression.span,
                    });
                }
                Some(dst)
            }
            HirExprKind::TextByteLength(text) => {
                let text = self.lower_expression(text)?;
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::TextByteLength {
                    dst,
                    text,
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::TextIsEmpty(text) => {
                let text = self.lower_expression(text)?;
                let dst = self.new_value(expression.ty);
                self.emit(FlowOp::TextIsEmpty {
                    dst,
                    text,
                    span: expression.span,
                });
                Some(dst)
            }
            HirExprKind::Unary { op, value } => self.lower_unary(expression, *op, value),
            HirExprKind::Binary { op, lhs, rhs } => self.lower_binary(expression, *op, lhs, rhs),
            HirExprKind::Field { base, field } => self.lower_field(expression, base, *field),
            HirExprKind::UncheckedLinkField { link, field } => {
                self.lower_unchecked_link_field(expression, link, *field)
            }
            HirExprKind::Call {
                function,
                arguments,
            } => self.lower_call(expression, *function, arguments),
            HirExprKind::ConstructStruct { definition, fields } => {
                Some(self.lower_struct(expression, *definition, fields))
            }
            HirExprKind::ConstructEntity { definition, fields } => {
                Some(self.lower_entity(expression, *definition, fields))
            }
            HirExprKind::EntityToLink(entity) => self.lower_entity_to_link(expression, entity),
        }
    }

    fn lower_int(&mut self, expression: &HirExpr, value: i64) -> ValueId {
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::ConstInt {
            dst,
            value,
            span: expression.span,
        });
        dst
    }

    fn lower_bool(&mut self, expression: &HirExpr, value: bool) -> ValueId {
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::ConstBool {
            dst,
            value,
            span: expression.span,
        });
        dst
    }

    fn lower_none(&mut self, expression: &HirExpr) -> Option<ValueId> {
        let TypeKind::Link { entity, .. } = *self.module.types.kind(expression.ty) else {
            return None;
        };
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::ConstNoneLink {
            dst,
            entity,
            span: expression.span,
        });
        Some(dst)
    }

    fn lower_unary(
        &mut self,
        expression: &HirExpr,
        op: HirUnaryOp,
        value: &HirExpr,
    ) -> Option<ValueId> {
        let value = self.lower_expression(value)?;
        let dst = self.new_value(expression.ty);
        match op {
            HirUnaryOp::Int(op) => self.emit(FlowOp::UnaryInt {
                dst,
                op,
                value,
                span: expression.span,
            }),
            HirUnaryOp::Not => self.emit(FlowOp::Not {
                dst,
                value,
                span: expression.span,
            }),
        }
        Some(dst)
    }

    fn lower_binary(
        &mut self,
        expression: &HirExpr,
        op: HirBinaryOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
    ) -> Option<ValueId> {
        if matches!(op, HirBinaryOp::And | HirBinaryOp::Or) {
            return self.lower_short_circuit(expression, op, lhs, rhs);
        }
        let lhs = self.lower_expression(lhs)?;
        let rhs = self.lower_expression(rhs)?;
        let dst = self.new_value(expression.ty);
        match op {
            HirBinaryOp::Int(op) => self.emit(FlowOp::BinaryInt {
                dst,
                op,
                lhs,
                rhs,
                span: expression.span,
            }),
            HirBinaryOp::TextConcat => self.emit(FlowOp::TextConcat {
                dst,
                lhs,
                rhs,
                span: expression.span,
            }),
            HirBinaryOp::Compare(op) => self.emit(FlowOp::Compare {
                dst,
                op,
                lhs,
                rhs,
                span: expression.span,
            }),
            HirBinaryOp::And | HirBinaryOp::Or => {
                unreachable!("handled before operand lowering")
            }
        }
        Some(dst)
    }

    fn lower_field(
        &mut self,
        expression: &HirExpr,
        base: &HirExpr,
        field: keld_semantics::FieldId,
    ) -> Option<ValueId> {
        let base_type = base.ty;
        let base = self.lower_expression(base)?;
        let dst = self.new_value(expression.ty);
        if matches!(self.module.types.kind(base_type), TypeKind::Struct(_)) {
            self.emit(FlowOp::ReadStructField {
                dst,
                base,
                field,
                span: expression.span,
            });
        } else {
            self.emit(FlowOp::ReadEntityField {
                dst,
                entity: base,
                field,
                span: expression.span,
            });
        }
        Some(dst)
    }

    fn lower_unchecked_link_field(
        &mut self,
        expression: &HirExpr,
        link: &HirExpr,
        field: keld_semantics::FieldId,
    ) -> Option<ValueId> {
        let link = self.lower_expression(link)?;
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::ReadUncheckedLinkField {
            dst,
            link,
            field,
            span: expression.span,
        });
        Some(dst)
    }

    fn lower_call(
        &mut self,
        expression: &HirExpr,
        function: keld_semantics::FunctionId,
        arguments: &[(keld_semantics::ParameterIndex, HirExpr)],
    ) -> Option<ValueId> {
        let call = self.next_call;
        self.next_call = self.next_call.saturating_add(1);
        self.emit(FlowOp::BeginCall {
            call,
            function,
            span: expression.span,
        });
        let mut lowered = Vec::with_capacity(arguments.len());
        let mut argument_places = Vec::with_capacity(arguments.len());
        for (parameter, argument) in arguments {
            if let Some(value) = self.lower_expression(argument) {
                lowered.push((*parameter, value));
                argument_places.push((*parameter, place_for_argument(argument)));
                self.emit(FlowOp::ReserveArgument {
                    call,
                    parameter: *parameter,
                    value,
                    place: place_for_argument(argument),
                    span: argument.span,
                });
            }
        }
        let dst = (expression.ty != TypeStore::UNIT).then(|| self.new_value(expression.ty));
        self.emit(FlowOp::Call {
            call,
            dst,
            function,
            arguments: lowered,
            argument_places,
            current_lifecycle: self.current_lifecycle(),
            span: expression.span,
        });
        dst
    }

    fn lower_struct(
        &mut self,
        expression: &HirExpr,
        definition: keld_semantics::DefId,
        fields: &[(keld_semantics::FieldId, HirExpr)],
    ) -> ValueId {
        let fields = self.lower_fields(fields);
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::ConstructStruct {
            dst,
            definition,
            fields,
            span: expression.span,
        });
        dst
    }

    fn lower_entity(
        &mut self,
        expression: &HirExpr,
        definition: keld_semantics::DefId,
        fields: &[(keld_semantics::FieldId, HirExpr)],
    ) -> ValueId {
        let fields = self.lower_fields(fields);
        let dst = self.new_value(expression.ty);
        let site = AllocationSite(self.next_allocation_site);
        self.next_allocation_site = self.next_allocation_site.saturating_add(1);
        self.emit(FlowOp::AllocateEntity {
            dst,
            definition,
            fields,
            lifecycle: self.current_lifecycle(),
            site,
            span: expression.span,
        });
        dst
    }

    fn lower_entity_to_link(&mut self, expression: &HirExpr, entity: &HirExpr) -> Option<ValueId> {
        let entity = self.lower_expression(entity)?;
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::EntityToLink {
            dst,
            entity,
            span: expression.span,
        });
        Some(dst)
    }

    fn lower_short_circuit(
        &mut self,
        expression: &HirExpr,
        op: HirBinaryOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
    ) -> Option<ValueId> {
        let lhs = self.lower_expression(lhs)?;
        let rhs_block = self.new_block();
        let short_block = self.new_block();
        let merge_block = self.new_block();
        let is_and = op == HirBinaryOp::And;
        self.terminate(Terminator::Branch {
            condition: lhs,
            then_block: if is_and { rhs_block } else { short_block },
            else_block: if is_and { short_block } else { rhs_block },
        });

        self.current = short_block;
        let short_value = self.new_value(TypeStore::BOOL);
        self.emit(FlowOp::ConstBool {
            dst: short_value,
            value: !is_and,
            span: expression.span,
        });
        let short_end = self.current;
        self.terminate(Terminator::Goto(merge_block));

        self.current = rhs_block;
        let rhs_value = self.lower_expression(rhs)?;
        let rhs_end = self.current;
        self.terminate(Terminator::Goto(merge_block));

        self.current = merge_block;
        let dst = self.new_value(expression.ty);
        self.emit(FlowOp::Phi {
            dst,
            inputs: vec![(short_end, short_value), (rhs_end, rhs_value)],
            span: expression.span,
        });
        Some(dst)
    }

    fn lower_fields(
        &mut self,
        fields: &[(keld_semantics::FieldId, HirExpr)],
    ) -> Vec<(keld_semantics::FieldId, ValueId)> {
        fields
            .iter()
            .filter_map(|(field, value)| self.lower_expression(value).map(|value| (*field, value)))
            .collect()
    }

    fn copy_local(&mut self, local: keld_semantics::LocalId, span: keld_source::Span) -> ValueId {
        let ty = self
            .function
            .local_types
            .get(local.0 as usize)
            .copied()
            .unwrap_or(TypeStore::ERROR);
        let dst = self.new_value(ty);
        self.emit(FlowOp::CopyLocal { dst, local, span });
        dst
    }

    fn new_value(&mut self, ty: TypeId) -> ValueId {
        let id = ValueId(u32::try_from(self.value_types.len()).unwrap_or(u32::MAX));
        self.value_types.push(ty);
        id
    }

    fn new_block(&mut self) -> BlockId {
        let id = BlockId(u32::try_from(self.blocks.len()).unwrap_or(u32::MAX));
        self.blocks.push(FlowBlock {
            id,
            storage_scope: self.current_storage_scope,
            operations: Vec::new(),
            terminator: Terminator::Unreachable,
        });
        self.sealed.push(false);
        id
    }

    fn emit(&mut self, operation: FlowOp) {
        if self.current_is_open() {
            self.blocks[self.current.0 as usize]
                .operations
                .push(operation);
        }
    }

    fn terminate(&mut self, terminator: Terminator) {
        let index = self.current.0 as usize;
        if !self.sealed[index] {
            self.blocks[index].terminator = terminator;
            self.sealed[index] = true;
        }
    }

    fn exit_to(&mut self, next: ExitTarget) {
        self.terminate(Terminator::ExitScopes {
            storage_scopes: self.storage_scopes_to_root(),
            lifecycles: self.active_lifecycles.iter().rev().copied().collect(),
            next,
        });
    }

    fn assign_local_scope(&mut self, local: LocalId) {
        if let Some(scope) = self.local_scopes.get_mut(local.0 as usize) {
            *scope = self.current_storage_scope;
        }
    }

    fn new_storage_scope(&mut self, parent: StorageScopeId) -> StorageScopeId {
        let id =
            StorageScopeId(u32::try_from(self.storage_scope_parents.len()).unwrap_or(u32::MAX));
        self.storage_scope_parents.push(Some(parent));
        id
    }

    fn storage_scopes_to_root(&self) -> Vec<StorageScopeId> {
        let mut scopes = Vec::new();
        let mut current = Some(self.current_storage_scope);
        while let Some(scope) = current {
            scopes.push(scope);
            current = self.storage_scope_parents[scope.0 as usize];
        }
        scopes
    }

    fn storage_scopes_until(&self, ancestor: StorageScopeId) -> Vec<StorageScopeId> {
        let mut scopes = Vec::new();
        let mut current = Some(self.current_storage_scope);
        while let Some(scope) = current {
            if scope == ancestor {
                break;
            }
            scopes.push(scope);
            current = self.storage_scope_parents[scope.0 as usize];
        }
        scopes
    }

    fn current_lifecycle(&self) -> LifecycleId {
        self.active_lifecycles
            .last()
            .copied()
            .unwrap_or(LifecycleId(0))
    }

    fn current_is_open(&self) -> bool {
        !self.sealed[self.current.0 as usize]
    }
}

fn place_for_argument(expression: &HirExpr) -> Option<Place> {
    match &expression.kind {
        HirExprKind::Local(local) => Some(Place {
            base: *local,
            fields: Vec::new(),
        }),
        HirExprKind::Field { base, field } => {
            let mut place = place_for_argument(base)?;
            place.fields.push(*field);
            Some(place)
        }
        HirExprKind::Copy(value) => place_for_argument(value),
        HirExprKind::Take(place) => Some(Place {
            base: place.base,
            fields: place.fields.clone(),
        }),
        _ => None,
    }
}

const fn flow_lifecycle(id: keld_semantics::HirLifecycleId) -> LifecycleId {
    LifecycleId(id.0.saturating_add(1))
}

fn identity_condition<'a>(
    condition: &'a HirExpr,
    types: &TypeStore,
) -> Option<(&'a HirExpr, &'a HirExpr, bool)> {
    let HirExprKind::Binary {
        op: HirBinaryOp::Compare(op),
        lhs,
        rhs,
    } = &condition.kind
    else {
        return None;
    };
    if !matches!(types.kind(lhs.ty), TypeKind::EntityRef(_)) {
        return None;
    }
    match op {
        CompareOp::Eq => Some((lhs, rhs, true)),
        CompareOp::NotEq => Some((lhs, rhs, false)),
        _ => None,
    }
}
