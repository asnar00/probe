# unwired
*a stream that is pushed into and that nothing in the program reads or wires: no storage, and a push into it is nothing but the step of its rate*

layer: runtime

> (suite) 2026-10-06T12:00:00
Parity hop 9 (question 54, fm3 log 96): a feature marked `static off` and the same feature switched off are one program.

## overview
This is `suite/zero/edges`' countdown and its beat with the feature that shows them, `shown`, marked `static off` in `product.md`. The product leaves `shown` out, so in the program that is compiled nothing reads `n$` or `beat$` and nothing wires them: they are only pushed into. Such a stream has no storage. A push into `n$` goes nowhere, and a push into `beat$`, which is at `2 hz`, is the half second each item lasts and nothing else.

That is exactly what `edges` does with `shown` switched off at run time, and it is the point: whether a feature is compiled out or switched off, a reader sees the same program.

## interface
- `count down from (k)` pushes `k` down to 1 into `n$` and then writes `liftoff`.
- `beats` pushes two items into `beat$`, a stream at `2 hz`, and then writes `end`.

## rules
- With nobody wired to `n$`, a countdown from 80 writes `liftoff` alone, as one from 3 does. With a queue behind `n$` the sixty-fifth push failed a check.
- A stream at a rate keeps its time whether or not anyone listens: `end` is written at 1 s, after two items of half a second each. With a queue behind `beat$` it was written at 0 s.
- The compiler knows the difference between a stream whose reader is compiled out and a stream nobody ever reads. `shown` is not in the program, but it is in the store, and it wires both streams. Had no feature of the store read or wired `n$`, compiled in or left out, the store would be refused: "'n$' is pushed into and nothing reads it or wires it, in any feature of the store, compiled in or left out: a mistyped name?".

## testing
>count down from (80) → "liftoff"
>beats() → "end" at 1 s
