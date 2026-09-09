# extra
*a dynamic feature under a static-off parent: it goes with its parent*

parent: gone
layer: runtime

> (suite) 2026-09-10T10:03:00
Marked `dynamic`, under `gone`, which is static off (log 71): a child under a parent that is never on could never be on, so it leaves the store with it, whatever its own line says.

## overview
Were it in, `bonus` would give 6.

## interface
- `bonus` gives 6.

## rules
- Not in the program: `bonus` gives 0 in every context, base's case.

## testing
>bonus() → 6
