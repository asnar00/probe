# keeps
*a program that keeps something from one second to the next: a clock, a running sum, a look back, and a function partway through a count*

layer: runtime

> (suite) 2026-10-12T12:00:00
Hop 46 (fm3 cut.md, "the cut, moved"; log 265). hello keeps nothing between two numbers: its stream has no storage and its count is the loop's own. This store is the smallest that does, so that a cut moved while it runs has something to hand across, and a thing forgotten at the move shows as a wrong number.

## overview
`int beat$ at (1 hz)` with `beat$ << beat$ + 1 if (beat$ < 4) forever` is a clock: four beats, 1 to 4, a second apart. `sum$ << sum$ + beat$ forever` is a running sum of them, a stream kept as one value, added to at each beat. `int rise$ = rising (sum$)` is how much the sum rose by, a stream function that looks back one item, `x$ - x$[-1]`. Two lines write them out, the sum and then its rise. Beside them `n$` is a second stream with a rate, pushed into by a function, `count to`, which is partway through its count for as many seconds as it is given; `total$` is the running sum of what it pushed.

## interface
- `begin` writes `go`.
- `count to (k)` pushes 1 to `k` into `n$`, one a second, and then writes their total.
- `so far` gives the sum.

## rules
- A stream's own name on the line that pushes into it is its latest item, and nothing before its first: the sums begin at 1.
- A line runs after what it reads, and two that read the same stream in the order written: the sum is written before its rise, the line that wires `rise$` being written after the line that writes `sum$`.
- What is kept from one second to the next is five things: the clock's latest beat and the time it is next due; the sum; the item `rising` looks back to; the total; and where `count to` has got to. Run at one place they are words of one context. Run at two, with the cut moved partway (`probe zero suite/zero/keeps run 'count to (5)' --fast --far all --then none at 2.5 s`), each is at the place its line ran at and is handed across once at the move, and the case gives the same line.

## testing
>begin() → "1 1\ngo\n" at 0 s, "3 2\n6 3\n10 4" at 1 hz from 1 s
>count to (5) → "1 1\n3 2\n6 3\n10 4\n" at 1 hz, "15" at 5 s
>so far() → 1
