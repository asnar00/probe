# lex
*the lexer as a task: `tokens = lex(chars)` from `lex-experiment.md`, in zero*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan items 8 and 12 of milestone 0: the lexer written as a stream node in `fm3/examples/lex.ssa`, as a zero task with the four cases of `fm3/lex-experiment.md`.

## overview
Characters arrive over time in `chars$`; tokens leave in `t$`. `lex` reads the unread characters, pushes every complete token it can see, leaves a partial token — a word cut off by the end of what has arrived — unread, and stops. The scheduler runs it again when more arrives, from where it stood, so a token may span two arrivals and the node keeps no state of its own. When `chars$` ends, the last word is complete, and `lex` runs once more to take it. A token is its kind (1 a word, 2 a number, 3 punctuation), the index of its first character, and its length.

## interface
- `kind of (c)` classes a byte: 0 space, 1 letter, 2 digit, 3 punctuation.
- `lex (c$)` is the task; `t$ = lex(chars$)` wires it.
- `arrive` and `arrive again` push the experiment's two arrivals, `"let x = 4"` and `"2;\n"`, the second ending the stream.
- `two arrivals`, `number start`, `number length`, `kinds`, `third kind` and `kinds tail` are the experiment's four cases, split to two results each; `a word at the end` is the case the sentinel byte stood in for.

## rules
- `lex` takes a whole token only once it has seen a character of another class after it, or the stream has ended.
- After the first arrival three tokens have left and `4` waits; after the second, `42` (start 8, length 2) and `;` follow, and the newline is skipped.
- `end chars$` runs the node once more; a word that was waiting is pushed then.

## testing
>two arrivals() → 3, 5
>number start() → 8
>number length() → 2
>kinds() → 1, 1
>third kind() → 3
>kinds tail() → 2, 3
>a word at the end() → 0, 1

## hostile
A push into `chars$` after `end chars$` is a failed check. A byte above 127 is punctuation. A token longer than the ring keeps resident cannot be taken: the reader falls behind and the library's check fails.
