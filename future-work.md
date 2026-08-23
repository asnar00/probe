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

- **Real register allocation** for the native backends: the every-value-in-a-
  stack-slot strategy emits roughly 4x the necessary instructions. Even a
  simple linear scan over block-local live ranges would transform output.
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
- Floats (`f32`/`f64`) — types are reserved in the spec; every target has
  probeable instruction groups for them.
