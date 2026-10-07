# edges
*an edge: a `<<` at feature scope that says `forever`, a standing connection with nothing stored between*

layer: runtime

> (suite) 2026-10-06T10:00:00
Parity hop 8 (questions 50 and 51, fm3 log 92): a stream no word reads has no storage, and a push into it is the call of each edge out of it.

## overview
`out$ << pair$ << " " forever` at feature scope is an edge: `forever` makes the push stand (fm3 question 79), so it holds for as long as the program runs, and everything pushed into `pair$` goes on into `out$`, a space after each item. Without the word a `<<` is one push, each time its line runs. Nothing is kept on the way. `pair$` is only ever pushed into and wired, no word anywhere in the store reads it, so it has no storage at all: `pair$ << 1 << 2 << 3` hands each item to each edge out of `pair$`, in the order the edges were written, and then goes on to the next item. There is nothing to fill, so a hundred items pass as three do.

## interface
- `n$`, `beat$`, `quick$`, `pair$` and `part$` are streams with no storage: each is the source of an edge and is named nowhere else but as the target of a push.
- `twice` pushes three numbers into `pair$`, which two edges carry into `out$`.
- `count down from (k)` pushes `k` down to 1 into `n$` and then writes `liftoff`; the edge that shows the numbers is the feature `shown`'s.
- `beats` pushes two items into `beat$`, a stream at `2 hz`, and then writes `end`.
- `turns` pushes by turns into `beat$` and into `quick$`, a stream at `5 hz`, and then writes `end`.
- `drum (k)` pushes one item into `quick$` and then 1 to `k` into `beat$`, a pass of a loop each, and then writes `end`.
- `summed down from (k)` pushes `k` down to 1 into `part$`; the edge `heard$ << part$ forever` carries each into `heard$`, a stream the sink `tally` is wired to, which adds each to `sum`.
- `levels` pushes four numbers into `level$`, which a standing filter, `out$ << level$ << "\n" if (level$ > 2) forever`, carries into the output where they are over two.
- `fed (k)` pushes 1 to `k` into `raw$`, a statement each, and gives `k`; `raw$` is read by the feature `tallied`'s sink.
- `a part summed down from (k)` calls `summed down from (k)`: a function whose name has the word `part` in it, which is the name of a stream and no reading of it.

## rules
- An item goes through every edge before the next item is pushed: `twice` writes `1 1`, then `2 2`, then `3 3`, each edge taking the item in turn. With a queue between, all three would go through the first edge and then all three through the second.
- An edge into a stream that is stored is that stream's push: `heard$` is a queue, `tally` reads it with `count`, `peek` and `advance`, and is run after each item arrives, so the queue never holds more than one and a countdown from 100 sums to 5 050. With `part$` stored it failed a check at the sixty-fifth item.
- An edge is its feature's: with `shown` off the numbers `count down from` pushes go nowhere, and it writes `liftoff` alone, for eighty as for three. Nothing waits for `shown` to come back on (question 51).
- A stream at a rate keeps its time whether or not anyone listens: `beats` pushes two items into `beat$` at `2 hz`, each lasting half a second, so `end` is written at 1 s with `shown` off as with it on.
- A stream with a rate has a beat, slots one period apart from 0 s, and an item pushed into it lands in the stream's next slot (question 52, fm3 log 98). `turns` pushes `1` into `beat$` at 0 s, which lasts half a second; at 0.5 s it pushes `2` into `quick$`, whose slots are every 200 ms, so `2` lands at 600 ms and lasts until 800 ms; then `3` into `beat$`, whose next slot is at 1 s; that slot ends at 1.5 s, and `end` is written then. The beat is the stream's whether or not anyone listens, so `end` is at 1.5 s with `shown` off too. Before the beat was built a push happened wherever now stood, and the three were at 0 s, 500 ms and 700 ms.
- A loop that pushes at a rate is what happens on each tick of that stream's clock (question 56, fm3 log 99). `drum (3)` pushes `0` into `quick$` at 0 s, which leaves the clock at 0.2 s, and then loops `beat$ << i`: the first pass waits for `beat$`'s next slot, at 500 ms, and every pass after it begins where the last one's half second ended, on the beat, so the loop finds the slot once, before it begins, and not on every pass: `1` at 500 ms, `2` at 1 s, `3` at 1.5 s, `end` at 2 s. `drum (0)` runs no pass and pushes nothing, so it waits for nothing, and `end` is written at 200 ms: the wait before the loop is made only where the loop's `while` holds of its first values.
- A consumer that is off holds nothing in a queue either: with `tallied` off, `fed (80)` pushes eighty items into `raw$` and none stays, the reader of the sink that is off being moved past each as it arrives. Left there, the sixty-fifth failed a check. Within one statement a queue still holds what that statement pushes, so `fed` pushes an item a statement.
- `if` on a line that stands is a filter (fm3 question 79, log 141): the condition is asked of each item as it arrives, the stream's own name in it being that item, and the item goes on only where it holds. `levels` pushes 1, 5, 2 and 7 and `5` and `7` are written. The words go in that order, `if (c)` and then `forever`.
- A stream is stored again the moment a word reads it: `count pair$` anywhere in the store would make `pair$` a queue and its edges nodes the scheduler runs.
- A word of a function's name is not a reading of a stream (parity hop 11, fm3 log 106). `a part summed down from (100)` is a phrase with the bare word `part` in it, and a bare word of a phrase may be a variable read; but the phrase is a call of the function of that name, so its words are the name and what it mentions is its argument. `part$` stays without storage and the hundred items sum to 5 050. Before this the phrase made `part$` a queue, silently, and both this case and `summed down from (100)` failed its check at the sixty-fifth item.

## testing
>twice() → "1 1\n2 2\n3 3"
>levels() → "5\n7"
>summed down from (100) → 5050
>a part summed down from (100) → 5050
>count down from (80) → "liftoff"
>beats() → "end" at 1 s
>turns() → "end" at 1.5 s
>fed (80) → 80
>drum (3) → "end" at 2 s
>drum (0) → "end" at 200 ms

## hostile
`out$ << pair$ << " "` with no `forever` is refused: "'out$ << pair$ << " "' has a stream on its right and no `forever`. If it is wiring, everything that arrives in 'pair$' going on into 'out$', write `out$ << pair$ << " " forever`. If it is one push when the store starts, of what 'pair$' holds then, that is what the line says (fm3 question 79) and it is not built: push it from a function". `out$ << pair$ << " " while (pair$ < 3)` is refused: "a line at feature scope that stands until its `while` fails is not built: wiring moves every item its stream receives, `x$ << y$ forever`"; with `while` and `forever` both: "a push takes `while` or `forever`, not both: `while` is `forever` with an end". `out$ << pair$ forever if (pair$ > 2)` is refused: "`forever` is the last word of its line: `x$ << item if (condition) forever`". `pair$ << pair$ + 1 forever` is refused: "a push into 'pair$' that reads 'pair$' and stands forever would never end: each item it pushes is something new on its own right, and 'pair$' has no rate to pace it. For one more item write it with no `forever`, in a function; for a clock give the stream a rate, `int pair$ at (1 hz)`"; and `beat$ << beat$ + 1 forever`, on a stream with a rate: "a stream that feeds itself forever at a rate is a clock, and is not built: it needs a schedule ordered by time, and a store's clock is still moved by the code that pushes. Until then a function's push says it with an end, `beat$ << 0 << (beat$ + 1) while (_ < 4)`". `out$ << 10 forever` is refused: "nothing on the right of 'out$ << 10' is a stream: `forever` makes a push happen again whenever what is on its right has something new, and a value never has". `pair$ << 1 forever` in a function is refused: "`forever` in a function is a line that would set up a standing connection each time the function runs: not built. Wire it at feature scope, where it stands from the start".
