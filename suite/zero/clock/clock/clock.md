# clock
*a task ticking at a rate, and a consumer reading `t$ at (m ms)`: the smallest program that needs time at the boundary*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 12 of milestone 0: section 9's `x$ at (t)` with the time computed once at the boundary, over section 10's task at a rate.

## overview
`ticks (5)` pushes 1 to 5 into `t$`, wired at `4 hz` on the store's clock, so the ticks fall at 0, 250, 500, 750 and 1000 ms. `value at (m)` turns the milliseconds into a `time` once, at the boundary, and reads the stream at it by its rule, nearest; `where it stands` moves the feature's reader on two items and reports where it is, `position t$` for the index and `time of t$` for the tick.

## interface
- `ticks (n)` is the task; `int t$ = ticks(5) at (4 hz)` wires it.
- `value at (m)` is `t$ at (m ms)`.
- `where it stands` advances `t$` by two and gives `position t$`, the index of the next unread item, and `time of t$`, its tick in microseconds; `position` asks no time and `time of` is what times the stream (log 85, question 42).

## rules
- Inside the stream time is the ring's ticks; at the boundary it is `time`, exact seconds (section 9). `m ms` on a variable is `millis(m)` computed once per call (log 33).
- Between two ticks the nearest wins: 300 ms reads the item at 250 ms, 400 ms the one at 500 ms.
- After the last item the stream holds its last value; before the first, a sample fails a check.

## testing
>value at (0) → 1
>value at (250) → 2
>value at (300) → 2
>value at (400) → 3
>value at (5000) → 5
>where it stands() → 2, 500000

## hostile
`t$ at (m)` without a unit is refused: "'x$ at (t)' takes a time, `1500 us`, `2 ms`, `1 s`". `value at (m)` with `m` a float parameter is refused: "'ms' takes a whole number, given a float".
