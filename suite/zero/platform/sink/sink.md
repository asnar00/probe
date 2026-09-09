# sink
*a store's own consumer of `out$`: a sink wired beside the platform's `write`, with a body in the IR*

parent: machine
layer: runtime

> (suite) 2026-09-10T10:00:00
Third pass item 2 (rulings-3, log 57): output is the system stream `out$`, declared by the compiler's `platform` feature and consumed by its task `write`, wired `write(out$)`; a store may wire a consumer of its own to the same stream.

## overview
`out$` is a stream of bytes the platform feature declares; a program writes by pushing into it, `out$ << "hello"`, and `print` is a zero function over it. Its consumer, `on write (uint8 c$)`, is a *sink*: a function with a stream parameter and no result, made a task by the line that wires it, `write(out$)`, at feature scope. This feature wires a second consumer, `log(out$)`, whose `ir` body counts the bytes it consumes into `logged` and moves its reader past them; the scheduler runs it after every push into `out$` from a function, as it runs `write`. The test runner reads `out$` through the platform feature's own reader, which no consumer moves.

## interface
- `log (c$)` is the sink: its `ir` body counts what it reads into `logged`, and returns its reader moved on, as every task returns the streams it moved.
- `logged after hello ()` writes "hello" and a newline and gives how many bytes `log` has consumed.
- `logged after nothing ()` writes nothing and gives the count.

## rules
- A sink is `on name (T c$)` with no result, wired by a bare line `name(x$)` at feature scope; the wiring makes it a task, so in the IR it takes and returns its stream parameters and `__hz`, and an `ir` body ends `ret` with the reader it moved.
- A sink is wired, never run: `log(out$)` inside a function is refused, and `uint8 x$ = log(out$)` is refused since it fills nothing. A sink takes no rate.
- A stream consumer's body is `ir`: a target's rule lines take one word per operand, and a stream is four (question 32).
- With this feature off its node does not run, and `logged` stays at its initial value.

## testing
>logged after hello() → 6
>logged after nothing() → 0
>logged after hello() with sink off → 0

## hostile
`log(out$)` in a function body is refused: "'log' is a sink: it is wired at feature scope, `log (out$)`, and not run here". `uint8 x$ = log(out$)` at feature scope is refused: "'log' is a sink: it fills no stream, so it is wired alone, `log (out$)`". `log(out$) at (2 hz)` is refused: "a sink has no rate: it runs when its input has more". Wiring a function with a result, `on (int64 r) = g (uint8 c$)` then `g(out$)`, is refused: "'g' gives a result: a wiring fills a stream, `T x$ = g (out$)`, or names a sink, a function with a `$` parameter and no result". Wiring one with no stream parameter, `h(3)`, is refused: "'h' reads no stream: a sink has a `$` parameter".
