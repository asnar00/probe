# countdown
*extends hello: a countdown from 10 at one hertz before the greeting, wired into the output as an edge*

parent: hello
layer: runtime

> (suite) 2026-09-08T10:01:00
Section 16's `feature Countdown extends Hello`: `count down()` then `existing run()`.

## overview
Before hello says hello, `count down` writes 10 to 1, one to a line: a stream at one hertz, `int i$ at (1 hz)`, an edge from it into the output, `out$ << i$ << "\n"`, and the range `[10 through 1]` pushed into it, so each number goes out as it is pushed and then its second passes: ten numbers take ten seconds.

## interface
- `run` counts down, then does what it did before.
- `count down` pushes `[10 through 1]` into `i$`; the edge at feature scope does the writing.

## rules
- A stream on the right of `<<` at feature scope is an edge (section 9, log 72): `out$ << i$ << "\n"` is a standing connection, moving each item into `out$` by the `<<` method for an `int` and pushing the rest of the chain, the newline, after each. Nothing in the store reads `i$`, it is only pushed into and wired, so it has no storage (question 50, fm3 log 92): the edge is a function of one item, `__edge1(__item: int)`, and `count down`'s push calls it for each number, with no queue, no node and no scheduler between. A word that read `i$` anywhere, `count i$` or `frame i$`, would make it a queue again. Section 16 writes `print [10 through 1] at (1 hz)`; here the rate is on the stream's declaration and the printing is the edge.
- With countdown off (`>run() with countdown off`), `run` falls through to hello's, and nothing pushes into `i$`. The edge is called under this feature's gate, read once before the ten numbers.
- `run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nhello world"` overrides hello's `run() → "hello world"` wherever this feature is on, and is itself overridden by bye's; it stands, and passes, in the runner's context with `bye` off.
- The count is in the code as the range's literal bounds, so `probe cost` on `run` counts the countdown's loop as ten passes without a bound anywhere: a bound is a product setting, never a word in feature code (question 19, log 41). The stream is at `1 hz`: each number is written when it is pushed and then its second passes (question 52), so the last number is written at 9 s and `count down` returns at 10 s; `probe zero suite/zero/hello run "run()"` writes a number a second on the real clock (log 77).

## testing
>count down() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1"
>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nhello world"

## hostile
`count down` after the clock has moved still writes the same numbers: the ticks are the clock's, the values the pushes'. `out$ << 10 << 9` at feature scope is refused: "a line at feature scope pushing into 'out$' is an edge, `out$ << x$`, and its first item is a stream; items are pushed on the declaration, `char out$ << ...`".
