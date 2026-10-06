# lex
*the lexer as a task: `tokens = lex(chars)` from `lex-experiment.md`, in zero*

layer: runtime

> (suite) 2026-09-08T10:00:00
Plan items 8 and 12 of milestone 0: the lexer written as a stream node in `fm3/examples/lex.ssa`, as a zero task with the four cases of `fm3/lex-experiment.md`. Third pass item 4 (rulings-3, log 62): the characters arrive in the platform's input stream `in$`, and the wiring is `token t$ = lex(in$)`. Parity hop ten (question 35, fm3 log 101): a program never writes its input, so the experiment's arrivals go into a stream of the store's own, `src$`, with the same task wired to it.

## overview
Characters arrive over time in a `char` stream; tokens leave in a stream of `token`. `lex` reads the unread characters, pushes every complete token it can see, leaves a partial token — a word cut off by the end of what has arrived — unread, and stops. The scheduler runs it again when more arrives, from where it stood, so a token may span two arrivals and the node keeps no state of its own. When the characters end, the last word is complete, and `lex` runs once more to take it. A token is its kind (1 a word, 2 a number, 3 punctuation), the index of its first character, and its length.

The task is wired twice. `token t$ = lex(in$)` is the lexer on the input device, `in$`, the `char` stream the platform feature declares (section 15, question 45): what arrives there comes from the platform alone, which under the runner is a case's `with in "text"`, pushed before the program starts. And `token u$ = lex(src$)` is the same task on `char src$`, a stream of the store's own, which the program itself pushes the experiment's two arrivals into and ends. A program never writes its input (question 35, fm3 log 101): `in$ << "2;\n"` and `end in$`, which this store's cases did until parity hop ten, are refused.

## interface
- `kind of (c)` classes a character: 0 space, 1 letter, 2 digit, 3 punctuation. A `char` is compared against the code points, `c <= 32`, and never computed with (question 44).
- `lex (c$)` is the task; `t$ = lex(in$)` wires it to the input device and `u$ = lex(src$)` to the store's own stream.
- `arrive first` pushes the experiment's first arrival, `"let x = 4"`, into `src$`; `arrive again` pushes the second, `"2;\n"`, and ends the stream.
- `two arrivals`, `number start`, `number length`, `kinds`, `third kind` and `kinds tail` are the experiment's four cases, split to two results each; `a word at the end` is the case the sentinel byte stood in for.
- `lexed` counts the tokens the lexer on the device has made.

## rules
- `lex` takes a whole token only once it has seen a character of another class after it, or the stream has ended.
- `t$` and `u$` are streams of the struct `token`, each one ring whose item is the struct (question 43, log 88): `u$ << token(k, start, n)` is one push and `peek u$ at (i)` one read.
- After the first arrival three tokens have left and `4` waits; after the second, `42` (start 8, length 2) and `;` follow, and the newline is skipped.
- `end src$` runs the node once more; a word that was waiting is pushed then.
- A push into `src$` from a function is followed by the scheduler's run of the nodes `src$` reaches, the one lexer wired to it, so `lex` has run over an arrival when the next line of the function reads `u$`. The lexer on the device is not run then: nothing has arrived there.
- The runner pushes a case's `in` before `__zero_start`, so the lexer on the device has run over it when the case's function is called: `lexed() with in "let x = 4"` is 3, the `4` still waiting. Nothing a program may say ends the device, so with `in "let"` the word waits and `lexed()` is 0; what ends an input is the platform's to say, and under the runner nothing says it yet.

## testing
>two arrivals() → 3, 5
>number start() → 8
>number length() → 2
>kinds() → 1, 1
>third kind() → 3
>kinds tail() → 2, 3
>a word at the end() → 0, 1
>lexed() with in "let x = 4" → 3
>lexed() with in "let" → 0

## hostile
`in$ << "2;\n"` in a function, `in$ << src$` at feature scope and `end in$` are each refused: "'in$' is the input device: a program reads it and never writes it or ends it. Input comes from the platform alone, which under the runner is a case's `with in \"text\"`; a program that makes its own arrivals pushes them into a stream of its own". A push into `src$` after `end src$` is a failed check. `>f() with in "a", in "b"` is refused: "a case has one `in \"text\"`". A character above 127 is punctuation. A token longer than the ring keeps resident cannot be taken: the reader falls behind and the library's check fails.
