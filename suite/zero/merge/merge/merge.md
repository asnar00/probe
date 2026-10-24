# merge
*the line is the merge: how two people's pushes into one shared stream combine is what the pushing line says*

layer: runtime

> (suite) 2026-10-24T12:00:00
fm3 places.md, "two pushes at once: the line is the merge"; question 149's point 2, ruled 24 October 2026; hop fifty-nine. The three lines are the paper's.

## overview
A key is a digit, `s`. Each key pushes it into three streams that everyone in the group shares, each by a line of its own, and writes the three as they stand:

    size$ << s                       the last write stands
    count$ << count$ + 1             a sum
    best$ << s if (s > best$)        the better of the two

There is no word for how two writes combine. The line says it: a push that reads nothing of the stream is the last write, a push that adds to the stream is a sum, a push made only when it is better is the better. Ann presses 3 and Bob presses 5 at the same moment, Ann named first:

    ann's screen    3 1 3        her push: size 3, one push, the best 3
    bob's screen    5 2 5        his after hers: size 5, two pushes, the best 5

and when Ann presses 4 a second later her line reads `4 3 5`: the last write is hers, three pushes in all, and 5 is still the best.

Each person has a device of their own, and the server is the home of the three streams, the place every device has a wire to. At every cut, with wires that take no time, the lines are the same.

Over a wire of 100 ms each device shows its own push at once, against its copy: Ann's screen `3 1 3`, Bob's `5 1 5`. The two pushes cross, and the home runs each line again against its own streams, Ann's first: size 5, two pushes, the best 5, which every copy has at 1.2 s, so Ann's next key at 2 s reads `4 3 5`. What crosses is the push and never the value: had the values crossed, the home would be handed `count$` 1 twice and `best$` 3 after 5. Where Ann's wire is gone from 500 ms to 3 s her two pushes reach the home at 3.1 s, after Bob's of 1 s, and the home puts them in its order by time and then by place: Ann's 3 of 1 s, Bob's 5 of 1 s, Ann's 4 of 2 s. Every copy ends with size 4, three pushes and the best 5.

## interface
- `presses (c$)` is wired to `in$`: each key runs `pressed (c)`, which pushes into the three streams and writes them.
- `run` is the case's call and does nothing.

## rules
- How two pushes into a shared stream combine is what the line that pushes says.
- Two pushes at one moment are taken in the order the case names the people.
- Over a wire, the home's order is by each push's own time and then by place; a screen keeps what it showed.

## testing
>run() with ann in "3" at 1 s, bob in "5" at 1 s → ann "3 1 3" at 1 s; bob "5 2 5" at 1 s
>run() with ann in "3" at 1 s, bob in "5" at 1 s, ann in "4" at 2 s → ann "3 1 3\n" at 1 s, "4 3 5" at 2 s; bob "5 2 5" at 1 s
>run() with bob in "5" at 1 s, ann in "3" at 1 s → bob "5 1 5" at 1 s; ann "3 2 5" at 1 s
>run() with in "7" at 1 s, in "2" at 2 s → "7 1 7\n" at 1 s, "2 2 7" at 2 s
>run() with ann in "3" at 1 s, bob in "5" at 1 s, ann in "4" at 2 s, wire 100 ms → ann "3 1 3\n" at 1 s, "4 3 5" at 2 s; bob "5 1 5" at 1 s
