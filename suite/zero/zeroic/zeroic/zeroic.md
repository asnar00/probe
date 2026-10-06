# zeroic
*a stream processor with no loop in it: every line of its body holds for every item that arrives (fm3 question 75)*

layer: runtime

> (suite) 2026-10-06T10:00:00
Parity hop sixteen, transformation 58 (fm3 log 124): question 75's first rule. A declaration with `<<` whose body has no loop and applies no reader's word to its input is read the new way: each of its lines holds for each item of its input as it arrives, and inside it the input's bare name is that item.

## overview
`on (int d$) << doubled (int x$)` with the one line `d$ << x$ * 2` is a stream processor: for each item that arrives in its input, twice that item goes out. It has no loop, no `peek`, no `count` and no `advance`; `doubled by walking` beside it is the same processor in the form every task had, a loop that walks what has arrived, and the two give the same items. Which reading a body has is told from the body (fm3 question 65): no loop, nothing assigned, and no reader's word on the input is the new reading.

A wiring, `int d$ = doubled(x$)`, is a standing connection as it always was. What changes is what the compiler makes of it. For each wiring it writes a function of one item, and where nothing else in the store reads the input, the input has no storage at all: a push into `x$` is that function's call, once an item, and a block or a literal pushed is a loop of calls the compiler writes, its count the literal's length where that is known. Where something else reads the input by a reading word, or it has items on its declaration, or it is the input device, the stream keeps its storage and the compiler writes the walking form itself.

## interface
- `doubled (x$)` pushes twice each item; `doubled by walking (x$)` is the same with a loop, kept beside it to be measured against.
- `placed (x$)` pushes ten times each item and which item it is: `position x$` is the item's place, counted from 0, an `index`.
- `trebled (x$)` says a stream of its own, `int two$ = x$ * 2`, and pushes `two$ + x$`.
- `class of (c)` classes a character, and `classes (c$)` pushes the class of each: a one-item function applied to the input's name is that function of the present item.
- `one at a time`, `as a block`, `said`, `walked`, `from a literal`, `from a range`, `through two`, `wired twice`, `over text`, `typed`, `also read`, `many` and `a line said` are the cases.

## rules
- Every line holds for every item: `d$ << x$ * 2` pushes one item for each that arrives.
- The input's bare name is the present item, and `position x$` is which item it is.
- A line that says a stream, `int two$ = x$ * 2`, is the processor's own: one value an item.
- `x$`, `y$`, `c$`, `once$`, `p$`, `r$`, `m$` and `text$` have no storage: nothing reads them but the processors wired to them, so a push into one is the processor's call. `once$` is the output of one processor and the input of the next, and `through two` is one call inside another.
- `lit$` and `ran$` have their items on their declarations, `s$` is read by `count s$` in `also read`, `old$` is walked by a task, and `in$` is the input device: each keeps its storage, and its processor is run over what has arrived, by the scheduler, as a task is.
- A processor wired twice has two of everything: `placed` on `p$` and on `r$` each count their own items, so `wired twice` gives the third item of `p$` place 2 and the first of `r$` place 0.
- `text$` is a stream of `char` with no storage because every push into it is of a string literal; `text$ << "ab 1"` is four calls in a loop that says four.

## testing
>one at a time() → 1, 326
>as a block() → 392
>said() → 326
>walked() → 326
>from a literal() → 326
>from a range() → 510
>through two() → 352
>wired twice() → 6182, 170
>over text() → 5102
>typed() with in "a1" → 212
>also read() → 2, 24
>many (60) → 60120
>many (3) → 3006
>a line said() → 1215

## hostile
A processor's body that pushes into anything but its own output, declares a name without a `$`, puts an `if` round a line or a `while` on a push is refused, with what to write. `int d$ = doubled(i$)` inside a function is refused: a stream processor is wired at feature scope, and running one inside a function is not built. `y$` has no storage, so nothing limits what is pushed into it; `q$`, which `doubled` fills from it, is a queue read only after the statement, so `many (80)` fails the queue's check at the sixty-fifth item as a push of eighty into any stored stream does.
