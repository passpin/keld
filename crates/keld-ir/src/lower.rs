use crate::{
    ArgumentSource, Function, Instruction, IrBlock, IrBlockId, IrDefinition, IrDefinitionKind,
    IrType, Module, Register, RegisterStorage, Terminator, ViewId, ViewMode,
};
use keld_flow::FlowModule;
use keld_flow::{ExitTarget, FlowFunction, FlowOp, LifecycleId, ValueId};
use keld_lifecycle::VerifiedFlowModule;
use keld_semantics::{CompareOp, DefinitionKind, LocalId, TypeId, TypeKind, TypeStore};
use keld_storage::{
    CleanupAction, FunctionStoragePlan, FunctionStorageSummary, HomeId, LocalStorage, StoreKind,
    ValueStorage, VerifiedStorageModule,
};
use std::collections::BTreeMap;

#[must_use]
pub trait VerifiedInput {
    fn flow(&self) -> &FlowModule;
    fn storage_summary(
        &self,
        function: keld_semantics::FunctionId,
    ) -> Option<&FunctionStorageSummary>;
    fn storage_plan(&self, function: keld_semantics::FunctionId) -> Option<&FunctionStoragePlan>;
}

impl VerifiedInput for VerifiedFlowModule {
    fn flow(&self) -> &FlowModule {
        &self.flow
    }

    fn storage_summary(
        &self,
        _function: keld_semantics::FunctionId,
    ) -> Option<&FunctionStorageSummary> {
        None
    }

    fn storage_plan(&self, _function: keld_semantics::FunctionId) -> Option<&FunctionStoragePlan> {
        None
    }
}

impl VerifiedInput for VerifiedStorageModule {
    fn flow(&self) -> &FlowModule {
        &self.lifecycle.flow
    }

    fn storage_summary(
        &self,
        function: keld_semantics::FunctionId,
    ) -> Option<&FunctionStorageSummary> {
        self.summaries.get(function.0 as usize)
    }

    fn storage_plan(&self, function: keld_semantics::FunctionId) -> Option<&FunctionStoragePlan> {
        self.annotations.functions.get(function.0 as usize)
    }
}

#[must_use]
pub fn lower<V: VerifiedInput>(verified: &V) -> Module {
    let flow = verified.flow();
    Module {
        definitions: flow
            .definitions
            .iter()
            .map(|definition| IrDefinition {
                id: definition.id,
                kind: match definition.kind {
                    DefinitionKind::Struct => IrDefinitionKind::Struct,
                    DefinitionKind::Entity => IrDefinitionKind::Entity,
                },
                fields: definition
                    .fields
                    .iter()
                    .map(|field| (field.id, map_type(&flow.types, field.ty)))
                    .collect(),
            })
            .collect(),
        functions: flow
            .functions
            .iter()
            .map(|function| {
                FunctionLowerer::new(
                    function,
                    &flow.types,
                    verified.storage_summary(function.id),
                    verified.storage_plan(function.id),
                )
                .lower()
            })
            .collect(),
        main: flow.main,
    }
}

struct Registers {
    values: Vec<Register>,
    locals: Vec<Register>,
    lifecycles: Vec<Register>,
    identity_conditions: BTreeMap<keld_flow::BlockId, Register>,
    types: Vec<IrType>,
    storage: Vec<RegisterStorage>,
}

impl Registers {
    fn new(
        function: &FlowFunction,
        types: &TypeStore,
        storage_plan: Option<&FunctionStoragePlan>,
    ) -> Self {
        let mut register_types = Vec::new();
        let mut register_storage = Vec::new();
        let mut push = |ty, storage| {
            let register = Register(
                u32::try_from(register_types.len()).expect("verified register count fits in u32"),
            );
            register_types.push(ty);
            register_storage.push(storage);
            register
        };
        let values = function
            .value_types
            .iter()
            .enumerate()
            .map(|(index, ty)| {
                push(
                    map_type(types, *ty),
                    storage_plan.map_or(RegisterStorage::Trivial, |plan| {
                        value_register_storage(plan, keld_flow::ValueId(index as u32))
                    }),
                )
            })
            .collect();
        let locals = function
            .local_types
            .iter()
            .enumerate()
            .map(|(index, ty)| {
                push(
                    map_type(types, *ty),
                    storage_plan.map_or(RegisterStorage::Trivial, |plan| {
                        local_register_storage(plan, LocalId(index as u32))
                    }),
                )
            })
            .collect();
        let lifecycles = function
            .lifecycle_parents
            .iter()
            .map(|_| push(IrType::Lifecycle, RegisterStorage::Trivial))
            .collect();
        let identity_conditions = function
            .blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.terminator,
                    keld_flow::Terminator::BranchIdentity { .. }
                )
            })
            .map(|block| (block.id, push(IrType::Bool, RegisterStorage::Trivial)))
            .collect();
        Self {
            values,
            locals,
            lifecycles,
            identity_conditions,
            types: register_types,
            storage: register_storage,
        }
    }

    fn value(&self, value: ValueId) -> Register {
        self.values[value.0 as usize]
    }

    fn local(&self, local: LocalId) -> Register {
        self.locals[local.0 as usize]
    }

    fn lifecycle(&self, lifecycle: LifecycleId) -> Register {
        self.lifecycles[lifecycle.0 as usize]
    }

    fn add_scratch(&mut self, ty: IrType, storage: RegisterStorage) -> Register {
        let register =
            Register(u32::try_from(self.types.len()).expect("verified register count fits in u32"));
        self.types.push(ty);
        self.storage.push(storage);
        register
    }

    fn storage(&self, register: Register) -> Option<&RegisterStorage> {
        self.storage.get(register.0 as usize)
    }
}

struct FunctionLowerer<'flow> {
    function: &'flow FlowFunction,
    types: &'flow TypeStore,
    registers: Registers,
    next_view: u32,
    storage_summary: Option<&'flow FunctionStorageSummary>,
    storage_plan: Option<&'flow FunctionStoragePlan>,
}

impl<'flow> FunctionLowerer<'flow> {
    fn new(
        function: &'flow FlowFunction,
        types: &'flow TypeStore,
        storage_summary: Option<&'flow FunctionStorageSummary>,
        storage_plan: Option<&'flow FunctionStoragePlan>,
    ) -> Self {
        Self {
            function,
            types,
            registers: Registers::new(function, types, storage_plan),
            next_view: 0,
            storage_summary,
            storage_plan,
        }
    }

    fn lower(mut self) -> Function {
        let blocks = self
            .function
            .blocks
            .iter()
            .map(|block| {
                let mut instructions = Vec::new();
                let block_plan = self
                    .storage_plan
                    .and_then(|plan| plan.blocks.get(block.id.0 as usize))
                    .cloned();
                for (operation_index, operation) in block.operations.iter().enumerate() {
                    let operation_plan = block_plan
                        .as_ref()
                        .and_then(|plan| plan.operations.get(operation_index));
                    self.operation(
                        operation,
                        operation_plan.and_then(|plan| plan.store),
                        &mut instructions,
                    );
                    if let Some(operation_plan) = operation_plan {
                        self.emit_cleanup_actions(&operation_plan.post_success, &mut instructions);
                    }
                }
                let terminator = self.terminator(
                    block.id,
                    &block.terminator,
                    block_plan.as_ref(),
                    &mut instructions,
                );
                IrBlock {
                    id: IrBlockId(block.id.0),
                    instructions,
                    terminator,
                }
            })
            .collect();
        Function {
            id: self.function.id,
            span: self.function.span,
            parameters: self
                .function
                .parameters
                .iter()
                .map(|local| self.registers.local(*local))
                .collect(),
            parameter_modes: self.function.parameter_modes.clone(),
            parameter_effects: self.storage_summary.map_or_else(
                || vec![keld_storage::LoanEffect::Read; self.function.parameters.len()],
                |summary| summary.effects.clone(),
            ),
            current_lifecycle: self.registers.lifecycle(self.function.current_lifecycle),
            register_types: self.registers.types,
            register_storage: self.registers.storage,
            storage_scope_parents: self.function.storage_scope_parents.clone(),
            return_type: map_type(self.types, self.function.return_type),
            blocks,
            entry: IrBlockId(self.function.entry.0),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn operation(
        &mut self,
        operation: &FlowOp,
        store_kind: Option<StoreKind>,
        output: &mut Vec<Instruction>,
    ) {
        match operation {
            FlowOp::ConstInt { dst, value, span } => output.push(Instruction::ConstInt {
                dst: self.registers.value(*dst),
                value: *value,
                span: *span,
            }),
            FlowOp::ConstBool { dst, value, span } => output.push(Instruction::ConstBool {
                dst: self.registers.value(*dst),
                value: *value,
                span: *span,
            }),
            FlowOp::ConstText { dst, value, span } => output.push(Instruction::ConstText {
                dst: self.registers.value(*dst),
                value: value.clone(),
                span: *span,
            }),
            FlowOp::ConstNoneLink { dst, entity, span } => {
                output.push(Instruction::ConstNoneLink {
                    dst: self.registers.value(*dst),
                    entity: *entity,
                    span: *span,
                });
            }
            FlowOp::BeginLifecycle {
                lifecycle,
                parent,
                span,
            } => output.push(Instruction::BeginLifecycle {
                dst: self.registers.lifecycle(*lifecycle),
                parent: self.registers.lifecycle(*parent),
                span: *span,
            }),
            FlowOp::CopyLocal { dst, local, span } => output.push(Instruction::Copy {
                dst: self.registers.value(*dst),
                src: self.registers.local(*local),
                span: *span,
            }),
            FlowOp::StoreLocal { local, value, span } => {
                let destination = self.registers.local(*local);
                let source = self.registers.value(*value);
                if self.is_home(destination) {
                    let displaced = self.registers.add_scratch(
                        self.registers.types[destination.0 as usize].clone(),
                        RegisterStorage::DropSlot,
                    );
                    output.push(Instruction::InstallHome {
                        destination,
                        source,
                        displaced,
                        span: *span,
                    });
                    if matches!(
                        store_kind,
                        Some(StoreKind::ReplaceLive | StoreKind::ReplaceMaybeLive)
                    ) {
                        output.push(Instruction::DropSlot {
                            slot: displaced,
                            span: *span,
                        });
                    }
                } else {
                    output.push(Instruction::Copy {
                        dst: destination,
                        src: source,
                        span: *span,
                    });
                }
            }
            FlowOp::TakeLocal { dst, local, span } => {
                let destination = self.registers.value(*dst);
                let source = self.registers.local(*local);
                if self.is_home(destination) {
                    output.push(Instruction::MoveHome {
                        destination,
                        source,
                        span: *span,
                    });
                } else {
                    output.push(Instruction::Take {
                        dst: destination,
                        src: source,
                        span: *span,
                    });
                }
            }
            FlowOp::CopyStorage { dst, source, span } => output.push(Instruction::Copy {
                dst: self.registers.value(*dst),
                src: self.registers.value(*source),
                span: *span,
            }),
            FlowOp::ListNew { dst, span } => output.push(Instruction::ListNew {
                dst: self.registers.value(*dst),
                span: *span,
            }),
            FlowOp::ListLength { dst, list, span } => output.push(Instruction::ListLength {
                dst: self.registers.value(*dst),
                list: self.registers.value(*list),
                span: *span,
            }),
            FlowOp::ListPush { list, value, span } => output.push(Instruction::ListPush {
                list: self.registers.value(*list),
                value: self.registers.value(*value),
                span: *span,
            }),
            FlowOp::ListPushPlace {
                list,
                place,
                value,
                span,
            } => output.push(Instruction::ListPushPlace {
                list: self.registers.value(*list),
                source: ArgumentSource {
                    base: self.registers.local(place.base),
                    fields: place.fields.clone(),
                },
                value: self.registers.value(*value),
                span: *span,
            }),
            FlowOp::ListLengthLocal { dst, local, span } => output.push(Instruction::ListLength {
                dst: self.registers.value(*dst),
                list: self.registers.local(*local),
                span: *span,
            }),
            FlowOp::ListPushLocal { local, value, span } => output.push(Instruction::ListPush {
                list: self.registers.local(*local),
                value: self.registers.value(*value),
                span: *span,
            }),
            FlowOp::ListRemove {
                dst,
                list,
                index,
                span,
            } => output.push(Instruction::ListRemove {
                dst: self.registers.value(*dst),
                list: self.registers.value(*list),
                index: self.registers.value(*index),
                span: *span,
            }),
            FlowOp::ListRemovePlace {
                dst,
                list,
                place,
                index,
                span,
            } => output.push(Instruction::ListRemovePlace {
                dst: self.registers.value(*dst),
                list: self.registers.value(*list),
                source: ArgumentSource {
                    base: self.registers.local(place.base),
                    fields: place.fields.clone(),
                },
                index: self.registers.value(*index),
                span: *span,
            }),
            FlowOp::ListRemoveLocal {
                dst,
                local,
                index,
                span,
            } => output.push(Instruction::ListRemove {
                dst: self.registers.value(*dst),
                list: self.registers.local(*local),
                index: self.registers.value(*index),
                span: *span,
            }),
            FlowOp::TextByteLength { dst, text, span } => {
                output.push(Instruction::TextByteLength {
                    dst: self.registers.value(*dst),
                    text: self.registers.value(*text),
                    span: *span,
                });
            }
            FlowOp::TextIsEmpty { dst, text, span } => output.push(Instruction::TextIsEmpty {
                dst: self.registers.value(*dst),
                text: self.registers.value(*text),
                span: *span,
            }),
            FlowOp::TextConcat {
                dst,
                lhs,
                rhs,
                span,
            } => output.push(Instruction::TextConcat {
                dst: self.registers.value(*dst),
                lhs: self.registers.value(*lhs),
                rhs: self.registers.value(*rhs),
                span: *span,
            }),
            FlowOp::UnaryInt {
                dst,
                op,
                value,
                span,
            } => output.push(Instruction::CheckedUnaryInt {
                dst: self.registers.value(*dst),
                op: *op,
                src: self.registers.value(*value),
                span: *span,
            }),
            FlowOp::BinaryInt {
                dst,
                op,
                lhs,
                rhs,
                span,
            } => output.push(Instruction::CheckedBinaryInt {
                dst: self.registers.value(*dst),
                op: *op,
                lhs: self.registers.value(*lhs),
                rhs: self.registers.value(*rhs),
                span: *span,
            }),
            FlowOp::Not { dst, value, span } => output.push(Instruction::Not {
                dst: self.registers.value(*dst),
                src: self.registers.value(*value),
                span: *span,
            }),
            FlowOp::Compare {
                dst,
                op,
                lhs,
                rhs,
                span,
            } => output.push(Instruction::Compare {
                dst: self.registers.value(*dst),
                op: *op,
                lhs: self.registers.value(*lhs),
                rhs: self.registers.value(*rhs),
                span: *span,
            }),
            _ => self.effect_operation(operation, output),
        }
    }

    fn effect_operation(&mut self, operation: &FlowOp, output: &mut Vec<Instruction>) {
        match operation {
            FlowOp::Phi { dst, inputs, span } => output.push(Instruction::Phi {
                dst: self.registers.value(*dst),
                inputs: inputs
                    .iter()
                    .map(|(block, value)| (IrBlockId(block.0), self.registers.value(*value)))
                    .collect(),
                span: *span,
            }),
            FlowOp::ConstructStruct {
                dst,
                definition,
                fields,
                span,
            } => output.push(Instruction::ConstructStruct {
                dst: self.registers.value(*dst),
                definition: *definition,
                fields: self.fields(fields),
                span: *span,
            }),
            FlowOp::AllocateEntity {
                dst,
                definition,
                fields,
                lifecycle,
                span,
                ..
            } => output.push(Instruction::AllocateEntity {
                dst: self.registers.value(*dst),
                definition: *definition,
                fields: self.fields(fields),
                lifecycle: self.registers.lifecycle(*lifecycle),
                span: *span,
            }),
            FlowOp::EntityToLink { dst, entity, span } => {
                output.push(Instruction::EntityToLink {
                    dst: self.registers.value(*dst),
                    entity: self.registers.value(*entity),
                    span: *span,
                });
            }
            FlowOp::ReadStructField {
                dst,
                base,
                field,
                span,
            } => output.push(Instruction::ReadStructField {
                dst: self.registers.value(*dst),
                base: self.registers.value(*base),
                field: *field,
                span: *span,
            }),
            _ => self.entity_operation(operation, output),
        }
    }

    fn entity_operation(&mut self, operation: &FlowOp, output: &mut Vec<Instruction>) {
        match operation {
            FlowOp::ReadEntityField {
                dst,
                entity,
                field,
                span,
            } => self.read_entity(*dst, *entity, *field, *span, output),
            FlowOp::WriteEntityField {
                entity,
                field,
                value,
                span,
            } => self.write_entity(*entity, *field, *value, *span, output),
            FlowOp::Call {
                dst,
                function,
                arguments,
                argument_places,
                current_lifecycle,
                span,
                ..
            } => output.push(Instruction::Call {
                dst: dst.map(|value| self.registers.value(value)),
                function: *function,
                arguments: arguments
                    .iter()
                    .map(|(parameter, value)| (*parameter, self.registers.value(*value)))
                    .collect(),
                argument_sources: argument_places
                    .iter()
                    .map(|(parameter, place)| {
                        (
                            *parameter,
                            place.as_ref().map(|place| ArgumentSource {
                                base: self.registers.local(place.base),
                                fields: place.fields.clone(),
                            }),
                        )
                    })
                    .collect(),
                current_lifecycle: self.registers.lifecycle(*current_lifecycle),
                span: *span,
            }),
            FlowOp::BeginCall { .. } | FlowOp::ReserveArgument { .. } => {}
            FlowOp::Keep {
                entity,
                target,
                span,
            } => output.push(Instruction::KeepEntity {
                entity: self.registers.value(*entity),
                lifecycle: self.registers.lifecycle(*target),
                span: *span,
            }),
            FlowOp::Retire { entity, span } => output.push(Instruction::RetireEntity {
                entity: self.registers.value(*entity),
                span: *span,
            }),
            FlowOp::ReadUncheckedLinkField { .. } => {
                unreachable!("verified flow cannot contain an unchecked link read")
            }
            FlowOp::ConstInt { .. }
            | FlowOp::ConstBool { .. }
            | FlowOp::ConstText { .. }
            | FlowOp::ConstNoneLink { .. }
            | FlowOp::BeginLifecycle { .. }
            | FlowOp::CopyLocal { .. }
            | FlowOp::StoreLocal { .. }
            | FlowOp::TakeLocal { .. }
            | FlowOp::CopyStorage { .. }
            | FlowOp::ListNew { .. }
            | FlowOp::ListLength { .. }
            | FlowOp::ListPush { .. }
            | FlowOp::ListPushPlace { .. }
            | FlowOp::ListLengthLocal { .. }
            | FlowOp::ListPushLocal { .. }
            | FlowOp::ListRemove { .. }
            | FlowOp::ListRemovePlace { .. }
            | FlowOp::ListRemoveLocal { .. }
            | FlowOp::TextByteLength { .. }
            | FlowOp::TextIsEmpty { .. }
            | FlowOp::TextConcat { .. }
            | FlowOp::UnaryInt { .. }
            | FlowOp::BinaryInt { .. }
            | FlowOp::Not { .. }
            | FlowOp::Compare { .. }
            | FlowOp::Phi { .. }
            | FlowOp::ConstructStruct { .. }
            | FlowOp::AllocateEntity { .. }
            | FlowOp::EntityToLink { .. }
            | FlowOp::ReadStructField { .. } => {
                unreachable!("non-entity flow operation handled before entity_operation")
            }
        }
    }

    fn fields(
        &self,
        fields: &[(keld_semantics::FieldId, ValueId)],
    ) -> Vec<(keld_semantics::FieldId, Register)> {
        fields
            .iter()
            .map(|(field, value)| (*field, self.registers.value(*value)))
            .collect()
    }

    fn read_entity(
        &mut self,
        dst: ValueId,
        entity: ValueId,
        field: keld_semantics::FieldId,
        span: keld_source::Span,
        output: &mut Vec<Instruction>,
    ) {
        let view = self.new_view();
        output.push(Instruction::OpenView {
            view,
            entity: self.registers.value(entity),
            mode: ViewMode::Read,
            span,
        });
        output.push(Instruction::ReadField {
            dst: self.registers.value(dst),
            view,
            field,
            span,
        });
        output.push(Instruction::CloseView { view, span });
    }

    fn write_entity(
        &mut self,
        entity: ValueId,
        field: keld_semantics::FieldId,
        value: ValueId,
        span: keld_source::Span,
        output: &mut Vec<Instruction>,
    ) {
        let view = self.new_view();
        output.push(Instruction::OpenView {
            view,
            entity: self.registers.value(entity),
            mode: ViewMode::Edit,
            span,
        });
        output.push(Instruction::WriteField {
            view,
            field,
            value: self.registers.value(value),
            span,
        });
        output.push(Instruction::CloseView { view, span });
    }

    fn new_view(&mut self) -> ViewId {
        let view = ViewId(self.next_view);
        self.next_view = self.next_view.saturating_add(1);
        view
    }

    fn is_home(&self, register: Register) -> bool {
        matches!(
            self.registers.storage(register),
            Some(RegisterStorage::Home { .. })
        )
    }

    fn home_register(&self, home: HomeId) -> Register {
        match home {
            HomeId::Local(local) => self.registers.local(local),
            HomeId::Temporary(value) => self.registers.value(value),
        }
    }

    fn emit_cleanup_actions(&self, actions: &[CleanupAction], output: &mut Vec<Instruction>) {
        for action in actions {
            match action {
                CleanupAction::Drop(home) => output.push(Instruction::DropHome {
                    home: self.home_register(*home),
                    span: self.function.span,
                }),
                CleanupAction::DropIfLive(home) => output.push(Instruction::DropIfLive {
                    home: self.home_register(*home),
                    span: self.function.span,
                }),
                CleanupAction::CleanupTrackedScope(scope) => {
                    output.push(Instruction::CleanupTrackedScope {
                        scope: *scope,
                        span: self.function.span,
                    });
                }
            }
        }
    }

    fn terminator(
        &self,
        block: keld_flow::BlockId,
        terminator: &keld_flow::Terminator,
        block_plan: Option<&keld_storage::BlockStoragePlan>,
        output: &mut Vec<Instruction>,
    ) -> Terminator {
        match terminator {
            keld_flow::Terminator::Goto(target) => Terminator::Goto(IrBlockId(target.0)),
            keld_flow::Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => Terminator::Branch {
                condition: self.registers.value(*condition),
                then_block: IrBlockId(then_block.0),
                else_block: IrBlockId(else_block.0),
            },
            keld_flow::Terminator::BranchIdentity {
                lhs,
                rhs,
                equal,
                not_equal,
            } => {
                let condition = self.registers.identity_conditions[&block];
                output.push(Instruction::Compare {
                    dst: condition,
                    op: CompareOp::Eq,
                    lhs: self.registers.value(*lhs),
                    rhs: self.registers.value(*rhs),
                    span: self.function.span,
                });
                Terminator::Branch {
                    condition,
                    then_block: IrBlockId(equal.0),
                    else_block: IrBlockId(not_equal.0),
                }
            }
            keld_flow::Terminator::ResolveLink {
                link,
                bind_local,
                live,
                absent,
                span,
            } => Terminator::ResolveLink {
                link: self.registers.value(*link),
                live_value: self.registers.local(*bind_local),
                live: IrBlockId(live.0),
                absent: IrBlockId(absent.0),
                span: *span,
            },
            keld_flow::Terminator::ExitScopes {
                lifecycles, next, ..
            } => {
                if let Some(block_plan) = block_plan {
                    self.emit_cleanup_actions(&block_plan.exit, output);
                }
                for lifecycle in lifecycles {
                    output.push(Instruction::EndLifecycle {
                        lifecycle: self.registers.lifecycle(*lifecycle),
                        span: self.function.span,
                    });
                }
                match next {
                    ExitTarget::Goto(target) => Terminator::Goto(IrBlockId(target.0)),
                    ExitTarget::Return(value) => {
                        Terminator::Return(value.map(|value| self.registers.value(value)))
                    }
                }
            }
            keld_flow::Terminator::Return(value) => {
                Terminator::Return(value.map(|value| self.registers.value(value)))
            }
            keld_flow::Terminator::Unreachable => Terminator::Unreachable,
        }
    }
}

fn map_type(types: &TypeStore, ty: TypeId) -> IrType {
    match types.kind(ty) {
        TypeKind::Unit => IrType::Unit,
        TypeKind::Bool => IrType::Bool,
        TypeKind::Int => IrType::Int,
        TypeKind::Struct(definition) => IrType::Struct(*definition),
        TypeKind::EntityRef(definition) => IrType::Entity(*definition),
        TypeKind::Link { entity, optional } => IrType::Link {
            entity: *entity,
            optional: *optional,
        },
        TypeKind::Text => IrType::Text,
        TypeKind::List(element) => IrType::List(Box::new(map_type(types, *element))),
        TypeKind::Optional(_) | TypeKind::Error => {
            unreachable!("verified bootstrap flow contains only executable types")
        }
    }
}

fn value_register_storage(plan: &FunctionStoragePlan, value: ValueId) -> RegisterStorage {
    match plan
        .values
        .get(value.0 as usize)
        .unwrap_or(&ValueStorage::Trivial)
    {
        ValueStorage::Trivial => RegisterStorage::Trivial,
        ValueStorage::EntityFlow => RegisterStorage::EntityFlow,
        ValueStorage::Loan(_) => RegisterStorage::Loan,
        ValueStorage::OwnedTemporary { scope } => RegisterStorage::Home {
            scope: *scope,
            conditional: plan.drop_flags.contains(&HomeId::Temporary(value)),
        },
    }
}

fn local_register_storage(plan: &FunctionStoragePlan, local: LocalId) -> RegisterStorage {
    match plan
        .locals
        .get(local.0 as usize)
        .unwrap_or(&LocalStorage::Trivial)
    {
        LocalStorage::Trivial => RegisterStorage::Trivial,
        LocalStorage::Loan => RegisterStorage::Loan,
        LocalStorage::Home { scope } => RegisterStorage::Home {
            scope: *scope,
            conditional: plan.drop_flags.contains(&HomeId::Local(local)),
        },
    }
}
