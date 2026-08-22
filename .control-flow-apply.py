from pathlib import Path

path = Path("crates/keld-cli/tests/cli.rs")
text = path.read_text()

old_list = '''        ("broad_retirement.keld", "40\\n"),\n        ("numeric_edges.keld", "0\\n"),\n'''
new_list = '''        ("broad_retirement.keld", "40\\n"),\n        ("numeric_edges.keld", "0\\n"),\n        ("control_flow_loop.keld", "8\\n"),\n        ("control_flow_allocations.keld", "6\\n"),\n'''
count = text.count(old_list)
if count != 2:
    raise RuntimeError(f"expected interpreter/native representative lists twice, found {count}")
text = text.replace(old_list, new_list)

old_unsupported = '''        (\n            "fail_unsupported.keld",\n            1,\n            19,\n            "KLD0004",\n            "`while` is parsed but not supported by the bootstrap compiler",\n            "remove this feature or use the currently supported bootstrap subset",\n        ),\n'''
new_unsupported = '''        (\n            "fail_unsupported.keld",\n            2,\n            5,\n            "KLD0004",\n            "`match` is parsed but not supported by the bootstrap compiler",\n            "remove this feature or use the currently supported bootstrap subset",\n        ),\n'''
if text.count(old_unsupported) != 1:
    raise RuntimeError("obsolete unsupported-loop CLI expectation not found exactly once")
text = text.replace(old_unsupported, new_unsupported, 1)

path.write_text(text)
print("CLI acceptance now requires loop fixtures and match as the deferred feature")