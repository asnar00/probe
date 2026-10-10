# beside
*a clock beside a countdown: a function with a now of its own, and a line with one*

layer: runtime

> (suite) 2026-10-10T17:10:00
Hop 37 (fm3 questions 80 and 121, log 217): a clock going on while a function pushes at a rate.

## overview
`i$` is a clock at `1 hz`, written out as `tick 1`, `tick 2` and on to 5. `count down` pushes `[3 through 1]` into `down$`, another stream at `1 hz`, and then writes `liftoff`. The function has a now of its own, moved on a second by each push; the clock has one too. Neither waits for the other: at each time the one started first has its turn first.

## interface
- `count down` pushes three numbers into `down$` and writes `liftoff`.
- `how far after (k)` pushes `k` numbers into `down$` and gives the clock's latest item.

## rules
- Items are handled in time order, across every stream. At each second the clock's item is written and then the countdown's: the clock's line has stood since the store started and the function was called after.
- The function's last number takes its second, so `liftoff` is at 3 s (question 52); the clock's fourth item is at 3 s too and is first.
- The clock goes on after the function has returned: `tick 5` is at 4 s.
- A clock is read for its latest by its name, as any stream is. After two numbers, two seconds have passed in the function, and the clock's items at 0 s, 1 s and 2 s have all been made: `how far after (2)` gives 3. The item at 2 s is made before the function goes on at 2 s.

## testing
>count down() → "tick 1\n3\n" at 0 s, "tick 2\n2\n" at 1 s, "tick 3\n1\n" at 2 s, "tick 4\nliftoff\n" at 3 s, "tick 5" at 4 s
>how far after (2) → 3
>how far after (1) → 2
