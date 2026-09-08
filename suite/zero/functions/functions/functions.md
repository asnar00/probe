# functions
*functions and expressions: open-syntax names, named and several results, operators, `if then else`, and the tower of numbers*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 2 of milestone 0: section 6 of zero.md — `on (results) = name (params)`, named results, several results, multiword names with bracketed parameter groups mangled per section 3, operators as functions, `if then else` as an expression, calls, arithmetic over the abstract tower.

## overview
A function is declared with `on`, its results first, then its name with the parameters in brackets anywhere among the words. `smaller of (a) and (b)` is one function; its IR name is `smaller_of_and`. A result is assigned once and the function gives it at its end.

## interface
- `smaller of (a) and (b)` gives the smaller of two numbers, written over `number` so it runs at every width.
- `divide (a) by (b)` gives the quotient and the remainder: two results.
- `sum of (a) and (b) and (c)` adds three numbers.
- `mean of (a) and (b)` takes both results of `divide` at once.
- `double (x)` and `negated (x)` are arithmetic and the library's `neg`.
- `(a) is less than (b)` is a comparison giving a `bool`.
- `clamp (x) between (lo) and (hi)` nests two `if then else` expressions.
- `narrow (x)` calls `smaller of` with `int32` arguments, so the template is instantiated at 32 bits.

## rules
- Two operands of an operator have one type, or one of them is a literal.
- Assigning a result does not end the function; the function ends at the end of its body.
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

## hostile
A call whose words match a function but whose bracket groups do not is refused: `f (1) g` against `f (a) g (b)` says "takes 2 argument(s), given 1"; a call whose words differ names no function. Two functions whose words mangle alike but whose brackets differ are refused as a clash.
