# Vectors, and the machines beyond them

A survey written on 2026-08-26, before any vector work on `main`: what the three targets have, what a GPU is from probe's point of view, what the newer AI machines and number formats look like, and where probe's existing pieces already reach. The parts about our own toolchain were checked by running `llvm-mc` and `wat2wasm` on this machine; the rest is reading.

## What the learner can already assemble

The learner never reads a manual: it needs an assembler that prints encodings. That turns out to reach further than the three targets.

| ISA | assembler | assembles | what it means |
|---|---|---|---|
| arm64 NEON | `llvm-mc -mattr=+neon,+bf16,+dotprod,+fp16fml` | `fadd v0.4s`, `fmla`, `fcvtn`, `bfdot`, `sdot`, `ld1 {v0.4s}` | 128-bit fixed vectors, learnable today |
| arm64 SVE / SME | `+sve`, `+sme` | `fadd z0.s, z1.s, z2.s`, `smstart`, `fmopa za0.s, ...` | scalable vectors and the matrix tile |
| arm64 FP8 | `+fp8,+fp8fma` | `fscale`, fp8 conversions (5 of 6 forms tried) | the 2024 formats are in the ISA |
| riscv64 RVV 1.0 | `+v,+zvfh,+zvfbfmin` | `vsetvli`, `vfadd.vv`, `vle32.v`, `vfwmul.vv`, `vfredosum.vs`, `vfncvt`, `vmfeq`, bf16 conversions | scalable, masked, widening |
| wasm32 SIMD | `wat2wasm --enable-all` (wabt 1.0.41) | `f32x4.add`, `f32x4.relaxed_madd`, `i32x4.dot_i16x8_s`, `i8x16.swizzle`, `all_true`, splat, extract | 128-bit fixed; `f16x8` and the bf16 dot are proposals wabt does not have yet |
| **AMD GPU** (gfx90a MI200, gfx942 MI300, gfx1200 RDNA4) | `llvm-mc -triple=amdgcn -mcpu=...` | `v_add_f32` (32-bit VOP2), `v_fma_f32` (64-bit VOP3), `v_pk_fma_f16`, `v_dot2_f32_f16`, `v_dot4_i32_i8`, **`v_mfma_f32_16x16x16f16`**, `v_mfma_..._fp8_fp8`, `v_wmma_...`, `v_cvt_pk_fp8_f32`, `ds_read_b32`, `s_waitcnt`, `v_cndmask`, `v_readlane`, `ds_swizzle` | a real GPU ISA, matrix cores and fp8 included, with encodings the learner could take from the same seed files |
| NVIDIA | — | PTX is text, not encodings; `ptxas` (the SASS assembler) is not open | probe could *emit* PTX but could not learn it |
| SPIR-V / Metal | — | bytecodes with vector types (≤ 4 lanes) and subgroup ops; no assembler installed here | a target of the "emit a portable IR" kind, like wasm |

The AMD row is the surprise: nothing in probe's learner is specific to CPUs, and the encodings of a compute GPU are fixed-width words with bit fields, exactly the shape it already learns (`v_add_f32_e32` is one 32-bit word; VOP3 forms are two). What we cannot do on this machine is *run* the result — there is no AMD GPU and no simulator to hand — so a GPU backend here would be learned and scorecarded but not referee'd, which the project would have to say plainly.

## Five execution models, in order of distance from probe's SSA

1. **Fixed-width SIMD** (NEON, wasm SIMD, SSE/AVX). A 128-bit register holds 4 f32 or 8 f16 or 16 i8; operations are elementwise, with a few horizontal ones (reductions, shuffles, `all_true`) and lane extract/insert. Types are the register shape: `f32x4`. This is a register class and a set of rules; the IR needs a vector type and lane access, nothing else.

2. **Scalable vectors** (SVE, RVV). The register length is unknown at compile time (VLEN); programs loop in chunks of "as many as fit", asking the machine (`vsetvli`) and using predicates/masks for the tail. RVV's twist is that the element width and grouping (`vtype`) live in a control register set by `vsetvli`, not in the instruction: an operation is really the pair (state, instruction). For probe's rules that is either a two-line composite rule per operation or a platform notion of "current vtype" — a first example of a rule that needs machine state.

3. **Matrix units** (Arm SME's `ZA` tiles and `fmopa`, AMD MFMA/WMMA, NVIDIA tensor cores, Intel AMX, and the TPU's systolic array). The unit computes `C += A × B` on a tile — 16×16×16 f16 into f32, say — as one instruction, with accumulator registers of their own (AMD's `a[0:3]`, SME's `za0.s`). This is the model that fits probe best, because it is precisely "a library defines the meaning, the platform substitutes an instruction": the meaning of a tile multiply is a three-deep loop nest anyone can read, and the rule is one line.

4. **SIMT** (GPU shaders and kernels: CUDA, HIP, Metal, Vulkan). A *scalar* program runs on every lane of a wave (32 or 64); vector width is implicit; `if` becomes an execution mask with reconvergence at the merge point; cross-lane operations (`readlane`, shuffles, ballots, subgroup reductions) are explicit instructions; memory has address spaces (registers, LDS/shared, global) with explicit waits (`s_waitcnt`). Two of probe's recent pieces point straight at this: the SSA is already a per-lane scalar program, and `src/structure.rs` — the dominator-tree nesting written for wasm — is exactly the reconvergence analysis a SIMT backend needs (a merge node is where the mask is restored). What is missing is a machine model in the platform file: wave size, divergence as mask arithmetic, address spaces as pointer classes, and waits as ordering rules.

5. **Spatial and dataflow machines** (Groq's deterministic VLIW TSP, Cerebras's PE mesh, Tenstorrent's RISC-V-plus-matrix Tensix cores on a NoC, Esperanto's thousand RISC-V cores). The compiler schedules time and place explicitly; there is no "instruction encoding" problem in probe's sense, but a scheduling one. Out of scope — except that several of them are built from RISC-V cores with vector units, which makes RVV learning directly relevant to the exotic end too.

## The format explosion, and how much of it is already here

| format | shape | probe today |
|---|---|---|
| bf16, f16, tf32 (19 bits) | `float(8, 7)`, `float(5, 10)`, `float(8, 10)` | `float(E, M)`, verified against TestFloat |
| fp8 E5M2 | `float(5, 2)` | works as is |
| fp8 E4M3 (fn) | 4/3 bits but **no infinity; NaN is mantissa all-ones**; the exponent's top code is finite | needs a variant of the library's special-value rules — a parameter (`float(E, M, fn)`) or a sibling type; exhaustively verifiable (256 values) |
| fp6 E2M3 / E3M2, fp4 E2M1 | `float(2, 3)`, `float(3, 2)`, `float(2, 1)` | `float(E, M)` instantiates them; E2M1 has 16 values |
| MX block formats (OCP MX v1.0): 32 elements of fp8/fp6/fp4/int8 sharing one E8M0 scale (a power of two) | a `pack`/`struct` of 32 elements plus an 8-bit scale; the fundamental operation is the block dot product | a `/format` library: `type mxfp4 = pack { scale: u8, e0: float(2, 1), ..., e31: float(2, 1) }` (136 bits — a wide pack), `dot` as a generic |
| NVFP4 (Blackwell) | blocks of 16 fp4 with an **E4M3** scale plus one fp32 per-tensor scale | the same shape with different parameters; a tensor-level scale lives outside the type |
| posits, int8 with per-channel scales, log formats | assorted | all libraries; the recipe covers them |

Every one of these is data plus a library in probe's terms; none is a compiler feature. The interesting new thing the block formats bring is that their *arithmetic* is vector-shaped from the start — a block is a vector, its dot product is the operation hardware provides (`v_dot4`, `sdot`, `i32x4.dot_i16x8_s`, MFMA on fp8) — so vectors and formats arrive together.

## What this suggests doing, in order

(Step 1 landed on 2026-08-27 as `TxN` — `f32x4`, `floatx4`, `intxN` — lane-by-lane by definition, on every backend; see ssa.md *Vectors*. Its second half, the register class and the rules, landed on 2026-08-28 as NEON on arm64: `class v` and a rule per operation in `targets/arm64.platform`, checked against the lane form by `targets/arm64-noneon.platform`. Step 3's first half followed the same day: the fixed-width types on RVV, every rule setting its own vtype — the scalable model, with `vl` a value, is still open.)

1. **Fixed-width vectors as a type and a class.** `type f32x4 = vector(f32, 4)`: a struct-like type of homogeneous lanes that a platform's `class v = f32x4, i32x4, ...` puts in a vector register; lanes reachable by `get`/`set`; elementwise operations defined in a library as loops over lanes (the meaning) and substituted by one rule each (`fadd {v.4s}, {v.4s}, {v.4s} = add(f32x4, f32x4) -> f32x4`). Needs: a seed slot kind for vector registers with an arrangement (`v0.4s`), rules for NEON and wasm SIMD, and — the part that makes it probe-shaped — every rule checked against the library loop by the suite and the fuzzer, on hardware that runs here (M1 NEON, node's SIMD, qemu's RVV).
2. **Horizontal operations and shuffles** as library generics with rules: reductions, `splat`, lane permutes, and the dot products the AI formats want.
3. **RVV**, which forces the "rule with machine state" question (`vsetvli`), and is the road to the scalable model.
4. **The E4M3 variant and one MX block format** through `/format`, verified exhaustively — a small, complete proof that the format explosion is a library problem here.
5. **A learned-but-unrun GPU target**: seed `targets/amdgcn.probe`, learn it, scorecard it against the ISA's own machine-readable description (AMD publishes one), emit a kernel, and say honestly that nothing here can execute it. Its value is as a test of the learner's generality and of the platform file's expressiveness (wave size, address spaces), and as the point where `structure.rs` becomes reconvergence.
6. **Matrix tiles** last, because they are the easiest to *express* (one rule, one loop nest) and the hardest to *verify* without the hardware — SME on an M4, or MFMA on a machine we do not have.

Sources for the format details: the OCP Microscaling (MX) v1.0 specification and the *Microscaling Data Formats for Deep Learning* paper (arXiv 2310.10537); NVIDIA's Transformer Engine documentation on MXFP8 and NVFP4.
