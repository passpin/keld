from pathlib import Path

# Keep the differential coverage test compact by lifting the fixed surface lists.
path = Path("crates/keld-native-backend/tests/differential.rs")
text = path.read_text()
start = text.index("#[test]\nfn every_executable_ir_variant_is_in_a_real_differential_fixture() {\n")
const_start = text.index("    const INSTRUCTIONS: &[&str] = &[\n", start)
body_start = text.index("    let mut instructions = BTreeSet::new();\n", const_start)
const_block = text[const_start:body_start]
module_consts = const_block.replace("    const INSTRUCTIONS", "const EXECUTABLE_INSTRUCTIONS", 1).replace("    const TERMINATORS", "const EXECUTABLE_TERMINATORS", 1)
module_consts = "\n".join(line[4:] if line.startswith("    ") else line for line in module_consts.splitlines()) + "\n\n"
text = text[:start] + module_consts + text[start:const_start] + text[body_start:]
text = text.replace("    for variant in INSTRUCTIONS {", "    for variant in EXECUTABLE_INSTRUCTIONS {", 1)
text = text.replace("    for variant in TERMINATORS {", "    for variant in EXECUTABLE_TERMINATORS {", 1)
path.write_text(text)

# A one-element proof check should be a direct assertion rather than a loop.
path = Path("crates/keld-cli/tests/milestone_acceptance.rs")
text = path.read_text()
old = '''    for proof in ["fn return_from_nested_lifecycle_has_an_explicit_exit_edge()"] {\n        assert!(flow.contains(proof), "missing Flow proof: {proof}");\n    }\n'''
new = '''    let flow_proof = "fn return_from_nested_lifecycle_has_an_explicit_exit_edge()";\n    assert!(\n        flow.contains(flow_proof),\n        "missing Flow proof: {flow_proof}"\n    );\n'''
if text.count(old) != 1:
    raise RuntimeError("expected one single-element Flow proof loop")
path.write_text(text.replace(old, new, 1))

print("lifted fixed IR surface constants and removed the single-element proof loop")
