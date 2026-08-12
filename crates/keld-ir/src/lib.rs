mod dump;
mod instruction;
mod lower;
mod module;
mod validate;

pub use instruction::{FaultKind, Instruction, Terminator, ViewMode};
pub use lower::{VerifiedInput, lower};
pub use module::{
    ArgumentSource, Function, IrBlock, IrBlockId, IrDefinition, IrDefinitionKind, IrType, Module,
    Register, TestModuleBuilder, ViewId,
};
pub use validate::validate;
