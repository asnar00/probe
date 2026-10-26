# spin
*the triangle turning: its angle a stream at four hertz, pushed on by a fixed step at each tick, and the frame worked out from the angle at each tick and drawn on a line of its own*

parent: triangle
layer: runtime

> (suite) 2026-10-26T10:00:00
fm3 `triangle.md`, "step three, the plan", and question 221: the frame as a value at each tick of a stream at a rate, worked out at the server and shown at the client at its own time, over a wire that takes time as over none.

## overview
`angle$` is a stream at four hertz. `spin()` pushes it on by half a radian at each tick, `angle$ << 0.0 << (angle$ + 0.5) while (angle$ < 1.5)`: 0 at 0 s, then 0.5, 1 and 1.5, each in the stream's next slot, a quarter of a second apart. `spun` is wired over the stream as `presses` is over the keys in `merge`: for each angle `shown` works out the frame of five pixels a side at that angle and draws it on a line of its own, a `#` for each pixel of the triangle and a `.` for each of the background, row after row. So `spin()` is four frames, one at each tick, the triangle turned a little more in each. Its four lines, each cut into its five rows:

    .....    .....    .....    .....
    ..#..    .....    ..#..    ..#..
    ..##.    .###.    .##..    .###.
    ..#..    ..#..    ..#..    ...#.
    .....    .....    .....    .....

Nothing in the program says where a frame is worked out. With everything on the server over a wire of 100 ms, the call of `spin` reaches the server at 100 ms, and the server, which reads no input, works all four frames out at once; they reach the client at 200 ms, and the client keeps each until its own time. So one piece is late, the first, by a round trip, and no other (fm3 question 220: the first frame's lateness is printed, not hidden and not put right by a rule of the clock).

## interface
- `spin()`: four ticks of the angle, a frame at each.
- `shown (a)`: draws `frame (5, a)` on a line of its own.
- `dot (c)`: a pixel as one character, `#` for the triangle's colour, 4278231295, and `.` for any other.
- `spun (a$)`: `shown` for each angle the stream is given.

## rules
- **The angle is pushed by the call, not by a clock line at feature scope.** `angle$ << angle$ + 0.5 if (angle$ < 2.0) forever` would tick in every case of the store, and the triangle's own cases, `said frame()` among them, would write four frames more than they say. Pushed in `spin`, the stream has items only in a case that calls it. As a clock line over a wire it would also start before the case's call, and the server would wait for the call to cross and come back before taking the next tick, so the frame for 250 ms would be late as well.
- **Five a side, not four.** At four a side the triangle covers the four middle pixels at every angle and never reaches another, so every frame would be the same; at five each of the four differs from the one before.
- **A character a pixel, not its colour as a number.** Written as `said frame()` writes it, a frame of 25 colours is 275 characters, and the case over a wire, which reads back when each character was shown and every row the wire carried, printed more than the arm64 machine under qemu writes in the suite's 30 seconds. Drawn, a frame is 26 characters and the case line shows the triangle turning.
- **The plainest form that runs.** `out$ << (frame (5, angle$) << "\n") forever` is refused, there being no `<<` of an array of `uint32` into `out$`; `written (frame (5, angle$))` at feature scope is refused, `written` reading no stream; `out$ << (shown (angle$) << "\n") forever` runs, but writes what `shown` gives after each frame. A function wired over the stream draws a frame and nothing else at each tick.

## testing
>spin() → ".......#....##...#.......\n" at 0 s, "...........###...#.......\n" at 250 ms, ".......#...##....#.......\n" at 500 ms, ".......#...###....#......" at 750 ms
>spin() with wire 100 ms → ".......#....##...#.......\n" at 200 ms, "...........###...#.......\n" at 250 ms, ".......#...##....#.......\n" at 500 ms, ".......#...###....#......" at 750 ms
