# variables
*declarations inside a function, and feature-scope variables with their scope and merge words as fields of the context*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 5 of milestone 0: section 5 of zero.md — declarations inside functions, single assignment, and feature-scope variables with the scope words parsed and recorded (`static`, `device`, `group`, `merge`) and lowered to fields of a context struct with generated accessors. In this milestone all scopes behave as one place.

## overview
A variable declared inside a function is a value: type, name, and a value or its type's zero. A variable declared at feature scope is state: it keeps its value from one call to the next, any function of the store may read or assign it, and the words in front of it say whose it is and how writes combine. Before every case the runner puts each one back to its declared initial value.

## interface
- `the size` reads a feature variable; `set size (s)` assigns it; `grown` does both through a call.
- `bump` adds one to `count`, which is declared `merge sum`; `bumped twice` and `bumped (k) times` show it kept between calls, the second from inside a loop; `fresh count` shows it reset between cases.
- `port number` reads a `static` variable, `opened once` a `device` one, `quota left after (used)` a `group` one.
- `origin sum` and `moved origin` read and assign a struct; `is auto` and `switched on` an enumeration; `named` and `renamed` a string; `flagged` a bool.
- `shadowed by (size)` and `shadowed locally` show a parameter and a local hiding the feature variable of the same name.
- `locals` declares the five forms of section 5 inside a function.
- `both` and `assigned both` take a call's two results into two feature variables at once.

## rules
- A feature variable is assigned anywhere, a loop body included: it is memory, not a value.
- A parameter or a local hides the feature variable of the same name inside its function.
- A name is declared at feature scope by one feature only; `enabled` is every feature's already and may not be declared.
- The scope is `user` unless `static`, `device` or `group` is written; the merge is `last` unless `merge` names one. Every scope is one place in this milestone.
- The runner resets every feature variable before each case.

## testing
>the size() → 40
>grown() → 50
>bumped twice() → 2
>bumped (5) times → 5
>fresh count() → 0
>port number() → 8822
>quota left after (30) → 70
>opened once() → 1
>origin sum() → 6
>moved origin() → 4
>is auto() → 1
>switched on() → 1
>named() → "zero"
>renamed() → "one"
>flagged() → 1
>shadowed by (7) → 7
>shadowed locally() → 5
>locals() → 5
>assigned both() → 16

## hostile
A second `int size` in another feature is refused: "'size' is already a variable of feature variables". `int enabled = 1` at feature scope: "every feature has 'enabled' already". `static device int x`: "a variable has one scope word". A `#` anywhere in the code stops the build.
