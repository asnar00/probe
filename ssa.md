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
- **Types live on variables, not opcodes — signedness included.** Every value
  definition is written `name: ty = op ...`, and a type like `u5` or `i32`
  carries both width and signedness. `div`, `rem`, `shr`, ordered `icmp`,
  `ext`, `itof`/`ftoi` all take their behavior from their operand types, so
  there is exactly one opcode per operation and the data layout states the
  intent once.
- **No nesting, no expressions.** One instruction per line, every intermediate
  value named. This is the layer *below* everything clever.

## Lexical

- Comments: `;` to end of line.
- Values: `name` — name is `[A-Za-z0-9_]+`. Each value is defined exactly once.
- Blocks: `^name` — same name rules.
- Functions: `name`.
- Integer literals: decimal, optionally negative (`42`, `-7`), or hex (`0x2a`).
- Whitespace is insignificant except as a separator; newlines end instructions.

## Types

| type  | meaning                                                  |
|-------|----------------------------------------------------------|
| `i1`  | boolean (result of `icmp`)                               |
| `i32` | 32-bit integer                                           |
| `i64` | 64-bit integer                                           |
| `ptr` | pointer (64-bit on our target)                           |
| `iN`  | any-width integer, N in 2..=63 (e.g. `i5`, `i52`)        |
| `$name` | packed bitfield struct (see *Structs*), <= 64 bits     |
| `TxN`   | short vector: N lanes of T (see *Vectors*), <= 64 bits |
| `f32` | 32-bit IEEE float — the same type as `float(8, 23)`      |
| `f64` | 64-bit IEEE float — the same type as `float(11, 52)`     |
| `float(E, M)` | the float family: 1 sign, E exponent, M mantissa |
|       | bits. Non-native instances (`float(4, 3)` = fp8 e4m3,    |
|       | `float(5, 10)` = fp16, `float(8, 7)` = bf16; E<=8, M<=24)|
|       | are storage formats with full arithmetic (see below)     |
| `int` | abstract integer — resolved to a concrete width by the   |
|       | target's replacement policy (see *Abstract numeric types*) |
| `float` | abstract float — resolved like `int` (`--float=f32|f64`) |
| `scalar` | abstract scalar, parent of float and rational — resolved |
|       | to a concrete float or to `$rat` (`--scalar=f32|f64|rat`)  |
| `half`  | abstract integers resolving to HALF the `int` policy's   |
| `uhalf` | width — for stating width *relationships* (see below)    |

## Structure

```
fn name(a: i64, b: ptr) -> i64 {
^entry:
    ...
^next(x: i64):
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

### Arithmetic and bitwise ops

Both operands and the result must share one type. There is **one opcode
per operation**: the type decides everything — `add` on integers is a
two's-complement add (wrapping at 2^N), on floats an IEEE add; `div` is
signed for iN, unsigned for uN, floating for floats. The bitwise ops
and `rem` are integer-only; shift amounts are taken mod N.

```
v: i64 = add a, b        ; integer add — the same opcode is fadd on floats
v: f64 = mul x, y        ; float multiply, because the type is f64
v: i64 = div a, b        ; signed (iN) / unsigned (uN) / float (fN)
v: i64 = rem a, b
v: u64 = and a, b        ; also or, xor
v: u64 = shl a, b        ; shr: zero-fill for uN, sign-fill for iN
```

(The legacy spellings `iadd`/`fadd`/`isub`/`fsub`/`imul`/`fmul`/`fdiv`
remain accepted.)

Odd widths lower to 64-bit container code before emission: unsigned
values live zero-extended, signed values sign-extended — so callers pass
and receive natural values (-6 as an `i5` argument is just -6).

### Float constants

Float literals require a decimal point (or exponent); `fconst` also
accepts integers.

```
v: f64 = fconst 2.5
```

### Comparison

Operands must share an integer type (or `ptr`, which compares unsigned);
the result is `u1`.

```
c: u1 = icmp.eq a, b     ; also: ne
c: u1 = icmp.lt a, b     ; lt le gt ge — signedness from the operands
```

### Width changes

The source and result types determine the conversion; the opcode only picks
how new bits are filled.

```
v: i64 = ext a            ; widen; the fill follows the SOURCE's sign
v: i32 = trunc a          ; truncate (sign-agnostic: keeps low bits)
v: u64 = bitcast a        ; same-width reinterpretation (sign flip,
                            ; int<->float, int<->struct)
```

`ptr` takes no part in width changes.

Float comparisons are *ordered* (false when either side is NaN), except
`une`, which is true on NaN:

```
c: u1 = fcmp.oeq a, b    ; also: une olt ole ogt oge
```

Float conversions carry direction and signedness in the opcode; widths
come from the value types as usual:

```
f: f64 = itof n           ; int -> float; signedness from the int type
n: i64 = ftoi f           ; float -> int, rounds toward zero
d: f64 = fpromote s       ; f32 -> f64
s: f32 = fdemote d        ; f64 -> f32
```

The int side of `itof`/`ftoi` must be 32 or 64 bits wide.

### Memory

The address operand must be `ptr`. The access width is the result type (loads)
or the stored value's type (stores): `i32`, `i64`, or `ptr` (64 bits).

```
v: i64 = load addr
store v, addr
p: ptr = ptradd base, off    ; base: ptr, off: i64
```

### Structs

`type $name = { field: iN, ... }` declares a packed bitfield struct at
module level: fields are integer-width types (or abstract — `int`,
`uint`, `half`, `uhalf` — resolved by the policy), declared
**low-first**: the first field occupies the low bits, the same
convention as vector lane 0 and as C layout viewed little-endian.

A struct up to 64 bits travels in one register, and `bitcast` converts
it to and from any equal-width scalar. Larger structs (to 8 words / 512
bits) pack C-like — a field never straddles a 64-bit word; one that
would cross starts the next word — and lower by *value splitting*: one
value per word, expanded through params, calls, returns, and branches,
with no memory involved (they cannot be `bitcast` as a whole; move
fields or words). Field access is by name either way:

```
type $fp = { frac: u52, exp: u11, sign: u1 }

p: $fp = bitcast x            ; x: f64 — same 64 bits, structured view
e: i11 = extract p, exp       ; read a field
q: $fp = insert p, frac, f2  ; copy with one field replaced
r: $fp = pack s, e, f       ; build from all fields, in order
```

Structs lower to shift/mask code on their carrier integer before emission
(whole-width identities cost nothing), so they exist only at SSA level.
They cannot be loaded or stored directly; move them as their bitcast
scalar.

### Vectors

`i16x4`, `f32x2`, `u8x8` — N lanes of a scalar element type (iN/uN/f32),
lane 0 in the low bits, total width at most 64 for now (the SIMD tier
will lift this to 128). Vectors add **no arithmetic opcodes**: because
types live on variables, the ordinary opcodes are elementwise when their
type is a vector — lanes wrap, divide, shift, and take their signedness
independently, per the element type:

```
s: i16x4 = iadd a, b         ; four independent 16-bit adds
q: i32x2 = div a, d          ; signed lanes: signed divides
```

Lane access reuses the struct ops with an integer lane index in place of
a field name; `pack` takes lanes in order, lane 0 first. `bitcast`
converts a vector to and from any equal-width scalar.

```
x: f32 = extract v, 0
w: u16x4 = insert v, 2, n
v: i16x2 = pack a0, a1
```

Two lowerings exist. The portable tier scalarizes: each vector type
becomes a packed struct, each elementwise op per-lane code, and struct
lowering finishes the job — every backend runs vectors with no new
machine instructions. On arm64 the emitter instead keeps vectors whole
and uses probe-learned NEON encodings (`add.4h`, `fadd.2s`, lane
`ins`/`umov`/`smov`, `dup`): one elementwise op is one instruction, and
vector values live in d registers. A function containing a vector op
NEON cannot express (integer div/rem, odd lane widths) falls back to
scalarization — body only: signatures keep their vector types, so NEON
and scalarized functions call each other freely (vectors always travel
in d registers). The two tiers are verified against each other by the
same suite across backends. Vector comparisons, splat/reduce sugar, and
memory access are future work; until then comparisons and reductions
are written per-lane, and vectors move through memory as their bitcast
scalar.

### Calls

Callees are named symbols — and the `@` sigil alone marks a call, so no
keyword is needed. Signatures are checked against the module if the
function is defined here, taken on trust if external.

```
v: i64 = f(a, b)                ; call with one result
q: i64, r: i64 = divmod(a, b)  ; call with two results
g(a)                              ; call with results ignored (or none)
```

A call binds either *all* of the callee's return values or *none* of
them (a call is the only form that may define more than one value).
The legacy `call f(...)` spelling is still accepted.

### Terminators

Branch arguments must match the target block's parameters in count and type.

```
jmp ^next(a, b)
br c, ^then(a), ^else()   ; c: i1 — empty parens may be omitted
ret v                      ; one return value
ret q, r                  ; multiple return values
ret                         ; none
```

## Example

```
; sum of 0..n
fn sum(n: i64) -> i64 {
^entry:
    zero: i64 = iconst 0
    jmp ^loop(zero, zero)
^loop(i: i64, acc: i64):
    done: i1 = icmp.sge i, n
    br done, ^exit, ^body
^body:
    acc2: i64 = iadd acc, i
    one:  i64 = iconst 1
    %i2:   i64 = iadd i, one
    jmp ^loop(%i2, acc2)
^exit:
    ret acc
}
```

## Abstract numeric types

**Abstract types are the house style**: original SSA code should say
`int`, `uint`, and `float` unless it genuinely means a specific layout.
Concrete types are for width-specific work — bit patterns, struct fields,
memory layout, code exact only at one width. Abstract code is
policy-portable by construction, and the suite enforces it: the same
programs run under every width policy and must agree.

An abstract type does not choose a width — the compiler does, at compile
time, by a *replacement policy* derived from the target (its natural
register width, or a size-oriented choice like i32 on wasm32) and from
user concerns (`--int=i32|i64`, `--float=f32|f64`; `uint` follows `int`'s
width). Because types live on variables, resolution is a single rewrite
of the value tables before verification; opcodes, instructions, and
everything downstream see only concrete types.

### half and uhalf: width relationships

Some code depends not on a width but on a *ratio* of widths: exact
rational arithmetic is correct only when intermediates are twice the
component width (cross products of w-bit values need 2w bits). `half`
and `uhalf` resolve to half the `int` policy's width, so a struct of
`half` fields with `int` intermediates keeps that invariant true under
every policy — under `int=i64` the rational library computes with
i32/u32 components and i64 math; under `int=i32`, i16/u16 components
and i32 math. Struct fields may be abstract (`int`, `uint`, `half`,
`uhalf`); the resolved layout must still fit the 64-bit carrier, which
the verifier checks after resolution. Width-agnostic range checks are
written as trunc/ext round-trips (narrow, re-widen, compare) instead of
magic constants.

```
fn gcd(a: int, b: int) -> int {     ; width chosen per target/policy
    ...
    r: int = srem x, y              ; same ops, abstractly typed
```

- Abstract and concrete types mix freely (`i1` conditions, `ptr`
  addresses, explicit `i32`/`i64` where a width is required).
- A width-change cast (`sext`/`zext`/`trunc`) between `int` and a concrete
  type is only valid under policies where the widths actually differ — the
  verifier checks the resolved program, so such code ties itself to a
  policy. Policy-portable code keeps casts among concrete types.
- Memory keeps concrete types in portable code: a load of `int` changes
  access width with the policy.
- `float` resolves the same way (`f64` by default on every target;
  `--float=f32` for size). Policy-portable abstract-float code sticks to
  values exact in both widths.

### scalar

`scalar` sits above `float`: it abstracts not just the width but the
*representation*. Scalar code is written with the float opcodes
(`fconst`, `fadd`..`fdiv`, `fcmp.*`, `itof`, `ftoi`), and the policy
decides what they mean:

- `--scalar=f64` / `--scalar=f32` (default: follows `float`): pure type
  substitution, exactly like `float`.
- `--scalar=rat`: scalar values become the rational library's `$rat`
  struct (`{ num: i32, den: u32 }`, lib/rational.ssa — linked in
  textually), and a pass rewrites the float opcodes into library calls:
  `fadd` -> `rat_add`, `fcmp.olt` -> `rat_lt`, `itof n` ->
  `rat_make(n, 1)`, `ftoi` -> `rat_to_int`, and `fconst` becomes the
  exact `num/den` pair (every finite float is a dyadic rational; a
  constant too precise to fit is a compile-time error). NaN-adjacent
  behavior maps to NaR (`den == 0`): ordered comparisons false, `une`
  true.

Portable scalar code keeps to values exact in every implementation —
dyadic constants, integer entry and exit — the same discipline abstract
`float` already has across widths. A future fixed-point implementation
joins the same seam: struct layout plus an op library, selected by
`--scalar=fx8.8`-style policies.

## Literal operands and expressions

Two pieces of parse-time sugar keep authorship cheap without touching
the instruction set. Both are one-way, like structured control flow:
the printer prints flat form.

**Literal operands.** Anywhere an arithmetic operand, comparison
operand, or loop initializer is expected, a literal may stand in for a
value; the parser synthesizes the `iconst`/`fconst` (`c1`, `c2`, ...)
just before the use. The literal's type comes from the result (for
arithmetic), the other operand (for comparisons), or the declared
variable (for loop inits) — a comparison of two literals is an error,
since nothing fixes the width. An integer literal in a float position
becomes a float constant, as with `fconst`.

```
%i2: int = iadd i, 1
done: u1 = icmp.ge i, 4
s: scalar = loop(i: int = 0, acc: scalar = 0.0) { ...
k: i64 = 42                    ; a bare literal is iconst/fconst
```

**Expressions.** A definition whose right-hand side is not an opcode is
a pure arithmetic expression over values and literals, with C precedence
(`|` lowest, then `^`, `&`, `<< >>`, `+ -`, `* / %`) and parentheses.
Every node has the declared result type, and the opcode family follows
from it — the types-on-variables rule applied to sugar: `/` is signed or
unsigned division by the type, `+` is `iadd` or `fadd`, and on a vector
type each operation is elementwise. The tree desugars to ordinary flat
instructions at parse time.

```
p: scalar = 2.0 * xf * xf - 3.0 * xf + 1.0
r: uint = x >> 5 | 0x80
t: i64 = a * dgi + c * bgi
```

**Comparisons.** The comparison operators `< <= > >= == !=` form a
single, non-associative level above the arithmetic ones, usable at the
root of a u1-typed definition and directly as an `if` condition. The
operand type comes from whichever side names a value (so one side may
be a literal), and picks `icmp` or ordered `fcmp` — with signedness, as
ever, from the type:

```
done: u1 = i >= n
odd: u1 = n & 1 == 1
if x > hi { ret hi }
```

**Statement-position sugar.** `break`, `continue`, `yield`, and `ret`
accept literals, typed positionally by what they feed (loop variables,
bound results, the function's return types). `ret call f(...)` returns
a call's results directly. A block containing one short statement can
close on the same line — together these make a guard a single line:

```
if xnar { ret call rat_nar() }
if y == 0 { break x }
loop(i: int = 0, acc: scalar = 0.0) { ...
```

**Call sugar.** Parsing is two-phase (all signatures are read before
any body), so calls compose with the rest: literal arguments take the
callee's parameter types (`clamp(x, 0, 100)`); a single-result call
is an expression atom (`r: u1 = f(x) == 0`) and an `if` condition
(`if rat_is_nar(x) { ret rat_nar() }`); and
`break`/`continue`/`yield`/`ret` operands are full expressions
(`ret x / 2`, `ret sq(x) + 1`).

There is deliberately no bare-copy form (`v: ty = x` is an error —
SSA has no copy opcode) and no unary minus on values (write `0 - x`).

### The small-float menagerie

`float(E, M)` names the whole IEEE-style family, and `f32`/`f64` are
its two native members (the parser canonicalizes `float(8, 23)` to
`f32`). Every other instance in range (E 2..=8, M 1..=24) is a
first-class arithmetic type: `fadd`/`fmul`/comparisons/`fconst`/
`itof`/`ftoi` all work, `fpromote`/`fdemote` climb between any two
formats (wider-or-equal in both parameters), and `bitcast` crosses to
the u(E+M+1) bit pattern.

Arithmetic lowers to promote -> native f64 op -> demote with round to
nearest even — correctly rounded in one step for every M <= 24 by the
double-rounding theorem — using conversion functions instantiated from
the width-generic lib/float.ssa (subnormals exact; NaN payloads
collapse to the quiet NaN; the finite-only e4m3 "FN" variant is future
work). Constants demote at compile time. On FPU-less targets the f64
ops soften in turn — fp8 arithmetic runs on an integer-only RISC-V
core. Verification is exhaustive where the formats are small: every
fp8 add and mul pair is checked bit-exact against an independent
reference in cargo test.

## Width-parametric types and functions

Types may take width parameters in round brackets, and any type
position accepts a **width expression** over parameters, integer
literals, `+ - * /`, and parens:

```
type $fp(E, M) = { frac: u(M), exp: u(E), sign: u1 }
type $rat(N)  = { num: i(N), den: u(N) }

fn mulwide(a: u(N), b: u(N)) -> u(2*N) {
    aw: u(2*N) = ext a
    ...
}
```

A struct instantiates by explicit arguments (`$fp(4, 3)` — the fp8
e4m3 layout; `$fp(5, 10)` is fp16, `$fp(11, 52)` is the double). A
function whose signature has free width parameters is **generic**: it
parses once as a template, and each call site infers the parameters
from the argument types (`sq(x)` with an `i16` argument instantiates
N=16) and monomorphizes on demand — instances are ordinary functions
and structs with mangled names (`sq__16`, `fp__4_3`), so the verifier,
lowering, and every backend see nothing new. Generics may call
generics; parameters propagate through inference.

The point is stating width **relationships**: `u(2*N)` in `mulwide`'s
signature makes "products need twice the width" checkable, the same
invariant `half`/`uhalf` hardcode at one ratio. Restrictions: a
parameter must be inferable from some argument position that names it
directly; literal arguments cannot drive inference; instantiated
widths must land in 1..=64 (and struct layouts within 8 words), checked
per instance with precise errors.

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
    done: i1 = icmp.sge i, n
    if done {
        break acc              ; exit the loop, yielding its results
    }
    ...
    continue %i2, acc2         ; back edge: new values for the loop vars
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
