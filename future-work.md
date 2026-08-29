# Future work

## Function values

In (`fn(i64) -> i64` types, `addr f`, calls through a value; a table and `call_indirect` on wasm). Left:

- **Function values in `data`**: `data handlers: array(fn(i64), 4) = { addr a, ... }` needs address relocations inside data — absolute on bare metal, unknown until mapping in the JIT — or a table of indices the way wasm does it. Today a table is built at run time with `store`.
- **Signatures with structs or wide values**: the aggregate and wide lowerings rewrite a function's parameters into fields and words; a function *type* would have to be rewritten the same way. Rejected for now.
- **Generics over function types**: `apply(T)(f: fn(T) -> T, x: T)` needs `unify` to see through `fn(...)`.
- **Signature mismatch is target-dependent**: wasm traps on a call through a value of the wrong type; the native targets execute whatever is there. The verifier catches every static case; a value loaded from memory is the program's promise.

## The GPU

AIR is in (`src/emit_air.rs`): a `__kernel` runs on any Apple GPU from bitcode we write. Left:

- **Group addresses stay in their function**: a `group` item's address is tracked through arithmetic and block arguments inside the function that took it, and may not be stored, passed or returned — there is no pointer type for the space it is in. A `ptr` that knows its address space would lift that; ash's rule against region-typed pointers is about lifetimes, not spaces, so it is open.
- **Cores in the OS programs**: the suite's runner brings up cores for a kernel; `os/*.ssa` still run on core 0. A task scheduler across cores (each core a scheduler, tasks dealt out, the timer per core) is the next OS milestone, and a `check` that a core's stack and its fibres' stacks are apart, which cost an evening here.
- **Fibres on wasm**: one stack, so bodies run one after another and kernel directives are skipped. But wasm has *threads* (shared memory, atomics, a worker per thread): the better model there is a real OS thread per kernel thread — `group_sync` an atomic barrier (`memory.atomic.wait`/`notify` on a counter), group items in shared memory at a per-group offset, `thread()` a per-instance global — with the learner picking the atomic opcodes off wat2wasm and `driver.js` spawning workers. Stack switching (JSPI) would give fibres too, later.
- **Simdgroup operations on 64-bit values**: Apple's intrinsics stop at 32 bits; a `simd_sum` on an `i64` is an error on the GPU. Two 32-bit halves with a carry, or a threadgroup reduction, would give it.
- **Simdgroup matrices** (`simdgroup_float8x8`, `simdgroup_multiply_ accumulate`): the tensor path on Apple GPUs; a type and a few operations, worth doing when a matrix program wants them.
- **Textures and samplers**: handles with platform operations, as decided when arrays were done; not arrays.
- **Vectors whole elsewhere**: AIR takes them (`builtin vectors`) and arm64 takes the classed ones (NEON: `class v` and rules); riscv64 takes them on RVV (`ext V`) with every rule setting its own vtype — the simplest answer to a rule with machine state, one `vsetivli` per operation (a pass could elide the repeats). wasm SIMD would use the same seam — the parser keeps whole what a platform has a class and a rule for. The scalable model — `vl` as a value, loops in chunks of "as many as fit", masks for the tail — is untouched; `reference/scalable-vectors.md` is the design note; decided (2026-08-29): slices `T[]` over buffers with a header, operations whole (in: `lib/slice.ssa`, scalar loops), then `chunk(T)` with `fit` as the register-level piece the library's strip-mining loop is written with, then the rules.
- **A vector at the JIT boundary**: a caller from Rust passes integer words, and a function whose parameter or result is a vector gets no wrapper (`ssa::jit_wrappers`); one that takes the lanes as words and packs them would let `jit.call` reach such a function, if a test or a tool ever wants to.
- **The rest of NEON and RVV**: conversions between lane widths that change signedness or skip a width (`u8x8` to `i32x8` is two vectors), and shuffles and dot products as library generics with rules (the reductions are in: `lib/reduce.ssa`).
- **Denormals**: the f32 instructions flush them and Apple's compiler has no switch (`air.compile.denorms_enable` is ignored, their own front end never writes anything but `denorms_disable`); half keeps them. A program that needs f32 denormals must use the library.
- **Apple's optimizer**: with inlining left to it, a 128-bit division came out wrong (the same bitcode was right through upstream LLVM at -O0 and -O2, and right with every function `noinline`); everything is `alwaysinline` now, which is also six times faster. If a kernel ever outgrows that, the bug is waiting.
- **Recursion**: left out and reported; a program that needs it gets an explicit stack in memory, by hand for now.
- **WebGPU**: the same shape for SPIR-V (`spirv-as` as the oracle, wgpu to run), when the wasm side comes back.

## Code quality

- **More SSA passes for higher tiers**: GVN/CSE, copy propagation (folding `x + 0` style identities needs use-rewriting), loop-invariant code motion (the structured front-end knows the loops), inlining.
- **Background tiering off the main thread**: the incremental arena is in (`probe live`): per-function slots with slack, counting trampolines, in-place recompiles, automatic hot promotion. Promotion currently runs on the main loop between calls; moving it to a worker thread needs the trampoline retarget made atomic (a literal-pool `ldr/br` trampoline with a single 64-bit target store, instead of the movz/movk chain).
- **Arena hygiene**: abandoned slots after growth are never reused (a free list would fix it), and invocation counters never reset on reload.
- **Caller-saved pool for leaf functions**: values in leaf functions could use x9..x17 with no prologue saves at all.
- **Constant materialization**: use `movn`/single-`movz` forms on arm64 and `lui+addiw` fast paths on riscv64 instead of worst-case chunk sequences.

## Robustness limits to lift

- **Frame sizes**: arm64 caps at 4095 bytes (single `sub sp` immediate), riscv64 at 2047 (`addi`); large frames need multi-instruction adjustment.
- **Long branches**: riscv64 conditional branches reach ±4K; big functions need branch relaxation (invert + `jal` trampoline — partially in place).
- **>8 arguments / branch args / return values**: spill to the stack per the platform ABI.
- **Rust FFI for >2 return values**: generate a wrapper in our own SSA that flattens results through a pointer argument (self-hosting shim), instead of fighting the C ABI's indirect-return convention.

## Verification

- **Dominance checking** in the SSA verifier (currently deferred; the structured front-end makes violations hard to write, but flat form allows them).
- **Differential testing against clang**: compile the same function from C, run both on random inputs, compare — closes the semantic loop the way the prober closed the encoding loop.
- **Validate learned arm64 encodings against ARM's Machine Readable Architecture** XML (see reference/README.md) — an independent scorecard the learner never consults during learning.

## Memory

The design session happened; `reference/memory-management.md` is the briefing and its §7 the direction. Built so far: `scratch` (a call's lifetime), `data` (the machine's), `lib/arena.ssa` (a frame's: bump allocation reset all at once), `lib/pool.ssa` (an object's: fixed slots given back one at a time), `lib/heap.ssa` (the root: a buddy allocator the rungs are carved from, sealable) and `check`. The discipline, stated: objects live in the rungs, the rungs come from the heap, and nothing allocates from the heap inside an interrupt.

Decided against, deliberately: lifetimes in the pointer type (`ptr frame`, `ptr call`, checked at stores and returns) and unique pointers. The IR is a target; whether a pointer outlives its memory is the contract of the code generator upstream of it, which is expected to be smart enough never to emit that, the way it is expected never to emit a use before a definition it cannot see. The verifier checks the IR's own well-formedness, not the front end's discipline. Still possible: the capacity analysis (`probe stack`: worst-case stack per task and bytes per arena per frame, over the call graph the compiler already has, indirect calls by type); generics over types when pools want to be typed. Not a heap for objects: the rungs are for those. Also still: frames are at most 4095 bytes on arm64 and 2047 on riscv64 (a single immediate).

## Language

- External calls (libc symbols) from JIT'd code.
- Floats: `lib/float.ssa` has generic add/sub/mul/div/sqrt/neg/abs/ min/max/fma, the comparisons, and conversions over packs, with the f32/f64 instances on hardware wherever the target's semantics agree. Left: the NaN-payload question (the library canonicalizes, hardware propagates — a platform could be asked to canonicalize), remainder and rounding functions (`rem`, `floor`, `round`), and decimal formatting. f16 on arm64 (FEAT_FP16) is one table line plus the `h` templates.
- Fixed point (`lib/fixed.ssa`) truncates toward zero in mul and div; rounding variants, and a `sqrt`, would fit beside them. Rationals (`lib/rational.ssa`) use 64-bit intermediates, so N and D are for 32 bits and under; a two-word version would lift that. Neither has sqrt, so a `scalar` program that takes a root is a float-family program.
- Platforms as data: the native table lives in `src/platform.rs`; a per-target file listing native instantiations next to the probe seed would let a target be described entirely outside the compiler.
- Native calls keep the library instance in the module even when every call to it was replaced; dropping unreferenced instances is a small module-level DCE.
- Memory for odd widths: a `u5` can't be loaded or stored; a load of the containing byte plus `bitcast`/`get` covers it by hand for now.
- Field-named pack construction; `cmp.eq` on packs without a `bitcast`.
- Generic instantiation is by re-parsing the template per instance; a generic that is instantiated many times pays the parse each time.
- Narrow shifts by the width or more are unspecified (the hardware shift in the container, then re-normalized) — deliberately, to keep them one instruction.
