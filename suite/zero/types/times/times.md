# times
*a `time` is a structure the language declares in zero, and does only what is declared on it*

parent: types
layer: runtime

> (suite) 2026-10-10T10:00:00
fm3 question 117, Ash, 10 October 2026: "it should be a structure and we define only the functions on it that make sense". Log 201 to 203.

## overview
`time` is declared in the feature the language brings with it, `src/zero/platform.zero`, as a structure of one field, and its operators are functions declared there: a time added to a time and taken from one, multiplied and divided by a number, divided by a time, compared with a time, written out. Here each of them has a case. The compiler keeps what no declaration can say: `250 ms`, a literal with a unit word, makes a time, and `x$ at (t)` takes one.

## interface
- `beat` is a `time` at feature scope, `250 ms`.
- `a time added`, `a time taken`, `a time scaled`, `a time scaled by (n)`, `a number first`, `a time divided`, `a ratio`, `times compared`, `the same` and `each written` each use one of the language's functions on a time, and write what it gives.
- `(time a) * (time b)` is this feature's own operator, declared as on any structure; `squared` uses it.
- `twice (a)` takes a time and gives one; `kept two` keeps two in an array and compares the array whole, `ts[] [==] [250 ms, 750 ms]`; `nothing given` is the zero of the type. A time that was worked out handed to `x$ at (t)` is `suite/zero/streams`' `sampled at a time worked out`, a time word making every stream of its store keep a time.

## rules
- A time is added to a time and taken from one: `beat + 100 ms`, `1 s - beat`. It is multiplied by a number on either side and divided by one: `beat * 2`, `1.5 * beat`, `beat / 2.5`. A whole number keeps it exact; a decimal is worked out in `float64` and cut to a whole step.
- A time divided by a time is a `float`, and is how a number comes out of one: `beat / 1 ms` is 250.0, and `int(2500 ms / 1 s)` its whole seconds.
- A time is compared with a time, `<`, `<=`, `>`, `>=`; `==` and `!=` are a structure's, every field the same (fm3 question 108), with nothing declared.
- `out$ << t` writes it as the language would read it: to the nanosecond, in the largest of `s`, `ms`, `us`, `ns` in which it is at least 1.
- A program may declare an operator of its own on a time, as on any structure: with `on (time t) << (time a) * (time b)` declared here, `beat * beat` is that function.
- `time t` with nothing given is `0 s`.

## testing
>a time added() → "350 ms 500 ms"
>a time taken() → "750 ms -750 ms"
>a time scaled() → "500 ms 125 ms"
>a time scaled by (3) → "750 ms 750 ms"
>a number first() → "500 ms 375 ms"
>a time divided() → "125 ms 100 ms 333.333333 ms"
>a ratio() → "2.5 250 2"
>times compared() → "true false true false true false true false"
>the same() → "true false false"
>each written() → "250 ms 2.5 s 1.5 ms 0 s -250 ms 1 ns"
>squared() → "62.5 ms"
>twice handed (2) → "1 s"
>kept two() → "true false"
>nothing given() → "0 s 250 ms"

## hostile
Each is refused where the program is compiled, in these words (a store that does not declare the operator, `beat` a `time` and `n` an `int`):

- `beat + 1`: "no '+' is defined on a time and an int: '+' on a time is `(time) + (time)`". So is `beat + 0.5`, and `1 + beat`: "no '+' is defined on an int and a time: '+' on a time is `(time) + (time)`".
- `beat * beat`, in a store that declares no `*` of its own on two times: "no '*' is defined on a time and a time: '*' on a time is `(time) * (number)` and `(number) * (time)`".
- `beat < 1`: "no '<' is defined on a time and an int: '<' on a time is `(time) < (time)`".
- `beat % beat`: "no '%' is defined on a time and a time: a time has '+', '-', '*', '/', '<', '<=', '>' and '>=' and no '%'". `n / beat`: "no '/' is defined on an int and a time: '/' on a time is `(time) / (number)` and `(time) / (time)`".
- `beat == 1`: "no '==' is defined on a time and an int: with none declared, '==' is of two of one structure, `(time) == (time)`, every field the same".
- `time t = 5`: "'t' is a time but the value is a bare number: say its unit, `5 s` or `5 ms`. No program sees a time's steps: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`". `time t = n`: "'t' is time but the value is int. No program sees a time's steps: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`".
- `int(beat)`: "int(x) converts a number, not a time: a conversion says no unit. A number out of a time is a time divided by a time, `t / 1 ms`, and its whole seconds are `int(t / 1 s)`". `float(beat)` says the same with `float(t / 1 s)`.
- `beat.__steps`: "'.__steps' on a time: a field whose name begins `__` is the language's own, and no other feature reads or gives it. No program sees a time's steps: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`".
- `time(5)` and `time(n)`: "`time(...)` gives a time its '__steps': a field whose name begins `__` is the language's own, and no other feature reads or gives it. No program sees a time's steps: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`".
- `int __x = 3`, a name of the program's own that begins `__`: "a name may not start with '_' ('_' alone is the accumulator; a name that begins `__` is the language's own)".

The words are the same for any structure: with `type Vec =` and only `+` declared on it, `v * v` is "no '*' is defined on a Vec and a Vec: a Vec has '+' and no '*'" and `v + 1` is "no '+' is defined on a Vec and an int: '+' on a Vec is `(Vec) + (Vec)`". The compiler lists what is declared and knows nothing of what a time is.

A program's own operator on a time is allowed, as on any structure: this feature declares `on (time t) << (time a) * (time b)` and `squared` calls it. The language's own operator of the same types may not be declared again: `on (time t) << (time a) + (time b)` in a program is refused, "an operator is not redefined in this milestone; a `<<` method is". `type time =` in a program is refused: "type 'time' is already declared: it is the language's own".
