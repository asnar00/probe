# History

What landed, one short entry per commit — or per group, when several arrived together as one piece of work. Newest first. `git show <hash>` has the full story for any of them.

---

### zero: a stream that keeps nothing does not fill — `0d2ed15` · 2026-10-08

```
on (int n) << climbed to (int k)
    up$ << 1 << (up$ + 1) while (_ <= k)
    n << up$
```

`suite/zero/words/words/words.zero:201-203`; `up$` is wired to the output. Read by its name and wired on, a stream was a queue of sixty-four that nothing gave back, so `climbed to (100)` failed a check at its sixty-fifth item; so did a stream a standing line's condition reads. In `src/zero/lower.rs`, `cell_candidates` no longer strikes what an edge mentions, so such a stream is a cell; and one something is wired to and that is otherwise only read by name keeps one word, its latest, stored in `emit_push` before what is wired is called, with no queue (`nowed`). Of twelve shapes tried, seven now pass at a hundred items; four are left, each said in fm3's log 177. Six cases in `words`. `brackets`, `timed` and `words` moved, every moved case cheaper as counted, `tallied` 512 to 184. The six rows unmoved. Zero 879/879 natively and on wasm; `cargo test zero` 70.

### zero: a published feature may be refactored, and must pass its own cases — `0bc8ef8` · 2026-10-08

```
on (int n) << answer()
    n << 42
```

`suite/zero/skeleton/skeleton/skeleton.zero:1-2`, the suite's published feature. fm3's question 89: "refactor when needed but tests must pass". The check was of the text and its date, and nothing was run. Now `check_published` in `src/zero/store.rs` records that a published feature's code has changed and refuses only what nothing could hold: code never committed, changed code with no case, and a change to the cases themselves. `held` in `src/zero/run.rs` then runs the feature's own cases before anything else and refuses the store if one fails, naming it. Tried on the lines above and not committed: `n << 40 + 2` passes with the date left alone, `n << 43` is refused. No store's IR moved; the six rows unmoved. The full run: zero 873/873 on four paths, 849 and 24 skipped on the GPU's, `probe test` 1024, 1015, 1024, 1024, 987, `cargo test` 164.

### zero: `until (an event)` ends a line that stands at the event — `6541c53` · 2026-10-08

```
stopped$ << tock$ until (stop$ == 1)
```

`suite/zero/words/words/words.zero:46`. fm3's question 85, ruled "B": a condition about the item is tested after the push, and one about something else is an event, at which the line ends with nothing going out after. `parse_decl` in `src/zero/syntax.rs` tells them by the streams the condition names and records a `Watch`. For a stream's value, `collect_edge` writes a second function of the line on that stream (`watcher`, `src/zero/lower.rs`), which sets the line's bit, so `stop$` needs no storage; for `ended key$` the bit is set where the stream is ended. `flowed to the end of the key` gives 2 where it gave 3, the one case that changed; two timed cases, each `"0\n1\n2" at 1 hz`. `words` alone moved; the six rows unmoved. Zero 873/873 natively and on wasm, `cargo test zero` 68.

### zero: three faults a visitor can reach, a conversion, a time written out, a literal held — `ad54175` · 2026-10-08

```
on times written()
    time t = 250 ms
    out$ << t << "\n" << t * 10 << "\n" << t * 40 << "\n" << 1 s / 3 << "\n" << 0 s - t
```

`suite/zero/types/types/types.zero:224-226`, which writes `250 ms`, `2.5 s`, `10 s`, `333.333333 ms`, `-250 ms`. From the playground's report (fm3 log 173). `float(n)` of an abstract `int` broke the lowered IR in any function whose first line did not name `int`: the IR's `conv` to a library number never asked the policy how wide a body's `int` is. One line of `src/ssa.rs`; 1 792 of 1 920 conversions compiled before, all now. A `time` gets a `<<` method in `src/zero/platform.zero`, and a literal may be a decimal. And `Lowerer.holds` refuses `uint8 low = 300` with the type's range, an abstract `int`'s literal under the products that cannot hold it. No store's IR moved; the six rows unmoved. The full run: zero 871/871 on four paths, 847 and 24 skipped on the GPU's, `probe test` 1024, 1015, 1024, 1024, 987, `cargo test` 162.

### zero: the old form refused, a result pushed once at the top level — `105aba6` · 2026-10-08

```
on (int n) << size of (int x)
    n << x if (x > 0) else 0 - x
    out$ << "sized "
```

`suite/zero/pushed/pushed/pushed.zero:98-100`. fm3's question 88, its third landing. The parser (`not_top`, `src/zero/syntax.rs`) refuses a result's push under an `if` statement, showing the program's own push with its `if` on it, one inside a loop, and one with `if` and no `else`; the lowering refuses a result pushed twice and one nothing pushes. And the push no longer ends the function: `Lowerer.tail` in `src/zero/lower.rs` writes the `ret` at a push only where it is the last thing the function does, so the line above's third line runs. No store's IR moved; the six rows unmoved; the meter 130 of 2 252. The full run: zero 867/867 on four paths, 843 and 24 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 160.

### zero: the suite says it, a result pushed once with its condition — `940417e` + `f0fd9d3` · 2026-10-08

```
on (int s) << sign of (int x)
    s << -1 if (x < 0)
         else 1 if (x > 0)
         else 0
```

`suite/zero/control/control/control.zero:1-4`. fm3's question 88, its second landing. `940417e` gives the meter a tenth row in `src/zero/meter.rs`, a result pushed under an `if` statement: 39 lines in ten stores, the meter 130 → 169. `f0fd9d3` rewrites all fourteen functions by a kept script: `kind of` in four lexers and `class of` become tables with no line of IR moved; `sign of` says its third case; `first positive of` says `else` where it leaned on a push ending the function; `power of two above` leaves its loop by `break` and pushes once; `either` keeps its check under the `if` and pushes after it, 3 dearer. The row 39 → 0, the meter 169 → 130 of 2 242. Zero 864/864 natively and on wasm; the six rows unmoved.

### zero: `else` on a push, and the table — `51b40d2` · 2026-10-08

```
on (int k) << class of (int c)
    k << 0 if (c <= 32)
         else 3 if (c > 122)
         else 1 if (c >= 97)
         else 3 if (c > 57)
         else 2 if (c >= 48)
         else 3
```

`suite/zero/pushed/pushed/pushed.zero:64-70`. fm3's question 88, its first landing: a condition goes on the push, `x << a if (c) else b`, read from the left, a case a line where the lines begin `else` indented under the push. `push_else` in `src/zero/syntax.rs` builds the `if` statement with a push in each arm that the line replaces, so the lowering is untouched and the lexer's stores emit the same 555 lines with `kind of` as a table. Into a stream it pushes one or the other; in a stream processor it is one item (`chosen`, `src/zero/zeroic.rs`); with a loop word it is refused, two rulings pulling apart. Zero 864/864 natively and on wasm; the six rows unmoved.

### zero: the six-line lexer's kinds are names — `293c88b` · 2026-10-08

```
on (token t$) << lex (char c$)
    kind k$ = if (empty c$) then (space) else (kind of (c$))
    bool new$ = k$ == mark or k$ != k$[-1]
    index start$ = if (new$) then (position c$) else (start$[-1])
    index n$ = if (new$) then (1) else (n$[-1] + 1)
    t$ << token(k$[-1], start$[-1], n$[-1]) if (new$ and k$[-1] != space)
```

`suite/zero/lex-zeroic/lex/lex.zero:25-30`, over `type kind = space | word | number | mark` on line 1. Respelled alone, the count rose 21, 502 to 523: an enumeration's case is one bare word, which the parser keeps like a call with no arguments, so `k$[-1] != space` was not a thing that "can do nothing but give a value" and the push stayed outside the branch its lines share. `plain` in `src/zero/zeroic.rs` is now told which bare words are a case. Before 502 and 509 as counted, 746 and 753 on the tool; after, the same four, delta 0. The two stores' IR differs in `int` written `kind`, a `u8`. No case's text changed. Zero 849/849 natively and on wasm.

### probe takes the zero playground's patch: where the compiler reads from, and what a host wants of the runner — `0345261` · 2026-10-08

```
/// `std::fs::read_to_string`, or the mounted text
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    MOUNTED.with(|m| match m.borrow().as_ref() {
        Some(map) => map.get(&key(path)).cloned().ok_or_else(|| missing(path)),
        None => std::fs::read_to_string(path),
    })
}
```

`src/vfs.rs:49-56`. The zero playground runs this front end in a page, built for wasm32, and kept a patch; probe has it now. Every read on the way from a store's text to a module goes through `src/vfs.rs`: the disk, or texts a host mounted. Two reads the patch lacked are routed too, so a refusal shows its line in the page. `src/host.rs` takes the policy a path compiles under and the wasm driver's call spec out of `src/suite.rs`; `Store.times` keeps `__out_mark` for a host; `LOCAL` in `src/bitcode.rs` fits a 32-bit word. No store's IR moved. The playground's crate builds from this tree unpatched, `cargo build --release --target wasm32-unknown-unknown`, and its 18 examples emit the bytes `probe zero test <store> wasm` writes. The full run: zero 849/849 on four paths, 826 and 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 158.

### zero: a function that takes an array whole is called in square brackets — `1da3eef` · 2026-10-08

```
on (int n) << sum of (int x[])
    n << x[] + _

on (int n) << passed()
    n << [sum of] ([1, 2, 3])
```

`suite/zero/sequences/sequences/sequences.zero:80-84`. The other half of fm3's question 77. The brackets stand round the name's words up to its first group; `src/zero/syntax.rs` reads `[`, words, `]` and then a group as a call, kept as its phrase with `Part::Whole` first. `lower_call` in `src/zero/lower.rs` has a bracketed call choose among the methods over an array and a plain call among the rest: `sum of (a[])` is refused showing the line with its brackets, `[doubled] (a[])` as applied to each plainly. `describe` has both kinds, and the call now says which. Six calls rewritten; no store's IR moved. The full run: zero 849/849 on four paths, 826 and 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 156.

### zero: `[==]` and `[!=]`, two arrays compared as wholes — `642cd1d` · 2026-10-08

```
on (bool same) << the same()
    int a[] = [1, 2, 3]
    int b[] = [1, 2, 3]
    same << a[] [==] b[]
```

`suite/zero/arrays/arrays/arrays.zero:75-78`. fm3's question 77: an operation on each item is written plainly, one on the whole array in square brackets. `[==]` gives one bool, the same length and the same items in order. `src/zero/syntax.rs` reads it as three tokens, `[`, an operator, `]`, which no list begins with. `src/zero/lower.rs` writes a loop that leaves at the first pair that differs, 20 an item as counted. A stream or one value on either side is refused, and so are `[<]`, `[+]` and the rest, as not ruled. `if (a[] == b[])` is refused showing the line with `[==]`. Eight cases, on all five paths; no other store's IR moved. The suite 849/849 natively, `cargo test zero` 61.

### zero: in a push that happens once, a stream's name is its value now — `023a1c5` · 2026-10-08

```
on pushed on()
    seen$ << 1 << 2 << 3
    out$ << seen$
```

`suite/zero/now/now/now.zero:19-21`. It writes `3`, where it wrote `1 2 3`: fm3's question 79. `src/zero/lower.rs` gains `now`, said of each item of a push in a function: a stream's name is its latest item, an array's the array. `total$ << total$ + other$` is one sum of two latest items, both streams one word each, 4 as counted where it was 209. Before its first item a name reads as zero. A line with `forever` and a stream processor are unchanged. The IR of three stores moved: `types`, its two lines now `out$ << frame said$`; one function each of `cells` and `words`, by the zero, 6 more. The suite 841/841 natively, `cargo test zero` 60, the meter 130 of 2 224.

### zero: the lowering knows which kind a name is, and three crossings of the two kinds are refused — `6d57dcd` · 2026-10-08

```
    fn arr_name(&self, name: &str, b: &Body) -> bool {
        match b.vars.get(name) {
            Some(v) => v.arr,
            None => self.fvar(name).is_some_and(|f| f.arr),
        }
    }
```

`src/zero/lower.rs:8480-8485`. Until now the mark on a name was written away just before the lowering ran, so it could not tell `out$ << a[]` from `out$ << x$`. The uses still are; the declaration's mark is kept, on `Var`, `FVar` and each parameter and result of `FnInfo`, and `Kind` says what an expression is: one value, an array or a stream. Three things that wanted a call resolved are now refused: a stream handed to a function over an array, `sum of (s$)`; `int v = doubled (a[])`, which gave the last item doubled; and a look back in a plain function, which read the buffer's header. No store's emitted IR moved. The suite 827/827 natively, `cargo test zero` 60.

### zero: `$` means a stream, each word held to its kind and the 21 names used both ways rewritten — `6246a2b` · 2026-10-08

```
on (int n) << indexed()
    int i$ << 5 << 6 << 7
    int a[] = frame i$
    n << a[1] * 10 + a[2]
```

`suite/zero/streams/streams/streams.zero:143-146`. The last part of fm3's question 90. `int i$ = [1, 2, 3]` is refused, the message showing the line as an array and as a stream that begins with those items. `src/zero/kinds.rs` holds each word to its kind: `peek`, `advance` and `latest` are a stream's, an item by its place, `for` and a reduce an array's, `count` both; an array is never pushed into and has no latest item. Twenty-one names were used both ways, and each case keeps its number: the one above asked a stream for `i$[1]` and now makes the array first. The IR of five stores moves for those cases; none of the six rows is in them. The full run: zero 827/827 on four paths, 804 and 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 153. The meter 130 of 2 150.

### zero: the suite says it, 47 names respelled as arrays and no store's IR moved — `ec9c6df` · 2026-10-08

```
on (int n) << zipped()
    int i[] = [1, 2, 3, 4]
    int j[] = [4, 5, 6]
    int k[] = i[] + j[]
    n << count k[] * 100 + k[3] * 10 + (k[] + _) - 25
```

`suite/zero/sequences/sequences/sequences.zero:41-45`. The second half of fm3's question 90. Every name the compiler's own print sorts as an array is written `a[]`: 90 lines in 12 files by a script that takes each name's declaration and uses from `probe zero names`, so a stream of the same spelling a few lines down is left alone. The platform's `print (int x[])` and its two block methods with them. No store's emitted IR moved, on a copy first, then a store at a time against the last binary, then after the rebuild. The 21 names used both ways wait for the next landing. The full run: zero 841/841 on four paths, 818 and 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 153. The six rows as they were.

### zero: an array is `int a[]`, the mark part of the name and the lines what they were — `95b2558` · 2026-10-08

```
on (int r[]) << squares to (int k)
    r[] << [1 through k] * [1 through k]
```

`suite/zero/arrays/arrays/arrays.zero:106-107`. fm3's question 90, Ash: "int i[] for an array, int i$ for a stream. The [] and $ suffix travel with the variable name." The lexer reads `a[]` as one token, an array's name whole, and `a[2]` as that name asked for an item. `src/zero/kinds.rs` holds every name to the mark it was declared with, `a$` for a declared `a[]` refused naming the declaration's line, and then writes each array's name as the lowering always read a sequence, so an array lowers to the same lines. The function above is plain, told from a task by its result's mark. Sixteen forms are written both ways in the new store and a test asserts each pair emits the same text. No existing store's IR moved; zero 841/841 natively; the six rows as they were.

### zero: `probe zero names`, every name with a `$` and each form it is used in — `d28071e` · 2026-10-08

```
int lit$ = [1, 2, 3]
int dl$ = doubled(lit$)
```

`suite/zero/zeroic/zeroic/zeroic.zero:5-6`. fm3's question 90 rules that a stream, `int i$`, and an array, `int a[]`, are two kinds, and before the suite can say so every name in it has to be sorted. `src/zero/kinds.rs` is one walk over a parsed store that scopes a name as the lowering does, and `probe zero names <store>` prints each name declared with a `$`: where, how, and every use with the form it stands in. The list above is given whole and then handed to a processor, so it cannot be told: one of 21 such names in 363, with 47 arrays and 295 streams. Of the 335 names sorted from the text at hop twenty, 331 are sorted the same. Nothing is lowered or refused.

### zero: `forever` applies to the last item, wiring of several items says so with brackets — `b42aecb` · 2026-10-07

```
int i$ at (1 hz)
out$ << (i$ << "\n") forever
```

`suite/zero/hello/countdown/countdown.zero:1-2`. The second half of fm3's question 84, Ash: "Yes." `forever` covers the last item of its chain as the other words do, so hello's wiring line is written with brackets: every number, each with a newline. Twenty-three standing lines in nine files gained them by a script, and no store's emitted IR moved by a line, the last binary on the old text against itself on the new, then against this one. Without the brackets the line is `i$` once and then a newline for ever, which nothing paces: refused when the program is compiled, the message saying what the line meant before the ruling and showing the program's own line bracketed. A count and an `until` on a line that stands cover a bracketed chain, `out$ << (greet$ << "\n") (3) times`, three lines of hello a second apart. The full run: zero 813/813 on four paths, 790 and 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 151. The six rows as they were; the meter 126 of 2 087.

### zero: brackets widen a word, a group of a push's items is one item for its word — `cd429a8` · 2026-10-07

```
on three lines()
    out$ << ("hello" << "\n") (3) times

on one line()
    out$ << "hello" << "\n" (3) times
    out$ << "."
```

`suite/zero/brackets/brackets/brackets.zero:12-17`. A word on a push applies to the last item of its chain, and brackets round several items make them the one it applies to (fm3 question 84, Ash, 7 October): three lines of hello, where the second function writes hello and three newlines. The parser tells a group from a bracketed value by a `<<` directly inside the bracket, which never compiled before, so `(a)`, `(i$ + 1)` and `(twice (k))` are the values they were. The tree keeps one flat list of items and a number, how many of its last the word covers; `lower_pushes` and `push_cell` are its two readers. `(n) times` pushes the group n times; `until` pushes it and then asks, `_` its last item; `while` works the group out whole, asks, and pushes all of it or none. A group stands last in its chain and is refused before another item. No existing store's emitted IR moved. The zero suite 811/811 natively and on wasm, `cargo test zero` 56; the six rows as they were; the meter 126 of 2 079.

### zero: the old form is refused, one way to declare a function — `4e804a6` · 2026-10-07

```
on (int d) << double (int x)
    d << x * 2
```
```
on (int r$) = squares to (int k)
    r$ = [1 through k] * [1 through k]
```

`suite/zero/pushed/pushed/pushed.zero:1-2` and `suite/zero/sequences/sequences/sequences.zero:86-87`. The first is the one way; written `on (int d) = double (int x)` it is refused, "a function is declared with `<<` and gives its result by pushing it; `=` says what a name is (fm3 question 77). Write `on (int d) << double (int x)`", and `d = x * 2` in its body the same way, each message showing the program's own line respelled. Both refusals are the parser's, which knows a function's results while it reads the body and has the text to show. The second is the one function that keeps `=`: a `$` on its result, a sequence given whole, and with `<<` a task's first line; it waits for arrays to have their mark (fm3 question 87). `pushed` loses its thirteen old halves and fourteen cases. Every store emits what it did from the last binary. The full run: zero 795/795 on four paths, 772 and 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 149. The six rows as they were; the meter 126 of 2 035.

### zero: the suite says it, 358 functions declared with `<<` — `35876e6` · 2026-10-07

```
on (int r) << first positive of (int a) and (int b)
    if (a > 0)
        r << a
    r << b
```

`suite/zero/functions/functions/functions.zero:31-34`. Every function of the suite whose results have no `$` is respelled, first line and body: 358 first lines and 418 body lines in 37 files, all by `scratchpad/respell.py`, none by hand. No store's emitted IR moved by a line: the script ran first on a copy, every store of the copy emitting what the tree's did (34 stores, 0 differ); then on the tree a store at a time, the last binary on the committed suite against this one on the tree after each of 26 stores, nothing named, the suite green each time. One function is left, `on (int r$) = squares to (int k)`, a task's first line once it says `<<` (fm3 question 87). `skeleton` is a published feature: respelled, and published again as of the day (question 89). The Rust tests' zero text says it too, 113 first lines and 128 body lines by the same script. The full run: zero 809/809 on four paths and 786 with 23 skipped on the GPU's, `probe test` 1022, 1013, 1022, 1022, 985, `cargo test` 149. The six rows as they were; the meter 128 of 2 072.

### zero: a function may be declared with `<<` and give its result by pushing it — `fa90584` · 2026-10-07

```
on (int d) << double (int x)
    d << x * 2

on (int d) = double (int x) as it was
    d = x * 2
```

`suite/zero/pushed/pushed/pushed.zero:1-5`. fm3's question 77 (a), ruled: one way to declare, and `=` left for saying what a name is. This landing accepts the new form beside the old. With `<<` on everything, the mark on the result tells a function from a task: no `$`, a value given once; a `$`, a stream produced over time. `y << value` is kept in the tree as the giving of a result an assignment was (`Target::pushed`), so it lowers to the same lines: fourteen pairs in the store, each the same IR but for the name (`scratchpad/pairs.py`). One function cannot be told, `on (int r$) = squares to (int k)`, a task's first line once it says `<<`; it keeps `=` until arrays have their mark (question 87). A result nothing pushed is zero and the last push ends the function, as before (question 88). Refused: a result pushed twice, or with `(n) times`, `while`, `until`, `forever`; a push into a local, a parameter, a variable. No existing store's IR moved; zero 809/809, `cargo test zero` 55; the six rows as they were; the meter 128 of 2 072, the new store's loops.

### zero: the running sum — `fdd6404` · 2026-10-07

```
int x$
int sum$
```
```
sum$ << sum$ + x$ forever
```

`suite/zero/words/words/words.zero:21-22` and `:28`. fm3's question 80, ruled: on the right of its own standing push a stream's own name is a read of its latest and sets nothing off, and the other stream there paces the line. The parser keeps the pacing stream as the line's first item and the expression beside it (`Decl::Edge`'s `first`); the edge's function pushes the expression. What `sum$` is kept as does not show. Read only by its name it is a cell, a load, an add and a store, 7 an item counted where the same sum as a stream processor read by name is 26. Wired on and named nowhere else it has no storage and the line keeps its last item, 86 an item with the digits against the processor's 83. Wired and read both, a queue, its read guarded before the first item and the queue not freed under it: the first try printed 1, 66, 67, 68. Two other streams are refused (question 86); the clock's message says it is ruled. 38 cases in `words`; no existing store's IR moved; zero 777/777, `cargo test zero` 54; the six rows as they were; the meter 124 of 1 991.

### zero: `until` on a push — `0cd1bd7` · 2026-10-07

```
on counted to five()
    up$ << 1 << (up$ + 1) until (up$ == 5)

on counted under five()
    up$ << 1 << (up$ + 1) while (_ < 5)
```

`suite/zero/words/words/words.zero:82-86`. The second of the two words fm3's question 79 added. `until` pushes and then asks, so the item that makes its condition true goes out, `1 2 3 4 5`; `while` asks of the candidate first, `1 2 3 4`. In an `until`, `_` and the stream's own name are both the item just pushed, and the two spellings lower to the same lines. `if` now goes with any one of `(n) times`, `while`, `until` and `forever` and comes first; two of the four are refused by name; `until (false)` and `while (true)` are refused as never ending. At feature scope `till$ << flow$ until (flow$ == 3)` stands until it holds, a bit of the context kept for the line; with `until (ended key$)` the first item after the end still goes out (fm3 question 85). A count or an `until` with more than one item on a standing line is refused, which the last landing read one way (question 84). 31 cases in `words`; no existing store's IR moved; zero 772/772, `cargo test zero` 53; the six rows as they were; the meter 124 of 1 958.

### zero: `(n) times` on a push — `3604cef` · 2026-10-07

```
on up to (int k)
    up$ << 0
    up$ << up$ + 1 (k) times

on called (int k)
    up$ << twice (k) (3) times
```

`suite/zero/words/words/words.zero:18-20` and `:32-33`. fm3's question 79 gave a push two more words; this is the first, n pushes. The trap was in the parser: a call swallows the count, and `times` is a word of two function names in the suite. The rule is one sentence: a bracketed group directly before `times` is the count and never an argument, unless a declared name has `times` after the words so far (`count_ahead`, `src/zero/syntax.rs`), the way `and` and `or` are told. One line cannot be told and is refused with both spellings, `x$ << three (k) times` where `three (int k) times` is declared. The count is worked out once and covers a chain's last item, as `while` does; each push works its item out again. Into a cell five bumps count 51 as they run, the `for` they replace 66. At feature scope `first$ << src$ (3) times` stands for the first three, its count a hidden field of the context. A new store, `suite/zero/words`, 19 cases; no existing store's IR moved; zero 760/760 natively, `cargo test zero` 52; the six measured rows as they were; the meter 124 of 1 924.

### zero: what changes at feature scope is a stream — `8dd11b2` · 2026-10-07

```
int written$ << 0

on (char o$) << (int x)
    written$ << written$ + 1
    existing o$ << x

on (int n) = counted()
    out$ << 1 << 2 << 3
    n = written$
```

`suite/zero/platform/watch/watch.zero:1-9`. fm3's question 70: no variable is ever modified. A name declared with a value keeps it, and what changes is a stream, pushed once each time its line runs and read by its name. The nineteen feature-scope variables the suite assigned, in nine files, are streams now, every scope word, a struct, an enumeration, a bool and a string among them; each is a cell, so every rewritten function costs what it cost, 446 functions on the tool and 178 counted as they ran with none moved (`scratchpad/costdiff2.py`, `countdiff2.py`). An assignment to a feature-scope name is refused with the declaration and the push to write (`not_assigned`, `src/zero/lower.rs`), and so is one to a parameter. A string that changes is a stream only a cell can hold, its bare name its latest item wherever it stands, `out$ << name$` (fm3 question 83, provisional). Of the meter's other three assigning lines: a local written twice is an `if` expression; a result given on two paths is not a modification and the meter learns so; and `tasks`' `s$ = far$`, a task pointing its parameter elsewhere, goes with its case (question 82, provisional). zero 741/741 natively, `cargo test zero` 51; the six measured rows as they were; the meter 150 of 1 882 → 124 of 1 869, "a name assigned again" 22 → 0.

### zero: a loop's variables are given by `continue`, its result by `break` — `4e975fe` · 2026-10-07

```
        index n = loop (index i = 1, index m = -1) yields m
            if (i >= count c$)
                if (ended c$)
                    break (i)
                break
            if (kind of (peek c$ at (i)) == k)
                continue (i + 1, -1)
            break (i)
```

`suite/zero/lex/lex/lex.zero:36-43`. fm3's question 70 has no exception for a loop: its variables are not assigned in its body, the next pass's values being `continue (...)`, and an assignment to one is refused when the program is compiled. A loop that found its answer partway used to assign the name it yields and then `break`; `break (values)` (fm3 question 81, provisional) gives the yielded names their values where the loop leaves, the mirror of `continue (values)` and the IR's own `break`. The nine lines the meter listed are rewritten, in `control`, `streams`, `lex` and `lex-static`, and the seven loops of the platform's `<<` methods with them. 960 functions of 32 stores costed before and after (`scratchpad/costdiff2.py`): none moved; the lexer's text is two lines shorter, a `break` in an arm where a value-yielding `if` fed one. zero 742/742 natively, `cargo test zero` 50; the six measured rows as they were; the meter 159 of 1 887 → 150 of 1 882.

### zero: a stream read only for its latest item is one word — `0e19c65` · 2026-10-07

```
int seen$ << 0
```
```
on bump()
    seen$ << seen$ + 1

on (int n) = bumped twice()
    bump()
    bump()
    n = seen$
```

`suite/zero/cells/cells/cells.zero:9` and `:18-24`. fm3's questions 70 and 79: nothing that changes is assigned; it is a stream, its value its latest item and a write a push. A feature-scope stream the store reads only for its latest item is a **cell**, one field of the context of the item's type: a push is a store of the field and a read a load, with no ring, no capacity and nothing made in the arena. And a stream's name where one value is wanted, `n = seen$`, `int x = seen$ + 1`, `t$ << token(3, seen$, 1)`, a condition, an argument of a function of one value, is its latest item; that is every place the compiler refused the name before, so no program that compiled changes and no store's emitted IR moved. Which streams are cells is settled by lowering, since whether one value is wanted is a matter of types: each candidate is lowered as a cell, a name met where a stream is wanted is noted, and the store is lowered once more with those as streams (`lower`, `cell_candidates`, `push_cell`, `src/zero/lower.rs`). `bump` is the four lines an assigned variable's was, 5 on the tool for both; the lexer of fm3 question 74's prototype with its state in two such streams is 691 on the tool and 1 005 as counted, what it is with assigned variables, where it was 2 074 and 2 858. `counted up to (10000)` pushes ten thousand times, where a queue of 64 fails its check at the sixty-fifth. zero 742/742 natively, `cargo test zero` 49; the six measured rows as they were; the meter 159 of 1 887.

### zero: `forever` decides whether a `<<` stands — `d0ac463` · 2026-10-07

```
out$ << pair$ << " " forever
out$ << pair$ << "\n" forever
heard$ << part$ forever
out$ << level$ << "\n" if (level$ > 2) forever
```

`suite/zero/edges/edges/edges.zero:10-13`. fm3's question 79: a `<<` sends once each time its line runs, wherever it is written, and `forever` makes it stand. So a wiring line says the word, where until now it was wiring because of where it stood. It is the last word of its line; with `if (c)` before it the line is a standing filter, the edge's push under the condition (`collect_edge`, `src/zero/lower.rs`). The fifteen wiring lines of the suite say it and no store's emitted IR moves but the two that gain a case. Refused, each tested in `forever_decides`: the same line with no word, for now, the message giving both things it could mean; `forever` in a function, in a stream processor, on a declaration, with `while`, before `if`, and on a line of values; and a stream that feeds itself forever, with no rate because it would never end and at a rate because the clock needs a schedule ordered by time, which is not built. `suite/zero/timed`'s `ticks` is the clock that can be written, a function's `tick$ << 0 << (tick$ + 1) while (_ < 4)` at `1 hz`. zero 723/723 natively, `cargo test zero` 48; the six measured rows as they were; the meter 159 of 1 808.

### zero: `if` on a push, where `when` was — `b3e7d19` · 2026-10-07

```
    t$ << token(k$[-1], start$[-1], n$[-1]) if (new$ and k$[-1] != 0)
```

`suite/zero/lex-zeroic/lex/lex.zero:28`. fm3's question 79: `when` reads as waiting, `if` as a test made now, so a push made only where a condition holds is `x$ << item if (condition)`. The tree and the lowering are what they were, and no store's emitted IR moves: 31 stores, the old binary on the old text against the new on the new. The parser (`src/zero/syntax.rs`) tells the three `if`s by where the word stands: first on a line, the statement; where a value is wanted, the expression `if (c) then (a) else (b)`; after a push's last item, the push's. `p$ << if (k > 5) then (10) else (20) if (k > 2)` is both. `when` is no word of the language: a line with one is refused saying to write `if`, and it may be a word of a function's name again, as `and` and `or` may. A name declared with a word that ends every phrase, `if` or `in`, is refused where it is declared. The meter's row is reworded so the statement cannot be taken for the word; 159 of 1 800. Eleven lines of the suite respelled. zero 719/719 natively, `cargo test zero` 47; the six measured rows as they were.

### zero: a gate is one load and one branch at any depth — `90733b9` · 2026-10-07

fm3's question 72. A feature is on when its own switch and every ancestor's are, and a gate used to work that out each time it was read, a load a level and an `and` between. The context now holds the answer for each feature under another, a field `__on_<feature>`, and every gate is one field loaded and one branch however deep. It is worked out again only in a switch's setter, for the feature and everything under it, parents before children; a feature's own switch is written by nothing else, so a parent off and on again leaves its children as they were. The runner now makes a case line's switches in order, a call for each `off` and each `on`, where it used to set what the line came to. `suite/zero/nested` is a chain three deep with ten case lines that switch each level by turns; a setter made to stop at its own feature fails nine of them. hello's `run` as counted 1 073 to 1 064, level with its hand-written oracle's 1 064, and 1 233 to 1 224 on `probe cost`; the lexers and static do not move. What rose is the setters themselves and the reset, the runner's and outside every count. zero 719 runs on four paths and 697 on the GPU's; `probe test` 1022; `cargo test` 141.

### zero: every function reaches its state through the current context, and a store runs in two — `80aac28` · 2026-10-07

The front end's half of fm3's question 71. A function that touches state forms no address: its first line is `_this: ptr = context()`, the register on arm64 and riscv64, and every field is one load or store through it. `__zero_context(k)` is the runner's and makes the k-th of the store's contexts the current one; the runner calls it before the reset on every path, and the reset is now two functions, `__zero_reset` for the store and `__zero_new` for the current context, so a second context is made beside the first. `two_contexts_each_keep_their_own` runs hello in two contexts by turns, countdown off in one and on in the other, and a six-line lexer whose word in progress, ended bit and counter are each context's own. One thing found: `elide-stores` did not know the context's pointer for the program's own memory, so a field written through it stored the whole struct back, the six lines counting 646 for 506; it knows now. As counted, `two_arrivals`: the six lines 506 to 502 static and 513 to 509 dynamic, against oracles of 504 and 536, the static store now under its oracle; the walking lexer 932 to 929 and 943 to 940; hello's `run` 1 077 to 1 073; static's 1 037, unmoved. On `probe cost` 750 to 746, 757 to 753, 590 to 587, 601 to 598, 1 237 to 1 233. Every path green: zero 698 and 676 on the GPU's, `probe test` 1022, `cargo test` 141.

### The IR has the current context: `context()` and `context_set(p)`, a register on arm64 and riscv64 — `2a59758` · 2026-10-07

fm3's question 71 ruled that the context is `this`: a pointer the machine keeps, which a function never asks for or is handed. The IR says it with a library's two operations, as it says the current thread: `c: ptr = context()` and `context_set(p)`, in a new `lib/context.ssa` whose bodies read and write one word of data, and that is their meaning. A platform says where it keeps the pointer by a rule: x28 on arm64, s11 on riscv64, a second global on wasm32; the GPU's path has no rule and runs the library's word. The compiler knows three things. The register leaves the allocator's pool only in a module that names the context. A read is no instruction where nothing can change the register: a function that sets the context, calls through a function value or calls such a function is unsettled, and in every other the value `context()` gives is the register itself, so a field of the context is one load, `ldr x, [x28, #8]`; elsewhere it is one move. And under the JIT, where x28 is the host's outside a call, every function of such a module has a wrapper that swaps the host's value for the context and back, keeping the context in the library's word between calls. A fibre carries its context for nothing, `fibre_switch` already saving the register. `probe cost` and `probe count` price a read at 0 and a set at 1. `suite/context.ssa` has seven cases on all five paths, two fibres in two contexts interleaved among them. A module that does not name the context compiles as it did: all 61 files of the suite, fm3's oracles and the zero stores' expected files have every function the same words as before. `probe test` 1022 on the three machines' paths, 1013 on wasm, 985 on the GPU's; `cargo test` 140. `ssa.md` has a section, *The current context*.

### README.md and the stores' notes: a block's loop has the lines in it — `566c854` · 2026-10-06

A doc follow-up to hop seventeen's two landings on the front end (`16abfac`, `f4ef5c9`). `suite/zero/lex-zeroic/lex/lex.md`, its static twin and `suite/zero/zeroic/zeroic/zeroic.md` said a literal pushed into a stream processor's input is a loop of calls of its function; it is a loop with the processor's lines in it, what they keep carried round it. The zero section of `README.md` says so, says that lines turning on one condition are one branch, and lists the three things `probe count` learned in the hop: `--where`, `--blocks=` and `--from=`. No store's emitted text moves.

### zero: a block pushed into a stream processor has its lines in the loop — `f4ef5c9` · 2026-10-06

```
on arrive first()
    src$ << "let x = 4"
```

`suite/zero/lex-zeroic/lex/lex.zero:30-31`. The parity pass, hop 17, transformation 68 (fm3 log 134). That push was a loop of nine calls of the lexer's function of one item. The loop holds the function's lines now, lowered in place by what lowers the function (`z_inline`, `src/zero/lower.rs`): no call, no return, the kept values the loop's own. A single item, a range and a stored input's sink still call. The six-line lexer's `two_arrivals`, counted as it ran: 542 → 506 against its oracle's 504, and 549 → 513 against 536, under it; the oracle counts its own reset. Lines 450 → 476. zero suite 698/698; `cargo test zero` 46.

### zero: a condition several lines turn on is branched on once — `16abfac` · 2026-10-06

```
on (int r$) << decades (int x$)
    bool new$ = x$ / 10 != x$[-1] / 10
    int n$ = if (new$) then (1) else (n$[-1] + 1)
    int first$ = if (new$) then (x$) else (first$[-1])
    r$ << first$[-1] * 100 + n$[-1] when (new$ and n$[-1] > 0)
```

`suite/zero/zeroic/zeroic/zeroic.zero:294-298`. The parity pass, hop 17, transformation 67 (fm3 log 133). Two lines and a push that turn on `new$` were three branches an item in the function the front end writes; they are one `if`, the lines' names its results and the push in the arm where it holds (`grouped`, `src/zero/zeroic.rs`). What is left of a push's condition waits for the branch only where it cannot fail. The six-line lexer's `two_arrivals`, counted as it ran: 575 → 542 against its oracle's 504, and 582 → 549 against 536. `tens to (100)` 3 085 → 2 615. zero suite 698/698; `cargo test zero` 45.

### `probe count --blocks`: a row for each block of one function — `ebbf84f` · 2026-10-06

The parity pass, hop 17, transformation 66 (fm3 log 132): nothing built but a way to see. `probe count <file> <case> --blocks=<fn>` gives each block of one function its weight, how many times the run entered it and what it counted, by counting that block alone (`count_blocks`, `src/cost.rs`). With it the six-line lexer's 575 and `lex-each-min.ssa`'s 504 were put side by side: 36 for a call and a return a character, 36 for three branches where a person writes one, 14 for a loop's count formed each pass, and the oracle's own reset, 16, that the store's count does not have. `cargo test` 137.

### `probe count --from`: one function as a case calls it — `590c7d3` · 2026-10-06

The parity pass, hop 17, transformation 65 (fm3 log 131): a whole case is priced by the count from here on (fm3 question 68). hello's modular oracle switches its features on inside its case, so its `run` could not be counted alone: 40, with every switch off. `probe count <file> <case> --from=<fn>` reports what the function counted from its entry to its return, over every call the case made, as a difference of the counter added to a second word (`count_from`, `src/cost.rs`); `--where` gives a row a function. hello's `run` 1 077 against `hello-mod.ssa`'s 1 064, static's 1 037 against 1 025; by `probe cost` they are 1 237 against 1 286 and 1 197 against 1 247. `cargo test` 137.

### README.md: the six-line lexer, the meter and the count — `745b756` · 2026-10-06

A doc follow-up to hop sixteen's seven landings (`3c78024` to `115f895`). The zero section of `README.md` quotes the lexer of `suite/zero/lex-zeroic`, six lines with no loop, says in three sentences what the front end makes of such a body, and lists the two commands the hop added: `probe zero meter`, the lines of each store that use a non-zeroic form, and `probe count`, the cost tool's count taken as a function runs.

### zero: `timed` with its task in one line, as a store beside it — `115f895` · 2026-10-06

```
on (int d$) << doubled (int x$)
    d$ << x$ * 2
```

`suite/zero/timed-zeroic/timed/timed.zero:7-8`. The parity pass, hop 16, transformation 64 (fm3 log 130): the smallest store the meter lists that the five rules can say, rewritten beside the first. Every other line and all five timed cases are `suite/zero/timed`'s, and nothing could not be said. With no word reading `i$` or `d$`, neither has storage: a push is the processor's function, then the edge's, then the rate's step. `count down` 965 against 2 440, the lowered text 276 lines against 406, the meter 0 of 22 against 3 of 26. 5 runs on all five paths; zero suite 696/696.

### zero: the meter, `probe zero meter` — `f866bfb` · 2026-10-06

The parity pass, hop 16, transformation 63 (fm3 log 129). zero is in two parts, the zeroic and the non-zeroic, and removing the second is an aspiration, so the compiler now counts it. `probe zero meter <store>` lists every line that uses one of the nine non-zeroic forms `fm3/touchstones.md` keeps, with its file, line and form, and prints the rule that finds each; over a folder of stores it gives a row a store and the total (`src/zero/meter.rs`). It reads the tree and lowers nothing, so it refuses nothing. Over `suite/zero`: 158 of 1 755 lines in 23 stores; `lex` 17 of 91, `lex-zeroic` 6 of 71, its cases' `peek`s alone. A test pins two small stores, one wholly zeroic.

### zero: the lexer in six lines, and a count taken as a function runs — `b8b88b6` · 2026-10-06

```
on (token t$) << lex (char c$)
    int k$ = if (empty c$) then (0) else (kind of (c$))
    bool new$ = k$ == 3 or k$ != k$[-1]
    index start$ = if (new$) then (position c$) else (start$[-1])
    index n$ = if (new$) then (1) else (n$[-1] + 1)
    t$ << token(k$[-1], start$[-1], n$[-1]) when (new$ and k$[-1] != 0)
```

`suite/zero/lex-zeroic/lex/lex.zero:23-28`. The parity pass, hop 16, transformation 62 (fm3 log 128). The lexer with no loop in it, all nine of `suite/zero/lex`'s cases unchanged, and a static twin. `probe cost` reads `two_arrivals` 829 and 822 against the walking lexer's 601 and 590 and the oracles' 500 and 468: its loops are bounded, so it alone is charged every character, each at its worst. `probe count` (`src/cost.rs`), new, takes the same count as the function runs: the six lines 582 and 575, the oracles 694 and 662, the walking lexer 943 and 932. The same algorithm by hand counts 501.

### zero: nothing in gives nothing out, and `empty` asks — `7e31bc8` · 2026-10-06

```
on (int o$) << capped (int x$)
    o$ << x$
    o$ << -1 when (empty x$)
```

`suite/zero/zeroic/zeroic/zeroic.zero:244-246`. The parity pass, hop 16, transformation 61 (fm3 log 127): question 75's fourth rule. The end of the input is one last tick, a function of its own for each wiring (`src/zero/zeroic.rs`). What is nothing on it is decided when the program is compiled: `empty` is true there and false for an item, so neither function branches on it, and a line or a push that still reads the item is left out. `end` of an input with no storage calls it once, under one bit that also fails a later push. The output ends after, where anything could tell. No existing store's text moved. `suite/zero/zeroic` 38 cases; zero suite 671/671; `cargo test zero` 43.

### zero: `when` on a push — `fe74ded` · 2026-10-06

```
on (int c$) << changes (int x$)
    c$ << x$ when (x$ != x$[-1])
```

`suite/zero/zeroic/zeroic/zeroic.zero:201-202`. The parity pass, hop 16, transformation 60 (fm3 log 126): question 75's third rule. `x$ << item when (condition)` pushes the item where the condition holds. It stands where `while` stands, and a push with both is refused. In a stream processor it holds for each item, and lowers to the push under a branch in the function of one item; in a plain function it is the `if` round the push. In the tree (`src/zero/syntax.rs`) it is an `if` marked as written with `when`, so every pass reads it as any `if`. No existing store's text moved. `suite/zero/zeroic` 31 cases on all five paths; zero suite 664/664; `cargo test zero` 42.

### zero: a stream looks back, `x$[-1]`, and `or` and `and` join two conditions — `8f697ac` · 2026-10-06

```
on (int s$) << summed (int x$)
    int t$ = t$[-1] + x$
    s$ << t$
```

`suite/zero/zeroic/zeroic/zeroic.zero:126-128`. The parity pass, hop 16, transformation 59 (fm3 log 125): question 75's second rule. `x$[-1]` is the item one before, zero before the start; a stream may be said from its own earlier items, never from itself now; an index forward is refused when compiled, and nothing is checked. What is kept is a field of the wiring for each earlier value read, fetched once a push statement, carried through its items and stored once. Lines shuffled give the same text. `or` and `and` are built (fm3 question 66), the parser told which words stand before `and` in a declared name. A running sum of three costs 183 with its reads. No existing store's text moved. `suite/zero/zeroic` 25 cases on all five paths.

### zero: every line of a stream processor holds for every item — `3c78024` · 2026-10-06

```
on (int d$) << doubled (int x$)
    d$ << x$ * 2
```

`suite/zero/zeroic/zeroic/zeroic.zero:26-27`. The parity pass, hop 16, transformation 58 (fm3 log 124): question 75's first rule. A body with no loop and no reader's word on its input is read the new way, each line holding for each item as it arrives (`src/zero/zeroic.rs`). For each wiring the front end writes a function of one item in zero's own tree, as it writes an edge's. Where nothing else reads the input it has no storage: a push is the call, and a string literal a loop whose count the cost tool bounds. Where something does, a sink in the walking form is written and wired. Three items through `doubled` cost 116 against the walking form's 241. No existing store's text moved; lex 601 and 590, hello 1 237 unchanged. `suite/zero/zeroic`, 14 cases on all five paths.

### zero: the lexer keeps its positions in `index`, and a static lex stands beside it — `6fecd22` · 2026-10-06

```
        index n = loop (index i = 1, index m = -1) yields m
            if (i >= count c$)
                if (ended c$)
                    m = i
                break
            if (kind of (peek c$ at (i)) == k)
                continue (i + 1, -1)
            m = i
            break
```

`suite/zero/lex/lex/lex.zero:36-44`. The parity pass, hop 15, transformation 57 (fm3 log 123). A token's `start` and `n`, and the lexer's own `start`, `i` and the length it carries, are `index`: three lines of the store, and not one `conv` is left in the lowered `lex`. lex `two_arrivals` 616 → 601, a turn 127 → 122, 1.20 times `lex-mod.ssa`'s 500; hello 1 237 and static 1 197 unchanged. `suite/zero/lex-static` is the same feature under a product marking it static on, with its own expected file: 590 against the monolith's 468, 1.26 times, so both comparisons are measured every hop. A negative literal handed to `peek`, `advance` or `behind` is refused at compile time naming the line; `x$[-1]` is not. The unsigned check that would catch a computed negative index is not built: measured at 9 on lex and needing a comparison the IR lacks, it is fm3 question 64. Full run green (`scratchpad/chain54.log`): zero 633, `probe test` 1015, `cargo test` 129.

### zero: `index` is a type a program can write — `5bc7210` · 2026-10-06

```
on (index k) = first above (int limit)
    int x$ = [10, 20, 30, 40]
    k = loop (index i = 0) yields i
        if (i >= count x$)
            break
        if (x$[i] > limit)
            break
        continue (i + 1)
```

`suite/zero/types/types/types.zero:188-195`, lowered by `src/zero/lower.rs` with no conversion in it. The parity pass, hop 15, transformation 56 (fm3 log 122). `index` is a builtin type of zero beside `int`. `count x$` and `position x$` give one, converted on the spot where an `int` place takes it, left alone where an operator does; `peek`, `advance`, `behind` and a subscript take one. `index` and `int` mix: an operator on the two computes in `int`, and either goes into a place of the other by an implied `conv`, as the words did inside themselves. No store changes; `types` gains `Span` and fifteen cases. lex 616, hello 1 237, static 1 197, unchanged; fourteen `conv`s leave five other stores and 42 of 649 functions cost less, a count compared with a literal no longer going through an `int`. A test runs `types` with `int` and `index` at different widths, which no path's own policy has. Full run green (`scratchpad/chain53.log`): zero 623, `cargo test` 128. (The entries for `1ffa005` and `eaa88f0` below said 592 and 1 365 functions; a script counted the cost tool's loop lines, and they are 266 and 649.)

### pool, heap: the pool and the heap count in `index` — `58bdb5e` · 2026-10-06

```
fn pool_take(p: ptr) -> ptr
    count: index = load p, 16
    head: index = load p, 24
    have: u1 = cmp.lt head, count
    check have
    q: ptr = pool_slot(p, head)
    next: index = load q
    store next, p, 24
    out: index = load p, 32
    out1: index = add out, 1
    store out1, p, 32
    f: ptr = pool_flag(p, head)
    store 1: u8, f
    ret q
```

`lib/pool.ssa:75-88`. The parity pass, hop 15, transformation 55, step (d), the last (fm3 log 121). The pool's slot size, count, slot numbers and free list, and the heap's sizes, levels, node numbers and bytes out, are `index`; a heap node's state and its seal count nothing and stay `i64`. Each number keeps its eight-byte word in the header. `lib/slice.ssa`'s `buffer_take` and `buffer_give` now count in `index` throughout, and the two conversions the step before left at the heap's door are gone. `suite/pool.ssa`, `suite/heap.ssa` and `matrix.ssa`'s `heap_buffers` hold their counts in one. `thread`, `fibre`, `core` and `gpu` count cores and stack bytes for the machine and are left. So every library that indexes a program's memory counts in `index`. lex 616, hello 1 237, static 1 197, unchanged; the suite passes natively at 16, 32 and 64. Full run green (`scratchpad/chain52.log`): `probe test` 1015, `cargo test` 127.

### zero: the front end's text counts in `index` — `eaa88f0` · 2026-10-06

```
fn __queue_token(hz: i64, cap: index) -> token$
    a: ptr = addr __arena
    r: ptr = arena_alloc(a, 64)
    sz: index = sizeof token
    bytes: index = mul sz, cap
    total: index = add bytes, 16
    vb: ptr = arena_alloc(a, total)
    buffer_init(vb, sz, cap)
    ring_queue(r, vb, hz, cap)
    s: token$ = stream r
    ret s
```

`suite/zero/lex.expected.ssa:448-458`, written by `src/zero/lower.rs`. The parity pass, hop 15, transformation 55, step (c) (fm3 log 121). Every count, length, capacity, position and byte size the front end writes is spelled `index` where it said `i64`; time stays `i64`, the rate above among it. A program's own integer handed to `peek`, `advance` or a subscript is converted to `index` (`as_index`). So a zero store is built at its path's own `index` again, and on wasm32 every count in a zero program is an `i32`: the zero suite passes 608/608 there. On a 64-bit path the nineteen stores' text differs by the spelling and two lines, and of 649 functions costed two moved by one: `__out_len`, the runner's, and `count doubled` in `platform`, which subscripts with an `int64`. `sink`'s IR body takes `count` as an `index`. lex 616, hello 1 237, static 1 197, unchanged. Full run green (`scratchpad/chain51.log`): `probe test` 1015, `cargo test` 127.

### ssa, slice, stream: a view's words and a stream's position are `index` — `1ffa005` · 2026-10-06

```
fn tick_of(s: any$, k: index) -> i64
    r: ptr = get s, ring
    step: i64 = load r, 40
    regular: u1 = cmp.gt step, 0
    t: i64 = if regular
        t0: i64 = load r, 48
        kt: i64 = conv k
        d: i64 = mul kt, step
        tr: i64 = add t0, d
        yield tr
```

`lib/stream.ssa:137-146`. The parity pass, hop 15, transformation 55, step (b) (fm3 log 121). The parser's two structures count in `index`: a view's count and stride, a stream's position, and all the arithmetic the parser writes for a subscript (`src/ssa.rs`); `lib/slice.ssa`, `lib/sample.ssa` and `lib/stream.ssa` follow. A place is not a time: ticks, rates, a step and `t0` stay `i64`, and where one becomes the other there is one `conv`, as above. Headers and views keep their eight-byte words; an index sits in a word's low bytes. On a 64-bit path nothing changed: lex 616, hello 1 237, static 1 197, and 266 functions of eight stores cost what they did. On wasm32 the suite's stream module has 59 `i32.wrap_i64` where it had 251 and is 1 724 bytes shorter. `suite/stream.ssa`, `slice.ssa` and `matrix.ssa` hold their lengths and counts in an `index`. The whole suite passes natively at 16, 32 and 64. Zero stores are still built at 64 until the front end's text changes. Full run green (`scratchpad/chain50.log`): `probe test` 1015, `cargo test` 127.

### arena: the arena counts in `index` — `02e9b15` · 2026-10-06

```
fn arena_alloc(a: ptr, n: index) -> ptr
    base: ptr = load a
    size: index = load a, 8
    used: index = load a, 16
    n15: index = add n, 15
    rounded: index = and n15, -16
    end: index = add used, rounded
    fits: u1 = cmp.le end, size
    check fits
    store end, a, 16
    p: ptr = ptradd base, used
    ret p
```

`lib/arena.ssa:35-46`. The parity pass, hop 15, transformation 55, step (a) (fm3 log 121): the first library to count in `index`. A size, a count of bytes used, a mark and the bytes asked for are `index`; each keeps its word of eight bytes at 8 and 16, and where `index` is 32 bits the library reads and writes the word's low four. On a 64-bit path it is the code it was. On wasm32 the function above is all `i32`: `i32.load` for `i64.load`, `i32.add`, `i32.le_s`, and no `i32.wrap_i64` before the address. `suite/arena.ssa` holds a mark and a count in an `index`. Until the front end's own text says `index`, a zero store is built with a 64-bit one on every path. lex 616, hello 1 237, static 1 197, unchanged. Full run green (`scratchpad/chain49.log`): `probe test` 1015, `cargo test` 127.

### ssa: the IR has `index`, the width of a count and a position in memory — `3bf53e8` · 2026-10-06

```
fn iraw_back(k: i64) -> i64
    p: ptr = fill_words()
    mid: ptr = ptradd p, 16
    one: index = conv k
    i: index = sub one, 3
    r: i64 = load mid, i, 8
    ret r
```
```
fn addresses(module: &Module, ty: Type) -> bool {
    matches!(ty, Type::I64 | Type::U64) || ty == module.index
}
```

`suite/index.ssa:162-168` and `src/ssa.rs:6654-6656`. The parity pass, hop 15, transformation 54 (fm3 log 120, question 73). Nothing in a program says how wide memory is: `index` is a signed integer the policy binds, 64 bits on the register machines and the GPU's path, 32 on wasm32, or `--index=16|32|64` and a product's `index:` line. It is not a tower name, so a function over it is a plain function and no template; the parser gives the policy's integer where it reads the name, and under 64 bits it and `i64` are one type, so old text stands. The verifier takes it as a `load`'s or `store`'s index and `ptradd`'s offset, and each emitter widens a narrow one: `sxtw` on arm64, nothing on riscv64, no `i32.wrap_i64` on wasm32, a sign-extension on the GPU. The instruction `index p, i` stands after `=`, the type after a colon; `ielem` has both on a line. `suite/index.ssa`, nineteen cases the same at every width; two tests, the suite at 16, 32 and 64 natively and the file on the other four paths at each. No store's IR changed: lex 616, hello 1 237, static 1 197. Full run green (`scratchpad/chain48.log`): `probe test` 1015, `cargo test` 127.

### stream: a queue's header is eight words again, its word at 16 where slot 0 is — `de38654` · 2026-10-06

```
    slots: ptr = ptradd vb, 16
    store slots, r, 16
```
```
    origin: i64 = load r, 8
    k: i64 = sub at, origin
    q: ptr(any) = load r, 16
    v: any = load q, k
```

`lib/stream.ssa:284-285`, in `ring_queue`, and `424-427`, in `peek_queue`. The parity pass, hop 15, transformation 53's second part (fm3 log 119). Hop fourteen kept the address of a queue's first slot in a ninth word of its header because one ring word was still applied to a queue, the ring's `push` filling a list's or a range's new stream, and it read the buffer through the word at 16. That fill is the queue's own since `23cf526`, so the slots' address goes where the buffer's was: a queue's header is 64 bytes like a ring's, the nine queue words load the word at 16, and the front end's `__queue_T` carves 64 with no literal of its own. No check touched and no number moved: lex 616, a turn 127, hello 1 237, static 1 197. Every store's text differs by `arena_alloc(a, 64)` for 72 and nothing else. `suite/stream.ssa`'s `queue_slot_zero` reads the word at 16 and gives `0, 9` on the library before. Full run green on five paths (`scratchpad/chain47.log`): zero 608, `probe test` 996, `cargo test` 125.

### zero: a list or a range of more than thirty-two items reads back right — `23cf526` · 2026-10-06

```
on (int s) = sum to (int n)
    s = [0 through n] + _
```
```
            Fill::Queue => format!("push_queue_open {}, {}", s, x),
            Fill::Ring => format!("push {}, {}", s, x),
            Fill::Timed(t) => format!("push {}, {}, {}", s, t, x),
```

`suite/zero/control/control/control.zero:19-20` and `src/zero/lower.rs:3182-3184`. The parity pass, hop 15, transformation 53 (fm3 log 119, question 62): a fault, found by agent 27. A list literal and a range make a new stream with their items present; where nothing in the store keeps history that stream is a queue, a plain run of slots, and its items were written by the ring's `push`, which stores each twice, half a buffer apart. From the thirty-third item the copies landed on each other and no check failed: `sum to (32)` gave 560 for 528, `sum to (100)` 7 550 for 5 050, a list of thirty-three literals 593 for 561, item 0 of a list of a hundred 51. `new_resident` now says what it made and the two fills take the queue's own push, in the open form, the stream being new. Nothing else applied a ring's word to a queue: searched on every all-queue store's emitted text. `control` gains seventeen cases, at 33, 64 and 100 items, fourteen of them wrong on the binary before. lex 616, hello 1 237, static 1 197, unchanged. `probe zero test` 608/608 native, `cargo test zero` 36.

### stream: a queue's header says where slot 0 is — `a6b1e19` · 2026-10-06

```
    slots: ptr = ptradd vb, 16
    store slots, r, 64
```
```
    origin: i64 = load r, 8
    k: i64 = sub at, origin
    q: ptr(any) = load r, 64
    v: any = load q, k
```

`lib/stream.ssa:287-288`, in `ring_queue`, and `422-425`, in `peek_queue`. The parity pass, hop 14, transformation 49 (fm3 log 115). Every queue word that touches a slot loaded the buffer's address from the ring's header, added sixteen to pass the buffer's own header and cast: 4 on the cost tool, and a queue never reads that header. A queue's header is now nine words, the ninth where slot 0 is, and the nine words load it as a typed pointer; no check is touched. It is a ninth word and not the word at 16 because a list literal and a range fill their new stream with the ring's own `push`, which reads the buffer through 16; that mixing reads back wrong from thirty-three items up (`sum to (32)` 560 for 528), which is fm3 question 62 and is left exactly as it was. The front end's `__queue_T` carves 72 bytes, the only change in any store's text. The peek the prompt asked for, told the turn's count and the header's fields, measured 15 at best and 3 after this, and is not built. `suite/stream.ssa` gains `queue_slot_zero` and `queue_views`. lex `two_arrivals` 649 → 616 (415 and 447 the oracles then, 468 and 500 now), a turn 136 → 127; hello 1 237, static 1 197. Full run green on five paths, `cargo test` 125.

### zero: a queue is freed by who reads it, not by how a parameter is spelled — `3c89f5b` · 2026-10-06

```
int x$
soak(x$)

on soak (int x$)
    loop
        if (count x$ == 0)
            break
        soaked = soaked + peek x$ at (0)
        advance x$ by (1)
```

`suite/zero/edges/tallied/tallied.zero:20-28`, with `>soaked from (100) → 5050`: a hundred items, one a statement, through a queue of 64. The parity pass, hop 13, transformation 47 (fm3 log 113); not cost, a fault agent 24 found in hop eleven. A queue gives its slots back after its one node has run only where no function reads its items by name, and "by name" was any `x$` in any function's body, so this sink reading its own parameter counted as a second reader of the feature's `x$`: the queue never freed and the sixty-fifth push failed the library's check, where the same sink spelled `soak (int s$)` ran for ever. `time_words_in` in `src/zero/lower.rs` now leaves out a name that is one of the function's own stream parameters. A local of that name still marks the stream, and a function that hands `x$` on names it at the call. The case is `a failed check` on the binaries before and passes here; only `edges`' emitted text changed. lex 649, hello 1 237, static 1 197. `probe zero test` 591/591, `cargo test zero` 36 on all five paths.

### zero: `count` asked once a turn — `2cf0ca7` · 2026-10-06

```
        _1: i64 = count c_2
        _2: int = conv _1
```
```
        n: int = loop(i: int = 1, m: int = -1)
            _11: u1 = cmp.ge i, _2
```

`suite/zero/lex.expected.ssa:209-210` and `228-229`, in `lex`. The parity pass, hop 13, transformation 46 (fm3 log 112). The lexer asked how much was waiting at the top of a turn and again on every pass of its inner loop, of the same reader; `_9` and `_10`, the second `count` and its `conv`, are gone and the loop compares against `_2`. `settle_counts` in `src/zero/lower.rs` drops a second `count R` in the finished text where the first is a statement of a block the second is inside and no line between could push: between is the lines from one to the other and the whole body of any loop the second is in and the first is not, and a line is harmless by a list of what is allowed, so a push, an `end`, a store or a word nobody listed gives up. lex calls `kind of` between its two askings, so a function of the store counts as harmless where its emitted body and all it calls are only such lines. lex's own `two arrivals → 3, 5` gives 3, 3 on a build that takes every line for harmless, and `suite/zero/streams`' new `counted round a push() → 23` gives 22. lex `two_arrivals` 679 → 649, 1.56× `lex-min`, 1.45× `lex-mod`, a turn 146 → 136 against 90; hello 1 237, static 1 197. `probe zero test` 589/589, `cargo test zero` 35 on all five paths.

### zero: a woken node keeps its position, not a whole reader — `bfb1a7e` · 2026-10-06

```
    _5: __ctx = load _this
    _6: i64 = get _5, __node2_c
    _7: u8$ = set _2, pos, _6
```
```
        _12: u8$ = lex(_11, _7, 0: i64)
        _13: i64 = get _12, pos
        _14: __ctx = load _this
        _15: __ctx = set _14, __node2_c, _13
        store _15, _this
```

`suite/zero/lex.expected.ssa:263-265` and `271-275`, in `arrive_first`. The parity pass, hop 13, transformation 45's third part (fm3 log 111). A node its pushers wake kept its reader, a stream value of four words, loaded and stored every run, where only the position moves and the hand-written lexer keeps one number. The field is now that position, an `i64`: `set` into the stream's own value, `_2`, which the push has in hand, and taken out of what the task gives back. It is sound only where the task gives back its parameter's own ring, so `ring_kept` in `src/zero/lower.rs` reads every definition of the task first: the parameter named only as the stream of `count`, `ended`, `position`, `latest`, `frame`, `peek .. at` and `advance .. by`, bound by nothing, handed to no task. Anything else keeps the whole reader, as `suite/zero/tasks`' new `hopping (s$)` does, which ends `s$ = far$`: `hopped() → 307` before and after, 301 on a build that forgets the rule. lex `two_arrivals` 697 → 679, 1.64× `lex-min`, 1.52× `lex-mod`; hello 1 237, static 1 197. `probe zero test` 588/588, `cargo test zero` 34 on all five paths.

### zero: the context reached in place, and once — `48ea2f3` · 2026-10-06

```
fn two_arrivals() -> (int, int)
    _this: ptr = addr __ctx_mem
    arrive_first()
    _1: __ctx = load _this
    _2: token$ = get _1, u
    _3: i64 = count _2
    a: int = conv _3
    arrive_again()
    _6: i64 = count _2
    b: int = conv _6
    ret a, b
```

`suite/zero/lex.expected.ssa:323-333`. The parity pass, hop 13, transformation 45 (fm3 log 110). Ash asked why state in the context should cost more than global data, and it need not: what cost was a call to `__get_u()` for every read and the same field fetched again. Now a function that touches the context forms its address once, its first line, and every read is a `load` there and a `get`, every write a `load`, `set` and `store`, which the IR dissolves to the one field; a gate reads its feature's switch and each dynamic ancestor's in line. And a field nothing in the store writes is fetched once a function: the second `count u$` above uses `_2`, across the call, because no function stores to `u` (a push changes the ring, not the field). `settle_context` in `src/zero/lower.rs` decides it in the finished text, from every store `field_put` recorded. `suite/zero/variables`' `seen round a bump() → 12` and `suite/zero/streams`' `counted round a skip() → 32` read a field something does write on both sides of the call; a build that reused them gives 11 and 33. lex `two_arrivals` 746 → 697, 1.68× `lex-min`, 1.56× `lex-mod`; hello 1 268 → 1 237; static 1 197. `probe zero test` 587/587, `cargo test zero` 34 on all five paths.

### zero: a short string literal lands in a queue an item at a time — `21e2edc` · 2026-10-06

```
on said fifteen()
    said$ << "fifteen letters"
    out$ << said$
```

`suite/zero/types/types/types.zero:66-68`, with `>said fifteen() → "fifteen letters"` and, beside it, `>said sixteen() → "sixteen letters!"`, which still takes the block push. The parity pass, hop 12, transformation 41 (fm3 log 109), built and proven by agent 25, parked on `zero-t41` when the forked child's death appeared in a case its prompt did not name, and landed by the orchestrator once `3e89209` had dealt with that. A string literal of fewer than sixteen bytes pushed into a queue went through `__str`, a view and the general `copy`, 59 and 10 a byte, where the hand-written lexer's loop is 10 and 7 a byte. It is now `push_queue_few` in `lib/stream.ssa`: the ended and room checks, then a plain loop from the literal's `data` to the slots. Sixteen is the machine's number, where a chunk first fits, not the tool's; a block of 480 costs what it did, 4 859 through the same wrapper. lex `two_arrivals` 844 → 746, 1.80× `lex-min`, 1.67× `lex-mod`; hello 1 268, static 1 197. Five paths and `cargo test` 122 (`scratchpad/chain41.log`).

### suite: a forked child says for itself that its JIT pages are to be run — `3e89209` · 2026-10-06

```
        #[cfg(target_os = "macos")]
        unsafe {
            pthread_jit_write_protect_np(1)
        };
```

`src/suite.rs:509-512`, the first thing the child does in `forked`. A case that must end in a failed check runs in a forked child, so the trap ends the child and not the suite. Under the whole of `cargo test`, and never alone, such a child sometimes died of signal 10 at its first instruction where a failed check is signal 5: five sightings on 6 October in four different tests (`slice.ssa`'s `mismatch` twice, `arena.ssa`'s `checks_pass`, one in `zero_suite_native`). Whether JIT pages may be written or run is a switch that belongs to the thread, and a child forked from a thread that was interrupted while it was writing code can be left with it the wrong way round. A standalone test of the mechanism, threads writing code, forking and the child calling it under load: 1 and 3 deaths in 12 000 forks without the line, none in 12 000 with it. Applied on that evidence at Ash's word, to be looked at again if it comes back. `scratchpad/chain.sh` also reruns any failed test alone three times now and writes the result under the failure.

### zero: a queue's push asks whether its stream has ended only where it could have — `65d3856` · 2026-10-06

```
on shut (int x$)
    end x$
```
```
on (int n) = pushed after shut()
    wide$ << 1
    shut(wide$)
    wide$ << 2
    n = count wide$
```

`suite/zero/checks/checks/checks.zero:37-38` and `46-50`, with `>pushed after shut() → check`: `wide$` is an `int64` stream. The parity pass, hop 12, transformation 40 (fm3 log 108). A queue's push loaded the ring's ended word and checked it, every time. `end` alone writes that word, and a ring is named only by its own element type or an abstract one above it, so a push through a type no `end` in the store reaches, by `fits` either way or by the IR's type, is `push_queue_open` in `lib/stream.ssa`: the room check and no other. The word is settled in the text once every body is lowered (`settle_pushes` in `src/zero/lower.rs`); a store with a `platform` body of its own keeps every check. lex `two_arrivals` 856 → 844, 2.03× `lex-min`, 1.89× `lex-mod`; hello 1 268, static 1 197. Five paths and `cargo test` 121.

### zero: a function's name is not a reading of a stream, and `in$` is not handed to what would write it — `9a4e7fc` · 2026-10-06

```
on (int s) = a part summed down from (int k)
    s = summed down from (k)
```

`suite/zero/edges/edges/edges.zero:49-50`, with `>a part summed down from (100) → 5050`. The parity pass, hop 11, transformation 38 (fm3 log 106): two things hop 10 found and left. `stream_uses` in `src/zero/lower.rs` took any bare word of a phrase that matched a stream's name as a reading of it, so calling a function with `part` in its name gave `part$` a queue, silently, and a hundred items in one statement failed its check. A phrase that is a call of a store's function now mentions its arguments alone, by the lowering's own `find_methods`; on `9f06954` this case and `summed down from (100)` fail in every context. And question 35's gap is closed: `in$` handed to a function that pushes into or ends the stream it is given, itself or through another, as a call or a wiring, is refused where it is handed, naming the function (`input_handed`, over the scheduler's `Pushes`). No number moves: lex 856, hello 1 268, static 1 197. zero 577 → 580 runs native, 31 zero unit tests.

### stream: a block pushed into a queue reaches its slots without a view of the buffer — `9f06954` · 2026-10-06

```
    check fits
    vb: ptr = load r, 16
    raw: ptr = ptradd vb, 16
    q: ptr(any) = cast raw
    p: ptr(any) = index q, slot
    d: any[] = pack p, n, 1
    copy d, block
```

`lib/stream.ssa:320-326`, in `push_queue(s, block)`. The parity pass, hop 11, transformation 37 (fm3 log 105). lex's two arrivals are block pushes, 77 + 10 n on the tool, and 26 of the 77 was `ring_values(r)` and `view vals, slot, n`: a view of the whole buffer, checked for what `fits` had proved two lines above. The block now reaches its slots as an item's push does, through the buffer's typed pointer, 8. Ended and room are still checked. `suite/stream.ssa` gains `queue_block_to_the_brim -> 79, 8` and `queue_block_full -> check`, the same on the old library. The log has what `copy` costs: 42 + 10 n, within 8 of an arm64 for these blocks, and over for one of sixteen bytes or more. lex's `two_arrivals` **892 → 856**, 2.06× its oracle; hello 1 268, static 1 197. Everything ran: zero 577 on four paths and 558 on air, `probe test` 986, 977, 986, 986, 953, `cargo test` 120.

### zero: a woken node's reader is read and written in place — `88332b6` · 2026-10-06

```
    _5: ptr = addr __ctx_mem
    _6: __ctx = load _5
    _7: u8$ = get _6, __node2_c
    _8: u1 = __on_lex()
    if _8
        _9: token$ = __get_u()
        _10: u8$ = lex(_9, _7, 0: i64)
        _11: __ctx = load _5
        _12: __ctx = set _11, __node2_c, _10
        store _12, _5
```

`suite/zero/lex.expected.ssa:321-330`, in `arrive_first`. The parity pass, hop 11, transformation 36 (fm3 log 104). After 35 a run of lex's node was 186, and 14 of the 35 round the lexer was its saved reader fetched and stored through two accessors of the context, 7 each. Costed through wrappers, the field read in line is 5, written 5, and with the address taken once the two are 9. So `wake` in `src/zero/lower.rs` takes the context's address once and loads and stores the reader there; the write loads again, the task's call standing between. Only the node's own reader, which no zero program names; the state stays in the context, and the gate and the output keep their calls. lex's `two_arrivals` **907 → 892** on 418 → 433 lines, 2.15× its oracle; `timed`'s `count down` 3 102 → 3 052; hello 1 268, static 1 197. zero 577 runs native, 31 zero unit tests.

### zero: a node that only plain functions wake is called where they push — `30d07b7` · 2026-10-06

```
    push_queue(_1, _4)
    _5: u8$ = __get___node2_c()
    _6: u1 = __on_lex()
    if _6
        _7: token$ = __get_u()
        _8: u8$ = lex(_7, _5, 0: i64)
        __set___node2_c(_8)
        free_queue(_8)
```

`suite/zero/lex.expected.ssa:333-340`, in `arrive_first`. The parity pass, hop 11, transformation 35 (fm3 log 103). That push called `__run_src()`: the scheduler's guard, then a node that read `fin`, `received`, `seen` and `ended` to learn it was due, 82 round a lexer of 150. `Beat::woken` in `src/zero/lower.rs` proves `src$` is pushed into and ended only by plain functions nothing the scheduler runs can reach, so the push is the reason and nothing is asked: the task is called in line under its feature's gate, and the node keeps its reader alone. A statement that may push nothing wakes under `received` before and after; an `end` under whether the stream had ended. The guard is gone from every store but `platform`. `tasks`' `nothing pushed`, `ended twice` and `a batch` give what they gave on `31874aa`. lex's `two_arrivals` **1 039 → 907** on 440 → 418 lines, 2.19× its oracle; `timed`'s `count down` 3 562 → 3 102; hello 1 268, static 1 197. zero 569 → 577 runs native, 31 zero unit tests.

### zero: a node asks how much its input holds once a run — `31874aa` · 2026-10-06

```
        _9: u8$ = lex(_8, _1, 0: i64)
        __set___node2_c(_9)
        free_queue(_9)
        __set___node2_c_seen(_2)
        __set___node2_fin(_5)
```

`suite/zero/lex.expected.ssa:290-294`. The parity pass, hop 10, transformation 34, chosen from a table (fm3 log 102): lex's case is 1 063 against 415, and the 648 between is the bookkeeping round the lexer's three runs (267), the lexer (180) and two block pushes (200), no candidate worth a tenth of it. A node read `received` and `ended` of its input to see whether it was due, and read both again after its task to store what it had seen. Under the static schedule no node can push into or end what it reads, which is what acyclic is computed from, so `emit_node` in `src/zero/lower.rs` stores the first reading, `_2` and `_5`. A node **229 → 221**; lex's `two_arrivals` **1 063 → 1 039** on 444 → 440 lines, 2.50× its oracle; `timed`'s `count down` 3 722 → 3 562; hello 1 268, static 1 197. zero 569 runs native.

### zero: a program never writes `in$` — `7f45a72` · 2026-10-06

```
on arrive first()
    src$ << "let x = 4"

on arrive again()
    src$ << "2;\n"
    end src$
```

`suite/zero/lex/lex/lex.zero:49-54`, beside `token t$ = lex(in$)`, `char src$` and `token u$ = lex(src$)` at lines 4 to 6. The parity pass, hop 10, transformation 33 (fm3 log 101; question 35, ruled 5 October). The input device is the mirror of the output device, read and never written, so `src/zero/lower.rs` refuses a program's push into `in$`, an edge into it and `end in$`, naming the device and saying input comes from the platform, a case's `with in "text"`. The `lex` store was the one that did it; its task is now wired a second time to a stream of its own and the experiment's cases give the same results over `u$`, with `lexed() with in "let x = 4" → 3` keeping the device proven. Its comparable case against `lex-min.ssa` is now `two_arrivals` alone, both arrivals inside it: **1 063 against 415**, 2.56×, on 444 lines; it was 1 143 with the runner's bytes and the start. zero 567 → 569 runs native, 25 zero unit tests.

### zero: a stream declared and named nowhere else has no storage — `9cac9ef` · 2026-10-06

```
int n$
int beat$ at (2 hz)
int spare$
```

`suite/zero/unwired/unwired/unwired.zero:1-3`. The parity pass, hop 10, transformation 32 (fm3 log 100; question 57, which ash ruled as recommended). `spare$` is declared and named by nothing: no push, no read, no wiring, no case. It kept a queue of 64 that nothing could fill, `_2: int$ = __queue_int(1000000, 64)` in `__zero_reset`, a field in `__ctx` and two accessors. `settle_bare` in `src/zero/lower.rs` now gives it no storage, the third class beside an edge's source and an unwired push's target, and does not refuse it, since a stream may be declared ahead of the feature that will use it; a `char` stream is included, nothing being able to pass it to a method. The reset now makes `in$` alone. No other store's IR changes; hello 1 268, static 1 197, lex 1 143. zero 567 runs native, 24 zero unit tests.

### zero: the clock of code — `e48a653` · 2026-10-06

```
on drum (int k)
    quick$ << 0
    loop (int i = 1) while (i <= k)
        beat$ << i
        continue (i + 1)
    out$ << "end" << "\n"
```

`suite/zero/edges/edges/edges.zero:28-33`, `quick$` at `5 hz` and `beat$` at `2 hz`. The parity pass, hop 10, transformation 31 (fm3 log 99; question 56, ash: code runs because a clock ticked). Hop 9 rounded the clock up before every push statement into a rated stream, 17 SSA, which hello paid to learn it was at 0 s. `Beat` in `src/zero/lower.rs` now knows, at each point of a plain function, what the clock is a whole multiple of: 0 where a case started it, a period after a rated push, the gcd where paths meet and over a function's callers; a push whose period divides it is not aligned. Tasks, edges, methods and platform functions are asked only whether they can move the clock. A loop whose first statement is such a push aligns once before it, under its own `while`, so `drum (0)` still writes `end` at 200 ms. No output changes. hello's `run` **1 285 → 1 268** on 261 → 255 lines against 1 286; static **1 214 → 1 197** against 1 247; lex 1 143. zero 559 → 567 runs native, 24 zero unit tests.

### zero: a push lands on the stream's beat — `f583089` · 2026-10-06

```
on turns()
    beat$ << 1
    quick$ << 2
    beat$ << 3
    out$ << "end" << "\n"
```

`suite/zero/edges/edges/edges.zero:22-26`, `beat$` at `2 hz` and `quick$` at `5 hz`. The parity pass, hop 9, transformation 30 (fm3 log 98; question 52 as ash refined it, the beat belongs to the stream). A stream with a rate has slots one period apart from 0 s, and an item pushed into it lands in the next slot at or after the pusher's now. Hop 8 pushed wherever now stood, so this wrote at 0 s, 500 ms and 700 ms; it now writes `1` at 0 s, `2` at 600 ms, `3` at 1 s and `end` at 1.5 s. After an item lands now is the next slot's start, so `align` in `src/zero/lower.rs` is emitted once a push statement, the clock rounded up to a multiple of the period by `add`, `rem`, `sub` and a `__wait`, 17 SSA, and not where the statement before pushed into the same stream. The period is `store::period`, the one a step adds. `suite/zero/timed`'s `late` is the stored shape. Against `69c1447`'s binary the four new runs fail. hello's `run` **1 268 → 1 285** on 255 → 261 lines against 1 246 (1.03×), one alignment; static **1 197 → 1 214** against 1 207 (1.01×); lex 1 143. zero 555 → 559 runs native, 23 zero unit tests.

### zero: the rate in a timed case — `69c1447` · 2026-10-06

```
>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "hello world\ngoodbye" at 10 s
```

`suite/zero/hello/bye/bye.md:24`. The parity pass, hop 9, transformation 29 (fm3 log 97, question 53, which ash ruled C). A piece of a timed result may be given `at` a rate in place of a time: it is then its lines, one a step, from 0 s or `from` a time, `"10\n9\n" at 1 hz from 3.5 s`. `parse_timed` in `src/zero/store.rs` expands it into the pieces the listed form gives, so nothing downstream changes, and a step is `store::period(hz)`, the one function `step` in `src/zero/lower.rs` now calls too, so a case and the program cannot differ by a rounding. What a failure prints is folded: a run of three or more single lines a whole rate's period apart is spelt in the rate form and parses back to itself, so `run "run()" --fast` on hello prints the line above, 82 characters where the listed form is 182. static's and `timed`'s cases follow; `edges`' `beats()` stays listed so both are run. A timed input, `with in "k" at 3.5 s`, is refused by name until restart. No IR changes: hello 1 268, static 1 197, lex 1 143. zero 555/555 native, 22 zero unit tests.

### zero: a stream nothing reads or wires has no storage — `e42604f` · 2026-10-06

```
int n$
int beat$ at (2 hz)

on count down from (int k)
    n$ << [k through 1]
    out$ << "liftoff" << "\n"
```

`suite/zero/unwired/unwired/unwired.zero:1-6`, with `shown: static off` in its `product.md`. The parity pass, hop 9, transformation 28 (fm3 log 96, question 54). The feature that wires `n$` to the output is left out by the product, so nothing in the program reads or wires the stream; it still got a queue of 64 and `count down from (80)` failed a check, where the same feature switched off at run time writes `liftoff`. ash ruled the two are one program. `settle_bare` in `src/zero/lower.rs` now gives no storage to any stream that is only pushed into, a push being its items and, at a rate, a step. And the safeguard: `store::read` keeps the features a product leaves out (`Store.left_out`), and where no feature of the store, in or out, reads or wires such a stream the store is refused as a mistyped name. Against `6a07bff`'s binary the new store fails 1 of 2; now 2/2. No other store's IR changes by a byte: hello 1 268, static 1 197, lex 1 143. zero 553 → 555 runs native.

### zero: the mark, by position — `6a07bff` · 2026-10-06

```
fn __wait(t: i64)
    p: ptr = addr __clock
    c: i64 = load p
    m: i64 = max(c, t)
    store m, p
    q: ptr = addr __out_n
    n: i64 = load q
    a: ptr = addr __out_t
    store m, a, n, 8
    ret
```

`suite/zero/hello.expected.ssa:92-101`. The parity pass, hop 9, transformation 27 (fm3 log 95, question 55). A time mark was two words appended to a list with a count, 12 SSA a wait, and it was all hello had left over its oracle; ash: "a stamp needn't cost that much". Now the time is stored at the place in the output where it begins to apply: `data __out_t`, a word for each byte of the capture and one for its end, indexed by how many bytes have been written. A second wait before more is written overwrites the same word, and a zero says the clock did not move there. `__out_marks()` is gone; the runner reads `__out_mark(i)` for each byte it read and one more on every path (`src/suite.rs`, `src/driver.js`), `pieces` in `src/zero/run.rs` cuts where a word is not zero, and `__zero_reset` clears the table as far as the last case wrote, a loop declared `bound 65536`. Measured: **a mark is exactly 4**; `__wait` 17 → 9; hello's `run` **1 348 → 1 268** on 258 → 255 lines against `hello-mod.ssa`'s 1 246 (1.02×); static **1 277 → 1 197** on 181 → 178 against `hello-min.ssa`'s 1 207 (0.99×); lex 1 143, unchanged to the byte. All five paths (`scratchpad/chain30.log`): zero 553/553 native, wasm, riscv, arm-qemu, 534/534 and 19 skipped on air; `probe test` 984, 975, 984, 984, 952; `cargo test` 114 passed, 2 ignored, `zero_suite_native` having failed once under the chain's load and passed alone and in four whole runs after.

### zero: the stores' prose after "the beat belongs to the stream" — `4258d20` · 2026-10-06

Prose only. While hop 8 ran, ash refined question 52 (fm3 `time.md`, "a stream has a beat"): a stream with a rate has a phase and a rate, an item pushed into it lands in its next slot, and the phase changes only on purpose. That replaces "a rate belongs to the activity that pushes", which `suite/zero/hello` and `static`'s `countdown.md` and `suite/zero/timed/timed/timed.md` repeated; they now say only what happens, a number written when it is pushed and then its second passing. What is built is unchanged and is not yet the beat: `step` in `src/zero/lower.rs` moves the clock on from the pusher's now and looks for no slot. Every store pushes each rated stream from one function starting at 0 s, so every case stands under the new ruling as written; fm3 log 94 has the program where the two differ and what landing on the beat would cost hello, 1 348 to about 1 600. zero 553/553 native.

### zero: a task between a rated stream and the output keeps the rate — `2b1623d` · 2026-10-06

```
int i$ at (1 hz)
int d$ = doubled(i$)
out$ << d$ << "\n"

on count down()
    i$ << [10 through 1]
```

The parity pass, hop 8, transformation 26 (fm3 log 93, questions 39 and 52). ash ruled that whatever a consumer does with an item it does at that item's time, a task as much as an edge; with `doubled` between hello's stream and the output the ten numbers were all written at 0 s, the function pushing its whole statement before the scheduler ran. A plain function's push into a stored stream declared with a rate is now, per item, the push, the trigger `__run_i()` and a step of the rate (`emit_push` and `paced` in `src/zero/lower.rs`), and the edge's own wait is gone for such a stream. `suite/zero/timed` (above) is new, its cases `"20\n" at 0 s, "18\n" at 1 s` down to `"2\n" at 9 s` and `"liftoff" at 10 s`; both fail on the commit before with every piece at 0 s. hello **1 348** and static **1 277**, their IR byte for byte the same; the price where it applies is a scheduler entry an item, `timed`'s `count down` 603 → 3 802. A rated task wired at feature scope is not yet paced. Front end only: zero 553/553 native.

### zero: a stream no word reads has no storage — `efce63e` · 2026-10-06

```
fn count_down()
    _1: u1 = __on_countdown()
    loop(_2: int = 10)
        _3: u1 = cmp.ge _2, 1
        if _3
        else
            break
        if _1
            __edge1(_2)
        _4: ptr = addr __clock
        _5: i64 = load _4
        _6: i64 = add _5, 1000000
        __wait(_6)
        _7: int = sub _2, 1
        continue _7
    ret
fn __edge1(__item: int)
    __out__int(__item)
    _1: u8 = const 10
    __out_ch(_1)
    ret
```

The parity pass, hop 8, transformation 25 (fm3 log 92, questions 50, 51, 52). ash ruled that storage depends on the words applied to a stream: a history or time word makes a ring, a reading word a queue, and **a stream no word reads has none**. hello's `i$` is one, and above is `suite/zero/hello.expected.ssa`'s countdown now: `settle_bare` in `src/zero/lower.rs` finds the streams that are only pushed into and wired by edges, an edge out of one is a function of one item, and a push calls each edge under its feature's gate, read once a statement. No queue, no node, no scheduler. If the stream has a rate a step then passes, so ten numbers at `1 hz` take ten seconds and `hello world` is stamped 10 s. A consumer that is off holds nothing: a node whose feature is off moves its reader past what arrives. `suite/zero/edges` is new; 11 of its 15 runs fail on the commit before. hello's `run` **1 742 → 1 348** on 373 → 258 lines against its oracle's 1 246 (1 228 without the time marks the oracle does not record); static **1 680 → 1 277** against 1 207; lex unchanged at 1 143. Front end only: zero 551/551 native.

### zero: a case asserts on time — `024f51e` · 2026-10-06

```
>run() → "10\n" at 0 s, "9\n" at 1 s, "8\n" at 2 s, "7\n" at 3 s, "6\n" at 4 s, "5\n" at 5 s, "4\n" at 6 s, "3\n" at 7 s, "2\n" at 8 s, "1\nhello world\ngoodbye" at 9 s
```

The parity pass, hop 8, transformation 24 (fm3 log 91, questions 52 and 53). ash ruled that time in a program is a label on an item and never a delay, so timed code is tested at once by checking stamps; until now no case could say when anything was written. A result may now be every piece of the output in order, each with its time (above, `suite/zero/hello/bye/bye.md`): a piece is everything written while the clock stood still, two pieces at one time are one, and the pieces joined are the whole output. The clock moves only in `__wait`, so the virtual clock's `__wait` in `src/zero/lower.rs` leaves a mark each time, the bytes written so far and the time reached; `suite::Call.times` has each of the five paths read the marks after the text, and `judge` in `src/zero/run.rs` cuts the text at them and prints a failure in the case's own spelling, so the line can be pasted back. A mark costs **12 SSA** a wait: hello's `run` **1 622 → 1 742**, static **1 560 → 1 680**; lex unchanged to the byte, a store that never waits keeping no marks. Green on all five paths, zero 536/536, `probe test` 984, `cargo test` 114.

### zero: a feature watches what is written, by redefining `<<` — `6525a2e` · 2026-09-10

```
on (char o$) << (int x)
    written = written + 1
    existing o$ << x
```

The parity pass, hop 22's second landing (fm3 log 90, question 46). ash ruled that a device is written and never read, and that a feature which wants to monitor what goes into one extends the `<<` method for the item it wants to see. `suite/zero/platform` gains `watch`, that feature (above), with `>counted() → 3`, `>counted() with watch off → 0` and the digits still written either way. Three things were in the way, all bugs in the chain of features rather than in the ruling: `declare` refused to redefine any operator, and a `<<` method is a method like any other; `existing o$ << x` did not parse, `existing` taking a phrase and a `<<` method's name being an operator, so `Stmt::Push` gains an `existing` flag; and the device copy of the method — a `<<` over a `char` stream is lowered twice (log 87) — was emitted under one name with no link below it, so it now chains under its own name, `__out__int__platform` and `__out__int__watch` the bodies and `__out__int` the link. A redefinition applies to both copies. Front-end only and nothing hello reaches: hello's `run` **1 622** before and after, its IR byte for byte the same. zero 536/536 runs over 301 cases on the four CPU paths and 517/517 on air, `probe test` 984/984, cargo test 112.

### stream: a stream is a queue unless a history word names it — `efc9d16` · 2026-09-10

```
fn push_queue(s: any$, v: any)
    ...
    slot: i64 = sub pushed, origin
    room: u1 = cmp.lt slot, cap
    check room
    store v, q, slot
```

The parity pass, hop 22 (fm3 log 89, question 47). ash ruled that the choice of underlying data structure depends on what is needed: a stream is a **queue** — it holds an item only until its reader has passed it — unless a history word or a time word in the store names it (`latest`, `behind`, `x$ at (t)`, `x$ from (a) to (b)`, `time of x$`), in which case it keeps the ring. The words in the text decide, as they decide timing; nothing is inferred from who reads a stream. The queue built first was the ring with a fullness check, and costing it before believing it gave 26 against the ring's 28 — the mirror and the seam are most of the push — so the queue drops the ring's arithmetic instead: the header's spare word holds `origin`, the index of the item in slot 0, and item k is at slot `k - origin`, a plain run with no mirror, no wrap and no seam. Push **22**, block push **88** against 248, `peek` **16** against 23. When a reader has passed everything, `free_queue` moves the origin and every slot comes back; the front end emits that after a node run where the node is the queue's only reader. `push_plain` and hop 17's system stream collapse into it. hello's `run` **1 683 → 1 622** against the oracle's 1 165, lex's case 1 195 → **1 143** against 415 and a turn of `lex` 174 → 150. `probe test` 984/984, zero 482/482 on the four CPU paths and 465/465 on air, cargo test 112.

### stream: a ring may hold a struct — `0de20dd` · 2026-09-10

```
fn struct_ring() -> (i64, i64)
    s: tok$ = fresh_toks()
    three_toks(s)
    n: i64 = count(s)
    x: tok = peek s, 1
```

The parity pass, hop 21 (fm3 log 88, question 43). ash ruled that array-of-structs is the default: a stream of structs is **one ring whose item is the struct** — one buffer of `sizeof T` stride, one header, one position — so a token's push is one push, not one per field. `lib/stream.ssa` was written over `number$`, and `number` ranges over the number tower alone, so the IR's tower gains **`any`**, above `number`, whose family is every type; every word that reads items one at a time is written over it. `sample`'s linear rule and `lerp` weigh two items and round between them, which two structs have no midpoint for, so they keep `number` and refuse a struct ring by name. The zero front end stops splitting a struct: one `pack` and one `__push` for a token, one `peek` for a read, and the generated `__s_T` struct of parallel rings is gone; `frame` and `behind` work on a stream of structs now, which they could not before. lex's `lex` **228 → 174** SSA against the oracle's 90, its case 1 397 → **1 195** against 415 on 403 → 334 lines. `suite/stream.ssa` gains `struct_ring` and `struct_frame`: `probe test` 982/982, zero 482/482 on the four CPU paths and 465/465 on air, cargo test 112.

### zero: `char` is a type of its own, and `out$` is a device — `96f1595` · 2026-09-10

```
fn __out_ch(c: u8)
    q: ptr = addr __out_n
    n: i64 = load q
    p: ptr = addr __out
    store c, p, n, 1
```

The parity pass, hop 20 (fm3 log 87, questions 44 and 45). Two of ash's rulings, which turned out to be one line of code. `uint8$ << 42` is ambiguous between a serialisation and human-readable text, so zero gains **`char`**, distinct from `uint8` and a `u8` in the IR: `string` is `char$`, a string literal is `char$`, and a char is compared and converted and never computed with — `c <= 32` is the lexer's line, `int(c)`, `uint8(c)`, `char(48 + d)` cross between. And **`out$` is not a buffer**: it looks like a stream, but pushing to it writes a character to the place and stores nothing, so a push is the platform's write (above, with `__out_block` for a block; an `ir` body now, a body per kind of place in milestone 1), with no ring, no capacity, no context field, no `write(out$)` sink and no scheduler node. Under the runner the place is the test capture that `__out_len` and `__out_byte` read. Inside `on (char o$) << (int x)` the front end cannot see which stream `o$` is, so each such method is lowered twice — over a stream, and over the device where every push into `o$` is the write — and the call site chooses. hello's `run` **2 597 → 1 683** SSA on 428 → 352 lines against the oracle's 1 165 on 174, 2.2× to **1.44×**; static 2 511 → 1 621; lex's case 1 475 → 1 397. All five paths, `probe test` 980/980, zero 481/481 on the four CPU paths and 464/464 on air, cargo test 111.

### stream: the ring's base moves to the read side — `41a5aff` · 2026-09-10

```
fn ring_base(r: ptr) -> i64
    ...
    over: i64 = sub pushed, half
    b: i64 = max(over, 0: i64)
```

The parity pass, hop 19 (fm3 log 86). `lib/stream.ssa`'s ring header held `base`, the index of the first resident item, and every push computed and stored it — but it is `max(pushed - half, 0)`, a function of the count in the header and the buffer's slots, so the readers compute it. `frame`, `unread`, `peek` and `latest` load `pushed`, the buffer, its capacity and `half` already, so each gains a subtract and a `max` and loses a load; the sampling and history words — `behind`, `last`, `before`, `sample`, `window`, `tick_of`'s irregular arm — call `ring_base` (above). `latest`'s check and the ticked push's ordering check become `pushed > 0`, which is what they meant, since the newest item's predecessor is always resident; the header's second slot is spare. hello's `run` 2 715 → 2 597 against the oracle's 1 165, static's 2 629 → 2 511 against 1 126, lex's case 1 499 → 1 475 against 415 and `lex` 236 → 228. Library only, so no store's emitted IR changes: all five paths, `probe test` 980/980 native, zero 477/477 on the four CPU paths and 460/460 on air, cargo test 111.

### zero: `position` is the index alone, `time of` asks the time — `b0ba7ae` · 2026-09-10

```
        int start = position c$
```

The parity pass, hop 18 (fm3 log 85, question 42). Ash ruled that a stream is timed only where the text mentions time, so `position x$` gives one int, the index of the next unread item, and asks no time — `k: i64 = get reader, pos`, no library call and no tick — while the new `time of x$` gives that item's tick and is the time word that times the stream. `src/zero/lower.rs`'s `stream_word` gains both arms, `lower_multi` refuses the two-result form by name, `time_words_in` trades `position` for `time of`, and hop 15's inspection of whether a bound tick was read goes with `dead_ticks`, `reads_name` and `reads_in`: the rule a reader can see replaces the rule the compiler inferred. `suite/zero/lex` writes the line above and its IR does not change, so lex's cost does not move — 236 for a turn, 1 499 for its case against the oracle's 415; `streams`, `tasks` and `clock` split their line into `position` and `time of` and keep their results, so the timed side stays covered, clock's IR gaining a getter and a call (286 → 288 lines). hello and static untouched at 2 715 and 2 629. A front-end change: zero 477/477 native, cargo test 111.

### zero: a system stream is pushed with a store and a count — `dfef61c` · 2026-09-10

```
fn push_plain(s: number$, v: number)
    ...
    room: u1 = cmp.lt pushed, half
    check room
    store v, q, pushed
```

The parity pass, hop 17 (fm3 log 84, question 41). Ash ruled that a system stream's only reader is the platform, so it needs no ring: `lib/stream.ssa` gains `push_plain` for a value and for a block (above) — the ended check, the capacity checked as a limit rather than a window, the store at slot k and the count — where the mirrored push computes a slot, a twin, two stores and a base, and the block push copies three times to keep the seam whole. The ring is an ordinary regular one, so `push` still works on it and no reader changes; `src/zero/lower.rs`'s `system()` names the streams the front end can see are the platform's own `out$` and `in$`, and `emit_push`, `push_view` and the prelude's `__in_ch` call `push_plain` there. A general stream keeps the full ring. hello's `run` 3 487 → 2 715 against the oracle's 1 165, static's 3 401 → 2 629 against 1 126, lex's case 1 813 → 1 499 against 415 (`__in_ch` 41 → 29, `two_arrivals` 1 036 → 830); `lex` itself unchanged at 236, its token stream being general; 428, 341 and 478 lines, unchanged. A library change: all five paths, `probe test` 980/980 native (three new cases in `suite/stream.ssa`), zero 477/477 on the four CPU paths and 460/460 on air, cargo test 111.

### zero: no sleep in a store with no rated wiring — `e569a59` · 2026-09-10

```
            __push(_13, _14)
            c_4: u8$ = advance(c_2, 1)
```

The parity pass, hop 16 (fm3 log 83). A task's body stepped its clock with `__sleep(__hz)` after every push into its own output, `__hz` being 0 wherever the wiring had no rate; `src/zero/lower.rs` now looks for an `at (n hz)` tail on every wiring in the store before lowering (`any_rated_wiring`, `is_rated_wiring`, `wired_at_a_rate`) and `emit_push` writes no sleep where there is none (above, `suite/zero/lex.expected.ssa`: the token's pushes then the advance, no sleep between). A store with a rated wiring keeps every sleep, since a nested task runs at its caller's `__hz`. lex's comparable case 1 861 → 1 813 against the oracle's 415, `lex` 252 → 236, 495 → 478 lines; clock and hello unchanged. A front-end change: zero 477/477 native, cargo test zero:: 18.

### zero: a dead tick asks no time — `394f9e7` · 2026-09-10

```
        _5: i64 = get c_2, pos
```

The parity pass, hop 15 (fm3 log 82, question 42). `src/zero/lower.rs`'s `time_words` no longer takes `T i, T t = position x$` as a time word when `t` is read nowhere in the function (`reads_name`, `dead_ticks`), and `lower_multi` emits the reader's index as a `get` (above, `suite/zero/lex.expected.ssa`) with no tick and no library call. lex's `int start, int tick = position c$` had timed `in$`, and through the parameter the whole store, for a tick it never used; `in$` is a plain ring now. lex's comparable case 2 700 → 1 861 on the tool against the hand oracle's 415, `lex` 377 → 252, 534 → 495 lines; hello and static unchanged at 3 487 and 3 401. A front-end change: zero 477/477 native, cargo test zero:: 18.

### stream: a block push is three copies — `a568218` · 2026-09-10

```
    slot: i64 = rem pushed, half
    d1: number[] = view vals, slot, n
    copy d1, block
```

The parity pass, hop 14 (fm3 log 80). `lib/stream.ssa`'s block pushes land the block whole from its slot (above) — under half from a slot under half, it never reaches the end, and past the seam it writes the twin positions — and split only the twin copy where it reaches the end: three copies where there were four, the same slots with the same items, `suite/stream.ssa` unchanged at 29. hello 3 767 → 3 487 on the tool, static 3 681 → 3 401. Every path: zero 477/477 on the CPU paths and 460 + 17 skipped on air, probe test 977/977, 968 + 9, 977, 977, 946 + 31; cargo test 111.

### zero: the arrival bound on an edge's loop — `fc3ac20` · 2026-09-10

```
    ; the most items one event pushes into i$: 10 (log 79)
    loop(_4: i64 = 0) bound 10
```

The parity pass, hop 13 (fm3 log 79). `arrivals` in `src/zero/lower.rs` counts what each push statement from a plain function pushes into a feature-scope stream when every item is countable, and an edge's loop over that stream is emitted with the largest as its `bound` (above, `suite/zero/hello.expected.ssa`): under the static schedule the edge runs after every such statement, so it never sees more. The tool is now honest for the countdown: hello 2 318 → 3 767, static 2 232 → 3 681, the edge's loop `x10 (declared)` where it was counted once. A front-end change: zero 477/477 native, cargo test zero:: 18.

---

### zero: the scheduler is a static schedule where the node graph is acyclic — `5499381` · 2026-09-10

```
fn __run_out()
    p: ptr = addr __running
    busy: i64 = load p
    idle: u1 = cmp.eq busy, 0
    if idle
        store 1: i64, p
        r1: u1 = __node1()
        store 0: i64, p
    ret
```

The parity pass, hop 12 (fm3 log 78, question 40). `src/zero/lower.rs` works out what each node may push into (`Pushes`, an over-approximation chased through the store's functions), orders the nodes with every producer before its consumers, and emits `__run()` as one ordered pass and `__run_<s>()` per stream a node reads (above, `suite/zero/hello.expected.ssa`), run after a push from a plain function; the loop stays for a cyclic graph, and no store in the suite has one. hello 3 919 → 2 318 on the tool, static 3 729 → 2 232, the loop's 712 a site becoming 88 at hello's. A front-end change: zero 477/477 native and wasm, cargo test zero:: 18.

### zero: the real clock on `run`, an edge taking each item at its tick, and the output laid out case line first — `beaec74` · 2026-09-10

```
        _6: i64 = add _3, _4
        _7: i64 = mul _6, 1000000
        __wait(_7)
```

Ash's review asks (fm3 log 77, question 39). `probe zero <store> run "run()"` now prints the case line, then the program's output as it lands — the JIT call on a thread of the forked child, the child reading the ring beside it (`src/suite.rs`, `watched`) — then `→` and what the case gave; hello's countdown takes 9.2 s, a number a second, and `--fast` 0.15 s. The one function that differs per clock is `__wait(t)`: a jump of the virtual clock, or a spin on `cntpct_el0` as a `platform arm64` body, chosen by the product's `clock:` line, which `run` sets to real and `test` to virtual (`src/zero/store.rs`, `lower.rs`). And an edge over a stream with a rate takes each item at its tick (above, `suite/zero/hello.expected.ssa`), since nothing in hello ever slept. hello 3 865 → 3 919, static 3 675 → 3 729 on the tool. Every path: zero 477/477 on the CPU paths and 460 + 17 skipped on air, probe test 977/977, 968 + 9, 977, 977, 946 + 31; cargo test 111.

---

### cost: a callee's loop is bounded by what its call site knows — `1630ada` · 2026-09-10

```
copy_u8: loop at b8 x11 (j from at least 0 by 1 to at most 11), body 10 ssa
emit_str: loop at loop x12 (i from 0 by 1 to 12), body 14 ssa
```

The parity pass, hop 11 (fm3 log 76): the measure. `src/cost.rs` costs a function per call site's knowledge — a range per integer argument, memoised by name and ranges, carried through by constants, arithmetic, `min`/`max`, a loop parameter's start and step, and every `check` or branch — so a string literal's length reaches the library's copy (above, from `probe cost suite/zero/hello.expected.ssa run` and the oracle); and the pass that leaves a loop is charged once, so a bottom-tested loop is no longer short a body. Both sides now count by one rule: `hello-mod.ssa` 441 → 735, `hello-min.ssa` 402 → 696; hello 2 113 → 3 865, static 1 995 → 3 675, the block pushes' copies and the loop scheduler's empty pass now visible. Every path: zero 477/477 on the CPU paths and 460 + 17 skipped on air, probe test 977/977, 968 + 9, 977, 977, 946 + 31; cargo test 110.

---

### zero: an edge moves a batch through one view — `162ba63` · 2026-09-10

```
fn __edge1(i: int$, __hz: i64) -> int$
    _1: int[] = unread(i)
    _2: i64 = len _1
    loop(_3: i64 = 0)
```

The parity pass, hop 10 (fm3 log 75). A `for` over a stream now takes the unread items as one view and loads each (`src/zero/lower.rs`, `lower_for_seq`) where it peeked at each, and the edge's sink (above, `suite/zero/hello.expected.ssa`) is `for __item in i$` with the pushes, then one `advance`; `lib/stream.ssa`'s `frame` and `unread` pack the view from the buffer's typed pointer as `peek` reads. The cost tool, counting the loop once, says hello 2 089 → 2 113 and static 1 971 → 1 995; honestly ten items lose about 190. Every path: zero 477/477 on the CPU paths and 460 + 17 skipped on air, probe test 977/977, 968 + 9, 977, 977, 946 + 31; cargo test 108.

---

### stream: the ring's item path is lean — `dc3526a` · 2026-09-10

```
    raw: ptr = ptradd vb, 16
    q: ptr(number) = cast raw
    v: number = load q, k
```

The parity pass, hop 9 (fm3 log 74). `lib/stream.ssa`'s item pushes do their count and base inline with no `ring_pushed` call and no `const` for the header, and `peek` and `latest` read one item back through the buffer's typed pointer (above) after the checks a reader is owed, where they packed a checked view and called `ring_index` for one load. Both checks on a push stay: `suite/stream.ssa`'s `irregular_push` pins the `regular` one, and `peeked_across_the_seam` is new. A push 32 → 30 on the tool, `peek` 34 → 18; hello 2 179 → 2 089, static 2 061 → 1 971, lex 395 → 357. Every path: zero 477/477 on the CPU paths and 460 + 17 skipped on air, probe test 977/977, 968 + 9, 977, 977, 946 + 31; cargo test 108.

---

### zero: a stream is timed only when something asks for time — `608bc51` · 2026-09-10

```
fn __push(s: number$, v: number)
    push(s, v)
    ret
```

The parity pass, hop 8 (fm3 log 73), Ash's ruling that a stream is timed only when a rate or a time word touches it. `time_words` in `src/zero/lower.rs` walks every body before a ring is made, for `x$ at (t)`, `x$ from (a) to (b)` and `position x$` by name (one on a function's stream parameter times the whole store); every other unrated stream, `in$` included, is a plain ring with no ticks, joining the `regular` set, and a store with no ticked ring gets the `__push` above (`suite/zero/hello.expected.ssa`) in place of the clock branch the cost tool always walked. Before → after: hello 2 335 on 435 → 2 179 on 401, static 2 217 on 348 → 2 061 on 314; `push__u8s_int` 150 → 98; lex unchanged, it asks for its input's tick. A front-end change: probe zero test 477/477 native, cargo test zero:: 17.

---

### zero: a stream on the right of `<<` at feature scope is an edge — `48615a8` · 2026-09-10

```
int i$ at (1 hz)
out$ << i$ << "\n"

on count down()
    i$ << [10 through 1]
```

The parity pass, hop 7 (fm3 log 72, question 38), Ash's ruling that `out$ << i$` is wiring. A feature-scope line beginning with a `$` name and `<<` (above, `suite/zero/hello/countdown/countdown.zero`) is an edge: the front end writes a sink for it, a loop of `count`, `peek`, the pushes and `advance` (`src/zero/lower.rs`, `collect_edge`), and wires it as `write(out$)` is, so the scheduler moves each item as it arrives by the dispatch a push uses and pushes the rest of the chain after it; each count is now a line. hello's frame, copy and sequence method go. Before → after: hello 1 940 on 410 → 2 335 on 435, static 1 870 on 325 → 2 217 on 348 — the tool charges the edge's loop to every scheduler pass at all three `__run()` sites, while the honest cost fell by the copy, about 450. A front-end change: probe zero test 477/477 native, cargo test zero:: 17.

---

### zero: static and dynamic features — `16e4e57` · 2026-09-10

```
platform: static on
hello: static on
countdown: static on
bye: static on
```

The parity pass, hop 6 (fm3 log 71), Ash's ruling that a product marks each feature. `product.md` (above, `suite/zero/static/product.md`) takes a line per feature, `static on`, `static off` or `dynamic`. Static off and the feature leaves the store with everything under it before lowering (`src/zero/store.rs`); static on and it has no context field, switch or gate, its chain body standing under its link's name so `run` calls `run__countdown` calls `run__hello` by name (`src/zero/lower.rs`), and the runner never switches it. `suite/zero/static` is hello with every feature static, the counterpart of the hand-written oracle: `run` 1 870 on 325 lines against 412 on 77; `suite/zero/marks` shows the three marks. A front-end change: probe zero test 476/476 native, cargo test zero:: 17.

---

### zero: the emitted IR carries only what the store reaches — `7f72bb3` · 2026-09-10

The parity pass, hop 5 (fm3 log 70). Every store's text carried the platform feature's whole library and an accessor for every context field. `lower` (`src/zero/lower.rs`, `prune`) now keeps what the roots reach — the store's own functions, the platform functions a case names, the runner's entries — by the calls written `name(` in the text, a name standing for a method set where it is one; what nothing reaches goes with its comment, and so does a `data` string nothing names. hello.expected.ssa 582 → 410 lines with `run` unchanged at 1 940; lex 583 → 525, clock 316 → 302. A front-end change: probe zero test 465/465 native, cargo test zero:: 16.

---

### stream: a block push is a copy into each half — `6f88776` · 2026-09-10

```
    a: number[] = view block, 0, first
    d1: number[] = view vals, slot, first
    copy d1, a
```

The parity pass, hop 4 (fm3 log 69). `lib/stream.ssa`'s `push(s, block)` pushed a block's items one by one; it now copies the block into both halves of the ring (above), split where it crosses the seam, and moves the count once; an irregular ring takes `push(s, tick, block)`, and `received(s)` joins the library. The front end pushes a string, a struct's field text or a sequence's unread items as one block, and a one-byte string as the byte itself. Hello's `run` 2 253 → 1 940 on 582 lines from 722; on the tool a block is 282 whatever its length, since `copy`'s loops are counted once, where the honest cost is a few operations a chunk (fm3 question 37). Three new stream cases. probe test 976/976 native, 967 + 9 skipped wasm, 976 riscv, 976 arm-qemu, 945 + 31 skipped air; zero 465/465 on the CPU paths, 448 + 17 skipped air; cargo test 107.

---

### zero: a node's due test reads the ring's count — `115ce71` · 2026-09-10

```
fn __pushed(s: number$) -> i64
    r: ptr = get s, ring
    n: i64 = load r
    ret n
```

The parity pass, hop 3 (fm3 log 68). A node ran when its input's `position` plus `count` passed what it had seen, computed before its run and after, and `position` costs 46 on the cost tool's longest path for a tick the test throws away. The prelude's `__pushed` (above, `src/zero/lower.rs`) is the ring's count, one load, and means the same. Hello's `run` 2 547 → 2 253 on 722 lines; `__node1` 175 → 77, a scheduler pass 187 → 89. A front-end change, proven on the native path under fm3's new testing rule: probe zero test 465/465.

---

### stream: a push through the buffer's pointer; opt: elide-stores — `6c83534` · 2026-09-10

```
    ("elide-stores", elide_stores),
```

The parity pass, hop 2 (fm3 log 66, 67). A push in `lib/stream.ssa` now reads the buffer's own header and stores through a typed pointer to its elements — its slot is under half by construction, so the view's checks would prove nothing — and costs 32 SSA where it cost 51. That leaner kernel hung Apple's compiler on the `platform` store; bisected with `PROBE_AIR_NOINLINE`, the trigger is whole-struct stores into the one context global, inlined hundreds of times. `src/opt.rs` (above) gains a pass: a store that writes back the value just loaded from the same place in the program's own memory goes, and dce drops the loads, so a context setter stores one field (3 to 6 SSA, from 44 to 47); `src/aggregate.rs` stores a `u1` field's loaded byte back as itself so the pass sees it. Hello's `run` 3 820 → 2 547 on 722 lines; the `platform` and `features` kernels compile on air at the first try, and cargo test takes 110 s where the retries made it 321. probe test 973/973 native, 964 + 9 skipped wasm, 973 riscv, 973 arm-qemu, 943 + 30 skipped air; zero 465/465 on the CPU paths, 448 + 17 skipped air; cargo test 107.

---

### stream: the ring never slides — `40a1098` · 2026-09-10

```
    slot: i64 = rem pushed, half
    twin: i64 = add slot, half
    store v, vals, slot
    store v, vals, twin
```

The parity pass, hop 1 (fm3 log 65). Hello's `run` cost 9 157 SSA and 9 100 of it was forty-one pushes at 222 each: `probe cost` walks the longest path, so every push was charged `ring_slot`'s slide of the newer half over the older. `lib/stream.ssa`'s ring (above) now stores each item in both halves of its buffer, so the last half items are always one contiguous view wherever the seam falls, nothing ever moves, and readers index by `k mod half`; the front end carves twice the items it keeps, and `suite/stream.ssa` gains `wrapped` and `wrapped_history` across the seam. `run` 9 157 → 3 820 on 722 lines (from 718), a push 222 → 51, lex's `lex` 956 → 599; less than the 2 200 hoped, since the checked stores and the slice are most of the 51. probe test 973/973 native (two new cases), 964 + 9 skipped wasm, 973 riscv, 973 arm-qemu, 943 + 30 skipped air; zero 465/465 on the CPU paths, 448 + 17 skipped air; cargo test 105.

---

### README: the store's clock — `8798d5e` · 2026-09-10

Third pass item 6, the docs. `README.md`'s `probe zero` section says what the last three landings made true: `<<` methods format into `out$`, a case's `with in "text"` pushes into `in$`, and the store's virtual clock is an integer tick counter. fm3's `zero.md` carries a third-pass line at the head of sections 3, 6, 9, 10, 14 and 15, and its log runs to entry 64 and its questions to 35. At the close of the pass: `probe test` 971/971 native, 962 + 9 skipped wasm, 971 riscv, 971 arm-qemu, 941 + 30 skipped air; `probe zero test` 465/465 on the four CPU paths, 448 + 17 skipped on air; cargo test 105 passed.

---

### zero: the clock is integer ticks — `81d2a6c` · 2026-09-10

```
fn __now() -> i64
    p: ptr = addr __clock
    k: i64 = load p
    ret k
```

Third pass item 5 (fm3 log 63, 64). Ash: the store's clock is an integer tick counter, microseconds in the bootstrap, and a push stamps a tick with no rational arithmetic on the per-item path. The prelude's `__clock` (`src/zero/lower.rs`, above) is one `i64`: `__now()` loads it and `__sleep` adds a period in whole ticks, as a rated ring steps; exact `time` stays at the boundary. `probe cost` on hello's `run`: 204 929 before, 9 157 after; a sparse push 49 196 → 253. The cheaper clock put the `tasks` kernel under air's inlining budget, where Apple's compiler hung on 23 inlined copies of the scheduler, so `emit_air` now counts copies through inlined callers and calls a function whose copies pass a quarter of the budget. zero 465/465 on the CPU paths, 448 + 17 skipped on air; cargo test 104.

---

### zero: `in$` is a system stream — `1c7e43f` · 2026-09-10

```
>two arrivals() with in "let x = 4" → 3, 5
```

Third pass item 4 (fm3 log 62, question 35). Input is the same shape as output the other way: the compiler's platform feature declares `uint8 in$`, a stream without a rate of 512 bytes, and under the runner a case's line is the platform's push — `with in "text"` in the `with` clause beside the switches, its bytes pushed through the prelude's `__in_ch` after the reset and before the program starts, on every path, so a task wired `token t$ = lex(in$)` has run over them when the call is made. The `lex` store (above, `suite/zero/lex/lex/lex.md`) loses `chars$` and keeps the experiment's seven results, the second arrival being the program's own push; `platform` gains `echo`. zero 465/465 on the CPU paths, 448 + 17 skipped on air; cargo test 104.

---

### zero: formatting by dispatch — `39009cd` · 2026-09-10

```
on (uint8 o$) << (pair p)
    o$ << "(" << p.a << ", " << p.b << ")"
```

Third pass item 3 (fm3 log 59, 60, 61; questions 33, 34). A push into a stream dispatches on the item's type like any operator: a `<<` method is `on (uint8 o$) << (int x)`, the stream first and no result, and the compiler's platform feature defines the library's in zero (`src/zero/platform.zero`) — an `int`, a `uint`, a `float` to six places with the trailing zeros dropped, a `bool`, a sequence of ints or floats with spaces — so `out$ << 42` writes the digits and `print (int x$)` is `out$ << x$ << "\n"`. A struct with no method is its fields with spaces, an enumeration its case's name, and a store's own method (above, `suite/zero/platform/format/format.zero`) wins for its type. The `platform` store's `format` feature has twenty cases. zero 418/418 on the CPU paths, 403 + 15 skipped on air; cargo test 103. hello's `run` costs 204 857 by the cost tool's clock branch at every push site, which item 5 removes.

---

### zero: `out$` is a system stream — `ad1a4fc` · 2026-09-10

```
on hello()
    out$ << "hello world" << "\n"
```

Third pass item 2 (fm3 log 57, questions 31, 32). Ash: output is a stream, not a function. The compiler's `platform` feature declares `uint8 out$` and its consumer `on write (uint8 c$)`, wired `write(out$)` — a *sink*, a function with a stream parameter and no result that the bare wiring line makes a task — and `print` is a zero word over it. The print buffer is gone: `out$` is a ring of 4096 bytes carved at every reset, and the runners read it through the platform's own reader on every path, unchanged in name. hello's `run` (above, `suite/zero/hello/hello/hello.zero`) costs 57 602 SSA where it cost 640 310: a string literal pushes its bytes straight, and a ring the compiler knows is regular takes the regular push. The `platform` store shows a store's own sink of `out$`. zero 346/346 on the CPU paths, 333 + 13 skipped on air; cargo test 101.

---

### air: compile again when Apple's compiler crashes — `c4a3fc9` · 2026-09-10

```
note: platform: Apple's compiler crashed on the kernel (100323 inlined under a budget of 400000); compiling again under 50161
```

Found under the third pass's item 2 (fm3 log 58). The zero `platform` store, with a second consumer of `out$`, crashed Apple's compile service as one kernel at 100 323 inlined instructions, where `streams` passes fully inlined at 361 871: the instruction count is not what Apple's compiler minds, and bisecting with a new `PROBE_AIR_NOINLINE=f,g` knob found that calling any of several large pieces lets it compile, not which one it is. So the air runner reacts to the crash itself: `compile_under` takes a budget, `Compiled` reports the kernel's size, and on an interrupted connection the runner compiles again under half of it, costliest copies called first, until the kernel runs. A kernel that compiles first time is untouched.

---

### zero: a braced file is read with a warning — `5632719` · 2026-09-10

```
warn  old.ssa:2: written with braces; the IR indents its blocks now, and `probe indent -w old.ssa` rewrites it
```

Third pass item 1, third landing (fm3 log 56). The brace form is still read — a file written last week runs — but every command that reads a file says so: the line above in `probe test`'s report, and `warning: ...` on standard error from `parse`, `compile`, `run`, `cost` and the rest. The line named is the file's own, the prelude being appended after it. Nothing generated earns one. That closes the item: the IR indents, the tree is converted, the printer and every emitter write the indented form, and the old form is a warning away from gone. Every path unchanged; cargo test 101 passed.

---

### zero: the tree indents — `99eedb7` · 2026-09-10

```
type rgb = pack
    r: u5
    g: u6
    b: u5
```

Third pass item 1, second landing (fm3 log 55). Every `.ssa` in the tree — `suite/`, `lib/`, `os/`, `examples/`, the three `.expected.ssa` — rewritten by `probe indent -w`, so no brace remains in any of them (`suite/packs.ssa` above). The emitters write the indented form themselves: `src/zero/lower.rs` first, whose output for hello, lex and clock is line for line what the converter made of the old expected files; then the suite's machine drivers, testfloat's wrappers and the fuzzer's programs. The `ir` bodies in `src/zero/platform.zero`, IR text inside a zero file, are converted too, and `ssa.md`'s and README's examples with them. The Rust unit tests' inline IR stays braced, since the parser reads both. Every path unchanged; `probe cost` reports hello's `run` at 640 310 as before; cargo test 101 passed.

---

### zero: the IR reads its blocks from indentation — `a7a0370` · 2026-09-10

```
fn sum(n: i64) -> i64
    acc: i64 = loop(i: i64 = 0, a: i64 = 0)
        done: u1 = cmp.ge i, n
        if done
            break a
        a2: i64 = add a, i
        i2: i64 = add i, 1
        continue i2, a2
    ret acc
```

Third pass item 1, first landing (fm3 log 53, 54). Ash: the IR drops its braces and indents its blocks as zero does. The lexer measures indentation and a layout pass makes of it the block tokens the braces gave, so the parser reads one stream: a line indented past its header opens a block, a dedent closes, a label heads a basic block with its instructions under it, `if c` then `else` is an empty arm, a `struct`'s fields and a `data` item's values are lines, `platform arm64` takes its rules under it. A literal `{` still opens a block of its own, so braced and indented text parse together — the prelude is appended to every program. `struct(x: f32, y: f32)` is the one-line spelling a type's name uses. `probe indent <file>` (`-w` in place) rewrites a braced file keeping every comment, and a test round-trips every `.ssa` in the tree through it. Nothing in the tree is converted yet. Every path unchanged; cargo test 101 passed.

---

### zero: the emitted IR is abstract — method sets, the policy choosing a width — `227218e` · 2026-09-09

```
on (bool ok) = literal takes the float width()
    ok = float width of (2.5) == float width()
```

Second rulings pass items 7 and 8 (questions 26 and 27, log 52). Ash: the front end emits abstract IR and never decides a width; the concrete IR is the policy's. The IR (`src/ssa.rs`) gains *method sets*: a plain name defined more than once with different parameter types is one name, the later definitions named inside by their types, and a call by the name resolved by the arguments' types once the policy has resolved them — `width_of(3: int)` reaches the 32-bit method on wasm and the 64-bit one elsewhere, from one text. The zero front end emits a name whose methods are all concrete as such a set, tries a bare literal at both widths, and where they choose differently writes the literal `: int` or `: float` and leaves the choice to the policy; `product.md`'s `int:` and new `float:` lines set the policy, never the text. The case above, in `suite/zero/functions`, holds on every path. zero 331/331 on the four CPU paths (255 cases, 13 overridden), 320 + 11 skipped on air; probe test on all five paths unchanged; cargo test 97 passed.

---

### zero: the enabled gate is the ancestor conjunction — `0732f86` · 2026-09-09

```
fn __on_countdown() -> u1 {
    own: u1 = __get___enabled_countdown()
    up: u1 = __on_hello()
    on: u1 = and own, up
    ret on
}
```

Second rulings pass item 6 (Ash's caveat under question 25, log 51). Switching a feature off must never write its descendants' fields, so that switching the parent back on restores what was beneath it. `src/zero/lower.rs` now generates one `__on_<feature>` per feature — the lines above are from `suite/zero/hello.expected.ssa` — and every gate reads it: chain links, nodes, bare `enabled` and `<feature>.enabled`. The runner switches one field per context and works out who is effectively off from the parent tree; a case line is a sequence of switches, and `features/most`'s `>switches() with more off, base off, base on → 0, 1` shows `more` still off and `most` still on after `base` has been off and on. The three expected files change by the new functions; hello's `run` costs 640 310 ssa. zero 325/325 on the four CPU paths (252 cases, 13 overridden), 314 + 11 skipped on air; cargo test 97 passed.

---

### zero: tests are functions — `b03dc6c` · 2026-09-09

```
## testing
>existing
>times described() → 2
>describe some() → "ints"
>describe (4) → "int"
```

Second rulings pass item 5 (question 25, log 50). Ash: every function `run` has a test function `test run`, extended by the same mechanism as `run`. So a feature's `## testing` cases for a method are its definition of that method's test, and `src/zero/run.rs`'s `plan` composes them per context as the front end composes the method: the newest feature that is on is outermost and replaces the older feature's cases for the method, unless its section says `>existing` — the lines above are `suite/zero/functions/more`'s, whose redefinition of `describe (int)` calls `existing` and leaves every older case true — and a feature that is off drops out of the chain. This is the general form of the first pass's "same call, newer wins"; a plainer line of the same feature still yields to one naming the context. zero 319/319 on the four CPU paths (250 cases, 12 overridden), 308 + 11 skipped on air; cargo test 97 passed.

---

### zero: creation time orders, and a published feature is immutable — `be8f4e0` · 2026-09-09

```
layer: runtime
published: 2026-09-09
```

Second rulings pass item 4 (question 24, log 49). Ash: features are created at different times and added together, so composition order is the origin timestamp whatever the commit, and two features created at once order by name. `src/zero/store.rs` drops the tie refusal and the `same commit` marker of the first pass. The two lines above are `suite/zero/skeleton`'s header: a feature `published` on a date has other users and its code is immutable from then, every change of meaning being a sub-feature. The bootstrap's ledger is git, so where the store is inside a repository the reader asks git about the `.zero` file and refuses an uncommitted change or a commit dated after the publication; outside a repository it cannot tell and says nothing, and the log lists what else it does not catch. A Rust test works a scratch store through a temporary repository. zero 319/319 on native, wasm, riscv and arm-qemu (250 cases, 12 overridden), 308 + 11 skipped on air; cargo test 97 passed.

---

### zero: `yields` — `5c0e052` · 2026-09-09

```
int n = loop (int i = 1, int m = -1) yields m
```

Second rulings pass item 3 (question 23, log 48). The word that names what a loop gives is `yields`, Ash's choice over the first pass's `gives` and the offered `after`: the same place at the end of the header, the same meaning, reserved only there. `src/zero/syntax.rs` and the lowering's messages say `yields`; the seven loops in `suite/zero/control`, `suite/zero/streams` and `suite/zero/lex` are rewritten, the line above being the lexer's; `lex.expected.ssa` is unchanged, since the word never reaches the IR. `gives` is a name word again, and `control` names a function `gives twice` to show it. zero 319/319 on native, wasm, riscv and arm-qemu (250 cases, 12 overridden), 308 + 11 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 96 passed.

---

### zero: a bare literal takes the product's int width — `740a8b7` · 2026-09-09

```
on (bool ok) = literal takes the int width()
    ok = width of (3) == int width()
```

Second rulings pass item 2 (question 22, log 47). With `width of` declared over `int32` and over `int64` and no `int` method, `width of (3)` was refused as ambiguous; Ash: pick the product's `int` width. Dispatch in `src/zero/lower.rs` gains a round between "the literal's own type" and "any number" that takes an integer literal as the product's width — the path's policy, `int64` here and `int32` on wasm, unless the store's `product.md` says `int: 32` or `int: 64`, which pins the store on every path and sets the policy it is built under (`src/zero/store.rs`, `src/zero/run.rs`). The IR notes each call the width decided. The case above holds at both widths, `int width` reading its answer from an overflow. zero 318/318 on native, wasm, riscv and arm-qemu (249 cases, 12 overridden), 307 + 11 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 96 passed.

---

### zero: an int64 may meet a float64 — `8f4931b` · 2026-09-09

```
on (int64 d) = rounding above fifty three bits()
    int64 big = 9007199254740993
    float64 f = big
    d = int64(f) - big
```

Second rulings pass item 1 (question 21, log 46). The accuracy rule of the first pass refused an `int64` beside any float, since no float holds every `int64`; Ash called that overly restrictive — arithmetic is modulo its range, and the writer knows it. `wider` in `src/zero/lower.rs` keeps the exact table as `wider_exact` and otherwise gives the widest type of the family, `float64` for an `int64` with a float, so the lines above are accepted and the case gives -1: the conversion rounds above 2^53, and the emitted IR says so in a comment above the `conv`, the warning the ruling allows. A narrowing, and a `float32` from an `int32` that `float64` holds better, stay refused. `suite/zero/types` gains six cases. zero 312/312 on native, wasm, riscv and arm-qemu (246 cases, 12 overridden), 301 + 11 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 95 passed.

---

### zero: the docs pass — `c5c9e2b` · 2026-09-09

```
device string open_tool = ""
```

Rulings pass item 11 (log 45). Every example in zero.md sections 3 to 14 was checked against a store or run from a scratch one, and the line above, section 5's own, did not build: `""` emitted an empty `data` item the IR refuses. `src/zero/lower.rs` now points an empty literal at the prelude's `__nul` with length zero, and `suite/zero/variables` carries the line with two cases; `suite/zero/control` gains section 7's `[0 to n] + _` as `sum below`. `README.md`'s `probe zero` paragraph describes the contexts and overrides. zero 306/306 on native, wasm, riscv and arm-qemu (240 cases, 12 overridden), 295 + 11 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 95 passed.

---

### zero: every case in every context — `95229da` · 2026-09-09

```
## testing
>hello() → "hello world"
>run() → "hello world"
```

Rulings pass item 10 (question 15, log 44). `probe zero test` runs each store's cases with every feature on and once per feature with it off alone, a feature off taking its descendants with it (`Store::subtree` in `src/zero/store.rs`); a case stands wherever its own feature is on. Two cases making the same call in one context are one promise: the newest feature's stands, then the line naming more features, and the rest print as `over`. So `suite/zero/hello/hello/hello.md` promises the line above, countdown's `run()` overrides it, bye's overrides that, and with `bye` off countdown's stands and passes. `src/zero/run.rs` plans the runs (`contexts`, `effective`, `plan`) and counts them: zero 302/302 on native, wasm, riscv and arm-qemu (236 cases, 12 overridden), 291 + 11 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 95 passed.

---

### zero: the case line's context — `0066081` · 2026-09-09

```
## testing
>run() → "10 9 8 7 6 5 4 3 2 1\nhello world\ngoodbye"
>run() with countdown off → "hello world\ngoodbye"
```

Rulings pass item 9 (question 14, log 43). Switching a feature is not in the language: `countdown.enabled = false` is refused in `src/zero/lower.rs`, and a case names the context it runs in after its call (`suite/zero/hello/bye/bye.md` above; `with more off, most on` joins several). `src/zero/store.rs` keeps the clause, `resolve_case` turns each feature into a call of its `enabled` setter, and `src/suite.rs` makes those calls on every path between `__zero_reset()` and a new `__zero_start()`, which runs the scheduler — split from the reset so a node of a feature that is off never runs (`Call.before`, `Case.before`, `driver.js`'s `before` list, the machines' `__caseN`). `suite/zero/features/most` says its switches on case lines now, nine cases where it had ten functions. zero 230/230 on native, wasm, riscv and arm-qemu, 221 + 9 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 94 passed.

---

### zero: `order.md` — `11d53a3` · 2026-09-09

```
# order
*the store's layers, lowest first*

- runtime
- tools
```

Rulings pass item 8 (question 16, log 42). The layer list is `order.md` beside a store's feature folders (`suite/zero/features/order.md` above): `src/zero/store.rs`'s `read_order` wants a line saying `lowest first` before the `- name` lines, refuses the file without it, and tells a store still holding `layers.md` of the rename. Composition order stays the earliest origin timestamp until a store is a repository with a ledger; two features that share one are now refused naming both, unless each origin says `same commit` after its timestamp, when they compose by name — the ledger's rule for one commit, spelled in the `.md` header, written up for Ash as question 24. zero 227/227 on native, wasm, riscv and arm-qemu, 218 + 9 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 94 passed.

---

### zero: bounds are product settings — `602c8e3` · 2026-09-09

```
on count down()
    int i$ at (1 hz)
    i$ << [10 through 1]
    print (frame i$)
```

Rulings pass item 7 (question 19, log 41). `bound N` leaves the language — the parser takes it nowhere and `bound` is a name word again — because a trip count is a product setting, never a number in feature code. hello's countdown is written as section 16 reads (`suite/zero/hello/countdown/countdown.zero` above): a stream at one hertz, the range pushed into it; a range pushed as a block now pushes its values straight in (`lower_range` with a sink in `src/zero/lower.rs`), so `probe cost suite/zero/hello.expected.ssa run` shows `count_down: loop at b1 x10 (_1 from 10 by -1 to 1)` with no bound anywhere and `run` at 640288 ssa, down from 1767127. Where the tool still needs a count, a store's `product.md` says `bound <function words>: N` (`src/zero/store.rs`) and the front end puts it on that function's loops, marked `; product setting`; `suite/zero/tasks/product.md` is the first. zero 227/227 on native, wasm, riscv and arm-qemu, 218 + 9 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 94 passed.

---

### zero: loops give their result — `a6d1bd8` · 2026-09-09

```
on (int g) = gcd of (int a) and (int b)
    g = loop (int x = a, int y = b) while (y != 0) gives x
        int r = x % y
        continue (y, r)

on (int t) = triangle (int n)
    t = row ([1 to n + 1]) + _
```

Rulings pass item 6 (question 4, log 40), reversing log 12. A `loop`'s carried variables are its own and gone after it; `gives` at the end of the header names the ones that come out, into a declared name, an existing one, a result, or several (`src/zero/syntax.rs` `parse_loop`, `src/zero/lower.rs` `lower_loop`), and every `break` yields them. `gives` is reserved only inside a loop's header; section 7's other candidate, `acc after loop (...)`, is written up in `questions.md` 23. A function with no result now maps over a stream as a loop of calls, and every store is rewritten to a sequence form where one would do — sums as `[0 through n] + _`, `triangle` as a map and a reduction, a check per item as a mapped function — leaving `loop` where a reader moves an item at a time or a search has no count. Found: `[1 through n]` counts down when n < 1, so "1 to n" is spelled `[1 to n + 1]`. zero 227/227 on native, wasm, riscv and arm-qemu, 218 + 9 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 93 passed.

---

### zero: `_` is the candidate — `cde1a74` · 2026-09-09

```
on (int i$) << count down from (int n)
    i$ << n << (i$ - 1) while (_ > 0)

on (int i$) << count up to (int n)
    i$ << 1 << (i$ + 1) while (_ <= n)
```

Rulings pass item 5 (question 9, log 39). A push chain's `while` still tests the item about to be pushed and pushes it only when the test holds, but that item is now `_` — the value in hand, as in a reduction — and the stream's own name in the condition is its latest item, the one meaning `x$` has everywhere (`Lowerer.candidate` in `src/zero/lower.rs`, read by the `_` arm, an operator, or a call's argument while the condition is lowered). `while (i$ < 5)` is not refused: it gives `[1, 2, 3, 4, 5]` where `while (_ < 5)` gives `[1, 2, 3, 4]`, and `suite/zero/streams` keeps one case of each. Every chain in the stores and the three programs is rewritten with `_`, their emitted IR unchanged; section 10's examples in zero.md follow section 9 now. zero 226/226 on native, wasm, riscv and arm-qemu, 217 + 9 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 93 passed.

---

### zero: one `$` — `d884367` · 2026-09-09

```
on (int n) = pushed into()
    int i$ = [1, 2]
    i$ << 3
    n = count i$ * 10 + latest i$

on (int n) = unread only()
    int i$ << 1 << 2 << 3 << 4
    advance i$ by (2)
    n = (i$ + _) * 10 + count i$
```

Rulings pass item 4 (question 12, log 38). The front end's two types for a `$` name, a sequence over the arena and a stream over a ring, are one: `Ty::Stream` in `src/zero/lower.rs`, the IR's `T$`, whether the items arrive over time or are all present. A bare `T x$` is an empty stream, `T x$ <<` with nothing after is refused, and a list, a range, a string literal or a function's result is a stream with those items resident. The section 8 words read a stream's unread items where its reader stands — `x$[i]` is a `peek`, `for`, map, zip and reduce go through the library's new `unread(s)` view — so `count` and the words agree after an `advance`. `frame`, `behind` and `from ... to` copy their view into a new ring; a push goes through a prelude template `__push` that reads the ring's step, so a regular ring is no longer a type of its own. Every sequence is a ring of at least 64 items, and a range with literal bounds fills it with the counted loop `probe cost` reads. `sequences` and `streams` each gain the other's words (ten cases), `tasks` and `lex` lose their bare `<<`. zero 225/225 on native, wasm, riscv and arm-qemu, 216 + 9 skipped on air; probe test 971, 962 + 9, 971, 971, 941 + 30; cargo test 93 passed.

---

### zero: implicit conversion — `2ef0fcc` · 2026-09-09

```
on (float64 q) = ratio of (int32 a) to (int32 b)
    q = a / b

on (int64 p) = product of (int32 a) and (int32 b)
    p = a * b
```

Rulings pass item 3 (question 1, log 37), loosening log 7's one-type rule to conversions that lose nothing. Two concrete number types compute in the type that holds every value of both exactly (`wider` in `src/zero/lower.rs`): the wider of two widths of one signedness, a signed type wide enough for an unsigned one, the wider float, and for a float with an integer the float whose significand holds it — so `int64` with `float64` is refused rather than rounded, where Julia would round. An operator then computes in the expression's wanted type when both operands widen to it exactly: `ratio of` divides as floats, `product of` multiplies at 64 bits. A value widens on assignment, into a field, a `continue`, a loop header, a call's result and a concrete parameter; an abstract parameter binds to the widest of what its arguments bring; `if then else` arms meet in the wider. Narrowing stays `int32(x)`, and every refusal names that form. `suite/zero/types` gains eight cases. zero 214/214 on native, wasm, riscv and arm-qemu, 205 + 9 skipped on air; probe test 970, 961 + 9, 970, 970, 940 + 30; cargo test 93 passed.

---

### zero: multiple dispatch — `9ac8778` · 2026-09-09

```
on (int32 k) = kind of (number x)
    k = 1

on (int32 k) = kind of (int x)
    k = 2

on (int32 k) = kind of (int32 x)
    k = 3
```

Rulings pass item 2 (question 18, log 36), Julia's model, generalising the platform-only overloading of log 31. A name is a set of methods: a declaration with the same parameter types redefines that method and joins its chain, other types are a new method, named in the IR by its types (`kind_of__i32`, `mul__Vec_Vec`). A call (`src/zero/lower.rs`, `find_methods`, `choose`, `pick`) tries each method on its arguments and takes the most specific of those that fit, `int32` before `int` before `number`; with none most specific it is refused as ambiguous, naming the contenders. A literal, or a list of literals, is its own type first — `describe (3)` takes `describe (int)` over `describe (float)`, `describe ([1, 2])` takes `describe (int$)` over `describe (string)` — and a method taking a sequence beats a map of the item method. Found on the way: a literal meeting an abstract parameter binds it to the literal's own type as a typed constant, since the IR reads a bare `2.5` under `number` as an integer. `suite/zero/functions` gains `describe`, `kind of`, `area of` and a second feature `more` (sixteen cases), `types` a second `*` on `Vec`. zero 206/206 on native, wasm, riscv and arm-qemu, 197 + 9 skipped on air; probe test 970, 961 + 9, 970, 970, 940 + 30; cargo test 93 passed.

---

### zero: assigning the result ends the function — `b060dff` · 2026-09-09

```
on (int r) = first positive of (int a) and (int b)
    if (a > 0)
        r = a
    r = b
```

Rulings pass item 1 (question 2 of `fm3/milestone-0/questions.md`, log 35), reversing log 4. An assignment that gives a function its last result ends it there: `src/zero/lower.rs` emits `ret` right after it — inside an `if` arm, a `loop` or a `for` as well as at the top level — and a statement after it is refused as never running, naming the assignment's line. Whether a statement leaves its block is now what the lowering did (`lower_stmt` returns it; the tree readers `terminates` and `has_break` are gone), since it depends on which results have values on that path; until the last of several results is assigned the function goes on and may read the ones it has. `control`'s `magnitude of` keeps a temporary; `functions` gains `first positive of`, `ordered` and `power of two above`, six cases; `lex.expected.ssa` changes where `kind of` returns inside its `if` chain. zero 191/191 on native, wasm, riscv and arm-qemu, 182 + 9 skipped on air; probe test 970, 961 + 9, 970, 970, 940 + 30; cargo test 93 passed.

---

### README: probe zero — `e74567d` · 2026-09-08

Plan item 13 of milestone 0, the documentation half that lives in probe: `README.md` gains a section on `probe zero` — the store it reads (`name/name.md` for the prose and cases, `name/name.zero` for the code), the commands (`zero <store> emit`, `zero <store> run <case>`, `zero test [dir] [path]`), the case form, the compiler's own `platform` feature and a store's platform bodies, the `.expected.ssa` files and the fourteen stores under `suite/zero/`, illustrated with `suite/zero/skeleton` verbatim. The other half — the milestone 0 line at the top of each of zero.md's sections 2 to 15, six sentences corrected, and the log and questions files — is in the fm3 project, which is not a repository.

---

### zero: the three programs — `9c90e1c` · 2026-09-08

```
int t$ = ticks(5) at (4 hz)

on (int t$) << ticks (int n)
    t$ << 1 << (t$ + 1) while (t$ <= n)

on (int v) = value at (int m)
    v = t$ at (m ms)
```

Plan item 12 of milestone 0. `suite/zero/clock` is the smallest program that needs time at the boundary: a task ticking at a rate on the store's clock and a consumer that turns milliseconds into a `time` once and reads the stream at it, nearest; six cases (`value at (300) → 2`, `where it stands() → 2, 500000`). A time unit may now follow a variable, and `bound N` a push chain's `while` (`src/zero/syntax.rs`, `lower.rs`): hello's countdown says `bound 10`, and `probe cost suite/zero/hello.expected.ssa run` reports `count_down_from: loop at b1 x10 (declared)`, 1619082 ssa for `run`. `hello.expected.ssa`, `lex.expected.ssa` and `clock.expected.ssa` sit beside their stores and `probe zero test` (`src/zero/run.rs`) checks the emitted IR against them, naming the first line that differs, so a lowering change is a diff in the commit. Also found: a node's literal task arguments needed their types. zero 185/185 on native, wasm, riscv and arm-qemu, 176 + 9 skipped on air; probe test 970, 961 + 9, 970, 970, 940 + 30; cargo test 93 passed.

---

### zero: platform functions — `cd7ed31` · 2026-09-08

```
on (int64 r) = (int64 a) plus (int64 b)
platform arm64 riscv64
    add r, a, b
platform wasm32
    i64.add
platform air
    add
```

Plan item 11 of milestone 0, section 15 of zero.md. A platform function has no zero body: each `platform <kind>` line gives it one for a kind of place — an IR target, whose body is that target's rule lines, or `ir`, a body in the IR serving every target without a rule. The IR gained the item that carries them (`src/ssa.rs`): `platform arm64 { ... }` after the function, in the platform file's grammar, joined to the file's rules by `Platform::natives` (`src/platform.rs`), so `suite/platform.ssa` runs its rules on all five paths. `print` is now a function of the compiler's own `platform` feature (`src/zero/platform.zero`), composed first into every store, declared twice — over a string and over `int$` — with the buffer loops as `ir` bodies; the runner (`src/zero/run.rs`) skips a case whose function reaches a platform function with no body for the path, saying so: `'minus' has no platform body for air`. Found on the way, an IR fix: a plain function's body `int` stored into an `int[]` view is judged under the policy (`aview` in `suite/abstract.ssa`). `suite/zero/platform`: 9 cases; zero 176/176 on native, wasm, riscv and arm-qemu, 167 + 9 skipped on air; probe test 970, 961 + 9, 970, 970, 940 + 30; cargo test 93 passed.

### zero: checks — `662ad8d` · 2026-09-08

```
on (int s) = guarded sum (int k)
    check (k >= 0)
    loop (int i = 1, int acc = 0) while (i <= k)
        continue (i + 1, acc + i)
    s = acc
```

Plan item 10 of milestone 0, section 14 of zero.md. `check (c)` is the IR's trap, with the site — `check at checks.zero:6` — printed into the program's output buffer first, and every runner reads it back (`src/suite.rs`, `checked_at`): the JIT's forked child through a `SIGTRAP` handler that reads the buffer via the JIT before it ends, wasm from the instance that outlives `unreachable` (`src/driver.js`), a machine from `__trap` printing the buffer after `check`. So `>guarded sum (-1) → check` passes with `(at checks.zero:6)` on its ok line, and a case that wanted a number reports `a failed check at checks.zero:6`. Every zero call now runs in a forked child under the JIT and reads its text back, so an unexpected trap is reported rather than ending the runner. `suite/zero/checks`: 11 cases; 167/167 on native, wasm, riscv and arm-qemu, 160 + 7 skipped on air; probe test 966, 957 + 9, 966, 966, 936 + 30; cargo test 91 passed.

### zero: features — `756dbd3` · 2026-09-08

```
on run()
    count down()
    existing run()

on run without countdown()
    countdown.enabled = false
    run()
```

Plan item 9 of milestone 0, section 12 of zero.md. A redefinition joins a chain: the bodies are `run__hello`, `run__countdown`, `run__bye`, the entry `run` is the outermost link, and each link below (`run__before_bye`) reads its feature's `enabled` and calls the body or falls through, which is what `existing run()` calls (`src/zero/lower.rs`, `emit_links`). Every feature has an `__enabled_` field in the context, on at every reset; `countdown.enabled = false` switches one from a test, and a node of a feature that is off keeps its readers. `layers.md` orders a store's layers and `src/zero/store.rs` checks parents and layers; a name may reach down or sideways, never up, while a chain's entry stays its first definer's so control flows up. `suite/zero/hello` is section 16's program with its countdown as a task at `1 hz`; `suite/zero/features` 12 cases. 156/156 on native, wasm, riscv and arm-qemu, 155 + 1 skipped on air; probe test 966, 957 + 9, 966, 966, 936 + 30; cargo test 91 passed.

### zero: tasks and the scheduler — `530e6e4` · 2026-09-08

```
uint8 chars$ <<
token t$ = lex(chars$)

on (token t$) << lex (uint8 c$)
    loop
        if (count c$ == 0)
            break
        int k = kind of (peek c$ at (0))
        int start, int tick = position c$
```

Plan item 8 of milestone 0, section 10 of zero.md. A task is a function over readers: `fn lex(t: __s_token, c: u8$, __hz: i64) -> u8$` takes its output's reader and its inputs and returns the inputs moved on, the shape `lex.ssa` wrote by hand. `token t$ = lex(chars$)` at feature scope is a node; `__run()` in `src/zero/lower.rs` passes over the nodes until nothing runs, a node running when an input has more than it last saw or has ended since, its reader carried in the context between runs. The clock is an exact `time` in data and `at (1 hz)` sleeps one period after each push. On the way: an abstract `int` now unifies with its family under the policy (`src/ssa.rs`), and the AIR emitter calls rather than inlines the costliest functions of a kernel past 400 000 inlined instructions, since the `tasks` store crashed Apple's compile service at 757k. `suite/zero/tasks` 14 cases, `suite/zero/lex` the experiment's four; 139/139 on native, wasm, riscv and arm-qemu, 138 + 1 skipped on air; probe test 966, 957 + 9, 966, 966, 936 + 30; cargo test 91 passed.

### zero: streams — `fcd8146` · 2026-09-08

```
on (int n) = repeated()
    int i$ << 1 << (i$ + 1) while (i$ < 5)
    n = count i$ * 10 + latest i$

on (int n) = walked()
    int i$ << 1 << 2 << 3 << 4
    loop (int acc = 0) while (count i$ > 0)
        acc = acc + peek i$ at (0)
        advance i$ by (1)
    n = acc
```

Plan item 7 of milestone 0, section 9 of zero.md. A stream is the IR's `T$`, a reader's view of a ring carved from the store's arena (64 items) where its declaration runs; it is spelled like a sequence and told apart by how it was made — `<<` or `at (n hz)` — so `frame` and `behind` give sequences back. On the right of `<<` the stream's name is its latest item, and `while` repeats the last push while the condition holds of the candidate, which is what makes section 9's example `[1, 2, 3, 4]`. `advance` and `frame` rebind the variable to the moved reader and a `loop` carries every stream its body moves, the loop `lex.ssa` wrote by hand. A stream of a struct is one ring per field. `lib/stream.ssa` gains `end`, `ended` and `position` in the header's spare word, with five cases in `suite/stream.ssa`. `suite/zero/streams`: 20 cases, 116/116 on native, wasm, riscv and arm-qemu, 115 + 1 skipped on air; probe test 965, 956 + 9, 965, 965, 935 + 30; cargo test 91 passed.

### zero: probe zero test on the machines — `876921b` · 2026-09-08

```
>answer() → 43
>greet() → "hi there"
>boom() → check
```

`probe zero test [dir] [wasm|riscv|arm-qemu|air]`: a zero store now runs on every path probe has, through the suite's own machine drivers. The three runners' prepare-build-run bodies in `src/suite.rs` became one `machine_output`, used by `run_riscv`, `run_arm_qemu`, `run_air` and the zero runner's `run_calls` alike; the driver calls `__zero_reset()` at the top of each case, prints an empty line for a call with no results, and after a case wanting text prints the output buffer as hex words through a generated `__ptext()`. A check case boots alone with the trap handler; air skips checks and recursion as it does for directives. The scratch store above (wrong number, wrong text, an index out of range) fails the first two with the machine's real values and passes the check on all four paths, air skipping it. Five `zero_suite_*` cargo tests run `suite/zero` on every path. 96/96 on native, wasm, riscv, arm-qemu and air; probe test 960, 951 + 9, 960, 960, 931 + 29; cargo test 91 passed, 0 failed, 2 ignored.

### zero: sequences — `aec4a77` · 2026-09-08

```
on (int n) = zipped()
    int i$ = [1, 2, 3, 4]
    int j$ = [4, 5, 6]
    int k$ = i$ + j$
    n = count k$ * 100 + k$[3] * 10 + (k$ + _) - 25

on (int m) = smallest()
    int i$ = [4, 1, 3]
    m = smaller of (i$) and (_)
```

Plan item 6 of milestone 0, section 8 of zero.md. A sequence `T$` is the IR's view `T[]`, carved from one arena per store that `__zero_reset` empties before each case, so nothing a call makes aliases what an earlier call made. Lists and ranges fill a new sequence, `x$[i]` is a checked `load`, `count` is `len`, `for` walks the index. A function of one item over a sequence is a loop giving a new sequence (an operator with a scalar, the slice library's chunked `add`); two sequences run to the longer length, the shorter reading as zero past its end; `_` folds one sequence from its first item, `+` through the library's `sum`. The wasm emitter now sizes its memory from the data rather than one page, with `suite/data.ssa`'s `big_end` covering it everywhere. `suite/zero/sequences`: 21 cases, 96/96 on both paths; probe test 960, wasm 951 + 9; cargo test 83 passed and the 3 Metal tests, qemu and llvm-dis having arrived on the machine during this item.

### zero: variables — `b9e9172` · 2026-09-08

```
int size = 40
static int port = 8822
int count = 0 merge sum

on bump()
    count = count + 1

on (int n) = bumped (int k) times
    for (i in [1 through k])
        bump()
    n = count
```

Plan item 5 of milestone 0, section 5 of zero.md. Feature-scope variables are the fields of one context struct per store, `type __ctx = struct { size: int, port: int, count: int, ... }` in `data`, each field's scope and merge words carried as a comment above the type — every scope is one place in this milestone — and reached through generated `__get_x` and `__set_x` functions, so `count = count + 1` is a load, an add and a store and may sit inside a loop. `__zero_reset`, which the runner calls before every case, is generated per store with every initial value. Two consequences: a phrase reads as a function's name before its words are taken as variables (`set size (50)`), and an enumeration is a byte, since the IR stores only memory widths. `suite/zero/variables`: 19 cases, 75/75 on both paths; 959 and 950 + 9 unchanged, cargo test 71.

### zero: control flow — `7c31485` · 2026-09-08

```
on (int s) = sum to (int n)
    loop (int i = 1, int acc = 0) while (i <= n) bound 100
        continue (i + 1, acc + i)
    s = acc

on blast off ()
    for (i in [3 through 1])
        if (i == 1)
            print "one"
        else
            print "more"
```

Plan item 4 of milestone 0, section 7 of zero.md. An `if` statement becomes the IR's value-yielding `if`, one result per variable an arm assigns, each arm yielding its version — so the code after reads the join's names and SSA is kept without the front end inventing anything. A `loop` carries exactly the variables in its header: `while` is tested at the top of every pass, `continue` gives the next values (a body that ends carries the current ones), every `break` yields them, and after the loop the variables hold what it left with; an assignment inside a loop to a variable declared outside it is refused, since SSA has no place for it. `for` over a range is that loop with the item carried, stepped by one, the direction fixed by literal bounds or chosen at run time. `probe cost` on the emitted IR reports `sum_to` as `x100 (declared)` and `blast_off` as `x3 (i from 3 by -1 to 1)` with no bound declared. `suite/zero/control`: 26 cases, 56/56 on both paths; the suite's 959 and 950 + 9 unchanged, cargo test 71 with the 15 environment failures.

### zero: types — `6421bc3` · 2026-09-08

```
type Vec =
    float x, y, z = 0

type Tristate = no | yes | maybe

on (Vec v) = (Vec a) + (Vec b)
    v = Vec(a.x + b.x, a.y + b.y, a.z + b.z)
```

Plan item 3 of milestone 0, section 4 of zero.md. A struct is the IR's `struct`, built by position (`Vec v(1, 2, 3)`), by name (`Vec v(z = 3, x = 1)`), in an expression (`Vec(...)`) or from its defaults alone, read with `v.x`; an enumeration is an unsigned integer wide enough for its cases with a `const` per case, compared with `==` and `!=` only; a `string` is `u8[]`, a view of bytes, a literal a `data` item made into a view by the prelude's `__str`, and `print` takes one; a conversion is a type applied to a value, `int(x)`. An operator on a declared type is a named function, `add_Vec(a, b)`, because the IR does not dispatch arithmetic on structs and plain functions cannot share a name — the one place the lowering departs from section 6's wording. The IR itself needed a fix: a struct's `int` fields are now resolved through the policy at declaration, as `float` fields were, so the layout is known and a later mention of the type finds the same entry (`suite/struct.ssa abstract_fields`; 959 native, 950 + 9 wasm). `suite/zero/types`: 15 cases, 30/30 on both paths with the other stores.

### zero: functions and expressions — `bff0df1` · 2026-09-08

```
on (number n) = smaller of (number a) and (number b)
    n = if (a < b) then (a) else (b)

on (int q, int r) = divide (int a) by (int b)
    q = a / b
    r = a - q * b
```

Plan item 2 of milestone 0, section 6 of zero.md: a function body may hold arithmetic, comparisons, `if (c) then (a) else (b)` as an expression, calls in open syntax — the arguments in bracket groups anywhere among the words, or bare, or a group first as in `(3) is less than (4)` — several results taken at once (`q, r = divide (a) by (b)`, or `int q, int r = ...` declaring them), and local declarations with a value. The IR keeps the program's names: `smaller_of_and(a: number, b: number) -> number`, `_1: u1 = cmp.lt a, b`, then `n: number = if _1 { yield a } else { yield b }`, each arm lowered into its own block. Types are strict in a body — two operands share a type, or one is a literal and takes the other's — and the tower binds only at a call: a `number` parameter takes any number and its result comes back as the argument's type, which is how `narrow (x)` gets a 32-bit instance of `smaller of` from the IR. `suite/zero/functions`: 13 cases, 15/15 with skeleton on native and wasm; the suite's 958 and 949 unchanged.

### zero: the front end's skeleton — `103c9da` · 2026-09-08

```
on (int n) = answer()
    n = 42

on hi()
    print "hi"
```

Milestone 0 of fm3 begins: zero, the feature-modular language defined in `fm3/zero.md`, gets its front end in probe — `src/zero/`, plain Rust, a lexer, a parser, a store reader and a lowering to IR *text*, which then goes through `parse` and everything after it, so the IR stays the meaning and `probe parse` is the oracle for the front end. `probe zero <store> emit` prints that text, `probe zero <store> run <case>` runs one case, `probe zero test [dir] [wasm]` runs every store under `suite/zero` the way the suite runs its files. A store is a flat folder of feature folders, `name/name.md` beside `name/name.zero`: the prose carries `parent:`, `layer:`, the origins with their timestamps (composition order), and the cases under `## testing` as `>answer() → 42` — a number, a quoted string compared with what `print` wrote, or `check`. Blocks are indentation; a `#` in a `.zero` file is refused with its line. `print` appends to a `data` buffer the runner reads back through two IR functions on both paths (`suite::run_calls`, and a `text:` line from `driver.js`), and `__zero_reset()` runs before every case because the JIT keeps one instance of the program. The parser already covers the whole surface of zero.md; the lowering handles a literal and a print and says "not in this item yet" to the rest. `suite/zero/skeleton`: 2/2 native and wasm; 958 and 949 on the suite as before; two Rust tests that walked `suite/` learned to skip the `zero` directory.

### Ticks: time at the boundary — `1576d32` · 2026-08-29

```
ring_init(r, a, b, 1000000)                   ; an irregular ring on a microsecond clock: a tick per item
ring_regular(r, a, 48000, 1, 0)               ; a regular ring: item k at tick t0 + k * step, no ticks stored
push s, 1000, 9                               ; the producer's side: (tick, v)
f: u8[], k0: i64, s2: u8$ = frame(s)          ; the values, the index of the first, the reader moved on
t: time = time_at(s, k0)                      ; the boundary: a tick over the rate, exact, once
```

`probe cost`'s first number was 300 000 SSA operations for one push into a regular stream — a 128-bit rational multiply-and-add per item, filling a times buffer a regular stream never needs. "Compile out the time stuff except where necessary... or store times as integer tick-counts and only convert to seconds when necessary" — both at once, and by the library's structure rather than an analysis: a ring has a clock, a rate in hz, and its items sit at integer ticks of it, which is what the machine's counter and a device's sample index are anyway; an irregular ring keeps a tick per item and a regular one keeps none, item k being at `t0 + k·step`. `time` — exact seconds — is the boundary type: `time_at` divides a tick by the rate, `sample` and `window` multiply a time by it, once per call and never per item, and two rings on different clocks will meet by one such conversion where they meet. The per-item paths do not mention `time` at all; the biquad went from 20.9 million SSA operations to 34 thousand, a push from 326 000 to 512, most of that the slide when the ring fills. The view lost its `dt` and `t0` and is four words. Underneath, the parser learned to type a stream's literal operands properly: by the operation's arity, counted ahead to the end of the line, a parameter that is concrete gives its type (`push s, 1000, v`: an `i64` tick), one that is the container's own abstract gives the element (`push reg, 10.0`: an `f32`), any other abstract the literal's kind — in the operation form and the statement form both, which had each typed literals their own way. The OS programs push the counter's ticks straight from `now()`. 958 on every path and both variants.

---

### `probe cost`: SSA time, K, hardware time — `a88cd26` · 2026-08-29

```
$ probe cost suite/stream.ssa biquad riscv --platform=rv64i --assume=8
biquad                     20876408 ssa   K  6.92     89679648 hw
    biquad: loop at b1 x64 (declared), body 326225 ssa
    push_f32: ... copy_time: loop at b4 x8 (assumed) ...

    loop(i: i64 = 0, x1: f32 = px1, ...) bound 64 {      ; the trip count the program declares
```

"If we can estimate bounded runtime for any kernel, we can schedule computation to happen just in time... having some kind of performance metric is crucial... for any SSA function there'll be some constant K mapping SSA time to hardware time." Two numbers and their ratio. **SSA time** is the longest path through a function's IR — one per instruction, a call one plus its callee, arithmetic on a machine number (an integer to 64 bits, `f32`, `f64`) one whichever library implements it, arithmetic on a rational or a fixed the code it is — each loop multiplied by its trip count: the bound the program declares (`loop(...) bound N {`, trusted, not checked), or the count a loop shows when it steps a variable by a constant to a constant (found by running the loop in the small), or one assumed on the command line, or — reported — none, counted once. It compares two programs with no target in sight. **K** is per function per platform: the platform's cost of the function's emitted code, a `cost mnemonic = n` line per instruction in the platform file and 1 without, over its IR instruction count; and **hardware time** is the same walk weighted by K, a library operation the platform has no instruction for descended into. Every cost is 1 today, so hardware time is an instruction count and K the machine instructions per IR instruction — 1.9 for a matmul, 6 to 7 for the biquad, 9 for a five-instruction vector function whose frame is most of it; learning the costs by measurement is next. The tool's first finding was immediate: a `push` into a regular stream costs some 300 000 SSA operations a sample, because `time_of` does a 128-bit rational multiply and add — with a `gcd128` and a 128-step division inside — for every item, to fill a times buffer a regular stream never needs. The parser-independent count says so before any machine is asked, which is the point. 958 on every path; two Rust tests (a loop counted, declared and assumed; arithmetic one on floats and the code on rationals).

---

### Blocks and history: a biquad as a stream node — `6b8697d` · 2026-08-29

```
fn biquad(x: f32$, out: f32$, b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) -> f32$ {
    f: f32[], ft: time[], x2: f32$ = frame(x)
    hx: f32[] = behind x, 2
    hy: f32[] = last out, 2
    ...
        push out, y
```

The audio case from `reference/streams.md`: 48 000 samples a second arrive a block at a time, and a filter reads behind itself on both its streams by a bounded amount. `push s, block` takes a view into a regular stream; `behind s, k` is the k items before this reader's position and `last s, k` the k last pushed — the history of an input and of a node's own output — fewer at a stream's start, where `tail(h, k)` gives 0 before the beginning. So the biquad keeps no state: `y[n-1]` and `y[n-2]` are the output ring's last two, and the decay of `y[n] = x[n] + y[n-1]/2` carries from an impulse block into a silent one through the ring — 1/32 at the fifth sample, 1/128 at the seventh. The ring is sized by hand (`buffer(f32, 16)`) with a comment saying what it needs — a frame of four and two behind, six — because that number is the point: what every reader reads behind and ahead of its position, plus its lag under the schedule, is what a ring must keep resident, so capacity can be derived, double and triple buffering are numbers rather than idioms (`x[t+dt] = f(x[t])` is 2, this filter 3), and a leak becomes a deadline miss, which the `stale` check already catches. The rest of the day's conversation is in `future-work.md`: a cost per function in SSA time and, through a platform's instruction costs, in hardware time, their ratio K a map of where a platform disagrees with the IR; and a virtual clock, so a stream program runs in the suite as fast as it can with simulated time. 958 on every path and both variants.

---

### A literal after a view or a stream — `e02b3ed` · 2026-08-29

```
    x: u8 = peek s, 2                         ; an i64: the parameter's type
    p: i32 = sample a, 1.5, 0.5               ; two f64s: the literal's own kind, over `scalar`
```

In an operation form a literal takes the first operand's type — `add x, 1`, `fma x, y, 0.0` — which is right for numbers and wrong for the operations of a view or a stream, which take indices and coordinates: `peek s, 2` failed and the day's cases carried `two: i64 = const 2`. Now, after a view or a stream, a literal takes the parameter's type when every generic of the name gives that position a concrete type, and its own kind's — `i64` for an integer, `f64` for a decimal — where a parameter is abstract, which the abstract then binds; so `sample img, 1.5, 0.5` samples at `f64` coordinates. Forty lines in the parser, no capability changed; the two cases lost their `const`s. 956 on every path and both variants.

---

### A view sampled by coordinate — `d4582a6` · 2026-08-29

```
    a: i32[,] = slice p, 3, 4
    ramp(a)
    u: f32 = const 1.5
    v: f32 = const 0.5
    x: i32 = sample a, u, v                   ; between 1, 2, 5 and 6: 3.5, into i32 as 3
```

"Interpolation rules such as nearest neighbour, bilinear filter, etc. are analogous to texture samplers in the spatial domain." The same word that samples a stream by time samples a rank-2 view by coordinate: `sample img, x, y` (`lib/sample.ssa`) is bilinear, `x` along the columns and `y` along the rows, a coordinate an index so that `(0, 0)` is the first element exactly, the edges clamped — off the left and below the bottom is the corner. The weights are the coordinate's type, which is any `scalar`, and the elements any `number`: two abstract names in one signature, each bound by its own argument; every element is converted into the coordinate's type, mixed there, and the result converted back once, so a byte image at `f32` coordinates gives a byte. Thirty lines, nothing in the compiler. One thing a writer meets: `min(k, last)` over `number` is what clamps an index. 956 on every path and both variants; what is left of the streams queue is the scheduler running a graph of stream nodes.

---

### Regular streams and the linear rule — `ae8d86a` · 2026-08-29

```
    reg: f32$ = regular(s, dt, t0)            ; a sample every millisecond from 0
    push reg, 10.0                            ; the time is computed
    push reg, 20.0
    l: f32$ = sampling(reg, 2)                ; linear
    x: f32 = sample l, t                      ; 26.0 at 1.6 ms
```

"If a stream has metadata dt, then when we ask for x[t], it returns x[t/dt]." A regular stream is the same ring seen with a `dt` and a `t0` on the view — `regular(s, dt, t0)` — so its producer says `push s, v` and the time is computed, and a sample at `t` finds its item by dividing rather than searching (and the irregular search is binary now, the times being sorted by construction); the ring still holds a time per item, which is what keeps every reader's operation one function over both kinds. The third rule is linear: the two items around `t` weighed by where `t` falls between their times, done in `time`'s exact rationals — `lerp` converts the difference to a rational, multiplies, and converts back — so the only rounding is the last, into the element's type: `26.0` between `20.0` and `30.0` at 1.6 ms, and on a byte stream `2` at 2.75 ms between 2 and 3, `7` at the midpoint of a slope from 10 down to 4 — a `u8` never goes negative on the way. A push without a time on an irregular stream is a failed `check`. 953 on every path and both variants; next, the same sampling on a spatial view.

---

### The OS programs as streams — `ca775d2` · 2026-08-29

```
fn drain() {                                  ; os/echo.ssa: the UART's handler is a producer
    ...
        c: u8 = load uart
        t: time = now_time()
        push keys, t, c
}
fn sys_read(p: u64, n: u64) -> u64 {          ; the reader, kept across system calls
    keys: u8$ = load rp
    ...
            c: u8 = peek keys, i
    ...
    keys2: u8$ = advance(keys, past)
    store keys2, rp
```

The two machines' handlers are the first real producers. `os/echo.ssa`'s `__irq` pushes each byte the port has into `keys: u8$` with the time it took it — a `now_time()` over the counter since boot, as `os/sleep.ssa` has — and `read` is a reader kept between system calls in `data` (a stream is a struct, so `load`/`store` move it whole): it sleeps in `idle` until a newline is among the unread bytes, copies the line, and moves on past it, which is less than a frame, so the library grew `peek s, i` and `advance(s, n)` beside `frame`. `os/clock.ssa`'s `__irq` pushes each tick's time into `tick: time$` — the value is the time — and `__start` reads it a frame at a time, printing a tick per item and sleeping between, until ten; the tenth tick's own time is the report, `10 ticks in 1006 ms` as before. The transcripts on both boards are the ones the boot tests have checked since the programs were written; what changed is that the ring, the head and the tail that `echo` kept by hand are the library's, and that the program's loop is a reader threaded through it — the shape every stream program will have. 949 on every path and both variants; next, regular streams.

---

### Streams — `da8966d` · 2026-08-29

```
fn rules() -> (u8, u8) {
    s: u8$ = fresh()
    five(s)
    t: time = micros(3600)
    x: u8 = sample s, t
    h: u8$ = sampling(s, 1)
    y: u8 = sample h, t
    ret x, y
}
```

"A stream is a value over time — a partially resident array of `(t, v)` with monotonically increasing `t`... a bit like arrays which take `x[int]`, except streams are `x[time]`, with interpolation rules analogous to texture samplers. The metadata should live in the stream; we can have multiple streams looking onto the same data with different sampling rules. Let's use `T$` to mean a stream of `T`." So `u8$` is a reader's view of a ring: the ring is the producer's — a header saying how many items have been pushed and where the resident ones start, and two buffers, the values and their times — and the view carries the reader's position, `dt` and `t0`, a sampling rule and an edge rule; the ring's newer half slides down over the older when it fills, so the resident items are always a contiguous view and a `frame` or a `window` is a slice, chunks and all. Everything is `lib/stream.ssa` over `number$`: `push` on the producer's side, checking that time does not go back; `frame`, which gives what arrived since this reader looked and the reader moved on — a position threaded through a loop as an SSA value, which is a program over streams written as SSA; `sample` by the view's rule, nearest or hold, failing before the first item unless the view clamps; `window`, `count`, `latest`. Nine cases, three of them `-> check`: a sample before the beginning, a reader that fell behind the resident items, a time going backwards. What it took underneath was more than the library: a template may be bound by its result alone (`ring_values(r) -> number[]`), so the defining call now passes its result type into resolution; a chunk of a 128-bit `time` is the `time`; a `frame` returns fourteen words, so results past eight of a class go on the stack as arguments do; `simplify-cfg` was threading past a block whose parameter was used after it — a loop with two `break`s feeding the next loop, which the library's `window` has — and a genuine plain function with matching parameters must be called as written while a template's default instance defers to the types, which `control.ssa`'s own `sum` against `reduce`'s taught on riscv, where the driver calls by name. 948 on every path and both variants; next, the OS programs' handlers as producers, then regular streams.

---

### The frictions — `cbaf920` · 2026-08-29

```
    store 1.0, a, 0, 0                        ; a literal takes the element's type: no `1.0: f32`
    one: i32 = const 1
    y: i32 = load c, one, one                 ; an index is any integer
```

"We should also definitely address the small frictions before we move to the next idea." Three, named a few hours earlier. A literal stored through a typed pointer or a view now takes the element's type — the parser looks at the target before typing the literal, so `store 1.0: f32, a, 0, 1` is `store 1.0, a, 0, 1`. An index or a count may be any integer, converted to `i64` where it is used, so the `conv`s around a loop's `i32` go. And the one that a user would not see but that would have bitten later: an operation had come to find its definition four ways — the operation form by its first operand, the lane-by-lane path, the statement form on a view, a call by name — each the minimal fix of its day, each with slightly different rules. They are one now: `resolve(name, argument types, wanted result)` — every parameter unifies with its argument, widths and abstract types together; a defining form wants a definition with a result of its type, a statement one with none, a call either; of what fits, the most specific wins — and `dispatch` and `choose_generic` are two lines over it. No capability changed; the suite is the same 939 on every path, with its `: f32` annotations gone and an `i32` index added. What is left of the frictions is a matter of taste noted in `future-work.md`: the statement form of a view operation has no result, which is the one place the syntax has two moods, and is so by design for want of an allocator. Next: streams.

---

### The array layer, rounded out — `ae35e05` · 2026-08-29

```
fn normalize() -> (f32, f32) {
    p: ptr = addr fr
    q: ptr = addr fc
    r: ptr = addr fs
    a: f32[,] = slice p, 2, 2
    c: f32[,] = slice q, 2, 2
    s: f32[] = slice r
    store 1.0: f32, a, 0, 0
    store 3.0: f32, a, 0, 1
    store 1.0: f32, a, 1, 0
    store 1.0: f32, a, 1, 1
    sum s, a
    at: f32[,] = transpose a
    ct: f32[,] = transpose c
    div ct, at, s
    x: f32 = load c, 0, 1
    y: f32 = load c, 1, 0
    ret x, y
}
```

"We should definitely round out the array layer properly." The pieces a matrix program reaches for, each a few lines of `lib/slice.ssa` over what was there: `dot a, b` a reduction of a product — a chunk of partial products while both views are contiguous, its lanes summed, then the tail one at a time; `matmul c, a, b` the dot of each row of `a` with each row of `b`'s transpose, shapes checked, a loop until a platform has a rule for a tile; a rank-1 view on the right of a rank-2 operation broadcast to every row, so a per-row scalar is a column broadcast, which is a row broadcast on the transposes, as the kernel above has it; and `buffer_take`/`buffer_give`, a buffer with its header from the heap of the day before, returned by its own numbers. The suite grew a small kernel — rows normalized by their sums — and a case that shows a row view aliasing its grid, which is what a view is for and what my first expectation forgot. 939 on every path and both variants; the model is what a front end would generate into, and reads well enough by hand that the frictions are the next thing, before streams.

---

### Views with a shape — `ef51522` · 2026-08-29

```
fn strided() -> (i32, i32) {
    p: ptr = addr m2
    b: i32[,] = slice p, 3, 4
    fill b, 1
    t: i32[,] = transpose b
    c2: i32[] = at t, 2
    fill c2, 5
    mul c2, c2, 2
    s: i32 = sum c2
    sc: i32 = sub s, 16
    all: i32 = sum b
    a2: i32 = add all, 3
    ret sc, a2
}
```

"Views carry the shape, and `f32[,]`." A view of rank 1 to 3 is now a typed pointer and, per axis, a count and a stride in elements, so the things a matrix program wants are views of the same memory and nothing moves: `at a, i` is a row, `transpose a` the axes swapped, a column is a row of the transpose with a stride, `block` a rectangle, `reshape` a shape laid over a contiguous run — each a `Pack` of the words with a `check` on every bound, and `load a, i, j` reaches an element with every index checked. `lib/slice.ssa` grew by rank rather than by operation: the rank-1 bodies honour a stride — chunks only when every view is contiguous, a column one element at a time from the start — an operation of rank 2 or 3 is the rank below over each row, a reduction to a scalar goes row by row, and a reduction one rank down, `sum c, a`, is one per row, the columns' being the transpose's rows'. Two things the paths taught: the `rv64i` footprint test found a `mul` in the parser's hidden index arithmetic, which now multiplies by the policy's multiply like everything else; and four cases that read a grid another case had filled passed on the machines and native, where memory persists between cases, and failed on wasm and the GPU, where it does not — so every case builds its own, which is what a test should have done anyway. Nine cases in `suite/matrix.ssa`, two of them `-> check`; 934 on every path and both variants. And from this entry on, each entry opens with the code it is about, verbatim from the suite or the library — the session's entries were given theirs today.

---

### Chunks — `0ec3a49` · 2026-08-29

```
fn fill(c: number[], s: number) {
    n: i64 = len c
    pc: ptr(number) = ptr c
    vc: ptr(chunk(number)) = cast pc
    stc: i64 = stride c
    cc: u1 = cmp.eq stc, 1
    contig: u1 = cast cc
    limit: i64 = if contig {
        yield n
    } else {
        yield 0
    }
    ks: chunk(number) = splat s
    k: i64 = lanes chunk(number)
    stop: i64 = loop(i: i64 = 0, ci: i64 = 0) {
        left: i64 = sub limit, i
        m: i64 = fit number, left
        partial: u1 = cmp.lt m, k
        if partial {
            break i
        }
        store ks, vc, ci
```

The lower layer, and with it the slice operations became vector code without a line of emitter changing. `chunk(T)` is as many lanes of T as the platform's vector register holds — `f32x4` on NEON and RVV, where the platform has a vector class, `f32` itself where it has none — resolved as the fixed vector type it is, so every rule of the last three days applies to it unchanged. Three constants of a type on a platform go with it: `lanes chunk(f32)`, `sizeof f32`, and `fit f32, left`, which is `min(left, lanes)` and the one thing a library needs to say "as many as fit" without knowing the number. `lib/slice.ssa` was rewritten over them: an operation takes whole chunks while `fit` says one is left, walking memory through a typed pointer to `chunk(number)` indexed by chunk — a computed byte offset is not addressing here, a lesson relearned — then the elements one at a time to the end; a reduction accumulates a chunk, reduces its lanes, and finishes the tail. On a platform without vector registers a chunk is one element, `splat` into it is the value, and the chunk loop *is* the definition; with them it is `ld1`, `fadd .4s`, `st1` and `addv` on one machine and `vle32`, `vfadd.vv`, `vse32` and `vredsum` on the other, which `probe footprint` shows for the same suite that the other paths run one element at a time. One thing it needed: an operation's dispatch now matches the operand count, since a one-argument `min` (over a slice, or the one lane of a scalar chunk) sits beside the two-argument one. 925 on every path and both variants. What this is not, said plainly in `reference/scalable-vectors.md`: a chunk is 128 bits because the platform says so; RVV on a wider chip would say more, and the type would have to say "the machine's" for the same binary to widen.

---

### Slices — `62d01d2` · 2026-08-29

```
fn views() -> (i32, i32) {
    p: ptr = addr ia
    all: i32[] = slice p
    lo: i32[] = view all, 0, 4
    hi: i32[] = view all, 4, 4
    fill lo, 1
    fill hi, 2
    s: i32 = sum all
    n: i64 = len hi
    n32: i32 = conv n
    t: i32 = mul s, n32
    m: i32 = sub t, 9
    ret m, t
}
```

The design note on scalable vectors came back with a different top layer than it proposed, and a better one: "size should ride with the ptr". So a `T[]` is a *slice* — a view into a buffer, a typed pointer to its first element and a length, two words, several of which may look into one buffer at different places — and a buffer is memory with a header, the element size and the capacity, then the elements (`data b: buffer(f32, 1024)`, or `buffer_init` on memory the heap gave). `slice p` takes the whole buffer and checks the element size against the slice's; `view a, off, n` a part, checked to lie within; `len` and `ptr` read the words. An operation on slices is whole and writes into its first operand, written like a store — `add c, a, b`, `mul c, a, 2.0`, `fill c, 0.0`, `copy c, a` — and a reduction gives a scalar, `s: f32 = sum a`. Each is a generic of `lib/slice.ssa` over `number[]`, which the tower of the day before made a one-liner to declare, and its body — a loop over the elements, one at a time — is the definition on every path, and what the chunked form to come will be checked against. In the parser a slice is a struct, so the struct lowering and the calling convention carry it with nothing new; the new things are `[` and `]`, `buffer(T, N)` with its header laid into the data, the four instructions with their `check`s, and a statement form for a generic whose first operand is a slice. One thing it sharpened: `min` over a slice is both a reduction (a result) and an elementwise operation (none), so an operation that defines a value dispatches only to a generic with a result and a statement only to one without. Eight cases, three of them `-> check` (lengths that differ, a view past the end, a slice of the wrong element size — the GPU skips those, as it must); 925 on every path and both variants. Next: `chunk(T)` and `fit`, then the library's loop over chunks, then the rules.

---

### The tower — `f50bfa9` · 2026-08-29

```
fn min(a: number, b: number) -> number {
    lt: u1 = cmp.lt a, b
    r: number = if lt {
        yield a
    } else {
        yield b
    }
    ret r
}
```

"If we have a scalar type that is parent to int/float/etc, wouldn't we define max and min on scalars and have that single function just work on all numerics?" — and, to the suggestion of a type parameter: "no, that's what abstracts are for. It's a tower of types at the top of which is... number". So it is. `number` is over `int`, `uint` and `scalar`; `scalar` over the number libraries; each of those over its widths; and the rule that makes it work is one sentence: an abstract name is bound by the nearest thing that binds it — the policy in a program's body, the *argument* in a function's signature. `fn min(a: number, b: number) -> number` in `lib/int.ssa` is then a template, instantiated as `min_i8`, `min_f32`, `min_fixed_16_16` at each call, its body's `number` that type and its `cmp.lt` dispatching as it would on it. Where several definitions of a name take the arguments, the most specific wins — `float` before `scalar` before `number` — which is how `lib/float.ssa` keeps its NaN-propagating `min` and `max` and `lib/rational.ssa` its NaR-aware ones while fixed, decimal, unit and sunit lose their copies, and how `lib/reduce.ssa`'s `sum(v: numberx4)` covers every lane type with one body per lane count. In the parser that is: a plain `fn` whose parameters mention an abstract name bare is recorded as a generic with type parameters; `unify` binds a name to a type as it binds `N` to a width; an instance carries its type bindings through the same re-parse (`tenv` beside `env`); the opcode form and a call by name both choose an instance by unifying every parameter with the arguments — a call by name to a generic had never been dispatched at all, which is what "call to undefined function sum" had been the day before — and a template that is the only definition of its name gets a default instance under that name, the policy's binding, so a program's `fn f(a: int)` is still `f` for a directive to reach. One thing it bumped into: `suite/wide.ssa` defines its own plain `add` on `u128`, so a plain function whose signature matches the arguments is called as written before any generic of the name is considered. 917 cases on every path and both variants; three parser tests rewritten to the new rule.

---

### Across the lanes — `5857a94` · 2026-08-29

```
fn allany(a0: i32, a1: i32, a2: i32, a3: i32, k: i32) -> (u1, u1) {
    v: i32x4 = pack a0, a1, a2, a3
    ks: i32x4 = splat k
    m: u1x4 = cmp.gt v, ks
    every: u1 = all m
    some: u1 = any m
    ret every, some
}
```

The horizontals: what a vector's lanes come to together. `lib/reduce.ssa` defines `sum`, `min` and `max` of a vector and `all` and `any` of a mask as generics over the vector's shape — `fn sum(W)(v: i(W)x4) -> i(W)`, for which the parser learned to take a lane count after a parameterized type — each a pairwise tree, `(l0 + l1) + (l2 + l3)`, because that is the order a pairwise instruction takes, and so the meaning a rule is checked against: NEON's `faddp` twice over an `f32x4` computes exactly it; RVV's `vfredosum` sums in another order and `vfredmin` takes a NaN as `vfmin` does, so on RVV the floats' reductions stay the library's, as their scalar `min` and `max` do. They are written as operations with a scalar result, `s: i32 = sum v`, which took a small change: the operation form on vectors had insisted the result be a vector, and a plain call to a generic's name is never dispatched by type — it is taken on trust as external, which natively had passed unnoticed because every `sum` had a rule and the rule is chosen by signature, and on riscv became "call to undefined function sum". The rules: `addv`, `smaxv`, `uminv` and the rest reduce into lane 0 of a vector register and `umov` reads it out, so a `v` operand may now take a slot named as one of its lanes (`addv {s}, {v}.4s`); two lanes use the pairwise forms; `all` is `uminv` of the 0/1 lanes and `any` `umaxv`; a vector rule's scalar result is recorded at its real width and class now, which two of the failures were. And a thing the lane form found: a `u8x16` reduction, lane by lane, is a function of sixteen integers, and the convention had stopped at eight — so the arguments past the registers go on the stack now, in both emitters, 8 bytes each and a vector 16, the caller lowering `sp` for the call and the callee reading above its frame; the eight-parameter limit is gone, and `suite/multi.ssa` calls a function of twenty-five arguments in three classes to prove it. Twenty cases in `suite/lanes.ssa`; 916 on every path.

---

### Integer min, max, abs, neg — `ec5ed77` · 2026-08-29

```
fn absneg(a: i8) -> (i8, i16) {
    x: i8 = abs a
    w: i16 = conv a
    y: i16 = neg w
    ret x, y
}
```

The smallest of the three items left on the vector list, done first. The integers had never had a `min` or a `neg` — floats and the number libraries define theirs, and an integer `neg` was "no neg from i32: define a generic". So `lib/int.ssa` defines them the same way, as generics over `i(N)` and `u(N)` — a compare and an `if` — and they run everywhere as bodies, while a platform with an instruction substitutes it: arm64 a `cmp` and a `csel` for scalar `min` and `max` (its first integer group besides the base, `select`), NEON `smin`/`umin` and their maxes, `abs` and `neg` over every vector shape, RVV `vmin`/`vminu`, `vmax`/`vmaxu`, `vrsub.vx` from `x0` for a negation and that with a `vmax` for an absolute value. `suite/minmax.ssa` has the scalars at every width and signedness, the most negative `i8` whose `abs` wraps to itself while its `neg` widened does not, and vectors of 32-, 16- and 8-bit lanes; `probe footprint` shows the `csel`s and `smin`s on one machine and the `vmin`s on the other; 895 cases on every path and both variants.

---

### A mask in memory — `c258520` · 2026-08-29

```
fn mask4_mem(p: ptr, a0: i32, a1: i32, a2: i32, a3: i32, k: i32) -> (u8, u32) {
    a: i32x4 = pack a0, a1, a2, a3
    ks: i32x4 = splat k
    m: u1x4 = cmp.gt a, ks
    store m, p
    b: u1x4 = load p
    b0: u1, b1: u1, b2: u1, b3: u1 = unpack b
    r: u8 = mask4(b0, b1, b2, b3)
    bytes: u32 = load p
    ret r, bytes
}
```

The last of the vector "not yet"s that was small. A `u1xN` — a comparison's result, N one-bit lanes — has three physical forms: lane by lane it is N `u1` values; in a register it is 0 or 1 in each of N lanes of 128/N bits, whatever produced it; in memory the IR says a lane is at its natural place and a `u1` is a byte, so N bytes. The third had never been built, and it turned out the lane form had never had it either: the struct lowering split a stored mask into stores of `u1`, which the verifier refuses — only the GPU, which had always kept a `u1` lane as a byte, agreed with the definition. Now the lowering stores a `u1` lane as a `u8` and loads it back through `conv`, and the register form narrows its lanes to bytes on a store (`xtn`, halving each time; on RVV `vnsrl.wi` under the narrower vtype) and stores exactly N of them — a whole register, half of one, or a single lane of it (`st1 {v.s}[0]`, `st1 {v.h}[0]`; on RVV `vse8` under `vl = N`, which is what `vl` is for) — and a load widens them back (`ushll`, `vzext.vf2`). On RVV the narrowings take turns between the even temporaries, the widenings between the odd ones, for the same register-pair rule as yesterday. Four cases store and reload masks of two, four, eight and sixteen lanes and read the bytes back as an integer, the same on every path; 883 cases everywhere.

---

### The other shapes — `376a210` · 2026-08-28

```
fn bytes8(x: u8, y: u8) -> (u8, i16) {
    a: u8x8 = splat x
    b: u8x8 = splat y
    s: u8x8 = add a, b
    r0: u8 = get s, 3
    x16: i16 = conv x
    y16: i16 = conv y
    c: i16x8 = splat x16
    d: i16x8 = splat y16
    t: i16x8 = add c, d
    r1: i16 = get t, 5
    ret r0, r1
}
```

"Let's definitely do those": the 64-bit vectors, and with them the 128-bit ones of narrow lanes, since it was the same work. Both platforms' vector sections are generated from one table of shapes now — `f32x2`, `i32x2`, `i16x4`, `u8x8` at 64 bits, `i16x8`, `u8x16` at 128, their signed and unsigned forms, and the masks `u1x2` to `u1x16` — a rule per operation per shape, the arrangement (`.8b`, `.4h`, `.2s`, `.8h`, `.16b`) or the vtype (`vsetivli x0, 8, e8`) following the type. A `u1xN` keeps its representation whatever it came from — each lane 0 or 1 in a 128/N-bit lane of a whole register — so a 64-bit vector's comparison is widened (`sshll`) before the `neg` makes ones of its all-ones. Conversions between lane widths arrived too: `sshll`/`ushll` and `xtn`, `fcvtl`/`fcvtn` between `f32x2` and `f64x2`, and on RVV `vsext`/`vzext.vf2`, `vnsrl.wi`, `vfwcvt`/`vfncvt` — which hung a machine until the ISA's rule about widened operands was respected: a doubled-width operand is a register *pair* that must be even-aligned, and an allocated register may well be odd, so those rules go through the rule's temporary, `v16`. In the emitters a vector's shape — lane bits, lanes, bits in all — now comes from its type: `ins`/`umov` by lane width, `smov` for a signed narrow lane, `dup` and `ld1`/`st1` by arrangement, `vsetivli` by lanes and width, a narrow lane normalized after `vmv.x.s`. One small thing found on the way: a one-line rule had counted only its template's slots, so a template with a literal operand (`sshll {v}.4s, {v}.4h, #0`) could not be used as one; it carries the literals now. Thirteen cases joined `suite/vector.ssa` — two-lane integers and floats, 16-bit multiplies, bytes wrapping where 16-bit lanes do not, eight- and sixteen-lane comparisons, widening and narrowing both ways, byte shifts, a two-lane vector through memory — and `bytes_sum`, which had carried a `u8x8` round a loop lane by lane since the day vectors arrived, runs whole now. 879 cases on every path, under both no-vector variants; the scorecards count 51 NEON and 52 RVV templates, every one matched.

---

### Each class in its own registers — `47cd7b3` · 2026-08-28

```
fn mixed(k: i32, a: f32x4, h: f32) -> (i32, f32x4) {
    k2: i32 = add k, 1
    hs: f32x4 = splat h
    c: f32x4 = add a, hs
    ret k2, c
}
```

"There's no real reason to always go through the int registers, is there?" There was not. The convention had been a shortcut from before floats existed: every value crossed a call as bits in `x0..x7`, a float `fmov`'d out of its register and back in on the other side, and a vector, being two words, simply refused. Now each class crosses in its own registers, counted separately, the way AAPCS64 and the RISC-V ABI have it: integers and pointers in `x0..x7` (`a0..a7`), floats in `v0..v7` (`fa0..fa7`), vectors in `v0..v7` on arm64 — shared with the floats — and `v8..v15` on riscv64, where the pool moved up to `v24..v31`; arguments and results alike, eight of each. In the emitters that is one function (`abi_regs`) assigning a register by class, used by calls, indirect calls, returns and the parameter prologue, and a rule that is a function now takes its operands where they arrive, with no moves through `x0`. The one real complication was the JIT boundary: Rust calls compiled code through `fn(&[i64]) -> i64` and cannot put a float in `s0` for a signature it does not know, so the compiler generates a wrapper for every function whose parameters or results have a class — `__w_f`: integer words in, cast, call, cast, words out — when a module is compiled for the JIT or installed in the arena, and the JIT's `call` prefers it; every Rust-side caller (the suite, the fuzzer, TestFloat, `probe run`) is unchanged. A vector is a parameter, result or argument like any other value now: `suite/vector.ssa` gained three cases, an `i32x4` through a call, an `f32x4` with a float, and a mixed signature with an integer, a vector and a float in and a vector among two results out, the same on all five paths. Two things it found: on the machines a call on vectors must go to a rule only when the platform has one for that signature (a function of the program's that takes vectors is a call like any other), and the GPU emitter had been applying *every* call with vector arguments per lane — with no function taking vectors before, nothing had noticed — which for a callee typed over vectors produced bitcode LLVM refused (`llvm-dis: Invalid record`, found by the emitter's body-stub switch); a callee whose own parameters are vectors is now called as it is.

---

### RVV — `0987ac4` · 2026-08-28

```
add(f32x4, f32x4) -> f32x4
    vsetivli x0, 4, e32, m1, ta, ma
    vfadd.vv r, a, b
gt(i32x4, i32x4) -> u1x4 with t: u1x4
    vsetivli x0, 4, e32, m1, ta, ma
    vmslt.vv v0, b, a
    vmv.v.i t, 0
    vmerge.vim r, t, 1, v0
```

The same eight vector types on the other machine, and the question `vectors.md` had left for RVV — a rule with machine state — answered the simplest way: every rule sets the vtype it runs under first (`vsetivli x0, 4, e32, m1, ta, ma`, or two 64-bit lanes), and the emitter does the same before its own lane moves and loads; a pass could elide the repeats later, and nothing depends on one. `targets/riscv64.platform` gained `ext V` with `class v` and a rule per operation: `vfadd.vv` and the rest, `vfsgnjn.vv r, a, a` for a negation, `vfmacc.vv` into a copy for `fma`, shifts by a vector of counts directly, a comparison as `vmflt.vv`/`vmslt.vv` into the mask `v0` and a `vmerge.vim` of 1 over a zero vector — so a `u1xN` is 0 or 1 per lane, as on NEON — and, as for scalars there, no `min`/`max` and no float-to-integer, whose NaN cases differ from the library's. In `src/emit_rv.rs` vectors are a third allocator class in v8..v15, saved and spilled as sixteen bytes under e8, their lanes packed by a chain of `vslide1up.vx` (each slide's destination another register than its source), read by `vslidedown.vi` and `vmv.x.s`, a lane set by a mask in `v0` and `vmerge.vvm`; memory is `vle32`/`vse32` at a lane's alignment; an interrupt frame keeps the caller-saved v registers. Two things the ISA made the tools say: an instruction that reads the mask `v0` may not write `v0`, which the learner met as a rejected probe — the seed now has `reg vn = v1..v31` for such a destination, a rule's `v` operand may take a `vn` slot, and the emitters renumber; and a vector instruction with `mstatus.VS` clear is an illegal-instruction hang, so the boot preamble enables the vector unit as it does the FPU, and qemu runs `-cpu rv64,v=true,vlen=128,elen=64`. 45 templates learned, all matched in riscv-opcodes; `targets/riscv64-nov.platform` is the reference (the variants test runs the suite under it, with `rv64im` and `rv64i`, which now say `without V` too); `probe footprint` shows the vector suite using `vfadd.vv`, `vmul.vv`, `vfmacc.vv`, `vmslt.vv` and the slides. The scalable model — `vl` as a value, a loop in chunks of as many as fit, a mask for the tail — is what remains of RVV, and is noted.

---

### NEON — `2d11460` · 2026-08-28

```
class v = f32x4, f64x2, i32x4, u32x4, i64x2, u64x2, u1x4, u1x2
fadd {v}.4s, {v}.4s, {v}.4s = add(f32x4, f32x4) -> f32x4
gt(f32x4, f32x4) -> u1x4 with t: u1x4
    fcmgt t.4s, a.4s, b.4s
    neg r.4s, t.4s
```

The second half of the vector step, and the first time a whole-vector instruction is checked against the lane-by-lane meaning that defines it — the move the project made once for floats, made again for vectors. `targets/arm64.platform` gives eight vector types a register class (`class v = f32x4, f64x2, i32x4, u32x4, i64x2, u64x2, u1x4, u1x2`) and a rule per operation: `fadd {v}.4s, {v}.4s, {v}.4s = add(f32x4, f32x4) -> f32x4`, a comparison as `fcmgt` and a `neg` so a `u1xN` holds each lane as 0 or 1, `fma` as a `mov` and an `fmla`, shifts as `sshl`/`ushl` (a right shift by a negated count), casts between vectors of one width as a `mov`. The parser keeps a vector whole only where the platform has both the class and the rule — `Policy.vectors` is none, all (the GPU), or the platform's types and signatures — so a divide of `i32x4`, a `u8x8` or an `i64x4` stays lane by lane and `probe parse` shows which. In the emitter a whole vector lives in the float file, which is now saved and spilled as 128 bits; lanes move by `ins` and `umov`, memory by `ld1`/`st1`. Rule lines learned to carry arrangements (`fmla t.4s, a.4s, b.4s`) and vector-typed temporaries, and seeds to write literal braces (`ld1 {{{v}.4s}}, [{x}]`). The learner took the 76 NEON templates in a second, every one matched against ARM's own XML on the scorecard. `targets/arm64-noneon.platform` is the same target without the rules — the reference, on the same machine, which a Rust test runs the suite under — and `suite/vector.ssa` gained seventeen cases (two-lane doubles and 64-bit integers, `fma`, `min`/`max`, a NaN in a comparison, unsigned against signed compares, shifts, conversions, memory, a vector carried round a loop, ten live at once so some spill, masks combined), the same on native, riscv, arm, wasm and the GPU. Two things the machine taught, each by hanging in silence: with the MMU off every access is to device memory and must be aligned to its size, so a 16-byte `ldr q` from an 8-aligned address faults — a vector in memory is aligned only to its lanes, which is what `ld1`/`st1` want — and the data section had been 16-aligned from the start of the *code*, not the image, so with the boot preamble in front a task's stack in a `data` array sat at 8 mod 16, which the interrupt frame's 128-bit saves found where the 64-bit ones never had. Not yet: a vector as a parameter, result or argument (the two-word convention is the natural one; `fmov {x}, {v}.d[1]` is learned for it), a `u1xN` in memory, 64-bit vectors, RVV and wasm SIMD through the same seam.

---

### Lockstep, and `-> check` — `1c58088` · 2026-08-28

```
;! __kernel 64 64 -> check

fn __kernel(mem: ptr, area: ptr, id: i64) {
    x: i32 = conv id
    five: u1 = cmp.eq id, 5
    r: i32 = if five {
        yield x
    } else {
        s: i32 = simd_sum x
        yield s
    }
```

The first item from the agent's note: the GPU's silence made an error. A simdgroup operation over fibres assumed every thread of the simdgroup reached it; one that went around a `simd_sum` read a stale slot and got a number nobody would question. Now each fibre counts its puts in a word of its own — `fibre_word`, at 9728 in the thread's block, zero when the fibre starts — and after every round `simd_lockstep` walks the simdgroup and `check`s that every thread has put as often as this one. `suite/lockstep.ssa` is a kernel where thread 5 skips the sum, in groups of 64, of 32, and across two OS threads. What the test needed was a suite that could *expect* a failed check, which it could not: natively a `brk` would have ended the test process, and a machine has no `__trap` in the driver. So `-> check` is a directive's expectation now, and passes when the case ends in one: under the JIT the case runs in a forked child (`fork`, `waitpid`, no crate) that dies of SIGTRAP; on a machine it boots on its own with a `__trap` in the driver that prints `check` and ends the machine; on wasm it is the driver's `trap:` line; the GPU, which runs on past a failed check, skips such cases and says why. `suite/arena.ssa` gained two — a `check` on zero and a third allocation that does not fit. One thing found on the way: the qemu drivers were compiled at origin 0 while the image begins with a preamble, so the vector table before `__trap` was 2K-aligned in the wrong frame and arm64 hung on the first trap; the drivers compile at the preamble's origin now, as `probe boot` always did.

---

### A vector has lanes, a simdgroup has threads — `c0e3842` · 2026-08-28

```
    t: i64 = simd_thread()                        ; 0..simd_size() - 1, a thread's index in its simdgroup
    r: i64 = add h, t
```

Before NEON, one word settled. "Lane" was doing two jobs: an element of a `TxN` and a thread of a simdgroup, which had never met in a sentence until an instruction that works on four lanes at once was about to be checked by a test about a thread that skips one. So a vector has lanes and a simdgroup has threads: `simd_lane()` is `simd_thread()`, a kernel's position in its group is `tid`, and the prose in `lib/gpu.ssa`, `ssa.md`, the platform file and the suites says which it means. `simd_*` itself stays — Apple's word, and what `air.simd_sum.f32` is named after; a SPIR-V path would spell it subgroup the way AIR spells it simdgroup. History is left as it was written.

---

### Four cores — `45540f0` · 2026-08-28

A kernel's groups are dealt across cores on the qemu machines now, as across OS threads under the JIT: `;! __kernel 512 64 4` sums 512 ids in eight groups on four cores of either board, the same answer as everywhere else. `lib/core.ssa`'s `core_launch(core, rec, stack_top, main, arg)` fills a record — stack top, function, argument, the core's key for `thread()` — and the platform's `core_start` does the rest: on arm64 virt PSCI `CPU_ON` with `core_boot` as the entry, a function-body rule that sets sp and `tpidrro_el0` (writable at EL1, learned) from the record and jumps; on riscv64 virt, where every hart runs the reset vector, the boot preamble parks harts other than 0 in `wfi` until a record for them is in a mailbox and hart 0 rings the CLINT's `msip`, then sets sp and `tp` and jumps. Both boards boot with `-smp 4`; a finished core idles in `wfi`. What it took, each found by putting a letter on the UART: a rule that said `u64` where the library said `i64` and so never matched (the fallback ran, and nothing happened); qemu waking a `wfi` hart only for an interrupt enabled in `mie`; a core's own stack given the same top as its last fibre's, so the scheduler and the fibres wrote over each other; and, above all, qemu's output being lost when it is killed on a timeout, so the "output so far" of a hung run says nothing about where it hung — an hour was spent suspecting cases that had already passed. `PROBE_DUMP_DRIVER=path` now writes the whole program a machine runs, for exactly that.

---

### The simdgroup over fibres — `c6f161b` · 2026-08-28

`simd_sum` and the rest meant something on a machine only for one thread; now they mean the same as on the GPU. With fibres running a group, a simdgroup is 32 consecutive lanes and every operation is an exchange through a 64-word table in the thread's block (`simd_put`, `simd_get`, `simd_done` in lib/gpu.ssa): a lane puts its word in its slot and yields — a round, so every lane has put — reads what it needs (its simdgroup's slots for a sum, a max, a prefix sum; one slot for a shuffle; the bits for a vote) and yields again, so no lane writes over a word another has yet to read. Floats travel as their bits, integers sign-extended; the one-thread forms stand off a kernel run, the rules on the GPU. `suite/simdgroup.ssa` — each thread's simdgroup sum, prefix sum and the lane across from it in one word, in groups of 64, of 32, and across two OS threads — passes on native, riscv, arm and the GPU alike.

---

### The thread key — `cb3c50c` · 2026-08-28

The register was the wrong half of the right idea. `tpidr_el0` looked free at EL0, and a block installed in it worked — until the first preemption: XNU keeps per-CPU information there (`libsystem_malloc` reads it, which is how lldb caught the first crash) and rewrites it on every context switch, so under parallel tests a runner thread would resume with the kernel's value and `thread()` would point at 1. What macOS does leave a thread is `tpidrro_el0`: read-only, unique per thread, zero at boot. So the platform now gives a *key* (`thread_key`: `tpidrro_el0`; `tp` on riscv64; nothing on wasm), and `lib/thread.ssa` keeps a table from keys to blocks that `thread_set` fills — `thread_set_slot` for several threads at once, no two writing one slot — with the default block for a key it does not know; a GPU keeps `thread()` as its rule. The JIT no longer installs and restores anything. The suite's `;! __kernel n g m` deals a kernel's groups across m OS threads under the JIT, each thread a slot, a block and stacks of its own, all Rust-allocated; `suite/reduce.ssa` sums 512 ids in 8 groups across 4 threads, and the parallel test run that found the race passes.

---

### The current thread — `48ce834` · 2026-08-28

`thread()` answers, from any function, where this thread's own things are: a pointer kept in a register the platform names — `ext thread` in the platform files: `mrs r, tpidr_el0` on arm64 (two templates learned for it), `addi r, x4, 0` on riscv64, the header every function's `tls` names on AIR — and, where the register is zero (at boot) or the platform has none (wasm), the default block `lib/thread.ssa` declares. The block holds the fibre scheduler, its frame, the fibres' frames and done flags, which `lib/fibre.ssa` now finds through `thread()` instead of global data, and at 16 KB this thread's copy of the program's `group` items: on a machine `addr` of a group item is now `thread()` plus its offset (`lower_group_addrs`, the machine backends' one lowering), so every group in flight has memory of its own, as the GPU gives it; the default block is laid out with the group section behind it. The suite's kernel runner installs a block sized for the program. Two things macOS taught: `tpidr_el0` is not free at EL0 — the kernel keeps a per-thread value there and `libsystem_malloc` reads it — so the JIT installs the program's block only around each call and restores the host's value before Rust runs again; the design (option B, a register) stands, with that one rule about whose register it is when. 837/837 natively, the group, fibre, reduce and simd suites on every path, the arm64 scorecard clean.

---

### Fibres: a group on one thread, and a kernel as a suite case — `4e37c8c` · 2026-08-28

The last of the three: a program written for a threadgroup now runs the same on a machine, and is checked there. `lib/fibre.ssa` runs bodies by turns on one thread, each on a stack of its own — `fibres_run(count, stacks, bytes, body)`, `fibre_yield()` — round-robin, so every body has yielded before any goes on, which is what `group_sync()` needs and now does while fibres run. The switch is a platform rule of a new kind: `fibre_switch(save, to) -> () called` names every callee-saved register (`str x19, [save, 0]` ...; a rule line may name a register by number now) and is reached by a real call, so the return goes where the other fibre's saved x30 says; a fibre that has never run is a frame whose lr is `fibre_entry` and whose sp is the top of its stack. arm64 and riscv64 have it; wasm, with one stack, runs the bodies one after another (`fibre_stacks = 0`). Then the directive: `;! __kernel n g -> words` runs a program's kernel as n threads in groups of g — dispatched on the GPU, a group of fibres per group on a machine through a runner the suite adds — and compares the area's first words; wasm skips it and says so. `suite/reduce.ssa` is the reduction, right on native, riscv, arm and the GPU. On the way: data is writable under the JIT now, on pages of its own after the code, as it always was on bare metal and wasm — a program that wrote its data crashed natively and ran everywhere else, the opposite of what the paths are for.

---

### The simdgroup — `9b8630e` · 2026-08-28

`lib/gpu.ssa` names what a simdgroup computes across its lanes — `simd_sum`, `simd_product`, `simd_max`, `simd_min`, `simd_prefix_sum`, `simd_shuffle`, `simd_broadcast`, `simd_shuffle_xor`, `_down`, `_up`, `simd_all`, `simd_any`, `simd_ballot`, `simd_first`, `simd_lane`, `simd_size` — as generics over floats and 32-bit integers with one-thread bodies (the argument, the value, 0, 1), and an opcode now reaches a library generic for an integer as it did for a pack, so `simd_sum x` reads the same on an `i32`. `targets/air.platform` makes them Apple's intrinsics, whose names and shapes were read off their compiler: `air.simd_sum.f32`, `.s.i32`, `.u.i32`, `.f16`, shuffles taking an `i16` lane, votes on `i1`, `simd_ballot.i64`; a `simd_*` with no rule is an error on AIR, not its one-thread body. The lane index and width are kernel arguments in AIR, so the wrapper leaves them in a 16-byte header at the start of each thread's slab, and every function takes a third hidden parameter, `tls`, to find it. A rule's integer arguments are typed as integers now (a float is a classed argument), which the intrinsics needed. `suite/simd.ssa` on all five paths; `examples/simd.ssa` on the GPU with a test: 64 threads, the sums 496 and 1520 with the lane beside them.

---

### Group memory, typed and sized — `8303dab` · 2026-08-28

`group tmp: array(i64, 64)` declares what a threadgroup shares the way `data` declares what the program has: an array type, nothing else, reached through `addr` and the typed loads and stores — a vector element, a shape, whatever the type says — in place of the 16 KB of `i64` words at byte offsets the library gave before. On AIR every group item is laid out in one `addrspace(3)` array sized to fit, and the emitter decides which loads and stores are threadgroup accesses by a fixpoint over the function: `addr` of a group item is one, and so is whatever arithmetic, cast or block argument carries it; such an address cannot be stored, passed or returned, which the emitter says. On a machine a group item is data — and the JIT, whose data pages are write-protected, now puts the group items on writable pages of their own right after the code (`layout_data_parts`, `Compiled:: writable_from`), where the PC-relative addresses expect them. `suite/group.ssa` runs on all five paths; `examples/reduce.ssa` reads as it should. `probe parse` prints shapes again.

---

### Vectors whole to the GPU — `4a008b9` · 2026-08-27

`targets/air.platform` says `builtin vectors`, and a `TxN` now reaches the AIR emitter as one value: the parser emits a single vector-typed instruction where it made one per lane — a library call on pack lanes takes vectors for its lanes — `aggregate.rs` leaves a lane struct whole, the wide lowering knows a 128-bit vector is not a 128-bit integer, the folder leaves vectors alone, and the verifier learns the lane rules (a `u1xN` from a `cmp` on `TxN`, a call on N lanes gives N lanes back). In the emitter a vector is `<N x T>`: arithmetic, comparisons and conversions are LLVM's own on vectors, `pack`/`unpack`/`get`/`set` are insertelement/extractelement, a literal of a vector type is a constant in every lane, a rule applies to the whole vector (`fadd <4 x float>`, an intrinsic by its vector name, `air.sqrt.v4f32`, as Apple's front end spells it), and a library operation with no rule is made once per lane. `u1` lanes are a byte each in memory, as the struct's layout has them. Every other backend is untouched: 22/22 vector and typed-pointer cases on the GPU, the suite unchanged on all five paths, `probe parse x.ssa air` shows the whole form (and prints the module even when the verifier objects).

---

### A reduction on the GPU: threadgroups — `60f319e` · 2026-08-27

`fn __kernel(mem, area, id, lane, group)` is a kernel that knows its place: the wrapper passes `thread_position_in_threadgroup` and `threadgroup_position_in_grid` when the signature asks. `lib/gpu.ssa` gives every program `group_load(off)`, `group_store(v, off)` and `group_sync()` — words at byte offsets in a buffer in data, and nothing, so a program runs everywhere as a group of one — and `targets/air.platform` makes them the platform's: a 16 KB `addrspace(3)` array with an `undef` initializer (the writer's first global with one) and `air.wg.barrier(2, 1)` declared `convergent`. `examples/reduce.ssa` sums each group of 64 ids by halving at every barrier; the driver takes the group size; a Rust test runs it (`[2016, 6112, 10208, 14304]`). The suite is unchanged on every path with the new library in every program.

---

### The GPU, from our own bitcode — `3ab3971` · 2026-08-27

A fifth execution path: `probe compile x.ssa air` writes a `.metallib` — LLVM bitcode in Apple's AIR dialect inside their container, byte by byte from `src/bitcode.rs`, with none of Apple's tools in the path — and the Metal driver compiles it for whatever GPU it finds. `fn __kernel(mem: ptr, area: ptr, id: i64)` is the entry: one buffer of memory where pointers are offsets (as on wasm), `data` at zero, a scratch slab per thread, the driver's area where it says. `src/emit_air.rs` maps blocks to blocks and parameters to phis, returns several values as a struct, dispatches function values by a switch, and leaves recursion out with a reason (`probe test air` counts it skipped: 15 cases). What Apple's compiler taught us, each by a bisection: it crashes on an `or` with a wide constant at an odd width, so every integer now lives normalized in an 8/16/32/64-bit container like wasm's; it compiles every function in a module, so only what the kernel reaches is emitted; and with inlining left to its judgement it miscompiled a 128-bit division — upstream LLVM ran the same bitcode right, so did `noinline` — which `alwaysinline` fixes and speeds up six times over. Platform rules with no operands (`fadd = add(f32, f32) -> f32`, `air.sqrt.f32`, `air.fma.f32`) are Apple's instructions; `targets/air.platform` has f32 and half. The suite runs 804/804 on the M1 Max in 33 s, with a Rust test; `probe testfloat air` runs TestFloat a thread per vector — 6.1M `mulAdd` cases in one dispatch, 19.4M in all: the library exact at every width, the f32 instructions missing only where the GPU flushes a denormal (116,504 of them, counted apart; Apple's compiler has no switch, and half keeps its denormals); `probe fuzz --air` puts the GPU in the fuzzer's panel. Along the way: bitcode value ids follow written order and blocks are written in the order first entered (LLVM forward references need types we don't track); function attributes; `i64::MIN` as a signed VBR; the Python driver zeroes its buffers, which Metal does not promise; `sleep_boots` now places each wake in the frame its printed lateness says it ran in.

---

### Typed pointers and shaped arrays — `17d787c` · 2026-08-27

The other half of the type work the GPU asked for. `ptr(T)` is an address that knows what it points at — a scalar, a vector, a struct, or `array(T, W, H, ...)` with a shape — so `load g, i, j`, `store v, g, i, j` and `index g, i, j` take indices, as many as the shape has dimensions, and check the element; `ptr` stays what it was, bytes at any step. An array is a memory type and never a value: it is what a typed pointer points at, what `scratch` sizes itself by when its result is typed, and what `data` declares, now with a shape. The lowering is the parser's: the shape makes the offset (row-major, innermost first), the element makes the step, a hidden cast to `ptr` keeps the printed program re-parsable, and the multiply goes to the library on a core without one — the `rv64i` footprint test caught that within the hour, as it did for vectors. No backend changed except one line: on wasm a typed pointer is an `i32`, like `ptr`. Textures are deliberately not arrays; they will be handles with platform operations. Nine cases on four paths; the AIR emitter is next, with `<N x T>` and typed pointers to lean on.

---

### Vectors, `TxN` — `6590476` · 2026-08-27

Before the GPU emitter, the type it will lean on. `f32x4`, `i32x8`, `u1x4`, `floatx4` with the policy's float, `intxN` in a generic: N lanes of one type, spelled the way one says it, and `Tx1` is `T`. A vector is a struct whose fields are its numbered lanes, so building, splitting, indexing, loading and storing one are the struct's operations already there; what is new is that `add`, `cmp.gt`, `conv` and `sqrt` on a vector mean the scalar operation on each lane — a definition the IR makes itself, as it does for integers wider than a word, rather than a library's, because "per lane" is structure, not arithmetic. The parser writes a vector operation out lane by lane and the struct lowering makes the packs and unpacks free, so vectors run on every backend today, verified against nothing but the scalar operations they are made of; a platform with vector registers may later keep a type whole and take the operation in one instruction, checked against this meaning — the choice between one register and many lanes being the platform's, which is what the architectures disagree about. One thing caught by the variant test: a `mul` lane on a core without a multiplier must go to the library like a scalar `mul` does.

---

### A metallib by hand — `0dc7a3d` · 2026-08-27

The GPU thread starts where the memory thread ended: on this Mac, and on the M6 that is not announced yet. Apple's GPUs have no public ISA and a different one each generation; what Apple keeps stable is the bitcode its shader compiler emits (AIR: LLVM bitcode with typed pointers, address spaces, `air.*` intrinsics and metadata naming a kernel's arguments), which the driver compiles for whatever chip it finds. So that is the binary, and the goal is to produce it with none of Apple's tools in the mix. The analysis went as it did for ARM: the Metal toolchain was fetched, a catalog of tiny kernels compiled, and what came out read with LLVM's own bcanalyzer and disassembler (`tools/probe-air.sh` keeps the record). Then `src/bitcode.rs`: the bitstream and the `.metallib` container, written from the public format and the observed records — and `add1`, built by hand in a test, disassembles under upstream LLVM and runs on the M1 Max. Two things learned the hard way: upstream LLVM 19's own encoding of the same module crashes Apple's compiler service (the format must be theirs, record for record), and a bitcode the driver has not seen validated by `llvm-dis` first is a way to crash it again — so the GPU only ever sees bytes LLVM has accepted. Also that the string table is a top-level block after the module, which cost an hour.

---

### Input by interrupt — `87ee51b` · 2026-08-27

`os/echo.ssa` no longer polls. `uart_irq_on` asks the board for an interrupt per received byte, `__irq` drains the port into a ring, and `read` sleeps in `idle` until a line is in the ring, so between keystrokes the machine does nothing at all; the last line says how many interrupts the input took (one or two: qemu hands a pipe over in bursts). The board side is the platform's again, and it exposed the one asymmetry between the two machines that a kernel has to know about: riscv64 delivers every device interrupt as a single cause, "external", and names the source only at its controller, so the platform gives `irq_claim()` for the source and `irq_done` completes it; arm64's controller names the source in `irq_ack` itself, and `irq_external` there is an id that never arrives. One kernel serves both. Found on the way, by a storm of interrupts before the first prompt: riscv's `irq_on` had been enabling the timer's interrupt while `mtimecmp` still held its reset value of zero, which the earlier kernels never noticed because they armed or disarmed the timer first. Arming the timer enables its interrupt now, as arm64's control register always did, and `irq_on` leaves the timer alone.

---

### A decision: no lifetimes in the pointer type · 2026-08-27

Considered and declined: regions in the pointer type — `ptr frame`, `ptr call`, and a verifier rule that a pointer never goes where something longer-lived is expected. It would have been small and static, and it is what Cyclone and Austral do. But the IR is a target, and whether a pointer outlives its memory is the contract of whatever generates the IR, which is expected to be smart enough never to do that — as it is expected never to emit a use before a definition it cannot see. The verifier checks the IR's own well-formedness; the front end's discipline is the front end's. Unique pointers go the same way, for the same reason.

---

### The heap is the machine's memory — `74bb298` · 2026-08-27

"Shouldn't the heap come from the data memory?" It did — a declared array of 256 KB, a stand-in sized by hand, which is the kind of number the rules want out of a program. What the machine actually has is 128 MB with the image at the bottom and the boot stack a few MB up; everything above is a program's to carve. So the platform files say where that is (`heap_base`, `ram_end`) and `os/sleep.ssa`'s heap is those 112 MB, in 4 KB units so the split tree stays small — the unit is now `heap_init`'s to choose. Under the JIT and wasm a heap stays over a declared array: those are not machines, and there is no rest of RAM to have.

---

### The root: a heap for regions — `25724a3` · 2026-08-27

The heap after all — but as the root the rungs are carved from, not a `malloc` for objects, which is seL4's shape and what keeps it inside the rules. `lib/heap.ssa` is a buddy allocator over one declared block: pieces are powers of two aligned to their own size, taking is a split and giving back a merge, a byte per node of the split tree makes a double give or a wrong size a failed `check`, and `heap_seal` lets a kernel forbid allocation inside an interrupt — the discipline, stated: objects live in the rungs, the rungs come from the heap, nothing allocates from the heap in a handler. `os/sleep.ssa` now carves its stacks' pool and its frame arenas from one at boot, seals it in `__irq`, and the one-shot task takes a region of its own, uses it through an arena, and gives it back: `66560 heap bytes out` at the end, the task's 4 KB gone home. Two harness lessons: the qemu driver was writing test buffers a store per byte, so ten 8 KB zero buffers became a megabyte of code and a call outran `jal` on riscv — RAM starts zeroed, so zeros need no store; and a timing test must not share the host with the regression suites, so everything that runs a machine now takes turns.

---

### The second rung: pools, and tasks that come and go — `e762343` · 2026-08-27

`lib/pool.ssa`: fixed-size slots over declared memory, taken and given back in any order, a free list threaded through the free ones and a flag per slot so that giving one back twice is a failed `check`. What it unlocks is the thing that had been waiting on it: a task that exists for a while. In `os/sleep.ssa` stacks come from a pool of four, tasks have a state, and `run_at(t, f, arg)` spawns a one-shot task — the half-second callback spawns one for 63/100 s — that sleeps to its time, runs, marks itself done and asks to be rescheduled; the scheduler hands its stack back once it is no longer the interrupted one, and the report ends `3 stacks out`. One `check` did its job on the way: a wrongly written slot search (`check over` where "a slot remains" was meant) stopped the machine at the callback, with the address, rather than running on into a spawn that never happened. And a lesson about what a test may demand of a machine: two events within its wake-up latency of each other merge into one interrupt and decide which of two lines prints first, so the count and the interleaving are the machine's; the wakes, the frames' contents and the stacks are the program's, and those are exact.

---

### The first rung of memory: arenas, and `check` — `7eff7ca` · 2026-08-27

The memory design session (`reference/memory-management.md`) came out where the safety-critical rules and the game engines both stand: no heap, a ladder of lifetimes instead — a call's (`scratch`), a frame's, an object's, the machine's (`data`) — each a bump or a free list over memory the program declared, each with a declared capacity, the only failure exhaustion at a known site. This is the simplest rung that is not wrong. `lib/arena.ssa`: bump allocation over any memory, sixteen-aligned absolutely, reset all at once, marks for the stack flavour. `check c`: the assertion — the one instruction memory asked of the IR — a breakpoint trap (`brk`, `ebreak`, `unreachable`) that reaches `__trap` with a cause and the address of the check, and what an arena does when it runs out rather than hand back a pointer nobody tests (`os/check.ssa`). And the frame allocator from GPU engines, in the OS: `os/sleep.ssa` keeps two arenas, the scheduler writes a record into the current one at every switch, a callback flips them every tenth of a second after checking the consumer has finished, the idle task prints each closed frame and resets it. Two lessons kept: a record is a *switch*, not a pick (a task re-picked after a callback's reschedule is not a wake), and events that fall within one machine's wake-up latency of each other merge into one interrupt, so the callbacks moved to times of their own and the count is exact: thirty-one events, thirty-one interrupts. Not built, on purpose: pools, regions in the pointer type, the capacity analysis — each sits on this and changes none of it.

---

### `at` and `after` — `fba9011` · 2026-08-27

The question was whether "at a time, do this" wants an instruction in the IR. It does not: a time is a library type and a callback is a function value, both already there, so `at(t, f, arg)` and `after(d, f, arg)` are two kernel functions — a queue of `(time, fn, arg)` in `data`, due entries run inside `__irq` before the scheduler picks a task, pending ones counted when the timer is armed. Exact and cheap, hence short and never sleeping; the task-level kind, which may sleep, waits for the memory design session because it needs a stack each. In `os/sleep.ssa` one callback is set half a second after boot and registers another at exactly a quarter of a second after its own time: `!` prints before `a 500000` on the interrupt they share, `!!` 250000 µs later to the microsecond, nineteen interrupts. `after` returns the time it registered, so a callback reports what was scheduled rather than the late moment it runs at. Two places time might later touch the compiler, noted and not taken: closures, so `at t { ... }` could capture; and static bounds on how long the code it emits takes, which is what would let a callback's deadline be checked rather than hoped for.

---

### Sleeping tasks and a tickless timer: `os/sleep.ssa` — `ada2d1f` · 2026-08-27

The fifth operating system: three tasks that sleep until exact times, every 1/10, 1/3 and 1/7 of a second, each deadline a `time` — a rational — so that 3/3, 7/7 and 10/10 of a second are one instant and the three wake on one interrupt, in index order. Twenty wakes, in the order the fractions say; eighteen interrupts, because the timer is armed to the next deadline rather than to a tick and a shared deadline costs one. A task going to sleep asks the machine for an interrupt — `reschedule()`, riscv's msip or an SGI on the GIC — so every switch still happens in `__irq`. Two lessons: a timer left armed in the past fires again the moment interrupts are enabled (both machines stormed until the scheduler learned to disarm it when nothing is pending), and the harness now keeps the output a machine produced before timing out, which is how that was seen. Lateness is qemu's wake-up granularity, milliseconds; on a board it would be microseconds.

---

### A decision: everything at kernel level · 2026-08-27

Considered and declined, for now: user mode. It would need no IR change — privilege is machine state the trap frame carries, so it is two more platform rules (the saved status register, `mstatus`/`spsr`) and a kernel stack at handler entry (free on arm64, an `mscratch` swap on riscv) — but what it buys is containment of one's own bugs, and only with memory protection (PMP; on arm64 the MMU and page tables), at the price of a trap per service, copies across the boundary, and a kernel/user split of every name. This is a single-user machine built by its one user, in the tradition of Oberon and the language-safe systems: one program, compiled together, every function in the verifier's sight. So: one level, "system calls" are calls, and two conventions keep the door open — only the handlers touch machine state, and the kernel's services are a table of function values, so a component that ever needs isolating can be moved behind a real trap without redesigning the rest.

---

### Preemptive tasks: `os/tasks.ssa` — `7f42d6c` · 2026-08-27

`probe boot os/tasks.ssa` prints `ababababab`: two tasks, preempted by the timer, a slice each — the fourth operating system. The mechanism is one signature: an interrupt handler written `fn __irq(sp: ptr) -> ptr` is handed its frame, where the interrupted code's registers now are, and returns the frame to go back from; the epilogue switches the stack pointer to it before restoring and returning. So a task is a stack and a place to resume, a switch is two stores and two loads around `resume()`/`resume_at()`, and a task that has never run is zeros for a frame and its function's address. The first try printed `abbbbbbbbb`: the handler had kept only the caller-saved registers, correct for returning to the same task and wrong for a switch, where the whole file belongs to the task — the callee-saved registers, float ones included, are kept too for this form now.

---

### Interrupts and the timer: `os/clock.ssa` — `664a7fc` · 2026-08-27

`probe boot os/clock.ssa` prints `tick` ten times, a tenth of a second apart, then `10 ticks in 1006 ms` (1010 on arm64): the third operating system keeps time. Interrupts land in `fn __irq()`, which `probe boot` compiles with a frame that keeps *every* register of the interrupted code, float scratch included, since an interrupt lands between any two instructions; both machines get a sixteen-entry vector table before `__trap` — arm64's IRQ entries branch to `__irq`, riscv64's mtvec goes vectored with entry 0 for exceptions and the rest, by cause, for interrupts. What the board does is the platform file's, as before: `now` and `hz` (the generic timer's counter and frequency; the `time` CSR at 10 MHz), `timer_at` and `timer_off` (`cntp_cval_el0`; the CLINT's mtimecmp), `irq_on` (the GICv2's distributor and cpu interface and `daifclr`; mie and mstatus), `irq_ack` and `irq_done`, `idle` (`wfi`). Those rules needed registers of their own for addresses and values, so a rule may now declare typed temporaries — `irq_on() -> () with gic: ptr, v: u32` — and `none` is a rule that does nothing. Each deadline is one step on from the previous deadline, never from "now", so the ticks do not drift; and the elapsed count becomes milliseconds exactly through `lib/time.ssa`'s rationals — a period times a count — which is the point: getting time right starts at the bottom.

---

### Dominance, fall-through, scratch — `147fe74` → `ad80a53` · 2026-08-27

Three items from the list. *Dominance* (`147fe74`): the verifier's rule 7 — every use is dominated by its definition, earlier in its block or in a block every path from the entry passes through; the dominator tree moved out of the wasm structuring into `structure::Dom` so both share it. It found that `addr` and `platform` results had never been on the verifier's definition list at all. *Fall-through* (`30430b9`): a jump to the block laid out next is not emitted, and a conditional branch whose taken side is next is inverted over the other; no relaxation pass was needed because block offsets were always patched after the whole function. *Scratch* (`ad80a53`): `p: ptr = scratch 64` is memory that is the function's while it runs — its frame on arm64 (`add x, sp, #imm`, newly learned) and riscv64, a shadow stack in linear memory on wasm (one mutable global, `global.get`/`set` newly learned) — 16-aligned, one area per instruction, one per activation. Until now every byte a program touched came from its caller or from `data`. Memory that outlives a call is deliberately not started: `future-work.md` says it needs a design session first.

---

### Traps and system calls: `os/echo.ssa` — `504879b` · 2026-08-27

`printf 'hello\nbye\n' | probe boot os/echo.ssa arm` and the machine answers `> echo: hello` then `> ` and stops: the second operating system. A kernel installs a trap handler and serves write, read and exit through a `data` table of function values (yesterday's feature, put to its purpose; `data` may now hold pointers and function values), and a program of nothing but system calls prompts, reads a line from the serial port and echoes it. The handler is `fn __trap(a, b, c) -> u64`: `probe boot` compiles it with a frame that keeps every register of the interrupted code and a return by `eret`/`mret`, and on arm64 lays a 2K-aligned vector table of branches before its entry; it is called with the trapped code's first three argument registers and its result replaces the first, so `syscall(n, a, b)` on one side meets `__trap(n, a, b)` on the other. How a machine takes a trap is the platform file's business: `vectors`, `cause`, `resume`, `resume_at` and `syscall` are functions whose bodies the platform supplies — `msr`/`mrs`/`svc` on arm64, `csrw`/`csrr`/`ecall` on riscv64, ten instructions newly learned — plus two constants for what a system call looks like and how far past it to resume, and three for the UART's receive side. Rule lines may now spell a template's fixed operands (`msr vbar_el1, t`). Not yet: device interrupts, which can land with float scratch registers live.

---

### Function values — `fd64eaa` · 2026-08-27

A function is now a value. Its type is its signature, spelled as the function declares it — `fn(i64) -> i64`, `fn(ptr, i64)`, `fn(i64, i64) -> (i64, i64)` — so a value carries everything the verifier needs; the same `addr` that reaches a `data` item makes one, and a call through it is written exactly like a call by name. The value goes wherever a value goes: parameters, results, block parameters (a reducer carried around a loop), memory (a table of handlers, built with `store`), `cast` to its bits. arm64 does it with `adr` and `blr`, riscv64 with `auipc`/`addi` and `jalr` — all already learned; wasm needed one new template, `call_indirect`, learned through a seed that declares a table and 130 identical types to range over, plus a table and element section listing the address-taken functions. In the incremental arena a value is the callee's trampoline, so it survives edits and promotion. Seventeen cases in `suite/indirect.ssa`, on all four paths under every policy and variant. This is the piece the OS needs next: trap and syscall tables.

---

### hello world ᕦ(ツ)ᕤ — `16cfa2b` · 2026-08-26

`probe boot os/hello.ssa` (or `... arm`) and qemu's serial port says `hello world ᕦ(ツ)ᕤ`: the first operating system written in probe, on both bare-metal machines from one source. It needed four small things the IR did not have: `data` — a string is an array of UTF-8 bytes, initialized memory laid out after the code and reached PC-relative (`adr` and `auipc` newly learned); `addr` and `len` on it; `platform uart`, a constant the platform file provides per board; and a way to end the machine — riscv's finisher is a store, arm's PSCI is a `hvc`, which the platform supplies as the body of a plain function. Along the way: with the MMU off aarch64 faults on an unaligned 64-bit load, and the image's preamble had left the data four bytes off.

---

### ISA variants — `2524621` · 2026-08-26

A platform file is now grouped by extension, and a variant is three lines: `target riscv64`, `base riscv64`, `without M, F, D`. Dropping F and D makes every float operation the library's; dropping M makes `mul`, `div` and `rem` library calls too (a shift-and-add `mul(W)` joins the division generics), the wide lowering included. `--platform= rv64i` selects a core for every command, and the same 736-case suite passes on qemu for `rv64im` and `rv64i`, and natively for `arm64-nofp` — slower, unchanged answers. `probe footprint` decodes what a program actually used against the learned templates, and a test proves the `rv64i` build of the whole suite touches nothing from M, F or D (it caught the emitter's own multiply in a struct stride). ARM's variants have their slot; only the no-FP one is populated.

---

### a look outward — `5be74a6` · 2026-08-26

`vectors.md`: a survey, before any vector work. The learner's assemblers turn out to reach NEON, SVE and SME, RVV, wasm SIMD — and AMD's GPU ISAs, matrix cores and fp8 included, so a GPU is a target probe could *learn* today even though nothing here could run it. Five execution models, from fixed SIMD to dataflow, measured against what the project already has; the AI format explosion (fp8 E4M3, fp6, fp4, MX blocks, NVFP4) as libraries; and an order: fixed-width vectors as a type and a register class first.

---

### struct — `b38526d` · 2026-08-26

`type point = struct { x: f32, y: f32, z: f32 }`. A `pack` is bits; a `struct` is fields side by side — at natural offsets in memory, as separate values in registers — and never a bit pattern: no `cast`, no literal, no arithmetic. That one refusal is what leaves the layout to the compiler. Right after parsing every struct value dissolves into its fields (`src/aggregate.rs`): `pack`, `get`, `set` and `unpack` become names for values that already exist, so `get (get l, to), z` on a line of two points compiles to nothing at all; a `load` or `store` becomes one per field at its offset, and `load p, i, 12` walks an array of 12-byte structs. The suite passes a struct as its fields.

---

### decimal — `a54cdd5` · 2026-08-26

`decimal(N, S)`: an `i(N)` significand at scale 10^S, so cents add exactly and 1000 × 0.10 is 100.00; `mul` and `div` round half away from zero in 128 bits. Written as a test of `formats.md` — the recipe held, with one addition: a constant like 10^S has no width-expression form, so it is a small generic helper (`pow10(S)()`) that the const-folder folds away.

---

### wasm without the dispatcher — `7f3fb19` · 2026-08-26

The wasm emitter used to turn every function into one big loop with a `label` local and a chain of `br_if`s — a switch pretending to be control flow. Now `src/structure.rs` computes the dominator tree and the emitter nests from it: a loop header becomes a `loop`, a block that several paths reach becomes a `block` ending where it starts, conditionals are `if`/`else`, and branches are `br` to a label or the target emitted in place. Structured source (`if`/`loop` sugar) and the flat block form both produce reducible graphs, and the passes keep them so; a test checks the whole suite at every level. The dispatcher is kept only for an irreducible graph, which another test constructs by hand.

---

### the recipe for a format — `40a0b66` · 2026-08-26

`formats.md`: how to add a number format, in seven steps, with `time` as the example — a type, generics named after opcodes, `conv` for literals, a suite file checked against an independent oracle, and only optionally a policy family or platform rules. `/format <name>` scaffolds one. The point of the document is what it does not contain: no compiler changes.

---

### time — `13514bd` · 2026-08-26

`lib/time.ssa`: `type time = rational(64, 64)`, an exact number of seconds, with `seconds`/`millis`/`micros`/`nanos`/`period` to make one and `to_*` to read one back. There is no arithmetic in the file — `add`, `mul`, `cmp` on a `time` are the rational library's by dispatch — so a sample period at 44100 Hz times 44100 is exactly one second and thirds and sixths add to a half. What made it possible: the rational library now works in 128 bits, so its parts can be 64 bits wide.

---

### addressing modes — `9220416` · 2026-08-26

`v: i64 = load p, 16` and `v: i32 = load p, i, 4` (base + index × step), and the same on `store`. The SSA's memory model is unchanged — values are named registers without limit, `load` and `store` are the only two memory instructions, spilling is the allocator's business — but the address forms are now the ones the targets' instructions take, so an offset no longer costs a `ptradd` first.

---

### float registers, and rules as single instructions — `3325600` · 2026-08-26

The first rule files spelled every float op with moves around it — `fmov s0, a` / `fadd s0, s0, s1` / `fmov r, s0` — because every value lived in an integer register. Now a platform file declares where a type's values live and maps one instruction to one operation:

```
class s = f32
fadd {s}, {s}, {s} = add(f32, f32) -> f32
```

The allocator has register classes (a linear scan per file, spill slots shared), the three emitters keep float values in float registers (arm64 `v8..v15`, riscv64 `fs0..fs11`, wasm `f32`/`f64` locals), and a chain of float operations compiles to just the instructions; a move between files happens only where a value really changes class. Types in rule files are written by their program names — `f32`, not `float(8, 23)`. Composite rules keep the indented form (`fcmp a, b` / `cset r, lo`).

---

### binary128 from the same library — `85be481` · 2026-08-26

`type f128 = float(15, 112)` and every float operation works, from the generic bodies that already served fp8 to f64 — nothing in `lib/float.ssa` knows about 128 bits. What it took: constants that hold 128 bits (`const 1 << 112` used to wrap), and the few places the library built products or shifted significands in hand-split `u64` pairs now just name a type wide enough (`u(2 * M + 10)` for a product) and let the lowering make words of it. `fixed` and `unit` multiply and divide in `u128` the same way, and the old 128-bit helper functions are gone. `suite/f128.ssa` checks add, sub, mul, div, sqrt, fma and conversions against exact rational arithmetic rounded at 113 bits.

---

### wide values — `4357f56` · 2026-08-26

`i128`, `u256`, a 136-bit pack: any integer or pack up to 256 bits. Nothing changed in the backends. A wide value is checked as written, then lowered to a row of 64-bit words right after parsing (`src/wide.rs`) — carry chains for `add`/`sub`, schoolbook products for `mul`, word arrangement for shifts (a branchless logarithmic select when the amount is a runtime value), lexicographic compares, sign-filled extensions, masked field access for packs, word-by-word memory. `div` and `rem` are the exception: they dispatch to `lib/wide.ssa`'s `div(W)`/`rem(W)`, restoring-division loops written in SSA over the wide type itself and lowered like anything else. A `u128` parameter is two word parameters and a `u128` result two results, which is what the suite directives give and expect (`suite/wide.ssa`, 130 cases against Python's integers, plus 400 random rows against Rust's `u128` in a test).

Two things surfaced by the first 256-bit multiply: the register allocator now shares spill slots between values whose lifetimes do not overlap, and the suite's bare-metal driver is one function per case.

---

### platforms as rule files — `3181412` (+ `7032ac7`, a test-race fix) · 2026-08-26

`targets/arm64.platform`, `riscv64.platform`, `wasm32.platform`: what a target does natively, as text. A rule is a library instance's full signature and the learned templates that compute it —

```
add(8, 23, 0)(a: float(8, 23), b: float(8, 23)) -> r: float(8, 23)
    fmov s0, a
    fmov s1, b
    fadd s0, s0, s1
    fmov r, s0
```

— with `a`/`b`/`c` the arguments, `r` the result, `s0`/`d0`/`f0` scratch float registers, and literals for immediates and conditions (`cset r, lo`). Each line resolves against the learned encodings by mnemonic and operand shape, so a rule can only name instructions the learner verified, and a line it has no template for is an error. The three Rust tables and the `Native` enum they hung off are gone; an emitter keeps only the register assignment. Adding a native op is now an edit to a text file, and a target whose ops take several instructions (a stencil) is the same kind of edit.

---

### encoding scorecards — `0afc68f` · 2026-08-26

The learner derives encodings from an assembler's bytes and never reads a manual; `probe scorecard` is the manual, afterwards. It checks every learned template against the official inventory of its target — Arm's Machine Readable Architecture XML, riscv-opcodes, wabt's opcode table — by decoding the template's fixed word to an official encoding and requiring the learned fields to sit inside that encoding's operand fields: `add {x}, {x}, {x}` is `ADD_64_addsub_shift` with `Rd`, `Rn`, `Rm`; `beq` puts its scrambled immediate exactly in `bimm12hi+bimm12lo`. All 150 arm64, 90 riscv64 and 125 wasm32 templates pass. The cards (`targets/*.scorecard.md`) also count what the inventory has that is not learned, by group and by mnemonic, which is the to-do list for the seed files.

---

### rounding modes, and TestFloat as the oracle — `b6a7bae` · 2026-08-26

The library's float operations gain a third width parameter, `round`: 0 nearest even, 1 toward zero, 2 down, 3 up, 4 nearest away. Only `fpack` ever rounds, so that is where the modes live — the rounding itself, overflow to infinity or to the largest finite depending on the side, the sign of an exact zero — and `add`, which used to round by hand, now goes through it too. A generic parameter nothing binds is filled by name from the enclosing instantiation or the policy, so `add x, y` and `add(8, 23)` keep working, `--round=up` changes a whole program, and `add(8, 23, 2)` pins one instance (`suite/round.ssa`). Platforms only claim the nearest-even instances.

`probe testfloat` runs Berkeley TestFloat's vectors through the library and the hardware: 19.4 million cases per mode across f16/f32/f64 and every operation, and every one of the five modes comes back 0 wrong. `tools/get-testfloat.sh` builds the generator.

---

### the fuzzer — `2978f8c` · 2026-08-26

`probe fuzz [count] [--seed=hex] [--slow]`. Programs are random but well-formed by construction — every integer width, packs, floats via the library and the platform, value-yielding `if`s, bounded loops, calls between functions — and built so they can't fail for a boring reason: divisors are `or`ed with 1, shift amounts are literals under the width, floats come from integers so no NaN payload reaches the hardware. Native `-O0` with the platform off is the reference; every optimization level, the platform, wasm, and (`--slow`) both qemu machines are referees. Disagreements are kept as suite files under `target/fuzz/`, and a printed seed reproduces its program alone.

First catch, within 300 programs: wasm's `div_s` traps on `MIN / -1` where the IR says wrap. The wasm emitter now guards it arithmetically (divide by `rhs + 2m`, `m = rhs == -1`, then conditionally negate).

---

### rational, scalar, and literals everywhere — `55a2f74` · 2026-08-26

`lib/rational.ssa`: `numerator / denominator`, reduced, a zero denominator being "not a rational"; exact while it fits, and `conv` from a float by continued fractions (`3.14159f32` is `22/7` at 8 bits). Then `scalar` — a bare name the policy points at one of `float`, `fixed`, `rational`, `unit`, `sunit` — and one program that runs unchanged in all five. What made that work: a literal on any library number type is read as an `i64` or `f64` and handed to that library's own `conv`, so `mul x, 0.5` and `sub 1, x` mean the same thing in every family without the compiler knowing any of them. 448/448 on all four paths, both ways.

### unit and sunit — `4ac4a9e` · 2026-08-26

`lib/unit.ssa`: `unit(N)` runs 0.0 to 1.0 over 0 to 2^N−1, `sunit(N)` −1.0 to 1.0 over ±(2^(N−1)−1). The scale is not a power of two, so a product is `(a·b + half) / max`, rounded; sums saturate; `conv` goes through floats. The two-word helpers move to `lib/wide.ssa`, gaining a `udiv128` that fixed and unit share. Bare `unit`/`sunit` follow the policy (`--unit=N`, `--sunit=N`). Exhaustive against their models at 8 bits; 413/413 on all four paths, both ways.

### fixed point — `633d681` · 2026-08-26

`lib/fixed.ssa`: `fixed(I, F)` as `pack { frac: u(F), int: i(I) }` with the arithmetic, comparisons, and conversions to and from integers and floats, all in integer instructions; and a bare `fixed` resolved by the policy (half the `int` width each side, `--fixed=I,F`) exactly as `float` is. Two libraries sharing `add`, `mul`, ... meant two parser adjustments: declarations may name types the prelude declares later, and by-name instantiation must be unambiguous (the float suite's aliases are wrappers now). 367/367 on all four paths, both ways.

### The abstract float, and a prelude — `32511ac` · 2026-08-26

`float` joins `int`: a bare `float` is `float(E, M)` for the policy's width — f64 on the register machines, f32 on wasm, `--float=f16|bf16| f32|f64|E,M` to choose — instantiated by the parser, since `float(E, M)` is the library's type, not the compiler's. `fn half(x: float) -> float` is written once and lands on the library or the platform's instruction at whatever width the policy picks. The float library is now `lib/float.ssa`, appended to every program as a prelude. `suite/afloat.ssa` runs the same programs at four widths; 337/337 on all four paths, both ways.

### min, max, fma — `da83428` · 2026-08-26

`min`/`max` (IEEE minimum/maximum: NaN propagates, -0 below +0) and `fma` with a single rounding: the exact product and the addend meet in a two-word accumulator with eight guard bits, built from a few `add128`/`shr128`-style helpers written in the SSA itself. On the platforms: `fmin`/`fmax`/`fmadd` (arm64), `fmadd` (riscv64, whose `fmin` drops NaNs and so stays in the library), `f32.min`/`max` (wasm, which has no fma). Bit-exact against `mul_add` and an exact reference. 327/327 on all four paths, both ways.

### The suite, with the sugar — `c1d472d` · 2026-08-26

The suite and the float library rewritten with literal operands: 139 named constants gone, 374 lines shorter, `cmp.ne ma, 0` and `pack 0, 0, sz` where there were `zero_m` and `zero_e` declarations. A mechanical pass, so the IR underneath is byte-for-byte the same and the matrix is unchanged: 302/302 on all four paths, both ways.

### const by type, literals as operands — `9b05c3e` · 2026-08-26

`iconst` is `const`, and the type decides: bits for an integer or a pack, a number for a float — `x: f32 = const 0.1` is the nearest f32, exactly rounded (decimal to binary by a small bignum, checked against Rust's `from_str`); `-inf` and `nan` too. And a literal can stand in for a value wherever the context fixes its type: `add a, 1`, `cmp.lt 0, b`, `mul x, 0.5`, `jmp loop(0, 0)`, `ret 0`, `g(b, 2)`, with `200: u8` when nothing does. Hidden consts carry them; the printer shows them inline again. 302/302 on all four paths.

### cmp on floats, neg, abs — `aa1829b` · 2026-08-25

`icmp` is `cmp`, and on floats `cmp.lt` is the library's `lt(E, M)`: six predicates over one `fcmp` that orders by sign and magnitude bits, with IEEE's rules (-0 equals +0; a NaN makes everything false but `ne`). `neg` and `abs` touch the sign field. The platforms have `fcmp`+`cset`, `feq`/`flt`/`fle`, `f32.lt`, `fneg`, `fabs`. Two 63-bit overflow bugs surfaced and were fixed. 302/302 on all four paths, both ways.

### conv and cast — `8eecfc7`, `0f2850f` (docs `424222c`, `cd4897e`) · 2026-08-25

Two opcodes for what used to be three: `conv` carries the value across (ext and trunc are gone — the widths always said which way), `cast` keeps the bits (was bitcast). `1.0 conv u32` is 1; `cast` is 0x3f800000. Between a float and anything, `conv` is the library's: float(E, M) to float(F, N), i(W)/u(W) to float(E, M), float(E, M) to i(W)/u(W) (truncating, saturating, NaN to 0) — five generics sharing one name, chosen by the types on both sides now that dispatch matches the result type too and generics may overload. The platforms map every f32/f64/i32/u32/i64/u64 pair to `fcvt`/`scvtf`/`fcvtzs` and friends (riscv64 keeps float to int in the library over its NaN rule). Checked against Rust's `as` and the exact reference; 274/274 on all four paths, both ways.

### sqrt, and operations a library invents — `b795912` · 2026-08-25

`r: f32 = sqrt a`. Dispatch is now open-ended: any name applied to a pack finds the generic of that name that takes the pack's origin type, at whatever arity it declares, so `sqrt` exists for floats without the integer language knowing the word. The library's `sqrt(E, M)` is a digit-by-digit root that never needs more than M + 8 bits; the platforms map f32/f64 to `fsqrt`. Checked against the FPU and an exact reference (fp8 exhaustively). 235/235 on all four paths, both ways.

### `call` retires — `3cc7b8e` · 2026-08-25

`r: f32 = fadd32(a, b)`, `touch(q)`, `q: i64, r: i64 = divmod(a, b)`. A name followed by `(` in operation position is a call — no opcode is ever followed by one, `const (expr)` and `loop(...)` aside — so the keyword said nothing. It is now rejected with a note that it is implied. Explicit instantiations read the same way: `add(8, 23)(x, y)`. 218/218 on all four paths, both ways.

### Float sub, mul, div — `da2c139` · 2026-08-25

The library grows `sub` (add of the negation), `mul`, and `div` over `float(E, M)`, sharing `fnorm` and `fpack` (subnormals, round to nearest even, overflow). `mul` builds f64's 106-bit product from 27-bit halves without ever holding it; `div` is a restoring long division. The three platforms gain `fsub`/`fmul`/`fdiv` for f32 and f64, so on those widths the opcodes are instructions and on fp8/fp16/bf16 they are the library. Bit-exact against the FPU on f32/f64 and an exact reference exhaustively on fp8, all four ops. 218/218 on all four paths, both ways.

### Native f32, emulated f16, one module — `a71f9a7` · 2026-08-25

A test that shows the platform choosing per width on arm64: `add` on two `f32` values compiles to `fmov`, `fmov`, `fadd s`, `fmov` with no call, while `add` on two `f16` values in the same module compiles to a `bl` into the library's `fadd16` (1632 bytes of integer code) with no `fadd`. The test inspects the machine code of both functions for the learned encodings and checks results against the FPU and the f16 reference. Moving f16 to hardware would be one line in `src/platform.rs`.

### One `add` — `e01e057` · 2026-08-25

`iadd`/`isub`/`imul` are `add`/`sub`/`mul`, and the opcode says nothing about the type: on integers it is the instruction, on a pack that came from a generic type it dispatches to the generic function of the same name taking that type. `add x, y` on two `f32` values is `add(8, 23)(x, y)` — the softfloat library — and on a platform with hardware for that width, the `fadd` instruction. The opcode set never grows; libraries add meanings, platforms add instructions. 191/191 on all four paths, both ways.

### Platforms — `74b903d` · 2026-08-25

A platform is the list of library instantiations a target has hardware for — `fadd(8, 23)` and `fadd(11, 52)` on all three. Each instantiated function now knows its (generic, args) identity, and a backend compiling one of these, or a call to one, emits the instruction sequence instead of the SSA body: `fadd32` on arm64 is `fmov`, `fmov`, `fadd`, `fmov`, `ret`. The library body stays the definition of the semantics; `--soft` compiles with an empty platform, and the two are checked against each other and the FPU. Newly probed: FP registers and adds on every target, plus the CSR/system-register writes that switch the FPU on bare metal. 188/188 on all four paths, both ways.

### Generic functions, and floats as a library — `039de73` · 2026-08-25

`fn fadd(E, M)(a: float(E, M), b: float(E, M)) -> float(E, M)` is a template; `fn fadd32 = fadd(8, 23)` and `fadd(5, 10)(x, y)` instantiate it, by re-parsing the body with E and M bound, so `u(M + 5)` and `const (1 << E) - 1` are concrete inside. With that, `suite/float.ssa` writes IEEE addition once — round-to-nearest-even, subnormals, signed zeros, infinities, canonical NaN — using only integer instructions, and instantiates it for fp8, fp16, bf16, f32, f64. The compiler learned nothing about floats. It matches the FPU bit-for-bit on f32/f64 over ~140k pairs and an independent reference exhaustively on fp8; 188/188 on all four paths.

### Parametric types — `7b9175a` · 2026-08-25

`type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }`, then `type f32 = float(8, 23)` and `type f16 = float(5, 10)`: a `type` declaration takes integer parameters that stand for widths, and its body is a pack, an `i(expr)`/`u(expr)` with `+ - *` over the parameters, a builtin, or another declared type applied to arguments. Instantiation happens at use (`x: float(8, 23)`) or at an alias; packs are interned structurally so every spelling of a layout is one type. Functions stay monomorphic. `suite/types.ssa` pulls pi's exponent out of an f32 and doubles it by incrementing the field; 174/174 on all four paths.

### The sigils retire — `2467869` · 2026-08-25

`%v`, `^b`, `@f`, `$t` become `v`, `b`, `f`, `t`. Position already said which was which — before `:` a value is defined, after `:` a type is named, after `call` a function, after `jmp`/`br` a block, and a label is a name opening a line and followed by `:` — so the lexer now has one word token and the parser's prescans apply that rule. `fn sum(n: i64)`, `done: u1 = cmp.ge i, n`, `br done, exit, body`. Old prefixes are rejected with a message. Suite, examples, tests, harness, and docs converted; 162/162 on all four paths.

### Narrow shifts just shift — `a987671` · 2026-08-25

Shifting an `i5` by 5 or more no longer takes the amount mod 5 (which cost a `ubfm`, or a `udiv`/`msub` for non-power-of-two widths): the backends emit the container's shift and re-normalize, and amounts at or past the width are unspecified — buyer beware, like any overflow. The const-folder leaves those shifts alone and the exhaustive arm64 test skips them. `i32`/`i64` keep the hardware's mod-32/64.

### Any-width integers and packs — `542b9a5` · 2026-08-25

Types are now `iN`/`uN` for any N from 1 to 64, and signedness lives in the type: one `div`, one `rem`, one `shr`, one `cmp.lt`, `ext` fills by the source's signedness, `bitcast` reinterprets. `u1` is the boolean. `pack rgb { r: u5, g: u6, b: u5 }` packs bitfields lowest-bits-first into ≤64 bits — nestable, storable at 8/16/32/64 bits — with `pack`, `unpack`, `get`, `set`. Every backend keeps values *canonical* in their container (sign- or zero-extended) and re-normalizes after ops that can carry out, using freshly probed `sbfm`/`ubfm`/`bfm`, byte/halfword loads, and wasm's narrow loads. 162/162 on all four paths; an exhaustive JIT-vs-model test covers every op on eighteen widths.

### Abstract `int` — `1f795ad` + `acd4764` · 2026-08-23

SSA can now say `int` instead of committing to `i32` or `i64`. A resolution pass swaps it for a concrete width before verification, using a *replacement policy* per target (i64 on arm64/riscv64, i32 on wasm32) or `--int=i32|i64`. Because types sit on variables, not opcodes, that pass is one sweep over the value tables — no instruction changes. The verifier rejects any `int` that survives, so nothing downstream ever meets one. `suite/abstract.ssa` is written to be policy-independent and the suite runs under both widths. 96/96 everywhere.

### Incremental JIT arena — `31bd115` · 2026-08-23

All compiled functions live in one `MAP_JIT` arena, each in a slot with 50% slack. Every call goes through a fixed per-function trampoline that counts invocations and branches to the current address, so a changed definition recompiles in place (or relocates to the tail if it grew) and no call site is ever patched. `probe live <file> <fn> [args]` runs the loop: edits recompile only the changed function at level 0, and a function crossing 10k calls is promoted through the full pass pipeline mid-run. `src/arena.rs`.

### SSA pass pipeline with levels — `136d065` · 2026-08-23

`src/opt.rs` becomes the single optimization engine: an ordered list of SSA→SSA passes where a level is a prefix, so every stopping point is valid — the foundation for gradual optimization. New passes: simplify-cfg (threads branches through empty forwarding blocks, drops unreachable ones), const-fold (typed wrapping arithmetic; leaves divide-by-constant-zero alone since wasm traps), and dce (pure unused instructions; divisions only with provably nonzero divisors). `-O<n>` on any command; `probe tiers` shows size and time at every level; the suite runs at every level as a test.

### Register allocation, in three steps — `d863c3d` → `6a90a80` · 2026-08-23

*Linear scan* (`d863c3d`, `src/regalloc.rs`): liveness by backward fixpoint, single-span intervals, furthest-end eviction, over a callee-saved pool only — so values survive calls by construction and prologues save exactly what a function uses. 3.8× on a hot sum loop. *Sink scheduling and parallel moves* (`eaf6780`): producers move toward consumers before allocation, shrinking intervals; branch arguments become true parallel moves (cycles break through one scratch register), lifting the 8-argument cap; arm64 saves pair into `stp`/`ldp`. *Coalescing* (`6a90a80`): precise per-point interference lets block parameters union-find with their branch-argument sources, so the move on a loop back edge disappears — sum's loop body is `cmp`/`cset`/`cbz` plus two in-place adds, 96 bytes down from 156.

### Foundation — `cd7ff7a` · 2026-08-23

Everything at once: the SSA IR (block parameters, multi-value returns, structured `if`/`loop` sugar lowered at parse time); two encoding learners — bit-scatter for fixed-width ISAs, byte+LEB128 for wasm — that probe `llvm-mc`/`wat2wasm` and verify every hypothesis against the oracle; and three emitters (arm64, riscv64, wasm32) containing no hand-written opcodes. One 86-case suite runs against all of them: native JIT, node, and bare-metal qemu for riscv64 and aarch64, with the runtime harness generated in the project's own SSA.
