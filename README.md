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
suite/*.ssa  --parse/verify-->  SSA  --emitter-->  bytes  --run-->  results
                                        ^
                     targets/*.encodings.json  (learned, verified)
```

## What's here

- **An SSA IR** (`ssa.md`, `src/ssa.rs`): typed values, block parameters
  instead of phi nodes, multiple return values, an optional structured
  front-end (`if`/`loop`/`break`/`continue`/`yield`) that lowers to the
  flat block graph at parse time, floats (`f32`/`f64`, full conversion and
  bitcast set), arbitrary-width integers of both signednesses (`i5`,
  `u52`...) — signedness lives in the TYPE, so there is one `div`, one
  `shr`, one `icmp.lt` — packed bitfield structs (`type $fp = { sign: u1,
  exp: u11, frac: u52 }` — an IEEE double, by construction), short
  vectors (`i16x4`, `f32x2` — elementwise with the ordinary opcodes, no
  new ones), and abstract numeric types (`int`, `uint`, `float`, and
  `scalar` — the parent of float and rational) resolved by a per-target
  replacement policy. Widths, structs, and (off arm64) vectors lower to
  core-integer code before emission. A layer of parse-time sugar —
  expressions with C precedence, comparisons, literal operands,
  one-line guards like `if call @rat_is_nar(%x) { ret call @rat_nar() }`
  — is being evolved downhill on measured authorship cost
  (`ergonomics.md`).
- **Two learners**:
  - `src/learn.rs` for fixed-width register ISAs: one-hot probes XORed
    against a baseline map each operand bit to its encoding bit — which
    handles RISC-V's scrambled branch immediates exactly as easily as
    ARM's contiguous fields. Nonlinear small domains become lookup tables;
    nonlinear large ones (ARM logical immediates) fail honestly.
  - `src/wlearn.rs` for byte-oriented stack machines (wasm): templates are
    fixed bytes plus LEB128 codecs at discovered positions, probed through
    wat2wasm with a bootstrap chain of stack context.
- **Three backends**, none of which contain a single hand-written opcode:
  - `arm64` — JIT: mmap/MAP_JIT on Apple Silicon, run in-process, with
    class-aware register allocation, an SSA optimization pipeline whose
    levels are prefixes of one pass list, an incremental JIT arena with
    hot-function promotion, and probe-learned NEON: a vector op is one
    instruction
  - `wasm32` — module emission, executed by node
  - `riscv64` — bare-metal on qemu-system-riscv64, with the runtime
    harness (UART printing, exit) generated in the project's own SSA
- **Numerics as libraries, not compiler features**: softfloat
  (`src/softfloat.rs` — SSA over the `$fp` struct, bit-exact against the
  host FPU, so int-only CPUs get floats) and exact rational arithmetic
  (`lib/rational.ssa` — `$rat` with canonical reduced form and NaR;
  `--scalar=rat` runs abstract-scalar programs in exact arithmetic).
  The same recipe extends to fixed-point, fp8/fp4, and block formats.
- **One regression suite** (`suite/*.ssa`): 200+ cases with expectations
  embedded as `;! gcd 48 36 -> 12` directives, run identically against
  every backend and policy — including arm64 under qemu-system-aarch64
  as an independent second referee, NEON vs scalarized vector emission
  refereeing each other, and softfloat vs the FPU.

**New here? Start with [`tutorial.md`](tutorial.md)** — a guided tour of
the language and the project's verification culture, by way of the
rational-number library.

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

# the regression suite, per backend
cargo run -- test              # native arm64 JIT
cargo run -- test wasm         # node
cargo run -- test riscv        # qemu-system-riscv64
cargo run -- test arm-qemu     # qemu-system-aarch64

# the optimization pipeline: levels are prefixes of one SSA pass list
# (simplify-cfg, const-fold, dce, sink); -O<n> works on any command, and
# `tiers` compiles at every prefix to show the gradual-optimization story
cargo run -- tiers examples/tiers-demo.ssa
cargo run -- -O0 run examples/sum.ssa sum 100

# software floating point, written in this compiler's own SSA over the
# $fp bitfield struct; float ops become calls into it, so int-only CPUs
# get floats. Verified bit-for-bit against the host FPU.
cargo run -- --softfloat test

# width and representation policies: the same abstract-typed programs,
# different arithmetic - and the answers must agree
cargo run -- --int=i32 --float=f32 test
cargo run -- --scalar=rat test                       # exact rationals
cargo run -- --scalar=rat --softfloat --int=i32 --float=f32 test

# the incremental compiler: one JIT arena, per-function slots with slack,
# counting trampolines. Edit the file while this runs — changed functions
# recompile in place at level 0 (microseconds), and functions that get hot
# are automatically promoted through the full pass pipeline.
cargo run -- live examples/fib.ssa fib 25
```

Toolchain expectations (macOS/arm64 host): `llvm-mc` (brew llvm), `wabt`
(wat2wasm), `node`, `qemu`. The learned `targets/*.encodings.json` files
are checked in, so the backends and suite work without re-learning.

See `future-work.md` for where this is headed (128-bit vectors, the
fp8/fp4 float menagerie, fixed-point via `--scalar=fx8.8`, indirect
calls) and `ergonomics.md` for the language-evolution methodology.
