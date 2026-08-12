use crate::fault::{InterpreterError, InterpreterFailure, RuntimeFault, RuntimeFaultKind};
use crate::frame::{ActiveView, Frame};
use crate::value::{CopyAllocation, EntityPayload, Value, try_copy_value};
use keld_ir::{
    FaultKind, Function, Instruction, IrBlockId, Module, Register, Terminator, ViewMode,
};
use keld_numeric::{NumericFault, eval_binary, eval_unary};
use keld_runtime::{RuntimeLifecycleId, RuntimeTypeId, Store, StoreError};
use keld_semantics::{CompareOp, DefId, FieldId, ParameterIndex};
use keld_source::Span;

#[derive(Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub value: Value,
}

pub struct Interpreter<'module> {
    module: &'module Module,
    store: Store<EntityPayload>,
    frames: Vec<Frame>,
    started: bool,
}

impl<'module> Interpreter<'module> {
    /// Creates an interpreter after validating the executable IR.
    ///
    /// # Errors
    ///
    /// Returns a static IR error, an internal store error, or an allocation
    /// fault associated with the main function span.
    pub fn new(module: &'module Module) -> Result<Self, InterpreterFailure> {
        let diagnostics = keld_ir::validate(module);
        if !diagnostics.is_empty() {
            return Err(InterpreterFailure::Internal(InterpreterError::InvalidIr(
                diagnostics,
            )));
        }
        let span = main_span(module);
        let store = Store::new().map_err(|error| store_failure(error, span))?;
        let mut frames = Vec::new();
        frames
            .try_reserve(module.functions.len().max(1))
            .map_err(|_| allocation_failure(span))?;
        Ok(Self {
            module,
            store,
            frames,
            started: false,
        })
    }

    /// Runs the validated zero-argument main function once.
    ///
    /// # Errors
    ///
    /// Returns a Keld runtime fault or an internal invariant error.
    pub fn run_main(&mut self) -> Result<ExecutionResult, InterpreterFailure> {
        if self.started {
            return Err(internal("interpreter is already executing"));
        }
        self.started = true;
        let main = self
            .module
            .functions
            .get(self.module.main.0 as usize)
            .ok_or_else(|| internal("validated main function is missing"))?;
        let root = self.store.root_lifecycle();
        let frame = build_frame(main, Vec::new(), root, None, main.span)?;
        self.frames.push(frame);
        enter_block(self.module, &mut self.frames, main.entry, None, main.span)?;
        loop {
            if let Some(value) = self.step()? {
                self.store
                    .finish_with(drop)
                    .map_err(|error| store_failure(error, main.span))?;
                return Ok(ExecutionResult { value });
            }
        }
    }

    fn step(&mut self) -> Result<Option<Value>, InterpreterFailure> {
        let (function_index, block_index, instruction_index) = {
            let frame = self
                .frames
                .last()
                .ok_or_else(|| internal("execution has no active frame"))?;
            (
                frame.function.0 as usize,
                frame.block.0 as usize,
                frame.instruction,
            )
        };
        let function = self
            .module
            .functions
            .get(function_index)
            .ok_or_else(|| internal("active frame references an unknown function"))?;
        let block = function
            .blocks
            .get(block_index)
            .ok_or_else(|| internal("active frame references an unknown block"))?;
        if let Some(instruction) = block.instructions.get(instruction_index) {
            execute_instruction(self.module, &mut self.store, &mut self.frames, instruction)?;
            Ok(None)
        } else {
            execute_terminator(
                self.module,
                &mut self.store,
                &mut self.frames,
                &block.terminator,
                function.span,
            )
        }
    }
}

/// Compiles and executes source used by interpreter integration tests.
///
/// # Errors
///
/// Returns a Keld runtime fault produced by otherwise valid test source.
///
/// # Panics
///
/// Panics if the supplied source has a static diagnostic or triggers an
/// internal compiler/interpreter invariant. Tests should pass valid source.
pub fn run_text_for_test(text: &str) -> Result<ExecutionResult, RuntimeFault> {
    let flow = keld_flow::lower_text_for_test(text)
        .unwrap_or_else(|diagnostics| panic!("test source failed analysis: {diagnostics:#?}"));
    let verification = keld_lifecycle::verify(flow);
    let verified = verification.module.unwrap_or_else(|| {
        panic!(
            "test source failed verification: {:#?}",
            verification.diagnostics
        )
    });
    let ir = keld_ir::lower(&verified);
    let mut interpreter = match Interpreter::new(&ir) {
        Ok(interpreter) => interpreter,
        Err(InterpreterFailure::Runtime(fault)) => return Err(fault),
        Err(InterpreterFailure::Internal(error)) => panic!("interpreter setup failed: {error}"),
    };
    match interpreter.run_main() {
        Ok(result) => Ok(result),
        Err(InterpreterFailure::Runtime(fault)) => Err(fault),
        Err(InterpreterFailure::Internal(error)) => panic!("interpreter execution failed: {error}"),
    }
}

fn execute_instruction(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::ConstInt { dst, value, .. } => {
            set_register(frames, *dst, Value::Int(*value))?;
        }
        Instruction::ConstBool { dst, value, .. } => {
            set_register(frames, *dst, Value::Bool(*value))?;
        }
        Instruction::ConstNoneLink { dst, .. } => {
            set_register(frames, *dst, Value::Link(None))?;
        }
        Instruction::Copy { dst, src, .. } => {
            let value =
                try_copy_value(frame_value(frames, *src)?).map_err(|_| allocation_failure(span))?;
            set_register(frames, *dst, value)?;
        }
        Instruction::CheckedUnaryInt { dst, op, src, .. } => {
            let value = expect_int(frame_value(frames, *src)?)?;
            let result = eval_unary(*op, value).map_err(|fault| numeric_failure(fault, span))?;
            set_register(frames, *dst, Value::Int(result))?;
        }
        Instruction::CheckedBinaryInt {
            dst, op, lhs, rhs, ..
        } => {
            let lhs = expect_int(frame_value(frames, *lhs)?)?;
            let rhs = expect_int(frame_value(frames, *rhs)?)?;
            let result =
                eval_binary(*op, lhs, rhs).map_err(|fault| numeric_failure(fault, span))?;
            set_register(frames, *dst, Value::Int(result))?;
        }
        Instruction::Not { dst, src, .. } => {
            let value = expect_bool(frame_value(frames, *src)?)?;
            set_register(frames, *dst, Value::Bool(!value))?;
        }
        Instruction::Compare {
            dst, op, lhs, rhs, ..
        } => {
            let result =
                compare_values(*op, frame_value(frames, *lhs)?, frame_value(frames, *rhs)?)?;
            set_register(frames, *dst, Value::Bool(result))?;
        }
        Instruction::Phi { .. } => {
            return Err(internal("Phi executed outside block-entry processing"));
        }
        Instruction::ConstructStruct {
            dst,
            definition,
            fields,
            ..
        } => {
            let values = materialize_fields(module, frames, *definition, fields, span)?;
            set_register(
                frames,
                *dst,
                Value::Struct {
                    definition: *definition,
                    fields: values,
                },
            )?;
        }
        Instruction::ReadStructField {
            dst, base, field, ..
        } => {
            let Value::Struct { definition, fields } = frame_value(frames, *base)? else {
                return Err(internal("validated struct read received a non-struct"));
            };
            let field_index = field_index(module, *definition, *field)?;
            let value = fields
                .get(field_index)
                .ok_or_else(|| internal("struct payload does not match its definition"))?;
            let value = try_copy_value(value).map_err(|_| allocation_failure(span))?;
            set_register(frames, *dst, value)?;
        }
        _ => return execute_effect_instruction(module, store, frames, instruction),
    }
    advance(frames)?;
    Ok(())
}

fn execute_effect_instruction(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::BeginLifecycle { dst, parent, .. } => {
            let parent = expect_lifecycle(frame_value(frames, *parent)?)?;
            let lifecycle = store
                .begin_lifecycle(parent)
                .map_err(|error| store_failure(error, span))?;
            set_register(frames, *dst, Value::Lifecycle(lifecycle))?;
        }
        Instruction::EndLifecycle { lifecycle, .. } => {
            let lifecycle = expect_lifecycle(frame_value(frames, *lifecycle)?)?;
            store
                .end_lifecycle_with(lifecycle, drop)
                .map_err(|error| store_failure(error, span))?;
        }
        Instruction::AllocateEntity {
            dst,
            definition,
            fields,
            lifecycle,
            ..
        } => {
            let lifecycle = expect_lifecycle(frame_value(frames, *lifecycle)?)?;
            let values = materialize_fields(module, frames, *definition, fields, span)?;
            let entity = store
                .allocate(
                    RuntimeTypeId(definition.0),
                    lifecycle,
                    EntityPayload {
                        definition: *definition,
                        fields: values,
                    },
                )
                .map_err(|error| store_failure(error, span))?;
            set_register(frames, *dst, Value::Entity(entity))?;
        }
        Instruction::EntityToLink { dst, entity, .. } => {
            let entity = expect_entity(frame_value(frames, *entity)?)?;
            let link = store
                .link(entity)
                .map_err(|error| store_failure(error, span))?;
            set_register(frames, *dst, Value::Link(Some(link)))?;
        }
        _ => return execute_view_instruction(module, store, frames, instruction),
    }
    advance(frames)?;
    Ok(())
}

fn execute_view_instruction(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::OpenView {
            view, entity, mode, ..
        } => {
            let entity = expect_entity(frame_value(frames, *entity)?)?;
            let frame = frames
                .last_mut()
                .ok_or_else(|| internal("view open has no active frame"))?;
            let destination = frame
                .views
                .get_mut(view.0 as usize)
                .ok_or_else(|| internal("validated view ID is outside the frame"))?;
            if destination.is_some() {
                return Err(internal("validated view ID is already open"));
            }
            *destination = Some(ActiveView {
                entity,
                mode: *mode,
            });
        }
        Instruction::ReadField {
            dst, view, field, ..
        } => {
            let active = active_view(frames, *view)?;
            let value = store
                .read(active.entity, |payload| {
                    let index = field_index(module, payload.definition, *field)?;
                    let field = payload
                        .fields
                        .get(index)
                        .ok_or_else(|| internal("entity payload layout is invalid"))?;
                    try_copy_value(field).map_err(|_| allocation_failure(span))
                })
                .map_err(|error| store_failure(error, span))??;
            set_register(frames, *dst, value)?;
        }
        Instruction::WriteField {
            view, field, value, ..
        } => {
            let active = active_view(frames, *view)?;
            if active.mode != ViewMode::Edit {
                return Err(internal("validated write used a read view"));
            }
            let value = try_copy_value(frame_value(frames, *value)?)
                .map_err(|_| allocation_failure(span))?;
            store
                .edit(active.entity, |payload| {
                    let index = field_index(module, payload.definition, *field)?;
                    let destination = payload
                        .fields
                        .get_mut(index)
                        .ok_or_else(|| internal("entity payload layout is invalid"))?;
                    *destination = value;
                    Ok::<(), InterpreterFailure>(())
                })
                .map_err(|error| store_failure(error, span))??;
        }
        Instruction::CloseView { view, .. } => {
            let frame = frames
                .last_mut()
                .ok_or_else(|| internal("view close has no active frame"))?;
            let active = frame
                .views
                .get_mut(view.0 as usize)
                .ok_or_else(|| internal("validated view ID is outside the frame"))?;
            if active.take().is_none() {
                return Err(internal("validated view is not open"));
            }
        }
        _ => return execute_call_or_retirement(module, store, frames, instruction),
    }
    advance(frames)?;
    Ok(())
}

fn execute_call_or_retirement(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::KeepEntity {
            entity, lifecycle, ..
        } => {
            let entity = expect_entity(frame_value(frames, *entity)?)?;
            let lifecycle = expect_lifecycle(frame_value(frames, *lifecycle)?)?;
            store
                .keep(entity, lifecycle)
                .map_err(|error| store_failure(error, span))?;
        }
        Instruction::RetireEntity { entity, .. } => {
            let entity = expect_entity(frame_value(frames, *entity)?)?;
            store
                .retire_with(entity, drop)
                .map_err(|error| store_failure(error, span))?;
        }
        Instruction::Call {
            dst,
            function,
            arguments,
            current_lifecycle,
            ..
        } => {
            return execute_call(
                module,
                frames,
                *dst,
                *function,
                arguments,
                *current_lifecycle,
                span,
            );
        }
        Instruction::ConstInt { .. }
        | Instruction::ConstBool { .. }
        | Instruction::ConstNoneLink { .. }
        | Instruction::Copy { .. }
        | Instruction::CheckedUnaryInt { .. }
        | Instruction::CheckedBinaryInt { .. }
        | Instruction::Not { .. }
        | Instruction::Compare { .. }
        | Instruction::Phi { .. }
        | Instruction::ConstructStruct { .. }
        | Instruction::ReadStructField { .. }
        | Instruction::BeginLifecycle { .. }
        | Instruction::EndLifecycle { .. }
        | Instruction::AllocateEntity { .. }
        | Instruction::EntityToLink { .. }
        | Instruction::OpenView { .. }
        | Instruction::ReadField { .. }
        | Instruction::WriteField { .. }
        | Instruction::CloseView { .. } => {
            return Err(internal("value instruction reached effect dispatch"));
        }
    }
    advance(frames)?;
    Ok(())
}

fn execute_call(
    module: &Module,
    frames: &mut Vec<Frame>,
    destination: Option<Register>,
    function: keld_semantics::FunctionId,
    arguments: &[(ParameterIndex, Register)],
    current_lifecycle: Register,
    span: Span,
) -> Result<(), InterpreterFailure> {
    let mut materialized = Vec::new();
    materialized
        .try_reserve_exact(arguments.len())
        .map_err(|_| allocation_failure(span))?;
    for (parameter, register) in arguments {
        let value = try_copy_value(frame_value(frames, *register)?)
            .map_err(|_| allocation_failure(span))?;
        materialized.push((*parameter, value));
    }
    let lifecycle = expect_lifecycle(frame_value(frames, current_lifecycle)?)?;
    let callee = module
        .functions
        .get(function.0 as usize)
        .ok_or_else(|| internal("validated call target is missing"))?;
    advance(frames)?;
    let frame = build_frame(callee, materialized, lifecycle, destination, span)?;
    frames
        .try_reserve(1)
        .map_err(|_| allocation_failure(span))?;
    frames.push(frame);
    enter_block(module, frames, callee.entry, None, span)
}

fn build_frame(
    function: &Function,
    arguments: Vec<(ParameterIndex, Value)>,
    current_lifecycle: RuntimeLifecycleId,
    return_destination: Option<Register>,
    span: Span,
) -> Result<Frame, InterpreterFailure> {
    let mut frame = Frame::new(function, return_destination)
        .map_err(|CopyAllocation| allocation_failure(span))?;
    frame
        .set(
            function.current_lifecycle,
            Value::Lifecycle(current_lifecycle),
        )
        .map_err(|()| internal("current lifecycle register is outside the frame"))?;
    for (parameter, value) in arguments {
        let register = function
            .parameters
            .get(parameter.0 as usize)
            .copied()
            .ok_or_else(|| internal("validated call parameter is missing"))?;
        frame
            .set(register, value)
            .map_err(|()| internal("parameter register is outside the frame"))?;
    }
    Ok(frame)
}

fn execute_terminator(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    terminator: &Terminator,
    fallback_span: Span,
) -> Result<Option<Value>, InterpreterFailure> {
    let predecessor = frames
        .last()
        .map(|frame| frame.block)
        .ok_or_else(|| internal("terminator has no active frame"))?;
    match terminator {
        Terminator::Goto(target) => {
            enter_block(module, frames, *target, Some(predecessor), fallback_span)?;
            Ok(None)
        }
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            let condition = expect_bool(frame_value(frames, *condition)?)?;
            let target = if condition { *then_block } else { *else_block };
            enter_block(module, frames, target, Some(predecessor), fallback_span)?;
            Ok(None)
        }
        Terminator::ResolveLink {
            link,
            live_value,
            live,
            absent,
            span,
        } => {
            let link = expect_link(frame_value(frames, *link)?)?;
            if let Some(entity) = link.and_then(|link| store.resolve(link)) {
                set_register(frames, *live_value, Value::Entity(entity))?;
                enter_block(module, frames, *live, Some(predecessor), *span)?;
            } else {
                enter_block(module, frames, *absent, Some(predecessor), *span)?;
            }
            Ok(None)
        }
        Terminator::Return(register) => return_from_frame(module, frames, *register, fallback_span),
        Terminator::Fault { kind, span } => Err(InterpreterFailure::Runtime(RuntimeFault {
            kind: match kind {
                FaultKind::Arithmetic => RuntimeFaultKind::Arithmetic,
                FaultKind::DivisionByZero => RuntimeFaultKind::DivisionByZero,
                FaultKind::Shift => RuntimeFaultKind::Shift,
                FaultKind::Allocation => RuntimeFaultKind::Allocation,
            },
            span: *span,
        })),
        Terminator::Unreachable => Err(internal("execution reached an unreachable terminator")),
    }
}

fn return_from_frame(
    _module: &Module,
    frames: &mut Vec<Frame>,
    register: Option<Register>,
    _span: Span,
) -> Result<Option<Value>, InterpreterFailure> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("return has no active frame"))?;
    let value = if let Some(register) = register {
        frame
            .registers
            .get_mut(register.0 as usize)
            .and_then(Option::take)
            .ok_or_else(|| internal("validated return register is undefined"))?
    } else {
        Value::Unit
    };
    let destination = frame.return_destination;
    frames.pop();
    let Some(caller) = frames.last_mut() else {
        return Ok(Some(value));
    };
    if let Some(destination) = destination {
        caller
            .set(destination, value)
            .map_err(|()| internal("call destination is outside the caller frame"))?;
    }
    Ok(None)
}

fn enter_block(
    module: &Module,
    frames: &mut [Frame],
    target: IrBlockId,
    predecessor: Option<IrBlockId>,
    span: Span,
) -> Result<(), InterpreterFailure> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("block entry has no active frame"))?;
    let function = module
        .functions
        .get(frame.function.0 as usize)
        .ok_or_else(|| internal("active function is missing"))?;
    let block = function
        .blocks
        .get(target.0 as usize)
        .ok_or_else(|| internal("branch target is missing"))?;
    let phi_count = block
        .instructions
        .iter()
        .take_while(|instruction| matches!(instruction, Instruction::Phi { .. }))
        .count();
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(phi_count)
        .map_err(|_| allocation_failure(span))?;
    for instruction in block.instructions.iter().take(phi_count) {
        let Instruction::Phi { dst, inputs, .. } = instruction else {
            return Err(internal("Phi group contains a non-Phi instruction"));
        };
        let predecessor = predecessor.ok_or_else(|| internal("entry block contains a Phi"))?;
        let source = inputs
            .iter()
            .find_map(|(block, register)| (*block == predecessor).then_some(*register))
            .ok_or_else(|| internal("Phi has no input for the predecessor"))?;
        let value = frame
            .value(source)
            .ok_or_else(|| internal("Phi input register is undefined"))?;
        pending.push((
            *dst,
            try_copy_value(value).map_err(|_| allocation_failure(span))?,
        ));
    }
    frame.block = target;
    frame.predecessor = predecessor;
    frame.instruction = phi_count;
    for (destination, value) in pending {
        frame
            .set(destination, value)
            .map_err(|()| internal("Phi destination is outside the frame"))?;
    }
    Ok(())
}

fn materialize_fields(
    module: &Module,
    frames: &[Frame],
    definition: DefId,
    fields: &[(FieldId, Register)],
    span: Span,
) -> Result<Vec<Value>, InterpreterFailure> {
    let layout = module
        .definitions
        .get(definition.0 as usize)
        .filter(|candidate| candidate.id == definition)
        .ok_or_else(|| internal("validated constructor definition is missing"))?;
    let mut source_values = Vec::new();
    source_values
        .try_reserve_exact(fields.len())
        .map_err(|_| allocation_failure(span))?;
    for (field, register) in fields {
        source_values.push((
            *field,
            Some(
                try_copy_value(frame_value(frames, *register)?)
                    .map_err(|_| allocation_failure(span))?,
            ),
        ));
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(layout.fields.len())
        .map_err(|_| allocation_failure(span))?;
    for (field, _) in &layout.fields {
        let value = source_values
            .iter_mut()
            .find_map(|(candidate, value)| (*candidate == *field).then_some(value))
            .and_then(Option::take)
            .ok_or_else(|| internal("validated constructor field is missing"))?;
        result.push(value);
    }
    Ok(result)
}

fn field_index(
    module: &Module,
    definition: DefId,
    field: FieldId,
) -> Result<usize, InterpreterFailure> {
    module
        .definitions
        .get(definition.0 as usize)
        .filter(|candidate| candidate.id == definition)
        .and_then(|definition| {
            definition
                .fields
                .iter()
                .position(|(candidate, _)| *candidate == field)
        })
        .ok_or_else(|| internal("validated field layout is missing"))
}

fn frame_value(frames: &[Frame], register: Register) -> Result<&Value, InterpreterFailure> {
    frames
        .last()
        .and_then(|frame| frame.value(register))
        .ok_or_else(|| internal("validated register is undefined"))
}

fn set_register(
    frames: &mut [Frame],
    register: Register,
    value: Value,
) -> Result<(), InterpreterFailure> {
    frames
        .last_mut()
        .ok_or_else(|| internal("register write has no active frame"))?
        .set(register, value)
        .map_err(|()| internal("validated destination is outside the frame"))
}

fn active_view(frames: &[Frame], view: keld_ir::ViewId) -> Result<ActiveView, InterpreterFailure> {
    frames
        .last()
        .and_then(|frame| frame.view(view))
        .ok_or_else(|| internal("validated field view is not active"))
}

fn advance(frames: &mut [Frame]) -> Result<(), InterpreterFailure> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("instruction has no active frame"))?;
    frame.instruction = frame
        .instruction
        .checked_add(1)
        .ok_or_else(|| internal("instruction index overflow"))?;
    Ok(())
}

fn expect_int(value: &Value) -> Result<i64, InterpreterFailure> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => Err(internal(
            "validated Int register contains another value kind",
        )),
    }
}

fn expect_bool(value: &Value) -> Result<bool, InterpreterFailure> {
    match value {
        Value::Bool(value) => Ok(*value),
        _ => Err(internal(
            "validated Bool register contains another value kind",
        )),
    }
}

fn expect_entity(value: &Value) -> Result<keld_runtime::EntityId, InterpreterFailure> {
    match value {
        Value::Entity(entity) => Ok(*entity),
        _ => Err(internal(
            "validated entity register contains another value kind",
        )),
    }
}

fn expect_link(value: &Value) -> Result<Option<keld_runtime::Link>, InterpreterFailure> {
    match value {
        Value::Link(link) => Ok(*link),
        _ => Err(internal(
            "validated link register contains another value kind",
        )),
    }
}

fn expect_lifecycle(value: &Value) -> Result<RuntimeLifecycleId, InterpreterFailure> {
    match value {
        Value::Lifecycle(lifecycle) => Ok(*lifecycle),
        _ => Err(internal(
            "validated lifecycle register contains another value kind",
        )),
    }
}

fn compare_values(
    operation: CompareOp,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, InterpreterFailure> {
    let equality = || match (lhs, rhs) {
        (Value::Int(lhs), Value::Int(rhs)) => Ok(lhs == rhs),
        (Value::Bool(lhs), Value::Bool(rhs)) => Ok(lhs == rhs),
        (Value::Entity(lhs), Value::Entity(rhs)) => Ok(lhs == rhs),
        _ => Err(internal(
            "validated equality operands have invalid value kinds",
        )),
    };
    match operation {
        CompareOp::Eq => equality(),
        CompareOp::NotEq => equality().map(|equal| !equal),
        CompareOp::Less => Ok(expect_int(lhs)? < expect_int(rhs)?),
        CompareOp::LessEq => Ok(expect_int(lhs)? <= expect_int(rhs)?),
        CompareOp::Greater => Ok(expect_int(lhs)? > expect_int(rhs)?),
        CompareOp::GreaterEq => Ok(expect_int(lhs)? >= expect_int(rhs)?),
    }
}

fn numeric_failure(fault: NumericFault, span: Span) -> InterpreterFailure {
    InterpreterFailure::Runtime(RuntimeFault {
        kind: match fault {
            NumericFault::Arithmetic => RuntimeFaultKind::Arithmetic,
            NumericFault::DivisionByZero => RuntimeFaultKind::DivisionByZero,
            NumericFault::Shift => RuntimeFaultKind::Shift,
        },
        span,
    })
}

fn allocation_failure(span: Span) -> InterpreterFailure {
    InterpreterFailure::Runtime(RuntimeFault {
        kind: RuntimeFaultKind::Allocation,
        span,
    })
}

fn store_failure(error: StoreError, span: Span) -> InterpreterFailure {
    match error {
        StoreError::Allocation => allocation_failure(span),
        StoreError::BrandExhausted | StoreError::InvalidOperation(_) => {
            InterpreterFailure::Internal(InterpreterError::Store(error))
        }
    }
}

fn internal(message: &'static str) -> InterpreterFailure {
    InterpreterFailure::Internal(InterpreterError::InvalidState(message))
}

fn main_span(module: &Module) -> Span {
    module.functions.get(module.main.0 as usize).map_or_else(
        || {
            Span::new(keld_source::SourceId(0), 0, 0)
                .expect("empty interpreter fallback span is valid")
        },
        |function| function.span,
    )
}

fn instruction_span(instruction: &Instruction) -> Span {
    match instruction {
        Instruction::ConstInt { span, .. }
        | Instruction::ConstBool { span, .. }
        | Instruction::ConstNoneLink { span, .. }
        | Instruction::Copy { span, .. }
        | Instruction::CheckedUnaryInt { span, .. }
        | Instruction::CheckedBinaryInt { span, .. }
        | Instruction::Not { span, .. }
        | Instruction::Compare { span, .. }
        | Instruction::Phi { span, .. }
        | Instruction::ConstructStruct { span, .. }
        | Instruction::ReadStructField { span, .. }
        | Instruction::BeginLifecycle { span, .. }
        | Instruction::EndLifecycle { span, .. }
        | Instruction::AllocateEntity { span, .. }
        | Instruction::EntityToLink { span, .. }
        | Instruction::OpenView { span, .. }
        | Instruction::ReadField { span, .. }
        | Instruction::WriteField { span, .. }
        | Instruction::CloseView { span, .. }
        | Instruction::KeepEntity { span, .. }
        | Instruction::RetireEntity { span, .. }
        | Instruction::Call { span, .. } => *span,
    }
}
