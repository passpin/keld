use crate::value::{EntityPayload, Value, ValueKind};
use keld_flow::StorageScopeId;
use keld_ir::{Module, Register};
use keld_semantics::{DefId, FieldId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupEvent {
    Home(Register),
    StructField {
        definition: DefId,
        field: FieldId,
    },
    EntityField {
        definition: DefId,
        field: FieldId,
    },
    ListElement {
        index: usize,
    },
    Text {
        byte_length: usize,
        first_byte: Option<u8>,
    },
}

#[derive(Debug, Default)]
pub(crate) struct CleanupTrace {
    pub(crate) events: Vec<CleanupEvent>,
}

impl CleanupTrace {
    pub(crate) fn text_markers(&self) -> Vec<(usize, u8)> {
        self.events
            .iter()
            .filter_map(|event| match event {
                CleanupEvent::Text {
                    byte_length,
                    first_byte: Some(first_byte),
                } => Some((*byte_length, *first_byte)),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn list_indices(&self) -> Vec<usize> {
        self.events
            .iter()
            .filter_map(|event| match event {
                CleanupEvent::ListElement { index } => Some(*index),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn field_ids(&self) -> Vec<FieldId> {
        self.events
            .iter()
            .filter_map(|event| match event {
                CleanupEvent::StructField { field, .. }
                | CleanupEvent::EntityField { field, .. } => Some(*field),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct ExecutionTrace {
    pub result: crate::ExecutionResult,
    pub cleanup: Vec<CleanupEvent>,
}

impl ExecutionTrace {
    #[must_use]
    pub fn text_markers(&self) -> Vec<(usize, u8)> {
        CleanupTrace {
            events: self.cleanup.clone(),
        }
        .text_markers()
    }

    #[must_use]
    pub fn list_indices(&self) -> Vec<usize> {
        CleanupTrace {
            events: self.cleanup.clone(),
        }
        .list_indices()
    }

    #[must_use]
    pub fn field_ids(&self) -> Vec<FieldId> {
        CleanupTrace {
            events: self.cleanup.clone(),
        }
        .field_ids()
    }
}

#[derive(Debug)]
pub(crate) struct ActiveHomeTracker {
    pub(crate) scope: StorageScopeId,
    pub(crate) homes: Vec<Register>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CleanupPath {
    Home(Register),
    StructField { definition: DefId, field: FieldId },
    EntityField { definition: DefId, field: FieldId },
    ListElement { index: usize },
}

enum CleanupTask {
    Value {
        value: Value,
        path: CleanupPath,
    },
    StructFields {
        definition: DefId,
        fields: std::vec::IntoIter<Value>,
    },
    EntityFields {
        definition: DefId,
        fields: std::vec::IntoIter<Value>,
    },
    ListElements {
        elements: std::vec::IntoIter<Value>,
    },
}

const CLEANUP_SCRATCH_CAPACITY: usize = 512;

pub(crate) struct CleanupScratch {
    work: Vec<CleanupTask>,
}

impl CleanupScratch {
    pub(crate) fn new() -> Result<Self, ()> {
        let mut work = Vec::new();
        work.try_reserve_exact(CLEANUP_SCRATCH_CAPACITY)
            .map_err(|_| ())?;
        Ok(Self { work })
    }
}

pub(crate) fn cleanup_value(
    module: &Module,
    value: Value,
    path: CleanupPath,
    scratch: &mut CleanupScratch,
    trace: Option<&mut CleanupTrace>,
) {
    scratch.work.clear();
    scratch.work.push(CleanupTask::Value { value, path });
    cleanup_work(module, &mut scratch.work, trace);
}

pub(crate) fn cleanup_payload(
    module: &Module,
    payload: EntityPayload,
    scratch: &mut CleanupScratch,
    trace: Option<&mut CleanupTrace>,
) {
    let EntityPayload { definition, fields } = payload;
    scratch.work.clear();
    scratch.work.push(CleanupTask::EntityFields {
        definition,
        fields: fields.into_iter(),
    });
    cleanup_work(module, &mut scratch.work, trace);
}

fn cleanup_work(
    module: &Module,
    work: &mut Vec<CleanupTask>,
    mut trace: Option<&mut CleanupTrace>,
) {
    while let Some(task) = work.pop() {
        match task {
            CleanupTask::Value { value, path } => {
                record_path(path, trace.as_deref_mut());
                match value.into_kind() {
                    ValueKind::Struct { definition, fields } => {
                        work.push(CleanupTask::StructFields {
                            definition,
                            fields: fields.into_iter(),
                        });
                    }
                    ValueKind::List(elements) => {
                        work.push(CleanupTask::ListElements {
                            elements: elements.into_elements(),
                        });
                    }
                    ValueKind::Text(text) => record(
                        trace.as_deref_mut(),
                        CleanupEvent::Text {
                            byte_length: text.byte_length(),
                            first_byte: text.as_bytes().first().copied(),
                        },
                    ),
                    ValueKind::Absent
                    | ValueKind::Unit
                    | ValueKind::Int(_)
                    | ValueKind::Bool(_)
                    | ValueKind::Entity(_)
                    | ValueKind::Link(_)
                    | ValueKind::Lifecycle(_) => {}
                }
            }
            CleanupTask::StructFields {
                definition,
                mut fields,
            } => {
                if let Some(index) = fields.len().checked_sub(1) {
                    let field = fields
                        .next_back()
                        .expect("cleanup field iterator length is exact");
                    work.push(CleanupTask::StructFields { definition, fields });
                    work.push(CleanupTask::Value {
                        value: field,
                        path: CleanupPath::StructField {
                            definition,
                            field: declared_field(module, definition, index),
                        },
                    });
                }
            }
            CleanupTask::EntityFields {
                definition,
                mut fields,
            } => {
                if let Some(index) = fields.len().checked_sub(1) {
                    let field = fields
                        .next_back()
                        .expect("cleanup entity field iterator length is exact");
                    work.push(CleanupTask::EntityFields { definition, fields });
                    work.push(CleanupTask::Value {
                        value: field,
                        path: CleanupPath::EntityField {
                            definition,
                            field: declared_field(module, definition, index),
                        },
                    });
                }
            }
            CleanupTask::ListElements { mut elements } => {
                if let Some(index) = elements.len().checked_sub(1) {
                    let element = elements
                        .next_back()
                        .expect("cleanup list iterator length is exact");
                    work.push(CleanupTask::ListElements { elements });
                    work.push(CleanupTask::Value {
                        value: element,
                        path: CleanupPath::ListElement { index },
                    });
                }
            }
        }
    }
}

fn record_path(path: CleanupPath, trace: Option<&mut CleanupTrace>) {
    let event = match path {
        CleanupPath::Home(register) => CleanupEvent::Home(register),
        CleanupPath::StructField { definition, field } => {
            CleanupEvent::StructField { definition, field }
        }
        CleanupPath::EntityField { definition, field } => {
            CleanupEvent::EntityField { definition, field }
        }
        CleanupPath::ListElement { index } => CleanupEvent::ListElement { index },
    };
    record(trace, event);
}

fn record(trace: Option<&mut CleanupTrace>, event: CleanupEvent) {
    if let Some(trace) = trace {
        trace.events.push(event);
    }
}

fn declared_field(module: &Module, definition: DefId, index: usize) -> FieldId {
    module
        .definitions
        .iter()
        .find(|candidate| candidate.id == definition)
        .and_then(|definition| definition.fields.get(index))
        .map_or_else(
            || panic!("validated definition layout is missing field {index}"),
            |(field, _)| *field,
        )
}

#[cfg(test)]
mod tests {
    use super::{CleanupPath, CleanupScratch, CleanupTask, CleanupTrace, cleanup_work};
    use crate::{RuntimeList, RuntimeText, Value};
    use keld_ir::{IrType, Module, Register};
    use keld_semantics::FunctionId;

    #[test]
    fn cleanup_scratch_does_not_grow_for_a_wide_list() {
        let module = Module {
            definitions: Vec::new(),
            functions: Vec::new(),
            main: FunctionId(0),
        };
        let mut scratch = CleanupScratch {
            work: Vec::with_capacity(2),
        };
        scratch.work.push(CleanupTask::Value {
            value: Value::List(RuntimeList::from_values(vec![Value::Int(1), Value::Int(2)])),
            path: CleanupPath::Home(Register(0)),
        });
        let capacity = scratch.work.capacity();

        cleanup_work(&module, &mut scratch.work, None);

        assert_eq!(scratch.work.capacity(), capacity);
    }

    #[test]
    fn cleanup_handles_maximum_depth_nested_lists_without_growing_scratch() {
        let module = Module {
            definitions: Vec::new(),
            functions: Vec::new(),
            main: FunctionId(0),
        };
        let mut value = Value::Int(0);
        for _ in 0..256 {
            value = Value::List(RuntimeList::from_values(vec![value]));
        }
        let mut scratch = CleanupScratch::new().expect("cleanup scratch allocates");
        let capacity = scratch.work.capacity();
        let mut trace = CleanupTrace::default();
        scratch.work.push(CleanupTask::Value {
            value,
            path: CleanupPath::Home(Register(0)),
        });

        cleanup_work(&module, &mut scratch.work, Some(&mut trace));

        assert_eq!(scratch.work.capacity(), capacity);
        assert_eq!(trace.list_indices().len(), 256);
    }

    #[test]
    fn nested_optional_envelopes_clean_the_managed_payload_once() {
        let module = Module {
            definitions: Vec::new(),
            functions: Vec::new(),
            main: FunctionId(0),
        };
        let text = IrType::Text;
        let optional_text = IrType::Optional(Box::new(text.clone()));
        let value = Value::Text(RuntimeText::from_string("managed".to_owned()))
            .into_optional_some(&module, &text)
            .expect("first optional layer")
            .into_optional_some(&module, &optional_text)
            .expect("second optional layer");
        let mut scratch = CleanupScratch::new().expect("cleanup scratch allocates");
        let mut trace = CleanupTrace::default();
        scratch.work.push(CleanupTask::Value {
            value,
            path: CleanupPath::Home(Register(0)),
        });

        cleanup_work(&module, &mut scratch.work, Some(&mut trace));

        assert_eq!(trace.text_markers(), vec![(7, b'm')]);
    }
}
