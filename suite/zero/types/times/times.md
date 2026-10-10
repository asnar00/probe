# times
*a `time` is a structure the language declares in zero, a count over a divisor, exact, and does only what is declared on it*

parent: types
layer: runtime

> (suite) 2026-10-10T10:00:00
fm3 question 117, Ash, 10 October 2026: "it should be a structure and we define only the functions on it that make sense". Log 201 to 203.

> (suite) 2026-10-10T18:00:00
fm3 question 120, Ash, the same day: a time is an exact rational, a count over a divisor. Log 210 to 213.

## overview
`time` is declared in the feature the language brings with it, `src/zero/platform.zero`, as a structure of two fields, a count and its divisor, both `hidden`, and its operators are functions declared there: a time added to a time and taken from one, multiplied and divided by a number, divided by a time, compared with a time, found the same as one, written out. Here each of them has a case. A time is exact: a third of a second is one over three, and three of them are a second. The compiler keeps what no declaration can say: `250 ms`, a literal with a unit word, makes a time, its nanoseconds over a thousand million, and `x$ at (t)` takes one.

## interface
- `beat` is a `time` at feature scope, `250 ms`.
- `a time added`, `a time taken`, `a time scaled`, `a time scaled by (n)`, `a number first`, `a time divided`, `a ratio`, `times compared`, `the same` and `each written` each use one of the language's functions on a time, and write what it gives.
- `a third given back`, `a third`, `two quarters` and `a ratio exactly` are what a reader must see of an exact time: `1 s / 3 * 3` is `1 s`.
- `two divisors` and `two divisors compared` add, take and compare times whose divisors differ; `a part of a second (n)` makes a time whose divisor is not known until the program runs, `1 s / n`; `a third and a half` gives a time as its result, which a case says as a time; `too fine (a, b)` adds two times whose sum no divisor that fits can hold.
- `(time a) * (time b)` is this feature's own operator, declared as on any structure; `squared` uses it.
- `twice (a)` takes a time and gives one; `kept two` keeps two in an array and compares the array whole, `ts[] [==] [250 ms, 750 ms]`; `nothing given` is the zero of the type. A time that was worked out handed to `x$ at (t)` is `suite/zero/streams`' `sampled at a time worked out`, a time word making every stream of its store keep a time.

## rules
- A time is added to a time and taken from one: `beat + 100 ms`, `1 s - beat`. Two of one divisor add by their counts; where one divisor is a multiple of the other the coarser count is scaled; otherwise both are brought to their least common multiple, `1 s / 3 + 1 s / 7` being ten twenty-firsts of a second. Where that cannot be held in 64 bits a check fails, saying a time is too fine to hold.
- It is multiplied by a whole number on either side and divided by one, exactly: `beat * 2`, `beat / 2`, `1 s / 3`. By a decimal, `1.5 * beat`, `beat / 2.5`, it is worked out in `float64` and cut to a whole nanosecond.
- A time divided by a time is a `float`, and is how a number comes out of one: `beat / 1 ms` is 250.0, `2500 ms / 1 ms` exactly 2500.0, and `int(2500 ms / 1 s)` its whole seconds.
- A time is compared with a time, `<`, `<=`, `>`, `>=`, `==`, `!=`: two that are the same moment are equal whatever their divisors, `1 s / 2 == 500 ms`.
- `out$ << t` writes it as the language would read it: to the nanosecond, toward zero, in the largest of `s`, `ms`, `us`, `ns` in which it is at least 1. So a third of a second is written `333.333333 ms` and is still a third.
- A program may declare an operator of its own on a time, as on any structure: with `on (time t) << (time a) * (time b)` declared here, `beat * beat` is that function.
- `time t` with nothing given is `0 s`.
- A case whose function gives a time says it as one, `→ 833.333333 ms`.

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
>a third given back() → "1 s"
>a third() → "333.333333 ms"
>two quarters() → "500 ms"
>a ratio exactly() → "2500.0"
>two divisors() → "500 ms 476.190476 ms 190.47619 ms"
>two divisors compared() → "true true true true"
>a part of a second (3) → "583.333333 ms false"
>a part of a second (4) → "500 ms true"
>a third and a half() → 833.333333 ms
>too fine (4000000007, 4000000009) → check

## hostile
Each is refused where the program is compiled, in these words (a store that does not declare the operator, `beat` a `time` and `n` an `int`):

- `beat + 1`: "no '+' is defined on a time and an int: '+' on a time is `(time) + (time)`". So is `beat + 0.5`, and `1 + beat`: "no '+' is defined on an int and a time: '+' on a time is `(time) + (time)`".
- `beat * beat`, in a store that declares no `*` of its own on two times: "no '*' is defined on a time and a time: '*' on a time is `(time) * (int)`, `(int) * (time)`, `(time) * (float)` and `(float) * (time)`".
- `beat < 1`: "no '<' is defined on a time and an int: '<' on a time is `(time) < (time)`".
- `beat % beat`: "no '%' is defined on a time and a time: a time has '+', '-', '*', '/', '==', '!=', '<', '<=', '>' and '>=' and no '%'". `n / beat`: "no '/' is defined on an int and a time: '/' on a time is `(time) / (int)`, `(time) / (float)` and `(time) / (time)`".
- `beat == 1`: "no '==' is defined on a time and an int: '==' on a time is `(time) == (time)`".
- `time t = 5`: "'t' is a time but the value is a bare number: say its unit, `5 s` or `5 ms`. No program sees a time's count or its divisor: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`". `time t = n`: "'t' is time but the value is int. No program sees a time's count or its divisor: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`".
- `int(beat)`: "int(x) converts a number, not a time: a conversion says no unit. A number out of a time is a time divided by a time, `t / 1 ms`, and its whole seconds are `int(t / 1 s)`". `float(beat)` says the same with `float(t / 1 s)`.
- `beat.count`: "'.count' on a time: a field declared `hidden` is the language's own, and no other feature reads or gives it. No program sees a time's count or its divisor: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`". `beat.divisor` says the same of '.divisor'.
- `time(5)`, `time(5, 1)` and `time(n)`: "`time(...)` gives a time its 'count': a field declared `hidden` is the language's own, and no other feature reads or gives it. No program sees a time's count or its divisor: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`".
- `int __x = 3`, a name that begins `__`, and `beat.__steps`, a field's: "a name may not start with '_' ('_' alone is the accumulator; a field a feature keeps to itself is declared `hidden`)".

The words are the same for any structure: with `type Vec =` and only `+` declared on it, `v * v` is "no '*' is defined on a Vec and a Vec: a Vec has '+' and no '*'" and `v + 1` is "no '+' is defined on a Vec and an int: '+' on a Vec is `(Vec) + (Vec)`". The compiler lists what is declared and knows nothing of what a time is.

A program's own operator on a time is allowed, as on any structure: this feature declares `on (time t) << (time a) * (time b)` and `squared` calls it. The language's own operator of the same types may not be declared again: `on (time t) << (time a) + (time b)` in a program is refused, "an operator is not redefined in this milestone; a `<<` method is". `type time =` in a program is refused: "type 'time' is already declared: it is the language's own".
