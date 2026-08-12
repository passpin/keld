# Keld Storage Values and List Semantics

Status: normative Keld 0.1 storage model

This document defines single-home managed storage, explicit transfer, function
loans, fields, `List[T]`, and `Text`. It does not define entity lifetime; entity
identity and lifecycle rules remain in the language design.

## 1. Value Storage Classes

The compiler classifies every value type as one of these storage classes:

| Class | Binding behavior | Examples |
|---|---|---|
| implicit-copy | assignment copies | integers, `Bool`, links |
| single-home | assignment requires a choice | `List[T]`, `Text` |
| entity-flow | assignment aliases proven identity | `EntityRef[T]`, scoped `EntityRef[T]?` |
| external resource | deferred in Keld 0.1 | file or socket handles |

The classification composes structurally:

- A struct, enum payload, or optional value is single-home
  when any contained value is single-home.
- `List[T]` is single-home regardless of `T`.
- `Text` is single-home.
- An aggregate is implicitly copyable only when every component is implicitly
  copyable.
- Direct entity references cannot be stored in structs, enum payloads, Lists,
  globals, or entity fields. Persistent relationships use
  links. Compiler-bounded temporary aggregates of direct references are a
  separate entity-flow class and never become owned storage values. Optional
  direct references are permitted only in compiler-bounded local flow.

Single-home means that exactly one live place is responsible for destroying the
managed storage. It does not expose an allocator, pointer, or ownership type in
source.

## 2. The Ambiguous Assignment Rule

Using a named single-home place in an owned-value context without an explicit
operation is a compile-time error:

```keld
let a: List[Int] = List()
let b = a
```

Keld does not guess whether this means alias, deep copy, or transfer.

The programmer chooses:

```keld
let moved = take a
let copied = moved.copy()
```

There is no implicit alias because two homes cannot own the same storage. There
is no implicit deep copy because it can be O(n) and allocate. There is no
implicit move because it would make an apparently ordinary read invalidate its
source.

The rule applies to initialization, assignment, arguments, returns, aggregate
construction, and field initialization whenever an owned value is required.

## 3. Places, Owned Values, and Loans

A place is a named storage location such as a local, parameter, field, or indexed
element. A single-home expression is used in one of three contexts:

| Context | Effect |
|---|---|
| loan | storage remains in its place for the operation |
| consume | an owned value is required |
| copy | an explicit independent value is requested |

Constructors, concatenation, consuming function results, and `take` produce
owned temporary values. An owned temporary transfers automatically into the next
consuming context because it has no separately usable source place.

When an owned temporary is supplied to a normal loan parameter, the temporary
remains its own hidden home for the call and is destroyed after the loan closes.
It also receives cleanup on a typed-error exit from argument evaluation or the
call.

Normal function arguments and receiver accesses are loan contexts unless the
parameter is declared `take`. Initialization and return are consuming contexts.

## 4. Explicit Transfer with `take`

`take place` transfers a named single-home local or owned `take` parameter and
produces an owned temporary value. It never copies a managed heap payload.
Transfer is O(1) for List and Text descriptors; an inline aggregate costs its
fixed inline size unless the backend elides the physical copy.

```keld
let source: List[Int] = List()
let destination = take source
```

After transfer, `source` is `Moved`:

- reading, borrowing, copying, or taking it again is a compile-time error;
- scope cleanup does not destroy its former storage;
- a moved `var` may be assigned a fresh or transferred value; and
- a moved `let` cannot be reinitialized.

At a control-flow join, a place is usable only when it is initialized on every
incoming path. Moving on one branch therefore makes the place unavailable after
the join unless each incoming path that moves it also reinitializes it before
the join. A branch that keeps the original value is already live.

The compiler implements local-home flow with this lattice:

```text
Empty(reason)   no value; reason is Uninitialized or Moved
Live            exactly one value is installed
MaybeLive       incoming paths disagree between Empty and Live
```

Joining only Live inputs yields Live. Joining only Empty inputs yields Empty;
the compiler retains all relevant reasons for diagnostics. Any Empty/Live mix
yields MaybeLive. Reads, loans, copies, and `take` require Live. Assignment to a
`var` is allowed in any of the three states: it evaluates the replacement first,
conditionally destroys an old value, installs the replacement, and yields Live.
Assignment to an initialized `let` remains forbidden.

MaybeLive requires one hidden local drop flag for conditional replacement and
scope cleanup. It is control-flow metadata, not a field in List or Text, and is
removed when static analysis proves a uniform state. Borrowed parameters have a
separate Loaned state and never receive cleanup; consuming parameters enter as
Live homes.

`take` is rejected when applied to:

- an implicitly copyable value;
- an entity-flow value or link;
- an owned temporary, where transfer is already automatic;
- a borrowed parameter;
- a field or indexed element; or
- a place with an active loan.

Self-transfer such as `value = take value` is rejected.

## 5. Explicit Structural Copy

`value.copy()` produces an independent structural copy and leaves `value` live.
The operation is explicit because it may allocate and take O(n) time.

A type is structurally duplicable when the compiler can copy every component
without invoking user code:

- implicit-copy values are duplicable;
- `Text` is duplicable;
- `List[T]` is duplicable when `T` is duplicable;
- structs, enums, and optional values are duplicable when every
  component is duplicable;
- links copy their identities and do not duplicate target entities; and
- external resources are not duplicable.

Keld 0.1 has no user-overridable copy hook. Copying nested Lists and Text values
recursively duplicates their logical values. The backend may share immutable
immortal literal bytes when that sharing is unobservable.

A generic body may call `.copy()` only when structural duplicability is provable
from the declared type. An unconstrained type parameter does not provide that
proof; storage-class constraint syntax is deferred.

An allocation failure during ordinary `copy()` produces `AllocationFault`.

### 5.1 Managed Cleanup Order

Managed cleanup cannot call user or foreign code and cannot produce a typed
error. On normal scope exit, return, or typed-error propagation, live local homes
are destroyed in reverse successful-initialization order. MaybeLive homes use
their hidden flag. Moved and uninitialized homes are skipped.

Struct fields and enum payload components are destroyed in reverse declaration
order. An optional destroys its contained value only when present. An entity is
first made unresolvable by the Custody Ledger retirement algorithm, then its
managed fields are destroyed in reverse declaration order after all ordinary
entity `View` values are closed.

These orders are deterministic even though initial Keld cleanup cannot execute
user-observable destructors. They provide one stable rule for tracing, allocator
tests, and future restricted resources.

## 6. Binding Mutability

`let` prevents rebinding but does not freeze the contents of managed storage:

```keld
let numbers: List[Int] = List()
numbers.push(1)
```

`var` permits replacing the entire value and permits reinitialization after
`take`:

```keld
var numbers: List[Int] = List()
let old = take numbers
numbers = List()
```

Both `let` and `var` values may be loaned to functions that mutate mutable
contents. Rebinding and content mutation are separate operations. `Text` remains
immutable regardless of binding kind.

For a local struct, replacing a field is rebinding part of the aggregate and
therefore requires a `var` base. Mutating a List stored in a `let` struct is
content mutation and remains allowed. Entity payload fields are mutable through
a live entity reference regardless of whether that reference's local binding is
`let` or `var`; changing the reference binding is a separate operation.

Replacing a live `var` evaluates the new owned value first, installs it, and
then destroys the displaced value. If evaluation propagates a typed error, the
old value remains installed. An uninitialized or moved `var` has no displaced
value to destroy.

## 7. Function Parameters

A normal single-home parameter is a call-scoped loan:

```keld
fn inspect(items: List[Int]) {
    show(items.length)
}
```

The callee may read or mutate the loaned value according to its inferred effect,
but it cannot:

- transfer it with `take`;
- store it in an owned field or global;
- return it as an owned value; or
- let an element access escape the call.

A consuming parameter declares `take` before its name:

```keld
fn consume(take items: List[Int]) {
    record_count(items.length)
}
```

On function entry, a consuming parameter is a new callee-owned home. The callee
destroys it on every normal or typed-error exit unless it has transferred the
value onward.

Calling rules are:

```keld
consume(take items)
consume(make_items())
consume(items)
```

The first two calls are valid. The third is a compile-time error because a named
single-home place was supplied to a consuming parameter without `take`.

The declared parameter type must be statically single-home independent of any
unconstrained type arguments. `take items: List[T]` qualifies because List is
always single-home; `take value: T` does not. Implicit-copy parameters use normal
copying parameters. Storage-class-polymorphic parameters are deferred.

## 8. Loan Effects and Call Aliasing

The compiler infers one summary for each storage parameter. A normal parameter
has the strongest of `reads`, `edits`, or `structural` needed by its body; a
declared consuming parameter has `takes`:

| Effect | Meaning |
|---|---|
| `reads(place)` | observes storage without mutation |
| `edits(place)` | replaces existing elements without changing structure |
| `structural(place)` | may change length, capacity, layout, or initialization |
| `takes(place)` | receives ownership through a declared `take` parameter |

Public module metadata contains these effects. They do not require ordinary
source annotations.

At each call, the compiler checks all arguments as one set rather than one at a
time:

- any number of read loans may overlap;
- an edit loan cannot overlap another read, edit, structural, or take access;
- a structural loan cannot overlap any other access;
- a take cannot overlap any loan or another take; and
- a place and any of its subplaces overlap.

Different fixed fields of one aggregate are disjoint. Two List element places
are disjoint only when their indices are proven unequal and neither access is
structural on the containing List; otherwise they may overlap. A structural List
access overlaps every element place regardless of its index.

Argument expressions evaluate left to right. After a place argument is
evaluated, its final access becomes a pending reservation. A reservation records
place identity, provenance, and effect, not a raw address. It constrains later
argument evaluation by the same overlap matrix, so a later argument cannot change
what an earlier element or field loan denotes. After all resulting places are
confirmed live, the complete set is checked, reservations become active loans
for the call, and those loans close when it returns or propagates a typed error.

A reached `take` is not rolled back if evaluation of a later argument propagates
a typed error. Its owned temporary is cleaned on that exit edge and the source
place remains moved. This follows ordinary left-to-right evaluation and avoids a
hidden conditional transfer.

This call is rejected when `change` edits or structurally mutates its first
parameter:

```keld
change(items, items)
```

Passing the same List to two read-only parameters is valid.

For entity fields, place overlap uses entity provenance. Fields with the same
path on entity references that may alias are treated as possibly overlapping.
An identity comparison may establish distinct entity bases in one branch.

An entity-field loan is represented semantically by the entity identity and
field path. It never holds an entity payload address across a call. Each callee
access opens and closes the short hidden entity `View` required by the Custody
Ledger model. Retiring a possibly equal entity, or a broad retirement effect
that may include it, conflicts with the field loan.

The callee may assume the call-site loan contract was checked. Indirect dispatch,
when added, must use the union of possible target effects.

## 9. Returns

A function return type `List[T]` or `Text` denotes an owned result.

Fresh owned temporaries return directly:

```keld
fn make_items() -> List[Int] {
    return List()
}
```

A named owned local requires `take`:

```keld
fn forward(take items: List[Int]) -> List[Int] {
    return take items
}
```

`return items` is an error for a named single-home place. A loaned parameter
cannot be returned as owned storage, even with `take`. A function that needs an
independent result calls `.copy()` explicitly.

The caller receives a fresh owned temporary. It transfers automatically into a
binding, consuming argument, return, or field assignment.

## 10. Composites and Fields

A local aggregate containing List or Text is single-home and moves only as a
whole. Keld 0.1 rejects partial moves:

```keld
let names = take team.names
```

This is rejected whether `team` is a struct local or an entity reference. It
would leave a partially initialized aggregate that other accesses could observe.

Managed List and Text storage may appear in entity fields. They are not external
resources: their cleanup is compiler-defined, non-throwing, and does not call
user code.

Construction and field assignment are consuming contexts. In this example,
`Team` is an entity type:

```keld
let team = Team(names: take names)
team.names = make_names()
team.names = names.copy()
team.names = names
```

The first three initializations or assignments are valid when their sources are
live. The final assignment is an error.

Field assignment evaluates the replacement completely before changing the
field. If evaluation propagates a typed error, the old field remains unchanged.
On success, the operation installs the owned replacement and extracts the old
value in one non-failing commit, closes any entity `View`, and then destroys the
displaced value. User code cannot observe a missing or partially replaced field.

Keld 0.1 provides no operation that extracts an owned field while leaving a
replacement atomically. Such a `replace` primitive is deferred until its alias
and error behavior is specified.

## 11. List Representation and Cleanup

`List[T]` is a contiguous growable sequence with one logical home. Its conceptual
representation contains a buffer, length, and capacity. Representation details
are not observable in safe source.

`List()` constructs an empty List without allocating an element buffer. Its
element type must be inferred from the expected type. The first operation that
requires positive room performs checked growth.

The List owns every initialized element from index zero through `length - 1`.
Dropping a List destroys those elements in reverse index order and releases its
buffer. Cleanup is compiler-generated and cannot call user code in Keld 0.1.

`length` has type `Int`. Capacity is not queryable by safe source. Both follow
the checked size rules in `numeric-safety.md`; implementations may choose growth
strategies subject to the specified minimum-capacity retry rule.

## 12. List Element Access

`list[index]` checks `0 <= index < list.length` in every build. Failure produces
`BoundsFault`. The compiler may eliminate the check only when it proves the
condition.

The result depends on the element storage class and use context:

- An implicitly copyable element copies in an owned-value context.
- A single-home element may be used only through a non-escaping loan bounded to
  the immediate built-in access or function call.
- Assignment to `list[index]` replaces the element after evaluating the new
  owned value.
- `take list[index]` is rejected because it would leave a hole.

Examples:

```keld
let count = counts[index]
show(texts[index])
let text = texts[index]
```

The first two uses are valid when `counts` contains `Int` and `texts` contains
`Text`. The final binding is an error because it requests an owned Text without
copying or structurally removing it.

For indexed assignment, the compiler records the base and evaluated index, then
evaluates the replacement. The replacement may not move the base or structurally
mutate the same List; this prevents the saved index from silently selecting a
different element. Bounds are checked against the current length at the final
commit.

`List.get(index)` exists only when `T` is implicitly copyable. It returns
`Option[T]` and never faults for an invalid index. `Option[T]` is the nominal
form of `T?`; explicit nesting such as `Option[T?]` preserves the difference
between an invalid index and a present optional element.

## 13. List Structural Operations

Required core operations are:

```text
List()
length
push(value)
remove(index)
try_remove(index)
clear()
reserve(additional)
try_reserve(additional)
copy()
```

`push` consumes its element argument. A named single-home argument uses `take`;
a temporary transfers automatically; an implicitly copyable argument copies.

`remove(index)` checks bounds, closes the gap, decreases length, and returns the
removed element as an owned temporary. `try_remove(index)` returns `Option[T]`
and leaves the List unchanged for an invalid index.

`remove`, successful `try_remove`, and `clear` do not allocate or reduce the
List's internal capacity. `clear` destroys all elements in reverse index order.
Reserved room therefore remains with the List value across removal, clearing,
and `take`; replacement or destruction ends that guarantee.

`clear`, `push`, `remove`, `try_remove`, `reserve`, and `try_reserve` are
structural operations. They require no overlapping element or List loan. A
reserve call remains structurally classified unless the compiler proves at that
call site that no layout change can occur.

A negative `additional` value is invalid. `reserve` produces `CapacityFault`;
`try_reserve` returns `false` and leaves the List unchanged. Both operations
request room for `length + additional` elements and follow the addressability
and growth-overflow rules in `numeric-safety.md`.

Method-call evaluation records the receiver place without opening an element
view, evaluates arguments left to right, verifies the receiver is still live,
then opens the required List access. This permits an owned temporary produced by
one operation to be passed safely into a later operation on the same List.

A whole-List receiver continues to denote the same List after an argument
structurally mutates it, so `items.push(items.remove(0))` is well-defined for a
non-empty List. A receiver that is an element or other subplace is instead a
pending reservation: later argument evaluation cannot structurally mutate its
containing List or retire its entity base.

No iterator, `for` loop protocol, or Slice is defined in this storage milestone.

## 14. Text

`Text` is immutable UTF-8 text and follows the same single-home rules as List.

```keld
let first = "Keld"
let moved = take first
let copied = moved.copy()
show(moved)
```

Text literals, concatenation results, and function results are owned temporaries.
Named Text places require `take` for transfer or `.copy()` for duplication.
Ordinary function parameters loan Text for the call.

The physical representation may be inline, static, or heap-backed. This is not
observable. The compiler may share immortal literal bytes because Text is
immutable, but each source value still follows single-home state transitions.

Text does not support integer indexing. Initial Text operations are:

```text
byte_length
is_empty
== !=
+
copy()
```

`byte_length` has type `Int`; `is_empty` has type `Bool`. `+` concatenates and
returns an owned temporary. Concatenation loans both operands and does not move
named Text values; allocation failure produces `AllocationFault`. Length,
emptiness, and equality also use non-escaping read loans.
Equality compares the UTF-8 bytes exactly; it does not perform normalization or
locale-sensitive comparison. Substrings, scalar iteration, and stored views are
deferred with Slice so no index unit or escaping text view is implicit.

Text uses the same managed cleanup rules as List and may appear in struct and
entity fields.

## 15. Deferred Containers

`Map`, `Set`, and `Slice` are deferred until the List checker, runtime, and UX
pass their required tests.

- `Map` requires stable key equality, hashing, and entry access rules.
- `Set` depends on the same key contract.
- `Slice` requires a first-class scoped-view design and structural invalidation
  rules.

These names have no standard-library meaning in the initial storage milestone.
User declarations may not claim the reserved standard-library module paths for
them.

Tuple and fixed-array syntax is also deferred. Their future storage classes must
compose from their elements, but no layout or expression syntax is reserved by
this milestone.

## 16. Diagnostics

Required diagnostics include:

### KLD2001: ambiguous single-home use

```text
error[KLD2001]: `items` owns managed storage and cannot be copied implicitly
  use `take items` to transfer it
  use `items.copy()` to create an independent value
```

### KLD2002: use after transfer

```text
error[KLD2002]: `items` was moved by `take items`
  assign a new value to this `var` before using it again
```

### KLD2003: borrowed value cannot be taken

```text
error[KLD2003]: parameter `items` is borrowed for this call
  declare the parameter as `take items: List[Int]` to consume it
```

### KLD2004: partial move is unavailable

```text
error[KLD2004]: cannot move storage out of field `team.names`
  use `team.names.copy()` to create an independent value
  owned field extraction is not available in Keld 0.1
```

### KLD2005: conflicting call loans

```text
error[KLD2005]: these arguments may access the same List incompatibly
  the first argument is structurally mutated
  the second argument reads the same storage during the call
```

### KLD2006: value is not structurally duplicable

```text
error[KLD2006]: this value's type is not structurally duplicable
  every component must support compiler-defined `copy()`
```

### KLD2007: indexed destination changed during replacement

```text
error[KLD2007]: the replacement structurally mutates the destination List
  the saved index could select a different element afterward
  perform the structural operation in a separate statement
```

## 17. Required Verification

The storage implementation must include compile-pass and compile-fail tests for:

- `let b = a`, `take a`, and `a.copy()` for List and Text;
- use after move, double move, branch joins, and `var` reinitialization;
- MaybeLive conditional replacement and cleanup on every exit edge;
- borrowed, consuming, temporary, and returned function arguments;
- overlapping read, edit, structural, and take call arguments;
- pending argument and subplace-receiver reservations during later argument
  evaluation;
- whole-aggregate transfer and rejected partial moves;
- entity fields whose bases are equal, distinct, and may alias;
- nested `List[List[Text]]` transfer, copy, removal, and cleanup;
- copyable and single-home element indexing;
- bounds, removal, capacity growth, and cleanup order;
- failed field-replacement evaluation preserving the old value;
- indexed-assignment right sides that attempt to move or structurally mutate the
  destination base;
- literal, inline, and heap-backed Text representations; and
- exact non-normalizing Text equality; and
- the absence of Map, Set, Slice, substring, iterator, and `for` support in the
  milestone.

Interpreter, optimized native, and WebAssembly executions must agree on all
observable results and deterministic faults. Allocation-failure comparison uses
the same injected failure schedule because real resource availability is
target-dependent.
