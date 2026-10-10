# now
*in a push that happens once, a stream's name is its value now*

layer: runtime

> (suite) 2026-10-08T14:00:00
fm3 questions 79 and 90, log 163. Ash, 7 October 2026: "the name of a stream, where a line that happens once wants one value, is its value now, its latest item", and of the two kinds, "`out$ << seen$`, its value now", against "`out$ << a[]`, all four".

## overview
A `<<` sends once, each time its line runs, and `forever` makes it stand. So in a function an item of a push is worked out once, and a stream named in it is read for what it holds now, its latest item:

    on pushed on()
        seen$ << 1 << 2 << 3
        out$ << seen$

writes `3`. The same line with the word, at feature scope, takes every item as it comes: `out$ << (flow$ << "\n") forever`, and `each as it comes` pushes 1, 2 and 3 into `flow$` and all three are written. An array is not a stream and has no now: `out$ << a[]` pushes all its items, in order.

## interface
- `pushed on` and `each as it comes` are the pair above.
- `kept from two` keeps a counter from two streams, `total$ << total$ + other$`: the stream's own name is its latest item, as it always was in its own push, and now `other$` is too. `kept with a local` is the same with a local taken first, `int o = late$`, which is how it had to be written before. Both streams of each are read for their latest item and nothing else, so each is one word of the context (`suite/zero/cells`).
- `one doubled` pushes `a$ * 2`, one item, the latest doubled; `doubled now` hands the name to a function of one value, `d$ << doubled (seen$)`.
- `a local pushed on` is a stream declared in the function, and `first items` a declaration's own first items, `int s$ << seen$ << 1`.
- `before the first` and `none yet` read a stream nothing has been pushed into: the zero of its type, whether the compiler keeps one word of it or a queue; `none yet` counts it too, `count both$`, 0, a counter beside the word.
- `all of an array` pushes an array, whole. `each with the latest` pushes `a[] + other$`: each item of the array with the stream's latest added.
- `three of the latest` pushes `seen$ (3) times`: the latest, three times.
- `by the word` says `latest seen$`, which is the same thing.

## rules
- An item of a push in a function is worked out once each time the line runs. A stream's name in it is the stream's latest item; an array's name is the array, all its items.
- That holds through an operator, `total$ + other$`, through `a if (c) else b`, and into an argument of a function of one value.
- A stream handed to a task in a chain, `d$ << twice (i$)`, is the stream; so is one a word is asked of, `out$ << frame x$`, which pushes everything unread.
- A line that stands is not changed: with `forever`, a count or an `until` at feature scope, each item is taken as it comes. Nor is a stream processor's line, where a name is the present item.
- Before anything has been pushed a stream's name reads as the zero of its type; so do `latest x$` and its own name on the right of a push into it, whatever it is kept as (fm3 question 98; `suite/zero/streams`' `latest before any` and `first of its own`).

## testing
>pushed on() → "3"
>each as it comes() → "1\n2\n3"
>kept from two() → 12
>kept with a local() → 12
>one doubled() → 106
>a local pushed on() → 103
>before the first() → "0"
>none yet() → 0
>all of an array() → "1 2 3"
>each with the latest() → 336
>doubled now() → 110
>three of the latest() → "8\n8\n8"
>by the word() → 3
>first items() → 209
