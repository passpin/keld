use keld_runtime::{
    EntityId, Generation, Link, RuntimeLifecycleId, RuntimeTypeId, SlotIndex, Store, StoreError,
    StoreInvariantError,
};
use std::collections::BTreeMap;

#[test]
fn store_matches_reference_model_for_generated_sequences() {
    for seed in 0_u64..256 {
        let mut rng = XorShift64::new(seed + 1);
        let mut real = TestStore::new();
        let mut model = ModelStore::new();
        for step in 0..1_000 {
            let operation = Operation::generate(&mut rng, &model);
            let actual = real.apply(&operation);
            let expected = model.apply(&operation);
            assert_eq!(actual, expected, "seed={seed} step={step} {operation:?}");
            real.assert_invariants(&model, seed, step);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObservedIdentity {
    slot: SlotIndex,
    generation: Generation,
    definition: RuntimeTypeId,
}

#[derive(Clone, Debug)]
enum Operation {
    Allocate {
        lifecycle: usize,
        definition: RuntimeTypeId,
    },
    BeginLifecycle {
        parent: usize,
    },
    Link {
        entity: usize,
    },
    Resolve {
        link: usize,
    },
    Retire {
        entity: usize,
    },
    Keep {
        entity: usize,
        target: usize,
    },
    EndLifecycle {
        lifecycle: usize,
    },
}

impl Operation {
    fn generate(rng: &mut XorShift64, model: &ModelStore) -> Self {
        let choice = rng.index(100);
        if model.entities.is_empty() || choice < 26 {
            return Self::Allocate {
                lifecycle: choose_lifecycle(rng, model),
                definition: RuntimeTypeId(u32::try_from(rng.index(4)).expect("definition fits")),
            };
        }
        match choice {
            26..=37 => Self::BeginLifecycle {
                parent: choose_lifecycle(rng, model),
            },
            38..=51 => Self::Link {
                entity: choose_entity(rng, model),
            },
            52..=64 => {
                if model.links.is_empty() {
                    Self::Link {
                        entity: choose_entity(rng, model),
                    }
                } else {
                    Self::Resolve {
                        link: rng.index(model.links.len()),
                    }
                }
            }
            65..=77 => Self::Retire {
                entity: choose_entity(rng, model),
            },
            78..=89 => Self::Keep {
                entity: choose_entity(rng, model),
                target: choose_lifecycle(rng, model),
            },
            _ => Self::EndLifecycle {
                lifecycle: rng.index(model.lifecycles.len()),
            },
        }
    }
}

fn choose_entity(rng: &mut XorShift64, model: &ModelStore) -> usize {
    let live = model
        .entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| entity.live.then_some(index))
        .collect::<Vec<_>>();
    if !live.is_empty() && rng.index(3) != 0 {
        live[rng.index(live.len())]
    } else {
        rng.index(model.entities.len())
    }
}

fn choose_lifecycle(rng: &mut XorShift64, model: &ModelStore) -> usize {
    let active = model
        .lifecycles
        .iter()
        .enumerate()
        .filter_map(|(index, lifecycle)| lifecycle.active.then_some(index))
        .collect::<Vec<_>>();
    if rng.index(3) != 0 {
        active[rng.index(active.len())]
    } else {
        rng.index(model.lifecycles.len())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Outcome {
    Identity(ObservedIdentity),
    Lifecycle(usize),
    Link(usize),
    Resolution(Option<ObservedIdentity>),
    Cleanup(Vec<u32>),
    Done,
    Error(ErrorClass),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorClass {
    Allocation,
    BrandExhausted,
    ForeignIdentity,
    StaleEntity,
    InvalidLifecycle,
    NonAncestorKeep,
    RootEnd,
    ActiveChild,
    AlreadyFinished,
}

impl From<StoreError> for ErrorClass {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Allocation => Self::Allocation,
            StoreError::BrandExhausted => Self::BrandExhausted,
            StoreError::InvalidOperation(error) => match error {
                StoreInvariantError::ForeignIdentity => Self::ForeignIdentity,
                StoreInvariantError::StaleEntity => Self::StaleEntity,
                StoreInvariantError::InvalidLifecycle => Self::InvalidLifecycle,
                StoreInvariantError::NonAncestorKeep => Self::NonAncestorKeep,
                StoreInvariantError::RootEnd => Self::RootEnd,
                StoreInvariantError::ActiveChild => Self::ActiveChild,
                StoreInvariantError::AlreadyFinished => Self::AlreadyFinished,
            },
        }
    }
}

struct TestStore {
    store: Store<u32>,
    entities: Vec<RealEntity>,
    links: Vec<RealLink>,
    lifecycles: Vec<RealLifecycle>,
}

struct RealEntity {
    id: EntityId,
    identity: ObservedIdentity,
    live: bool,
    lifecycle: usize,
}

struct RealLink {
    link: Link,
    target: usize,
}

struct RealLifecycle {
    id: RuntimeLifecycleId,
    parent: Option<usize>,
    active: bool,
}

impl TestStore {
    fn new() -> Self {
        let store = Store::new().expect("test store brand must be available");
        let root = store.root_lifecycle();
        Self {
            store,
            entities: Vec::new(),
            links: Vec::new(),
            lifecycles: vec![RealLifecycle {
                id: root,
                parent: None,
                active: true,
            }],
        }
    }

    fn apply(&mut self, operation: &Operation) -> Outcome {
        match *operation {
            Operation::Allocate {
                lifecycle,
                definition,
            } => self.allocate(lifecycle, definition),
            Operation::BeginLifecycle { parent } => self.begin_lifecycle(parent),
            Operation::Link { entity } => self.link(entity),
            Operation::Resolve { link } => {
                let resolved = self.store.resolve(self.links[link].link);
                Outcome::Resolution(resolved.map(|entity| self.observe_resolved(entity)))
            }
            Operation::Retire { entity } => self.retire(entity),
            Operation::Keep { entity, target } => self.keep(entity, target),
            Operation::EndLifecycle { lifecycle } => self.end_lifecycle(lifecycle),
        }
    }

    fn allocate(&mut self, lifecycle: usize, definition: RuntimeTypeId) -> Outcome {
        let handle = u32::try_from(self.entities.len()).expect("test entity handle fits u32");
        match self
            .store
            .allocate(definition, self.lifecycles[lifecycle].id, handle)
        {
            Ok(id) => {
                let identity = observe(id, definition);
                self.entities.push(RealEntity {
                    id,
                    identity,
                    live: true,
                    lifecycle,
                });
                Outcome::Identity(identity)
            }
            Err(error) => Outcome::Error(error.into()),
        }
    }

    fn begin_lifecycle(&mut self, parent: usize) -> Outcome {
        match self.store.begin_lifecycle(self.lifecycles[parent].id) {
            Ok(id) => {
                let handle = self.lifecycles.len();
                self.lifecycles.push(RealLifecycle {
                    id,
                    parent: Some(parent),
                    active: true,
                });
                Outcome::Lifecycle(handle)
            }
            Err(error) => Outcome::Error(error.into()),
        }
    }

    fn link(&mut self, entity: usize) -> Outcome {
        match self.store.link(self.entities[entity].id) {
            Ok(link) => {
                let handle = self.links.len();
                self.links.push(RealLink {
                    link,
                    target: entity,
                });
                Outcome::Link(handle)
            }
            Err(error) => Outcome::Error(error.into()),
        }
    }

    fn retire(&mut self, entity: usize) -> Outcome {
        let mut cleanup = Vec::new();
        match self
            .store
            .retire_with(self.entities[entity].id, |payload| cleanup.push(payload))
        {
            Ok(()) => {
                self.record_cleanup(&cleanup);
                Outcome::Cleanup(cleanup)
            }
            Err(error) => Outcome::Error(error.into()),
        }
    }

    fn keep(&mut self, entity: usize, target: usize) -> Outcome {
        match self
            .store
            .keep(self.entities[entity].id, self.lifecycles[target].id)
        {
            Ok(()) => {
                self.entities[entity].lifecycle = target;
                Outcome::Done
            }
            Err(error) => Outcome::Error(error.into()),
        }
    }

    fn end_lifecycle(&mut self, lifecycle: usize) -> Outcome {
        let mut cleanup = Vec::new();
        match self
            .store
            .end_lifecycle_with(self.lifecycles[lifecycle].id, |payload| {
                cleanup.push(payload);
            }) {
            Ok(()) => {
                self.record_cleanup(&cleanup);
                self.lifecycles[lifecycle].active = false;
                Outcome::Cleanup(cleanup)
            }
            Err(error) => Outcome::Error(error.into()),
        }
    }

    fn record_cleanup(&mut self, cleanup: &[u32]) {
        for payload in cleanup {
            self.entities[*payload as usize].live = false;
        }
    }

    fn observe_resolved(&self, entity: EntityId) -> ObservedIdentity {
        self.entities
            .iter()
            .find(|candidate| candidate.id == entity)
            .map(|candidate| candidate.identity)
            .expect("resolved identity must have been allocated by this store")
    }

    fn assert_invariants(&self, model: &ModelStore, seed: u64, step: usize) {
        assert_eq!(self.entities.len(), model.entities.len());
        assert_eq!(self.links.len(), model.links.len());
        assert_eq!(self.lifecycles.len(), model.lifecycles.len());

        let mut live_slots = BTreeMap::new();
        for (handle, (actual, expected)) in self.entities.iter().zip(&model.entities).enumerate() {
            assert_eq!(
                actual.identity, expected.identity,
                "seed={seed} step={step}"
            );
            assert_eq!(actual.live, expected.live, "seed={seed} step={step}");
            assert_eq!(
                actual.lifecycle, expected.lifecycle,
                "seed={seed} step={step}"
            );
            let read = self
                .store
                .read(actual.id, |payload| *payload)
                .map_err(ErrorClass::from);
            if expected.live {
                assert_eq!(
                    read,
                    Ok(u32::try_from(handle).expect("test entity handle fits")),
                    "seed={seed} step={step}"
                );
                assert!(
                    live_slots
                        .insert(actual.identity.slot.0, (handle, actual.identity))
                        .is_none(),
                    "two live identities occupy one slot: seed={seed} step={step}"
                );
            } else {
                assert_eq!(
                    read,
                    Err(ErrorClass::StaleEntity),
                    "seed={seed} step={step}"
                );
            }
        }

        for (handle, (actual, expected)) in self.links.iter().zip(&model.links).enumerate() {
            assert_eq!(actual.target, expected.target, "seed={seed} step={step}");
            let resolved = self
                .store
                .resolve(actual.link)
                .map(|entity| self.observe_resolved(entity));
            assert_eq!(resolved, model.resolve(handle), "seed={seed} step={step}");
        }

        for (actual, expected) in self.lifecycles.iter().zip(&model.lifecycles) {
            assert_eq!(actual.parent, expected.parent, "seed={seed} step={step}");
            assert_eq!(actual.active, expected.active, "seed={seed} step={step}");
        }

        let expected_slots = model
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot, state)| {
                state.live.map(|entity| {
                    (
                        u32::try_from(slot).expect("model slot fits"),
                        (entity, model.entities[entity].identity),
                    )
                })
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(live_slots, expected_slots, "seed={seed} step={step}");
    }
}

const fn observe(entity: EntityId, definition: RuntimeTypeId) -> ObservedIdentity {
    ObservedIdentity {
        slot: entity.slot_index(),
        generation: entity.generation(),
        definition,
    }
}

struct ModelStore {
    slots: Vec<ModelSlot>,
    reusable: Vec<usize>,
    entities: Vec<ModelEntity>,
    links: Vec<ModelLink>,
    lifecycles: Vec<ModelLifecycle>,
}

struct ModelSlot {
    generation: Generation,
    live: Option<usize>,
}

struct ModelEntity {
    identity: ObservedIdentity,
    live: bool,
    lifecycle: usize,
    adoption: usize,
}

struct ModelLink {
    target: usize,
}

struct ModelLifecycle {
    parent: Option<usize>,
    active: bool,
    active_children: u32,
    adoptions: Vec<Option<usize>>,
}

impl ModelStore {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            reusable: Vec::new(),
            entities: Vec::new(),
            links: Vec::new(),
            lifecycles: vec![ModelLifecycle {
                parent: None,
                active: true,
                active_children: 0,
                adoptions: Vec::new(),
            }],
        }
    }

    fn apply(&mut self, operation: &Operation) -> Outcome {
        match *operation {
            Operation::Allocate {
                lifecycle,
                definition,
            } => self.allocate(lifecycle, definition),
            Operation::BeginLifecycle { parent } => self.begin_lifecycle(parent),
            Operation::Link { entity } => self.link(entity),
            Operation::Resolve { link } => Outcome::Resolution(self.resolve(link)),
            Operation::Retire { entity } => self.retire(entity),
            Operation::Keep { entity, target } => self.keep(entity, target),
            Operation::EndLifecycle { lifecycle } => self.end_lifecycle(lifecycle),
        }
    }

    fn allocate(&mut self, lifecycle: usize, definition: RuntimeTypeId) -> Outcome {
        if !self.lifecycles[lifecycle].active {
            return Outcome::Error(ErrorClass::InvalidLifecycle);
        }
        if self.reusable.is_empty() {
            let base = self.slots.len();
            self.slots.extend((0..64).map(|_| ModelSlot {
                generation: Generation(0),
                live: None,
            }));
            self.reusable.extend((base..base + 64).rev());
        }
        let slot = self.reusable.pop().expect("a model slot was provisioned");
        assert!(self.slots[slot].live.is_none());
        let generation = self.slots[slot].generation;
        let handle = self.entities.len();
        let adoption = self.lifecycles[lifecycle].adoptions.len();
        self.lifecycles[lifecycle].adoptions.push(Some(handle));
        self.slots[slot].live = Some(handle);
        let identity = ObservedIdentity {
            slot: SlotIndex(u32::try_from(slot).expect("model slot fits u32")),
            generation,
            definition,
        };
        self.entities.push(ModelEntity {
            identity,
            live: true,
            lifecycle,
            adoption,
        });
        Outcome::Identity(identity)
    }

    fn begin_lifecycle(&mut self, parent: usize) -> Outcome {
        if !self.lifecycles[parent].active {
            return Outcome::Error(ErrorClass::InvalidLifecycle);
        }
        self.lifecycles[parent].active_children += 1;
        let handle = self.lifecycles.len();
        self.lifecycles.push(ModelLifecycle {
            parent: Some(parent),
            active: true,
            active_children: 0,
            adoptions: Vec::new(),
        });
        Outcome::Lifecycle(handle)
    }

    fn link(&mut self, entity: usize) -> Outcome {
        if !self.entities[entity].live {
            return Outcome::Error(ErrorClass::StaleEntity);
        }
        let handle = self.links.len();
        self.links.push(ModelLink { target: entity });
        Outcome::Link(handle)
    }

    fn resolve(&self, link: usize) -> Option<ObservedIdentity> {
        let entity = &self.entities[self.links[link].target];
        entity.live.then_some(entity.identity)
    }

    fn retire(&mut self, entity: usize) -> Outcome {
        if !self.entities[entity].live {
            return Outcome::Error(ErrorClass::StaleEntity);
        }
        self.remove_adoption(entity);
        self.finalize_retirement(entity);
        Outcome::Cleanup(vec![
            u32::try_from(entity).expect("test entity handle fits"),
        ])
    }

    fn keep(&mut self, entity: usize, target: usize) -> Outcome {
        if !self.entities[entity].live {
            return Outcome::Error(ErrorClass::StaleEntity);
        }
        if !self.lifecycles[target].active {
            return Outcome::Error(ErrorClass::InvalidLifecycle);
        }
        let source = self.entities[entity].lifecycle;
        if !self.is_strict_ancestor(target, source) {
            return Outcome::Error(ErrorClass::NonAncestorKeep);
        }
        let new_adoption = self.lifecycles[target].adoptions.len();
        self.lifecycles[target].adoptions.push(Some(entity));
        self.remove_adoption(entity);
        self.entities[entity].lifecycle = target;
        self.entities[entity].adoption = new_adoption;
        Outcome::Done
    }

    fn end_lifecycle(&mut self, lifecycle: usize) -> Outcome {
        if !self.lifecycles[lifecycle].active {
            return Outcome::Error(ErrorClass::InvalidLifecycle);
        }
        if lifecycle == 0 {
            return Outcome::Error(ErrorClass::RootEnd);
        }
        if self.lifecycles[lifecycle].active_children != 0 {
            return Outcome::Error(ErrorClass::ActiveChild);
        }
        let cleanup = self.lifecycles[lifecycle]
            .adoptions
            .iter()
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        for entity in &cleanup {
            assert!(self.entities[*entity].live);
            assert_eq!(self.entities[*entity].lifecycle, lifecycle);
            self.finalize_retirement(*entity);
        }
        self.lifecycles[lifecycle]
            .adoptions
            .iter_mut()
            .for_each(|adoption| *adoption = None);
        self.lifecycles[lifecycle].active = false;
        let parent = self.lifecycles[lifecycle]
            .parent
            .expect("non-root lifecycle has a parent");
        self.lifecycles[parent].active_children -= 1;
        Outcome::Cleanup(
            cleanup
                .into_iter()
                .map(|entity| u32::try_from(entity).expect("test entity handle fits"))
                .collect(),
        )
    }

    fn remove_adoption(&mut self, entity: usize) {
        let owner = self.entities[entity].lifecycle;
        let adoption = self.entities[entity].adoption;
        assert_eq!(self.lifecycles[owner].adoptions[adoption], Some(entity));
        self.lifecycles[owner].adoptions[adoption] = None;
    }

    fn finalize_retirement(&mut self, entity: usize) {
        let slot = self.entities[entity].identity.slot.0 as usize;
        assert_eq!(self.slots[slot].live, Some(entity));
        self.slots[slot].live = None;
        self.slots[slot].generation = Generation(
            self.slots[slot]
                .generation
                .0
                .checked_add(1)
                .expect("generated sequence cannot exhaust u32 generations"),
        );
        self.reusable.push(slot);
        self.entities[entity].live = false;
    }

    fn is_strict_ancestor(&self, target: usize, source: usize) -> bool {
        let mut current = self.lifecycles[source].parent;
        while let Some(lifecycle) = current {
            if lifecycle == target {
                return true;
            }
            current = self.lifecycles[lifecycle].parent;
        }
        false
    }
}

struct XorShift64(u64);

impl XorShift64 {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn index(&mut self, upper: usize) -> usize {
        let upper = u64::try_from(upper).expect("test collection length fits u64");
        usize::try_from(self.next() % upper).expect("generated index fits usize")
    }
}
