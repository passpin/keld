use keld_flow::{FlowFunction, FlowModule, Place, PlaceProjection, ValueId};
use keld_lifecycle::{
    AliasRelation, EntityOperationFacts, EntityReferenceFact, VerifiedFlowModule,
};
use keld_semantics::{DefId, LocalId, TypeId, TypeKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccessRoot {
    Home(LocalId),
    Entity(EntityAccessRoot),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EntityAccessRoot {
    Exact(EntityReferenceFact),
    Any(DefId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccessPath {
    pub root: AccessRoot,
    pub projections: Vec<PlaceProjection>,
}

impl AccessPath {
    pub fn for_place(
        flow: &FlowModule,
        function: &FlowFunction,
        facts: &EntityOperationFacts,
        place: &Place,
    ) -> Self {
        let root = entity_definition(flow, function.local_types[place.base.0 as usize]).map_or(
            AccessRoot::Home(place.base),
            |definition| {
                let entity = facts
                    .local_reference(place.base)
                    .filter(|reference| reference.definition == definition)
                    .map_or(EntityAccessRoot::Any(definition), EntityAccessRoot::Exact);
                AccessRoot::Entity(entity)
            },
        );
        Self {
            root,
            projections: place.projections.clone(),
        }
    }

    pub fn push(&mut self, projection: PlaceProjection) {
        self.projections.push(projection);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ValueOrigin {
    Implicit,
    Local(LocalId),
    Borrowed(LocalId),
    BorrowedPlace {
        place: Place,
        access: AccessPath,
        loaned: bool,
    },
    BorrowedValue {
        root: ValueId,
        projections: Vec<PlaceProjection>,
    },
    BorrowedUnknown,
    Entity(LocalId),
    Owned,
    Unknown,
}

pub(crate) struct OriginAccess {
    pub path: AccessPath,
    pub place: Place,
    pub loaned: bool,
}

pub(crate) fn origin_access(
    flow: &FlowModule,
    function: &FlowFunction,
    facts: &EntityOperationFacts,
    origin: &ValueOrigin,
) -> Option<OriginAccess> {
    let (place, loaned) = match origin {
        ValueOrigin::Local(local) => (
            Place {
                base: *local,
                projections: Vec::new(),
            },
            false,
        ),
        ValueOrigin::Borrowed(local) | ValueOrigin::Entity(local) => (
            Place {
                base: *local,
                projections: Vec::new(),
            },
            true,
        ),
        ValueOrigin::BorrowedPlace {
            place,
            access,
            loaned,
        } => {
            return Some(OriginAccess {
                path: access.clone(),
                place: place.clone(),
                loaned: *loaned,
            });
        }
        ValueOrigin::BorrowedValue { .. }
        | ValueOrigin::BorrowedUnknown
        | ValueOrigin::Implicit
        | ValueOrigin::Owned
        | ValueOrigin::Unknown => return None,
    };
    Some(OriginAccess {
        path: AccessPath::for_place(flow, function, facts, &place),
        place,
        loaned,
    })
}

pub(crate) fn paths_overlap(
    facts: &EntityOperationFacts,
    left: &AccessPath,
    right: &AccessPath,
) -> bool {
    if !roots_overlap(facts, &left.root, &right.root) {
        return false;
    }
    let shared = left
        .projections
        .iter()
        .zip(&right.projections)
        .take_while(|(left, right)| projections_overlap(left, right))
        .count();
    shared == left.projections.len().min(right.projections.len())
}

pub(crate) fn is_strict_prefix(
    facts: &EntityOperationFacts,
    prefix: &AccessPath,
    descendant: &AccessPath,
) -> bool {
    roots_overlap(facts, &prefix.root, &descendant.root)
        && prefix.projections.len() < descendant.projections.len()
        && prefix
            .projections
            .iter()
            .zip(&descendant.projections)
            .all(|(prefix, descendant)| projections_overlap(prefix, descendant))
}

pub(crate) fn indexed_destination_conflict(
    facts: &EntityOperationFacts,
    list: &AccessPath,
    access: &AccessPath,
) -> bool {
    roots_overlap(facts, &list.root, &access.root)
        && access.projections.len() <= list.projections.len()
        && list
            .projections
            .iter()
            .zip(&access.projections)
            .all(|(list, access)| projections_overlap(list, access))
}

fn roots_overlap(facts: &EntityOperationFacts, left: &AccessRoot, right: &AccessRoot) -> bool {
    match (left, right) {
        (AccessRoot::Home(left), AccessRoot::Home(right)) => left == right,
        (AccessRoot::Entity(left), AccessRoot::Entity(right)) => match (left, right) {
            (EntityAccessRoot::Exact(left), EntityAccessRoot::Exact(right)) => {
                facts.alias_relation(*left, *right) != AliasRelation::MustDistinct
            }
            (EntityAccessRoot::Any(left), EntityAccessRoot::Any(right)) => left == right,
            (EntityAccessRoot::Any(left), EntityAccessRoot::Exact(right))
            | (EntityAccessRoot::Exact(right), EntityAccessRoot::Any(left)) => {
                *left == right.definition
            }
        },
        (AccessRoot::Home(_), AccessRoot::Entity(_))
        | (AccessRoot::Entity(_), AccessRoot::Home(_)) => false,
    }
}

fn projections_overlap(left: &PlaceProjection, right: &PlaceProjection) -> bool {
    match (left, right) {
        (PlaceProjection::Field(left), PlaceProjection::Field(right)) => left == right,
        (PlaceProjection::Index(left), PlaceProjection::Index(right)) => match (left, right) {
            (
                keld_flow::IndexIdentity::Constant(left),
                keld_flow::IndexIdentity::Constant(right),
            ) => left == right,
            _ => true,
        },
        _ => false,
    }
}

fn entity_definition(flow: &FlowModule, ty: TypeId) -> Option<DefId> {
    match flow.types.kind(ty) {
        TypeKind::EntityRef(definition) => Some(*definition),
        TypeKind::Optional(inner) => entity_definition(flow, *inner),
        TypeKind::Unit
        | TypeKind::Bool
        | TypeKind::Int
        | TypeKind::Text
        | TypeKind::List(_)
        | TypeKind::Struct(_)
        | TypeKind::Link { .. }
        | TypeKind::Error => None,
    }
}

pub(crate) fn operation_facts<'module>(
    lifecycle: &'module VerifiedFlowModule,
    function: &FlowFunction,
    block: keld_flow::BlockId,
    operation_index: u32,
) -> &'module EntityOperationFacts {
    lifecycle
        .entity_facts_at(function.id, block, operation_index)
        .expect("verified flow publishes facts for every operation")
}
