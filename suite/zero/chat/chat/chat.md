# chat
*chat, step one: what anyone types goes into the room's stream, and the room's stream goes to everyone's screen*

layer: runtime

> (suite) 2026-10-24T10:00:00
fm3 places.md, "chat, step one and step two"; touchstones.md, chat; question 149, ruled 24 October 2026; hop fifty-seven. The program is the paper's three lines, and `run`, the case's call, which does nothing.

## overview
    group char said$
    said$ << in$ forever
    out$ << said$ forever

What I type goes into the room's stream, `said$`, which is `group` and so everyone's in the group I am in. What is in the room's stream goes to my screen. Both lines stand, `forever`, and both stand on every person's device: Ann types `hi` at 1 s, her line pushes it into `said$`, and the line on every device that reads `said$` writes it, so both screens show `hi` at 1 s. It is three lines for two people or twenty, which is the touchstone's step two.

At one place, on one clock, an item pushed into `said$` carries the time of the push, and a push into a shared stream runs the standing lines wired to it on every device at that time, the device that pushed first and then the others in the order the case names the people.

At two places `said$` has its home at the server and a copy at the client. With one person each line may run at either place: a push at the client is made again at the home, and one at the home is made again at the client where the line that reads it is. Each case with one person gives its own line at every cut, over a wire that takes no time and over one of 100 ms. With two people or three there are three places or four, each person's device a client of its own and the server the home, a wire from each device to the server; over wires that take no time every case gives its own line at every cut.

## interface
- `said$ << in$ forever` pushes each character typed into the room's stream.
- `out$ << said$ forever` writes each character of the room's stream to the screen.
- `run` is the case's call and does nothing.

## rules
- A `group` stream is everyone's in the group; a standing line that reads it stands on every device.
- A push into it reaches every device's lines wired to it, at the time of the push.
- A person named on the right only has typed nothing and still reads the room.
- Each person's screen is their own.

## testing
>run() with ann in "hi" at 1 s → ann "hi" at 1 s; bob "hi" at 1 s
>run() with ann in "hi" at 1 s, bob in "yo" at 2 s → ann "hi" at 1 s, "yo" at 2 s; bob "hi" at 1 s, "yo" at 2 s
>run() with ann in "a" at 1 s, bob in "b" at 2 s, cy in "c" at 3 s → ann "a" at 1 s, "b" at 2 s, "c" at 3 s; bob "a" at 1 s, "b" at 2 s, "c" at 3 s; cy "a" at 1 s, "b" at 2 s, "c" at 3 s
>run() with ann in "hi" → ann "hi"; bob "hi"
>run() with in "hi" at 1 s → "hi" at 1 s

## hostile
`merge` is no word now: `group char said$ merge last` is refused, "`group char said$ merge last`: how two writes combine is what the line that pushes says (fm3 places.md, "the line is the merge"): `count$ << count$ + 1` is a sum, `size$ << s` the last write stands, `best$ << s if (s > best$)` the better of the two; write `group char said$`". A case that names a person twice after `→` is refused, "ann is named twice after `→`: one part says all of what a person's device wrote". At two places a case runs with one person: `probe zero suite/zero/chat run 'run() with ann in "hi" at 1 s, bob in "yo" at 2 s' --server all` says "a case here names two people or more, and two people over a wire are the hop after fifty-eight (fm3 places.md; hop-58.md)".
