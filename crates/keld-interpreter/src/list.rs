use crate::value::{CopyAllocation, Value, try_copy_value};

#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeList {
    elements: Vec<Value>,
}

impl RuntimeList {
    #[doc(hidden)]
    #[must_use]
    pub fn with_capacity_for_test(capacity: usize) -> Self {
        Self {
            elements: Vec::with_capacity(capacity),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn from_values(values: Vec<Value>) -> Self {
        Self { elements: values }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn length(&self) -> usize {
        self.elements.len()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn capacity_for_test(&self) -> usize {
        self.elements.capacity()
    }

    #[doc(hidden)]
    pub fn push_for_test(&mut self, value: Value) {
        self.elements.push(value);
    }

    #[doc(hidden)]
    pub fn get_copy(&self, index: usize) -> Result<Option<Value>, CopyAllocation> {
        self.elements.get(index).map(try_copy_value).transpose()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn into_elements(self) -> std::vec::IntoIter<Value> {
        self.elements.into_iter()
    }

    pub(crate) fn try_reserve(&mut self, additional: usize) -> Result<(), ()> {
        self.elements.try_reserve(additional).map_err(|_| ())
    }

    pub(crate) fn push(&mut self, value: Value) {
        self.elements.push(value);
    }

    #[doc(hidden)]
    pub fn remove(&mut self, index: usize) -> Value {
        self.elements.remove(index)
    }

    #[doc(hidden)]
    pub fn try_remove(&mut self, index: usize) -> Option<Value> {
        (index < self.elements.len()).then(|| self.remove(index))
    }

    #[doc(hidden)]
    pub fn clear_into(&mut self, cleanup: &mut Vec<Value>) {
        while let Some(value) = self.elements.pop() {
            cleanup.push(value);
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&Value> {
        self.elements.get(index)
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.elements.get_mut(index)
    }

    pub(crate) fn as_slice(&self) -> &[Value] {
        &self.elements
    }
}
