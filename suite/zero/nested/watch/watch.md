# watch
*beside `one`, under `root`: reads the three nested switches*

parent: root
layer: runtime

> (suite) 2026-10-07T10:04:00
A read of another feature's `enabled` is its effective state (zero.md section 5), the same field its gates read: one load at any depth.

## overview
`lights` gives a digit a level: 1 where `one` is on, 10 where `two` is, 100 where `three` is, each with everything above it.

## interface
- `lights` gives the three effective states as one number.

## rules
- Switching a feature writes its own switch and works out the effective state of everything under it; no other switch is written, so `two` off stays off through `one` going off and on.

## testing
>lights() → 111
>lights() with three off, one off, two off, two on, one on → 11
>lights() with one off, two off, one on → 1
>lights() with one off → 0
>lights() with root off → 0
