//! The riscv64 emitter: SSA -> RV64IM machine code, every instruction word
//! encoded from the learned table (targets/riscv64.encodings.json) via the
//! same Encoder the arm64 backend uses — the JSON format is identical.
//!
//! Strategy mirrors the arm64 backend: linear-scan allocation over the
//! callee-saved pool s2..s11 (x18..x27), spills staged through t0/t1,
//! branch arguments two-phased through a0..a7. Differences that are
//! genuinely RISC-V:
//!
//! - No flags register: icmp lowers to slt/sltu/xor/sltiu sequences.
//! - One register size: every value is canonical in 64 bits — `iN`
//!   sign-extended, `uN`/ptr/packs zero-extended (see `ssa::Repr`). i32
//!   uses the W-instructions, which produce exactly that; every other
//!   narrow type is a 64-bit op followed by a shift pair (or andi) that
//!   re-normalizes. Fields of packs are shift pairs too.
//! - Conditional branches reach only ±4K, so `br` lowers to a two-word
//!   skip (`beq cond, x0, +8` over a `jal`) with ±1MB range.
//!
//! Registers: x0 zero | x1 ra | x2 sp | x5-x7 (t0-t2) scratch |
//! x10-x17 (a0-a7) arguments, results, branch staging | x18-x27 the pool.
//!
//! Frame: sp+0 saved ra, sp+16 callee-saved save area, then spill slots.

use crate::emit::{Compiled, Encoder};
use crate::regalloc::{self, Loc};
use crate::platform::{Native, Natives, Operand, Platform};
use crate::ssa::{BinOp, BlockId, Cond, Function, Inst, Module, Repr, Type, ValueId};

/// pool for the allocator: callee-saved s2..s11 (x18..x27) — values placed
/// here survive calls by construction
const REG_POOL: &[i64] = &[18, 19, 20, 21, 22, 23, 24, 25, 26, 27];

const ZERO: i64 = 0; // x0
const RA: i64 = 1;
const SP: i64 = 2;
const T0: i64 = 5;
const T1: i64 = 6;
const T2: i64 = 7;
const A0: i64 = 10;
const SLLI: &str = "slli {r}, {r}, {i 0..63}";
const FLD: &str = "fld {f}, {i -2048..2047}({r})";
const AUIPC: &str = "auipc {r}, {i 0..1048575}";
const FSD: &str = "fsd {f}, {i -2048..2047}({r})";
const FMV_D: &str = "fmv.d {f}, {f}";
/// the float file's pool: fs0..fs11, callee-saved
const F_POOL: &[i64] = &[8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
/// the vector registers values live in: v24..v31, saved by the callee
/// (every callee is this compiler's); a rule's vector temporaries are
/// v16..v19, the emitter's own v20..v23, v8..v15 carry arguments, and
/// v0 is the mask
const V_POOL: &[i64] = &[24, 25, 26, 27, 28, 29, 30, 31];
/// where a vector argument or result crosses a call: v8..v15, as the
/// RISC-V vector calling convention has it; floats cross in fa0..fa7
const V_ARGS: i64 = 8;
const FA0: i64 = 10;
const V_TEMPS: &[i64] = &[16, 17, 18, 19];
/// the vtype every vector instruction runs under: the lanes and their
/// width; sixteen bytes for a whole register saved or spilled
fn vset(bits: u32, n: u32) -> Result<&'static str, String> {
    Ok(match (n, bits) {
        (4, 32) => "vsetivli x0, 4, e32, m1, ta, ma",
        (2, 64) => "vsetivli x0, 2, e64, m1, ta, ma",
        (16, 8) => "vsetivli x0, 16, e8, m1, ta, ma",
        (2, 32) => "vsetivli x0, 2, e32, m1, ta, ma",
        (4, 16) => "vsetivli x0, 4, e16, m1, ta, ma",
        (8, 8) => "vsetivli x0, 8, e8, m1, ta, ma",
        (8, 16) => "vsetivli x0, 8, e16, m1, ta, ma",
        (2, 8) => "vsetivli x0, 2, e8, m1, ta, ma",
        (2, 16) => "vsetivli x0, 2, e16, m1, ta, ma",
        (4, 8) => "vsetivli x0, 4, e8, m1, ta, ma",
        _ => return Err(format!("no vtype for {} lanes of {} bits", n, bits)),
    })
}
const VSET_E8: &str = "vsetivli x0, 16, e8, m1, ta, ma";
/// a u1xN mask to and from memory, where it is N bytes: the register's
/// 128/N-bit lanes narrowed to bytes (vnsrl.wi, halving each time under
/// the narrower vtype) and stored under vl = N; a load widens them back
/// (vzext.vf2, doubling). A narrowing's source is a register pair and
/// must be even: v20 and v22 take turns; a widening's destination may
/// not be its source: v21 and v23.
fn mask_store(e: &mut RvEmit, rv: i64, ra: i64, n: u32) -> Result<(), String> {
    let mut lane = 128 / n;
    let mut cur = rv;
    if lane > 8 && cur % 2 != 0 {
        e.emit(VMV1R, &[20, cur])?;
        cur = 20;
    }
    while lane > 8 {
        let next = if cur == 20 { 22 } else { 20 };
        e.emit(vset(lane / 2, n)?, &[])?;
        e.emit("vnsrl.wi {v}, {v}, 0", &[next, cur])?;
        cur = next;
        lane /= 2;
    }
    e.emit(vset(8, n)?, &[])?;
    e.emit(&vle(8, true), &[cur, ra]).map(|_| ())
}
fn mask_load(e: &mut RvEmit, rd: i64, ra: i64, n: u32) -> Result<(), String> {
    e.emit(vset(8, n)?, &[])?;
    e.emit(&vle(8, false), &[21, ra])?;
    let mut lane = 8;
    let mut cur = 21;
    while lane < 128 / n {
        let next = if cur == 21 { 23 } else { 21 };
        e.emit(vset(lane * 2, n)?, &[])?;
        e.emit("vzext.vf2 {v}, {v}", &[next, cur])?;
        cur = next;
        lane *= 2;
    }
    if rd != cur {
        e.emit(VMV1R, &[rd, cur])?;
    }
    Ok(())
}
fn vle(bits: u32, store: bool) -> String {
    format!("{}{}.v {{v}}, ({{r}})", if store { "vse" } else { "vle" }, bits)
}
const VLE8: &str = "vle8.v {v}, ({r})";
const VSE8: &str = "vse8.v {v}, ({r})";
const VMV1R: &str = "vmv1r.v {v}, {v}";
/// the vector registers an interrupt handler keeps: the caller-saved
/// ones (v24..v31 are the allocator's, kept by it), and for a switch those too
const IRQ_V_SAVED: &[i64] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23];
const SRLI: &str = "srli {r}, {r}, {i 0..63}";
const SRAI: &str = "srai {r}, {r}, {i 0..63}";
const ANDI: &str = "andi {r}, {r}, {i -2048..2047}";

const ADDI: &str = "addi {r}, {r}, {i -2048..2047}";
const LD: &str = "ld {r}, {i -2048..2047}({r})";
const SD: &str = "sd {r}, {i -2048..2047}({r})";
const JAL: &str = "jal {r}, {i -1048576..1048574 /2}";
const BEQ: &str = "beq {r}, {r}, {i -4096..4094 /2}";
const BNE: &str = "bne {r}, {r}, {i -4096..4094 /2}";

enum FixTarget {
    Block(BlockId),
    Func(String),
    /// a data item, by name: an auipc/addi pair gets the distance to it
    Data(String),
    /// a function's entry, by name, the same way (a function value)
    FuncAddr(String),
}

struct Fixup {
    at: usize,
    values: Vec<i64>, // for JAL: [rd, offset]
    imm_slot: usize,
    target: FixTarget,
}

pub fn compile_with(module: &Module, enc: &Encoder, platform: &Platform) -> Result<Compiled, String> {
    compile_image(module, enc, platform, 0)
}

/// Compile a module for an image whose code begins at byte `origin`;
/// riscv's trap vector is the handler's entry itself (mtvec in direct
/// mode), so nothing here depends on the origin
pub fn compile_image(module: &Module, enc: &Encoder, platform: &Platform, origin: usize) -> Result<Compiled, String> {
    // group items through the thread's block
    let lowered = crate::ssa::lower_group_addrs(module);
    let module = &lowered;
    let natives = platform.natives(module);
    let mut code: Vec<u8> = Vec::new();
    let mut funcs = std::collections::HashMap::new();
    let mut call_fixups: Vec<Fixup> = Vec::new();

    let has_irq = module.funcs.iter().any(|f| f.name == crate::emit::IRQ);
    for func in &module.funcs {
        if func.name == crate::emit::TRAP {
            // the vector table for mtvec's vectored mode, 64-aligned:
            // entry 0 (exceptions) to __trap, the rest (interrupts, by
            // cause) to __irq when there is one
            while (origin + code.len()) % 64 != 0 {
                code.push(0);
            }
            for k in 0..16 {
                let target = if k > 0 && has_irq { crate::emit::IRQ } else { crate::emit::TRAP };
                let at = code.len();
                code.extend_from_slice(&enc.encode(JAL, &[ZERO, 0])?.to_le_bytes());
                call_fixups.push(Fixup { at, values: vec![ZERO, 0], imm_slot: 1, target: FixTarget::Func(target.into()) });
            }
        }
        funcs.insert(func.name.clone(), code.len());
        compile_function(func, enc, &natives, &mut code, &mut call_fixups)
            .map_err(|e| format!("{}: {}", func.name, e))?;
    }
    // data after the code, 16-aligned in memory (the alignment is
    // measured from the image's origin: a boot preamble precedes the code)
    let code_end = code.len();
    while (origin + code.len()) % 16 != 0 {
        code.push(0);
    }
    let (data, data_offsets) = crate::ssa::layout_data(module);
    let data_base = code.len();
    code.extend_from_slice(&data);

    for fix in call_fixups {
        match &fix.target {
            FixTarget::Func(name) => {
                let target = *funcs.get(name.as_str()).ok_or_else(|| format!("call to undefined function {}", name))?;
                let mut values = fix.values;
                values[fix.imm_slot] = target as i64 - fix.at as i64;
                let word = enc.encode(JAL, &values)?;
                code[fix.at..fix.at + 4].copy_from_slice(&word.to_le_bytes());
            }
            FixTarget::Data(_) | FixTarget::FuncAddr(_) => {
                let target = match &fix.target {
                    FixTarget::Data(name) => data_base + *data_offsets.get(name.as_str()).ok_or_else(|| format!("no data named {}", name))?,
                    FixTarget::FuncAddr(name) => *funcs.get(name.as_str()).ok_or_else(|| format!("addr of undefined function {}", name))?,
                    _ => unreachable!(),
                };
                // auipc takes the page distance, addi the rest (the low
                // part is signed, so the page rounds to nearest)
                let delta = target as i64 - fix.at as i64;
                let hi = (delta + 0x800) >> 12;
                let lo = delta - (hi << 12);
                let rd = fix.values[0];
                let w1 = enc.encode(AUIPC, &[rd, hi & 0xfffff])?;
                let w2 = enc.encode(ADDI, &[rd, rd, lo])?;
                code[fix.at..fix.at + 4].copy_from_slice(&w1.to_le_bytes());
                code[fix.at + 4..fix.at + 8].copy_from_slice(&w2.to_le_bytes());
            }
            FixTarget::Block(_) => unreachable!(),
        }
    }
    Ok(Compiled { code, funcs, code_end, data_base, writable_from: None })
}

/// where an argument or result crosses a call: a register of a class
/// (0 integer, 1 float, 2 vector), or the stack at an offset
#[derive(Clone, Copy)]
enum Abi {
    Reg(u8, i64),
    Stack(u8, i64),
}

struct RvEmit<'a> {
    enc: &'a Encoder,
    func: &'a Function,
    natives: &'a Natives,
    code: &'a mut Vec<u8>,
    frame: i64,
    alloc: &'a regalloc::Alloc,
    /// per value: the platform's register class (`f`, `v`), if any
    classes: Vec<Option<String>>,
    /// per value: a vector kept whole, 128 bits in its v register
    vecs: Vec<bool>,
    spill_base: i64,
    /// a spill slot is 8 bytes, or 16 when the function has vectors
    slot_size: i64,
    /// while a call's stack arguments are below sp: how far sp moved
    sp_adjust: i64,
    /// a trap or interrupt handler: (the frame area for the interrupted
    /// code's registers, an interrupt — a0 goes back too and the float
    /// scratch registers are kept as well); it leaves by mret
    trap: Option<(i64, bool)>,
    /// the registers a handler keeps (integer, float), see TRAP_SAVED
    saved: (Vec<i64>, Vec<i64>),
    /// the vector registers a handler keeps, and where they start
    vsaved: Vec<i64>,
    vsaved_base: i64,
    /// each `scratch` value's offset from sp
    scratch: std::collections::HashMap<ValueId, i64>,
    /// the block being emitted: a jump to the next one is a fall-through
    cur: usize,
    block_offsets: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
}

/// the caller-saved registers a trap handler preserves: t0-t2, a0-a7,
/// t3-t6; ra is in the frame already
const TRAP_SAVED: &[i64] = &[5, 6, 7, 10, 11, 12, 13, 14, 15, 16, 17, 28, 29, 30, 31];
/// the float registers an interrupt handler keeps: ft0-ft7, fa0-fa7,
/// ft8-ft11 (the fs registers are the allocator's, kept by it)
const IRQ_FP_SAVED: &[i64] = &[0, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 16, 17, 28, 29, 30, 31];
/// what a task switch keeps on top of those: the callee-saved s0-s11
/// and fs0-fs11 — the whole file is the task's
const SWITCH_SAVED: &[i64] = &[8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
const SWITCH_FP_SAVED: &[i64] = &[8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];

impl RvEmit<'_> {
    fn emit(&mut self, template: &str, values: &[i64]) -> Result<usize, String> {
        let at = self.code.len();
        let word = self.enc.encode(template, values)?;
        self.code.extend_from_slice(&word.to_le_bytes());
        Ok(at)
    }

    fn slot_off(&self, idx: usize) -> i64 {
        self.spill_base + self.slot_size * idx as i64 + self.sp_adjust
    }

    /// does v live in a float register?
    fn is_f(&self, v: ValueId) -> bool {
        self.classes[v.0 as usize].is_some() && !self.vecs[v.0 as usize]
    }

    /// is v a vector kept whole (in a v register)?
    fn is_v(&self, v: ValueId) -> bool {
        self.vecs[v.0 as usize]
    }

    /// a vector's shape: (bits per lane, lanes) — a u1xN holds each lane
    /// as 0 or 1 in a 128/N-bit lane
    fn shape(&self, v: ValueId) -> (u32, u32) {
        let ty = self.func.ty(v);
        let (lane, n) = self.func.vector(ty).unwrap();
        let total = if self.func.width(lane) == Some(1) { 128 } else { self.func.width(ty).unwrap() };
        (total / n, n)
    }

    fn tyname(&self, v: ValueId) -> String {
        self.func.tyname(self.func.ty(v))
    }

    /// the vtype for a vector's shape
    fn vset(&mut self, v: ValueId) -> Result<(), String> {
        let (bits, n) = self.shape(v);
        self.emit(vset(bits, n)?, &[]).map(|_| ())
    }

    /// a whole vector register to or from a frame slot: sixteen bytes at
    /// sp + off, the address through t2
    fn vslot(&mut self, load: bool, vr: i64, off: i64) -> Result<(), String> {
        self.emit(ADDI, &[T2, SP, off])?;
        self.emit(VSET_E8, &[])?;
        self.emit(if load { VLE8 } else { VSE8 }, &[vr, T2]).map(|_| ())
    }

    /// the vector register holding v: its own, or the spill slot reloaded
    /// into `vscratch`
    fn src_vreg(&mut self, v: ValueId, vscratch: i64) -> Result<i64, String> {
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => Ok(r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.vslot(true, vscratch, off)?;
                Ok(vscratch)
            }
        }
    }

    fn dst_vreg(&self, v: ValueId, vscratch: i64) -> i64 {
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => r,
            Loc::Slot(_) => vscratch,
        }
    }

    fn finish_v(&mut self, v: ValueId, vr: i64) -> Result<(), String> {
        if let Loc::Slot(i) = self.alloc.loc[v.0 as usize] {
            let off = self.slot_off(i);
            self.vslot(false, vr, off)?;
        }
        Ok(())
    }

    /// x <- the bits of a float register; fmv.x.w sign-extends, so a
    /// 32-bit value is re-normalized to its (unsigned) canonical form
    fn f_to_x(&mut self, x: i64, fr: i64, v: ValueId) -> Result<(), String> {
        let r = self.repr(v);
        if r.container() == 32 {
            self.emit("fmv.x.w {r}, {f}", &[x, fr])?;
            self.norm(x, x, r)
        } else {
            self.emit("fmv.x.d {r}, {f}", &[x, fr]).map(|_| ())
        }
    }

    fn x_to_f(&mut self, fr: i64, x: i64, v: ValueId) -> Result<(), String> {
        let t = if self.repr(v).container() == 32 { "fmv.w.x {f}, {r}" } else { "fmv.d.x {f}, {r}" };
        self.emit(t, &[fr, x]).map(|_| ())
    }

    /// the float register holding v: its own, or the spill slot reloaded
    /// into `fscratch` (ft0..ft3 = f0..f3: caller-saved, never allocated)
    fn src_freg(&mut self, v: ValueId, fscratch: i64) -> Result<i64, String> {
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => Ok(r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(FLD, &[fscratch, off, SP])?;
                Ok(fscratch)
            }
        }
    }

    fn dst_freg(&self, v: ValueId, fscratch: i64) -> i64 {
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => r,
            Loc::Slot(_) => fscratch,
        }
    }

    fn finish_f(&mut self, v: ValueId, fr: i64) -> Result<(), String> {
        if let Loc::Slot(i) = self.alloc.loc[v.0 as usize] {
            let off = self.slot_off(i);
            self.emit(FSD, &[fr, off, SP])?;
        }
        Ok(())
    }

    fn src_reg(&mut self, v: ValueId, scratch: i64) -> Result<i64, String> {
        if self.is_f(v) {
            let fr = self.src_freg(v, 0)?;
            self.f_to_x(scratch, fr, v)?;
            return Ok(scratch);
        }
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => Ok(r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LD, &[scratch, off, SP])?;
                Ok(scratch)
            }
        }
    }

    fn dst_reg(&self, v: ValueId, scratch: i64) -> i64 {
        if self.is_f(v) {
            return scratch;
        }
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => r,
            Loc::Slot(_) => scratch,
        }
    }

    fn finish(&mut self, v: ValueId, reg: i64) -> Result<(), String> {
        if self.is_f(v) {
            let fr = self.dst_freg(v, 3);
            self.x_to_f(fr, reg, v)?;
            return self.finish_f(v, fr);
        }
        if let Loc::Slot(i) = self.alloc.loc[v.0 as usize] {
            let off = self.slot_off(i);
            self.emit(SD, &[reg, off, SP])?;
        }
        Ok(())
    }

    fn mov(&mut self, dst: i64, src: i64) -> Result<(), String> {
        if dst != src {
            self.emit(ADDI, &[dst, src, 0])?;
        }
        Ok(())
    }

    /// place v into a specific register (targets a0..a7 / staging; sources
    /// are pool registers or slots — disjoint, so sequences never clobber)
    /// which file a value crosses a call in: 0 an integer register, 1 a
    /// float register, 2 a vector register
    fn kind(&self, v: ValueId) -> u8 {
        if self.is_v(v) { 2 } else { self.is_f(v) as u8 }
    }

    /// the calling convention: integers in a0.. in their order, floats in
    /// fa0.. in theirs, vectors in v8.. in theirs — each class counts its
    /// own, eight of each; the rest on the stack, in order, 8 bytes each
    /// and a vector 16, from the caller's sp up
    fn abi_regs(&self, vals: &[ValueId]) -> Result<Vec<Abi>, String> {
        let mut n = [0i64; 3];
        let mut off = 0i64;
        let mut out = Vec::new();
        for &v in vals {
            let k = self.kind(v);
            if n[k as usize] < 8 {
                out.push(Abi::Reg(k, [A0, FA0, V_ARGS][k as usize] + n[k as usize]));
                n[k as usize] += 1;
            } else {
                if k == 2 {
                    off = (off + 15) & !15;
                }
                out.push(Abi::Stack(k, off));
                off += if k == 2 { 16 } else { 8 };
            }
        }
        Ok(out)
    }

    /// what a call's stack arguments take below sp, 16-aligned
    fn stack_args(abi: &[Abi]) -> i64 {
        let end = abi.iter().map(|a| match a { Abi::Stack(k, off) => off + if *k == 2 { 16 } else { 8 }, _ => 0 }).max().unwrap_or(0);
        (end + 15) & !15
    }

    /// v to where the convention puts it: a register, or the stack
    /// below sp (sp already lowered by `stack_args`)
    fn arg_out(&mut self, abi: Abi, v: ValueId) -> Result<(), String> {
        match abi {
            Abi::Reg(k, r) => self.arg_to(k, r, v),
            Abi::Stack(0, off) => {
                let r = self.src_reg(v, T0)?;
                self.emit(SD, &[r, off, SP]).map(|_| ())
            }
            Abi::Stack(1, off) => {
                let fr = self.src_freg(v, 0)?;
                self.emit(FSD, &[fr, off, SP]).map(|_| ())
            }
            Abi::Stack(_, off) => {
                let vr = self.src_vreg(v, 20)?;
                self.emit(ADDI, &[T2, SP, off])?;
                self.emit(VSET_E8, &[])?;
                self.emit(VSE8, &[vr, T2]).map(|_| ())
            }
        }
    }

    /// a result to where the convention wants it: a register, or the
    /// stack above this frame — the caller's area
    fn res_out(&mut self, abi: Abi, v: ValueId) -> Result<(), String> {
        match abi {
            Abi::Reg(k, r) => self.arg_to(k, r, v),
            Abi::Stack(k, off) => {
                self.emit(ADDI, &[T2, SP, self.frame])?;
                match k {
                    0 => {
                        let r = self.src_reg(v, T0)?;
                        self.emit(SD, &[r, off, T2]).map(|_| ())
                    }
                    1 => {
                        let fr = self.src_freg(v, 0)?;
                        self.emit(FSD, &[fr, off, T2]).map(|_| ())
                    }
                    _ => {
                        let vr = self.src_vreg(v, 20)?;
                        self.emit(ADDI, &[T2, T2, off])?;
                        self.emit(VSET_E8, &[])?;
                        self.emit(VSE8, &[vr, T2]).map(|_| ())
                    }
                }
            }
        }
    }

    /// a result from where the callee left it: a register, or the area
    /// below sp (still lowered) it was given
    fn res_in(&mut self, abi: Abi, v: ValueId) -> Result<(), String> {
        match abi {
            Abi::Reg(k, r) => self.arg_from(k, r, v),
            Abi::Stack(0, off) => {
                self.emit(LD, &[T0, off, SP])?;
                self.value_from(v, T0)
            }
            Abi::Stack(1, off) => {
                self.emit(FLD, &[0, off, SP])?;
                self.arg_from(1, 0, v)
            }
            Abi::Stack(_, off) => {
                self.emit(ADDI, &[T2, SP, off])?;
                self.emit(VSET_E8, &[])?;
                self.emit(VLE8, &[20, T2])?;
                self.arg_from(2, 20, v)
            }
        }
    }

    /// v from where the convention put it: a register, or the stack
    /// above this frame (the address through t2: frame plus offset may
    /// pass an immediate)
    fn arg_in(&mut self, abi: Abi, v: ValueId) -> Result<(), String> {
        match abi {
            Abi::Reg(k, r) => self.arg_from(k, r, v),
            Abi::Stack(k, off) => {
                self.emit(ADDI, &[T2, SP, self.frame])?;
                match k {
                    0 => {
                        self.emit(LD, &[T0, off, T2])?;
                        self.value_from(v, T0)
                    }
                    1 => {
                        self.emit(FLD, &[0, off, T2])?;
                        self.arg_from(1, 0, v)
                    }
                    _ => {
                        self.emit(ADDI, &[T2, T2, off])?;
                        self.emit(VSET_E8, &[])?;
                        self.emit(VLE8, &[20, T2])?;
                        self.arg_from(2, 20, v)
                    }
                }
            }
        }
    }

    /// v into the register the convention gives it (a caller-saved one,
    /// never where a value lives)
    fn arg_to(&mut self, kind: u8, reg: i64, v: ValueId) -> Result<(), String> {
        match kind {
            0 => self.value_to(reg, v),
            1 => {
                let fr = self.src_freg(v, 0)?;
                if fr != reg {
                    self.emit(FMV_D, &[reg, fr])?;
                }
                Ok(())
            }
            _ => {
                let vr = self.src_vreg(v, 20)?;
                if vr != reg {
                    self.emit(VMV1R, &[reg, vr])?;
                }
                Ok(())
            }
        }
    }

    /// the register the convention put v in, into v's own place
    fn arg_from(&mut self, kind: u8, reg: i64, v: ValueId) -> Result<(), String> {
        match kind {
            0 => self.value_from(v, reg),
            1 => {
                let fr = self.dst_freg(v, 0);
                if fr != reg {
                    self.emit(FMV_D, &[fr, reg])?;
                }
                self.finish_f(v, fr)
            }
            _ => {
                let vr = self.dst_vreg(v, 20);
                if vr != reg {
                    self.emit(VMV1R, &[vr, reg])?;
                }
                self.finish_v(v, vr)
            }
        }
    }

    fn value_to(&mut self, target: i64, v: ValueId) -> Result<(), String> {
        if self.is_f(v) {
            let fr = self.src_freg(v, 0)?;
            return self.f_to_x(target, fr, v);
        }
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => self.mov(target, r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LD, &[target, off, SP]).map(|_| ())
            }
        }
    }

    fn value_from(&mut self, v: ValueId, source: i64) -> Result<(), String> {
        if self.is_f(v) {
            let fr = self.dst_freg(v, 0);
            self.x_to_f(fr, source, v)?;
            return self.finish_f(v, fr);
        }
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => self.mov(r, source),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(SD, &[source, off, SP]).map(|_| ())
            }
        }
    }

    fn is_next(&self, target: BlockId) -> bool {
        target.0 as usize == self.cur + 1
    }

    /// unconditional jump to an SSA block, unless it is the next one laid
    /// out — then the code simply falls into it
    fn goto(&mut self, target: BlockId) -> Result<(), String> {
        if self.is_next(target) {
            return Ok(());
        }
        self.jump(target)
    }

    /// unconditional jump to an SSA block (offset patched later)
    fn jump(&mut self, target: BlockId) -> Result<(), String> {
        let at = self.emit(JAL, &[ZERO, 0])?;
        self.fixups.push(Fixup {
            at,
            values: vec![ZERO, 0],
            imm_slot: 1,
            target: FixTarget::Block(target),
        });
        Ok(())
    }

    /// one location-to-location move (registers or spill slots), in the
    /// integer file (0), the float file (1) or the vector file (2)
    fn loc_move(&mut self, kind: u8, dst: Loc, src: Loc) -> Result<(), String> {
        if kind == 2 {
            return match (dst, src) {
                (Loc::Reg(d), Loc::Reg(s)) => {
                    if d != s {
                        self.emit(VMV1R, &[d, s])?;
                    }
                    Ok(())
                }
                (Loc::Reg(d), Loc::Slot(s)) => self.vslot(true, d, self.slot_off(s)),
                (Loc::Slot(d), Loc::Reg(s)) => self.vslot(false, s, self.slot_off(d)),
                (Loc::Slot(d), Loc::Slot(s)) => {
                    // transit through v21; v20 stays free for cycle breaking
                    self.vslot(true, 21, self.slot_off(s))?;
                    self.vslot(false, 21, self.slot_off(d))
                }
            };
        }
        if kind == 1 {
            return match (dst, src) {
                (Loc::Reg(d), Loc::Reg(s)) => {
                    if d != s {
                        self.emit(FMV_D, &[d, s])?;
                    }
                    Ok(())
                }
                (Loc::Reg(d), Loc::Slot(s)) => self.emit(FLD, &[d, self.slot_off(s), SP]).map(|_| ()),
                (Loc::Slot(d), Loc::Reg(s)) => self.emit(FSD, &[s, self.slot_off(d), SP]).map(|_| ()),
                (Loc::Slot(d), Loc::Slot(s)) => {
                    // transit through ft1; ft0 stays free for cycle breaking
                    self.emit(FLD, &[1, self.slot_off(s), SP])?;
                    self.emit(FSD, &[1, self.slot_off(d), SP]).map(|_| ())
                }
            };
        }
        match (dst, src) {
            (Loc::Reg(d), Loc::Reg(s)) => self.mov(d, s),
            (Loc::Reg(d), Loc::Slot(s)) => {
                let off = self.slot_off(s);
                self.emit(LD, &[d, off, SP]).map(|_| ())
            }
            (Loc::Slot(d), Loc::Reg(s)) => {
                let off = self.slot_off(d);
                self.emit(SD, &[s, off, SP]).map(|_| ())
            }
            (Loc::Slot(d), Loc::Slot(s)) => {
                // transit through t1; t0 stays free for cycle breaking
                self.emit(LD, &[T1, self.slot_off(s), SP])?;
                self.emit(SD, &[T1, self.slot_off(d), SP]).map(|_| ())
            }
        }
    }

    /// branch arguments as a parallel move: emit moves whose destination
    /// nobody still reads; break cycles by stashing one source in t0
    fn branch_args(&mut self, target: BlockId, args: &[ValueId]) -> Result<(), String> {
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        // (file: 0 integer, 1 float, 2 vector; destination, source): three
        // files that never alias
        let mut pending: Vec<(u8, Loc, Loc)> = params
            .iter()
            .zip(args)
            .map(|(&p, &a)| (if self.is_v(p) { 2 } else { self.is_f(p) as u8 }, self.alloc.loc[p.0 as usize], self.alloc.loc[a.0 as usize]))
            .filter(|(_, d, s)| d != s)
            .collect();
        while !pending.is_empty() {
            if let Some(i) =
                (0..pending.len()).find(|&i| !pending.iter().any(|&(f, _, s)| f == pending[i].0 && s == pending[i].1))
            {
                let (f, d, s) = pending.swap_remove(i);
                self.loc_move(f, d, s)?;
            } else {
                let (f, _, s) = pending[0];
                let scratch = Loc::Reg([T0, 0, 20][f as usize]);
                self.loc_move(f, scratch, s)?;
                for m in pending.iter_mut().filter(|m| m.0 == f && m.2 == s) {
                    m.2 = scratch;
                }
            }
        }
        Ok(())
    }

    fn repr(&self, v: ValueId) -> Repr {
        self.func.repr(self.func.ty(v))
    }

    /// rd = the canonical 64-bit form of the low N bits of rs
    fn norm(&mut self, rd: i64, rs: i64, r: Repr) -> Result<(), String> {
        let n = r.bits();
        match (r.signed(), n) {
            (_, 64) => self.mov(rd, rs),
            (true, 32) => self.emit("addiw {r}, {r}, {i -2048..2047}", &[rd, rs, 0]).map(|_| ()),
            (false, n) if n <= 11 => self.emit(ANDI, &[rd, rs, (1i64 << n) - 1]).map(|_| ()),
            (signed, n) => {
                let k = 64 - n as i64;
                self.emit(SLLI, &[rd, rs, k])?;
                self.emit(if signed { SRAI } else { SRLI }, &[rd, rd, k]).map(|_| ())
            }
        }
    }

    /// rd = rs converted between canonical forms
    fn cast(&mut self, rd: i64, rs: i64, from: Repr, to: Repr) -> Result<(), String> {
        if from.fits_in(to) {
            self.mov(rd, rs)
        } else {
            self.norm(rd, rs, to)
        }
    }

    /// rd = the `w`-bit field at `off` in rs, sign- or zero-extended
    fn extract(&mut self, rd: i64, rs: i64, off: u32, w: u32, signed: bool) -> Result<(), String> {
        self.emit(SLLI, &[rd, rs, 64 - (off + w) as i64])?;
        self.emit(if signed { SRAI } else { SRLI }, &[rd, rd, 64 - w as i64]).map(|_| ())
    }

    /// rd = the low `w` bits of rv, moved to bit `off`, zero elsewhere
    fn place(&mut self, rd: i64, rv: i64, off: u32, w: u32) -> Result<(), String> {
        self.emit(SLLI, &[rd, rv, 64 - w as i64])?;
        self.emit(SRLI, &[rd, rd, 64 - (off + w) as i64]).map(|_| ())
    }

    /// the base register and 12-bit immediate for an access at base +
    /// off + index * step: the immediate form when the offset fits,
    /// else the address computed into `scratch` (`scratch2` for the
    /// scaled index or a wide offset)
    /// the whole address of a vector access in one register: vle/vse take
    /// no offset
    fn vector_address(&mut self, base: ValueId, off: i64, index: Option<(ValueId, u32)>) -> Result<i64, String> {
        let (ra, imm) = self.address(base, off, index, T0, T2)?;
        if imm == 0 {
            return Ok(ra);
        }
        self.emit(ADDI, &[T0, ra, imm])?;
        Ok(T0)
    }

    fn address(&mut self, base: ValueId, off: i64, index: Option<(ValueId, u32)>, scratch: i64, scratch2: i64) -> Result<(i64, i64), String> {
        let mut rb = self.src_reg(base, scratch)?;
        if let Some((i, step)) = index {
            let ri = self.src_reg(i, scratch2)?;
            if step.is_power_of_two() && step > 1 {
                self.emit(SLLI, &[scratch2, ri, step.trailing_zeros() as i64])?;
                self.emit("add {r}, {r}, {r}", &[scratch, rb, scratch2])?;
            } else if step > 1 {
                // a stride that is no power of two (an array of structs): the
                // index times each set bit, summed — no multiplier needed, so
                // a core without M is served the same
                let mut acc: Option<i64> = None;
                let mut bits = step;
                while bits != 0 {
                    let k = bits.trailing_zeros();
                    bits &= bits - 1;
                    let term = if acc.is_none() { scratch2 } else { scratch };
                    if k == 0 {
                        self.mov(term, ri)?;
                    } else {
                        self.emit(SLLI, &[term, ri, k as i64])?;
                    }
                    match acc {
                        None => acc = Some(term),
                        Some(a) => {
                            self.emit("add {r}, {r}, {r}", &[a, a, term])?;
                        }
                    }
                }
                let rb2 = self.src_reg(base, scratch)?;
                self.emit("add {r}, {r}, {r}", &[scratch, rb2, scratch2])?;
            } else {
                self.emit("add {r}, {r}, {r}", &[scratch, rb, ri])?;
            }
            rb = scratch;
        }
        if (-2048..=2047).contains(&off) {
            return Ok((rb, off));
        }
        self.iconst(scratch2, off)?;
        self.emit("add {r}, {r}, {r}", &[scratch, rb, scratch2])?;
        Ok((scratch, 0))
    }

    fn epilogue(&mut self) -> Result<(), String> {
        // a task switch's result is the frame to go back from
        if let Some((_, irq)) = self.trap {
            if irq && !self.func.rets.is_empty() {
                self.emit(ADDI, &[SP, A0, 0])?;
            }
        }
        // the vector registers first: their addresses go through t2
        let vbase = self.vsaved_base;
        for (k, &vr) in self.vsaved.clone().iter().enumerate() {
            self.vslot(true, vr, vbase + 16 * k as i64)?;
        }
        let pool_v_base = v_save_base(self.alloc.used_regs.len(), self.alloc.used_by_class[1].len());
        for (k, &vr) in self.alloc.used_by_class[2].clone().iter().enumerate() {
            self.vslot(true, vr, pool_v_base + 16 * k as i64)?;
        }
        for (k, &r) in self.alloc.used_regs.clone().iter().enumerate() {
            self.emit(LD, &[r, 16 + 8 * k as i64, SP])?;
        }
        let fbase = 16 + 8 * self.alloc.used_regs.len() as i64;
        for (k, &fr) in self.alloc.used_by_class[1].clone().iter().enumerate() {
            self.emit(FLD, &[fr, fbase + 8 * k as i64, SP])?;
        }
        if let Some((base, irq)) = self.trap {
            // a trap's result is in a0; everything else goes back as it was
            let (ints, fp) = self.saved.clone();
            let fp_base = base + 8 * ints.len() as i64;
            for (k, &fr) in fp.iter().enumerate() {
                self.emit(FLD, &[fr, fp_base + 8 * k as i64, SP])?;
            }
            for (k, &r) in ints.iter().enumerate() {
                if r != A0 || irq {
                    self.emit(LD, &[r, base + 8 * k as i64, SP])?;
                }
            }
        }
        self.emit(LD, &[RA, 0, SP])?;
        self.emit(ADDI, &[SP, SP, self.frame])?;
        if self.trap.is_some() {
            self.emit("mret", &[])?;
        } else {
            self.emit("jalr {r}, {i -2048..2047}({r})", &[ZERO, 0, RA])?;
        }
        Ok(())
    }

    /// materialize a 64-bit constant into `reg`
    fn iconst(&mut self, reg: i64, imm: i64) -> Result<(), String> {
        if (-2048..=2047).contains(&imm) {
            self.emit(ADDI, &[reg, ZERO, imm])?;
            return Ok(());
        }
        // byte-by-byte build, MSB first: always correct, never clever
        let v = imm as u64;
        let mut started = false;
        for i in (0..8).rev() {
            let byte = ((v >> (8 * i)) & 0xff) as i64;
            if !started {
                if byte == 0 {
                    continue;
                }
                self.emit(ADDI, &[reg, ZERO, byte])?;
                started = true;
            } else {
                self.emit(SLLI, &[reg, reg, 8])?;
                if byte != 0 {
                    self.emit(ADDI, &[reg, reg, byte])?;
                }
            }
        }
        // remaining shift if low bytes were zero is handled in the loop;
        // a value whose top byte >= 0x80 still works: addi sign-extends the
        // first byte only if > 2047, which 0..255 never is
        if !started {
            self.emit(ADDI, &[reg, ZERO, 0])?;
        }
        Ok(())
    }
}

fn compile_function(
    func: &Function,
    enc: &Encoder,
    natives: &Natives,
    code: &mut Vec<u8>,
    call_fixups: &mut Vec<Fixup>,
) -> Result<(), String> {
    if let Some(native) = natives.get(&func.name) {
        // this function *is* a platform instruction: arguments arrive
        // where the convention puts them — integers in a0.., floats in
        // fa0.. — the rule runs, its result in a0 or fa0
        let mut seq: Vec<(&str, Vec<i64>)> = Vec::new();
        let mut args = Vec::new();
        let (mut nx, mut nf) = (0, 0);
        for class in &native.arg_class {
            if class.is_some() {
                args.push(FA0 + nf);
                nf += 1;
            } else {
                args.push(A0 + nx);
                nx += 1;
            }
        }
        let rd = if native.ret_class.is_some() { FA0 } else { A0 };
        seq.extend(rule_seq(enc, native, rd, &args)?);
        for (t, v) in seq {
            code.extend_from_slice(&enc.encode(t, &v)?.to_le_bytes());
        }
        code.extend_from_slice(&enc.encode("jalr {r}, {i -2048..2047}({r})", &[ZERO, 0, RA])?.to_le_bytes());
        return Ok(());
    }
    let classes: Vec<Option<String>> = func.values.iter().map(|v| natives.class_of(func, v.ty).map(str::to_string)).collect();
    let vecs: Vec<bool> = func.values.iter().map(|v| func.vector(v.ty).is_some()).collect();
    if let Some(i) = (0..func.values.len()).find(|&i| vecs[i] && classes[i].as_deref() != Some("v")) {
        return Err(format!("a vector ({}) the platform has no register class for", func.tyname(func.values[i].ty)));
    }
    let class_idx: Vec<usize> = classes.iter().map(|c| match c.as_deref() { None => 0, Some("v") => 2, Some(_) => 1 }).collect();
    let alloc = regalloc::allocate_classes(func, &class_idx, &[REG_POOL, F_POOL, V_POOL]);
    // the callee-saved area: ra, the integer and float registers at 8
    // each, then the vector registers at 16 each, 16-aligned
    let nv = alloc.used_by_class[2].len();
    let saved_end = if nv > 0 { v_save_base(alloc.used_regs.len(), alloc.used_by_class[1].len()) + 16 * nv as i64 } else { 16 + 8 * (alloc.used_regs.len() + alloc.used_by_class[1].len()) as i64 };
    let slot_size: i64 = if vecs.iter().any(|&b| b) { 16 } else { 8 };
    let has_v = natives.classes.values().any(|c| c == "v");
    let irq = func.name == crate::emit::IRQ;
    if irq && !(func.params.is_empty() && func.rets.is_empty() || func.params.len() == 1 && func.rets.len() == 1 && func.ty(func.params[0]) == Type::Ptr && func.rets[0] == Type::Ptr) {
        return Err("__irq is fn __irq() or fn __irq(sp: ptr) -> ptr".into());
    }
    let trap = (func.name == crate::emit::TRAP || irq).then_some((saved_end, irq));
    let switching = irq && !func.params.is_empty();
    let mut saved: (Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new());
    let mut vsaved: Vec<i64> = Vec::new();
    if trap.is_some() {
        saved.0 = TRAP_SAVED.to_vec();
        if irq && !natives.classes.is_empty() {
            saved.1 = IRQ_FP_SAVED.to_vec();
        }
        if irq && has_v {
            vsaved = IRQ_V_SAVED.to_vec();
        }
        if switching {
            saved.0.extend_from_slice(SWITCH_SAVED);
            if !natives.classes.is_empty() {
                saved.1.extend_from_slice(SWITCH_FP_SAVED);
            }
            if has_v {
                vsaved.extend_from_slice(V_POOL);
            }
        }
    }
    let vsaved_base = (saved_end + 8 * (saved.0.len() + saved.1.len()) as i64 + 15) & !15;
    let trap_end = vsaved_base + 16 * vsaved.len() as i64;
    let spill_base = if trap.is_some() { trap_end } else { saved_end };
    let spill_base = if slot_size == 16 { (spill_base + 15) & !15 } else { spill_base };
    let (scratch, scratch_end) = crate::emit::scratch_layout(func, (spill_base + slot_size * alloc.nslots as i64 + 15) & !15);
    let frame = scratch_end;
    if frame > 2047 {
        return Err(format!("function needs a {}-byte frame; 2047 is the most for now", frame));
    }

    let mut e = RvEmit {
        enc,
        func,
        natives,
        code,
        frame,
        alloc: &alloc,
        classes,
        vecs,
        spill_base,
        slot_size,
        sp_adjust: 0,
        trap,
        saved: saved.clone(),
        vsaved: vsaved.clone(),
        vsaved_base,
        scratch,
        cur: 0,
        block_offsets: vec![None; func.blocks.len()],
        fixups: Vec::new(),
    };

    e.emit(ADDI, &[SP, SP, -frame])?;
    e.emit(SD, &[RA, 0, SP])?;
    for (k, &r) in alloc.used_regs.iter().enumerate() {
        e.emit(SD, &[r, 16 + 8 * k as i64, SP])?;
    }
    let fbase = 16 + 8 * alloc.used_regs.len() as i64;
    for (k, &fr) in alloc.used_by_class[1].iter().enumerate() {
        e.emit(FSD, &[fr, fbase + 8 * k as i64, SP])?;
    }
    let pool_v_base = v_save_base(alloc.used_regs.len(), alloc.used_by_class[1].len());
    for (k, &vr) in alloc.used_by_class[2].iter().enumerate() {
        e.vslot(false, vr, pool_v_base + 16 * k as i64)?;
    }
    if let Some((base, irq)) = trap {
        for (k, &r) in saved.0.iter().enumerate() {
            e.emit(SD, &[r, base + 8 * k as i64, SP])?;
        }
        let fp_base = base + 8 * saved.0.len() as i64;
        for (k, &fr) in saved.1.iter().enumerate() {
            e.emit(FSD, &[fr, fp_base + 8 * k as i64, SP])?;
        }
        for (k, &vr) in vsaved.iter().enumerate() {
            e.vslot(false, vr, vsaved_base + 16 * k as i64)?;
        }
        // an interrupt handler that switches tasks is given its frame
        if irq && !func.params.is_empty() {
            e.emit(ADDI, &[A0, SP, 0])?;
        }
    }
    for (&p, abi) in func.params.iter().zip(e.abi_regs(&func.params)?) {
        e.arg_in(abi, p)?;
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        e.block_offsets[bi] = Some(e.code.len());
        e.cur = bi;
        for inst in &block.insts {
            compile_inst(&mut e, inst)?;
        }
    }

    for fix in std::mem::take(&mut e.fixups) {
        match fix.target {
            FixTarget::Block(b) => {
                let target = e.block_offsets[b.0 as usize].unwrap();
                let mut values = fix.values;
                values[fix.imm_slot] = target as i64 - fix.at as i64;
                let word = e.enc.encode(JAL, &values)?;
                e.code[fix.at..fix.at + 4].copy_from_slice(&word.to_le_bytes());
            }
            FixTarget::Func(_) | FixTarget::Data(_) | FixTarget::FuncAddr(_) => call_fixups.push(fix),
        }
    }
    Ok(())
}

/// the instruction sequence for a platform rule: the arguments and the
/// result in the registers given, each in its own class's file
/// the registers a rule's temporaries `t0`..`t3` are: t3..t6, which no
/// argument or result of a rule ever lands in
const RULE_TEMPS: &[i64] = &[28, 29, 30, 31];

/// where the vector registers' save area begins: after ra and the
/// integer and float registers, 16-aligned
fn v_save_base(nints: usize, nfloats: usize) -> i64 {
    (16 + 8 * (nints + nfloats) as i64 + 15) & !15
}

fn rule_seq<'a>(enc: &'a Encoder, native: &Native, rd: i64, args: &[i64]) -> Result<Vec<(&'a str, Vec<i64>)>, String> {
    let templates = enc.templates();
    let mut seq = Vec::new();
    for line in &native.rule.lines {
        if line.mnemonic == "none" {
            continue;
        }
        let (t, vals) = crate::platform::resolve(native, line, &templates)?;
        let slots = crate::platform::template_slots(t);
        let v = vals
            .into_iter()
            .enumerate()
            .map(|(i, (op, v))| {
                let r = match op {
                    Operand::Arg(i) => args[i],
                    Operand::Ret => rd,
                    Operand::Tmp(k) => {
                        if native.tmp_class.get(k).is_some_and(|c| c.is_some()) {
                            V_TEMPS[k]
                        } else {
                            RULE_TEMPS[k]
                        }
                    }
                    Operand::Lit(_) => v,
                };
                // a `vn` slot is the seed's v1..v31: index 0 is v1
                if slots.get(i).is_some_and(|s| s.starts_with("vn")) { r - 1 } else { r }
            })
            .collect();
        seq.push((t, v));
    }
    Ok(seq)
}

/// an operation on whole vectors: the platform's rule for its signature,
/// every operand in the vector file
fn vector_op(e: &mut RvEmit, sig: &str, dst: Option<ValueId>, args: &[ValueId]) -> Result<(), String> {
    let natives: &Natives = e.natives;
    let native = natives.vector(sig).ok_or_else(|| format!("no rule for {} on this platform", sig))?;
    let mut regs = Vec::new();
    for (j, &a) in args.iter().enumerate() {
        regs.push(if e.is_v(a) { e.src_vreg(a, 20 + j as i64)? } else { e.src_reg(a, [T0, T1, T2][j])? });
    }
    let Some(d) = dst else { return Ok(()) };
    let rd = if e.is_v(d) { e.dst_vreg(d, 23) } else { e.dst_reg(d, T0) };
    for (t, v) in rule_seq(e.enc, native, rd, &regs)? {
        e.emit(t, &v)?;
    }
    if e.is_v(d) { e.finish_v(d, rd) } else { e.finish(d, rd) }
}

/// the instructions on vectors kept whole: lanes moved through v0..v23
/// and x registers, memory by vle/vse at a lane's alignment, everything
/// else the platform's rule for the operation's signature; None when the
/// instruction has no vector in it
fn compile_vector_inst(e: &mut RvEmit, inst: &Inst) -> Option<Result<(), String>> {
    let r = match inst {
        Inst::Bin { op, dst, lhs, rhs } if e.is_v(*dst) => {
            let sig = format!("{}({}, {}) -> {}", op.name(), e.tyname(*lhs), e.tyname(*rhs), e.tyname(*dst));
            vector_op(e, &sig, Some(*dst), &[*lhs, *rhs])
        }
        Inst::ICmp { cond, dst, lhs, rhs } if e.is_v(*dst) => {
            let sig = format!("{}({}, {}) -> {}", cond.name(), e.tyname(*lhs), e.tyname(*rhs), e.tyname(*dst));
            vector_op(e, &sig, Some(*dst), &[*lhs, *rhs])
        }
        Inst::Cast { op, dst, src } if e.is_v(*dst) || e.is_v(*src) => {
            if !(e.is_v(*dst) && e.is_v(*src)) {
                return Some(Err(format!("a {} between a vector and a scalar ({} to {}): not yet", if *op == crate::ssa::CastOp::Conv { "conv" } else { "cast" }, e.tyname(*src), e.tyname(*dst))));
            }
            if *op == crate::ssa::CastOp::Conv {
                let sig = format!("conv({}) -> {}", e.tyname(*src), e.tyname(*dst));
                vector_op(e, &sig, Some(*dst), &[*src])
            } else {
                (|| {
                    let rs = e.src_vreg(*src, 20)?;
                    let rd = e.dst_vreg(*dst, 23);
                    if rd != rs {
                        e.emit(VMV1R, &[rd, rs])?;
                    }
                    e.finish_v(*dst, rd)
                })()
            }
        }
        // a library operation on vectors the platform has a rule for; a
        // call on vectors with no rule (a function of the program's) is a
        // call like any other, its vectors crossing in vector registers
        Inst::Call { dsts, callee, args } if (args.iter().any(|&a| e.is_v(a)) || dsts.iter().any(|&d| e.is_v(d))) && dsts.len() <= 1 && {
            let natives: &Natives = e.natives;
            let ret = dsts.first().map(|&d| e.tyname(d)).unwrap_or_else(|| "()".into());
            let argn: Vec<String> = args.iter().map(|&a| e.tyname(a)).collect();
            natives.vector(&natives.vector_sig(callee, &argn, &ret)).is_some()
        } => {
            let natives: &Natives = e.natives;
            let ret = dsts.first().map(|&d| e.tyname(d)).unwrap_or_else(|| "()".into());
            let argn: Vec<String> = args.iter().map(|&a| e.tyname(a)).collect();
            let sig = natives.vector_sig(callee, &argn, &ret);
            vector_op(e, &sig, dsts.first().copied(), args)
        }
        Inst::Get { dst, src, field } if e.is_v(*src) => (|| {
            let (bits, _) = e.shape(*src);
            let rs = e.src_vreg(*src, 20)?;
            let rd = e.dst_reg(*dst, T1);
            e.vset(*src)?;
            let from = if *field == 0 { rs } else {
                e.emit("vslidedown.vi {v}, {v}, {i 0..15}", &[21, rs, *field as i64])?;
                21
            };
            e.emit("vmv.x.s {r}, {v}", &[rd, from])?;
            // a narrow lane arrives sign-extended: the canonical form of
            // the lane's type
            if bits < 64 {
                let rr = e.repr(*dst);
                e.norm(rd, rd, rr)?;
            }
            e.finish(*dst, rd)
        })(),
        Inst::Set { dst, src, field, val } if e.is_v(*dst) => (|| {
            let rs = e.src_vreg(*src, 20)?;
            let rv = e.src_reg(*val, T1)?;
            e.vset(*dst)?;
            // the mask is lane `field`; merge a splat of the value over the source
            e.iconst(T0, 1 << *field)?;
            e.emit("vmv.s.x {v}, {r}", &[0, T0])?;
            e.emit("vmv.v.x {v}, {r}", &[21, rv])?;
            e.emit("vmerge.vvm {vn}, {v}, {v}, v0", &[22 - 1, rs, 21])?;
            let rd = e.dst_vreg(*dst, 23);
            e.emit(VMV1R, &[rd, 22])?;
            e.finish_v(*dst, rd)
        })(),
        Inst::Pack { dst, args } if e.is_v(*dst) => (|| {
            let rd = e.dst_vreg(*dst, 23);
            e.vset(*dst)?;
            if args.iter().all(|a| a == &args[0]) {
                // a splat
                let ra = e.src_reg(args[0], T0)?;
                e.emit("vmv.v.x {v}, {r}", &[rd, ra])?;
                return e.finish_v(*dst, rd);
            }
            // the last lane first, each slid up over the one before, between
            // v21 and v22 (a slide's destination may not be its source)
            let mut cur = 21;
            for &a in args.iter().rev() {
                let ra = e.src_reg(a, T0)?;
                let next = if cur == 21 { 22 } else { 21 };
                e.emit("vslide1up.vx {v}, {v}, {r}", &[next, cur, ra])?;
                cur = next;
            }
            e.emit(VMV1R, &[rd, cur])?;
            e.finish_v(*dst, rd)
        })(),
        Inst::Unpack { dsts, src } if e.is_v(*src) => (|| {
            let (bits, _) = e.shape(*src);
            let rs = e.src_vreg(*src, 20)?;
            e.vset(*src)?;
            for (k, &d) in dsts.iter().enumerate() {
                let rd = e.dst_reg(d, T1);
                let from = if k == 0 { rs } else {
                    e.emit("vslidedown.vi {v}, {v}, {i 0..15}", &[21, rs, k as i64])?;
                    21
                };
                e.emit("vmv.x.s {r}, {v}", &[rd, from])?;
                if bits < 64 {
                    let rr = e.repr(d);
                    e.norm(rd, rd, rr)?;
                }
                e.finish(d, rd)?;
            }
            Ok(())
        })(),
        Inst::Load { dst, addr, off, index } if e.is_v(*dst) => (|| {
            let (lane, _) = e.func.vector(e.func.ty(*dst)).unwrap();
            let (bits, n) = e.shape(*dst);
            let ra = e.vector_address(*addr, *off, *index)?;
            if e.func.width(lane) == Some(1) {
                let rd = e.dst_vreg(*dst, 20);
                mask_load(e, rd, ra, n)?;
                return e.finish_v(*dst, rd);
            }
            let rd = e.dst_vreg(*dst, 23);
            e.vset(*dst)?;
            e.emit(&vle(bits, false), &[rd, ra])?;
            e.finish_v(*dst, rd)
        })(),
        Inst::Store { val, addr, off, index } if e.is_v(*val) => (|| {
            let (lane, _) = e.func.vector(e.func.ty(*val)).unwrap();
            let (bits, n) = e.shape(*val);
            let rv = e.src_vreg(*val, 20)?;
            let ra = e.vector_address(*addr, *off, *index)?;
            if e.func.width(lane) == Some(1) {
                return mask_store(e, rv, ra, n);
            }
            e.vset(*val)?;
            e.emit(&vle(bits, true), &[rv, ra]).map(|_| ())
        })(),
        Inst::IConst { dst, .. } if e.is_v(*dst) => Err(format!("a literal of a vector type ({}): not yet — splat it", e.tyname(*dst))),
        _ => return None,
    };
    Some(r)
}

/// the 64-bit or W form of an op; W forms sign-extend, i.e. produce
/// canonical i32 for free
fn bin_template(op: BinOp, signed: bool, w: bool) -> &'static str {
    match (op, signed, w) {
        (BinOp::IAdd, _, false) => "add {r}, {r}, {r}",
        (BinOp::IAdd, _, true) => "addw {r}, {r}, {r}",
        (BinOp::ISub, _, false) => "sub {r}, {r}, {r}",
        (BinOp::ISub, _, true) => "subw {r}, {r}, {r}",
        (BinOp::IMul, _, false) => "mul {r}, {r}, {r}",
        (BinOp::IMul, _, true) => "mulw {r}, {r}, {r}",
        (BinOp::Div, true, false) => "div {r}, {r}, {r}",
        (BinOp::Div, true, true) => "divw {r}, {r}, {r}",
        (BinOp::Div, false, false) => "divu {r}, {r}, {r}",
        (BinOp::Div, false, true) => "divuw {r}, {r}, {r}",
        (BinOp::Rem, true, false) => "rem {r}, {r}, {r}",
        (BinOp::Rem, true, true) => "remw {r}, {r}, {r}",
        (BinOp::Rem, false, false) => "remu {r}, {r}, {r}",
        (BinOp::Rem, false, true) => "remuw {r}, {r}, {r}",
        (BinOp::And, _, _) => "and {r}, {r}, {r}",
        (BinOp::Or, _, _) => "or {r}, {r}, {r}",
        (BinOp::Xor, _, _) => "xor {r}, {r}, {r}",
        (BinOp::Shl, _, false) => "sll {r}, {r}, {r}",
        (BinOp::Shl, _, true) => "sllw {r}, {r}, {r}",
        (BinOp::Shr, true, false) => "sra {r}, {r}, {r}",
        (BinOp::Shr, true, true) => "sraw {r}, {r}, {r}",
        (BinOp::Shr, false, false) => "srl {r}, {r}, {r}",
        (BinOp::Shr, false, true) => "srlw {r}, {r}, {r}",
    }
}

fn compile_inst(e: &mut RvEmit, inst: &Inst) -> Result<(), String> {
    if let Some(r) = compile_vector_inst(e, inst) {
        return r;
    }
    const SLT: &str = "slt {r}, {r}, {r}";
    const SLTU: &str = "sltu {r}, {r}, {r}";
    const XORI: &str = "xori {r}, {r}, {i -2048..2047}";
    match inst {
        Inst::IConst { dst, imm } => {
            let v = crate::opt::norm(e.repr(*dst), *imm as i64);
            let rd = e.dst_reg(*dst, T0);
            e.iconst(rd, v)?;
            e.finish(*dst, rd)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let r = e.repr(*dst);
            let n = r.bits();
            // i32 gets the W instructions (canonical for free); u32 and
            // every other narrow type compute in 64 bits and re-normalize
            let w = n == 32 && r.signed();
            let full = n == 64 || w;
            let rl = e.src_reg(*lhs, T0)?;
            let rr = e.src_reg(*rhs, T1)?;
            let rd = e.dst_reg(*dst, T0);
            let t = bin_template(*op, r.signed(), w);
            match op {
                _ => {
                    // shifts by >= n are unspecified for narrow types: the
                    // hardware shift, then re-normalize what can carry out
                    e.emit(t, &[rd, rl, rr])?;
                    let carries = matches!(op, BinOp::IAdd | BinOp::ISub | BinOp::IMul | BinOp::Shl)
                        || (*op == BinOp::Div && r.signed()); // MIN / -1
                    if !full && carries {
                        e.norm(rd, rd, r)?;
                    }
                }
            }
            e.finish(*dst, rd)
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let signed = e.repr(*lhs).signed();
            let rl = e.src_reg(*lhs, T0)?;
            let rr = e.src_reg(*rhs, T1)?;
            let rd = e.dst_reg(*dst, T0);
            let slt = if signed { SLT } else { SLTU };
            // canonical values compare correctly with the 64-bit slt/sltu
            match cond {
                Cond::Lt => {
                    e.emit(slt, &[rd, rl, rr])?;
                }
                Cond::Gt => {
                    e.emit(slt, &[rd, rr, rl])?;
                }
                Cond::Ge => {
                    e.emit(slt, &[rd, rl, rr])?;
                    e.emit(XORI, &[rd, rd, 1])?;
                }
                Cond::Le => {
                    e.emit(slt, &[rd, rr, rl])?;
                    e.emit(XORI, &[rd, rd, 1])?;
                }
                Cond::Eq => {
                    e.emit("xor {r}, {r}, {r}", &[rd, rl, rr])?;
                    e.emit("sltiu {r}, {r}, {i -2048..2047}", &[rd, rd, 1])?;
                }
                Cond::Ne => {
                    e.emit("xor {r}, {r}, {r}", &[rd, rl, rr])?;
                    e.emit(SLTU, &[rd, ZERO, rd])?;
                }
            }
            e.finish(*dst, rd)
        }
        Inst::Cast { dst, src, .. } => {
            let from = e.repr(*src);
            let to = e.repr(*dst);
            let rs = e.src_reg(*src, T0)?;
            let rd = e.dst_reg(*dst, T1);
            e.cast(rd, rs, from, to)?;
            e.finish(*dst, rd)
        }
        Inst::Get { dst, src, field } => {
            let (off, fty) = e.func.field(e.func.ty(*src), *field).unwrap();
            let fr = e.func.repr(fty);
            let rs = e.src_reg(*src, T0)?;
            let rd = e.dst_reg(*dst, T1);
            e.extract(rd, rs, off, fr.bits(), fr.signed())?;
            e.finish(*dst, rd)
        }
        Inst::Set {
            dst,
            src,
            field,
            val,
        } => {
            let (off, fty) = e.func.field(e.func.ty(*src), *field).unwrap();
            let w = e.func.width(fty).unwrap();
            let rs = e.src_reg(*src, T0)?;
            let rv = e.src_reg(*val, T1)?;
            let rd = e.dst_reg(*dst, T0);
            e.place(T2, rv, off, w)?; // the new field, in position
            let mask = if w >= 64 { -1i64 } else { ((1u64 << w) - 1) as i64 } << off;
            e.iconst(T1, !mask)?;
            e.emit("and {r}, {r}, {r}", &[T1, rs, T1])?;
            e.emit("or {r}, {r}, {r}", &[rd, T1, T2])?;
            e.finish(*dst, rd)
        }
        Inst::Pack { dst, args } => {
            let ty = e.func.ty(*dst);
            let rd = e.dst_reg(*dst, T0);
            // accumulate in t2; every source is read through t0/t1
            for (k, &a) in args.iter().enumerate() {
                let (off, fty) = e.func.field(ty, k as u32).unwrap();
                let w = e.func.width(fty).unwrap();
                let ra = e.src_reg(a, T0)?;
                if k == 0 {
                    e.place(T2, ra, off, w)?;
                } else {
                    e.place(T1, ra, off, w)?;
                    e.emit("or {r}, {r}, {r}", &[T2, T2, T1])?;
                }
            }
            e.mov(rd, T2)?;
            e.finish(*dst, rd)
        }
        Inst::Unpack { dsts, src } => {
            let ty = e.func.ty(*src);
            let rs = e.src_reg(*src, T0)?;
            e.mov(T2, rs)?; // results may be allocated over the source
            for (k, &d) in dsts.iter().enumerate() {
                let (off, fty) = e.func.field(ty, k as u32).unwrap();
                let fr = e.func.repr(fty);
                let rd = e.dst_reg(d, T1);
                e.extract(rd, T2, off, fr.bits(), fr.signed())?;
                e.finish(d, rd)?;
            }
            Ok(())
        }
        Inst::Load { dst, addr, off, index } => {
            let r = e.repr(*dst);
            let (ra, imm) = e.address(*addr, *off, *index, T0, T2)?;
            let rd = e.dst_reg(*dst, T1);
            let t = match (r.bits(), r.signed()) {
                (8, true) => "lb {r}, {i -2048..2047}({r})",
                (8, false) => "lbu {r}, {i -2048..2047}({r})",
                (16, true) => "lh {r}, {i -2048..2047}({r})",
                (16, false) => "lhu {r}, {i -2048..2047}({r})",
                (32, true) => "lw {r}, {i -2048..2047}({r})",
                (32, false) => "lwu {r}, {i -2048..2047}({r})",
                (64, _) => LD,
                (n, _) => return Err(format!("no {}-bit memory access", n)),
            };
            e.emit(t, &[rd, imm, ra])?;
            e.finish(*dst, rd)
        }
        Inst::Store { val, addr, off, index } => {
            let r = e.repr(*val);
            let rv = e.src_reg(*val, T1)?;
            let (ra, imm) = e.address(*addr, *off, *index, T0, T2)?;
            let t = match r.bits() {
                8 => "sb {r}, {i -2048..2047}({r})",
                16 => "sh {r}, {i -2048..2047}({r})",
                32 => "sw {r}, {i -2048..2047}({r})",
                64 => SD,
                n => return Err(format!("no {}-bit memory access", n)),
            };
            e.emit(t, &[rv, imm, ra])?;
            Ok(())
        }
        Inst::Addr { dst, name } => {
            let rd = e.dst_reg(*dst, T0);
            let at = e.emit(AUIPC, &[rd, 0])?;
            e.emit(ADDI, &[rd, rd, 0])?;
            e.fixups.push(Fixup { at, values: vec![rd, 0], imm_slot: 1, target: FixTarget::Data(name.clone()) });
            e.finish(*dst, rd)
        }
        Inst::Scratch { dst, .. } => {
            let off = e.scratch[dst];
            let rd = e.dst_reg(*dst, T0);
            e.emit(ADDI, &[rd, SP, off])?;
            e.finish(*dst, rd)
        }
        Inst::Check { cond } => {
            let rc = e.src_reg(*cond, T0)?;
            e.emit(BNE, &[rc, ZERO, 8])?;
            e.emit("ebreak", &[]).map(|_| ())
        }
        Inst::Platform { dst, name } => {
            let v = *e.natives.consts.get(name).ok_or_else(|| format!("the platform has no constant '{}'", name))?;
            let rd = e.dst_reg(*dst, T0);
            e.iconst(rd, v)?;
            e.finish(*dst, rd)
        }
        Inst::PtrAdd { dst, base, off } => {
            let rb = e.src_reg(*base, T0)?;
            let ro = e.src_reg(*off, T1)?;
            let rd = e.dst_reg(*dst, T0);
            e.emit("add {r}, {r}, {r}", &[rd, rb, ro])?;
            e.finish(*dst, rd)
        }
        Inst::Call { dsts, callee, args } if e.natives.get(callee).is_some_and(|n| n.inline) && !args.iter().any(|&a| e.is_v(a)) => {
            // the platform has this one: the rule's sequence instead of
            // the call, each operand in its own file
            let natives: &Natives = e.natives;
            let native = natives.get(callee).unwrap();
            let dst = dsts.first().copied();
            if dst.is_none() && native.ret_bits != 0 {
                return Ok(()); // result unused: nothing to compute
            }
            let mut regs = Vec::new();
            for (j, &a) in args.iter().enumerate() {
                regs.push(if native.arg_class[j].is_some() { e.src_freg(a, j as i64)? } else { e.src_reg(a, [T0, T1, T2][j])? });
            }
            let ret_f = native.ret_class.is_some();
            let rd = match dst {
                Some(d) if ret_f => e.dst_freg(d, 3),
                Some(d) => e.dst_reg(d, T0),
                None => T0,
            };
            for (t, v) in rule_seq(e.enc, native, rd, &regs)? {
                e.emit(t, &v)?;
            }
            match dst {
                Some(d) if ret_f => e.finish_f(d, rd),
                Some(d) => e.finish(d, rd),
                None => Ok(()),
            }
        }
        Inst::FnAddr { dst, name } => {
            let rd = e.dst_reg(*dst, T0);
            let at = e.emit(AUIPC, &[rd, 0])?;
            e.emit(ADDI, &[rd, rd, 0])?;
            e.fixups.push(Fixup { at, values: vec![rd, 0], imm_slot: 1, target: FixTarget::FuncAddr(name.clone()) });
            e.finish(*dst, rd)
        }
        Inst::CallInd { dsts, callee, args } => {
            let abi = e.abi_regs(args)?;
            let rabi = e.abi_regs(dsts)?;
            let below = RvEmit::stack_args(&abi).max(RvEmit::stack_args(&rabi));
            if below > 0 {
                e.emit(ADDI, &[SP, SP, -below])?;
                e.sp_adjust = below;
            }
            for (&a, abi) in args.iter().zip(abi) {
                e.arg_out(abi, a)?;
            }
            let rc = e.src_reg(*callee, T0)?;
            e.emit("jalr {r}, {i -2048..2047}({r})", &[RA, 0, rc])?;
            for (&d, abi) in dsts.iter().zip(rabi) {
                e.res_in(abi, d)?;
            }
            if below > 0 {
                e.emit(ADDI, &[SP, SP, below])?;
                e.sp_adjust = 0;
            }
            Ok(())
        }
        Inst::Call { dsts, callee, args } => {
            let abi = e.abi_regs(args)?;
            let rabi = e.abi_regs(dsts)?;
            let below = RvEmit::stack_args(&abi).max(RvEmit::stack_args(&rabi));
            if below > 0 {
                e.emit(ADDI, &[SP, SP, -below])?;
                e.sp_adjust = below;
            }
            for (&a, abi) in args.iter().zip(abi) {
                e.arg_out(abi, a)?;
            }
            let at = e.emit(JAL, &[RA, 0])?;
            e.fixups.push(Fixup {
                at,
                values: vec![RA, 0],
                imm_slot: 1,
                target: FixTarget::Func(callee.clone()),
            });
            for (&d, abi) in dsts.iter().zip(rabi) {
                e.res_in(abi, d)?;
            }
            if below > 0 {
                e.emit(ADDI, &[SP, SP, below])?;
                e.sp_adjust = 0;
            }
            Ok(())
        }
        Inst::Jmp { target, args } => {
            e.branch_args(*target, args)?;
            e.goto(*target)
        }
        Inst::Br {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            let rc = e.src_reg(*cond, T0)?;
            if e.is_next(*then_target) && then_args.is_empty() {
                // the then block follows: hop over the else side when the
                // condition holds, and fall into it
                let bne_at = e.emit(BNE, &[rc, ZERO, 0])?;
                e.branch_args(*else_target, else_args)?;
                e.goto(*else_target)?;
                let then_here = e.code.len() as i64 - bne_at as i64;
                let word = e.enc.encode(BNE, &[rc, ZERO, then_here])?;
                e.code[bne_at..bne_at + 4].copy_from_slice(&word.to_le_bytes());
                return Ok(());
            }
            // cond == 0 hops over the then-side moves + jump; the hop is a
            // local distance (tens of bytes), patched once the then side is
            // emitted, so argument moves only run on the taken path
            let beq_at = e.emit(BEQ, &[rc, ZERO, 0])?;
            e.branch_args(*then_target, then_args)?;
            e.jump(*then_target)?;
            let else_here = e.code.len() as i64 - beq_at as i64;
            let word = e.enc.encode(BEQ, &[rc, ZERO, else_here])?;
            e.code[beq_at..beq_at + 4].copy_from_slice(&word.to_le_bytes());
            e.branch_args(*else_target, else_args)?;
            e.goto(*else_target)
        }
        Inst::Ret { vals } => {
            for (&v, abi) in vals.iter().zip(e.abi_regs(vals)?) {
                e.res_out(abi, v)?;
            }
            e.epilogue()
        }
    }
}
