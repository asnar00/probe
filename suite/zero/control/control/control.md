# control
*control flow: `if` and `else` as statements, `loop` with carried variables, `while`, `continue`, `break` and `bound`, and `for` over a range*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 4 of milestone 0: section 7 of zero.md — `if`/`else`, `loop ... while ... continue`, `break`, `bound N`, `for` over a sequence; `probe cost` counting a bounded loop in the lowered IR.

## overview
An `if` statement runs one of two blocks; a variable it assigns has, after it, the value from whichever arm ran. A `loop` names the variables it carries and their starting values; `while` is tested at the top of every pass, `continue` gives the next values, `break` leaves, and after the loop the carried variables hold what the loop left with. A `for` runs its body once per value of a range.

## interface
- `sign of (x)` is -1, 1 or 0: an `else if` whose arms each assign the result and so end the function there, and a result left alone on the third path is its zero.
- `magnitude of (x)` keeps a temporary, assigns it again inside an `if` with no `else`, and assigns the result last: assigning the result would end the function, so a value that is tested or changed is a temporary until it is final.
- `describe (x)` prints one of two strings.
- `sum to (n)` carries a counter and an accumulator under `while` with `bound 100`, and takes the accumulator after the loop.
- `gcd of (a) and (b)` declares a variable inside the body and continues with it.
- `power of two above (n)` has no `while`: it leaves by `break` inside an `if`, and assigns its carried variable in the body, which the pass's end carries.
- `digits of (n)` breaks from an `if` and continues with computed values.
- `collatz steps (start)` assigns a carried variable in both arms of an `if`, and another after it.
- `triangle (n)` nests one loop in another and reads the inner loop's variable after it.
- `ticks`, `blast off`, `halves` run a `for` over a literal range up, down, and exclusive; `steps from (a) to (b)` over a range whose bounds are decided at run time.

## rules
- A `loop`'s carried variables are exactly those in its header; `continue` gives them in that order, and a bare `continue`, or a body that ends, continues with their current values.
- A variable declared outside a loop is not assigned inside it: carry it in the header. A `for`'s item is not assigned.
- After a loop its carried variables are in scope with the values it left with; a `for`'s item is not.
- A statement after `break` or `continue` is refused: it would never run.
- `[a through b]` includes b and `[a to b]` stops before it; with a > b the range counts down.
- `bound N` on a loop is the trip count `probe cost` uses; a `for` over a literal range shows its count without one.

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
>ticks() → "tick\ntick\ntick"
>blast off() → "more\nmore\none"
>halves() → "odd\nodd"
>steps from (1) to (3) → "step\nstep\nstep"
>steps from (3) to (1) → "step\nstep\nstep"
>steps from (2) to (2) → "step"

## hostile
Each of these is refused with the file and line named. An assignment inside a loop to a variable declared before it: "'s' is declared outside the loop: carry it in the loop's header, `loop (int s = ...)`". A statement after `break`: "this never runs: the statement before it leaves the block". `break` outside a loop: "'break' outside a loop". `continue (1)` inside a `for`: "a `for` steps its item by itself: 'continue' takes no values here". `i = 0` on a `for`'s item: "'i' is the item of the `for`: it steps by itself and is not assigned". `continue (i + 1, 2)` in a loop carrying one variable: "the loop carries 1 variable(s), 'continue' gives 2". `int i = 5` after a loop that carried `i`: "'i' is already declared". A variable declared in an arm of an `if`, read after it: "no function named 't'". `[1.5 through n]`: "a range's bounds are integers".

`probe cost` on the emitted IR (`probe zero suite/zero/control emit > control.ssa; probe cost control.ssa sum_to ticks blast_off`) reports:

    sum_to: loop at b1 x100 (declared), body 5 ssa
    ticks: loop at b1 x3 (i from 1 by 1 to 3), body 37 ssa
    blast_off: loop at b1 x3 (i from 3 by -1 to 1), body 40 ssa

and `steps_from_to`'s loop, whose bounds are run-time values, as unbounded until a `bound` is declared.
