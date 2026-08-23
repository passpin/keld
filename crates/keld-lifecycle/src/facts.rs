use crate::provenance::AbstractState;
use crate::{ProvenanceId, RefState};
use keld_flow::ValueId;
use keld_semantics::{DefId, LocalId};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntityReferenceFact {
    pub definition: DefId,
    pub provenance: ProvenanceId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntityOriginFact {
    pub parameters: BTreeSet<u32>,
    pub fresh: bool,
    pub broad: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AliasRelation {
    MustAlias,
    MustDistinct,
    MayAlias,
}

#[derive(Clone, Debug)]
pub struct EntityOperationFacts {
    local_references: Vec<Option<EntityReferenceFact>>,
    value_references: Vec<Option<EntityReferenceFact>>,
    origins: Vec<EntityOriginFact>,
    reference_states: Vec<RefState>,
    equivalence_classes: Vec<BTreeSet<ProvenanceId>>,
    distinct_pairs: BTreeSet<(ProvenanceId, ProvenanceId)>,
}

impl EntityOperationFacts {
    pub(crate) fn project(state: &AbstractState) -> Self {
        let reference = |value: crate::provenance::RefValue| EntityReferenceFact {
            definition: value.entity,
            provenance: value.provenance,
        };
        let origins = state
            .origins
            .iter()
            .map(|origin| EntityOriginFact {
                parameters: origin.parameters.clone(),
                fresh: origin.fresh,
                broad: origin.resolved || origin.opaque,
            })
            .collect();
        let equivalence_classes = (0..state.refs.len())
            .map(|index| {
                state.equivalence_class(ProvenanceId(
                    u32::try_from(index).expect("provenance index fits in u32"),
                ))
            })
            .collect();
        Self {
            local_references: state
                .locals
                .iter()
                .copied()
                .map(|value| value.map(reference))
                .collect(),
            value_references: state
                .values
                .iter()
                .copied()
                .map(|value| value.map(reference))
                .collect(),
            origins,
            reference_states: state.refs.clone(),
            equivalence_classes,
            distinct_pairs: state.distinct.clone(),
        }
    }

    #[must_use]
    pub fn local_reference(&self, local: LocalId) -> Option<EntityReferenceFact> {
        self.local_references
            .get(local.0 as usize)
            .copied()
            .flatten()
    }

    #[must_use]
    pub fn value_reference(&self, value: ValueId) -> Option<EntityReferenceFact> {
        self.value_references
            .get(value.0 as usize)
            .copied()
            .flatten()
    }

    #[must_use]
    pub fn origin(&self, provenance: ProvenanceId) -> Option<&EntityOriginFact> {
        self.origins.get(provenance.0 as usize)
    }

    #[must_use]
    pub fn reference_state(&self, provenance: ProvenanceId) -> Option<&RefState> {
        self.reference_states.get(provenance.0 as usize)
    }

    #[must_use]
    pub fn alias_relation(
        &self,
        lhs: EntityReferenceFact,
        rhs: EntityReferenceFact,
    ) -> AliasRelation {
        if lhs.definition != rhs.definition {
            return AliasRelation::MustDistinct;
        }
        let lhs_class = self
            .equivalence_classes
            .get(lhs.provenance.0 as usize)
            .cloned()
            .unwrap_or_else(|| BTreeSet::from([lhs.provenance]));
        let rhs_class = self
            .equivalence_classes
            .get(rhs.provenance.0 as usize)
            .cloned()
            .unwrap_or_else(|| BTreeSet::from([rhs.provenance]));
        if lhs_class.contains(&rhs.provenance) {
            return AliasRelation::MustAlias;
        }
        if lhs_class.iter().any(|left| {
            rhs_class
                .iter()
                .any(|right| self.distinct_pairs.contains(&ordered(*left, *right)))
        }) {
            AliasRelation::MustDistinct
        } else {
            AliasRelation::MayAlias
        }
    }
}

fn ordered(lhs: ProvenanceId, rhs: ProvenanceId) -> (ProvenanceId, ProvenanceId) {
    if lhs <= rhs { (lhs, rhs) } else { (rhs, lhs) }
}
