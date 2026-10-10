# keys
*input that arrives at a time: a key between two numbers of a countdown*

layer: runtime

> (suite) 2026-10-10T18:00:00
Hop 37 (fm3 questions 52, 53 and 121, log 220): a case's input given a time.

## overview
`count down` pushes `[5 through 1]` into `i$` at `1 hz` and writes `liftoff`. `codes`, wired over the input device, pushes each character's code into `key$`, which a line writes out as `key 107`. A case gives its input a time, `with in "k" at 3.5 s`, and the key is then handled at 3.5 s: after the 2 at 3 s and before the 1 at 4 s, and what it sets off is stamped 3.5 s.

## interface
- `count down` pushes five numbers into `i$` and writes `liftoff`.
- `codes (c$)` pushes the code of each character that arrives.

## rules
- The input is a thing going on like any other: it is due at the time of its next character, and items are handled in time order across every stream (`time.md`, the fourth sentence).
- Several pieces may each have a time, in the order they arrive, and a piece of several characters is each of them at that time, in order.
- A character due at the same time as a number is first: the input device is the platform's, there before any feature's line or any function a case calls (question 121, the order started). `ab` at 1 s is written before the 4 at 1 s.
- A case's input is known for ever, so the program computes its whole timeline: a key at 7 s, two seconds after the function has returned, is handled at 7 s.
- An input with no time means what it has always meant: all of it has arrived before the program starts.

## testing
>count down() with in "k" at 3.5 s → "5\n4\n3\n2\n" at 1 hz, "key 107\n" at 3.5 s, "1\n" at 4 s, "liftoff" at 5 s
>count down() with in "ab" at 1 s, in "c" at 2.5 s → "5\n" at 0 s, "key 97\nkey 98\n4\n" at 1 s, "3\n" at 2 s, "key 99\n" at 2.5 s, "2\n" at 3 s, "1\n" at 4 s, "liftoff" at 5 s
>count down() with in "z" at 7 s → "5\n4\n3\n2\n1\n" at 1 hz, "liftoff\n" at 5 s, "key 122" at 7 s
>count down() with in "k" → "key 107\n5\n" at 0 s, "4\n3\n2\n1\n" at 1 hz from 1 s, "liftoff" at 5 s
>count down() → "5\n4\n3\n2\n1\n" at 1 hz, "liftoff" at 5 s
