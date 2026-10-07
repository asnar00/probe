# variables
*declarations inside a function, and feature-scope variables with their scope and merge words as fields of the context*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 5 of milestone 0: section 5 of zero.md — declarations inside functions, single assignment, and feature-scope variables with the scope words parsed and recorded (`static`, `device`, `group`, `merge`) and lowered to fields of a context struct with generated accessors. In this milestone all scopes behave as one place.

## overview
A variable declared inside a function is a value: type, name, and a value or its type's zero. A name declared at feature scope without a `$` is a value too: `static int port = 8822` keeps the value it was declared with, and nothing may assign it. **What changes is a stream** (fm3 question 70): `int size$ << 40` is a stream whose first item is 40, `size$ << s` pushes its next, once each time the line runs, and its name, `size$`, is its latest item wherever one value is wanted (question 79). Each of the streams here is read only for its latest item, so the compiler keeps one value of it, a field of the context (`suite/zero/cells`), and each function lowers to what it did when these were assigned variables. The words in front say whose the state is and how writes combine. Before every case the runner puts each back to its first value.

## interface
- `the size` reads a feature's stream by its name; `set size (s)` pushes into it; `grown` does both through a call.
- `bump` pushes one more into `count$`, which is declared `merge sum`; `bumped twice` and `bumped (k) times` show it kept between calls, the second from inside a loop; `fresh count` shows it reset between cases. `seen round a bump` reads `count$`, calls `bump`, and reads it again in the same function: the second read is the new value, 12 and not 11. State is read where it is named, every time; only a field nothing in the store writes is fetched once a function (fm3 log 110).
- `port number` reads a `static` variable, which nothing writes; `opened once` pushes into a `device` stream, `quota left after (used)` into a `group` one; `open_tool$` is section 5's device string, empty until `tool opened` pushes a name, and its length is asked of a local that takes its latest, `string tool = open_tool$`.
- `origin sum` and `moved origin` read and push a struct, its fields read through the name, `origin$.x`; `is auto` and `switched on` an enumeration; `named` and `renamed` a string, written by its name, `out$ << name$`; `flagged` a bool.
- `shadowed by (size)` and `shadowed locally` show a parameter and a local hiding the feature's stream of the same name.
- `locals` declares the five forms of section 5 inside a function.
- `both` takes a call's two results into two locals and pushes each; `assigned both` reads the two streams.

## rules
- Nothing at feature scope is assigned. A name declared with a value keeps it; what changes is declared with `$` and pushed into, anywhere, a loop body included.
- A stream's name is its latest item wherever one value is wanted. A stream of strings, or of bools, is one only a cell can hold, and its bare name is its latest item wherever it stands (fm3 question 83).
- A parameter or a local hides the feature's name inside its function, and neither is assigned.
- A name is declared at feature scope by one feature only; `enabled` is every feature's already and may not be declared.
- The scope is `user` unless `static`, `device` or `group` is written; the merge is `last` unless `merge` names one. Every scope is one place in this milestone, and no merge is applied yet: the words are recorded with the field.
- The runner resets every feature's state before each case.

## testing
>the size() → 40
>grown() → 50
>bumped twice() → 2
>bumped (5) times → 5
>seen round a bump() → 12
>fresh count() → 0
>port number() → 8822
>quota left after (30) → 70
>opened once() → 1
>no tool open() → 0
>tool opened() → 4
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
`size = 50` where `int size = 40` is declared: "'size' is a variable, and a variable keeps the value it was declared with (fm3 question 70): what changes is a stream. Declare it `int size$ << 40` and push its next value, `size$ << 50`; its name, `size$`, is then its latest item wherever one value is wanted". `name$ = "one"`: "'name$' is a stream: it is pushed into, `name$ << "one"`, not assigned". An assignment to a parameter: "'s' is a parameter: it is what the function was handed, and is not assigned". `count name$`, a word that wants the stream of strings whole: "a stream of string: a stream holds numbers, enumerations or structs of those". A second `int size$` in another feature is refused: "'size' is already a variable of feature variables". `int enabled = 1` at feature scope: "every feature has 'enabled' already". `static device int x`: "a variable has one scope word". A `#` anywhere in the code stops the build.
