# shown
*extends tick: the line that writes the diamond's stream, in a feature of its own*

parent: tick
layer: runtime

> (suite) 2026-10-10T10:01:00
fm3 question 121, log 207: a tick whose lines are two features'.

## overview
`out$ << (both$ << "\n") forever` writes each item of `both$`, the stream `tick`'s diamond ends in. The tick of `d$` then has four lines, three of `tick`'s and this one, and each stands under its own feature's gate.

## interface
- The one line, and nothing else.

## rules
- With this feature on, `diamond()` writes `4` and `7`, one line for each item pushed into `d$`: this line runs once a tick, after the line that makes `both$`, which itself runs once, after both the lines it reads.
- With it off the three lines of `tick` run as they did and nothing is written: `tick`'s own case stands, `diamond() → 7`.

## testing
>diamond() → "4\n7"
