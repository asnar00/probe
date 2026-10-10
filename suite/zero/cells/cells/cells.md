# cells
*a stream read only for its latest item is one word, and a stream's name where one value is wanted is its latest item*

layer: runtime

> (suite) 2026-10-07T12:00:00
fm3 questions 70 and 79, log 143. No variable is ever modified: anything that changes is a stream, its value its latest item and a write a push. Ash, 7 October 2026: "a mutable variable is a stream where we only keep one value in the ring buffer."

## overview
`int seen$ << 0` is a stream whose first item is 0. `seen$ << seen$ + 1` pushes one more item, once each time its line runs: the stream's own name on the right of a push into itself is its latest item. And wherever a line wants one value, a stream's name is its latest item: `n << seen$`, `int x = seen$ + 1`, `t$ << token(3, seen$, 1)`.

Every stream here but `t$` is read for its latest item and nothing else, or for that and its `count`, so the compiler keeps one value of it, a field of the context of the item's type: a push is a store of the field and a read a load. There is no ring, no queue and no capacity, and nothing that can fill. That is the smallest rung of what a stream keeps, and it is chosen as every rung is, by the words the store applies to the stream. `both$` is counted as well, `count both$`, how many items it has had: that is one more number of the context beside the latest item, one more at each push, and still no queue (fm3 question 145, hop forty-three; until then it was a queue of sixty-four).

## interface
- `bump` pushes one more into `seen$`; `bumped twice` reads it by its name, and `bumped (k) times` bumps once for each number of a range. `counted up to (k)` pushes with `while`, `seen$ << (seen$ + 1) while (_ <= k)`: ten thousand pushes, where a queue of sixty-four would fail its check at the sixty-fifth.
- `one more` gives the name to a declaration through an operator, `int x = seen$ + 1`; `the latest` says `latest seen$`, which is the same thing; `before any` reads a stream nothing has been pushed into, which is the zero of its type.
- `made` builds a struct from it, `t$ << token(3, seen$, 1)`.
- `past (limit)` reads it in a condition, `checked` in a `check`, `twice seen` as the argument of a function of one value, and `summed to seen` as the bound of a range.
- `raised`, `is auto` and `switched on`, `origin sum` and `moved origin` keep a bool, an enumeration and a struct, the struct's fields read through the name, `origin$.x`.
- `doubled past (limit)` pushes with `while`: each item is worked out from the one before and stored while it passes.
- `counted` reads `both$` by its name and counts it too. `counted far (k)` pushes into it `k` times more, `both$ << (both$ + 1) (k) times`, and gives its count and its latest: a hundred pushes, a hundred and one items with the one on its declaration, where a queue of sixty-four failed its check at the sixty-fifth.

- **In a function** (fm3 log 186) a stream declared there and read only for its latest item is one value of the function, kept in no queue and no field: where its push has a word, it is what the loop that push lowers to carries. `local up to (k)` is `int i$ << 0 << (i$ + 1) while (_ <= k)`, ten thousand items where a queue fails at its sixty-fifth; `local doubled past (k)` is the same with `until` and `local doubled (k)` with `(k) times`, a count below zero failing a check. `local chosen (k)` pushes into it under an `if`, and `local before any` reads one nothing has been pushed into. `local struct (k)` keeps a structure as its fields, apart, `token t$ << token(1, 0, 0) << token(t$.kind + 1, t$.start + t$.kind, t$.n * 2 + 1) (k) times`, and makes the structure once, where it is read whole, `token last = t$`; `local by the candidate (k)` asks `_.kind` of the candidate. `local shown` writes its value now, `out$ << i$`.
- `local and counted (k)` asks `count` of such a stream: nothing takes from it, so the count is how many were pushed, a second value carried beside the latest, and no queue (fm3 question 112, log 190); `local counted far (k)` counts ten thousand. `local pushed round a loop (k)` pushes into one inside a `loop` that began after it, and the loop carries what the stream keeps as it carries a stream it moves, with nothing written in its header; `local summed round a loop (k)` begins with nothing, its name the zero of its type before its first item, and `local pair round a loop (k)` carries a structure's two fields. A push inside a `for`, which carries nothing, still makes the stream a queue of sixty-four; it reads zero before its first item there too. `local handed on` hands the stream to a function that takes one, and is the queue it was.

## rules
- A stream's name is its latest item wherever one value is wanted: a declaration without `$`, the push of a result, a condition, a `check`, an index, the bounds of a range, the base of `.field`, a conversion, a field of a struct being built, both sides of an operator there, and an argument of a function whose parameter is one value.
- An item of a push in a function is read the same way, the line happening once: `out$ << x$` pushes the latest item of `x$`, and `total$ << total$ + other$` reads both for their latest, so both are cells (`suite/zero/now`, fm3 question 79, log 163). An array's name there is the array, whole.
- Before anything is pushed, the latest item is the zero of the type.
- What a stream keeps is the compiler's to work out, and may not show: a cell and a queue read the same.
- A stream declared in a function is one value where the function's every mention of it is the target of a push of single values, `latest`, or its name where one value is wanted. `peek`, `advance`, `frame`, `behind`, `end` and `ended`, a time, a block pushed into it, a task or a function handed it, and a push into it inside a loop that began after it was declared, make it the stream it was.

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
>counted far (100) → 101105
>local up to (10000) → 10000
>local doubled past (100) → 128
>local doubled (10) → 1024
>local doubled (0) → 1
>local doubled (-1) → check
>local chosen (1) → 22
>local chosen (0) → 2
>local before any() → 1
>local struct (3) → 467
>local by the candidate (5) → 54
>local and counted (3) → 403
>local pushed round a loop (4) → 6
>local counted far (10000) → 10001
>local summed round a loop (1000) → 500500
>local pair round a loop (10) → 55
>local handed on() → 3
>local shown() → "7"

## hostile
`int x = out$`: "'out$' is the output device: it is written and never read, so it has no latest item".
