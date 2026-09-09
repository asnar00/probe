# countdown
*extends hello: a countdown from 10 at one hertz before the greeting, wired into the output as an edge*

parent: hello
layer: runtime

> (suite) 2026-09-08T10:01:00
Section 16's `feature Countdown extends Hello`: `count down()` then `existing run()`.

## overview
Before hello says hello, `count down` writes 10 to 1, one to a line: a stream at one hertz, `int i$ at (1 hz)`, an edge from it into the output, `out$ << i$ << "\n"`, and the range `[10 through 1]` pushed into it, so the ten numbers sit one second apart on the stream's clock and each goes out as it arrives.

## interface
- `run` counts down, then does what it did before.
- `count down` pushes `[10 through 1]` into `i$`; the edge at feature scope does the writing.

## rules
- A stream on the right of `<<` at feature scope is an edge (section 9, log 72): `out$ << i$ << "\n"` is a standing connection, a node the scheduler runs whenever `i$` has more than it has seen, moving each item into `out$` by the `<<` method for an `int` and pushing the rest of the chain, the newline, after each; `frame i$` is the one-shot read. Section 16 writes `print [10 through 1] at (1 hz)`; here the rate is on the stream's declaration and the printing is the edge.
- With countdown off (`>run() with countdown off`), `run` falls through to hello's, and the edge's node does not run.
- `run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nhello world"` overrides hello's `run() → "hello world"` wherever this feature is on, and is itself overridden by bye's; it stands, and passes, in the runner's context with `bye` off.
- The count is in the code as the range's literal bounds, so `probe cost` on `run` counts the countdown's loop as ten passes without a bound anywhere: a bound is a product setting, never a word in feature code (question 19, log 41). The edge's own loop, over whatever has arrived, is counted once; inside it each item is taken at its tick, `i$` being at 1 hz, so `probe zero suite/zero/hello run "run()"` writes a number a second on the real clock (log 77).

## testing
>count down() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1"
>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nhello world"

## hostile
`count down` after the clock has moved still writes the same numbers: the ticks are the clock's, the values the pushes'. `out$ << 10 << 9` at feature scope is refused: "a line at feature scope pushing into 'out$' is an edge, `out$ << x$`, and its first item is a stream; items are pushed on the declaration, `uint8 out$ << ...`".
