# tasks
*tasks: `on (T t$) << name (U u$)`, wiring with `T t$ = name(u$)`, the scheduler, the position carried between runs, and `at (n hz)` on the virtual clock*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 8 of milestone 0: section 10 of zero.md — a task declared with `<<`, wiring, a cooperative scheduler that runs a task when its input has more and stops it when the input ends, the task's position carried between runs, and a virtual clock so `at (1 hz)` runs as fast as it can.

## overview
A task is a function that produces a stream over time: it is declared with `<<` in place of `=`, its result is the stream it pushes into, and its `$` parameters are streams it reads. `int i$ = sawtooth(5)` inside a function runs the task now; at feature scope the same line is wiring: a node the scheduler runs whenever its input has unread items, carrying the task's own position in the input between runs, until the input ends. `at (1 hz)` on the call makes the task sleep one period after each item, on a clock the suite moves as fast as it can.

## interface
- `count down from (n)`, `count up to (n)` and `sawtooth (n)` are section 10's tasks; `sawtooth` composes the other two into its own stream.
- `doubled (x$)` is each item doubled and `closer (x$)` is 99 once the input has ended, each a stream processor of one line that holds for every item, `d$ << x$ * 2` and `e$ << 99 if (empty x$)` (fm3 question 75; walking tasks with a `loop`, `peek` and `advance` until fm3 log 189).
- `d$`, `e$` and `q$` are wired at feature scope to `x$` and `y$`, empty streams the cases push into; `w$` chains two tasks; `z$` is wired with a feature variable as its argument.
- `sawtoothed`, `counted down`, `counted up` run tasks now, into a local stream.
- `wired at feature scope`, `wired from a variable`, `fed twice`, `carried between runs`, `closed`, `closed once` read the nodes' streams.
- `rated`, `sampled at a rate`, `composed at a rate` wire a task at `1 hz` and read the clock through `time of x$` and `x$ at (t)`; `time of x$` is a time, the latest item's index over the rate, `2 s` for item 2 of three at `1 hz` and `3 s` for the last of the sawtooth's four, and `position x$` beside it is the index alone and asks no time.
- `fed a literal` hands `doubled` a list written out, `int d$ << doubled ([1, 2, 3])`: inside a function a processor is handed an array and gives one, its line for each item, each once (fm3 question 113). `run now moves the reader` and `run now inside a loop` hand it `frame i$`, the array of what has arrived in a stream of the function's own, the frame taking what it reads. `a task run now inside a loop` passes that stream to `runs`, a task that walks, which moves it.
- `runs (x$)` pushes how many items it found unread each time it is run, and takes them: the store's one task that walks its input, since no line that holds for each item can say "what arrived together". What it found unread is `count x$ - position x$`, how many the stream has had less where this reader stands (fm3 question 137), `count x$` alone being how many it has had (question 94). `r$` is wired to `v$`, so `r$` holds one item a run of the node. `a batch`, `nothing pushed (k)` and `ended twice` read it.
- `right$ << (left$ << 0) forever` is an edge (section 9, log 72; `forever` makes it stand, fm3 question 79): `left$` wired into `right$`, each item moved as it arrives and a `0` pushed after each; `edged` pushes into `left$`, counts `right$` and reads what arrived in it as an array, `int r[] = frame right$`.

## rules
- `product.md` beside this folder bounds `count down from` and `count up to` at 5 for `probe cost`, the largest their wirings ask: the emitted IR carries `; product setting: bound count down from: 5` and `loop() bound 5` on each chain, and the code says no number (log 41).
- A task has one result, a `$`; its `$` parameters are the streams it reads and moves, a bare `T x$` at feature scope being an empty one. A task named as a value is refused: it is wired into a stream.
- In a function, `T x$ = task(...)`, `T x$ << task(...)` and `x$ << task(...)` run the task now; each stream argument is a stream variable and takes the reader the task returns, and a `loop` around the call carries it: `a task run now inside a loop() → 21`, two runs of `runs` that found five items and then one.
- A stream processor with no loop in it is wired at feature scope, and `x$` and `y$`, which only processors read, have no storage: a push into one is the call of each processor's function of one item, and `end x$` their last tick. Inside a function it is handed an array, `doubled ([1, 2, 3])`, `doubled (frame i$)`, and its lines are lowered in line at a loop over the items, pushing straight into the function's own stream; what it keeps starts at the zero of its type, and the array being all there, its last tick follows its last item. Handed a stream there, `int d$ = doubled (i$)`, it is refused, naming `doubled (frame i$)`.
- At feature scope the same forms wire a node. A node runs when an input has unread items, or has ended and the node has not run since; a node with no input runs once, when the store is reset; after a run a node is finished when all its inputs have ended.
- The scheduler runs at the end of the store's reset and after every push or `end`, from a plain function, into a stream some node reads; a task never starts it.
- A task wired `at (n hz)` sleeps one period after each push into its own output; the clock starts at zero for every case and nothing else moves it. The output ring is irregular, its ticks a period apart. A task composed into another's output runs at the outer rate.
- One push statement is one run of a node, after all its items: `v$ << 1 << 2 << 3` gives `a batch() → 13`, one run that found three. A statement that pushes nothing runs nothing: `v$ << [k to 1]` with `k` at 1 gives `nothing pushed (1) → 0`. The first `end` of a stream runs its nodes once more and a second runs nothing: `ended twice() → 20`, two runs, the push's and the first end's, the last finding nothing unread.
- Those three are what a program could see of how a node is run, and they are pinned because the lowering changed under them (parity hop 11, fm3 log 103): the node on `v$` is woken by its pushers (those on `x$` and `y$` were too, until their tasks were processors). Each of those streams is pushed into and ended only by plain functions that nothing the scheduler runs can reach, so a push statement calls the node's task there, in line, under the feature's gate, without asking whether the node is due; where the statement might push nothing the call stands under whether the stream received anything, and at an `end` under whether the stream had already ended. The three nodes with no input still run at the start.
- `while` after a task call is refused; so is a task call before a pushed item in a feature-scope chain.
- A `<<` at feature scope with a stream on its right and `forever` after it is an edge: it moves every item into the stream on the left by the dispatch a push uses, an item of the element type as itself, and pushes the rest of the chain after each item; it is gated by its feature. `left$` is only pushed into and wired, no word reads it, so it has no storage (question 50, fm3 log 92): `left$ << 1 << 2` calls the edge for each item, which pushes the item and then `0` into `right$`, a stream `edged` reads and so a stored one. Where a word reads an edge's source it is stored and the edge is a node like a wiring's, run when the source has more than it has seen and carrying its reader between runs; `suite/zero/edges` shows both. The first item of such a line must be a stream, and it takes no `while`.

## testing
>sawtoothed() → 1051
>counted down() → 31
>counted up() → 14
>wired at feature scope() → 631
>wired from a variable() → 4
>fed twice() → 2, 6
>carried between runs() → 246
>closed() → 0, 991
>closed once() → 11
>rated() → 2, 2 s
>sampled at a rate() → 2
>composed at a rate() → 3, 3 s
>fed a literal() → 36
>run now moves the reader() → 3
>run now inside a loop() → 82
>a task run now inside a loop() → 21
>edged() → 4, 1020
>a batch() → 13
>nothing pushed (1) → 0
>nothing pushed (3) → 1
>ended twice() → 20

## hostile
`int n = count up to (3)` is refused: "'count up to' is a task: it is wired into a stream, `int x$ = count up to (3)`". `x$ << count up to (3) while (x$ < 9)` is refused: "a task call is not repeated with `while`: the task's own chain says when it stops". `int c$ = count up to (3) at (2 ms)` is refused: "a rate is `at (n hz)` or `at (n khz)`, n positive". `on (int a$, int b$) << two()` is refused: "a task produces one stream: `on (T x$) << name (...)`". `on (int n) << f()` is refused: "a task's result is the stream it produces: `on (int n$) << ...`". `int d$ << doubled(x$) << 5` at feature scope is refused: "a value pushed after a task call at feature scope: a chain's items come before its tasks". `right$ << left$ while (_ > 0)` at feature scope is refused: "a line at feature scope that stands until its `while` fails is not built: wiring moves every item its stream receives, `x$ << y$ forever`". `int x$ <<` with nothing after it is refused: "a bare `int x$` declares an empty stream: drop the `<<`". A node whose task never takes what its input has would run once per arrival and no more; a task whose own loop never stops is what would hang, and the runner's timeout is what stops it.
