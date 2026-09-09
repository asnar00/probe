# sequences
*sequences: streams whose items are all present — literals, ranges, indexing, `count`, iteration, map, zip and reduce, and the stream words on them*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 6 of milestone 0: section 8 of zero.md — `T$` as arrays, literals including `through` and `to`, indexing, count, iteration, map by applying a one-item function to a sequence, zip pairwise with the longer-length rule, and reduce with `_` as the accumulator. Shaped views are not in this milestone. Rulings pass item 4 (question 12): one `$`, so a sequence is a stream whose items are resident and takes every word of section 9.

## overview
A name ending in `$` is a stream. It is a sequence when its items are all present: made from a list, a range, a string, or the result of applying a function to another sequence — a function of one item applies to every item, a function of two items applies pairwise, and `_` in a call marks an accumulator that folds the sequence to one value. `count` gives how many items are unread and `x$[i]` reads one, counting from zero. Being a stream, it can be pushed into, peeked at, advanced and framed like any other.

## interface
- `how many`, `third`, `through`, `up to (m)`, `down`, `nothing` make sequences from lists and ranges and read them by `count` and by index.
- `summed` and `product` reduce with `+` and `*`; `smallest` and `smallest of none` reduce with the feature's own `smaller of`, the second over an empty sequence.
- `mapped` adds one to every item; `zipped` adds two sequences of different lengths, the shorter reading as zero past its end.
- `mapped call` applies `doubled (x)` to a list; `zipped call` applies `scaled (a) by (b)` pairwise.
- `iterated` walks a sequence with `for`, adding into the feature variable `total`.
- `sum of (x$)` takes a sequence parameter; `passed` calls it with a list. `squares to (k)` gives a sequence result, which `squared` indexes.
- `letters` and `first byte` treat a string as the sequence of bytes it is; `marks total` reduces a feature-scope sequence; `halves` reduces floats.
- `pushed into`, `read ahead`, `framed sum` and `bytes pushed` use the stream words on sequences: a push after a list, `advance` before an index, `frame` of a range, a push onto a string.

## rules
- A sequence's items are numbers or enumerations; a `string` is a `uint8$`.
- `[a through b]` includes b, `[a to b]` stops before it, and both count down when a > b.
- An index counts from zero over the unread items and is checked; `count` is an `int`, how many are unread.
- A function of one item applied to a sequence gives a sequence; of two items applied to two sequences, one as long as the longer, the shorter one's items zero past its end.
- A reduction folds one sequence from its first item; an empty sequence gives the type's zero.
- Map, zip, reduce, `for` and an index read the unread items and leave the reader where it is; `advance` and `frame` move it, and the words then agree with `count`.
- A `for`'s item is not assigned; a sequence's memory is the store's, emptied before every case.

## testing
>how many() → 4
>third() → 30
>through() → 44
>up to (7) → 6
>down() → 51
>nothing() → 0
>summed() → 10
>product() → 24
>mapped() → 14
>zipped() → 440
>mapped call() → 20
>zipped call() → 32
>smallest() → 1
>smallest of none() → 0
>iterated() → 10
>passed() → 6
>squared() → 16
>letters() → 5
>first byte() → 104
>marks total() → 7
>halves() → 8
>pushed into() → 33
>read ahead() → 62
>framed sum() → 100
>bytes pushed() → 333

## hostile
`i$[4]` on four items is a failed check from the IR. `_` with no sequence in the call is refused: "'_' marks the accumulator of a reduction, or the candidate in a chain's `while`". `x = 1` on a `for`'s item is refused. `Vec v$ = [Vec(1, 2, 3)]` is refused: "a list of Vec: only numbers and enumerations in this milestone". `int i$ at (1 khz) = [1, 2]` is refused: "a rate goes on an empty stream, `int i$ at (n hz)`, which `<<` then fills". More than 64K bytes of sequences in one case is a failed check in `arena_alloc`.
