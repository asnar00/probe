# clock
*a task ticking at a rate, and a consumer reading `t$ at (m ms)`: the smallest program that needs time at the boundary*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 12 of milestone 0: section 9's `x$ at (t)` with the time computed once at the boundary, over section 10's task at a rate.

## overview
`ticks (5)` pushes 1 to 5 into `t$`, wired at `4 hz` on the store's clock, so the ticks fall at 0, 250, 500, 750 and 1000 ms. `value at (m)` turns the milliseconds into a `time` once, at the boundary, and reads the stream at it by its rule, `nearest`, which its declaration says; `where it stands` moves the feature's reader on two items and reports `position t$`, where the reader is, and `time of t$`, the time of the stream's latest item.

## interface
- `ticks (n)` is the task; `int t$ nearest = ticks(5) at (4 hz)` wires it, and says that a time between two ticks reads the nearer (fm3 question 127, log 250). Until hop forty-three it said nothing and read the nearer; with nothing said a stream now reads the item at or before.
- `value at (m)` is `t$ at (m ms)`.
- `mark thrice` pushes 1, 2 and 3 into `mark$`, a stream with no rate, with a push into `pace$` at `2 hz` between them to move the clock: the case's function starts at 1.25 s, after the five ticks, so the three fall at 1.25 s, 2 s and 2.5 s. `marked at (m)` is `mark$ at (m ms)` after them: 0 before the first; between two the item at or before, `mark$` saying nothing, so 1.8 s reads the item at 1.25 s and 2.3 s the one at 2 s; the last after the last. `paced` reads `pace$` by its name, so that it is a stream of the program. `marked when` is `time of mark$` after the three, 2.5 s, and `none marked when` the same of a stream nothing has been pushed into, `0 s`. **`x$ at (time of x$)` is `x$`'s latest item**: `marked at its own time` gives 3 and `ticked at its own time`, `t$ at (time of t$)`, 5.
- `where it stands` advances `t$` by two and gives `position t$`, the index of the next unread item, and `time of t$`, the time of the stream's latest item whatever the reader has taken, the fifth tick's, 1 s (fm3 question 132; until hop forty-three it was the next unread item's, 500 ms); `position` asks no time and `time of` is what times the stream (log 85, question 42).

## rules
- Inside the stream time is the ring's ticks; at the boundary it is `time`, exact seconds (section 9). `m ms` on a variable is `millis(m)` computed once per call (log 33).
- Between two ticks of `t$` the nearest wins, as it is declared: 300 ms reads the item at 250 ms, 400 ms the one at 500 ms.
- After the last item the stream holds its last value; before the first it reads as zero, as its name does before anything is pushed (fm3 question 127): `value at (-10)` is 0, and so is `none marked at (10)`, a stream nothing has been pushed into. Until hop forty-three each of those two was a failed check.
- A read at a time is worked out on indices (fm3 question 120, log 248): the time's count times a ratio the compiler has reduced, one multiply and at most one divide, no divisor at run time. `t$` is wired at feature scope and `mark$` has no rate: their items carry the ticks of the store's clock, a microsecond, so `m ms` becomes a tick, `m * 1000000 / 1000`, and the item at or before it is found, by arithmetic for `t$` and by halving over the ticks `mark$` keeps, at most seven rounds for sixty-four items. `value at (250)` is 248 as counted where it was 3 213.

## testing
>value at (0) → 1
>value at (250) → 2
>value at (300) → 2
>value at (400) → 3
>through (400) → 3
>through (300) → 2
>value at (5000) → 5
>where it stands() → 2, 1 s
>ticked at its own time() → 5
>marked when() → 2.5 s
>marked at its own time() → 3
>none marked when() → 0 s
>marked at (0) → 0
>marked at (1300) → 1
>marked at (1600) → 1
>marked at (1800) → 1
>marked at (2300) → 2
>marked at (9000) → 3
>marked at (-10) → 0
>none marked at (10) → 0
>value at (-10) → 0
>paced() → 0

## hostile
`t$ at (m)` without a unit is refused: "'x$ at (t)' takes a time, `1500 us`, `2 ms`, `1 s`". `value at (m)` with `m` a float parameter is refused: "'ms' takes a whole number, given a float".
