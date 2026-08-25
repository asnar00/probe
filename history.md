# History

What landed, one short entry per commit — or per group, when several
arrived together as one piece of work. Newest first. `git show <hash>`
has the full story for any of them.

---

### Platforms — `74b903d` · 2026-08-25

A platform is the list of library instantiations a target has hardware
for — `fadd(8, 23)` and `fadd(11, 52)` on all three. Each instantiated
function now knows its (generic, args) identity, and a backend compiling
one of these, or a call to one, emits the instruction sequence instead
of the SSA body: `fadd32` on arm64 is `fmov`, `fmov`, `fadd`, `fmov`,
`ret`. The library body stays the definition of the semantics; `--soft`
compiles with an empty platform, and the two are checked against each
other and the FPU. Newly probed: FP registers and adds on every target,
plus the CSR/system-register writes that switch the FPU on bare metal.
188/188 on all four paths, both ways.

### Generic functions, and floats as a library — `039de73` · 2026-08-25

`fn fadd(E, M)(a: float(E, M), b: float(E, M)) -> float(E, M)` is a
template; `fn fadd32 = fadd(8, 23)` and `call fadd(5, 10)(x, y)`
instantiate it, by re-parsing the body with E and M bound, so `u(M + 5)`
and `iconst (1 << E) - 1` are concrete inside. With that, `suite/float.ssa`
writes IEEE addition once — round-to-nearest-even, subnormals, signed
zeros, infinities, canonical NaN — using only integer instructions, and
instantiates it for fp8, fp16, bf16, f32, f64. The compiler learned
nothing about floats. It matches the FPU bit-for-bit on f32/f64 over
~140k pairs and an independent reference exhaustively on fp8; 188/188 on
all four paths.

### Parametric types — `7b9175a` · 2026-08-25

`type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }`,
then `type f32 = float(8, 23)` and `type f16 = float(5, 10)`: a `type`
declaration takes integer parameters that stand for widths, and its
body is a pack, an `i(expr)`/`u(expr)` with `+ - *` over the parameters,
a builtin, or another declared type applied to arguments. Instantiation
happens at use (`x: float(8, 23)`) or at an alias; packs are interned
structurally so every spelling of a layout is one type. Functions stay
monomorphic. `suite/types.ssa` pulls pi's exponent out of an f32 and
doubles it by incrementing the field; 174/174 on all four paths.

### The sigils retire — `2467869` · 2026-08-25

`%v`, `^b`, `@f`, `$t` become `v`, `b`, `f`, `t`. Position already said
which was which — before `:` a value is defined, after `:` a type is
named, after `call` a function, after `jmp`/`br` a block, and a label is
a name opening a line and followed by `:` — so the lexer now has one word
token and the parser's prescans apply that rule. `fn sum(n: i64)`,
`done: u1 = icmp.ge i, n`, `br done, exit, body`. Old prefixes are
rejected with a message. Suite, examples, tests, harness, and docs
converted; 162/162 on all four paths.

### Narrow shifts just shift — `a987671` · 2026-08-25

Shifting an `i5` by 5 or more no longer takes the amount mod 5 (which
cost a `ubfm`, or a `udiv`/`msub` for non-power-of-two widths): the
backends emit the container's shift and re-normalize, and amounts at or
past the width are unspecified — buyer beware, like any overflow. The
const-folder leaves those shifts alone and the exhaustive arm64 test
skips them. `i32`/`i64` keep the hardware's mod-32/64.

### Any-width integers and packs — `542b9a5` · 2026-08-25

Types are now `iN`/`uN` for any N from 1 to 64, and signedness lives in
the type: one `div`, one `rem`, one `shr`, one `icmp.lt`, `ext` fills by
the source's signedness, `bitcast` reinterprets. `u1` is the boolean.
`pack rgb { r: u5, g: u6, b: u5 }` packs bitfields lowest-bits-first
into ≤64 bits — nestable, storable at 8/16/32/64 bits — with `pack`,
`unpack`, `get`, `set`. Every backend keeps values *canonical* in their
container (sign- or zero-extended) and re-normalizes after ops that can
carry out, using freshly probed `sbfm`/`ubfm`/`bfm`, byte/halfword
loads, and wasm's narrow loads. 162/162 on all four paths; an exhaustive
JIT-vs-model test covers every op on eighteen widths.

### Abstract `int` — `1f795ad` + `acd4764` · 2026-08-23

SSA can now say `int` instead of committing to `i32` or `i64`. A
resolution pass swaps it for a concrete width before verification,
using a *replacement policy* per target (i64 on arm64/riscv64, i32 on
wasm32) or `--int=i32|i64`. Because types sit on variables, not opcodes,
that pass is one sweep over the value tables — no instruction changes.
The verifier rejects any `int` that survives, so nothing downstream ever
meets one. `suite/abstract.ssa` is written to be policy-independent and
the suite runs under both widths. 96/96 everywhere.

### Incremental JIT arena — `31bd115` · 2026-08-23

All compiled functions live in one `MAP_JIT` arena, each in a slot with
50% slack. Every call goes through a fixed per-function trampoline that
counts invocations and branches to the current address, so a changed
definition recompiles in place (or relocates to the tail if it grew)
and no call site is ever patched. `probe live <file> <fn> [args]` runs
the loop: edits recompile only the changed function at level 0, and a
function crossing 10k calls is promoted through the full pass pipeline
mid-run. `src/arena.rs`.

### SSA pass pipeline with levels — `136d065` · 2026-08-23

`src/opt.rs` becomes the single optimization engine: an ordered list of
SSA→SSA passes where a level is a prefix, so every stopping point is
valid — the foundation for gradual optimization. New passes:
simplify-cfg (threads branches through empty forwarding blocks, drops
unreachable ones), const-fold (typed wrapping arithmetic; leaves
divide-by-constant-zero alone since wasm traps), and dce (pure unused
instructions; divisions only with provably nonzero divisors). `-O<n>` on
any command; `probe tiers` shows size and time at every level; the suite
runs at every level as a test.

### Register allocation, in three steps — `d863c3d` → `6a90a80` · 2026-08-23

*Linear scan* (`d863c3d`, `src/regalloc.rs`): liveness by backward
fixpoint, single-span intervals, furthest-end eviction, over a
callee-saved pool only — so values survive calls by construction and
prologues save exactly what a function uses. 3.8× on a hot sum loop.
*Sink scheduling and parallel moves* (`eaf6780`): producers move toward
consumers before allocation, shrinking intervals; branch arguments
become true parallel moves (cycles break through one scratch register),
lifting the 8-argument cap; arm64 saves pair into `stp`/`ldp`.
*Coalescing* (`6a90a80`): precise per-point interference lets block
parameters union-find with their branch-argument sources, so the move on
a loop back edge disappears — sum's loop body is `cmp`/`cset`/`cbz` plus
two in-place adds, 96 bytes down from 156.

### Foundation — `cd7ff7a` · 2026-08-23

Everything at once: the SSA IR (block parameters, multi-value returns,
structured `if`/`loop` sugar lowered at parse time); two encoding
learners — bit-scatter for fixed-width ISAs, byte+LEB128 for wasm — that
probe `llvm-mc`/`wat2wasm` and verify every hypothesis against the
oracle; and three emitters (arm64, riscv64, wasm32) containing no
hand-written opcodes. One 86-case suite runs against all of them: native
JIT, node, and bare-metal qemu for riscv64 and aarch64, with the runtime
harness generated in the project's own SSA.
