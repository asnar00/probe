# arrays
*an array is `int a[]` and a stream is `int i$`, and the mark is part of the name wherever it is written*

layer: runtime

> (suite) 2026-10-08T10:00:00
fm3 question 90, log 159. Ash, 7 October 2026: "streams and arrays ARE different kinds. For example, you can sort an array, but you can't sort a stream." And: "int i[] for an array, int i$ for a stream. The [] and $ suffix travel with the variable name, so the intent of any code is always clear. So it would be: `int a[] = [1, 2, 3, 4]` not `int[] a = [1, 2, 3, 4]`."

## overview
An array's items are all there, and it never changes. A stream's arrive, and it has a now. The text shows which a name is: `int a[] = [1, 2, 3, 4]` is an array and `int seen$` a stream, and the mark is written with the name everywhere, so no line has to be read with its declaration beside it.

The whole of an array is its name with empty brackets, `a[]`: `[count] (a[])`, `a[] + _`, `for (x in a[])`, `out$ << a[]`. One item is the name with a place in the brackets, `a[2]`. A list written out, a range and a text are arrays, and so is what `frame x$` gives: everything that has arrived in a stream and not been read, all there.

A function says what it takes and gives by the same marks: `on (int n) << total of (int x[])` takes an array, whole, and is called with its name in square brackets, `[total of] (a[])` (fm3 question 77, log 165), and `on (int r[]) << squares to (int k)` gives one, by pushing it once, `r[] << [1 through k] * [1 through k]`, as any function gives its result. The mark on the result is what tells it from a task, whose result is a stream, `on (int i$) << count up to (int n)`.

## what this store shows
Sixteen forms, a function each: an array from a list, from a range, from a text; one item by its place; `count`; a reduce; a map; a zip of two; `for`; a function of one item applied to each; a function that takes an array, handed a name and a list written out; a function that gives one, and its caller; what `frame` makes of a stream; an array pushed whole into the output; and an array at feature scope.

**An array lowers to the lines a sequence given whole by `=` always did.** Until `$` on an array was refused (fm3 log 161) this store had each form written twice, with the array's mark and, in a function named `... old` beside it, with the `$` every sequence had before the ruling, and the compiler's test asserted the two emitted the same text. The ruling changed how an array is written and nothing about how it is stored or what it costs.

## two arrays compared as wholes
An operation applied to each item of an array is written plainly, and an operation on the array as a whole is written in square brackets (fm3 question 77, log 164). `a[] + b[]` adds each pair. `a[] [==] b[]` is one `bool`: the two arrays have the same length and the same items in the same order; `a[] [!=] b[]` is its opposite. Both sides are arrays, a name, a list or a range written out, a `frame`; a `string` stands as its characters, so `s [==] cs[]` compares a text with an array of them. It is read where `==` is read and binds as `==` does. `the same`, `differing by an item`, `differing by length`, `two empty`, `not the same`, `a text and its letters`, `compared for a condition` and `what arrived` are its cases.

**Nothing is copied to compare** (fm3 log 183). A list written out as one side, `frame x$ [==] [4, 5, 6]`, is no array: its length is in the text, so the other side's is compared with that number once, and the items are compared each against the value where it stands, leaving at the first that differs. An item that is not a constant, `[k - 1, k + 1, k * 2]` in `worked out`, is worked out before anything is compared, in the order written. And `frame x$` as one side is read where the stream's items lie, with no array made of them; so is an array that is nothing but a frame, `point got[] = frame x$` in `a frame named`, where every use of it is a side of `[==]` or `[!=]`. That is done only where nothing can push into the stream between the frame and the last read of its items: `framed before a push` pushes in between, so its `got[]` is the copy it always was. `frame` moves the reader whether or not anything is copied: `framed twice` frames one stream twice and sees everything and then nothing, and `nothing arrived` compares with an empty list. `listed first` writes the list on the left.

Refused: one value or a stream on either side, `a[] [==] 2`, `a[] [==] x$` (the array of what has arrived is `frame x$`); two arrays of different types of item; arrays of structs. `[<]`, `[+]` and the other bracketed operators are refused as not ruled. And `if (a[] == b[])`, a bool for each pair where one is wanted, is refused showing the line with `[==]`; where an array is wanted, a plain comparison's answer would be an array of `bool`, which is not built.

## what is refused
- A name declared `a[]` and written `a$`, or declared `x$` and written `x[]` or `x[2]`: the mark is part of the name. A marked name written bare, `n << a`, the same.
- **`int i$ = [1, 2, 3]`**: what `=` gives is an array, and `$` is a stream's mark. The message shows the line as an array, `int i[] = [1, 2, 3]`, and the stream that begins with those items, `int i$ << [1, 2, 3]`. A `$` name given by `=` is a task's or a processor's stream, `int d$ = doubled(x$)`, and nothing else.
- An array declared any way but whole: `int a[] << 1 << 2`, `int a[] at (1 hz)`, a bare `int a[]`. An empty one is `int a[] = []`.
- A push into an array, `a[] << 9`: an array never changes.
- A stream's word on an array: `peek`, `latest`, `advance`, `position`, `time of`, `at`, `ended`, `end`, `empty`, `frame`, `behind`, `from ... to`. One item of an array is `a[k]`.
- An array's form on a stream: an item by its place, `x$[2]`, `for (v in x$)`, a reduce, `x$ + _`. The array of what has arrived is `frame x$`. A look back, `x$[-1]`, is a stream's; `a[-1]` is refused.
- An array where one value is wanted, `int v = a[]`, `if (a[] > 0)`: an array has no latest item.
- The same through a call: `int v = doubled (a[])`, `doubled` taking one item. Applied to each it gives an array, `int v[] = doubled (a[])`; for one, `doubled (a[1])`.
- A function declared over an array called plainly, `total of (a[])`: the message shows the line with its brackets. And a function of one item called in them, `[doubled] (a[])`: it is applied to each plainly, `doubled (a[])`.
- A stream handed to a function declared over an array, `[total of] (x$)`: the array of what has arrived is `frame x$`. And an array handed to a function or a task declared over a stream: what begins with those items is a stream, `int s$ << a[]`.
- A look back in a plain function or a task that walks, `x$[-1]`: only a stream processor has a present item to look back from.
- `count` is asked of both: an array's length, and how much of a stream is waiting.
- `int[] a`: the mark is on the name, `int a[]`.
- `token ops[]$`, a stream of arrays, and `int m[][]`, an array of arrays: ruled, and not built.
- An array as a field of a struct: not built.
- `on (int r[]) = squares to (int k)`: a function gives its result by pushing it, an array as any other.

## what is not here
`[sort] (a[])` and the other operations on an array as a whole but `[==]` and `[!=]` (fm3 question 77): ruled, and not built. A comparison applied to each item, `a[] == b[]`, whose answer is an array of `bool`: not built.

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
>the same() → 1
>differing by an item() → 0
>differing by length() → 0
>two empty() → 1
>not the same() → 1
>a text and its letters() → 1
>compared for a condition() → 7
>what arrived() → 1
>two points() → 1, 0
>points apart() → 1
>two spans() → 1, 0
>points listed() → 3
>points the same() → 1, 0
>points differing() → 0, 0
>points that arrived() → 1
>framed twice() → 1, 0
>listed first() → 1
>worked out (3) → 1
>worked out (4) → 0
>nothing arrived() → 1, 0
>a frame named() → 1, 1
>framed before a push() → 1, 1
