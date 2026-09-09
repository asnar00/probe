# base
*a feature the product marks static on: no gate, no switch, no field*

layer: runtime

> (suite) 2026-09-10T10:00:00
The parity pass (log 71): a `product.md` line per feature, `static on`, `static off` or `dynamic`. This is the static root; `more` is dynamic under it, `gone` is static off and takes `extra` with it.

## overview
`value` is 1. Being static on, this feature has no `enabled` field, no `__on_base` gate and no `__set___enabled_base` setter in the IR, and the runner never tries a context with it off.

## interface
- `value` gives 1, before any feature extends it.
- `bonus` gives 0: a hook `gone` would have extended, and does not, being static off.

## rules
- A static-on feature is always on: `base.enabled` reads as true with no field behind it, a constant in the IR.
- A case line `with base off` is refused: "base is static on in the product and cannot be switched" (a Rust test in `src/zero/run.rs`).
- A product line naming no feature of the store is refused, and `platform: static off` is refused.

## testing
>value() → 1
>bonus() → 0
>on() → 1
