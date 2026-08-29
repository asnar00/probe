# probe SSA — v0.1

The input language of the lowest compiler stage. A module of functions; each function is a graph of basic blocks in SSA form. Deliberately small: just enough to express straight-line integer code, control flow, memory, and calls — the things we can learn ARM64 encodings for by probing LLVM.

## Design choices

- **Block parameters, not phi nodes.** A value flowing into a join point is passed as an argument on the branch, and received as a parameter on the target block (the Cranelift / MLIR style). This keeps "where does this value come from" local to the branch instruction and makes the emitter simpler.
- **Types live on variables, not opcodes.** Every value definition is written `name: ty = op ...` — the same form as function and block parameters. Opcodes are pure operations with no type suffixes, which keeps the opcode set small; the verifier checks that operand and result types are consistent. Signedness is part of the type too: there is one `div`, one `shr`, one `cmp.lt`, and `i5` versus `u5` says which one you mean. And there is one `add`: on an integer it is the integer instruction, on a `float(8, 23)` it is whatever the library's `add(E, M)` says — or the platform's `fadd`.
- **No nesting, no expressions.** One instruction per line, every intermediate value named. This is the layer *below* everything clever.
- **No sigils.** `sum`, `entry`, `n`, `f32` are all plain words; the grammar never needs a prefix to tell them apart, so there are none.

## Lexical

- Comments: `;` to end of line.
- Names: `[A-Za-z0-9_]+`, for values, blocks, functions, and types alike. No prefixes — position says which is which: a name before `:` defines a value, after `:` names a type, before `(` names a function being called, after `jmp`/`br` names a block. Each value is defined exactly once.
- Integer literals: decimal, optionally negative (`42`, `-7`), or hex (`0x2a`). Float literals: a decimal with a fraction or an exponent (`1.5`, `2e10`, `-1.0e-3`), plus `inf`, `-inf`, `nan`.
- Whitespace is insignificant except as a separator; newlines end instructions.

## Types

| type    | meaning                                                    |
|---------|------------------------------------------------------------|
| `iN`    | signed integer of N bits, 1 ≤ N ≤ 256 (`i1`, `i5`, `i32`, `i64`, `i128`) |
| `uN`    | unsigned integer of N bits (`u1` is the boolean, `u23`, `u64`) |
| `ptr`   | pointer (64-bit natively; a 32-bit offset on wasm)         |
| `name`, `name(8, 23)` | a declared type, plain or instantiated with widths (see *Type declarations*) |
| `pack { ... }` | bitfields packed into at most 256 bits (see *Packs*) |
| `struct { ... }` | fields side by side, never a bit pattern (see *Structs*) |
| `fn(i64, i64) -> i64`, `fn(ptr)`, `fn() -> (i64, i64)` | a function value: the signature is the type (see *Calls*) |
| `f32x4`, `i32x8`, `u1x4`, `floatx4`, `intxN` | a vector: N lanes of one type, `TxN` (see *Vectors*); `Tx1` is `T` |
| `ptr(T)`, `ptr(array(f32, 512, 512))` | a typed pointer: an address that knows what it points at (see *Typed pointers and arrays*) |
| `array(T, W, H, ...)` | an array with a shape — a memory type, never a value: what a typed pointer points at, or a `data` item's type |
| `i(expr)`, `u(expr)` | an integer whose width is an expression — inside type declarations |
| `int`, `uint`, `float`, `fixed`, `unit`, `sunit`, `rational`, `scalar` | abstract numbers — resolved to a concrete type by the target's replacement policy (see *Abstract numeric types*) |

Any width works anywhere a value lives — registers, block parameters, calls, packs. Memory is the exception: only 8-, 16-, 32-, 64-bit types and whole words above that (128, 192, 256) can be loaded and stored.

A value wider than 64 bits is *wide*: it is written and checked like any other, then lowered to a row of 64-bit words, lowest first, right after parsing (`src/wide.rs`), so that no backend ever meets one. A `u128` parameter is two `u64` parameters, a `u128` result two results, and each operation becomes the word operations that compute it — carry chains, schoolbook products, word-and-bit arrangement for shifts, lexicographic compares. `div` and `rem` alone go to the library (`lib/wide.ssa`'s `div(W)`/`rem(W)`, loops written over the wide type and lowered like everything else). `suite/wide.ssa` shows the shape: `;! add 1 0 2 0 -> 3, 0` is `add` on two `u128`s given as words. A `const` on a wide value takes a 128-bit width expression (`const 1 << 112`). The libraries use wide types wherever an intermediate outgrows a word — `float`'s `mul` forms the exact `2M + 2`-bit product in `u(2 * M + 10)`, `fixed`'s in `u128` — which is what lets one `add(E, M, round)` body serve `float(4, 3)` and `float(15, 112)` alike (`suite/f128.ssa`).

Floats are reserved for a later version (`float` will join `int` as an abstract type when they land); their bit layouts are already expressible as packs.

## Structure

```
fn name(a: i64, b: ptr) -> i64 {
entry:
    ...
next(x: i64):
    ...
}
```

- A function declares named, typed parameters and zero or more return types. Internally the return is always a tuple; the text format allows `-> i64` as shorthand for `-> (i64)`, `-> (i64, i64)` for multiple values, and omitting the arrow entirely for none.
- The first block is the entry block. It takes no parameters; the function's parameters are in scope from the start.
- Every other block may declare parameters — the values it receives from branches that target it.
- Every block ends with exactly one terminator (`jmp`, `br`, or `ret`), and terminators appear nowhere else.

## Instructions

Every value-defining instruction declares its result's type:

```
name: ty = op operands...
```

Instructions with no result (`store`, result-less calls, terminators) have no left-hand side.

### Constants

```
v: i64 = const 42
p: ptr = const 0          ; null
u: ptr = const 0x10000000 ; a raw address — MMIO registers, fixed buffers
c: rgb = const 4095       ; a pack, by its bit pattern
w: u(M) = const (1 << M) - 1   ; inside a generic: an expression over its parameters
x: f32 = const 0.1        ; a float, by its value: the nearest f32, exactly rounded
y: f16 = const 3          ; 3.0
z: f64 = const -inf       ; also inf, nan
```

On a `float(E, M)` the literal is a number, converted to the nearest value of that type (ties to even, subnormals included) — the same bits the FPU would produce from the same decimal. Its bit pattern is a `cast` from an integer away.

**Literals as operands.** Anywhere an operand's type is fixed by context, a literal may stand in for a value: the other operand of `add`, `cmp`, and friends; a pack's field; a call's parameter; a block's parameter; the function's return type; the loop variable's declared type.

```
b: i64 = add a, 1
lt: u1 = cmp.lt 0, b
half: f32 = mul x, 0.5
jmp loop(0, 0)
ret 0
r: i64 = g(b, 2)
```

Where nothing fixes the type — a stored value, a `conv` source — write it after the literal: `store 1: u8, p`. A literal becomes a hidden `const` just before the instruction; `probe parse` prints it back inline.

On a library number type (`fixed`, `rational`, `unit`, ... — any pack its library can `conv` into from `i64`), a literal is a *value*: `const 0.5` on a `fixed(8, 8)` is 128/256, `mul x, 0.5` halves whatever `x` is, and `y: scalar = sub 1, x` means one minus `x` in every family. This costs nothing in the compiler: the literal is read as an `i64` or an `f64` and handed to the library's own `conv`, hidden like the `const`.

### Integer arithmetic and bitwise ops

Both operands and the result must all have the same type. On integers, results wrap at the type's width and the type's signedness selects the operation. On a pack instantiated from a generic type, the opcode is dispatched to the generic function of the same name that takes that type (see *Generic functions*).

```
v: i5 = add a, b
v: i5 = sub a, b
v: i5 = mul a, b
v: i5 = div a, b        ; signed for iN, unsigned for uN (truncating)
v: i5 = rem a, b        ; remainder, sign follows the dividend for iN
v: i5 = and a, b
v: i5 = or  a, b
v: i5 = xor a, b
v: i5 = shl a, b        ; amount mod 32/64 for those widths; >= N unspecified otherwise
v: i5 = shr a, b        ; arithmetic (sign-fill) for iN, logical for uN
```

Division by zero is target-dependent (wasm traps, the CPUs return 0); `MIN div -1` wraps to `MIN` at every width (the wasm emitter guards its trapping `div_s` to make that so).

### Comparison

Operands must share a type; the result is `u1`. The condition is part of the opcode; on integers the ordering is signed for `iN` and unsigned for `uN` and `ptr`. On a pack, `cmp.lt` is the library's `lt` for that type (the float library's six predicates give IEEE's answers: everything but `ne` is false when a NaN is involved, and -0 equals +0).

```
c: u1 = cmp.eq a, b    ; also: ne
c: u1 = cmp.lt a, b    ; also: le gt ge
```

### Conversion and reinterpretation

Two opcodes, and the types on both sides decide the rest. `conv` carries the *value* across: between integers it widens (sign-filling from an `iN`, zero-filling from a `uN`), narrows to the low bits, or re-reads at the same width; with a pack on either side it is the library's `conv` generic for that pair of types — `f32` to `f64`, `i32` to `f16`, `f64` to `u8` (see *Generic functions*). `cast` keeps the *bits* and changes the reading, between any two types of the same width.

```
v: i64 = conv a            ; i8 -> i64: sign-extended
v: u8  = conv a            ; i64 -> u8: the low byte
v: f64 = conv a            ; f32 -> f64: the same number, exactly
v: i32 = conv a            ; f32 -> i32: 1.0 -> 1, truncating, saturating, NaN -> 0
v: u32 = cast a            ; f32 -> u32: 1.0 -> 0x3f800000
v: u5  = cast a            ; i5 -> u5: -3 -> 29
```

### Memory

The address operand must be `ptr`. The access width is the result type (loads) or the stored value's type (stores), which must be 8, 16, 32, or 64 bits wide — an integer, a pack, or `ptr` — or a wide value's whole words (128, 192, 256 bits, stored low word first). Loads of `iN` sign-extend, of `uN` zero-extend.

```
v: i64 = load addr
b: u8  = load addr
store v, addr
v: i64 = load base, 16         ; base + 16
v: i32 = load base, i, 4       ; base + i * 4 (i: i64 or u64; step 1, 2, 4 or 8)
store v, base, i, 4
p: ptr = ptradd base, off    ; base: ptr, off: i64 or u64
```

Values are an unbounded set of named registers; `load` and `store` are the only way in and out of memory, and the two addressing forms are what the targets' load and store instructions take themselves (an immediate offset on all three; the scaled index computes an address first where the target has no form for it). Whether a value spills to the stack is the allocator's business and invisible here.

### Check

```
check c                    ; c: u1 — if it does not hold, stop here
```

An assertion. A `check` that holds costs a compare; one that does not is a breakpoint trap — `brk` on arm64, `ebreak` on riscv64, `unreachable` on wasm — which on a machine lands in `fn __trap` with the platform's `trap_check` cause and, from `resume()`, the address of the check (`os/check.ssa` prints it and stops). Under the JIT it ends the process, as an assertion would. It is how a library says a capacity was exceeded or a bound broken, instead of returning a value nobody checks.

### Scratch

```
p: ptr = scratch 64        ; 64 bytes of the function's own memory
```

`scratch N` is the address of N bytes that belong to the function for as long as it runs — its frame on arm64 and riscv64, a shadow stack in linear memory on wasm — 16-aligned, uninitialized, gone when it returns (so never returned or stored where it outlives the call). Each `scratch` instruction is one area, the same on every pass through it; a recursive function gets one per activation. A callee may be handed it. It is the only memory a program owns besides `data`: a frame is at most 4095 bytes on arm64 and 2047 on riscv64 for now. Memory with a longer lifetime than a call and a shorter one than the machine is the arena's (`lib/arena.ssa`): bump allocation over memory the program declared, reset all at once at a moment nothing points into it — a frame's end, a period boundary — and a failed `check` when it runs out. `os/sleep.ssa` keeps two, produced into by the scheduler and consumed by the idle task, flipped every tenth of a second. Memory with an object's lifetime — given back one piece at a time, in any order — is the pool's (`lib/pool.ssa`): fixed-size slots, a free list through the free ones, a flag per slot so that giving one back twice is a failed `check`. `os/sleep.ssa`'s task stacks come from one; a one-shot task spawned by a callback sleeps until its time, runs, and hands its stack back. And the memory those are carved from is the heap's (`lib/heap.ssa`): a buddy allocator over one block — a declared array, or on a machine the RAM above the image, `platform heap_base` to `platform ram_end` — blocks a power of two and aligned to their size, taken rarely and in big pieces — a pool's slots, an arena's bytes, a task's region — and given back whole; a byte per node of its split tree makes a double give or a wrong size a failed `check`, and a kernel *seals* it inside every interrupt so that nothing allocates in a handler. It is a heap for regions, not for objects: those are the rungs — a call's, a frame's, an object's, the machine's — and the discipline is that objects live in the rungs and the rungs come from the heap.

### Typed pointers and arrays

```
g: ptr(array(i32, 4, 3)) = cast p    ; a 4-wide, 3-high grid at p
v: i32 = load g, i, j                ; element (i, j): i + j * 4
store v, g, i, j
e: ptr(i32) = index g, i, j          ; the element's own address
q: ptr(i64) = cast p                 ; a scalar pointer ...
a: i64 = load q                      ; ... the i64 at it
b: i64 = load q, i                   ; ... or the i-th i64 from it
t: ptr(array(f32x4, 8)) = scratch    ; sized by its type
d: ptr(array(i32, 3, 2)) = addr table
data table: array(i32, 3, 2) = { 1, 2, 3, 10, 20, 30 }
```

`ptr` is an address of bytes, and stays that. `ptr(T)` is an address that knows it points at a T — a scalar, a vector, a struct, or an array `array(T, W, H, ...)` with a shape, innermost dimension first, row-major, naturally aligned — so that `load`, `store` and `index` take indices instead of a byte offset and a step: as many as the shape has dimensions (none or one for a scalar pointee), each an `i64` or a literal. The element type is checked against the value loaded or stored. An array is a memory type: no value has one; it is what a typed pointer points at, what `scratch` sizes itself by when its result is typed, and what `data` declares (`addr` gives a `ptr`, or a `ptr(...)` to the item's array or its element). A typed pointer casts to and from `ptr` and `u64`, compares, and travels like any 64-bit value (32 on wasm). A typed access is lowered as it is parsed — the shape makes the offset, the element the step, through a hidden cast to `ptr` — so no backend meets one and `probe parse` shows the arithmetic. Textures and other opaque device resources are not arrays: those will be handles with platform operations, when a target has them.

### Data, and the machine

```
data greeting = "hello world ᕦ(ツ)ᕤ\n"        ; an array of bytes, UTF-8, no terminator
data table: array(i32, 4) = { 10, 20, 30, -40 }
data buffer: array(u8, 256)                   ; zeros
p: ptr = addr greeting                        ; its address
n: i64 = len greeting                         ; its element count, a constant (24 here)
uart: ptr = platform uart                     ; a constant the platform file provides
```

`data` is initialized memory — elements are integers of up to 64 bits at their natural size, little-endian, or pointers and function values, which start as zeros (a table of handlers the program fills) — laid out after the code (8-aligned) and reached by a PC-relative address (`adr`, `auipc`/`addi`, a `data` segment on wasm). It is the program's RAM everywhere: writable pages after the code under the JIT, memory on bare metal and wasm. `platform name` is a constant from the platform file's `const` lines — a board's addresses (`uart`, `finisher` on qemu's virt machines) — resolved when the target is known, so one program can say `store c, uart` for two boards. `probe boot file.ssa [riscv|arm]` runs a program with a `fn __start()` on bare-metal qemu and prints what it wrote to the UART, feeding it what is piped in (or the terminal): `os/hello.ssa` is the first one.

Traps are the second machine convention. A program with a `fn __trap(a: u64, b: u64, c: u64) -> u64` has a trap handler: `probe boot` compiles it with a frame that keeps every register of the code it interrupted and a return that resumes that code (`eret`, `mret`), and on arm64 lays a 2K-aligned vector table of branches to it just before its entry. The handler is called with the trapped code's first three argument registers and its result replaces the first — so a system call is `syscall(n, a, b) -> u64` on one side and `__trap(n, a, b) -> u64` on the other. How the machine takes and leaves a trap is the platform's: `vectors(t: ptr)` installs the handler (`msr vbar_el1` / `csrw mtvec`), `cause()` reads why (`esr_el1`'s class / `mcause`), `resume()` and `resume_at(p)` read and set where the trapped code continues (`elr_el1` / `mepc`), `syscall` is `svc` / `ecall`, and the constants `trap_syscall` and `resume_skip` say what a system call looks like and how far past it to resume (arm64 steps past its `svc`, riscv leaves `mepc` on the `ecall`). A program declares those functions with placeholder bodies and the platform supplies the real ones, as it does for `psci`. `os/echo.ssa` is the second operating system: a handler dispatching write, read and exit through a `data` table of function values, and a program that uses only those calls.

Interrupts land in `fn __irq()`. `probe boot` lays a vector table before `__trap` on both machines — sixteen entries; on arm64 the exception entries branch to `__trap` and the IRQ entries to `__irq`, on riscv64 mtvec is put in vectored mode with entry 0 to `__trap` and the rest, one per interrupt cause, to `__irq` — and compiles `__irq` with a frame that keeps *every* register of the interrupted code, float scratch included, since an interrupt can land between any two instructions. The board's side is again the platform's: `now()` and `hz()` read the counter and its frequency, `timer_at(t)` and `timer_off()` arm and disarm the timer, `irq_on()` enables the timer's interrupt (the GICv2 on the aarch64 virt board; mie and mstatus on the riscv64 one), `irq_ack()` and `irq_done(id)` take and finish one, and `idle()` waits for the next; `irq_timer` is the timer's interrupt id. A device's interrupt is the same shape with two more: `uart_irq_on()` asks the board for one per received byte, and — since riscv64 delivers every device interrupt as one cause, "external", and names the source only at its controller — a handler that sees `irq_external` asks `irq_claim()` for the source (on arm64 the controller names it in `irq_ack` and `irq_external` is an id that never arrives), then `irq_done` completes it; `irq_uart` is the port's source. `os/echo.ssa` reads its input that way: `__irq` drains the port into a ring and `read` sleeps until a line is in it. `os/clock.ssa` is the third operating system: ten ticks a tenth of a second apart, each deadline a step from the previous one, and the elapsed count turned into exact milliseconds by `lib/time.ssa`.

An interrupt handler that switches tasks is `fn __irq(sp: ptr) -> ptr`: it is handed its frame — where the interrupted code's registers now are, the *whole* register file for this form, callee-saved ones included — and returns the frame to go back from; the epilogue restores from that frame and returns to it. A task is then a stack and a place to resume: the frame's address and `resume()`'s, saved when it is interrupted, restored (`resume_at`, the frame returned) when it is chosen; a task that has never run is a stack whose top 4096 bytes are zeros — an empty frame, frames being smaller than that — and its function's address as the place to resume. `os/tasks.ssa` is the fourth operating system: two tasks, a time slice each, `ababababab`.

A task that wants to give up the cpu before its slice ends asks the machine for an interrupt — `reschedule()`, a software interrupt to this cpu (riscv's msip, an SGI on the GIC; `irq_soft` is its id) — so that switching stays in the one place it happens. `os/sleep.ssa` is the fifth operating system: tasks that sleep until a `time`, a scheduler that picks the earliest passed deadline and arms the timer to the earliest future one (tickless: twenty wakes, eighteen interrupts), and deadlines like 3/3 s and 7/7 s and 10/10 s that are one instant because a `time` is a rational. It also has callbacks: `at(t, f, arg)` and `after(d, f, arg)` run a function value at a time, inside the interrupt and before the scheduler picks a task — exact and cheap, so short and never sleeping. Neither is anything the IR knows about: a time is a library type, a callback a function value, and what the kernel does with them is the kernel's.

### Group memory

```
group tmp: array(i64, 64)                     ; what a threadgroup shares: typed, sized, zero
t: ptr(array(i64, 64)) = addr tmp
store id, t, tid
group_sync()                                  ; the barrier (lib/gpu.ssa)
v: i64 = load t, other
```

`group` declares memory a threadgroup shares, the way `data` declares the program's: an array type, no initializer (it starts as nothing), reached through `addr` and the typed loads and stores. On a GPU it is the threadgroup array (AIR: one `addrspace(3)` global holding every item), and an address computed from `addr` of a group item — through arithmetic, casts and block arguments — is a threadgroup access; such an address cannot be stored, passed to a function or returned, since no pointer type says which memory it is in. On a machine that runs a group of one, group items are data. `group_sync()` is the barrier — `air.wg.barrier` on the GPU, nothing on one thread — so a program written for many threads parses and runs everywhere (`suite/group.ssa`; `examples/reduce.ssa` for the many-thread case).

### The current thread

```
t: ptr = thread()                             ; this thread's block (lib/thread.ssa)
thread_set(block)                             ; install one: 16 KB plus the program's group section
```

`thread()` is a pointer to the current thread's own block of memory. The platform gives a *key* that tells threads apart (`ext thread`: `tpidrro_el0` on arm64 — unique per thread under macOS, zero at boot — `tp` on riscv64, nothing on wasm), and `lib/thread.ssa` maps keys to blocks in a small table `thread_set` fills (a slot per thread, `thread_set_slot`, when several install at once); a thread with no block has the default block the library declares. On a GPU `thread()` is the platform's, the thread's header. The block holds the fibre scheduler and its frames, and, at 16384, this thread's copy of the program's `group` items: on a machine `addr` of a group item is `thread()` plus its offset (the machine backends' lowering), so every group in flight has its own, as on the GPU. (The writable `tpidr_el0` was the first design; XNU rewrites it at every context switch, so a value left there is gone the moment a thread is preempted.)

### Cores

```
core_launch(1, rec, stack_top, main, arg)     ; core 1 runs main(rec) on its own stack (lib/core.ssa)
```

`lib/core.ssa` starts another core: `core_launch` fills a record — the stack top, the function, its argument, the core's key for `thread()` — and hands it to the platform's `core_start`. On arm64 virt that is PSCI `CPU_ON` with `core_boot` as the entry, which sets sp and `tpidrro_el0` from the record and jumps; on riscv64 virt every hart runs the boot preamble, which parks harts other than 0 in `wfi` until their record is in the mailbox and hart 0 rings the CLINT's `msip`, then sets sp and `tp` from it and jumps. A core has no way back: its main ends in `core_idle()` (`wfi`) forever. Both machines boot with four cores (`-smp 4`); a platform with one core (the JIT, wasm, the GPU) does nothing and main never runs.

### Fibres, and a kernel as a suite case

```
fibres_run(3, stacks, 4096, body)             ; body(0), body(1), body(2) by turns (lib/fibre.ssa)
fibre_yield()                                 ; the current fibre gives up its turn
;! __kernel 256 64 -> 2016, 6112, 10208, 14304   ; a directive: n threads in groups of g
```

`lib/fibre.ssa` runs bodies by turns on one thread, each on a stack of its own: `fibres_run(count, stacks, stack_bytes, body)` runs `body(k)` for every k round-robin, a body running until it calls `fibre_yield()` or returns, so every body has yielded before any goes on — the shape of a barrier. The switch is the platform's (`fibre_switch`, the callee-saved registers and sp saved and loaded; `targets/arm64.platform`, `riscv64.platform`); wasm, with one stack, runs the bodies one after another instead (`fibre_stacks = 0`). `group_sync()` yields while fibres run. So a program written for a threadgroup is a suite case everywhere: `;! __kernel n g [m] -> words` runs its `__kernel` as n threads in groups of g — the real dispatch on the GPU, a group of fibres per group on a machine, dealt across m threads: OS threads under the JIT, cores on the qemu machines (thread t takes groups t, t + m, ..., each with a thread block and group memory of its own) — and compares the area's first words; on wasm it is skipped and counted. A program with a `__kernel` takes only such directives (`suite/reduce.ssa`). A directive may expect a failed check instead of results — `;! overflows i64[0,0,0,0,0,0,0,0,0,0] -> check` — and passes when the case ends in one: under the JIT it runs in a forked child that `brk` ends, on a machine in a boot of its own whose `__trap` says so, on wasm as the driver's trap; the GPU, which runs on past a failed check, skips it and says why.

### The simdgroup

```
s: f32 = simd_sum x                           ; across the threads that execute together
m: i32 = simd_max y                           ; on floats, halves, 32-bit integers
zero: i64 = const 0
b: f32 = simd_broadcast x, zero               ; a thread index or delta is an i64
p: u32 = simd_prefix_sum z                    ; inclusive
l: i64 = simd_thread()                        ; 0..simd_size() - 1
v: u1 = simd_all(c)                           ; simd_any, simd_ballot -> u64, simd_first
```

`lib/gpu.ssa` names the simdgroup's operations — `simd_sum`, `simd_product`, `simd_max`, `simd_min`, `simd_prefix_sum`, `simd_shuffle`, `simd_broadcast`, `simd_shuffle_xor`, `simd_shuffle_down`, `simd_shuffle_up`, the votes, `simd_thread` and `simd_size` — as generics over floats and integers, written by opcode (`simd_sum x`) or called. On a GPU they are the platform's (`air.simd_sum.f32`, ...; 32 threads on an Apple GPU). On one thread the simdgroup is that thread — every reduction is its argument, a shuffle the value, the index 0, the width 1. With fibres running a group (a `__kernel` directive) the simdgroup is 32 consecutive threads and an operation is an exchange through a table in the thread's block: every thread puts its word in its slot and yields, reads what it needs, and yields again — so `suite/simdgroup.ssa` holds on four referees. Every thread of a simdgroup must reach the same operation, as on the hardware — and a machine checks it: each fibre counts its puts in a word of its own (`fibre_word`), and after a round every thread of the simdgroup must have put as often as this one, or a `check` fails (`suite/lockstep.ssa`, where one thread goes around a `simd_sum`: `-> check` on native, riscv and arm; the GPU reads what it reads and skips the case). (A vector has lanes; a simdgroup has threads.) A `simd_*` operation the platform has no instruction for (a 64-bit value) is an error on the GPU rather than its one-thread form.

### Calls

A name followed by an argument list is a call — no opcode is ever followed by `(`, so no keyword is needed. Signatures are checked against the module if the function is defined here, taken on trust if external.

```
v: i64 = f(a, b)             ; call with one result
q: i64, r: i64 = divmod(a, b)  ; call with two results
g(a)                           ; call with results ignored (or none)
```

A call binds either *all* of the callee's return values or *none* of them (calls and `unpack` are the only instructions that define more than one value).

On the machines each class of value crosses a call in its own registers, counted separately: integers and pointers in `x0..x7` (arm64) or `a0..a7` (riscv64), floats in `v0..v7` (arm64) or `fa0..fa7`, vectors in `v0..v7` (arm64, shared with the floats, as AAPCS64 has it) or `v8..v15` (riscv64) — arguments and results alike, eight of each class, and the arguments beyond those on the stack in order (8 bytes each, a vector 16, from the caller's `sp` up, which the caller lowers for the call and restores after). A caller from Rust (`jit.call`) passes and reads integer words; a function whose parameters or results have a register class gets a wrapper (`__w_f`, generated when the module is compiled for the JIT: bits in, cast, call, cast, bits out), which the JIT calls in its place — a vector at that boundary has no wrapper, and a caller passes its lanes.

A function is also a value. Its type is its signature, written the way the function declares it — `fn(i64) -> i64`, `fn(ptr, i64)`, `fn(i64, i64) -> (i64, i64)` — and `addr` makes one, the same `addr` that reaches a `data` item. Calling a value looks exactly like calling a name:

```
type unary = fn(i64) -> i64
f: unary = addr sq            ; the verifier checks sq against the type
r: i64 = f(x)                 ; a call through the value
```

Because the signature is on the type, a call through a value is checked like any other, and the value goes wherever a value goes: parameters, results, block parameters (a reducer carried around a loop), memory (`store f, p`; a table of handlers), and `cast` to a `u64` or `ptr` when its bits are wanted. Two spellings of one signature are one type. On arm64 and riscv64 the value is the function's address (`adr` / `auipc`+`addi`) and the call is `blr` / `jalr`; on wasm it is an index into a table of the address-taken functions and the call is `call_indirect`, which also checks the signature at run time. In the incremental arena it is the function's trampoline, so a value taken before an edit calls the new code. A function type may not yet take or return a struct or a value wider than 64 bits, and generics do not range over function types. `suite/indirect.ssa` has the cases.

### Terminators

Branch arguments must match the target block's parameters in count and type.

```
jmp next(a, b)
br c, then(a), else()   ; c: u1 — empty parens may be omitted
ret v                      ; one return value
ret q, r                  ; multiple return values
ret                         ; none
```

### Packs

A `pack` is a record of bitfields laid out **lowest bits first**: the first field occupies bit 0 upward, the next starts where it ends, and the total must fit in 256 bits (above 64 it is a wide value, in words). Fields are integers or other packs; a pack value is carried as the unsigned integer of its total width and can go anywhere a value can — parameters, block parameters, returns, memory if it is 8, 16, 32, or 64 bits wide or whole words.

```
type rgb = pack { r: u5, g: u6, b: u5 }      ; 16 bits: r = bits 0-4, g = 5-10, b = 11-15
type pix = pack { c: rgb, a: u8 }            ; 24 bits, nested

c: rgb = pack r, g, b                        ; one value per field, in order
g: u6 = get c, g                             ; read a field (iN fields sign-extend)
d: rgb = set c, g, g2                        ; a copy with one field replaced
r: u5, g: u6, b: u5 = unpack c               ; every field at once
w: u16 = cast c                           ; the raw bits, and back again
```

Packs are compared structurally: two spellings of the same layout are the same type. `unpack` is, with calls, the only instruction that defines several values.

### Vectors

```
type v4 = i32x4                 ; four lanes of i32
a: f32x4 = pack x, y, z, w      ; lanes, in order
k: f32x4 = splat h              ; every lane h
c: f32x4 = add a, k             ; lane by lane
m: u1x4 = cmp.gt c, k           ; a comparison gives u1 lanes
x: f32 = get c, 2               ; lane 2
d: f32x4 = set c, 0, x          ; a copy with lane 0 replaced
r0: f32, r1: f32, r2: f32, r3: f32 = unpack c
s: f32x4 = sqrt c               ; a library operation, on every lane
```

`TxN` is N lanes of T, where T is any integer or pack of at most 64 bits — concrete (`f32x4`), abstract (`floatx4`, `intx8`: the policy's type), or in a generic with the lane count a width parameter (`i32xN`). `Tx1` is T. A vector is a struct whose fields are its lanes, numbered from 0, laid out consecutively in memory; so `pack`, `unpack`, `get`, `set`, loads and stores are the struct's, `get`/`set` taking a lane number. Arithmetic, comparisons, conversions and library operations (`sqrt`, `fma`) on vectors are **lane by lane**: the scalar operation on each lane — an instruction, or the library's for a pack lane — which is a definition the IR makes itself, as it does for integers wider than a word, rather than a library's. The parser writes a vector operation out that way (`probe parse` shows the lanes), and the struct lowering makes the packs and unpacks free; so vectors run on every backend, verified lane by lane. A platform with vector registers holds a vector type in a class and takes the whole operation at once, checked against that meaning: `targets/arm64.platform` gives a class to every vector of 64 or 128 bits — `f32x4`, `f64x2`, `f32x2`, the integer vectors of 8- to 64-bit lanes (`i32x4`, `u16x4`, `i8x8`, `u8x16`, ...), and the masks `u1x2` to `u1x16` — and has a rule per operation (`fadd {v}.4s, {v}.4s, {v}.4s = add(f32x4, f32x4) -> f32x4`; a comparison is `fcmgt` and a `neg`, so a `u1xN` holds each lane as 0 or 1 in a 128/N-bit lane of a whole register — a 64-bit vector's comparison is widened first; `fma` is a `mov` and an `fmla`; a `conv` between lane widths is `sshll`/`ushll`/`xtn`, and `fcvtl`/`fcvtn` between `f32x2` and `f64x2`), and the parser keeps a vector of a classed type whole only for the operations that have a rule — a divide of `i32x4`, a `u8x8`, an `i64x4` stay lane by lane, and `probe parse` shows which. A whole vector lives in the float file (v8–v15, saved and spilled as 128 bits), its lanes moved by `ins`/`umov`, and reaches memory by `ld1`/`st1` at the alignment of a lane, which is all a vector in memory has (with the MMU off a 16-byte `ldr q` from an 8-aligned address faults). On riscv64 the same types are RVV registers (`targets/riscv64.platform`, `ext V`): a rule sets the vtype it runs under first (`vsetivli x0, 4, e32, m1, ta, ma` — the one piece of machine state a rule carries, set by whoever needs it, the emitter too before its lane moves), a comparison leaves its mask in `v0` and merges 1 over 0 into the result, lanes move by `vslide1up`/`vslidedown` and `vmv.x.s`, memory by `vle32`/`vse32`; as for scalars there, no `min`/`max` and no float-to-integer, whose NaN cases differ from the library's. The GPU (`builtin vectors`) takes every vector whole. `targets/arm64-noneon.platform` and `targets/riscv64-nov.platform` are the machines without the rules — the reference the rules are checked against, on the same machine. A vector is a parameter, result or argument like any other value, crossing a call in a vector register (see *Calls*). In memory a `u1xN` is a byte per lane, on every path: the lane form stores each `u1` as a `u8`, and the register form narrows its lanes to bytes on a store and widens them on a load (`xtn`/`ushll`, `vnsrl`/`vzext`), storing and loading exactly N bytes. Not yet: vectors wider than a register. Across the lanes, `lib/reduce.ssa` defines `sum`, `min` and `max` of a vector and `all` and `any` of a mask as generics over the vector's shape (`fn sum(W)(v: i(W)x4) -> i(W)`: a parameterized lane type may carry a lane count), each a pairwise tree — `(l0 + l1) + (l2 + l3)` — which is the order a pairwise instruction takes; written as operations with a scalar result, `s: i32 = sum v`, the vector's type choosing the generic. NEON has a rule for each (`addv`, `smaxv`, `faddp`, `fmaxv`, `uminv` for `all`...), RVV for the integer ones (`vredsum`, `vredmin`...) — a float sum in another order, or a `vfredmin` that takes NaN as `vfmin` does, is no rule, and the body runs. Shuffles and dot products are written over lanes for now; a vector's lanes are values like any other, so a `u8x8` can be a block parameter and a `u1x4` the result of a compare.

### Structs

```
type point = struct { x: f32, y: f32, z: f32 }
p: point = pack x, y, z
z: f32 = get p, z
q: point = set p, z, 1.0
a: point = load base, i, 12     ; element i of an array of 12-byte structs
store q, base, 16
```

A `struct` is a group of fields — integers, packs, `ptr`, wide values, other structs — side by side: in memory at their natural offsets (each field aligned to its size, the whole to its largest field), and in registers as separate values. It shares the pack vocabulary (`pack`, `unpack`, `get`, `set`, `load`, `store`, parameters, results, block parameters) and differs in one thing: it is never a bit pattern. There is no `cast` to or from a struct, no literal, no arithmetic dispatch — a program cannot observe how one is laid out, which is what leaves the layout to the compiler (an array of structs may be stored field-major one day without a program noticing). A struct is dissolved into its fields right after parsing (`src/aggregate.rs`): `pack`, `get`, `set` and `unpack` become names for values that already exist, a `load` or `store` becomes one per field at its offset, and a struct parameter is its fields in order — which is also how the suite passes one (`suite/struct.ssa`).

### Type declarations

`type` names a type, optionally with integer parameters that stand for widths. The right-hand side is any type expression: a pack, `i(expr)` or `u(expr)` with a width expression over the parameters (`+ - *` and parentheses), a builtin, or another declared type instantiated with arguments.

```
type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }
type f32 = float(8, 23)
type f16 = float(5, 10)
type bits(E, M) = u(E + M + 1)
type byte = u8
```

A parametric type is instantiated wherever it is used with arguments — `x: float(8, 23)`, `y: bits(5, 10)` — and an alias is instantiated where it is declared. `f32`, `float(8, 23)`, and `pack { mantissa: u23, exponent: u8, sign: u1 }` are one type; it prints under the first name it was given. Declarations may appear anywhere at the top level; each may refer only to types declared before it.

### Generic functions

A function can take the same kind of width parameters, in a group before its value parameters. It is a template: nothing is compiled until it is instantiated, either by name or at a call site, and each instantiation is an ordinary function whose body was parsed with the parameters bound — so `u(M + 5)` is a concrete type there, and `const` may be a width expression.

```
fn add(E, M, round)(a: float(E, M), b: float(E, M)) -> float(E, M) {
    ...
    n1: float(E, M) = fnan(E, M)()      ; instantiates fnan for this E, M
    ...
    r: float(E, M) = fpack(E, M, round)(sh, nx32, nf)
    ret r
}
fn fadd32 = add(8, 23)                      ; a named instantiation
r: f16 = add(5, 10)(x, y)              ; an anonymous one, add_5_10_0
s: f16 = add x, y                           ; the same, by dispatch
```

A parameter nothing binds is supplied by the policy when it has a value by that name: `round` is the one such name, 0 nearest even, 1 toward zero, 2 down, 3 up, 4 nearest away (`--round=even|zero|down|up|away`). That is why `add(8, 23)` and `add x, y` name two of the three parameters: the mode comes from the policy — or from the enclosing instantiation when a generic with a `round` of its own calls another, so `sub`'s `add a, nb` rounds as `sub` does. `add(8, 23, 2)` fixes it regardless of policy (`suite/round.ssa`). In the library every rounding decision is in `fpack`, and since the mode is a width parameter, all but one of its tests fold away in each instance.

Instantiations are shared: `add(8, 23)` anywhere is `fadd32` once that name exists. `probe parse` prints the instantiated functions and not the templates — like structured control flow, generics are sugar the parser lowers. A pack literal `const` is its bit pattern.

**The prelude.** Every program compiled by probe gets `lib/*.ssa` appended (integers' `min`/`max`/`abs`/`neg` `lib/int.ssa`, floats `lib/float.ssa`, fixed point `lib/fixed.ssa`, unit fractions `lib/unit.ssa`, two-word integer helpers `lib/wide.ssa`), so `float(E, M)`, `fixed(I, F)`, `unit(N)`, `sunit(N)`, `f32`, `f64`, `f16`, `bf16`, and the operations on them are always in scope; a file may re-declare a type identically, and may name a type the prelude declares after it. An explicit instantiation by name (`add(8, 23)`) needs the name to be unambiguous — `add` has a float and a fixed form, so apply it as an operation and let the types choose.

**Dispatch.** A generic function whose first parameter and first result are written in terms of its own width parameters *is* an operation of its name: applying that name to a value whose type matches the parameter — and whose declared result matches the result — instantiates it with the widths the match binds. `add x, y` on two `f16` values lowers to `add(5, 10)(x, y)`; `sqrt x` — not an integer opcode at all — to `sqrt(5, 10)(x)`; `r: f16 = conv x` with `x: i32` finds the `conv(W, E, M)` that takes `i(W)` and returns `float(E, M)`, binding all three. Generics may share a name when their signatures differ, which is how `conv` from `i(W)` and from `u(W)` coexist. The opcode set never grows: a library adds operations, and a platform adds instructions.

### Platforms

A library instantiation defines what an operation *means*; a platform says which of them the target has an instruction for, and where values of a type live, as rules in `targets/<target>.platform`:

```
class s = f32
class d = f64
fadd {s}, {s}, {s} = add(f32, f32) -> f32
fcvtzs {w}, {s} = conv(f32) -> i32
lt(f32, f32) -> u1
    fcmp a, b
    cset r, lo
```

A `class` line gives the types named a register class — the slot letter the learned templates use for it (`s`/`d` on arm64, `f` on riscv64, a local type on wasm) — so the allocator keeps such values in that file: a chain of float operations is `fadd`, `fmul`, `fsub` with nothing in between, and a move between files happens only where a value really changes class (a `cast` to its bits, a call boundary, a pack field read). A one-line rule is a learned template and the library instance it computes, the template's slots being the result then the arguments in order; a rule that takes several instructions is a header with indented lines over `a`, `b`, `c` and `r`, with literals for condition and immediate slots. Types are written by their program names (`f32` for `float(8, 23)`), resolved through the module's declarations, and every line must resolve to a template the learner verified — a rule naming an instruction it has no template for is an error, not a guess. The files today cover `add`, `sub`, `mul`, `div`, `sqrt`, `neg`, `abs`, `min`, `max`, `fma`, the six comparisons, on `f32` and `f64`, and `conv` between those and to and from 32/64-bit integers — minus what a target's own semantics rule out, which the file simply leaves out: riscv64 has no float-to-int rule (its `fcvt.w.s` gives the maximum integer for NaN where the library gives 0) and no `min`/`max` (its `fmin` returns the number when one operand is NaN, the library returns NaN); wasm has no `fma`. When an emitter compiles such an instance, or a call to one, it emits the rule instead of the SSA body. A rule matches the nearest-even instance (`add(8, 23, 0)`), which is what the instructions compute; an instance in another rounding mode stays in the library.

**Variants.** An ISA comes in variants, so a platform file is grouped by extension and a variant is a file that names its target, a base, and what it lacks:

```
target riscv64
base riscv64
without M, F, D
```

`ext NAME` starts a group in the base file (`targets/riscv64.platform` has `M`, `F`, `D`; `arm64.platform` has `FP`); the `class`, rule and `builtin` lines that follow belong to it. `builtin mul, div, rem` in the `M` group says which integer opcodes the emitters otherwise assume: on a variant without it the parser sends `mul`, `div` and `rem` to the library's `mul(W)`/`div(W)`/`rem(W)` generics (`lib/wide.ssa`), and the wide lowering's word products call `mul(64)` — the same program, slower, still correct. `--platform=NAME` selects a variant for every command (`rv64im`, `rv64i`, `arm64-nofp` exist today; a target's own name is the full ISA). `probe footprint file.ssa [riscv]` decodes the emitted code against the learned templates and lists what a program actually used, which is how a variant is checked to keep its word — the suite compiled for `rv64i` shows no `mul`, `div`, `rem` or `f*` instruction, and a test says so. The library body remains the reference: `--soft` compiles with an empty platform, and the two must agree — and both are checked against Berkeley TestFloat's vectors, bit for bit, in every mode, by `probe testfloat` (`tools/get-testfloat.sh` builds the generator). NaN payloads are the one place they may differ — the library canonicalizes, hardware propagates — as on any real platform.

## Example

```
; sum of 0..n
fn sum(n: i64) -> i64 {
entry:
    zero: i64 = const 0
    jmp loop(zero, zero)
loop(i: i64, acc: i64):
    done: u1 = cmp.ge i, n
    br done, exit, body
body:
    acc2: i64 = add acc, i
    one:  i64 = const 1
    i2:   i64 = add i, one
    jmp loop(i2, acc2)
exit:
    ret acc
}
```

## Abstract numeric types

`int` is an **abstract integer type**: code written with it does not choose a width — the compiler does, at compile time, by a *replacement policy* derived from the target (its natural register width, or a size-oriented choice like i32 on wasm32) and from user concerns (`--int=i32|i64`). `uint` is its unsigned twin and always takes the same width.

`float` is the same idea for the library's `float(E, M)`: a bare `float` is `float(E, M)` for the policy's E and M — `(11, 52)` on the register machines, `(8, 23)` on wasm32, or whatever `--float=f16|bf16|f32|f64|E,M` says — instantiated as the parser meets it (a parametric type's bare name is abstract when the policy has arguments for it). So `fn half(x: float) -> float { r: float = div x, 2.0 }` is written once, dispatches to the library's `div(E, M)` for the chosen width, and lands on the platform's `fdiv` where there is one.

`rational(N, D)` (`lib/rational.ssa`) is `numerator / denominator`, an `i(N)` over a `u(D)` kept reduced, with 128-bit intermediates so N and D go to 64; `lib/time.ssa` builds on `rational(64, 64)`: `type time`, `seconds`/`millis`/`micros`/`nanos`/`period` in, `to_*` out, and every operation the rational library's — exact, so nothing drifts.

`fixed` is the same again for the library's `fixed(I, F)` — a two's-complement integer of I + F bits with F fraction bits, in `lib/fixed.ssa` — resolved to half the `int` width each side (`fixed(32, 32)` with i64, `fixed(16, 16)` with i32) or `--fixed=I,F`. Its operations are integer instructions all the way down; `conv` reaches it from integers and floats, and back.

`unit` and `sunit` are fractions of one (`lib/unit.ssa`): `unit(N)` is 0.0 at 0 and 1.0 at 2^N − 1, `sunit(N)` is −1.0 at −(2^(N−1) − 1) and 1.0 at 2^(N−1) − 1. The scale is not a power of two, so a product is `(a·b + half) / max`, rounded; sums saturate at the ends of the range; `conv` goes to and from floats and integers. Bare, they take the policy's N (half the `int` width; `--unit=N`, `--sunit=N`).

`rational(N, D)` (`lib/rational.ssa`) is `numerator / denominator`, an `i(N)` over a `u(D)`, kept reduced with a positive denominator; a zero denominator is *not a rational* (NaR) and propagates like NaN. Its arithmetic is exact while the reduced result fits, and halved down to an approximation when it doesn't; `conv` from a float takes the best approximation the widths allow, by continued fractions (`0.33333334f32` is `1/3` in `rational(8, 8)`, `3.14159` is `22/7`). Bare `rational` is the policy's `(N, D)` (`--rational=N,D`).

`scalar` names a *family*: a bare `scalar` is whichever of `float`, `fixed`, `rational`, `unit`, `sunit` the policy says (`float` unless `--scalar=...`), itself bare, so that family's width applies. A program over `scalar` — `suite/scalar.ssa` — runs unchanged in every family; the suite runs it in all five. Because types live on variables, resolution is a single rewrite of the value tables before verification; opcodes, instructions, and everything downstream see only concrete types.

```
fn gcd(a: int, b: int) -> int {     ; width chosen per target/policy
    ...
    r: int = rem x, y               ; same ops, abstractly typed
```

- Abstract and concrete types mix freely (`i1` conditions, `ptr` addresses, explicit `i32`/`i64` where a width is required).
- A `conv` between `int` and a concrete type is only valid under policies where the widths actually differ — the verifier checks the resolved program, so such code ties itself to a policy. Policy-portable code keeps casts among concrete types.
- Memory keeps concrete types in portable code: a load of `int` changes access width with the policy.
- `float` code is policy-portable when its inputs and outputs are integers or its values stay exactly representable at every width in play (`suite/afloat.ssa`); the suite runs under both `int` policies and both `float` policies to keep it so.

## Structured control flow

A function body that opens with a statement instead of a `label:` is in **structured form**: control flow is expressed with `if`/`loop` instead of labels and branches (`jmp`, `br`, and labels are not allowed there). This is sugar — the parser lowers it to the same block graph on the fly, so everything downstream (verifier, emitters, printer) sees only flat form, and `probe parse` prints the lowered graph. Flat and structured functions can mix freely in one module. Lowering is one-way; the reverse direction (CFG → structured, the "relooper" problem) is deliberately not attempted.

The design follows the MLIR `scf` pattern: constructs *yield values* instead of writing to variables, preserving SSA.

### if

```
if c {                         ; plain: arms fall through to what follows
    ...
}

if c { ... } else { ... }      ; either arm may end with break/continue/ret

r: i64 = if c {               ; value-yielding: results bound on the left,
    yield a                    ; each arm must end with 'yield' (matching
} else {                        ; count and types), and else is required
    yield b
}
```

Lowering: `br` into two arm blocks; `yield`s and fallthroughs become jumps to a join block whose parameters are the bound results.

### loop

```
sum: i64 = loop(i: i64 = zero, acc: i64 = zero) {
    done: u1 = cmp.ge i, n
    if done {
        break acc              ; exit the loop, yielding its results
    }
    ...
    continue i2, acc2         ; back edge: new values for the loop vars
}
```

- The parenthesized list declares **loop-carried variables** with their initial values; `continue` supplies the next iteration's values.
- `break` exits, yielding the loop's results (bound on the left; a loop with no results uses bare `break`).
- Every path through the body must end with `break`, `continue`, or `ret`.
- `break`/`continue` bind to the innermost enclosing loop.

Lowering: a header block whose parameters are the loop variables (`continue` jumps to it), and an exit block whose parameters are the results (`break` jumps to it).

### Termination rules

A statement list "terminates" when it ends with `ret`, `break`, `continue`, `yield`, or an `if` all of whose arms terminate. Code after a terminating statement is an error (it would be unreachable). A structured body that falls off the end without `ret` fails verification (the final block has no terminator), same as flat form.

## Rules (checked by the verifier)

1. Every value is defined exactly once (function param, block param, or instruction result).
2. Every block ends with exactly one terminator; no instruction follows it.
3. Branch argument counts and types match the target block's parameters.
4. Operand types obey each instruction's typing rule above; `br` conditions and `icmp` results are `u1`; `const` literals fit their type under either the signed or the unsigned reading.
5. The entry block has no parameters and is not the target of any branch.
6. `ret` operands match the function's declared return types in count and type; a result-binding call matches the callee's return types the same way.
7. Every use is dominated by its definition: the value was defined earlier in the same block, or in a block that every path from the entry to this one passes through (a block's parameters are defined at its top). A value defined in one arm of a branch cannot be used after the join — pass it as a branch argument. Unreachable blocks are not checked; the passes remove them.
