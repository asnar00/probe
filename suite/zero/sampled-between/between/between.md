# between
*a start and a step, and between: the coordinates an array spans, and what a coordinate between two items reads*

layer: runtime

> (suite) 2026-10-10T18:00:00
fm3 question 127's second part, `sampler.md`, log 240. Ash, 10 October 2026: "If you think about it, a sampler also has two floats (t0, dt) that scale and offset the value inside the [] to get to an index, just like for streams and time."

## overview
What is written in an array's brackets is a coordinate, and the array's declaration says how one becomes an item. `float curve[] from (0) to (1) linear clamped = [0.0, 0.1, 0.4, 0.9, 1.0]` says three things: its items span the coordinates 0 to 1, the first item at 0 and the last at 1, so a read is by a fraction of the whole and does not know how many items there are; a coordinate between two items reads the two blended; and one outside reads the nearest edge. `curve[0.5]` is 0.4, the middle item, and `curve[0.625]` is 0.65, halfway from 0.4 to 0.9.

An array that says nothing is read by a plain place, and a coordinate between two items reads the item at or before it, which is what a stream read at a time gives.

A case line compares whole numbers, so every function here gives thousandths.

## interface
- `curved (k)` reads `curve[]` at `k` thousandths; `halfway` and `five eighths` read it at a coordinate written out.
- `stepped (k)` reads `steps[]`, which says nothing, at `k` thousandths of a place; `written` reads it at three decimals written out, one inside, one below the first item and one past the last.
- `closest (k)` reads an array that says `nearest`; `blended (k)` one that says `linear` and nothing for outside, `blended or one (k)` one that says `linear else 1.0`; `round (k)` reads `ring[]`, `linear wrapped`.
- `barred (k)` and `barred at (i)` read `bars[]`, which spans `from (2) to (6)`, by a decimal and by a whole number; `lowered (k)` reads `below[]`, `from (-1) to (1) nearest`.
- `edge (i)` and `edges` read an array of three that says `clamped` by a whole place; `held (a[]) at (i)` is a parameter that says `clamped`, handed four items and none.
- `spread (a[]) at (k)` is a parameter that says `from (0) to (1) linear clamped`, handed three items, one, and none.
- `letter (k)` reads a `string` that says `from (0) to (1) clamped`.

## rules
- **`from (a) to (b)`**: the first item is at `a` and the last at `b`. The index is the coordinate less the start, over the step, the step being the span over the last item's place: `barred (4000)` is the middle item of five, and `barred at (3)` the one before it at or before. Each end is a number written out, and the two are not the same number. An array of one item reads it everywhere inside: `spread one (300)` is 700.
- **Between, with nothing said: the item at or before.** `stepped (2500)` and `stepped (2999)` are item 2; `stepped (-500)` is before the first item and reads zero, as any coordinate outside does. The last item is read until the place after it: `barred (6100)` is still the fifth of five, and `barred (7000)`, a whole step past it, is outside.
- **`nearest`**: the closer of the two, and the later where the coordinate is exactly halfway: `closest (2400)` is item 2 and `closest (2500)` item 3.
- **`linear`**: the item at or before plus the difference to the next times how far along. Each of the two is read by the array's own rule for outside: with nothing said the item past the last is zero, so `blended (2500)` is half of 0.8; `blended or one (2500)` is halfway from 0.8 to 1; `round (3500)` is halfway from the last item to the first. `linear` is for items that are a `float`.
- **`clamped`**: the nearest of the first and last items, `edge (-4)` 10 and `edge (9)` 30. An array of no items has no edge and reads zero.
- **What it costs**: nothing is checked and nothing can fail. A whole place on an array that says no `from` is the read it always was. A coordinate that is a `float` is brought to the item at or before, five operations; `from` is a multiply and a divide, and a subtract where the start is not 0; `linear` is two reads and three operations between them.

## testing
>curved (500) → 400
>curved (625) → 650
>halfway() → 400
>five eighths() → 650
>curved (0) → 0
>curved (125) → 50
>curved (1000) → 1000
>curved (-200) → 0
>curved (1500) → 1000
>stepped (2000) → 400
>stepped (2500) → 400
>stepped (2999) → 400
>stepped (4000) → 1000
>stepped (5000) → 0
>stepped (-500) → 0
>written() → 400
>closest (2400) → 400
>closest (2500) → 900
>closest (-400) → 0
>closest (4400) → 1000
>closest (4600) → 0
>blended (500) → 300
>blended (1500) → 600
>blended (2500) → 400
>blended (-500) → 100
>blended (3000) → 0
>blended or one (2500) → 900
>blended or one (-500) → 600
>blended or one (4000) → 1000
>round (500) → 500
>round (3500) → 100
>round (-500) → 100
>round (4250) → 250
>barred (2000) → 100
>barred (4000) → 300
>barred (4500) → 300
>barred (6000) → 500
>barred (1900) → 0
>barred (6100) → 500
>barred (7000) → 0
>barred at (3) → 200
>barred at (5) → 400
>barred at (7) → 0
>lowered (-1000) → 100
>lowered (0) → 300
>lowered (200) → 300
>lowered (300) → 400
>lowered (1000) → 500
>lowered (1300) → 0
>edge (-4) → 10
>edge (0) → 10
>edge (1) → 20
>edge (2) → 30
>edge (9) → 30
>edges() → 1230
>held four (-1) → 1
>held four (2) → 3
>held four (9) → 4
>held none (0) → 0
>spread three (0) → 0
>spread three (250) → 500
>spread three (500) → 1000
>spread three (750) → 750
>spread three (2000) → 500
>spread one (300) → 700
>spread one (-300) → 700
>spread none (500) → 0
>letter (0) → 108
>letter (500) → 116
>letter (1000) → 120
>letter (3000) → 120
