# kept
*a read by a place cannot fail, in a store that keeps a ring*

layer: runtime

> (suite) 2026-10-10T10:00:00
fm3 question 127, log 236. `suite/zero/sampled` is the notion; this is the same read where the store's streams are rings.

## overview
`x$ behind (3)` asks for what a reader has passed, so every stream of this store is kept as a ring and not a queue, and an array, which the compiler keeps as a stream is kept, is a ring too. A read by a place is the same lines but for where the items lie: the buffer's header is stepped over. Nothing a ring does for a stream that is pushed round, its residency and its remainder by half the buffer, is done for an array, which is given its items once.

## interface
- `item (i)`, `or seven (i)`, `kicked (i)`, `waltzed (i)` and `letter (i)` are `sampled`'s.
- `earlier (i)` reads the three items before a reader that has passed four of five, an array that `behind` made, through a parameter that says `wrapped` and by its own name, which says nothing.

## testing
>item (2) → 30
>item (4) → 0
>item (-1) → 0
>or seven (0) → 1
>or seven (2) → 7
>kicked (30) → 1
>kicked (-1) → 0
>waltzed (17) → 0
>waltzed (-1) → 1
>earlier (1) → 33
>earlier (4) → 30
>earlier (-1) → 40
>letter (0) → 108
>letter (7) → 0
