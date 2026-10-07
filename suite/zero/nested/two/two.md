# two
*under `one`: adds 100 to `reach`*

parent: one
layer: runtime

> (suite) 2026-10-07T10:02:00
A level of the chain: `two` is on when its own switch, `one`'s and every switch above are.

## overview
`reach` is what came before plus 100.

## interface
- `reach` gives the earlier definition's result plus 100.

## rules
- `two` off, or anything above it off, and the chain falls through to the definition before.

## testing
