use crate::Value;
use crate::place::{RegisterSlot, RuntimePlace};
use keld_ir::{Function, IrBlockId, Register, RegisterStorage, ViewId, ViewMode};
use keld_runtime::EntityId;

#[derive(Clone, Copy)]
pub(crate) struct ActiveView {
    pub entity: EntityId,
    pub mode: ViewMode,
}

pub(crate) struct Frame {
    pub function: keld_semantics::FunctionId,
    pub registers: Vec<RegisterSlot>,
    pub block: IrBlockId,
    pub instruction: usize,
    pub predecessor: Option<IrBlockId>,
    pub return_destination: Option<Register>,
    pub views: Vec<Option<ActiveView>>,
}

impl Frame {
    pub fn new(
        function: &Function,
        return_destination: Option<Register>,
    ) -> Result<Self, crate::value::CopyAllocation> {
        let mut registers = Vec::new();
        registers
            .try_reserve_exact(function.register_types.len())
            .map_err(|_| crate::value::CopyAllocation)?;
        for storage in &function.register_storage {
            registers.push(match storage {
                RegisterStorage::DropSlot => RegisterSlot::DropSlot(None),
                RegisterStorage::Trivial
                | RegisterStorage::EntityFlow
                | RegisterStorage::Loan
                | RegisterStorage::Home { .. } => RegisterSlot::Empty,
            });
        }
        let view_count = function
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .filter_map(|instruction| match instruction {
                keld_ir::Instruction::OpenView { view, .. }
                | keld_ir::Instruction::ReadField { view, .. }
                | keld_ir::Instruction::WriteField { view, .. }
                | keld_ir::Instruction::CloseView { view, .. } => Some(view.0),
                _ => None,
            })
            .max()
            .map_or(0, |maximum| maximum.saturating_add(1));
        let view_count = usize::try_from(view_count).map_err(|_| crate::value::CopyAllocation)?;
        let mut views = Vec::new();
        views
            .try_reserve_exact(view_count)
            .map_err(|_| crate::value::CopyAllocation)?;
        views.resize(view_count, None);
        Ok(Self {
            function: function.id,
            registers,
            block: function.entry,
            instruction: 0,
            predecessor: None,
            return_destination,
            views,
        })
    }

    pub fn value(&self, register: Register) -> Option<&Value> {
        match self.registers.get(register.0 as usize)? {
            RegisterSlot::Owned(value) | RegisterSlot::DropSlot(Some(value)) => Some(value),
            RegisterSlot::Empty | RegisterSlot::Loan(_) | RegisterSlot::DropSlot(None) => None,
        }
    }

    pub fn value_mut(&mut self, register: Register) -> Option<&mut Value> {
        match self.registers.get_mut(register.0 as usize)? {
            RegisterSlot::Owned(value) | RegisterSlot::DropSlot(Some(value)) => Some(value),
            RegisterSlot::Empty | RegisterSlot::Loan(_) | RegisterSlot::DropSlot(None) => None,
        }
    }

    pub fn set(&mut self, register: Register, value: Value) -> Result<(), ()> {
        let destination = self.registers.get_mut(register.0 as usize).ok_or(())?;
        match destination {
            RegisterSlot::Empty | RegisterSlot::Owned(_) => {
                *destination = RegisterSlot::Owned(value);
                Ok(())
            }
            RegisterSlot::DropSlot(slot) => {
                *slot = Some(value);
                Ok(())
            }
            RegisterSlot::Loan(_) => Err(()),
        }
    }

    pub fn set_loan(&mut self, register: Register, place: RuntimePlace) -> Result<(), ()> {
        let destination = self.registers.get_mut(register.0 as usize).ok_or(())?;
        if !matches!(destination, RegisterSlot::Empty) {
            return Err(());
        }
        *destination = RegisterSlot::Loan(place);
        Ok(())
    }

    pub fn take(&mut self, register: Register) -> Option<Value> {
        let slot = self.registers.get_mut(register.0 as usize)?;
        match slot {
            RegisterSlot::Owned(_) => {
                let old = std::mem::replace(slot, RegisterSlot::Empty);
                match old {
                    RegisterSlot::Owned(value) => Some(value),
                    _ => unreachable!(),
                }
            }
            RegisterSlot::DropSlot(value) => value.take(),
            RegisterSlot::Empty | RegisterSlot::Loan(_) => None,
        }
    }

    pub fn loan(&self, register: Register) -> Option<&RuntimePlace> {
        match self.registers.get(register.0 as usize)? {
            RegisterSlot::Loan(place) => Some(place),
            _ => None,
        }
    }

    pub fn view(&self, view: ViewId) -> Option<ActiveView> {
        self.views.get(view.0 as usize).copied().flatten()
    }
}
