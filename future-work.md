# Future work

## Function pointers / indirect calls

Add first-class function values to the SSA:

```
f: ptr = funcaddr sq
r: i64 = calli f(x)        ; signature checked against uses, or annotated
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

## Language

- External calls (libc symbols) from JIT'd code.
- An `alloca`-style op for function-local scratch memory (today all memory
  comes from the caller).
- Floats: `lib/float.ssa` has generic add/sub/mul/div/sqrt/neg/abs/
  min/max/fma, the comparisons, and conversions over packs, with the
  f32/f64 instances on hardware wherever the target's semantics agree.
  Left: the NaN-payload question (the library canonicalizes, hardware
  propagates — a platform could be asked to canonicalize), remainder and
  rounding functions (`rem`, `floor`, `round`), and decimal formatting. f16 on arm64 (FEAT_FP16) is one table line plus the `h` templates.
- Fixed point (`lib/fixed.ssa`) truncates toward zero in mul and div;
  rounding variants, and a `sqrt`, would fit beside them. Rationals
  (`lib/rational.ssa`) use 64-bit intermediates, so N and D are for 32
  bits and under; a two-word version would lift that. Neither has sqrt,
  so a `scalar` program that takes a root is a float-family program.
- Platforms as data: the native table lives in `src/platform.rs`; a
  per-target file listing native instantiations next to the probe seed
  would let a target be described entirely outside the compiler.
- Native calls keep the library instance in the module even when every
  call to it was replaced; dropping unreferenced instances is a small
  module-level DCE.
- Memory for odd widths: a `u5` can't be loaded or stored; a load of the
  containing byte plus `bitcast`/`get` covers it by hand for now.
- Field-named pack construction; `cmp.eq` on packs without a `bitcast`.
- Generic instantiation is by re-parsing the template per instance; a
  generic that is instantiated many times pays the parse each time.
- Narrow shifts by the width or more are unspecified (the hardware shift
  in the container, then re-normalized) — deliberately, to keep them one
  instruction.
