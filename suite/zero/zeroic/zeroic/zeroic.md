# zeroic
*a stream processor with no loop in it: every line of its body holds for every item that arrives (fm3 question 75)*

layer: runtime

> (suite) 2026-10-06T10:00:00
Parity hop sixteen, transformation 58 (fm3 log 124): question 75's first rule. A declaration with `<<` whose body has no loop and applies no reader's word to its input is read the new way: each of its lines holds for each item of its input as it arrives, and inside it the input's bare name is that item.

## overview
`on (int d$) << doubled (int x$)` with the one line `d$ << x$ * 2` is a stream processor: for each item that arrives in its input, twice that item goes out. It has no loop, no `peek`, no `count` and no `advance`; `doubled by walking` beside it is the same processor in the form every task had, a loop that walks what has arrived, and the two give the same items. Which reading a body has is told from the body (fm3 question 65): no loop, nothing assigned, and no reader's word on the input is the new reading.

A wiring, `int d$ = doubled(x$)`, is a standing connection as it always was. What changes is what the compiler makes of it. For each wiring it writes a function of one item, and where nothing else in the store reads the input, the input has no storage at all: a push into `x$` is that function's call, once an item, and a block or a literal pushed is a loop the compiler writes with the function's lines in it (parity hop seventeen, fm3 log 134), its count the literal's length where that is known. Where something else reads the input by a reading word, or it has items on its declaration, or it is the input device, the stream keeps its storage and the compiler writes the walking form itself.

## interface
- `doubled (x$)` pushes twice each item; `doubled by walking (x$)` is the same with a loop, kept beside it to be measured against.
- `placed (x$)` pushes ten times each item and which item it is: `position x$` is the item's place, counted from 0, an `index`.
- `trebled (x$)` says a stream of its own, `int two$ = x$ * 2`, and pushes `two$ + x$`.
- `class of (c)` classes a character, and `classes (c$)` pushes the class of each: a one-item function applied to the input's name is that function of the present item.
- `one at a time`, `as a block`, `said`, `walked`, `from a literal`, `from a range`, `through two`, `wired twice`, `over text`, `typed`, `also read`, `many` and `a line said` are the cases. Each says what its stream holds as one array compared with a list written out, `b << frame d$ [==] [2, 4, 6]`, where until fm3 log 183 it looked at a count and an item or two with `peek`; a count taken while the stream is still arriving stays beside it, `a << count d$` in `one at a time`. `many`, `every tenth to` and `tens to` still `peek` at the last of many items by a place worked out, and have a case beside them, `many whole`, `every tenth whole`, `tens whole`, that compares the whole with an array worked out, `[1 through k] * 2`, which costs three times as much, both arrays being made. `pushed above two whole` is beside `pushed above two`, which says what the stream holds for both of its arguments where one list cannot.
- `decades (x$)` pushes, for each run of items in one ten, the run's first item times a hundred and its length; `tens to (k)` is its case over a range (parity hop seventeen, fm3 log 133).

## rules
- Every line holds for every item: `d$ << x$ * 2` pushes one item for each that arrives.
- The input's bare name is the present item, and `position x$` is which item it is.
- A line that says a stream, `int two$ = x$ * 2`, is the processor's own: one value an item.
- `x$`, `y$`, `c$`, `once$`, `p$`, `r$`, `m$` and `text$` have no storage: nothing reads them but the processors wired to them, so a push into one is the processor's call. `once$` is the output of one processor and the input of the next, and `through two` is one call inside another.
- `lit$` and `ran$` have their items on their declarations, `s$` is read by `count s$` in `also read`, `old$` is walked by a task, and `in$` is the input device: each keeps its storage, and its processor is run over what has arrived, by the scheduler, as a task is.
- A processor wired twice has two of everything: `placed` on `p$` and on `r$` each count their own items, so `wired twice` gives the third item of `p$` place 2 and the first of `r$` place 0.
- `text$` is a stream of `char` with no storage because every push into it is of a string literal; `text$ << "ab 1"` is a loop that says four with the line of `classes` in it.
- Two lines that turn on one condition, `int n$ = if (new$) then (1) else (n$[-1] + 1)` and `int first$ = if (new$) then (x$) else (first$[-1])` in `decades`, and a push that goes out `if (new$ and n$[-1] > 0)`, are one branch on `new$` in the function the compiler writes: the cost of a decision does not depend on how many lines it was said in.

## testing
>one at a time() → 1, 1
>as a block() → 1
>said() → 1
>walked() → 1
>from a literal() → 1
>from a range() → 1
>through two() → 1
>wired twice() → 1, 1
>over text() → 1
>typed() with in "a1" → 1
>also read() → 2, 1
>many (60) → 60120
>many (3) → 3006
>a line said() → 1
>a running sum() → 1, 1
>summed from a literal() → 1
>a difference() → 1
>two back() → 1
>lines shuffled() → 1
>kept through a block() → 1
>two conditions() → 1
>joined (5) → 1
>joined (4) → 0
>joined (100) → 1
>joined (11) → 0
>filtered() → 1
>changed() → 1
>every tenth to (100) → 10100
>every tenth to (60) → 6060
>pushed above two (5) → 25
>pushed above two (1) → 19
>no stray zero() → 2, 1
>closed() → 0, 1
>a chain ends() → 0, 1
>how many came() → 1
>a stored input ends() → 0, 1
>pushed after its end() → check
>an output ends() → 0, 1
>tens to (100) → 1009010
>tens to (25) → 201010
>many whole (60) → 1
>many whole (3) → 1
>every tenth whole (100) → 1
>every tenth whole (60) → 1
>pushed above two whole (5) → 1
>pushed above two whole (1) → 0
>tens whole() → 1

## hostile
A processor's body that pushes into anything but its own output, declares a name without a `$`, puts an `if` round a line or a `while` on a push is refused, with what to write. `int d$ = doubled(i$)` inside a function is refused: a stream processor is wired at feature scope, and running one inside a function is not built. `y$` has no storage, so nothing limits what is pushed into it; `q$`, which `doubled` fills from it, is a queue read only after the statement, so `many (80)` fails the queue's check at the sixty-fifth item as a push of eighty into any stored stream does.
