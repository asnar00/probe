# hello
*the sketch's hello world, section 16 of zero.md: the root*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan items 9 and 12 of milestone 0: section 16's three features, `hello`, `countdown` and `bye`, with the countdown at `1 hz` on the virtual clock.

## overview
`run` says hello. The other two features extend it: `countdown` counts down before it, `bye` says goodbye after it.

## interface
- `run` is the program; `hello` prints "hello world".

## rules
- `run` is a chain: bye's, then countdown's, then hello's, each calling `existing run()` where it wants the earlier ones.

## testing
>hello() → "hello world"

## hostile
A `#` in any of the three `.zero` files stops the build naming the line.
