# more
*extends functions: a redefinition extends one method of a name, and a new signature is a new method*

parent: functions
layer: runtime

> (suite) 2026-09-08T10:01:00
Rulings pass item 2 (question 18): with `existing`, a redefinition with the same signature extends that method and chains; a definition with a different signature is a new method of the same name.

## overview
`more` redefines `describe (int x)`, the one method of the four with that signature, to count in `noted` and then call `existing`; the other three methods are untouched, so `describe (2.5)` still prints "float" and counts nothing. It also declares a fifth method, `describe (int x$)`, over a sequence: `describe ([1, 2])` takes it rather than mapping the `int` method over the items.

## rules
- This feature's cases for `describe (int x)` are its definition of that method's test function (section 14, log 50): being the newest, they would replace functions' `describe (3) → "int"` wherever `more` is on. The `>existing` line says the older cases run too, as the redefinition calls `existing` and every older case still holds.

## interface
- `describe (int x)` counts the call in `noted` and calls the previous definition.
- `describe (int x$)` prints "ints".
- `times described` calls `describe` on two ints and a float and gives `noted`.
- `describe some` calls `describe` on a sequence.

## rules
- `existing describe (x)` inside the redefinition calls the previous definition of this method only.
- A redefinition keeps its method's signature; `on describe (int x)` with a `bool` result would be refused.

## testing
>existing
>times described() → 2
>describe some() → "ints"
>describe (4) → "int"

## hostile
`on (bool b) = describe (int x)` in this feature is refused: "'describe (int)' redefines feature functions's with different results: a redefinition keeps the signature".
