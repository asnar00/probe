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

## What's here

- **An SSA IR** (`ssa.md`, `src/ssa.rs`): integers of any width from 1 to
  64 bits, signed or unsigned (`i5`, `u23`, `i64`, plus `ptr`) — signedness
  lives in the type, so there is one `div`, one `shr`, one `cmp.lt`;
  `pack` types that lay bitfields out lowest-bits-first in up to 64 bits
  (`type rgb = pack { r: u5, g: u6, b: u5 }`, nestable, storable), and
  parametric declarations instantiated by width — `type float(E, M) =
  pack { mantissa: u(M), exponent: u(E), sign: u1 }`, `type f32 =
  float(8, 23)` — and generic functions monomorphized the same way
  (`fn add(E, M)(a: float(E, M), b: float(E, M))`, `fn fadd32 =
  add(8, 23)`) — so `add x, y` on two `f32` values is that function, or
  the platform's instruction, and a library can add operations of its
  own (`sqrt x`); literals wherever the type is known (`add a, 1`,
  `mul x, 0.5`, `ret 0`), with float literals rounded exactly; block
  parameters instead of phi nodes; multiple return values; an optional
  structured front-end (`if`/`loop`/`break`/`continue`/`yield`) that
  lowers to the flat block graph at parse time; and abstract `int`/`uint`
  types resolved to a concrete width by a per-target replacement policy
  (`--int=i32|i64` to override), and abstract `float`, `fixed`, `unit`,
  and `sunit` resolved the same way (`--float=f16|bf16|f32|f64|E,M`,
  `--fixed=I,F`, `--unit=N`, `--sunit=N`) to the libraries' `float(E, M)`,
  `fixed(I, F)`, `unit(N)`, `sunit(N)`.
- **Two learners**:
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
- **A platform per backend** (`src/platform.rs`): the generic
  instantiations the target implements natively (`add`/`sub`/`mul`/`div`/
  `sqrt`/`neg`/`abs`/`min`/`max`/`fma`/`cmp.*` on `float(8, 23)` and
  `float(11, 52)`, and `conv` between those and 32/64-bit integers —
  minus what a target's own semantics rule out: riscv64's NaN-dropping
  `fmin`, wasm's missing fma). Compiling such an instance, or a call to one, emits the
  hardware sequence instead of the SSA body; `--soft` turns that off, and
  the library remains the reference the hardware path is checked against.
- **Three backends**, none of which contain a single hand-written opcode:
  - `arm64` (`src/emit.rs`) — JIT: mmap/MAP_JIT on Apple Silicon, run
    in-process
  - `riscv64` (`src/emit_rv.rs`) — bare-metal on qemu-system-riscv64, with
    the runtime harness (UART printing, exit) generated in the project's
    own SSA
  - `wasm32` (`src/emit_wasm.rs`) — module emission, executed by node via
    `src/driver.js`
- **A linear-scan register allocator** (`src/regalloc.rs`), target
  independent: the emitter hands it a callee-saved pool and gets back a
  register or spill slot per value.
- **An SSA pass pipeline** (`src/opt.rs`): simplify-cfg, const-fold, dce,
  sink. Optimization levels are prefixes of that one list, so every level
  is a correct stopping point and every pass is checked by the suite on
  every backend.
- **An incremental JIT arena** (`src/arena.rs`): each function in its own
  slot with slack, calls routed through counting trampolines, so an edited
  function recompiles in place and a hot one is promoted through the full
  pipeline without disturbing its neighbours.
- **One regression suite** (`suite/*.ssa`, runner in `src/suite.rs`): 413
  cases with expectations embedded as `;! gcd 48 36 -> 12` directives, run
  identically against every backend — including arm64 under
  qemu-system-aarch64 as an independent second referee for the same bytes
  the M-series CPU runs.

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

# floating point is a library (lib/float.ssa, appended to every program):
# add/sub/mul/div/sqrt/neg/abs/min/max/fma/cmp/conv over float(E, M), done
# with integer instructions, instantiated for fp8/fp16/bf16/f32/f64 and
# checked against the FPU. On a platform with hardware for it, fadd32 *is*
# the instruction; --soft keeps the library body. A bare `float` is the
# policy's width
cargo run -- run suite/float.ssa fadd32 0x3dcccccd 0x3e4ccccd   # 0.1 + 0.2 -> 0x3e99999a
cargo run -- --soft run suite/float.ssa fadd32 0x3dcccccd 0x3e4ccccd   # same answer, ~100 instructions
cargo run -- run suite/afloat.ssa hyp 3 4                  # sqrt(3*3 + 4*4) over abstract floats -> 5
cargo run -- --float=f16 run suite/afloat.ssa hyp 3 4      # the same program, at 16 bits
cargo run -- run suite/fixed.ssa divf 7 2                   # fixed point (lib/fixed.ssa): 7 / 2 -> 3
cargo run -- run suite/unit.ssa pct 50 50                   # unit fractions (lib/unit.ssa): 50% of 50% -> 25

# the incremental compiler. Edit the file while this runs — changed
# functions recompile in place at level 0, and functions that get hot
# are automatically promoted through the full pass pipeline.
cargo run -- live examples/fib.ssa fib 25
```

`history.md` has a short entry for every commit.

Toolchain expectations (macOS/arm64 host): `llvm-mc` (brew llvm), `wabt`
(wat2wasm), `node`, `qemu`. The learned `targets/*.encodings.json` files
are checked in, so the backends and suite work without re-learning.

## Status

Integers of every width, packed bitfields, parametric types and
functions, and floating-point arithmetic, comparison, and conversion as a pure-SSA library that
the platform swaps for hardware where it has it, on three targets,
everything differentially verified: the suite on four execution paths,
every narrow-type op against the const-folder's model over every value
pair, and the softfloat ops against the FPU for f32/f64 and against an
exact reference exhaustively for fp8. What is
deliberately not here yet, from `future-work.md`: indirect
calls / function pointers, external (libc) calls from JIT'd code, a
dominance check in the verifier, and differential testing against clang
to close the semantic loop the way the prober closed the encoding loop.
