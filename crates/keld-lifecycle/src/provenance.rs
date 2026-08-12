use crate::{LifecycleFact, ProvenanceId, RefState};
use keld_semantics::DefId;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Origin {
    pub parameters: BTreeSet<u32>,
    pub fresh: bool,
    pub resolved: bool,
    pub opaque: bool,
}

impl Origin {
    pub fn union_with(&mut self, other: &Self) {
        self.parameters.extend(other.parameters.iter().copied());
        self.fresh |= other.fresh;
        self.resolved |= other.resolved;
        self.opaque |= other.opaque;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RefValue {
    pub provenance: ProvenanceId,
    pub entity: DefId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureKind {
    Alias,
    Join,
    Scope,
    Retired,
}

#[derive(Clone, Debug)]
pub(crate) struct AbstractState {
    pub locals: Vec<Option<RefValue>>,
    pub values: Vec<Option<RefValue>>,
    pub refs: Vec<RefState>,
    pub origins: Vec<Origin>,
    pub failures: Vec<Option<FailureKind>>,
    pub equal: BTreeSet<(ProvenanceId, ProvenanceId)>,
    pub distinct: BTreeSet<(ProvenanceId, ProvenanceId)>,
}

impl AbstractState {
    pub fn new(local_count: usize, value_count: usize, provenance_count: usize) -> Self {
        Self {
            locals: vec![None; local_count],
            values: vec![None; value_count],
            refs: vec![RefState::OutOfScope; provenance_count],
            origins: vec![Origin::default(); provenance_count],
            failures: vec![None; provenance_count],
            equal: BTreeSet::new(),
            distinct: BTreeSet::new(),
        }
    }

    pub fn set_live(&mut self, provenance: ProvenanceId, lifecycle: LifecycleFact, origin: Origin) {
        let index = provenance.0 as usize;
        self.refs[index] = RefState::Live {
            lifecycle,
            provenance,
        };
        self.origins[index] = origin;
        self.failures[index] = None;
    }

    pub fn equivalent(&self, lhs: ProvenanceId, rhs: ProvenanceId) -> bool {
        if lhs == rhs {
            return true;
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![lhs];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            for &(first, second) in &self.equal {
                let next = if first == current {
                    Some(second)
                } else if second == current {
                    Some(first)
                } else {
                    None
                };
                if next == Some(rhs) {
                    return true;
                }
                if let Some(next) = next {
                    stack.push(next);
                }
            }
        }
        false
    }

    pub fn add_equal(&mut self, lhs: ProvenanceId, rhs: ProvenanceId) {
        if lhs != rhs {
            self.equal.insert(ordered(lhs, rhs));
        }
    }

    pub fn add_distinct(&mut self, lhs: ProvenanceId, rhs: ProvenanceId) {
        if lhs != rhs {
            self.distinct.insert(ordered(lhs, rhs));
        }
    }

    pub fn are_distinct(&self, lhs: ProvenanceId, rhs: ProvenanceId) -> bool {
        let lhs_class = self.equivalence_class(lhs);
        let rhs_class = self.equivalence_class(rhs);
        lhs_class.iter().any(|left| {
            rhs_class
                .iter()
                .any(|right| self.distinct.contains(&ordered(*left, *right)))
        })
    }

    pub fn equivalence_class(&self, root: ProvenanceId) -> BTreeSet<ProvenanceId> {
        let mut class = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(current) = stack.pop() {
            if !class.insert(current) {
                continue;
            }
            for &(first, second) in &self.equal {
                if first == current {
                    stack.push(second);
                } else if second == current {
                    stack.push(first);
                }
            }
        }
        class
    }

    pub fn join(states: &[Self], fallback_cause: keld_source::Span) -> Self {
        let mut joined = states[0].clone();
        for state in &states[1..] {
            for index in 0..joined.locals.len() {
                if joined.locals[index] != state.locals[index] {
                    joined.locals[index] = None;
                }
            }
            for index in 0..joined.values.len() {
                if joined.values[index] != state.values[index] {
                    joined.values[index] = None;
                }
            }
            for index in 0..joined.refs.len() {
                joined.origins[index].union_with(&state.origins[index]);
                joined.refs[index] =
                    join_ref(&joined.refs[index], &state.refs[index], fallback_cause);
                joined.failures[index] = join_failure(
                    joined.failures[index],
                    state.failures[index],
                    &joined.refs[index],
                );
            }
            joined.equal = joined.equal.intersection(&state.equal).copied().collect();
            joined.distinct = joined
                .distinct
                .intersection(&state.distinct)
                .copied()
                .collect();
        }
        joined
    }

    pub fn lifecycle(&self, provenance: ProvenanceId) -> Option<LifecycleFact> {
        match self.refs.get(provenance.0 as usize) {
            Some(RefState::Live { lifecycle, .. }) => Some(*lifecycle),
            _ => None,
        }
    }
}

fn ordered(lhs: ProvenanceId, rhs: ProvenanceId) -> (ProvenanceId, ProvenanceId) {
    if lhs <= rhs { (lhs, rhs) } else { (rhs, lhs) }
}

fn join_ref(lhs: &RefState, rhs: &RefState, cause: keld_source::Span) -> RefState {
    match (lhs, rhs) {
        (
            RefState::Live {
                lifecycle: lhs_lifecycle,
                provenance,
            },
            RefState::Live {
                lifecycle: rhs_lifecycle,
                ..
            },
        ) => RefState::Live {
            lifecycle: if lhs_lifecycle == rhs_lifecycle {
                *lhs_lifecycle
            } else {
                LifecycleFact::Dynamic
            },
            provenance: *provenance,
        },
        (RefState::Retired { cause }, RefState::Retired { .. }) => {
            RefState::Retired { cause: *cause }
        }
        (RefState::Invalidated { cause }, RefState::Invalidated { .. }) => {
            RefState::Invalidated { cause: *cause }
        }
        (RefState::OutOfScope, RefState::OutOfScope) => RefState::OutOfScope,
        _ => RefState::Invalidated { cause },
    }
}

fn join_failure(
    lhs: Option<FailureKind>,
    rhs: Option<FailureKind>,
    joined: &RefState,
) -> Option<FailureKind> {
    if matches!(joined, RefState::Live { .. }) {
        None
    } else if lhs == rhs && lhs.is_some() {
        lhs
    } else {
        Some(FailureKind::Join)
    }
}
