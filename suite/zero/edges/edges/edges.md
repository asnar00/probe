# edges
*an edge: a stream on the right of `<<` at feature scope, a standing connection with nothing stored between*

layer: runtime

> (suite) 2026-10-06T10:00:00
Parity hop 8 (questions 50 and 51, fm3 log 92): a stream no word reads has no storage, and a push into it is the call of each edge out of it.

## overview
`out$ << pair$ << " "` at feature scope is an edge: it stands for as long as the program runs, and everything pushed into `pair$` goes on into `out$`, a space after each item. Nothing is kept on the way. `pair$` is only ever pushed into and wired, no word anywhere in the store reads it, so it has no storage at all: `pair$ << 1 << 2 << 3` hands each item to each edge out of `pair$`, in the order the edges were written, and then goes on to the next item. There is nothing to fill, so a hundred items pass as three do.

## interface
- `n$`, `beat$`, `quick$`, `pair$` and `part$` are streams with no storage: each is the source of an edge and is named nowhere else but as the target of a push.
- `twice` pushes three numbers into `pair$`, which two edges carry into `out$`.
- `count down from (k)` pushes `k` down to 1 into `n$` and then writes `liftoff`; the edge that shows the numbers is the feature `shown`'s.
- `beats` pushes two items into `beat$`, a stream at `2 hz`, and then writes `end`.
- `turns` pushes by turns into `beat$` and into `quick$`, a stream at `5 hz`, and then writes `end`.
- `summed down from (k)` pushes `k` down to 1 into `part$`; the edge `heard$ << part$` carries each into `heard$`, a stream the sink `tally` is wired to, which adds each to `sum`.
- `fed (k)` pushes 1 to `k` into `raw$`, a statement each, and gives `k`; `raw$` is read by the feature `tallied`'s sink.

## rules
- An item goes through every edge before the next item is pushed: `twice` writes `1 1`, then `2 2`, then `3 3`, each edge taking the item in turn. With a queue between, all three would go through the first edge and then all three through the second.
- An edge into a stream that is stored is that stream's push: `heard$` is a queue, `tally` reads it with `count`, `peek` and `advance`, and is run after each item arrives, so the queue never holds more than one and a countdown from 100 sums to 5 050. With `part$` stored it failed a check at the sixty-fifth item.
- An edge is its feature's: with `shown` off the numbers `count down from` pushes go nowhere, and it writes `liftoff` alone, for eighty as for three. Nothing waits for `shown` to come back on (question 51).
- A stream at a rate keeps its time whether or not anyone listens: `beats` pushes two items into `beat$` at `2 hz`, each lasting half a second, so `end` is written at 1 s with `shown` off as with it on.
- A stream with a rate has a beat, slots one period apart from 0 s, and an item pushed into it lands in the stream's next slot (question 52, fm3 log 98). `turns` pushes `1` into `beat$` at 0 s, which lasts half a second; at 0.5 s it pushes `2` into `quick$`, whose slots are every 200 ms, so `2` lands at 600 ms and lasts until 800 ms; then `3` into `beat$`, whose next slot is at 1 s; that slot ends at 1.5 s, and `end` is written then. The beat is the stream's whether or not anyone listens, so `end` is at 1.5 s with `shown` off too. Before the beat was built a push happened wherever now stood, and the three were at 0 s, 500 ms and 700 ms.
- A consumer that is off holds nothing in a queue either: with `tallied` off, `fed (80)` pushes eighty items into `raw$` and none stays, the reader of the sink that is off being moved past each as it arrives. Left there, the sixty-fifth failed a check. Within one statement a queue still holds what that statement pushes, so `fed` pushes an item a statement.
- A stream is stored again the moment a word reads it: `count pair$` anywhere in the store would make `pair$` a queue and its edges nodes the scheduler runs.

## testing
>twice() → "1 1\n2 2\n3 3"
>summed down from (100) → 5050
>count down from (80) → "liftoff"
>beats() → "end" at 1 s
>turns() → "end" at 1.5 s
>fed (80) → 80

## hostile
`out$ << pair$ << " " while (pair$ < 3)` is refused: "an edge has no `while`: it moves every item its stream receives". `pair$ << pair$` is refused: "'pair$' would feed itself".
