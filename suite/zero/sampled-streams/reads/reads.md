# reads
*a stream says how it is read, in an array's words: `nearest`, `linear`, `else (v)`*

layer: runtime

> (suite) 2026-10-10T20:00:00
fm3 question 127, `sampler.md`, log 250. Ash, 10 October 2026: "A sampler is a bit like a slice, right, it's a view of an underlying array (and I guess so is a stream)."

## overview
A stream read at a time, `x$ at (t)`, is a read by a coordinate, as an array read by a place is, and the stream's declaration says how the coordinate becomes an item, in the words an array uses. `float32 level$ at (1 khz) linear` says that a time between two items reads the two blended; `int near$ nearest` that it reads the closer; `int note$ at (4 hz) else 60` that a read before the stream's first item gives 60.

A stream that says nothing reads, between two items, the item at or before the time, which is what its name was at that time, and zero before its first item. After its latest item every stream reads its latest.

The words stand after the stream's name and its rate, before `<<` or `=`.

## interface
- `held at (m)`, `nearest at (m)` and `blended at (m)` read a stream at `1 khz` holding 10, 20, 30 and 40 at `m` microseconds: with nothing said, with `nearest`, and with `linear` (in tenths); `blended halfway` reads it at a time written out.
- `slid at (m)` and `neared at (m)` read streams with no rate, `float32 slide$ linear` and `int near$ nearest`, whose three items fall at 0 s, 500 ms and 1 s, a push into `pace$` at `2 hz` between them moving the clock.
- `before any`, `latest before any`, `after one` and `after two` read `int note$ at (4 hz) else 60` by its name, by `latest`, by a time, and through `stepped`, a processor wired to it whose line is `d$ << x$ - x$[-1]`; `seen and pushed (k)` reads `int seen$ else 7` by its name before and after a push.
- `local before (k)` and `local stored before (k)` declare a stream with `else` in a function, one kept as one value and one stored.
- `handed (m)` hands a stream declared `nearest` to `read (x$) at (m)`, a function over a stream.

## rules
- **Between two items** (`nearest`, `linear`, nothing). With nothing said the item at or before: `held at (1600)` is 20 and `held at (1999)` still 20. `nearest` is the closer, the earlier where the time is halfway: `nearest at (1600)` is 30, `(1500)` 20. `linear` blends the two by how far along the time is: `blended at (1500)` is 25.0 and `(1250)` 22.5. An item's own time reads that item under all three.
- **`linear` wants items that can be blended**, a `float`: on whole numbers it is refused, as it is for an array (fm3 question 143).
- **Before the first item** (`else (v)`). `note$` reads 60 before anything is pushed, by its name, by `latest note$` and by `note$ at (1 s)`: `before any` is `60, 60`. Once 64 is pushed its name is 64, and a time before it, `note$ at (0 s - 1 s)`, is still 60. A look back in a processor wired to it reads the same: `stepped`'s `x$[-1]` is 60 for the first item, so `after one` gives `step$` as 4, and `after two` 3. With nothing said all of these are zero.
- **After the latest item** every stream reads its latest: `held at (99000)` is 40.
- **What it costs**, the read alone, counted as it runs: the item at or before about 38 where the compiler knows the stream's rate and the time's divisor, 31 where the time is written out; `nearest` about 40; `linear` about 75, a second item read and the blend. A stream with no rate is searched by its ticks, 130 to 250.
- **The rule goes with the stream**: `handed (1600)` is 30, the function reading by the `nearest` its argument was declared with. An `else` does not go with it: read through a parameter before its first item a stream gives zero.

## testing
>held at (1600) → 20
>held at (1999) → 20
>held at (2000) → 30
>held at (-5) → 0
>held at (99000) → 40
>nearest at (1600) → 30
>nearest at (1500) → 20
>nearest at (1400) → 20
>blended at (1500) → 250
>blended at (1250) → 225
>blended at (2000) → 300
>blended at (2750) → 375
>blended at (99000) → 400
>blended at (-5) → 0
>blended halfway() → 250
>slid at (250) → 15
>slid at (750) → 30
>slid at (500) → 20
>slid at (9000) → 40
>neared at (300) → 2
>neared at (200) → 1
>before any() → 60, 60
>latest before any() → 60, 7
>after one() → 6464, 4
>after two() → 6760, 3
>seen and pushed (3) → 703
>local before (8) → 508
>local stored before (4) → 904
>handed (1600) → 30
>handed (1400) → 20
>paced() → 0

## hostile
A stream takes three of an array's words and no others. `int a$ wrapped` is refused: "`wrapped` on 'a$': a stream has no end to go round to: its items go on arriving. A stream's declaration may say what a read at a time between two items gives, `nearest` or `linear`, and what a read before its first item gives, `else 0` (fm3 question 127)". `int a$ mirrored`: "`mirrored` on 'a$': a stream has no end to turn back at: its items go on arriving", and the same close. `int a$ clamped`: "`clamped` on 'a$': a read after a stream's latest item gives the latest already, and what a read before its first gives is said with `else`". `int a$ from (0) to (1)`: "`from` on 'a$': a stream's start and its step are its phase and its rate, which `at (n hz)` says".

`int a$ at (10 hz) linear` is refused: "`linear` on 'a$' blends the two items either side of a coordinate, and two whole numbers blended are not a whole number: declare the items `float`. `nearest` gives the closer of the two, and with nothing said it is the item at or before". `int a$ nearest linear`: "'a$' says twice what a read between two items gives, `nearest` and `linear`: a stream has one rule for between". `int a$ else 1.5`: "`else` on 'a$' says what a read outside it gives, a value of the item's type written out; its items are int: write a number, `else 0`".

The words come after the rate: `float a$ linear at (10 hz)` is refused, "'a$' says how it is read before it says its rate: the rate comes first, `float a$ at (n hz) linear`". They are said where a stream is declared and not on a parameter: `on (int n) << f (int x$ nearest)` is refused, "`nearest` on 'x$', a parameter: a stream is read by what its own declaration says, and `nearest`, `linear` and `else` are written there, after its name and its rate, `int x$ nearest`. A function reads the stream it is handed by that". And a stream said by a line of a stream processor takes none: `int a$ else 5 = x$ + a$[-1]` there is refused, "'a$' is said by a line of a stream processor, and how such a stream is read, `nearest`, `linear` or `else`, is not built: say it where the stream the processor is wired into is declared".
