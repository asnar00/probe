# sampled
*a read by a place cannot fail: an array says, where it is declared and once, how a coordinate is read*

layer: runtime

> (suite) 2026-10-10T10:00:00
fm3 question 127, `sampler.md`, log 236. Ash, 10 October 2026: "For array indices, I want to try and avoid out of bounds exceptions as another billion-dollar-mistake (BDM). I'd actually want to introduce the concept of a sampler, like a texture sampler (GPU shaders)". And: "If we don't specify sampler behaviour, it should default to sensible values eg 0 outside, t0=0, dt=1".

## overview
What is written in an array's brackets is a coordinate, and no coordinate stops the program. An array that says nothing is read by a plain place, and a read outside its items gives zero, the value a declaration with nothing given has: 0 for a number, a structure made from its own defaults for a structure. So `a[i]` compiles for any `i`, a negative one included, and `a[12]` of four items is 0.

An array may say otherwise where it is declared, after its name and before its value, as a stream says its rate: `int a[] else 7 = [1, 2]` reads 7 outside, and `int kick[] wrapped = [...]` reads round again, one past the end being the first and one before the start the last. The same is said of a `string`, and of a parameter, `(int a[] else 5)`, which is how the function reads what it is handed.

The rules govern a read by a place and nothing else: `kick[]`, `[count] (kick[])` and a reduce are the items that are there.

## interface
- `item (i)` reads an array that says nothing; `written` reads it by three places written out, one inside, one past the end and one before the start; `ranged (i)` reads a range.
- `or seven (i)` and `or less (i)` say `else`; `kicked (i)` reads `kick[]`, sixteen items and `wrapped`; `waltzed (i)` reads `waltz[]`, fifteen; `round (a[]) at (i)` is a parameter that says `wrapped`, handed five items by `round five` and none by `round none`.
- `letter (i)` reads a `string` that says nothing, `spaced (i)` one that says `else char (32)`, and `characters (i)` an array of `char`.
- `corner (i)` reads an array of `point`, whose `y` is 7 where nothing is given; `shade (i)` an array of an enumeration that says `else blue`.
- `handed (i)` hands one array, declared `else 9`, to `pick`, whose parameter says `else 5`, and to `plainly`, whose parameter says nothing.
- `viewed (i)` reads `three[]` through a second name that says `wrapped`, and by its own.
- `looped` pushes twenty beats into `beat$`; the line `hit$ << kick[beat$] forever` reads the pattern by each, and `out$` writes what it gave.

## rules
- **Outside is zero where nothing is said.** `item (4)` and `item (-1)` are 0; `letter (9)` is the zero character; `corner (2)` is `point`'s own defaults, 7.
- **`else (v)`**: outside reads `v`, a value of the item's type written out: a number, with a minus where the type has one, a name of an enumeration, `char (n)`.
- **`wrapped`**: outside reads round again by a true remainder, so `kicked (-1)` is the last item and `kicked (30)` the fifteenth. An array of no items has nothing to go round and reads zero.
- **The rule is the name's, said where the name is declared.** A parameter that says a rule reads by it whatever the array it is handed said, and a parameter that says nothing reads zero outside whatever the caller's array said: `handed (4)` is 50, `pick`'s 5 and `plainly`'s 0, and the caller's 9 nowhere. What a line means does not depend on a declaration somewhere else.
- **A second name for the same items reads them by its own rule**: `int loop[] wrapped = three[]` copies nothing, and `viewed (5)` is 30 through `loop[]` and 0 through `three[]`.
- **What it costs**: nothing is checked and nothing can fail. The read is written where it stands: the place compared with how many items there are, and a choice between the item and the outside value. Where the place is written out and the array's items are written on its declaration, the compiler compares them itself and the program compares nothing: `written` reads one item and adds two zeros. `kick[i]`, sixteen items, is a mask; `waltz[i]`, fifteen, a remainder.

## testing
>item (0) → 10
>item (2) → 30
>item (4) → 0
>item (-1) → 0
>written() → 30
>ranged (4) → 5
>ranged (5) → 0
>ranged (-2) → 0
>or seven (1) → 2
>or seven (2) → 7
>or seven (-3) → 7
>or less (0) → 1
>or less (9) → -1
>kicked (0) → 1
>kicked (14) → 1
>kicked (16) → 1
>kicked (30) → 1
>kicked (-1) → 0
>kicked (-2) → 1
>waltzed (3) → 1
>waltzed (15) → 1
>waltzed (17) → 0
>waltzed (-1) → 1
>waltzed (-2) → 0
>waltzed (-15) → 1
>waltzed (-16) → 1
>round five (7) → 2
>round five (5) → 0
>round five (-1) → 4
>round none (3) → 0
>letter (0) → 108
>letter (9) → 0
>letter (-1) → 0
>spaced (1) → 101
>spaced (5) → 32
>spaced (-1) → 32
>characters (4) → 120
>characters (5) → 32
>corner (1) → 34
>corner (2) → 7
>corner (-1) → 7
>shade (0) → 0
>shade (1) → 1
>shade (5) → 2
>handed (1) → 22
>handed (4) → 50
>handed (-1) → 50
>viewed (1) → 40
>viewed (5) → 30
>viewed (-1) → 30
>looped() → "1 0 0 0 1 0 0 0 1 0 0 0 1 0 1 0 1 0 0 0 "

## hostile
Two rules for outside on one array, `int a[] wrapped else 0 = [1, 2]`: "'a[]' says twice what a read outside it gives, `wrapped` and `else`: an array has one rule for outside".

A rule on something that is not an array or a string, `int a wrapped = 3`: "`wrapped` says how an array is read by a place, and 'a' is one int. It is said of an array or a string where it is declared, `int a[] wrapped = [...]`". On a stream, `int x$ wrapped`: "`wrapped` on 'x$': a stream has no end to go round to: its items go on arriving. A stream's declaration may say what a read at a time between two items gives, `nearest` or `linear`, and what a read before its first item gives, `else 0` (fm3 question 127)" (`suite/zero/sampled-streams` has a stream's own words). On a parameter that is one value, `(int i else 0)`: "`else` says how an array is read by a place, and 'i' is one int. It is said of an array or a string where it is declared, `int a[] else 0 = [...]`".

`else` with a value that is not one of the item's type written out, `int a[] else 1.5 = [1, 2]`, or `else k` with `k` a variable: "`else` on 'a[]' says what a read outside it gives, a value of the item's type written out; its items are int: write a number, `else 0`". On a `string`, `string s else 7.5 = "ab"`: "... its items are characters: write `else char (32)`". On an array of structures, `point ps[] else 3 = [point(1)]`: "... its items are structures, and a read outside it gives a `point` made from its own defaults: leave `else` out". With no value, `int a[] else = [1, 2]`: "`else` on 'a[]' wants the value a read outside gives, `a[] else 0`".

`clamped`, `from (a) to (b)`, `nearest` and `linear` are built since hop forty-one and are `suite/zero/sampled-between`'s. Ruled and not built, refused by name:
- `int a[] mirrored = [1, 2]`: "`mirrored` on 'a[]', a read outside going back the way it came, is ruled and not built yet (fm3 question 127). What a read outside the items gives is `else (v)`, `wrapped` or `clamped`, and zero where nothing is said".
