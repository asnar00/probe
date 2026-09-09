# tasks
*tasks: `on (T t$) << name (U u$)`, wiring with `T t$ = name(u$)`, the scheduler, the position carried between runs, and `at (n hz)` on the virtual clock*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 8 of milestone 0: section 10 of zero.md — a task declared with `<<`, wiring, a cooperative scheduler that runs a task when its input has more and stops it when the input ends, the task's position carried between runs, and a virtual clock so `at (1 hz)` runs as fast as it can.

## overview
A task is a function that produces a stream over time: it is declared with `<<` in place of `=`, its result is the stream it pushes into, and its `$` parameters are streams it reads. `int i$ = sawtooth(5)` inside a function runs the task now; at feature scope the same line is wiring: a node the scheduler runs whenever its input has unread items, carrying the task's own position in the input between runs, until the input ends. `at (1 hz)` on the call makes the task sleep one period after each item, on a clock the suite moves as fast as it can.

## interface
- `count down from (n)`, `count up to (n)` and `sawtooth (n)` are section 10's tasks; `sawtooth` composes the other two into its own stream.
- `doubled (x$)` reads a stream and pushes each item doubled; `closer (x$)` reads everything and pushes 99 once the input has ended.
- `d$`, `e$` and `q$` are wired at feature scope to `x$` and `y$`, empty streams the cases push into; `w$` chains two tasks; `z$` is wired with a feature variable as its argument.
- `sawtoothed`, `counted down`, `counted up` run tasks now, into a local stream.
- `wired at feature scope`, `wired from a variable`, `fed twice`, `carried between runs`, `closed`, `closed once` read the nodes' streams.
- `rated`, `sampled at a rate`, `composed at a rate` wire a task at `1 hz` and read the clock through `position` and `x$ at (t)`.
- `fed a literal` passes a stream made from a list to a task; `run now moves the reader`, `run now inside a loop` pass a local stream to a task, which moves it.
- `right$ << left$ << 0` is an edge (section 9, log 72): `left$` wired into `right$`, each item moved as it arrives and a `0` pushed after each; `edged` pushes into `left$` and reads `right$`.

## rules
- `product.md` beside this folder bounds `count down from` and `count up to` at 5 for `probe cost`, the largest their wirings ask: the emitted IR carries `; product setting: bound count down from: 5` and `loop() bound 5` on each chain, and the code says no number (log 41).
- A task has one result, a `$`; its `$` parameters are the streams it reads and moves, a bare `T x$` at feature scope being an empty one. A task named as a value is refused: it is wired into a stream.
- In a function, `T x$ = task(...)`, `T x$ << task(...)` and `x$ << task(...)` run the task now; each stream argument is a stream variable and takes the reader the task returns, and a `loop` around the call carries it.
- At feature scope the same forms wire a node. A node runs when an input has unread items, or has ended and the node has not run since; a node with no input runs once, when the store is reset; after a run a node is finished when all its inputs have ended.
- The scheduler runs at the end of the store's reset and after every push or `end`, from a plain function, into a stream some node reads; a task never starts it.
- A task wired `at (n hz)` sleeps one period after each push into its own output; the clock starts at zero for every case and nothing else moves it. The output ring is irregular, its ticks a period apart. A task composed into another's output runs at the outer rate.
- `while` after a task call is refused; so is a task call before a pushed item in a feature-scope chain.
- A stream on the right of `<<` at feature scope is an edge, a node like a wiring's: it runs when the stream on the right has more than it has seen, moves every unread item into the stream on the left by the dispatch a push uses, an item of the element type as itself, and pushes the rest of the chain after each item; it is gated by its feature and carries its reader between runs. The first item of such a line must be a stream, and it takes no `while`.

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
>rated() → 22
>sampled at a rate() → 2
>composed at a rate() → 33
>fed a literal() → 36
>run now moves the reader() → 3
>run now inside a loop() → 82
>edged() → 4, 1020

## hostile
`int n = count up to (3)` is refused: "'count up to' is a task: it is wired into a stream, `int x$ = count up to (3)`". `x$ << count up to (3) while (x$ < 9)` is refused: "a task call is not repeated with `while`: the task's own chain says when it stops". `int c$ = count up to (3) at (2 ms)` is refused: "a rate is `at (n hz)` or `at (n khz)`, n positive". `on (int a$, int b$) << two()` is refused: "a task produces one stream: `on (T x$) << name (...)`". `on (int n) << f()` is refused: "a task's result is the stream it produces: `on (int n$) << ...`". `int d$ << doubled(x$) << 5` at feature scope is refused: "a value pushed after a task call at feature scope: a chain's items come before its tasks". `right$ << left$ while (_ > 0)` at feature scope is refused: "an edge has no `while`: it moves every item its stream receives". `int x$ <<` with nothing after it is refused: "a bare `int x$` declares an empty stream: drop the `<<`". A node whose task never takes what its input has would run once per arrival and no more; a task whose own loop never stops is what would hang, and the runner's timeout is what stops it.
