# one
*under `root`: adds 10 to `reach`*

parent: root
layer: runtime

> (suite) 2026-10-07T10:01:00
A level of the chain: `one` is on when its own switch, `root`'s and every switch above are.

## overview
`reach` is what came before plus 10.

## interface
- `reach` gives the earlier definition's result plus 10.

## rules
- `one` off, or anything above it off, and the chain falls through to the definition before.

## testing
