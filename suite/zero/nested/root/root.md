# root
*the root of a chain of features three deep*

layer: runtime

> (suite) 2026-10-07T10:00:00
fm3 question 72: a feature's gate is one load and one branch however deeply the feature is nested. `root` has `one` under it, `one` has `two`, `two` has `three`; `watch` stands beside `one` and reads the three switches.

## overview
`reach` is the function each level redefines, adding its own place: `root` gives 1.

## interface
- `reach` gives 1.

## rules
- A feature is on when its own switch and every ancestor's are. The context holds that for each nested feature, already worked out, and it is worked out again only when a switch is written.

## testing
