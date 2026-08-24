# Future work

## Function pointers / indirect calls

Add first-class function values to the SSA:

```
%f: ptr = funcaddr @sq
%r: i64 = calli %f(%x)        ; signature checked against uses, or annotated
```

Backend sketch:

- **arm64**: nearly free. `funcaddr` materializes the function's code address
  (pc-relative `adr`, or `movz/movk` of a known offset at JIT layout time);
  `calli` is `blr {x}` — already learned and verified.
- **riscv64**: same shape — `auipc`/`addi` for the address, `jalr` (already
  learned) for the call.
- **wasm**: functions are not addressable; a "function pointer" is an index
  into a module-level **table**. Emit a table + element section listing every
  address-taken function; `funcaddr` becomes that index; `calli` becomes
  `call_indirect` (one new template to probe: opcode + type-index uleb +
  table byte). Note the semantic difference: wasm type-checks the callee
  signature at call time and traps on mismatch, where native targets would
  execute garbage — the suite needs to treat mis-typed-callee tests as
  target-dependent. The function-references proposal (`ref.func`/`call_ref`)
  is a cleaner future mapping once universally shipped.

## Code quality

- **Fall-through branch elimination**: simplify-cfg now threads branches
  through empty forwarding blocks at SSA level, but emitters still emit a
  `b`/`jal` to a block that is laid out immediately after it. Removing
  those needs layout-aware sizing (dropping an instruction shifts every
  later offset), i.e. a two-pass or relaxation-style emitter.
- **More SSA passes for higher tiers**: GVN/CSE, copy propagation (folding
  `x + 0` style identities needs use-rewriting), loop-invariant code
  motion (the structured front-end knows the loops), inlining.
- **Background tiering off the main thread**: the incremental arena is in
  (`probe live`): per-function slots with slack, counting trampolines,
  in-place recompiles, automatic hot promotion. Promotion currently runs
  on the main loop between calls; moving it to a worker thread needs the
  trampoline retarget made atomic (a literal-pool `ldr/br` trampoline with
  a single 64-bit target store, instead of the movz/movk chain).
- **Arena hygiene**: abandoned slots after growth are never reused (a free
  list would fix it), and invocation counters never reset on reload.
- **Caller-saved pool for leaf functions**: values in leaf functions could
  use x9..x17 with no prologue saves at all.
- **Constant materialization**: use `movn`/single-`movz` forms on arm64 and
  `lui+addiw` fast paths on riscv64 instead of worst-case chunk sequences.

## Robustness limits to lift

- **Frame sizes**: arm64 caps at 4095 bytes (single `sub sp` immediate),
  riscv64 at 2047 (`addi`); large frames need multi-instruction adjustment.
- **Long branches**: riscv64 conditional branches reach ±4K; big functions
  need branch relaxation (invert + `jal` trampoline — partially in place).
- **>8 arguments / branch args / return values**: spill to the stack per the
  platform ABI.
- **Rust FFI for >2 return values**: generate a wrapper in our own SSA that
  flattens results through a pointer argument (self-hosting shim), instead
  of fighting the C ABI's indirect-return convention.

## Verification

- **Dominance checking** in the SSA verifier (currently deferred; the
  structured front-end makes violations hard to write, but flat form allows
  them).
- **Differential testing against clang**: compile the same function from C,
  run both on random inputs, compare — closes the semantic loop the way the
  prober closed the encoding loop.
- **Validate learned arm64 encodings against ARM's Machine Readable
  Architecture** XML (see reference/README.md) — an independent scorecard
  the learner never consults during learning.

## The platform model

The direction (user-proposed): a *platform* is an object exposing
concretely-typed functions whose bodies are a single `emit`
meta-instruction populated from the learned encodings JSON —
`add(f32, f32) -> f32 { emit "fadd {s}, {s}, {s}" }`. Instruction
selection becomes overload resolution: a platform function shadows the
DIY generic library (fp_add(E, M), rationals, softfloat); anything the
platform doesn't export falls through to the library and compiles as
integer code. Inlining an emit-bodied function IS instruction emission,
so emit.rs's hand-written dispatch tables become data, --softfloat
becomes "a platform that exports no float functions", and porting a
target approaches "learn the JSON, write the platform file".

Landed toward it: the fallthrough half — fp_add/fp_sub/fp_mul(E, M) as
direct bitwise generics (full subnormals, guard/round/sticky RNE),
called directly by small-float lowering with no f64 dependency, and
proven equivalent to the native path at (8, 23) against the M1 FPU.
Next steps: fp_div direct; route --softfloat's f32/f64 add/sub through
fp_add(8,23)/(11,52) and retire the duplicated runtime (f64 mul/div
need $wide products); then the emit meta-instruction itself.

## Vectors and SIMD

- **NEON emission is live**: arm64 keeps vectors whole in d registers
  and emits the learned encodings; per-function fallback scalarizes
  bodies (never signatures) for ops NEON lacks (integer div/rem, odd
  lane widths), so both tiers interoperate across calls. wasm/riscv
  still scalarize — RVV and wasm-simd follow the same probe->emit
  recipe when wanted. Remaining NEON polish: fold the mod-N shift mask
  when amounts are constant, dup-based splat detection (pack of one
  repeated value), and lane-pair moves for f32 extracts (currently
  umov+fmov).
- **128-bit vectors** (f32x4, i16x8 — SSE/NEON width): needs multi-
  register SSA values (register pairs in regalloc, call-ABI rules,
  two-word spills). The type syntax already generalizes; only the
  64-bit total cap comes off.
- **Vector comparisons and selects** (mask vectors, u1xN), `splat` and
  `reduce.*` sugar, vector load/store — arriving with the SIMD tier,
  where they map to real instructions.
- **Small-float lanes**: fp8/fp4 elements (see the numeric tower) make
  `vec(16, fp4e2m1)` expressible, which with a scale field gives the
  nvfp4/MX block formats as ordinary structs + libraries.

## Width generics (landed) — next steps

- **`W` in width expressions**: bind the policy's word width so
  `type $rat = $rat(W/2)` writes the policy-sized rational once and
  `half`/`uhalf` become sugar (`i(W/2)`); needs parse to know the
  policy (or a post-parse instantiation stage).
- **Parametric type aliases** (`type $r16 = $rat(16)`), inference from
  return-type context, and literal args to generic calls (defer literal
  typing until parameters solve).
- **Menagerie follow-ons**: the finite-only e4m3 "FN" variant (no inf,
  one NaN); float(2, 1) fp4 lanes in vectors + the nvfp4 block format;
  collapsing softfloat's duplicated f64/f32 runtime into the generic
  library; E > 8 non-native formats (need f64-subnormal handling in
  promote).
- **$rat(N) library**: the rational library generic over N, subsuming
  the half/uhalf version once W lands.

## Numeric tower

- **Rationals, generalized**: lib/rational.ssa is the recipe — `$rat` =
  `{ num: i32, den: u32 }` plus an SSA op library (canonical reduced
  form, NaR = den 0, exact-or-NaR semantics), zero compiler changes —
  and `--scalar=rat` plugs it under the abstract `scalar` type. Next
  rungs: parametrized `rat(N)` widths (r16 = i16/u16 fits 32 bits), a
  mediant / continued-fraction `float -> rat` best-approximation, and
  detecting the one 64-bit intermediate overflow corner in add.
- **More scalar implementations**: `scalar` abstracts the numeric
  representation itself; fixed-point (below) joins as `--scalar=fx8.8`
  the day its library exists, and so could intervals, posits, or a
  decimal type — each is a struct layout, an op library, and one arm in
  the scalarize mapping.
- **$wide (i128) as a library**: multi-word structs make it a struct +
  SSA functions (suite/wide.ssa's add-with-carry is the seed); closes
  full-word-field rationals ({ num: int, den: uint }) and fixed-point
  at every width.
- **Struct load/store**: per-word memory ops at computed offsets — the
  C-interop payoff (arrays of structs, records in buffers, MMIO
  blocks). Layout is already C-like.
- **Structure-of-vectors (AoS -> SoA)**: vec-of-struct types resolving
  to struct-of-vectors under a layout policy, for the well-understood
  cache/SIMD wins. The multi-word value-splitting machinery is the
  intended substrate: same decomposition, vectors as the parts; field
  access is symbolic until lowering precisely so this stays possible.
- **Fixed-point types**: `fx8.8` as sugar for a struct-backed numeric —
  `{ int: i8, frac: u8 }` — with its operation set written as an SSA
  library, exactly the softfloat recipe: struct layout states the format,
  a lowering pass rewrites `fadd`-style ops on fixed values into calls,
  and differential tests pin the semantics. Generalizes to `fxM.N` and to
  unsigned `ufxM.N`; an abstract `fixed` type joins the replacement
  policy alongside `int`/`uint`/`float`.
- Saturating/checked arithmetic variants as type- or op-level opt-ins.

## Ergonomics (token-cost descent)

The language is being evolved downhill on measured authorship cost (see
ergonomics.md). Landed: literal operands and expression right-hand
sides (-26% atoms on expression-heavy code, -6% on call/control-heavy).
Next candidate mutations, by observed remaining ceremony:

- **Unary minus on values** (currently `0 - %x`); calls nested in call
  arguments (`call @f(call @g(%x))`).
- The live half of the methodology: a task battery (write-new,
  edit-existing, find-the-bug) measured in real model tokens-to-green,
  across models, per ergonomics.md.

## Language

- External calls (libc symbols) from JIT'd code.
- An `alloca`-style op for function-local scratch memory (today all memory
  comes from the caller).
- Float folding in const-fold (needs rounding-mode care), and fused
  multiply-add selection (`fmadd`/`fma` exist on every target).
- Softfloat tier 2: subnormals (currently flush-to-zero, checked by the
  differential tests' expectations), NaN payload propagation, and an
  int-only target profile that turns `--softfloat` on by policy.
- NaN-semantics suite cases end to end (the softfloat runtime and the
  differential tests cover NaN, the .ssa suite doesn't yet).
