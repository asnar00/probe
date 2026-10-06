# shown
*extends edges: the edges that show a countdown and a beat, in a feature of their own*

parent: edges
layer: runtime

> (suite) 2026-10-06T10:01:00
Parity hop 8 (question 51, fm3 log 92): a producer in one feature and its edge in another.

## overview
`out$ << n$ << "\n"` wires the stream `edges` counts down into, and `out$ << beat$ << "\n"` the one it beats into. The producer does not know: it pushes, and the push calls this feature's edges where this feature is on.

## interface
- The two edges, and nothing else.

## rules
- With this feature on, `count down from (3)` writes its numbers and then `liftoff`; with it off, `liftoff` alone, which is `edges`' own case and stands there.
- `beat$` is at `2 hz`: an item is written when it is pushed and lasts half a second, so `1` is at 0 s, `2` at 500 ms and `end` at 1 s. The times are the same with this feature off, when nothing but `end` is written. The case lists its pieces each with a time, which is the form a rate abbreviates; `end` is not an item of the stream, so it is not written as a third line `at 2 hz`, though that would say the same.

## testing
>count down from (3) → "3\n2\n1\nliftoff"
>beats() → "1\n" at 0 s, "2\n" at 500 ms, "end" at 1 s
