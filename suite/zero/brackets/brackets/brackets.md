# brackets
*a word on a push applies to the last item of its chain, and brackets round several items make them the one it applies to*

layer: runtime

> (suite) 2026-10-07T18:00:00
fm3 question 84, log 155. Ash, 7 October 2026: "I'd say that the loop constructs should apply to the last expression in the chain by default - we can always use brackets to indicate otherwise."

## overview
A push may have several items, `x$ << a << b`, and a word after them says how often: `(n) times`, `while (c)`, `until (c)`, `forever`. The word applies to the last item of the chain. `out$ << "hello" << "\n" (3) times` is hello and then three newlines.

Brackets round several items joined by `<<` make them the one item the word applies to: `out$ << ("hello" << "\n") (3) times` is three lines of hello. The bracketed items are pushed in order each time the word has them pushed, each reading the one before it by the stream's name as in any chain, so `up$ << 0 << (up$ + 1 << up$ * 10) (2) times` is `0`, then `1 10`, then `11 110`.

Brackets round one item are ordinary grouping and change nothing: `up$ << (7) (3) times` is `up$ << 7 (3) times`.

## interface
- `three lines` is `out$ << ("hello" << "\n") (3) times`; `one line` the same without the brackets, and then a full stop to show where the newlines went.
- `seeded` is a first item and then a group, counted.
- `pairs to six` is `up$ << 0 << (up$ + 1 << up$ + 1) until (up$ == 6)`: the pair is pushed and then the condition is asked, `_` and the stream's own name both the pair's last item; `pairs to six by the item` writes it with `_`. `pairs under six` is the same pair under `while (_ < 6)`: the pair is the candidate, whole, worked out and asked about before either of it is pushed, so the pair 5, 6 fails as a pair and neither goes out.
- `stepped`, `stepped past (limit)` and `stepped under (limit)` push a group into `seen$`, a cell, under each of the three words.
- `maybe (c)` puts `if` before the count: `up$ << 9 << (1 << 2) if (c > 0) (2) times`. The `if` is of the whole push, the 9 with the rest.
- `plain` is a group with no word, `up$ << (1 << 2)`: its items in order.
- `one bracketed` is `up$ << (7) (3) times`.
- `a call (k)` has a call in a group, `up$ << (k << twice (k)) (2) times`: the call's bracket is its argument.
- `d$` is declared with a group, `int d$ << 0 << (d$ + 1 << d$ + 1) (2) times`: five first items, which `declared` counts.
- `greetings` pushes a group three times into `said$`, a stream at `1 hz` wired to the output a line an item: six lines, a second apart.

## rules
- A bracket where an item begins is a group of items where a `<<` stands inside it at its own depth. `<<` is no operator of an expression, so a bracketed value never has one there: `(a)`, `(up$ + 1)` and `(twice (k))` are values.
- A group stands last in its chain. Before the last item it would be its items in order and nothing more, and is refused.
- `(n) times` over a group: the count worked out once, the items worked out and pushed each time round.
- `until (c)` over a group: the group pushed, then the condition asked. `_` is the group's last item, and the stream's own name its latest item, which is that item.
- `while (c)` over a group: the items worked out in order and held, the condition asked with `_` the last of them, and all of them pushed where it holds, none where it fails. The stream's own name in the condition is its latest item, nothing of the group having gone out.
- `if (c)` comes first, as on any push, and is of the whole push.

## testing
>three lines() → "hello\nhello\nhello"
>one line() → "hello\n\n\n."
>seeded() → "0\n1\n10\n11\n110"
>pairs to six() → "0\n1\n2\n3\n4\n5\n6"
>pairs to six by the item() → "0\n1\n2\n3\n4\n5\n6"
>pairs under six() → "0\n1\n2\n3\n4"
>stepped() → 14
>stepped past (20) → 30
>stepped under (20) → 14
>maybe (1) → "9\n1\n2\n1\n2\ndone"
>maybe (0) → "done"
>plain() → "1\n2"
>one bracketed() → "7\n7\n7"
>a call (5) → "5\n10\n5\n10"
>declared() → 5
>greetings() → "hello\nworld\nhello\nworld\nhello\nworld" at 1 hz

## hostile
`up$ << (1 << 2) << 3 (3) times` is refused: "brackets round several items of a push make them the one item its word applies to, and they stand last in the chain (fm3 question 84): `x$ << a << (b << c) (3) times`. Before the last item a group would be its items in order and nothing more: write them without the brackets". `up$ << (1 << (2 << 3)) (3) times` is refused: "a group of items inside a group: one pair of brackets says it, `x$ << (a << b << c) (3) times`". `up$ << (1 << 2) + 1` is refused: "brackets round several items of a push make them one item for the word that follows, `x$ << (a << b) (3) times`: a group is not a value, and what may follow it is `if`, `(n) times`, `while`, `until` or `forever`".
