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

pub fn compile(module: &Module, enc: &Encoder) -> Result<Compiled, String> {
    compile_with(module, enc, &Platform::riscv64())
}

pub fn compile_with(module: &Module, enc: &Encoder, platform: &Platform) -> Result<Compiled, String> {
    compile_image(module, enc, platform, 0)
}

/// Compile a module for an image whose code begins at byte `origin`;
/// riscv's trap vector is the handler's entry itself (mtvec in direct
/// mode), so nothing here depends on the origin
pub fn compile_image(module: &Module, enc: &Encoder, platform: &Platform, origin: usize) -> Result<Compiled, String> {
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
    // data after the code, 8-aligned
    let code_end = code.len();
    while code.len() % 8 != 0 {
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
    Ok(Compiled { code, funcs, code_end, data_base })
}

struct RvEmit<'a> {
    enc: &'a Encoder,
    func: &'a Function,
    natives: &'a Natives,
    code: &'a mut Vec<u8>,
    frame: i64,
    alloc: &'a regalloc::Alloc,
    /// per value: the platform's float register class (`f`), if any
    classes: Vec<Option<String>>,
    spill_base: i64,
    /// a trap or interrupt handler: (the frame area for the interrupted
    /// code's registers, an interrupt — a0 goes back too and the float
    /// scratch registers are kept as well); it leaves by mret
    trap: Option<(i64, bool)>,
    /// the registers a handler keeps (integer, float), see TRAP_SAVED
    saved: (Vec<i64>, Vec<i64>),
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
        self.spill_base + 8 * idx as i64
    }

    fn is_f(&self, v: ValueId) -> bool {
        self.classes[v.0 as usize].is_some()
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
    /// integer file or (`f`) the float file
    fn loc_move(&mut self, f: bool, dst: Loc, src: Loc) -> Result<(), String> {
        if f {
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
        let mut pending: Vec<(bool, Loc, Loc)> = params
            .iter()
            .zip(args)
            .map(|(&p, &a)| (self.is_f(p), self.alloc.loc[p.0 as usize], self.alloc.loc[a.0 as usize]))
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
                let scratch = Loc::Reg(if f { 0 } else { T0 });
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
        for (k, &r) in self.alloc.used_regs.clone().iter().enumerate() {
            self.emit(LD, &[r, 16 + 8 * k as i64, SP])?;
        }
        let fbase = 16 + 8 * self.alloc.used_regs.len() as i64;
        for (k, &fr) in self.alloc.used_by_class[1].clone().iter().enumerate() {
            self.emit(FLD, &[fr, fbase + 8 * k as i64, SP])?;
        }
        if let Some((base, irq)) = self.trap {
            // a task switch's result is the frame to go back from
            if irq && !self.func.rets.is_empty() {
                self.emit(ADDI, &[SP, A0, 0])?;
            }
            // a trap's result is in a0; everything else goes back as it was
            let (ints, fp) = self.saved.clone();
            for (k, &r) in ints.iter().enumerate() {
                if r != A0 || irq {
                    self.emit(LD, &[r, base + 8 * k as i64, SP])?;
                }
            }
            let fp_base = base + 8 * ints.len() as i64;
            for (k, &fr) in fp.iter().enumerate() {
                self.emit(FLD, &[fr, fp_base + 8 * k as i64, SP])?;
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
        // this function *is* a platform instruction: bits arrive in a0..,
        // float ones move to ft0.., the rule runs, a float result comes
        // back through a0 (zero-extended when 32 bits wide)
        let mut seq: Vec<(&str, Vec<i64>)> = Vec::new();
        let mut args = Vec::new();
        for (i, class) in native.arg_class.iter().enumerate() {
            if class.is_some() {
                seq.push((if native.arg_bits[i] <= 32 { "fmv.w.x {f}, {r}" } else { "fmv.d.x {f}, {r}" }, vec![i as i64, A0 + i as i64]));
                args.push(i as i64);
            } else {
                args.push(A0 + i as i64);
            }
        }
        let rd = if native.ret_class.is_some() { 3 } else { A0 };
        seq.extend(rule_seq(enc, native, rd, &args)?);
        if native.ret_class.is_some() {
            if native.ret_bits <= 32 {
                seq.push(("fmv.x.w {r}, {f}", vec![A0, 3]));
                seq.push((SLLI, vec![A0, A0, 32]));
                seq.push((SRLI, vec![A0, A0, 32]));
            } else {
                seq.push(("fmv.x.d {r}, {f}", vec![A0, 3]));
            }
        }
        for (t, v) in seq {
            code.extend_from_slice(&enc.encode(t, &v)?.to_le_bytes());
        }
        code.extend_from_slice(&enc.encode("jalr {r}, {i -2048..2047}({r})", &[ZERO, 0, RA])?.to_le_bytes());
        return Ok(());
    }
    let classes: Vec<Option<String>> = func.values.iter().map(|v| natives.class_of(func, v.ty).map(str::to_string)).collect();
    let class_idx: Vec<usize> = classes.iter().map(|c| c.is_some() as usize).collect();
    let alloc = regalloc::allocate_classes(func, &class_idx, &[REG_POOL, F_POOL]);
    let nsaved = alloc.used_regs.len() as i64 + alloc.used_by_class[1].len() as i64;
    let irq = func.name == crate::emit::IRQ;
    if irq && !(func.params.is_empty() && func.rets.is_empty() || func.params.len() == 1 && func.rets.len() == 1 && func.ty(func.params[0]) == Type::Ptr && func.rets[0] == Type::Ptr) {
        return Err("__irq is fn __irq() or fn __irq(sp: ptr) -> ptr".into());
    }
    let trap = (func.name == crate::emit::TRAP || irq).then_some((16 + 8 * nsaved, irq));
    let switching = irq && !func.params.is_empty();
    let mut saved: (Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new());
    if trap.is_some() {
        saved.0 = TRAP_SAVED.to_vec();
        if irq && !natives.classes.is_empty() {
            saved.1 = IRQ_FP_SAVED.to_vec();
        }
        if switching {
            saved.0.extend_from_slice(SWITCH_SAVED);
            if !natives.classes.is_empty() {
                saved.1.extend_from_slice(SWITCH_FP_SAVED);
            }
        }
    }
    let trap_area = (8 * (saved.0.len() + saved.1.len()) as i64 + 15) & !15;
    let spill_base = 16 + 8 * nsaved + trap_area;
    let (scratch, scratch_end) = crate::emit::scratch_layout(func, (spill_base + 8 * alloc.nslots as i64 + 15) & !15);
    let frame = scratch_end;
    if frame > 2047 {
        return Err(format!("function needs a {}-byte frame; 2047 is the most for now", frame));
    }
    if func.params.len() > 8 {
        return Err("more than 8 parameters not supported yet".into());
    }

    let mut e = RvEmit {
        enc,
        func,
        natives,
        code,
        frame,
        alloc: &alloc,
        classes,
        spill_base,
        trap,
        saved: saved.clone(),
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
    if let Some((base, irq)) = trap {
        for (k, &r) in saved.0.iter().enumerate() {
            e.emit(SD, &[r, base + 8 * k as i64, SP])?;
        }
        let fp_base = base + 8 * saved.0.len() as i64;
        for (k, &fr) in saved.1.iter().enumerate() {
            e.emit(FSD, &[fr, fp_base + 8 * k as i64, SP])?;
        }
        // an interrupt handler that switches tasks is given its frame
        if irq && !func.params.is_empty() {
            e.emit(ADDI, &[A0, SP, 0])?;
        }
    }
    for (i, &p) in func.params.iter().enumerate() {
        e.value_from(p, A0 + i as i64)?;
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

fn rule_seq<'a>(enc: &'a Encoder, native: &Native, rd: i64, args: &[i64]) -> Result<Vec<(&'a str, Vec<i64>)>, String> {
    let templates = enc.templates();
    let mut seq = Vec::new();
    for line in &native.rule.lines {
        if line.mnemonic == "none" {
            continue;
        }
        let (t, vals) = crate::platform::resolve(native, line, &templates)?;
        let v = vals
            .into_iter()
            .map(|(op, v)| match op {
                Operand::Arg(i) => args[i],
                Operand::Ret => rd,
                Operand::Tmp(k) => RULE_TEMPS[k],
                Operand::Lit(_) => v,
            })
            .collect();
        seq.push((t, v));
    }
    Ok(seq)
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
        Inst::Call { dsts, callee, args } if e.natives.get(callee).is_some_and(|n| n.inline) => {
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
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            for (j, &a) in args.iter().enumerate() {
                e.value_to(A0 + j as i64, a)?;
            }
            let rc = e.src_reg(*callee, T0)?;
            e.emit("jalr {r}, {i -2048..2047}({r})", &[RA, 0, rc])?;
            for (j, &d) in dsts.iter().enumerate() {
                e.value_from(d, A0 + j as i64)?;
            }
            Ok(())
        }
        Inst::Call { dsts, callee, args } => {
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            for (j, &a) in args.iter().enumerate() {
                e.value_to(A0 + j as i64, a)?;
            }
            let at = e.emit(JAL, &[RA, 0])?;
            e.fixups.push(Fixup {
                at,
                values: vec![RA, 0],
                imm_slot: 1,
                target: FixTarget::Func(callee.clone()),
            });
            for (j, &d) in dsts.iter().enumerate() {
                e.value_from(d, A0 + j as i64)?;
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
            if vals.len() > 8 {
                return Err("more than 8 return values not supported yet".into());
            }
            for (j, &v) in vals.iter().enumerate() {
                e.value_to(A0 + j as i64, v)?;
            }
            e.epilogue()
        }
    }
}
