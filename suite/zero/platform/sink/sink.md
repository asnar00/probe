# sink
*a sink: a function with a stream parameter and no result, made a task by the line that wires it*

parent: machine
layer: runtime

> (suite) 2026-09-10T10:00:00
Third pass item 2 (rulings-3, log 57): a consumer of a stream is a *sink*, a function with a `$` parameter and no result, wired at feature scope. Rewritten in parity hop 20 (question 45, log 87): the sink used to read `out$`, and `out$` is now the output device, which has one reader and no storage, so it reads a stream of the feature's own instead.

## overview
`out$` is the output device the platform feature declares: a program writes by pushing into it, `out$ << "hello"`, and the push is the platform's write, which stores nothing (question 45). A stream of the store's own is the other thing, and this feature declares one, `char copy$`, with a consumer wired to it: `on log (char c$)` is a *sink*, a function with a stream parameter and no result, made a task by the line that wires it, `log(copy$)`, at feature scope. Its `ir` body counts the characters it consumes into `logged` and moves its reader past them; the scheduler runs it after every push into `copy$` from a function. `logged after hello` writes the same text to both, to the device and to the stream, and gives what the sink has seen.

## interface
- `log (c$)` is the sink: its `ir` body counts what it reads into `logged`, and returns its reader moved on, as every task returns the streams it moved.
- `logged after hello ()` writes "hello" and a newline to `out$` and to `copy$`, and gives how many characters `log` has consumed.
- `logged after nothing ()` writes nothing and gives the count.

## rules
- A sink is `on name (T c$)` with no result, wired by a bare line `name(x$)` at feature scope; the wiring makes it a task, so in the IR it takes and returns its stream parameters and `__hz`, and an `ir` body ends `ret` with the reader it moved.
- A sink is wired, never run: `log(copy$)` inside a function is refused, and `char x$ = log(copy$)` is refused since it fills nothing. A sink takes no rate.
- A stream consumer's body is `ir`: a target's rule lines take one word per operand, and a stream is four (question 32).
- With this feature off its node does not run, and `logged` stays at its initial value.
- `out$` is not a stream to wire a sink to (section 15, question 45, log 87): it is the output device, its only reader is the place, and it keeps nothing for a second reader to see. A store that wants to observe what it wrote keeps a stream of its own beside the device, as this feature does.
- `in$` keeps a ring, because a consumer wired to it — `token t$ = lex(in$)` in the `lex` store — reads it with `peek` and `advance`; that ring is the input device's own lookahead, sized by the runner (log 87).

## testing
>logged after hello() → 6
>logged after nothing() → 0
>logged after hello() with sink off → 0

## hostile
`log(copy$)` in a function body is refused: "'log' is a sink: it is wired at feature scope, `log (copy$)`, and not run here". `char x$ = log(copy$)` at feature scope is refused: "'log' is a sink: it fills no stream, so it is wired alone, `log (copy$)`". `log(copy$) at (2 hz)` is refused: "a sink has no rate: it runs when its input has more". Wiring a function with a result, `on (int64 r) = g (char c$)` then `g(copy$)`, is refused: "'g' gives a result: a wiring fills a stream, `T x$ = g (copy$)`, or names a sink, a function with a `$` parameter and no result". Wiring one with no stream parameter, `h(3)`, is refused: "'h' reads no stream: a sink has a `$` parameter".
