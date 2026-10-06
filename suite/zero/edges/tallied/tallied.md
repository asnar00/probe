# tallied
*extends edges: a sink over a stored stream, in a feature that may be off*

parent: edges
layer: runtime

> (suite) 2026-10-06T10:02:00
Parity hop 8 (question 51, fm3 log 92): a consumer that is off holds nothing.

## overview
`keep(raw$)` wires a sink to `raw$`, the stream `edges`' `fed` pushes into. `raw$` is read with `count`, `peek` and `advance`, so it is a queue, and `keep` is a node the scheduler runs after each push.

## interface
- `keep (x$)` adds every item to `kept`.
- `fed (k)` does what it did before, then gives `kept`.

## rules
- With this feature on, `fed (80)` gives the sum of 1 to 80, 3 240: the sink runs after each statement's push and the queue's slot comes back.
- With it off, `edges`' own case stands, `fed (80) → 80`: eighty items are pushed toward a consumer that is off and none is held, so the queue of 64 never fills.

## testing
>fed (80) → 3240
