from pathlib import Path

path = Path("crates/keld-lifecycle/src/verify.rs")
text = path.read_text()
old = r'''    fn solve_fixed_point(&mut self) -> Vec<Option<AbstractState>> {
        let block_count = self.function.blocks.len();
        let mut incoming = vec![None; block_count];
        incoming[self.function.entry.0 as usize] = Some(self.initial_state());

        let mut queue = VecDeque::from([self.function.entry]);
        let mut queued = vec![false; block_count];
        queued[self.function.entry.0 as usize] = true;

        while let Some(block_id) = queue.pop_front() {
            let block_index = block_id.0 as usize;
            queued[block_index] = false;
            let Some(mut state) = incoming[block_index].clone() else {
                continue;
            };
            let block = &self.function.blocks[block_index];
            self.clear_block_entity_values(&block.operations, &mut state);
            for (index, operation) in block.operations.iter().enumerate() {
                let index = u32::try_from(index).expect("flow operation index fits in u32");
                self.operation(block_id, index, operation, &mut state);
            }

            for (successor, successor_state) in
                self.terminator(block_id, &block.terminator, state)
            {
                let successor_index = successor.0 as usize;
                let next_state = match incoming[successor_index].as_ref() {
                    None => Some(successor_state),
                    Some(existing) => {
                        let joined = self.join_at(
                            successor,
                            &[existing.clone(), successor_state],
                        );
                        (joined != *existing).then_some(joined)
                    }
                };
                let Some(next_state) = next_state else {
                    continue;
                };
                incoming[successor_index] = Some(next_state);
                if !queued[successor_index] {
                    queue.push_back(successor);
                    queued[successor_index] = true;
                }
            }
        }

        incoming
    }
'''
new = r'''    fn solve_fixed_point(&mut self) -> Vec<Option<AbstractState>> {
        let block_count = self.function.blocks.len();
        let mut incoming = vec![None; block_count];
        incoming[self.function.entry.0 as usize] = Some(self.initial_state());

        let mut predecessors = vec![Vec::<BlockId>::new(); block_count];
        for block in &self.function.blocks {
            for successor in successors(&block.terminator) {
                predecessors[successor.0 as usize].push(block.id);
            }
        }
        let mut edge_states = BTreeMap::<(BlockId, BlockId), AbstractState>::new();

        let mut queue = VecDeque::from([self.function.entry]);
        let mut queued = vec![false; block_count];
        queued[self.function.entry.0 as usize] = true;

        while let Some(block_id) = queue.pop_front() {
            let block_index = block_id.0 as usize;
            queued[block_index] = false;
            let Some(mut state) = incoming[block_index].clone() else {
                continue;
            };
            let block = &self.function.blocks[block_index];
            self.clear_block_entity_values(&block.operations, &mut state);
            for (index, operation) in block.operations.iter().enumerate() {
                let index = u32::try_from(index).expect("flow operation index fits in u32");
                self.operation(block_id, index, operation, &mut state);
            }

            let mut produced = BTreeMap::<BlockId, Vec<AbstractState>>::new();
            for (successor, successor_state) in
                self.terminator(block_id, &block.terminator, state)
            {
                produced
                    .entry(successor)
                    .or_default()
                    .push(successor_state);
            }

            for successor in successors(&block.terminator) {
                let key = (block_id, successor);
                let next_edge = produced.remove(&successor).map(|states| {
                    if states.len() == 1 {
                        states.into_iter().next().expect("one edge state")
                    } else {
                        self.join_at(successor, &states)
                    }
                });
                let edge_changed = match next_edge {
                    Some(next_edge) => {
                        if edge_states.get(&key) == Some(&next_edge) {
                            false
                        } else {
                            edge_states.insert(key, next_edge);
                            true
                        }
                    }
                    None => edge_states.remove(&key).is_some(),
                };
                if !edge_changed {
                    continue;
                }

                let successor_index = successor.0 as usize;
                let mut states = predecessors[successor_index]
                    .iter()
                    .filter_map(|predecessor| {
                        edge_states.get(&(*predecessor, successor)).cloned()
                    })
                    .collect::<Vec<_>>();
                if successor == self.function.entry {
                    states.push(self.initial_state());
                }
                let next_incoming = match states.len() {
                    0 => None,
                    1 => states.into_iter().next(),
                    _ => Some(self.join_at(successor, &states)),
                };
                if incoming[successor_index] == next_incoming {
                    continue;
                }
                incoming[successor_index] = next_incoming;
                if !queued[successor_index] {
                    queue.push_back(successor);
                    queued[successor_index] = true;
                }
            }
        }

        incoming
    }
'''
count = text.count(old)
if count != 1:
    raise RuntimeError(f"expected one solver body, found {count}")
path.write_text(text.replace(old, new, 1))
print("recomputed successor inputs from current CFG edge states")
