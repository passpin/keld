use keld_ir::Register;
use keld_runtime::EntityId;
use keld_semantics::FieldId;

#[derive(Debug)]
pub(crate) enum RegisterSlot {
    Empty,
    Owned(crate::Value),
    Loan(RuntimePlace),
    DropSlot(Option<crate::Value>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameId(pub(crate) usize);

#[derive(Debug)]
pub(crate) struct RuntimePlace {
    pub(crate) root: RuntimePlaceRoot,
    pub(crate) projections: Vec<RuntimeProjection>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RuntimePlaceRoot {
    Frame { frame: FrameId, register: Register },
    Entity { entity: EntityId },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RuntimeProjection {
    Field(FieldId),
    #[allow(dead_code)]
    Index(usize),
}

impl RuntimePlace {
    pub(crate) fn frame(frame: FrameId, register: Register) -> Self {
        Self {
            root: RuntimePlaceRoot::Frame { frame, register },
            projections: Vec::new(),
        }
    }

    pub(crate) fn entity(entity: EntityId) -> Self {
        Self {
            root: RuntimePlaceRoot::Entity { entity },
            projections: Vec::new(),
        }
    }

    pub(crate) fn try_clone_with_capacity(
        &self,
        additional: usize,
        allow_allocation: impl FnOnce() -> bool,
    ) -> Result<Self, crate::value::CopyAllocation> {
        let capacity = self
            .projections
            .len()
            .checked_add(additional)
            .ok_or(crate::value::CopyAllocation)?;
        let mut projections = Vec::new();
        if capacity > 0 {
            if !allow_allocation() {
                return Err(crate::value::CopyAllocation);
            }
            projections
                .try_reserve_exact(capacity)
                .map_err(|_| crate::value::CopyAllocation)?;
            projections.extend_from_slice(&self.projections);
        }
        Ok(Self {
            root: self.root,
            projections,
        })
    }

    pub(crate) fn try_reserve_projections(
        &mut self,
        additional: usize,
        allow_allocation: impl FnOnce() -> bool,
    ) -> Result<(), crate::value::CopyAllocation> {
        if additional
            <= self
                .projections
                .capacity()
                .saturating_sub(self.projections.len())
        {
            return Ok(());
        }
        if !allow_allocation() {
            return Err(crate::value::CopyAllocation);
        }
        self.projections
            .try_reserve_exact(additional)
            .map_err(|_| crate::value::CopyAllocation)
    }

    pub(crate) fn push_reserved(
        &mut self,
        projection: RuntimeProjection,
    ) -> Result<(), crate::value::CopyAllocation> {
        if self.projections.len() >= self.projections.capacity() {
            return Err(crate::value::CopyAllocation);
        }
        self.projections.push(projection);
        Ok(())
    }
}
