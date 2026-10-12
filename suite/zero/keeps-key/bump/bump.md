# bump
*a program that keeps something across a key: a clock, a running sum, and a key that adds to the sum by how many beats there have been since the last key*

layer: runtime

> (suite) 2026-10-12T22:00:00
Hop 48 (fm3 cut.md, "working ahead of a key"; log 273). `restart` keeps nothing a key changes but which run it is. This store does, so that a place that has worked ahead of a key, and is not put back to what it was when the key was pressed, shows as a wrong number and not only as a wrong order.

## overview
`int beat$ at (1 hz)` with `beat$ << beat$ + 1 if (beat$ < 6) forever` is a clock: six beats, 1 to 6, a second apart. `sum$ << sum$ + beat$ forever` is their running sum, a stream kept as one value, and a line writes it out at each beat. `log$ << beat$ forever` keeps every beat, a stored stream. `int seen$ = bumps (in$)` wires a function over the input: for each key, `bumped by` takes what `log$` has had since the last key, `frame log$`, and adds a hundred for each to the sum, which the line writes out at the key's own time.

## interface
- `begin` writes `go`.
- `so far` gives the sum.
- a key adds to the sum a hundred times the number of beats since the last key.

## rules
- With no key the sums are 1, 3, 6, 10, 15, 21, one a second.
- A key at 2.25 s arrives when three beats have been: the sum, 6, becomes 306 at 2.25 s, and the beats after it add to that, 310, 315, 321.
- A second key at 4.25 s finds two beats since the first, and adds two hundred: 315 becomes 515.
- What is kept across a key is the clock's latest beat and when it is next due, the sum, and the beats `log$` holds with how far its reader has read. Run at two places over a wire that takes time (`probe zero suite/zero/keeps-key run 'begin() with in "k" at 2.25 s' --far all --wire 100 ms`), the place the lines run at works every beat out before the key is pressed, six beats and a sum of 21, and when the key reaches it goes back to what it was at 2 s: three beats, a sum of 6. Not put back, it would add six hundred to 21.

## testing
>begin() → "1\ngo\n" at 0 s, "3\n6\n10\n15\n21" at 1 hz from 1 s
>begin() with in "k" at 2.25 s → "1\ngo\n" at 0 s, "3\n" at 1 s, "6\n" at 2 s, "306\n" at 2.25 s, "310\n315\n321" at 1 hz from 3 s
>begin() with in "k" at 2.25 s, in "k" at 4.25 s → "1\ngo\n" at 0 s, "3\n" at 1 s, "6\n" at 2 s, "306\n" at 2.25 s, "310\n" at 3 s, "315\n" at 4 s, "515\n" at 4.25 s, "521" at 5 s
>so far() → 1
