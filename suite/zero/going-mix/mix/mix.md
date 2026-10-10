# mix
*two streams with a rate in one slot: one item of the line that reads both*

layer: runtime

> (suite) 2026-10-10T20:00:00
Hop 39 (fm3 questions 86, 121 and 124, log 234): `schedule.md`'s third shape.

## overview
`left$` and `right$` are clocks at `1 hz`, counting in ones and in tens, and `mix$ << left$ + right$ forever` reads both. In every slot each of them gets an item, and `mix$` gets **one item a slot, made from that slot's two**: `11`, `22`, `33`. Until this was built the line ran where each stream was pushed, twice a slot, and its first item of every slot was made with the right channel of the slot before: a value that was never true of anything. `third$` is a third clock, in hundreds, and `all$ << left$ + right$ + third$ forever` reads the three, into a stream that has the same rate.

## interface
- `begin` checks that the clocks have begun, and writes nothing: the lines write.
- `so far` gives `mix$`'s latest item.
- `louder` pushes 100 into `left$`.

## rules
- **A line set off by two or more streams, each with a rate, runs at the close of a slot.** A push into any of them does not run the line: it marks it. When everything else due at that time has had its turn, the line runs once and reads the latest of each stream. So the order the clocks are written in does not show, and there is one item of `mix$` at 0 s, 1 s and 2 s.
- **Three are as two**: `all$` gets `111`, `222`, `333`, one a slot.
- **The line's own item lands at the time of the slot**, with no wait and no step: `all$` has a rate and its items are at 0 s, 1 s and 2 s, where a function's push into it would move that function's now on a second.
- **The close of a time comes after a case's function has had its turn at that time.** The clocks' first items are there before `begin` or `so far` is called, the lines having stood since the store started; the slot at 0 s closes when the function has returned or stepped past it. `so far` asks before that and gives 0: `mix$` holds nothing yet.
- **Two items for one slot: the later replaces the earlier** (fm3 question 124, provisional). `louder` pushes 100 into `left$` at 0 s, the slot its clock has already pushed 1 into. The line that mixes runs once, at the close, and reads the latest: `110`, and `210` for the three. The 1 is in no item of `mix$`. The clock then reads 100 as its stream's latest, its `if` fails, and it stops; the other two go on.
- A line over a stream with a rate and one without is not such a line: it runs where each is pushed (`tick`'s `mixed`).

## testing
>begin() → "11 111\n22 222\n33 333" at 1 hz
>so far() → 0
>louder() → "110 210\n" at 0 s, "120 320\n" at 1 s, "130 430" at 2 s
