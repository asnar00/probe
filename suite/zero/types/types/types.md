# types
*numbers abstract and concrete, `bool`, structs with defaults, enumerations, and `string` as bytes*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 3 of milestone 0: section 4 of zero.md — abstract and concrete numbers, `bool`, structs with defaults and construction by position and name, field access, enumerations, `string` as `uint8$` with literals. Vectors and packs are not in this milestone.

## overview
`Vec` is three floats side by side with a default of zero each; it is built by position, by name, or from nothing, and `+` and `*` on it are functions the feature defines. `Tristate` is an enumeration of three cases. A `string` is a sequence of bytes, printed as it is.

## interface
- `(a) + (b)` on two `Vec`s adds field by field; `(a) * (k)` scales a `Vec` by a float.
- `sum of fields (v)` folds a `Vec` into an `int`, converting with `int(...)`.
- `added`, `scaled`, `by name`, `defaulted` build `Vec`s the four ways and sum them.
- `flipped (x)` swaps `no` and `yes` and leaves `maybe`; `is maybe (t)` compares with the qualified case `Tristate.maybe`.
- `widened (x)` converts an `int32` to `int`; `low byte (x)` converts an `int` to `uint8`, which wraps.
- `halved` divides a float and truncates; `flags` compares bools.
- `greet (who)` prints a string it was given; `hello` calls it; `name` gives a string result and `named` prints it.

## rules
- A struct's fields are concrete from its declaration: an `int` field is the target's width.
- A field missing from a construction takes its declared default, or zero.
- An enumeration compares with `==` and `!=` only; a case is named bare when the name is unique, or as `Type.case`.
- A conversion is a type applied to a value: `int(x)`, `uint8(x)`.

## testing
>added() → 66
>scaled() → 15
>by name() → 4
>defaulted() → 0
>flipped (1) → 0
>flipped (0) → 1
>flipped (2) → 2
>is maybe (2) → 1
>is maybe (1) → 0
>widened (21) → 42
>low byte (300) → 44
>halved() → 3
>flags() → 0
>hello() → "world"
>named() → "zero"

## hostile
`Vec v(1, 2, 3, 4)` is refused: "Vec has 3 field(s), given 4". `t < yes` on a `Tristate` is refused: only `==` and `!=` apply. A second `+` whose first operand is a `Vec` is refused as a clash.
