use keld_runtime::{EntityId, Link, RuntimeLifecycleId};
use keld_semantics::DefId;

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

#[derive(Debug, Eq, PartialEq)]
pub enum Value {
    Unit,
    Int(i64),
    Bool(bool),
    Text(RuntimeText),
    Struct {
        definition: DefId,
        fields: Vec<Value>,
    },
    List(Vec<Value>),
    Entity(EntityId),
    Link(Option<Link>),
    #[doc(hidden)]
    Lifecycle(RuntimeLifecycleId),
}

#[derive(Debug, Eq, PartialEq)]
pub struct EntityPayload {
    pub definition: DefId,
    pub fields: Vec<Value>,
}

#[derive(Clone, Copy)]
pub(crate) struct CopyAllocation;

enum CopyTask<'value> {
    Visit(&'value Value),
    FinishStruct { definition: DefId, fields: usize },
    FinishList { elements: usize },
}

pub(crate) fn try_copy_value(value: &Value) -> Result<Value, CopyAllocation> {
    let mut work = Vec::new();
    let mut completed = Vec::new();
    work.try_reserve(1).map_err(|_| CopyAllocation)?;
    completed.try_reserve(1).map_err(|_| CopyAllocation)?;
    work.push(CopyTask::Visit(value));
    while let Some(task) = work.pop() {
        match task {
            CopyTask::Visit(value) => match value {
                Value::Unit => push_completed(&mut completed, Value::Unit)?,
                Value::Int(value) => push_completed(&mut completed, Value::Int(*value))?,
                Value::Bool(value) => push_completed(&mut completed, Value::Bool(*value))?,
                Value::Text(value) => push_completed(
                    &mut completed,
                    Value::Text(match value {
                        RuntimeText::Inline { len, bytes } => RuntimeText::Inline {
                            len: *len,
                            bytes: *bytes,
                        },
                        RuntimeText::Heap(text) => RuntimeText::Heap(text.clone()),
                    }),
                )?,
                Value::Entity(entity) => {
                    push_completed(&mut completed, Value::Entity(*entity))?;
                }
                Value::Link(link) => push_completed(&mut completed, Value::Link(*link))?,
                Value::Lifecycle(lifecycle) => {
                    push_completed(&mut completed, Value::Lifecycle(*lifecycle))?;
                }
                Value::Struct { definition, fields } => {
                    work.try_reserve(fields.len().saturating_add(1))
                        .map_err(|_| CopyAllocation)?;
                    work.push(CopyTask::FinishStruct {
                        definition: *definition,
                        fields: fields.len(),
                    });
                    for field in fields.iter().rev() {
                        work.push(CopyTask::Visit(field));
                    }
                }
                Value::List(elements) => {
                    work.try_reserve(elements.len().saturating_add(1))
                        .map_err(|_| CopyAllocation)?;
                    work.push(CopyTask::FinishList {
                        elements: elements.len(),
                    });
                    for element in elements.iter().rev() {
                        work.push(CopyTask::Visit(element));
                    }
                }
            },
            CopyTask::FinishStruct { definition, fields } => {
                let start = completed.len().checked_sub(fields).ok_or(CopyAllocation)?;
                let mut copied_fields = Vec::new();
                copied_fields
                    .try_reserve_exact(fields)
                    .map_err(|_| CopyAllocation)?;
                copied_fields.extend(completed.drain(start..));
                push_completed(
                    &mut completed,
                    Value::Struct {
                        definition,
                        fields: copied_fields,
                    },
                )?;
            }
            CopyTask::FinishList { elements } => {
                let start = completed
                    .len()
                    .checked_sub(elements)
                    .ok_or(CopyAllocation)?;
                let mut copied = Vec::new();
                copied
                    .try_reserve_exact(elements)
                    .map_err(|_| CopyAllocation)?;
                copied.extend(completed.drain(start..));
                push_completed(&mut completed, Value::List(copied))?;
            }
        }
    }
    if completed.len() == 1 {
        completed.pop().ok_or(CopyAllocation)
    } else {
        Err(CopyAllocation)
    }
}

fn push_completed(values: &mut Vec<Value>, value: Value) -> Result<(), CopyAllocation> {
    values.try_reserve(1).map_err(|_| CopyAllocation)?;
    values.push(value);
    Ok(())
}
