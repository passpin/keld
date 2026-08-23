from pathlib import Path

path = Path("crates/keld-storage/tests/cleanup_planner.rs")
text = path.read_text()
old = "HomeId::Local(LocalId(0) | LocalId(1))"
new = "HomeId::Local(LocalId(0 | 1))"
if text.count(old) != 1:
    raise RuntimeError("expected exactly one unnested LocalId OR-pattern")
path.write_text(text.replace(old, new, 1))
print("nested the LocalId OR-pattern for Rust 1.97 Clippy")
