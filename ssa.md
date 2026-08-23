# probe SSA — v0.1

The input language of the lowest compiler stage. A module of functions; each
function is a graph of basic blocks in SSA form. Deliberately small: just enough
to express straight-line integer code, control flow, memory, and calls — the
things we can learn ARM64 encodings for by probing LLVM.

## Design choices

- **Block parameters, not phi nodes.** A value flowing into a join point is
  passed as an argument on the branch, and received as a parameter on the
  target block (the Cranelift / MLIR style). This keeps "where does this value
  come from" local to the branch instruction and makes the emitter simpler.
- **Types live on variables, not opcodes.** Every value definition is written
  `%name: ty = op ...` — the same form as function and block parameters. Opcodes
  are pure operations with no type suffixes, which keeps the opcode set small;
  the verifier checks that operand and result types are consistent.
- **No nesting, no expressions.** One instruction per line, every intermediate
  value named. This is the layer *below* everything clever.

## Lexical

- Comments: `;` to end of line.
- Values: `%name` — name is `[A-Za-z0-9_]+`. Each value is defined exactly once.
- Blocks: `^name` — same name rules.
- Functions: `@name`.
- Integer literals: decimal, optionally negative (`42`, `-7`), or hex (`0x2a`).
- Whitespace is insignificant except as a separator; newlines end instructions.

## Types

| type  | meaning                        |
|-------|--------------------------------|
| `i1`  | boolean (result of `icmp`)     |
| `i32` | 32-bit integer                 |
| `i64` | 64-bit integer                 |
| `ptr` | pointer (64-bit on our target) |

Floats are reserved for a later version.

## Structure

```
fn @name(%a: i64, %b: ptr) -> i64 {
^entry:
    ...
^next(%x: i64):
    ...
}
```

- A function declares named, typed parameters and zero or more return types.
  Internally the return is always a tuple; the text format allows `-> i64` as
  shorthand for `-> (i64)`, `-> (i64, i64)` for multiple values, and omitting
  the arrow entirely for none.
- The first block is the entry block. It takes no parameters; the function's
  parameters are in scope from the start.
- Every other block may declare parameters — the values it receives from
  branches that target it.
- Every block ends with exactly one terminator (`jmp`, `br`, or `ret`), and
  terminators appear nowhere else.

## Instructions

Every value-defining instruction declares its result's type:

```
%name: ty = op operands...
```

Instructions with no result (`store`, result-less `call`, terminators) have no
left-hand side.

### Constants

```
%v: i64 = iconst 42
%p: ptr = iconst 0          ; null
%u: ptr = iconst 0x10000000 ; a raw address — MMIO registers, fixed buffers
```

### Integer arithmetic and bitwise ops

Both operands and the result must all have the same type (`i32` or `i64`).

```
%v: i64 = iadd %a, %b
%v: i64 = isub %a, %b
%v: i64 = imul %a, %b
%v: i64 = sdiv %a, %b       ; signed divide
%v: i64 = udiv %a, %b       ; unsigned divide
%v: i64 = srem %a, %b       ; signed remainder
%v: i64 = urem %a, %b       ; unsigned remainder
%v: i64 = and  %a, %b
%v: i64 = or   %a, %b
%v: i64 = xor  %a, %b
%v: i64 = shl  %a, %b       ; shift amount taken mod bit-width
%v: i64 = lshr %a, %b       ; logical (zero-fill) shift right
%v: i64 = ashr %a, %b       ; arithmetic (sign-fill) shift right
```

### Comparison

Operands must share a type (`i32`, `i64`, or `ptr`); the result is `i1`. The
condition is part of the opcode (it selects an operation, not a type).

```
%c: i1 = icmp.eq  %a, %b    ; also: ne
%c: i1 = icmp.slt %a, %b    ; signed:   slt sle sgt sge
%c: i1 = icmp.ult %a, %b    ; unsigned: ult ule ugt uge
```

### Width changes

The source and result types determine the conversion; the opcode only picks
how new bits are filled.

```
%v: i64 = sext %a           ; sign-extend  (result wider than source)
%v: i64 = zext %a           ; zero-extend  (result wider than source)
%v: i32 = trunc %a          ; truncate     (result narrower than source)
```

Widths are ranked `i1 < i32 < i64`; `ptr` takes no part in width changes.

### Memory

The address operand must be `ptr`. The access width is the result type (loads)
or the stored value's type (stores): `i32`, `i64`, or `ptr` (64 bits).

```
%v: i64 = load %addr
store %v, %addr
%p: ptr = ptradd %base, %off    ; %base: ptr, %off: i64
```

### Calls

Callees are named symbols. Signatures are checked against the module if the
function is defined here, taken on trust if external.

```
%v: i64 = call @f(%a, %b)             ; call with one result
%q: i64, %r: i64 = call @divmod(%a, %b)  ; call with two results
call @g(%a)                           ; call with results ignored (or none)
```

A call binds either *all* of the callee's return values or *none* of them
(`call` is the only instruction that may define more than one value).

### Terminators

Branch arguments must match the target block's parameters in count and type.

```
jmp ^next(%a, %b)
br %c, ^then(%a), ^else()   ; %c: i1 — empty parens may be omitted
ret %v                      ; one return value
ret %q, %r                  ; multiple return values
ret                         ; none
```

## Example

```
; sum of 0..n
fn @sum(%n: i64) -> i64 {
^entry:
    %zero: i64 = iconst 0
    jmp ^loop(%zero, %zero)
^loop(%i: i64, %acc: i64):
    %done: i1 = icmp.sge %i, %n
    br %done, ^exit, ^body
^body:
    %acc2: i64 = iadd %acc, %i
    %one:  i64 = iconst 1
    %i2:   i64 = iadd %i, %one
    jmp ^loop(%i2, %acc2)
^exit:
    ret %acc
}
```

## Structured control flow

A function body that opens with a statement instead of a `^label:` is in
**structured form**: control flow is expressed with `if`/`loop` instead of
labels and branches (`jmp`, `br`, and labels are not allowed there). This is
sugar — the parser lowers it to the same block graph on the fly, so
everything downstream (verifier, emitters, printer) sees only flat form, and
`probe parse` prints the lowered graph. Flat and structured functions can
mix freely in one module. Lowering is one-way; the reverse direction
(CFG → structured, the "relooper" problem) is deliberately not attempted.

The design follows the MLIR `scf` pattern: constructs *yield values* instead
of writing to variables, preserving SSA.

### if

```
if %c {                         ; plain: arms fall through to what follows
    ...
}

if %c { ... } else { ... }      ; either arm may end with break/continue/ret

%r: i64 = if %c {               ; value-yielding: results bound on the left,
    yield %a                    ; each arm must end with 'yield' (matching
} else {                        ; count and types), and else is required
    yield %b
}
```

Lowering: `br` into two arm blocks; `yield`s and fallthroughs become jumps
to a join block whose parameters are the bound results.

### loop

```
%sum: i64 = loop(%i: i64 = %zero, %acc: i64 = %zero) {
    %done: i1 = icmp.sge %i, %n
    if %done {
        break %acc              ; exit the loop, yielding its results
    }
    ...
    continue %i2, %acc2         ; back edge: new values for the loop vars
}
```

- The parenthesized list declares **loop-carried variables** with their
  initial values; `continue` supplies the next iteration's values.
- `break` exits, yielding the loop's results (bound on the left; a loop
  with no results uses bare `break`).
- Every path through the body must end with `break`, `continue`, or `ret`.
- `break`/`continue` bind to the innermost enclosing loop.

Lowering: a header block whose parameters are the loop variables (`continue`
jumps to it), and an exit block whose parameters are the results (`break`
jumps to it).

### Termination rules

A statement list "terminates" when it ends with `ret`, `break`, `continue`,
`yield`, or an `if` all of whose arms terminate. Code after a terminating
statement is an error (it would be unreachable). A structured body that
falls off the end without `ret` fails verification (the final block has no
terminator), same as flat form.

## Rules (checked by the verifier)

1. Every value is defined exactly once (function param, block param, or
   instruction result).
2. Every block ends with exactly one terminator; no instruction follows it.
3. Branch argument counts and types match the target block's parameters.
4. Operand types obey each instruction's typing rule above.
5. The entry block has no parameters and is not the target of any branch.
6. `ret` operands match the function's declared return types in count and
   type; a result-binding call matches the callee's return types the same way.

Deliberately *not* checked in v0.1: dominance (that every use is reached only
after its definition). The parser's scoping makes most violations awkward to
write, and the emitter will surface the rest; a real dominance check can come
with the optimizer.
