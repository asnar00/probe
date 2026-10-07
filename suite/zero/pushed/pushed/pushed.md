# pushed
*every function is declared with `<<` and gives its result by pushing it: one way to declare*

layer: runtime

> (suite) 2026-10-07T14:00:00
fm3 question 77 (a), log 151. Ash, 7 October 2026: "all functions are now defined using on (result) << blah(), right? So there's only one colour of function?" A `<<` sends once each time its line runs (question 79), and a function runs when it is called, so a function's push of its result is its answer. `=` is left for saying what a name is.

## overview
`on (int d) << double (int x)` with `d << x * 2` is a function. There is one way to declare: the form before, `on (int d) = double (int x)` with `d = x * 2`, is refused when the program is compiled, each line with the line to write in its place. The new form lowers to the lines the old one did: when both stood, this store had each shape written both ways and every pair gave the same IR (fm3 log 151), and the whole suite was respelled without one store's IR moving (log 152).

What tells a function from a task or a stream processor, now that all three say `<<`, is the mark on the result: a result with no `$` is a value, pushed once, and a result with a `$` is a stream, produced over time.

## interface
- `double (x)` pushes its result at the top of its body; `(a) is under (b)` pushes a `bool`, and `smaller of (a) and (b)` a value that is one thing or another.
- `sign of (x)` pushes under an `if` and an `else if`, and nothing for 0: a result nothing pushed is the zero of its type.
- `first positive of (a) and (b)` pushes under an `if` with no `else`, and the function ends there; the push after the `if` is the other path. `... on one line` says the same with `if` on the push, `r << a if (a > 0)`.
- `power of two above (n)` pushes inside a loop that has no `break`: the push is the loop's only way out.
- `gcd of (a) and (b)` is given what a loop yields, `g << loop (...) ... yields x`.
- `divide (a) by (b)` pushes two results, the second reading the first; `ordered (a) and (b)` pushes them in the other order from the one its first line names them in; `both of (a) by (b)` takes two from one call, `q, r << divide (a) by (b)`.
- `sum of (x$)` takes a sequence and pushes one value; `read` pushes a stream's latest item, `n << seen$`.
- `halved (x)` says a local with `=`, `int half = x / 2`, and pushes its result.

## rules
- A function is declared `on (results) << name (parameters)`. A result with no `$` is one value, and the function gives it by pushing it, once: `d << x * 2`.
- Pushing the last result ends the function, wherever the push stands (fm3 question 88, provisional): inside an `if`, inside a loop. With several results, each is pushed once, in any order, and the function goes on until the last is.
- A result nothing pushed when the body ends is the zero of its type.
- `=` says what a name is: a local is declared with `=` and is not pushed into.
- One value is pushed once: `y << a << b`, and `(n) times`, `while`, `until` and `forever` on the push of a result, are refused; so is a second push of a result on a path that has pushed it.
- A function that gives a sequence whole, `on (int r$) = squares to (int k)` in the `sequences` store, keeps `=`: with `<<` it would be a task's first line, and it waits for an array to have its mark (fm3 question 87).

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

## hostile
`on (int d) = double (int x)` is refused: "a function is declared with `<<` and gives its result by pushing it; `=` says what a name is (fm3 question 77). Write `on (int d) << double (int x)`". `d = x * 2` in its body is refused: "'d' is a result, and a result is given by pushing it; `=` says what a name is (fm3 question 77). Write `d << x * 2`". `half << 1` where `half` is a local is refused: "'half' is not pushed into: `=` says what a name is, where it is declared, `int half = ...`, and it keeps that value. What `<<` sends into is a stream, `half$`, or a result of the function". `d << x << 2` is refused: "'d' is one value, given once: this line pushes it twice. What takes more than one item is a stream, `d$`". With two results, a second `q << 2` before the other is pushed is refused: "'q' is pushed twice on this path: a function gives each of its results once".
