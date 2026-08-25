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
  lives in the type, so there is one `div`, one `shr`, one `icmp.lt`;
  `pack` types that lay bitfields out lowest-bits-first in up to 64 bits
  (`pack $rgb { r: u5, g: u6, b: u5 }`, nestable, storable); block
  parameters instead of phi nodes; multiple return values; an optional
  structured front-end (`if`/`loop`/`break`/`continue`/`yield`) that
  lowers to the flat block graph at parse time; and abstract `int`/`uint`
  types resolved to a concrete width by a per-target replacement policy
  (`--int=i32|i64` to override). No floats yet; their bit layouts are
  already expressible as packs.
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
- **One regression suite** (`suite/*.ssa`, runner in `src/suite.rs`): 162
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

# narrow types and packs
cargo run -- run suite/bits.ssa add5 15 1          # i5: 15 + 1 -> -16
cargo run -- run suite/packs.ssa mkrgb 31 63 1     # -> 4095 (b:g:r = 1:63:31)

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

Integers of every width and packed bitfields, three targets, everything
differentially verified (the arm64 backend additionally checks every
narrow-type op against the const-folder's model over every value pair). What is
deliberately not here yet, from `future-work.md`: floats (the types are
reserved; `float` will join `int` in the replacement policy), indirect
calls / function pointers, external (libc) calls from JIT'd code, a
dominance check in the verifier, and differential testing against clang
to close the semantic loop the way the prober closed the encoding loop.
