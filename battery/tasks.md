# task battery v1 — 15 tasks (5 write, 5 edit, 5 debug)

Each task's acceptance test is a directive set; the runner appends the
directives to the model's file (or they're stated in the prompt to be
included). All directive values below were computed by reference
implementations, not by hand.

## Write-new (spec = directives only)

W1. `@digits(%n: int) -> int` — count of decimal digits, n >= 0.
    ;! digits 0 -> 1   ;! digits 9 -> 1   ;! digits 10 -> 2
    ;! digits 99999 -> 5   ;! digits 100000 -> 6

W2. `@dot3(%a1..%a3, %b1..%b3: float) -> float` — 3-element dot product.
    ;! dot3 1.0 2.0 3.0 4.0 5.0 6.0 -> 32.0
    ;! dot3 0.5 0.0 -1.0 2.0 7.0 3.0 -> -2.0

W3. `@popcnt(%x: u64) -> int` — population count, loop or tricks.
    ;! popcnt 0 -> 0   ;! popcnt 0xff -> 8
    ;! popcnt 0x8000000000000001 -> 2   ;! popcnt 0xffffffffffffffff -> 64

W4. `@median3(%a, %b, %c: int) -> int` — median of three.
    ;! median3 1 2 3 -> 2   ;! median3 3 1 2 -> 2   ;! median3 2 3 1 -> 2
    ;! median3 5 5 1 -> 5   ;! median3 -3 7 -3 -> -3

W5. `@fixmul(%a: i64, %b: i64) -> i64` — 16.16 fixed-point multiply
    (the product of two 16.16 values, truncated).
    ;! fixmul 0x10000 0x10000 -> 0x10000
    ;! fixmul 0x18000 0x20000 -> 0x30000
    ;! fixmul -65536 131072 -> -131072

## Edit-existing (start from the named corpus function)

E1. suite/sugar.ssa @clamp: add saturation counting — change signature
    to return (int, u1) where the u1 says whether clamping happened.
    New directives replace the old ones:
    ;! clamp 5 0 10 -> 5, 0   ;! clamp 15 0 10 -> 10, 1
    ;! clamp -3 0 10 -> 0, 1

E2. lib/rational.ssa @rat_to_int: change truncation to
    round-half-away-from-zero. Directives (t_to_int wrappers):
    ;! t_to_int 7 2 -> 4   ;! t_to_int -7 2 -> -4   ;! t_to_int 1 3 -> 0
    ;! t_to_int 2 3 -> 1

E3. suite/scalar.ssa @s_geo: parameterize the term count —
    `@s_geo(%n: int) -> int`, still scaled by 8.
    ;! s_geo 4 -> 15   ;! s_geo 1 -> 8   ;! s_geo 2 -> 12

E4. examples/fib.ssa @fib: memoize nothing, but convert the recursion
    to an iterative loop (same directives; verify with `run` at 40 —
    should be fast).

E5. suite/vectors.ssa @vsum: make the step vector a parameter pair —
    `@vsum(%s0: i64, %s1: i64) -> (i64, i64)` (trunc to lanes inside).
    ;! vsum 1 2 -> 5, 10   ;! vsum 3 -1 -> 15, -5

## Find-the-bug (one seeded defect; suite failure output is the clue)

D1. A `%` swapped for `/` in a gcd loop (nontermination — the runner's
    timeout is the symptom).
D2. Pack args in the wrong lane order in a vectors function.
D3. A `u32` field declared `i32` in a struct, flipping an extract's
    sign extension.
D4. An off-by-one loop bound (`<` for `<=`) in an accumulator.
D5. A comparison operand order swap in a clamp (max where min belongs).

Seeding: apply one mutation to a green corpus copy in a scratch dir;
hand the model the failing file + suite output.
