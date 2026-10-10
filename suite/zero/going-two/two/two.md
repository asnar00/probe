# two
*two countdowns at once, neither ended: what is not held, and stops*

layer: runtime

> (suite) 2026-10-10T18:30:00
Hop 37 (fm3 question 135, log 220): the shape the list does not hold.

## overview
`launch` counts down from 10 at `1 hz` and writes `liftoff`; `launches`, wired over the input device, calls `launch` for each key. There is no `restart`. A key at 3.5 s starts a second countdown while the first is partway through its own.

## interface
- `launch` counts down and writes `liftoff`.
- `launched by (c)` calls `launch` and gives 1; `launches (c$)` pushes that for each character that arrives.

## rules
- A function that pushes at a rate holds its place on the machine's stack, and only the function on top can go on. The second countdown runs to its end above the first, its now passing the first's next time.
- The first then cannot go on at the time it was due, and the program stops there, a failed check: it does not write a timeline that is out of order.
- With no key there is one countdown, and it is hello's.

## testing
>launch() with in "k" at 3.5 s → check
>launch() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "liftoff" at 10 s
