use crate::Value;
use crate::cleanup::ActiveHomeTracker;
use crate::place::{RegisterSlot, RuntimePlace};
use keld_flow::StorageScopeId;
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
    home_scopes: Vec<Option<StorageScopeId>>,
    pub(crate) cleanup_trackers: Vec<ActiveHomeTracker>,
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
        let home_scopes = function
            .register_storage
            .iter()
            .map(|storage| match storage {
                RegisterStorage::Home { scope, .. } => Some(*scope),
                _ => None,
            })
            .collect();
        let mut cleanup_trackers = Vec::new();
        cleanup_trackers
            .try_reserve_exact(function.storage_scope_parents.len())
            .map_err(|_| crate::value::CopyAllocation)?;
        for (index, _) in function.storage_scope_parents.iter().enumerate() {
            let mut homes = Vec::new();
            homes
                .try_reserve_exact(function.register_storage.len())
                .map_err(|_| crate::value::CopyAllocation)?;
            cleanup_trackers.push(ActiveHomeTracker {
                scope: StorageScopeId(u32::try_from(index).expect("storage scope count fits")),
                homes,
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
            home_scopes,
            cleanup_trackers,
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
        let result = match destination {
            RegisterSlot::Empty | RegisterSlot::Owned(_) => {
                *destination = RegisterSlot::Owned(value);
                Ok(())
            }
            RegisterSlot::DropSlot(slot) => {
                *slot = Some(value);
                Ok(())
            }
            RegisterSlot::Loan(_) => Err(()),
        };
        if result.is_ok() {
            self.activate_home(register);
        }
        result
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
        let value = match slot {
            RegisterSlot::Owned(_) => {
                let old = std::mem::replace(slot, RegisterSlot::Empty);
                match old {
                    RegisterSlot::Owned(value) => Some(value),
                    _ => unreachable!(),
                }
            }
            RegisterSlot::DropSlot(value) => value.take(),
            RegisterSlot::Empty | RegisterSlot::Loan(_) => None,
        };
        if value.is_some() {
            self.deactivate_home(register);
        }
        value
    }

    pub fn loan(&self, register: Register) -> Option<&RuntimePlace> {
        match self.registers.get(register.0 as usize)? {
            RegisterSlot::Loan(place) => Some(place),
            _ => None,
        }
    }

    pub fn pop_cleanup_home(&mut self, scope: StorageScopeId) -> Option<Register> {
        self.cleanup_trackers
            .iter_mut()
            .find(|tracker| tracker.scope == scope)
            .and_then(|tracker| tracker.homes.pop())
    }

    fn activate_home(&mut self, register: Register) {
        let Some(Some(scope)) = self.home_scopes.get(register.0 as usize).copied() else {
            return;
        };
        let Some(tracker) = self
            .cleanup_trackers
            .iter_mut()
            .find(|tracker| tracker.scope == scope)
        else {
            return;
        };
        if !tracker.homes.contains(&register) {
            tracker.homes.push(register);
        }
    }

    fn deactivate_home(&mut self, register: Register) {
        let Some(Some(scope)) = self.home_scopes.get(register.0 as usize).copied() else {
            return;
        };
        if let Some(tracker) = self
            .cleanup_trackers
            .iter_mut()
            .find(|tracker| tracker.scope == scope)
            && let Some(index) = tracker
                .homes
                .iter()
                .position(|candidate| *candidate == register)
        {
            tracker.homes.remove(index);
        }
    }

    pub fn view(&self, view: ViewId) -> Option<ActiveView> {
        self.views.get(view.0 as usize).copied().flatten()
    }
}
