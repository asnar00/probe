# echo
*the input stream `in$`: a case pushes into it, and the program reads it*

parent: sink
layer: runtime

> (suite) 2026-09-10T10:02:00
Third pass item 4 (rulings-3, log 62): input is the system stream `in$`, declared by the compiler's `platform` feature, which the platform pushes into; under the runner a case's line is the push.

## overview
`in$` is a stream of characters the platform feature declares beside `out$`, without a rate: a keyboard is a sparse stream on the clock. It is the input device, and its ring is the device's own lookahead (question 45, log 87). A case says what arrives with `with in "text"` in its `with` clause, beside the switches, and the runner pushes the bytes after the reset and the switches and before the program starts, so a node reading `in$` has run when the case's function is called (`lex` is the store that shows one). `echo` copies what is waiting to `out$`; `bytes waiting` counts it.

## interface
- `echo ()` writes `frame in$` to `out$`.
- `bytes waiting ()` gives `count in$`.

## rules
- `with in "text"` is one clause of the case's `with`, in any order with the switches, and one per line; the clause is read by the lexer, so a comma or a bracket inside the string is the string's.
- A case with no `in` starts with `in$` empty.
- `in$` holds 512 characters: the input device's own lookahead, sized by the runner rather than by the language (question 45, log 87); a program pushes into `in$` and ends it like any stream (question 35).

## testing
>echo() with in "hi" → "hi"
>echo() with in "a, (b)" → "a, (b)"
>bytes waiting() with in "hello" → 5
>bytes waiting() → 0
>bytes waiting() with sink off, in "abc" → 3

## hostile
`>echo() with in "a", in "b"` is refused: "a case has one `in \"text\"`". `>echo() with in hi` is refused: "a case's `with` clause is `<feature> off`, `<feature> on` or `in \"text\"`, several joined by commas". `on (char o$) << (char c$)` is not the way to consume `in$`: a consumer is a sink wired to it, `log(in$)` (see `sink`), or a task, `token t$ = lex(in$)`.
