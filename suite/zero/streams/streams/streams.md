# streams
*streams: push with `<<`, `peek`, `advance by`, `frame`, `latest`, `count`, `position`, `ended`, `end`, `behind`, a stream of structs, a rate, and the sequence words on a stream*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 7 of milestone 0: section 9 of zero.md — push with `<<`, `peek`, `advance by`, `frame`, `latest`, `count`, `position`, `ended`, `end`, `behind`, a stream of structs, rate `at (n hz)`. The scheduler and time at the boundary are item 8. Rulings pass item 4 (question 12): one `$`, a bare `T x$` an empty stream, and the sequence words of section 8 on any stream.

## overview
A stream is a value over time. `T x$` declares an empty one; `<<` pushes its first items; `T x$ at (n hz)` gives it a rate. One producer pushes into it and a reader moves through it: `peek` reads an unread item without moving, `advance` moves on, `frame` takes everything unread as a new stream and moves past it, `latest` is the most recent item pushed, `count` how many are unread. `end` closes a stream and `ended` says whether it is closed. On the right of `<<` the stream's own name is its latest item, and `while` repeats the last push for as long as the condition holds of `_`, the item about to be pushed. The unread items are also a sequence: `x$[i]`, `for`, map, zip and reduce read them where the reader stands.

## interface
- `pushed`, `chained`, `repeated` and `counted down from (k)` make streams with `<<`, the last two with a `while` testing `_`; `latest tested` tests `i$` instead and gets one item more.
- `peeked`, `advanced`, `framed`, `history` read a stream with `peek`, `advance`, `frame` and `behind`.
- `walked` moves a stream inside a `loop`, which carries it and gives the sum it made.
- `positioned` and `position unread` take `position`'s two results, on a regular and an irregular stream.
- `still open`, `now closed`, `pushed after end` are `end` and `ended`.
- `sampled` and `windowed` declare a rate and read by time, `x$ at (t)` and `x$ from (t1) to (t2)`.
- `tokens` and `tokens moved` push and read a stream of the struct `token`.
- `logged` and `logged and read` push into and read the feature-scope stream `log$`.
- `blocked` and `blocked regular` push a block, `x$ << block$`: a string into a stream of bytes, a list into a regular stream.
- `indexed`, `summed`, `mapped`, `walked over`, `unread only` and `framed pushed` use the sequence words on a pushed stream: an index, `+ _`, `* 2`, a `for` adding into the feature variable `seen`, a reduction after `advance`, and a push into a frame.

## rules
- Every `$` name is a stream: a bare declaration is empty, `<<` pushes, `at (n hz)` sets a rate, and `=` gives it the items of a list, a range, a string or another stream's result. `frame`, `behind` and `from ... to` give new streams holding copies of the items, stamped at the clock's now.
- `x$ << block$` pushes every unread item of another stream, one push each.
- `x$ << e while (c)`: the candidate is computed from the latest item and pushed only when the condition holds; in the condition `_` is the candidate and `x$` is still the latest item, so `while (_ < 5)` stops before 5 and `while (i$ < 5)` after it (question 9).
- `advance` and `frame` move the reader; inside a `loop` the loop carries the stream, and after it the variable holds the moved reader.
- `x$[i]`, `for`, map, zip and reduce read the unread items and do not move the reader, so they agree with `count`.
- A stream of structs holds numbers and enumerations in its fields, and takes `peek`, `x$[i]`, `latest`, `advance`, `count`, `for`, `end` and `ended`; `frame`, `behind`, `at`, `from`/`to`, map, zip and reduce on one wait for sequences of structs.
- A push after `end` is a failed check. A push is stamped from the store's clock on a stream without a rate, which nothing moves yet; `position`'s tick is then 0, or -1 when nothing is unread.
- A ring keeps 64 items resident, or as many as a list, a range or a copy puts in it, in twice that many slots (each item sits in both halves, so a frame across the seam is one view and a push never slides); a reader more than that behind fails a check. Every ring is carved from the store's arena.

## testing
>pushed() → 3
>chained() → 20
>repeated() → 44
>latest tested() → 55
>counted down from (10) → 1001
>peeked() → 6
>advanced() → 71
>framed() → 36
>walked() → 10
>history() → 23
>positioned() → 22
>position unread() → 19
>still open() → 0
>now closed() → 1
>pushed after end() → check
>sampled() → 30
>windowed() → 23
>tokens() → 341
>tokens moved() → 1252
>logged() → 3300
>logged and read() → 7
>blocked() → 2105
>blocked regular() → 33
>indexed() → 67
>summed() → 6
>mapped() → 12
>walked over() → 6
>unread only() → 72
>framed pushed() → 39

## hostile
`int i$ <<` with nothing after it is refused: "a bare `int i$` declares an empty stream: drop the `<<`". `int f$ = frame t$` on a stream of structs is refused: "'frame' on a stream of structs is not in this milestone"; so is `t$ + _`: "'+' does not reduce a stream of token". `advance i$ by (1)` inside a `for` is refused: "move a stream inside a `loop`, which carries it". `x$ at (1 hz)` in an expression is refused: "a rate belongs on a stream's declaration". `peek i$ at (5)` past the unread items is a failed check from the library; so is `i$[5]`. `int i$ << 3` in a function and `i$ << 4` in a `loop` body that does not carry `i$` are fine: a push moves no reader.
