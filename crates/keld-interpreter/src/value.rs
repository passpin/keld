use keld_runtime::{EntityId, Link, RuntimeLifecycleId};
use keld_semantics::DefId;

#[derive(Debug, Eq, PartialEq)]
pub enum Value {
    Unit,
    Int(i64),
    Bool(bool),
    Struct {
        definition: DefId,
        fields: Vec<Value>,
    },
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
