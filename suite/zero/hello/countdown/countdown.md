# countdown
*extends hello: a countdown from 10 at one hertz before the greeting*

parent: hello
layer: runtime

> (suite) 2026-09-08T10:01:00
Section 16's `feature Countdown extends Hello`: `count down()` then `existing run()`.

## overview
Before hello says hello, `count down` prints 10 to 1 on one line, one number a second on the store's clock, which the suite moves as fast as it can.

## interface
- `run` counts down, then does what it did before.
- `count down` wires section 10's task `count down from (10)` at `1 hz` and prints the frame it produced.
- `count down from (n)` is the task: n, n − 1, ... 1, pushing while the candidate `_` is above zero.

## rules
- Section 16 writes `print [10 through 1] at (1 hz)`; here the rate is on the task's wiring and `print` takes the frame, one line with a space between the numbers (log 28).
- With `countdown.enabled` false, `run` falls through to hello's.
- `bound 10` on the task's chain is section 7's declared trip count on the repeated push, the number section 16 counts from; it goes onto the IR's `loop() bound 10 {`, trusted, not checked, so `probe cost` on `run` counts the countdown as ten passes (log 33).

## testing
>count down() → "10 9 8 7 6 5 4 3 2 1"

## hostile
`count down` after the clock has moved still prints the same numbers: the ticks are the clock's, the values the task's.
