# said
*a failed check says which line of the program it came from, and what was asked that could not be had*

parent: checks
layer: runtime

> (suite) 2026-10-08T10:00:00
fm3 log 199, question 115: a case may say where the check fails, `→ check at <file>:<line>`.

## overview
A program is compiled as it always was, with nothing in it that says where it is. Where a case fails a check, the runner lowers the same store a second time as a diagnostic build, in which each statement first stores its site, and runs the case again; the site read back is a row of a table, the file, the line, and what was being asked in the program's own names. So `kept up (70)` says "a failed check at said.zero:4: the stream `kept$` is full: 64 items pushed and nothing has read them", and costs nothing where `kept up (3)` runs and does not fail.

## interface
- `kept up (k)` pushes `k` items into `kept$`, a stream of this feature's that holds sixty-four, and gives how many it holds.
- `counted (k)` pushes into a stream of its own `k` times, which fails where `k` is negative.
- `looked` asks `checks`' `outside` for an item past the end of its array, and is given 0: an array's item by its place is no check and cannot fail (fm3 question 127). `peeked (i)` asks a stream of two items for the one `i` on from its reader, which is a stream's word and a check still.
- `share (a) among (k)` divides and `left of (a) among (k)` takes the remainder, by a number the compiler cannot see; `halved (a)` divides by a literal, and `shared safely (a, k)` by a number its own `if` has tested.

## rules
- `→ check at <file>:<line>` holds where the check that fails is on that line: the file's own name, and the line of its text. `→ check` holds for any, as it did.
- What is reported is `a failed check at <file>:<line>: <what>`: a stream that is full, with its name and how many it holds unread; `item I of N` for a stream's item by its place, `peek x$ at (i)`, and for no array's, a read of an array by a place giving zero outside it; `a count of K times`; a push into a stream that has ended, with its name. The program's own `check (c)` says its line, and anything else that stops in a statement says that statement's line.
- A division or a remainder of whole numbers by zero is a failed check on every path, `a division by zero` (fm3 question 116): the machines do not agree what it gives, arm64 saying 0 and wasm stopping, so the program stops, at its line. The check is a comparison and stands before the division; it is left out where the compiler can see the divisor is not zero, a literal, or a value that the loop's `while`, an `if` or an earlier `check` has just tested, so `halved` and `shared safely` have none. A `float` divided by zero is the machine's infinity and is not this.
- This feature's cases for `within (9)` and `pushed after end()` stand over `checks`' own wherever this feature is on, and say the line; with it off, `checks`' `→ check` stands, as written.
- On the GPU, which does not stop at a failed check, the cases are skipped.

## testing
>kept up (3) → 3
>kept up (70) → check at said.zero:4
>counted (2) → 2
>counted (-2) → check at said.zero:9
>looked() → 0
>peeked (1) → 5
>peeked (3) → check at said.zero:29
>within (9) → check at checks.zero:2
>pushed after end() → check at checks.zero:41
>share (7) among (2) → 3
>share (7) among (0) → check at said.zero:16
>left of (7) among (2) → 1
>left of (7) among (0) → check at said.zero:19
>halved (9) → 4
>shared safely (7, 0) → 0
>shared safely (7, 2) → 3

## hostile
`→ check at said.zero` with no line, or `→ check at 4`, is refused: "a case that says where a check fails is `→ check at <file>:<line>`". A case that says the wrong line fails, and is told the right one: "(a failed check at said.zero:4: the stream `kept$` is full: 64 items pushed and nothing has read them)".
