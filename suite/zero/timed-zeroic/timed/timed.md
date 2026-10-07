# timed
*a stream processor with no loop in it between a stream with a rate and the output: each item is taken at the time it was pushed, and nothing between is stored*

layer: runtime

> (suite) 2026-10-06T12:00:00
Parity hop sixteen, transformation 64 (fm3 log 130): `suite/zero/timed` with its task written the zeroic way, in one line, and every other line and every case as it is there.

## overview
`int i$ at (1 hz)` is a stream at one hertz. `int d$ = doubled(i$)` wires a stream processor to it, and `out$ << d$ << "\n" forever` wires what it makes into the output. `doubled` is one line, `d$ << x$ * 2`: twice each item, for every item that arrives. `count down` pushes `[10 through 1]` into `i$`; each number is pushed and then its second passes, so `20` is written at 0 s, `18` at 1 s, and on to `2` at 9 s. The rate survives the processor, as in `suite/zero/timed` it survives the task.

## interface
- `doubled (x$)` pushes twice each item of its input.
- `count down` pushes ten numbers into `i$`.
- `launch` counts down and then writes `liftoff`.
- `late` pushes one item into `half$`, a stream at `2 hz` wired straight to the output, then one into `i$`, then writes `done`.
- `one` pushes one item into `i$`; `later` pushes one into `half$`, calls `one`, and writes `done`.

## rules
- Nothing reads `i$` but the processor wired to it, and nothing reads `d$` but the edge into the output, so neither has storage (fm3 questions 50 and 75): a push into `i$` is the processor's function called with the item, whose push into `d$` is the edge's function, which writes the number and its newline; then a step of `i$`'s rate passes. There is no queue, no node and no scheduler in the lowered store, where `suite/zero/timed` has two queues and two nodes.
- The cases are those of `suite/zero/timed`, word for word, with the same times: an item still lands on its stream's beat, `late` writing `8` at 1 s and `done` at 2 s, and `one`, reached once on the beat and once off it, still finds `i$`'s next slot.
- `count down` costs 965 on `probe cost` where `suite/zero/timed`'s costs 2 440, and the lowered text is 276 lines where that is 406, comments and blanks apart.

## testing
>count down() → "20\n18\n16\n14\n12\n10\n8\n6\n4\n2" at 1 hz
>launch() → "20\n18\n16\n14\n12\n10\n8\n6\n4\n2\n" at 1 hz, "liftoff" at 10 s
>late() → "0\n" at 0 s, "8\n" at 1 s, "done" at 2 s
>one() → "6" at 0 s
>later() → "0\n" at 0 s, "6\n" at 1 s, "done" at 2 s
