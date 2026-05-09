# DByte Language Specification — v0.1

## Overview

DByte is a statically-typed, Python-syntax-inspired language that compiles to native binaries.

**File extension:** `.dby`  
**CLI:** `dbyte run <file>` / `dbyte check <file>`

---

## Primitives

| Type    | Example              |
|---------|----------------------|
| `int`   | `42`, `-7`           |
| `float` | `3.14`, `-0.5`       |
| `bool`  | `true`, `false`      |
| `str`   | `"hello"`            |

---

## Variables

```dbyte
let name: str = "Deadbyte"
let age: int = 17
let fast = true
```

Type annotation is optional — the compiler infers it.

---

## Functions

```dbyte
fn add(a: int, b: int) -> int:
    return a + b
```

Return type annotation is optional. Functions without explicit return type return `void`.

---

## Operators

| Category       | Operators                        |
|----------------|----------------------------------|
| Arithmetic     | `+` `-` `*` `/`                  |
| Comparison     | `==` `!=` `<` `<=` `>` `>=`     |
| Unary          | `-` (negate) `!` (boolean not)   |

---

## Control Flow

```dbyte
if age >= 18:
    print("adult")
else:
    print("teen")
```

---

## Built-ins

| Function | Description                        |
|----------|------------------------------------|
| `print`  | Print values to stdout, space-sep  |

---

## Block Syntax

Blocks use **indentation** (4 spaces recommended, tabs = 4 spaces).

```dbyte
fn foo():
    let x: int = 1
    if x == 1:
        print("one")
```

---

## Error Reporting

All errors include file path, line, and column:

```
TypeError: expected int, found str
 --> main.dby:1:14
  |
1 | let x: int = "hello"
  |              ^
```

---

## Roadmap

| Version | Target                                  |
|---------|-----------------------------------------|
| v0.1    | Lexer, Parser, Tree-walk interpreter, Type checker |
| v0.2    | `for` loops, `while`, `list[T]`         |
| v0.3    | Bytecode VM                             |
| v0.4    | `spawn` / `await` concurrency           |
| v0.5    | Cranelift AOT backend (native binary)   |
| v0.6    | Python interop (`py import`)            |
| v1.0    | LSP, package manager, Tree-sitter grammar |
