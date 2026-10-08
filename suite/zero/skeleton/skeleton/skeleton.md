# skeleton
*the smallest store: one feature, a function that returns a literal and one that prints*

layer: runtime
published: 2026-10-07

> (suite) 2026-09-08T10:00:00
Plan item 1 of milestone 0: the `zero` subcommand, the store reader, the `.md` header, the lexer with indentation blocks, the `#` error, and a lowering that handles a feature with one function returning a literal.

## overview
A feature is a folder with prose and code. `answer` gives the number 42; `hi` prints one word.

## interface
- `answer` gives 42.
- `hi` prints "hi".

## rules
- A `.zero` file may not contain `#`; the build refuses it and names the line.
- A result is pushed once, at the top of the body, as `n << 42` is; a result nothing pushes is refused (fm3 question 88; until hop twenty-six it was its type's zero).
- This feature is published, `published: 2026-10-07`, so its cases are what it promises: where the store is a repository the build lets `skeleton.zero` be refactored, runs the two cases below first, and refuses the store if either fails; a change to the cases themselves is refused where the store is read (structure.md's lifecycle; fm3 question 89, log 176). It was first published on 2026-09-09 and is published again as of 2026-10-07, the day every function came to be declared with `<<` and its two lines were respelled, their meaning and their IR unchanged.

## testing
>answer() → 42
>hi() → "hi"

## hostile
A `#` anywhere in `skeleton.zero` stops the build with the file and line named.
