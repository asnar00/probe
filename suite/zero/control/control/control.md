# control
*control flow: `if` and `else` as statements, a stream said by a rule from its own last item where a loop was, a function applied to a range where a `for` was, and the sequence forms that replace most loops*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 4 of milestone 0: section 7 of zero.md — `if`/`else`, `loop ... while ... continue`, `break`, `bound N`, `for` over a sequence; `probe cost` counting a bounded loop in the lowered IR. Rulings pass item 6 (question 4): a loop's carried variables are its own and a word at the end of the header names what leaves; a sum, a triangle and a walk over a range are written as sequence forms, and a loop stays only where nothing else says it. Second rulings pass item 3 (question 23): the word is `yields`, and `gives` is an ordinary name word.

## overview
An `if` statement runs one of two blocks; a variable it assigns has, after it, the value from whichever arm ran. A repetition that only computes is a stream said by a rule from its own last item: `int q$ << 1 << (q$ * 2) while (q$ <= n)` is 1 and then the last item doubled for as long as the test holds, and the answer is its last item, `p << q$`. Nothing is kept of such a stream but that item: the compiler carries it round the loop the push lowers to, which is the loop a person would write, `loop (int q = 1) yields q`, line for line (fm3 log 186). Until hop thirty-one each of these was that `loop`, with its carried variables, `while`, `continue`, `break` and `yields`; `loop` is still the language's, and `suite/zero/pushed` has two that give a function's result, `suite/zero/streams`' `walked` one that moves a stream, and the walking lexer of `suite/zero/lex` two. A `for` runs its body once per value of a range. A repetition that a sequence form can say, a sum over a range, a function mapped over one, is written that way, and the compiler makes the loop.

## interface
- `sign of (x)` is -1, 1 or 0: one push with its condition on it, written as a table, a case a line, and saying all three (fm3 question 88). Until hop twenty-six it was an `if` and an `else if` statement with a push under each and nothing pushed for 0.
- `magnitude of (x)` keeps a temporary that is one thing or another, `int m = if (x < 0) then (-x) else (x)`, and pushes the result last: pushing the result would end the function, so a value that is tested is a temporary until it is final.
- `describe (x)` prints one of two strings.
- `sum to (n)` is a range reduced, `[0 through n] + _`: no loop is written; `sum below (n)` is section 7's `[0 to n] + _`, the exclusive range, 0 to n - 1. **Nothing is made of the range** (fm3 log 187): it is one loop that counts and carries the sum, 81 as counted for `(10)` where the range made an array and then summed was 385, and `sum to (10000)`, which filled the arena, is a case. `doubled sum to (k)` is `[1 through k] * 2 + _`, the doubling worked out for each value in the same loop.
- `thirty three counted`, `sixty four counted` and `a hundred counted` are the same reduction over a range whose bounds are literals, `[1 through 33] + _`; `thirty three listed`, `sixty four listed` and `a hundred listed` over a list of that many literals; `listed item (i)` and `counted item (i) of (n)` read one item by its index from a list of a hundred and from `[0 through n]`. A list or a range is a new stream with its items present, and these say that it reads back right at any length, in the order written: a store whose streams are all queues gave 560 for `sum to (32)` and 51 for `listed item (0)` until parity hop fifteen (fm3 question 62).
- `gcd of (a) and (b)` is Euclid's rule said of a stream of a structure, `pair p$ << pair(a, b) << pair(p$.y, p$.x % p$.y) while (p$.y != 0)`, the answer `p$.x`. The two values move together as a loop's `continue (y, r)` moved them, and no `pair` is ever made: the compiler keeps the two fields apart and carries them round the loop. The test is made before the rule is worked out, the condition not reading `_` (fm3 question 111), so the remainder is never taken by a `p$.y` that is 0; worked out first, as every push was until hop thirty-one, it was a remainder by zero on the last pass, which wasm refuses.
- `power of two above (n)` is `int q$ << 1 << (q$ * 2) while (q$ <= n)`: 20 as counted for `(10)`, what the loop it replaced cost, where the same line was 308 while a stream in a function was a queue of sixty-four.
- `digits of (n)` and `collatz steps (start)` carry a count beside the value, as a structure: `tally t$ << tally(n, 1) << tally(t$.x / 10, t$.n + 1) while (t$.x >= 10)`, the answer `t$.n`. `collatz steps (27)` is 111 steps.
- `digits said of (n)` and `collatz steps said (start)` beside them say the same with `count`, `int x$ << n << (x$ / 10) while (x$ >= 10)` and `d << count x$`, which reads better: nothing takes from a stream said so, so how many are waiting is how many were pushed, a second value carried round the loop beside the latest (fm3 question 112, log 190), 34 as counted against 33 for `(12345)` and 108 against 105 for `(6)`, the count's own conversion and subtraction the difference; it was a queue of sixty-four until then, 283 and 613, and `collatz steps said (27)`, 111 steps, failed the queue's check at its sixty-fifth item.
- `triangle (n)` maps `row (i)`, itself a reduction, over `[1 to n + 1]` and reduces: two loops in the IR, none written, the call of `row` made for each value in the loop that sums.
- `two counters` says two such streams in one function, `int a$ << 0 << (a$ + 1) while (a$ < 3)`, and reads both.
- `ticks` writes a line three times by a push with a count, `out$ << ("tick" << "\n") (3) times`. `blast off` and `halves` apply a function of one item to a literal range, down and exclusive, `count off ([3 through 1])` and `odd one ([0 to 4])`, the function's push taking `if` and `else`. Applied so, the range is the loop (fm3 log 187) and the function's lines stand in it, with no call, where the function gives nothing, is said once in the store and is small (fm3 log 191): 328 and 231 as counted, what the `for` with an `if` in it that each was until then cost, to the unit. The store has no `for` now; `suite/zero/arrays`' `walked` is the suite's case of one. `steps from (a) to (b)` maps `step`, a function with no result, over a range whose bounds are decided at run time.
- `kept twice to (k)` pushes a function of one item applied to a range into a stream of its own kind of item, `kept$ << gives twice ([1 through k])`: the range's loop, each value worked out and pushed. `show twice to (k)` pushes the same into `out$`, where numbers given together are written with spaces between, `2 4 6`: there the array is made, as it was, because what is pushed is the array.

## rules
- A stream said by a rule: its first items, then the rule in brackets and `while`, `until` or `(n) times`. In the rule the stream's own name is its latest item. A `while` whose condition reads `_`, the item about to be pushed, works the item out and then tests it; one whose condition does not is tested first, and the rule is worked out only where it holds.
- A stream declared in a function and read only for its latest item keeps that item and nothing else (`suite/zero/cells` has every case of it). `count` of it keeps a queue.
- A `loop`'s carried variables are exactly those in its header; `continue` gives them in that order, and a bare `continue`, or a body that ends, continues with their current values. A loop's own variables are not assigned in its body (fm3 question 70), and a variable declared outside a loop is not assigned inside it. A `for`'s item is not assigned.
- `[1 through n]` counts down when n < 1, so a sum from 1 to n is `[0 through n] + _` and rows 1 to n are `[1 to n + 1]`.
- A statement after `break` or `continue` is refused: it would never run.
- `[a through b]` includes b and `[a to b]` stops before it; with a > b the range counts down.
- A `for` over a literal range shows its count to `probe cost` without a bound.
- A range that is used once is never made: reduced by `+`, a function applied to it as a statement, or pushed into a stream of its own kind of item, it is the loop that counts, and what maps over it, arithmetic with one value or a function of one item, is worked out for each value there. Given a name, `int x[] = [0 through n]`, zipped with another, or pushed where a method takes the array, it is an array.

## testing
>gives twice (4) → 8
>kept twice to (3) → 306
>show twice to (3) → "2 4 6"
>sign of (-5) → -1
>sign of (7) → 1
>sign of (0) → 0
>magnitude of (-3) → 3
>magnitude of (4) → 4
>describe (-1) → "negative"
>describe (2) → "not negative"
>sum to (10) → 55
>sum to (0) → 0
>sum to (31) → 496
>sum to (32) → 528
>sum to (63) → 2016
>sum to (99) → 4950
>sum to (100) → 5050
>sum to (10000) → 50005000
>doubled sum to (4) → 20
>doubled sum to (10000) → 100010000
>thirty three counted() → 561
>sixty four counted() → 2080
>a hundred counted() → 5050
>thirty three listed() → 561
>sixty four listed() → 2080
>a hundred listed() → 5050
>listed item (0) → 1
>listed item (32) → 33
>listed item (99) → 100
>counted item (0) of (99) → 0
>counted item (40) of (99) → 40
>counted item (99) of (99) → 99
>sum below (5) → 10
>sum below (0) → 0
>gcd of (48) and (18) → 6
>gcd of (7) and (5) → 1
>power of two above (10) → 16
>power of two above (16) → 32
>power of two above (0) → 1
>digits of (12345) → 5
>digits of (7) → 1
>digits said of (12345) → 5
>digits said of (7) → 1
>collatz steps (6) → 8
>collatz steps (1) → 0
>collatz steps (27) → 111
>collatz steps said (6) → 8
>collatz steps said (1) → 0
>collatz steps said (27) → 111
>triangle (3) → 10
>triangle (0) → 0
>two counters() → 35
>ticks() → "tick\ntick\ntick"
>blast off() → "more\nmore\none"
>halves() → "odd\nodd"
>steps from (1) to (3) → "step\nstep\nstep"
>steps from (3) to (1) → "step\nstep\nstep"
>steps from (2) to (2) → "step"

## hostile
Each of these is refused with the file and line named. An assignment to a loop's own variable in its body: "'q' is the loop's own: it is not assigned in the loop's body. Give its next value with `continue (...)`, and the loop's result where it leaves with `break (...)`". An assignment inside a loop to a variable declared before it: "'s' is declared outside the loop: carry it in the loop's header, `loop (int s = ...)`". A statement after `break`: "this never runs: the statement before it leaves the block". `break` outside a loop: "'break' outside a loop". `continue (1)` inside a `for`: "a `for` steps its item by itself: 'continue' takes no values here". `i = 0` on a `for`'s item: "'i' is the item of the `for`: it steps by itself and is not assigned". `continue (i + 1, 2)` in a loop carrying one variable: "the loop carries 1 variable(s), 'continue' gives 2". `acc` read after a loop that carried it: "no function named 'acc'". `yields total` naming no carried variable: "'total' is not a variable the loop carries: `yields` names one of its header's, i, acc". `int s = loop (...)` without `yields`: "`= loop` names what the loop yields: `... while (c) yields acc`". `loop (...) yields acc` on its own: "the loop yields 'acc' to nothing". `int s = loop (int i = 0) yields i` with no `while` and no `break`: "the loop never leaves, so it yields nothing". A variable declared in an arm of an `if`, read after it: "no function named 't'". `[1.5 through n]`: "a range's bounds are integers".

`probe cost` on the emitted IR (`probe zero suite/zero/control emit > control.ssa; probe cost control.ssa ticks blast_off`) reports:

    ticks: loop at b1 x3 (i from 1 by 1 to 3), body 121 ssa
    blast_off: loop at b1 x3 (i from 3 by -1 to 1), body 124 ssa

(a pass is one write of a string literal to `out$`, its bytes handed whole from `data` to the platform's write, `__out_block`, log 57, 69 and 87; the write's copy shows as a counted inner loop of its own, `copy_u8: loop at b8 x4 (j from at least 0 by 1 to 4)`, the literal's length carried into the library by the cost tool, log 76)

and `sum_to`'s range and `steps_from_to`'s, whose bounds are run-time values, as unbounded, counted once.
