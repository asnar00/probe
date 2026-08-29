# Scalable vectors — a design note (2026-08-29)

The last vector item that is not "more of the same". Everything vector-shaped in probe today is *fixed*: `f32x4` is four lanes, in a register on NEON and RVV, lane by lane everywhere else, and the platform rules set RVV's `vl` to 4 or 2 as a constant. RVV (and SVE) exist to do something else, and this note is about whether, and how, probe should do it. Nothing here is built; the point is to decide the shape before the code.

## What "scalable" means

A RISC-V vector register is VLEN bits wide, and VLEN is a property of the *chip*: 128 on the qemu we run, 256 or 512 on others, and the same binary is meant to run on all of them. So a program does not say "four lanes"; it asks the machine. `vsetvli rd, rs1, e32` says "I have rs1 elements left, they are 32 bits each — how many will you take this time?" and answers in `rd`: `min(rs1, VLEN/32)`. The loop does that many, advances by that many, and asks again; the last time the answer is the tail, and the instructions leave the lanes past it alone. There are no masks for the common case — `vl` *is* the mask — and no remainder loop. SVE is the same idea with a predicate register in place of `vl`, and a `whilelt` that makes it.

What we do now is a strict subset: `vl` is always the constant the type names, and a `u8x16` on a 512-bit chip uses a quarter of the register. That is honest and it is what NEON is, but it is not RVV.

## What it would take

The heart of it is one question: **is the number of lanes a value?** Today it is part of the type (`i32x4`) and so a compile-time constant, which is what lets a vector be a struct of lanes, `get v, 2` be a field read, and the lane form be the referee. A scalable vector has a lane count known only when the program runs. Three ways to meet that:

**A. Leave it.** Fixed-width vectors, as now. On a 512-bit RVV chip the code runs, correctly, at a quarter of the width. Cost: nothing. What it gives up: the thing RVV is for, and the road to SVE.

**B. A library over fixed vectors — strip-mining.** Keep the IR as it is and write loops in `lib/` that take an element count and chunk it by the fixed width, with a scalar tail. No new types; a platform constant (`vector_bits`) would let the chunk follow the platform. This is what most compilers emit for NEON, and it is a fine thing to have; but the tail is scalar, the chunk is fixed at compile time per platform, and `vl` never appears — on RVV it is still "NEON with vsetivli". Cost: a day; mostly library.

**C. `vl` as a value, and a vector "of as many as fit".** A new type family, spelled `i32xV` (or `i32x[]`), whose lane count is a *value* carried with it. One new operation:

```
n: i64 = fit i32, left            ; how many i32 lanes the machine takes of `left` (1 <= n <= left)
```

and the vector operations extended to the scalable type: `v: i32xV = load p, n` loads n lanes; `w: i32xV = add v, u` requires equal counts (the IR's contract, as lane counts must match today); `store w, q` stores its count; `sum w`, `min w` reduce over the count; `splat x, n`. No `get`/`set`/`pack`/`unpack` with a constant lane on a scalable vector (a lane index is a value, so `lane v, i` would be an indexed read — a new instruction if wanted). A loop is then:

```
loop(i: i64 = 0) {
    left: i64 = sub count, i
    done: u1 = cmp.eq left, 0
    if done { break }
    n: i64 = fit f32, left
    a: f32xV = load pa, i, 4
    b: f32xV = load pb, i, 4
    c: f32xV = add a, b
    store c, pc, i, 4
    i2: i64 = add i, n
    continue i2
}
```

The meaning is *independent of the machine*: `fit` may return any number from 1 up to `left`, and the program is correct for all of them — which is exactly the property that makes it checkable. On RVV, `fit` is `vsetvli` and every rule sets `vsetvli x0, n_reg, e32` from the value (as they set `vsetivli` from the constant now — the design of "the rule sets its own state" carries over unchanged). On NEON, `fit` is `min(left, 4)` and a scalable `f32xV` is an `f32x4` plus a count, with the tail handled by... masked loads NEON does not have: so `fit` on NEON returns 4 only while `left >= 4`, and 1 otherwise — the tail runs one lane at a time, in the same code. On wasm and the GPU, `fit` returns 1 (or 4 for the GPU, which takes vectors whole). **The lane form and the referee**: `fit` = 1 everywhere it is not a rule, and `i32xV` with one lane is an `i32` — so the scalable program *is* its own scalar program, and the check against the lane form is the same loop with `fit` pinned to 1 (`--platform=riscv64-nov`).

What C costs: a type kind that is not a struct of lanes (the parser, the verifier's lane rules, the struct lowering all treat vectors as packs today), the count riding with the value (a pair in registers: the vector and an integer), `fit` as an instruction with a rule, the load/store/elementwise/reduction operations over the new type in both emitters, and the lowering to "one lane" on the paths without it. Two to three days, most of it in `ssa.rs`. Also a first for the project: a program whose *behaviour* (how many iterations) differs by machine while its *result* does not — the suite can check results only, and the count of iterations is a footprint fact.

## What I would do

C, sized to the minimal set — `fit`, `load`/`store` with a count, the elementwise operations and the reductions on `TxV`, `splat` — and nothing else until a program wants it (no indexed lanes, no scalable masks beyond what compares need, no LMUL > 1). It is the one design that says what RVV says; B is a good library to have on top of it, not instead of it; A is where we are. Before building: (1) is `fit` the right name and shape (an instruction with the lane type and the count, or a platform function `fit_i32(left)`?), (2) `TxV` versus `Tx[]` versus `vector(T)` as the spelling, (3) whether a scalable vector may cross a call (it is a register and a count: two arguments) — probably yes, by the convention as it stands, and (4) whether the count rides with the value (my proposal) or is machine state read by `vl()` (RVV's own view; simpler emitters, but a value the IR cannot see, which is against its grain).
