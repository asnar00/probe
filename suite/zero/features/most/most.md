# most
*extends base: `greet` after again, `count (k)` twice under a condition, `describe` replaced; and the cases that say what each context gives*

parent: base
layer: runtime

> (suite) 2026-09-08T10:02:00
Plan item 9: the newest is outermost; a redefinition that calls `existing` twice or under a condition is whatever it says; one that never calls it is a replacement; a feature that is off falls its link through and keeps its state. Rulings pass item 9: a case names the context it runs in, `with <feature> off`, and no code switches a feature. Rulings pass item 10: the runner runs every case in every context it stands in, and a case a context changes is overridden there by a case that names that context.

## overview
`most` is the outermost: `greet` is (base's 1 + more's 10) × 100, `count (k)` doubles more's result for k over 5 and is k otherwise, and `describe` replaces base's. The contexts: a case `with more off` runs with more's link dropped from every chain, so `visit` never reaches its `seen`, which keeps its value; `with most off` drops this feature's `describe` and the tool's `existing` reaches base's; `with tool off` leaves `describe` at this feature's 20; and `with base off` switches off base and, with it, every feature under base — `more`, `most` and `tool` are its children — so every link of `greet` is off and the chain gives zero, though only base's own field is written (log 44, 51).

## interface
- `greeted`, `counted (k)`, `described` call the chains; `describe` is `tool`'s by then, the newest being outermost.
- `visited twice` visits twice and reads both counters, `seen` then `visits`.
- `switch of more` reads another feature's switch; `own switch` reads this feature's; `switches` reads both.

## rules
- Features compose in provenance order: base, more, most; `greet` is most's, whose `existing` is more's, whose `existing` is base's.
- A feature off falls through at the next call; its variables keep their values, and on again it continues from them — the half a case line cannot show, since a case runs in one context, and a context switched inside a call waits for contexts as values (section 14).
- The innermost definition, off, gives the results' zeros.
- Every feature is on unless the case line says `with <feature> off`; the runner sets the context after the reset and before the call (log 43). A feature off takes its descendants with it.
- A feature's switch is its own field, and a feature is on when its own switch and every ancestor's are: the gate reads that conjunction, `__on_most` in the IR, and switching a feature never writes its descendants' fields. So `>switches() with more off, base off, base on → 0, 1` — the case line is a sequence of switches — leaves `more` off under `base` after `base` has been off and on again, and `most` on, as it was (section 14, log 51).
- The runner also runs each case with every other feature off alone, where this feature stays on: `greeted() → 1100` runs with `tool` off (still 1100) and would run with `more` off, but there the line `greeted() with more off → 100` makes the same call and stands instead, being the more specific: a case overridden in a context is reported as `over` and not run (section 14, log 44). So every plain case here holds with `tool` off, and where `more` off changes a result the line that says so is beside it.

## testing
>greeted() → 1100
>counted (3) → 3
>counted (10) → 40
>counted (10) with more off → 20
>described() → 1020
>described() with tool off → 20
>greeted() with more off → 100
>greeted() with base off → 0
>described() with most off → 1010
>described() with most off, tool off → 10
>visited twice() → 2, 2
>visited twice() with more off → 0, 2
>switch of more() → 1
>switch of more() with more off → 0
>own switch() → 1
>own switch() with most off → 0
>switches() → 1, 1
>switches() with more off, base off, base on → 0, 1

## hostile
`on (int n) = greet (int k)` here is refused: "'greet' clashes with a function of feature base that mangles to the same name". `on (bool n) = greet()` is refused: "'greet' redefines feature more's with different parameters or results: a redefinition keeps the signature". `most.other = 1` is refused: "'most.other': a feature's implicit variable is `most.enabled`". `more.enabled = false` in a body is refused: "'more.enabled' is not assigned in feature code: a feature is switched by the chooser, and a case says `with more off` on its line (section 14)". A case `>greeted() with nobody off → 1` is refused: "`with nobody off`: no feature named 'nobody' in the store"; `>greeted() with more → 1`: "a case's context is `with <feature> off` or `on`, several joined by commas"; `>greeted() → 1100` written twice in this feature is refused when the runner plans the store, "most.md:N and line M both claim `greeted()` in one context (every feature on): one case per call per context"; `>greeted() with tool off → 1100` beside it is not, since the line naming `tool` is the more specific and stands where the runner tries `tool` off. `int x$ = count up to (3)` is refused as before: a task is not redefined either.
