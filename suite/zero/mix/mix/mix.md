# mix
*whose is not where: two `group` streams at a rate, mixed into each viewer's own*

layer: runtime

> (suite) 2026-10-24T10:00:00
fm3 places.md, "whose is not where"; question 149, ruled 24 October 2026; hop fifty-seven. The program is the paper's, and `run`, the case's call, which does nothing.

## overview
`kick$` and `hat$` are the group's, each at two a second: the kick counts up to 4 and the hat by tens up to 40. `mix$` has no word and is each viewer's own, and it is worked out from the group's two streams and nothing else, so every viewer's screen shows the same:

    0 s      11
    0.5 s    22
    1 s      33
    1.5 s    44

The two clocks push into the group's streams, so each beat is worked out once for the group, whoever is watching: at one place, on the first device the case names. The line that mixes them runs at the close of each slot on every device, since a push into a shared stream sets off every device's lines wired to it. Two viewers see 11, 22, 33 and 44 at the same times, and not the kick counted twice a beat.

## interface
- `kick$ << kick$ + 1 if (kick$ < 4) forever` and `hat$ << hat$ + 10 if (hat$ < 40) forever` push at their rate.
- `mix$ << kick$ + hat$ forever` runs once a slot, from that slot's two; `out$ << (mix$ << "\n") forever` writes each.
- `run` is the case's call and does nothing.

## rules
- A scope word says whose a stream is and never where a line runs.
- A line that pushes into a group's stream at a rate runs once for the group.
- A line that reads only the group's streams runs on every device, each from the same items.
- At two places the group's streams have their home at the server. With one viewer every line may run at either place, and the viewer's screen shows the same at every cut.

## testing
>run() → "11\n22\n33\n44" at 2 hz
>run() → ann "11\n22\n33\n44" at 2 hz; bob "11\n22\n33\n44" at 2 hz
>run() → ann "11\n22\n33\n44" at 2 hz; bob "11\n22\n33\n44" at 2 hz; cy "11\n22\n33\n44" at 2 hz

## hostile
`static int kick$ at (2 hz)` is refused: "`static int kick$ at (2 hz)`: `static` is `global`, everyone's (fm3 places.md, question 149): write `global int kick$ at (2 hz)`". At every cut, over wires that take no time, the cases that name two viewers or three give their own lines too: each viewer's device is a client of its own, and the server the home of the two clocks' streams.
