# lex
*the lexer as a stream processor with no loop in it: six lines, each holding for every character (fm3 question 75), beside `suite/zero/lex`, which walks*

layer: runtime

> (suite) 2026-10-06T10:00:00
Parity hop sixteen, transformation 62 (fm3 log 128): the lexer of `suite/zero/lex` written the zeroic way, as `fm3/lex-ways.md` has it, with every other line of the store, and every case, as it is there. Hop twenty-six (fm3 log 168): a token's kind is a name, `type kind = space | word | number | mark`, where it was a number from 0 to 3; every case is as it was.

## overview
Characters arrive over time in a `char` stream; tokens leave in a stream of `token`. `lex` says five things, each for every character that arrives. `k$` is the character's class, a `kind`, and `space` on the one tick after the last character, which is what `empty c$` asks. `new$` is whether a token begins here: where the class is punctuation or differs from the one before. `start$` and `n$` are where the present run began and how long it is. And when a token begins, the run before it goes out, unless it was only space. There is no loop, no `peek`, no `count`, no `advance` and no index that could be wrong; `k$[-1]` is the class one character back, and before the first it is the zero of its type, which for an enumeration is its first case, `space`.

A token is its kind (`word`, `number`, or `mark` for punctuation), the index of its first character, and its length. A word cut off by the end of what has arrived is not read again when more comes, as the walking lexer reads it: the lexer keeps the run in progress, one earlier value of `k$`, `start$` and `n$`, and how many characters have come, four words for each place it is wired, and the next character goes on from there.

It is wired twice, as the walking lexer is. `token u$ = lex(src$)` is the lexer on `char src$`, a stream of the store's own that the program pushes the two arrivals into and ends. Nothing reads `src$` but the lexer, and every push into it is of a string literal, so it has no storage: `src$ << "let x = 4"` is a loop the compiler writes that says nine, with the lexer's lines in it and what they keep carried round it (parity hop seventeen, fm3 log 134), and `end src$` is its last tick. `token t$ = lex(in$)` is the same lexer on the input device, which the platform fills before the program starts and which has storage, so there the compiler writes the walk itself.

## interface
- `kind of (c)` classes a character: `space`, `word` for a letter, `number` for a digit, `mark` for punctuation. A case reads a `kind` as its place in the type, 0 to 3, so `kinds() → 1, 1` is two words.
- `lex (c$)` is the stream processor; `t$ = lex(in$)` wires it to the input device and `u$ = lex(src$)` to the store's own stream.
- `arrive first` pushes the first arrival, `"let x = 4"`, into `src$`; `arrive again` pushes the second, `"2;\n"`, and ends the stream.
- `two arrivals`, `number start`, `number length`, `kinds`, `third kind`, `kinds tail`, `a word at the end` and `lexed` are the cases of `suite/zero/lex`, each as it is there.

## rules
- A token goes out when the character after it arrives, or the input ends: after the first arrival three tokens have left and `4` is in progress; after the second, `42` (start 8, length 2) and `;` follow, and the newline is a space.
- `end src$` is one last tick on which `c$` is empty: `k$` is `space` there, so a word in progress is finished and goes out. `a word at the end` has no token before the end and one after.
- A second `end src$` is nothing, and a push into `src$` after its end fails a check.
- Nothing a program may say ends the input device, so with `in "let"` the word is in progress and `lexed()` is 0, as it is in `suite/zero/lex`.

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
A character above 127 is a `mark`. `c$[1]` in `lex` is refused when the program is compiled: it would be a character that has not come. A token of any length is taken, there being no queue of characters for it to outgrow.
