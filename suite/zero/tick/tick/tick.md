# tick
*the tick of a stream: every line it sets off, once, each after what it reads*

layer: runtime

> (suite) 2026-10-10T10:00:00
fm3 question 121 (Ash, 10 October 2026: what a tick is, "A"; the order within one, "Yes") and question 86 ("A, definitely"); log 206 to 208.

## overview
A line that stands, `sum$ << sum$ + x$ forever`, runs each time a stream it reads has a new item. A stream with no rate **ticks once for each push into it**, and a tick of `x$` is everything the standing lines then do: the lines `x$` sets off, the lines the streams they push into set off, and so on down. Within the tick **each line runs once, after every line it reads from has run, and lines that do not depend on each other run in the order they are written**.

    int x$
    int sum$
    sum$ << sum$ + x$ forever
    out$ << (x$ << " ") forever
    out$ << (sum$ << "\n") forever

`summed` pushes 1, 2 and 3 into `x$` and writes `1 1`, `2 3`, `3 6`: the sum is worked out, the number is written, and then the line that reads `sum$`, which had to wait for the first, writes the sum. Until this was built the first line ran to the bottom before the second began, and the same text wrote each sum before the number that made it.

## interface
- `summed` is the program above. `summed the number first` is the same with the first two lines written the other way round, the number's line and then the sum's. `summed the sum first` has the line that writes the sum written before the line that writes the number.
- `summed and counted` and `counted and summed` are `summed` again with a word that reads one of the streams in order, `[count] (frame ksum$)` and `[count] (frame q$)`, how many are waiting, so that the stream is stored.
- `diamond` pushes 1 and 2 into `d$`, which two lines read, `twice$ << d$ * 2 forever` and `next$ << d$ + 1 forever`, and a third reads both, `both$ << twice$ + next$ forever`. It gives `both$`; the feature `shown` writes it.
- `both` pushes 1 into `l$` and then 10 into `r$`, which one line reads, `either$ << l$ + r$ forever`.
- `got` pushes 1 and 2 into `i$`, which `out$ << ("got " << i$ << "\n") forever` reads after a text.
- `ended within a tick` pushes 1, 2 and 3 into `n$`: `low$ << n$ until (n$ == 2)` ends at the second, `hi$ << n$ * 10 forever` does not, and `out$ << (low$ << " " << hi$ << "\n") forever` reads both.
- `doubled beside` pushes 1 and 2 into `p$`, which a stream function reads, `int dbl$ = doubled (p$)`, and a line beside it, `out$ << (p$ << " ") forever`; a third line reads the function's output, `out$ << (dbl$ << "\n") forever`. `doubled where even` is the same over `e$` with a function that pushes only for an even number, `d$ << twice$ if (v$ % 2 == 0)`.
- `mixed` pushes by turns into `gain$`, which has no rate, and `beat$`, at `1 hz`; `loud$ << beat$ * gain$ forever` reads both.

## rules
- **Two lines that do not depend on each other run as written.** `summed the number first` writes the number and then works out the sum, and its text is the same, `1 1`, `2 3`, `3 6`. `summed the sum first` has `out$ << (run$ << "\n") forever` before `out$ << (w$ << " ") forever`: each sum is written before its number, `1`, `1 3`, `2 6`, `3 `, because that is the order on the page. A reader who wants the sum first writes its line first.
- **A line reached by two paths runs once.** In `diamond` a tick of `d$` is `twice$`, `next$`, and then `both$` once, 4 for the 1 and 7 for the 2. Run where each push is written it would be twice an item, the first time with a `next$` not yet worked out: a value that was never true of anything.
- **A line that reads several streams runs when any of them has something new, the others read for their latest** (question 86), which before a stream's first item is the zero of its type. `both` is two statements and so two ticks, one of `l$` and one of `r$`: `either$` gets 1, made with an `r$` that holds nothing yet, and then 11. Nothing in the text says the two pushes are one moment.
- **A line is set off by the streams its items name, wherever they stand in its chain.** `got` writes `got 1` and `got 2`: the text, the item, the newline, once for each item of `i$`. A stream named only in a line's `if` or its `until` is read for its value and sets nothing off (fm3 question 122).
- **What a stream is kept as does not show.** `[count] (frame ksum$)` makes `ksum$` a queue and `[count] (frame q$)` makes `q$` one; both functions write what `summed` writes and then the count, 3. (`count ksum$`, how many it has had, would not: a stream that only its name and `count` read keeps a counter beside its latest item and no queue, fm3 question 145, hop forty-three.) A line out of a stored stream is called where each item is pushed, after it is stored. Before this hop a stored `q$` handed all three items to the first line and then all three to the next.
- **A line that has ended is passed over.** In `ended within a tick` the line into `low$` ends once it has pushed the 2. The line that writes is still set off by `hi$` at the third item, and reads `low$` for its latest, which is still 2: `1 10`, `2 20`, `2 30`.
- **A stream with a rate and one without.** `mixed` pushes `gain$ << 2` at 0 s, and `loud$` gets 0, `beat$` holding nothing yet; then `beat$ << 1 << 2`, 2 at 0 s and 4 at 1 s; then `gain$ << 3` at 2 s, where the 2 is still `beat$`'s latest, 6; then `beat$ << 3`, 9, at 2 s as well. The unrated stream's tick falls where its push is written, between the beats.
- **A stream function is a line of the tick like any other** (fm3 question 123 as the eight principles settle it, log 226). `doubled beside` writes each number and then its double, `1 2`, `2 4`: the wiring is written first and runs first, the number's line is next as written, and the line that reads `dbl$` waits for the wiring. Until hop thirty-eight the function's output was pushed where its input was, and the same text wrote the double before the number, `2`, `1 4`, `2 `. Where the function does not push, `doubled where even` at 1 and 3, the line that reads its output is not set off: `1 2 4`, `3 4 8`. This holds for a function of each item that keeps nothing: one that looks back, counts its place, or pushes in more than one place of its text is still called where its input is pushed, its output's lines running inside it.
- **How it is compiled** (fm3 log 207). Where calling each line where its stream is pushed already runs the tick in order, which is every chain and every tree whose lines are written in the order they run, the push calls the lines and nothing more is written. Where it does not, the tick is one function, the lines' own statements in order, a pushed item going on in a local of the function to the lines that read it; where a push is under a condition, an `if`, a count, an `until` or a feature's switch, a second local says whether it was made. Nothing is kept between ticks: no list of lines waiting and no marks.
- **Left as it was** (fm3 question 123): a stream that takes more than one item in a tick, because a line pushes several items into it or two lines of the tick push into it, and a stream with a rate of its own, has its lines run where each item is pushed, once an item.

## testing
>summed() → "1 1\n2 3\n3 6"
>summed the number first() → "1 1\n2 3\n3 6"
>summed the sum first() → "1\n1 3\n2 6\n3 "
>summed and counted() → "1 1\n2 3\n3 6\n3"
>counted and summed() → "1 1\n2 3\n3 6\n3"
>diamond() → 7
>both() → "1\n11"
>got() → "got 1\ngot 2"
>ended within a tick() → "1 10\n2 20\n2 30"
>doubled beside() → "1 2\n2 4"
>doubled where even() → "1 2 4\n3 4 8"
>mixed() → "0\n2\n" at 0 s, "4\n" at 1 s, "6\n9\n" at 2 s, "end" at 3 s

## hostile
Two lines each set off by what the other pushes, `a$ << b$ forever` and `b$ << a$ forever`, are refused: "'a$ << b$' sets off 'b$ << a$', and that sets off the first again: a circle, and a tick of either would never end. A stream may be said from its own earlier items and never from itself at the present one: in a stream processor one of them looks back, `a$[-1]`". A line reached in order and also where a stream that takes several items is pushed, `a$ << (x$ << x$) forever`, `b$ << x$ forever` and `z$ << a$ + b$ forever`, is refused: "'z$ << ...' is set off twice in one tick of 'x$': through 'b$', and through 'a$', which a line pushes more than one item into. Whether it then runs once, or once for each item, is not ruled (fm3 question 123). Not built".
