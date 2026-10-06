# bye
*extends hello: goodbye after the greeting; and the program's case, static on*

parent: hello
layer: runtime

> (suite) 2026-09-08T10:02:00
Section 16's `feature Bye extends Hello`: `existing run()` then `goodbye()`; and the program's one case, every feature static on.

## overview
After everything `run` did before, `goodbye` prints "goodbye". Being the newest feature, `bye` is the outermost link of `run`, so its cases see the whole program.

## interface
- `run` does what it did before, then says goodbye.
- `goodbye` prints "goodbye".

## rules
- The program writes the countdown, a number to a line, then "hello world", then "goodbye".
- The case says when (question 52, fm3 log 91): each piece of the output with the time on the store's clock at which it was written, a number a second from 0 s, the greeting and the goodbye following the last number at 9 s.
- Static on (log 71): `run` is this feature's body under the plain name, since no link stands above the newest static feature; `existing run()` calls `run__countdown` by name.
- Being the newest, this feature's `run()` case overrides countdown's and hello's.

## testing
>run() → "10\n" at 0 s, "9\n" at 1 s, "8\n" at 2 s, "7\n" at 3 s, "6\n" at 4 s, "5\n" at 5 s, "4\n" at 6 s, "3\n" at 7 s, "2\n" at 8 s, "1\nhello world\ngoodbye" at 9 s

## hostile
`existing run()` in hello's own `run` would be refused: hello is the first definition.
