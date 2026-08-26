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
| `iN`    | signed integer of N bits, 1 ≤ N ≤ 256 (`i1`, `i5`, `i32`, `i64`, `i128`) |
| `uN`    | unsigned integer of N bits (`u1` is the boolean, `u23`, `u64`) |
| `ptr`   | pointer (64-bit natively; a 32-bit offset on wasm)         |
| `name`, `name(8, 23)` | a declared type, plain or instantiated with widths (see *Type declarations*) |
| `pack { ... }` | bitfields packed into at most 256 bits (see *Packs*) |
| `struct { ... }` | fields side by side, never a bit pattern (see *Structs*) |
| `i(expr)`, `u(expr)` | an integer whose width is an expression — inside type declarations |
| `int`, `uint`, `float`, `fixed`, `unit`, `sunit`, `rational`, `scalar` | abstract numbers — resolved to a concrete type by the target's replacement policy (see *Abstract numeric types*) |

Any width works anywhere a value lives — registers, block parameters,
calls, packs. Memory is the exception: only 8-, 16-, 32-, 64-bit types
and whole words above that (128, 192, 256) can be loaded and stored.

A value wider than 64 bits is *wide*: it is written and checked like any
other, then lowered to a row of 64-bit words, lowest first, right after
parsing (`src/wide.rs`), so that no backend ever meets one. A `u128`
parameter is two `u64` parameters, a `u128` result two results, and
each operation becomes the word operations that compute it — carry
chains, schoolbook products, word-and-bit arrangement for shifts,
lexicographic compares. `div` and `rem` alone go to the library
(`lib/wide.ssa`'s `div(W)`/`rem(W)`, loops written over the wide type
and lowered like everything else). `suite/wide.ssa` shows the shape:
`;! add 1 0 2 0 -> 3, 0` is `add` on two `u128`s given as words. A
`const` on a wide value takes a 128-bit width expression (`const 1 <<
112`). The libraries use wide types wherever an intermediate outgrows a
word — `float`'s `mul` forms the exact `2M + 2`-bit product in `u(2 * M
+ 10)`, `fixed`'s in `u128` — which is what lets one `add(E, M, round)`
body serve `float(4, 3)` and `float(15, 112)` alike (`suite/f128.ssa`).

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

On a library number type (`fixed`, `rational`, `unit`, ... — any pack
its library can `conv` into from `i64`), a literal is a *value*: `const
0.5` on a `fixed(8, 8)` is 128/256, `mul x, 0.5` halves whatever `x` is,
and `y: scalar = sub 1, x` means one minus `x` in every family. This
costs nothing in the compiler: the literal is read as an `i64` or an
`f64` and handed to the library's own `conv`, hidden like the `const`.

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
`MIN div -1` wraps to `MIN` at every width (the wasm emitter guards its
trapping `div_s` to make that so).

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
wide — an integer, a pack, or `ptr` — or a wide value's whole words
(128, 192, 256 bits, stored low word first). Loads of `iN` sign-extend, of `uN`
zero-extend.

```
v: i64 = load addr
b: u8  = load addr
store v, addr
v: i64 = load base, 16         ; base + 16
v: i32 = load base, i, 4       ; base + i * 4 (i: i64 or u64; step 1, 2, 4 or 8)
store v, base, i, 4
p: ptr = ptradd base, off    ; base: ptr, off: i64 or u64
```

Values are an unbounded set of named registers; `load` and `store` are
the only way in and out of memory, and the two addressing forms are what
the targets' load and store instructions take themselves (an immediate
offset on all three; the scaled index computes an address first where
the target has no form for it). Whether a value spills to the stack is
the allocator's business and invisible here.

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
must fit in 256 bits (above 64 it is a wide value, in words). Fields are
integers or other packs; a pack value is carried as the unsigned integer
of its total width and can go anywhere a value can — parameters, block
parameters, returns, memory if it is 8, 16, 32, or 64 bits wide or whole
words.

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

### Structs

```
type point = struct { x: f32, y: f32, z: f32 }
p: point = pack x, y, z
z: f32 = get p, z
q: point = set p, z, 1.0
a: point = load base, i, 12     ; element i of an array of 12-byte structs
store q, base, 16
```

A `struct` is a group of fields — integers, packs, `ptr`, wide values,
other structs — side by side: in memory at their natural offsets (each
field aligned to its size, the whole to its largest field), and in
registers as separate values. It shares the pack vocabulary (`pack`,
`unpack`, `get`, `set`, `load`, `store`, parameters, results, block
parameters) and differs in one thing: it is never a bit pattern. There
is no `cast` to or from a struct, no literal, no arithmetic dispatch —
a program cannot observe how one is laid out, which is what leaves the
layout to the compiler (an array of structs may be stored field-major
one day without a program noticing). A struct is dissolved into its
fields right after parsing (`src/aggregate.rs`): `pack`, `get`, `set`
and `unpack` become names for values that already exist, a `load` or
`store` becomes one per field at its offset, and a struct parameter is
its fields in order — which is also how the suite passes one
(`suite/struct.ssa`).

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
fn add(E, M, round)(a: float(E, M), b: float(E, M)) -> float(E, M) {
    ...
    n1: float(E, M) = fnan(E, M)()      ; instantiates fnan for this E, M
    ...
    r: float(E, M) = fpack(E, M, round)(sh, nx32, nf)
    ret r
}
fn fadd32 = add(8, 23)                      ; a named instantiation
r: f16 = add(5, 10)(x, y)              ; an anonymous one, add_5_10_0
s: f16 = add x, y                           ; the same, by dispatch
```

A parameter nothing binds is supplied by the policy when it has a value
by that name: `round` is the one such name, 0 nearest even, 1 toward
zero, 2 down, 3 up, 4 nearest away (`--round=even|zero|down|up|away`).
That is why `add(8, 23)` and `add x, y` name two of the three
parameters: the mode comes from the policy — or from the enclosing
instantiation when a generic with a `round` of its own calls another,
so `sub`'s `add a, nb` rounds as `sub` does. `add(8, 23, 2)` fixes it
regardless of policy (`suite/round.ssa`). In the library every rounding
decision is in `fpack`, and since the mode is a width parameter, all
but one of its tests fold away in each instance.

Instantiations are shared: `add(8, 23)` anywhere is `fadd32` once that
name exists. `probe parse` prints the instantiated functions and not the
templates — like structured control flow, generics are sugar the parser
lowers. A pack literal `const` is its bit pattern.

**The prelude.** Every program compiled by probe gets `lib/*.ssa`
appended (floats `lib/float.ssa`, fixed point `lib/fixed.ssa`, unit
fractions `lib/unit.ssa`, two-word integer helpers `lib/wide.ssa`), so
`float(E, M)`, `fixed(I, F)`, `unit(N)`, `sunit(N)`, `f32`, `f64`,
`f16`, `bf16`, and the operations on them are always in scope; a file may
re-declare a type identically, and may name a type the prelude declares
after it. An explicit instantiation by name (`add(8, 23)`) needs the
name to be unambiguous — `add` has a float and a fixed form, so apply
it as an operation and let the types choose.

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
says which of them the target has an instruction for, and where values
of a type live, as rules in `targets/<target>.platform`:

```
class s = f32
class d = f64
fadd {s}, {s}, {s} = add(f32, f32) -> f32
fcvtzs {w}, {s} = conv(f32) -> i32
lt(f32, f32) -> u1
    fcmp a, b
    cset r, lo
```

A `class` line gives the types named a register class — the slot letter
the learned templates use for it (`s`/`d` on arm64, `f` on riscv64, a
local type on wasm) — so the allocator keeps such values in that file:
a chain of float operations is `fadd`, `fmul`, `fsub` with nothing in
between, and a move between files happens only where a value really
changes class (a `cast` to its bits, a call boundary, a pack field
read). A one-line rule is a learned template and the library instance
it computes, the template's slots being the result then the arguments
in order; a rule that takes several instructions is a header with
indented lines over `a`, `b`, `c` and `r`, with literals for condition
and immediate slots. Types are written by their program names (`f32`
for `float(8, 23)`), resolved through the module's declarations, and
every line must resolve to a template the learner verified — a rule
naming an instruction it has no template for is an error, not a guess.
The files today cover `add`, `sub`, `mul`, `div`, `sqrt`, `neg`, `abs`,
`min`, `max`, `fma`, the six comparisons, on `f32` and `f64`, and `conv`
between those and to and from 32/64-bit integers — minus what a
target's own semantics rule out, which the file simply leaves out:
riscv64 has no float-to-int rule (its `fcvt.w.s` gives the maximum
integer for NaN where the library gives 0) and no `min`/`max` (its
`fmin` returns the number when one operand is NaN, the library returns
NaN); wasm has no `fma`. When an emitter compiles such an instance, or
a call to one, it emits the rule instead of the SSA body. A rule
matches the nearest-even instance (`add(8, 23, 0)`), which is what the
instructions compute; an instance in another rounding mode stays in the
library. The library body remains the reference: `--soft` compiles with
an empty platform, and the two must agree — and both are checked
against Berkeley TestFloat's vectors, bit for bit, in every mode, by
`probe testfloat` (`tools/get-testfloat.sh` builds the generator). NaN payloads are the one place they may differ — the
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

`rational(N, D)` (`lib/rational.ssa`) is `numerator / denominator`, an
`i(N)` over a `u(D)` kept reduced, with 128-bit intermediates so N and
D go to 64; `lib/time.ssa` builds on `rational(64, 64)`: `type time`,
`seconds`/`millis`/`micros`/`nanos`/`period` in, `to_*` out, and every
operation the rational library's — exact, so nothing drifts.

`fixed` is the same again for the library's `fixed(I, F)` — a
two's-complement integer of I + F bits with F fraction bits, in
`lib/fixed.ssa` — resolved to half the `int` width each side
(`fixed(32, 32)` with i64, `fixed(16, 16)` with i32) or `--fixed=I,F`.
Its operations are integer instructions all the way down; `conv`
reaches it from integers and floats, and back.

`unit` and `sunit` are fractions of one (`lib/unit.ssa`): `unit(N)` is
0.0 at 0 and 1.0 at 2^N − 1, `sunit(N)` is −1.0 at −(2^(N−1) − 1) and
1.0 at 2^(N−1) − 1. The scale is not a power of two, so a product is
`(a·b + half) / max`, rounded; sums saturate at the ends of the range;
`conv` goes to and from floats and integers. Bare, they take the policy's
N (half the `int` width; `--unit=N`, `--sunit=N`).

`rational(N, D)` (`lib/rational.ssa`) is `numerator / denominator`, an
`i(N)` over a `u(D)`, kept reduced with a positive denominator; a zero
denominator is *not a rational* (NaR) and propagates like NaN. Its
arithmetic is exact while the reduced result fits, and halved down to an
approximation when it doesn't; `conv` from a float takes the best
approximation the widths allow, by continued fractions (`0.33333334f32`
is `1/3` in `rational(8, 8)`, `3.14159` is `22/7`). Bare `rational` is
the policy's `(N, D)` (`--rational=N,D`).

`scalar` names a *family*: a bare `scalar` is whichever of `float`,
`fixed`, `rational`, `unit`, `sunit` the policy says (`float` unless
`--scalar=...`), itself bare, so that family's width applies. A program
over `scalar` — `suite/scalar.ssa` — runs unchanged in every family; the
suite runs it in all five.
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
