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
- The case says when (questions 52 and 53, fm3 log 91, 97): `>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "hello world\ngoodbye" at 10 s` is every piece of the output in order with the time on the store's clock at which it was written, and the pieces joined are the whole output. A piece given `at` a rate is its lines, one a step of that rate, from 0 s: `10` at 0 s, `9` at 1 s and on to `1` at 9 s; a piece given `at` a time is written then. The same case listed line by line, `"10\n" at 0 s, "9\n" at 1 s` and so on, says the same thing in 182 characters. The suite runs on the virtual clock, so the nine seconds pass in no time and the stamps are what is checked. Each number is written when it is pushed and lasts a second, the stream being at `1 hz`, so ten numbers take ten seconds: the last is written at 9 s and the greeting and the goodbye at 10 s (question 52: an item in a stream with a rate has a length).
- With Countdown off — `>run() with countdown off`, the runner's context, since no code switches a feature (section 14, log 43) — `run` is bye's link calling hello's: "hello world", then "goodbye"; the countdown's state is kept for when it is on again.
- Being the newest, this feature's `run()` case overrides countdown's and hello's wherever `bye` is on; and in the runner's own `countdown off` context the second line here stands over the first, being the one that names the context (log 44).

## testing
>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "hello world\ngoodbye" at 10 s
>run() with countdown off → "hello world\ngoodbye"

## hostile
`existing run()` in hello's own `run` would be refused: hello is the first definition.
