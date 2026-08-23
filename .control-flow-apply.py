from pathlib import Path

path = Path("crates/keld-semantics/tests/control_flow_semantics.rs")
text = path.read_text()
if text.count('r#"') != 1 or text.count('"#,') != 1:
    raise RuntimeError("expected exactly one needlessly hashed raw string")
text = text.replace('r#"', 'r"', 1).replace('"#,', '",', 1)
path.write_text(text)
print("removed unnecessary hashes from the Control Flow-1 semantic fixture string")
