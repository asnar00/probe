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
  `name: ty = op ...` — the same form as function and block parameters. Opcodes
  are pure operations with no type suffixes, which keeps the opcode set small;
  the verifier checks that operand and result types are consistent. Signedness
  is part of the type too: there is one `div`, one `shr`, one `icmp.lt`, and
  `i5` versus `u5` says which one you mean.
- **No nesting, no expressions.** One instruction per line, every intermediate
  value named. This is the layer *below* everything clever.
- **No sigils.** `sum`, `entry`, `n`, `f32` are all plain words; the grammar
  never needs a prefix to tell them apart, so there are none.

## Lexical

- Comments: `;` to end of line.
- Names: `[A-Za-z0-9_]+`, for values, blocks, functions, and types alike.
  No prefixes — position says which is which: a name before `:` defines a
  value, after `:` names a type, after `call` names a function, after
  `jmp`/`br` names a block. Each value is defined exactly once.
- Integer literals: decimal, optionally negative (`42`, `-7`), or hex (`0x2a`).
- Whitespace is insignificant except as a separator; newlines end instructions.

## Types

| type    | meaning                                                    |
|---------|------------------------------------------------------------|
| `iN`    | signed integer of N bits, 1 ≤ N ≤ 64 (`i1`, `i5`, `i32`, `i64`) |
| `uN`    | unsigned integer of N bits (`u1` is the boolean, `u23`, `u64`) |
| `ptr`   | pointer (64-bit natively; a 32-bit offset on wasm)         |
| `name` | a pack: bitfields packed into at most 64 bits (see *Packs*) |
| `int`, `uint` | abstract integers — resolved to a concrete width by the target's replacement policy (see *Abstract numeric types*) |

Any width works anywhere a value lives — registers, block parameters,
calls, packs. Memory is the exception: only 8-, 16-, 32-, and 64-bit types
can be loaded and stored.

Floats are reserved for a later version (`float` will join `int` as an
abstract type when they land); their bit layouts are already expressible
as packs.

## Structure

```
fn name(a: i64, b: ptr) -> i64 {
entry:
    ...
next(x: i64):
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
name: ty = op operands...
```

Instructions with no result (`store`, result-less `call`, terminators) have no
left-hand side.

### Constants

```
v: i64 = iconst 42
p: ptr = iconst 0          ; null
u: ptr = iconst 0x10000000 ; a raw address — MMIO registers, fixed buffers
```

### Integer arithmetic and bitwise ops

Both operands and the result must all have the same integer type. Results
wrap at the type's width; the type's signedness selects the operation.

```
v: i5 = iadd a, b
v: i5 = isub a, b
v: i5 = imul a, b
v: i5 = div a, b        ; signed for iN, unsigned for uN (truncating)
v: i5 = rem a, b        ; remainder, sign follows the dividend for iN
v: i5 = and a, b
v: i5 = or  a, b
v: i5 = xor a, b
v: i5 = shl a, b        ; amount mod 32/64 for those widths; >= N unspecified otherwise
v: i5 = shr a, b        ; arithmetic (sign-fill) for iN, logical for uN
```

Division by zero is target-dependent (wasm traps, the CPUs return 0);
`MIN div -1` wraps to `MIN` at every width.

### Comparison

Operands must share an integer or `ptr` type; the result is `u1`. The
condition is part of the opcode; the ordering is signed for `iN` and
unsigned for `uN` and `ptr`.

```
c: u1 = icmp.eq a, b    ; also: ne
c: u1 = icmp.lt a, b    ; also: le gt ge
```

### Width changes and reinterpretation

The source and result types determine everything; the opcode says which
direction you meant, and the verifier holds you to it.

```
v: i64 = ext a            ; widen: sign-fills from an iN, zero-fills from a uN
v: u8  = trunc a          ; narrow: keeps the low bits
v: u5  = bitcast a        ; same width, reinterpreted (i5 <-> u5, pack <-> uN, ptr <-> i64/u64)
```

The result is always a proper value of its type: `ext` of an `i5` holding
-3 into a `u8` gives 253, `bitcast` of it into `u5` gives 29.

### Memory

The address operand must be `ptr`. The access width is the result type (loads)
or the stored value's type (stores), which must be 8, 16, 32, or 64 bits
wide — an integer, a pack, or `ptr`. Loads of `iN` sign-extend, of `uN`
zero-extend.

```
v: i64 = load addr
b: u8  = load addr
store v, addr
p: ptr = ptradd base, off    ; base: ptr, off: i64 or u64
```

### Calls

Callees are named symbols. Signatures are checked against the module if the
function is defined here, taken on trust if external.

```
v: i64 = call f(a, b)             ; call with one result
q: i64, r: i64 = call divmod(a, b)  ; call with two results
call g(a)                           ; call with results ignored (or none)
```

A call binds either *all* of the callee's return values or *none* of them
(`call` and `unpack` are the only instructions that define more than one value).

### Terminators

Branch arguments must match the target block's parameters in count and type.

```
jmp next(a, b)
br c, then(a), else()   ; c: u1 — empty parens may be omitted
ret v                      ; one return value
ret q, r                  ; multiple return values
ret                         ; none
```

### Packs

A `pack` is a record of bitfields laid out **lowest bits first**: the first
field occupies bit 0 upward, the next starts where it ends, and the total
must fit in 64 bits. Fields are integers or other packs; a pack value is
carried as the unsigned integer of its total width and can go anywhere a
value can — parameters, block parameters, returns, memory if it is 8, 16,
32, or 64 bits wide.

```
pack rgb { r: u5, g: u6, b: u5 }       ; 16 bits: r = bits 0-4, g = 5-10, b = 11-15
pack pix { c: rgb, a: u8 }            ; 24 bits, nested

c: rgb = pack r, g, b              ; one value per field, in order
g: u6 = get c, g                      ; read a field (iN fields sign-extend)
d: rgb = set c, g, g2               ; a copy with one field replaced
r: u5, g: u6, b: u5 = unpack c      ; every field at once
w: u16 = bitcast c                    ; the raw bits, and back again
```

Declarations may appear anywhere at the top level; a pack must be declared
before it is used as a field of another. `unpack` is, with `call`, the only
instruction that defines several values.

## Example

```
; sum of 0..n
fn sum(n: i64) -> i64 {
entry:
    zero: i64 = iconst 0
    jmp loop(zero, zero)
loop(i: i64, acc: i64):
    done: u1 = icmp.ge i, n
    br done, exit, body
body:
    acc2: i64 = iadd acc, i
    one:  i64 = iconst 1
    i2:   i64 = iadd i, one
    jmp loop(i2, acc2)
exit:
    ret acc
}
```

## Abstract numeric types

`int` is an **abstract integer type**: code written with it does not choose
a width — the compiler does, at compile time, by a *replacement policy*
derived from the target (its natural register width, or a size-oriented
choice like i32 on wasm32) and from user concerns (`--int=i32|i64`).
`uint` is its unsigned twin and always takes the same width.
Because types live on variables, resolution is a single rewrite of the
value tables before verification; opcodes, instructions, and everything
downstream see only concrete types.

```
fn gcd(a: int, b: int) -> int {     ; width chosen per target/policy
    ...
    r: int = rem x, y               ; same ops, abstractly typed
```

- Abstract and concrete types mix freely (`i1` conditions, `ptr`
  addresses, explicit `i32`/`i64` where a width is required).
- A width-change cast (`ext`/`trunc`) between `int` and a concrete
  type is only valid under policies where the widths actually differ — the
  verifier checks the resolved program, so such code ties itself to a
  policy. Policy-portable code keeps casts among concrete types.
- Memory keeps concrete types in portable code: a load of `int` changes
  access width with the policy.
- `float` will join `int` when concrete float types land.

## Structured control flow

A function body that opens with a statement instead of a `label:` is in
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
if c {                         ; plain: arms fall through to what follows
    ...
}

if c { ... } else { ... }      ; either arm may end with break/continue/ret

r: i64 = if c {               ; value-yielding: results bound on the left,
    yield a                    ; each arm must end with 'yield' (matching
} else {                        ; count and types), and else is required
    yield b
}
```

Lowering: `br` into two arm blocks; `yield`s and fallthroughs become jumps
to a join block whose parameters are the bound results.

### loop

```
sum: i64 = loop(i: i64 = zero, acc: i64 = zero) {
    done: u1 = icmp.ge i, n
    if done {
        break acc              ; exit the loop, yielding its results
    }
    ...
    continue i2, acc2         ; back edge: new values for the loop vars
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
4. Operand types obey each instruction's typing rule above; `br` conditions
   and `icmp` results are `u1`; `iconst` literals fit their type under
   either the signed or the unsigned reading.
5. The entry block has no parameters and is not the target of any branch.
6. `ret` operands match the function's declared return types in count and
   type; a result-binding call matches the callee's return types the same way.

Deliberately *not* checked in v0.1: dominance (that every use is reached only
after its definition). The parser's scoping makes most violations awkward to
write, and the emitter will surface the rest; a real dominance check can come
with the optimizer.
