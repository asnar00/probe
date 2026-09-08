# skeleton
*the smallest store: one feature, a function that returns a literal and one that prints*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan item 1 of milestone 0: the `zero` subcommand, the store reader, the `.md` header, the lexer with indentation blocks, the `#` error, and a lowering that handles a feature with one function returning a literal.

## overview
A feature is a folder with prose and code. `answer` gives the number 42; `hi` prints one word.

## interface
- `answer` gives 42.
- `hi` prints "hi".

## rules
- A `.zero` file may not contain `#`; the build refuses it and names the line.
- A result never assigned is its type's zero.

## testing
>answer() → 42
>hi() → "hi"

## hostile
A `#` anywhere in `skeleton.zero` stops the build with the file and line named.
