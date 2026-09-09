# countdown
*extends hello: a countdown from 10 at one hertz before the greeting*

parent: hello
layer: runtime

> (suite) 2026-09-08T10:01:00
Section 16's `feature Countdown extends Hello`: `count down()` then `existing run()`.

## overview
Before hello says hello, `count down` prints 10 to 1 on one line: a stream at one hertz, the range `[10 through 1]` pushed into it, so the ten numbers sit one second apart on the stream's clock.

## interface
- `run` counts down, then does what it did before.
- `count down` declares `int i$ at (1 hz)`, pushes `[10 through 1]` into it, and prints the frame.

## rules
- Section 16 writes `print [10 through 1] at (1 hz)`; here the rate is on the stream's declaration, the range is pushed into it, and `print` takes the frame, one line with a space between the numbers (log 28, 41).
- With countdown off (`>run() with countdown off`), `run` falls through to hello's.
- `run() → "10 9 8 7 6 5 4 3 2 1\nhello world"` overrides hello's `run() → "hello world"` wherever this feature is on, and is itself overridden by bye's; it stands, and passes, in the runner's context with `bye` off.
- The count is in the code as the range's literal bounds, so `probe cost` on `run` counts the countdown's loop as ten passes without a bound anywhere: a bound is a product setting, never a word in feature code (question 19, log 41).

## testing
>count down() → "10 9 8 7 6 5 4 3 2 1"
>run() → "10 9 8 7 6 5 4 3 2 1\nhello world"

## hostile
`count down` after the clock has moved still prints the same numbers: the ticks are the clock's, the values the task's.
