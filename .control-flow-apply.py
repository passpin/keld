from pathlib import Path

path = Path("crates/keld-semantics/src/check.rs")
text = path.read_text()
old = "        HirStmtKind::While(_) => false,\n"
if text.count(old) != 1:
    raise RuntimeError("expected exactly one redundant nested-loop false arm")
path.write_text(text.replace(old, "", 1))
print("removed redundant nested-loop match arm; wildcard preserves false semantics")
