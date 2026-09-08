# sequences
*sequences as arrays: literals, ranges, indexing, `count`, iteration, map, zip and reduce*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 6 of milestone 0: section 8 of zero.md — `T$` as arrays, literals including `through` and `to`, indexing, count, iteration, map by applying a one-item function to a sequence, zip pairwise with the longer-length rule, and reduce with `_` as the accumulator. Shaped views are not in this milestone.

## overview
A name ending in `$` is a sequence. It is made from a list, a range, or the result of applying a function to another sequence: a function of one item applies to every item, a function of two items applies pairwise, and `_` in a call marks an accumulator that folds the sequence to one value. `count` gives how many items there are and `x$[i]` reads one, counting from zero.

## interface
- `how many`, `third`, `through`, `up to (m)`, `down`, `nothing` make sequences from lists and ranges and read them by `count` and by index.
- `summed` and `product` reduce with `+` and `*`; `smallest` and `smallest of none` reduce with the feature's own `smaller of`, the second over an empty sequence.
- `mapped` adds one to every item; `zipped` adds two sequences of different lengths, the shorter reading as zero past its end.
- `mapped call` applies `doubled (x)` to a list; `zipped call` applies `scaled (a) by (b)` pairwise.
- `iterated` walks a sequence with `for`, adding into the feature variable `total`.
- `sum of (x$)` takes a sequence parameter; `passed` calls it with a list. `squares to (k)` gives a sequence result, which `squared` indexes.
- `letters` and `first byte` treat a string as the sequence of bytes it is; `marks total` reduces a feature-scope sequence; `halves` reduces floats.

## rules
- A sequence's items are numbers or enumerations; a `string` is a `uint8$`.
- `[a through b]` includes b, `[a to b]` stops before it, and both count down when a > b.
- An index counts from zero and is checked; `count` is an `int`.
- A function of one item applied to a sequence gives a sequence; of two items applied to two sequences, one as long as the longer, the shorter one's items zero past its end.
- A reduction folds one sequence from its first item; an empty sequence gives the type's zero.
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

## hostile
`i$[4]` on four items is a failed check from the IR. `_` with no sequence in the call is refused: "'_' marks the accumulator of a reduction". `x = 1` on a `for`'s item is refused. A sequence of a struct type is refused: "only numbers and enumerations in this milestone". More than 64K bytes of sequences in one case is a failed check in `arena_alloc`.
