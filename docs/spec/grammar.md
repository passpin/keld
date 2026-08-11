# Keld Core Grammar

Status: normative Keld 0.1 core

The machine-readable production grammar is in [keld.ebnf](keld.ebnf). This file
defines lexical behavior and semantic restrictions that EBNF alone cannot
express.

## 1. Source Encoding

- A source file is UTF-8.
- A leading UTF-8 byte-order mark is accepted and ignored.
- Invalid UTF-8 is a lexical error.
- Line endings may be LF or CRLF and are normalized to LF.
- Tabs are permitted as whitespace but do not define block structure.

Keld uses braces for blocks. Indentation is conventional, not syntactic.

## 2. Identifiers

Keld 0.1 identifiers use ASCII for deterministic bootstrap behavior:

```text
IDENT := [A-Za-z][A-Za-z0-9_]* | _[A-Za-z0-9_]+
```

Identifiers are case-sensitive. Unicode is permitted in comments and strings.
Unicode identifiers may be added only with a pinned Unicode identifier version.
The lone `_` token is the wildcard pattern and is not an identifier.

## 3. Keywords

The following identifiers are reserved:

```text
any       as        break     continue  else      entity
enum      extern    false     fn        handle    if
in        keep      let       lifecycle link      match
module    none      pub       raises    retire    retires
return    struct    true      try       unsafe    use
var       when      while
```

These words are reserved for later language versions and are rejected as
identifiers even though Keld 0.1 has no productions for them:

```text
async await dynamic impl interface resource shared
```

## 4. Comments and Whitespace

- `//` begins a line comment.
- `/*` begins a non-nesting block comment terminated by `*/`.
- An unterminated block comment is a lexical error.
- Comments otherwise behave as whitespace.
- Newlines inside block comments participate in statement termination.

Whitespace inside parentheses and brackets never terminates a statement.

## 5. Literals

Keld 0.1 supports:

```text
INT    := DIGIT { DIGIT | "_" DIGIT }
STRING := '"' { character | escape } '"'
```

Integer literals are decimal. An underscore must occur between digits. Integer
signs are unary operators, not part of the literal.

Strings contain UTF-8 text. The required escapes are:

```text
\0 \n \r \t \\ \" \u{HEX}
```

`\u{HEX}` contains one to six hexadecimal digits and must denote a Unicode scalar
value. Raw strings, byte strings, floating-point literals, and non-decimal
integers are outside the first milestone.

## 6. Statement Terminators

The parser consumes a normalized `TERM` token. The lexer emits `TERM` for:

- an explicit semicolon;
- a qualifying newline;
- the end of a file after a token that can end a statement; or
- immediately before `}` after a token that can end a statement.

A newline qualifies when nesting depth for `(` and `[` is zero and the previous
significant token is one of:

```text
IDENT INT STRING true false none break continue return
) ] } ?
```

No `TERM` is emitted after an infix operator, comma, dot, colon, opening
delimiter, `=`, `=>`, or `->`. Consequently, a multiline expression should break
after an operator or comma, or remain inside parentheses or brackets.

Blank lines may produce repeated `TERM` tokens; the grammar accepts them. A
newline between `}` and `else` or `handle` may produce `TERM`; the corresponding
grammar productions consume it.

The formatter omits explicit semicolons except when multiple statements appear
on one physical line.

## 7. Types

`T?` is an optional value with either `T` or `none`.

`link T` is a non-empty persistent identity value. Its target can still retire,
so resolving it is conditional.

`link T?` means `(link T)?`: the link storage itself may contain `none`. It never
means `link (T?)`.

Examples:

```keld
let current: Enemy? = none
let target: link Enemy = enemy
let optional_target: link Enemy? = none
```

`none` obtains its concrete optional type from context.

Generic parameters and arguments use square brackets:

```keld
struct Pair[A, B] {
    first: A
    second: B
}

var enemies: List[link Enemy]
```

Square brackets avoid a lexical conflict between a closing generic delimiter and
the `>` comparison operator. A link target must be a named entity type, possibly
with generic arguments; `link (Enemy?)` is not grammatical.

## 8. Bindings and Mutation

`let` creates a binding that cannot be rebound. `var` creates a binding that may
be rebound.

A `let` binding requires an initializer. A `var` binding may omit its initializer
only when it has an explicit type; definite-assignment analysis rejects reads
before the first assignment.

Binding immutability does not freeze an entity payload:

```keld
let enemy = Enemy(health: 100)
enemy.health -= 10
```

Assignment is a statement, not an expression. An expression statement must have
type `Unit`; silently discarding another value is a type error.

## 9. Construction and Calls

`Name(arguments)` is parsed uniformly as a call. Name resolution decides whether it is
a function call, value constructor, enum variant, or entity constructor.

Arguments are either all positional or all named. Named arguments use `:`:

```keld
Enemy(health: 100, target: none)
```

A positional argument cannot follow a named argument. Duplicate named arguments
are rejected.

Using an `EntityRef[T]` in a context that explicitly expects `link T` creates the
persistent link value implicitly. This includes a typed field, argument, return,
or local binding. No other implicit entity-to-value conversion exists.

## 10. Entity Identity and Link Resolution

For entity references, `==` and `!=` compare stable identity. Inside the true
branch of `a != b`, the verifier may treat `a` and `b` as distinct.

`when expression as name` evaluates and validates `expression` once. The
expression must have an optional value type or a link type. For a link, success
binds a scoped live `EntityRef`; for another optional value, success binds its
contained value.

```keld
when boss.target as target {
    target.health -= 10
} else {
    choose_target(boss)
}
```

There is no force-unwrap operator in safe Keld 0.1.

## 11. Lifecycles

`lifecycle name { statements }` introduces `name` in a separate lexical lifecycle
namespace. The name is visible inside its block and nested blocks.

`keep entity in name` requires `name` to denote an active ancestor lifecycle.
The operation extends the entity's lifetime and does not move its storage.

`retire entity` requires a live `EntityRef`. It retires that identity and updates
the compile-time state of every must-alias and may-alias reference.

Lifecycle names are not ordinary values in Keld 0.1. They cannot be stored,
passed as arguments, returned, or captured.

## 12. Function Effects

Function clauses occur after the return type in this order:

```text
raises-clause, then retires-clause
```

Function parameter types and non-`Unit` return types are explicit. Omitting the
return clause means `Unit`; it does not request return-type inference.

Examples:

```keld
fn load(path: Path) -> Document raises IoError, ParseError {
    return read_and_parse(path)
}

fn remove(enemy: Enemy) retires enemy {
    retire enemy
}

fn sweep(world: World) retires any Enemy {
    when world.next_enemy as enemy {
        retire enemy
    }
}
```

`retires name` identifies a direct entity parameter. `retires any T` is a broad
effect over compatible `T` entities in the current hidden store. Duplicate,
contradictory, or unneeded declared retirement effects are compile-time errors.

## 13. Operator Precedence

From highest to lowest:

| Level | Operators | Associativity |
|---:|---|---|
| 1 | `()` call, `[]` index, `.` field | left |
| 2 | `!`, unary `-` | right |
| 3 | `*`, `/`, `%` | left |
| 4 | `+`, `-` | left |
| 5 | `<`, `<=`, `>`, `>=` | non-associative |
| 6 | `==`, `!=` | non-associative |
| 7 | `&&` | left |
| 8 | `||` | left |

Comparison chaining such as `a < b < c` is rejected. Write
`a < b && b < c`.

Assignment operators are outside expression precedence:

```text
+= -= *= /= %=
```

## 14. Evaluation Order

Expressions evaluate left to right. The compiler cannot reorder observable I/O,
error, retirement, lifecycle, or allocation effects.

An entity field read opens a read `View`, copies or materializes the field result,
and closes the view before the next structural effect. Passing an entity to a
function passes its `EntityRef`; the callee opens any required views.

Assignment evaluates in this order:

1. evaluate the base and index components of the destination to values and
   `EntityRef` identities without retaining a payload address;
2. evaluate the right-hand expression;
3. verify that the destination's live proof remains valid;
4. open the final edit `View`, perform the store, and close the view.

A compound assignment also reads and saves the old destination value after step
1, closes that read view, then evaluates the right-hand expression. Its final
calculation uses the saved value.

Therefore `enemy.health = make_enemy().health` does not hold a view of `enemy`
across the allocation performed by `make_enemy`.

## 15. Module Form

A file may begin with one module declaration followed by imports and items:

```keld
module game.world

use game.math
use game.render.Color
```

An unsafe module begins with `unsafe module`. Foreign declarations are permitted
only in unsafe modules. Unsafe and foreign features are parsed by the core
grammar but excluded from the first executable milestone.

## 16. Grammar Scope

The EBNF covers the accepted Keld 0.1 core syntax, including syntax scheduled
after the first executable milestone. Parsing does not imply semantic support:
the compiler must issue a focused unsupported-feature diagnostic when a parsed
feature is not enabled by the selected language version.

Interfaces, implementations, closures, async functions, shared lifecycles,
resource declarations, macros, and package manifests remain ungrammatical in
Keld 0.1. Their reserved keywords prevent accidental source incompatibility.
