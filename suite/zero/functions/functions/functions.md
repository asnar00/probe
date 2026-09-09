# functions
*functions and expressions: open-syntax names, named and several results, operators, `if then else`, and the tower of numbers*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 2 of milestone 0: section 6 of zero.md — `on (results) = name (params)`, named results, several results, multiword names with bracketed parameter groups mangled per section 3, operators as functions, `if then else` as an expression, calls, arithmetic over the abstract tower.

## overview
A function is declared with `on`, its results first, then its name with the parameters in brackets anywhere among the words. `smaller of (a) and (b)` is one function; its IR name is `smaller_of_and`. Assigning the result ends the function; with several results, assigning the last of them does. A value that must be tested or changed before it is final is a temporary.

## interface
- `smaller of (a) and (b)` gives the smaller of two numbers, written over `number` so it runs at every width.
- `divide (a) by (b)` gives the quotient and the remainder: two results.
- `sum of (a) and (b) and (c)` adds three numbers.
- `mean of (a) and (b)` takes both results of `divide` at once.
- `double (x)` and `negated (x)` are arithmetic and the library's `neg`.
- `(a) is less than (b)` is a comparison giving a `bool`.
- `clamp (x) between (lo) and (hi)` nests two `if then else` expressions.
- `narrow (x)` calls `smaller of` with `int32` arguments, so the template is instantiated at 32 bits.
- `first positive of (a) and (b)` assigns its result inside an `if` with no `else`: the function ends there, and the assignment after the `if` is the other path.
- `ordered (a) and (b)` has two results: after `lo` is assigned the function goes on, and reads `lo`, until `hi` is assigned.
- `power of two above (n)` assigns its result inside a `loop` that has no `break`: the assignment is the loop's only way out.

## rules
- Two operands of an operator have one type, or one of them is a literal.
- Assigning the result ends the function, wherever the assignment stands: inside an `if`, inside a loop. With several results, the function ends when the last of them is assigned; until then it goes on and may read the ones assigned. A result never assigned when the body ends is its type's zero.
- To test a result, assign a temporary, test it, then assign the result (`magnitude of` in the `control` store).
- A `number` parameter takes any number; the result comes back as the argument's type.

## testing
>smaller of (3) and (4) → 3
>smaller of (9) and (4) → 4
>divide (17) by (5) → 3, 2
>sum of (1) and (2) and (3) → 6
>mean of (3) and (5) → 4
>double (21) → 42
>negated (5) → -5
>(3) is less than (4) → 1
>(4) is less than (3) → 0
>clamp (15) between (0) and (10) → 10
>clamp (-3) between (0) and (10) → 0
>clamp (7) between (0) and (10) → 7
>narrow (300000) → 300000
>first positive of (3) and (5) → 3
>first positive of (-3) and (5) → 5
>ordered (5) and (2) → 2, 5
>ordered (2) and (5) → 2, 5
>power of two above (5) → 8
>power of two above (8) → 16

## hostile
A call whose words match a function but whose bracket groups do not is refused: `f (1) g` against `f (a) g (b)` says "takes 2 argument(s), given 1"; a call whose words differ names no function. Two functions whose words mangle alike but whose brackets differ are refused as a clash. Code after the result is assigned is unreachable and refused: `n = 42` followed by `print "done"` says "this never runs: the function ended when its result was assigned on line 2", naming the line of the statement that never runs; a statement after an `if` whose two arms both assign the result is refused the same way, as "the statement before it leaves the block". A result read in its own function after it is assigned cannot happen, since nothing runs after; a result that is not the last of several is read freely.
