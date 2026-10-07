# words
*the words a push takes after its items: how often it happens*

layer: runtime

> (suite) 2026-10-07T15:00:00
fm3 question 79, log 147 and 148. Ash, 7 October 2026: "we could also add `until (cond)` and `(n) times`."

## overview
A `<<` sends once, each time its line runs. The words after its items say otherwise: `if (c)`, only where the condition holds; `(n) times`, n pushes; `while (c)`, for as long as it holds, tested before each push; `until (c)`, until it holds, tested after each push; `forever`, standing. `if` comes first and goes with any of the others; a push takes one of the others.

`up$ << up$ + 1 (k) times` is k pushes, each working the item out again, so each reads the one before: after `up$ << 0`, four times gives `1 2 3 4`. The count is worked out once, before the first push. In a chain it covers the last item, as `while` does: `up$ << 0 << (up$ + 1) (4) times` is `0` and then four more.

`up$ << 1 << (up$ + 1) until (up$ == 5)` pushes, then asks: the item that makes the condition true goes out, so it gives `1 2 3 4 5`, where `up$ << 1 << (up$ + 1) while (_ < 5)`, which asks before it pushes, gives `1 2 3 4`. In an `until`, `_` is the item just pushed and the stream's own name is its latest item, which is that item: the two spellings are one program.

## interface
- `hello thrice` pushes a text three times, `out$ << "hello\n" (3) times`.
- `up to (k)` pushes `0` and then one more than the latest, k times; `chained` a first item and then four; `none` pushes zero times, which is nothing.
- `called (k)` pushes a call's result three times, `up$ << twice (k) (3) times`: the bracket before `times` is the count, not the call's second argument. `summed (a)` is `up$ << a + twice (a) (2) times`, the whole sum twice; `bracketed (a)` is `up$ << (a) (3) times`; `a range` pushes `[1 through 2]` twice.
- `maybe (c)` is `up$ << 5 if (c > 0) (2) times`: the condition is tested once, before the count.
- `three (k) times` and `bump (k) times` are functions whose names end in the word. `bumped by (k)` calls the second as a statement; `the name` pushes the first once, in brackets, `up$ << (three (4) times)`, and `the name twice` pushes it twice, `up$ << three (4) times (2) times`.
- `bump (k) times` pushes into `seen$`, a cell: the same loop with a store of the field each time round.
- `beats` pushes `0` and then three more into `beat$`, a stream at `1 hz`: one a second.
- `first$ << src$ (3) times` at feature scope is a line that stands for the first three items of `src$` and then no more; `first three` pushes five. `some$ << kept$ (2) times` is the same out of a stream that is stored, `kept$` being counted by `first two`.

- `counted to five` is the `until` line above, `counted under five` the `while` line beside it, and `counted to five by the item` the `until` written with `_`. `once` is `until (true)`: one push.
- `doubled past (limit)` pushes into `seen$`, a cell, until the value stored is past the limit.
- `maybe to three (c)` and `maybe under three (c)` put `if` before `until` and before `while`: the condition is tested once, and first.
- `beats to three` is `beat$ << 0 << (beat$ + 1) until (beat$ == 3)` at `1 hz`.
- `till$ << flow$ until (flow$ == 3)` at feature scope stands until an item of `flow$` is 3, which goes out; `flowed to three` pushes five. `open$ << flow$ until (ended key$)` stands until `key$` has ended, asked after each item that goes out: `flowed to the end of the key` pushes two items, ends `key$`, and pushes two more, and three are in `open$` (fm3 question 85).

## rules
- A bracketed group that stands directly before the word `times` is the push's count and never an argument, unless a declared function's name has `times` after the words of the phrase so far. So `twice (k) (3) times` is `twice (k)` three times, and a function of two groups pushed n times is `f (a) (b) (n) times`.
- The count is an integer, worked out once. Zero pushes nothing. A count worked out below zero is a failed check.
- Any item may be repeated but a task call: a text and a range are, where `while`, which tests a candidate, refuses a block.
- On a stream with a rate each push lands on the stream's beat and lasts its period.
- On a line that stands the count is kept for the line, a number of the context that starts at zero when the store does; after the count the line moves nothing. An `until` keeps a bit the same way, set by the condition after each item that goes out.
- `until` always pushes once. Its condition is asked of an item, so a range or a text is not repeated by it, as by `while`.
- A push that can be seen never to end is refused: `until (false)`, and `while (true)`.

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
>flowed to the end of the key() → 3

## hostile
`up$ << k times` is refused: "a push's count is the bracketed group before `times`, after the item: `x$ << item (n) times`". `up$ << 1 (-1) times` is refused: "a push cannot happen -1 times". `up$ << 1 (2) times if (k > 0)` is refused: "`if` comes first on a push, then how often: `x$ << item if (condition) (n) times`". `up$ << 1 (2) times while (_ < 3)` is refused: "a push takes `(n) times` or `while`, not both". `up$ << three (k) times`, where `three (int k) times` is declared, is refused: "'... (k) times' at the end of a push reads two ways: a function whose name ends `(...) times`, called and pushed once, or what stands before the bracket pushed that many times. For the call put it in brackets, `x$ << (name (k) times)`; for the count put the item in brackets, `x$ << (item) (k) times`". `up$ << 1 until (_ > 3) while (_ < 9)` is refused: "a push takes `until` or `while`, not both". `up$ << 1 until (_ > 3) if (k > 0)` is refused: "`if` comes first on a push, then how often: `x$ << item if (condition) until (...)`". `up$ << 1 until (false)` is refused: "this push would never end: its `until` can never hold"; `up$ << 1 while (true)`: "this push would never end: its `while` always holds". `out$ << src$ << "\n" (3) times` at feature scope is refused: "'out$ << src$ << "\n" (3) times' on a line that stands could be the first 3 items of 'src$', each with what follows it, or every item and what follows it 3 times: not built. For the first, put a stream between: `first$ << src$ (3) times` and `out$ << first$ << ... forever`" (fm3 question 84). `first$ << src$ (k) times` at feature scope is refused: "the count of a line that stands is a number written out, `first$ << src$ (3) times`: a count that is worked out is worked out once, and a line that stands from the start has no one moment for it. Not built".
