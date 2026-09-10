# lex
*the lexer as a task: `tokens = lex(chars)` from `lex-experiment.md`, in zero*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan items 8 and 12 of milestone 0: the lexer written as a stream node in `fm3/examples/lex.ssa`, as a zero task with the four cases of `fm3/lex-experiment.md`. Third pass item 4 (rulings-3, log 62): the characters arrive in the platform's input stream `in$`, and the wiring is `token t$ = lex(in$)`.

## overview
Characters arrive over time in `in$`, the `char` stream the platform feature declares for the input device (section 15, question 45); tokens leave in `t$`. `lex` reads the unread characters, pushes every complete token it can see, leaves a partial token — a word cut off by the end of what has arrived — unread, and stops. The scheduler runs it again when more arrives, from where it stood, so a token may span two arrivals and the node keeps no state of its own. When `in$` ends, the last word is complete, and `lex` runs once more to take it. A case's first arrival is its line's `with in "let x = 4"`, pushed by the runner before the program starts; the second, `"2;\n"`, is the program's own push. A token is its kind (1 a word, 2 a number, 3 punctuation), the index of its first character, and its length.

## interface
- `kind of (c)` classes a character: 0 space, 1 letter, 2 digit, 3 punctuation. A `char` is compared against the code points, `c <= 32`, and never computed with (question 44).
- `lex (c$)` is the task; `t$ = lex(in$)` wires it.
- `arrive again` pushes the experiment's second arrival, `"2;\n"`, and ends the stream; the first, `"let x = 4"`, is each case's `with in`.
- `two arrivals`, `number start`, `number length`, `kinds`, `third kind` and `kinds tail` are the experiment's four cases, split to two results each; `a word at the end` is the case the sentinel byte stood in for.

## rules
- `lex` takes a whole token only once it has seen a character of another class after it, or the stream has ended.
- `t$` is a stream of the struct `token`, which is one ring whose item is the struct (question 43, log 88): `t$ << token(k, start, n)` is one push and `peek t$ at (i)` one read.
- After the first arrival three tokens have left and `4` waits; after the second, `42` (start 8, length 2) and `;` follow, and the newline is skipped.
- `end in$` runs the node once more; a word that was waiting is pushed then.
- The runner pushes a case's `in` before `__zero_start`, so `lex` has run over the first arrival when the case's function is called.

## testing
>two arrivals() with in "let x = 4" → 3, 5
>number start() with in "let x = 4" → 8
>number length() with in "let x = 4" → 2
>kinds() with in "let x = 4" → 1, 1
>third kind() with in "let x = 4" → 3
>kinds tail() with in "let x = 4" → 2, 3
>a word at the end() with in "let" → 0, 1

## hostile
A push into `in$` after `end in$` is a failed check. `>f() with in "a", in "b"` is refused: "a case has one `in \"text\"`". A character above 127 is punctuation. A token longer than the ring keeps resident cannot be taken: the reader falls behind and the library's check fails.
