# most
*extends base: `greet` after again, `count (k)` twice under a condition, `describe` replaced; and the switches*

parent: base
layer: runtime

> (suite) 2026-09-08T10:02:00
Plan item 9: the newest is outermost; a redefinition that calls `existing` twice or under a condition is whatever it says; one that never calls it is a replacement; `enabled` falls a link through at the next call and keeps the feature's state.

## overview
`most` is the outermost: `greet` is (base's 1 + more's 10) × 100, `count (k)` doubles more's result for k over 5 and is k otherwise, and `describe` replaces base's. The switches: `more.enabled = false` drops more's link from every chain and stops `visit` reaching its `seen`, which keeps its value; `base.enabled = false` leaves the innermost link off, which gives zero; `most.enabled = false` drops this feature's `describe` and the tool's `existing` reaches base's.

## interface
- `greeted`, `counted (k)`, `described` call the chains; `describe` is `tool`'s by then, the newest being outermost.
- `greeted without more`, `greeted without base`, `described without most` call them with a feature off.
- `state kept when off` visits with `more` on, off and on again, and reads both counters.
- `switch read` reads another feature's switch; `own switch` reads this feature's.

## rules
- Features compose in provenance order: base, more, most; `greet` is most's, whose `existing` is more's, whose `existing` is base's.
- A feature off falls through at the next call; its variables keep their values; on again, it continues from them.
- The innermost definition, off, gives the results' zeros.
- Every feature starts on at every case.

## testing
>greeted() → 1100
>counted (3) → 3
>counted (10) → 40
>described() → 1020
>greeted without more() → 100
>greeted without base() → 1000
>described without most() → 1010
>state kept when off() → 3, 4
>switch read() → 0
>own switch() → 1

## hostile
`on (int n) = greet (int k)` here is refused: "'greet' clashes with a function of feature base that mangles to the same name". `on (bool n) = greet()` is refused: "'greet' redefines feature more's with different parameters or results: a redefinition keeps the signature". `most.other = 1` is refused: "'most.other': a feature's implicit variable is `most.enabled`". `int x$ = count up to (3)` is refused as before: a task is not redefined either.
