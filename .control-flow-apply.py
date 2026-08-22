from pathlib import Path

path = Path("crates/keld-native-backend/tests/differential.rs")
text = path.read_text()

# The Task 6 IR metadata addition must already be reflected in this hand-built helper.
needle = "        parameters: Vec::new(),\n        locals: Vec::new(),\n        parameter_modes: Vec::new(),\n"
if text.count(needle) != 1:
    raise RuntimeError("native differential ir_function local metadata is missing or duplicated")

# Add both Control Flow-1 fixtures to the established source-fixture differential corpus.
old = '''        "runtime_div_zero.keld",\n        "runtime_capacity.keld",\n    ] {\n'''
new = '''        "runtime_div_zero.keld",\n        "runtime_capacity.keld",\n        "control_flow_loop.keld",\n        "control_flow_allocations.keld",\n    ] {\n'''
# This fixture list appears both in the main differential corpus and the executable-surface audit.
count = text.count(old)
if count != 2:
    raise RuntimeError(f"expected two source fixture lists, found {count}")
text = text.replace(old, new)

# Extend the source-surface differential schedule with the same external fixtures.
old = '''    &[\n        (\n            "text_surface",\n'''
new = '''    &[\n        (\n            "control_flow_loop_surface",\n            include_str!("../../keld-cli/tests/fixtures/control_flow_loop.keld"),\n        ),\n        (\n            "control_flow_allocations_surface",\n            include_str!("../../keld-cli/tests/fixtures/control_flow_allocations.keld"),\n        ),\n        (\n            "text_surface",\n'''
if text.count(old) != 1:
    raise RuntimeError("source_surface_cases insertion point changed")
text = text.replace(old, new, 1)

# Freeze the key repeated-static-site contract before running native parity.
marker = '''#[test]\nfn every_source_fixture_has_a_shared_allocation_failure_schedule_at_o0_and_o2() {\n'''
if text.count(marker) != 1:
    raise RuntimeError("differential test insertion marker changed")
focused = r'''#[test]
fn repeated_loop_concat_allocation_uses_one_static_site_with_three_attempts() {
    let (_, source) = fixture("control_flow_allocations.keld");
    let (module, _) = compile_fixture(&source);
    let (result, events) = interpreter_run(&module, &[]);
    assert_eq!(result, Observation::Returned(6));

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
text = text.replace(marker, focused + marker, 1)

path.write_text(text)
print("added Control Flow-1 fixtures and repeated concat-site differential contract")