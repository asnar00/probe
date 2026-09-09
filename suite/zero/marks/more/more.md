# more
*a dynamic feature under a static parent: switchable, its gate reading its own flag alone*

parent: base
layer: runtime

> (suite) 2026-09-10T10:01:00
Dynamic under `base`, which is static on (log 71): `__on_more` reads its own `enabled` only, there being no dynamic ancestor.

## overview
`value` becomes 11 while `more` is on and 1 again with it off.

## interface
- `value` gives 10 more than before.

## rules
- With `more` off (`>value() with more off → 1`, and the runner's own context), the chain's link falls through to base's body directly, `value__base`, since base has no link of its own.

## testing
>value() → 11
>value() with more off → 1
