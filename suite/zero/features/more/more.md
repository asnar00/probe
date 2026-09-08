# more
*extends base: `greet` after, `count (k)` after, `visit` before*

parent: base
layer: runtime

> (suite) 2026-09-08T10:01:00
Plan item 9: a redefinition that calls `existing` first runs after the earlier definition; one that calls it last runs before.

## overview
`more` redefines `greet` to add 10 to what came before, `count (k)` to double it, and `visit` to note the visit in `seen` before passing it on.

## interface
- `greet` gives base's 1 plus 10.
- `count (k)` gives twice base's k.
- `visit` adds one to `seen`, then to `visits`.
- `seen by more` reads `seen`.

## rules
- `existing greet()` inside `greet` calls the definition before this feature's.

## testing
>seen by more() → 0

## hostile
`existing greet()` inside `count (k)` is refused: "'existing' names the function it is in: `existing count(...)`". `existing greet()` in base's own `greet` is refused: "no earlier definition of 'greet' for 'existing' to call".
