# control
*control flow: `if` and `else` as statements, `loop` with its own carried variables giving a result, `while`, `continue`, `break`, `for` over a range, and the sequence forms that replace most loops*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 4 of milestone 0: section 7 of zero.md — `if`/`else`, `loop ... while ... continue`, `break`, `bound N`, `for` over a sequence; `probe cost` counting a bounded loop in the lowered IR. Rulings pass item 6 (question 4): a loop's carried variables are its own and `gives` names what leaves; a sum, a triangle and a walk over a range are written as sequence forms, and a loop stays only where nothing else says it.

## overview
An `if` statement runs one of two blocks; a variable it assigns has, after it, the value from whichever arm ran. A `loop` names the variables it carries and their starting values; `while` is tested at the top of every pass, `continue` gives the next values, `break` leaves, and `gives` names the carried variables that come out, into a declared or an existing name: `int total = loop (int i = 0, int acc = 0) while (i < n) gives acc`. The carried variables are the loop's own, gone after it. A `for` runs its body once per value of a range. A repetition that a sequence form can say — a sum over a range, a function mapped over one — is written that way, and the compiler makes the loop.

## interface
- `sign of (x)` is -1, 1 or 0: an `else if` whose arms each assign the result and so end the function there, and a result left alone on the third path is its zero.
- `magnitude of (x)` keeps a temporary, assigns it again inside an `if` with no `else`, and assigns the result last: assigning the result would end the function, so a value that is tested or changed is a temporary until it is final.
- `describe (x)` prints one of two strings.
- `sum to (n)` is a range reduced, `[0 through n] + _`: no loop is written.
- `gcd of (a) and (b)` gives `x` straight into the result, declares a variable inside the body and continues with it.
- `power of two above (n)` has no `while`: it leaves by `break` inside an `if`, assigns its carried variable in the body, which the pass's end carries, and gives it into a declared `int q`.
- `digits of (n)` breaks from an `if`, continues with computed values, and gives both carried variables into two declared names.
- `collatz steps (start)` assigns a carried variable in both arms of an `if`, and another after it, and gives `steps` into the result.
- `triangle (n)` maps `row (i)`, itself a reduction, over `[1 to n + 1]` and reduces: two loops in the IR, none written.
- `two counters` declares `int i` in two loops of one function.
- `ticks`, `blast off`, `halves` run a `for` over a literal range up, down, and exclusive; `steps from (a) to (b)` maps `step`, a function with no result, over a range whose bounds are decided at run time.

## rules
- A `loop`'s carried variables are exactly those in its header; `continue` gives them in that order, and a bare `continue`, or a body that ends, continues with their current values.
- A variable declared outside a loop is not assigned inside it: carry it in the header. A `for`'s item is not assigned.
- After a loop its carried variables are gone; `gives` names the ones that come out, as many as the names on the left of `= loop`, and every `break` (the `while` test's included) gives their values then. A loop that never leaves gives nothing.
- `[1 through n]` counts down when n < 1, so a sum from 1 to n is `[0 through n] + _` and rows 1 to n are `[1 to n + 1]`.
- A statement after `break` or `continue` is refused: it would never run.
- `[a through b]` includes b and `[a to b]` stops before it; with a > b the range counts down.
- A `for` over a literal range shows its count to `probe cost` without a bound.

## testing
>sign of (-5) → -1
>sign of (7) → 1
>sign of (0) → 0
>magnitude of (-3) → 3
>magnitude of (4) → 4
>describe (-1) → "negative"
>describe (2) → "not negative"
>sum to (10) → 55
>sum to (0) → 0
>gcd of (48) and (18) → 6
>gcd of (7) and (5) → 1
>power of two above (10) → 16
>power of two above (16) → 32
>power of two above (0) → 1
>digits of (12345) → 5
>digits of (7) → 1
>collatz steps (6) → 8
>collatz steps (1) → 0
>triangle (3) → 10
>triangle (0) → 0
>two counters() → 35
>ticks() → "tick\ntick\ntick"
>blast off() → "more\nmore\none"
>halves() → "odd\nodd"
>steps from (1) to (3) → "step\nstep\nstep"
>steps from (3) to (1) → "step\nstep\nstep"
>steps from (2) to (2) → "step"

## hostile
Each of these is refused with the file and line named. An assignment inside a loop to a variable declared before it: "'s' is declared outside the loop: carry it in the loop's header, `loop (int s = ...)`". A statement after `break`: "this never runs: the statement before it leaves the block". `break` outside a loop: "'break' outside a loop". `continue (1)` inside a `for`: "a `for` steps its item by itself: 'continue' takes no values here". `i = 0` on a `for`'s item: "'i' is the item of the `for`: it steps by itself and is not assigned". `continue (i + 1, 2)` in a loop carrying one variable: "the loop carries 1 variable(s), 'continue' gives 2". `acc` read after a loop that carried it: "no function named 'acc'". `gives total` naming no carried variable: "'total' is not a variable the loop carries: `gives` names one of its header's, i, acc". `int s = loop (...)` without `gives`: "`= loop` names what the loop gives: `... while (c) gives acc`". `loop (...) gives acc` on its own: "the loop gives 'acc' to nothing". `int s = loop (int i = 0) gives i` with no `while` and no `break`: "the loop never leaves, so it gives nothing". A variable declared in an arm of an `if`, read after it: "no function named 't'". `[1.5 through n]`: "a range's bounds are integers".

`probe cost` on the emitted IR (`probe zero suite/zero/control emit > control.ssa; probe cost control.ssa ticks blast_off`) reports:

    ticks: loop at b1 x3 (i from 1 by 1 to 3), body 49362 ssa
    blast_off: loop at b1 x3 (i from 3 by -1 to 1), body 49365 ssa

(a pass is one `print` of a string literal, which makes a stream of bytes stamped from the clock, and `__now()`'s exact rational arithmetic is most of the body's cost, log 38)

and `sum_to`'s range and `steps_from_to`'s, whose bounds are run-time values, as unbounded, counted once.
