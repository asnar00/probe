# bye
*extends hello: goodbye after the greeting; and the program's two cases*

parent: hello
layer: runtime

> (suite) 2026-09-08T10:02:00
Section 16's `feature Bye extends Hello`: `existing run()` then `goodbye()`; and the two cases of the program, the second in a context with Countdown off.

## overview
After everything `run` did before, `goodbye` prints "goodbye". Being the newest feature, `bye` is the outermost link of `run`, so its cases see the whole program.

## interface
- `run` does what it did before, then says goodbye.
- `goodbye` prints "goodbye".

## rules
- The program prints the countdown on one line, then "hello world", then "goodbye".
- With Countdown off — `>run() with countdown off`, the runner's context, since no code switches a feature (section 14, log 43) — `run` is bye's link calling hello's: "hello world", then "goodbye"; the countdown's state is kept for when it is on again.

## testing
>run() → "10 9 8 7 6 5 4 3 2 1\nhello world\ngoodbye"
>run() with countdown off → "hello world\ngoodbye"

## hostile
`existing run()` in hello's own `run` would be refused: hello is the first definition.
