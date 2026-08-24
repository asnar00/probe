# Understanding probe: a tour of the real code

This is a guided walk through the codebase as it actually is. Every
code block below is quoted verbatim from a file in this repo, or is the
actual output of a command you can run. Nothing is idealized.

The route: one small function, followed from source to silicon —
through the parser, into the library that implements its arithmetic,
down to the machine instructions it becomes on two different kinds of
target — and then the machinery that makes any of it believable.

## 0. The map

```
src/*.rs            the compiler (~17k lines of Rust)
  ssa.rs              the language: lexer, parser, generics, verifier
  lower.rs            lowering: wide structs, vectors, structs, widths
  scalar.rs           representation policies: rationals, small floats
  softfloat.rs        the (older, hand-written) f64/f32 soft-float runtime
  opt.rs              the SSA pass pipeline (levels are prefixes of it)
  regalloc.rs         class-aware linear scan, coalescing
  emit.rs / emit_rv.rs / emit_wasm.rs   the three backends
  learn.rs / wlearn.rs / oracle.rs      the encoding learners
lib/float.ssa       the float family + group softfloat (this tour's star)
lib/rational.ssa    exact fractions, same recipe
suite/*.ssa         ~240 test cases as ;! directives, run on 4 machines
targets/*.probe     probe seeds; *.encodings.json = learned, verified
```

## 1. A real function

From `suite/menagerie.ssa` — it adds two fp8 numbers, taking and
returning their bit patterns (the test harness passes 64-bit integers):

```
fn f8add(a: u64, b: u64) -> u64 {
    pa: u8 = trunc a
    pb: u8 = trunc b
    xa: float(4, 3) = bitcast pa
    xb: float(4, 3) = bitcast pb
    s: float(4, 3) = xa + xb
    c: u8 = bitcast s
    r: u64 = ext c
    ret r
}
```

Language basics, in three lines: `name: type = ...` defines a value
(once — this is SSA, values never change); `name(...)` calls a
function; and the *types* carry the meaning — there is one `+`, and
here it is a float add because its operands are `float(4, 3)`.

`float(E, M)` is the whole float family: 1 sign bit, E exponent bits,
M mantissa bits. `float(4, 3)` is fp8 (e4m3), `float(5, 10)` is fp16,
`float(8, 7)` is bf16 — and `float(8, 23)` **is** `f32`, the same
type; the parser canonicalizes it. The familiar floats are just two
members of the family.

`trunc`/`ext` narrow and widen integers; `bitcast` reinterprets bits
without changing them. Conversions are always written.

## 2. What the parser does

The surface syntax — expressions, `+`, dot access — is sugar that
disappears at parse time. `cargo run -- parse suite/menagerie.ssa`
prints what everything downstream actually sees; here is `f8add` from
that output:

```
fn f8add(a: u64, b: u64) -> u64 {
^entry:
    pa: u8 = trunc a
    pb: u8 = trunc b
    xa: float(4, 3) = bitcast pa
    xb: float(4, 3) = bitcast pb
    s: float(4, 3) = add xa, xb
    c: u8 = bitcast s
    r: u64 = ext c
    ret r
}
```

One instruction per line, explicit blocks (`^entry:`), no expressions
— the assembler-shaped core. Control-flow sugar (`if`/`loop`) lowers
the same way, into a flat graph of blocks. The sugar is one-way: it
exists for authors, and the verifier, optimizer, and backends never
see it.

## 3. Where the add comes from: group softfloat

No machine has an fp8 add instruction, so where does `add xa, xb` go?
To `lib/float.ssa`, which defines the operation *once*, for every
format at the same time, in a group that marks it as the default
implementation:

```
group softfloat {
```

```
fn add(a: float(E, M), b: float(E, M)) -> float(E, M) {
    ; unpack: a float's fields are directly accessible, at their own
    ; widths
    sa: u1 = a.sign
    sb: u1 = b.sign
    ea: u(E) = a.exp
    eb: u(E) = b.exp
    fa: u(M) = a.frac
    fb: u(M) = b.frac
```

`E` and `M` are width parameters. A float's fields — sign, exponent,
fraction — read straight off the value (`a.exp`), each at its own
width: the exponent is a `u(E)`, an E-bit unsigned integer. When you
use `float(4, 3)`, the compiler stamps out this function with E=4,
M=3 (it appears in compiled output as `softfloat_add__4_3`).

Two passages further down show the algorithm's character. Magnitude
comparison is free, because of how IEEE chose the layout:

```
    ; order so x has the larger magnitude — IEEE layouts compare as
    ; plain integers
    eaw: u(E+M) = ext ea
    ebw: u(E+M) = ext eb
    faw: u(E+M) = ext fa
    fbw: u(E+M) = ext fb
    maga: u(E+M) = eaw << M | faw
    magb: u(E+M) = ebw << M | fbw
    swap: u1 = maga < magb
```

and the add itself is two cases, chosen by sign agreement:

```
    ; effective add or subtract, by sign agreement
    ms0: u(M+5), rs: u1 = if xs == ys {
        msum: u(M+5) = mx3 + my3
        yield msum, xs
    } else {
        mdiff: u(M+5) = mx3 - my3
        yield mdiff, xs
    }
```

Notice the widths. `u(M+5)` is the significand with its implicit bit
and three rounding bits — its *true* size, stated as arithmetic over
the parameters. Nothing in this file is a `u64`: products are
`u(max(2*M+2, M+5) + 1)`, signed exponent math is `i(max(E, 8) + 2)`.
The code assumes nothing about the machine's word size; fitting these
widths onto a target's words is the width-lowering pass's job (64-bit
words today). A side effect worth knowing: the verifier's width bound
becomes each operation's honest limit — `add` requires M+5 ≤ 64 (so it
covers f64), `mul` requires 2M+2 ≤ 64 (f32 and below).

The rest of `add` — alignment with guard/round/sticky bits, subnormal
handling, renormalization after carries and cancellation, round to
nearest even — is about a hundred more lines of the same kind. `sub`
is five lines (flip b's sign, call `add`). Read the file; it reads
like the algorithm.

## 4. The platform rule, in actual machine code

The selection rule: **if the target has a native instruction for the
format, use it; otherwise fall through to group softfloat.** Here is
that rule in the emitted arm64 code for two adds, disassembled
(`f32` first):

```
	fadd	s10, s8, s9
```

One instruction — the learned `fadd` encoding. And `float(4, 3)`:

```
	bl	#3208
```

A call — into `softfloat_add__4_3`, the E=4, M=3 instance of the
function you just read, compiled to pure integer code. Same source
shape, two mechanisms, one rule.

The `fadd` encoding itself was never copied from a manual. From
`targets/arm64.encodings.json`, learned by probing an assembler and
verified against it:

```
{"template": "fadd {s}, {s}, {s}", "fixed": "0x1e202800", "verified": 65,
 "rejected": 0, "fields": [
  {"slot": "{s}", "kind": "linear", "signed": false, "bits": [0, 1, 2, 3, 4]},
  {"slot": "{s}", "kind": "linear", "signed": false, "bits": [5, 6, 7, 8, 9]},
  {"slot": "{s}", "kind": "linear", "signed": false, "bits": [16, 17, 18, 19, 20]}]}
```

## 5. Why any of this is believable

The project's rule is that nothing is trusted without an independent
referee, and floats get three kinds:

- **Exhaustion.** fp8 has 256 values, so `cargo test` checks *every*
  add and mul pair — 131,072 cases — against a separate Rust
  implementation that computes the answers a different way
  (`fp8_exhaustive_add_mul` in `src/scalar.rs`).
- **Silicon.** The same generic `add`, instantiated at (8, 23) — the
  f32 shape — is compared against this machine's actual FPU over
  hundreds of thousands of cases, including subnormal boundaries,
  cancellation ladders, and overflow (`f32_diy_vs_fpu`). The
  hand-built integer version and the hardware agree bit for bit —
  which is what makes "use the native instruction" a pure
  optimization.
- **Four machines.** Expected answers live next to the code as `;!`
  directives — from `suite/menagerie.ssa`:

```
;! f8add 0x38 0x40 -> 0x44
;! h_third -> 0x3555
```

  (fp8: 1.0 + 2.0 must give exactly 3.0; the constant 1/3 must round
  into fp16 as 0x3555.) The same directives run natively, in
  WebAssembly, and on emulated RISC-V and ARM:

```sh
cargo run -- test
cargo run -- test wasm
cargo run -- test riscv
cargo run -- test arm-qemu
cargo run -- --softfloat test    # all float hardware forbidden
```

That last flag is the platform rule pushed to its limit: with no FPU
at all, the fp8 dot products still run, still bit-exact, as pure
integer code.

## 6. The same recipe, different numbers

`lib/rational.ssa` is exact-fraction arithmetic built the same way —
a struct and a library, no compiler support:

```
type $rat = { num: half, den: uhalf }
```

`half`/`uhalf` resolve to half the machine word, because a product
needs twice the bits of its factors — the same width-relationship idea
as `u(2*M+2)`, fixed at one ratio. `1/2 + 1/3` is exactly `5/6`; the
suite demands `1 + 1/2 + 1/3 + 1/4 -> 25, 12`, exactly; unrepresentable
results become a NaN-like "not a rational" marker instead of rounding.
And `--scalar=rat` runs float-looking programs in exact fractions.

## 7. Where to read next

- `lib/float.ssa` — this tour's subject, whole. Start at `group
  softfloat`.
- `suite/menagerie.ssa`, `suite/generic.ssa` — the family exercised;
  `suite/wide.ssa` — 128-bit integers via multi-word structs.
- `ssa.md` — the full language reference (types, sugar, generics,
  groups).
- `future-work.md` — where this is heading: the platform model
  (instruction selection as overload resolution, with `emit`-bodied
  platform functions shadowing groups like softfloat).

And the habit that holds it together: every claim above is a test you
can run, and "it works" always means "independent things agree, bit
for bit."
