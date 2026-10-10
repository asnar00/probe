# checks
*`check (c)`: the one assertion; a failed check is a trap that names its site*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 10 of milestone 0: section 14 of zero.md — `check (c)`, `→ check` in a case, a failed check naming its site.

## overview
`check (c)` stops the program when `c` does not hold: the IR's trap, which every backend has. Before it stops, the program names the site, `check at checks.zero:2`, and the runner reports it: an expected failure (`→ check`) shows where, and an unexpected one says "a failed check at checks.zero:2". A check that holds costs a compare.

## interface
- `within (k)` checks k < 5 and gives k; `guarded sum (k)` checks k is not negative and sums 1 to k.
- `bounded (k)` maps `small (i)`, which checks its item, over a range and reduces, so the check runs once per item; `either (k)` checks inside one arm of an `if`, and pushes its result after it, at the top of its body, `n << 2 if (k > 10) else 1`.
- `outside` reads item 5 of an array of two. It gives 0: a read by a place cannot fail, and outside an array's items it gives zero (fm3 question 127, `suite/zero/sampled`). Until 10 October 2026 it failed the library's own check, an index past the end, and this was the case that showed it.
- `after printing` writes to `out$`, then fails: what was written comes before the site.
- `pushed after end` pushes into `late$` after `end late$`, and `pushed after shut` into `wide$`, a stream of `int64`, after `shut`, a function over `int x$`, has ended it through its parameter: each fails the library's check in the push. `pushed steadily` pushes into `steady$`, a stream of a type no `end` in the store reaches, and asks how many are waiting, `[count] (frame steady$)`, which keeps it a queue.

## rules
- `check` takes a bool; the check is a statement, and the code after it runs only when it held.
- A failed check ends the case at once; on the GPU, which does not stop at a failed check, the case is skipped.
- The site is the `.zero` file and line of the `check`, printed on a line of its own as `check at file:line`.
- A push into a stream that has ended is a failed check. The compiler makes the test only where it could fail (parity hop twelve, fm3 log 108): `ended` is written by `end` alone, so a push through a name of element type T asks only where some `end` in the store has an operand whose element type T fits, or that fits T, or that is the same type underneath. `late$` and `wide$` are reached, `wide$` because an `int64` fits `shut`'s `int`; `steady$`, a `uint16`, is not, and its push is the library's `push_queue_open`, which checks for room and nothing else.

## testing
>within (3) → 3
>within (9) → check
>guarded sum (4) → 10
>guarded sum (-1) → check
>bounded (3) → 6
>bounded (5) → check
>either (5) → 1
>either (50) → 2
>either (500) → check
>outside() → 0
>after printing() → check
>pushed after end() → check
>pushed after shut() → check
>pushed steadily() → 2

## hostile
`check (k)` with `int k` is refused: "'check' takes a bool". `check` at feature scope is refused by the parser: a check is a statement. A `→ check` case whose call returns is reported "(no check failed; got ...)"; a case expecting a number whose call fails a check is reported "(a failed check at checks.zero:2)".
