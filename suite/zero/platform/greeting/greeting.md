# greeting
*`print` is a platform function of the compiler's own `platform` feature, not a special case*

layer: runtime

> (suite) 2026-09-08T10:01:00
Plan item 11 of milestone 0: `print` written as a platform function with an `ir` body, in `src/zero/platform.zero`, composed first into every store.

## overview
Every store composes the compiler's `platform` feature first, in the lowest layer. It declares `print (string s)` and `print (int x$)`, each with a `platform ir` body that appends to the output buffer the runner reads; `probe zero <store> emit` shows them under `; feature platform`. The two share their words and differ in their parameter's type: a call takes the one whose parameter fits its argument, and the IR names the second by its types, `print__ints`.

## interface
- `greet` prints a string; `count to three` prints a sequence of ints on one line.

## rules
- A store folder named `platform` is refused: the name is the compiler's.
- A feature may not redefine `print`: the platform is called, not composed (section 15).

## testing
>greet() → "hello"
>count to three() → "1 2 3"

## hostile
`on print (string s)` in a store's feature is refused: "'print' is a platform function of feature platform: the platform is called, not redefined". `print (1.5)` is refused: "no 'print' takes these arguments: it is declared for (u8[]) and (int[])".
