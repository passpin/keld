from pathlib import Path
import re

expected = {
    "crates/keld-ir/tests/allocation_schedule.rs": 2,
    "crates/keld-ir/tests/entity_liveness_validation.rs": 3,
    "crates/keld-interpreter/tests/allocation_faults.rs": 1,
    # Task 7 compiles this helper next; keep its hand-built IR current too.
    "crates/keld-native-backend/tests/differential.rs": 1,
}
pattern = re.compile(r"(?m)^(\s*)parameters: ([^\n]+),\n\1parameter_modes:")
for filename, count in expected.items():
    path = Path(filename)
    text = path.read_text()
    if "locals:" in text and filename != "crates/keld-native-backend/tests/differential.rs":
        # Some files may contain unrelated updated literals; rely on exact replacement count below.
        pass
    replacement = r"\1parameters: \2,\n\1locals: Vec::new(),\n\1parameter_modes:"
    text, replaced = pattern.subn(replacement, text)
    if replaced != count:
        raise RuntimeError(f"{filename}: expected {count} Function parameter sites, found {replaced}")
    path.write_text(text)
print("annotated hand-built IR Functions with explicit empty local-slot tables")