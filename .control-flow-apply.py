from pathlib import Path

path = Path("crates/keld-lifecycle/tests/provenance_facts.rs")
text = path.read_text()
old = '        "entity Item { value: Int }\\nfn inspect(left: Item, right: Item, choose_right: Bool) -> Bool { var carried = left; var i = 0; while i < 1 { if choose_right { carried = right }; i = i + 1 }; return carried == left }\\nfn main() -> Int { return 0 }\\n",\n'
new = '        "entity Item { value: Int }\\nfn inspect(left: Item, right: Item, choose_right: Bool) -> Int { var carried = left; var i = 0; while i < 1 { if choose_right { carried = right }; i = i + 1 }; if carried == left { return 1 } else { return 0 } }\\nfn main() -> Int { return 0 }\\n",\n'
if text.count(old) != 1:
    raise RuntimeError("nonfresh loop fixture marker is not unique")
path.write_text(text.replace(old, new, 1))
print("fixed nonfresh identity fixture to use BranchIdentity")
