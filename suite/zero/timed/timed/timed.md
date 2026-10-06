# timed
*a task between a stream with a rate and the output: each item is taken at the time it was pushed*

layer: runtime

> (suite) 2026-10-06T11:00:00
Parity hop 8 (questions 39 and 52, fm3 log 93): hello's countdown with `doubled`, the task from `suite/zero/tasks`, between the stream and the output.

## overview
`int i$ at (1 hz)` is a stream at one hertz. `int d$ = doubled(i$)` wires a task to it, and `out$ << d$ << "\n"` wires what the task makes into the output. `count down` pushes `[10 through 1]` into `i$`. Each number is pushed and then its second passes; whatever a consumer does with an item it does at that item's time, so `doubled` doubles the 10 at 0 s and the edge writes `20` at 0 s, the 9 is pushed at 1 s and `18` is written at 1 s, and so on. The rate survives the task.

## interface
- `doubled (x$)` reads a stream and pushes each item doubled.
- `count down` pushes ten numbers into `i$`.
- `launch` counts down and then writes `liftoff`.

## rules
- `i$` is read by a task, with `count`, `peek` and `advance`, so it is stored, a queue; `d$` is the result of a wiring and stored too. The push into `i$` is therefore not the call of an edge, as hello's is: for each item it is the push, then the scheduler's run of the nodes `i$` reaches, `doubled` and then the edge, then a step of the rate.
- An item in a stream with a rate has a length: the last number is pushed at 9 s and `count down` returns at 10 s, so `launch` writes `liftoff` at 10 s (question 52).
- The readers do not wait: `doubled` and the edge run when the item is pushed, and `d$` has no rate of its own.
- Before this was built the ten numbers were pushed in one go and the task took them all at once, so every piece was written at 0 s.

## testing
>count down() → "20\n" at 0 s, "18\n" at 1 s, "16\n" at 2 s, "14\n" at 3 s, "12\n" at 4 s, "10\n" at 5 s, "8\n" at 6 s, "6\n" at 7 s, "4\n" at 8 s, "2" at 9 s
>launch() → "20\n" at 0 s, "18\n" at 1 s, "16\n" at 2 s, "14\n" at 3 s, "12\n" at 4 s, "10\n" at 5 s, "8\n" at 6 s, "6\n" at 7 s, "4\n" at 8 s, "2\n" at 9 s, "liftoff" at 10 s
