use crate::cleanup::{CleanupPath, CleanupTrace, ExecutionTrace, cleanup_payload, cleanup_value};
use crate::fault::{InterpreterError, InterpreterFailure, RuntimeFault, RuntimeFaultKind};
use crate::frame::{ActiveView, Frame};
use crate::place::{FrameId, RuntimePlace, RuntimePlaceRoot, RuntimeProjection};
use crate::value::RuntimeText;
use crate::value::{CopyAllocation, EntityPayload, Value, try_copy_value};
use crate::{AllocationController, ReserveFailure};
use keld_ir::{
    ArgumentProjection, ArgumentSource, FaultKind, Function, Instruction, IrBlockId, Module,
    Receiver, Register, Terminator, ViewMode,
};
use keld_numeric::{NumericFault, eval_binary, eval_unary};
use keld_runtime::{RuntimeLifecycleId, RuntimeTypeId, Store, StoreError};
use keld_semantics::{CompareOp, DefId, FieldId, ParameterIndex};
use keld_source::Span;

#[derive(Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub value: Value,
}

#[derive(Clone, Debug, Default)]
pub struct TestControls {
    fail_structural_copies: usize,
    allocations: AllocationController,
}

impl TestControls {
    #[must_use]
    pub fn fail_structural_copy(count: usize) -> Self {
        Self {
            fail_structural_copies: count,
            allocations: AllocationController::default(),
        }
    }

    #[must_use]
    pub fn fail_list_attempts(attempts: impl IntoIterator<Item = u64>) -> Self {
        Self {
            fail_structural_copies: 0,
            allocations: AllocationController::fail_list_attempts(attempts),
        }
    }
}

pub struct Interpreter<'module> {
    module: &'module Module,
    store: Store<EntityPayload>,
    frames: Vec<Frame>,
    started: bool,
    controls: TestControls,
    cleanup_trace: Option<CleanupTrace>,
}

impl<'module> Interpreter<'module> {
    /// Creates an interpreter after validating the executable IR.
    ///
    /// # Errors
    ///
    /// Returns a static IR error, an internal store error, or an allocation
    /// fault associated with the main function span.
    pub fn new(module: &'module Module) -> Result<Self, InterpreterFailure> {
        Self::with_options(module, TestControls::default(), false)
    }

    fn with_controls(
        module: &'module Module,
        controls: TestControls,
    ) -> Result<Self, InterpreterFailure> {
        Self::with_options(module, controls, false)
    }

    fn with_trace(module: &'module Module) -> Result<Self, InterpreterFailure> {
        Self::with_options(module, TestControls::default(), true)
    }

    fn with_options(
        module: &'module Module,
        controls: TestControls,
        trace_cleanup: bool,
    ) -> Result<Self, InterpreterFailure> {
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
            controls,
            cleanup_trace: trace_cleanup.then(CleanupTrace::default),
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
        let frame = build_frame(main, Vec::new(), Vec::new(), root, None, main.span)?;
        self.frames.push(frame);
        enter_block(
            self.module,
            &self.store,
            &mut self.frames,
            main.entry,
            None,
            main.span,
            &mut self.controls,
        )?;
        loop {
            if let Some(value) = self.step()? {
                let module = self.module;
                let cleanup_trace = &mut self.cleanup_trace;
                self.store
                    .finish_with(|payload| {
                        cleanup_payload(module, payload, cleanup_trace.as_mut());
                    })
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
            execute_instruction(
                self.module,
                &mut self.store,
                &mut self.frames,
                instruction,
                &mut self.controls,
                &mut self.cleanup_trace,
            )?;
            Ok(None)
        } else {
            execute_terminator(
                self.module,
                &mut self.store,
                &mut self.frames,
                &block.terminator,
                function.span,
                &mut self.controls,
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
    run_text_with_controls_for_test(text, TestControls::default())
}

/// Compiles and executes source with deterministic test-only allocation controls.
///
/// # Errors
///
/// Returns a Keld runtime fault produced by otherwise valid test source.
///
/// # Panics
///
/// Panics when the test source fails static analysis, verification, or an
/// internal compiler/interpreter invariant is violated.
pub fn run_text_with_controls_for_test(
    text: &str,
    controls: TestControls,
) -> Result<ExecutionResult, RuntimeFault> {
    let ir = compile_text_for_test(text);
    let mut interpreter = match Interpreter::with_controls(&ir, controls) {
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

/// Compiles and executes source while exposing the test-only cleanup event stream.
///
/// # Errors
///
/// Returns a Keld runtime fault produced by otherwise valid test source.
///
/// # Panics
///
/// Panics when the test source fails static analysis, verification, or an
/// internal compiler/interpreter invariant is violated.
pub fn trace_text_for_test(text: &str) -> Result<ExecutionTrace, RuntimeFault> {
    let ir = compile_text_for_test(text);
    let mut interpreter = match Interpreter::with_trace(&ir) {
        Ok(interpreter) => interpreter,
        Err(InterpreterFailure::Runtime(fault)) => return Err(fault),
        Err(InterpreterFailure::Internal(error)) => panic!("interpreter setup failed: {error}"),
    };
    let result = match interpreter.run_main() {
        Ok(result) => result,
        Err(InterpreterFailure::Runtime(fault)) => return Err(fault),
        Err(InterpreterFailure::Internal(error)) => panic!("interpreter execution failed: {error}"),
    };
    let cleanup = interpreter
        .cleanup_trace
        .take()
        .map_or_else(Vec::new, |trace| trace.events);
    Ok(ExecutionTrace { result, cleanup })
}

fn compile_text_for_test(text: &str) -> Module {
    let flow = keld_flow::lower_text_for_test(text)
        .unwrap_or_else(|diagnostics| panic!("test source failed analysis: {diagnostics:#?}"));
    let verification = keld_lifecycle::verify(flow);
    let verified = verification.module.unwrap_or_else(|| {
        panic!(
            "test source failed verification: {:#?}",
            verification.diagnostics
        )
    });
    let storage = keld_storage::verify(verified);
    let verified = storage.module.unwrap_or_else(|| {
        panic!(
            "test source failed storage verification: {:#?}",
            storage.diagnostics
        )
    });
    keld_ir::lower(&verified)
}

#[allow(clippy::too_many_lines)]
fn execute_instruction(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
    controls: &mut TestControls,
    cleanup_trace: &mut Option<CleanupTrace>,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::ConstInt { dst, value, .. } => {
            set_register(frames, *dst, Value::Int(*value))?;
        }
        Instruction::ConstBool { dst, value, .. } => {
            set_register(frames, *dst, Value::Bool(*value))?;
        }
        Instruction::ConstText { dst, value, .. } => {
            set_register(
                frames,
                *dst,
                Value::Text(RuntimeText::from_string(value.clone())),
            )?;
        }
        Instruction::ConstNoneLink { dst, .. } => {
            set_register(frames, *dst, Value::Link(None))?;
        }
        Instruction::Copy { dst, src, .. } => {
            if is_loan_register(module, frames, *dst) {
                let place = runtime_place_for_register(frames, *src);
                frames
                    .last_mut()
                    .ok_or_else(|| internal("loan copy has no active frame"))?
                    .set_loan(*dst, place)
                    .map_err(|()| internal("validated loan destination is not empty"))?;
            } else {
                let value = with_register_value(module, store, frames, *src, span, |value| {
                    structural_copy(value, span, controls)
                })?;
                set_register(frames, *dst, value)?;
            }
        }
        Instruction::Take { dst, src, .. } => {
            let value = take_register(frames, *src)?;
            set_register(frames, *dst, value)?;
        }
        Instruction::ListNew { dst, .. } => {
            set_register(frames, *dst, Value::List(crate::RuntimeList::new()))?;
        }
        Instruction::ListLength { dst, list, .. } => {
            let length = with_register_value(module, store, frames, *list, span, |value| {
                let Value::List(elements) = value else {
                    return Err(internal("validated list length received a non-list"));
                };
                i64::try_from(elements.length())
                    .map_err(|_| internal("validated List length exceeds Int"))
            })?;
            set_register(frames, *dst, Value::Int(length))?;
        }
        Instruction::ListPush { list, value, .. } => {
            let value = take_argument(module, store, frames, *value, span, controls)?;
            with_register_mut(module, store, frames, *list, span, |list_value| {
                let Value::List(elements) = list_value else {
                    return Err(internal("validated list push received a non-list"));
                };
                elements
                    .push(value, &mut controls.allocations)
                    .map_err(|failure| reserve_failure(failure, span))?;
                Ok(())
            })?;
        }
        Instruction::ListPushPlace { source, value, .. } => {
            let value = take_argument(module, store, frames, *value, span, controls)?;
            let place = runtime_place_for_source(frames, source, span)?;
            with_place_mut(module, store, frames, &place, span, |list_value| {
                let Value::List(elements) = list_value else {
                    return Err(internal("validated list push received a non-list"));
                };
                elements
                    .push(value, &mut controls.allocations)
                    .map_err(|failure| reserve_failure(failure, span))?;
                Ok(())
            })?;
        }
        Instruction::ListRemove {
            dst, list, index, ..
        } => {
            let index = with_register_value(module, store, frames, *index, span, |value| {
                usize::try_from(expect_int(value)?).map_err(|_| bounds_failure(span))
            })?;
            let removed = with_register_mut(module, store, frames, *list, span, |list_value| {
                let Value::List(elements) = list_value else {
                    return Err(internal("validated list remove received a non-list"));
                };
                if index >= elements.length() {
                    return Err(bounds_failure(span));
                }
                Ok(elements.remove(index))
            })?;
            set_register(frames, *dst, removed)?;
        }
        Instruction::ListRemovePlace {
            dst, source, index, ..
        } => {
            let index = with_register_value(module, store, frames, *index, span, |value| {
                usize::try_from(expect_int(value)?).map_err(|_| bounds_failure(span))
            })?;
            let place = runtime_place_for_source(frames, source, span)?;
            let removed = with_place_mut(module, store, frames, &place, span, |list_value| {
                let Value::List(elements) = list_value else {
                    return Err(internal("validated list remove received a non-list"));
                };
                if index >= elements.length() {
                    return Err(bounds_failure(span));
                }
                Ok(elements.remove(index))
            })?;
            set_register(frames, *dst, removed)?;
        }
        Instruction::ListIndex {
            dst,
            receiver,
            index,
            ..
        } => {
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            let Some(index) = checked_receiver_index(module, store, frames, &place, *index, span)?
            else {
                return Err(bounds_failure(span));
            };
            if is_loan_register(module, frames, *dst) {
                let loan = place.project(RuntimeProjection::Index(index));
                frames
                    .last_mut()
                    .ok_or_else(|| internal("indexed loan has no active frame"))?
                    .set_loan(*dst, loan)
                    .map_err(|()| internal("validated indexed loan destination is not empty"))?;
            } else {
                let value = with_place_value(module, store, frames, &place, span, |list| {
                    let Value::List(elements) = list else {
                        return Err(internal("validated list index received a non-list"));
                    };
                    let element = elements.get(index).ok_or_else(|| bounds_failure(span))?;
                    structural_copy(element, span, controls)
                })?;
                set_register(frames, *dst, value)?;
            }
        }
        Instruction::ListGet {
            dst,
            receiver,
            index,
            ..
        } => {
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            let value = match checked_receiver_index(module, store, frames, &place, *index, span)? {
                Some(index) => with_place_value(module, store, frames, &place, span, |list| {
                    let Value::List(elements) = list else {
                        return Err(internal("validated list get received a non-list"));
                    };
                    let element = elements.get(index).ok_or_else(|| {
                        internal("validated list get index changed during access")
                    })?;
                    structural_copy(element, span, controls).map(|value| Some(Box::new(value)))
                })?,
                None => None,
            };
            set_register(frames, *dst, Value::Optional(value))?;
        }
        Instruction::ListReplace {
            receiver,
            index,
            value,
            displaced,
            ..
        } => {
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            let Some(index) = checked_receiver_index(module, store, frames, &place, *index, span)?
            else {
                return Err(bounds_failure(span));
            };
            let incoming = take_argument(module, store, frames, *value, span, controls)?;
            let previous = with_place_mut(module, store, frames, &place, span, |list| {
                let Value::List(elements) = list else {
                    return Err(internal("validated list replacement received a non-list"));
                };
                let destination = elements
                    .get_mut(index)
                    .ok_or_else(|| bounds_failure(span))?;
                Ok(std::mem::replace(destination, incoming))
            })?;
            set_register(frames, *displaced, previous)?;
        }
        Instruction::ListTryRemove {
            dst,
            receiver,
            index,
            ..
        } => {
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            let value = match checked_receiver_index(module, store, frames, &place, *index, span)? {
                Some(index) => {
                    let removed = with_place_mut(module, store, frames, &place, span, |list| {
                        let Value::List(elements) = list else {
                            return Err(internal("validated try_remove received a non-list"));
                        };
                        elements.try_remove(index).ok_or_else(|| {
                            internal("validated try_remove index changed during access")
                        })
                    })?;
                    Some(Box::new(removed))
                }
                None => None,
            };
            set_register(frames, *dst, Value::Optional(value))?;
        }
        Instruction::ListClear { receiver, .. } => {
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            let length = with_place_value(module, store, frames, &place, span, |list| {
                let Value::List(elements) = list else {
                    return Err(internal("validated clear received a non-list"));
                };
                Ok(elements.length())
            })?;
            let mut removed = Vec::new();
            removed
                .try_reserve_exact(length)
                .map_err(|_| allocation_failure(span))?;
            with_place_mut(module, store, frames, &place, span, |list| {
                let Value::List(elements) = list else {
                    return Err(internal("validated clear received a non-list"));
                };
                elements.clear_into(&mut removed);
                Ok(())
            })?;
            for (index, value) in (0..length).rev().zip(removed) {
                cleanup_value(
                    module,
                    value,
                    CleanupPath::ListElement { index },
                    cleanup_trace.as_mut(),
                );
            }
        }
        Instruction::ListReserve {
            receiver,
            additional,
            ..
        } => {
            let additional =
                with_register_value(module, store, frames, *additional, span, |value| {
                    expect_int(value)
                })?;
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            with_place_mut(module, store, frames, &place, span, |list| {
                let Value::List(elements) = list else {
                    return Err(internal("validated reserve received a non-list"));
                };
                elements
                    .reserve(additional, &mut controls.allocations)
                    .map_err(|failure| reserve_failure(failure, span))
            })?;
        }
        Instruction::ListTryReserve {
            dst,
            receiver,
            additional,
            ..
        } => {
            let additional =
                with_register_value(module, store, frames, *additional, span, |value| {
                    expect_int(value)
                })?;
            let place = runtime_place_for_receiver(frames, receiver, span)?;
            let success = with_place_mut(module, store, frames, &place, span, |list| {
                let Value::List(elements) = list else {
                    return Err(internal("validated try_reserve received a non-list"));
                };
                Ok(elements.try_reserve(additional, &mut controls.allocations))
            })?;
            set_register(frames, *dst, Value::Bool(success))?;
        }
        Instruction::TextByteLength { dst, text, .. } => {
            let length = with_register_value(module, store, frames, *text, span, |value| {
                let Value::Text(value) = value else {
                    return Err(internal("validated text byte length received a non-text"));
                };
                i64::try_from(value.byte_length())
                    .map_err(|_| internal("validated Text length exceeds Int"))
            })?;
            set_register(frames, *dst, Value::Int(length))?;
        }
        Instruction::TextIsEmpty { dst, text, .. } => {
            let is_empty = with_register_value(module, store, frames, *text, span, |value| {
                let Value::Text(value) = value else {
                    return Err(internal("validated text is_empty received a non-text"));
                };
                Ok(value.byte_length() == 0)
            })?;
            set_register(frames, *dst, Value::Bool(is_empty))?;
        }
        Instruction::TextConcat { dst, lhs, rhs, .. } => {
            let Value::Text(left) = frame_value(frames, *lhs)? else {
                return Err(internal("validated text concat received a non-text lhs"));
            };
            let Value::Text(right) = frame_value(frames, *rhs)? else {
                return Err(internal("validated text concat received a non-text rhs"));
            };
            let value = RuntimeText::concat(left, right).ok_or_else(|| allocation_failure(span))?;
            set_register(frames, *dst, Value::Text(value))?;
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
            let values =
                materialize_fields(module, store, frames, *definition, fields, span, controls)?;
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
            if is_loan_register(module, frames, *dst) {
                let place = runtime_place_for_register(frames, *base)
                    .project(RuntimeProjection::Field(*field));
                frames
                    .last_mut()
                    .ok_or_else(|| internal("struct loan has no active frame"))?
                    .set_loan(*dst, place)
                    .map_err(|()| internal("validated struct loan destination is not empty"))?;
            } else {
                let value = with_register_value(module, store, frames, *base, span, |base| {
                    let Value::Struct { definition, fields } = base else {
                        return Err(internal("validated struct read received a non-struct"));
                    };
                    let field_index = field_index(module, *definition, *field)?;
                    let value = fields
                        .get(field_index)
                        .ok_or_else(|| internal("struct payload does not match its definition"))?;
                    structural_copy(value, span, controls)
                })?;
                set_register(frames, *dst, value)?;
            }
        }
        Instruction::InstallHome {
            destination,
            source,
            displaced,
            ..
        } => {
            let incoming = take_register(frames, *source)?;
            let previous = take_register(frames, *destination).ok();
            set_register(frames, *destination, incoming)?;
            if let Some(previous) = previous {
                set_register(frames, *displaced, previous)?;
            }
        }
        Instruction::MoveHome {
            destination,
            source,
            ..
        } => {
            let value = take_register(frames, *source)?;
            set_register(frames, *destination, value)?;
        }
        Instruction::DropHome { home, .. } => {
            let value = take_register(frames, *home)?;
            cleanup_value(
                module,
                value,
                CleanupPath::Home(*home),
                cleanup_trace.as_mut(),
            );
        }
        Instruction::DropIfLive { home, .. } => {
            if let Some(value) = frames.last_mut().and_then(|frame| frame.take(*home)) {
                cleanup_value(
                    module,
                    value,
                    CleanupPath::Home(*home),
                    cleanup_trace.as_mut(),
                );
            }
        }
        Instruction::DropSlot { slot, .. } => {
            if let Some(value) = frames.last_mut().and_then(|frame| frame.take(*slot)) {
                cleanup_value(
                    module,
                    value,
                    CleanupPath::Home(*slot),
                    cleanup_trace.as_mut(),
                );
            }
        }
        Instruction::CleanupTrackedScope { scope, .. } => {
            while let Some(home) = frames
                .last_mut()
                .and_then(|frame| frame.pop_cleanup_home(*scope))
            {
                if let Some(value) = frames.last_mut().and_then(|frame| frame.take(home)) {
                    cleanup_value(
                        module,
                        value,
                        CleanupPath::Home(home),
                        cleanup_trace.as_mut(),
                    );
                }
            }
        }
        Instruction::ReplacePlace {
            destination,
            source,
            displaced,
            ..
        } => {
            let incoming = take_register(frames, *source)?;
            let previous = take_source(module, store, frames, destination, span).ok();
            write_source(module, store, frames, destination, incoming, span)?;
            if let Some(previous) = previous {
                set_register(frames, *displaced, previous)?;
            }
        }
        Instruction::ReplaceField {
            view,
            field,
            source,
            displaced,
            ..
        } => {
            let active = active_view(frames, *view)?;
            if active.mode != ViewMode::Edit {
                return Err(internal("validated field replacement used a read view"));
            }
            let incoming = take_register(frames, *source)?;
            let previous = store
                .edit(active.entity, |payload| {
                    let index = field_index(module, payload.definition, *field)?;
                    let destination = payload
                        .fields
                        .get_mut(index)
                        .ok_or_else(|| internal("entity payload layout is invalid"))?;
                    Ok::<Value, InterpreterFailure>(std::mem::replace(destination, incoming))
                })
                .map_err(|error| store_failure(error, span))??;
            set_register(frames, *displaced, previous)?;
        }
        _ => {
            return execute_effect_instruction(
                module,
                store,
                frames,
                instruction,
                controls,
                cleanup_trace,
            );
        }
    }
    advance(frames)?;
    Ok(())
}

fn execute_effect_instruction(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
    controls: &mut TestControls,
    cleanup_trace: &mut Option<CleanupTrace>,
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
                .end_lifecycle_with(lifecycle, |payload| {
                    cleanup_payload(module, payload, cleanup_trace.as_mut());
                })
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
            let values =
                materialize_fields(module, store, frames, *definition, fields, span, controls)?;
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
        _ => {
            return execute_view_instruction(
                module,
                store,
                frames,
                instruction,
                controls,
                cleanup_trace,
            );
        }
    }
    advance(frames)?;
    Ok(())
}

fn execute_view_instruction(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
    controls: &mut TestControls,
    cleanup_trace: &mut Option<CleanupTrace>,
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
            if is_loan_register(module, frames, *dst) {
                let place = RuntimePlace {
                    root: RuntimePlaceRoot::Entity {
                        entity: active.entity,
                    },
                    projections: vec![RuntimeProjection::Field(*field)],
                };
                frames
                    .last_mut()
                    .ok_or_else(|| internal("entity loan has no active frame"))?
                    .set_loan(*dst, place)
                    .map_err(|()| internal("validated entity loan destination is not empty"))?;
            } else {
                let value = store
                    .read(active.entity, |payload| {
                        let index = field_index(module, payload.definition, *field)?;
                        let field = payload
                            .fields
                            .get(index)
                            .ok_or_else(|| internal("entity payload layout is invalid"))?;
                        structural_copy(field, span, controls)
                    })
                    .map_err(|error| store_failure(error, span))??;
                set_register(frames, *dst, value)?;
            }
        }
        Instruction::WriteField {
            view, field, value, ..
        } => {
            let active = active_view(frames, *view)?;
            if active.mode != ViewMode::Edit {
                return Err(internal("validated write used a read view"));
            }
            let value = take_argument(module, store, frames, *value, span, controls)?;
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
        _ => {
            return execute_call_or_retirement(
                module,
                store,
                frames,
                instruction,
                controls,
                cleanup_trace,
            );
        }
    }
    advance(frames)?;
    Ok(())
}

fn execute_call_or_retirement(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    instruction: &Instruction,
    controls: &mut TestControls,
    cleanup_trace: &mut Option<CleanupTrace>,
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
                .retire_with(entity, |payload| {
                    cleanup_payload(module, payload, cleanup_trace.as_mut());
                })
                .map_err(|error| store_failure(error, span))?;
        }
        Instruction::Call {
            dst,
            function,
            arguments,
            argument_sources,
            current_lifecycle,
            ..
        } => {
            return execute_call(
                module,
                store,
                frames,
                *dst,
                *function,
                arguments,
                argument_sources,
                *current_lifecycle,
                span,
                controls,
            );
        }
        Instruction::ConstInt { .. }
        | Instruction::ConstBool { .. }
        | Instruction::ConstText { .. }
        | Instruction::ConstNoneLink { .. }
        | Instruction::Copy { .. }
        | Instruction::Take { .. }
        | Instruction::ListNew { .. }
        | Instruction::ListLength { .. }
        | Instruction::ListPush { .. }
        | Instruction::ListPushPlace { .. }
        | Instruction::ListRemove { .. }
        | Instruction::ListRemovePlace { .. }
        | Instruction::ListIndex { .. }
        | Instruction::ListGet { .. }
        | Instruction::ListReplace { .. }
        | Instruction::ListTryRemove { .. }
        | Instruction::ListClear { .. }
        | Instruction::ListReserve { .. }
        | Instruction::ListTryReserve { .. }
        | Instruction::TextByteLength { .. }
        | Instruction::TextIsEmpty { .. }
        | Instruction::TextConcat { .. }
        | Instruction::CheckedUnaryInt { .. }
        | Instruction::CheckedBinaryInt { .. }
        | Instruction::Not { .. }
        | Instruction::Compare { .. }
        | Instruction::Phi { .. }
        | Instruction::ConstructStruct { .. }
        | Instruction::ReadStructField { .. }
        | Instruction::InstallHome { .. }
        | Instruction::MoveHome { .. }
        | Instruction::DropHome { .. }
        | Instruction::DropIfLive { .. }
        | Instruction::DropSlot { .. }
        | Instruction::CleanupTrackedScope { .. }
        | Instruction::ReplacePlace { .. }
        | Instruction::ReplaceField { .. }
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

#[allow(clippy::too_many_arguments)]
fn execute_call(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    destination: Option<Register>,
    function: keld_semantics::FunctionId,
    arguments: &[(ParameterIndex, Register)],
    argument_sources: &[(ParameterIndex, Option<ArgumentSource>)],
    current_lifecycle: Register,
    span: Span,
    controls: &mut TestControls,
) -> Result<(), InterpreterFailure> {
    let callee = module
        .functions
        .get(function.0 as usize)
        .ok_or_else(|| internal("validated call target is missing"))?;
    let mut materialized = Vec::new();
    let mut loans = Vec::new();
    materialized
        .try_reserve(arguments.len())
        .map_err(|_| allocation_failure(span))?;
    loans
        .try_reserve(arguments.len())
        .map_err(|_| allocation_failure(span))?;
    for (parameter, register) in arguments {
        let parameter_index = parameter.0 as usize;
        let mode = callee
            .parameter_modes
            .get(parameter_index)
            .copied()
            .unwrap_or(keld_semantics::ParameterMode::Loan);
        let callee_register = callee
            .parameters
            .get(parameter_index)
            .copied()
            .ok_or_else(|| internal("validated call parameter is missing"))?;
        if mode == keld_semantics::ParameterMode::Loan {
            let source = argument_sources
                .iter()
                .find(|(candidate, _)| *candidate == *parameter)
                .and_then(|(_, source)| source.as_ref());
            let place = if frames
                .last()
                .and_then(|frame| frame.loan(*register))
                .is_some()
            {
                runtime_place_for_register(frames, *register)
            } else if let Some(source) = source {
                runtime_place_for_source(frames, source, span)?
            } else {
                runtime_place_for_register(frames, *register)
            };
            loans.push((callee_register, place));
            continue;
        }
        let value = if mode == keld_semantics::ParameterMode::Take {
            take_register(frames, *register)?
        } else {
            structural_copy(frame_value(frames, *register)?, span, controls)?
        };
        materialized.push((*parameter, value));
    }
    let lifecycle = expect_lifecycle(frame_value(frames, current_lifecycle)?)?;
    advance(frames)?;
    let frame = build_frame(callee, materialized, loans, lifecycle, destination, span)?;
    frames
        .try_reserve(1)
        .map_err(|_| allocation_failure(span))?;
    frames.push(frame);
    enter_block(module, store, frames, callee.entry, None, span, controls)
}

fn build_frame(
    function: &Function,
    arguments: Vec<(ParameterIndex, Value)>,
    loans: Vec<(Register, RuntimePlace)>,
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
    for (register, place) in loans {
        frame
            .set_loan(register, place)
            .map_err(|()| internal("validated loan parameter register is not empty"))?;
    }
    Ok(frame)
}

fn execute_terminator(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    terminator: &Terminator,
    fallback_span: Span,
    controls: &mut TestControls,
) -> Result<Option<Value>, InterpreterFailure> {
    let predecessor = frames
        .last()
        .map(|frame| frame.block)
        .ok_or_else(|| internal("terminator has no active frame"))?;
    match terminator {
        Terminator::Goto(target) => {
            enter_block(
                module,
                store,
                frames,
                *target,
                Some(predecessor),
                fallback_span,
                controls,
            )?;
            Ok(None)
        }
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            let condition = expect_bool(frame_value(frames, *condition)?)?;
            let target = if condition { *then_block } else { *else_block };
            enter_block(
                module,
                store,
                frames,
                target,
                Some(predecessor),
                fallback_span,
                controls,
            )?;
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
                enter_block(
                    module,
                    store,
                    frames,
                    *live,
                    Some(predecessor),
                    *span,
                    controls,
                )?;
            } else {
                enter_block(
                    module,
                    store,
                    frames,
                    *absent,
                    Some(predecessor),
                    *span,
                    controls,
                )?;
            }
            Ok(None)
        }
        Terminator::Return(register) => return_from_frame(frames, *register),
        Terminator::Fault { kind, span } => Err(InterpreterFailure::Runtime(RuntimeFault {
            kind: match kind {
                FaultKind::Arithmetic => RuntimeFaultKind::Arithmetic,
                FaultKind::DivisionByZero => RuntimeFaultKind::DivisionByZero,
                FaultKind::Shift => RuntimeFaultKind::Shift,
                FaultKind::Allocation => RuntimeFaultKind::Allocation,
                FaultKind::Capacity => RuntimeFaultKind::Capacity,
            },
            span: *span,
        })),
        Terminator::Unreachable => Err(internal("execution reached an unreachable terminator")),
    }
}

fn return_from_frame(
    frames: &mut Vec<Frame>,
    register: Option<Register>,
) -> Result<Option<Value>, InterpreterFailure> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("return has no active frame"))?;
    let value = if let Some(register) = register {
        frame
            .take(register)
            .ok_or_else(|| internal("validated return register is undefined"))?
    } else {
        Value::Unit
    };
    let destination = frame.return_destination;
    frames.pop();
    if frames.last().is_none() {
        return Ok(Some(value));
    }
    if let Some(destination) = destination {
        set_register(frames, destination, value)?;
    }
    Ok(None)
}

fn enter_block(
    module: &Module,
    store: &Store<EntityPayload>,
    frames: &mut [Frame],
    target: IrBlockId,
    predecessor: Option<IrBlockId>,
    span: Span,
    controls: &mut TestControls,
) -> Result<(), InterpreterFailure> {
    let function_id = frames
        .last()
        .ok_or_else(|| internal("block entry has no active frame"))?
        .function;
    let function = module
        .functions
        .get(function_id.0 as usize)
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
    let mut pending_values = Vec::new();
    let mut pending_loans = Vec::new();
    pending_values
        .try_reserve_exact(phi_count)
        .map_err(|_| allocation_failure(span))?;
    pending_loans
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
        if is_loan_register(module, frames, *dst) {
            pending_loans.push((*dst, runtime_place_for_register(frames, source)));
        } else if is_home_register(module, frames, *dst) && is_home_register(module, frames, source)
        {
            pending_values.push((*dst, take_register(frames, source)?));
        } else {
            let value = with_register_value(module, store, frames, source, span, |value| {
                structural_copy(value, span, controls)
            })?;
            pending_values.push((*dst, value));
        }
    }
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("block entry has no active frame"))?;
    frame.block = target;
    frame.predecessor = predecessor;
    frame.instruction = phi_count;
    for (destination, value) in pending_values {
        frame
            .set(destination, value)
            .map_err(|()| internal("Phi destination is outside the frame"))?;
    }
    for (destination, place) in pending_loans {
        frame
            .set_loan(destination, place)
            .map_err(|()| internal("Phi loan destination is not empty"))?;
    }
    Ok(())
}

fn materialize_fields(
    module: &Module,
    store: &Store<EntityPayload>,
    frames: &mut [Frame],
    definition: DefId,
    fields: &[(FieldId, Register)],
    span: Span,
    controls: &mut TestControls,
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
        let value = if is_home_register(module, frames, *register) {
            take_register(frames, *register)?
        } else {
            with_register_value(module, store, frames, *register, span, |value| {
                structural_copy(value, span, controls)
            })?
        };
        source_values.push((*field, Some(value)));
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

fn runtime_place_for_register(frames: &[Frame], register: Register) -> RuntimePlace {
    let frame = FrameId(frames.len().saturating_sub(1));
    if let Some(place) = frames.last().and_then(|frame| frame.loan(register)) {
        return place.clone();
    }
    RuntimePlace::frame(frame, register)
}

fn runtime_place_for_receiver(
    frames: &[Frame],
    receiver: &Receiver,
    span: Span,
) -> Result<RuntimePlace, InterpreterFailure> {
    if let Some(place) = frames.last().and_then(|frame| frame.loan(receiver.list)) {
        return Ok(place.clone());
    }
    receiver.source.as_ref().map_or_else(
        || Ok(runtime_place_for_register(frames, receiver.list)),
        |source| runtime_place_for_source(frames, source, span),
    )
}

fn checked_receiver_index(
    module: &Module,
    store: &Store<EntityPayload>,
    frames: &[Frame],
    place: &RuntimePlace,
    index: Register,
    span: Span,
) -> Result<Option<usize>, InterpreterFailure> {
    let raw = with_register_value(module, store, frames, index, span, |value| {
        expect_int(value)
    })?;
    let length = with_place_value(module, store, frames, place, span, |value| {
        let Value::List(elements) = value else {
            return Err(internal("validated indexed receiver is not a List"));
        };
        Ok(elements.length())
    })?;
    Ok(usize::try_from(raw).ok().filter(|index| *index < length))
}

fn is_loan_register(module: &Module, frames: &[Frame], register: Register) -> bool {
    frames
        .last()
        .and_then(|frame| module.functions.get(frame.function.0 as usize))
        .and_then(|function| function.register_storage.get(register.0 as usize))
        .is_some_and(|storage| matches!(storage, keld_ir::RegisterStorage::Loan))
}

fn is_home_register(module: &Module, frames: &[Frame], register: Register) -> bool {
    frames
        .last()
        .and_then(|frame| module.functions.get(frame.function.0 as usize))
        .and_then(|function| function.register_storage.get(register.0 as usize))
        .is_some_and(|storage| matches!(storage, keld_ir::RegisterStorage::Home { .. }))
}

fn runtime_place_for_source(
    frames: &[Frame],
    source: &ArgumentSource,
    span: Span,
) -> Result<RuntimePlace, InterpreterFailure> {
    let mut place = if source.projections.is_empty() {
        runtime_place_for_register(frames, source.base)
    } else {
        match frames.last().and_then(|frame| frame.value(source.base)) {
            Some(Value::Entity(entity)) => RuntimePlace {
                root: RuntimePlaceRoot::Entity { entity: *entity },
                projections: Vec::new(),
            },
            _ => runtime_place_for_register(frames, source.base),
        }
    };
    for projection in &source.projections {
        place = match projection {
            ArgumentProjection::Field(field) => place.project(RuntimeProjection::Field(*field)),
            ArgumentProjection::Index(index) => {
                let raw = expect_int(frame_value(frames, *index)?)?;
                let index = usize::try_from(raw).map_err(|_| bounds_failure(span))?;
                place.project(RuntimeProjection::Index(index))
            }
        };
    }
    Ok(place)
}

fn with_register_value<R>(
    module: &Module,
    store: &Store<EntityPayload>,
    frames: &[Frame],
    register: Register,
    span: Span,
    access: impl FnOnce(&Value) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    let Some(frame) = frames.last() else {
        return Err(internal("register read has no active frame"));
    };
    if let Some(place) = frame.loan(register).cloned() {
        return with_place_value(module, store, frames, &place, span, access);
    }
    let value = frame
        .value(register)
        .ok_or_else(|| internal("validated register is undefined"))?;
    access(value)
}

fn with_register_mut<R>(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut [Frame],
    register: Register,
    span: Span,
    access: impl FnOnce(&mut Value) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    let frame_index = frames
        .len()
        .checked_sub(1)
        .ok_or_else(|| internal("register mutation has no active frame"))?;
    if let Some(place) = frames[frame_index].loan(register).cloned() {
        return with_place_mut(module, store, frames, &place, span, access);
    }
    let value = frames[frame_index]
        .value_mut(register)
        .ok_or_else(|| internal("validated register is undefined"))?;
    access(value)
}

fn with_place_value<R>(
    module: &Module,
    store: &Store<EntityPayload>,
    frames: &[Frame],
    place: &RuntimePlace,
    span: Span,
    access: impl FnOnce(&Value) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    match place.root {
        RuntimePlaceRoot::Frame { frame, register } => {
            let value = frames
                .get(frame.0)
                .and_then(|frame| frame.value(register))
                .ok_or_else(|| internal("loan root register is undefined"))?;
            access_projected(module, value, &place.projections, span, access)
        }
        RuntimePlaceRoot::Entity { entity } => store
            .read(entity, |payload| {
                let Some(RuntimeProjection::Field(field)) = place.projections.first() else {
                    return Err(internal("entity loan is missing a field projection"));
                };
                let index = field_index(module, payload.definition, *field)?;
                let value = payload
                    .fields
                    .get(index)
                    .ok_or_else(|| internal("entity payload layout is invalid"))?;
                access_projected(module, value, &place.projections[1..], span, access)
            })
            .map_err(|error| store_failure(error, span))?,
    }
}

fn with_place_mut<R>(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut [Frame],
    place: &RuntimePlace,
    span: Span,
    access: impl FnOnce(&mut Value) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    match place.root {
        RuntimePlaceRoot::Frame { frame, register } => {
            let value = frames
                .get_mut(frame.0)
                .and_then(|frame| frame.value_mut(register))
                .ok_or_else(|| internal("loan root register is undefined"))?;
            access_projected_mut(module, value, &place.projections, span, access)
        }
        RuntimePlaceRoot::Entity { entity } => store
            .edit(entity, |payload| {
                let Some(RuntimeProjection::Field(field)) = place.projections.first() else {
                    return Err(internal("entity loan is missing a field projection"));
                };
                let index = field_index(module, payload.definition, *field)?;
                let value = payload
                    .fields
                    .get_mut(index)
                    .ok_or_else(|| internal("entity payload layout is invalid"))?;
                access_projected_mut(module, value, &place.projections[1..], span, access)
            })
            .map_err(|error| store_failure(error, span))?,
    }
}

fn access_projected<R>(
    module: &Module,
    mut value: &Value,
    projections: &[RuntimeProjection],
    span: Span,
    access: impl FnOnce(&Value) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    for projection in projections {
        value = match projection {
            RuntimeProjection::Field(field) => {
                let Value::Struct { definition, fields } = value else {
                    return Err(internal("loan field projection requires a struct"));
                };
                let index = field_index(module, *definition, *field)?;
                fields
                    .get(index)
                    .ok_or_else(|| internal("struct payload layout is invalid"))?
            }
            RuntimeProjection::Index(index) => {
                let Value::List(elements) = value else {
                    return Err(internal("loan index projection requires a list"));
                };
                elements.get(*index).ok_or_else(|| bounds_failure(span))?
            }
        };
    }
    access(value)
}

fn access_projected_mut<R>(
    module: &Module,
    mut value: &mut Value,
    projections: &[RuntimeProjection],
    span: Span,
    access: impl FnOnce(&mut Value) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    for projection in projections {
        value = match projection {
            RuntimeProjection::Field(field) => {
                let Value::Struct { definition, fields } = value else {
                    return Err(internal("loan field projection requires a struct"));
                };
                let index = field_index(module, *definition, *field)?;
                fields
                    .get_mut(index)
                    .ok_or_else(|| internal("struct payload layout is invalid"))?
            }
            RuntimeProjection::Index(index) => {
                let Value::List(elements) = value else {
                    return Err(internal("loan index projection requires a list"));
                };
                elements
                    .get_mut(*index)
                    .ok_or_else(|| bounds_failure(span))?
            }
        };
    }
    access(value)
}

fn structural_copy(
    value: &Value,
    span: Span,
    controls: &mut TestControls,
) -> Result<Value, InterpreterFailure> {
    if controls.fail_structural_copies > 0 {
        controls.fail_structural_copies -= 1;
        return Err(allocation_failure(span));
    }
    try_copy_value(value).map_err(|_| allocation_failure(span))
}

fn take_argument(
    module: &Module,
    store: &Store<EntityPayload>,
    frames: &mut [Frame],
    register: Register,
    span: Span,
    controls: &mut TestControls,
) -> Result<Value, InterpreterFailure> {
    if is_home_register(module, frames, register) {
        return take_register(frames, register);
    }
    with_register_value(module, store, frames, register, span, |value| {
        structural_copy(value, span, controls)
    })
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

fn take_register(frames: &mut [Frame], register: Register) -> Result<Value, InterpreterFailure> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("register move has no active frame"))?;
    frame
        .take(register)
        .ok_or_else(|| internal("validated source register is undefined or already moved"))
}

fn take_source(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut [Frame],
    source: &ArgumentSource,
    span: Span,
) -> Result<Value, InterpreterFailure> {
    let place = runtime_place_for_source(frames, source, span)?;
    with_place_mut(module, store, frames, &place, span, |destination| {
        Ok(std::mem::replace(destination, Value::Unit))
    })
}

fn write_source(
    module: &Module,
    store: &mut Store<EntityPayload>,
    frames: &mut [Frame],
    source: &ArgumentSource,
    value: Value,
    span: Span,
) -> Result<(), InterpreterFailure> {
    let place = runtime_place_for_source(frames, source, span)?;
    with_place_mut(module, store, frames, &place, span, |destination| {
        *destination = value;
        Ok(())
    })
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
        (Value::Text(lhs), Value::Text(rhs)) => Ok(lhs == rhs),
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

fn reserve_failure(failure: ReserveFailure, span: Span) -> InterpreterFailure {
    match failure {
        ReserveFailure::Capacity => InterpreterFailure::Runtime(RuntimeFault {
            kind: RuntimeFaultKind::Capacity,
            span,
        }),
        ReserveFailure::Allocation => allocation_failure(span),
    }
}

fn bounds_failure(span: Span) -> InterpreterFailure {
    InterpreterFailure::Runtime(RuntimeFault {
        kind: RuntimeFaultKind::Bounds,
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
        | Instruction::ConstText { span, .. }
        | Instruction::ConstNoneLink { span, .. }
        | Instruction::Copy { span, .. }
        | Instruction::Take { span, .. }
        | Instruction::ListNew { span, .. }
        | Instruction::ListLength { span, .. }
        | Instruction::ListPush { span, .. }
        | Instruction::ListPushPlace { span, .. }
        | Instruction::ListRemove { span, .. }
        | Instruction::ListRemovePlace { span, .. }
        | Instruction::ListIndex { span, .. }
        | Instruction::ListGet { span, .. }
        | Instruction::ListReplace { span, .. }
        | Instruction::ListTryRemove { span, .. }
        | Instruction::ListClear { span, .. }
        | Instruction::ListReserve { span, .. }
        | Instruction::ListTryReserve { span, .. }
        | Instruction::TextByteLength { span, .. }
        | Instruction::TextIsEmpty { span, .. }
        | Instruction::TextConcat { span, .. }
        | Instruction::CheckedUnaryInt { span, .. }
        | Instruction::CheckedBinaryInt { span, .. }
        | Instruction::Not { span, .. }
        | Instruction::Compare { span, .. }
        | Instruction::Phi { span, .. }
        | Instruction::ConstructStruct { span, .. }
        | Instruction::ReadStructField { span, .. }
        | Instruction::InstallHome { span, .. }
        | Instruction::MoveHome { span, .. }
        | Instruction::DropHome { span, .. }
        | Instruction::DropIfLive { span, .. }
        | Instruction::DropSlot { span, .. }
        | Instruction::CleanupTrackedScope { span, .. }
        | Instruction::ReplacePlace { span, .. }
        | Instruction::ReplaceField { span, .. }
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
