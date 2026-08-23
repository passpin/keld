use crate::cleanup::{
    CleanupPath, CleanupScratch, CleanupTrace, ExecutionTrace, cleanup_payload, cleanup_value,
};
use crate::fault::{InterpreterError, InterpreterFailure, RuntimeFault, RuntimeFaultKind};
use crate::frame::{ActiveView, Frame};
use crate::place::{FrameId, RuntimePlace, RuntimePlaceRoot, RuntimeProjection};
use crate::value::RuntimeText;
use crate::value::{
    CopyAllocation, CopyAllocationPolicy, EntityPayload, Value, ValueKind,
    try_copy_value_with_allocation_controls,
};
use crate::{AllocationController, AllocationPolicy, ReserveFailure};
use keld_ir::{
    AllocationPhase, AllocationSchedule, ArgumentProjection, ArgumentSource, FaultKind, Function,
    Instruction, IrBlockId, IrType, Module, Receiver, Register, Terminator, ViewMode,
};
use keld_numeric::{NumericFault, eval_binary, eval_unary};
use keld_runtime::{RuntimeLifecycleId, RuntimeTypeId, Store, StoreError};
use keld_semantics::{CompareOp, DefId, FieldId, ParameterIndex};
use keld_source::Span;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub value: Value,
}

/// One semantic allocation attempt observed by the interpreter test runtime.
/// The record uses the same frozen phase and site-ID representation as the
/// Native-1 DLL observation stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationObservation {
    pub site_id: u32,
    pub phase: AllocationPhase,
    pub attempt: u64,
    pub allowed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TestControls {
    fail_structural_copies: usize,
    text_attempt: u64,
    fail_text_attempts: BTreeSet<u64>,
    place_attempt: u64,
    fail_place_attempts: BTreeSet<u64>,
    allocations: AllocationController,
    schedule_enabled: bool,
    schedule_failures: BTreeSet<(u32, AllocationPhase, u64)>,
    schedule_attempts: BTreeMap<(u32, AllocationPhase), u64>,
    schedule_observations: Vec<AllocationObservation>,
    current_base_site: u32,
    allocation_ordinals: BTreeMap<AllocationPhase, u32>,
}

impl TestControls {
    #[must_use]
    pub fn fail_structural_copy(count: usize) -> Self {
        Self {
            fail_structural_copies: count,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn fail_list_attempts(attempts: impl IntoIterator<Item = u64>) -> Self {
        Self {
            allocations: AllocationController::fail_list_attempts(attempts),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn fail_text_attempts(attempts: impl IntoIterator<Item = u64>) -> Self {
        Self {
            fail_text_attempts: attempts.into_iter().collect(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn fail_place_attempts(attempts: impl IntoIterator<Item = u64>) -> Self {
        Self {
            fail_place_attempts: attempts.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Creates controls from the frozen cross-engine failure schedule.
    ///
    /// Each tuple is `(site_id, phase, one_based_attempt)`. The schedule is
    /// intentionally test-only and does not alter production interpreter
    /// behavior or the source language.
    #[doc(hidden)]
    #[must_use]
    pub fn fail_allocation_schedule(
        failures: impl IntoIterator<Item = (u32, AllocationPhase, u64)>,
    ) -> Self {
        Self {
            schedule_enabled: true,
            schedule_failures: failures.into_iter().collect(),
            ..Self::default()
        }
    }

    /// Alias retained for differential-test call sites that describe the
    /// controls as a schedule rather than a failure list.
    #[doc(hidden)]
    #[must_use]
    pub fn with_allocation_schedule_for_test(
        failures: impl IntoIterator<Item = (u32, AllocationPhase, u64)>,
    ) -> Self {
        Self::fail_allocation_schedule(failures)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn allocation_observations_for_test(&self) -> &[AllocationObservation] {
        &self.schedule_observations
    }

    fn allow_text_legacy_attempt(&mut self) -> bool {
        self.text_attempt = self.text_attempt.saturating_add(1);
        !self.fail_text_attempts.contains(&self.text_attempt)
    }

    fn allow_text_copy_attempt(&mut self) -> bool {
        self.text_attempt = self.text_attempt.saturating_add(1);
        !self.fail_text_attempts.contains(&self.text_attempt)
    }

    fn allow_place_attempt(&mut self) -> bool {
        self.place_attempt = self.place_attempt.saturating_add(1);
        !self.fail_place_attempts.contains(&self.place_attempt)
    }

    fn begin_allocation_operation(&mut self, base_site: u32) {
        self.current_base_site = base_site;
        self.allocation_ordinals.clear();
    }

    fn allow_allocation(&mut self, phase: AllocationPhase) -> bool {
        let ordinal = self.allocation_ordinals.get(&phase).copied().unwrap_or(0);
        self.allocation_ordinals
            .insert(phase, ordinal.saturating_add(1));
        self.allow_allocation_at(phase, ordinal)
    }

    fn allow_allocation_at(&mut self, phase: AllocationPhase, ordinal: u32) -> bool {
        if !self.schedule_enabled {
            return true;
        }
        let site_id = if self.current_base_site == 0 {
            0
        } else {
            keld_ir::allocation_site_id(self.current_base_site, phase, ordinal)
        };
        let key = (site_id, phase);
        let attempt = self
            .schedule_attempts
            .entry(key)
            .and_modify(|value| *value = value.saturating_add(1))
            .or_insert(1);
        let attempt = *attempt;
        let allowed = !self.schedule_failures.contains(&(site_id, phase, attempt));
        self.schedule_observations.push(AllocationObservation {
            site_id,
            phase,
            attempt,
            allowed,
        });
        allowed
    }
}

impl AllocationPolicy for TestControls {
    fn allow_list_attempt(&mut self, phase: AllocationPhase) -> bool {
        self.allocations.allow_list_attempt(phase) && self.allow_allocation(phase)
    }
}

impl CopyAllocationPolicy for TestControls {
    fn allow_copy(&mut self, ordinal: u32) -> bool {
        self.allow_allocation_at(AllocationPhase::Copy, ordinal)
    }

    fn allow_text(&mut self) -> bool {
        self.allow_text_copy_attempt()
    }

    fn allow_handle(&mut self, ordinal: u32) -> bool {
        self.allow_allocation_at(AllocationPhase::Handle, ordinal)
    }
}

pub struct Interpreter<'module> {
    module: &'module Module,
    allocation_schedule: AllocationSchedule,
    store: Store<EntityPayload>,
    frames: Vec<Frame>,
    started: bool,
    controls: TestControls,
    cleanup_trace: Option<CleanupTrace>,
    cleanup_scratch: CleanupScratch,
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

    #[doc(hidden)]
    pub fn with_controls_for_test(
        module: &'module Module,
        controls: TestControls,
    ) -> Result<Self, InterpreterFailure> {
        Self::with_controls(module, controls)
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
        mut controls: TestControls,
        trace_cleanup: bool,
    ) -> Result<Self, InterpreterFailure> {
        let diagnostics = keld_ir::validate(module);
        if !diagnostics.is_empty() {
            return Err(InterpreterFailure::Internal(InterpreterError::InvalidIr(
                diagnostics,
            )));
        }
        let span = main_span(module);
        let allocation_schedule = AllocationSchedule::from_module(module);
        let main = module
            .functions
            .iter()
            .find(|function| function.id == module.main)
            .ok_or_else(|| internal("validated main function is missing"))?;
        let context_site = allocation_schedule
            .base_id(module.main, main.entry, 0)
            .unwrap_or(1);
        controls.begin_allocation_operation(context_site);
        if !controls.allow_allocation_at(AllocationPhase::Context, 0) {
            return Err(allocation_failure(span));
        }
        let store = Store::new().map_err(|error| store_failure(error, span))?;
        let cleanup_scratch = CleanupScratch::new().map_err(|()| allocation_failure(span))?;
        let mut frames = Vec::new();
        frames
            .try_reserve(module.functions.len().max(1))
            .map_err(|_| allocation_failure(span))?;
        Ok(Self {
            module,
            allocation_schedule,
            store,
            frames,
            started: false,
            controls,
            cleanup_trace: trace_cleanup.then(CleanupTrace::default),
            cleanup_scratch,
        })
    }

    /// Runs the validated zero-argument main function once.
    ///
    /// # Errors
    ///
    /// Returns a Keld runtime fault or an internal invariant error.
    pub fn run_main(&mut self) -> Result<ExecutionResult, InterpreterFailure> {
        self.run_main_with_hook(&mut |_| {})
    }

    /// Runs main while invoking a test-only callback before each instruction.
    ///
    /// This is intentionally hidden from normal API documentation. It lets
    /// integration tests begin instrumentation at a specific instruction
    /// after setup has completed without exposing interpreter internals.
    #[doc(hidden)]
    pub fn run_main_with_test_hook(
        &mut self,
        before_instruction: &mut dyn FnMut(&Instruction),
    ) -> Result<ExecutionResult, InterpreterFailure> {
        self.run_main_with_hook(before_instruction)
    }

    /// Returns the semantic allocation observations collected by a test
    /// schedule. Production callers receive an empty slice.
    #[doc(hidden)]
    #[must_use]
    pub fn allocation_observations_for_test(&self) -> &[AllocationObservation] {
        &self.controls.schedule_observations
    }

    fn run_main_with_hook(
        &mut self,
        before_instruction: &mut dyn FnMut(&Instruction),
    ) -> Result<ExecutionResult, InterpreterFailure> {
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
        let frame = build_frame(
            self.module,
            main,
            Vec::new(),
            Vec::new(),
            root,
            None,
            main.span,
        )?;
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
            if let Some(instruction) = self.current_instruction() {
                before_instruction(instruction);
            }
            if let Some(value) = self.step()? {
                let module = self.module;
                let cleanup_trace = &mut self.cleanup_trace;
                let cleanup_scratch = &mut self.cleanup_scratch;
                let mut cleanup_failed = false;
                self.store
                    .finish_with(|payload| {
                        cleanup_failed |= cleanup_payload(
                            module,
                            payload,
                            cleanup_scratch,
                            cleanup_trace.as_mut(),
                        )
                        .is_err();
                    })
                    .map_err(|error| store_failure(error, main.span))?;
                if cleanup_failed {
                    return Err(allocation_failure(main.span));
                }
                return Ok(ExecutionResult { value });
            }
        }
    }

    fn current_instruction(&self) -> Option<&Instruction> {
        let frame = self.frames.last()?;
        let function = self.module.functions.get(frame.function.0 as usize)?;
        let block = function.blocks.get(frame.block.0 as usize)?;
        block.instructions.get(frame.instruction)
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
        let allocation_site = self
            .allocation_schedule
            .base_id(
                function.id,
                block.id,
                u32::try_from(instruction_index)
                    .map_err(|_| internal("instruction index exceeds allocation schedule"))?,
            )
            .unwrap_or(0);
        self.controls.begin_allocation_operation(allocation_site);
        if let Some(instruction) = block.instructions.get(instruction_index) {
            execute_instruction(
                self.module,
                &mut self.store,
                &mut self.frames,
                instruction,
                &mut self.controls,
                &mut self.cleanup_trace,
                &mut self.cleanup_scratch,
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
    cleanup_scratch: &mut CleanupScratch,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::ConstInt { dst, value, .. } => {
            set_register(module, frames, *dst, Value::Int(*value))?;
        }
        Instruction::ConstBool { dst, value, .. } => {
            set_register(module, frames, *dst, Value::Bool(*value))?;
        }
        Instruction::ConstText { dst, value, .. } => {
            if !controls.allow_allocation(AllocationPhase::Text) {
                return Err(allocation_failure(span));
            }
            let text = RuntimeText::try_from_str(value, || controls.allow_text_legacy_attempt())
                .map_err(|CopyAllocation| allocation_failure(span))?;
            if !controls.allow_allocation(AllocationPhase::Handle) {
                return Err(allocation_failure(span));
            }
            set_register(module, frames, *dst, Value::Text(text))?;
        }
        Instruction::ConstNoneLink { dst, .. } => {
            set_register(module, frames, *dst, Value::Link(None))?;
        }
        Instruction::Copy { dst, src, .. } => {
            if is_loan_register(module, frames, *dst) {
                let place = runtime_place_for_register(frames, *src, span, controls)?;
                frames
                    .last_mut()
                    .ok_or_else(|| internal("loan copy has no active frame"))?
                    .set_loan(*dst, place)
                    .map_err(|()| internal("validated loan destination is not empty"))?;
            } else {
                let value = with_register_value(module, store, frames, *src, span, |value| {
                    structural_copy(value, span, controls)
                })?;
                set_register(module, frames, *dst, value)?;
            }
        }
        Instruction::Take { dst, src, .. } => {
            let value = take_register(frames, *src)?;
            set_register(module, frames, *dst, value)?;
        }
        Instruction::ListNew { dst, .. } => {
            if !controls.allow_allocation(AllocationPhase::Handle) {
                return Err(allocation_failure(span));
            }
            set_register(module, frames, *dst, Value::List(crate::RuntimeList::new()))?;
        }
        Instruction::ListLength { dst, list, .. } => {
            let length = with_register_value(module, store, frames, *list, span, |value| {
                let ValueKind::List(elements) = value.kind() else {
                    return Err(internal("validated list length received a non-list"));
                };
                i64::try_from(elements.length())
                    .map_err(|_| internal("validated List length exceeds Int"))
            })?;
            set_register(module, frames, *dst, Value::Int(length))?;
        }
        Instruction::ListPush { list, value, .. } => {
            let value = take_argument(module, store, frames, *value, span, controls)?;
            with_register_mut(module, store, frames, *list, span, |list_value| {
                let ValueKind::List(elements) = list_value.kind_mut() else {
                    return Err(internal("validated list push received a non-list"));
                };
                elements
                    .push(value, controls)
                    .map_err(|failure| reserve_failure(failure, span))?;
                Ok(())
            })?;
        }
        Instruction::ListPushPlace { source, value, .. } => {
            let value = take_argument(module, store, frames, *value, span, controls)?;
            let place = runtime_place_for_source(frames, source, span, controls)?;
            with_place_mut(module, store, frames, &place, span, |list_value| {
                let ValueKind::List(elements) = list_value.kind_mut() else {
                    return Err(internal("validated list push received a non-list"));
                };
                elements
                    .push(value, controls)
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
                let ValueKind::List(elements) = list_value.kind_mut() else {
                    return Err(internal("validated list remove received a non-list"));
                };
                if index >= elements.length() {
                    return Err(bounds_failure(span));
                }
                Ok(elements.remove(index))
            })?;
            set_register(module, frames, *dst, removed)?;
        }
        Instruction::ListRemovePlace {
            dst, source, index, ..
        } => {
            let index = with_register_value(module, store, frames, *index, span, |value| {
                usize::try_from(expect_int(value)?).map_err(|_| bounds_failure(span))
            })?;
            let place = runtime_place_for_source(frames, source, span, controls)?;
            let removed = with_place_mut(module, store, frames, &place, span, |list_value| {
                let ValueKind::List(elements) = list_value.kind_mut() else {
                    return Err(internal("validated list remove received a non-list"));
                };
                if index >= elements.length() {
                    return Err(bounds_failure(span));
                }
                Ok(elements.remove(index))
            })?;
            set_register(module, frames, *dst, removed)?;
        }
        Instruction::ListIndex {
            dst,
            receiver,
            index,
            ..
        } => {
            let place = runtime_place_for_receiver(frames, receiver, span, controls)?;
            let Some(index) =
                checked_receiver_index(module, store, frames, place.as_ref(), *index, span)?
            else {
                return Err(bounds_failure(span));
            };
            if is_loan_register(module, frames, *dst) {
                let loan = place.into_projected(RuntimeProjection::Index(index), span, controls)?;
                frames
                    .last_mut()
                    .ok_or_else(|| internal("indexed loan has no active frame"))?
                    .set_loan(*dst, loan)
                    .map_err(|()| internal("validated indexed loan destination is not empty"))?;
            } else {
                let value =
                    with_place_value(module, store, frames, place.as_ref(), span, |list| {
                        let ValueKind::List(elements) = list.kind() else {
                            return Err(internal("validated list index received a non-list"));
                        };
                        let element = elements.get(index).ok_or_else(|| bounds_failure(span))?;
                        structural_copy(element, span, controls)
                    })?;
                set_register(module, frames, *dst, value)?;
            }
        }
        Instruction::ListGet {
            dst,
            receiver,
            index,
            ..
        } => {
            let place = runtime_place_for_receiver(frames, receiver, span, controls)?;
            let value = match checked_receiver_index(
                module,
                store,
                frames,
                place.as_ref(),
                *index,
                span,
            )? {
                Some(index) => {
                    with_place_value(module, store, frames, place.as_ref(), span, |list| {
                        let ValueKind::List(elements) = list.kind() else {
                            return Err(internal("validated list get received a non-list"));
                        };
                        let element = elements.get(index).ok_or_else(|| {
                            internal("validated list get index changed during access")
                        })?;
                        structural_copy(element, span, controls).map(Some)
                    })?
                }
                None => None,
            };
            let expected = register_type(module, frames, *dst)?;
            let value = match value {
                Some(value) => value
                    .into_optional_some(module, optional_inner(expected)?)
                    .map_err(|_| internal("validated list get produced an invalid optional"))?,
                None => Value::optional_none(module, expected)
                    .map_err(|_| internal("validated list get destination is not optional"))?,
            };
            set_register(module, frames, *dst, value)?;
        }
        Instruction::ListReplace {
            receiver,
            index,
            value,
            displaced,
            ..
        } => {
            let previous = with_receiver_place_mut(
                store,
                frames,
                receiver,
                span,
                controls,
                |place, store, frames, controls| {
                    let Some(index) =
                        checked_receiver_index(module, store, frames, place, *index, span)?
                    else {
                        return Err(bounds_failure(span));
                    };
                    let incoming = take_argument(module, store, frames, *value, span, controls)?;
                    with_place_mut(module, store, frames, place, span, |list| {
                        let ValueKind::List(elements) = list.kind_mut() else {
                            return Err(internal("validated list replacement received a non-list"));
                        };
                        let destination = elements
                            .get_mut(index)
                            .ok_or_else(|| bounds_failure(span))?;
                        Ok(std::mem::replace(destination, incoming))
                    })
                },
            )?;
            set_register(module, frames, *displaced, previous)?;
        }
        Instruction::ListTryRemove {
            dst,
            receiver,
            index,
            ..
        } => {
            let value = with_receiver_place_mut(
                store,
                frames,
                receiver,
                span,
                controls,
                |place, store, frames, _controls| match checked_receiver_index(
                    module, store, frames, place, *index, span,
                )? {
                    Some(index) => with_place_mut(module, store, frames, place, span, |list| {
                        let ValueKind::List(elements) = list.kind_mut() else {
                            return Err(internal("validated try_remove received a non-list"));
                        };
                        elements
                            .try_remove(index)
                            .ok_or_else(|| {
                                internal("validated try_remove index changed during access")
                            })
                            .map(Some)
                    }),
                    None => Ok(None),
                },
            )?;
            let expected = register_type(module, frames, *dst)?;
            let value = match value {
                Some(value) => value
                    .into_optional_some(module, optional_inner(expected)?)
                    .map_err(|_| internal("validated try_remove produced an invalid optional"))?,
                None => Value::optional_none(module, expected)
                    .map_err(|_| internal("validated try_remove destination is not optional"))?,
            };
            set_register(module, frames, *dst, value)?;
        }
        Instruction::ListClear { receiver, .. } => {
            with_receiver_place_mut(
                store,
                frames,
                receiver,
                span,
                controls,
                |place, store, frames, _controls| {
                    let length = with_place_value(module, store, frames, place, span, |list| {
                        let ValueKind::List(elements) = list.kind() else {
                            return Err(internal("validated clear received a non-list"));
                        };
                        Ok(elements.length())
                    })?;
                    let mut index = length;
                    while index > 0 {
                        index -= 1;
                        let value = with_place_mut(module, store, frames, place, span, |list| {
                            let ValueKind::List(elements) = list.kind_mut() else {
                                return Err(internal("validated clear received a non-list"));
                            };
                            elements.pop().ok_or_else(|| {
                                internal("validated clear length changed during access")
                            })
                        })?;
                        cleanup_value(
                            module,
                            value,
                            CleanupPath::ListElement { index },
                            cleanup_scratch,
                            cleanup_trace.as_mut(),
                        )
                        .map_err(|()| allocation_failure(span))?;
                    }
                    Ok(())
                },
            )?;
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
            with_receiver_place_mut(
                store,
                frames,
                receiver,
                span,
                controls,
                |place, store, frames, controls| {
                    with_place_mut(module, store, frames, place, span, |list| {
                        let ValueKind::List(elements) = list.kind_mut() else {
                            return Err(internal("validated reserve received a non-list"));
                        };
                        elements
                            .reserve(additional, controls)
                            .map_err(|failure| reserve_failure(failure, span))
                    })
                },
            )?;
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
            let success = with_receiver_place_mut(
                store,
                frames,
                receiver,
                span,
                controls,
                |place, store, frames, controls| {
                    with_place_mut(module, store, frames, place, span, |list| {
                        let ValueKind::List(elements) = list.kind_mut() else {
                            return Err(internal("validated try_reserve received a non-list"));
                        };
                        Ok(elements.try_reserve(additional, controls))
                    })
                },
            )?;
            set_register(module, frames, *dst, Value::Bool(success))?;
        }
        Instruction::TextByteLength { dst, text, .. } => {
            let length = with_register_value(module, store, frames, *text, span, |value| {
                let ValueKind::Text(value) = value.kind() else {
                    return Err(internal("validated text byte length received a non-text"));
                };
                i64::try_from(value.byte_length())
                    .map_err(|_| internal("validated Text length exceeds Int"))
            })?;
            set_register(module, frames, *dst, Value::Int(length))?;
        }
        Instruction::TextIsEmpty { dst, text, .. } => {
            let is_empty = with_register_value(module, store, frames, *text, span, |value| {
                let ValueKind::Text(value) = value.kind() else {
                    return Err(internal("validated text is_empty received a non-text"));
                };
                Ok(value.byte_length() == 0)
            })?;
            set_register(module, frames, *dst, Value::Bool(is_empty))?;
        }
        Instruction::TextConcat { dst, lhs, rhs, .. } => {
            if !controls.allow_allocation(AllocationPhase::Concat) {
                return Err(allocation_failure(span));
            }
            let value = with_register_value(module, store, frames, *lhs, span, |left| {
                let ValueKind::Text(left) = left.kind() else {
                    return Err(internal("validated text concat received a non-text lhs"));
                };
                with_register_value(module, store, frames, *rhs, span, |right| {
                    let ValueKind::Text(right) = right.kind() else {
                        return Err(internal("validated text concat received a non-text rhs"));
                    };
                    RuntimeText::concat(left, right)
                        .ok_or_else(|| allocation_failure(span))
                        .map(Value::Text)
                })
            })?;
            if !controls.allow_allocation(AllocationPhase::Handle) {
                return Err(allocation_failure(span));
            }
            set_register(module, frames, *dst, value)?;
        }
        Instruction::CheckedUnaryInt { dst, op, src, .. } => {
            let value = expect_int(frame_value(frames, *src)?)?;
            let result = eval_unary(*op, value).map_err(|fault| numeric_failure(fault, span))?;
            set_register(module, frames, *dst, Value::Int(result))?;
        }
        Instruction::CheckedBinaryInt {
            dst, op, lhs, rhs, ..
        } => {
            let lhs = expect_int(frame_value(frames, *lhs)?)?;
            let rhs = expect_int(frame_value(frames, *rhs)?)?;
            let result =
                eval_binary(*op, lhs, rhs).map_err(|fault| numeric_failure(fault, span))?;
            set_register(module, frames, *dst, Value::Int(result))?;
        }
        Instruction::Not { dst, src, .. } => {
            let value = expect_bool(frame_value(frames, *src)?)?;
            set_register(module, frames, *dst, Value::Bool(!value))?;
        }
        Instruction::Compare {
            dst, op, lhs, rhs, ..
        } => {
            let result = with_register_value(module, store, frames, *lhs, span, |left| {
                with_register_value(module, store, frames, *rhs, span, |right| {
                    compare_values(*op, left, right)
                })
            })?;
            set_register(module, frames, *dst, Value::Bool(result))?;
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
            if !controls.allow_allocation(AllocationPhase::Struct) {
                return Err(allocation_failure(span));
            }
            if !controls.allow_allocation(AllocationPhase::Handle) {
                return Err(allocation_failure(span));
            }
            set_register(
                module,
                frames,
                *dst,
                Value::struct_value(*definition, values),
            )?;
        }
        Instruction::ReadStructField {
            dst, base, field, ..
        } => {
            if is_loan_register(module, frames, *dst) {
                let mut place =
                    runtime_place_for_register_with_capacity(frames, *base, 1, span, controls)?;
                place
                    .push_reserved(RuntimeProjection::Field(*field))
                    .map_err(|CopyAllocation| allocation_failure(span))?;
                frames
                    .last_mut()
                    .ok_or_else(|| internal("struct loan has no active frame"))?
                    .set_loan(*dst, place)
                    .map_err(|()| internal("validated struct loan destination is not empty"))?;
            } else {
                let value = with_register_value(module, store, frames, *base, span, |base| {
                    let ValueKind::Struct { definition, fields } = base.kind() else {
                        return Err(internal("validated struct read received a non-struct"));
                    };
                    let field_index = field_index(module, *definition, *field)?;
                    let value = fields
                        .get(field_index)
                        .ok_or_else(|| internal("struct payload does not match its definition"))?;
                    structural_copy(value, span, controls)
                })?;
                set_register(module, frames, *dst, value)?;
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
            set_register(module, frames, *destination, incoming)?;
            if let Some(previous) = previous {
                set_register(module, frames, *displaced, previous)?;
            }
        }
        Instruction::MoveHome {
            destination,
            source,
            ..
        } => {
            let value = take_register(frames, *source)?;
            set_register(module, frames, *destination, value)?;
        }
        Instruction::DropHome { home, .. } => {
            let value = take_register(frames, *home)?;
            cleanup_value(
                module,
                value,
                CleanupPath::Home(*home),
                cleanup_scratch,
                cleanup_trace.as_mut(),
            )
            .map_err(|()| allocation_failure(span))?;
        }
        Instruction::DropIfLive { home, .. } => {
            if let Some(value) = frames.last_mut().and_then(|frame| frame.take(*home)) {
                cleanup_value(
                    module,
                    value,
                    CleanupPath::Home(*home),
                    cleanup_scratch,
                    cleanup_trace.as_mut(),
                )
                .map_err(|()| allocation_failure(span))?;
            }
        }
        Instruction::DropSlot { slot, .. } => {
            if let Some(value) = frames.last_mut().and_then(|frame| frame.take(*slot)) {
                cleanup_value(
                    module,
                    value,
                    CleanupPath::Home(*slot),
                    cleanup_scratch,
                    cleanup_trace.as_mut(),
                )
                .map_err(|()| allocation_failure(span))?;
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
                        cleanup_scratch,
                        cleanup_trace.as_mut(),
                    )
                    .map_err(|()| allocation_failure(span))?;
                }
            }
        }
        Instruction::ReplacePlace {
            destination,
            source,
            displaced,
            ..
        } => {
            let place = runtime_place_for_source(frames, destination, span, controls)?;
            let incoming = take_register(frames, *source)?;
            let previous = with_place_mut(module, store, frames, &place, span, |destination| {
                Ok(std::mem::replace(destination, incoming))
            })?;
            set_register(module, frames, *displaced, previous)?;
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
            set_register(module, frames, *displaced, previous)?;
        }
        _ => {
            return execute_effect_instruction(
                module,
                store,
                frames,
                instruction,
                controls,
                cleanup_trace,
                cleanup_scratch,
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
    cleanup_scratch: &mut CleanupScratch,
) -> Result<(), InterpreterFailure> {
    let span = instruction_span(instruction);
    match instruction {
        Instruction::BeginLifecycle { dst, parent, .. } => {
            let parent = expect_lifecycle(frame_value(frames, *parent)?)?;
            if !controls.allow_allocation(AllocationPhase::Lifecycle) {
                return Err(allocation_failure(span));
            }
            let lifecycle = store
                .begin_lifecycle(parent)
                .map_err(|error| store_failure(error, span))?;
            set_register(module, frames, *dst, Value::Lifecycle(lifecycle))?;
        }
        Instruction::EndLifecycle { lifecycle, .. } => {
            let lifecycle = expect_lifecycle(frame_value(frames, *lifecycle)?)?;
            let mut cleanup_failed = false;
            store
                .end_lifecycle_with(lifecycle, |payload| {
                    cleanup_failed |=
                        cleanup_payload(module, payload, cleanup_scratch, cleanup_trace.as_mut())
                            .is_err();
                })
                .map_err(|error| store_failure(error, span))?;
            if cleanup_failed {
                return Err(allocation_failure(span));
            }
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
            if !controls.allow_allocation(AllocationPhase::Entity) {
                return Err(allocation_failure(span));
            }
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
            set_register(module, frames, *dst, Value::Entity(entity))?;
        }
        Instruction::EntityToLink { dst, entity, .. } => {
            let entity = expect_entity(frame_value(frames, *entity)?)?;
            let link = store
                .link(entity)
                .map_err(|error| store_failure(error, span))?;
            set_register(module, frames, *dst, Value::Link(Some(link)))?;
        }
        _ => {
            return execute_view_instruction(
                module,
                store,
                frames,
                instruction,
                controls,
                cleanup_trace,
                cleanup_scratch,
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
    cleanup_scratch: &mut CleanupScratch,
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
                let mut place = RuntimePlace::entity(active.entity);
                place
                    .try_reserve_projections(1, || controls.allow_place_attempt())
                    .map_err(|CopyAllocation| allocation_failure(span))?;
                place
                    .push_reserved(RuntimeProjection::Field(*field))
                    .map_err(|CopyAllocation| allocation_failure(span))?;
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
                set_register(module, frames, *dst, value)?;
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
                cleanup_scratch,
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
    cleanup_scratch: &mut CleanupScratch,
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
            let mut cleanup_failed = false;
            store
                .retire_with(entity, |payload| {
                    cleanup_failed =
                        cleanup_payload(module, payload, cleanup_scratch, cleanup_trace.as_mut())
                            .is_err();
                })
                .map_err(|error| store_failure(error, span))?;
            if cleanup_failed {
                return Err(allocation_failure(span));
            }
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
    _argument_sources: &[(ParameterIndex, Option<ArgumentSource>)],
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
            let place = runtime_place_for_register(frames, *register, span, controls)?;
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
    let frame = build_frame(
        module,
        callee,
        materialized,
        loans,
        lifecycle,
        destination,
        span,
    )?;
    frames
        .try_reserve(1)
        .map_err(|_| allocation_failure(span))?;
    frames.push(frame);
    enter_block(module, store, frames, callee.entry, None, span, controls)
}

fn build_frame(
    module: &Module,
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
            module,
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
            .set(module, register, value)
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
                set_register(module, frames, *live_value, Value::Entity(entity))?;
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
        Terminator::Return(register) => return_from_frame(module, frames, *register),
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
    module: &Module,
    frames: &mut Vec<Frame>,
    register: Option<Register>,
) -> Result<Option<Value>, InterpreterFailure> {
    let function_id = frames
        .last()
        .ok_or_else(|| internal("return has no active frame"))?
        .function;
    let return_type = &module
        .functions
        .get(function_id.0 as usize)
        .ok_or_else(|| internal("return function is missing"))?
        .return_type;
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
    value
        .validate_for_type(module, return_type)
        .map_err(|_| internal("runtime return value does not match the validated return type"))?;
    let destination = frame.return_destination;
    frames.pop();
    if frames.last().is_none() {
        return Ok(Some(value));
    }
    if let Some(destination) = destination {
        set_register(module, frames, destination, value)?;
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
            pending_loans.push((
                *dst,
                runtime_place_for_register(frames, source, span, controls)?,
            ));
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
            .set(module, destination, value)
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

fn runtime_place_for_register(
    frames: &[Frame],
    register: Register,
    span: Span,
    controls: &mut TestControls,
) -> Result<RuntimePlace, InterpreterFailure> {
    runtime_place_for_register_with_capacity(frames, register, 0, span, controls)
}

fn runtime_place_for_register_with_capacity(
    frames: &[Frame],
    register: Register,
    additional: usize,
    span: Span,
    controls: &mut TestControls,
) -> Result<RuntimePlace, InterpreterFailure> {
    let frame = FrameId(frames.len().saturating_sub(1));
    if let Some(place) = frames.last().and_then(|frame| frame.loan(register)) {
        return place
            .try_clone_with_capacity(additional, || controls.allow_place_attempt())
            .map_err(|CopyAllocation| allocation_failure(span));
    }
    let mut place = RuntimePlace::frame(frame, register);
    place
        .try_reserve_projections(additional, || controls.allow_place_attempt())
        .map_err(|CopyAllocation| allocation_failure(span))?;
    Ok(place)
}

enum RuntimePlaceRef<'place> {
    Borrowed(&'place RuntimePlace),
    Owned(RuntimePlace),
}

impl RuntimePlaceRef<'_> {
    fn as_ref(&self) -> &RuntimePlace {
        match self {
            Self::Borrowed(place) => place,
            Self::Owned(place) => place,
        }
    }

    fn into_owned(
        self,
        span: Span,
        controls: &mut TestControls,
    ) -> Result<RuntimePlace, InterpreterFailure> {
        match self {
            Self::Borrowed(place) => place
                .try_clone_with_capacity(0, || controls.allow_place_attempt())
                .map_err(|CopyAllocation| allocation_failure(span)),
            Self::Owned(place) => Ok(place),
        }
    }

    fn into_projected(
        self,
        projection: RuntimeProjection,
        span: Span,
        controls: &mut TestControls,
    ) -> Result<RuntimePlace, InterpreterFailure> {
        let mut place = match self {
            Self::Borrowed(place) => place
                .try_clone_with_capacity(1, || controls.allow_place_attempt())
                .map_err(|CopyAllocation| allocation_failure(span))?,
            Self::Owned(mut place) => {
                place
                    .try_reserve_projections(1, || controls.allow_place_attempt())
                    .map_err(|CopyAllocation| allocation_failure(span))?;
                place
            }
        };
        place
            .push_reserved(projection)
            .map_err(|CopyAllocation| allocation_failure(span))?;
        Ok(place)
    }
}

fn runtime_place_for_receiver<'a>(
    frames: &'a [Frame],
    receiver: &Receiver,
    span: Span,
    controls: &mut TestControls,
) -> Result<RuntimePlaceRef<'a>, InterpreterFailure> {
    if let Some(place) = frames.last().and_then(|frame| frame.loan(receiver.list)) {
        return Ok(RuntimePlaceRef::Borrowed(place));
    }
    if let Some(source) = &receiver.source {
        runtime_place_for_source(frames, source, span, controls).map(RuntimePlaceRef::Owned)
    } else {
        runtime_place_for_register(frames, receiver.list, span, controls)
            .map(RuntimePlaceRef::Owned)
    }
}

fn with_receiver_place_mut<R>(
    store: &mut Store<EntityPayload>,
    frames: &mut Vec<Frame>,
    receiver: &Receiver,
    span: Span,
    controls: &mut TestControls,
    access: impl FnOnce(
        &RuntimePlace,
        &mut Store<EntityPayload>,
        &mut Vec<Frame>,
        &mut TestControls,
    ) -> Result<R, InterpreterFailure>,
) -> Result<R, InterpreterFailure> {
    let loan = frames
        .last_mut()
        .and_then(|frame| frame.take_loan(receiver.list));
    if let Some(place) = loan {
        let result = access(&place, store, frames, controls);
        let restored = frames
            .last_mut()
            .ok_or_else(|| internal("receiver loan has no active frame"))?
            .set_loan(receiver.list, place)
            .map_err(|()| internal("receiver loan could not be restored"));
        return restored.and(result);
    }
    let place =
        runtime_place_for_receiver(frames, receiver, span, controls)?.into_owned(span, controls)?;
    access(&place, store, frames, controls)
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
        let ValueKind::List(elements) = value.kind() else {
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
    controls: &mut TestControls,
) -> Result<RuntimePlace, InterpreterFailure> {
    let projection_count = source.projections.len();
    let mut place = if source.projections.is_empty() {
        runtime_place_for_register(frames, source.base, span, controls)?
    } else {
        match frames
            .last()
            .and_then(|frame| frame.value(source.base))
            .map(Value::kind)
        {
            Some(ValueKind::Entity(entity)) => {
                let mut place = RuntimePlace::entity(*entity);
                place
                    .try_reserve_projections(projection_count, || controls.allow_place_attempt())
                    .map_err(|CopyAllocation| allocation_failure(span))?;
                place
            }
            _ => runtime_place_for_register_with_capacity(
                frames,
                source.base,
                projection_count,
                span,
                controls,
            )?,
        }
    };
    for projection in &source.projections {
        let projection = match projection {
            ArgumentProjection::Field(field) => RuntimeProjection::Field(*field),
            ArgumentProjection::Index(index) => {
                let raw = expect_int(frame_value(frames, *index)?)?;
                let index = usize::try_from(raw).map_err(|_| bounds_failure(span))?;
                RuntimeProjection::Index(index)
            }
        };
        place
            .push_reserved(projection)
            .map_err(|CopyAllocation| allocation_failure(span))?;
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
    if let Some(place) = frame.loan(register) {
        return with_place_value(module, store, frames, place, span, access);
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
    if let Some(place) = frames[frame_index].take_loan(register) {
        let result = with_place_mut(module, store, frames, &place, span, access);
        let restored = frames[frame_index]
            .set_loan(register, place)
            .map_err(|()| internal("mutable register loan could not be restored"));
        return restored.and(result);
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
                let ValueKind::Struct { definition, fields } = value.kind() else {
                    return Err(internal("loan field projection requires a struct"));
                };
                let index = field_index(module, *definition, *field)?;
                fields
                    .get(index)
                    .ok_or_else(|| internal("struct payload layout is invalid"))?
            }
            RuntimeProjection::Index(index) => {
                let ValueKind::List(elements) = value.kind() else {
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
                let ValueKind::Struct { definition, fields } = value.kind_mut() else {
                    return Err(internal("loan field projection requires a struct"));
                };
                let index = field_index(module, *definition, *field)?;
                fields
                    .get_mut(index)
                    .ok_or_else(|| internal("struct payload layout is invalid"))?
            }
            RuntimeProjection::Index(index) => {
                let ValueKind::List(elements) = value.kind_mut() else {
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
    try_copy_value_with_allocation_controls(value, controls).map_err(|_| allocation_failure(span))
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
    module: &Module,
    frames: &mut [Frame],
    register: Register,
    value: Value,
) -> Result<(), InterpreterFailure> {
    frames
        .last_mut()
        .ok_or_else(|| internal("register write has no active frame"))?
        .set(module, register, value)
        .map_err(|()| internal("runtime value does not match its validated destination"))
}

fn register_type<'module>(
    module: &'module Module,
    frames: &[Frame],
    register: Register,
) -> Result<&'module IrType, InterpreterFailure> {
    let function = frames
        .last()
        .and_then(|frame| module.functions.get(frame.function.0 as usize))
        .ok_or_else(|| internal("register type lookup has no active function"))?;
    function
        .register_types
        .get(register.0 as usize)
        .ok_or_else(|| internal("validated register type is missing"))
}

fn optional_inner(ty: &IrType) -> Result<&IrType, InterpreterFailure> {
    let IrType::Optional(inner) = ty else {
        return Err(internal("validated optional destination is not Optional"));
    };
    Ok(inner)
}

fn take_register(frames: &mut [Frame], register: Register) -> Result<Value, InterpreterFailure> {
    let frame = frames
        .last_mut()
        .ok_or_else(|| internal("register move has no active frame"))?;
    frame
        .take(register)
        .ok_or_else(|| internal("validated source register is undefined or already moved"))
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
    match value.kind() {
        ValueKind::Int(value) => Ok(*value),
        _ => Err(internal(
            "validated Int register contains another value kind",
        )),
    }
}

fn expect_bool(value: &Value) -> Result<bool, InterpreterFailure> {
    match value.kind() {
        ValueKind::Bool(value) => Ok(*value),
        _ => Err(internal(
            "validated Bool register contains another value kind",
        )),
    }
}

fn expect_entity(value: &Value) -> Result<keld_runtime::EntityId, InterpreterFailure> {
    match value.kind() {
        ValueKind::Entity(entity) => Ok(*entity),
        _ => Err(internal(
            "validated entity register contains another value kind",
        )),
    }
}

fn expect_link(value: &Value) -> Result<Option<keld_runtime::Link>, InterpreterFailure> {
    match value.kind() {
        ValueKind::Link(link) => Ok(*link),
        _ => Err(internal(
            "validated link register contains another value kind",
        )),
    }
}

fn expect_lifecycle(value: &Value) -> Result<RuntimeLifecycleId, InterpreterFailure> {
    match value.kind() {
        ValueKind::Lifecycle(lifecycle) => Ok(*lifecycle),
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
    let equality = || match (lhs.kind(), rhs.kind()) {
        (ValueKind::Int(lhs), ValueKind::Int(rhs)) => Ok(lhs == rhs),
        (ValueKind::Bool(lhs), ValueKind::Bool(rhs)) => Ok(lhs == rhs),
        (ValueKind::Text(lhs), ValueKind::Text(rhs)) => Ok(lhs == rhs),
        (ValueKind::Entity(lhs), ValueKind::Entity(rhs)) => Ok(lhs == rhs),
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
