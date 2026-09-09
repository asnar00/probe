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
- The program prints the countdown on one line, then "hello world", then "goodbye".
- Static on (log 71): `run` is this feature's body under the plain name, since no link stands above the newest static feature; `existing run()` calls `run__countdown` by name.
- Being the newest, this feature's `run()` case overrides countdown's and hello's.

## testing
>run() → "10 9 8 7 6 5 4 3 2 1\nhello world\ngoodbye"

## hostile
`existing run()` in hello's own `run` would be refused: hello is the first definition.
