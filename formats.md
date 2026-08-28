# Adding a number format

A number format in probe is a library, not a compiler feature. This is the whole recipe, with `lib/time.ssa` (the newest one) as the worked example. `/format <name>` scaffolds steps 1–4.

## 1. Declare the type

A format is a `pack` (bitfields, lowest bits first, up to 256 bits), usually parametric in its widths, or an alias of one:

```
type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }
type rational(N, D) = pack { numerator: i(N), denominator: u(D) }
type time = rational(64, 64)
```

Put it in `lib/<name>.ssa`. Every file compiled gets `lib/*.ssa` appended in name order (`src/ssa.rs`):

```
/// the prelude: every `lib/*.ssa`, in name order, appended to a program
/// so its types and generics are always available (and appended, not
/// prepended, so the program's own line numbers hold)
pub fn with_prelude(src: &str) -> String {
```

Declaration order between files does not matter: types are parsed first (with retries for names declared later), then generics and signatures, then bodies.

## 2. Define the operations as generics named after opcodes

`add x, y` on a value of your type is a call to *your* `add`:

```
/// The instance of operation `op` for a source type and a result type:
/// the generic named `op` whose first parameter matches the source and
/// whose first result matches the destination, with the width
/// parameters those matches bind.
```

So write `fn add(N, D)(a: rational(N, D), b: rational(N, D)) -> rational(N, D)`, `fn lt(...) -> u1`, `fn conv(W, N, D)(a: i(W)) -> rational(N, D)` and so on, over the type's own parameters; the bodies use integer instructions, other library types, and — when an intermediate outgrows a word — a wide type (`p: u128 = mul aw, bw`), which the lowering turns into words. An alias inherits everything: `time` defines no arithmetic at all, because `mul` on a `time` is `mul(64, 64)` by its origin. A generic parameter nothing binds is filled by the policy (`round`), so a format with modes takes `round` as its last parameter.

Any name works for operations that are not opcodes (`sqrt`, `period`, `to_nanos`): they are plain calls.

## 3. Literals

A literal on your type goes through your `conv` from `i64` or `f64` (`src/ssa.rs`, `make_literal`):

```
/// The hidden instructions for a literal of type `ty`: a `const` when
/// the type reads literals itself (integers, pointers, floats, plain
/// packs by bit pattern); for a library number type (fixed, rational,
/// unit, ...) a `const` in i64 or f64 followed by the library's own
/// `conv` into the type — so every family gets literals through its
/// conversion, and `x: scalar = const 0.5` means 0.5 whatever scalar is.
```

Define `conv(W, ...)(a: i(W))` and, if fractions make sense, `conv(E, M, ...)(a: float(E, M))`, and `div t, 1000` just works.

## 4. Verify

A suite file, `suite/<name>.ssa`, with `;! f args -> expected` directives whose expected values come from something that shares no code with the library: Python's `fractions` for `time` and `f128`, Berkeley TestFloat for the floats, Rust's `u128` for wide integers. A pack result prints as its bits, a wide one as words, low first. Then

```
cargo run -- test            # native, and: test wasm, test riscv, test arm-qemu
cargo run -- --soft test     # the library body even where a platform has hardware
```

For a format with an exhaustive small instance (`float(4, 3)`, `unit(8)`, `fixed(8, 8)`), a Rust test against a model in `src/emit.rs` is worth it: see `rationals_match_model` and `unit_types_match_model`.

## 5. Optional: make it a policy family

Only if programs should be able to say a bare `<name>` and have the policy choose the widths, like `float`, `fixed`, `unit`, `sunit` and `rational`. That is four small edits in `src/ssa.rs` and `src/main.rs`: a field on `Policy` and its `Policy::new` defaults, a `with_<name>` and the `--<name>=` flag, an arm in `default_args`

```
    fn default_args(&self, name: &str) -> Option<Vec<i64>> {
        match name {
            "float" => Some(vec![self.float.0 as i64, self.float.1 as i64]),
            "fixed" => Some(vec![self.fixed.0 as i64, self.fixed.1 as i64]),
```

and, if `scalar` may stand for it, the name in `Policy::SCALARS` — after which `regression_suite_scalar_families` runs the whole suite under it. `time` is not a family: it has one width and no bare form.

## 6. Optional: platform rules

If a target has instructions for an instance, add them to `targets/<target>.platform` (`ssa.md`, *Platforms*): a `class` line for where its values live, and one line per instruction — `fadd {s}, {s}, {s} = add(f32, f32) -> f32`. The library stays the reference; `probe testfloat`-style oracles compare both.

## What the first run of this recipe found

`lib/decimal.ssa` (`decimal(N, S)`, an `i(N)` at scale 10^S) was written to this document in one sitting; the only friction: width expressions have no power operator, so a constant that depends on a parameter non-linearly (10^S) is a generic helper computing it at run time — `pow10(S)()` — which the const-folder then folds. Everything else — dispatch, literals through `conv`, wide intermediates, the suite harness giving and expecting significands — worked as described.

## 7. Write it down

An entry in `history.md`, and the README's list if it is a family.
