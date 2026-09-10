# watch
*a feature that watches what the program writes, by redefining the platform's `<<` and calling `existing`*

parent: format
layer: runtime

> (suite) 2026-09-10T10:03:00
Question 46 (Ash, 10 September 2026): a device is written and never read, and a feature that wants to monitor what is going into one extends the `<<` method for the item it wants to see.

## overview
`out$` is the output device: a push into it writes a character to the place and stores nothing, and nothing in the program may read it back (question 45). So a program cannot ask what it wrote. It does not need to: **a feature that wants to watch what is written redefines the platform's method for the item it wants to see and calls `existing`**, which is the one extension mechanism the language has. This feature redefines `on (char o$) << (int x)`, the method that writes an integer's digits, counts the integers into a variable of its own, and calls `existing` so the digits still go out. Nothing else in the store changes, and with the feature off the digits still go out and nothing is counted.

## interface
- `counted ()` writes three numbers to `out$` and gives how many this feature has seen.
- `numbers written ()` writes the same three and gives what the place was given.

## rules
- A redefinition of a `<<` method applies wherever the item is pushed, into the device or into a stream: the platform's method is lowered twice (log 87) and both copies go through the chain of features, so a store that watches integers sees every one.
- `existing o$ << x` calls the definition below this one in the chain — the platform's own, which divides out the digits and writes them.
- With this feature off its link is not taken, so `written` stays 0 and the digits are written by the definition below.

## testing
>counted() → 3
>counted() with watch off → 0
>numbers written() → "123"
>numbers written() with watch off → "123"

## hostile
Redefining a method the compiler's `platform` feature declares is ordinary composition: the platform feature is the first feature in every store, so every later feature is above it and may extend it. A feature that redefines `<<` and does not call `existing` writes nothing, which is what it asked for.
