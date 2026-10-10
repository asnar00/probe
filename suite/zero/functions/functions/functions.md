# functions
*functions and expressions: open-syntax names, named and several results, operators, a value on a condition, and the tower of numbers*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 2 of milestone 0: section 6 of zero.md — `on (results) << name (params)`, named results, several results, multiword names with bracketed parameter groups mangled per section 3, operators as functions, a value on a condition (`if then else` in that plan, `a if (c) else b` since hop forty-one), calls, arithmetic over the abstract tower.

## overview
A function is declared with `on`, its results first, then its name with the parameters in brackets anywhere among the words. `smaller of (a) and (b)` is one function; its IR name is `smaller_of_and`. A function is declared with `<<` and gives its result by pushing it, `d << x * 2` (fm3 question 77; `suite/zero/pushed` is the store for the form). A result is pushed once, at the top level of the body, with its condition on the push where it has one, `r << a if (a > 0) else b` (fm3 question 88). A value that must be tested before it is final is a temporary, said with `=`.

## interface
- `smaller of (a) and (b)` gives the smaller of two numbers, written over `number` so it runs at every width.
- `divide (a) by (b)` gives the quotient and the remainder: two results.
- `sum of (a) and (b) and (c)` adds three numbers.
- `mean of (a) and (b)` takes both results of `divide` at once.
- `double (x)` and `negated (x)` are arithmetic and the library's `neg`.
- `(a) is less than (b)` is a comparison giving a `bool`.
- `clamp (x) between (lo) and (hi)` says three cases on one push, `lo if (x < lo) else hi if (x > hi) else x`.
- `narrow (x)` calls `smaller of` with `int32` arguments, so the template is instantiated at 32 bits.
- `first positive of (a) and (b)` pushes one value or the other, `r << a if (a > 0) else b`: the condition is on the push (fm3 question 88).
- `ordered (a) and (b)` has two results: after `lo` is pushed the function goes on, and reads `lo`, until `hi` is pushed.
- `power of two above (n)` is the last item of a stream said by a rule, `int q$ << 1 << (q$ * 2) while (q$ <= n)` and `p << q$`: the stream is one value carried round a loop (fm3 log 186), and until hop thirty-one the function was that loop written out, `p << loop (int q = 1) yields q`.
- `describe (x)` is one name with four methods, over `int`, `float`, `bool` and `string`; a call picks by its argument; `describe a word` and `describe a decimal` call the string and float ones, since a case line's arguments are integers.
- `kind of (x)` has methods over `number`, `int` and `int32`, each giving an `int32` (a result declared `int` would be bound by the argument, as `smaller of` shows); the most specific that takes the argument wins: `kind at thirty two bits`, `kind at sixty four bits` and `kind of a float` call it with an `int32`, an `int64` and a `float32`, and `kind of a decimal` with `2.5`, a `float` first.
- `area of (w) by (h)` and `area of (side)` are two methods with different bracket groups: different names, not a dispatch.
- `width of (x)` has methods over `int32` and `int64` and no `int` one, so a bare `3` between them takes the product's `int` width (question 22): `int width` reads that width from the arithmetic itself — 2^31 - 1 plus one is negative at 32 bits and not at 64 — and `literal takes the int width` compares the two, which holds on wasm at 32 and on the other paths at 64. `width at thirty two bits` and `width at sixty four bits` say the width with an explicit conversion.

## rules
- Two operands of an operator have one type, or one of them is a literal.
- A result is pushed once, at the top level of the function's body: not under an `if` statement and not in a loop, each of which is refused showing what to write. A condition goes on the push, and a push with `if` has its `else`, so every path gives every result; a result nothing pushes is refused. Pushing a result does not end the function: the lines after it run, and with several results a later one may read an earlier (fm3 question 88, ruled 8 October 2026; until hop twenty-six the push of the last result ended the function and a result nothing pushed was its type's zero).
- To test a result, say a temporary, test it, then push the result (`magnitude of` in the `control` store).
- A name is a set of methods. A call picks the method by the types of all its arguments: among the methods that take them, the one whose every parameter type fits the others' — `int32` before `int` before `number` — and a call with no such method is ambiguous and refused. A literal is its own type first, `int` or `float`, and fits any number type only when no method takes that. A method that takes an argument as it is beats one the call would map over a sequence. A method with the same parameter types is a redefinition (the `more` feature); with other types, a new method.
- A `number` parameter takes any number; the result comes back as the argument's type.
- A program's names and the IR's do not meet (fm3 log 229). `fill` is a function of the IR's library, `idle` an instruction a platform file gives the machine, and `div` the opcode the language's own `/` is declared under; a program may have a function of each name, called from zero and from a case. In the IR each is named apart, `__f_fill`, as is any function whose name the library or a platform file has, read from those files when the store is compiled. Before hop thirty-eight a case could not call `fill`, `div` was refused as a clash, and `idle` ran the machine's instruction and died.
- A bare integer literal that no method takes as an `int` is the product's `int` width next: the path's policy (`int64` on native, riscv, arm-qemu and air, `int32` on wasm) unless the store's `product.md` says `int: 32` or `int: 64`, which pins the store on every path. Only when no method takes that either does the literal fit any number type. The emitted IR says where the width decided a call: `; a bare literal took the product's int width, int64, choosing among width of (int32), width of (int64)`.

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
>literal takes the int width() → 1
>width at thirty two bits() → 32
>width at sixty four bits() → 64
>literal takes the float width() → 1
>float width at thirty two bits() → 32
>float width at sixty four bits() → 64
>fill (3) → 6
>div (7, 2) → 5
>idle() → 7
>named as the machine names() → 657

## hostile
A call whose words match a function but whose bracket groups do not is refused: `f (1) g` against `f (a) g (b)` says "takes 2 argument(s), given 1"; a call whose words differ names no function. Two functions whose words mangle alike but whose brackets differ are refused as a clash. Code after the result is pushed is unreachable and refused: `n << 42` followed by `print "done"` says "this never runs: the function ended when its result was pushed on line 2", naming the line of the statement that never runs; a statement after an `if` whose two arms both push the result is refused the same way, as "the statement before it leaves the block". A result read in its own function after it is pushed cannot happen, since nothing runs after; a result that is not the last of several is read freely. An ambiguous call is refused naming the contenders: with `pick (number a) and (int b)` and `pick (int a) and (number b)`, `pick (x) and (y)` on two `int32`s says "ambiguous: pick and (number, int) and pick and (int, number) all take these arguments and none is the most specific; convert an argument, or declare a method for these types"; and `narrow (3)` with only `narrow (int8 x)` and `narrow (int16 x)` declared is ambiguous the same way, neither being a width the policy may take, where `width of (3)` between `int32` and `int64` is not: that call is emitted as `width_of(3: int)` on the two methods, which the IR holds as one set, and the policy chooses the method when it resolves `int` — 64 bits on the register machines, 32 on wasm — so the emitted IR is one text on every path (question 27, log 52); `float width of (2.5)` between `float32` and `float64` is emitted as `float_width_of(2.5: float)` the same way. A `product.md` saying `int: 16` is refused: "the product's int width is 32 or 64, not '16'"; `int: 64` or `float: 32` pins the policy on every path. Two such methods giving different results, or a method of the name over an abstract type (a template in the IR, which cannot join the set), leave the call ambiguous, and the message says to convert the literal. A call no method takes is refused naming them all: `describe (v)` on a `Vec` says "no 'describe' takes these arguments: the methods are describe (int), describe (float), describe (bool), describe (string), describe (int$)". A second method with the same parameter types in the same feature is "defined twice".
