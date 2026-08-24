# A tour of probe, by way of rational numbers

This is a walk through `lib/rational.ssa` — an exact-arithmetic number
library written in probe's SSA language. It's a good front door to the
whole project, because it demonstrates the central claim: **new kinds of
numbers are libraries, not compiler features**. There is not one line of
Rust behind rational arithmetic. There's a struct, some functions, and a
test file — and by the end of this tour, `1 + 1/2 + 1/3 + 1/4` computes
to exactly `25/12` on four different machines.

You don't need to have read anything else. Where the language does
something unusual, we'll stop and look at it.

## 1. A number is a struct

```
type $rat = { num: half, den: uhalf }
```

That's the entire type definition: a packed struct — a signed numerator
and an unsigned denominator. Three things are worth noticing already.

First, **signedness lives in the type, not in the operations**. probe
has one `div`, one `%`, one `<` — each does signed or unsigned (or
float) arithmetic depending on the *types of its operands*. So the
declaration above isn't just layout; it's semantics. Dividing a `num`
by something is signed division, dividing a `den` is unsigned, and
nobody ever writes `udiv` or `sdiv` because the data already said which
one is meant.

Second, the widths are *abstract*. `half` and `uhalf` resolve at
compile time to half the width of the platform's `int` policy: build
with `--int=i64` (the default) and a `$rat` is i32/u32; build with
`--int=i32` and it's i16/u16. The library doesn't know its own size —
you'll see below how it's written so the *arithmetic stays correct at
any width*, which is why "half" is the right abstraction and not just
a convenience. (It's also a fixed special case of something fully
general — width-parametric types like `$fp(E, M)` — which section 7
gets to.)

Third, this struct fits in a register. probe lowers packed structs to
shift-and-mask code on an ordinary integer, so a `$rat` value travels
through the program (and through function calls) like any other value.
No memory, no pointers, no ABI ceremony.

We adopt two conventions on top of the layout:

- **Canonical form**: `den > 0`, `gcd(|num|, den) = 1`, sign carried by
  the numerator. Every function returns canonical values. This buys us
  something lovely later: equality is just bit-comparison.
- **NaR** ("not a rational"): `den == 0` is the error value. It behaves
  like NaN — it propagates through arithmetic and compares false — and
  it's what you get when a result can't be represented. The semantics
  are *exact-or-NaR*: this library never silently rounds.

## 2. The workhorse: gcd

```
fn @rat_gcd(%a: uint, %b: uint) -> uint {
    %g: uint = loop(%x: uint = %a, %y: uint = %b) {
        if %y == 0 { break %x }
        %r: uint = %x % %y
        continue %y, %r
    }
    ret %g
}
```

This is Euclid's algorithm, and it introduces most of the language in
eight lines:

- **Every value is named and typed.** `%r: uint = %x % %y` declares a
  new value, states its type, and computes it. Values are assigned
  exactly once — this is SSA (static single assignment) — so a name
  means the same thing everywhere it appears. (`uint` is the abstract
  unsigned word — u64 by default, u32 under `--int=i32`.)
- **The loop declares its own state.** `loop(%x: uint = %a, ...)` lists
  the *loop-carried variables* with their initial values. `continue`
  supplies the next iteration's values; `break` exits, yielding the
  loop's result, which is bound on the left (`%g`). There is no
  mutation anywhere — the loop header is the complete list of what
  changes per iteration, which is exactly the thing you want to know
  when reading a loop.
- **`%x % %y` is unsigned remainder** — because `%x` is `uint`. Same
  opcode, signed, for an `int`. The one-line guard
  `if %y == 0 { break %x }` compares, branches, and exits in a form
  that reads like the algorithm.

Under the hood this parses (one-way) into a flat graph of basic blocks
with explicit branches — the form the verifier checks and the backends
consume. The sugar you're reading never survives past the parser.

## 3. Making a rational: normalize, reduce, or refuse

Values enter through `rat_make(n, d)`, which fixes signs and delegates
to `rat_fit` — the function that enforces the canonical form:

```
fn @rat_make(%n0: int, %d0: int) -> $rat {
    if %d0 == 0 { ret @rat_nar() }
    %dneg: u1 = %d0 < 0
    %n: int, %d: int = if %dneg {
        %nn: int = 0 - %n0
        %nd: int = 0 - %d0
        yield %nn, %nd
    } else {
        yield %n0, %d0
    }
    %du: uint = bitcast %d
    ret @rat_fit(%n, %du)
}
```

New pieces here:

- **`if` is an expression.** The value-yielding form binds results on
  the left (`%n: int, %d: int = if ...`) and each arm ends with
  `yield`. Notice it yields *two* values — multiple results are
  ordinary in probe, for `if`, for `loop`, and for functions.
- **`bitcast` reinterprets bits** between equal-width types — here
  `int` to `uint` after we've made the value positive, so that
  everything downstream does unsigned arithmetic. Casts are always
  written; nothing converts silently.
- **`ret @rat_fit(...)`** returns another function's results
  directly. And the first line is the library's signature move, the
  one-line guard:

  ```
  if %d0 == 0 { ret @rat_nar() }
  ```

Inside `rat_fit`, after dividing out the gcd, the range check shows the
exact-or-NaR policy in action — and it's width-blind. "Does this value
fit the field?" is answered by narrowing and re-widening:

```
%n32: half = trunc %nsig
%nback: int = ext %n32
if %nback != %nsig { ret @rat_nar() }
```

If the round trip changes the value, it didn't fit; the answer isn't a
rounded lie — it's NaR, and every later operation will faithfully pass
it along. Notice there's no `0x7fffffff` anywhere: the library contains
no width as a constant, which is what lets the policy choose the width.

## 4. Arithmetic that can't overflow (mostly)

Addition is the interesting one. The naive formula
`a/b + c/d = (ad + cb)/bd` overflows its intermediates easily, so the
library uses the classic pre-reduction (Knuth's trick): divide both
denominators by their gcd first.

```
fn @rat_add(%x: $rat, %y: $rat) -> $rat {
    if @rat_is_nar(%x) { ret @rat_nar() }
    if @rat_is_nar(%y) { ret @rat_nar() }
    %a: int, %b: uint = @rat_parts(%x)
    %c: int, %d: uint = @rat_parts(%y)
    %g: uint = @rat_gcd(%b, %d)
    %bg: uint = %b / %g
    %dg: uint = %d / %g
    %bgi: int = bitcast %bg
    %dgi: int = bitcast %dg
    %t: int = %a * %dgi + %c * %bgi
    %den: uint = %bg * %d
    ret @rat_fit(%t, %den)
}
```

Read it top to bottom: two NaR guards, unpack both operands
(`rat_parts` returns a pair — multiple return values again), one gcd,
the cross-multiply as a single expression, and hand the un-reduced
result to `rat_fit` to finish.

And here is why `half` was the right abstraction: **cross products of
w-bit values need 2w bits.** Because the fields are half a word and the
intermediates (`int`, `uint`) are a whole word, that relationship holds
at *every* policy — i32 fields with i64 math, or i16 fields with i32
math. Had the fields been `int`, the library's own arithmetic would
overflow. The struct declaration encodes the correctness argument.
(The one truly extreme corner — both cross products near the top of the
word — is documented scope, in the same spirit as "tier 1 softfloat
flushes subnormals".)

Comparison exploits canonicity twice. Ordering cross-multiplies
(denominators are positive, so no sign flips):

```
%r: u1 = %a * %di < %c * %bi
```

— and equality doesn't compute anything at all:

```
%xb: uint = bitcast %x
%yb: uint = bitcast %y
%r: u1 = %xb == %yb
```

(A `$rat` is two half-words — exactly one word — so it bitcasts to
`uint` under any policy.)

Canonical forms are unique, so equal rationals are equal *bit patterns*.
That's the payoff for making every function reduce.

## 5. How you know any of this is true

probe's culture is that nothing is believed until something independent
agrees. The rational tests live in `suite/rational.ssa` as *directives*
— executable expectations written next to the code:

```
;! t_add 1 2 1 3 -> 5, 6          ; 1/2 + 1/3 = 5/6
;! t_make 6 -8 -> -3, 4           ; normalization: 6/-8 = -3/4
;! t_div 2 3 -4 5 -> -5, 6
;! t_nar 1 0 -> 1                 ; 1/0 is NaR
;! t_harmonic 4 -> 25, 12         ; 1 + 1/2 + 1/3 + 1/4, exactly
```

Run them:

```sh
cargo run -- test              # native arm64 (JIT, in-process)
cargo run -- test wasm         # compiled to WebAssembly, run by node
cargo run -- test riscv        # bare-metal RISC-V under qemu
cargo run -- test arm-qemu     # arm64 again, under qemu as a second referee
cargo run -- --int=i32 test    # the same library as 16-bit rationals
```

The same source, the same directives, four independent execution paths
— machine code emitted from instruction encodings that were themselves
*learned* by probing an assembler and verified against it (that's the
"probe" in probe, and a story for another tour). If the library had a
bug that one backend happened to mask, another would catch it.

## 6. The twist: your code may already be using this

probe has abstract numeric types. Code written against `scalar` doesn't
choose a representation — a compile-time policy does:

```
fn @s_lerp(%a: int, %b: int) -> int {
    %af: scalar = itof %a
    %bf: scalar = itof %b
    %m: scalar = %af + (%bf - %af) * 0.5
    %r: int = ftoi %m
    ret %r
}
```

```sh
cargo run -- run suite/scalar.ssa ...            # scalar = f64
cargo run -- --scalar=f32 test                   # scalar = f32
cargo run -- --scalar=rat test                   # scalar = EXACT RATIONALS
```

Under `--scalar=rat`, that `+` becomes a call to `@rat_add`, the `0.5`
becomes the exact pair `1/2`, comparisons become `@rat_lt`, and the
whole suite must produce identical answers to the floating-point runs.
The library you just read *is the arithmetic* — plugged in underneath
unchanged programs by a rewrite pass, exactly the way software floating
point is plugged in for CPUs with no FPU. (Yes, you can stack them:
`--scalar=rat --softfloat --int=i32` computes rationals whose
float-conversion path is itself software floats, on a 32-bit integer
policy. It passes.)

## 7. Widths as parameters: the same idea, fully general

`half` fixed one ratio into the type system. The general mechanism is
**width-parametric types and functions**: parameters in round brackets,
and width *expressions* in any type position.

```
type $fp(E, M) = { frac: u(M), exp: u(E), sign: u1 }

fn @mulwide(%a: u(N), %b: u(N)) -> u(2*N) {
    %aw: u(2*N) = ext %a
    %bw: u(2*N) = ext %b
    ret %aw * %bw
}
```

`$fp(4, 3)` instantiates the struct at concrete widths (that's the fp8
e4m3 layout; `$fp(5, 10)` is fp16, `$fp(11, 52)` is the IEEE double —
one declaration is the whole family). A function whose signature has
free parameters is generic: call `@mulwide` with two `u16` values and
the parser infers `N = 16`, stamping out an ordinary function behind
the scenes (monomorphization — the verifier and backends never know).
Look at that return type: `u(2*N)`. That's `rat_add`'s overflow
argument — *products need twice the width* — stated in a signature and
checked at every instantiation, rather than hoped about in a comment.

The float story completes the thought. `float(E, M)` names the float
family as arithmetic types, and the native ones are just its members:
`f32` **is** `float(8, 23)`, `f64` **is** `float(11, 52)`. The rest —
`float(4, 3)` = fp8, `float(5, 10)` = fp16, `float(8, 7)` = bf16 —
compute by promoting to f64, using the hardware, and demoting with
round-to-nearest-even:

```
fn @f8dot(%xa: float(4, 3), %xb: float(4, 3),
          %xc: float(4, 3), %xd: float(4, 3)) -> float(4, 3) {
    %s: float(4, 3) = %xa * %xb + %xc * %xd
    ret %s
}
```

The promote/demote conversions are themselves one width-generic SSA
library (`lib/float.ssa` — two functions for every format that will
ever exist), and because fp8 has only 256 values, its arithmetic is
verified *exhaustively*: every possible add and mul, bit-exact against
an independent reference. Small formats don't get sampled confidence;
they get total enumeration.

## 8. Things to try

- **Trace a value.** `cargo run -- parse suite/rational.ssa` prints the
  desugared, flat form — see what the one-line guards and expressions
  actually become.
- **Break it honestly.** Change `%x % %y` to `%x / %y` in `rat_gcd` and
  run the suite; watch four backends agree that you broke it.
- **Extend it.** A `@rat_pow(%x: $rat, %n: i64)` by repeated squaring
  is an afternoon exercise, and the directive syntax makes the tests
  cheap: decide the answers with pencil and paper (or better, a Python
  `fractions` one-liner), write `;!` lines, done.
- **Meet the menagerie.** `cargo run -- test` runs `suite/menagerie.ssa`
  with the rest; try `cargo run -- run suite/menagerie.ssa f8add 56 64`
  (fp8: 1.0 + 2.0 = 3.0 = 0x44), and `cargo test` includes the
  exhaustive fp8 sweep.
- **Steal the recipe.** The library pattern — a struct for layout, SSA
  functions for semantics, canonical forms for cheap equality, an error
  value that propagates, directives for truth — is how fixed-point and
  interval arithmetic will arrive too, and exactly how fp8/fp16/bf16
  just did (`suite/menagerie.ssa`). `future-work.md` has the map.

The whole library is ~220 lines. Read it end to end with this page
beside you, and you'll have met most of the language: types that carry
meaning, values that never mutate, control flow that declares its
state, sugar that disappears at the parser, and a test culture where
"it works" always means "four machines agree, bit for bit."
