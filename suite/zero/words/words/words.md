# words
*the words a push takes after its items: how often it happens; and a stream that sums itself*

layer: runtime

> (suite) 2026-10-07T15:00:00
fm3 questions 79 and 80, log 147 to 149. Ash, 7 October 2026: "we could also add `until (cond)` and `(n) times`."

## overview
A `<<` sends once, each time its line runs. The words after its items say otherwise: `if (c)`, only where the condition holds; `(n) times`, n pushes; `while (c)`, for as long as it holds, tested before each push, and before the item is worked out where the condition does not read `_` (fm3 question 111); `until (c)`, until it holds, tested after each push; `forever`, standing. `if` comes first and goes with any of the others; a push takes one of the others.

`up$ << up$ + 1 (k) times` is k pushes, each working the item out again, so each reads the one before: after `up$ << 0`, four times gives `1 2 3 4`. The count is worked out once, before the first push. In a chain it covers the last item, as `while` does: `up$ << 0 << (up$ + 1) (4) times` is `0` and then four more.

`up$ << 1 << (up$ + 1) until (up$ == 5)` pushes, then asks: the item that makes the condition true goes out, so it gives `1 2 3 4 5`, where `up$ << 1 << (up$ + 1) while (_ < 5)`, which asks before it pushes, gives `1 2 3 4`. In an `until`, `_` is the item just pushed and the stream's own name is its latest item, which is that item: the two spellings are one program.

`sum$ << sum$ + x$ forever` is a running sum: one item of `sum$` for each item of `x$`, the sum so far. On the right of its own standing push a stream's own name is a read of its latest item and sets nothing off; the other stream there, `x$`, paces the line (Ash, 7 October 2026, question 80).

## interface
- `hello thrice` pushes a text three times, `out$ << "hello\n" (3) times`.
- `up to (k)` pushes `0` and then one more than the latest, k times; `chained` a first item and then four; `none` pushes zero times, which is nothing.
- `called (k)` pushes a call's result three times, `up$ << twice (k) (3) times`: the bracket before `times` is the count, not the call's second argument. `summed (a)` is `up$ << a + twice (a) (2) times`, the whole sum twice; `bracketed (a)` is `up$ << (a) (3) times`; `a range` pushes `[1 through 2]` twice.
- `maybe (c)` is `up$ << 5 if (c > 0) (2) times`: the condition is tested once, before the count.
- `three (k) times` and `bump (k) times` are functions whose names end in the word. `bumped by (k)` calls the second as a statement; `the name` pushes the first once, in brackets, `up$ << (three (4) times)`, and `the name twice` pushes it twice, `up$ << three (4) times (2) times`.
- `bump (k) times` pushes into `seen$`, a cell: the same loop with a store of the field each time round.
- `beats` pushes `0` and then three more into `beat$`, a stream at `1 hz`: one a second.
- `first$ << src$ (3) times` at feature scope is a line that stands for the first three items of `src$` and then no more; `first three` pushes five. `some$ << kept$ (2) times` is the same out of a stream that is stored, `first two` asking how many are waiting in `kept$`, `[count] (frame kept$)`.

- `counted to five` is the `until` line above, `counted under five` the `while` line beside it, and `counted to five by the item` the `until` written with `_`. `once` is `until (true)`: one push.
- `doubled past (limit)` pushes into `seen$`, a cell, until the value stored is past the limit.
- `maybe to three (c)` and `maybe under three (c)` put `if` before `until` and before `while`: the condition is tested once, and first.
- `beats to three` is `beat$ << 0 << (beat$ + 1) until (beat$ == 3)` at `1 hz`.
- `till$ << flow$ until (flow$ == 3)` at feature scope stands until an item of `flow$` is 3, which goes out; `flowed to three` pushes five. `open$ << flow$ until (ended key$)` stands until `key$` ends: a condition that names neither the stream being moved, nor the line's target, nor `_`, is an event, and the line ends when it comes to hold, nothing going out after (fm3 question 85). `flowed to the end of the key` pushes two items, ends `key$`, and pushes two more, and two are in `open$`.
- `held$ << tick$ until (ended hold$)` is the same on a stream at `1 hz`: `passed till the hold ends` pushes 0, 1 and 2, ends `hold$`, and pushes 3. A push into a rated stream is followed by its step, so `hold$` ends at 3 s, the tick 3 would have had: the event is first, and 3 does not go out.
- `stopped$ << tock$ until (stop$ == 1)` ends at a stream's value. The condition is asked of each item of `stop$` as it arrives, by a second function of the line, and `stop$` needs no storage for it. `passed till the stop` pushes `stop$` 0, which ends nothing, then 1 at 3 s, where the line ends, then 0 again: the line has ended and does not start again, so neither 3 nor 4 goes out.

- `sums` pushes four items into `x$` and reads `sum$`, which started at zero, and `from$`, declared `int from$ << 100` and summed by `from$ << from$ + x$ forever`: 10 and 110. `summed to (k)` pushes a range: a hundred items, 5 050.
- `evens$ << evens$ + x$ if (x$ % 2 == 0) forever` is a sum of some, and `twice$ << x$ * 2 forever` a line that stands with no name of its own on its right; `some and twice` reads both.
- `tot$ << tot$ + y$ forever` is wired on, `out$ << (tot$ << "\n") forever`: `running` pushes four and the output has each sum on a line.
- `tally$` is summed, wired on and read by its name as well, in `tallied`.
- A line is set off by every stream its items name (fm3 questions 86 and 121, hop thirty-five). `out$ << (lead$ << " " << beside$ << "\n") forever` writes a line for each item of `lead$` and for each item of `beside$`, the other read for its latest, the zero of its type before its first: `labelled` says `lead$ << 1`, `beside$ << 3 << 4`, `lead$ << 2`, `beside$ << 5`, `lead$ << 3`, and writes `1 0`, `1 3`, `1 4`, `2 4`, `2 5`, `3 5`. Until then the line was paced by its first item alone and wrote `1 0`, `2 4`, `3 5` (fm3 question 106). `labelled past (100)` pushes a hundred items into `beside$`, which writes a hundred lines, and neither stream keeps more than its latest.
- A stream that keeps nothing does not fill (fm3 log 177). `let$ << through$ if (gate$ == 0) forever` reads `gate$` for its value now, so `gate$` is one number however often it is pushed: `gated (100)` pushes it a hundred times, then lets 5 through, shuts the gate and offers 7, and `passed$` is 5. `climbed to (100)` pushes a hundred items into `up$`, which is wired to the output and read by its name, and reads 100; `tallied to (100)` does the same through `tally$`, which its own line reads, another line takes and the function reads, 5 050; `asked of the stop (100)` pushes `stop$`, which a line's event watches, a hundred and one times and reads it; `beaten to (100)` is `up$` again at `1 hz`; and `fed to (100)` pushes a hundred into `fed$`, which a stream processor takes, `int two$ = twofold (fed$)`, and a function reads by name.

## rules
- A bracketed group that stands directly before the word `times` is the push's count and never an argument, unless a declared function's name has `times` after the words of the phrase so far. So `twice (k) (3) times` is `twice (k)` three times, and a function of two groups pushed n times is `f (a) (b) (n) times`.
- The count is an integer, worked out once. Zero pushes nothing. A count worked out below zero is a failed check, the IR's `check` and no more, as the library's refusal of an index out of range is: until fm3 log 186 it wrote its site first, which brought `print` into every store that says `(k) times`.
- Any item may be repeated but a task call: a text and a range are, where `while`, which tests a candidate, refuses a block.
- On a stream with a rate each push lands on the stream's beat and lasts its period.
- On a line that stands the count is kept for the line, a number of the context that starts at zero when the store does; after the count the line moves nothing. An `until` keeps a bit the same way, set by the condition after each item that goes out.
- `until` always pushes once. Its condition is asked of an item, so a range or a text is not repeated by it, as by `while`.
- A push that can be seen never to end is refused: `until (false)`, and `while (true)`.
- A line that stands may have an expression for its first item. The streams it names other than its own target pace it, and it is built for one: the expression is worked out for each item of that stream, the stream's name in it the item that has arrived and the target's own name the target's latest item, which before anything is pushed is the zero of its type or the first item its declaration gave it.
- What the target of such a line is kept as is the compiler's to work out, and does not show. `sum$`, read only by its name, is a cell: one number, a push a store, nothing that can fill, so a hundred items sum as four do. `tot$`, wired on and named nowhere else, has no storage, and the line keeps its last item itself. `tally$`, wired on and read by name, has no queue either: its latest item is kept, one number, stored where an item is pushed, and each item is handed to what is wired as it comes. So is `up$`, and so is any stream that something is wired to and that is otherwise only read by its name for its value now; a stream a line reads by name in its condition, `gate$`, is a cell. What still keeps a queue is a stream something reads in order: one that is counted, peeked, framed or ended, or handed to a task that walks.

## testing
>hello thrice() → "hello\nhello\nhello"
>up to (4) → "0\n1\n2\n3\n4"
>up to (0) → "0"
>up to (-1) → check
>chained() → "0\n1\n2\n3\n4"
>none() → "none"
>called (5) → "10\n10\n10"
>summed (1) → "3\n3"
>bracketed (7) → "7\n7\n7"
>maybe (1) → "5\n5\ndone"
>maybe (0) → "done"
>three (4) times → 12
>bumped by (5) → 5
>the name() → "12"
>the name twice() → "12\n12"
>a range() → "1\n2\n1\n2"
>beats() → "0\n1\n2\n3" at 1 hz
>first three() → "1\n2\n3"
>first two() → "5\n6\n5"
>counted to five() → "1\n2\n3\n4\n5"
>counted under five() → "1\n2\n3\n4"
>counted to five by the item() → "1\n2\n3\n4\n5"
>once() → "9"
>doubled past (100) → 128
>maybe to three (1) → "1\n2\n3\ndone"
>maybe to three (0) → "done"
>maybe under three (1) → "1\n2\ndone"
>maybe under three (0) → "done"
>beats to three() → "0\n1\n2\n3" at 1 hz
>flowed to three() → "1\n2\n3"
>flowed to the end of the key() → 2
>passed till the hold ends() → "0\n1\n2" at 1 hz
>passed till the stop() → "0\n1\n2" at 1 hz
>sums() → 10, 110
>some and twice() → 6, 8
>summed to (100) → 5050
>running() → "1\n3\n6\n10"
>tallied() → 6
>gated (100) → 5
>climbed to (100) → 100
>tallied to (100) → 5050
>asked of the stop (100) → 102
>beaten to (100) → 100
>fed to (100) → 100, 10100
>labelled() → "1 0\n1 3\n1 4\n2 4\n2 5\n3 5"
>labelled past (100) → 3
>begun at five (0) → 5, 5
>begun at five (100) → 105, 5555
>shut after (100) → 100, 1
>shut and pushed() → check
>twofold to (100) → 200

## hostile
`up$ << k times` is refused: "a push's count is the bracketed group before `times`, after the item: `x$ << item (n) times`". `up$ << 1 (-1) times` is refused: "a push cannot happen -1 times". `up$ << 1 (2) times if (k > 0)` is refused: "`if` comes first on a push, then how often: `x$ << item if (condition) (n) times`". `up$ << 1 (2) times while (_ < 3)` is refused: "a push takes `(n) times` or `while`, not both". `up$ << three (k) times`, where `three (int k) times` is declared, is refused: "'... (k) times' at the end of a push reads two ways: a function whose name ends `(...) times`, called and pushed once, or what stands before the bracket pushed that many times. For the call put it in brackets, `x$ << (name (k) times)`; for the count put the item in brackets, `x$ << (item) (k) times`". `up$ << 1 until (_ > 3) while (_ < 9)` is refused: "a push takes `until` or `while`, not both". `up$ << 1 until (_ > 3) if (k > 0)` is refused: "`if` comes first on a push, then how often: `x$ << item if (condition) until (...)`". `up$ << 1 until (false)` is refused: "this push would never end: its `until` can never hold"; `up$ << 1 while (true)`: "this push would never end: its `while` always holds". `out$ << src$ << "\n" (3) times` at feature scope is refused: "'out$ << src$ << "\n" (3) times': `(3) times` applies to the last item of its chain (fm3 question 84), so this is `src$` once and then `"\n"` 3 times, when the store starts, and a push then is not built (fm3 question 80). For the first 3 items of 'src$', each with what follows it, put the items in brackets: `out$ << (src$ << "\n") (3) times`". `z$ << z$ + x$ + y$ forever` was refused until hop thirty-five and is now a line over two streams, set off by an item of either with the other's latest (fm3 question 86; `suite/zero/tick`). `sum$ << sum$ + 1 forever` is refused: "a push into 'sum$' that reads 'sum$' and stands forever would never end: nothing else on its right paces it, and 'sum$' has no rate to. For one more item write it with no `forever`, in a function; for a clock give the stream a rate, `int sum$ at (1 hz)`". `first$ << src$ (k) times` at feature scope is refused: "the count of a line that stands is a number written out, `first$ << src$ (3) times`: a count that is worked out is worked out once, and a line that stands from the start has no one moment for it. Not built". A line that stands until an event may ask `ended x$` alone, or read one other stream by its name: `open$ << flow$ until (empty key$)` is refused, "a line that stands until an event ends when the event comes to hold (fm3 question 85), and this one is not built: `empty x$` is a stream processor's word, true on its one tick after the last. A line that stands ends with `until (ended x$)`"; `until (ended key$ or ended flow$)`, "... `ended` inside a larger condition, or of two streams. The condition may be `ended x$` alone"; `until (count key$ > 2)`, "... `count` asked of the stream the condition reads. An event's condition is `ended x$`, or reads one stream by its name, its value now: `until (stop$ == 1)`"; and one that reads two other streams, "... its condition reads 2 streams, 'b$' and 'c$', and an event's condition reads one".
