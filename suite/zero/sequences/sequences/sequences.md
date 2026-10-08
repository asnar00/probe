# sequences
*sequences: streams whose items are all present — literals, ranges, indexing, `count`, iteration, map, zip and reduce, and the stream words on them*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 6 of milestone 0: section 8 of zero.md — `T$` as arrays, literals including `through` and `to`, indexing, count, iteration, map by applying a one-item function to a sequence, zip pairwise with the longer-length rule, and reduce with `_` as the accumulator. Shaped views are not in this milestone. Rulings pass item 4 (question 12): one `$`, so a sequence is a stream whose items are resident and takes every word of section 9.

## overview
A name ending in `[]` is an array (fm3 question 90): its items are all there, and it never changes. It is made from a list, a range, a text, or the result of applying a function to another array — a function of one item applies to every item, a function of two items applies pairwise, and `_` in a call marks an accumulator that folds the array to one value. `count i[]` is how many items it has and `i[2]` reads one, counting from zero. A name ending in `$` is a stream, whose items arrive: an array pushed into one, `int i$ << [1, 2]`, sends its items one by one, and `frame i$` gives the array of what has arrived. Until the ruling one `$` stood for both, and four cases here tested that a list took every stream word; they now say which kind each line is of.

## interface
- `how many`, `third`, `through`, `up to (m)`, `down` make arrays from lists and ranges and read them by `count` and by index; `nothing` counts a stream nothing has been pushed into.
- `summed` and `product` reduce with `+` and `*`; `smallest` and `smallest of none` reduce with the feature's own `smaller of`, the second over an empty array, `int i[] = []`.
- `mapped` adds one to every item; `zipped` adds two sequences of different lengths, the shorter reading as zero past its end.
- `mapped call` applies `doubled (x)` to a list; `zipped call` applies `scaled (a) by (b)` pairwise.
- `iterated` walks a sequence with `for`, pushing each sum into the feature's stream `total$`, read for its latest item (fm3 question 70).
- `sum of (x[])` takes an array; `passed` calls it with a list, `[sum of] ([1, 2, 3])`, the name in square brackets because the function takes an array whole (fm3 question 77). `squares to (k)` gives an array, `on (int r[]) << squares to (int k)`, which `squared` indexes.
- `letters` and `first byte` treat a string as the sequence of bytes it is; `marks total` reduces a feature-scope sequence; `halves` reduces floats.
- `pushed into`, `read ahead`, `framed sum` and `bytes pushed` give a stream an array as its first items and use the stream's words on it: a push after a list, `advance` and then `peek`, `frame` of a range into an array, a push after a text.

## rules
- A sequence's items are numbers, characters or enumerations; a `string` is a `char$`.
- `[a through b]` includes b, `[a to b]` stops before it, and both count down when a > b.
- An index counts from zero and is checked; `count` of an array is how many items it has.
- A function of one item applied to a sequence gives a sequence; of two items applied to two sequences, one as long as the longer, the shorter one's items zero past its end.
- A reduction folds one sequence from its first item; an empty sequence gives the type's zero.
- Map, zip, reduce, `for` and an index are an array's; `peek`, `advance`, `latest` and `frame` are a stream's. Each is refused of the other kind, and `int i$ = [1, 2, 3]` is refused showing `int i[] = [1, 2, 3]` (fm3 log 161).
- A `for`'s item is not assigned; a sequence's memory is the store's, emptied before every case.
- A push into a stream of *characters* is by dispatch (third pass, log 59), so `out$ << 33` writes the digits `3` `3`; a push into a stream of `uint8` is the byte itself (question 44, log 87), so `bytes pushed` writes `b$ << 33` and gets one byte of 33. A string literal is a `char$`, and its bytes fill a `uint8$` too: the literal is bytes, and its element follows the stream it goes into.

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
`i[4]` on four items is a failed check from the IR. `_` with no sequence in the call is refused: "'_' goes with a stream among the arguments". `x = 1` on a `for`'s item is refused. `Vec v[] = [Vec(1, 2, 3)]` is refused: "a list of Vec: only numbers and enumerations in this milestone". `int i[] at (1 khz) = [1, 2]` is refused: "a rate is a stream's, and 'i[]' is an array, all there". More than 64K bytes of sequences in one case is a failed check in `arena_alloc`.
