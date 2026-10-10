# probe

A compiler back-end that **learns instruction encodings by probing a toolchain** instead of transcribing them from architecture manuals.

The lowest stage of a compiler — turning IR into machine-code bytes — is usually built by hand-copying bit layouts out of thousand-page reference PDFs, which is exactly the kind of transcription that breeds bugs. This project takes a different route: generate lots of tiny probe programs, feed them to an existing assembler (llvm-mc, wat2wasm), diff the bytes that come back, and *derive* the encoding — the fixed opcode bits, where each operand field lives, how immediates are encoded. Nothing is trusted until it survives randomized verification against the oracle; anything nonlinear or surprising is reported as unlearned rather than silently mis-encoded.

The result is a small, portable pipeline:

```
suite/*.ssa  --parse/resolve/verify-->  SSA  --passes-->  SSA  --emitter-->  bytes  --run-->  results
                                                                     ^
                                                  targets/*.encodings.json  (learned, verified)
```



## Hello, world

```sh
cargo run -- boot os/hello.ssa          # riscv64 on qemu's virt machine
cargo run -- boot os/hello.ssa arm      # aarch64, the same source
hello world ᕦ(ツ)ᕤ
```

`os/hello.ssa` is the first operating system written in probe: a `data` string (an array of UTF-8 bytes), a loop storing each byte to the board's UART (`platform uart`, a constant from the platform file), and an exit through the board's finisher or a PSCI call (a platform rule with `hvc` as its body). No runtime, no linker script, no assembly: the image is the learned encodings and a six-instruction preamble that sets the stack pointer.

```sh
printf 'hello\nbye\n' | cargo run -- boot os/echo.ssa arm
> echo: hello
> 
```

`os/echo.ssa` is the second: a kernel that installs a trap handler (`fn __trap`, compiled with a frame that keeps the interrupted code's registers and returns by `eret`/`mret`) and serves write, read and exit through a `data` table of function values, and a program that uses nothing but those system calls (`svc`/`ecall`). Its input is a stream (`keys: u8$`, `lib/stream.ssa`): `__irq` pushes each byte the port has, with the time it arrived, and `read` — a reader kept across system calls — sleeps until a newline is among the unread bytes, so between keystrokes the machine does nothing. The trap instructions are learned like every other; how a board takes a trap is a handful of platform rules and constants.

```sh
cargo run -- boot os/clock.ssa
tick            # ten of them, a tenth of a second apart
10 ticks in 1006 ms
```

```sh
cargo run -- boot os/tasks.ssa arm
ababababab
```

```sh
cargo run -- boot os/sleep.ssa
frame 1: a b c    # the previous frame's record, from an arena
a 100000 +5415    # letter, scheduled wake in µs, lateness
c 142857 +3679
...
! 520000 +5503    # a callback, run inside the interrupt
...
31 interrupts, worst +7206 us
```

`os/sleep.ssa` is the fifth: three tasks sleeping until exact times — every 1/10, 1/3 and 1/7 of a second — woken in the order the fractions say, 3/3 and 7/7 and 10/10 of a second being one instant, and the timer armed to the next deadline rather than to a tick. A task giving up the cpu asks for a software interrupt (`reschedule`), so every switch still happens in `__irq`; `at(t, f, arg)` / `after(d, f, arg)` run a function value at a time inside the interrupt — the `!` lines — with nothing added to the IR: a time is a library type and a callback a function value. And it has memory with the lifetime of a frame: two arenas (`lib/arena.ssa`) the scheduler writes wake records into and the idle task reads and resets, flipped by a callback every tenth of a second — the `frame` lines; and with the lifetime of a task: stacks from a pool (`lib/pool.ssa`), so a callback can spawn a one-shot task (`* 630000`) that sleeps to its time, runs, and hands its stack back — `3 stacks out` at the end. The pool, the arenas and the task's own region all come from one heap (`lib/heap.ssa`) over the RAM the machine has above the image — 112 MB, `platform heap_base` to `platform ram_end` — sealed inside every interrupt: `73728 heap bytes out` at the end, the task's region gone back.

`os/tasks.ssa` is the fourth: two tasks preempted by the timer. The interrupt handler is `fn __irq(sp: ptr) -> ptr` — handed the frame holding the interrupted task's whole register file, answering with the frame to resume — so a task is a stack and a place to resume, and a switch is two stores and two loads.

`os/clock.ssa` is the third: it keeps time. The timer interrupt lands in `fn __irq` (a frame that keeps every register, float scratch included) and pushes the tick's time into a stream (`tick: time$`) the program reads a frame at a time, sleeping between; each deadline is one step on from the last so the ticks never drift, and the tenth tick's time — the machine's counter since boot — becomes milliseconds exactly, through `lib/time.ssa`'s rationals. The generic timer and GICv2 on one board, the CLINT and `time` CSR on the other, are eight platform rules — some with typed temporaries (`irq_on() -> () with gic: ptr, v: u32`) for the addresses and values they need.

## The IR

`ssa.md` is the reference; `src/ssa.rs` the parser, verifier and printer. In outline:

- **Integers of any width from 1 to 256 bits**, signed or unsigned (`i5`, `u23`, `i64`, `u128`, plus `ptr`). Signedness lives in the type, so there is one `div`, one `shr`, one `cmp.lt`. Above 64 bits a value is lowered to a row of words right after parsing (`src/wide.rs`), so no backend ever meets one.
- **Packs**: bitfields laid out lowest-bits-first in up to 256 bits — `type rgb = pack(r: u5, g: u6, b: u5)`, or the fields on lines under `pack` — nestable and storable. A pack is its bit pattern.
- **Typed pointers**: `ptr(T)` and `ptr(array(f32, 512, 512))` — an address that knows what it points at, so `load g, i, j` takes indices and checks the element; `ptr` stays bytes. Arrays with a shape are memory types, in `data`, `scratch` or behind a pointer.
- **Vectors**: `f32x4`, `i32x8`, `floatx4` — N lanes of a type, a struct of numbered lanes; `add`, `cmp.`*, `conv`, `sqrt` on a vector work lane by lane, defined by the IR itself the way wide integers are, so they run on every backend today and a platform with vector registers can take the whole vector later.
- **Structs**: fields side by side, in memory at natural offsets and in registers as separate values. Never a bit pattern — no `cast`, no literal — which leaves the layout to the compiler. Dissolved into their fields after parsing (`src/aggregate.rs`).
- **Parametric types and generic functions**, instantiated by width: `type float(E, M) = pack(mantissa: u(M), exponent: u(E), sign: u1)`, `type f32 = float(8, 23)`, `fn add(E, M)(a: float(E, M), b: float(E, M))`. An opcode on a pack dispatches to the generic of that name, so `add x, y` on two `f32` values *is* the library's function — or the platform's instruction — and a library can add operations of its own (`sqrt x`).
- **Literals wherever the type is known** (`add a, 1`, `mul x, 0.5`, `ret 0`), float literals rounded exactly; block parameters instead of phi nodes; multiple return values; `load`/`store` with `base, off` and `base, index, step` addressing; an optional structured front-end (`if`/`loop`/`break`/`continue`/`yield`) that lowers to the flat block graph at parse time.
- **Data**: `data greeting = "..."` is an array of UTF-8 bytes, `data table: array(i32, 4) = 10, 20, 30, -40` an initialized array, `addr` and `len` reach it, and `platform uart` is a constant the platform file provides per board.
- **Scratch**: `p: ptr = scratch 64` is memory that is the function's while it runs — its frame, or a shadow stack on wasm.
- **Check**: `check c` is an assertion; one that fails is a breakpoint trap the kernel's `__trap` reports with the address (`os/check.ssa`). How a library says a capacity was exceeded.
- **Function values**: `fn(i64, i64) -> i64` is a type — the signature — and `f: binary = addr add64` a value of it, taken with the same `addr` that reaches data. Calling it, `r: i64 = f(a, b)`, is spelled like any call and checked like one; the value goes anywhere a value goes, including memory. `adr`+`blr` on arm64, `auipc`+`jalr` on riscv64, a table and `call_indirect` on wasm.
- **Abstract types resolved by policy**: `int`/`uint` take a width per target (`--int=i32|i64`); `index`, the type of a count, a position and a byte offset, takes the width of the target's memory, 64 bits on the register machines and 32 on wasm32 (`--index=16|32|64`, `suite/index.ssa`); `float`, `fixed`, `unit`, `sunit` and `rational` resolve to the libraries' `float(E, M)`, `fixed(I, F)`, `unit(N)`, `sunit(N)`, `rational(N, D)` (`--float=`, `--fixed=`, `--unit=`, `--sunit=`, `--rational=`, `--round=` for the rounding mode); `scalar` is whichever family the policy names (`--scalar=`).



## The libraries

Every file compiled gets `lib/*.ssa` appended. Number formats are libraries, never compiler features; `formats.md` is the recipe and `/format` scaffolds one.

- `lib/float.ssa` — IEEE floats over `float(E, M)`: add, sub, mul, div, sqrt, neg, abs, min, max, fma, the comparisons, conversions, every rounding mode; one body per operation, instantiated from fp8 to binary128.
- `lib/fixed.ssa`, `lib/unit.ssa` — fixed point, and fractions of one (signed and unsigned).
- `lib/rational.ssa` — exact rationals, reduced, in 128 bits.
- `lib/time.ssa` — a `rational(64, 64)` of seconds with units: a sample period at 44100 Hz times 44100 is exactly one second.
- `lib/decimal.ssa` — `decimal(N, S)`, an `i(N)` significand at scale 10^S: cents that add exactly.
- `lib/wide.ssa` — division (and, on a core without a multiplier, multiplication) for wide integers.
- `lib/sample.ssa` — a rank-2 view sampled by coordinate as a texture is: `sample img, x, y`, bilinear, the edges clamped, the weights in the coordinate's type and the result rounded once into the element's.
- `lib/stream.ssa` — streams: `T$` a reader's view of a ring of values over time, the ring on a clock and its items at integer ticks of it (a tick per item on an irregular ring, `t0 + k·step` on a regular one), `time` the boundary type converted once per call; `push`, `frame`, `sample` by a rule (nearest, hold, linear — exact, rounded once), `window`, `count`, `received`, `latest`, `peek`/`advance` for a reader taking less than a frame, `push s, block` (a copy into both halves of the ring, which mirrors each item so that a frame is always one view and nothing ever slides), `behind s, k` / `last s, k` for a node's history (a biquad reads two behind its frame and two behind its own output); several views of one ring with their own rules; the interrupt handlers of `os/echo.ssa` and `os/clock.ssa` are producers.
- `lib/slice.ssa` — views: `T[]`, `T[,]`, `T[,,]` into a buffer (a typed pointer, a count and a stride per axis); `add c, a, b`, `mul c, a, k`, `fill`, `copy`, `sum`... over a rank-1 view as loops over chunks, over a higher rank row by row, a reduction one rank down per row — `chunk(T)`, a vector register's worth of T, or T itself where there are none — then the tail one at a time: vector instructions on NEON and RVV, the definition on every other path.
- `lib/reduce.ssa` — across the lanes: `sum`, `min`, `max` of a vector, `all`, `any` of a mask, as pairwise trees; one instruction on NEON (`addv`, `fmaxv`...) and, for the integers, RVV (`vredsum`...).
- `lib/int.ssa` — `min`, `max`, `abs`, `neg` over `number`, the top of the tower of abstract types: one body for every integer and number library (float keeps its own, for NaN), instructions where a platform has them (`cmp`/`csel` on arm64; `smin`/`umin`, `vmin`/`vminu`, `abs`, `neg` over vectors).
- `lib/arena.ssa` — bump allocation over memory the program declared: `arena_alloc`, `arena_mark`/`arena_release`, `arena_reset`; the frame allocator of game engines, and the bottom rung of a lifetime ladder (call, frame, object, machine) that never needs a heap. Running out is a failed `check`.
- `lib/pool.ssa` — fixed-size slots taken and given back in any order: `pool_take`, `pool_give`, a free list through the free slots and a flag per slot, so a double give is a failed `check`. The object rung.
- `lib/heap.ssa` — the root the rungs are carved from: a buddy allocator over one declared block, `heap_take`/`heap_give` in powers of two aligned to their size, a state per node of the split tree so a wrong give is a failed `check`, and `heap_seal` so a kernel can forbid allocation inside interrupts. For regions, not objects.



## The learners

- `src/learn.rs` for fixed-width register ISAs: one-hot probes XORed against a baseline map each operand bit to its encoding bit — which handles RISC-V's scrambled branch immediates exactly as easily as ARM's contiguous fields. Nonlinear small domains become lookup tables; nonlinear large ones (ARM logical immediates) fail honestly.
- `src/wlearn.rs` for byte-oriented stack machines (wasm): templates are fixed bytes plus LEB128 codecs at discovered positions, probed through wat2wasm with a bootstrap chain of stack context.
- Both talk to an assembler only through `src/oracle.rs`; the per-target seed files (`targets/*.probe`, read by `src/target.rs`) say how to *spell* instructions and nothing else.



## Platforms and variants

A platform file (`targets/*.platform`, read by `src/platform.rs`) says what a target does natively, as rules over the library's operations:

- `class s = f32` — f32 values live in `s` registers. The allocator (`src/regalloc.rs`) keeps each value in its class's file, so a chain of float operations compiles to the instructions alone. `class v = f32x4, i32x4, ...` does the same for vectors: `fadd {v}.4s, {v}.4s, {v}.4s = add(f32x4, f32x4) -> f32x4` takes the whole vector, and the parser keeps a vector whole only for the operations with a rule — the rest stays lane by lane, as on every other backend. `targets/arm64-noneon.platform` and `targets/riscv64-nov.platform` are the same targets without the vector rules, the reference they are checked against.
- `fadd {s}, {s}, {s} = add(f32, f32) -> f32` — one learned instruction for one library operation. Rules can only name templates the learner verified. Compiling such an instance, or a call to one, emits the rule instead of the SSA body; `--soft` turns that off, and the library remains the reference the hardware path is checked against.
- `const uart = 0x10000000` — a board's addresses, for `platform uart`; `heap_base` and `ram_end`, the RAM above the image that is a program's to carve.
- `psci(code: u64) -> ()` with `hvc 0` under it — a plain function the platform gives a body: how the board is ended, how a trap is installed, read and returned from (`vectors`, `cause`, `resume`, `resume_at`, `syscall`), how time is read and the timer's interrupt taken (`now`, `hz`, `timer_at`, `irq_on`, `irq_ack`, ...). A body line may spell a template's fixed operands (`msr vbar_el1, t`); a rule may declare typed temporaries (`with gic: ptr, v: u32`) for addresses and values it needs; `none` is a rule that does nothing.
- **Cost** (`probe cost file.ssa [fn...] [arm|riscv] [--assume=N]`): a function's **SSA time** — the longest path through its IR, one per instruction, arithmetic on machine numbers one whichever library implements it, each loop its trip count times the longest path from its header to a latch and once the longest path to the block that leaves: declared (`loop(...) bound N`, N traversals of the back edge), shown by the loop (a variable stepped by a constant, compared with a value whose range is known), assumed (`--assume`), or reported as unbounded and counted as one traversal. A callee is costed with what its call site knows — a range per integer argument, carried through by constants, arithmetic, `min` and `max`, a loop parameter's start and the sign of its step, and every `check` or branch on the way — so a string literal's length bounds the copy inside the library that moves it (`x11 (j from at least 0 by 1 to at most 11)`), and a function met with different constants is costed once per constant. It compares two programs with no target in sight. With a target, **K** — the platform's cost of the function's emitted code (`cost mnemonic = n` lines in the platform file; 1 without) per IR instruction — and **hardware time**, the same walk weighted by K, library arithmetic the platform has no instruction for descended into. K is a constant per function per platform, and where it is far from the platform's usual value the IR's idea of cost and the machine's disagree.
- **Variants** are files too: the base file is grouped by extension (`ext M`, `ext F`, `ext D`), and `targets/rv64i.platform` is `base riscv64` plus `without M, F, D` — a core on which `mul`/`div`/`rem` and every float operation are the library's. `--platform=rv64i` selects it everywhere; `probe footprint` lists the instructions a program really used, to prove it.



## The backends

None of them contains a single hand-written opcode.

- `arm64` (`src/emit.rs`) — JIT: mmap/MAP_JIT on Apple Silicon, run in-process; also bare metal under qemu-system-aarch64. Vectors of a classed type (`f32x4`, `i32x4`, `f64x2`, `i64x2`, their unsigned and `u1` forms) are NEON registers, one instruction per operation the platform has a rule for.
- `riscv64` (`src/emit_rv.rs`) — bare metal on qemu-system-riscv64, with the runtime harness (UART printing, exit) generated in the project's own SSA. The classed vector types are RVV registers, each rule setting its own vtype (`vsetivli`).
- `wasm32` (`src/emit_wasm.rs`) — module emission, executed by node via `src/driver.js`; control flow becomes nested `block`/`loop`/`if` from the dominator tree (`src/structure.rs`), a dispatcher loop only for an irreducible graph.
- `air` (`src/emit_air.rs`, `src/bitcode.rs`) — Apple's GPUs: the SSA as LLVM bitcode in Apple's AIR dialect, in a `.metallib` the Metal driver compiles for whatever GPU it finds — written byte by byte by our own bitstream writer, with none of Apple's tools in the path. Pointers are offsets into one memory buffer (as on wasm), `data` and `scratch` live there, a `__kernel(mem, area, id)` — or `(mem, area, id, lane, group)` — becomes the compute kernel; `lib/gpu.ssa`'s a `group tmp: array(i64, 64)` item is the threadgroup's memory here and writable data everywhere else, `group_sync()` its barrier, and `simd_sum x`, shuffles and votes are the simdgroup's (`lib/gpu.ssa`: Apple's intrinsics here, the one-thread forms elsewhere); a `;! __kernel n g [m] -> words` directive runs a kernel as a suite case — dispatched here, as fibres taking turns at every barrier on a machine (`lib/fibre.ssa`, a stack switch the platform provides), the groups dealt across m OS threads under the JIT and m cores on the qemu machines (`lib/core.ssa`: PSCI on arm64, a parking preamble and mailbox on riscv64); vectors reach it whole (`<4 x float>`, `air.sqrt.v4f32`); recursion, which Metal has not, is left out and reported. `tools/driver_metal.py` dispatches it (pyobjc).

Shared by the register machines and wasm:

- **A linear-scan register allocator** (`src/regalloc.rs`) with register classes: the emitter hands it pools and gets back a register or spill slot per value.
- **An SSA pass pipeline** (`src/opt.rs`): simplify-cfg, const-fold, elide-stores (a store that writes back what was just loaded from the program's own memory goes, so a struct written back with one field changed stores one word), dce, sink. Optimization levels are prefixes of that one list, so every level is a correct stopping point and every pass is checked by the suite on every backend.
- **An incremental JIT arena** (`src/arena.rs`): each function in its own slot with slack, calls routed through counting trampolines, so an edited function recompiles in place and a hot one is promoted through the full pipeline without disturbing its neighbours.



## Verification

- **One regression suite** (`suite/*.ssa`, runner in `src/suite.rs`): 863 cases with expectations embedded as `;! gcd 48 36 -> 12` directives (or `-> check`, for a case that must end in a failed check), run identically against every backend — including arm64 under qemu-system-aarch64 as an independent second referee for the same bytes the M-series CPU runs, and this Mac's GPU through Metal — and under every policy and variant (wasm, with one stack, skips the six kernel cases and says so). `probe testfloat air` runs the 19.4M TestFloat vectors on the GPU too, a thread per vector: the library is exact there at every width; the f32 instructions miss only where the GPU flushes a denormal, which the report counts apart.
- **Encoding scorecards** (`src/scorecard.rs`, `targets/*.scorecard.md`): every learned template checked against the official inventory its learner never saw — Arm's Machine Readable Architecture XML, riscv-opcodes, wabt's opcode table (`tools/get-isa-tables.sh`): the encoding the fixed bits decode to must exist and the learned fields must be its operand fields. 154/154, 93/93, 125/125 — and the cards list what the inventory has that is not learned yet.
- **An IEEE-754 oracle** (`src/testfloat.rs`): Berkeley TestFloat's vectors — 19 million cases over f16/f32/f64, every operation, every rounding mode — run through the library instances and, where the platform has the instruction, the hardware, compared bit for bit.
- **A fuzzer** (`src/fuzz.rs`): random well-formed programs — every integer width, packs, floats through the library and the platform, value-yielding `if`s, bounded loops, calls — with their results at native `-O0` and the platform off as the reference; every optimization level, the platform, wasm, and (with `--slow`) both qemu machines must agree. A disagreement is kept as a suite file that reproduces it. Its first run found wasm trapping on `MIN div -1`, which the IR says wraps.
- **Model tests** in Rust: every narrow-type op against the const-folder's model over every value pair, the softfloat ops against the FPU for f32/f64 and against an exact reference exhaustively for fp8, 128-bit arithmetic against Rust's `u128`.



## Usage

```sh
cargo build

# learn encodings (requires llvm-mc; wat2wasm for wasm)
cargo run -- learn targets/arm64.probe   -o targets/arm64.encodings.json
cargo run -- learn targets/riscv64.probe -o targets/riscv64.encodings.json
cargo run -- learn targets/wasm32.probe  -o targets/wasm32.encodings.json

# parse/verify + pretty-print (structured functions print lowered)
cargo run -- parse examples/sum.ssa

# compile and run natively (Apple Silicon)
cargo run -- run examples/sum.ssa sum 100        # -> 4950
cargo run -- compile examples/sum.ssa            # print the arm64 words

# the regression suite, per backend
cargo run -- test              # native arm64 JIT
cargo run -- test wasm         # node
cargo run -- test riscv        # qemu-system-riscv64
cargo run -- test arm-qemu     # qemu-system-aarch64
cargo run -- test --platform=arm64-noneon   # the same machine, every vector operation lane by lane
cargo run -- test riscv --platform=riscv64-nov   # likewise without RVV
cargo run -- test air          # this Mac's GPU, through Metal

# a program for the GPU: a .metallib with none of Apple's tools
cargo run -- compile examples/gpu.ssa air
python3 tools/driver_metal.py --kernel target/air/gpu.metallib target/air/gpu.air.json 8
# ... and a reduction over threadgroups of 64 (lib/gpu.ssa)
cargo run -- compile examples/reduce.ssa air
python3 tools/driver_metal.py --kernel target/air/reduce.metallib target/air/reduce.air.json 256 64
# ... and the simdgroup summing across its 32 threads
cargo run -- compile examples/simd.ssa air
python3 tools/driver_metal.py --kernel target/air/simd.metallib target/air/simd.air.json 64

# the optimization pipeline: -O<n> works on any command, and `tiers`
# compiles at every prefix to show the gradual-optimization story
cargo run -- tiers examples/tiers-demo.ssa
cargo run -- -O0 run examples/sum.ssa sum 100

# the abstract 'int' type: pick its width on any command
cargo run -- --int=i32 run suite/abstract.ssa agcd 1071 462   # -> 21
cargo run -- --index=16 run suite/index.ssa iraw_back 1       # -> 100, a 16-bit index under nothing
cargo run -- --int=i32 test wasm

# narrow types, packs, parametric types
cargo run -- run suite/bits.ssa add5 15 1          # i5: 15 + 1 -> -16
cargo run -- run suite/packs.ssa mkrgb 31 63 1     # -> 4095 (b:g:r = 1:63:31)
cargo run -- run suite/types.ssa f32exp 0x40490fdb # f32 = float(8, 23): pi's exponent, 128

# floating point is a library: on a platform with hardware for it, fadd32
# *is* the instruction; --soft keeps the library body
cargo run -- run suite/float.ssa fadd32 0x3dcccccd 0x3e4ccccd          # 0.1 + 0.2 -> 0x3e99999a
cargo run -- --soft run suite/float.ssa fadd32 0x3dcccccd 0x3e4ccccd   # same answer, ~100 instructions
cargo run -- run suite/afloat.ssa hyp 3 4                              # sqrt(3*3 + 4*4) over abstract floats -> 5
cargo run -- --float=f16 run suite/afloat.ssa hyp 3 4                  # the same program, at 16 bits
cargo run -- run suite/f128.ssa fdiv128 0 0x3fff000000000000 0 0x4000800000000000   # binary128 1 / 3

# the other number libraries
cargo run -- run suite/fixed.ssa divf 7 2                      # fixed point: 7 / 2 -> 3
cargo run -- run suite/unit.ssa pct 50 50                      # unit fractions: 50% of 50% -> 25
cargo run -- run suite/rational.ssa thirds 7                   # rationals: (7 / 3) * 3 -> 7, exactly
cargo run -- run suite/time.ssa third_plus_sixth_ms            # time: 1/3 s + 1/6 s in ms -> 500
cargo run -- run suite/wide.ssa mul 0 1 0 1                    # u128 as words, low first: (1 << 64)^2 mod 2^128 -> 0, 0
cargo run -- run suite/indirect.ssa chosen 1 10                # a function value, returned then called -> 20
cargo run -- --scalar=rational run suite/scalar.ssa sweighted 20 80   # one program, any family

# the scorecards (sh tools/get-isa-tables.sh once, to fetch the tables)
cargo run -- scorecard                           # all three, rewriting targets/*.scorecard.md

# the IEEE-754 oracle (sh tools/get-testfloat.sh once, to build it)
cargo run -- testfloat                           # every op, f16/f32/f64, nearest even
cargo run -- --round=down testfloat f32_add      # one op, one mode
cargo run -- --round=zero run suite/round.ssa sumsq 0x3f800000 0x33800000

# ISA variants: the same suite on a RISC-V core without M/F/D, and what a program uses
cargo run -- --platform=rv64i test riscv
cargo run -- --platform=rv64i footprint suite/float.ssa riscv

# what a function costs: SSA time (no target), and on a machine its K and hardware time
cargo run -- cost suite/stream.ssa biquad --assume=8
cargo run -- cost suite/stream.ssa biquad riscv --platform=rv64i --assume=8

# the fuzzer: N programs from a seed; a printed seed reproduces one program
cargo run -- fuzz 300
cargo run -- fuzz 1 --seed=65a47abe4364edd2 --slow   # the qemu machines too

# the incremental compiler. Edit the file while this runs — changed
# functions recompile in place at level 0, and functions that get hot
# are automatically promoted through the full pass pipeline.
cargo run -- live examples/fib.ssa fib 25
```

`history.md` has a short entry for every commit; `handover.md` is the working knowledge for whoever picks the project up next; `vectors.md` is a survey of where vectors, GPUs and the AI number formats would fit.

Toolchain expectations (macOS/arm64 host): `llvm-mc` (brew llvm), `wabt` (wat2wasm), `node`, `qemu`; for the GPU, `pyobjc-framework-Metal` (the driver is Python) and, to inspect what we emit, brew llvm's `llvm-dis`. The learned `targets/*.encodings.json` files are checked in, so the backends and suite work without re-learning.

## zero

`probe zero` is the front end for **zero**, the feature-modular language defined in the fm3 project (`fm3/zero.md`); milestone 0 of that project built it here, in `src/zero/`, on the `zero` branch. It reads a *store* — a folder of feature folders, each `name/name.md` (the prose: `parent:`, `layer:`, origins with timestamps, and `## testing` cases) and `name/name.zero` (the code, with no comments: a `#` is an error naming its line) — lowers it to this IR as text, and runs the cases through the same drivers as the suite. The IR is the meaning; the front end adds none of its own, and the emitted text is what a person reads when a lowering surprises them. `suite/zero/skeleton`:

```
on (int n) << answer()
    n << 42
```

`suite/zero/skeleton/skeleton/skeleton.zero:1-2`. There is one way to declare (fm3 question 77): every function is `on (results) << name (parameters)` and gives its result by pushing it, once; `=` is left for saying what a name is, `int half = x / 2`. A result with no `$` makes a plain function, one with a `$` a task or a stream processor, which produce a stream over time. The form before, `on (int n) = answer()` and `n = 42`, is refused when the program is compiled, each line with the line to write in its place. A result is pushed once, at the top level of the body, and a condition goes on the push (fm3 question 88): `r << a if (a > 0) else b`, a long one written as a table, a case a line, each line after the first beginning `else` and indented under the push. A result's push under an `if` statement or in a loop is refused showing the line to write, and so is one with `if` and no `else`, so every path gives every result; the push does not end the function, and the lines after it run. `suite/zero/pushed` is the store for the form.

```
on (int n) << how many()
    int i[] = [1, 2, 3, 4]
    n << count i[]
```

`suite/zero/sequences/sequences/sequences.zero:4-6`. An array and a stream are two kinds, and the text shows which a name is (fm3 question 90): `int i[]` is an array, whose items are all there, and `int seen$` a stream, whose items arrive. The mark is part of the name wherever it is written, `i[]` for the whole of it and `i[2]` for one item, and a name declared one way and written the other is refused naming its declaration. `$` means a stream and nothing else: `int i$ = [1, 2, 3]` is refused showing the line as an array, and each word is held to its kind, `peek`, `advance` and `latest` a stream's, an item by its place, `for` and a reduce an array's, `count` asked of both. An array becomes a stream's items by being pushed, `int i$ << [1, 2, 3]`, and `frame i$` is the array of what has arrived. A function that gives an array says so on its result, `on (int r[]) << squares to (int k)`. `suite/zero/arrays` is the store for the form, and `probe zero names <store>` prints every marked name of a store, how it is declared and each form it is used in.

```sh
cargo run -- zero suite/zero/hello emit            # the store's IR, as text
cargo run -- zero suite/zero/hello run "run()"     # one ## testing case, natively, on the real clock: the case line, the output as it lands, then → what it gave
cargo run -- zero suite/zero/hello run "run()" --fast   # ... on the virtual clock, as fast as it can
cargo run -- zero test                             # every store under suite/zero, on the native JIT
cargo run -- zero test suite/zero wasm             # or wasm, riscv, arm-qemu, air
cargo run -- cost suite/zero/hello.expected.ssa run   # the lowered IR is IR: cost, parse, footprint all read it
cargo run -- count suite/zero/lex-zeroic.expected.ssa two_arrivals --after=__zero_reset,__zero_start   # the same count, taken as the function runs
cargo run -- count suite/zero/lex-zeroic.expected.ssa two_arrivals --after=__zero_reset,__zero_start --where   # ... a row a function; --blocks=f a row a block of f; --from=f one function as the case calls it
cargo run -- zero meter suite/zero                 # the lines of each store that use a non-zeroic form, and the total
```

A whole-number literal is held to the type it is given to (fm3 log 173): `uint8 low = 300` is refused naming its line and that a `uint8` holds 0 to 255, and a literal given to an abstract `int` is refused under the products whose `int` cannot hold it, the width reaching the front end where a case is resolved. A conversion is a type applied to a value, `float(n)`, `int(x)`, `uint8(300)`, and none is refused. A `time` is written out as the language reads it, to the nanosecond, in the largest unit in which it is at least 1; with `time t = 250 ms` this writes `250 ms`, `2.5 s`, `10 s`, `333.333333 ms` and `-250 ms`:

```
    out$ << t << "\n" << t * 10 << "\n" << t * 40 << "\n" << 1 s / 3 << "\n" << 0 s - t
```

`suite/zero/types/types/types.zero:226`. The method is the platform's own text, `on (char o$) << (time x)` in `src/zero/platform.zero`, written with character codes and no string, since a string there renumbers every store's data.

A `time` is a structure declared in zero, in the feature the language brings with it (`src/zero/platform.zero`; fm3 question 117), and the only things a time can do are the functions declared on it there. It is an exact rational, a count over a divisor (fm3 question 120): `1 s / 3 * 3` is `1 s`.

```
type time =
    hidden int64 count
    hidden int64 divisor = 1
```

```
on (time t) << (time a) * (int k)
    t << time(a.count * k, a.divisor)
```

`src/zero/platform.zero:48-50` and `82-83`. The compiler keeps what no declaration can say: a literal with a unit word, `250 ms`, makes one, its nanoseconds over a thousand million, and the time words on a stream, `x$ at (t)`, take one. An operator or a `<<` method of the language's own that is one line is written in line where it is used, and there the compiler knows what a structure made in hand was given: a field read from it is that value, an operator on two whole numbers it knows is worked out, and an `if` on a condition it knows is its one arm. So `beat + 100 ms` is one integer addition, a divisor the compiler knows is never made, and a store with no time in it has no line of one; a time whose divisor is not known until the program runs, `1 s / n`, carries it, and two such are compared when they are added. A form with no function is refused in words that list what is declared, for any structure: `beat + 1` is "no '+' is defined on a time and an int: '+' on a time is `(time) + (time)`". A field declared `hidden` is read and given only in the feature that declares its type (fm3 question 118), so no program sees the count: a number out of a time is `t / 1 ms`, and `int(t)` is refused naming it. `time of x$` gives a time, the item's index over the stream's rate, and a case whose function gives a time says it as one, `→ 2, 500 ms`.

```
on a time added()
    out$ << beat + 100 ms << " " << beat + beat
```

`suite/zero/types/times/times.zero:9-10`, which writes `350 ms 500 ms`.

A stream processor may be written with no loop in it, each line holding for every item that arrives (fm3 question 75; `src/zero/zeroic.rs`, `suite/zero/zeroic`). This is the lexer of `suite/zero/lex-zeroic`, whole:

```
on (token t$) << lex (char c$)
    kind k$ = if (empty c$) then (space) else (kind of (c$))
    bool new$ = k$ == mark or k$ != k$[-1]
    index start$ = if (new$) then (position c$) else (start$[-1])
    index n$ = if (new$) then (1) else (n$[-1] + 1)
    t$ << token(k$[-1], start$[-1], n$[-1]) if (new$ and k$[-1] != space)
```

`suite/zero/lex-zeroic/lex/lex.zero:19-24`. A character's class is a name, `type kind = space | word | number | mark` (line 1), an enumeration, which lowers to a `u8`; `k$[-1]` is the class one character back, and before the first the zero of its type, `space`; `if` on the push says at which characters a token goes out; `empty c$` is true on the one tick after the last. The front end writes a function of one item for each wiring and calls it where an item is pushed into the input, with what the lines keep carried in the caller's own values, so the input needs no storage; where a block is pushed, a string literal here, the lines stand in the loop over it, and lines that turn on one condition are one branch. The same task written to walk its input, with `loop`, `peek` and `advance`, still compiles to what it did: `probe zero meter` counts the lines that use such a form, and nothing refuses them.

A case is `>call(args) → result`: a number or several, a quoted string (what the program wrote to the output device `out$`), or `check` (the call must trap), or `check at said.zero:4` (the check that fails is on that line of the zero text); a case may say *when* the text was written, every piece of the output in order with its time on the store's clock, `>run() → "10\n" at 0 s, "9\n" at 1 s, ...` (a time is a number and `s` or `ms`; the pieces joined are the whole output; two pieces at one time are one), and a piece may be given `at` a rate in place of a time, its lines one a step from 0 s or `from` a time, so hello's case is `>run() → "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n" at 1 hz, "hello world\ngoodbye" at 10 s`, a step being the same `period` the program steps by (log 97) — the virtual clock's `__wait` leaves a mark each time it moves, the time it reached stored at the place in the output where that time begins to apply, a word for each byte of the capture and four operations a wait, the runner reads a word for each byte of the text and one more on every path and cuts the text where a word is not zero, and a failure prints what was written in the same spelling, a regular run of three lines or more folded back into the rate form, as `run "run()" --fast` does; `>run() with countdown off → ...` runs it in a context with that feature off, which the runner sets between the program's reset and its start, since no feature code switches a feature; `>lexed() with in "let x = 4" → 3` pushes the string into the platform's input device `in$` there too, before the start, so a task wired `token t$ = lex(in$)` has run over it when the call is made; a program reads `in$` and never writes it, a push into it and `end in$` being refused, and `in$` handed to a function that pushes into or ends the stream it is given being refused where it is handed (log 106), so a store that makes its own arrivals pushes them into a stream of its own, as `suite/zero/lex` does into `src$` with the same task wired `token u$ = lex(src$)` (log 101). `zero test` runs every case with every feature on and once per feature of the store with that feature off alone, and a case must hold in every context where its own feature is on. Tests are functions: a feature's cases for a method are its definition of that method's test, composed as the method is — the newest feature that is on replaces the older feature's cases for the method unless its `## testing` says `>existing` on a line of its own, and a feature that is off drops out, so the older cases stand again (`hello`'s `>run() → "hello world"` is replaced by `countdown`'s `run()` and that by `bye`'s; `functions/more` says `>existing`); each replacement is reported as `over`. A feature is on when its own `enabled` field and every ancestor's are, and the context holds that already worked out for each feature under another, `__on_<feature>`, so every gate is one field loaded and one branch however deeply the feature is nested (fm3 question 72); it is worked out again only in a switch's setter, for the feature and everything under it, and a feature's own switch is written by nothing else, so a parent off and on again leaves its children as they were — `suite/zero/nested` is a chain three deep whose case lines switch each level by turns, `>reach() with two off, one off, one on → 11`, the runner making a line's switches in order, a setter's call each; the context is `this` (fm3 question 71): a function that touches it names the current context's pointer once, its first line `_this: ptr = context()`, which is a register on arm64 and riscv64 and no instruction there (`ssa.md`, *The current context*), so a store may have any number of contexts and its code is the same for all of them — the runner makes the store's first the current one with `__zero_context(0)` before `__zero_reset`, a second is made beside it with `__zero_context(1)` and `__zero_new()`, and `two_contexts_each_keep_their_own` runs hello and a lexer in two by turns, each keeping its own switches, its own word in progress and its own variables; every read of a feature-scope variable, a switch or a node's state is a `load` there and a `get`, every write a `load`, a `set` and a `store`, which the IR dissolves to the one field, with no accessor's call, and a field nothing in the store writes is fetched once a function, a later read under an earlier one being the earlier value (`suite/zero/variables`' `seen round a bump` and `suite/zero/streams`' `counted round a skip` read a field something does write on both sides of the call that writes it), log 110; a case line is a sequence of switches, `with more off, base off, base on`. The last line counts the runs, cases times contexts. A feature's header may say `published: <date>`, after which its promise is kept by its cases (fm3 question 89): where the store is a git repository the reader notes an uncommitted change to its `.zero`, or a commit dated after the publication, and `probe zero <store> run` and `probe zero test` then run the feature's own cases first, the lines of its own `## testing`, and refuse the store if one fails, naming it; a change to those cases, or to the code of a published feature that has none, is refused where the store is read. Every store composes the compiler's own `platform` feature first (`src/zero/platform.zero`), which declares the output device `char out$` and the input device `char in$` and `print` as a word over `out$` (a program writes `out$ << "hello"` itself, and the runner reads back what was written after the call), together with the `<<` methods that format an `int`, a `float`, a `bool` or a sequence of numbers into a stream of characters — `out$ << 42` writes the digits, `on (char o$) << (pair p)` in a store adds a method for its own type, and a struct without one is its fields with spaces. `out$` is not a buffer: it looks like a stream, but a push into it is the platform's write and stores nothing, so it has no ring, no capacity and no consumer to schedule, and under the runner the place it writes to is the test capture (`char` is a type of its own, distinct from `uint8`, so that `char$ << 42` asks for text where `uint8$ << 42` would ask for a serialisation; a sink, a function with a stream parameter and no result that a bare wiring line makes a task, is wired to a stream of the store's own, as `suite/zero/platform`'s `sink` shows); a store's own platform functions carry a body per kind of place, an IR target's rule lines under a `platform <target>` line or IR under `ir`, and a case that reaches a function with no body for the path is skipped saying so. A stream is timed only when something asks for time: a rate on its declaration or wiring, or `x$ at (t)`, `x$ from (a) to (b)` or `position x$` applied to its name anywhere in the store (a time word on a function's stream parameter times the whole store); every other stream, `in$` included, is a plain ring with no ticks, and a store with no ticked ring pushes with no clock branch. `suite/zero/<store>.expected.ssa` beside a store is the IR it emitted when accepted, re-checked by `zero test`. A `product.md` beside a store's feature folders carries the product's settings, the numbers feature code never states: `bound <function words>: N`, a trip count the front end puts onto that function's loops for `probe cost`; `int: 32` or `64` and `float: 32` or `64`, the widths the store is built at; `clock: real` or `virtual`, whether a wait is on the machine's counter (a `platform arm64` body, `mrs r, cntpct_el0`) or a jump of the virtual clock — `run` takes real unless told `--fast`, `test` always virtual, and the one function that differs is `__wait(t)`; a push from a plain function into a stream declared with a rate moves the clock on a step through it after the item, so `out$ << i$ forever` over `int i$ at (1 hz)` writes a number a second and ten numbers take ten seconds, and where the stream is stored the nodes below it run between the push and the step, so a task between a rated stream and its output keeps the rate (`suite/zero/timed`, log 93); and a mark per feature, `<feature>: static on`, `static off` or `dynamic` (dynamic where none is written) — static on and the feature has no switch, no gate and no context field, its chain body standing under its link's name so the chain is called by name; static off and the feature and everything under it are not in the program; `suite/zero/static` is hello with every feature static on, the counterpart of the hand-written oracle, and `suite/zero/marks` shows the three. The emitted IR is one text on every path: a name whose methods are all concrete is one method set in the IR, and a bare literal between two widths is emitted as `width_of(3: int)` for the policy to choose the method (see *Method sets* in `ssa.md`). The stores under `suite/zero/` are one per plan item — skeleton, functions, types, control, variables, sequences, streams, tasks, features, checks, platform — with `edges` (an edge, and a stream with no storage), `timed` (a task between a rated stream and the output) and `marks`, and three programs, `hello` (section 16 of zero.md, a countdown at one hertz on the store's virtual clock, an integer tick counter in microseconds that a rated task moves by whole periods; a push stamps a tick and exact time is computed only at the boundary, `x$ at (t)`; its countdown is wired into the output by an edge, `out$ << (i$ << "\n") forever` at feature scope — `forever` is what makes a `<<` stand (fm3 question 79): a `<<` sends once each time its line runs, wherever it is written, and with the word it is a standing connection, each item moved by the dispatch a push uses and the rest of the chain pushed after it; `if (c)` before the word makes it a filter, and the same line with no `forever` is refused, saying what to write for wiring and for one push. Storage depends on the words applied to a stream: a history or a time word makes a ring, a reading word a queue, and a stream no word reads, only pushed into and wired by edges, has none at all — its edge is a function of one item, `__edge1(__item: int)`, a push into the stream is the call of each edge out of it under that edge's feature's gate, and a step of the rate then passes, so hello has no queue, no node and no scheduler, log 92; a stream with a rate has a beat, slots one period apart from 0 s, and a push statement into it first moves now to the stream's next slot, `add`, `rem`, `sub` and a `__wait`, 17 SSA, skipped where the statement before pushed into the same stream, so one function pushing by turns into a `2 hz` and a `5 hz` stream writes at 0 s, 600 ms and 1 s (`suite/zero/edges`' `turns`), log 98; the front end knows at each point of a plain function what the clock is a whole multiple of — 0 s where a case started it, a period after a push at a rate, the weaker of two paths, the weakest of a function's callers — and a push whose stream's period divides that is not aligned, so hello's `count down`, only ever reached at 0 s, rounds nothing, and a loop whose first statement is a rated push finds the slot once, before the loop and under the loop's own `while`, where it found it on every pass (`suite/zero/edges`' `drum`, `suite/zero/timed`'s `one` and `later`), log 99; a stream that is pushed into and that nothing reads or wires has no storage either, a push into it being nothing but that step, so a feature the product marks `static off` and the same feature switched off are one program (`suite/zero/unwired`), and where no feature of the store reads or wires it, compiled in or left out, the store is refused as a mistyped name, log 96; a stream that is declared and named nowhere else, not even pushed into, has no storage either and is not refused, since it may be declared ahead of the feature that will use it (`unwired`'s `spare$`), log 100. Where a stream is stored its edge is a node: a `for` over the unread items, one view and one load each, then one advance, run by a scheduler that is a static schedule where the node graph is acyclic — one pass in producer-before-consumer order, and after a push from a plain function only the nodes the pushed stream reaches, `__run_x()`, with no loop and no empty pass, the loop kept for a cyclic graph, log 78; a node of an acyclic graph asks how much its input holds, and whether it has ended, once a run and not again after its task returns, no node there being able to push into or end what it reads, log 102; a node whose one input only plain functions push into and end, none of them reachable from anything the scheduler runs, is not asked whether it is due at all: the push statement calls its task in line under its feature's gate, under whether anything arrived where the statement might push nothing, and the stream's first `end` does the same, so the node keeps its reader and no `seen`, no `fin` and no `__node` function, and a store where no trigger can be met while a node runs has no `__running` guard (`suite/zero/lex`'s `arrive first` is the push and then `lex(...)`; `suite/zero/tasks`' `nothing pushed`, `ended twice` and `a batch` pin what a program could have seen), log 103; such a node keeps its position, a word, and not a whole reader where its task only reads and advances its parameter, the rest of the reader being the stream's own value, which the push or the `end` has in hand, and its whole reader where the task might give back another ring (`suite/zero/tasks`' `hopping`, which assigns its parameter), log 111; a second `count` of the same reader value is the first's number where no line between could push, so the lexer asks how much is waiting once a turn: between is what stands between the two askings and the whole body of any loop the second is in and the first is not, a line is harmless by a list of what is allowed, arithmetic, comparisons, loads, branches, the library's reading words and a call to a function of the store whose own emitted body and all it calls are only such lines, and everything else gives up, every push, `end`, store and unknown word (`suite/zero/streams`' `counted round a push`, and lex's `two arrivals`, which counts on both sides of a call that pushes), log 112; a queue gives its slots back by who reads it and not by how a parameter is spelled, a function's own stream parameter being no reading of the feature's stream of that name, so a sink `soak (int x$)` wired to a stream `x$` takes a hundred items through a queue of 64 (`suite/zero/edges`' `soaked from`), log 113; a word of a function's own name is not a reading of a stream, so a call of `a part summed down from (k)` leaves `part$` without storage, a phrase that is a call of a store's function mentioning its arguments alone (`suite/zero/edges`), log 106; a queue's push asks whether its stream has ended only where it could have, `ended` being written by `end` alone: a push through a name of element type T is the library's `push_queue_open`, which checks for room and nothing else, unless some `end` in the store has an operand whose element type T fits, that fits T, or that is the same type in the IR, and every push keeps the check in a store with a `platform` body of its own (`suite/zero/checks`' `pushed after end`, `pushed after shut` and `pushed steadily`), log 108; a string literal of fewer than sixteen bytes pushed into a queue lands an item at a time from its `data`, `push_queue_few(s, p, n)`, with no view and no `copy`, sixteen bytes being the smallest chunk the machines have and a block of sixteen or more staying the chunked copy it was (`suite/zero/types`' `said fifteen` and `said sixteen`), log 109; a queue's header says where slot 0 is, in the word where a ring's keeps its buffer, the address past the buffer's own header, added once when the queue is made, so every queue word that touches a slot loads a typed pointer where it loaded the buffer, added sixteen and cast (`suite/stream.ssa`'s `queue_slot_zero` and `queue_views`), log 115 and 119; a list or a range, a new stream with its items present, is filled by the queue's own push where it is a queue, so it reads back right at any length (`suite/zero/control`'s `sum to (100)` and `listed item (0)`), log 119; the edge's loop carries `bound N`, the most items one push statement from a plain function pushes into its source when every such push is countable, log 79; and a node whose feature is off moves its reader past what arrives, a consumer that is off holding nothing, log 79), `lex` (the lexer as a task) and `clock`; each reads as the documentation of its construct. `fm3/milestone-0/log.md` has every judgement the build made. **A failed check says which line it came from and what was asked**, `a failed check at said.zero:4: the stream `kept$` is full: 64 items pushed and nothing has read them`, and the program pays nothing for it: where a case stops, the runner lowers the same store a second time as a *diagnostic build*, in which each statement first stores its site and each checked read or push the numbers it is about to use, runs the case again, and reads the site back through the two platform words every path's host already calls after a stop; `probe zero <store> emit sites` prints that build and its table. The same on the native JIT, wasm, riscv64 and arm64 under qemu; the GPU's path does not stop at a failed check.

Nothing that changes is assigned: it is a stream, its value its latest item and a write a push (fm3 questions 70 and 79; `suite/zero/cells`).

```
int seen$ << 0

on bump()
    seen$ << seen$ + 1

on (int n) << bumped twice()
    bump()
    bump()
    n << seen$
```

`suite/zero/cells/cells/cells.zero:9` and `:18-24`. A `<<` sends once, each time its line runs, and a stream's name where one value is wanted is its latest item. A stream the store reads only so is a **cell**: one field of the context of the item's type, a push a store of it and a read a load, with no ring and nothing that can fill; `bump` lowers to the four lines an assigned variable did. Apply one more word to the stream, `count seen$`, and it is the queue it would have been, the name reading the same.

The words after a push's items say how often it happens (fm3 question 79; `suite/zero/words`): `if (c)` first, only where the condition holds, and then one of `(n) times`, `while (c)`, `until (c)` and `forever`.

```
on up to (int k)
    up$ << 0
    up$ << up$ + 1 (k) times

on called (int k)
    up$ << twice (k) (3) times
```

`suite/zero/words/words/words.zero:42-44` and `:56-57`. `(n) times` is n pushes, the count worked out once before the first and each push working its item out again, so `up to (4)` writes `0 1 2 3 4`; in a chain the count covers the last item, as `while` does. A bracketed group that stands directly before the word `times` is the count and never an argument, unless a declared function's name has `times` there, so `twice (k) (3) times` is `twice (k)` three times. At feature scope `first$ << src$ (3) times` is a line that stands for the first three items of `src$`, its count a number of the context kept for the line.

```
on counted to five()
    up$ << 1 << (up$ + 1) until (up$ == 5)

on counted under five()
    up$ << 1 << (up$ + 1) while (_ < 5)
```

`suite/zero/words/words/words.zero:99-103`. `until` pushes and then asks, so the item that makes its condition true goes out, `1 2 3 4 5`; `while` asks of the candidate before it pushes, `1 2 3 4`. In an `until`, `_` and the stream's own name are both the item just pushed. `if` goes with any one of the four words of how often and comes first; two of the four on one push are refused by name, and so is a push that can be seen never to end, `until (false)` and `while (true)`. `till$ << flow$ until (flow$ == 3)` at feature scope stands until the condition holds, a bit of the context kept for the line.

```
int x$
int sum$
```
```
sum$ << sum$ + x$ forever
```

`suite/zero/words/words/words.zero:21-22` and `:28`. A running sum, one item of `sum$` for each item of `x$` (fm3 question 80): on the right of its own standing push a stream's own name is a read of its latest item and sets nothing off, and the one other stream there paces the line. `sum$` is read only by its name, so it is a cell and the line's function is a load, an add and a store, 7 an item counted as it runs where the same sum as a stream processor read by name is 26. Wired on, `tot$ << tot$ + y$ forever` and `out$ << (tot$ << "\n") forever`, the stream has no storage and the line keeps its last item itself, 86 an item with the number's digits where the processor's form is 83. Two other streams on the right are refused until what paces such a line is settled (question 86), and a line with nothing else on its right is a clock at a rate, ruled and not built, and never ending without one.

```
on three lines()
    out$ << ("hello" << "\n") (3) times

on one line()
    out$ << "hello" << "\n" (3) times
    out$ << "."
```
```
on pairs under six()
    up$ << 0 << (up$ + 1 << up$ + 1) while (_ < 6)
```

`suite/zero/brackets/brackets/brackets.zero:16-21` and `:32-33`. A word on a push applies to the last item of its chain, and brackets round several items joined by `<<` make them the one item it applies to (fm3 question 84): three lines of hello, where the line without the brackets is hello and three newlines. The bracketed items are pushed in order each time the word has them pushed. Under `until` the group is pushed and then the condition asked, `_` its last item; under `while` the group is the candidate, whole, its items worked out and held, the condition asked with `_` the last of them, and all pushed where it holds and none where it fails, so the pair 5, 6 fails as a pair and `pairs under six` writes `0 1 2 3 4`. A bracket is a group where a `<<` stands directly inside it, which no bracketed value has; brackets round one item are ordinary grouping. A group stands last in its chain.

```
out$ << (up$ << "\n") forever
out$ << (said$ << "\n") forever
word greet$ at (1 hz)
int flow$
out$ << (greet$ << "\n") (3) times
```

`suite/zero/brackets/brackets/brackets.zero:6-10`. `forever` follows the same rule, so a wiring line of more than one item is written with brackets: every item of `up$`, each with a newline after it. Without them the line would be `up$` once and then a newline for ever, which nothing paces; it is refused when the program is compiled, and since every such line was written when the word covered the whole push the message shows the program's own line with the brackets. A count and an `until` on a line that stands cover the bracketed chain the same way: the first three items of `greet$`, a line each, three lines of hello a second apart at `1 hz`. Where items stand before what the word covers, `out$ << "values: " << i$ forever`, they would be pushed once when the store starts, which is not built.

### The front end in a page

Everything on the way from a store's text to an emitted wasm module builds for `wasm32-unknown-unknown`, so the compiler can run where there is no disk and no process: in a web page. Three things make that so. `src/vfs.rs` is where the compiler reads from: a store's folders (`zero/store.rs`), a refusal's own line (`zero/kinds.rs`, `zero/lower.rs`), the IR's libraries (`ssa::with_prelude`), the wasm encodings (`emit_wasm::WEncoder::load`) and a platform file (`platform::Platform::load_named`) all read through it. With nothing mounted each read is `std::fs`'s and probe is as it was; from a host's first `vfs::mount(path, text)` every read on that thread is of what was mounted and of nothing else. `src/host.rs` holds what a host that compiles and does not run wants of the suite's runner: the policy a path compiles under (`backend_policy(Backend::Wasm)`), and how a call is described to the wasm driver, `wasm_args`, `wasm_rets` and `wasm_cases`, which is the spec `src/driver.js` reads. And a host that wants the time of everything a program writes sets `Store.times` before lowering, which keeps the marks' reader `__out_mark` as a case that asserts on time does.

A host is a crate of its own, a `cdylib`, that names probe's files by `#[path]`: `aggregate`, `emit`, `emit_wasm`, `host`, `opt`, `platform`, `regalloc`, `ssa`, `structure`, `vfs`, `wide`, `wlearn`, and `zero`'s `kinds`, `lex`, `lower`, `run`, `store`, `syntax` and `zeroic`; it carries the texts of `lib/*.ssa`, `targets/wasm32.encodings.json` and `targets/wasm32.platform` and mounts them. `zero/run.rs` holds the runner beside what makes a module, so the host supplies one function in a module named `suite`, `run_calls`, that refuses. The zero playground (zero.nøøb.org) is such a crate, and from this tree as it is, with no patch, it builds with

```
cargo build --release --target wasm32-unknown-unknown
```

run in the host crate's folder. The module it emits for a store is, byte for byte, what `probe zero test <store> wasm` writes (checked on the playground's eighteen examples when this landed; `history.md`).

## Status

What is here:

- **Types.** Integers to 256 bits, packs, structs, views of rank 1 to 3 (`f32[]`, `f32[,]`, `f32[,,]`: a typed pointer and a count and stride per axis into a buffer with a header — rows, columns, blocks and transposes are views of the same memory; operations over the whole view, reductions to a scalar or one rank down), streams (`u8$`: a reader's view of a ring over time, sampled by a rule), a tower of abstract types (`number` over `int` and `scalar`) that a function may be written over — a template bound by its argument, the most specific definition winning — vectors of 64 or 128 bits with lanes of 8 to 64 bits (`f32x4`, `u8x8`, `i16x8`, ...: per-lane by definition; whole on the GPU, NEON on arm64 and RVV on riscv64, each checked against the lane form), typed pointers and shaped arrays, parametric types and generic functions, function values.
- **Numbers as libraries.** Floats, fixed point, unit fractions, rationals, time and decimals, with hardware substituted where a platform has it.
- **Memory as a ladder of lifetimes**, not a heap for objects: `scratch`, arenas, pools, a buddy heap over the rest of RAM as the root; `check` is the one assertion, a breakpoint trap that names its site.
- **Four backends and their variants**, the fourth Apple's GPU through a `.metallib` we write ourselves; on the machines each class of value — integer, float, vector — crosses a call in its own registers (and past eight, on the stack), and a caller from Rust reaches classed functions through a wrapper the compiler generates.
- **The GPU's model everywhere.** `group` memory, a barrier and the simdgroup operations are library functions that mean the same on a machine — fibres run a group by turns on one thread, a simdgroup is an exchange through a table; `thread()` reads a platform-named register; a kernel's groups are dealt across OS threads under the JIT, or across four cores on the qemu boards.
- **Six bootable kernels.** Hello world; an echo with system calls and interrupt-driven input; a clock on the timer interrupt; two preempted tasks; tasks sleeping to exact times on a tickless timer, with `at`/`after` callbacks and tasks that come and go; four cores sharing a kernel's groups.
- **All of it differentially verified.** The suite on five execution paths under every policy, `;! __kernel` running a program's kernel on the GPU and on every machine, `-> check` for what must fail (a thread that skips a `simd_*` fails a check on every machine, where the GPU reads stale words), the oracle (on the CPU and the GPU), the scorecards, the fuzzer (`--air` puts the GPU in its panel), the model tests.

Deliberately not here yet (`future-work.md` has the queue, `handover.md` the reasons):

- A kernel's threads as real worker threads on wasm.
- Tasks dealt across cores in the OS programs.
- Textures as handles with platform operations; WebGPU as a second GPU path.
- A chunk wider than 128 bits (RVV's `vl` on a machine with a wider VLEN: the chunk width is the platform's constant, not the chip's); wasm SIMD through the same seam; shuffles and dot products.
- Capacity analysis for arenas and stacks.
- External (libc) calls from JIT'd code.
- Differential testing against clang, to close the semantic loop the way the prober closed the encoding loop.

## Credits where due

Claude Fable 5 wrote all this code; the concept and direction is mine.