use crate::{
    ArgumentProjection, ArgumentSource, Function, Instruction, IrBlockId, IrDefinition,
    IrDefinitionKind, IrType, Module, Receiver, Register, RegisterStorage, Terminator, ViewId,
    ViewMode,
};
use keld_semantics::{CompareOp, DefId, FieldId};
use keld_source::{Diagnostic, DiagnosticCode, Span};
use std::collections::{BTreeMap, BTreeSet};

const VIEW_ERROR: &str = "KLD9001";
const REGISTER_ERROR: &str = "KLD9002";
const CFG_ERROR: &str = "KLD9003";
const LIFECYCLE_ERROR: &str = "KLD9004";
const MODULE_ERROR: &str = "KLD9005";
const STORAGE_ERROR: &str = "KLD9006";

#[must_use]
pub fn validate(module: &Module) -> Vec<Diagnostic> {
    let mut sink = DiagnosticSink::default();
    validate_module_shape(module, &mut sink);
    for function in &module.functions {
        FunctionValidator::new(module, function, &mut sink).validate();
    }
    sink.finish()
}

#[derive(Default)]
struct DiagnosticSink {
    diagnostics: Vec<Diagnostic>,
    seen: BTreeSet<(&'static str, keld_source::SourceId, u32, u32)>,
}

impl DiagnosticSink {
    fn error(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        let key = (code, span.source(), span.start().0, span.end().0);
        if self.seen.insert(key) {
            let mut diagnostic = Diagnostic::error(DiagnosticCode(code), span, message);
            diagnostic.help = Some("rebuild executable IR from verified Keld flow".to_owned());
            self.diagnostics.push(diagnostic);
        }
    }

    fn finish(mut self) -> Vec<Diagnostic> {
        keld_source::sort_diagnostics(&mut self.diagnostics);
        self.diagnostics
    }
}

fn validate_module_shape(module: &Module, sink: &mut DiagnosticSink) {
    for (index, definition) in module.definitions.iter().enumerate() {
        if definition.id.0 as usize != index {
            sink.error(
                MODULE_ERROR,
                module_span(module),
                "definition IDs must be contiguous and source ordered",
            );
        }
        let mut fields = BTreeSet::new();
        for (field, ty) in &definition.fields {
            if !fields.insert(*field) {
                sink.error(
                    MODULE_ERROR,
                    module_span(module),
                    "definition contains a duplicate field ID",
                );
            }
            validate_named_type(module, ty, module_span(module), sink);
        }
    }
    for (index, function) in module.functions.iter().enumerate() {
        if function.id.0 as usize != index {
            sink.error(
                MODULE_ERROR,
                function.span,
                "function IDs must be contiguous and source ordered",
            );
        }
    }
    let Some(main) = module.functions.get(module.main.0 as usize) else {
        sink.error(
            MODULE_ERROR,
            module_span(module),
            "module main function does not exist",
        );
        return;
    };
    if main.return_type != IrType::Int {
        sink.error(MODULE_ERROR, main.span, "main must return Int");
    }
}

fn validate_named_type(module: &Module, ty: &IrType, span: Span, sink: &mut DiagnosticSink) {
    let (definition, expected_kind) = match ty {
        IrType::Struct(definition) => (*definition, IrDefinitionKind::Struct),
        IrType::Entity(definition)
        | IrType::Link {
            entity: definition, ..
        } => (*definition, IrDefinitionKind::Entity),
        IrType::List(element) | IrType::Optional(element) => {
            validate_named_type(module, element, span, sink);
            return;
        }
        IrType::Unit | IrType::Bool | IrType::Int | IrType::Text | IrType::Lifecycle => return,
    };
    if module
        .definitions
        .get(definition.0 as usize)
        .is_none_or(|candidate| candidate.id != definition || candidate.kind != expected_kind)
    {
        sink.error(
            REGISTER_ERROR,
            span,
            "IR type references an unknown or incompatible definition",
        );
    }
}

fn module_span(module: &Module) -> Span {
    module.functions.first().map_or_else(
        || Span::new(keld_source::SourceId(0), 0, 0).expect("empty fallback span is valid"),
        |function| function.span,
    )
}

struct FunctionValidator<'module, 'sink> {
    module: &'module Module,
    function: &'module Function,
    sink: &'sink mut DiagnosticSink,
    predecessors: Vec<Vec<IrBlockId>>,
    incoming_definitions: Vec<BTreeSet<Register>>,
    outgoing_definitions: Vec<BTreeSet<Register>>,
    incoming_lifecycles: Vec<BTreeSet<Register>>,
    incoming_homes: Vec<BTreeMap<Register, HomeState>>,
    outgoing_homes: Vec<BTreeMap<Register, HomeState>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HomeState {
    Empty,
    Live,
    MaybeLive,
}

impl<'module, 'sink> FunctionValidator<'module, 'sink> {
    fn new(
        module: &'module Module,
        function: &'module Function,
        sink: &'sink mut DiagnosticSink,
    ) -> Self {
        let block_count = function.blocks.len();
        Self {
            module,
            function,
            sink,
            predecessors: vec![Vec::new(); block_count],
            incoming_definitions: vec![BTreeSet::new(); block_count],
            outgoing_definitions: vec![BTreeSet::new(); block_count],
            incoming_lifecycles: vec![BTreeSet::new(); block_count],
            incoming_homes: vec![BTreeMap::new(); block_count],
            outgoing_homes: vec![BTreeMap::new(); block_count],
        }
    }

    fn validate(&mut self) {
        self.validate_shape();
        if self.function.blocks.is_empty()
            || self.function.entry.0 as usize >= self.function.blocks.len()
        {
            return;
        }
        self.build_predecessors();
        self.compute_definition_dataflow();
        self.compute_lifecycle_dataflow();
        self.compute_home_dataflow();
        for block in &self.function.blocks {
            self.validate_block(block.id);
        }
    }

    fn validate_shape(&mut self) {
        if self.function.blocks.is_empty() {
            self.sink.error(
                CFG_ERROR,
                self.function.span,
                "function must contain an entry block",
            );
        }
        if self.function.entry.0 as usize >= self.function.blocks.len() {
            self.sink.error(
                CFG_ERROR,
                self.function.span,
                "function entry block is outside the block table",
            );
        }
        for (index, block) in self.function.blocks.iter().enumerate() {
            if block.id.0 as usize != index {
                self.sink.error(
                    CFG_ERROR,
                    self.function.span,
                    "block IDs must be contiguous",
                );
            }
        }
        for ty in &self.function.register_types {
            validate_named_type(self.module, ty, self.function.span, self.sink);
        }
        if self.function.register_storage.len() != self.function.register_types.len() {
            self.sink.error(
                STORAGE_ERROR,
                self.function.span,
                "register storage roles must align with register types",
            );
        }
        for parent in &self.function.storage_scope_parents {
            if let Some(parent) = parent
                && parent.0 as usize >= self.function.storage_scope_parents.len()
            {
                self.sink.error(
                    STORAGE_ERROR,
                    self.function.span,
                    "storage scope parent is outside the scope table",
                );
            }
        }
        validate_named_type(
            self.module,
            &self.function.return_type,
            self.function.span,
            self.sink,
        );
        let mut predefined = BTreeSet::new();
        for parameter in &self.function.parameters {
            self.check_register(*parameter, self.function.span);
            if !predefined.insert(*parameter) {
                self.sink.error(
                    REGISTER_ERROR,
                    self.function.span,
                    "function parameter register is duplicated",
                );
            }
        }
        self.expect_type(
            self.function.current_lifecycle,
            &IrType::Lifecycle,
            self.function.span,
        );
        if !predefined.insert(self.function.current_lifecycle) {
            self.sink.error(
                REGISTER_ERROR,
                self.function.span,
                "current lifecycle register overlaps a source parameter",
            );
        }
        self.validate_unique_definitions(&predefined);
    }

    fn validate_unique_definitions(&mut self, predefined: &BTreeSet<Register>) {
        let mut defined = predefined.clone();
        for block in &self.function.blocks {
            for instruction in &block.instructions {
                if let Some(destination) = instruction_destination(instruction)
                    && !defined.insert(destination)
                {
                    self.sink.error(
                        REGISTER_ERROR,
                        instruction_span(instruction),
                        "register is defined more than once",
                    );
                }
            }
            if let Terminator::ResolveLink {
                live_value, span, ..
            } = block.terminator
                && !defined.insert(live_value)
            {
                self.sink.error(
                    REGISTER_ERROR,
                    span,
                    "link resolution destination is defined more than once",
                );
            }
        }
    }

    fn build_predecessors(&mut self) {
        for block in &self.function.blocks {
            for target in terminator_targets(&block.terminator) {
                let Some(predecessors) = self.predecessors.get_mut(target.0 as usize) else {
                    self.sink.error(
                        CFG_ERROR,
                        terminator_span(&block.terminator, self.function.span),
                        "terminator targets a block outside the function",
                    );
                    continue;
                };
                predecessors.push(block.id);
            }
        }
        for predecessors in &mut self.predecessors {
            predecessors.sort_unstable();
        }
    }

    fn compute_definition_dataflow(&mut self) {
        let predefined = self
            .function
            .parameters
            .iter()
            .copied()
            .chain(std::iter::once(self.function.current_lifecycle))
            .collect::<BTreeSet<_>>();
        let mut universe = predefined.clone();
        for block in &self.function.blocks {
            for instruction in &block.instructions {
                if let Some(destination) = instruction_destination(instruction) {
                    universe.insert(destination);
                }
                if let Instruction::InstallHome { destination, .. } = instruction {
                    universe.insert(*destination);
                }
            }
            if let Terminator::ResolveLink { live_value, .. } = block.terminator {
                universe.insert(live_value);
            }
        }
        for block in &self.function.blocks {
            self.incoming_definitions[block.id.0 as usize] = if block.id == self.function.entry {
                predefined.clone()
            } else {
                universe.clone()
            };
            let mut outgoing = self.incoming_definitions[block.id.0 as usize].clone();
            add_block_definitions(block, &mut outgoing);
            self.outgoing_definitions[block.id.0 as usize] = outgoing;
        }

        let iteration_limit = self
            .function
            .blocks
            .len()
            .saturating_mul(self.function.register_types.len().saturating_add(1));
        for _ in 0..=iteration_limit {
            let mut changed = false;
            for block in &self.function.blocks {
                let index = block.id.0 as usize;
                let incoming = if block.id == self.function.entry {
                    predefined.clone()
                } else {
                    self.intersect_predecessor_definitions(block.id)
                };
                let mut outgoing = incoming.clone();
                add_block_definitions(block, &mut outgoing);
                changed |= incoming != self.incoming_definitions[index]
                    || outgoing != self.outgoing_definitions[index];
                self.incoming_definitions[index] = incoming;
                self.outgoing_definitions[index] = outgoing;
            }
            if !changed {
                break;
            }
        }
    }

    fn intersect_predecessor_definitions(&self, block: IrBlockId) -> BTreeSet<Register> {
        let mut edges = self.predecessors[block.0 as usize]
            .iter()
            .map(|predecessor| {
                let mut definitions = self.outgoing_definitions[predecessor.0 as usize].clone();
                if let Terminator::ResolveLink {
                    live_value, live, ..
                } = self.function.blocks[predecessor.0 as usize].terminator
                    && live == block
                {
                    definitions.insert(live_value);
                }
                definitions
            });
        let Some(first) = edges.next() else {
            return BTreeSet::new();
        };
        edges.fold(first, |current, edge| {
            current.intersection(&edge).copied().collect()
        })
    }

    fn compute_lifecycle_dataflow(&mut self) {
        let all_lifecycles = self
            .function
            .register_types
            .iter()
            .enumerate()
            .filter(|(_, ty)| *ty == &IrType::Lifecycle)
            .map(|(index, _)| Register(u32::try_from(index).expect("register index fits in u32")))
            .collect::<BTreeSet<_>>();
        let root = BTreeSet::from([self.function.current_lifecycle]);
        let mut outgoing = vec![BTreeSet::new(); self.function.blocks.len()];
        for block in &self.function.blocks {
            self.incoming_lifecycles[block.id.0 as usize] = if block.id == self.function.entry {
                root.clone()
            } else {
                all_lifecycles.clone()
            };
            outgoing[block.id.0 as usize] = transfer_lifecycles(
                &self.incoming_lifecycles[block.id.0 as usize],
                &block.instructions,
            );
        }
        let iteration_limit = self
            .function
            .blocks
            .len()
            .saturating_mul(all_lifecycles.len().saturating_add(1));
        for _ in 0..=iteration_limit {
            let mut changed = false;
            for block in &self.function.blocks {
                let index = block.id.0 as usize;
                let incoming = if block.id == self.function.entry {
                    root.clone()
                } else {
                    intersect_sets(
                        self.predecessors[index]
                            .iter()
                            .map(|predecessor| outgoing[predecessor.0 as usize].clone()),
                    )
                };
                let next = transfer_lifecycles(&incoming, &block.instructions);
                changed |= incoming != self.incoming_lifecycles[index] || next != outgoing[index];
                self.incoming_lifecycles[index] = incoming;
                outgoing[index] = next;
            }
            if !changed {
                break;
            }
        }
    }

    fn compute_home_dataflow(&mut self) {
        let initial = self.initial_home_state();
        for block in &self.function.blocks {
            self.incoming_homes[block.id.0 as usize] = if block.id == self.function.entry {
                initial.clone()
            } else {
                self.all_home_states(HomeState::Empty)
            };
            self.outgoing_homes[block.id.0 as usize] = transfer_home_states(
                &self.incoming_homes[block.id.0 as usize],
                &block.instructions,
                &self.function.register_storage,
                self.module,
            );
        }
        let iteration_limit = self
            .function
            .blocks
            .len()
            .saturating_mul(self.function.register_types.len().saturating_add(1));
        for _ in 0..=iteration_limit {
            let mut changed = false;
            for block in &self.function.blocks {
                let index = block.id.0 as usize;
                let incoming = if block.id == self.function.entry {
                    initial.clone()
                } else {
                    self.join_predecessor_homes(block.id)
                };
                let outgoing = transfer_home_states(
                    &incoming,
                    &block.instructions,
                    &self.function.register_storage,
                    self.module,
                );
                changed |= incoming != self.incoming_homes[index]
                    || outgoing != self.outgoing_homes[index];
                self.incoming_homes[index] = incoming;
                self.outgoing_homes[index] = outgoing;
            }
            if !changed {
                break;
            }
        }
    }

    fn initial_home_state(&self) -> BTreeMap<Register, HomeState> {
        let mut state = self.all_home_states(HomeState::Empty);
        for parameter in &self.function.parameters {
            if matches!(
                self.register_storage(*parameter),
                Some(RegisterStorage::Home { .. })
            ) {
                state.insert(*parameter, HomeState::Live);
            }
        }
        state
    }

    fn all_home_states(&self, value: HomeState) -> BTreeMap<Register, HomeState> {
        self.function
            .register_storage
            .iter()
            .enumerate()
            .filter(|(_, storage)| {
                matches!(
                    storage,
                    RegisterStorage::Home { .. } | RegisterStorage::DropSlot
                )
            })
            .map(|(index, _)| {
                (
                    Register(u32::try_from(index).expect("register index fits in u32")),
                    value,
                )
            })
            .collect()
    }

    fn join_predecessor_homes(&self, block: IrBlockId) -> BTreeMap<Register, HomeState> {
        let mut joined = self.all_home_states(HomeState::Empty);
        for register in joined.keys().copied().collect::<Vec<_>>() {
            let mut state = None;
            for predecessor in &self.predecessors[block.0 as usize] {
                let predecessor_state = self.outgoing_homes[predecessor.0 as usize]
                    .get(&register)
                    .copied()
                    .unwrap_or(HomeState::Empty);
                state = Some(match state {
                    Some(current) => join_home_state(current, predecessor_state),
                    None => predecessor_state,
                });
            }
            if let Some(state) = state {
                joined.insert(register, state);
            }
        }
        joined
    }

    fn register_storage(&self, register: Register) -> Option<&RegisterStorage> {
        self.function.register_storage.get(register.0 as usize)
    }

    fn validate_block(&mut self, block_id: IrBlockId) {
        let block = &self.function.blocks[block_id.0 as usize];
        let mut available = self.incoming_definitions[block_id.0 as usize].clone();
        let mut active_lifecycles = self.incoming_lifecycles[block_id.0 as usize].clone();
        let mut home_states = self.incoming_homes[block_id.0 as usize].clone();
        let mut views = BTreeMap::<ViewId, (ViewMode, DefId)>::new();
        let mut passed_phi_group = false;

        self.validate_lifecycle_join(block_id);
        for instruction in &block.instructions {
            let span = instruction_span(instruction);
            if matches!(instruction, Instruction::Phi { .. }) {
                if passed_phi_group || block_id == self.function.entry {
                    self.sink.error(
                        CFG_ERROR,
                        span,
                        "Phi instructions must form the leading group of a non-entry block",
                    );
                }
                self.validate_phi(block_id, instruction);
            } else {
                passed_phi_group = true;
                for register in instruction_uses(instruction) {
                    self.require_available(register, &available, span);
                }
            }

            if is_structural(instruction) && !views.is_empty() {
                self.sink.error(
                    VIEW_ERROR,
                    span,
                    "structural operation cannot execute while a field view is open",
                );
            }
            self.validate_instruction(instruction, &mut views, &mut active_lifecycles);
            add_instruction_definitions(instruction, &mut available);
            self.validate_home_instruction(instruction, &mut home_states, &views);
        }
        let terminator_span = terminator_span(&block.terminator, self.function.span);
        if !views.is_empty() {
            self.sink.error(
                VIEW_ERROR,
                terminator_span,
                "all field views must close before a terminator",
            );
        }
        for register in terminator_uses(&block.terminator) {
            self.require_available(register, &available, terminator_span);
        }
        self.validate_home_terminator(&block.terminator, &home_states, terminator_span);
        self.validate_terminator(&block.terminator, &active_lifecycles);
    }

    #[allow(clippy::too_many_lines)]
    fn validate_home_instruction(
        &mut self,
        instruction: &Instruction,
        state: &mut BTreeMap<Register, HomeState>,
        views: &BTreeMap<ViewId, (ViewMode, DefId)>,
    ) {
        let span = instruction_span(instruction);
        let storage = self.function.register_storage.clone();
        let role = |register: Register| storage.get(register.0 as usize).cloned();
        let home_state =
            |register: Register| state.get(&register).copied().unwrap_or(HomeState::Empty);
        let require_role = |sink: &mut DiagnosticSink,
                            register: Register,
                            expected: fn(&RegisterStorage) -> bool,
                            message: &str| {
            if role(register).is_some_and(|candidate| expected(&candidate)) {
                true
            } else {
                sink.error(STORAGE_ERROR, span, message);
                false
            }
        };
        let require_live = |sink: &mut DiagnosticSink, register: Register, message: &str| {
            if home_state(register) == HomeState::Live {
                true
            } else {
                sink.error(STORAGE_ERROR, span, message);
                false
            }
        };
        let require_empty = |sink: &mut DiagnosticSink, register: Register, message: &str| {
            if home_state(register) == HomeState::Empty {
                true
            } else {
                sink.error(STORAGE_ERROR, span, message);
                false
            }
        };

        match instruction {
            Instruction::InstallHome {
                destination,
                source,
                displaced,
                ..
            } => {
                let destination_ok = require_role(
                    self.sink,
                    *destination,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "home installation destination must be a Home register",
                );
                let source_ok = require_role(
                    self.sink,
                    *source,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "home installation source must be a Home register",
                );
                let displaced_ok = require_role(
                    self.sink,
                    *displaced,
                    |role| matches!(role, RegisterStorage::DropSlot),
                    "home installation displaced register must be a DropSlot",
                );
                if destination == source {
                    self.sink.error(
                        STORAGE_ERROR,
                        span,
                        "home installation cannot move a home into itself",
                    );
                }
                if source_ok {
                    require_live(self.sink, *source, "home installation source is empty");
                }
                if displaced_ok {
                    require_empty(self.sink, *displaced, "displaced slot is already live");
                }
                let _ = destination_ok;
            }
            Instruction::MoveHome {
                destination,
                source,
                ..
            } => {
                let destination_role = role(*destination);
                let source_role = role(*source);
                if matches!(destination_role, Some(RegisterStorage::Loan))
                    || matches!(source_role, Some(RegisterStorage::Loan))
                {
                    self.sink
                        .error(STORAGE_ERROR, span, "loan register used as an owned source");
                }
                require_role(
                    self.sink,
                    *destination,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "home move destination must be a Home register",
                );
                let source_ok = require_role(
                    self.sink,
                    *source,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "home move source must be a Home register",
                );
                if destination == source {
                    self.sink.error(
                        STORAGE_ERROR,
                        span,
                        "home move cannot move a home into itself",
                    );
                }
                if source_ok {
                    require_live(self.sink, *source, "move of empty home");
                }
                require_empty(
                    self.sink,
                    *destination,
                    "home move destination is already live",
                );
            }
            Instruction::DropHome { home, .. } => {
                let home_ok = require_role(
                    self.sink,
                    *home,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "home drop register must be a Home",
                );
                if home_ok {
                    require_live(self.sink, *home, "drop of empty home");
                }
            }
            Instruction::DropIfLive { home, .. } => {
                require_role(
                    self.sink,
                    *home,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "conditional drop register must be a Home",
                );
            }
            Instruction::DropSlot { slot, .. } => {
                let slot_ok = require_role(
                    self.sink,
                    *slot,
                    |role| matches!(role, RegisterStorage::DropSlot),
                    "drop slot register must be a DropSlot",
                );
                if slot_ok && home_state(*slot) == HomeState::Empty {
                    self.sink.error(STORAGE_ERROR, span, "drop of empty home");
                }
            }
            Instruction::CleanupTrackedScope { scope, .. } => {
                if scope.0 as usize >= self.function.storage_scope_parents.len() {
                    self.sink
                        .error(STORAGE_ERROR, span, "unknown cleanup scope");
                }
            }
            Instruction::ReplacePlace {
                destination,
                source,
                displaced,
                ..
            } => {
                let destination_type = self.validate_argument_source(destination, span);
                if let Some(destination_type) = destination_type {
                    self.expect_type(*source, &destination_type, span);
                    self.expect_type(*displaced, &destination_type, span);
                } else {
                    self.check_register(*source, span);
                    self.check_register(*displaced, span);
                }
                let source_role = role(*source);
                let source_ok = !matches!(source_role, Some(RegisterStorage::Loan) | None);
                if !source_ok {
                    self.sink.error(
                        STORAGE_ERROR,
                        span,
                        "place replacement source must be owned or trivial",
                    );
                }
                let displaced_ok = require_role(
                    self.sink,
                    *displaced,
                    |role| matches!(role, RegisterStorage::DropSlot),
                    "place replacement displaced register must be a DropSlot",
                );
                if source_ok && matches!(source_role, Some(RegisterStorage::Home { .. })) {
                    require_live(self.sink, *source, "place replacement source is empty");
                }
                if displaced_ok {
                    require_empty(self.sink, *displaced, "displaced slot is already live");
                }
            }
            Instruction::ReplaceField {
                view,
                field: _,
                source,
                displaced,
                ..
            } => {
                if !matches!(views.get(view), Some((ViewMode::Edit, _))) {
                    self.sink.error(
                        STORAGE_ERROR,
                        span,
                        "field replacement requires an open edit view",
                    );
                }
                let source_ok = require_role(
                    self.sink,
                    *source,
                    |role| matches!(role, RegisterStorage::Home { .. }),
                    "field replacement source must be a Home register",
                );
                let displaced_ok = require_role(
                    self.sink,
                    *displaced,
                    |role| matches!(role, RegisterStorage::DropSlot),
                    "field replacement displaced register must be a DropSlot",
                );
                if source_ok {
                    require_live(self.sink, *source, "field replacement source is empty");
                }
                if displaced_ok {
                    require_empty(self.sink, *displaced, "displaced slot is already live");
                }
            }
            Instruction::ListReplace {
                value, displaced, ..
            } => {
                let source_role = role(*value);
                let source_ok = !matches!(source_role, Some(RegisterStorage::Loan) | None);
                if !source_ok {
                    self.sink.error(
                        STORAGE_ERROR,
                        span,
                        "indexed replacement source must be owned or trivial",
                    );
                }
                if source_ok && matches!(source_role, Some(RegisterStorage::Home { .. })) {
                    require_live(self.sink, *value, "indexed replacement source is empty");
                }
                let displaced_ok = require_role(
                    self.sink,
                    *displaced,
                    |role| matches!(role, RegisterStorage::DropSlot),
                    "indexed replacement displaced register must be a DropSlot",
                );
                if displaced_ok {
                    require_empty(self.sink, *displaced, "displaced slot is already live");
                }
            }
            _ => {}
        }
        *state = transfer_home_states(
            state,
            std::slice::from_ref(instruction),
            &self.function.register_storage,
            self.module,
        );
    }

    fn validate_home_terminator(
        &mut self,
        terminator: &Terminator,
        state: &BTreeMap<Register, HomeState>,
        span: Span,
    ) {
        let Terminator::Return(value) = terminator else {
            return;
        };
        for (register, home_state) in state {
            if Some(*register) == *value {
                continue;
            }
            if !matches!(home_state, HomeState::Live | HomeState::MaybeLive) {
                continue;
            }
            let message = match self.register_storage(*register) {
                Some(RegisterStorage::DropSlot) => "live displaced value at return",
                Some(RegisterStorage::Home { .. }) => "live managed home at return",
                _ => continue,
            };
            self.sink.error(STORAGE_ERROR, span, message);
        }
    }

    fn validate_lifecycle_join(&mut self, block: IrBlockId) {
        let predecessors = &self.predecessors[block.0 as usize];
        let Some(first) = predecessors.first() else {
            return;
        };
        let first_state = transfer_lifecycles(
            &self.incoming_lifecycles[first.0 as usize],
            &self.function.blocks[first.0 as usize].instructions,
        );
        if predecessors.iter().skip(1).any(|predecessor| {
            transfer_lifecycles(
                &self.incoming_lifecycles[predecessor.0 as usize],
                &self.function.blocks[predecessor.0 as usize].instructions,
            ) != first_state
        }) {
            self.sink.error(
                LIFECYCLE_ERROR,
                self.function.span,
                "control-flow join has inconsistent active lifecycles",
            );
        }
    }

    fn validate_phi(&mut self, block: IrBlockId, instruction: &Instruction) {
        let Instruction::Phi { dst, inputs, span } = instruction else {
            return;
        };
        let expected = self.predecessors[block.0 as usize]
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let actual = inputs
            .iter()
            .map(|(predecessor, _)| *predecessor)
            .collect::<BTreeSet<_>>();
        if inputs.len() != expected.len() || actual != expected {
            self.sink.error(
                CFG_ERROR,
                *span,
                "Phi must have exactly one input from every predecessor",
            );
        }
        let destination_type = self.register_type(*dst, *span).cloned();
        for (predecessor, register) in inputs {
            let Some(predecessor_block) = self.function.blocks.get(predecessor.0 as usize) else {
                self.sink
                    .error(CFG_ERROR, *span, "Phi references an unknown predecessor");
                continue;
            };
            let mut edge_available = self.outgoing_definitions[predecessor.0 as usize].clone();
            if let Terminator::ResolveLink {
                live_value, live, ..
            } = predecessor_block.terminator
                && live == block
            {
                edge_available.insert(live_value);
            }
            self.require_available(*register, &edge_available, *span);
            if let Some(destination_type) = &destination_type
                && self.register_type(*register, *span) != Some(destination_type)
            {
                self.sink.error(
                    REGISTER_ERROR,
                    *span,
                    "Phi input type does not match its destination",
                );
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn validate_instruction(
        &mut self,
        instruction: &Instruction,
        views: &mut BTreeMap<ViewId, (ViewMode, DefId)>,
        active_lifecycles: &mut BTreeSet<Register>,
    ) {
        let span = instruction_span(instruction);
        match instruction {
            Instruction::ConstInt { dst, .. } => self.expect_type(*dst, &IrType::Int, span),
            Instruction::ConstBool { dst, .. } => self.expect_type(*dst, &IrType::Bool, span),
            Instruction::ConstText { dst, .. } => self.expect_type(*dst, &IrType::Text, span),
            Instruction::ConstNoneLink { dst, entity, .. } => self.expect_type(
                *dst,
                &IrType::Link {
                    entity: *entity,
                    optional: true,
                },
                span,
            ),
            Instruction::Copy { dst, src, .. } | Instruction::Take { dst, src, .. } => {
                self.expect_same_type(*dst, *src, span);
            }
            Instruction::ListNew { dst, .. } => {
                if !matches!(self.register_type(*dst, span), Some(IrType::List(_))) {
                    self.sink.error(
                        "KLD9006",
                        span,
                        "list construction destination must have a List type",
                    );
                }
            }
            Instruction::ListLength { dst, list, .. } => {
                self.expect_type(*dst, &IrType::Int, span);
                if !matches!(self.register_type(*list, span), Some(IrType::List(_))) {
                    self.sink
                        .error("KLD9006", span, "list length requires a List value");
                }
            }
            Instruction::ListPush { list, value, .. } => {
                let Some(IrType::List(element)) = self.register_type(*list, span).cloned() else {
                    self.sink
                        .error("KLD9006", span, "list push requires a List value");
                    self.check_register(*value, span);
                    return;
                };
                self.expect_type(*value, &element, span);
            }
            Instruction::ListPushPlace {
                list,
                source,
                value,
                ..
            } => {
                let Some(IrType::List(element)) = self.register_type(*list, span).cloned() else {
                    self.sink
                        .error("KLD9006", span, "list push requires a List value");
                    self.check_register(source.base, span);
                    self.check_register(*value, span);
                    return;
                };
                if self.validate_argument_source(source, span)
                    != Some(IrType::List(element.clone()))
                {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "list push source does not resolve to the receiver List",
                    );
                }
                self.expect_type(*value, &element, span);
            }
            Instruction::ListRemove {
                dst, list, index, ..
            } => {
                let Some(IrType::List(element)) = self.register_type(*list, span).cloned() else {
                    self.sink
                        .error("KLD9006", span, "list remove requires a List value");
                    self.check_register(*dst, span);
                    self.check_register(*index, span);
                    return;
                };
                self.expect_type(*dst, &element, span);
                self.expect_type(*index, &IrType::Int, span);
            }
            Instruction::ListRemovePlace {
                dst,
                list,
                source,
                index,
                ..
            } => {
                let Some(IrType::List(element)) = self.register_type(*list, span).cloned() else {
                    self.sink
                        .error("KLD9006", span, "list remove requires a List value");
                    self.check_register(*dst, span);
                    self.check_register(source.base, span);
                    self.check_register(*index, span);
                    return;
                };
                self.expect_type(*dst, &element, span);
                if self.validate_argument_source(source, span)
                    != Some(IrType::List(element.clone()))
                {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "list remove source does not resolve to the receiver List",
                    );
                }
                self.expect_type(*index, &IrType::Int, span);
            }
            Instruction::ListIndex {
                dst,
                receiver,
                index,
                ..
            } => {
                let Some(element) = self.validate_receiver(receiver, span) else {
                    self.check_register(*dst, span);
                    self.check_register(*index, span);
                    return;
                };
                self.expect_type(*index, &IrType::Int, span);
                self.expect_type(*dst, &element, span);
                if !is_implicit_copy_type(self.module, &element)
                    && !matches!(self.register_storage(*dst), Some(RegisterStorage::Loan))
                {
                    self.sink.error(
                        STORAGE_ERROR,
                        span,
                        "non-copyable List index results must be Loan registers",
                    );
                }
            }
            Instruction::ListGet {
                dst,
                receiver,
                index,
                ..
            } => {
                let Some(element) = self.validate_receiver(receiver, span) else {
                    self.check_register(*dst, span);
                    self.check_register(*index, span);
                    return;
                };
                self.expect_type(*index, &IrType::Int, span);
                if !is_implicit_copy_type(self.module, &element) {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "List get requires an implicitly copyable element",
                    );
                }
                self.expect_type(*dst, &IrType::Optional(Box::new(element)), span);
            }
            Instruction::ListReplace {
                receiver,
                index,
                value,
                displaced,
                ..
            } => {
                let Some(element) = self.validate_receiver(receiver, span) else {
                    self.check_register(*index, span);
                    self.check_register(*value, span);
                    self.check_register(*displaced, span);
                    return;
                };
                self.expect_type(*index, &IrType::Int, span);
                self.expect_type(*value, &element, span);
                self.expect_type(*displaced, &element, span);
            }
            Instruction::TextByteLength { dst, text, .. } => {
                self.expect_type(*dst, &IrType::Int, span);
                self.expect_type(*text, &IrType::Text, span);
            }
            Instruction::TextIsEmpty { dst, text, .. } => {
                self.expect_type(*dst, &IrType::Bool, span);
                self.expect_type(*text, &IrType::Text, span);
            }
            Instruction::TextConcat { dst, lhs, rhs, .. } => {
                self.expect_type(*dst, &IrType::Text, span);
                self.expect_type(*lhs, &IrType::Text, span);
                self.expect_type(*rhs, &IrType::Text, span);
            }
            Instruction::CheckedUnaryInt { dst, src, .. } => {
                self.expect_type(*dst, &IrType::Int, span);
                self.expect_type(*src, &IrType::Int, span);
            }
            Instruction::CheckedBinaryInt { dst, lhs, rhs, .. } => {
                self.expect_type(*dst, &IrType::Int, span);
                self.expect_type(*lhs, &IrType::Int, span);
                self.expect_type(*rhs, &IrType::Int, span);
            }
            Instruction::Not { dst, src, .. } => {
                self.expect_type(*dst, &IrType::Bool, span);
                self.expect_type(*src, &IrType::Bool, span);
            }
            Instruction::Compare {
                dst, op, lhs, rhs, ..
            } => self.validate_compare(*dst, *op, *lhs, *rhs, span),
            Instruction::Phi { dst, .. } => {
                self.check_register(*dst, span);
            }
            Instruction::ConstructStruct {
                dst,
                definition,
                fields,
                ..
            } => {
                self.expect_type(*dst, &IrType::Struct(*definition), span);
                self.validate_fields(*definition, IrDefinitionKind::Struct, fields, span);
            }
            Instruction::ReadStructField {
                dst, base, field, ..
            } => self.validate_struct_read(*dst, *base, *field, span),
            Instruction::InstallHome {
                destination,
                source,
                displaced,
                ..
            } => {
                self.expect_same_type(*destination, *source, span);
                self.expect_same_type(*destination, *displaced, span);
            }
            Instruction::MoveHome {
                destination,
                source,
                ..
            } => self.expect_same_type(*destination, *source, span),
            Instruction::DropHome { home, .. } | Instruction::DropIfLive { home, .. } => {
                self.check_register(*home, span);
            }
            Instruction::DropSlot { slot, .. } => self.check_register(*slot, span),
            Instruction::CleanupTrackedScope { .. } => {}
            Instruction::ReplacePlace {
                destination,
                source,
                displaced,
                ..
            } => {
                self.expect_same_type(destination.base, *source, span);
                self.expect_same_type(destination.base, *displaced, span);
            }
            Instruction::ReplaceField {
                source, displaced, ..
            } => {
                self.check_register(*source, span);
                self.expect_same_type(*source, *displaced, span);
            }
            _ => self.validate_effect_instruction(instruction, views, active_lifecycles),
        }
    }

    fn validate_effect_instruction(
        &mut self,
        instruction: &Instruction,
        views: &mut BTreeMap<ViewId, (ViewMode, DefId)>,
        active_lifecycles: &mut BTreeSet<Register>,
    ) {
        let span = instruction_span(instruction);
        match instruction {
            Instruction::BeginLifecycle { dst, parent, .. } => {
                self.expect_type(*dst, &IrType::Lifecycle, span);
                self.expect_type(*parent, &IrType::Lifecycle, span);
                if !active_lifecycles.contains(parent) || active_lifecycles.contains(dst) {
                    self.sink.error(
                        LIFECYCLE_ERROR,
                        span,
                        "lifecycle begin requires a live parent and an inactive destination",
                    );
                }
                active_lifecycles.insert(*dst);
            }
            Instruction::EndLifecycle { lifecycle, .. } => {
                self.expect_type(*lifecycle, &IrType::Lifecycle, span);
                if *lifecycle == self.function.current_lifecycle
                    || !active_lifecycles.remove(lifecycle)
                {
                    self.sink.error(
                        LIFECYCLE_ERROR,
                        span,
                        "only an active non-root lifecycle can end",
                    );
                }
            }
            Instruction::AllocateEntity {
                dst,
                definition,
                fields,
                lifecycle,
                ..
            } => {
                self.expect_type(*dst, &IrType::Entity(*definition), span);
                self.require_active_lifecycle(*lifecycle, active_lifecycles, span);
                self.validate_fields(*definition, IrDefinitionKind::Entity, fields, span);
            }
            Instruction::EntityToLink { dst, entity, .. } => {
                let Some(IrType::Entity(definition)) = self.register_type(*entity, span).cloned()
                else {
                    self.sink
                        .error(REGISTER_ERROR, span, "link conversion requires an entity");
                    return;
                };
                match self.register_type(*dst, span) {
                    Some(IrType::Link { entity, .. }) if *entity == definition => {}
                    _ => self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "link result type does not match the entity",
                    ),
                }
            }
            _ => self.validate_view_instruction(instruction, views, active_lifecycles),
        }
    }

    fn validate_view_instruction(
        &mut self,
        instruction: &Instruction,
        views: &mut BTreeMap<ViewId, (ViewMode, DefId)>,
        active_lifecycles: &BTreeSet<Register>,
    ) {
        let span = instruction_span(instruction);
        match instruction {
            Instruction::OpenView {
                view, entity, mode, ..
            } => self.open_view(*view, *entity, *mode, span, views),
            Instruction::ReadField {
                dst, view, field, ..
            } => {
                self.validate_view_read(*dst, *view, *field, span, views);
            }
            Instruction::WriteField {
                view, field, value, ..
            } => self.validate_view_write(*view, *field, *value, span, views),
            Instruction::CloseView { view, .. } => {
                if views.remove(view).is_none() {
                    self.sink
                        .error(VIEW_ERROR, span, "cannot close an unknown field view");
                }
            }
            Instruction::KeepEntity {
                entity, lifecycle, ..
            } => {
                self.expect_entity(*entity, span);
                self.require_active_lifecycle(*lifecycle, active_lifecycles, span);
            }
            Instruction::RetireEntity { entity, .. } => self.expect_entity(*entity, span),
            Instruction::Call {
                dst,
                function,
                arguments,
                argument_sources,
                current_lifecycle,
                ..
            } => self.validate_call(
                *dst,
                *function,
                arguments,
                argument_sources,
                *current_lifecycle,
                active_lifecycles,
                span,
            ),
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
            | Instruction::EntityToLink { .. } => {
                unreachable!("instruction handled before validate_view_instruction")
            }
        }
    }

    fn validate_compare(
        &mut self,
        dst: Register,
        op: CompareOp,
        lhs: Register,
        rhs: Register,
        span: Span,
    ) {
        self.expect_type(dst, &IrType::Bool, span);
        self.expect_same_type(lhs, rhs, span);
        if matches!(
            op,
            CompareOp::Less | CompareOp::LessEq | CompareOp::Greater | CompareOp::GreaterEq
        ) {
            self.expect_type(lhs, &IrType::Int, span);
        } else if !matches!(
            self.register_type(lhs, span),
            Some(IrType::Int | IrType::Bool | IrType::Text | IrType::Entity(_))
        ) {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "equality comparison requires Int, Bool, Text, or entity operands",
            );
        }
    }

    fn validate_fields(
        &mut self,
        definition: DefId,
        kind: IrDefinitionKind,
        fields: &[(FieldId, Register)],
        span: Span,
    ) {
        let Some(definition) = self.definition(definition, kind, span).cloned() else {
            return;
        };
        let expected = definition
            .fields
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>();
        let actual = fields
            .iter()
            .map(|(field, _)| *field)
            .collect::<BTreeSet<_>>();
        if actual.len() != fields.len()
            || actual != expected.keys().copied().collect::<BTreeSet<_>>()
        {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "constructor must supply every declared field exactly once",
            );
        }
        for (field, register) in fields {
            if let Some(expected) = expected.get(field) {
                self.expect_type(*register, expected, span);
            }
        }
    }

    fn validate_struct_read(&mut self, dst: Register, base: Register, field: FieldId, span: Span) {
        let Some(IrType::Struct(definition)) = self.register_type(base, span).cloned() else {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "struct field read requires a struct base",
            );
            return;
        };
        if let Some(field_type) = self.field_type(definition, field, span).cloned() {
            self.expect_type(dst, &field_type, span);
        }
    }

    fn open_view(
        &mut self,
        view: ViewId,
        entity: Register,
        mode: ViewMode,
        span: Span,
        views: &mut BTreeMap<ViewId, (ViewMode, DefId)>,
    ) {
        let Some(IrType::Entity(definition)) = self.register_type(entity, span).cloned() else {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "field view requires an entity register",
            );
            return;
        };
        if views.insert(view, (mode, definition)).is_some() {
            self.sink
                .error(VIEW_ERROR, span, "field view ID is already open");
        }
    }

    fn validate_view_read(
        &mut self,
        dst: Register,
        view: ViewId,
        field: FieldId,
        span: Span,
        views: &BTreeMap<ViewId, (ViewMode, DefId)>,
    ) {
        let Some((_, definition)) = views.get(&view) else {
            self.sink
                .error(VIEW_ERROR, span, "read uses an unknown field view");
            return;
        };
        if let Some(field_type) = self.field_type(*definition, field, span).cloned() {
            self.expect_type(dst, &field_type, span);
        }
    }

    fn validate_view_write(
        &mut self,
        view: ViewId,
        field: FieldId,
        value: Register,
        span: Span,
        views: &BTreeMap<ViewId, (ViewMode, DefId)>,
    ) {
        let Some((mode, definition)) = views.get(&view) else {
            self.sink
                .error(VIEW_ERROR, span, "write uses an unknown field view");
            return;
        };
        if *mode != ViewMode::Edit {
            self.sink
                .error(VIEW_ERROR, span, "writing requires an edit view");
        }
        if let Some(field_type) = self.field_type(*definition, field, span).cloned() {
            self.expect_type(value, &field_type, span);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_call(
        &mut self,
        dst: Option<Register>,
        function: keld_semantics::FunctionId,
        arguments: &[(keld_semantics::ParameterIndex, Register)],
        argument_sources: &[(
            keld_semantics::ParameterIndex,
            Option<crate::ArgumentSource>,
        )],
        current_lifecycle: Register,
        active_lifecycles: &BTreeSet<Register>,
        span: Span,
    ) {
        self.require_active_lifecycle(current_lifecycle, active_lifecycles, span);
        for (_, source) in argument_sources {
            if let Some(source) = source {
                self.check_register(source.base, span);
            }
        }
        let Some(callee) = self.module.functions.get(function.0 as usize).cloned() else {
            self.sink
                .error(REGISTER_ERROR, span, "call targets an unknown function");
            return;
        };
        let actual = arguments
            .iter()
            .map(|(parameter, _)| parameter.0)
            .collect::<BTreeSet<_>>();
        let expected = (0..callee.parameters.len())
            .map(|index| u32::try_from(index).expect("parameter index fits in u32"))
            .collect::<BTreeSet<_>>();
        if actual.len() != arguments.len() || actual != expected {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "call must supply every parameter exactly once",
            );
        }
        for (parameter, argument) in arguments {
            if let Some(callee_register) = callee.parameters.get(parameter.0 as usize)
                && let Some(expected_type) = callee.register_types.get(callee_register.0 as usize)
            {
                self.expect_type(*argument, expected_type, span);
            }
        }
        match (&callee.return_type, dst) {
            (IrType::Unit, None) => {}
            (IrType::Unit, Some(_)) | (_, None) => self.sink.error(
                REGISTER_ERROR,
                span,
                "call result presence does not match the callee return type",
            ),
            (return_type, Some(destination)) => self.expect_type(destination, return_type, span),
        }
    }

    fn validate_terminator(
        &mut self,
        terminator: &Terminator,
        active_lifecycles: &BTreeSet<Register>,
    ) {
        let span = terminator_span(terminator, self.function.span);
        match terminator {
            Terminator::Goto(_) | Terminator::Unreachable | Terminator::Fault { .. } => {}
            Terminator::Branch { condition, .. } => {
                self.expect_type(*condition, &IrType::Bool, span);
            }
            Terminator::ResolveLink {
                link, live_value, ..
            } => {
                let Some(IrType::Link { entity, .. }) = self.register_type(*link, span).cloned()
                else {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "link resolution requires a link register",
                    );
                    return;
                };
                self.expect_type(*live_value, &IrType::Entity(entity), span);
            }
            Terminator::Return(value) => match (&self.function.return_type, value) {
                (IrType::Unit, None) => {}
                (IrType::Unit, Some(_)) | (_, None) => self.sink.error(
                    REGISTER_ERROR,
                    span,
                    "return value presence does not match the function return type",
                ),
                (return_type, Some(value)) => self.expect_type(*value, return_type, span),
            },
        }
        if matches!(terminator, Terminator::Return(_))
            && active_lifecycles != &BTreeSet::from([self.function.current_lifecycle])
        {
            self.sink.error(
                LIFECYCLE_ERROR,
                span,
                "function return must end every explicit lifecycle",
            );
        }
    }

    fn definition(
        &mut self,
        definition: DefId,
        expected_kind: IrDefinitionKind,
        span: Span,
    ) -> Option<&IrDefinition> {
        let candidate = self.module.definitions.get(definition.0 as usize);
        if candidate
            .is_none_or(|candidate| candidate.id != definition || candidate.kind != expected_kind)
        {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "operation references an unknown or incompatible definition",
            );
            None
        } else {
            candidate
        }
    }

    fn field_type(&mut self, definition: DefId, field: FieldId, span: Span) -> Option<&IrType> {
        let Some(definition) = self.module.definitions.get(definition.0 as usize) else {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "field owner definition does not exist",
            );
            return None;
        };
        let field_type = definition
            .fields
            .iter()
            .find_map(|(candidate, ty)| (*candidate == field).then_some(ty));
        if field_type.is_none() {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "field ID does not exist on this definition",
            );
        }
        field_type
    }

    fn require_available(
        &mut self,
        register: Register,
        available: &BTreeSet<Register>,
        span: Span,
    ) {
        self.check_register(register, span);
        if !available.contains(&register) {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "register use is not dominated by its definition",
            );
        }
    }

    fn check_register(&mut self, register: Register, span: Span) {
        if register.0 as usize >= self.function.register_types.len() {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "register is outside the contiguous type table",
            );
        }
    }

    fn register_type(&self, register: Register, _span: Span) -> Option<&IrType> {
        self.function.register_types.get(register.0 as usize)
    }

    fn expect_type(&mut self, register: Register, expected: &IrType, span: Span) {
        self.check_register(register, span);
        if self.register_type(register, span) != Some(expected) {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "register type does not match the operation",
            );
        }
    }

    fn expect_same_type(&mut self, lhs: Register, rhs: Register, span: Span) {
        self.check_register(lhs, span);
        self.check_register(rhs, span);
        if self.register_type(lhs, span) != self.register_type(rhs, span) {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "operation registers have incompatible types",
            );
        }
    }

    fn validate_receiver(&mut self, receiver: &Receiver, span: Span) -> Option<IrType> {
        let list_type = self.register_type(receiver.list, span).cloned();
        let Some(IrType::List(element)) = list_type else {
            self.sink.error(
                STORAGE_ERROR,
                span,
                "List receiver register must have a List type",
            );
            self.check_register(receiver.list, span);
            if let Some(source) = &receiver.source {
                self.validate_argument_source(source, span);
            }
            return None;
        };
        if let Some(source) = &receiver.source
            && self.validate_argument_source(source, span) != Some(IrType::List(element.clone()))
        {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "projected receiver source does not resolve to the receiver List",
            );
        }
        Some(*element)
    }

    fn validate_argument_source(&mut self, source: &ArgumentSource, span: Span) -> Option<IrType> {
        let mut current = self.register_type(source.base, span).cloned();
        if current.is_none() {
            self.check_register(source.base, span);
            return None;
        }
        for projection in &source.projections {
            current = match (current, projection) {
                (
                    Some(IrType::Struct(definition) | IrType::Entity(definition)),
                    ArgumentProjection::Field(field),
                ) => self.field_type(definition, *field, span).cloned(),
                (Some(IrType::List(element)), ArgumentProjection::Index(index)) => {
                    self.expect_type(*index, &IrType::Int, span);
                    Some(*element)
                }
                (Some(_), ArgumentProjection::Field(_)) => {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "field projection requires a struct or entity value",
                    );
                    None
                }
                (Some(_), ArgumentProjection::Index(_)) => {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "index projection requires a List value",
                    );
                    None
                }
                (None, _) => None,
            };
            if current.is_none() {
                break;
            }
        }
        current
    }

    fn expect_entity(&mut self, register: Register, span: Span) {
        self.check_register(register, span);
        if !matches!(self.register_type(register, span), Some(IrType::Entity(_))) {
            self.sink.error(
                REGISTER_ERROR,
                span,
                "operation requires an entity register",
            );
        }
    }

    fn require_active_lifecycle(
        &mut self,
        lifecycle: Register,
        active: &BTreeSet<Register>,
        span: Span,
    ) {
        self.expect_type(lifecycle, &IrType::Lifecycle, span);
        if !active.contains(&lifecycle) {
            self.sink.error(
                LIFECYCLE_ERROR,
                span,
                "operation requires an active lifecycle",
            );
        }
    }
}

fn join_home_state(left: HomeState, right: HomeState) -> HomeState {
    match (left, right) {
        (HomeState::Live, HomeState::Live) => HomeState::Live,
        (HomeState::Empty, HomeState::Empty) => HomeState::Empty,
        _ => HomeState::MaybeLive,
    }
}

fn is_implicit_copy_type(module: &Module, ty: &IrType) -> bool {
    match ty {
        IrType::Unit | IrType::Bool | IrType::Int | IrType::Link { .. } => true,
        IrType::Optional(inner) => is_implicit_copy_type(module, inner),
        IrType::Struct(definition) => module
            .definitions
            .iter()
            .find(|candidate| candidate.id == *definition)
            .is_some_and(|definition| {
                definition
                    .fields
                    .iter()
                    .all(|(_, field)| is_implicit_copy_type(module, field))
            }),
        IrType::Text | IrType::List(_) | IrType::Entity(_) | IrType::Lifecycle => false,
    }
}

#[allow(clippy::too_many_lines)]
fn transfer_home_states(
    incoming: &BTreeMap<Register, HomeState>,
    instructions: &[Instruction],
    storage: &[RegisterStorage],
    module: &Module,
) -> BTreeMap<Register, HomeState> {
    let mut state = incoming.clone();
    for instruction in instructions {
        let role = |register: Register| storage.get(register.0 as usize);
        let set_live = |state: &mut BTreeMap<Register, HomeState>, register: Register| {
            if matches!(role(register), Some(RegisterStorage::Home { .. })) {
                state.insert(register, HomeState::Live);
            }
        };
        match instruction {
            Instruction::Copy { dst, .. }
            | Instruction::ConstInt { dst, .. }
            | Instruction::ConstBool { dst, .. }
            | Instruction::ConstText { dst, .. }
            | Instruction::ConstNoneLink { dst, .. }
            | Instruction::ListNew { dst, .. }
            | Instruction::ListRemove { dst, .. }
            | Instruction::ListRemovePlace { dst, .. }
            | Instruction::ListIndex { dst, .. }
            | Instruction::ListGet { dst, .. }
            | Instruction::TextConcat { dst, .. }
            | Instruction::CheckedUnaryInt { dst, .. }
            | Instruction::CheckedBinaryInt { dst, .. }
            | Instruction::Not { dst, .. }
            | Instruction::Compare { dst, .. }
            | Instruction::ListLength { dst, .. }
            | Instruction::Phi { dst, .. }
            | Instruction::ReadStructField { dst, .. }
            | Instruction::ReadField { dst, .. }
            | Instruction::BeginLifecycle { dst, .. }
            | Instruction::EntityToLink { dst, .. } => set_live(&mut state, *dst),
            Instruction::ConstructStruct { dst, fields, .. } => {
                set_live(&mut state, *dst);
                for (_, source) in fields {
                    if matches!(role(*source), Some(RegisterStorage::Home { .. })) {
                        state.insert(*source, HomeState::Empty);
                    }
                }
            }
            Instruction::AllocateEntity { dst, fields, .. } => {
                set_live(&mut state, *dst);
                for (_, source) in fields {
                    if matches!(role(*source), Some(RegisterStorage::Home { .. })) {
                        state.insert(*source, HomeState::Empty);
                    }
                }
            }
            Instruction::TextByteLength { dst, .. } | Instruction::TextIsEmpty { dst, .. } => {
                set_live(&mut state, *dst);
            }
            Instruction::Take { dst, src, .. }
            | Instruction::MoveHome {
                destination: dst,
                source: src,
                ..
            } => {
                set_live(&mut state, *dst);
                if matches!(role(*src), Some(RegisterStorage::Home { .. })) {
                    state.insert(*src, HomeState::Empty);
                }
            }
            Instruction::InstallHome {
                destination,
                source,
                displaced,
                ..
            } => {
                let previous = state.get(destination).copied().unwrap_or(HomeState::Empty);
                set_live(&mut state, *destination);
                if matches!(role(*source), Some(RegisterStorage::Home { .. })) {
                    state.insert(*source, HomeState::Empty);
                }
                if matches!(role(*displaced), Some(RegisterStorage::DropSlot)) {
                    state.insert(*displaced, previous);
                }
            }
            Instruction::DropHome { home, .. } | Instruction::DropIfLive { home, .. } => {
                if matches!(role(*home), Some(RegisterStorage::Home { .. })) {
                    state.insert(*home, HomeState::Empty);
                }
            }
            Instruction::DropSlot { slot, .. } => {
                if matches!(role(*slot), Some(RegisterStorage::DropSlot)) {
                    state.insert(*slot, HomeState::Empty);
                }
            }
            Instruction::CleanupTrackedScope { scope, .. } => {
                for (index, register_role) in storage.iter().enumerate() {
                    if let RegisterStorage::Home {
                        scope: home_scope, ..
                    } = register_role
                        && home_scope == scope
                    {
                        state.insert(
                            Register(u32::try_from(index).expect("register index fits in u32")),
                            HomeState::Empty,
                        );
                    }
                }
            }
            Instruction::ReplacePlace {
                source, displaced, ..
            }
            | Instruction::ReplaceField {
                source, displaced, ..
            } => {
                if matches!(role(*source), Some(RegisterStorage::Home { .. })) {
                    state.insert(*source, HomeState::Empty);
                }
                if matches!(role(*displaced), Some(RegisterStorage::DropSlot)) {
                    state.insert(*displaced, HomeState::Live);
                }
            }
            Instruction::ListReplace {
                value, displaced, ..
            } => {
                if matches!(role(*value), Some(RegisterStorage::Home { .. })) {
                    state.insert(*value, HomeState::Empty);
                }
                if matches!(role(*displaced), Some(RegisterStorage::DropSlot)) {
                    state.insert(*displaced, HomeState::Live);
                }
            }
            Instruction::Call {
                dst,
                function,
                arguments,
                ..
            } => {
                if let Some(callee) = module.functions.get(function.0 as usize) {
                    for (parameter, argument) in arguments {
                        if callee.parameter_modes.get(parameter.0 as usize)
                            == Some(&keld_semantics::ParameterMode::Take)
                            && matches!(role(*argument), Some(RegisterStorage::Home { .. }))
                        {
                            state.insert(*argument, HomeState::Empty);
                        }
                    }
                }
                if let Some(dst) = dst {
                    set_live(&mut state, *dst);
                }
            }
            Instruction::ListPush { value, .. }
            | Instruction::ListPushPlace { value, .. }
            | Instruction::WriteField { value, .. } => {
                if matches!(role(*value), Some(RegisterStorage::Home { .. })) {
                    state.insert(*value, HomeState::Empty);
                }
            }
            Instruction::OpenView { .. }
            | Instruction::CloseView { .. }
            | Instruction::EndLifecycle { .. }
            | Instruction::KeepEntity { .. }
            | Instruction::RetireEntity { .. } => {}
        }
    }
    state
}

fn add_block_definitions(block: &crate::IrBlock, definitions: &mut BTreeSet<Register>) {
    for instruction in &block.instructions {
        add_instruction_definitions(instruction, definitions);
    }
}

fn add_instruction_definitions(instruction: &Instruction, definitions: &mut BTreeSet<Register>) {
    if let Some(destination) = instruction_destination(instruction) {
        definitions.insert(destination);
    }
    if let Instruction::InstallHome { destination, .. } = instruction {
        definitions.insert(*destination);
    }
}

fn transfer_lifecycles(
    incoming: &BTreeSet<Register>,
    instructions: &[Instruction],
) -> BTreeSet<Register> {
    let mut active = incoming.clone();
    for instruction in instructions {
        match instruction {
            Instruction::BeginLifecycle { dst, .. } => {
                active.insert(*dst);
            }
            Instruction::EndLifecycle { lifecycle, .. } => {
                active.remove(lifecycle);
            }
            _ => {}
        }
    }
    active
}

fn intersect_sets(mut sets: impl Iterator<Item = BTreeSet<Register>>) -> BTreeSet<Register> {
    let Some(first) = sets.next() else {
        return BTreeSet::new();
    };
    sets.fold(first, |current, next| {
        current.intersection(&next).copied().collect()
    })
}

fn instruction_destination(instruction: &Instruction) -> Option<Register> {
    match instruction {
        Instruction::ConstInt { dst, .. }
        | Instruction::ConstBool { dst, .. }
        | Instruction::ConstText { dst, .. }
        | Instruction::ConstNoneLink { dst, .. }
        | Instruction::Copy { dst, .. }
        | Instruction::Take { dst, .. }
        | Instruction::ListNew { dst, .. }
        | Instruction::ListLength { dst, .. }
        | Instruction::ListRemove { dst, .. }
        | Instruction::ListRemovePlace { dst, .. }
        | Instruction::ListIndex { dst, .. }
        | Instruction::ListGet { dst, .. }
        | Instruction::TextByteLength { dst, .. }
        | Instruction::TextIsEmpty { dst, .. }
        | Instruction::TextConcat { dst, .. }
        | Instruction::CheckedUnaryInt { dst, .. }
        | Instruction::CheckedBinaryInt { dst, .. }
        | Instruction::Not { dst, .. }
        | Instruction::Compare { dst, .. }
        | Instruction::Phi { dst, .. }
        | Instruction::ConstructStruct { dst, .. }
        | Instruction::ReadStructField { dst, .. }
        | Instruction::BeginLifecycle { dst, .. }
        | Instruction::AllocateEntity { dst, .. }
        | Instruction::EntityToLink { dst, .. }
        | Instruction::ReadField { dst, .. } => Some(*dst),
        Instruction::MoveHome { destination, .. } => Some(*destination),
        Instruction::InstallHome { displaced, .. }
        | Instruction::ListReplace { displaced, .. }
        | Instruction::ReplacePlace { displaced, .. }
        | Instruction::ReplaceField { displaced, .. } => Some(*displaced),
        Instruction::Call { dst, .. } => *dst,
        Instruction::EndLifecycle { .. }
        | Instruction::DropHome { .. }
        | Instruction::DropIfLive { .. }
        | Instruction::DropSlot { .. }
        | Instruction::CleanupTrackedScope { .. }
        | Instruction::OpenView { .. }
        | Instruction::WriteField { .. }
        | Instruction::CloseView { .. }
        | Instruction::ListPush { .. }
        | Instruction::ListPushPlace { .. }
        | Instruction::KeepEntity { .. }
        | Instruction::RetireEntity { .. } => None,
    }
}

#[allow(clippy::too_many_lines)]
fn instruction_uses(instruction: &Instruction) -> Vec<Register> {
    match instruction {
        Instruction::ConstInt { .. }
        | Instruction::ConstBool { .. }
        | Instruction::ConstText { .. }
        | Instruction::ConstNoneLink { .. }
        | Instruction::ListNew { .. }
        | Instruction::Phi { .. }
        | Instruction::ReadField { .. }
        | Instruction::CloseView { .. }
        | Instruction::CleanupTrackedScope { .. }
        | Instruction::DropIfLive { .. } => Vec::new(),
        Instruction::Copy { src, .. }
        | Instruction::Take { src, .. }
        | Instruction::CheckedUnaryInt { src, .. }
        | Instruction::Not { src, .. } => vec![*src],
        Instruction::InstallHome { source, .. } | Instruction::MoveHome { source, .. } => {
            vec![*source]
        }
        Instruction::DropHome { home, .. } => vec![*home],
        Instruction::DropSlot { slot, .. } => vec![*slot],
        Instruction::ReplacePlace {
            destination,
            source,
            ..
        } => source_registers(destination)
            .into_iter()
            .chain(std::iter::once(*source))
            .collect(),
        Instruction::ReplaceField { source, .. } => vec![*source],
        Instruction::ListLength { list, .. } => vec![*list],
        Instruction::ListPush { list, value, .. } => vec![*list, *value],
        Instruction::ListPushPlace {
            list,
            source,
            value,
            ..
        } => std::iter::once(*list)
            .chain(source_registers(source))
            .chain(std::iter::once(*value))
            .collect(),
        Instruction::ListRemove { list, index, .. } => vec![*list, *index],
        Instruction::ListRemovePlace {
            list,
            source,
            index,
            ..
        } => std::iter::once(*list)
            .chain(source_registers(source))
            .chain(std::iter::once(*index))
            .collect(),
        Instruction::ListIndex {
            receiver, index, ..
        }
        | Instruction::ListGet {
            receiver, index, ..
        } => receiver_registers(receiver)
            .into_iter()
            .chain(std::iter::once(*index))
            .collect(),
        Instruction::ListReplace {
            receiver,
            index,
            value,
            ..
        } => receiver_registers(receiver)
            .into_iter()
            .chain([*index, *value])
            .collect(),
        Instruction::TextByteLength { text, .. } | Instruction::TextIsEmpty { text, .. } => {
            vec![*text]
        }
        Instruction::TextConcat { lhs, rhs, .. } => vec![*lhs, *rhs],
        Instruction::CheckedBinaryInt { lhs, rhs, .. } | Instruction::Compare { lhs, rhs, .. } => {
            vec![*lhs, *rhs]
        }
        Instruction::ConstructStruct { fields, .. } => {
            fields.iter().map(|(_, register)| *register).collect()
        }
        Instruction::ReadStructField { base, .. } => vec![*base],
        Instruction::BeginLifecycle { parent, .. } => vec![*parent],
        Instruction::EndLifecycle { lifecycle, .. } => vec![*lifecycle],
        Instruction::AllocateEntity {
            fields, lifecycle, ..
        } => fields
            .iter()
            .map(|(_, register)| *register)
            .chain(std::iter::once(*lifecycle))
            .collect(),
        Instruction::EntityToLink { entity, .. }
        | Instruction::OpenView { entity, .. }
        | Instruction::RetireEntity { entity, .. } => vec![*entity],
        Instruction::WriteField { value, .. } => vec![*value],
        Instruction::KeepEntity {
            entity, lifecycle, ..
        } => vec![*entity, *lifecycle],
        Instruction::Call {
            arguments,
            argument_sources,
            current_lifecycle,
            ..
        } => arguments
            .iter()
            .map(|(_, register)| *register)
            .chain(
                argument_sources
                    .iter()
                    .flat_map(|(_, source)| source.as_ref().into_iter().flat_map(source_registers)),
            )
            .chain(std::iter::once(*current_lifecycle))
            .collect(),
    }
}

fn source_registers(source: &crate::ArgumentSource) -> Vec<Register> {
    std::iter::once(source.base)
        .chain(
            source
                .projections
                .iter()
                .filter_map(|projection| match projection {
                    crate::ArgumentProjection::Field(_) => None,
                    crate::ArgumentProjection::Index(register) => Some(*register),
                }),
        )
        .collect()
}

fn receiver_registers(receiver: &crate::Receiver) -> Vec<Register> {
    std::iter::once(receiver.list)
        .chain(receiver.source.iter().flat_map(source_registers))
        .collect()
}

fn terminator_uses(terminator: &Terminator) -> Vec<Register> {
    match terminator {
        Terminator::Branch { condition, .. } => vec![*condition],
        Terminator::ResolveLink { link, .. } => vec![*link],
        Terminator::Return(Some(value)) => vec![*value],
        Terminator::Goto(_)
        | Terminator::Return(None)
        | Terminator::Fault { .. }
        | Terminator::Unreachable => Vec::new(),
    }
}

fn is_structural(instruction: &Instruction) -> bool {
    matches!(
        instruction,
        Instruction::BeginLifecycle { .. }
            | Instruction::EndLifecycle { .. }
            | Instruction::AllocateEntity { .. }
            | Instruction::KeepEntity { .. }
            | Instruction::RetireEntity { .. }
            | Instruction::ListPush { .. }
            | Instruction::ListRemove { .. }
            | Instruction::ListIndex { .. }
            | Instruction::ListGet { .. }
            | Instruction::ListReplace { .. }
            | Instruction::TextByteLength { .. }
            | Instruction::TextIsEmpty { .. }
            | Instruction::TextConcat { .. }
            | Instruction::Call { .. }
            | Instruction::InstallHome { .. }
            | Instruction::MoveHome { .. }
            | Instruction::DropHome { .. }
            | Instruction::DropIfLive { .. }
            | Instruction::DropSlot { .. }
            | Instruction::CleanupTrackedScope { .. }
            | Instruction::ReplacePlace { .. }
    )
}

fn terminator_targets(terminator: &Terminator) -> Vec<IrBlockId> {
    match terminator {
        Terminator::Goto(block) => vec![*block],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::ResolveLink { live, absent, .. } => vec![*live, *absent],
        Terminator::Return(_) | Terminator::Fault { .. } | Terminator::Unreachable => Vec::new(),
    }
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

fn terminator_span(terminator: &Terminator, fallback: Span) -> Span {
    match terminator {
        Terminator::ResolveLink { span, .. } | Terminator::Fault { span, .. } => *span,
        Terminator::Goto(_)
        | Terminator::Branch { .. }
        | Terminator::Return(_)
        | Terminator::Unreachable => fallback,
    }
}
