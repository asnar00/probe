# streams
*streams: push with `<<`, `peek`, `advance by`, `frame`, `latest`, `count`, `position`, `ended`, `end`, `behind`, a stream of structs, a rate, and the sequence words on a stream*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 7 of milestone 0: section 9 of zero.md — push with `<<`, `peek`, `advance by`, `frame`, `latest`, `count`, `position`, `ended`, `end`, `behind`, a stream of structs, rate `at (n hz)`. The scheduler and time at the boundary are item 8. Rulings pass item 4 (question 12): one `$`, a bare `T x$` an empty stream, and the sequence words of section 8 on any stream.

## overview
A stream is a value over time. `T x$` declares an empty one; `<<` pushes its first items; `T x$ at (n hz)` gives it a rate. One producer pushes into it and a reader moves through it: `peek` reads an unread item without moving, `advance` moves on, `frame` takes everything unread as an array, `int f[] = frame i$`, and moves past it, `latest` is the most recent item pushed, `count` how many are unread. `end` closes a stream and `ended` says whether it is closed. On the right of `<<` the stream's own name is its latest item, and `while` repeats the last push for as long as the condition holds of `_`, the item about to be pushed. The unread items are also a sequence: `x$[i]`, `for`, map, zip and reduce read them where the reader stands.

## interface
- `pushed`, `chained`, `repeated` and `counted down from (k)` make streams with `<<`, the last two with a `while` testing `_`; `latest tested` tests `i$` instead and gets one item more. `chained` reads its stream for its latest item and nothing else, so it is three values and no stream at all (fm3 log 186); the others count theirs.
- `peeked`, `advanced`, `framed`, `history` read a stream with `peek`, `advance`, `frame` and `behind`.
- `walked` moves a stream inside a `loop`, which carries it and gives the sum it made.
- `positioned` and `position unread` take `position x$` and `time of x$`, on a stream with a rate and on one with none: `time of x$` is a time, the item's index over the stream's rate, `2 ms` for item 2 at `1000 hz`, and for a stream with no rate the tick its item keeps over the clock's own rate, `-1 us` where nothing is unread.
- `still open`, `now closed`, `pushed after end` are `end` and `ended`.
- `sampled` and `windowed` declare a rate and read by time, `x$ at (t)` and `x$ from (t1) to (t2)`. `sampled at a time worked out` and `windowed between times worked out` hand them a `time` that is a value, `800 us * 2`, where the others hand a literal: a time is a structure declared in zero (fm3 question 117), and the words take one.
- `tokens`, `tokens moved` and `tokens framed` push and read a stream of the struct `token`, which is one ring whose item is the struct (question 43, log 88).
- `logged` and `logged and read` push into and read the feature-scope stream `log$`. `counted round a push` counts a stream of its own, pushes into it and counts again, 2 and then 3: a second `count` of the same reader is the first's number only where nothing between could have pushed (fm3 log 112). `counted round a skip` counts `log$`, calls `skip one logged`, which advances the feature's reader, and counts again in the same function: 3 and then 2, so the second count is of the reader as the call left it (fm3 log 110).
- `blocked` and `blocked regular` push a block, `x$ << block$`: a string into a stream of bytes, a list into a regular stream.
- `indexed`, `summed`, `mapped`, `walked over` and `unread only` make an array of a pushed stream, `int a[] = frame i$`, and use an array's forms on it: an index, `+ _`, `* 2`, the sum pushed into the feature's stream `seen$`, `seen$ << seen$ + (a[] + _)` (a `for` pushing each until fm3 log 184), and a reduction of what `advance` left unread, the count taken before the frame, which moves the reader. Until fm3 question 90 each asked the stream itself, one `$` standing for both kinds; an item by its place, a reduce and a `for` are an array's, and are refused of a stream. `framed pushed` gives a stream a frame as its first items, `int f$ << frame i$`, and pushes into that.
- `framed before a ring wraps` frames a stream that keeps a history, pushes seventy more items into it, and then compares what it framed: `int got[] = frame x$` is a copy there, as `frame` always gave, because something is pushed before its last use; read in place it would be the items that overwrote it (fm3 log 183).

## rules
- Every `$` name is a stream: a bare declaration is empty, `<<` pushes, `at (n hz)` sets a rate, and `=` wires it to a task. What is all there is an array, `int a[]` (fm3 question 90; `suite/zero/arrays`): a list, a range or a text pushed into a stream sends its items one by one, `int r$ at (1 khz) << [1, 2, 3]`, and `frame`, `behind` and `from ... to` give arrays holding copies of a stream's items.
- `x$ << a[]` pushes every item of an array, one push each.
- `x$ << e while (c)`: where the condition reads `_`, the candidate is computed from the latest item and pushed only when the condition holds; in the condition `_` is the candidate and `x$` is still the latest item, so `while (_ < 5)` stops before 5 and `while (i$ < 5)` after it (question 9). Where it does not read `_`, as in `latest tested`, the test is made first and the item worked out only where it holds (fm3 question 111, log 186): the same items, and nothing worked out that is not pushed.
- `advance` and `frame` move the reader; inside a `loop` the loop carries the stream, and after it the variable holds the moved reader.
- An item by its place, `for` and a reduce are an array's and are refused of a stream; `peek x$ at (i)` reads the item i on from the reader without moving it, and `count x$` is how many are unread.
- A stream of structs is a ring of structs (question 43, log 88): one buffer whose item is the struct, one header, one position, so a token's push is one push and a `peek` one read. Its fields are numbers and enumerations, the ring storing the struct whole. It takes `peek`, `x$[i]`, `latest`, `advance`, `count`, `for`, `frame`, `behind`, `end` and `ended`; `x$ at (t)` and `from`/`to` do not, a struct having no midpoint, and map, zip and reduce wait for sequences of structs.
- A push after `end` is a failed check. A stream is timed only when something asks for time (section 9, log 73, 85): a rate on its declaration or its wiring, or a time word applied to its name anywhere in the store — `x$ at (t)`, `x$ from (a) to (b)`, `time of x$` — and a time word on a function's stream parameter times every stream in the store, since any may be passed there. `position x$` is not one of them: it gives the index of the next unread item and nothing else, a `get` of the reader's own position (question 42), so a program that never mentions time keeps every ring plain. A timed stream without a rate keeps a tick per item, stamped from the store's clock, which nothing moves yet, so `time of x$` is 0, or -1 when nothing is unread; every other stream is a plain ring with no ticks, and a push into it is a store and a count. Here `x$` and `i$` are timed by the `time of` in `positioned` and `position unread`, and the rest are plain.
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
>positioned() → 2, 2 ms
>position unread() → 2, -1 us
>still open() → 0
>now closed() → 1
>pushed after end() → check
>sampled() → 30
>windowed() → 23
>sampled at a time worked out() → 30
>windowed between times worked out() → 1
>tokens() → 341
>tokens moved() → 1252
>tokens framed() → 3391
>logged() → 3300
>logged and read() → 7
>counted round a push() → 23
>counted round a skip() → 32
>blocked() → 2105
>blocked regular() → 33
>indexed() → 67
>summed() → 6
>mapped() → 12
>walked over() → 6
>unread only() → 72
>framed pushed() → 39
>framed before a ring wraps() → 1

## hostile
`int k, int t = position x$` is refused: "'position' gives one int, the index of the next unread item: `int i = position x$`, and the time of that item is `time of x$`". `int i$ <<` with nothing after it is refused: "a bare `int i$` declares an empty stream: drop the `<<`". `t$ at (2 ms)` on a stream of structs is refused: "'at' on a stream of structs: a struct has no midpoint, so there is no value between two of them", and the IR refuses `sample` on a ring of structs by name for the same reason; `t$ + _` is refused: "'+' does not reduce a stream of token". `advance i$ by (1)` inside a `for` is refused: "move a stream inside a `loop`, which carries it". `x$ at (1 hz)` in an expression is refused: "a rate belongs on a stream's declaration". `peek i$ at (5)` past the unread items is a failed check from the library; so is `i$[5]`. `int i$ << 3` in a function and `i$ << 4` in a `loop` body that does not carry `i$` are fine: a push moves no reader.
