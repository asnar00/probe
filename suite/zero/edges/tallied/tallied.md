# tallied
*extends edges: a sink over a stored stream, in a feature that may be off*

parent: edges
layer: runtime

> (suite) 2026-10-06T10:02:00
Parity hop 8 (question 51, fm3 log 92): a consumer that is off holds nothing.

## overview
`keep(raw$)` wires a sink to `raw$`, the stream `edges`' `fed` pushes into. `raw$` is read with `count`, `peek` and `advance`, so it is a queue, and `keep` is a node the scheduler runs after each push.

## interface
- `keep (x$)` adds every item to `kept$`.
- `fed (k)` does what it did before, then gives `kept$`.
- `kept from (k)` pushes the numbers from `k` down to 2 in one statement, a range that may hold nothing, and gives `kept$`.
- `soak (x$)` adds every item to `soaked$`, and is wired to a stream that is also called `x$`; `soaked from (k)` pushes 1 to `k` into it, one a statement, and gives `soaked$`.

## rules
- With this feature on, `fed (80)` gives the sum of 1 to 80, 3 240: the sink runs after each statement's push and the queue's slot comes back.
- With it off, `edges`' own case stands, `fed (80) → 80`: eighty items are pushed toward a consumer that is off and none is held, so the queue of 64 never fills.

- A queue is freed by who reads it, not by how a parameter is spelled (parity hop 13, fm3 log 113): `soak`'s parameter is `x$` and so is the stream it is wired to, and `soaked from (100)` puts a hundred items through a queue of 64, the slot coming back after each. Until then the parameter's name was taken for a second reader of the feature's `x$`, the queue never freed, and the sixty-fifth push failed the library's check.

- Parity hop 11 (fm3 log 103): `keep` is a node its pushers wake. `raw$` is pushed into only by plain functions that nothing the scheduler runs can reach, so a push into it calls `keep` there, under this feature's gate, and asks nothing; where the statement may have pushed no item, as `raw$ << [k to 1]` with `k` at 1, the call stands under whether `raw$` received anything, so a sink is never run over nothing.

## testing
>fed (80) → 3240
>kept from (4) → 9
>kept from (1) → 0
>soaked from (100) → 5050
