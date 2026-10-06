# countdown
*extends hello: a countdown from 10 at one hertz before the greeting, wired into the output as an edge, static on*

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
- Static on (log 71): `run` is bye's body calling `run__countdown`, this feature's, by name, with no gate; the edge's call has no gate either; there is no `with countdown off` context.
- `run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nhello world"` overrides hello's `run() → "hello world"` and is itself overridden by bye's, which is the one that runs: every feature is on and static.
- The count is in the code as the range's literal bounds, so `probe cost` on `run` counts the countdown's loop as ten passes without a bound anywhere: a bound is a product setting, never a word in feature code (question 19, log 41). The stream is at `1 hz`, so it has a beat, a slot every second from 0 s: the first number lands in the stream's next slot, which is at once when `count down` is called on the beat as `run` calls it — and the compiler can see that it always is, `run` and its `existing` links reaching it at 0 s and nowhere else, so it checks nothing (question 56, fm3 log 99) — and each number is written when it lands and then its second passes (question 52, fm3 log 98), so the last number is written at 9 s and `count down` returns at 10 s; `probe zero suite/zero/static run "run()"` writes a number a second on the real clock (log 77).

## testing
>count down() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1"
>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nhello world"

## hostile
`count down` after the clock has moved still writes the same numbers: the ticks are the clock's, the values the pushes'.
