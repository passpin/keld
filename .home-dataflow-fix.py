from pathlib import Path

path = Path("crates/keld-ir/src/validate.rs")
text = path.read_text(encoding="utf-8")

old_enum = '''enum HomeState {
    Empty,
    Live,
    MaybeLive,
}
'''
new_enum = '''enum HomeState {
    Unknown,
    Empty,
    Live,
    MaybeLive,
}
'''
if old_enum not in text:
    if new_enum in text:
        print("HomeState::Unknown already present")
    else:
        raise SystemExit("HomeState enum marker not found")
else:
    text = text.replace(old_enum, new_enum, 1)

old_init = '''            self.incoming_homes[block.id.0 as usize] = if block.id == self.function.entry {
                initial.clone()
            } else {
                self.all_home_states(HomeState::Empty)
            };
'''
new_init = '''            self.incoming_homes[block.id.0 as usize] = if block.id == self.function.entry {
                initial.clone()
            } else {
                self.all_home_states(HomeState::Unknown)
            };
'''
if old_init in text:
    text = text.replace(old_init, new_init, 1)
elif new_init not in text:
    raise SystemExit("home dataflow initialization marker not found")

old_join_start = '''        let mut joined = self.all_home_states(HomeState::Empty);
        for register in joined.keys().copied().collect::<Vec<_>>() {
            let mut state = None;
            for predecessor in &self.predecessors[block.0 as usize] {
                let predecessor_state = self.outgoing_homes[predecessor.0 as usize]
                    .get(&register)
                    .copied()
                    .unwrap_or(HomeState::Empty);
                state = Some(match state {
                    Some(current) => join_home_state(current, predecessor_state),
                    None => predecessor_state,
                });
            }
            if let Some(state) = state {
                joined.insert(register, state);
            }
        }
'''
new_join_start = '''        let mut joined = self.all_home_states(HomeState::Unknown);
        for register in joined.keys().copied().collect::<Vec<_>>() {
            let mut state = HomeState::Unknown;
            for predecessor in &self.predecessors[block.0 as usize] {
                if !self.reachable_blocks.contains(predecessor) {
                    continue;
                }
                let predecessor_state = self.outgoing_homes[predecessor.0 as usize]
                    .get(&register)
                    .copied()
                    .unwrap_or(HomeState::Unknown);
                state = join_home_state(state, predecessor_state);
            }
            joined.insert(register, state);
        }
'''
if old_join_start in text:
    text = text.replace(old_join_start, new_join_start, 1)
elif new_join_start not in text:
    raise SystemExit("join_predecessor_homes marker not found")

old_join = '''fn join_home_state(left: HomeState, right: HomeState) -> HomeState {
    match (left, right) {
        (HomeState::Live, HomeState::Live) => HomeState::Live,
        (HomeState::Empty, HomeState::Empty) => HomeState::Empty,
        _ => HomeState::MaybeLive,
    }
}
'''
new_join = '''fn join_home_state(left: HomeState, right: HomeState) -> HomeState {
    match (left, right) {
        (HomeState::Unknown, state) | (state, HomeState::Unknown) => state,
        (HomeState::Live, HomeState::Live) => HomeState::Live,
        (HomeState::Empty, HomeState::Empty) => HomeState::Empty,
        _ => HomeState::MaybeLive,
    }
}
'''
if old_join in text:
    text = text.replace(old_join, new_join, 1)
elif new_join not in text:
    raise SystemExit("join_home_state marker not found")

path.write_text(text, encoding="utf-8")
print("cyclic Home dataflow fix staged")
