# most
*extends base: `greet` after again, `count (k)` twice under a condition, `describe` replaced; and the cases that run with a feature off*

parent: base
layer: runtime

> (suite) 2026-09-08T10:02:00
Plan item 9: the newest is outermost; a redefinition that calls `existing` twice or under a condition is whatever it says; one that never calls it is a replacement; a feature that is off falls its link through and keeps its state. Rulings pass item 9: a case names the context it runs in, `with <feature> off`, and no code switches a feature.

## overview
`most` is the outermost: `greet` is (base's 1 + more's 10) × 100, `count (k)` doubles more's result for k over 5 and is k otherwise, and `describe` replaces base's. The contexts: a case `with more off` runs with more's link dropped from every chain, so `visit` never reaches its `seen`, which keeps its value; `with base off` leaves the innermost link off, which gives zero; `with most off` drops this feature's `describe` and the tool's `existing` reaches base's.

## interface
- `greeted`, `counted (k)`, `described` call the chains; `describe` is `tool`'s by then, the newest being outermost.
- `visited twice` visits twice and reads both counters, `seen` then `visits`.
- `switch of more` reads another feature's switch; `own switch` reads this feature's.

## rules
- Features compose in provenance order: base, more, most; `greet` is most's, whose `existing` is more's, whose `existing` is base's.
- A feature off falls through at the next call; its variables keep their values, and on again it continues from them — the half a case line cannot show, since a case runs in one context, and a context switched inside a call waits for contexts as values (section 14).
- The innermost definition, off, gives the results' zeros.
- Every feature is on unless the case line says `with <feature> off`; the runner sets the context after the reset and before the call (log 43).

## testing
>greeted() → 1100
>counted (3) → 3
>counted (10) → 40
>described() → 1020
>greeted() with more off → 100
>greeted() with base off → 1000
>described() with most off → 1010
>visited twice() → 2, 2
>visited twice() with more off → 0, 2
>switch of more() → 1
>switch of more() with more off → 0
>own switch() → 1
>own switch() with most off → 0

## hostile
`on (int n) = greet (int k)` here is refused: "'greet' clashes with a function of feature base that mangles to the same name". `on (bool n) = greet()` is refused: "'greet' redefines feature more's with different parameters or results: a redefinition keeps the signature". `most.other = 1` is refused: "'most.other': a feature's implicit variable is `most.enabled`". `more.enabled = false` in a body is refused: "'more.enabled' is not assigned in feature code: a feature is switched by the chooser, and a case says `with more off` on its line (section 14)". A case `>greeted() with nobody off → 1` is refused: "`with nobody off`: no feature named 'nobody' in the store"; `>greeted() with more → 1`: "a case's context is `with <feature> off` or `on`, several joined by commas". `int x$ = count up to (3)` is refused as before: a task is not redefined either.
