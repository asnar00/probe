# format
*formatting by dispatch: the `<<` methods on a byte stream, the library's and a store's own*

parent: greeting
layer: runtime

> (suite) 2026-09-10T10:01:00
Third pass item 3 (rulings-3, log 59, 60): the library defines `<<` methods on a byte stream, so `out$ << 42` and `out$ << (frame i$)` need no `print` variant per type.

## overview
A push into a stream of bytes is dispatched on the item's type (section 6). The compiler's `platform` feature defines `<<` methods in zero for an `int`, a `uint`, a `float`, a `bool`, and a sequence of ints or floats with a space between the items; a struct with no method of its own is pushed as its fields with a space between, each by the same rule, and an enumeration as its case's name; a string is section 9's block push, the primitive the methods are written over. A store adds a method by declaring one: `on (uint8 o$) << (pair p)` here writes a `pair` in brackets, and wins for its type over the compiler's rule for a struct. A literal is its own type first, so `out$ << 42` writes the digits `42`; a raw byte is a `uint8` value, `out$ << uint8(65)`.

## interface
- `an int`, `a negative int`, `an int64` write integers in decimal; `ints` and `a frame` a sequence of them with spaces.
- `a float`, `floats`, `a third`, `a negative float`, `a large float`, `two thirds` write floats: the whole part, a point, up to six places with the trailing zeros dropped but one kept; `not a number` and `infinity` write `nan`, `inf`, `-inf`.
- `a bool` writes `true` and `false`; `a byte` writes a `uint8` as itself.
- `a struct` writes `point`'s fields with spaces; `a pair` takes this feature's own method; `a colour` writes an enumeration's case.
- `mixed` and `lines` chain strings and numbers in one `<<` line.

## rules
- The methods are `on (uint8 o$) << (T x)`: the stream, then the item, no result. A method may not take the stream's own element type or a sequence of it: those are the push itself.
- A method over an abstract type is a template: `(int x)` serves `int64` and `int32`, `(float x)` serves `float32` and `float64`, each at its own precision.
- A float prints six places rounded half up; a `float32` past 2^24 or a float past 2^63 prints digits it does not have (question 34).
- A stream of structs is pushed an item at a time, as before.

## testing
>an int() → "42"
>a negative int() → "-7 -10"
>an int64() → "1234567890123"
>ints() → "1 2 3"
>a frame() → "5 6 7"
>a float() → "2.5"
>floats() → "1.5 3.0 0.25"
>a third() → "0.333333"
>a negative float() → "-0.5"
>a large float() → "1000000.0"
>two thirds() → "0.666667"
>not a number() → "nan"
>infinity() → "inf -inf"
>a bool() → "true false"
>a byte() → "A"
>a struct() → "1 2 3"
>a pair() → "(1, 2)"
>a colour() → "green"
>mixed() → "n = 5, x = 1.5"
>lines() → "1\n2"

## hostile
`on (uint8 o$) = (uint8 o$) << (int x)` is refused: "unexpected '<<' in a function's name". `on (int o$) << (int x)` is refused: "'int' pushed into 'int$' is the push itself, not a method". `on (uint8 o$) << (uint8 c$)` is refused: "'string' pushed into 'string' is the block push of section 9, not a method". `out$ << t$` on a stream of the struct `token` is refused: "'out$' holds u8 but the item is token$: no `<<` method takes it"; so is any item no method takes.
