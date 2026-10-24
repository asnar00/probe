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

Everyone a case names is one person with one device, and all of them are in one group. At one place, on one clock, a push into `all$` is the push of the line, and how two pushes combine is what the line says, `all$ << all$ + 1`, a sum (fm3 places.md, "the line is the merge").

At two places, the client and the server, `all$` has a home: the server, the place every other place has a wire to and the one without devices. The client holds a copy. `mine$` has no home of its own: it is the device's, and is wherever the lines that use it are. One person at the client, with everything on the server and a wire of 100 ms between them, presses a key at 1 s, at 4 s and at 7 s. Each line comes back to the client 200 ms after its key:

    world    the client            the server, the home of all$
    1.0 s    a key
    1.1 s                          all$ is 1; "1 of 1" is written
    1.2 s    "1 of 1" is shown

With no wire named, the case gives its lines at their own times, 1 s, 4 s and 7 s, at every cut.

The home can be lost. When the wire goes, from 3 s to 6 s, the client takes the server's part by running it again from the server's start and what it sent the server, and being the home is part of it: the press at 4 s is worked out at the client, and `2 of 2` is shown at 4.25 s, as soon as the client sees the server is not answering. When the wire is steady again the client gives the part back, and the home's copy is what the client worked out: nothing lost, nothing counted twice. That wire is the runner's, `--wire 100 ms then gone at 3 s then 100 ms at 6 s`, and not a case's (`probe zero suite/zero/counter run '...' --fast --server all --choose 250 ms --wire ...`).

## interface
- `presses (c$)` is wired to `in$`: each key pressed runs `pressed (c)`, which pushes one more into `mine$` and into `all$` and writes the line.
- `run` is the case's call and does nothing.

## rules
- With no scope word a stream is one person's on one device. `group` makes it everyone's in the group the person's context is in.
- Which group a person is in is set from outside, by the case line: everyone it names is in one group. Nothing in a feature's code says it.
- Each person's screen is their own: what one person's device writes is on their screen alone.
- A case that names nobody is one person.
- At two places a group's stream has its home at the server, and the client holds a copy. A case runs with one person at two places; two people over a wire are the hop after fifty-eight.

## testing
>run() with in "k" at 1 s, in "k" at 2 s → "1 of 1\n" at 1 s, "2 of 2" at 2 s
>run() with in "k" at 1 s, in "k" at 4 s, in "k" at 7 s → "1 of 1\n" at 1 s, "2 of 2\n" at 4 s, "3 of 3" at 7 s
>run() with ann in "k" at 1 s, bob in "k" at 2 s, ann in "k" at 3 s → ann "1 of 1\n" at 1 s, "2 of 3" at 3 s; bob "1 of 2" at 2 s
>run() with ann in "k" at 1 s, bob in "k" at 1 s → ann "1 of 1" at 1 s; bob "1 of 2" at 1 s
>run() with ann in "kk" at 1 s → ann "1 of 1\n2 of 2" at 1 s; bob ""

## hostile
A case that names people and gives a value has no one to give it to: `>run() with ann in "k" → 3` is refused, "a case that names people says what each one's device wrote, `→ ann "text"`, the parts joined by `;`: a value, a text with no name or `check` cannot say whose it is (fm3 places.md, "what a case says"; principle 7)". A person's name is a word the line reads no other way: `>run() → off "0"` is refused, "'off' cannot name a person in a case: a person's name is a word the line reads no other way, not `in`, `on`, `off`, `wire`, `check`, a feature's name or a type's (fm3 places.md, "what a case says"): `ann in "k" at 1 s`", and so is `counter` or `int` where a name stands. A case may name four people: a fifth is refused, "a case names 5 people, and a store has memory for 4 at one place: name at most 4". In a case that names people every input says whose it is: `with ann in "k", in "j"` is refused. At two places a case runs with one person: `probe zero suite/zero/counter run 'run() with ann in "k" at 1 s, bob in "k" at 2 s, ann in "k" at 3 s' --server all` says "a case here names two people or more, and two people over a wire are the hop after fifty-eight (fm3 places.md; hop-58.md)", and the runner passes such cases over at every cut in one line that says so.
