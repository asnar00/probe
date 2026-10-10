# two
*two countdowns at once, neither ended: taken by turns, an item at a time*

layer: runtime

> (suite) 2026-10-10T18:30:00
Hop 37 (fm3 question 135, log 220): the shape the list did not hold. Hop 39 (log 232): it holds it.

## overview
`launch` counts down from 10 at `1 hz` and writes `liftoff`; `launches`, wired over the input device, calls `launch` for each key. There is no `restart`. A key at 3.5 s starts a second countdown while the first is partway through its own.

## interface
- `launch` counts down and writes `liftoff`.
- `launched by (c)` calls `launch` and gives 1; `launches (c$)` pushes that for each character that arrives.

## rules
- Items are handled in time order, across every stream (`time.md`, the fourth sentence): two countdowns each partway through are taken by turns, an item at a time. The first has pushed 10, 9, 8 and 7 when the key arrives at 3.5 s; from 4 s both push into `i$` each second, and the first says `liftoff` at 10 s, the second at 14 s.
- The second countdown's first number waits for `i$`'s next slot, 4 s: there is no `restart`, so the stream's beat is what it was.
- Two things due at the same time run in the order they were started (question 121): in every slot the first countdown's number is written before the second's, `6` and then `10`. That holds when the key arrives at exactly 4 s too: what the key starts waits for its slot like any push, and the countdown started first has its turn first.
- A third may start while two are going on: a second key at 5.5 s gives three numbers a second from 6 s.
- A function that pushes at a rate does not hold the machine's stack here: it is left at each step and gone on with when it is next due, its place and its variables kept in a record of its own (fm3 log 232). The compiler does this only in a store where something that arrives can start such a function; in every other store the function is the loop it was.
- With no key there is one countdown, and it is hello's.

## testing
>launch() with in "k" at 3.5 s → "10\n9\n8\n7\n" at 1 hz, "6\n10\n" at 4 s, "5\n9\n" at 5 s, "4\n8\n" at 6 s, "3\n7\n" at 7 s, "2\n6\n" at 8 s, "1\n5\n" at 9 s, "liftoff4\n" at 10 s, "3\n2\n1\n" at 1 hz from 11 s, "liftoff" at 14 s
>launch() with in "k" at 4 s → "10\n9\n8\n7\n" at 1 hz, "6\n10\n" at 4 s, "5\n9\n" at 5 s, "4\n8\n" at 6 s, "3\n7\n" at 7 s, "2\n6\n" at 8 s, "1\n5\n" at 9 s, "liftoff4\n" at 10 s, "3\n2\n1\n" at 1 hz from 11 s, "liftoff" at 14 s
>launch() with in "k" at 3.5 s, in "k" at 5.5 s → "10\n9\n8\n7\n" at 1 hz, "6\n10\n" at 4 s, "5\n9\n" at 5 s, "4\n8\n10\n" at 6 s, "3\n7\n9\n" at 7 s, "2\n6\n8\n" at 8 s, "1\n5\n7\n" at 9 s, "liftoff4\n6\n" at 10 s, "3\n5\n" at 11 s, "2\n4\n" at 12 s, "1\n3\n" at 13 s, "liftoff2\n" at 14 s, "1\n" at 15 s, "liftoff" at 16 s
>launch() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "liftoff" at 10 s
