use keld_ir::{IrDefinitionKind, IrType, Module};
use keld_runtime::{EntityId, Link, RuntimeLifecycleId};
use keld_semantics::DefId;

use crate::RuntimeList;

#[derive(Debug, Eq)]
pub enum RuntimeText {
    Inline { len: u8, bytes: [u8; 22] },
    Heap(Box<str>),
}

impl PartialEq for RuntimeText {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl RuntimeText {
    pub(crate) fn from_string(value: String) -> Self {
        if value.len() <= 22 {
            let mut bytes = [0; 22];
            bytes[..value.len()].copy_from_slice(value.as_bytes());
            Self::Inline {
                len: u8::try_from(value.len()).expect("inline text length fits in u8"),
                bytes,
            }
        } else {
            Self::Heap(value.into_boxed_str())
        }
    }

    pub(crate) fn byte_length(&self) -> usize {
        match self {
            Self::Inline { len, .. } => usize::from(*len),
            Self::Heap(value) => value.len(),
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Self::Heap(value) => value.as_bytes(),
        }
    }

    pub(crate) fn concat(left: &Self, right: &Self) -> Option<Self> {
        let length = left.byte_length().checked_add(right.byte_length())?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).ok()?;
        bytes.extend_from_slice(left.as_bytes());
        bytes.extend_from_slice(right.as_bytes());
        let value = String::from_utf8(bytes).ok()?;
        Some(Self::from_string(value))
    }
}

/// A runtime value whose optional shape is stored without allocating wrappers.
#[derive(Debug, Eq, PartialEq)]
pub struct Value {
    optional_some_layers: u32,
    kind: ValueKind,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ValueKind {
    Absent,
    Unit,
    Int(i64),
    Bool(bool),
    Text(RuntimeText),
    Struct {
        definition: DefId,
        fields: Vec<Value>,
    },
    List(RuntimeList),
    Entity(EntityId),
    Link(Option<Link>),
    #[doc(hidden)]
    Lifecycle(RuntimeLifecycleId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueTypeError;

impl Value {
    #[allow(non_upper_case_globals)]
    pub const Unit: Self = Self::new(ValueKind::Unit);

    #[allow(non_snake_case)]
    #[must_use]
    pub const fn Int(value: i64) -> Self {
        Self::new(ValueKind::Int(value))
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub const fn Bool(value: bool) -> Self {
        Self::new(ValueKind::Bool(value))
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn Text(value: RuntimeText) -> Self {
        Self::new(ValueKind::Text(value))
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub fn List(value: RuntimeList) -> Self {
        Self::new(ValueKind::List(value))
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub const fn Entity(value: EntityId) -> Self {
        Self::new(ValueKind::Entity(value))
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub const fn Link(value: Option<Link>) -> Self {
        Self::new(ValueKind::Link(value))
    }

    #[allow(non_snake_case)]
    #[must_use]
    pub const fn Lifecycle(value: RuntimeLifecycleId) -> Self {
        Self::new(ValueKind::Lifecycle(value))
    }

    const fn new(kind: ValueKind) -> Self {
        Self {
            optional_some_layers: 0,
            kind,
        }
    }

    pub(crate) fn struct_value(definition: DefId, fields: Vec<Self>) -> Self {
        Self::new(ValueKind::Struct { definition, fields })
    }

    #[must_use]
    pub const fn optional_some_layers(&self) -> u32 {
        self.optional_some_layers
    }

    #[must_use]
    pub const fn kind(&self) -> &ValueKind {
        &self.kind
    }

    pub(crate) fn kind_mut(&mut self) -> &mut ValueKind {
        &mut self.kind
    }

    pub(crate) fn into_kind(self) -> ValueKind {
        self.kind
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn from_parts_for_test(optional_some_layers: u32, kind: ValueKind) -> Self {
        Self {
            optional_some_layers,
            kind,
        }
    }

    /// Checks that the envelope and payload exactly represent `expected`.
    ///
    /// # Errors
    ///
    /// Returns [`ValueTypeError`] for an invalid optional depth or payload kind.
    pub fn validate_for_type(
        &self,
        module: &Module,
        expected: &IrType,
    ) -> Result<(), ValueTypeError> {
        let (depth, base) = expected.optional_depth().map_err(|_| ValueTypeError)?;
        self.validate_shape(module, depth, base)
    }

    fn validate_shape(
        &self,
        module: &Module,
        depth: u32,
        base: &IrType,
    ) -> Result<(), ValueTypeError> {
        if self.optional_some_layers > depth {
            return Err(ValueTypeError);
        }
        if matches!(self.kind, ValueKind::Absent) {
            return (depth > 0 && self.optional_some_layers < depth)
                .then_some(())
                .ok_or(ValueTypeError);
        }
        if self.optional_some_layers != depth {
            return Err(ValueTypeError);
        }
        self.validate_base(module, base)
    }

    fn validate_base(&self, module: &Module, base: &IrType) -> Result<(), ValueTypeError> {
        match (&self.kind, base) {
            (ValueKind::Unit, IrType::Unit)
            | (ValueKind::Bool(_), IrType::Bool)
            | (ValueKind::Int(_), IrType::Int)
            | (ValueKind::Text(_), IrType::Text)
            | (ValueKind::Entity(_), IrType::Entity(_))
            | (ValueKind::Link(_), IrType::Link { .. })
            | (ValueKind::Lifecycle(_), IrType::Lifecycle) => Ok(()),
            (ValueKind::List(values), IrType::List(element)) => values
                .as_slice()
                .iter()
                .try_for_each(|value| value.validate_for_type(module, element)),
            (ValueKind::Struct { definition, fields }, IrType::Struct(expected_definition))
                if definition == expected_definition =>
            {
                let layout = module
                    .definitions
                    .get(definition.0 as usize)
                    .filter(|candidate| {
                        candidate.id == *definition && candidate.kind == IrDefinitionKind::Struct
                    })
                    .ok_or(ValueTypeError)?;
                if fields.len() != layout.fields.len() {
                    return Err(ValueTypeError);
                }
                fields
                    .iter()
                    .zip(&layout.fields)
                    .try_for_each(|(value, (_, ty))| value.validate_for_type(module, ty))
            }
            _ => Err(ValueTypeError),
        }
    }

    /// Creates the absent value for an optional IR type.
    ///
    /// # Errors
    ///
    /// Returns [`ValueTypeError`] if `expected` is not optional or its depth
    /// cannot be represented.
    pub fn optional_none(module: &Module, expected: &IrType) -> Result<Self, ValueTypeError> {
        let value = Self::new(ValueKind::Absent);
        value.validate_for_type(module, expected)?;
        Ok(value)
    }

    /// Wraps `self` in one present optional layer without allocating.
    ///
    /// # Errors
    ///
    /// Returns [`ValueTypeError`] if `self` does not represent `inner` or the
    /// additional layer cannot be represented.
    pub fn into_optional_some(
        mut self,
        module: &Module,
        inner: &IrType,
    ) -> Result<Self, ValueTypeError> {
        self.validate_for_type(module, inner)?;
        let (inner_depth, base) = inner.optional_depth().map_err(|_| ValueTypeError)?;
        let outer_depth = inner_depth.checked_add(1).ok_or(ValueTypeError)?;
        self.optional_some_layers = self
            .optional_some_layers
            .checked_add(1)
            .ok_or(ValueTypeError)?;
        self.validate_shape(module, outer_depth, base)?;
        Ok(self)
    }

    /// Removes one optional layer, returning `None` only for the outer absence.
    ///
    /// # Errors
    ///
    /// Returns [`ValueTypeError`] if the input envelope does not represent
    /// `Optional[inner]` or the peeled result does not represent `inner`.
    pub fn peel_optional(
        mut self,
        module: &Module,
        inner: &IrType,
    ) -> Result<Option<Self>, ValueTypeError> {
        let (inner_depth, base) = inner.optional_depth().map_err(|_| ValueTypeError)?;
        let outer_depth = inner_depth.checked_add(1).ok_or(ValueTypeError)?;
        self.validate_shape(module, outer_depth, base)?;
        if matches!(self.kind, ValueKind::Absent) && self.optional_some_layers == 0 {
            return Ok(None);
        }
        self.optional_some_layers = self
            .optional_some_layers
            .checked_sub(1)
            .ok_or(ValueTypeError)?;
        self.validate_for_type(module, inner)?;
        Ok(Some(self))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct EntityPayload {
    pub definition: DefId,
    pub fields: Vec<Value>,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyAllocation;

enum CopyTask<'value> {
    Visit(&'value Value),
    FinishStruct {
        optional_some_layers: u32,
        definition: DefId,
        fields: usize,
    },
    FinishList {
        optional_some_layers: u32,
        elements: usize,
    },
}

pub(crate) fn try_copy_value(value: &Value) -> Result<Value, CopyAllocation> {
    let mut work = Vec::new();
    let mut completed = Vec::new();
    work.try_reserve(1).map_err(|_| CopyAllocation)?;
    completed.try_reserve(1).map_err(|_| CopyAllocation)?;
    work.push(CopyTask::Visit(value));
    while let Some(task) = work.pop() {
        match task {
            CopyTask::Visit(value) => {
                let layers = value.optional_some_layers;
                if let Some(value) = copy_leaf(value) {
                    push_completed(&mut completed, value)?;
                    continue;
                }
                match &value.kind {
                    ValueKind::Struct { definition, fields } => {
                        work.try_reserve(fields.len().saturating_add(1))
                            .map_err(|_| CopyAllocation)?;
                        work.push(CopyTask::FinishStruct {
                            optional_some_layers: layers,
                            definition: *definition,
                            fields: fields.len(),
                        });
                        for field in fields.iter().rev() {
                            work.push(CopyTask::Visit(field));
                        }
                    }
                    ValueKind::List(elements) => {
                        work.try_reserve(elements.length().saturating_add(1))
                            .map_err(|_| CopyAllocation)?;
                        work.push(CopyTask::FinishList {
                            optional_some_layers: layers,
                            elements: elements.length(),
                        });
                        for element in elements.as_slice().iter().rev() {
                            work.push(CopyTask::Visit(element));
                        }
                    }
                    ValueKind::Absent
                    | ValueKind::Unit
                    | ValueKind::Int(_)
                    | ValueKind::Bool(_)
                    | ValueKind::Text(_)
                    | ValueKind::Entity(_)
                    | ValueKind::Link(_)
                    | ValueKind::Lifecycle(_) => unreachable!("leaf values were copied above"),
                }
            }
            CopyTask::FinishStruct {
                optional_some_layers,
                definition,
                fields,
            } => {
                let start = completed.len().checked_sub(fields).ok_or(CopyAllocation)?;
                let mut copied_fields = Vec::new();
                copied_fields
                    .try_reserve_exact(fields)
                    .map_err(|_| CopyAllocation)?;
                copied_fields.extend(completed.drain(start..));
                push_completed(
                    &mut completed,
                    Value::from_parts_for_test(
                        optional_some_layers,
                        ValueKind::Struct {
                            definition,
                            fields: copied_fields,
                        },
                    ),
                )?;
            }
            CopyTask::FinishList {
                optional_some_layers,
                elements,
            } => {
                let start = completed
                    .len()
                    .checked_sub(elements)
                    .ok_or(CopyAllocation)?;
                let mut copied = Vec::new();
                copied
                    .try_reserve_exact(elements)
                    .map_err(|_| CopyAllocation)?;
                copied.extend(completed.drain(start..));
                push_completed(
                    &mut completed,
                    Value::from_parts_for_test(
                        optional_some_layers,
                        ValueKind::List(RuntimeList::from_values(copied)),
                    ),
                )?;
            }
        }
    }
    if completed.len() == 1 {
        completed.pop().ok_or(CopyAllocation)
    } else {
        Err(CopyAllocation)
    }
}

fn copy_leaf(value: &Value) -> Option<Value> {
    let kind = match &value.kind {
        ValueKind::Absent => ValueKind::Absent,
        ValueKind::Unit => ValueKind::Unit,
        ValueKind::Int(value) => ValueKind::Int(*value),
        ValueKind::Bool(value) => ValueKind::Bool(*value),
        ValueKind::Text(value) => ValueKind::Text(match value {
            RuntimeText::Inline { len, bytes } => RuntimeText::Inline {
                len: *len,
                bytes: *bytes,
            },
            RuntimeText::Heap(text) => RuntimeText::Heap(text.clone()),
        }),
        ValueKind::Entity(entity) => ValueKind::Entity(*entity),
        ValueKind::Link(link) => ValueKind::Link(*link),
        ValueKind::Lifecycle(lifecycle) => ValueKind::Lifecycle(*lifecycle),
        ValueKind::Struct { .. } | ValueKind::List(_) => return None,
    };
    Some(Value::from_parts_for_test(value.optional_some_layers, kind))
}

fn push_completed(values: &mut Vec<Value>, value: Value) -> Result<(), CopyAllocation> {
    values.try_reserve(1).map_err(|_| CopyAllocation)?;
    values.push(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Value, ValueKind, ValueTypeError, try_copy_value};
    use keld_ir::{IrType, Module};
    use keld_semantics::{DefId, FunctionId};

    fn module() -> Module {
        Module {
            definitions: Vec::new(),
            functions: Vec::new(),
            main: FunctionId(0),
        }
    }

    #[test]
    fn optional_envelope_rejects_every_malformed_shape() {
        let module = module();
        let optional_int = IrType::Optional(Box::new(IrType::Int));

        assert_eq!(
            Value::Int(1).validate_for_type(&module, &optional_int),
            Err(ValueTypeError)
        );
        assert_eq!(
            Value::from_parts_for_test(1, ValueKind::Absent)
                .validate_for_type(&module, &optional_int),
            Err(ValueTypeError)
        );
        assert_eq!(
            Value::from_parts_for_test(2, ValueKind::Int(1))
                .validate_for_type(&module, &optional_int),
            Err(ValueTypeError)
        );
        assert_eq!(
            Value::from_parts_for_test(1, ValueKind::Bool(true))
                .validate_for_type(&module, &optional_int),
            Err(ValueTypeError)
        );
    }

    #[test]
    fn nested_some_none_peels_copies_and_compares_by_shape() {
        let module = module();
        let optional_int = IrType::Optional(Box::new(IrType::Int));
        let inner_none = Value::optional_none(&module, &optional_int).expect("inner none");
        let outer_some = inner_none
            .into_optional_some(&module, &optional_int)
            .expect("Some[None] is valid");

        assert_eq!(outer_some.optional_some_layers(), 1);
        assert_eq!(
            try_copy_value(&outer_some),
            Ok(Value::from_parts_for_test(1, ValueKind::Absent))
        );
        let peeled = outer_some
            .peel_optional(&module, &optional_int)
            .expect("outer envelope is valid")
            .expect("outer layer is present");
        assert_eq!(peeled, Value::from_parts_for_test(0, ValueKind::Absent));
    }

    #[test]
    fn optional_link_remains_a_link_payload_beneath_the_envelope() {
        let module = module();
        let link = IrType::Link {
            entity: DefId(0),
            optional: true,
        };
        let optional_link = IrType::Optional(Box::new(link.clone()));
        let value = Value::Link(None)
            .into_optional_some(&module, &link)
            .expect("link absence is distinct from optional absence");

        assert!(matches!(value.kind(), ValueKind::Link(None)));
        assert_eq!(value.optional_some_layers(), 1);
        assert_eq!(value.validate_for_type(&module, &optional_link), Ok(()));
    }
}
