# checks
*`check (c)`: the one assertion; a failed check is a trap that names its site*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 10 of milestone 0: section 14 of zero.md — `check (c)`, `→ check` in a case, a failed check naming its site.

## overview
`check (c)` stops the program when `c` does not hold: the IR's trap, which every backend has. Before it stops, the program names the site, `check at checks.zero:2`, and the runner reports it: an expected failure (`→ check`) shows where, and an unexpected one says "a failed check at checks.zero:2". A check that holds costs a compare.

## interface
- `within (k)` checks k < 5 and gives k; `guarded sum (k)` checks k is not negative and sums 1 to k.
- `bounded (k)` checks inside a `loop`, on every pass; `either (k)` inside one arm of an `if`.
- `outside` fails the library's own check, an index past the end, which names no zero site.
- `after printing` prints, then fails: what was printed comes before the site.

## rules
- `check` takes a bool; the check is a statement, and the code after it runs only when it held.
- A failed check ends the case at once; on the GPU, which does not stop at a failed check, the case is skipped.
- The site is the `.zero` file and line of the `check`, printed on a line of its own as `check at file:line`.

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
>outside() → check
>after printing() → check

## hostile
`check (k)` with `int k` is refused: "'check' takes a bool". `check` at feature scope is refused by the parser: a check is a statement. A `→ check` case whose call returns is reported "(no check failed; got ...)"; a case expecting a number whose call fails a check is reported "(a failed check at checks.zero:2)".
