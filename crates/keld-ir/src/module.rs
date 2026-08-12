use crate::{Instruction, Terminator};
use keld_flow::StorageScopeId;
use keld_semantics::{DefId, FieldId, FunctionId, ParameterMode};
use keld_source::{SourceId, Span};
use keld_storage::LoanEffect;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Register(pub u32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterStorage {
    Trivial,
    EntityFlow,
    Loan,
    Home {
        scope: StorageScopeId,
        conditional: bool,
    },
    DropSlot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArgumentSource {
    pub base: Register,
    pub fields: Vec<keld_semantics::FieldId>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ViewId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IrBlockId(pub u32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrType {
    Unit,
    Bool,
    Int,
    Struct(DefId),
    Entity(DefId),
    Link { entity: DefId, optional: bool },
    Text,
    List(Box<IrType>),
    Lifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IrDefinitionKind {
    Struct,
    Entity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrDefinition {
    pub id: DefId,
    pub kind: IrDefinitionKind,
    pub fields: Vec<(FieldId, IrType)>,
}

#[derive(Clone, Debug)]
pub struct IrBlock {
    pub id: IrBlockId,
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub id: FunctionId,
    pub span: Span,
    pub parameters: Vec<Register>,
    pub parameter_modes: Vec<ParameterMode>,
    pub parameter_effects: Vec<LoanEffect>,
    pub current_lifecycle: Register,
    pub register_types: Vec<IrType>,
    pub register_storage: Vec<RegisterStorage>,
    pub storage_scope_parents: Vec<Option<StorageScopeId>>,
    pub return_type: IrType,
    pub blocks: Vec<IrBlock>,
    pub entry: IrBlockId,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub definitions: Vec<IrDefinition>,
    pub functions: Vec<Function>,
    pub main: FunctionId,
}

impl Module {
    #[must_use]
    pub fn dump(&self) -> String {
        crate::dump::dump(self)
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct TestModuleBuilder {
    parameters: Vec<(Register, IrType)>,
    instructions: Vec<Instruction>,
}

impl TestModuleBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn parameter(mut self, register: Register, ty: IrType) -> Self {
        self.parameters.push((register, ty));
        self
    }

    #[must_use]
    pub fn instruction(mut self, instruction: Instruction) -> Self {
        self.instructions.push(instruction);
        self
    }

    #[must_use]
    /// Builds the intentionally small module used by IR validation tests.
    ///
    /// # Panics
    ///
    /// Panics only if the synthetic zero-length source span cannot be represented.
    pub fn finish(self) -> Module {
        let span = Span::new(SourceId(0), 0, 0).expect("empty test span is valid");
        let parameter_count = self.parameters.len();
        let register_count = self
            .parameters
            .iter()
            .map(|(register, _)| register.0.saturating_add(1))
            .max()
            .unwrap_or(0);
        let current_lifecycle = Register(register_count);
        let result = Register(register_count.saturating_add(1));
        let mut register_types = vec![IrType::Unit; register_count as usize];
        let mut definitions = Vec::new();
        for (register, ty) in &self.parameters {
            register_types[register.0 as usize] = ty.clone();
            if let IrType::Entity(definition) = ty
                && !definitions
                    .iter()
                    .any(|candidate: &IrDefinition| candidate.id == *definition)
            {
                definitions.push(IrDefinition {
                    id: *definition,
                    kind: IrDefinitionKind::Entity,
                    fields: vec![(FieldId(0), IrType::Int)],
                });
            }
        }
        definitions.sort_by_key(|definition| definition.id);
        register_types.push(IrType::Lifecycle);
        register_types.push(IrType::Int);
        let register_storage = vec![RegisterStorage::Trivial; register_types.len()];
        let mut instructions = self.instructions;
        instructions.push(Instruction::ConstInt {
            dst: result,
            value: 0,
            span,
        });
        Module {
            definitions,
            functions: vec![Function {
                id: FunctionId(0),
                span,
                parameters: self
                    .parameters
                    .into_iter()
                    .map(|(register, _)| register)
                    .collect(),
                parameter_modes: vec![ParameterMode::Loan; parameter_count],
                parameter_effects: vec![LoanEffect::Read; parameter_count],
                current_lifecycle,
                register_types,
                register_storage,
                storage_scope_parents: vec![None],
                return_type: IrType::Int,
                blocks: vec![IrBlock {
                    id: IrBlockId(0),
                    instructions,
                    terminator: Terminator::Return(Some(result)),
                }],
                entry: IrBlockId(0),
            }],
            main: FunctionId(0),
        }
    }
}
