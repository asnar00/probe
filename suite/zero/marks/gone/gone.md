# gone
*a feature the product marks static off: not in the program at all*

parent: base
layer: runtime

> (suite) 2026-09-10T10:02:00
Static off (log 71): this feature's code is not emitted, its cases do not run, and `extra` under it goes with it.

## overview
Were it on, `bonus` would give 5; the product leaves it out, so `bonus` gives 0 and this case never runs.

## interface
- `bonus` gives 5.

## rules
- A case line `with gone off` anywhere is refused: "gone is static off in the product, so its code and its cases are not in the program" (a Rust test).

## testing
>bonus() → 5
