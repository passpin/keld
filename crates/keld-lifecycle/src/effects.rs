use keld_semantics::DefId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReturnProvenance {
    NonEntity,
    EntitySources {
        parameters: Vec<u32>,
        fresh_in_caller_lifecycle: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionSummary {
    pub retires_parameters: Vec<u32>,
    pub retires_any: Vec<DefId>,
    pub return_provenance: ReturnProvenance,
}

impl Default for FunctionSummary {
    fn default() -> Self {
        Self {
            retires_parameters: Vec::new(),
            retires_any: Vec::new(),
            return_provenance: ReturnProvenance::NonEntity,
        }
    }
}
