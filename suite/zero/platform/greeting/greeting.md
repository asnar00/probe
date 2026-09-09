# greeting
*`print` is a word of the compiler's own `platform` feature over the output stream `out$`, not a special case*

layer: runtime

> (suite) 2026-09-08T10:01:00
Plan item 11 of milestone 0: `print` written as a platform function with an `ir` body, in `src/zero/platform.zero`, composed first into every store. Third pass item 3 (log 59): both `print`s are zero words over `out$`, the formatting being the `<<` methods of the `format` feature.

## overview
Every store composes the compiler's `platform` feature first, in the lowest layer. It declares the output stream `uint8 out$` and its consumer `write`, wired `write(out$)` (third pass, log 57; see the `sink` feature), and two `print`s, both zero functions: `print (string s)` is `out$ << s << "\n"` and `print (int x$)` is `out$ << x$ << "\n"`, the digits and the spaces being the `<<` method for a sequence of ints (third pass, log 59; see `format`); `probe zero <store> emit` shows them under `; feature platform`. The two share their words and differ in their parameter's type: a call takes the one whose parameter fits its argument, and the IR names the second by its types, `print__ints`. A program may write `out$ << "hello"` itself; `print` stays as the library word.

## interface
- `greet` prints a string; `count to three` prints a sequence of ints on one line.

## rules
- A store folder named `platform` is refused: the name is the compiler's.
- A feature may redefine either `print`, a zero function of the platform layer, with `existing` as for any function; a platform function with an `ir` or a target body (`write`, `machine`'s) it may not: the platform is called, not composed (section 15).

## testing
>greet() → "hello"
>count to three() → "1 2 3"

## hostile
`on write (uint8 c$)` in a store's feature is refused: "'write (string)' is a platform function of feature platform: the platform is called, not redefined (section 15)". `print (1.5)` is refused: "no 'print' takes these arguments: the methods are print (string), print (int$)".
