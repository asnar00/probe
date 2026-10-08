# pushed
*every function is declared with `<<` and gives its result by pushing it: one way to declare*

layer: runtime

> (suite) 2026-10-07T14:00:00
fm3 question 77 (a), log 151. Ash, 7 October 2026: "all functions are now defined using on (result) << blah(), right? So there's only one colour of function?" A `<<` sends once each time its line runs (question 79), and a function runs when it is called, so a function's push of its result is its answer. `=` is left for saying what a name is.

## overview
`on (int d) << double (int x)` with `d << x * 2` is a function. There is one way to declare: the form before, `on (int d) = double (int x)` with `d = x * 2`, is refused when the program is compiled, each line with the line to write in its place. The new form lowers to the lines the old one did: when both stood, this store had each shape written both ways and every pair gave the same IR (fm3 log 151), and the whole suite was respelled without one store's IR moving (log 152).

What tells a function from a task or a stream processor, now that all three say `<<`, is the mark on the result: a result with no `$` is a value, pushed once, and a result with a `$` is a stream, produced over time.

A condition goes on the push (fm3 question 88, log 169): `n << a if (a > b) else b` is `a` where the condition holds and `b` where it does not. After `else` another `if` may stand, and a long one is written as a table, a case a line, each line after the first beginning `else` and indented under the push: `class of` is the lexer's `kind of` written so. It is read from the left, and each value is worked out only where it is the one chosen: `guarded (0)` never calls `into a hundred (0)`, which fails a check. In the compiler's tree it is the `if` statement with a push in each arm that it is written in place of, and lowers to that statement's lines. A push into a stream takes it too, `say how many`, and in a stream processor it is one item for each arriving item, `capped`.

## interface
- `double (x)` pushes its result at the top of its body; `(a) is under (b)` pushes a `bool`, and `smaller of (a) and (b)` a value that is one thing or another.
- `sign of (x)` is a table of three cases, a line each, and says all three: -1, 1, and 0 for 0.
- `first positive of (a) and (b)` pushes one value or the other, its `else` on a line of its own under the push. `... on one line` says the same on one, `r << a if (a > 0) else b`.
- `power of two above (n)` leaves its loop with `break (q)`, which gives the loop's result where it leaves, and that is pushed once, at the top of the body: `p << loop (int q = 1, int found = 0) yields found`.
- `gcd of (a) and (b)` is given what a loop yields, `g << loop (...) ... yields x`.
- `divide (a) by (b)` pushes two results, the second reading the first; `ordered (a) and (b)` pushes them in the other order from the one its first line names them in; `both of (a) by (b)` takes two from one call, `q, r << divide (a) by (b)`.
- `sum of (x[])` takes an array and pushes one value, and is called `[sum of] ([1, 2, 3, 4])`; `read` pushes a stream's latest item, `n << seen$`.
- `halved (x)` says a local with `=`, `int half = x / 2`, and pushes its result.
- `larger of (a) and (b)` pushes one value or the other, `n << a if (a > b) else b`; `class of (c)` is a table of six cases, a line each; `guarded (x)` pushes 0 for 0 and otherwise calls `into a hundred (x)`, which checks that `x` is not 0.
- `say how many (x)` pushes one of three texts into `out$`; `capped (x$)` is a stream processor whose push has an `else`, and `levels` pushes three items through it.

## rules
- A function is declared `on (results) << name (parameters)`. A result with no `$` is one value, and the function gives it by pushing it, once: `d << x * 2`.
- Pushing the last result ends the function, wherever the push stands (fm3 question 88, provisional): inside an `if`, inside a loop. With several results, each is pushed once, in any order, and the function goes on until the last is.
- A result nothing pushed when the body ends is the zero of its type.
- `=` says what a name is: a local is declared with `=` and is not pushed into.
- One value is pushed once: `y << a << b`, and `(n) times`, `while`, `until` and `forever` on the push of a result, are refused; so is a second push of a result on a path that has pushed it.
- A function that gives an array is declared the same way, its result with an array's mark: `on (int r[]) << squares to (int k)` in the `sequences` store, giving it by `r[] << [1 through k] * [1 through k]`, once. The mark is what tells it from a task, whose result is a stream, `i$` (fm3 questions 87 and 90).

## testing
>double (21) → 42
>(3) is under (4) → 1
>smaller of (9) and (4) → 4
>sign of (-5) → -1
>sign of (0) → 0
>sign of (7) → 1
>first positive of (3) and (5) → 3
>first positive of (-3) and (5) → 5
>first positive of (3) and (5) on one line → 3
>first positive of (-3) and (5) on one line → 5
>power of two above (5) → 8
>gcd of (12) and (18) → 6
>divide (17) by (5) → 3, 2
>ordered (5) and (2) → 5, 2
>both of (17) by (5) → 3, 2
>summed() → 10
>read() → 5
>halved (9) → 4
>larger of (3) and (8) → 8
>larger of (8) and (3) → 8
>class of (32) → 0
>class of (200) → 3
>class of (104) → 1
>class of (61) → 3
>class of (52) → 2
>class of (40) → 3
>guarded (0) → 0
>guarded (4) → 25
>into a hundred (0) → check
>say how many (0) → "none"
>say how many (1) → "one"
>say how many (7) → "many"
>levels() → "3 9 9 "

## hostile
`on (int d) = double (int x)` is refused: "a function is declared with `<<` and gives its result by pushing it; `=` says what a name is (fm3 question 77). Write `on (int d) << double (int x)`". `d = x * 2` in its body is refused: "'d' is a result, and a result is given by pushing it; `=` says what a name is (fm3 question 77). Write `d << x * 2`". `half << 1` where `half` is a local is refused: "'half' is not pushed into: `=` says what a name is, where it is declared, `int half = ...`, and it keeps that value. What `<<` sends into is a stream, `half$`, or a result of the function". `d << x << 2` is refused: "'d' is one value, given once: this line pushes it twice. What takes more than one item is a stream, `d$`". With two results, a second `q << 2` before the other is pushed is refused: "'q' is pushed twice on this path: a function gives each of its results once".
