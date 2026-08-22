from pathlib import Path

verify_path = Path("crates/keld-lifecycle/src/verify.rs")
text = verify_path.read_text()
start_marker = "    fn run(&mut self) {\n"
end_marker = "    fn initial_state(&self) -> AbstractState {\n"
if text.count(start_marker) != 1 or text.count(end_marker) != 1:
    raise RuntimeError("Analyzer run markers are not unique")
start = text.index(start_marker)
end = text.index(end_marker, start)
replacement = r'''    fn run(&mut self) {
        let replay_diagnostics = self.diagnose;
        self.diagnose = false;
        let incoming = self.solve_fixed_point();
        let solved_inference = self.inference.clone();

        self.reachable_blocks = incoming
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                state.as_ref().map(|_| {
                    BlockId(u32::try_from(index).expect("flow block index fits in u32"))
                })
            })
            .collect();

        if replay_diagnostics {
            self.diagnose = true;
            self.replay_converged_states(&incoming);
            self.inference = solved_inference;
        }
    }

    fn solve_fixed_point(&mut self) -> Vec<Option<AbstractState>> {
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
                        let joined = AbstractState::join(
                            &[existing.clone(), successor_state],
                            self.function.span,
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

    fn replay_converged_states(&mut self, incoming: &[Option<AbstractState>]) {
        self.sink = DiagnosticSink::default();
        self.proofs.clear();
        self.entity_facts.clear();
        self.next_proof = 0;

        for (block_index, incoming_state) in incoming.iter().enumerate() {
            let Some(mut state) = incoming_state.clone() else {
                continue;
            };
            let block_id =
                BlockId(u32::try_from(block_index).expect("flow block index fits in u32"));
            let block = &self.function.blocks[block_index];
            for (index, operation) in block.operations.iter().enumerate() {
                let index = u32::try_from(index).expect("flow operation index fits in u32");
                let previous = self
                    .entity_facts
                    .insert((block_id, index), EntityOperationFacts::project(&state));
                debug_assert!(previous.is_none(), "operation facts are recorded once");
                self.operation(block_id, index, operation, &mut state);
            }
            let _ = self.terminator(block_id, &block.terminator, state);
        }
    }

'''
text = text[:start] + replacement + text[end:]

reachable_marker = "fn reachable_blocks(function: &FlowFunction) -> BTreeSet<BlockId> {\n"
successor_end_marker = "fn is_ancestor(function: &FlowFunction, ancestor: LifecycleId, descendant: LifecycleId) -> bool {\n"
if text.count(reachable_marker) != 1 or text.count(successor_end_marker) != 1:
    raise RuntimeError("obsolete graph helper markers are not unique")
helper_start = text.index(reachable_marker)
helper_end = text.index(successor_end_marker, helper_start)
text = text[:helper_start] + text[helper_end:]
verify_path.write_text(text)
print("replaced DAG lifecycle analysis with solve/replay fixed point")

provenance_path = Path("crates/keld-lifecycle/src/provenance.rs")
text = provenance_path.read_text()
old = "#[derive(Clone, Debug)]\npub(crate) struct AbstractState {\n"
new = "#[derive(Clone, Debug, Eq, PartialEq)]\npub(crate) struct AbstractState {\n"
if text.count(old) != 1:
    raise RuntimeError("AbstractState derive marker is not unique")
provenance_path.write_text(text.replace(old, new, 1))
print("made abstract states comparable for fixed-point convergence")
