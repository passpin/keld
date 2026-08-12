use crate::value::{EntityPayload, Value};
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
    Value { value: Value, path: CleanupPath },
}

pub(crate) fn cleanup_value(
    module: &Module,
    value: Value,
    path: CleanupPath,
    trace: Option<&mut CleanupTrace>,
) {
    let mut work = Vec::with_capacity(1);
    work.push(CleanupTask::Value { value, path });
    cleanup_work(module, &mut work, trace);
}

pub(crate) fn cleanup_payload(
    module: &Module,
    payload: EntityPayload,
    trace: Option<&mut CleanupTrace>,
) {
    let EntityPayload { definition, fields } = payload;
    let mut work = Vec::with_capacity(fields.len());
    for (index, value) in fields.into_iter().enumerate() {
        work.push(CleanupTask::Value {
            value,
            path: CleanupPath::EntityField {
                definition,
                field: declared_field(module, definition, index),
            },
        });
    }
    cleanup_work(module, &mut work, trace);
}

fn cleanup_work(
    module: &Module,
    work: &mut Vec<CleanupTask>,
    mut trace: Option<&mut CleanupTrace>,
) {
    while let Some(CleanupTask::Value { value, path }) = work.pop() {
        record_path(path, trace.as_deref_mut());
        match value {
            Value::Struct { definition, fields } => {
                for (index, field) in fields.into_iter().enumerate() {
                    work.push(CleanupTask::Value {
                        value: field,
                        path: CleanupPath::StructField {
                            definition,
                            field: declared_field(module, definition, index),
                        },
                    });
                }
            }
            Value::List(elements) => {
                for (index, element) in elements.into_elements().enumerate() {
                    work.push(CleanupTask::Value {
                        value: element,
                        path: CleanupPath::ListElement { index },
                    });
                }
            }
            Value::Optional(value) => {
                if let Some(value) = value {
                    work.push(CleanupTask::Value {
                        value: *value,
                        path,
                    });
                }
            }
            Value::Text(text) => record(
                trace.as_deref_mut(),
                CleanupEvent::Text {
                    byte_length: text.byte_length(),
                    first_byte: text.as_bytes().first().copied(),
                },
            ),
            Value::Unit
            | Value::Int(_)
            | Value::Bool(_)
            | Value::Entity(_)
            | Value::Link(_)
            | Value::Lifecycle(_) => {}
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
