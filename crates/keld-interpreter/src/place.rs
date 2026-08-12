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

#[derive(Clone, Debug)]
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

    pub(crate) fn project(mut self, projection: RuntimeProjection) -> Self {
        self.projections.push(projection);
        self
    }
}
