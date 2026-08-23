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
  flat block graph at parse time, and abstract numeric types (`int`)
  resolved to concrete widths by a per-target replacement policy
  (`--int=i32|i64` to override).
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
  - `arm64` — JIT: mmap/MAP_JIT on Apple Silicon, run in-process
  - `wasm32` — module emission, executed by node
  - `riscv64` — bare-metal on qemu-system-riscv64, with the runtime
    harness (UART printing, exit) generated in the project's own SSA
- **One regression suite** (`suite/*.ssa`): 86 cases with expectations
  embedded as `;! gcd 48 36 -> 12` directives, run identically against
  every backend — including arm64 under qemu-system-aarch64 as an
  independent second referee for the same bytes the M-series CPU runs.

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

# the incremental compiler: one JIT arena, per-function slots with slack,
# counting trampolines. Edit the file while this runs — changed functions
# recompile in place at level 0 (microseconds), and functions that get hot
# are automatically promoted through the full pass pipeline.
cargo run -- live examples/fib.ssa fib 25
```

Toolchain expectations (macOS/arm64 host): `llvm-mc` (brew llvm), `wabt`
(wat2wasm), `node`, `qemu`. The learned `targets/*.encodings.json` files
are checked in, so the backends and suite work without re-learning.

See `future-work.md` for where this is headed (indirect calls, register
allocation, floats, differential testing against clang).
