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
  definition is written `%name: ty = op ...`, and a type like `u5` or `i32`
  carries both width and signedness. `div`, `rem`, `shr`, ordered `icmp`,
  `ext`, `itof`/`ftoi` all take their behavior from their operand types, so
  there is exactly one opcode per operation and the data layout states the
  intent once.
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

| type  | meaning                                                  |
|-------|----------------------------------------------------------|
| `i1`  | boolean (result of `icmp`)                               |
| `i32` | 32-bit integer                                           |
| `i64` | 64-bit integer                                           |
| `ptr` | pointer (64-bit on our target)                           |
| `iN`  | any-width integer, N in 2..=63 (e.g. `i5`, `i52`)        |
| `$name` | packed bitfield struct (see *Structs*), <= 64 bits     |
| `f32` | 32-bit IEEE float                                        |
| `f64` | 64-bit IEEE float                                        |
| `int` | abstract integer — resolved to a concrete width by the   |
|       | target's replacement policy (see *Abstract numeric types*) |
| `float` | abstract float — resolved like `int` (`--float=f32|f64`) |

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

Both operands and the result must share one integer type (width >= 2).
Values wrap at 2^N; shift amounts are taken mod N. Where signedness
matters, the type decides — one opcode each:

```
%v: i64 = iadd %a, %b       ; two's-complement: sign-agnostic
%v: i64 = isub %a, %b
%v: i64 = imul %a, %b
%v: i64 = div %a, %b        ; signed for iN operands, unsigned for uN
%v: i64 = rem %a, %b        ; likewise
%v: i64 = and %a, %b
%v: u64 = or  %a, %b
%v: u64 = xor %a, %b
%v: u64 = shl %a, %b        ; sign-agnostic
%v: u64 = shr %a, %b        ; zero-fill for uN, sign-fill for iN
```

Odd widths lower to 64-bit container code before emission: unsigned
values live zero-extended, signed values sign-extended — so callers pass
and receive natural values (-6 as an `i5` argument is just -6).

### Float arithmetic

Both operands and the result must share a float type. Float literals
require a decimal point (or exponent); `fconst` also accepts integers.

```
%v: f64 = fconst 2.5
%v: f64 = fadd %a, %b       ; also fsub, fmul, fdiv
```

### Comparison

Operands must share an integer type (or `ptr`, which compares unsigned);
the result is `u1`.

```
%c: u1 = icmp.eq %a, %b     ; also: ne
%c: u1 = icmp.lt %a, %b     ; lt le gt ge — signedness from the operands
```

### Width changes

The source and result types determine the conversion; the opcode only picks
how new bits are filled.

```
%v: i64 = ext %a            ; widen; the fill follows the SOURCE's sign
%v: i32 = trunc %a          ; truncate (sign-agnostic: keeps low bits)
%v: u64 = bitcast %a        ; same-width reinterpretation (sign flip,
                            ; int<->float, int<->struct)
```

`ptr` takes no part in width changes.

Float comparisons are *ordered* (false when either side is NaN), except
`une`, which is true on NaN:

```
%c: u1 = fcmp.oeq %a, %b    ; also: une olt ole ogt oge
```

Float conversions carry direction and signedness in the opcode; widths
come from the value types as usual:

```
%f: f64 = itof %n           ; int -> float; signedness from the int type
%n: i64 = ftoi %f           ; float -> int, rounds toward zero
%d: f64 = fpromote %s       ; f32 -> f64
%s: f32 = fdemote %d        ; f64 -> f32
```

The int side of `itof`/`ftoi` must be 32 or 64 bits wide.

### Memory

The address operand must be `ptr`. The access width is the result type (loads)
or the stored value's type (stores): `i32`, `i64`, or `ptr` (64 bits).

```
%v: i64 = load %addr
store %v, %addr
%p: ptr = ptradd %base, %off    ; %base: ptr, %off: i64
```

### Structs

`type $name = { field: iN, ... }` declares a packed bitfield struct at
module level: fields are integer-width types, declared **MSB-first** (the
first field occupies the top bits), total width at most 64. A struct value
travels in one register; `bitcast` converts it to and from any equal-width
scalar. Field access is by name:

```
type $fp = { sign: i1, exp: i11, frac: i52 }

%p: $fp = bitcast %x            ; x: f64 — same 64 bits, structured view
%e: i11 = extract %p, exp       ; read a field
%q: $fp = insert %p, frac, %f2  ; copy with one field replaced
%r: $fp = pack %s, %e, %f       ; build from all fields, in order
```

Structs lower to shift/mask code on their carrier integer before emission
(whole-width identities cost nothing), so they exist only at SSA level.
They cannot be loaded or stored directly; move them as their bitcast
scalar.

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

## Abstract numeric types

`int` is an **abstract integer type**: code written with it does not choose
a width — the compiler does, at compile time, by a *replacement policy*
derived from the target (its natural register width, or a size-oriented
choice like i32 on wasm32) and from user concerns (`--int=i32|i64`).
Because types live on variables, resolution is a single rewrite of the
value tables before verification; opcodes, instructions, and everything
downstream see only concrete types.

```
fn @gcd(%a: int, %b: int) -> int {     ; width chosen per target/policy
    ...
    %r: int = srem %x, %y              ; same ops, abstractly typed
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
