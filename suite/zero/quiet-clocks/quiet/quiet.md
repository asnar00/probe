# quiet
*a clock whose stream no line reads, and one a function reads by its name: each is a clock all the same*

layer: runtime

> (suite) 2026-10-10T16:00:00
fm3 question 80, log 217 and log 255. A clock is a line paced by its own stream's rate, `tick$ << tick$ + 1 if (tick$ < 3) forever` over `int tick$ at (1 hz)`: one of the things going on, called once a beat until its `if` fails.

## overview
`tick$` is read by nothing: no line is wired to it and no function names it. `beat$` is read by a function, by its name. Until hop forty-four both were refused, "'tick$' has no rate", the compiler having looked for the rate only where a stream has no storage and something is wired to it. A clock's rate is the one its stream is declared with, whatever the stream is kept as.

## interface
- `run()` writes `go`.
- `read()` gives `beat$` as the call finds it.
- `read after()` lets two seconds pass, pushing two items into `wait$` at `1 hz`, and then gives `beat$`.
- `waited()` does the same and writes it.

## rules
- **A clock ticks whether or not anything reads its stream.** `tick$` goes 1 at 0 s, 2 at 1 s, 3 at 2 s, and at 3 s its `if` fails and the line is over. Nothing shows it but the time: `probe zero suite/zero/quiet-clocks trace "read after()"` has a row for each tick, `tick$` in its column.
- **A clock's stream read by its name is its latest item.** The first tick is when the store starts, so `read()` is 1. `beat$` at `2 hz` goes 1, 2, 3, 4 at 0 s, 500 ms, 1 s and 1.5 s; `read after()`, two seconds on, is 4.
- The clocks have their turns in the step of another stream's rate: `waited()` writes `7` at 0 s, `8` at 1 s and then `4` at 2 s.

## testing
>run() → "go"
>read() → 1
>read after() → 4
>waited() → "7\n" at 0 s, "8\n" at 1 s, "4" at 2 s

## hostile
A clock over a stream with no rate is refused as before: "a push into 'tick$' that reads 'tick$' and stands forever would never end: nothing else on its right paces it, and 'tick$' has no rate to".
