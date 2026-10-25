# triangle
*a frame is an array of colours, a pixel an ordinary function of its place and an angle, and the frame that function applied across every place*

layer: runtime

> (suite) 2026-10-25T10:00:00
fm3 question 211 and `triangle.md`, step two: the touchstone's spinning triangle with no shader word. "A frame is an array of colours, a pixel is given by an ordinary function of its coordinates and the angle; the frame is that function applied across the arrays of coordinates, the rank rule of question 77, which zero has."

## overview
A frame of `side` by `side` pixels is an array of `side × side` colours, row after row. `pixel (i, side, angle)` gives the colour of the pixel at place `i`: the triangle's where the pixel's centre is inside the triangle turned by `angle`, the background's where it is not. `frame (side, angle)` is `pixel` applied across the places `[0 to side * side]`: `pixel` is declared over one `int`, so given an array of them it is applied to each and gives an array, with `side` and `angle` handed to every application as they are. Nothing in the program says where that application runs. `probe zero <store> kernel 'frame (64, 0.5)' --wgsl` runs it as a kernel on WebGPU, a thread a pixel, and compares the frame with the one the CPU's loop makes, word for word.

The triangle's three corners are at three-eighths of the side from the frame's centre, a third of a turn apart, the first at `angle` radians anticlockwise from the right. `sine` and `cosine` are written here, a polynomial after the angle is brought within half a turn of zero: the language has no sine of its own.

A colour is four bytes, red, green, blue and alpha, the first in the lowest byte, so it is a `uint32`: alpha 255 is the top byte and does not fit a signed 32-bit `int`, which is what `int` is on wasm. The triangle is `rgba (255, 160, 0, 255)`, 4278231295; the background `rgba (16, 16, 32, 255)`, 4280291344.

## interface
- `pixel (i, side, angle)`: the colour of pixel `i` of a frame `side` wide.
- `frame (side, angle)`: every pixel of it, `side × side` colours.
- `sine (a)`, `cosine (a)`, in radians; `snapped (v)`, a coordinate on a grid of sixteenths of a pixel; `edge (...)`, which side of a line a point is; `rgba (r, g, b, a)`, four bytes packed.
- `centre at (k)` and `corner at (k)` read the centre pixel and the first of `frame (8, k / 2)`; `said frame` writes `frame (8, 0.0)`; `a third of a turn` and `half a radian` compare two frames whole.

## rules
- **A pixel is the function's, not the frame's.** `pixel` reads only its arguments, so each application is free of every other, which is what lets a place with many threads run one per pixel and a place with one run a loop, with the same frame.
- **The place is split into its row and column without dividing two integers**: the row is `int ((float (i) + 0.5) / float (side))`, exact for any frame under two thousand pixels a side, in either width of `float`, since the half keeps the quotient clear of a whole number. The WGSL path refuses a division of two 64-bit integers (fm3 question 202), and this is the program's arithmetic made what every path runs.
- **Inside is decided in whole numbers.** The corners are worked out in `float` and snapped to sixteenths of a pixel; the three tests of which side of each edge the pixel's centre lies on are then integer arithmetic, a pixel on an edge being inside. `float` is 64 bits on the CPU and 32 on wasm and the GPU, so a test made in `float` could fall either side of an edge on two paths; once the corners are snapped, every path draws the same pixels.
- **Three-fold.** Turned by a third of a turn the triangle is the same triangle, its corners taken in another order: `frame (8, 0.0)` and `frame (8, 2.0943951)` are the same frame.

## testing
>centre at (0) → 4278231295
>centre at (1) → 4278231295
>centre at (4) → 4278231295
>centre at (9) → 4278231295
>centre at (-3) → 4278231295
>corner at (0) → 4280291344
>corner at (1) → 4280291344
>corner at (9) → 4280291344
>said frame() → "4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4278231295 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4278231295 4278231295 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4278231295 4278231295 4278231295 4278231295 4280291344 4280291344 4280291344 4280291344 4278231295 4278231295 4278231295 4278231295 4280291344 4280291344 4280291344 4280291344 4278231295 4278231295 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4278231295 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 4280291344 "
>a third of a turn() → 1
>half a radian() → 0
