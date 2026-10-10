# alone
*a clock: a line that sets itself off again a beat later*

layer: runtime

> (suite) 2026-10-10T17:00:00
Hop 37 (fm3 question 80, log 217): the smallest program with a clock in it.

## overview
`int i$ at (1 hz)` with `i$ << i$ + 1 if (i$ < 5) forever`. The line stands, and nothing on its right but `i$` itself could set it off; a stream's own name on its own line is a read of its latest item and sets nothing off. So the line is paced by the stream's rate: one more item of `i$` each beat, from the start, for as long as its `if` holds.

## interface
- `so far` gives the clock's latest item.
- `begin` checks that the clock's first item is there, and writes nothing.

## rules
- The first item is at 0 s, the stream's first slot, and each item after it a second later: `1` to `5` at 0 s to 4 s.
- A stream reads as zero before its first item, so the first item is 1.
- The clock's first item is there before the case's function begins, the line having stood since the store started: `so far` gives 1, and `begin`'s check holds.
- At 5 s the `if` fails. Nothing is pushed, and the line is no longer due: the program has no more to do, and the case ends.
- With no `if` the clock would never stop, and a case of it would have no end: it fails a check after ten thousand turns.

## testing
>begin() → "1\n2\n3\n4\n5" at 1 hz
>so far() → 1
