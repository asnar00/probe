# History

What landed, one short entry per commit — or per group, when several arrived together as one piece of work. Newest first. `git show <hash>` has the full story for any of them.

---

### The array layer, rounded out — `ae35e05` · 2026-08-29

```
fn normalize() -> (f32, f32) {
    p: ptr = addr fr
    q: ptr = addr fc
    r: ptr = addr fs
    a: f32[,] = slice p, 2, 2
    c: f32[,] = slice q, 2, 2
    s: f32[] = slice r
    store 1.0: f32, a, 0, 0
    store 3.0: f32, a, 0, 1
    store 1.0: f32, a, 1, 0
    store 1.0: f32, a, 1, 1
    sum s, a
    at: f32[,] = transpose a
    ct: f32[,] = transpose c
    div ct, at, s
    x: f32 = load c, 0, 1
    y: f32 = load c, 1, 0
    ret x, y
}
```

"We should definitely round out the array layer properly." The pieces a matrix program reaches for, each a few lines of `lib/slice.ssa` over what was there: `dot a, b` a reduction of a product — a chunk of partial products while both views are contiguous, its lanes summed, then the tail one at a time; `matmul c, a, b` the dot of each row of `a` with each row of `b`'s transpose, shapes checked, a loop until a platform has a rule for a tile; a rank-1 view on the right of a rank-2 operation broadcast to every row, so a per-row scalar is a column broadcast, which is a row broadcast on the transposes, as the kernel above has it; and `buffer_take`/`buffer_give`, a buffer with its header from the heap of the day before, returned by its own numbers. The suite grew a small kernel — rows normalized by their sums — and a case that shows a row view aliasing its grid, which is what a view is for and what my first expectation forgot. 939 on every path and both variants; the model is what a front end would generate into, and reads well enough by hand that the frictions are the next thing, before streams.

---

### Views with a shape — `ef51522` · 2026-08-29

```
fn strided() -> (i32, i32) {
    p: ptr = addr m2
    b: i32[,] = slice p, 3, 4
    fill b, 1
    t: i32[,] = transpose b
    c2: i32[] = at t, 2
    fill c2, 5
    mul c2, c2, 2
    s: i32 = sum c2
    sc: i32 = sub s, 16
    all: i32 = sum b
    a2: i32 = add all, 3
    ret sc, a2
}
```

"Views carry the shape, and `f32[,]`." A view of rank 1 to 3 is now a typed pointer and, per axis, a count and a stride in elements, so the things a matrix program wants are views of the same memory and nothing moves: `at a, i` is a row, `transpose a` the axes swapped, a column is a row of the transpose with a stride, `block` a rectangle, `reshape` a shape laid over a contiguous run — each a `Pack` of the words with a `check` on every bound, and `load a, i, j` reaches an element with every index checked. `lib/slice.ssa` grew by rank rather than by operation: the rank-1 bodies honour a stride — chunks only when every view is contiguous, a column one element at a time from the start — an operation of rank 2 or 3 is the rank below over each row, a reduction to a scalar goes row by row, and a reduction one rank down, `sum c, a`, is one per row, the columns' being the transpose's rows'. Two things the paths taught: the `rv64i` footprint test found a `mul` in the parser's hidden index arithmetic, which now multiplies by the policy's multiply like everything else; and four cases that read a grid another case had filled passed on the machines and native, where memory persists between cases, and failed on wasm and the GPU, where it does not — so every case builds its own, which is what a test should have done anyway. Nine cases in `suite/matrix.ssa`, two of them `-> check`; 934 on every path and both variants. And from this entry on, each entry opens with the code it is about, verbatim from the suite or the library — the session's entries were given theirs today.

---

### Chunks — `0ec3a49` · 2026-08-29

```
fn fill(c: number[], s: number) {
    n: i64 = len c
    pc: ptr(number) = ptr c
    vc: ptr(chunk(number)) = cast pc
    stc: i64 = stride c
    cc: u1 = cmp.eq stc, 1
    contig: u1 = cast cc
    limit: i64 = if contig {
        yield n
    } else {
        yield 0
    }
    ks: chunk(number) = splat s
    k: i64 = lanes chunk(number)
    stop: i64 = loop(i: i64 = 0, ci: i64 = 0) {
        left: i64 = sub limit, i
        m: i64 = fit number, left
        partial: u1 = cmp.lt m, k
        if partial {
            break i
        }
        store ks, vc, ci
```

The lower layer, and with it the slice operations became vector code without a line of emitter changing. `chunk(T)` is as many lanes of T as the platform's vector register holds — `f32x4` on NEON and RVV, where the platform has a vector class, `f32` itself where it has none — resolved as the fixed vector type it is, so every rule of the last three days applies to it unchanged. Three constants of a type on a platform go with it: `lanes chunk(f32)`, `sizeof f32`, and `fit f32, left`, which is `min(left, lanes)` and the one thing a library needs to say "as many as fit" without knowing the number. `lib/slice.ssa` was rewritten over them: an operation takes whole chunks while `fit` says one is left, walking memory through a typed pointer to `chunk(number)` indexed by chunk — a computed byte offset is not addressing here, a lesson relearned — then the elements one at a time to the end; a reduction accumulates a chunk, reduces its lanes, and finishes the tail. On a platform without vector registers a chunk is one element, `splat` into it is the value, and the chunk loop *is* the definition; with them it is `ld1`, `fadd .4s`, `st1` and `addv` on one machine and `vle32`, `vfadd.vv`, `vse32` and `vredsum` on the other, which `probe footprint` shows for the same suite that the other paths run one element at a time. One thing it needed: an operation's dispatch now matches the operand count, since a one-argument `min` (over a slice, or the one lane of a scalar chunk) sits beside the two-argument one. 925 on every path and both variants. What this is not, said plainly in `reference/scalable-vectors.md`: a chunk is 128 bits because the platform says so; RVV on a wider chip would say more, and the type would have to say "the machine's" for the same binary to widen.

---

### Slices — `62d01d2` · 2026-08-29

```
fn views() -> (i32, i32) {
    p: ptr = addr ia
    all: i32[] = slice p
    lo: i32[] = view all, 0, 4
    hi: i32[] = view all, 4, 4
    fill lo, 1
    fill hi, 2
    s: i32 = sum all
    n: i64 = len hi
    n32: i32 = conv n
    t: i32 = mul s, n32
    m: i32 = sub t, 9
    ret m, t
}
```

The design note on scalable vectors came back with a different top layer than it proposed, and a better one: "size should ride with the ptr". So a `T[]` is a *slice* — a view into a buffer, a typed pointer to its first element and a length, two words, several of which may look into one buffer at different places — and a buffer is memory with a header, the element size and the capacity, then the elements (`data b: buffer(f32, 1024)`, or `buffer_init` on memory the heap gave). `slice p` takes the whole buffer and checks the element size against the slice's; `view a, off, n` a part, checked to lie within; `len` and `ptr` read the words. An operation on slices is whole and writes into its first operand, written like a store — `add c, a, b`, `mul c, a, 2.0`, `fill c, 0.0`, `copy c, a` — and a reduction gives a scalar, `s: f32 = sum a`. Each is a generic of `lib/slice.ssa` over `number[]`, which the tower of the day before made a one-liner to declare, and its body — a loop over the elements, one at a time — is the definition on every path, and what the chunked form to come will be checked against. In the parser a slice is a struct, so the struct lowering and the calling convention carry it with nothing new; the new things are `[` and `]`, `buffer(T, N)` with its header laid into the data, the four instructions with their `check`s, and a statement form for a generic whose first operand is a slice. One thing it sharpened: `min` over a slice is both a reduction (a result) and an elementwise operation (none), so an operation that defines a value dispatches only to a generic with a result and a statement only to one without. Eight cases, three of them `-> check` (lengths that differ, a view past the end, a slice of the wrong element size — the GPU skips those, as it must); 925 on every path and both variants. Next: `chunk(T)` and `fit`, then the library's loop over chunks, then the rules.

---

### The tower — `f50bfa9` · 2026-08-29

```
fn min(a: number, b: number) -> number {
    lt: u1 = cmp.lt a, b
    r: number = if lt {
        yield a
    } else {
        yield b
    }
    ret r
}
```

"If we have a scalar type that is parent to int/float/etc, wouldn't we define max and min on scalars and have that single function just work on all numerics?" — and, to the suggestion of a type parameter: "no, that's what abstracts are for. It's a tower of types at the top of which is... number". So it is. `number` is over `int`, `uint` and `scalar`; `scalar` over the number libraries; each of those over its widths; and the rule that makes it work is one sentence: an abstract name is bound by the nearest thing that binds it — the policy in a program's body, the *argument* in a function's signature. `fn min(a: number, b: number) -> number` in `lib/int.ssa` is then a template, instantiated as `min_i8`, `min_f32`, `min_fixed_16_16` at each call, its body's `number` that type and its `cmp.lt` dispatching as it would on it. Where several definitions of a name take the arguments, the most specific wins — `float` before `scalar` before `number` — which is how `lib/float.ssa` keeps its NaN-propagating `min` and `max` and `lib/rational.ssa` its NaR-aware ones while fixed, decimal, unit and sunit lose their copies, and how `lib/reduce.ssa`'s `sum(v: numberx4)` covers every lane type with one body per lane count. In the parser that is: a plain `fn` whose parameters mention an abstract name bare is recorded as a generic with type parameters; `unify` binds a name to a type as it binds `N` to a width; an instance carries its type bindings through the same re-parse (`tenv` beside `env`); the opcode form and a call by name both choose an instance by unifying every parameter with the arguments — a call by name to a generic had never been dispatched at all, which is what "call to undefined function sum" had been the day before — and a template that is the only definition of its name gets a default instance under that name, the policy's binding, so a program's `fn f(a: int)` is still `f` for a directive to reach. One thing it bumped into: `suite/wide.ssa` defines its own plain `add` on `u128`, so a plain function whose signature matches the arguments is called as written before any generic of the name is considered. 917 cases on every path and both variants; three parser tests rewritten to the new rule.

---

### Across the lanes — `5857a94` · 2026-08-29

```
fn allany(a0: i32, a1: i32, a2: i32, a3: i32, k: i32) -> (u1, u1) {
    v: i32x4 = pack a0, a1, a2, a3
    ks: i32x4 = splat k
    m: u1x4 = cmp.gt v, ks
    every: u1 = all m
    some: u1 = any m
    ret every, some
}
```

The horizontals: what a vector's lanes come to together. `lib/reduce.ssa` defines `sum`, `min` and `max` of a vector and `all` and `any` of a mask as generics over the vector's shape — `fn sum(W)(v: i(W)x4) -> i(W)`, for which the parser learned to take a lane count after a parameterized type — each a pairwise tree, `(l0 + l1) + (l2 + l3)`, because that is the order a pairwise instruction takes, and so the meaning a rule is checked against: NEON's `faddp` twice over an `f32x4` computes exactly it; RVV's `vfredosum` sums in another order and `vfredmin` takes a NaN as `vfmin` does, so on RVV the floats' reductions stay the library's, as their scalar `min` and `max` do. They are written as operations with a scalar result, `s: i32 = sum v`, which took a small change: the operation form on vectors had insisted the result be a vector, and a plain call to a generic's name is never dispatched by type — it is taken on trust as external, which natively had passed unnoticed because every `sum` had a rule and the rule is chosen by signature, and on riscv became "call to undefined function sum". The rules: `addv`, `smaxv`, `uminv` and the rest reduce into lane 0 of a vector register and `umov` reads it out, so a `v` operand may now take a slot named as one of its lanes (`addv {s}, {v}.4s`); two lanes use the pairwise forms; `all` is `uminv` of the 0/1 lanes and `any` `umaxv`; a vector rule's scalar result is recorded at its real width and class now, which two of the failures were. And a thing the lane form found: a `u8x16` reduction, lane by lane, is a function of sixteen integers, and the convention had stopped at eight — so the arguments past the registers go on the stack now, in both emitters, 8 bytes each and a vector 16, the caller lowering `sp` for the call and the callee reading above its frame; the eight-parameter limit is gone, and `suite/multi.ssa` calls a function of twenty-five arguments in three classes to prove it. Twenty cases in `suite/lanes.ssa`; 916 on every path.

---

### Integer min, max, abs, neg — `ec5ed77` · 2026-08-29

```
fn absneg(a: i8) -> (i8, i16) {
    x: i8 = abs a
    w: i16 = conv a
    y: i16 = neg w
    ret x, y
}
```

The smallest of the three items left on the vector list, done first. The integers had never had a `min` or a `neg` — floats and the number libraries define theirs, and an integer `neg` was "no neg from i32: define a generic". So `lib/int.ssa` defines them the same way, as generics over `i(N)` and `u(N)` — a compare and an `if` — and they run everywhere as bodies, while a platform with an instruction substitutes it: arm64 a `cmp` and a `csel` for scalar `min` and `max` (its first integer group besides the base, `select`), NEON `smin`/`umin` and their maxes, `abs` and `neg` over every vector shape, RVV `vmin`/`vminu`, `vmax`/`vmaxu`, `vrsub.vx` from `x0` for a negation and that with a `vmax` for an absolute value. `suite/minmax.ssa` has the scalars at every width and signedness, the most negative `i8` whose `abs` wraps to itself while its `neg` widened does not, and vectors of 32-, 16- and 8-bit lanes; `probe footprint` shows the `csel`s and `smin`s on one machine and the `vmin`s on the other; 895 cases on every path and both variants.

---

### A mask in memory — `c258520` · 2026-08-29

```
fn mask4_mem(p: ptr, a0: i32, a1: i32, a2: i32, a3: i32, k: i32) -> (u8, u32) {
    a: i32x4 = pack a0, a1, a2, a3
    ks: i32x4 = splat k
    m: u1x4 = cmp.gt a, ks
    store m, p
    b: u1x4 = load p
    b0: u1, b1: u1, b2: u1, b3: u1 = unpack b
    r: u8 = mask4(b0, b1, b2, b3)
    bytes: u32 = load p
    ret r, bytes
}
```

The last of the vector "not yet"s that was small. A `u1xN` — a comparison's result, N one-bit lanes — has three physical forms: lane by lane it is N `u1` values; in a register it is 0 or 1 in each of N lanes of 128/N bits, whatever produced it; in memory the IR says a lane is at its natural place and a `u1` is a byte, so N bytes. The third had never been built, and it turned out the lane form had never had it either: the struct lowering split a stored mask into stores of `u1`, which the verifier refuses — only the GPU, which had always kept a `u1` lane as a byte, agreed with the definition. Now the lowering stores a `u1` lane as a `u8` and loads it back through `conv`, and the register form narrows its lanes to bytes on a store (`xtn`, halving each time; on RVV `vnsrl.wi` under the narrower vtype) and stores exactly N of them — a whole register, half of one, or a single lane of it (`st1 {v.s}[0]`, `st1 {v.h}[0]`; on RVV `vse8` under `vl = N`, which is what `vl` is for) — and a load widens them back (`ushll`, `vzext.vf2`). On RVV the narrowings take turns between the even temporaries, the widenings between the odd ones, for the same register-pair rule as yesterday. Four cases store and reload masks of two, four, eight and sixteen lanes and read the bytes back as an integer, the same on every path; 883 cases everywhere.

---

### The other shapes — `376a210` · 2026-08-28

```
fn bytes8(x: u8, y: u8) -> (u8, i16) {
    a: u8x8 = splat x
    b: u8x8 = splat y
    s: u8x8 = add a, b
    r0: u8 = get s, 3
    x16: i16 = conv x
    y16: i16 = conv y
    c: i16x8 = splat x16
    d: i16x8 = splat y16
    t: i16x8 = add c, d
    r1: i16 = get t, 5
    ret r0, r1
}
```

"Let's definitely do those": the 64-bit vectors, and with them the 128-bit ones of narrow lanes, since it was the same work. Both platforms' vector sections are generated from one table of shapes now — `f32x2`, `i32x2`, `i16x4`, `u8x8` at 64 bits, `i16x8`, `u8x16` at 128, their signed and unsigned forms, and the masks `u1x2` to `u1x16` — a rule per operation per shape, the arrangement (`.8b`, `.4h`, `.2s`, `.8h`, `.16b`) or the vtype (`vsetivli x0, 8, e8`) following the type. A `u1xN` keeps its representation whatever it came from — each lane 0 or 1 in a 128/N-bit lane of a whole register — so a 64-bit vector's comparison is widened (`sshll`) before the `neg` makes ones of its all-ones. Conversions between lane widths arrived too: `sshll`/`ushll` and `xtn`, `fcvtl`/`fcvtn` between `f32x2` and `f64x2`, and on RVV `vsext`/`vzext.vf2`, `vnsrl.wi`, `vfwcvt`/`vfncvt` — which hung a machine until the ISA's rule about widened operands was respected: a doubled-width operand is a register *pair* that must be even-aligned, and an allocated register may well be odd, so those rules go through the rule's temporary, `v16`. In the emitters a vector's shape — lane bits, lanes, bits in all — now comes from its type: `ins`/`umov` by lane width, `smov` for a signed narrow lane, `dup` and `ld1`/`st1` by arrangement, `vsetivli` by lanes and width, a narrow lane normalized after `vmv.x.s`. One small thing found on the way: a one-line rule had counted only its template's slots, so a template with a literal operand (`sshll {v}.4s, {v}.4h, #0`) could not be used as one; it carries the literals now. Thirteen cases joined `suite/vector.ssa` — two-lane integers and floats, 16-bit multiplies, bytes wrapping where 16-bit lanes do not, eight- and sixteen-lane comparisons, widening and narrowing both ways, byte shifts, a two-lane vector through memory — and `bytes_sum`, which had carried a `u8x8` round a loop lane by lane since the day vectors arrived, runs whole now. 879 cases on every path, under both no-vector variants; the scorecards count 51 NEON and 52 RVV templates, every one matched.

---

### Each class in its own registers — `47cd7b3` · 2026-08-28

```
fn mixed(k: i32, a: f32x4, h: f32) -> (i32, f32x4) {
    k2: i32 = add k, 1
    hs: f32x4 = splat h
    c: f32x4 = add a, hs
    ret k2, c
}
```

"There's no real reason to always go through the int registers, is there?" There was not. The convention had been a shortcut from before floats existed: every value crossed a call as bits in `x0..x7`, a float `fmov`'d out of its register and back in on the other side, and a vector, being two words, simply refused. Now each class crosses in its own registers, counted separately, the way AAPCS64 and the RISC-V ABI have it: integers and pointers in `x0..x7` (`a0..a7`), floats in `v0..v7` (`fa0..fa7`), vectors in `v0..v7` on arm64 — shared with the floats — and `v8..v15` on riscv64, where the pool moved up to `v24..v31`; arguments and results alike, eight of each. In the emitters that is one function (`abi_regs`) assigning a register by class, used by calls, indirect calls, returns and the parameter prologue, and a rule that is a function now takes its operands where they arrive, with no moves through `x0`. The one real complication was the JIT boundary: Rust calls compiled code through `fn(&[i64]) -> i64` and cannot put a float in `s0` for a signature it does not know, so the compiler generates a wrapper for every function whose parameters or results have a class — `__w_f`: integer words in, cast, call, cast, words out — when a module is compiled for the JIT or installed in the arena, and the JIT's `call` prefers it; every Rust-side caller (the suite, the fuzzer, TestFloat, `probe run`) is unchanged. A vector is a parameter, result or argument like any other value now: `suite/vector.ssa` gained three cases, an `i32x4` through a call, an `f32x4` with a float, and a mixed signature with an integer, a vector and a float in and a vector among two results out, the same on all five paths. Two things it found: on the machines a call on vectors must go to a rule only when the platform has one for that signature (a function of the program's that takes vectors is a call like any other), and the GPU emitter had been applying *every* call with vector arguments per lane — with no function taking vectors before, nothing had noticed — which for a callee typed over vectors produced bitcode LLVM refused (`llvm-dis: Invalid record`, found by the emitter's body-stub switch); a callee whose own parameters are vectors is now called as it is.

---

### RVV — `0987ac4` · 2026-08-28

```
add(f32x4, f32x4) -> f32x4
    vsetivli x0, 4, e32, m1, ta, ma
    vfadd.vv r, a, b
gt(i32x4, i32x4) -> u1x4 with t: u1x4
    vsetivli x0, 4, e32, m1, ta, ma
    vmslt.vv v0, b, a
    vmv.v.i t, 0
    vmerge.vim r, t, 1, v0
```

The same eight vector types on the other machine, and the question `vectors.md` had left for RVV — a rule with machine state — answered the simplest way: every rule sets the vtype it runs under first (`vsetivli x0, 4, e32, m1, ta, ma`, or two 64-bit lanes), and the emitter does the same before its own lane moves and loads; a pass could elide the repeats later, and nothing depends on one. `targets/riscv64.platform` gained `ext V` with `class v` and a rule per operation: `vfadd.vv` and the rest, `vfsgnjn.vv r, a, a` for a negation, `vfmacc.vv` into a copy for `fma`, shifts by a vector of counts directly, a comparison as `vmflt.vv`/`vmslt.vv` into the mask `v0` and a `vmerge.vim` of 1 over a zero vector — so a `u1xN` is 0 or 1 per lane, as on NEON — and, as for scalars there, no `min`/`max` and no float-to-integer, whose NaN cases differ from the library's. In `src/emit_rv.rs` vectors are a third allocator class in v8..v15, saved and spilled as sixteen bytes under e8, their lanes packed by a chain of `vslide1up.vx` (each slide's destination another register than its source), read by `vslidedown.vi` and `vmv.x.s`, a lane set by a mask in `v0` and `vmerge.vvm`; memory is `vle32`/`vse32` at a lane's alignment; an interrupt frame keeps the caller-saved v registers. Two things the ISA made the tools say: an instruction that reads the mask `v0` may not write `v0`, which the learner met as a rejected probe — the seed now has `reg vn = v1..v31` for such a destination, a rule's `v` operand may take a `vn` slot, and the emitters renumber; and a vector instruction with `mstatus.VS` clear is an illegal-instruction hang, so the boot preamble enables the vector unit as it does the FPU, and qemu runs `-cpu rv64,v=true,vlen=128,elen=64`. 45 templates learned, all matched in riscv-opcodes; `targets/riscv64-nov.platform` is the reference (the variants test runs the suite under it, with `rv64im` and `rv64i`, which now say `without V` too); `probe footprint` shows the vector suite using `vfadd.vv`, `vmul.vv`, `vfmacc.vv`, `vmslt.vv` and the slides. The scalable model — `vl` as a value, a loop in chunks of as many as fit, a mask for the tail — is what remains of RVV, and is noted.

---

### NEON — `2d11460` · 2026-08-28

```
class v = f32x4, f64x2, i32x4, u32x4, i64x2, u64x2, u1x4, u1x2
fadd {v}.4s, {v}.4s, {v}.4s = add(f32x4, f32x4) -> f32x4
gt(f32x4, f32x4) -> u1x4 with t: u1x4
    fcmgt t.4s, a.4s, b.4s
    neg r.4s, t.4s
```

The second half of the vector step, and the first time a whole-vector instruction is checked against the lane-by-lane meaning that defines it — the move the project made once for floats, made again for vectors. `targets/arm64.platform` gives eight vector types a register class (`class v = f32x4, f64x2, i32x4, u32x4, i64x2, u64x2, u1x4, u1x2`) and a rule per operation: `fadd {v}.4s, {v}.4s, {v}.4s = add(f32x4, f32x4) -> f32x4`, a comparison as `fcmgt` and a `neg` so a `u1xN` holds each lane as 0 or 1, `fma` as a `mov` and an `fmla`, shifts as `sshl`/`ushl` (a right shift by a negated count), casts between vectors of one width as a `mov`. The parser keeps a vector whole only where the platform has both the class and the rule — `Policy.vectors` is none, all (the GPU), or the platform's types and signatures — so a divide of `i32x4`, a `u8x8` or an `i64x4` stays lane by lane and `probe parse` shows which. In the emitter a whole vector lives in the float file, which is now saved and spilled as 128 bits; lanes move by `ins` and `umov`, memory by `ld1`/`st1`. Rule lines learned to carry arrangements (`fmla t.4s, a.4s, b.4s`) and vector-typed temporaries, and seeds to write literal braces (`ld1 {{{v}.4s}}, [{x}]`). The learner took the 76 NEON templates in a second, every one matched against ARM's own XML on the scorecard. `targets/arm64-noneon.platform` is the same target without the rules — the reference, on the same machine, which a Rust test runs the suite under — and `suite/vector.ssa` gained seventeen cases (two-lane doubles and 64-bit integers, `fma`, `min`/`max`, a NaN in a comparison, unsigned against signed compares, shifts, conversions, memory, a vector carried round a loop, ten live at once so some spill, masks combined), the same on native, riscv, arm, wasm and the GPU. Two things the machine taught, each by hanging in silence: with the MMU off every access is to device memory and must be aligned to its size, so a 16-byte `ldr q` from an 8-aligned address faults — a vector in memory is aligned only to its lanes, which is what `ld1`/`st1` want — and the data section had been 16-aligned from the start of the *code*, not the image, so with the boot preamble in front a task's stack in a `data` array sat at 8 mod 16, which the interrupt frame's 128-bit saves found where the 64-bit ones never had. Not yet: a vector as a parameter, result or argument (the two-word convention is the natural one; `fmov {x}, {v}.d[1]` is learned for it), a `u1xN` in memory, 64-bit vectors, RVV and wasm SIMD through the same seam.

---

### Lockstep, and `-> check` — `1c58088` · 2026-08-28

```
;! __kernel 64 64 -> check

fn __kernel(mem: ptr, area: ptr, id: i64) {
    x: i32 = conv id
    five: u1 = cmp.eq id, 5
    r: i32 = if five {
        yield x
    } else {
        s: i32 = simd_sum x
        yield s
    }
```

The first item from the agent's note: the GPU's silence made an error. A simdgroup operation over fibres assumed every thread of the simdgroup reached it; one that went around a `simd_sum` read a stale slot and got a number nobody would question. Now each fibre counts its puts in a word of its own — `fibre_word`, at 9728 in the thread's block, zero when the fibre starts — and after every round `simd_lockstep` walks the simdgroup and `check`s that every thread has put as often as this one. `suite/lockstep.ssa` is a kernel where thread 5 skips the sum, in groups of 64, of 32, and across two OS threads. What the test needed was a suite that could *expect* a failed check, which it could not: natively a `brk` would have ended the test process, and a machine has no `__trap` in the driver. So `-> check` is a directive's expectation now, and passes when the case ends in one: under the JIT the case runs in a forked child (`fork`, `waitpid`, no crate) that dies of SIGTRAP; on a machine it boots on its own with a `__trap` in the driver that prints `check` and ends the machine; on wasm it is the driver's `trap:` line; the GPU, which runs on past a failed check, skips such cases and says why. `suite/arena.ssa` gained two — a `check` on zero and a third allocation that does not fit. One thing found on the way: the qemu drivers were compiled at origin 0 while the image begins with a preamble, so the vector table before `__trap` was 2K-aligned in the wrong frame and arm64 hung on the first trap; the drivers compile at the preamble's origin now, as `probe boot` always did.

---

### A vector has lanes, a simdgroup has threads — `c0e3842` · 2026-08-28

```
    t: i64 = simd_thread()                        ; 0..simd_size() - 1, a thread's index in its simdgroup
    r: i64 = add h, t
```

Before NEON, one word settled. "Lane" was doing two jobs: an element of a `TxN` and a thread of a simdgroup, which had never met in a sentence until an instruction that works on four lanes at once was about to be checked by a test about a thread that skips one. So a vector has lanes and a simdgroup has threads: `simd_lane()` is `simd_thread()`, a kernel's position in its group is `tid`, and the prose in `lib/gpu.ssa`, `ssa.md`, the platform file and the suites says which it means. `simd_*` itself stays — Apple's word, and what `air.simd_sum.f32` is named after; a SPIR-V path would spell it subgroup the way AIR spells it simdgroup. History is left as it was written.

---

### Four cores — `45540f0` · 2026-08-28

A kernel's groups are dealt across cores on the qemu machines now, as across OS threads under the JIT: `;! __kernel 512 64 4` sums 512 ids in eight groups on four cores of either board, the same answer as everywhere else. `lib/core.ssa`'s `core_launch(core, rec, stack_top, main, arg)` fills a record — stack top, function, argument, the core's key for `thread()` — and the platform's `core_start` does the rest: on arm64 virt PSCI `CPU_ON` with `core_boot` as the entry, a function-body rule that sets sp and `tpidrro_el0` (writable at EL1, learned) from the record and jumps; on riscv64 virt, where every hart runs the reset vector, the boot preamble parks harts other than 0 in `wfi` until a record for them is in a mailbox and hart 0 rings the CLINT's `msip`, then sets sp and `tp` and jumps. Both boards boot with `-smp 4`; a finished core idles in `wfi`. What it took, each found by putting a letter on the UART: a rule that said `u64` where the library said `i64` and so never matched (the fallback ran, and nothing happened); qemu waking a `wfi` hart only for an interrupt enabled in `mie`; a core's own stack given the same top as its last fibre's, so the scheduler and the fibres wrote over each other; and, above all, qemu's output being lost when it is killed on a timeout, so the "output so far" of a hung run says nothing about where it hung — an hour was spent suspecting cases that had already passed. `PROBE_DUMP_DRIVER=path` now writes the whole program a machine runs, for exactly that.

---

### The simdgroup over fibres — `c6f161b` · 2026-08-28

`simd_sum` and the rest meant something on a machine only for one thread; now they mean the same as on the GPU. With fibres running a group, a simdgroup is 32 consecutive lanes and every operation is an exchange through a 64-word table in the thread's block (`simd_put`, `simd_get`, `simd_done` in lib/gpu.ssa): a lane puts its word in its slot and yields — a round, so every lane has put — reads what it needs (its simdgroup's slots for a sum, a max, a prefix sum; one slot for a shuffle; the bits for a vote) and yields again, so no lane writes over a word another has yet to read. Floats travel as their bits, integers sign-extended; the one-thread forms stand off a kernel run, the rules on the GPU. `suite/simdgroup.ssa` — each thread's simdgroup sum, prefix sum and the lane across from it in one word, in groups of 64, of 32, and across two OS threads — passes on native, riscv, arm and the GPU alike.

---

### The thread key — `cb3c50c` · 2026-08-28

The register was the wrong half of the right idea. `tpidr_el0` looked free at EL0, and a block installed in it worked — until the first preemption: XNU keeps per-CPU information there (`libsystem_malloc` reads it, which is how lldb caught the first crash) and rewrites it on every context switch, so under parallel tests a runner thread would resume with the kernel's value and `thread()` would point at 1. What macOS does leave a thread is `tpidrro_el0`: read-only, unique per thread, zero at boot. So the platform now gives a *key* (`thread_key`: `tpidrro_el0`; `tp` on riscv64; nothing on wasm), and `lib/thread.ssa` keeps a table from keys to blocks that `thread_set` fills — `thread_set_slot` for several threads at once, no two writing one slot — with the default block for a key it does not know; a GPU keeps `thread()` as its rule. The JIT no longer installs and restores anything. The suite's `;! __kernel n g m` deals a kernel's groups across m OS threads under the JIT, each thread a slot, a block and stacks of its own, all Rust-allocated; `suite/reduce.ssa` sums 512 ids in 8 groups across 4 threads, and the parallel test run that found the race passes.

---

### The current thread — `48ce834` · 2026-08-28

`thread()` answers, from any function, where this thread's own things are: a pointer kept in a register the platform names — `ext thread` in the platform files: `mrs r, tpidr_el0` on arm64 (two templates learned for it), `addi r, x4, 0` on riscv64, the header every function's `tls` names on AIR — and, where the register is zero (at boot) or the platform has none (wasm), the default block `lib/thread.ssa` declares. The block holds the fibre scheduler, its frame, the fibres' frames and done flags, which `lib/fibre.ssa` now finds through `thread()` instead of global data, and at 16 KB this thread's copy of the program's `group` items: on a machine `addr` of a group item is now `thread()` plus its offset (`lower_group_addrs`, the machine backends' one lowering), so every group in flight has memory of its own, as the GPU gives it; the default block is laid out with the group section behind it. The suite's kernel runner installs a block sized for the program. Two things macOS taught: `tpidr_el0` is not free at EL0 — the kernel keeps a per-thread value there and `libsystem_malloc` reads it — so the JIT installs the program's block only around each call and restores the host's value before Rust runs again; the design (option B, a register) stands, with that one rule about whose register it is when. 837/837 natively, the group, fibre, reduce and simd suites on every path, the arm64 scorecard clean.

---

### Fibres: a group on one thread, and a kernel as a suite case — `4e37c8c` · 2026-08-28

The last of the three: a program written for a threadgroup now runs the same on a machine, and is checked there. `lib/fibre.ssa` runs bodies by turns on one thread, each on a stack of its own — `fibres_run(count, stacks, bytes, body)`, `fibre_yield()` — round-robin, so every body has yielded before any goes on, which is what `group_sync()` needs and now does while fibres run. The switch is a platform rule of a new kind: `fibre_switch(save, to) -> () called` names every callee-saved register (`str x19, [save, 0]` ...; a rule line may name a register by number now) and is reached by a real call, so the return goes where the other fibre's saved x30 says; a fibre that has never run is a frame whose lr is `fibre_entry` and whose sp is the top of its stack. arm64 and riscv64 have it; wasm, with one stack, runs the bodies one after another (`fibre_stacks = 0`). Then the directive: `;! __kernel n g -> words` runs a program's kernel as n threads in groups of g — dispatched on the GPU, a group of fibres per group on a machine through a runner the suite adds — and compares the area's first words; wasm skips it and says so. `suite/reduce.ssa` is the reduction, right on native, riscv, arm and the GPU. On the way: data is writable under the JIT now, on pages of its own after the code, as it always was on bare metal and wasm — a program that wrote its data crashed natively and ran everywhere else, the opposite of what the paths are for.

---

### The simdgroup — `9b8630e` · 2026-08-28

`lib/gpu.ssa` names what a simdgroup computes across its lanes — `simd_sum`, `simd_product`, `simd_max`, `simd_min`, `simd_prefix_sum`, `simd_shuffle`, `simd_broadcast`, `simd_shuffle_xor`, `_down`, `_up`, `simd_all`, `simd_any`, `simd_ballot`, `simd_first`, `simd_lane`, `simd_size` — as generics over floats and 32-bit integers with one-thread bodies (the argument, the value, 0, 1), and an opcode now reaches a library generic for an integer as it did for a pack, so `simd_sum x` reads the same on an `i32`. `targets/air.platform` makes them Apple's intrinsics, whose names and shapes were read off their compiler: `air.simd_sum.f32`, `.s.i32`, `.u.i32`, `.f16`, shuffles taking an `i16` lane, votes on `i1`, `simd_ballot.i64`; a `simd_*` with no rule is an error on AIR, not its one-thread body. The lane index and width are kernel arguments in AIR, so the wrapper leaves them in a 16-byte header at the start of each thread's slab, and every function takes a third hidden parameter, `tls`, to find it. A rule's integer arguments are typed as integers now (a float is a classed argument), which the intrinsics needed. `suite/simd.ssa` on all five paths; `examples/simd.ssa` on the GPU with a test: 64 threads, the sums 496 and 1520 with the lane beside them.

---

### Group memory, typed and sized — `8303dab` · 2026-08-28

`group tmp: array(i64, 64)` declares what a threadgroup shares the way `data` declares what the program has: an array type, nothing else, reached through `addr` and the typed loads and stores — a vector element, a shape, whatever the type says — in place of the 16 KB of `i64` words at byte offsets the library gave before. On AIR every group item is laid out in one `addrspace(3)` array sized to fit, and the emitter decides which loads and stores are threadgroup accesses by a fixpoint over the function: `addr` of a group item is one, and so is whatever arithmetic, cast or block argument carries it; such an address cannot be stored, passed or returned, which the emitter says. On a machine a group item is data — and the JIT, whose data pages are write-protected, now puts the group items on writable pages of their own right after the code (`layout_data_parts`, `Compiled:: writable_from`), where the PC-relative addresses expect them. `suite/group.ssa` runs on all five paths; `examples/reduce.ssa` reads as it should. `probe parse` prints shapes again.

---

### Vectors whole to the GPU — `4a008b9` · 2026-08-27

`targets/air.platform` says `builtin vectors`, and a `TxN` now reaches the AIR emitter as one value: the parser emits a single vector-typed instruction where it made one per lane — a library call on pack lanes takes vectors for its lanes — `aggregate.rs` leaves a lane struct whole, the wide lowering knows a 128-bit vector is not a 128-bit integer, the folder leaves vectors alone, and the verifier learns the lane rules (a `u1xN` from a `cmp` on `TxN`, a call on N lanes gives N lanes back). In the emitter a vector is `<N x T>`: arithmetic, comparisons and conversions are LLVM's own on vectors, `pack`/`unpack`/`get`/`set` are insertelement/extractelement, a literal of a vector type is a constant in every lane, a rule applies to the whole vector (`fadd <4 x float>`, an intrinsic by its vector name, `air.sqrt.v4f32`, as Apple's front end spells it), and a library operation with no rule is made once per lane. `u1` lanes are a byte each in memory, as the struct's layout has them. Every other backend is untouched: 22/22 vector and typed-pointer cases on the GPU, the suite unchanged on all five paths, `probe parse x.ssa air` shows the whole form (and prints the module even when the verifier objects).

---

### A reduction on the GPU: threadgroups — `60f319e` · 2026-08-27

`fn __kernel(mem, area, id, lane, group)` is a kernel that knows its place: the wrapper passes `thread_position_in_threadgroup` and `threadgroup_position_in_grid` when the signature asks. `lib/gpu.ssa` gives every program `group_load(off)`, `group_store(v, off)` and `group_sync()` — words at byte offsets in a buffer in data, and nothing, so a program runs everywhere as a group of one — and `targets/air.platform` makes them the platform's: a 16 KB `addrspace(3)` array with an `undef` initializer (the writer's first global with one) and `air.wg.barrier(2, 1)` declared `convergent`. `examples/reduce.ssa` sums each group of 64 ids by halving at every barrier; the driver takes the group size; a Rust test runs it (`[2016, 6112, 10208, 14304]`). The suite is unchanged on every path with the new library in every program.

---

### The GPU, from our own bitcode — `3ab3971` · 2026-08-27

A fifth execution path: `probe compile x.ssa air` writes a `.metallib` — LLVM bitcode in Apple's AIR dialect inside their container, byte by byte from `src/bitcode.rs`, with none of Apple's tools in the path — and the Metal driver compiles it for whatever GPU it finds. `fn __kernel(mem: ptr, area: ptr, id: i64)` is the entry: one buffer of memory where pointers are offsets (as on wasm), `data` at zero, a scratch slab per thread, the driver's area where it says. `src/emit_air.rs` maps blocks to blocks and parameters to phis, returns several values as a struct, dispatches function values by a switch, and leaves recursion out with a reason (`probe test air` counts it skipped: 15 cases). What Apple's compiler taught us, each by a bisection: it crashes on an `or` with a wide constant at an odd width, so every integer now lives normalized in an 8/16/32/64-bit container like wasm's; it compiles every function in a module, so only what the kernel reaches is emitted; and with inlining left to its judgement it miscompiled a 128-bit division — upstream LLVM ran the same bitcode right, so did `noinline` — which `alwaysinline` fixes and speeds up six times over. Platform rules with no operands (`fadd = add(f32, f32) -> f32`, `air.sqrt.f32`, `air.fma.f32`) are Apple's instructions; `targets/air.platform` has f32 and half. The suite runs 804/804 on the M1 Max in 33 s, with a Rust test; `probe testfloat air` runs TestFloat a thread per vector — 6.1M `mulAdd` cases in one dispatch, 19.4M in all: the library exact at every width, the f32 instructions missing only where the GPU flushes a denormal (116,504 of them, counted apart; Apple's compiler has no switch, and half keeps its denormals); `probe fuzz --air` puts the GPU in the fuzzer's panel. Along the way: bitcode value ids follow written order and blocks are written in the order first entered (LLVM forward references need types we don't track); function attributes; `i64::MIN` as a signed VBR; the Python driver zeroes its buffers, which Metal does not promise; `sleep_boots` now places each wake in the frame its printed lateness says it ran in.

---

### Typed pointers and shaped arrays — `17d787c` · 2026-08-27

The other half of the type work the GPU asked for. `ptr(T)` is an address that knows what it points at — a scalar, a vector, a struct, or `array(T, W, H, ...)` with a shape — so `load g, i, j`, `store v, g, i, j` and `index g, i, j` take indices, as many as the shape has dimensions, and check the element; `ptr` stays what it was, bytes at any step. An array is a memory type and never a value: it is what a typed pointer points at, what `scratch` sizes itself by when its result is typed, and what `data` declares, now with a shape. The lowering is the parser's: the shape makes the offset (row-major, innermost first), the element makes the step, a hidden cast to `ptr` keeps the printed program re-parsable, and the multiply goes to the library on a core without one — the `rv64i` footprint test caught that within the hour, as it did for vectors. No backend changed except one line: on wasm a typed pointer is an `i32`, like `ptr`. Textures are deliberately not arrays; they will be handles with platform operations. Nine cases on four paths; the AIR emitter is next, with `<N x T>` and typed pointers to lean on.

---

### Vectors, `TxN` — `6590476` · 2026-08-27

Before the GPU emitter, the type it will lean on. `f32x4`, `i32x8`, `u1x4`, `floatx4` with the policy's float, `intxN` in a generic: N lanes of one type, spelled the way one says it, and `Tx1` is `T`. A vector is a struct whose fields are its numbered lanes, so building, splitting, indexing, loading and storing one are the struct's operations already there; what is new is that `add`, `cmp.gt`, `conv` and `sqrt` on a vector mean the scalar operation on each lane — a definition the IR makes itself, as it does for integers wider than a word, rather than a library's, because "per lane" is structure, not arithmetic. The parser writes a vector operation out lane by lane and the struct lowering makes the packs and unpacks free, so vectors run on every backend today, verified against nothing but the scalar operations they are made of; a platform with vector registers may later keep a type whole and take the operation in one instruction, checked against this meaning — the choice between one register and many lanes being the platform's, which is what the architectures disagree about. One thing caught by the variant test: a `mul` lane on a core without a multiplier must go to the library like a scalar `mul` does.

---

### A metallib by hand — `0dc7a3d` · 2026-08-27

The GPU thread starts where the memory thread ended: on this Mac, and on the M6 that is not announced yet. Apple's GPUs have no public ISA and a different one each generation; what Apple keeps stable is the bitcode its shader compiler emits (AIR: LLVM bitcode with typed pointers, address spaces, `air.*` intrinsics and metadata naming a kernel's arguments), which the driver compiles for whatever chip it finds. So that is the binary, and the goal is to produce it with none of Apple's tools in the mix. The analysis went as it did for ARM: the Metal toolchain was fetched, a catalog of tiny kernels compiled, and what came out read with LLVM's own bcanalyzer and disassembler (`tools/probe-air.sh` keeps the record). Then `src/bitcode.rs`: the bitstream and the `.metallib` container, written from the public format and the observed records — and `add1`, built by hand in a test, disassembles under upstream LLVM and runs on the M1 Max. Two things learned the hard way: upstream LLVM 19's own encoding of the same module crashes Apple's compiler service (the format must be theirs, record for record), and a bitcode the driver has not seen validated by `llvm-dis` first is a way to crash it again — so the GPU only ever sees bytes LLVM has accepted. Also that the string table is a top-level block after the module, which cost an hour.

---

### Input by interrupt — `87ee51b` · 2026-08-27

`os/echo.ssa` no longer polls. `uart_irq_on` asks the board for an interrupt per received byte, `__irq` drains the port into a ring, and `read` sleeps in `idle` until a line is in the ring, so between keystrokes the machine does nothing at all; the last line says how many interrupts the input took (one or two: qemu hands a pipe over in bursts). The board side is the platform's again, and it exposed the one asymmetry between the two machines that a kernel has to know about: riscv64 delivers every device interrupt as a single cause, "external", and names the source only at its controller, so the platform gives `irq_claim()` for the source and `irq_done` completes it; arm64's controller names the source in `irq_ack` itself, and `irq_external` there is an id that never arrives. One kernel serves both. Found on the way, by a storm of interrupts before the first prompt: riscv's `irq_on` had been enabling the timer's interrupt while `mtimecmp` still held its reset value of zero, which the earlier kernels never noticed because they armed or disarmed the timer first. Arming the timer enables its interrupt now, as arm64's control register always did, and `irq_on` leaves the timer alone.

---

### A decision: no lifetimes in the pointer type · 2026-08-27

Considered and declined: regions in the pointer type — `ptr frame`, `ptr call`, and a verifier rule that a pointer never goes where something longer-lived is expected. It would have been small and static, and it is what Cyclone and Austral do. But the IR is a target, and whether a pointer outlives its memory is the contract of whatever generates the IR, which is expected to be smart enough never to do that — as it is expected never to emit a use before a definition it cannot see. The verifier checks the IR's own well-formedness; the front end's discipline is the front end's. Unique pointers go the same way, for the same reason.

---

### The heap is the machine's memory — `74bb298` · 2026-08-27

"Shouldn't the heap come from the data memory?" It did — a declared array of 256 KB, a stand-in sized by hand, which is the kind of number the rules want out of a program. What the machine actually has is 128 MB with the image at the bottom and the boot stack a few MB up; everything above is a program's to carve. So the platform files say where that is (`heap_base`, `ram_end`) and `os/sleep.ssa`'s heap is those 112 MB, in 4 KB units so the split tree stays small — the unit is now `heap_init`'s to choose. Under the JIT and wasm a heap stays over a declared array: those are not machines, and there is no rest of RAM to have.

---

### The root: a heap for regions — `25724a3` · 2026-08-27

The heap after all — but as the root the rungs are carved from, not a `malloc` for objects, which is seL4's shape and what keeps it inside the rules. `lib/heap.ssa` is a buddy allocator over one declared block: pieces are powers of two aligned to their own size, taking is a split and giving back a merge, a byte per node of the split tree makes a double give or a wrong size a failed `check`, and `heap_seal` lets a kernel forbid allocation inside an interrupt — the discipline, stated: objects live in the rungs, the rungs come from the heap, nothing allocates from the heap in a handler. `os/sleep.ssa` now carves its stacks' pool and its frame arenas from one at boot, seals it in `__irq`, and the one-shot task takes a region of its own, uses it through an arena, and gives it back: `66560 heap bytes out` at the end, the task's 4 KB gone home. Two harness lessons: the qemu driver was writing test buffers a store per byte, so ten 8 KB zero buffers became a megabyte of code and a call outran `jal` on riscv — RAM starts zeroed, so zeros need no store; and a timing test must not share the host with the regression suites, so everything that runs a machine now takes turns.

---

### The second rung: pools, and tasks that come and go — `e762343` · 2026-08-27

`lib/pool.ssa`: fixed-size slots over declared memory, taken and given back in any order, a free list threaded through the free ones and a flag per slot so that giving one back twice is a failed `check`. What it unlocks is the thing that had been waiting on it: a task that exists for a while. In `os/sleep.ssa` stacks come from a pool of four, tasks have a state, and `run_at(t, f, arg)` spawns a one-shot task — the half-second callback spawns one for 63/100 s — that sleeps to its time, runs, marks itself done and asks to be rescheduled; the scheduler hands its stack back once it is no longer the interrupted one, and the report ends `3 stacks out`. One `check` did its job on the way: a wrongly written slot search (`check over` where "a slot remains" was meant) stopped the machine at the callback, with the address, rather than running on into a spawn that never happened. And a lesson about what a test may demand of a machine: two events within its wake-up latency of each other merge into one interrupt and decide which of two lines prints first, so the count and the interleaving are the machine's; the wakes, the frames' contents and the stacks are the program's, and those are exact.

---

### The first rung of memory: arenas, and `check` — `7eff7ca` · 2026-08-27

The memory design session (`reference/memory-management.md`) came out where the safety-critical rules and the game engines both stand: no heap, a ladder of lifetimes instead — a call's (`scratch`), a frame's, an object's, the machine's (`data`) — each a bump or a free list over memory the program declared, each with a declared capacity, the only failure exhaustion at a known site. This is the simplest rung that is not wrong. `lib/arena.ssa`: bump allocation over any memory, sixteen-aligned absolutely, reset all at once, marks for the stack flavour. `check c`: the assertion — the one instruction memory asked of the IR — a breakpoint trap (`brk`, `ebreak`, `unreachable`) that reaches `__trap` with a cause and the address of the check, and what an arena does when it runs out rather than hand back a pointer nobody tests (`os/check.ssa`). And the frame allocator from GPU engines, in the OS: `os/sleep.ssa` keeps two arenas, the scheduler writes a record into the current one at every switch, a callback flips them every tenth of a second after checking the consumer has finished, the idle task prints each closed frame and resets it. Two lessons kept: a record is a *switch*, not a pick (a task re-picked after a callback's reschedule is not a wake), and events that fall within one machine's wake-up latency of each other merge into one interrupt, so the callbacks moved to times of their own and the count is exact: thirty-one events, thirty-one interrupts. Not built, on purpose: pools, regions in the pointer type, the capacity analysis — each sits on this and changes none of it.

---

### `at` and `after` — `fba9011` · 2026-08-27

The question was whether "at a time, do this" wants an instruction in the IR. It does not: a time is a library type and a callback is a function value, both already there, so `at(t, f, arg)` and `after(d, f, arg)` are two kernel functions — a queue of `(time, fn, arg)` in `data`, due entries run inside `__irq` before the scheduler picks a task, pending ones counted when the timer is armed. Exact and cheap, hence short and never sleeping; the task-level kind, which may sleep, waits for the memory design session because it needs a stack each. In `os/sleep.ssa` one callback is set half a second after boot and registers another at exactly a quarter of a second after its own time: `!` prints before `a 500000` on the interrupt they share, `!!` 250000 µs later to the microsecond, nineteen interrupts. `after` returns the time it registered, so a callback reports what was scheduled rather than the late moment it runs at. Two places time might later touch the compiler, noted and not taken: closures, so `at t { ... }` could capture; and static bounds on how long the code it emits takes, which is what would let a callback's deadline be checked rather than hoped for.

---

### Sleeping tasks and a tickless timer: `os/sleep.ssa` — `ada2d1f` · 2026-08-27

The fifth operating system: three tasks that sleep until exact times, every 1/10, 1/3 and 1/7 of a second, each deadline a `time` — a rational — so that 3/3, 7/7 and 10/10 of a second are one instant and the three wake on one interrupt, in index order. Twenty wakes, in the order the fractions say; eighteen interrupts, because the timer is armed to the next deadline rather than to a tick and a shared deadline costs one. A task going to sleep asks the machine for an interrupt — `reschedule()`, riscv's msip or an SGI on the GIC — so every switch still happens in `__irq`. Two lessons: a timer left armed in the past fires again the moment interrupts are enabled (both machines stormed until the scheduler learned to disarm it when nothing is pending), and the harness now keeps the output a machine produced before timing out, which is how that was seen. Lateness is qemu's wake-up granularity, milliseconds; on a board it would be microseconds.

---

### A decision: everything at kernel level · 2026-08-27

Considered and declined, for now: user mode. It would need no IR change — privilege is machine state the trap frame carries, so it is two more platform rules (the saved status register, `mstatus`/`spsr`) and a kernel stack at handler entry (free on arm64, an `mscratch` swap on riscv) — but what it buys is containment of one's own bugs, and only with memory protection (PMP; on arm64 the MMU and page tables), at the price of a trap per service, copies across the boundary, and a kernel/user split of every name. This is a single-user machine built by its one user, in the tradition of Oberon and the language-safe systems: one program, compiled together, every function in the verifier's sight. So: one level, "system calls" are calls, and two conventions keep the door open — only the handlers touch machine state, and the kernel's services are a table of function values, so a component that ever needs isolating can be moved behind a real trap without redesigning the rest.

---

### Preemptive tasks: `os/tasks.ssa` — `7f42d6c` · 2026-08-27

`probe boot os/tasks.ssa` prints `ababababab`: two tasks, preempted by the timer, a slice each — the fourth operating system. The mechanism is one signature: an interrupt handler written `fn __irq(sp: ptr) -> ptr` is handed its frame, where the interrupted code's registers now are, and returns the frame to go back from; the epilogue switches the stack pointer to it before restoring and returning. So a task is a stack and a place to resume, a switch is two stores and two loads around `resume()`/`resume_at()`, and a task that has never run is zeros for a frame and its function's address. The first try printed `abbbbbbbbb`: the handler had kept only the caller-saved registers, correct for returning to the same task and wrong for a switch, where the whole file belongs to the task — the callee-saved registers, float ones included, are kept too for this form now.

---

### Interrupts and the timer: `os/clock.ssa` — `664a7fc` · 2026-08-27

`probe boot os/clock.ssa` prints `tick` ten times, a tenth of a second apart, then `10 ticks in 1006 ms` (1010 on arm64): the third operating system keeps time. Interrupts land in `fn __irq()`, which `probe boot` compiles with a frame that keeps *every* register of the interrupted code, float scratch included, since an interrupt lands between any two instructions; both machines get a sixteen-entry vector table before `__trap` — arm64's IRQ entries branch to `__irq`, riscv64's mtvec goes vectored with entry 0 for exceptions and the rest, by cause, for interrupts. What the board does is the platform file's, as before: `now` and `hz` (the generic timer's counter and frequency; the `time` CSR at 10 MHz), `timer_at` and `timer_off` (`cntp_cval_el0`; the CLINT's mtimecmp), `irq_on` (the GICv2's distributor and cpu interface and `daifclr`; mie and mstatus), `irq_ack` and `irq_done`, `idle` (`wfi`). Those rules needed registers of their own for addresses and values, so a rule may now declare typed temporaries — `irq_on() -> () with gic: ptr, v: u32` — and `none` is a rule that does nothing. Each deadline is one step on from the previous deadline, never from "now", so the ticks do not drift; and the elapsed count becomes milliseconds exactly through `lib/time.ssa`'s rationals — a period times a count — which is the point: getting time right starts at the bottom.

---

### Dominance, fall-through, scratch — `147fe74` → `ad80a53` · 2026-08-27

Three items from the list. *Dominance* (`147fe74`): the verifier's rule 7 — every use is dominated by its definition, earlier in its block or in a block every path from the entry passes through; the dominator tree moved out of the wasm structuring into `structure::Dom` so both share it. It found that `addr` and `platform` results had never been on the verifier's definition list at all. *Fall-through* (`30430b9`): a jump to the block laid out next is not emitted, and a conditional branch whose taken side is next is inverted over the other; no relaxation pass was needed because block offsets were always patched after the whole function. *Scratch* (`ad80a53`): `p: ptr = scratch 64` is memory that is the function's while it runs — its frame on arm64 (`add x, sp, #imm`, newly learned) and riscv64, a shadow stack in linear memory on wasm (one mutable global, `global.get`/`set` newly learned) — 16-aligned, one area per instruction, one per activation. Until now every byte a program touched came from its caller or from `data`. Memory that outlives a call is deliberately not started: `future-work.md` says it needs a design session first.

---

### Traps and system calls: `os/echo.ssa` — `504879b` · 2026-08-27

`printf 'hello\nbye\n' | probe boot os/echo.ssa arm` and the machine answers `> echo: hello` then `> ` and stops: the second operating system. A kernel installs a trap handler and serves write, read and exit through a `data` table of function values (yesterday's feature, put to its purpose; `data` may now hold pointers and function values), and a program of nothing but system calls prompts, reads a line from the serial port and echoes it. The handler is `fn __trap(a, b, c) -> u64`: `probe boot` compiles it with a frame that keeps every register of the interrupted code and a return by `eret`/`mret`, and on arm64 lays a 2K-aligned vector table of branches before its entry; it is called with the trapped code's first three argument registers and its result replaces the first, so `syscall(n, a, b)` on one side meets `__trap(n, a, b)` on the other. How a machine takes a trap is the platform file's business: `vectors`, `cause`, `resume`, `resume_at` and `syscall` are functions whose bodies the platform supplies — `msr`/`mrs`/`svc` on arm64, `csrw`/`csrr`/`ecall` on riscv64, ten instructions newly learned — plus two constants for what a system call looks like and how far past it to resume, and three for the UART's receive side. Rule lines may now spell a template's fixed operands (`msr vbar_el1, t`). Not yet: device interrupts, which can land with float scratch registers live.

---

### Function values — `fd64eaa` · 2026-08-27

A function is now a value. Its type is its signature, spelled as the function declares it — `fn(i64) -> i64`, `fn(ptr, i64)`, `fn(i64, i64) -> (i64, i64)` — so a value carries everything the verifier needs; the same `addr` that reaches a `data` item makes one, and a call through it is written exactly like a call by name. The value goes wherever a value goes: parameters, results, block parameters (a reducer carried around a loop), memory (a table of handlers, built with `store`), `cast` to its bits. arm64 does it with `adr` and `blr`, riscv64 with `auipc`/`addi` and `jalr` — all already learned; wasm needed one new template, `call_indirect`, learned through a seed that declares a table and 130 identical types to range over, plus a table and element section listing the address-taken functions. In the incremental arena a value is the callee's trampoline, so it survives edits and promotion. Seventeen cases in `suite/indirect.ssa`, on all four paths under every policy and variant. This is the piece the OS needs next: trap and syscall tables.

---

### hello world ᕦ(ツ)ᕤ — `16cfa2b` · 2026-08-26

`probe boot os/hello.ssa` (or `... arm`) and qemu's serial port says `hello world ᕦ(ツ)ᕤ`: the first operating system written in probe, on both bare-metal machines from one source. It needed four small things the IR did not have: `data` — a string is an array of UTF-8 bytes, initialized memory laid out after the code and reached PC-relative (`adr` and `auipc` newly learned); `addr` and `len` on it; `platform uart`, a constant the platform file provides per board; and a way to end the machine — riscv's finisher is a store, arm's PSCI is a `hvc`, which the platform supplies as the body of a plain function. Along the way: with the MMU off aarch64 faults on an unaligned 64-bit load, and the image's preamble had left the data four bytes off.

---

### ISA variants — `2524621` · 2026-08-26

A platform file is now grouped by extension, and a variant is three lines: `target riscv64`, `base riscv64`, `without M, F, D`. Dropping F and D makes every float operation the library's; dropping M makes `mul`, `div` and `rem` library calls too (a shift-and-add `mul(W)` joins the division generics), the wide lowering included. `--platform= rv64i` selects a core for every command, and the same 736-case suite passes on qemu for `rv64im` and `rv64i`, and natively for `arm64-nofp` — slower, unchanged answers. `probe footprint` decodes what a program actually used against the learned templates, and a test proves the `rv64i` build of the whole suite touches nothing from M, F or D (it caught the emitter's own multiply in a struct stride). ARM's variants have their slot; only the no-FP one is populated.

---

### a look outward — `5be74a6` · 2026-08-26

`vectors.md`: a survey, before any vector work. The learner's assemblers turn out to reach NEON, SVE and SME, RVV, wasm SIMD — and AMD's GPU ISAs, matrix cores and fp8 included, so a GPU is a target probe could *learn* today even though nothing here could run it. Five execution models, from fixed SIMD to dataflow, measured against what the project already has; the AI format explosion (fp8 E4M3, fp6, fp4, MX blocks, NVFP4) as libraries; and an order: fixed-width vectors as a type and a register class first.

---

### struct — `b38526d` · 2026-08-26

`type point = struct { x: f32, y: f32, z: f32 }`. A `pack` is bits; a `struct` is fields side by side — at natural offsets in memory, as separate values in registers — and never a bit pattern: no `cast`, no literal, no arithmetic. That one refusal is what leaves the layout to the compiler. Right after parsing every struct value dissolves into its fields (`src/aggregate.rs`): `pack`, `get`, `set` and `unpack` become names for values that already exist, so `get (get l, to), z` on a line of two points compiles to nothing at all; a `load` or `store` becomes one per field at its offset, and `load p, i, 12` walks an array of 12-byte structs. The suite passes a struct as its fields.

---

### decimal — `a54cdd5` · 2026-08-26

`decimal(N, S)`: an `i(N)` significand at scale 10^S, so cents add exactly and 1000 × 0.10 is 100.00; `mul` and `div` round half away from zero in 128 bits. Written as a test of `formats.md` — the recipe held, with one addition: a constant like 10^S has no width-expression form, so it is a small generic helper (`pow10(S)()`) that the const-folder folds away.

---

### wasm without the dispatcher — `7f3fb19` · 2026-08-26

The wasm emitter used to turn every function into one big loop with a `label` local and a chain of `br_if`s — a switch pretending to be control flow. Now `src/structure.rs` computes the dominator tree and the emitter nests from it: a loop header becomes a `loop`, a block that several paths reach becomes a `block` ending where it starts, conditionals are `if`/`else`, and branches are `br` to a label or the target emitted in place. Structured source (`if`/`loop` sugar) and the flat block form both produce reducible graphs, and the passes keep them so; a test checks the whole suite at every level. The dispatcher is kept only for an irreducible graph, which another test constructs by hand.

---

### the recipe for a format — `40a0b66` · 2026-08-26

`formats.md`: how to add a number format, in seven steps, with `time` as the example — a type, generics named after opcodes, `conv` for literals, a suite file checked against an independent oracle, and only optionally a policy family or platform rules. `/format <name>` scaffolds one. The point of the document is what it does not contain: no compiler changes.

---

### time — `13514bd` · 2026-08-26

`lib/time.ssa`: `type time = rational(64, 64)`, an exact number of seconds, with `seconds`/`millis`/`micros`/`nanos`/`period` to make one and `to_*` to read one back. There is no arithmetic in the file — `add`, `mul`, `cmp` on a `time` are the rational library's by dispatch — so a sample period at 44100 Hz times 44100 is exactly one second and thirds and sixths add to a half. What made it possible: the rational library now works in 128 bits, so its parts can be 64 bits wide.

---

### addressing modes — `9220416` · 2026-08-26

`v: i64 = load p, 16` and `v: i32 = load p, i, 4` (base + index × step), and the same on `store`. The SSA's memory model is unchanged — values are named registers without limit, `load` and `store` are the only two memory instructions, spilling is the allocator's business — but the address forms are now the ones the targets' instructions take, so an offset no longer costs a `ptradd` first.

---

### float registers, and rules as single instructions — `3325600` · 2026-08-26

The first rule files spelled every float op with moves around it — `fmov s0, a` / `fadd s0, s0, s1` / `fmov r, s0` — because every value lived in an integer register. Now a platform file declares where a type's values live and maps one instruction to one operation:

```
class s = f32
fadd {s}, {s}, {s} = add(f32, f32) -> f32
```

The allocator has register classes (a linear scan per file, spill slots shared), the three emitters keep float values in float registers (arm64 `v8..v15`, riscv64 `fs0..fs11`, wasm `f32`/`f64` locals), and a chain of float operations compiles to just the instructions; a move between files happens only where a value really changes class. Types in rule files are written by their program names — `f32`, not `float(8, 23)`. Composite rules keep the indented form (`fcmp a, b` / `cset r, lo`).

---

### binary128 from the same library — `85be481` · 2026-08-26

`type f128 = float(15, 112)` and every float operation works, from the generic bodies that already served fp8 to f64 — nothing in `lib/float.ssa` knows about 128 bits. What it took: constants that hold 128 bits (`const 1 << 112` used to wrap), and the few places the library built products or shifted significands in hand-split `u64` pairs now just name a type wide enough (`u(2 * M + 10)` for a product) and let the lowering make words of it. `fixed` and `unit` multiply and divide in `u128` the same way, and the old 128-bit helper functions are gone. `suite/f128.ssa` checks add, sub, mul, div, sqrt, fma and conversions against exact rational arithmetic rounded at 113 bits.

---

### wide values — `4357f56` · 2026-08-26

`i128`, `u256`, a 136-bit pack: any integer or pack up to 256 bits. Nothing changed in the backends. A wide value is checked as written, then lowered to a row of 64-bit words right after parsing (`src/wide.rs`) — carry chains for `add`/`sub`, schoolbook products for `mul`, word arrangement for shifts (a branchless logarithmic select when the amount is a runtime value), lexicographic compares, sign-filled extensions, masked field access for packs, word-by-word memory. `div` and `rem` are the exception: they dispatch to `lib/wide.ssa`'s `div(W)`/`rem(W)`, restoring-division loops written in SSA over the wide type itself and lowered like anything else. A `u128` parameter is two word parameters and a `u128` result two results, which is what the suite directives give and expect (`suite/wide.ssa`, 130 cases against Python's integers, plus 400 random rows against Rust's `u128` in a test).

Two things surfaced by the first 256-bit multiply: the register allocator now shares spill slots between values whose lifetimes do not overlap, and the suite's bare-metal driver is one function per case.

---

### platforms as rule files — `3181412` (+ `7032ac7`, a test-race fix) · 2026-08-26

`targets/arm64.platform`, `riscv64.platform`, `wasm32.platform`: what a target does natively, as text. A rule is a library instance's full signature and the learned templates that compute it —

```
add(8, 23, 0)(a: float(8, 23), b: float(8, 23)) -> r: float(8, 23)
    fmov s0, a
    fmov s1, b
    fadd s0, s0, s1
    fmov r, s0
```

— with `a`/`b`/`c` the arguments, `r` the result, `s0`/`d0`/`f0` scratch float registers, and literals for immediates and conditions (`cset r, lo`). Each line resolves against the learned encodings by mnemonic and operand shape, so a rule can only name instructions the learner verified, and a line it has no template for is an error. The three Rust tables and the `Native` enum they hung off are gone; an emitter keeps only the register assignment. Adding a native op is now an edit to a text file, and a target whose ops take several instructions (a stencil) is the same kind of edit.

---

### encoding scorecards — `0afc68f` · 2026-08-26

The learner derives encodings from an assembler's bytes and never reads a manual; `probe scorecard` is the manual, afterwards. It checks every learned template against the official inventory of its target — Arm's Machine Readable Architecture XML, riscv-opcodes, wabt's opcode table — by decoding the template's fixed word to an official encoding and requiring the learned fields to sit inside that encoding's operand fields: `add {x}, {x}, {x}` is `ADD_64_addsub_shift` with `Rd`, `Rn`, `Rm`; `beq` puts its scrambled immediate exactly in `bimm12hi+bimm12lo`. All 150 arm64, 90 riscv64 and 125 wasm32 templates pass. The cards (`targets/*.scorecard.md`) also count what the inventory has that is not learned, by group and by mnemonic, which is the to-do list for the seed files.

---

### rounding modes, and TestFloat as the oracle — `b6a7bae` · 2026-08-26

The library's float operations gain a third width parameter, `round`: 0 nearest even, 1 toward zero, 2 down, 3 up, 4 nearest away. Only `fpack` ever rounds, so that is where the modes live — the rounding itself, overflow to infinity or to the largest finite depending on the side, the sign of an exact zero — and `add`, which used to round by hand, now goes through it too. A generic parameter nothing binds is filled by name from the enclosing instantiation or the policy, so `add x, y` and `add(8, 23)` keep working, `--round=up` changes a whole program, and `add(8, 23, 2)` pins one instance (`suite/round.ssa`). Platforms only claim the nearest-even instances.

`probe testfloat` runs Berkeley TestFloat's vectors through the library and the hardware: 19.4 million cases per mode across f16/f32/f64 and every operation, and every one of the five modes comes back 0 wrong. `tools/get-testfloat.sh` builds the generator.

---

### the fuzzer — `2978f8c` · 2026-08-26

`probe fuzz [count] [--seed=hex] [--slow]`. Programs are random but well-formed by construction — every integer width, packs, floats via the library and the platform, value-yielding `if`s, bounded loops, calls between functions — and built so they can't fail for a boring reason: divisors are `or`ed with 1, shift amounts are literals under the width, floats come from integers so no NaN payload reaches the hardware. Native `-O0` with the platform off is the reference; every optimization level, the platform, wasm, and (`--slow`) both qemu machines are referees. Disagreements are kept as suite files under `target/fuzz/`, and a printed seed reproduces its program alone.

First catch, within 300 programs: wasm's `div_s` traps on `MIN / -1` where the IR says wrap. The wasm emitter now guards it arithmetically (divide by `rhs + 2m`, `m = rhs == -1`, then conditionally negate).

---

### rational, scalar, and literals everywhere — `55a2f74` · 2026-08-26

`lib/rational.ssa`: `numerator / denominator`, reduced, a zero denominator being "not a rational"; exact while it fits, and `conv` from a float by continued fractions (`3.14159f32` is `22/7` at 8 bits). Then `scalar` — a bare name the policy points at one of `float`, `fixed`, `rational`, `unit`, `sunit` — and one program that runs unchanged in all five. What made that work: a literal on any library number type is read as an `i64` or `f64` and handed to that library's own `conv`, so `mul x, 0.5` and `sub 1, x` mean the same thing in every family without the compiler knowing any of them. 448/448 on all four paths, both ways.

### unit and sunit — `4ac4a9e` · 2026-08-26

`lib/unit.ssa`: `unit(N)` runs 0.0 to 1.0 over 0 to 2^N−1, `sunit(N)` −1.0 to 1.0 over ±(2^(N−1)−1). The scale is not a power of two, so a product is `(a·b + half) / max`, rounded; sums saturate; `conv` goes through floats. The two-word helpers move to `lib/wide.ssa`, gaining a `udiv128` that fixed and unit share. Bare `unit`/`sunit` follow the policy (`--unit=N`, `--sunit=N`). Exhaustive against their models at 8 bits; 413/413 on all four paths, both ways.

### fixed point — `633d681` · 2026-08-26

`lib/fixed.ssa`: `fixed(I, F)` as `pack { frac: u(F), int: i(I) }` with the arithmetic, comparisons, and conversions to and from integers and floats, all in integer instructions; and a bare `fixed` resolved by the policy (half the `int` width each side, `--fixed=I,F`) exactly as `float` is. Two libraries sharing `add`, `mul`, ... meant two parser adjustments: declarations may name types the prelude declares later, and by-name instantiation must be unambiguous (the float suite's aliases are wrappers now). 367/367 on all four paths, both ways.

### The abstract float, and a prelude — `32511ac` · 2026-08-26

`float` joins `int`: a bare `float` is `float(E, M)` for the policy's width — f64 on the register machines, f32 on wasm, `--float=f16|bf16| f32|f64|E,M` to choose — instantiated by the parser, since `float(E, M)` is the library's type, not the compiler's. `fn half(x: float) -> float` is written once and lands on the library or the platform's instruction at whatever width the policy picks. The float library is now `lib/float.ssa`, appended to every program as a prelude. `suite/afloat.ssa` runs the same programs at four widths; 337/337 on all four paths, both ways.

### min, max, fma — `da83428` · 2026-08-26

`min`/`max` (IEEE minimum/maximum: NaN propagates, -0 below +0) and `fma` with a single rounding: the exact product and the addend meet in a two-word accumulator with eight guard bits, built from a few `add128`/`shr128`-style helpers written in the SSA itself. On the platforms: `fmin`/`fmax`/`fmadd` (arm64), `fmadd` (riscv64, whose `fmin` drops NaNs and so stays in the library), `f32.min`/`max` (wasm, which has no fma). Bit-exact against `mul_add` and an exact reference. 327/327 on all four paths, both ways.

### The suite, with the sugar — `c1d472d` · 2026-08-26

The suite and the float library rewritten with literal operands: 139 named constants gone, 374 lines shorter, `cmp.ne ma, 0` and `pack 0, 0, sz` where there were `zero_m` and `zero_e` declarations. A mechanical pass, so the IR underneath is byte-for-byte the same and the matrix is unchanged: 302/302 on all four paths, both ways.

### const by type, literals as operands — `9b05c3e` · 2026-08-26

`iconst` is `const`, and the type decides: bits for an integer or a pack, a number for a float — `x: f32 = const 0.1` is the nearest f32, exactly rounded (decimal to binary by a small bignum, checked against Rust's `from_str`); `-inf` and `nan` too. And a literal can stand in for a value wherever the context fixes its type: `add a, 1`, `cmp.lt 0, b`, `mul x, 0.5`, `jmp loop(0, 0)`, `ret 0`, `g(b, 2)`, with `200: u8` when nothing does. Hidden consts carry them; the printer shows them inline again. 302/302 on all four paths.

### cmp on floats, neg, abs — `aa1829b` · 2026-08-25

`icmp` is `cmp`, and on floats `cmp.lt` is the library's `lt(E, M)`: six predicates over one `fcmp` that orders by sign and magnitude bits, with IEEE's rules (-0 equals +0; a NaN makes everything false but `ne`). `neg` and `abs` touch the sign field. The platforms have `fcmp`+`cset`, `feq`/`flt`/`fle`, `f32.lt`, `fneg`, `fabs`. Two 63-bit overflow bugs surfaced and were fixed. 302/302 on all four paths, both ways.

### conv and cast — `8eecfc7`, `0f2850f` (docs `424222c`, `cd4897e`) · 2026-08-25

Two opcodes for what used to be three: `conv` carries the value across (ext and trunc are gone — the widths always said which way), `cast` keeps the bits (was bitcast). `1.0 conv u32` is 1; `cast` is 0x3f800000. Between a float and anything, `conv` is the library's: float(E, M) to float(F, N), i(W)/u(W) to float(E, M), float(E, M) to i(W)/u(W) (truncating, saturating, NaN to 0) — five generics sharing one name, chosen by the types on both sides now that dispatch matches the result type too and generics may overload. The platforms map every f32/f64/i32/u32/i64/u64 pair to `fcvt`/`scvtf`/`fcvtzs` and friends (riscv64 keeps float to int in the library over its NaN rule). Checked against Rust's `as` and the exact reference; 274/274 on all four paths, both ways.

### sqrt, and operations a library invents — `b795912` · 2026-08-25

`r: f32 = sqrt a`. Dispatch is now open-ended: any name applied to a pack finds the generic of that name that takes the pack's origin type, at whatever arity it declares, so `sqrt` exists for floats without the integer language knowing the word. The library's `sqrt(E, M)` is a digit-by-digit root that never needs more than M + 8 bits; the platforms map f32/f64 to `fsqrt`. Checked against the FPU and an exact reference (fp8 exhaustively). 235/235 on all four paths, both ways.

### `call` retires — `3cc7b8e` · 2026-08-25

`r: f32 = fadd32(a, b)`, `touch(q)`, `q: i64, r: i64 = divmod(a, b)`. A name followed by `(` in operation position is a call — no opcode is ever followed by one, `const (expr)` and `loop(...)` aside — so the keyword said nothing. It is now rejected with a note that it is implied. Explicit instantiations read the same way: `add(8, 23)(x, y)`. 218/218 on all four paths, both ways.

### Float sub, mul, div — `da2c139` · 2026-08-25

The library grows `sub` (add of the negation), `mul`, and `div` over `float(E, M)`, sharing `fnorm` and `fpack` (subnormals, round to nearest even, overflow). `mul` builds f64's 106-bit product from 27-bit halves without ever holding it; `div` is a restoring long division. The three platforms gain `fsub`/`fmul`/`fdiv` for f32 and f64, so on those widths the opcodes are instructions and on fp8/fp16/bf16 they are the library. Bit-exact against the FPU on f32/f64 and an exact reference exhaustively on fp8, all four ops. 218/218 on all four paths, both ways.

### Native f32, emulated f16, one module — `a71f9a7` · 2026-08-25

A test that shows the platform choosing per width on arm64: `add` on two `f32` values compiles to `fmov`, `fmov`, `fadd s`, `fmov` with no call, while `add` on two `f16` values in the same module compiles to a `bl` into the library's `fadd16` (1632 bytes of integer code) with no `fadd`. The test inspects the machine code of both functions for the learned encodings and checks results against the FPU and the f16 reference. Moving f16 to hardware would be one line in `src/platform.rs`.

### One `add` — `e01e057` · 2026-08-25

`iadd`/`isub`/`imul` are `add`/`sub`/`mul`, and the opcode says nothing about the type: on integers it is the instruction, on a pack that came from a generic type it dispatches to the generic function of the same name taking that type. `add x, y` on two `f32` values is `add(8, 23)(x, y)` — the softfloat library — and on a platform with hardware for that width, the `fadd` instruction. The opcode set never grows; libraries add meanings, platforms add instructions. 191/191 on all four paths, both ways.

### Platforms — `74b903d` · 2026-08-25

A platform is the list of library instantiations a target has hardware for — `fadd(8, 23)` and `fadd(11, 52)` on all three. Each instantiated function now knows its (generic, args) identity, and a backend compiling one of these, or a call to one, emits the instruction sequence instead of the SSA body: `fadd32` on arm64 is `fmov`, `fmov`, `fadd`, `fmov`, `ret`. The library body stays the definition of the semantics; `--soft` compiles with an empty platform, and the two are checked against each other and the FPU. Newly probed: FP registers and adds on every target, plus the CSR/system-register writes that switch the FPU on bare metal. 188/188 on all four paths, both ways.

### Generic functions, and floats as a library — `039de73` · 2026-08-25

`fn fadd(E, M)(a: float(E, M), b: float(E, M)) -> float(E, M)` is a template; `fn fadd32 = fadd(8, 23)` and `fadd(5, 10)(x, y)` instantiate it, by re-parsing the body with E and M bound, so `u(M + 5)` and `const (1 << E) - 1` are concrete inside. With that, `suite/float.ssa` writes IEEE addition once — round-to-nearest-even, subnormals, signed zeros, infinities, canonical NaN — using only integer instructions, and instantiates it for fp8, fp16, bf16, f32, f64. The compiler learned nothing about floats. It matches the FPU bit-for-bit on f32/f64 over ~140k pairs and an independent reference exhaustively on fp8; 188/188 on all four paths.

### Parametric types — `7b9175a` · 2026-08-25

`type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }`, then `type f32 = float(8, 23)` and `type f16 = float(5, 10)`: a `type` declaration takes integer parameters that stand for widths, and its body is a pack, an `i(expr)`/`u(expr)` with `+ - *` over the parameters, a builtin, or another declared type applied to arguments. Instantiation happens at use (`x: float(8, 23)`) or at an alias; packs are interned structurally so every spelling of a layout is one type. Functions stay monomorphic. `suite/types.ssa` pulls pi's exponent out of an f32 and doubles it by incrementing the field; 174/174 on all four paths.

### The sigils retire — `2467869` · 2026-08-25

`%v`, `^b`, `@f`, `$t` become `v`, `b`, `f`, `t`. Position already said which was which — before `:` a value is defined, after `:` a type is named, after `call` a function, after `jmp`/`br` a block, and a label is a name opening a line and followed by `:` — so the lexer now has one word token and the parser's prescans apply that rule. `fn sum(n: i64)`, `done: u1 = cmp.ge i, n`, `br done, exit, body`. Old prefixes are rejected with a message. Suite, examples, tests, harness, and docs converted; 162/162 on all four paths.

### Narrow shifts just shift — `a987671` · 2026-08-25

Shifting an `i5` by 5 or more no longer takes the amount mod 5 (which cost a `ubfm`, or a `udiv`/`msub` for non-power-of-two widths): the backends emit the container's shift and re-normalize, and amounts at or past the width are unspecified — buyer beware, like any overflow. The const-folder leaves those shifts alone and the exhaustive arm64 test skips them. `i32`/`i64` keep the hardware's mod-32/64.

### Any-width integers and packs — `542b9a5` · 2026-08-25

Types are now `iN`/`uN` for any N from 1 to 64, and signedness lives in the type: one `div`, one `rem`, one `shr`, one `cmp.lt`, `ext` fills by the source's signedness, `bitcast` reinterprets. `u1` is the boolean. `pack rgb { r: u5, g: u6, b: u5 }` packs bitfields lowest-bits-first into ≤64 bits — nestable, storable at 8/16/32/64 bits — with `pack`, `unpack`, `get`, `set`. Every backend keeps values *canonical* in their container (sign- or zero-extended) and re-normalizes after ops that can carry out, using freshly probed `sbfm`/`ubfm`/`bfm`, byte/halfword loads, and wasm's narrow loads. 162/162 on all four paths; an exhaustive JIT-vs-model test covers every op on eighteen widths.

### Abstract `int` — `1f795ad` + `acd4764` · 2026-08-23

SSA can now say `int` instead of committing to `i32` or `i64`. A resolution pass swaps it for a concrete width before verification, using a *replacement policy* per target (i64 on arm64/riscv64, i32 on wasm32) or `--int=i32|i64`. Because types sit on variables, not opcodes, that pass is one sweep over the value tables — no instruction changes. The verifier rejects any `int` that survives, so nothing downstream ever meets one. `suite/abstract.ssa` is written to be policy-independent and the suite runs under both widths. 96/96 everywhere.

### Incremental JIT arena — `31bd115` · 2026-08-23

All compiled functions live in one `MAP_JIT` arena, each in a slot with 50% slack. Every call goes through a fixed per-function trampoline that counts invocations and branches to the current address, so a changed definition recompiles in place (or relocates to the tail if it grew) and no call site is ever patched. `probe live <file> <fn> [args]` runs the loop: edits recompile only the changed function at level 0, and a function crossing 10k calls is promoted through the full pass pipeline mid-run. `src/arena.rs`.

### SSA pass pipeline with levels — `136d065` · 2026-08-23

`src/opt.rs` becomes the single optimization engine: an ordered list of SSA→SSA passes where a level is a prefix, so every stopping point is valid — the foundation for gradual optimization. New passes: simplify-cfg (threads branches through empty forwarding blocks, drops unreachable ones), const-fold (typed wrapping arithmetic; leaves divide-by-constant-zero alone since wasm traps), and dce (pure unused instructions; divisions only with provably nonzero divisors). `-O<n>` on any command; `probe tiers` shows size and time at every level; the suite runs at every level as a test.

### Register allocation, in three steps — `d863c3d` → `6a90a80` · 2026-08-23

*Linear scan* (`d863c3d`, `src/regalloc.rs`): liveness by backward fixpoint, single-span intervals, furthest-end eviction, over a callee-saved pool only — so values survive calls by construction and prologues save exactly what a function uses. 3.8× on a hot sum loop. *Sink scheduling and parallel moves* (`eaf6780`): producers move toward consumers before allocation, shrinking intervals; branch arguments become true parallel moves (cycles break through one scratch register), lifting the 8-argument cap; arm64 saves pair into `stp`/`ldp`. *Coalescing* (`6a90a80`): precise per-point interference lets block parameters union-find with their branch-argument sources, so the move on a loop back edge disappears — sum's loop body is `cmp`/`cset`/`cbz` plus two in-place adds, 96 bytes down from 156.

### Foundation — `cd7ff7a` · 2026-08-23

Everything at once: the SSA IR (block parameters, multi-value returns, structured `if`/`loop` sugar lowered at parse time); two encoding learners — bit-scatter for fixed-width ISAs, byte+LEB128 for wasm — that probe `llvm-mc`/`wat2wasm` and verify every hypothesis against the oracle; and three emitters (arm64, riscv64, wasm32) containing no hand-written opcodes. One 86-case suite runs against all of them: native JIT, node, and bare-metal qemu for riscv64 and aarch64, with the runtime harness generated in the project's own SSA.
