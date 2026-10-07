# shown
*extends edges: the edges that show a countdown and a beat, in a feature of their own*

parent: edges
layer: runtime

> (suite) 2026-10-06T10:01:00
Parity hop 8 (question 51, fm3 log 92): a producer in one feature and its edge in another.

## overview
`out$ << n$ << "\n" forever` wires the stream `edges` counts down into, `out$ << beat$ << "\n" forever` the one it beats into, and `out$ << quick$ << "\n" forever` the faster one `turns` pushes into. The producer does not know: it pushes, and the push calls this feature's edges where this feature is on.

## interface
- The three edges, and nothing else.

## rules
- With this feature on, `count down from (3)` writes its numbers and then `liftoff`; with it off, `liftoff` alone, which is `edges`' own case and stands there.
- `beat$` is at `2 hz`: an item is written when it is pushed and lasts half a second, so `1` is at 0 s, `2` at 500 ms and `end` at 1 s. The times are the same with this feature off, when nothing but `end` is written. The case lists its pieces each with a time, which is the form a rate abbreviates; `end` is not an item of the stream, so it is not written as a third line `at 2 hz`, though that would say the same.
- `turns` pushes into two streams by turns, and each item is written in its own stream's next slot: `1` at 0 s and `3` at 1 s on `beat$`'s half seconds, `2` at 600 ms on `quick$`'s fifths, and `end` at 1.5 s when the last half second has passed. The case lists its pieces, since nothing here is regular.
- `drum (3)` writes `0` at 0 s on `quick$`, then `1`, `2` and `3` on `beat$`'s half seconds from 500 ms, and `end` at 2 s; `drum (0)` writes `0` and then `end` at 200 ms, its loop running no pass and taking no time.

## testing
>count down from (3) → "3\n2\n1\nliftoff"
>beats() → "1\n" at 0 s, "2\n" at 500 ms, "end" at 1 s
>turns() → "1\n" at 0 s, "2\n" at 600 ms, "3\n" at 1 s, "end" at 1.5 s
>drum (3) → "0\n" at 0 s, "1\n" at 500 ms, "2\n" at 1 s, "3\n" at 1.5 s, "end" at 2 s
>drum (0) → "0\n" at 0 s, "end" at 200 ms
