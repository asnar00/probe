# Memory management: where the field is, and what it means here

A bring-up-to-speed, written for probe's memory design session. Sources
are at the end; the claims in the body come from them. The last section
is the part that is opinion.

## 1. The map

Every memory-management scheme answers two questions: **who decides when
a piece of memory is done** (the programmer, the type system, or the
runtime), and **at what granularity** (an object, a region of objects
with a shared lifetime, or everything at once). The classical menu —
manual `malloc`/`free`, reference counting, tracing garbage collection —
sits in one corner: per-object, decided by the programmer or the
runtime. Most of what has happened since 1990 is the field discovering
that the other corners are better for most programs:

- **Regions / arenas**: lifetime decided per *group*, freed all at once.
  Theory from 1994 (Tofte–Talpin), practice from 1967 (zoning) and 1990
  (Hanson), mainstream in games since forever and in systems languages
  since Zig/Odin/Jai.
- **Ownership / linearity**: lifetime decided by the *type system*, per
  object, with the runtime doing nothing. Wadler 1990, Clean, Cyclone,
  Rust, Austral; and the reference-counting revival (Koka's Perceus,
  Lean 4, Swift's ownership) where the type system removes almost all of
  the counting.
- **Static allocation**: lifetime decided *at build time*, the whole
  program's memory a fixed image. The safety-critical rule (Holzmann,
  JPL, MISRA), and the design principle of the most interesting recent
  kernels (Hubris, seL4, Tock).
- **Handles**: an object's identity separated from its address — an index
  plus a generation — so that lifetime can be checked cheaply at use and
  layout is free to move. Game engines and ECS, Vale, Zig's compiler.

And two things that cut across all of it: **hardware** (CHERI puts
bounds in the pointer; MTE tags memory) and **static bounds** (proving
how much memory a program needs, the way WCET proves how long it takes).

Bacon, Cheng and Rajan's *unified theory* (2004) is the right frame for
the classical corner: tracing computes the live objects from the roots,
reference counting computes the dead ones from the decrements, they are
duals, and every real collector is a hybrid. The rest of this document
is about leaving that corner.

## 2. The classical corner, in 2025

For completeness — this is what a general-purpose runtime does, and why
it is not what a bare-metal single-user kernel wants.

**Tracing GC** is region-based and concurrent now: ZGC (generational
since JDK 21) does marking and relocation concurrently with sub-
millisecond pauses, paying for it with *colored pointers* and a load
barrier on every reference read; Shenandoah is the same shape. Immix
(2008) is the substrate underneath most research collectors: allocate
by bumping through free *lines* of 32 KB blocks, reclaim at line
granularity, copy only opportunistically. The 2022 result worth knowing
is **LXR** (Zhao, Blackburn, McKinley): reference counting for the
common case on an Immix heap, tracing only as a backup for cycles, and
*brief regular stop-the-world pauses* instead of concurrent copying —
6× the throughput and 30× lower tail latency than Shenandoah on a
large-heap workload. The lesson generalises: the cost of concurrent
collectors is the barriers and the headroom, and regular short pauses
are often cheaper than avoiding pauses.

**Allocators**: mimalloc (a free list *per page* rather than per size
class, so the fast path has bump-allocator locality; no locks), snmalloc
(cross-thread frees are *messages* to the owning allocator), TCMalloc
(per-CPU caches via restartable sequences), jemalloc (arenas, size
classes spaced to bound internal fragmentation at 20%). All fast, all
with worst cases in fragmentation and abandoned caches, none with a
bound you could put in a certification argument.

**The general-purpose heap's problems are the ones the rest of this
document avoids**: unbounded worst-case time, fragmentation that is
emergent rather than designed, and the fact that "how much memory does
this program need" has no static answer.

## 3. Regions and arenas

### The theory, and what went wrong

Tofte and Talpin (1994/1997) put every value in a region, made the
store a *stack* of regions (`letregion ρ in e end` pushes and pops), and
inferred the placement with a type-and-effect system. Region
polymorphism — functions abstracted over regions, with polymorphic
recursion — was essential: without it every recursive call's
intermediates land in the caller's region. Soundness, an inference
algorithm and principal types were all proved. The MLKit implemented it.

The 2004 retrospective by the same authors is unusually honest and is
the single most useful document on this list. Early results were
"terrible". Even with polymorphic recursion, iterative loops leak,
because both branches of a conditional are forced into one region and
an accumulator's region grows with every iteration. Results "varied from
the excellent to the poor": excellent for programs whose data lifetimes
nest like the call stack, bad for anything else — servers, event loops,
long-lived structures updated in place. The list of practical problems:
inference favoured a programming discipline nobody could articulate;
annotated programs were huge; small edits changed every annotation;
leaks could only be found by reading the whole program.

The fixes are each a lesson in themselves. *Storage-mode analysis*
resets a region in place (`atbot`) when nothing in it is live, which is
what makes tail-recursive loops constant-space. *Multiplicity inference*
found that regions receive either one value or unboundedly many, so the
former become stack slots — over 90% of allocations. A *region profiler*
was "a breakthrough", the only practical way to find leaks. And finally
regions were *combined with a garbage collector* (2002): regions reclaim
most memory (75–100% in most benchmarks) and the collector catches what
the discipline cannot. Aiken, Fähndrich and Levien (1995) showed that
dropping the stack discipline — freeing a region as early as a constraint
solver allows — is asymptotically better in cases, at the price of the
very property that made regions analyzable.

### Cyclone: the lesson that generates everything

Cyclone (2002–2004) made regions explicit in a safe C: every pointer
type carries its region, functions are region-polymorphic with effects,
and an *outlives* relation gives subtyping (a pointer into a longer-lived
region may stand in for one into a shorter). Three kinds: stack, lexical
arenas, and a GC'd heap. Stack allocation was crucial and mostly
inferred. Then the experience report: LIFO arenas "are not suited to
computations such as server and event loops". The remedy was **unique
pointers** — affine, consumed by `swap`, freeable at any point, checked
by a flow analysis — and from lexical regions plus unique pointers they
derived the rest of the menu: dynamic regions with non-nested lifetimes
(a unique pointer is the key that opens one), reference-counted regions,
and a scoped `alias` (borrowing) that relaxes uniqueness. Results:
throughput indistinguishable from a GC, footprint tiny (a 6.5 KB working
set where the Boehm collector reserved 635 KB). Reference counting was
the part that went badly: manual increments, leaks on failure paths.
Cyclone is the direct ancestor of Rust's lifetimes.

Gay and Aiken's RC (2001) is the dynamic alternative: count the pointers
*into* a region from outside it, refuse to delete a region that has any,
and annotate the three common cases (same-region, legacy, parent) to
avoid the counting — 39–99.98% of pointer assignments covered.

### Practice: arenas as everyone actually uses them

Hanson (1990): group objects by lifetime, bump-allocate within large
chunks, free the whole list at once; half the cost per byte of a
free-list allocator. The game-engine vocabulary (Gregory, *Game Engine
Architecture*): **stack allocator** with `getMarker`/`freeToMarker`;
**single-frame allocator** cleared at the top of each frame; **double-
buffered allocator**, two stacks swapped per frame so that frame N's
results (asynchronous job output) are readable during frame N+1 while
the other buffer is cleared. That is exactly the frame allocator from
GPU-engine practice, and it has been rediscovered by everyone since.

The modern write-ups — Fleury's "Untangling Lifetimes" (2022) and
Wellons' "Arena allocator tips and tricks" (2023) — turn it into a
discipline: most lifetime complexity is self-inflicted ("lifetime soup",
"religious freeing"); an arena is `{beg, end}`; reserve a huge virtual
range and commit on demand so arenas grow without moving; pass the
*scratch* arena by value so the callee's bumps vanish on return, and the
*result* arena by pointer; resolve the conflict where the callee's
scratch is the caller's result arena by handing out a scratch that is
not the one the caller returns through. Pools compose with arenas for
fixed-size objects of arbitrary lifetime. Zig's `ArenaAllocator` wraps
any allocator and offers `reset(retain_capacity)` for per-cycle reuse;
Odin's `context.temp_allocator` and Jai's temporary storage are the
same thing baked into the runtime and cleared once per frame. Rust's
`bumpalo` and `typed-arena` do it with a lifetime parameter, which makes
cyclic graphs safe and destructors awkward.

### Recent research

Verona (Microsoft, 2023): the heap is a forest of isolated regions,
capabilities `iso`/`mut`/`imm` ensure no reference crosses into a region
except through its single `iso` entry, a thread mutates one region at a
time, and so *each region can choose its own strategy* — arena, RC,
tracing — with costs localised. Austral: linear types plus explicitly
named regions for borrows; a checker under a thousand lines. Go tried
arenas as an experiment (1.20) and shelved them because they "compose
poorly" — every API needs an arena parameter, an arena value can never be
stack-allocated; its successor proposal, "memory regions", is implicit
(bump within a function's dynamic extent, with a write barrier that
"fades" escaping objects into the GC heap).

### Space safety

Regions make memory *analyzable* because deallocation points are
syntactic. MLKit's multiplicity inference is a static per-region bound.
Garbervetsky, Braberman and Yovine (2008) synthesize each region's size
as a closed-form polynomial in the method's arguments by counting
allocation-site iterations. Regehr, Reid and Webb (2003) bound stack
depth by abstract interpretation of machine code. Hofmann and Jost, and
the whole *automatic amortized resource analysis* line (Hoffmann; survey
2022, still very active through 2026), bound heap use by typing values
with "potential" and solving linear constraints. Cyclone's web server is
the practical face: per-request arenas make the footprint literally
concurrency × per-request size.

## 4. Ownership, linearity, and the reference-counting revival

Wadler (1990): a linear value is used exactly once, so it needs no
counting or collection and admits destructive update; `let!` is an early
borrow. Clean's uniqueness types made this practical for arrays and I/O
with inference. Austral (2022) is the modern pure form: linear types,
explicit destructors, no GC, second-class borrows that cannot escape
their region, no exceptions (unwinding cannot consume linear values) —
a checker whose rules "fit on a page".

Rust is the mainstream form: one owner, aliasing xor mutation, no
reference outlives its referent, deterministic drops. The costs are well
documented (a 2023 usability study; self-referential structs; the arena
patterns people reach for to escape the checker), and the frontier is
Polonius, a location-sensitive reformulation that accepts the cases NLL
rejects, on nightly since 2025 and still too slow. In embedded Rust the
answer to "no heap" is `heapless` — fixed-capacity collections whose
capacity is a type parameter and whose `push` returns a `Result`.

**The reference-counting revival is the important 2020s development.**
Perceus (Koka, 2021) inserts `dup`/`drop` precisely so memory is freed
at *last use*, then pairs a drop of a cell with an allocation of the same
size in the same branch and reuses it in place — "functional but in
place": a red-black-tree insert on a uniquely referenced tree mutates
like C, and the Koka benchmarks match C++ `std::map`. FP² (2023) makes
the no-allocation guarantee a checked function attribute. Lean 4 does
the same (borrowed-parameter inference, `reset`/`reuse`) and its compiler
"often outperforms ocamlopt and GHC". Swift's ownership manifesto became
`borrowing`/`consuming` and non-copyable types in 5.9. Lobster's
ownership analysis removes ~95% of counts. The shared argument: RC's cost
is the bookkeeping, not the model; when uniqueness is inferred or
declared, most of it vanishes, and you keep prompt deterministic
reclamation and a small peak footprint — the thing tracing GCs pay for
with headroom.

**Handles.** Weissflog's "Handles are the better pointers" (2018):
systems own their objects in dense arrays and hand out small integers —
an index plus a *generation* incremented on destroy — so dangling handles
are detected by comparison, items stay packed, and handles survive
serialization. This is the ECS entity id, the slot map, Vale's
generational references (a pointer that carries the generation it
expects, checked on dereference at 2–10% cost), and Andrew Kelley's Zig
compiler (`u32` indices for every node, structure-of-arrays storage).
Handles are also what you get when the compiler can't do ownership:
lifetime is checked cheaply at use rather than proved at compile time.

**Escape analysis** is the runtime's way of finding regions: HotSpot
scalar-replaces objects that don't escape; Go decides at compile time
what stays on the stack. Both fail at exactly the places regions fail —
control-flow merges and calls it can't see through.

## 5. What safety-critical practice actually does

**The rules.** Holzmann's rule 3 (2006): no dynamic allocation after
initialization, because allocators and collectors are unpredictable,
their error classes (use-after-free, leaks, exhaustion, overruns) are
hard to catch, and with everything sized statically "the analysis for
memory bounds becomes trivial": a task's memory is its image plus its
stack. Rules 1 and 2 (no recursion, bounded loops) are the timing and
stack counterparts; rule 9 (one level of dereference, no function
pointers) exists to keep the call graph static. The point of the whole
set is that every resource is verifiable *by tools*, not by inspection.
JPL's C standard says the same ("no dynamic memory allocation after task
initialization"); MISRA is stricter (no `malloc` at all); DO-178C via
DO-332 does not ban heaps but requires each named vulnerability
(fragmentation starvation, premature deallocation, heap exhaustion, …)
to be argued away — which "allocate at init, never free" does trivially.

**What people build.** Pools — fixed-size blocks, a free list, O(1),
failure an explicit testable event: FreeRTOS `heap_1` (bump, `free`
asserts) through `heap_4` (coalescing first-fit), Zephyr slabs, every
RTOS's "partitions". Where variable sizes are unavoidable, **TLSF**
(2004): two-level segregated lists indexed by bitmaps and find-first-set,
under 200 instructions worst case for `malloc`/`free`, fragmentation as
good as best-fit; the de facto real-time allocator.

**Static analysis.** The stack bound is the longest weighted path
through the call graph; gcc's `-fstack-usage` and Rust's
`-Z emit-stack-sizes` give the frame sizes, `cargo-call-stack` and
AbsInt's StackAnalyzer stitch the graph. Recursion makes the graph cyclic
(no bound without an external depth); indirect calls make the edge set
unknown, so tools over-approximate by *type* — every function of the
call's signature is a possible target. WCET tools (aiT) enter memory
through cache analysis: accesses the analysis cannot classify are
assumed misses, so layout is part of timing.

**Kernels without a heap.** Hubris (Oxide): every task declared in a
build-time manifest, regions assigned by the build, no task creation, no
kernel allocation — the stated reason is that dynamic resource creation
is a source of "hard-to-account-for" usage, and a static layout gives
compile-time peak-memory verification and a kernel that stays "out of
the resource allocation business". Tock: drivers may not allocate; per-
process driver state lives in a *grant* region inside the process's own
memory, so the process pays for it and it vanishes with the process.
seL4: the kernel never allocates after boot; all memory is handed out as
*untyped* capabilities that user level *retypes* into kernel objects,
which is what made the kernel verifiable. Common shape: kernel objects
are either fixed at build time or carved from memory the caller supplies
and is accountable for; failure is "region exhausted" at a known site.

**Real-time GC and RTSJ scopes** are the cautionary tales. Metronome
(2003) delivers bounded pauses by time-based scheduling, given a user-
supplied allocation rate and live size — inputs that are themselves a
WCET-class problem. RTSJ's `ScopedMemory` is a region system in a
mainstream runtime with *dynamic* assignment checks (a reference from an
outer scope to an inner one throws at run time); the experience reports
call scopes "quite challenging", the checks cost throughput, standard
libraries could not be used inside them, and RTSJ 2.0 made them
optional. The lesson: a region system without ownership *types* pushes
the discipline onto the programmer and the checking onto the runtime.

**Audio and control loops** (Bencina, "time waits for nothing"): in the
callback nothing may block or have an unbounded worst case — no locks,
no allocation, no I/O, no page faults; preallocate everything;
communicate through lock-free single-producer/single-consumer rings; send
"please free this" back to the non-real-time thread. Double buffering is
the two-slot ring. With allocation out of the periodic path, a period's
memory is a static sum and its latency is buffer depth × period.

## 6. Hardware, verification, layout

**CHERI**: a pointer is a 128-bit capability carrying bounds and
permissions, checked on every access, derivable only by narrowing; Arm's
Morello ships it. Spatial safety becomes a property of the pointer, not
of the type system. Temporal safety still needs *revocation* —
Cornucopia quarantines freed memory and sweeps for capabilities into it
(concurrently, with a load barrier, in the 2024 version) — which pushes
CHERI allocators toward GC-like structure. CHERIoT brings it to
microcontrollers with hardware-assisted revocation at real-time cost.
**MTE** is the cheap probabilistic version (4-bit tags per 16 bytes).
The idea to keep: *a pointer that knows its bounds*.

**Verified allocators** exist: a verified mimalloc in Verus (SOSP 2024),
StarMalloc (a verified hardened allocator that runs Firefox), seL4's and
CertiKOS's kernel allocators. Allocation is now a thing one proves.

**Layout is memory management.** Acton's doctrine and Kelley's
"practical DOD": structure-of-arrays so a pass touches only the fields
it reads, indices instead of pointers, variants encoded in a tag array.
ECS storage (archetype tables vs sparse sets) is the point where the
allocator's job — bump into columns, swap-remove, generation-counted ids
— is inseparable from the data structure. Lifetime and locality are
decided by the same thing.

## 7. What this means for probe

Where probe stands: `data` (machine lifetime), `scratch` (call
lifetime), no heap, raw `ptr`, fn values with a closed target set, the
Power of 10 as the chosen spirit. Against the field above, this is not a
gap to be filled with a heap; it is the static-allocation corner, which
is where the systems worth imitating (Hubris, seL4, audio engines) sit
on purpose. What follows is the synthesis.

1. **The lifetime ladder is the design.** Call (`scratch`) ⊂ frame or
   period (an arena reset at a scheduler boundary, double-buffered for
   producer/consumer across periods) ⊂ object (a pool slot, released
   explicitly) ⊂ machine (`data`). Every one is a bump or free-list
   allocator over `data`; every one has a declared capacity; the only
   failure is exhaustion at a known site. This is Hanson, Gregory,
   Fleury and Wellons in probe's terms, and it is also rule 3 repeated
   per period. Nothing here needs the compiler.

2. **Regions belong in the pointer type, checked statically.** Cyclone's
   *outlives* subtyping is the model: `ptr` for machine lifetime, `ptr
   call`, `ptr frame`; a pointer may be stored only through a pointer of
   the same or a shorter region, and never returned or stored past its
   region's end. Types live on variables in probe, so this is in its
   grain, and it is the one IR change memory has asked for. RTSJ is the
   warning about doing this dynamically; Austral's tiny checker is the
   encouragement that the static rule can be simple. The MLKit's
   failures do not apply: probe would not *infer* regions, only check
   declared ones, and the frame arena's reset is a scheduler event, not
   a lexical scope — which is precisely the non-nested lifetime regions
   could not handle and a period boundary handles for free.

3. **Unique pointers are the second primitive.** Cyclone's finding is
   that lexical regions plus unique pointers generate the whole menu —
   dynamic regions, pools, reference counting. A `ptr` that is affine
   (consumed on store or on release to its pool) is what makes a pool
   slot's explicit release safe. This is the ownership corner reduced to
   the minimum a single-user kernel needs; Perceus-style reuse is the
   direction it grows in if the language ever wants values that are
   functional but in place.

4. **Handles for everything with an arbitrary lifetime.** Tasks, timers,
   pool objects: an index plus a generation in a `data` table, checked at
   use. This is what the fn-value tables already are, it survives the
   "no function pointers" rule in spirit (the target set is closed), and
   it is what game engines and Zig's compiler converged on independently.

5. **Capacity is an analysis the compiler should do.** probe sees the
   whole program and every frame size it emits. The stack bound per
   task (longest path over the call graph, indirect edges by type — the
   `cargo-call-stack` approach, which the wasm table already computes)
   and the worst-case bytes bumped from an arena per period (the same
   analysis with allocation sizes, given bounded loops) are one pass.
   Regehr's stack bound and Garbervetsky's region bound, done with
   information the compiler has rather than reconstructed from a binary.
   With it, "8192 bytes per task stack" becomes a proven number. `check`
   (rule 5) is the instruction that makes exhaustion and bounds failures
   land in `__trap` rather than in silence.

6. **What not to build**: a general heap (the whole of §2 and §5 says
   why); inferred regions (§3 says why); a real-time GC (§5); dynamic
   region checks (RTSJ). And keep two things the strict reading of the
   rules would forbid, for the reasons given: recursion outside strict
   mode, and function values with a closed typed target set.

The order that follows from this: arena and pool libraries over `data`
with `check`, used by the OS at the period boundary; then region-typed
pointers as the one deliberate IR addition; then the capacity/stack
analysis; unique pointers when pools need explicit release to be safe;
type generics when pools want to be typed.

## Glossary

- **Arena / region / zone**: memory freed as a unit. *Bump* or *linear*
  allocation within it; a *stack allocator* adds markers to pop back to;
  a *frame allocator* is reset once per period; *double-buffered* keeps
  the previous period's readable.
- **Pool / slab / partition**: fixed-size blocks and a free list; O(1);
  fails only by exhaustion.
- **Outlives**: region R outlives S if every value in S dies before any
  in R; a pointer into R may be used where one into S is expected.
- **Linear / affine / unique**: used exactly once / at most once / has
  one reference; the basis of deterministic deallocation and in-place
  update.
- **Handle / generational index**: index into a table plus a counter
  that changes when the slot is reused.
- **Capability**: a pointer with bounds and permissions the hardware
  checks (CHERI), or an unforgeable token for a resource (seL4).
- **TLSF**: two-level segregated fit; the O(1) real-time allocator.
- **AARA**: automatic amortized resource analysis; static memory/time
  bounds from types with potential.

## Sources

Regions and arenas: Tofte & Talpin, *Region-Based Memory Management*
(I&C 1997) http://ropas.snu.ac.kr/lib/dock/ToTa1997.pdf · Tofte,
Birkedal, Elsman, Hallenberg, *A Retrospective on Region-Based Memory
Management* (HOSC 2004) https://melsman.github.io/mlkit/pdf/retro.pdf ·
Aiken, Fähndrich, Levien, *Better Static Memory Management* (PLDI 1995)
https://theory.stanford.edu/~aiken/publications/papers/pldi95.pdf ·
Hallenberg, Elsman, Tofte, *Combining Region Inference and Garbage
Collection* (PLDI 2002) · Grossman et al., *Region-Based Memory
Management in Cyclone* (PLDI 2002)
https://www.cs.umd.edu/projects/cyclone/papers/cyclone-regions.pdf ·
Hicks et al., *Experience with Safe Manual Memory Management in
Cyclone* (ISMM 2004)
https://homes.cs.washington.edu/~djg/papers/cyclone_ismm04.pdf · Gay &
Aiken, *Language Support for Regions* (PLDI 2001)
https://theory.stanford.edu/~aiken/publications/papers/pldi01.pdf ·
Hanson, *Fast Allocation and Deallocation of Memory Based on Object
Lifetimes* (SP&E 1990) · Fleury, *Untangling Lifetimes: The Arena
Allocator* (2022)
http://www.dgtlgrove.com/p/untangling-lifetimes-the-arena-allocator ·
Wellons, *Arena allocator tips and tricks* (2023)
https://nullprogram.com/blog/2023/09/27/ · Zig `ArenaAllocator`
https://github.com/ziglang/zig/blob/master/lib/std/heap/arena_allocator.zig
· Odin temp allocator
https://zylinski.se/posts/temporary-allocator-your-first-arena/ ·
Goregaokar, *Arenas in Rust* (2021)
https://manishearth.github.io/blog/2021/03/15/arenas-in-rust/ · Verona
regions (OOPSLA 2023) https://arxiv.org/abs/2309.02983 · Go arenas
https://github.com/golang/go/issues/51317 and memory regions
https://github.com/golang/go/discussions/70257 · Garbervetsky et al.,
*Parametric Prediction of Heap Memory Requirements* (ISMM 2008) ·
Regehr, Reid, Webb, *Eliminating Stack Overflow by Abstract
Interpretation* (EMSOFT 2003)
http://web.cs.ucla.edu/~palsberg/course/cs239/S04/papers/RegehrReidWebb03.pdf

Ownership and RC: Wadler, *Linear Types Can Change the World!* (1990)
https://www.cs.cornell.edu/courses/cs6110/2017sp/lectures/lec30.pdf ·
Clean uniqueness typing
https://clean.cs.ru.nl/download/html_report/CleanRep.2.2_11.htm ·
Borretti, *Type Systems for Memory Safety*
https://borretti.me/article/type-systems-memory-safety and Austral
https://austral-lang.org/ · Rust usability study (2023)
https://arxiv.org/pdf/2301.02308 · Haberman, *Arenas and Rust*
https://blog.reverberate.org/2021/12/19/arenas-and-rust.html · Polonius
https://rust-lang.github.io/goals/2025h2/polonius.html · Embedded Rust
collections https://docs.rust-embedded.org/book/collections/index.html ·
Reinking, Xie, de Moura, Leijen, *Perceus* (PLDI 2021)
https://xnning.github.io/papers/perceus.pdf · Lorenzen, Leijen,
Swierstra, *FP²* (ICFP 2023) https://dl.acm.org/doi/10.1145/3607840 ·
Ullrich & de Moura, *Counting Immutable Beans* (IFL 2019)
https://arxiv.org/pdf/1908.05647 · Swift Ownership Manifesto
https://github.com/swiftlang/swift/blob/main/docs/OwnershipManifesto.md ·
Nim ORC https://nim-lang.org/docs/mm.html · Lobster
https://aardappel.github.io/lobster/memory_management.html · Vale
generational references
https://verdagon.dev/blog/generational-references · Weissflog, *Handles
are the better pointers*
https://floooh.github.io/2018/06/17/handles-vs-pointers.html · Hoffmann
et al., AARA https://www.cs.cmu.edu/~janh/publications/

Safety-critical practice: Holzmann, *The Power of 10* (2006)
https://spinroot.com/gerard/pdf/P10.pdf · JPL C standard
https://yurichev.com/mirrors/C/JPL_Coding_Standard_C.pdf · AdaCore on
DO-332 and dynamic memory
https://www.adacore.com/uploads/papers/DynamicMemoryManagement.pdf ·
FreeRTOS heaps
https://github.com/FreeRTOS/FreeRTOS-Kernel/tree/main/portable/MemMang ·
Zephyr slabs
https://docs.zephyrproject.org/latest/kernel/memory_management/slabs.html
· TLSF https://github.com/mattconte/tlsf · gcc `-fstack-usage`
https://gcc.gnu.org/onlinedocs/gcc/Developer-Options.html ·
cargo-call-stack https://github.com/japaric/cargo-call-stack · AbsInt
aiT https://www.absint.com/ait/analysis.htm · Hubris
https://hubris.oxide.computer/reference/ and Cantrill's essay
https://cliffle.com/blog/on-hubris-and-humility/ · Tock grants
https://www.tockos.org/documentation/design/ · seL4 untyped memory
https://docs.sel4.systems/Tutorials/untyped.html · Bacon, Cheng, Rajan,
*Metronome* (POPL 2003) https://dl.acm.org/doi/10.1145/604131.604155 ·
Pizlo & Vitek on RTSJ scopes https://janvitek.org/pubs/isorc08.pdf ·
Bencina, *Real-time audio programming 101: time waits for nothing*
http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing

GC, allocators, hardware: Bacon, Cheng, Rajan, *A Unified Theory of
Garbage Collection* (OOPSLA 2004)
https://dl.acm.org/doi/10.1145/1028976.1028982 · Jones, Hosking, Moss,
*The Garbage Collection Handbook* (2nd ed. 2023) https://gchandbook.org/
· JEP 439 Generational ZGC https://openjdk.org/jeps/439 · Blackburn &
McKinley, *Immix* (PLDI 2008)
https://users.cecs.anu.edu.au/~steveb/pubs/papers/immix-pldi-2008.pdf ·
Zhao, Blackburn, McKinley, *LXR* (PLDI 2022)
https://arxiv.org/abs/2210.17175 · MMTk https://www.mmtk.io/ · mimalloc
https://www.microsoft.com/en-us/research/uploads/prod/2019/06/mimalloc-tr-v1.pdf
· snmalloc https://github.com/microsoft/snmalloc · TCMalloc rseq
https://google.github.io/tcmalloc/rseq.html · CHERI/Morello
https://www.cl.cam.ac.uk/research/security/ctsrd/cheri/cheri-morello.html
· Cornucopia (S&P 2020)
https://www.cl.cam.ac.uk/research/security/ctsrd/pdfs/2020oakland-cornucopia.pdf
· CHERIoT (MICRO 2023) https://dl.acm.org/doi/10.1145/3613424.3614266 ·
Arm MTE https://developer.android.com/ndk/guides/arm-mte · Zig
Allocator https://github.com/ziglang/zig/blob/master/lib/std/mem/Allocator.zig
· Fil-C https://fil-c.org/invisicaps · Verus verified mimalloc
https://github.com/verus-lang/verified-memory-allocator · StarMalloc
https://arxiv.org/abs/2403.09435 · Acton, *Data-Oriented Design and C++*
(CppCon 2014) · Kelley, *Practical Data-Oriented Design* (2021)
https://vimeo.com/649009599
