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
Each of sixteen forms is written twice, side by side: with the array's mark, and, in the function named `... old` beside it, with the `$` every sequence had before the ruling. **The two lower to the same lines.** An array is what a sequence given whole by `=` always was; this hop changes how it is written and nothing about how it is stored or what it costs. The compiler's own test, `an_array_is_the_sequence_it_was`, reads this store's emitted IR and asserts each pair's two functions are the same text but for their names.

The `old` halves are here to be compared, and go when a `$` on an array is refused.

## what is refused
- A name declared `a[]` and written `a$`, or declared `x$` and written `x[]` or `x[2]`: the mark is part of the name. A marked name written bare, `n << a`, the same.
- `int[] a`: the mark is on the name, `int a[]`.
- `token ops[]$`, a stream of arrays, and `int m[][]`, an array of arrays: ruled, and not built.
- An array as a field of a struct: not built.

## what is not here
An operation on an array as a whole written in square brackets, `a[] [==] b[]`, `[sort] (a[])` (fm3 question 77): ruled, and the next piece of work. In this store every operator and every function on an array means what it meant on a sequence given whole: an operator and a function of one item are applied to each, and a function declared over an array is handed it.

## testing
>listed() → 4
>listed old() → 4
>third() → 30
>third old() → 30
>ranged to (5) → 51
>ranged old to (5) → 51
>texted() → 5
>texted old() → 5
>summed() → 10
>summed old() → 10
>mapped() → 14
>mapped old() → 14
>zipped() → 440
>zipped old() → 440
>walked() → 10
>walked old() → 10
>each doubled() → 20
>each doubled old() → 20
>handed() → 69
>handed old() → 69
>squared() → 16
>squared old() → 16
>framed() → 60
>framed old() → 60
>said whole() → "1 2 3"
>said whole old() → "1 2 3"
>marked() → 10
>marked old() → 10
