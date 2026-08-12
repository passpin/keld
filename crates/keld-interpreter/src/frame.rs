use crate::Value;
use keld_ir::{ArgumentSource, Function, IrBlockId, Register, ViewId, ViewMode};
use keld_runtime::EntityId;

#[derive(Clone, Copy)]
pub(crate) struct ActiveView {
    pub entity: EntityId,
    pub mode: ViewMode,
}

pub(crate) struct LoanReturn {
    pub source: ArgumentSource,
    pub callee: Register,
}

pub(crate) struct Frame {
    pub function: keld_semantics::FunctionId,
    pub registers: Vec<Option<Value>>,
    pub block: IrBlockId,
    pub instruction: usize,
    pub predecessor: Option<IrBlockId>,
    pub return_destination: Option<Register>,
    pub loan_returns: Vec<LoanReturn>,
    pub views: Vec<Option<ActiveView>>,
}

impl Frame {
    pub fn new(
        function: &Function,
        return_destination: Option<Register>,
        loan_returns: Vec<LoanReturn>,
    ) -> Result<Self, crate::value::CopyAllocation> {
        let mut registers = Vec::new();
        registers
            .try_reserve_exact(function.register_types.len())
            .map_err(|_| crate::value::CopyAllocation)?;
        registers.resize_with(function.register_types.len(), || None);
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
            loan_returns,
            views,
        })
    }

    pub fn value(&self, register: Register) -> Option<&Value> {
        self.registers.get(register.0 as usize)?.as_ref()
    }

    pub fn set(&mut self, register: Register, value: Value) -> Result<(), ()> {
        let destination = self.registers.get_mut(register.0 as usize).ok_or(())?;
        *destination = Some(value);
        Ok(())
    }

    pub fn view(&self, view: ViewId) -> Option<ActiveView> {
        self.views.get(view.0 as usize).copied().flatten()
    }
}
