# base
*the root feature: the functions the others redefine, and a variable*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 9 of milestone 0: section 12 of zero.md — the store composed in provenance order, redefinition with `existing`, the implicit `enabled` per feature gating each link, a feature switched off at the next event with its state kept, `parent` and `layer` checked.

## overview
`base` is the root: `greet`, `count (k)` and `describe` are the functions later features redefine, `base only` is one they leave alone, and `visits` counts calls of `visit`.

## interface
- `greet` gives 1; `count (k)` gives k; `describe` gives 10.
- `base only` gives 7 whatever else is on.
- `visit` adds one to `visits`; `visited` reads it.

## rules
- A function defined once is called directly and is not gated.

## testing
>base only() → 7

## hostile
A second `on greet()` in this same feature is refused: "'greet' is defined twice in feature base".
