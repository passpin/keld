use crate::{FlowOp, Terminator};
use keld_semantics::{
    BindingMutability, Definition, FunctionEffects, FunctionId, LocalId, ParameterMode, TypeId,
    TypeStore,
};
use keld_source::Span;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValueId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LifecycleId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AllocationSite(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorageScopeId(pub u32);

#[derive(Clone, Debug)]
pub struct FlowModule {
    pub definitions: Vec<Definition>,
    pub types: TypeStore,
    pub functions: Vec<FlowFunction>,
    pub main: FunctionId,
}

impl FlowModule {
    #[must_use]
    pub fn function_named(&self, name: &str) -> Option<&FlowFunction> {
        self.functions.iter().find(|function| function.name == name)
    }

    #[must_use]
    pub fn dump(&self) -> String {
        crate::dump::dump(self)
    }
}

#[derive(Clone, Debug)]
pub struct FlowFunction {
    pub id: FunctionId,
    pub name: String,
    pub span: Span,
    pub parameters: Vec<LocalId>,
    pub parameter_modes: Vec<ParameterMode>,
    pub local_types: Vec<TypeId>,
    pub local_mutability: Vec<BindingMutability>,
    pub return_type: TypeId,
    pub effects: FunctionEffects,
    pub current_lifecycle: LifecycleId,
    pub value_types: Vec<TypeId>,
    pub lifecycle_parents: Vec<Option<LifecycleId>>,
    pub storage_scope_parents: Vec<Option<StorageScopeId>>,
    pub local_scopes: Vec<StorageScopeId>,
    pub blocks: Vec<FlowBlock>,
    pub entry: BlockId,
}

impl FlowFunction {
    #[must_use]
    pub fn linear_ops(&self) -> Vec<&FlowOp> {
        self.blocks
            .iter()
            .flat_map(|block| block.operations.iter())
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct FlowBlock {
    pub id: BlockId,
    pub storage_scope: StorageScopeId,
    pub operations: Vec<FlowOp>,
    pub terminator: Terminator,
}
