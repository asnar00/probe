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
- `describe (x)` is one name with four methods, over `int`, `float`, `bool` and `string`; a call picks by its argument; `describe a word` and `describe a decimal` call the string and float ones, since a case line's arguments are integers.
- `kind of (x)` has methods over `number`, `int` and `int32`, each giving an `int32` (a result declared `int` would be bound by the argument, as `smaller of` shows); the most specific that takes the argument wins: `kind at thirty two bits`, `kind at sixty four bits` and `kind of a float` call it with an `int32`, an `int64` and a `float32`, and `kind of a decimal` with `2.5`, a `float` first.
- `area of (w) by (h)` and `area of (side)` are two methods with different bracket groups: different names, not a dispatch.

## rules
- Two operands of an operator have one type, or one of them is a literal.
- Assigning the result ends the function, wherever the assignment stands: inside an `if`, inside a loop. With several results, the function ends when the last of them is assigned; until then it goes on and may read the ones assigned. A result never assigned when the body ends is its type's zero.
- To test a result, assign a temporary, test it, then assign the result (`magnitude of` in the `control` store).
- A name is a set of methods. A call picks the method by the types of all its arguments: among the methods that take them, the one whose every parameter type fits the others' — `int32` before `int` before `number` — and a call with no such method is ambiguous and refused. A literal is its own type first, `int` or `float`, and fits any number type only when no method takes that. A method that takes an argument as it is beats one the call would map over a sequence. A method with the same parameter types is a redefinition (the `more` feature); with other types, a new method.
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
>describe (3) → "int"
>describe a decimal() → "float"
>describe (true) → "bool"
>describe a word() → "string"
>kind of (7) → 2
>kind of a decimal() → 1
>kind at thirty two bits (7) → 3
>kind at sixty four bits (7) → 2
>kind of a float (7) → 1
>area of (3) by (4) → 12
>area of (5) → 25

## hostile
A call whose words match a function but whose bracket groups do not is refused: `f (1) g` against `f (a) g (b)` says "takes 2 argument(s), given 1"; a call whose words differ names no function. Two functions whose words mangle alike but whose brackets differ are refused as a clash. Code after the result is assigned is unreachable and refused: `n = 42` followed by `print "done"` says "this never runs: the function ended when its result was assigned on line 2", naming the line of the statement that never runs; a statement after an `if` whose two arms both assign the result is refused the same way, as "the statement before it leaves the block". A result read in its own function after it is assigned cannot happen, since nothing runs after; a result that is not the last of several is read freely. An ambiguous call is refused naming the contenders: with `pick (number a) and (int b)` and `pick (int a) and (number b)`, `pick (x) and (y)` on two `int32`s says "ambiguous: pick and (number, int) and pick and (int, number) all take these arguments and none is the most specific; convert an argument, or declare a method for these types". A call no method takes is refused naming them all: `describe (v)` on a `Vec` says "no 'describe' takes these arguments: the methods are describe (int), describe (float), describe (bool), describe (string), describe (int$)". A second method with the same parameter types in the same feature is "defined twice".
