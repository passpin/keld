from pathlib import Path

# Interpreter lifecycle/control-flow runtime regressions.
path = Path("crates/keld-interpreter/tests/control_flow.rs")
text = path.read_text()
old = "use keld_interpreter::{ValueKind, run_text_for_test};\n"
new = "use keld_interpreter::{RuntimeFaultKind, ValueKind, run_text_for_test};\n"
if text.count(old) != 1:
    raise RuntimeError("interpreter control_flow import changed")
text = text.replace(old, new, 1)
append = r'''

#[test]
fn plain_loop_does_not_create_an_implicit_lifecycle() {
    assert_eq!(
        run_int(
            "entity Item { value: Int }\nentity Anchor { target: link Item? }\nfn main() -> Int { lifecycle game { let anchor = Anchor(target: none); var i = 0; while i < 1 { let item = Item(value: 7); anchor.target = item; i += 1 }; when anchor.target as live { return live.value } else { return 0 } } }\n",
        ),
        7
    );
}

#[test]
fn keep_to_ancestor_survives_break_out_of_inner_lifecycle() {
    assert_eq!(
        run_int(
            "entity Item { value: Int }\nentity Anchor { target: link Item? }\nfn main() -> Int { lifecycle game { let anchor = Anchor(target: none); var i = 0; while i < 1 { lifecycle frame { let item = Item(value: 9); anchor.target = item; keep item in game; break } }; when anchor.target as live { return live.value } else { return 0 } } }\n",
        ),
        9
    );
}

#[test]
fn checked_fault_in_condition_occurs_before_body() {
    let fault = run_text_for_test(
        "fn zero() -> Int { return 0 }\nfn main() -> Int { while 1 / zero() == 0 { return 99 }; return 0 }\n",
    )
    .unwrap_err();
    assert_eq!(fault.kind, RuntimeFaultKind::DivisionByZero);
}
'''
if "fn plain_loop_does_not_create_an_implicit_lifecycle()" in text:
    raise RuntimeError("interpreter self-review tests already present")
path.write_text(text + append)

# Runtime cleanup evidence for iteration exits and condition temporaries.
path = Path("crates/keld-interpreter/tests/cleanup.rs")
text = path.read_text()
append = r'''

#[test]
fn continue_drops_body_local_text_on_every_iteration() {
    let trace = trace_text_for_test(
        "fn main() -> Int { var i = 0; while i < 2 { let text: Text = \"x\"; i += 1; continue }; return i }\n",
    )
    .expect("continue cleanup executes");
    assert_eq!(trace.result.value, Value::Int(2));
    assert_eq!(trace.text_markers(), vec![(1, b'x'), (1, b'x')]);
}

#[test]
fn break_drops_body_local_list_before_loop_exit() {
    let trace = trace_text_for_test(
        "fn main() -> Int { while true { let values: List[Text] = List(); values.push(\"b\"); break }; return 0 }\n",
    )
    .expect("break cleanup executes");
    assert_eq!(trace.result.value, Value::Int(0));
    assert_eq!(trace.list_indices(), vec![0]);
    assert_eq!(trace.text_markers(), vec![(1, b'b')]);
}

#[test]
fn managed_condition_temporary_is_cleaned_at_the_condition_boundary() {
    let trace = trace_text_for_test(
        "fn main() -> Int { while (\"abcdefghijklmnopqrstuvwxyz\" + \"!\").is_empty { return 1 }; return 0 }\n",
    )
    .expect("managed condition executes");
    assert_eq!(trace.result.value, Value::Int(0));
    assert_eq!(
        trace
            .text_markers()
            .into_iter()
            .filter(|(length, first)| *length == 27 && *first == b'a')
            .count(),
        1,
        "the concatenation temporary must be destroyed exactly once"
    );
}
'''
if "fn continue_drops_body_local_text_on_every_iteration()" in text:
    raise RuntimeError("cleanup self-review tests already present")
path.write_text(text + append)

# Lifecycle fixed-point must apply condition call effects before the zero-iteration exit.
path = Path("crates/keld-lifecycle/tests/control_flow.rs")
text = path.read_text()
append = r'''

#[test]
fn retirement_call_effect_in_condition_invalidates_post_loop_use() {
    let result = verify_text_for_test(
        "entity E { value: Int }\nfn retire_and_false(e: E) -> Bool retires e { retire e; return false }\nfn inspect(e: E) -> Int { while retire_and_false(e) { }; return e.value }\nfn main() -> Int { return 0 }\n",
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD1001"),
        "{:#?}",
        result.diagnostics
    );
}
'''
if "fn retirement_call_effect_in_condition_invalidates_post_loop_use()" in text:
    raise RuntimeError("lifecycle condition-effect regression already present")
path.write_text(text + append)

# Add a condition-allocation case to the established O0/O2 differential corpus.
path = Path("crates/keld-native-backend/tests/differential.rs")
text = path.read_text()
anchor = '        (\n            "text_surface",\n'
condition_case = r'''        (
            "control_flow_condition_allocation_surface",
            "fn main() -> Int { var i = 0; while !(\"abcdefghijklmnopqrstuvwxyz\" + \"!\").is_empty && i < 2 { i += 1 }; return i }\n",
        ),
'''
if text.count(anchor) != 1:
    raise RuntimeError("text_surface label anchor changed")
text = text.replace(anchor, condition_case + anchor, 1)

marker = '''#[test]\nfn every_source_fixture_has_a_shared_allocation_failure_schedule_at_o0_and_o2() {\n'''
focused = r'''#[test]
fn repeated_condition_concat_uses_one_static_site_with_three_attempts() {
    let (_, source) = source_surface_cases()
        .iter()
        .find(|(label, _)| *label == "control_flow_condition_allocation_surface")
        .expect("condition allocation fixture");
    let (module, _) = compile_fixture(source);
    let (result, events) = interpreter_run(&module, &[]);
    assert_eq!(result, Observation::Returned(2));

    let concat = events
        .iter()
        .filter(|event| event.phase == AllocationPhase::Concat)
        .collect::<Vec<_>>();
    assert_eq!(concat.len(), 3, "{events:#?}");
    let site_id = concat[0].site_id;
    assert!(concat.iter().all(|event| event.site_id == site_id), "{concat:#?}");
    assert_eq!(
        concat.iter().map(|event| event.attempt).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

'''
if text.count(marker) != 1:
    raise RuntimeError("differential focused-test insertion point changed")
if "fn repeated_condition_concat_uses_one_static_site_with_three_attempts()" in text:
    raise RuntimeError("condition allocation focused test already present")
text = text.replace(marker, focused + marker, 1)
path.write_text(text)

print("added lifecycle, cleanup, condition, and repeated-allocation self-review regressions")