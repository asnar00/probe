# going
*two clocks at two rates: two things going on at once, their items in time order*

layer: runtime

> (suite) 2026-10-10T16:00:00
Hop 37 (fm3 questions 80 and 121, log 217): the second half of `schedule.md`. The first program with more than one thing going on.

## overview
`int slow$ at (2 hz)` with `slow$ << slow$ + 1 if (slow$ < 3) forever` is a clock: a line that stands, with nothing on its right but its own stream, so nothing arriving sets it off. It is paced by its stream's rate, one more item each beat, the stream's own name read for its latest. `quick$` is a second, five times a second and counting in tens. Each is written out by a line of its own. Nothing calls either: they go on from the start, each with a now of its own, while the case's function runs and after it has returned.

## interface
- `begin` writes `go`.
- `both` gives the sum of the two clocks' latest items.

## rules
- Items are handled in time order, across every stream (`time.md`, the fourth sentence). `slow$` has an item at 0 s, 500 ms and 1 s, `quick$` at 0 s and every 200 ms to 1 s, and they are written in the order of their times, whichever line is written first.
- Two things due at the same time run in the order they were started (question 121). The lines stand from the start, in the order written, so at 0 s and again at 1 s `slow$`'s item is first and `quick$`'s second; and the case's function, called after the store has started, is after both at 0 s: `go` follows `1` and `10`.
- `if` stops a clock: where it fails the line pushes nothing and is no longer due. `slow$` stops after 3 and `quick$` after 60.
- The program computes its whole timeline: the case's function returns at 0 s and the clocks go on to 1 s, which the case sees.
- A stream's own name on a clock's line is its latest item, and nothing before its first: the first items are 1 and 10.

## testing
>begin() → "1\n10\ngo\n" at 0 s, "20\n" at 200 ms, "30\n" at 400 ms, "2\n" at 500 ms, "40\n" at 600 ms, "50\n" at 800 ms, "3\n60" at 1 s
>both() → 11
