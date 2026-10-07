# three
*under `two`: adds 1000 to `reach`*

parent: two
layer: runtime

> (suite) 2026-10-07T10:03:00
A level of the chain: `three` is on when its own switch, `two`'s and every switch above are.

## overview
`reach` is what came before plus 1000.

## interface
- `reach` gives the earlier definition's result plus 1000.

## rules
- `three` off, or anything above it off, and the chain falls through to the definition before.
- A case line is a sequence of switches (zero.md section 14), and a case is known by which features are on when its switches are done: so each line here ends in a different arrangement, and the lines that switch a level off and on again on the way say that the levels under it come back as they were, `two` off staying off through `one` going off and on.

## testing
>reach() with two off, root off, root on, two on → 1111
>reach() with root off, three off, root on → 111
>reach() with two off, one off, one on → 11
>reach() with one off, three off, three on → 1
>reach() with root off → 0
