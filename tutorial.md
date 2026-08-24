# A first look at probe: floats of every size

probe is a small compiler with an unusual habit: instead of trusting
big reference manuals, it checks everything against something
independent. It learns machine instruction encodings by probing real
assemblers, and it runs every test on four different machines that must
all agree, bit for bit. This page shows you the language it compiles,
starting with its favorite trick: **floating-point numbers of any
size**.

You don't need to know the project already. You do need to vaguely
remember what a floating-point number is; we'll rebuild the rest as we
go.

## 1. One type, every float

Modern AI hardware is full of tiny floats — fp8, bf16, fp16 — because
smaller numbers move faster and pack tighter. Each format is usually a
separate, special thing. In probe, they're all one type with two
numbers in it:

```
float(E, M)
```

`E` is how many bits the exponent gets, `M` is how many the fraction
gets, and there's always one sign bit on top. So a `float(4, 3)` is 8
bits laid out like this:

```
   [ s ][ e e e e ][ m m m ]
    sign  exponent  fraction     = fp8 (the "e4m3" format)
```

Pick different numbers, get different formats:

| you write | you get | size |
|---|---|---|
| `float(4, 3)` | fp8 | 8 bits |
| `float(5, 10)` | fp16 (half precision) | 16 bits |
| `float(8, 7)` | bf16 (bfloat) | 16 bits |
| `float(8, 23)` | **this is `f32`** — the same type | 32 bits |
| `float(11, 52)` | **this is `f64`** | 64 bits |

That last part matters: the ordinary floats aren't special built-ins
with the small ones bolted on. `f32` *is* `float(8, 23)`. There's one
family, and the familiar sizes are just two members of it.

## 2. Using one

Here's a complete probe function that adds two fp8 numbers:

```
fn @f8add(%a: float(4, 3), %b: float(4, 3)) -> float(4, 3) {
    %s: float(4, 3) = %a + %b
    ret %s
}
```

Three things to know about the language, and then you can read it:

- Names starting with `%` are values, and `@` marks a function.
- Every value states its type when it's created: `%s: float(4, 3) = ...`.
  Types do a lot of work in probe — there's one `+`, one `/`, one `<`,
  and each looks at the types of its operands to decide what kind of
  add, divide, or compare it is.
- A value is set once and never changes. (This style is called SSA;
  compilers use it internally because it makes programs easy to check.)

And it really is arithmetic: `+`, `*`, comparisons, constants, and
conversions all work on every format in the family. This computes a dot
product in 8-bit floats:

```
%s: float(4, 3) = %xa * %xb + %xc * %xd
```

## 3. How it works: borrow a bigger float

The machine has no fp8 add instruction. probe doesn't need one. To add
two fp8 values it:

1. **promotes** each one to a regular f64 — this is exact, every fp8
   value is also an f64 value,
2. does the add with the ordinary f64 hardware,
3. **demotes** the result back to fp8, rounding to the nearest
   representable value (ties go to the even one, like IEEE says).

A known theorem guarantees this gives *exactly* the right answer — the
same bits a native fp8 unit would produce — as long as the small format
has 24 fraction bits or fewer. All of ours do.

The promote and demote steps are the only format-specific code, and
here's the good part: they're written **once**, for all formats at the
same time. probe lets a function take widths as parameters:

```
fn @fp_to_f64(%bits: u(E+M+1)) -> f64 {
    %b: u64 = ext %bits
    %e: u64 = %b >> M & ((1 << E) - 1)      ; pull out the exponent
    ...
```

`E` and `M` are parameters, usable both as bit-widths (`u(E+M+1)` — "an
unsigned integer as wide as the whole float") and as plain numbers in
the code (shift by `M`, mask with `(1 << E) - 1`). When you use
`float(4, 3)` somewhere, the compiler stamps out a copy of this
function with E=4, M=3. Two generic functions in `lib/float.ssa` cover
every format that will ever exist.

(That `u(E+M+1)` trick is worth a second look. Widths that are
*arithmetic on parameters* let a signature state facts like "my result
is twice as wide as my inputs" — and the compiler checks the fact at
every size. We'll meet that again in a moment.)

## 4. Why you can trust it

fp8 has exactly 256 possible values. So probe doesn't test a sample of
additions — it tests **all of them**. Every one of the 65,536 possible
pairs is added and multiplied and compared, bit for bit, against a
separate reference implementation. Small formats don't get "probably
right"; they get checked completely.

Day-to-day tests are simpler. You write the expected answers next to
the code, in comments that start with `;!`:

```
;! f8add 0x38 0x40 -> 0x44        ; 1.0 + 2.0 = 3.0, in fp8 bits
;! h_third -> 0x3555              ; 1/3 rounded to fp16
```

and one command checks them — on your CPU, in WebAssembly, and on two
emulated machines (RISC-V and ARM under qemu). Same code, same
expected answers, four independent referees:

```sh
cargo run -- test              # native
cargo run -- test wasm
cargo run -- test riscv
cargo run -- test arm-qemu
```

One more flag worth knowing: `--softfloat` replaces all f64 hardware
instructions with software routines (also written in probe's own
language). With it, your fp8 arithmetic runs — still bit-exact — on a
processor that has no floating-point unit at all.

## 5. Looking inside a float

Because formats are just bits, probe lets you take one apart. A packed
struct names the bit fields, and the same `(E, M)` parameters describe
its layout:

```
type $fp(E, M) = { frac: u(M), exp: u(E), sign: u1 }

fn @fp8_exp(%b: u8) -> u4 {
    %p: $fp(4, 3) = bitcast %b      ; same 8 bits, structured view
    %e: u4 = extract %p, exp        ; read a field by name
    ret %e
}
```

`bitcast` reinterprets bits without changing them, and `extract` reads
a named field. This is how the software-float routines are written, and
it's a nice way to *learn* how floats work: parse one by hand, poke at
its fields, put it back together.

## 6. The other kind of number: exact fractions

Floats round. Sometimes you'd rather they didn't. probe's second number
library stores fractions exactly — a numerator over a denominator, like
you learned in school:

```
type $rat = { num: half, den: uhalf }
```

`1/3` stays `1/3`. Adding `1/2 + 1/3` gives exactly `5/6`. The test
suite sums `1 + 1/2 + 1/3 + 1/4` and demands exactly `25/12`. When a
result can't be stored exactly, you don't get a quietly rounded value —
you get a special "not a rational" marker that spreads through later
math, so you always know.

Two details connect back to what you've seen:

- Those field types, `half` and `uhalf`, mean "half a machine word."
  Why half? Because multiplying two fractions multiplies their parts,
  and a product needs **twice the bits** of its inputs. Half-word
  pieces guarantee the math always fits in a machine word — the same
  "twice as wide" fact that `u(2*N)` can state in a signature. Build
  for a 64-bit machine and fractions use 32-bit parts; build with
  `--int=i32` and the *same file* uses 16-bit parts, still correct.
- There is no compiler code for rationals. The whole thing is a library
  written in probe's own language (`lib/rational.ssa`, ~200 readable
  lines) — just like the float conversions. New kinds of numbers are
  libraries here, not compiler features. There's even a flag,
  `--scalar=rat`, that runs your floating-point-looking code in exact
  fractions instead.

## 7. Try it

```sh
# run the whole test suite (the ;! lines everywhere)
cargo run -- test

# fp8: 1.0 + 2.0  (0x38 + 0x40 -> 68 = 0x44 = 3.0)
cargo run -- run suite/menagerie.ssa f8add 56 64

# exact fractions: 1 + 1/2 + 1/3 + 1/4 -> 25, 12
cargo run -- run suite/rational.ssa t_harmonic 4

# see what the compiler actually sees (sugar expanded, blocks explicit)
cargo run -- parse suite/menagerie.ssa

# the exhaustive fp8 sweep lives in here
cargo test
```

Good next reads: `suite/menagerie.ssa` (the float family in action),
`lib/float.ssa` (the two generic conversion functions — now you can
read them), `lib/rational.ssa` (the fraction library), and `ssa.md`
(the full language reference). The habit to take with you: every claim
here is backed by a test you can run, and "it works" always means
"four machines agree, bit for bit."
