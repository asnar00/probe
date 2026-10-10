# restart
*a countdown that begins again from 10 when a key arrives: the touchstone*

layer: runtime

> (suite) 2026-10-10T19:00:00
Hop 37 (fm3 questions 52, 53 and 121, log 220): `touchstones.md`'s `restart`, as today's forms write it.

## overview
`launch` counts down from 10 at `1 hz` and then writes `liftoff`. `launches`, wired over the input device, calls `launch` for each key that arrives. `count down` begins with `restart i$`. A key at 3.5 s finds the first countdown partway through its ten seconds: it is ended there, and a new one begins, its 10 at 3.5 s and its 9 a second later. The first launch never says `liftoff`.

## interface
- `launch` counts down and writes `liftoff`.
- `count down` restarts `i$` and pushes `[10 through 1]` into it.
- `launched by (c)` calls `launch` and gives 1; `launches (c$)` pushes that for each character that arrives, so `seen$` has an item for each launch a key began.

## rules
- `restart i$` does three things (question 52). The stream's beat begins again from now, so the push after it lands at once, at 3.5 s, and the next a full second later. The run in progress ends: the first countdown's 6 is never pushed. And the activity that was doing that run ends there: nothing after the countdown in the first `launch` runs.
- Nothing is taken back. The first countdown moved along with time and had put nothing ahead of itself when the key arrived.
- A key at the very time a number is due is first, the input being there before the function was called (question 121): a key at 4 s gives no 6, only the new 10.
- A second key while `launches` is still inside the launch the first key began is not handled yet: it is a function partway through, and only what is running can go on (fm3 question 135). The program stops there, a failed check, and does not write the second key's countdown late.
- A key after `liftoff` ends nothing: the second launch is a launch of its own.
- With no key, `restart i$` at 0 s restarts a stream nothing was pushing into, and the countdown is hello's.
- `launches` is wired with a stream to fill, `int seen$ = launches (in$)`, a function of one character applied to the input's name: the form the suite has for a task over the input. A line that says "when a key arrives, do this" with nothing to fill is not ruled, and `touchstones.md`'s `watch(in$)` on a line of its own is not a form the language has.

## testing
>launch() with in "k" at 3.5 s → "10\n9\n8\n7\n" at 1 hz, "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz from 3.5 s, "liftoff" at 13.5 s
>launch() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "liftoff" at 10 s
>launch() with in "k" at 4 s → "10\n9\n8\n7\n" at 1 hz, "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz from 4 s, "liftoff" at 14 s
>launch() with in "k" at 3.5 s, in "k" at 5 s → check
>launch() with in "k" at 12 s → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "liftoff" at 10 s, "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz from 12 s, "liftoff" at 22 s
