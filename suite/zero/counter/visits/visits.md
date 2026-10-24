# visits
*a `global` stream is everyone's, and a `user` stream one person's on every device of theirs*

layer: runtime

> (suite) 2026-10-24T10:01:00
fm3 places.md, "the words"; hop fifty-seven. A second feature beside the counter, so that the paper's program stays as it is written.

## overview
`visits$` is `global`, everyone's: there is one of it whoever reads it. `been$` is `user`, one person's on every device of theirs. `visit` pushes one more into each and writes both, `1 1`. When Ann visits and then Bob, each on their own device, the case's call made on each in the order the case names them, Ann's screen shows `1 1` and Bob's `2 1`: the visits are counted once for everyone, and each has visited once.

A person in a case has one device, so a `user` stream behaves as one with no word does, and lives in the user's state, where a second device of theirs will find it when a case can say one (fm3 places.md, "what it leaves open").

## interface
- `visit` pushes one more into `visits$` and into `been$` and writes the two.

## rules
- `global` is everyone's. `user` is one person's, on every device of theirs.
- A value may carry the word too, `global int port = 8822`, and means the same value everywhere (`suite/zero/variables`).

## testing
>visit() → "1 1"
>visit() → ann "1 1"; bob "2 1"
>visit() → ann "1 1"; bob "2 1"; cy "3 1"

## hostile
`device int visits$ << 0` is refused: "`device int visits$ << 0`: `device` says what no word says: a stream with no scope word is one person's on one device (fm3 places.md, question 149): write `int visits$ << 0`".
