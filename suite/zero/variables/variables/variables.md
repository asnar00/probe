# variables
*declarations inside a function, and feature-scope variables with their scope words as fields of the context*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 5 of milestone 0: section 5 of zero.md — declarations inside functions, single assignment, and feature-scope variables with the scope words parsed and recorded (`static`, `device`, `group`, `merge`) and lowered to fields of a context struct with generated accessors. In this milestone all scopes behave as one place. Hop fifty-seven (fm3 places.md, question 149, ruled 24 October 2026) respelled the words: `static` is `global`, `device` is no word, and there is no `merge`.

## overview
A variable declared inside a function is a value: type, name, and a value or its type's zero. A name declared at feature scope without a `$` is a value too: `global int port = 8822` keeps the value it was declared with, and nothing may assign it. **What changes is a stream** (fm3 question 70): `int size$ << 40` is a stream whose first item is 40, `size$ << s` pushes its next, once each time the line runs, and its name, `size$`, is its latest item wherever one value is wanted (question 79). Each of the streams here is read only for its latest item, so the compiler keeps one value of it, a field of the context (`suite/zero/cells`), and each function lowers to what it did when these were assigned variables. The word in front says whose the state is: with none a stream is one person's on one device; `user` is one person's on every device of theirs, `group` everyone's in the group, `global` everyone's (`suite/zero/counter` names two people and shows them share). How two writes combine is what the line that pushes says, and not a word on the declaration: `count$ << count$ + 1` is a sum (fm3 places.md, "the line is the merge"). Before every case the runner puts each back to its first value.

## interface
- `the size` reads a feature's stream by its name; `set size (s)` pushes into it; `grown` does both through a call.
- `bump` pushes one more into `count$`, `count$ << count$ + 1`, which is a sum because the line says so; `bumped twice` and `bumped (k) times` show it kept between calls, the second by a push with a count, `count$ << count$ + 1 (k) times` (a `for` over `bump()` until fm3 log 184: 7 a pass as counted where the `for` and its call were 12); `fresh count` shows it reset between cases. `seen round a bump` reads `count$`, calls `bump`, and reads it again in the same function: the second read is the new value, 12 and not 11. State is read where it is named, every time; only a field nothing in the store writes is fetched once a function (fm3 log 110).
- `port number` reads a `global` value, which nothing writes; `opened once` pushes into a stream with no word, the device's own, `quota left after (used)` into a `group` one; `open_tool$` is section 5's device string, no word on it either, empty until `tool opened` pushes a name, and its length is asked of a local that takes its latest, `string tool = open_tool$`.
- `origin sum` and `moved origin` read and push a struct, its fields read through the name, `origin$.x`; `is auto` and `switched on` an enumeration; `named` and `renamed` a string, written by its name, `out$ << name$`; `flagged` a bool.
- `shadowed by (size)` and `shadowed locally` show a parameter and a local hiding the feature's stream of the same name.
- `locals` declares the five forms of section 5 inside a function.
- `both` takes a call's two results into two locals and pushes each; `assigned both` reads the two streams.

## rules
- Nothing at feature scope is assigned. A name declared with a value keeps it; what changes is declared with `$` and pushed into, anywhere, a loop body included.
- A stream's name is its latest item wherever one value is wanted. A stream of strings, or of bools, is one only a cell can hold, and its bare name is its latest item wherever it stands (fm3 question 83).
- A parameter or a local hides the feature's name inside its function, and neither is assigned.
- A name is declared at feature scope by one feature only; `enabled` is every feature's already and may not be declared.
- With no scope word a stream is the device's own; `user`, `group` or `global` says it is a user's, the group's or everyone's, and it lives in that state, the device holding a copy while it is the current one (hop fifty-seven). A value may carry the word and is the same value everywhere. A variable has one scope word. How two writes combine is what the pushing line says.
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
`size = 50` where `int size = 40` is declared: "'size' is a variable, and a variable keeps the value it was declared with (fm3 question 70): what changes is a stream. Declare it `int size$ << 40` and push its next value, `size$ << 50`; its name, `size$`, is then its latest item wherever one value is wanted". `name$ = "one"`: "'name$' is a stream: it is pushed into, `name$ << "one"`, not assigned". An assignment to a parameter: "'s' is a parameter: it is what the function was handed, and is not assigned". `count name$`, a word that wants the stream of strings whole: "a stream of string: a stream holds numbers, enumerations or structs of those". A second `int size$` in another feature is refused: "'size' is already a variable of feature variables". `int enabled = 1` at feature scope: "every feature has 'enabled' already". `user group int x$ << 0`: "a variable has one scope word: user, group or global".

The words that left are refused, each showing the line to write (fm3 places.md, question 149; principles 7 and 8). `device int opened$ << 0`: "`device int opened$ << 0`: `device` says what no word says: a stream with no scope word is one person's on one device (fm3 places.md, question 149): write `int opened$ << 0`". `static int port = 8822`: "`static int port = 8822`: `static` is `global`, everyone's (fm3 places.md, question 149): write `global int port = 8822`". `int count$ << 0 merge sum`: "`int count$ << 0 merge sum`: how two writes combine is what the line that pushes says (fm3 places.md, "the line is the merge"): `count$ << count$ + 1` is a sum, `size$ << s` the last write stands, `best$ << s if (s > best$)` the better of the two; write `int count$ << 0`". A scope word on a local, `group int k = 1` in a function: "`group int k = 1`: a scope word is for a stream declared at feature scope, outside any function, and 'group' here is the function's own (fm3 places.md, question 149): write `int k = 1`"; on a parameter, `on (int n) << f (user int k)`, the same, ending "write `on (int n) << f (int k)`".

A case line may name people (`suite/zero/counter`), and two refusals hold it to that. One that names people and gives a value: `>the size() with ann in "k" → 40`, "a case that names people says what each one's device wrote, `→ ann "text"`, the parts joined by `;`: a value, a text with no name or `check` cannot say whose it is (fm3 places.md, "what a case says"; principle 7)". A name the line reads otherwise: `>named() → variables "zero"`, "'variables' cannot name a person in a case: it is a feature's name, and a person's name is a word the line reads no other way, not `in`, `on`, `off`, `wire`, `check`, a feature's name or a type's (fm3 places.md, "what a case says")"; `off`, `in`, `int` are refused the same way. A `#` anywhere in the code stops the build.
