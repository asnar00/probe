# stamp
*the home's time stamps an item of a group's stream: the time an item was pushed is when the push reached its home*

layer: runtime

> (suite) 2026-10-24T12:00:00
fm3 places.md, "whose clock stamps a shared item"; question 149's point 3, ruled 24 October 2026 ("Go with the recommendations"); hop fifty-eight.

## overview
`said$` is `group`, everyone's in the group, and has no rate. Each key pressed pushes one item into it, and the line after the push writes the time of that item, `time of said$`.

A shared stream has a home, the place that every other place has a wire to. With the client and the server, the home is the server, the place without devices. The home stamps every item pushed into the stream with its own time, and every copy of the stream shows the same time for that item. At one place, and over a wire that takes no time, that is the time of the push, and a key at 1 s writes `1 s`. Over a wire of 100 ms, with everything on the server, the key pressed at 1 s by the client's clock reaches the home at 1.1 s. The item's time is then `1.1 s`. The line that writes it is still the line for 1 s, and the client shows it at 1.2 s, when it comes back over the wire:

    world    the client             the server, the home
    1.0 s    the key is pressed
    1.1 s                           the push arrives; said$ gets an item stamped 1.1 s
    1.2 s    "1.1 s" is shown

With nothing on the server the program is at one place, the client is the home, and the item's time is `1 s`.

So a case that reads the home's time depends on the wire, and says so with `with wire`, the only kind of case that does. It runs with everything on the server over that wire, and the times on its right are when each piece is shown at the client.

A stream with a rate needs none of this: an item's time is its slot, wherever it was made.

## interface
- `presses (c$)` is wired to `in$`: each key runs `pressed (c)`, which pushes into `said$` and writes the item's time.
- `run` is the case's call and does nothing.

## rules
- An item of a group's or everyone's stream with no rate carries its home's time: the later of the time of the push and the moment the push reached the home by the world's clock.
- At one place, and over a wire that takes no time, that is the time of the push.
- A case about a wire that takes time says `with wire <time>`. It runs with everything on the server over that wire, and its times are when each piece is shown at the client.

## testing
>run() with in "k" at 1 s → "1 s" at 1 s
>run() with in "k" at 1 s, in "k" at 2.5 s → "1 s\n" at 1 s, "2.5 s" at 2.5 s
>run() with in "k" at 1 s, wire 100 ms → "1.1 s" at 1.2 s
>run() with in "k" at 1 s, in "k" at 2.5 s, wire 100 ms → "1.1 s\n" at 1.2 s, "2.6 s" at 2.7 s

## hostile
`wire` comes last in a case's `with` clause, after the inputs: `>run() with wire 100 ms, in "k" at 1 s` is refused, "`wire <time>` is the last part of a case's `with` clause, after its inputs and switches, and a case has one (fm3 places.md, "what a case says"): `>launch() with in "k" at 3.5 s, wire 100 ms → ...`". Its time is spelled as the command line spells it: `wire fast` is refused, "a case about lateness says its wire's delay as the command line does, `wire 100 ms` or `wire 2 s`, the last part of its `with` clause (fm3 places.md, "what a case says")". A case about lateness says what was shown and when: `>run() with in "k" at 1 s, wire 100 ms → 3` is refused, "a case about lateness says what its text was and when each piece was shown at the client, `→ "10\n" at 200 ms`: a value or a failed check is the same over any wire, and is said by a case with no `wire` (fm3 places.md, point 4)".
