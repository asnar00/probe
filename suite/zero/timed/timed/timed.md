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
- `late` pushes one item into `half$`, a stream at `2 hz` wired straight to the output, then one into `i$`, then writes `done`.
- `one` pushes one item into `i$`; `later` pushes one into `half$`, calls `one`, and writes `done`.

## rules
- `i$` is read by a task, with `count`, `peek` and `advance`, so it is stored, a queue; `d$` is the result of a wiring and stored too. The push into `i$` is therefore not the call of an edge, as hello's is: for each item it is the push, then the scheduler's run of the nodes `i$` reaches, `doubled` and then the edge, then a step of the rate.
- An item in a stream with a rate has a length: the last number is pushed at 9 s and `count down` returns at 10 s, so `launch` writes `liftoff` at 10 s (question 52).
- The readers do not wait: `doubled` and the edge run when the item is pushed, and `d$` has no rate of its own.
- Before this was built the ten numbers were pushed in one go and the task took them all at once, so every piece was written at 0 s.
- An item lands on its stream's beat (question 52, fm3 log 98): `i$` has a slot every second from 0 s. `late` writes `0` at 0 s through `half$`, which takes half a second, so it pushes `4` into `i$` at 0.5 s; the item lands in `i$`'s next slot, at 1 s, where `doubled` and the edge write `8`; the slot ends at 2 s, and `done` is written then. Before the beat was built `8` was written at 500 ms and `done` at 1.5 s.
- The compiler knows what the clock is a whole multiple of at each point in a function (question 56, fm3 log 99), and a push that is on its stream's beat by that is not checked: `count down` is only ever called where the clock is a multiple of a second, so it rounds nothing. `one` is called by a case at 0 s, where its `6` is written at once, and by `later` after the push into `half$` has left the clock at 0.5 s; one of its callers is off the beat, so `one` still finds `i$`'s next slot, and `later` writes `6` at 1 s and `done` at 2 s. A compiler that looked only at the case's call would write it at 500 ms.
- The cases give the numbers `at 1 hz`: the lines of the text, one a second from 0 s (question 53). The last piece of a result is written without its final newline, as a plain text result is, so `count down()`'s ends `2` and `launch()`'s ends `2\n` before `liftoff`.

## testing
>count down() → "20\n18\n16\n14\n12\n10\n8\n6\n4\n2" at 1 hz
>launch() → "20\n18\n16\n14\n12\n10\n8\n6\n4\n2\n" at 1 hz, "liftoff" at 10 s
>late() → "0\n" at 0 s, "8\n" at 1 s, "done" at 2 s
>one() → "6" at 0 s
>later() → "0\n" at 0 s, "6\n" at 1 s, "done" at 2 s
