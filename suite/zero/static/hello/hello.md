# hello
*the sketch's hello world with every feature static: the root, under a product that marks all three static on*

layer: runtime

> (suite) 2026-09-08T10:00:00
The parity pass (log 71): section 16's three features as in `suite/zero/hello`, under a `product.md` marking `platform`, `hello`, `countdown` and `bye` static on, so the store lowers with no gate, no switch and no context field for a feature, and its `run` is the chain called by name. Its expected IR is the counterpart of `fm3/examples/hello-min.ssa`, the static oracle.

## overview
`run` says hello. The other two features extend it: `countdown` counts down before it, `bye` says goodbye after it.

## interface
- `run` is the program; `hello` writes "hello world" to `out$` (third pass: output is a stream, `out$ << "hello world" << "\n"`).

## rules
- `run` is a chain: bye's, then countdown's, then hello's, each calling `existing run()` where it wants the earlier ones.
- `run() → "hello world"` is this feature's promise, overridden by countdown's and bye's `run()` cases: every feature is static on, so there is one context and the newest case stands (section 14, log 44, 71).
- A static feature cannot be switched: a case line `with hello off` is refused, "hello is static on in the product and cannot be switched".

## testing
>hello() → "hello world"
>run() → "hello world"

## hostile
A `#` in any of the three `.zero` files stops the build naming the line. `with countdown off` on a case line is refused here, where `suite/zero/hello` runs it.
