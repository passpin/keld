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
    Text,
    List(TypeId),
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageClass {
    ImplicitCopy,
    SingleHome,
    EntityFlow,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingMutability {
    Let,
    Var,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterMode {
    Loan,
    Take,
}

#[derive(Clone, Debug)]
pub struct TypeStore {
    kinds: Vec<TypeKind>,
    struct_fields: Vec<Vec<TypeId>>,
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
            struct_fields: Vec::new(),
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

    pub fn register_struct_fields(
        &mut self,
        definition: DefId,
        fields: impl IntoIterator<Item = TypeId>,
    ) {
        let index = definition.0 as usize;
        if self.struct_fields.len() <= index {
            self.struct_fields.resize_with(index + 1, Vec::new);
        }
        self.struct_fields[index] = fields.into_iter().collect();
    }

    #[must_use]
    pub fn storage_class(&self, id: TypeId) -> StorageClass {
        let mut visiting = vec![false; self.kinds.len()];
        self.storage_class_inner(id, &mut visiting)
    }

    fn storage_class_inner(&self, id: TypeId, visiting: &mut [bool]) -> StorageClass {
        let index = id.0 as usize;
        if index >= self.kinds.len() || visiting[index] {
            return StorageClass::Error;
        }
        visiting[index] = true;
        let class = match self.kind(id) {
            TypeKind::Unit | TypeKind::Bool | TypeKind::Int | TypeKind::Link { .. } => {
                StorageClass::ImplicitCopy
            }
            TypeKind::Text | TypeKind::List(_) => StorageClass::SingleHome,
            TypeKind::EntityRef(_) => StorageClass::EntityFlow,
            TypeKind::Optional(inner) => self.storage_class_inner(*inner, visiting),
            TypeKind::Struct(definition) => {
                let fields = self
                    .struct_fields
                    .get(definition.0 as usize)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let mut has_entity_flow = false;
                let mut has_single_home = false;
                for field in fields {
                    match self.storage_class_inner(*field, visiting) {
                        StorageClass::SingleHome => has_single_home = true,
                        StorageClass::EntityFlow => has_entity_flow = true,
                        StorageClass::Error => return StorageClass::Error,
                        StorageClass::ImplicitCopy => {}
                    }
                }
                if has_single_home {
                    StorageClass::SingleHome
                } else if has_entity_flow {
                    StorageClass::EntityFlow
                } else {
                    StorageClass::ImplicitCopy
                }
            }
            TypeKind::Error => StorageClass::Error,
        };
        visiting[index] = false;
        class
    }

    #[must_use]
    pub fn is_structurally_duplicable(&self, id: TypeId) -> bool {
        let mut visiting = vec![false; self.kinds.len()];
        self.is_duplicable_inner(id, &mut visiting)
    }

    fn is_duplicable_inner(&self, id: TypeId, visiting: &mut [bool]) -> bool {
        let index = id.0 as usize;
        if index >= self.kinds.len() || visiting[index] {
            return false;
        }
        visiting[index] = true;
        let duplicable = match self.kind(id) {
            TypeKind::Unit
            | TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Link { .. }
            | TypeKind::Text => true,
            TypeKind::List(element) | TypeKind::Optional(element) => {
                self.is_duplicable_inner(*element, visiting)
            }
            TypeKind::Struct(definition) => self
                .struct_fields
                .get(definition.0 as usize)
                .is_some_and(|fields| {
                    fields
                        .iter()
                        .all(|field| self.is_duplicable_inner(*field, visiting))
                }),
            TypeKind::EntityRef(_) | TypeKind::Error => false,
        };
        visiting[index] = false;
        duplicable
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
