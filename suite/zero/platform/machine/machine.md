# machine
*platform functions with a body per kind of place: the IR's rule lines for a target, or IR for every target*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 11 of milestone 0: section 15's `platform <kind>` bodies on a function declaration, lowered to the IR's platform rules.

## overview
A platform function has no zero body. Each `platform <kind>` line gives it a body for one kind of place: `arm64`, `riscv64`, `wasm32` and `air` are the IR's targets, and a body for one is written in that target's rule lines, which the IR carries in a `platform <target> { ... }` block after the function; `ir` is a body in the IR itself, and serves every target that has no rule of its own. A function with no body for the place a program runs on is out of reach there: the runner skips its cases, naming the function and the kind.

## interface
- `(a) plus (b)` adds two `int64` by the machine's own instruction on every target: `add` on arm64 and riscv64, `i64.add` on wasm32, `add` on air.
- `(a) minus (b)` subtracts, with bodies for the three CPU targets and none for the GPU: its cases are out of reach on air.
- `(a) doubled` has one body, in the IR, so every target runs it.
- `sum of three (a, b, c)` and `three minus one (a)` are zero functions calling the platform ones.
- `count (x$) doubled` maps `doubled` over a sequence, like any function.

## rules
- A platform function is called, never redefined, and never a task or an operator.
- A rule body names the machine's types, `int64` and the like, one number each way; an `ir` body may use any type.
- A kind not among the five is refused, so a misspelt target is not a body nobody runs.

## testing
>(2) plus (3) → 5
>(-7) plus (3) → -4
>sum of three (1, 2, 3) → 6
>(10) minus (4) → 6
>three minus one (5) → 4
>(21) doubled → 42
>count doubled (3) → 3, 6

## hostile
`on (int64 r) = (int64 a) plus (int64 b)` with a zero body and a platform body is refused: "a platform function has no zero body". `platform arm46` is refused naming the five kinds. `on (int r) = f (int a)` with `platform arm64` is refused: "'a' is int in a platform rule: a rule takes the machine's types". A `#` in a platform body is refused like any other. A rule line naming an instruction the target has no template for fails when that target compiles the store, in the IR's own words.
