# History

What landed, one short entry per commit — or per group, when several
arrived together as one piece of work. Newest first. `git show <hash>`
has the full story for any of them.

---

### min, max, fma — `da83428` · 2026-08-26

`min`/`max` (IEEE minimum/maximum: NaN propagates, -0 below +0) and
`fma` with a single rounding: the exact product and the addend meet in
a two-word accumulator with eight guard bits, built from a few
`add128`/`shr128`-style helpers written in the SSA itself. On the
platforms: `fmin`/`fmax`/`fmadd` (arm64), `fmadd` (riscv64, whose
`fmin` drops NaNs and so stays in the library), `f32.min`/`max` (wasm,
which has no fma). Bit-exact against `mul_add` and an exact reference.
327/327 on all four paths, both ways.

### The suite, with the sugar — `c1d472d` · 2026-08-26

The suite and the float library rewritten with literal operands: 139
named constants gone, 374 lines shorter, `cmp.ne ma, 0` and `pack 0, 0,
sz` where there were `zero_m` and `zero_e` declarations. A mechanical
pass, so the IR underneath is byte-for-byte the same and the matrix is
unchanged: 302/302 on all four paths, both ways.

### const by type, literals as operands — `9b05c3e` · 2026-08-26

`iconst` is `const`, and the type decides: bits for an integer or a
pack, a number for a float — `x: f32 = const 0.1` is the nearest f32,
exactly rounded (decimal to binary by a small bignum, checked against
Rust's `from_str`); `-inf` and `nan` too. And a literal can stand in
for a value wherever the context fixes its type: `add a, 1`,
`cmp.lt 0, b`, `mul x, 0.5`, `jmp loop(0, 0)`, `ret 0`, `g(b, 2)`, with
`200: u8` when nothing does. Hidden consts carry them; the printer
shows them inline again. 302/302 on all four paths.

### cmp on floats, neg, abs — `aa1829b` · 2026-08-25

`icmp` is `cmp`, and on floats `cmp.lt` is the library's `lt(E, M)`:
six predicates over one `fcmp` that orders by sign and magnitude bits,
with IEEE's rules (-0 equals +0; a NaN makes everything false but `ne`).
`neg` and `abs` touch the sign field. The platforms have `fcmp`+`cset`,
`feq`/`flt`/`fle`, `f32.lt`, `fneg`, `fabs`. Two 63-bit overflow bugs
surfaced and were fixed. 302/302 on all four paths, both ways.

### conv and cast — `0f2850f` (and the syntax before it)  · 2026-08-25

Two opcodes for what used to be three: `conv` carries the value across
(ext and trunc are gone — the widths always said which way), `cast`
keeps the bits (was bitcast). `1.0 conv u32` is 1; `cast` is
0x3f800000. Between a float and anything, `conv` is the library's:
float(E, M) to float(F, N), i(W)/u(W) to float(E, M), float(E, M) to
i(W)/u(W) (truncating, saturating, NaN to 0) — five generics sharing
one name, chosen by the types on both sides now that dispatch matches
the result type too and generics may overload. The platforms map every
f32/f64/i32/u32/i64/u64 pair to `fcvt`/`scvtf`/`fcvtzs` and friends
(riscv64 keeps float to int in the library over its NaN rule). Checked
against Rust's `as` and the exact reference; 274/274 on all four paths,
both ways.

### sqrt, and operations a library invents — `b795912` · 2026-08-25

`r: f32 = sqrt a`. Dispatch is now open-ended: any name applied to a pack
finds the generic of that name that takes the pack's origin type, at
whatever arity it declares, so `sqrt` exists for floats without the
integer language knowing the word. The library's `sqrt(E, M)` is a
digit-by-digit root that never needs more than M + 8 bits; the
platforms map f32/f64 to `fsqrt`. Checked against the FPU and an exact
reference (fp8 exhaustively). 235/235 on all four paths, both ways.

### `call` retires — `3cc7b8e` · 2026-08-25

`r: f32 = fadd32(a, b)`, `touch(q)`, `q: i64, r: i64 = divmod(a, b)`. A
name followed by `(` in operation position is a call — no opcode is ever
followed by one, `const (expr)` and `loop(...)` aside — so the keyword
said nothing. It is now rejected with a note that it is implied. Explicit
instantiations read the same way: `add(8, 23)(x, y)`. 218/218 on all
four paths, both ways.

### Float sub, mul, div — `da2c139` · 2026-08-25

The library grows `sub` (add of the negation), `mul`, and `div` over
`float(E, M)`, sharing `fnorm` and `fpack` (subnormals, round to nearest
even, overflow). `mul` builds f64's 106-bit product from 27-bit halves
without ever holding it; `div` is a restoring long division. The three
platforms gain `fsub`/`fmul`/`fdiv` for f32 and f64, so on those widths
the opcodes are instructions and on fp8/fp16/bf16 they are the library.
Bit-exact against the FPU on f32/f64 and an exact reference exhaustively
on fp8, all four ops. 218/218 on all four paths, both ways.

### Native f32, emulated f16, one module — `a71f9a7` · 2026-08-25

A test that shows the platform choosing per width on arm64: `add` on two
`f32` values compiles to `fmov`, `fmov`, `fadd s`, `fmov` with no call,
while `add` on two `f16` values in the same module compiles to a `bl`
into the library's `fadd16` (1632 bytes of integer code) with no `fadd`.
The test inspects the machine code of both functions for the learned
encodings and checks results against the FPU and the f16 reference.
Moving f16 to hardware would be one line in `src/platform.rs`.

### One `add` — `e01e057` · 2026-08-25

`iadd`/`isub`/`imul` are `add`/`sub`/`mul`, and the opcode says nothing
about the type: on integers it is the instruction, on a pack that came
from a generic type it dispatches to the generic function of the same
name taking that type. `add x, y` on two `f32` values is
`add(8, 23)(x, y)` — the softfloat library — and on a platform with
hardware for that width, the `fadd` instruction. The opcode set never
grows; libraries add meanings, platforms add instructions. 191/191 on
all four paths, both ways.

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
template; `fn fadd32 = fadd(8, 23)` and `fadd(5, 10)(x, y)`
instantiate it, by re-parsing the body with E and M bound, so `u(M + 5)`
and `const (1 << E) - 1` are concrete inside. With that, `suite/float.ssa`
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
`done: u1 = cmp.ge i, n`, `br done, exit, body`. Old prefixes are
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
the type: one `div`, one `rem`, one `shr`, one `cmp.lt`, `ext` fills by
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
