# counter
*a counter, mine and everyone's: a stream with no scope word is one person's, and a `group` stream is everyone's in the group*

layer: runtime

> (suite) 2026-10-24T10:00:00
fm3 places.md, "a counter, mine and everyone's"; question 149, ruled 24 October 2026 ("Go with the recommendations"); hop fifty-seven. The program is the paper's, as written there, and `run`, the case's call, which does nothing: what the program does it does from its standing line.

## overview
Each key pressed counts twice: once in `mine$`, which has no scope word and so is one person's on one device, and once in `all$`, which is `group` and so is everyone's in the group the person is in. The line it writes says both, `1 of 1`.

A case names people by a word before `in`, and says what each one's screen showed by the same word before a result, the parts joined by `;`. Ann presses a key at 1 s and at 3 s, and Bob one at 2 s:

    time    ann's screen    bob's screen
    1 s     1 of 1
    2 s                     1 of 2
    3 s     2 of 3

Ann's second line reads `2 of 3`: Bob's press is in `all$`, which they share, and not in her `mine$`. A case that names nobody is one person, as every case was before people could be named, and two presses read `1 of 1` and `2 of 2`.

Everyone a case names is one person with one device, and all of them are in one group. All of it is at one place, on one clock: a push into `all$` is the push of the line, and how two pushes combine is what the line says, `all$ << all$ + 1`, a sum (fm3 places.md, "the line is the merge").

## interface
- `presses (c$)` is wired to `in$`: each key pressed runs `pressed (c)`, which pushes one more into `mine$` and into `all$` and writes the line.
- `run` is the case's call and does nothing.

## rules
- With no scope word a stream is one person's on one device. `group` makes it everyone's in the group the person's context is in.
- Which group a person is in is set from outside, by the case line: everyone it names is in one group. Nothing in a feature's code says it.
- Each person's screen is their own: what one person's device writes is on their screen alone.
- A case that names nobody is one person.

## testing
>run() with in "k" at 1 s, in "k" at 2 s → "1 of 1\n" at 1 s, "2 of 2" at 2 s
>run() with ann in "k" at 1 s, bob in "k" at 2 s, ann in "k" at 3 s → ann "1 of 1\n" at 1 s, "2 of 3" at 3 s; bob "1 of 2" at 2 s
>run() with ann in "k" at 1 s, bob in "k" at 1 s → ann "1 of 1" at 1 s; bob "1 of 2" at 1 s
>run() with ann in "kk" at 1 s → ann "1 of 1\n2 of 2" at 1 s; bob ""

## hostile
A case that names people and gives a value has no one to give it to: `>run() with ann in "k" → 3` is refused, "a case that names people says what each one's device wrote, `→ ann "text"`, the parts joined by `;`: a value, a text with no name or `check` cannot say whose it is (fm3 places.md, "what a case says"; principle 7)". A person's name is a word the line reads no other way: `>run() → off "0"` is refused, "'off' cannot name a person in a case: a person's name is a word the line reads no other way, not `in`, `on`, `off`, `wire`, `check`, a feature's name or a type's (fm3 places.md, "what a case says"): `ann in "k" at 1 s`", and so is `counter` or `int` where a name stands. A case may name four people: a fifth is refused, "a case names 5 people, and a store has memory for 4 at one place: name at most 4". In a case that names people every input says whose it is: `with ann in "k", in "j"` is refused. The store is not yet built for a wire: `probe zero suite/zero/counter placement` says "'all$' is a group's, and a store with a stream that is a user's, a group's or everyone's is not yet built for a wire (fm3 places.md; the hop after fifty-seven)".
