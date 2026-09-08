# streams
*streams: push with `<<`, `peek`, `advance by`, `frame`, `latest`, `count`, `position`, `ended`, `end`, `behind`, a stream of structs, a rate*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 7 of milestone 0: section 9 of zero.md — push with `<<`, `peek`, `advance by`, `frame`, `latest`, `count`, `position`, `ended`, `end`, `behind`, a stream of structs, rate `at (n hz)`. The scheduler and time at the boundary are item 8.

## overview
A stream is a value over time. It is declared with `<<`, which pushes its first items, or with a rate, `T x$ at (n hz)`; a sequence is declared with `=`. One producer pushes into it and a reader moves through it: `peek` reads an unread item without moving, `advance` moves on, `frame` takes everything unread as a sequence and moves past it, `latest` is the most recent item pushed, `count` how many are unread. `end` closes a stream and `ended` says whether it is closed. On the right of `<<` the stream's own name is its latest item, and `while` repeats the last push for as long as the condition holds of the item about to be pushed.

## interface
- `pushed`, `chained`, `repeated` and `counted down from (k)` make streams with `<<`, the last two with a `while`.
- `peeked`, `advanced`, `framed`, `history` read a stream with `peek`, `advance`, `frame` and `behind`.
- `walked` moves a stream inside a `loop`, which carries it.
- `positioned` and `position unread` take `position`'s two results, on a regular and an irregular stream.
- `still open`, `now closed`, `pushed after end` are `end` and `ended`.
- `sampled` and `windowed` declare a rate and read by time, `x$ at (t)` and `x$ from (t1) to (t2)`.
- `tokens` and `tokens moved` push and read a stream of the struct `token`.
- `logged` and `logged and read` push into and read the feature-scope stream `log$`.

## rules
- A declaration with `<<` or `at (n hz)` makes a stream; with `=`, a list or a range, a sequence. `frame` and `behind` give sequences.
- `x$ << e while (c)`: the candidate is computed from the latest item, `x$` in the condition reads as the candidate, and it is pushed only when the condition holds.
- `advance` and `frame` move the reader; inside a `loop` the loop carries the stream, and after it the variable holds the moved reader.
- A stream of structs holds numbers and enumerations in its fields; `frame`, `behind`, `at` and `from`/`to` on one wait for sequences of structs.
- A push after `end` is a failed check. A stream without a rate stamps its items from the store's clock, which nothing moves yet; `position`'s tick is then 0, or -1 when nothing is unread.
- A ring holds 64 items; a reader more than 32 behind fails a check. Every ring is carved from the store's arena.

## testing
>pushed() → 3
>chained() → 20
>repeated() → 44
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

## hostile
`peek` on a sequence falls through to "no function named 'peek at'". `i$ << 3` on `int i$ = [1, 2]` is refused: "'i$' is a int[], not a stream: a stream is declared with `<<` or `at (n hz)`". `int f$ = frame t$` on a stream of structs is refused: "'frame' on a stream of structs: a sequence of structs is not in this milestone". `advance i$ by (1)` inside a `for` is refused: "move a stream inside a `loop`, which carries it". `x$ at (1 hz)` in an expression is refused: "a rate belongs on a stream's declaration". `peek i$ at (5)` past the unread items is a failed check from the library.
