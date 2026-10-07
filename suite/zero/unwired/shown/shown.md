# shown
*extends unwired: the edges that show the countdown and the beat, left out by the product*

parent: unwired
layer: runtime

> (suite) 2026-10-06T12:01:00
Parity hop 9 (question 54, fm3 log 96): the feature that holds the edges, marked `static off`.

## overview
`out$ << n$ << "\n" forever` wires the stream `unwired` counts down into, and `out$ << beat$ << "\n" forever` the one it beats into. `product.md` marks this feature `static off`, so neither edge is in the program and the cases below do not run; they are what the store would do with the feature in, and `suite/zero/edges` is where they are run.

## interface
- The two edges, and nothing else.

## testing
>count down from (3) → "3\n2\n1\nliftoff"
>beats() → "1\n" at 0 s, "2\n" at 500 ms, "end" at 1 s
