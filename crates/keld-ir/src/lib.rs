mod allocation;
mod dump;
mod instruction;
mod lower;
mod module;
mod validate;

pub use allocation::{AllocationCoordinate, AllocationSchedule};
pub use instruction::{FaultKind, Instruction, Terminator, ViewMode};
pub use keld_native_abi::{AllocationPhase, allocation_site_id};
pub use keld_numeric::{IntBinaryOp, IntUnaryOp};
pub use keld_semantics::{CompareOp, DefId, FieldId, FunctionId, ParameterIndex, ParameterMode};
pub use lower::{VerifiedInput, lower};
pub use module::{
    ArgumentProjection, ArgumentSource, Function, IrBlock, IrBlockId, IrDefinition,
    IrDefinitionKind, IrType, Module, OptionalDepthOverflow, Receiver, Register, RegisterStorage,
    TestModuleBuilder, ViewId,
};
pub use validate::validate;
