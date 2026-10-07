# cells
*a stream read only for its latest item is one word, and a stream's name where one value is wanted is its latest item*

layer: runtime

> (suite) 2026-10-07T12:00:00
fm3 questions 70 and 79, log 143. No variable is ever modified: anything that changes is a stream, its value its latest item and a write a push. Ash, 7 October 2026: "a mutable variable is a stream where we only keep one value in the ring buffer."

## overview
`int seen$ << 0` is a stream whose first item is 0. `seen$ << seen$ + 1` pushes one more item, once each time its line runs: the stream's own name on the right of a push into itself is its latest item. And wherever a line wants one value, a stream's name is its latest item: `n = seen$`, `int x = seen$ + 1`, `t$ << token(3, seen$, 1)`.

Every stream here but `both$` and `t$` is read for its latest item and nothing else, so the compiler keeps one value of it, a field of the context of the item's type: a push is a store of the field and a read a load. There is no ring, no queue and no capacity, and nothing that can fill. That is the smallest rung of what a stream keeps, and it is chosen as every rung is, by the words the store applies to the stream: `both$` is counted as well, so it is a queue, and its name where one value is wanted reads the same.

## interface
- `bump` pushes one more into `seen$`; `bumped twice` reads it by its name, and `bumped (k) times` bumps once for each number of a range. `counted up to (k)` pushes with `while`, `seen$ << (seen$ + 1) while (_ <= k)`: ten thousand pushes, where a queue of sixty-four would fail its check at the sixty-fifth.
- `one more` gives the name to a declaration through an operator, `int x = seen$ + 1`; `the latest` says `latest seen$`, which is the same thing; `before any` reads a stream nothing has been pushed into, which is the zero of its type.
- `made` builds a struct from it, `t$ << token(3, seen$, 1)`.
- `past (limit)` reads it in a condition, `checked` in a `check`, `twice seen` as the argument of a function of one value, and `summed to seen` as the bound of a range.
- `raised`, `is auto` and `switched on`, `origin sum` and `moved origin` keep a bool, an enumeration and a struct, the struct's fields read through the name, `origin$.x`.
- `doubled past (limit)` pushes with `while`: each item is worked out from the one before and stored while it passes.
- `counted` reads `both$` by its name and counts it too.

## rules
- A stream's name is its latest item wherever one value is wanted: a declaration without `$`, an assignment to a result, a condition, a `check`, an index, the bounds of a range, the base of `.field`, a conversion, a field of a struct being built, both sides of an operator there, and an argument of a function whose parameter is one value.
- An item of a push is not such a place: `out$ << x$` pushes what is unread in `x$`, as it did, and a stream pushed whole is not a cell.
- Before anything is pushed, the latest item is the zero of the type.
- What a stream keeps is the compiler's to work out, and may not show: a cell and a queue read the same.

## testing
>bumped twice() → 2
>bumped (5) times → 5
>counted up to (10000) → 10000
>one more() → 2
>the latest() → 1
>before any() → 0
>made() → 2
>past (0) → 1
>past (5) → 0
>checked() → 1
>raised() → 1
>is auto() → 1
>switched on() → 1
>origin sum() → 6
>moved origin() → 10
>twice seen() → 6
>summed to seen() → 6
>doubled past (100) → 64
>counted() → 62

## hostile
`int x = out$`: "'out$' is the output device: it is written and never read, so it has no latest item".
