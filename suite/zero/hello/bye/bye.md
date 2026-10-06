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
- The program writes the countdown, a number to a line, then "hello world", then "goodbye".
- The case says when (question 52, fm3 log 91): `>run() → "10\n" at 0 s, "9\n" at 1 s, "8\n" at 2 s, "7\n" at 3 s, "6\n" at 4 s, "5\n" at 5 s, "4\n" at 6 s, "3\n" at 7 s, "2\n" at 8 s, "1\nhello world\ngoodbye" at 9 s` is every piece of the output in order, each with the time on the store's clock at which it was written, and the pieces joined are the whole output. The suite runs on the virtual clock, so the nine seconds pass in no time and the stamps are what is checked. Each number is written when its second comes; the greeting and the goodbye follow the last number at once, at 9 s.
- With Countdown off — `>run() with countdown off`, the runner's context, since no code switches a feature (section 14, log 43) — `run` is bye's link calling hello's: "hello world", then "goodbye"; the countdown's state is kept for when it is on again.
- Being the newest, this feature's `run()` case overrides countdown's and hello's wherever `bye` is on; and in the runner's own `countdown off` context the second line here stands over the first, being the one that names the context (log 44).

## testing
>run() → "10\n" at 0 s, "9\n" at 1 s, "8\n" at 2 s, "7\n" at 3 s, "6\n" at 4 s, "5\n" at 5 s, "4\n" at 6 s, "3\n" at 7 s, "2\n" at 8 s, "1\nhello world\ngoodbye" at 9 s
>run() with countdown off → "hello world\ngoodbye"

## hostile
`existing run()` in hello's own `run` would be refused: hello is the first definition.
