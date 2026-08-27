# probe

A compiler back-end that **learns instruction encodings by probing a
toolchain** instead of transcribing them from architecture manuals.

The lowest stage of a compiler — turning IR into machine-code bytes — is
usually built by hand-copying bit layouts out of thousand-page reference
PDFs, which is exactly the kind of transcription that breeds bugs. This
project takes a different route: generate lots of tiny probe programs,
feed them to an existing assembler (llvm-mc, wat2wasm), diff the bytes
that come back, and *derive* the encoding — the fixed opcode bits, where
each operand field lives, how immediates are encoded. Nothing is trusted
until it survives randomized verification against the oracle; anything
nonlinear or surprising is reported as unlearned rather than silently
mis-encoded.

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

`os/hello.ssa` is the first operating system written in probe: a
`data` string (an array of UTF-8 bytes), a loop storing each byte to
the board's UART (`platform uart`, a constant from the platform file),
and an exit through the board's finisher or a PSCI call (a platform
rule with `hvc` as its body). No runtime, no linker script, no
assembly: the image is the learned encodings and a six-instruction
preamble that sets the stack pointer.

```sh
printf 'hello\nbye\n' | cargo run -- boot os/echo.ssa arm
> echo: hello
> 
```

`os/echo.ssa` is the second: a kernel that installs a trap handler
(`fn __trap`, compiled with a frame that keeps the interrupted code's
registers and returns by `eret`/`mret`) and serves write, read and exit
through a `data` table of function values, and a program that uses
nothing but those system calls (`svc`/`ecall`). The trap instructions
are learned like every other; how a board takes a trap is five
platform rules and two constants.

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
a 100000 +5415    # letter, scheduled wake in µs, lateness
c 142857 +3679
...
18 interrupts, worst +7554 us
```

`os/sleep.ssa` is the fifth: three tasks sleeping until exact times —
every 1/10, 1/3 and 1/7 of a second — woken in the order the fractions
say, 3/3 and 7/7 and 10/10 of a second being one instant, and the
timer armed to the next deadline rather than to a tick: twenty wakes,
eighteen interrupts. A task giving up the cpu asks for a software
interrupt (`reschedule`), so every switch still happens in `__irq`.

`os/tasks.ssa` is the fourth: two tasks preempted by the timer. The
interrupt handler is `fn __irq(sp: ptr) -> ptr` — handed the frame
holding the interrupted task's whole register file, answering with
the frame to resume — so a task is a stack and a place to resume, and
a switch is two stores and two loads.

`os/clock.ssa` is the third: it keeps time. The timer interrupt lands
in `fn __irq` (a frame that keeps every register, float scratch
included), each deadline is one step on from the last so the ticks
never drift, and the elapsed count of the machine's counter becomes
milliseconds exactly, through `lib/time.ssa`'s rationals. The generic
timer and GICv2 on one board, the CLINT and `time` CSR on the other,
are eight platform rules — some with typed temporaries (`irq_on() ->
() with gic: ptr, v: u32`) for the addresses and values they need.

## The IR

`ssa.md` is the reference; `src/ssa.rs` the parser, verifier and
printer. In outline:

- **Integers of any width from 1 to 256 bits**, signed or unsigned
  (`i5`, `u23`, `i64`, `u128`, plus `ptr`). Signedness lives in the
  type, so there is one `div`, one `shr`, one `cmp.lt`. Above 64 bits a
  value is lowered to a row of words right after parsing
  (`src/wide.rs`), so no backend ever meets one.

- **Packs**: bitfields laid out lowest-bits-first in up to 256 bits —
  `type rgb = pack { r: u5, g: u6, b: u5 }` — nestable and storable.
  A pack is its bit pattern.

- **Structs**: fields side by side, in memory at natural offsets and in
  registers as separate values. Never a bit pattern — no `cast`, no
  literal — which leaves the layout to the compiler. Dissolved into
  their fields after parsing (`src/aggregate.rs`).

- **Parametric types and generic functions**, instantiated by width:
  `type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }`,
  `type f32 = float(8, 23)`, `fn add(E, M)(a: float(E, M), b: float(E, M))`.
  An opcode on a pack dispatches to the generic of that name, so
  `add x, y` on two `f32` values *is* the library's function — or the
  platform's instruction — and a library can add operations of its own
  (`sqrt x`).

- **Literals wherever the type is known** (`add a, 1`, `mul x, 0.5`,
  `ret 0`), float literals rounded exactly; block parameters instead of
  phi nodes; multiple return values; `load`/`store` with `base, off`
  and `base, index, step` addressing; an optional structured front-end
  (`if`/`loop`/`break`/`continue`/`yield`) that lowers to the flat
  block graph at parse time.

- **Data**: `data greeting = "..."` is an array of UTF-8 bytes, `data
  table: array(i32, 4) = { ... }` an initialized array, `addr` and
  `len` reach it, and `platform uart` is a constant the platform file
  provides per board.

- **Scratch**: `p: ptr = scratch 64` is memory that is the function's
  while it runs — its frame, or a shadow stack on wasm.

- **Function values**: `fn(i64, i64) -> i64` is a type — the signature —
  and `f: binary = addr add64` a value of it, taken with the same `addr`
  that reaches data. Calling it, `r: i64 = f(a, b)`, is spelled like any
  call and checked like one; the value goes anywhere a value goes,
  including memory. `adr`+`blr` on arm64, `auipc`+`jalr` on riscv64, a
  table and `call_indirect` on wasm.

- **Abstract types resolved by policy**: `int`/`uint` take a width per
  target (`--int=i32|i64`); `float`, `fixed`, `unit`, `sunit` and
  `rational` resolve to the libraries' `float(E, M)`, `fixed(I, F)`,
  `unit(N)`, `sunit(N)`, `rational(N, D)` (`--float=`, `--fixed=`,
  `--unit=`, `--sunit=`, `--rational=`, `--round=` for the rounding
  mode); `scalar` is whichever family the policy names (`--scalar=`).

## The libraries

Every file compiled gets `lib/*.ssa` appended. Number formats are
libraries, never compiler features; `formats.md` is the recipe and
`/format` scaffolds one.

- `lib/float.ssa` — IEEE floats over `float(E, M)`: add, sub, mul, div,
  sqrt, neg, abs, min, max, fma, the comparisons, conversions, every
  rounding mode; one body per operation, instantiated from fp8 to
  binary128.
- `lib/fixed.ssa`, `lib/unit.ssa` — fixed point, and fractions of one
  (signed and unsigned).
- `lib/rational.ssa` — exact rationals, reduced, in 128 bits.
- `lib/time.ssa` — a `rational(64, 64)` of seconds with units: a sample
  period at 44100 Hz times 44100 is exactly one second.
- `lib/decimal.ssa` — `decimal(N, S)`, an `i(N)` significand at scale
  10^S: cents that add exactly.
- `lib/wide.ssa` — division (and, on a core without a multiplier,
  multiplication) for wide integers.

## The learners

- `src/learn.rs` for fixed-width register ISAs: one-hot probes XORed
  against a baseline map each operand bit to its encoding bit — which
  handles RISC-V's scrambled branch immediates exactly as easily as
  ARM's contiguous fields. Nonlinear small domains become lookup tables;
  nonlinear large ones (ARM logical immediates) fail honestly.
- `src/wlearn.rs` for byte-oriented stack machines (wasm): templates are
  fixed bytes plus LEB128 codecs at discovered positions, probed through
  wat2wasm with a bootstrap chain of stack context.
- Both talk to an assembler only through `src/oracle.rs`; the per-target
  seed files (`targets/*.probe`, read by `src/target.rs`) say how to
  *spell* instructions and nothing else.

## Platforms and variants

A platform file (`targets/*.platform`, read by `src/platform.rs`) says
what a target does natively, as rules over the library's operations:

- `class s = f32` — f32 values live in `s` registers. The allocator
  (`src/regalloc.rs`) keeps each value in its class's file, so a chain
  of float operations compiles to the instructions alone.
- `fadd {s}, {s}, {s} = add(f32, f32) -> f32` — one learned instruction
  for one library operation. Rules can only name templates the learner
  verified. Compiling such an instance, or a call to one, emits the rule
  instead of the SSA body; `--soft` turns that off, and the library
  remains the reference the hardware path is checked against.
- `const uart = 0x10000000` — a board's addresses, for `platform uart`.
- `psci(code: u64) -> ()` with `hvc 0` under it — a plain function the
  platform gives a body: how the board is ended, how a trap is
  installed, read and returned from (`vectors`, `cause`, `resume`,
  `resume_at`, `syscall`), how time is read and the timer's interrupt
  taken (`now`, `hz`, `timer_at`, `irq_on`, `irq_ack`, ...). A body
  line may spell a template's fixed operands (`msr vbar_el1, t`); a
  rule may declare typed temporaries (`with gic: ptr, v: u32`) for
  addresses and values it needs; `none` is a rule that does nothing.
- **Variants** are files too: the base file is grouped by extension
  (`ext M`, `ext F`, `ext D`), and `targets/rv64i.platform` is `base
  riscv64` plus `without M, F, D` — a core on which `mul`/`div`/`rem`
  and every float operation are the library's. `--platform=rv64i`
  selects it everywhere; `probe footprint` lists the instructions a
  program really used, to prove it.

## The backends

None of them contains a single hand-written opcode.

- `arm64` (`src/emit.rs`) — JIT: mmap/MAP_JIT on Apple Silicon, run
  in-process; also bare metal under qemu-system-aarch64.
- `riscv64` (`src/emit_rv.rs`) — bare metal on qemu-system-riscv64, with
  the runtime harness (UART printing, exit) generated in the project's
  own SSA.
- `wasm32` (`src/emit_wasm.rs`) — module emission, executed by node via
  `src/driver.js`; control flow becomes nested `block`/`loop`/`if` from
  the dominator tree (`src/structure.rs`), a dispatcher loop only for an
  irreducible graph.

Shared by all three:

- **A linear-scan register allocator** (`src/regalloc.rs`) with register
  classes: the emitter hands it pools and gets back a register or spill
  slot per value.
- **An SSA pass pipeline** (`src/opt.rs`): simplify-cfg, const-fold,
  dce, sink. Optimization levels are prefixes of that one list, so every
  level is a correct stopping point and every pass is checked by the
  suite on every backend.
- **An incremental JIT arena** (`src/arena.rs`): each function in its own
  slot with slack, calls routed through counting trampolines, so an
  edited function recompiles in place and a hot one is promoted through
  the full pipeline without disturbing its neighbours.

## Verification

- **One regression suite** (`suite/*.ssa`, runner in `src/suite.rs`):
  747 cases with expectations embedded as `;! gcd 48 36 -> 12`
  directives, run identically against every backend — including arm64
  under qemu-system-aarch64 as an independent second referee for the
  same bytes the M-series CPU runs — and under every policy and variant.
- **Encoding scorecards** (`src/scorecard.rs`, `targets/*.scorecard.md`):
  every learned template checked against the official inventory its
  learner never saw — Arm's Machine Readable Architecture XML,
  riscv-opcodes, wabt's opcode table (`tools/get-isa-tables.sh`): the
  encoding the fixed bits decode to must exist and the learned fields
  must be its operand fields. 154/154, 93/93, 125/125 — and the cards
  list what the inventory has that is not learned yet.
- **An IEEE-754 oracle** (`src/testfloat.rs`): Berkeley TestFloat's
  vectors — 19 million cases over f16/f32/f64, every operation, every
  rounding mode — run through the library instances and, where the
  platform has the instruction, the hardware, compared bit for bit.
- **A fuzzer** (`src/fuzz.rs`): random well-formed programs — every
  integer width, packs, floats through the library and the platform,
  value-yielding `if`s, bounded loops, calls — with their results at
  native `-O0` and the platform off as the reference; every optimization
  level, the platform, wasm, and (with `--slow`) both qemu machines must
  agree. A disagreement is kept as a suite file that reproduces it. Its
  first run found wasm trapping on `MIN div -1`, which the IR says wraps.
- **Model tests** in Rust: every narrow-type op against the
  const-folder's model over every value pair, the softfloat ops against
  the FPU for f32/f64 and against an exact reference exhaustively for
  fp8, 128-bit arithmetic against Rust's `u128`.

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

# the optimization pipeline: -O<n> works on any command, and `tiers`
# compiles at every prefix to show the gradual-optimization story
cargo run -- tiers examples/tiers-demo.ssa
cargo run -- -O0 run examples/sum.ssa sum 100

# the abstract 'int' type: pick its width on any command
cargo run -- --int=i32 run suite/abstract.ssa agcd 1071 462   # -> 21
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

# the fuzzer: N programs from a seed; a printed seed reproduces one program
cargo run -- fuzz 300
cargo run -- fuzz 1 --seed=65a47abe4364edd2 --slow   # the qemu machines too

# the incremental compiler. Edit the file while this runs — changed
# functions recompile in place at level 0, and functions that get hot
# are automatically promoted through the full pass pipeline.
cargo run -- live examples/fib.ssa fib 25
```

`history.md` has a short entry for every commit; `vectors.md` is a
survey of where vectors, GPUs and the AI number formats would fit.

Toolchain expectations (macOS/arm64 host): `llvm-mc` (brew llvm), `wabt`
(wat2wasm), `node`, `qemu`. The learned `targets/*.encodings.json` files
are checked in, so the backends and suite work without re-learning.

## Status

Integers to 256 bits, packs, structs, parametric types and generic
functions, function values; floats, fixed point, unit fractions, rationals, time and
decimals as libraries, with hardware substituted where a platform has
it; three backends and their variants; four bootable kernels — hello
world, an echo with system calls, a clock on the timer interrupt, two
preempted tasks, tasks sleeping to exact times. All of
it differentially verified — the suite on four execution paths under
every policy, the oracle, the scorecards, the fuzzer, the model tests.

Deliberately not here yet: vectors (`vectors.md` says how they would
go), external (libc) calls from JIT'd code, a dominance check in the verifier, and differential testing
against clang to close the semantic loop the way the prober closed the
encoding loop.

## Credits where due

Claude Fable 5 wrote all this code.
