from pathlib import Path

# README: implemented/deferred surface and normative links.
path = Path("README.md")
text = path.read_text()
old = '''- functions, blocks, conditionals, assignments, calls, checked unary/binary\n  arithmetic, boolean operators, scalar comparisons, and explicit runtime\n  faults;\n'''
new = '''- functions, blocks, conditionals, `while`, `break`, `continue`, assignments,\n  calls, checked unary/binary arithmetic, boolean operators, scalar comparisons,\n  and explicit runtime faults;\n'''
if text.count(old) != 1:
    raise RuntimeError("README implemented control-flow anchor changed")
text = text.replace(old, new, 1)
old = '''rejects imports/`use`, enums, externs/foreign declarations, generics, loops,\n`break`, `continue`, `match`, exceptions/typed errors, unsafe modules, and\nrecursion. Map, Set, Slice, iterators, Text integer indexing, and substrings\n'''
new = '''rejects imports/`use`, enums, externs/foreign declarations, generics, `match`,\nexceptions/typed errors, unsafe modules, and recursion. `for`, loop labels,\n`while let`, loop `else`, and break values remain deferred. Map, Set, Slice,\niterators, Text integer indexing, and substrings\n'''
if text.count(old) != 1:
    raise RuntimeError("README deferred control-flow anchor changed")
text = text.replace(old, new, 1)
old = '''and numeric rules are in [`docs/spec/numeric-safety.md`](docs/spec/numeric-safety.md).\n'''
new = '''and numeric rules are in [`docs/spec/numeric-safety.md`](docs/spec/numeric-safety.md).\nAccepted `while`, `break`, and `continue` semantics are normative in\n[`docs/spec/control-flow.md`](docs/spec/control-flow.md).\n'''
if text.count(old) != 1:
    raise RuntimeError("README normative links anchor changed")
text = text.replace(old, new, 1)
path.write_text(text)

# Grammar: point semantic loop rules to the normative supplement.
path = Path("docs/spec/grammar.md")
text = path.read_text()
old = '''Numeric operation behavior and faults are normative in\n[numeric-safety.md](numeric-safety.md). Single-home storage, loans, List, and\nText are normative in [storage-values.md](storage-values.md). Syntax acceptance\ndoes not override either document's static restrictions.\n'''
new = '''Numeric operation behavior and faults are normative in\n[numeric-safety.md](numeric-safety.md). Single-home storage, loans, List, and\nText are normative in [storage-values.md](storage-values.md). Accepted `while`,\n`break`, and `continue` semantics, including condition boundaries, backedges,\nand structured exits, are normative in [control-flow.md](control-flow.md). Syntax\nacceptance does not override those documents' static restrictions.\n'''
if text.count(old) != 1:
    raise RuntimeError("grammar normative tail changed")
text = text.replace(old, new, 1)
path.write_text(text)

# Storage values: existing Home joins explicitly include loop backedges/re-entry.
path = Path("docs/spec/storage-values.md")
text = path.read_text()
anchor = '''MaybeLive requires one hidden local drop flag for conditional replacement and\nscope cleanup. It is control-flow metadata, not a field in List or Text, and is\nremoved when static analysis proves a uniform state. Borrowed parameters have a\nseparate Loaned state and never receive cleanup; consuming parameters enter as\nLive homes.\n'''
addition = anchor + '''\nThe same Home lattice and joins apply at cyclic CFG backedges. A loop reaches a\nfixed point over the static homes in the function; analysis metadata does not\ngrow with dynamic iteration count. A lexical home whose loop-body scope is\nexited is cleaned before the backedge, `continue`, or `break` transfer and is in\nits scope-entry state when that static body scope is entered again. Outer homes\nretain only the converged state justified by every incoming loop edge. See\n[control-flow.md](control-flow.md) for the normative loop rules.\n'''
if text.count(anchor) != 1:
    raise RuntimeError("storage Home lattice anchor changed")
text = text.replace(anchor, addition, 1)
path.write_text(text)

# Compiler architecture: make cyclic fixed-point ownership explicit.
path = Path("docs/compiler-architecture.md")
text = path.read_text()
anchor = '''Uniform cleanup paths lower to direct reverse-order drops. A hidden per-scope\norder tracker is emitted only for a scope whose successful-initialization order\ndiverges at a CFG join; `MaybeLive` uses conditional drop metadata. Direct drops,\ntracked cleanup, normal scope exits, and returns therefore share one executable\ncleanup contract. `List[T]` and `Text` remain single-home, aggregate cleanup is\nrecursive and iterative, `List()` has no element buffer, and bounds/capacity\nchecks execute in every build.\n'''
addition = anchor + '''\nControl Flow-1 permits cyclic Flow CFGs. `keld-lifecycle` and `keld-storage` own\nmonotone fixed-point verification across backedges using their existing\nliveness, provenance, Home, loan, and cleanup domains. Flow owns structured\nloop targets and `ExitScopes`; executable IR receives only the converged result\nand represents loops as ordinary validated CFG cycles. Neither the interpreter\nnor the LLVM backend infers loop-specific lifecycle or storage policy.\n'''
if text.count(anchor) != 1:
    raise RuntimeError("compiler architecture cleanup anchor changed")
text = text.replace(anchor, addition, 1)
path.write_text(text)

# Approved baseline: add the normative control-flow document only; preserve the
# historical first-buildable-milestone exclusion list verbatim.
path = Path("docs/superpowers/specs/2026-08-11-keld-language-design.md")
text = path.read_text()
old = '''Two additional normative files define operations whose safety cannot be\nexpressed by grammar:\n\n- [`docs/spec/numeric-safety.md`](../../spec/numeric-safety.md) defines numeric\n  types, arithmetic, sizes, faults, and backend obligations.\n- [`docs/spec/storage-values.md`](../../spec/storage-values.md) defines\n  single-home values, `take`, loans, fields, List, and Text.\n'''
new = '''Three additional normative files define operations whose safety cannot be\nexpressed by grammar:\n\n- [`docs/spec/numeric-safety.md`](../../spec/numeric-safety.md) defines numeric\n  types, arithmetic, sizes, faults, and backend obligations.\n- [`docs/spec/storage-values.md`](../../spec/storage-values.md) defines\n  single-home values, `take`, loans, fields, List, and Text.\n- [`docs/spec/control-flow.md`](../../spec/control-flow.md) defines accepted\n  `while`, `break`, and `continue` semantics, cyclic joins, and structured exits.\n'''
if text.count(old) != 1:
    raise RuntimeError("language design normative list anchor changed")
text = text.replace(old, new, 1)
# Explicit guard against accidentally rewriting historical milestone scope.
historical = "- imports, enums, `var`, loops, `break`, `continue`, and `match`;"
if historical not in text:
    raise RuntimeError("historical first-buildable-milestone exclusion was changed")
path.write_text(text)

# Milestone evidence: add a distinct Control Flow-1 section without renumbering
# the historical 16 bootstrap criteria.
path = Path("docs/milestone-acceptance.md")
text = path.read_text()
marker = '''## Verification command for this historical milestone\n'''
section = r'''## Completed Control Flow-1 acceptance

Control Flow-1 implements the normative rules in
[`spec/control-flow.md`](spec/control-flow.md) without changing the historical
16 bootstrap criteria above. `while`, `break`, and `continue` lower through the
existing structured CFG and cleanup machinery; there is no implicit loop
lifecycle and no executable-IR or native loop opcode.

| Requirement | Executable evidence |
|---|---|
| ordinary structured loop result | `control_flow_loop.keld` returns `8` through the library, CLI interpreter, Native O0, and Native O2 paths |
| repeated managed allocation | `control_flow_allocations.keld` returns `6`; the repeated concat site keeps one site ID with attempts `1,2,3` |
| loop-control diagnostics | semantics/CLI acceptance rejects outside-loop `break` and `continue` with `KLD0112` |
| cyclic Home fixed point | `keld-storage/tests/control_flow.rs` covers `MaybeLive`, repair, zero iteration, and body-home re-entry |
| structured managed cleanup | `keld-interpreter/tests/cleanup.rs` covers body `Text` cleanup on `continue`, `List[Text]` cleanup on `break`, and condition-temporary cleanup |
| lifecycle exits | `keld-flow/tests/{control_flow,storage_scopes}.rs` proves `ExitScopes` for return and iteration lifecycle exits |
| no implicit lifecycle / ancestor keep | `keld-interpreter/tests/control_flow.rs` proves plain-loop entities remain in the enclosing lifecycle and kept entities survive an inner lifecycle `break` |
| lifecycle/provenance fixed point | `keld-lifecycle/tests/control_flow.rs` covers path retirement, loop-carried dynamic identity, and retirement call effects in conditions |
| condition boundary | interpreter/lifecycle regressions cover checked faults, call effects, and managed full-expression cleanup before body or loop exit |
| cyclic executable IR | `keld-ir` cyclic validation plus CLI milestone acceptance require a validated CFG backedge with no loop opcode |
| native parity and repeated condition allocation | `keld-native-backend/tests/differential.rs` compares interpreter with LLVM O0/O2, including one static condition-concat site with attempts `1,2,3` |

The Native-1 executable surface remains **48 instruction variants and 6
terminators**. Control Flow-1 expands which CFG shapes are accepted, not the
instruction/terminator enum surface. The `surface_audit` suite remains the
exhaustive executable-surface guard.

'''
if text.count(marker) != 1:
    raise RuntimeError("milestone historical verification marker changed")
if "## Completed Control Flow-1 acceptance" in text:
    raise RuntimeError("Control Flow-1 milestone section already present")
text = text.replace(marker, section + marker, 1)
path.write_text(text)

print("updated Control Flow-1 README, normative cross-references, architecture, and acceptance docs")