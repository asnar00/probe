# types
*numbers abstract and concrete, `bool`, structs with defaults, enumerations, and `string` as bytes*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 3 of milestone 0: section 4 of zero.md — abstract and concrete numbers, `bool`, structs with defaults and construction by position and name, field access, enumerations, `string` as `char$` with literals. Vectors and packs are not in this milestone.

## overview
`Vec` is three floats side by side with a default of zero each; it is built by position, by name, or from nothing, and `+` and `*` on it are functions the feature defines. `Tristate` is an enumeration of three cases. A `string` is a sequence of bytes, printed as it is.

## interface
- `(a) + (b)` on two `Vec`s adds field by field; `(a) * (k)` scales a `Vec` by a float, and `(a) * (b)` on two `Vec`s multiplies field by field: two methods of `*` on a `Vec`, chosen by the right operand.
- `sum of fields (v)` folds a `Vec` into an `int`, converting with `int(...)`.
- `added`, `scaled`, `by name`, `defaulted` build `Vec`s the four ways and sum them; `scaled by a vec` uses the second `*`.
- `flipped (x)` swaps `no` and `yes` and leaves `maybe`; `is maybe (t)` compares with the qualified case `Tristate.maybe`.
- `widened (x)` converts an `int32` to `int`; `low byte (x)` converts an `int` to `uint8`, which wraps.
- `halved` divides a float and truncates; `flags` compares bools.
- `greet (who)` prints a string it was given; `hello` calls it; `name` gives a string result and `named` prints it.
- `ratio of (a) to (b)` gives a `float64` from two `int32`s: the result type drives the conversion, so the division is a float one; `ratio times ten` reads it through `int(...)`, since a case line compares integers.
- `product of (a) and (b)` gives an `int64` from two `int32`s, multiplied at 64 bits, so `100000 * 100000` does not overflow.
- `mixed sum` adds an `int32` to an `int64`; `mixed float` an `int16` to a `float32`, which holds it exactly, into a `float64`; `compared across widths` compares an `int16` with a `float32`.
- `smaller of (a) and (b)` over `number` called with an `int16` and a `float32` binds `number` to `float32`, the wider, and converts the `int16`.
- `widened on assignment` assigns an `int32` to an `int64`; `narrowed explicitly` assigns an `int64` to an `int32` through `int32(...)`, the explicit conversion narrowing.
- `mixed wide` adds an `int64` to a `float64`: no float holds every `int64`, so the sum is computed in `float64`, the widest float (question 21). `rounding above fifty three bits` assigns 2^53 + 1 to a `float64` and reads it back: the conversion is implied and rounds, so the difference is -1; `compared above fifty three bits` compares that `int64` with 2^53 as a `float64` and finds them equal, since the comparison is made in `float64`. `smaller across wide types` binds `number` to `float64` for an `int64` and a `float64`; `picked wide` meets an `int64` and a `float32` in the arms of `if then else`, which join in `float64`.

## rules
- A struct's fields are concrete from its declaration: an `int` field is the target's width.
- A field missing from a construction takes its declared default, or zero.
- An enumeration compares with `==` and `!=` only; a case is named bare when the name is unique, or as `Type.case`.
- A conversion is a type applied to a value: `int(x)`, `uint8(x)`, `char(x)`. It is the explicit form, needed to narrow. A `char` is a character, not a small number (question 44): it is compared, converted, and never added to.
- Conversions are implied towards the type that holds the operands best (questions 1 and 21). Two concrete numbers in an operator compute in the type that holds every value of both exactly where one exists: the wider of two signed or two unsigned widths, a signed type wide enough for an unsigned one, the wider float, and for a float with an integer the float that holds the integer exactly (`float32` holds `int16`, `float64` holds `int32`). Where none does, the widest of the family: an `int64` or wider with any float computes in `float64` and rounds above 2^53; arithmetic is modulo its range, and the writer of the code is expected to know it. Where the expression's result type is a concrete number both widen to, the operands are converted to it first. A value widens on assignment, into a field, into a parameter, and into the type an abstract name binds to, which is the widest of what its arguments bring. A narrowing, a conversion to a type where a wider one holds the pair better (`float32` from an `int32`, since `float64` holds both), and any conversion touching an abstract type (`int` has no width in the source) are refused naming `T(x)`.
- A conversion that can lose bits is implied but noted: the emitted IR carries `; int64 into float64: rounded above 2^53` above the `conv`, the warning the ruling allows.

## testing
>added() → 66
>scaled() → 15
>scaled by a vec() → 12
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
>ratio times ten (7) and (2) → 35
>product of (100000) and (100000) → 10000000000
>mixed sum() → 11
>mixed float() → 7
>compared across widths() → 1
>smaller across types() → 5
>widened on assignment() → 40
>narrowed explicitly() → 300000
>mixed wide() → 7
>rounding above fifty three bits() → -1
>compared above fifty three bits() → 0
>smaller across wide types() → 5
>picked wide (true) → 18
>picked wide (false) → 3
>flags() → 0
>hello() → "world"
>named() → "zero"

## hostile
`Vec v(1, 2, 3, 4)` is refused: "Vec has 3 field(s), given 4". `t < yes` on a `Tristate` is refused: only `==` and `!=` apply. A second `+` on the same operand types, `(Vec a) + (Vec b)` again, is refused: "an operator is not redefined in this milestone"; on other types it is a second method (`*` here), and `a * 2` with both `(Vec) * (float)` and `(Vec) * (Vec)` declared takes the float one, a literal fitting a number. Conversions that narrow are refused naming the explicit form: `int32 y = big` with `big` an `int64` says "'y' is int32 but the value is int64: int32(...) narrows it, which is not implied"; `float32 x = i` with `i` an `int32` says "float32(...) is not implied: float64 holds both better", since `float32` holds only 24 bits of an `int32`; `float32 y = a + x` on an `int64` and a `float32` says the sum, made in `float64`, "narrows it, which is not implied", where `float64 y = a + x` is accepted with the note in the IR; `int32 y = x` with `x` an `int` says "int32(...) converts it; nothing widens it", an abstract type having no width in the source, and `a + x` on an `int` and an `int32` says "no number type computes both; convert one"; and `2.5 * a` where an `int32` is wanted is "a decimal where an int32 is wanted".
