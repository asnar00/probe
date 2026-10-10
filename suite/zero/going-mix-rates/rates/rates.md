# rates
*two streams at two rates, mixed: a slot of one that has no item of the other*

layer: runtime

> (suite) 2026-10-10T20:10:00
Hop 39 (fm3 questions 86 and 124, log 234): the line that reads both, at `2 hz` and `5 hz`.

## overview
`slow$` is a clock at `2 hz` counting to 3, `quick$` one at `5 hz` counting in tens to 60, and `mix$ << slow$ + quick$ forever` reads both. The two have an item at one time at 0 s and at 1 s, and there `mix$` gets one item made from both. At every other time only one of them has an item, and the line reads the other for its latest (question 86).

## interface
- `begin` checks that the clocks have begun, and writes nothing.

## rules
- **One item of `mix$` at each time either stream has one**: 0 s, 200 ms, 400 ms, 500 ms, 600 ms, 800 ms and 1 s, seven items for the nine pushed.
- **A slot of one stream with no item of the other reads its latest**: at 500 ms `slow$` has its 2 and `quick$` nothing new, so the item is 2 and 30, `32`; at 600 ms `quick$` has 40 and `slow$` still holds 2, `42`.
- At 0 s and at 1 s it is one item, `11` and `63`, never two.

## testing
>begin() → "11\n" at 0 s, "21\n" at 200 ms, "31\n" at 400 ms, "32\n" at 500 ms, "42\n" at 600 ms, "52\n" at 800 ms, "63" at 1 s
