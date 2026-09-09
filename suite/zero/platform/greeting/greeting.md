# greeting
*`print` is a word of the compiler's own `platform` feature over the output stream `out$`, not a special case*

layer: runtime

> (suite) 2026-09-08T10:01:00
Plan item 11 of milestone 0: `print` written as a platform function with an `ir` body, in `src/zero/platform.zero`, composed first into every store.

## overview
Every store composes the compiler's `platform` feature first, in the lowest layer. It declares the output stream `uint8 out$` and its consumer `write`, wired `write(out$)` (third pass, log 57; see the `sink` feature), and two `print`s: `print (string s)` is a zero function, `out$ << s << "\n"`, and `print (int x$)` has a `platform ir` body that pushes each int in decimal with a space between; `probe zero <store> emit` shows them under `; feature platform`. The two share their words and differ in their parameter's type: a call takes the one whose parameter fits its argument, and the IR names the second by its types, `print__ints`. A program may write `out$ << "hello"` itself; `print` stays as the library word.

## interface
- `greet` prints a string; `count to three` prints a sequence of ints on one line.

## rules
- A store folder named `platform` is refused: the name is the compiler's.
- A feature may not redefine `print (int x$)`, a platform function: the platform is called, not composed (section 15). `print (string s)`, a zero function of the platform layer, it may.

## testing
>greet() → "hello"
>count to three() → "1 2 3"

## hostile
`on print (int x$)` in a store's feature is refused: "'print' is a platform function of feature platform: the platform is called, not redefined". `print (1.5)` is refused: "no 'print' takes these arguments: the methods are print (string), print (int$)".
