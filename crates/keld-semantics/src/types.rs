use crate::{DefId, TypeId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeKind {
    Unit,
    Bool,
    Int,
    Struct(DefId),
    EntityRef(DefId),
    Link { entity: DefId, optional: bool },
    Optional(TypeId),
    Error,
}

#[derive(Clone, Debug)]
pub struct TypeStore {
    kinds: Vec<TypeKind>,
}

impl TypeStore {
    pub const UNIT: TypeId = TypeId(0);
    pub const BOOL: TypeId = TypeId(1);
    pub const INT: TypeId = TypeId(2);
    pub const ERROR: TypeId = TypeId(3);

    #[must_use]
    pub fn new() -> Self {
        Self {
            kinds: vec![
                TypeKind::Unit,
                TypeKind::Bool,
                TypeKind::Int,
                TypeKind::Error,
            ],
        }
    }

    #[must_use]
    pub fn kind(&self, id: TypeId) -> &TypeKind {
        self.kinds
            .get(id.0 as usize)
            .unwrap_or(&self.kinds[Self::ERROR.0 as usize])
    }

    pub fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(index) = self.kinds.iter().position(|existing| *existing == kind) {
            return TypeId(u32::try_from(index).unwrap_or(u32::MAX));
        }
        let id = TypeId(u32::try_from(self.kinds.len()).unwrap_or(u32::MAX));
        self.kinds.push(kind);
        id
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

impl Default for TypeStore {
    fn default() -> Self {
        Self::new()
    }
}
