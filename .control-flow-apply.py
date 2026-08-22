from pathlib import Path

path = Path("crates/keld-semantics/tests/control_flow_semantics.rs")
text = path.read_text()
old = '    let analysis = analyze_text("fn main() -> Int { while 1 { break } return 0 }\\n");\n'
new = '    let analysis = analyze_text("fn main() -> Int { while 1 { break }; return 0 }\\n");\n'
count = text.count(old)
if count != 1:
    raise RuntimeError(f"expected one test match, found {count}")
path.write_text(text.replace(old, new, 1))
print("fixed while-condition test terminator")
