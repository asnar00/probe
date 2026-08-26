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
  is part of the type too: there is one `div`, one `shr`, one `cmp.lt`, and
  `i5` versus `u5` says which one you mean. And there is one `add`: on an
  integer it is the integer instruction, on a `float(8, 23)` it is whatever
  the library's `add(E, M)` says — or the platform's `fadd`.
- **No nesting, no expressions.** One instruction per line, every intermediate
  value named. This is the layer *below* everything clever.
- **No sigils.** `sum`, `entry`, `n`, `f32` are all plain words; the grammar
  never needs a prefix to tell them apart, so there are none.

## Lexical

- Comments: `;` to end of line.
- Names: `[A-Za-z0-9_]+`, for values, blocks, functions, and types alike.
  No prefixes — position says which is which: a name before `:` defines a
  value, after `:` names a type, before `(` names a function being called,
  after `jmp`/`br` names a block. Each value is defined exactly once.
- Integer literals: decimal, optionally negative (`42`, `-7`), or hex (`0x2a`).
  Float literals: a decimal with a fraction or an exponent (`1.5`, `2e10`,
  `-1.0e-3`), plus `inf`, `-inf`, `nan`.
- Whitespace is insignificant except as a separator; newlines end instructions.

## Types

| type    | meaning                                                    |
|---------|------------------------------------------------------------|
| `iN`    | signed integer of N bits, 1 ≤ N ≤ 64 (`i1`, `i5`, `i32`, `i64`) |
| `uN`    | unsigned integer of N bits (`u1` is the boolean, `u23`, `u64`) |
| `ptr`   | pointer (64-bit natively; a 32-bit offset on wasm)         |
| `name`, `name(8, 23)` | a declared type, plain or instantiated with widths (see *Type declarations*) |
| `pack { ... }` | bitfields packed into at most 64 bits (see *Packs*) |
| `i(expr)`, `u(expr)` | an integer whose width is an expression — inside type declarations |
| `int`, `uint`, `float` | abstract numbers — resolved to a concrete width by the target's replacement policy (see *Abstract numeric types*) |

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

Instructions with no result (`store`, result-less calls, terminators) have no
left-hand side.

### Constants

```
v: i64 = const 42
p: ptr = const 0          ; null
u: ptr = const 0x10000000 ; a raw address — MMIO registers, fixed buffers
c: rgb = const 4095       ; a pack, by its bit pattern
w: u(M) = const (1 << M) - 1   ; inside a generic: an expression over its parameters
x: f32 = const 0.1        ; a float, by its value: the nearest f32, exactly rounded
y: f16 = const 3          ; 3.0
z: f64 = const -inf       ; also inf, nan
```

On a `float(E, M)` the literal is a number, converted to the nearest
value of that type (ties to even, subnormals included) — the same bits the
FPU would produce from the same decimal. Its bit pattern is a `cast` from
an integer away.

**Literals as operands.** Anywhere an operand's type is fixed by context,
a literal may stand in for a value: the other operand of `add`, `cmp`,
and friends; a pack's field; a call's parameter; a block's parameter; the
function's return type; the loop variable's declared type.

```
b: i64 = add a, 1
lt: u1 = cmp.lt 0, b
half: f32 = mul x, 0.5
jmp loop(0, 0)
ret 0
r: i64 = g(b, 2)
```

Where nothing fixes the type — a stored value, a `conv` source — write it
after the literal: `store 1: u8, p`. A literal becomes a hidden `const`
just before the instruction; `probe parse` prints it back inline.

### Integer arithmetic and bitwise ops

Both operands and the result must all have the same type. On integers,
results wrap at the type's width and the type's signedness selects the
operation. On a pack instantiated from a generic type, the opcode is
dispatched to the generic function of the same name that takes that type
(see *Generic functions*).

```
v: i5 = add a, b
v: i5 = sub a, b
v: i5 = mul a, b
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

Operands must share a type; the result is `u1`. The condition is part of
the opcode; on integers the ordering is signed for `iN` and unsigned for
`uN` and `ptr`. On a pack, `cmp.lt` is the library's `lt` for that type
(the float library's six predicates give IEEE's answers: everything but
`ne` is false when a NaN is involved, and -0 equals +0).

```
c: u1 = cmp.eq a, b    ; also: ne
c: u1 = cmp.lt a, b    ; also: le gt ge
```

### Conversion and reinterpretation

Two opcodes, and the types on both sides decide the rest. `conv` carries
the *value* across: between integers it widens (sign-filling from an
`iN`, zero-filling from a `uN`), narrows to the low bits, or re-reads at
the same width; with a pack on either side it is the library's `conv`
generic for that pair of types — `f32` to `f64`, `i32` to `f16`, `f64`
to `u8` (see *Generic functions*). `cast` keeps the *bits* and changes
the reading, between any two types of the same width.

```
v: i64 = conv a            ; i8 -> i64: sign-extended
v: u8  = conv a            ; i64 -> u8: the low byte
v: f64 = conv a            ; f32 -> f64: the same number, exactly
v: i32 = conv a            ; f32 -> i32: 1.0 -> 1, truncating, saturating, NaN -> 0
v: u32 = cast a            ; f32 -> u32: 1.0 -> 0x3f800000
v: u5  = cast a            ; i5 -> u5: -3 -> 29
```

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

A name followed by an argument list is a call — no opcode is ever
followed by `(`, so no keyword is needed. Signatures are checked against
the module if the function is defined here, taken on trust if external.

```
v: i64 = f(a, b)             ; call with one result
q: i64, r: i64 = divmod(a, b)  ; call with two results
g(a)                           ; call with results ignored (or none)
```

A call binds either *all* of the callee's return values or *none* of them
(calls and `unpack` are the only instructions that define more than one value).

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
type rgb = pack { r: u5, g: u6, b: u5 }      ; 16 bits: r = bits 0-4, g = 5-10, b = 11-15
type pix = pack { c: rgb, a: u8 }            ; 24 bits, nested

c: rgb = pack r, g, b                        ; one value per field, in order
g: u6 = get c, g                             ; read a field (iN fields sign-extend)
d: rgb = set c, g, g2                        ; a copy with one field replaced
r: u5, g: u6, b: u5 = unpack c               ; every field at once
w: u16 = cast c                           ; the raw bits, and back again
```

Packs are compared structurally: two spellings of the same layout are the
same type. `unpack` is, with calls, the only instruction that defines
several values.

### Type declarations

`type` names a type, optionally with integer parameters that stand for
widths. The right-hand side is any type expression: a pack, `i(expr)` or
`u(expr)` with a width expression over the parameters (`+ - *` and
parentheses), a builtin, or another declared type instantiated with
arguments.

```
type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }
type f32 = float(8, 23)
type f16 = float(5, 10)
type bits(E, M) = u(E + M + 1)
type byte = u8
```

A parametric type is instantiated wherever it is used with arguments —
`x: float(8, 23)`, `y: bits(5, 10)` — and an alias is instantiated where
it is declared. `f32`, `float(8, 23)`, and `pack { mantissa: u23,
exponent: u8, sign: u1 }` are one type; it prints under the first name it
was given. Declarations may appear anywhere at the top level; each may
refer only to types declared before it.

### Generic functions

A function can take the same kind of width parameters, in a group before
its value parameters. It is a template: nothing is compiled until it is
instantiated, either by name or at a call site, and each instantiation is
an ordinary function whose body was parsed with the parameters bound —
so `u(M + 5)` is a concrete type there, and `const` may be a width
expression.

```
fn add(E, M)(a: float(E, M), b: float(E, M)) -> float(E, M) {
    hidden: u(M + 5) = const 1 << M
    ...
    n: float(E, M) = fnan(E, M)()      ; instantiates fnan for this E, M
    ...
}
fn fadd32 = add(8, 23)                      ; a named instantiation
r: f16 = add(5, 10)(x, y)              ; an anonymous one, add_5_10
s: f16 = add x, y                           ; the same, by dispatch
```

Instantiations are shared: `add(8, 23)` anywhere is `fadd32` once that
name exists. `probe parse` prints the instantiated functions and not the
templates — like structured control flow, generics are sugar the parser
lowers. A pack literal `const` is its bit pattern.

**The prelude.** Every program compiled by probe gets `lib/*.ssa`
appended (the float library lives in `lib/float.ssa`), so `float(E, M)`,
`f32`, `f64`, `f16`, `bf16`, and the operations on them are always in
scope; a file may re-declare a type identically.

**Dispatch.** A generic function whose first parameter and first result
are written in terms of its own width parameters *is* an operation of
its name: applying that name to a value whose type matches the parameter
— and whose declared result matches the result — instantiates it with
the widths the match binds. `add x, y` on two `f16` values lowers to
`add(5, 10)(x, y)`; `sqrt x` — not an integer opcode at all — to
`sqrt(5, 10)(x)`; `r: f16 = conv x` with `x: i32` finds the `conv(W, E, M)`
that takes `i(W)` and returns `float(E, M)`, binding all three. Generics
may share a name when their signatures differ, which is how `conv` from
`i(W)` and from `u(W)` coexist. The opcode set never grows: a library
adds operations, and a platform adds instructions.

### Platforms

A library instantiation defines what an operation *means*; a platform
says which of them the target has hardware for. Each backend carries a
table of generic instantiations it implements natively — today
`add`, `sub`, `mul`, `div`, `sqrt`, `neg`, `abs`, `min`, `max`, `fma`,
the six comparisons, on `float(8, 23)` and `float(11, 52)`, and `conv`
between those and to and from 32/64-bit integers — with the exceptions a
platform's own semantics force: riscv64 keeps float to int (its hardware
gives the maximum integer for NaN where the library gives 0) and
`min`/`max` (its `fmin` returns the number when one operand is NaN, the
library returns NaN) in the library, and wasm has no fused
multiply-add. On all three targets — and when it
compiles such an instance, or a call to one, it emits the instruction
sequence (arm64 `fmov`/`fadd`/`fmov`, riscv `fmv`/`fadd.s`, wasm
`f32.add` between reinterprets) instead of the SSA body. The library body
remains the reference: `--soft` compiles with an empty platform, and the
two must agree. NaN payloads are the one place they may differ — the
library canonicalizes, hardware propagates — as on any real platform.

## Example

```
; sum of 0..n
fn sum(n: i64) -> i64 {
entry:
    zero: i64 = const 0
    jmp loop(zero, zero)
loop(i: i64, acc: i64):
    done: u1 = cmp.ge i, n
    br done, exit, body
body:
    acc2: i64 = add acc, i
    one:  i64 = const 1
    i2:   i64 = add i, one
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

`float` is the same idea for the library's `float(E, M)`: a bare `float`
is `float(E, M)` for the policy's E and M — `(11, 52)` on the register
machines, `(8, 23)` on wasm32, or whatever `--float=f16|bf16|f32|f64|E,M`
says — instantiated as the parser meets it (a parametric type's bare name
is abstract when the policy has arguments for it). So `fn half(x: float)
-> float { r: float = div x, 2.0 }` is written once, dispatches to the
library's `div(E, M)` for the chosen width, and lands on the platform's
`fdiv` where there is one.
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
- A `conv` between `int` and a concrete
  type is only valid under policies where the widths actually differ — the
  verifier checks the resolved program, so such code ties itself to a
  policy. Policy-portable code keeps casts among concrete types.
- Memory keeps concrete types in portable code: a load of `int` changes
  access width with the policy.
- `float` code is policy-portable when its inputs and outputs are
  integers or its values stay exactly representable at every width in
  play (`suite/afloat.ssa`); the suite runs under both `int` policies and
  both `float` policies to keep it so.

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
    done: u1 = cmp.ge i, n
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
   and `icmp` results are `u1`; `const` literals fit their type under
   either the signed or the unsigned reading.
5. The entry block has no parameters and is not the target of any branch.
6. `ret` operands match the function's declared return types in count and
   type; a result-binding call matches the callee's return types the same way.

Deliberately *not* checked in v0.1: dominance (that every use is reached only
after its definition). The parser's scoping makes most violations awkward to
write, and the emitter will surface the rest; a real dominance check can come
with the optimizer.
