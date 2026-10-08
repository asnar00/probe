# arrays
*an array is `int a[]` and a stream is `int i$`, and the mark is part of the name wherever it is written*

layer: runtime

> (suite) 2026-10-08T10:00:00
fm3 question 90, log 159. Ash, 7 October 2026: "streams and arrays ARE different kinds. For example, you can sort an array, but you can't sort a stream." And: "int i[] for an array, int i$ for a stream. The [] and $ suffix travel with the variable name, so the intent of any code is always clear. So it would be: `int a[] = [1, 2, 3, 4]` not `int[] a = [1, 2, 3, 4]`."

## overview
An array's items are all there, and it never changes. A stream's arrive, and it has a now. The text shows which a name is: `int a[] = [1, 2, 3, 4]` is an array and `int seen$` a stream, and the mark is written with the name everywhere, so no line has to be read with its declaration beside it.

The whole of an array is its name with empty brackets, `a[]`: `count a[]`, `a[] + _`, `for (x in a[])`, `out$ << a[]`. One item is the name with a place in the brackets, `a[2]`. A list written out, a range and a text are arrays, and so is what `frame x$` gives: everything that has arrived in a stream and not been read, all there.

A function says what it takes and gives by the same marks: `on (int n) << total of (int x[])` takes an array, and `on (int r[]) << squares to (int k)` gives one, by pushing it once, `r[] << [1 through k] * [1 through k]`, as any function gives its result. The mark on the result is what tells it from a task, whose result is a stream, `on (int i$) << count up to (int n)`.

## what this store shows
Sixteen forms, a function each: an array from a list, from a range, from a text; one item by its place; `count`; a reduce; a map; a zip of two; `for`; a function of one item applied to each; a function that takes an array, handed a name and a list written out; a function that gives one, and its caller; what `frame` makes of a stream; an array pushed whole into the output; and an array at feature scope.

**An array lowers to the lines a sequence given whole by `=` always did.** Until `$` on an array was refused (fm3 log 161) this store had each form written twice, with the array's mark and, in a function named `... old` beside it, with the `$` every sequence had before the ruling, and the compiler's test asserted the two emitted the same text. The ruling changed how an array is written and nothing about how it is stored or what it costs.

## what is refused
- A name declared `a[]` and written `a$`, or declared `x$` and written `x[]` or `x[2]`: the mark is part of the name. A marked name written bare, `n << a`, the same.
- **`int i$ = [1, 2, 3]`**: what `=` gives is an array, and `$` is a stream's mark. The message shows the line as an array, `int i[] = [1, 2, 3]`, and the stream that begins with those items, `int i$ << [1, 2, 3]`. A `$` name given by `=` is a task's or a processor's stream, `int d$ = doubled(x$)`, and nothing else.
- An array declared any way but whole: `int a[] << 1 << 2`, `int a[] at (1 hz)`, a bare `int a[]`. An empty one is `int a[] = []`.
- A push into an array, `a[] << 9`: an array never changes.
- A stream's word on an array: `peek`, `latest`, `advance`, `position`, `time of`, `at`, `ended`, `end`, `empty`, `frame`, `behind`, `from ... to`. One item of an array is `a[k]`.
- An array's form on a stream: an item by its place, `x$[2]`, `for (v in x$)`, a reduce, `x$ + _`. The array of what has arrived is `frame x$`. A look back, `x$[-1]`, is a stream's; `a[-1]` is refused.
- An array where one value is wanted, `int v = a[]`, `if (a[] > 0)`: an array has no latest item.
- The same through a call: `int v = doubled (a[])`, `doubled` taking one item. Applied to each it gives an array, `int v[] = doubled (a[])`; for one, `doubled (a[1])`.
- A stream handed to a function declared over an array, `total of (x$)`: the array of what has arrived is `frame x$`. And an array handed to a function or a task declared over a stream: what begins with those items is a stream, `int s$ << a[]`.
- A look back in a plain function or a task that walks, `x$[-1]`: only a stream processor has a present item to look back from.
- `count` is asked of both: an array's length, and how much of a stream is waiting.
- `int[] a`: the mark is on the name, `int a[]`.
- `token ops[]$`, a stream of arrays, and `int m[][]`, an array of arrays: ruled, and not built.
- An array as a field of a struct: not built.
- `on (int r[]) = squares to (int k)`: a function gives its result by pushing it, an array as any other.

## what is not here
An operation on an array as a whole written in square brackets, `a[] [==] b[]`, `[sort] (a[])` (fm3 question 77): ruled, and the next piece of work. In this store every operator and every function on an array means what it meant on a sequence given whole: an operator and a function of one item are applied to each, and a function declared over an array is handed it.

## testing
>listed() → 4
>third() → 30
>ranged to (5) → 51
>texted() → 5
>summed() → 10
>mapped() → 14
>zipped() → 440
>walked() → 10
>each doubled() → 20
>handed() → 69
>squared() → 16
>framed() → 60
>said whole() → "1 2 3"
>marked() → 10
