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
//! - i32 convention: slots hold values *sign-extended* to 64 bits — that is
//!   what the W-instructions and lw produce naturally. (Unsigned i32
//!   comparisons still work: sign-extension preserves unsigned 32-bit
//!   order in the unsigned 64-bit domain.) The execution harness
//!   normalizes i32 results to the suite's zero-extended convention.
//! - Conditional branches reach only ±4K, so `br` lowers to a two-word
//!   skip (`beq cond, x0, +8` over a `jal`) with ±1MB range.
//!
//! Registers: x0 zero | x1 ra | x2 sp | x5-x7 (t0-t2) scratch |
//! x10-x17 (a0-a7) arguments, results, branch staging | x18-x27 the pool.
//!
//! Frame: sp+0 saved ra, sp+16 callee-saved save area, then spill slots.

use crate::emit::{Compiled, Encoder};
use crate::regalloc::{self, Loc};
use crate::ssa::{BinOp, BlockId, CastOp, Cond, Function, Inst, Module, Type, ValueId};

/// pools for the allocator: callee-saved s2..s11 (x18..x27) and
/// fs2..fs11 (f18..f27) — values placed there survive calls by construction
const INT_POOL: &[i64] = &[18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
const FLOAT_POOL: &[i64] = &[18, 19, 20, 21, 22, 23, 24, 25, 26, 27];

const ZERO: i64 = 0; // x0
const RA: i64 = 1;
const SP: i64 = 2;
const T0: i64 = 5;
const T1: i64 = 6;
const A0: i64 = 10;

const ADDI: &str = "addi {r}, {r}, {i -2048..2047}";
const FLD: &str = "fld {f}, {i -2048..2047}({r})";
const FSD: &str = "fsd {f}, {i -2048..2047}({r})";
const FMV: &str = "fmv.d {f}, {f}";
const FT0: i64 = 0; // ft0/ft1/ft2: float scratch
const FT1: i64 = 1;
const FT2: i64 = 2;
const LD: &str = "ld {r}, {i -2048..2047}({r})";
const SD: &str = "sd {r}, {i -2048..2047}({r})";
const JAL: &str = "jal {r}, {i -1048576..1048574 /2}";
const BEQ: &str = "beq {r}, {r}, {i -4096..4094 /2}";

enum FixTarget {
    Block(BlockId),
    Func(String),
}

struct Fixup {
    at: usize,
    values: Vec<i64>, // for JAL: [rd, offset]
    imm_slot: usize,
    target: FixTarget,
}

pub fn compile(module: &Module, enc: &Encoder) -> Result<Compiled, String> {
    let mut code: Vec<u8> = Vec::new();
    let mut funcs = std::collections::HashMap::new();
    let mut call_fixups: Vec<Fixup> = Vec::new();

    for func in &module.funcs {
        funcs.insert(func.name.clone(), code.len());
        compile_function(func, enc, &mut code, &mut call_fixups)
            .map_err(|e| format!("@{}: {}", func.name, e))?;
    }
    for fix in call_fixups {
        let FixTarget::Func(name) = &fix.target else {
            unreachable!()
        };
        let target = *funcs
            .get(name.as_str())
            .ok_or_else(|| format!("call to undefined function @{}", name))?;
        let mut values = fix.values;
        values[fix.imm_slot] = target as i64 - fix.at as i64;
        let word = enc.encode(JAL, &values)?;
        code[fix.at..fix.at + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(Compiled { code, funcs })
}

struct RvEmit<'a> {
    enc: &'a Encoder,
    func: &'a Function,
    code: &'a mut Vec<u8>,
    frame: i64,
    alloc: &'a regalloc::Alloc,
    spill_base: i64,
    block_offsets: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
}

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

    /// float spill slots always move as 8 bytes via fld/fsd, preserving
    /// the NaN boxing of f32 values
    fn src_reg(&mut self, v: ValueId, scratch: i64) -> Result<i64, String> {
        let float = self.func.ty(v).is_float();
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => Ok(r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(if float { FLD } else { LD }, &[scratch, off, SP])?;
                Ok(scratch)
            }
        }
    }

    fn dst_reg(&self, v: ValueId, scratch: i64) -> i64 {
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => r,
            Loc::Slot(_) => scratch,
        }
    }

    fn finish(&mut self, v: ValueId, reg: i64) -> Result<(), String> {
        let float = self.func.ty(v).is_float();
        if let Loc::Slot(i) = self.alloc.loc[v.0 as usize] {
            let off = self.slot_off(i);
            self.emit(if float { FSD } else { SD }, &[reg, off, SP])?;
        }
        Ok(())
    }

    fn mov(&mut self, dst: i64, src: i64) -> Result<(), String> {
        if dst != src {
            self.emit(ADDI, &[dst, src, 0])?;
        }
        Ok(())
    }

    fn fmov(&mut self, dst: i64, src: i64) -> Result<(), String> {
        if dst != src {
            self.emit(FMV, &[dst, src])?;
        }
        Ok(())
    }

    fn class_mov(&mut self, float: bool, dst: i64, src: i64) -> Result<(), String> {
        if float {
            self.fmov(dst, src)
        } else {
            self.mov(dst, src)
        }
    }

    /// place v into a specific register (targets a0..a7 / staging; sources
    /// are pool registers or slots — disjoint, so sequences never clobber)
    fn value_to(&mut self, target: i64, v: ValueId) -> Result<(), String> {
        let float = self.func.ty(v).is_float();
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => self.class_mov(float, target, r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(if float { FLD } else { LD }, &[target, off, SP])
                    .map(|_| ())
            }
        }
    }

    fn value_from(&mut self, v: ValueId, source: i64) -> Result<(), String> {
        let float = self.func.ty(v).is_float();
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => self.class_mov(float, r, source),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(if float { FSD } else { SD }, &[source, off, SP])
                    .map(|_| ())
            }
        }
    }

    /// unconditional jump to an SSA block (offset patched later)
    fn goto(&mut self, target: BlockId) -> Result<(), String> {
        let at = self.emit(JAL, &[ZERO, 0])?;
        self.fixups.push(Fixup {
            at,
            values: vec![ZERO, 0],
            imm_slot: 1,
            target: FixTarget::Block(target),
        });
        Ok(())
    }

    /// one location-to-location move; `float` selects the register class
    fn loc_move(&mut self, float: bool, dst: Loc, src: Loc) -> Result<(), String> {
        let (ld, sd) = if float { (FLD, FSD) } else { (LD, SD) };
        match (dst, src) {
            (Loc::Reg(d), Loc::Reg(s)) => self.class_mov(float, d, s),
            (Loc::Reg(d), Loc::Slot(s)) => {
                let off = self.slot_off(s);
                self.emit(ld, &[d, off, SP]).map(|_| ())
            }
            (Loc::Slot(d), Loc::Reg(s)) => {
                let off = self.slot_off(d);
                self.emit(sd, &[s, off, SP]).map(|_| ())
            }
            (Loc::Slot(d), Loc::Slot(s)) => {
                // transit through t1 (byte copy); cycle scratches stay free
                self.emit(LD, &[T1, self.slot_off(s), SP])?;
                self.emit(SD, &[T1, self.slot_off(d), SP]).map(|_| ())
            }
        }
    }

    /// branch arguments as a parallel move, resolved per register class;
    /// cycles break through t0 (integers) / ft2 (floats)
    fn branch_args(&mut self, target: BlockId, args: &[ValueId]) -> Result<(), String> {
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        for float in [false, true] {
            let mut pending: Vec<(Loc, Loc)> = params
                .iter()
                .zip(args)
                .filter(|(p, _)| self.func.ty(**p).is_float() == float)
                .map(|(p, a)| (self.alloc.loc[p.0 as usize], self.alloc.loc[a.0 as usize]))
                .filter(|(d, s)| d != s)
                .collect();
            let scratch = Loc::Reg(if float { FT2 } else { T0 });
            while !pending.is_empty() {
                if let Some(i) =
                    (0..pending.len()).find(|&i| !pending.iter().any(|&(_, s)| s == pending[i].0))
                {
                    let (d, s) = pending.swap_remove(i);
                    self.loc_move(float, d, s)?;
                } else {
                    let s = pending[0].1;
                    self.loc_move(float, scratch, s)?;
                    for m in pending.iter_mut().filter(|m| m.1 == s) {
                        m.1 = scratch;
                    }
                }
            }
        }
        Ok(())
    }

    fn epilogue(&mut self) -> Result<(), String> {
        for (k, &r) in self.alloc.used_int.clone().iter().enumerate() {
            self.emit(LD, &[r, 16 + 8 * k as i64, SP])?;
        }
        let fbase = 16 + 8 * self.alloc.used_int.len() as i64;
        for (k, &r) in self.alloc.used_float.clone().iter().enumerate() {
            self.emit(FLD, &[r, fbase + 8 * k as i64, SP])?;
        }
        self.emit(LD, &[RA, 0, SP])?;
        self.emit(ADDI, &[SP, SP, self.frame])?;
        self.emit("jalr {r}, {i -2048..2047}({r})", &[ZERO, 0, RA])?;
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
                self.emit("slli {r}, {r}, {i 0..63}", &[reg, reg, 8])?;
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
    code: &mut Vec<u8>,
    call_fixups: &mut Vec<Fixup>,
) -> Result<(), String> {
    let alloc = regalloc::allocate(func, INT_POOL, FLOAT_POOL);
    let nsaved = (alloc.used_int.len() + alloc.used_float.len()) as i64;
    let spill_base = 16 + 8 * nsaved;
    let frame = (spill_base + 8 * alloc.nslots as i64 + 15) & !15;
    if frame > 2047 {
        return Err("function needs too large a frame for v0".into());
    }
    if func.params.len() > 8 {
        return Err("more than 8 parameters not supported yet".into());
    }

    let mut e = RvEmit {
        enc,
        func,
        code,
        frame,
        alloc: &alloc,
        spill_base,
        block_offsets: vec![None; func.blocks.len()],
        fixups: Vec::new(),
    };

    e.emit(ADDI, &[SP, SP, -frame])?;
    e.emit(SD, &[RA, 0, SP])?;
    for (k, &r) in alloc.used_int.iter().enumerate() {
        e.emit(SD, &[r, 16 + 8 * k as i64, SP])?;
    }
    let fbase = 16 + 8 * alloc.used_int.len() as i64;
    for (k, &r) in alloc.used_float.iter().enumerate() {
        e.emit(FSD, &[r, fbase + 8 * k as i64, SP])?;
    }
    let (mut gi, mut fi) = (0i64, 0i64);
    for &p in &func.params {
        if func.ty(p).is_float() {
            e.value_from(p, 10 + fi)?; // fa0..
            fi += 1;
        } else {
            e.value_from(p, A0 + gi)?;
            gi += 1;
        }
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        e.block_offsets[bi] = Some(e.code.len());
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
            FixTarget::Func(_) => call_fixups.push(fix),
        }
    }
    Ok(())
}

fn bin_template(op: BinOp, ty: Type) -> &'static str {
    let w = ty.width() == Some(32);
    let signed = ty.is_signed();
    match op {
        BinOp::IAdd => {
            if w {
                "addw {r}, {r}, {r}"
            } else {
                "add {r}, {r}, {r}"
            }
        }
        BinOp::ISub => {
            if w {
                "subw {r}, {r}, {r}"
            } else {
                "sub {r}, {r}, {r}"
            }
        }
        BinOp::IMul => {
            if w {
                "mulw {r}, {r}, {r}"
            } else {
                "mul {r}, {r}, {r}"
            }
        }
        BinOp::Div => match (w, signed) {
            (true, true) => "divw {r}, {r}, {r}",
            (true, false) => "divuw {r}, {r}, {r}",
            (false, true) => "div {r}, {r}, {r}",
            (false, false) => "divu {r}, {r}, {r}",
        },
        BinOp::Rem => match (w, signed) {
            (true, true) => "remw {r}, {r}, {r}",
            (true, false) => "remuw {r}, {r}, {r}",
            (false, true) => "rem {r}, {r}, {r}",
            (false, false) => "remu {r}, {r}, {r}",
        },
        BinOp::And => "and {r}, {r}, {r}",
        BinOp::Or => "or {r}, {r}, {r}",
        BinOp::Xor => "xor {r}, {r}, {r}",
        BinOp::Shl => {
            if w {
                "sllw {r}, {r}, {r}"
            } else {
                "sll {r}, {r}, {r}"
            }
        }
        BinOp::Shr => match (w, signed) {
            (true, true) => "sraw {r}, {r}, {r}",
            (true, false) => "srlw {r}, {r}, {r}",
            (false, true) => "sra {r}, {r}, {r}",
            (false, false) => "srl {r}, {r}, {r}",
        },
        BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv => {
            unreachable!("float ops matched before bin_template")
        }
    }
}

fn compile_inst(e: &mut RvEmit, inst: &Inst) -> Result<(), String> {
    const SLT: &str = "slt {r}, {r}, {r}";
    const SLTU: &str = "sltu {r}, {r}, {r}";
    const XORI: &str = "xori {r}, {r}, {i -2048..2047}";
    match inst {
        Inst::IConst { dst, imm } => {
            // 32-bit constants live sign-extended, per the W convention
            // (which serves u32 too: W ops read the low 32 bits)
            let v = if e.func.ty(*dst).width() == Some(32) {
                *imm as i32 as i64
            } else {
                *imm
            };
            let rd = e.dst_reg(*dst, T0);
            e.iconst(rd, v)?;
            e.finish(*dst, rd)
        }
        Inst::FConst { dst, bits } => {
            let rd = e.dst_reg(*dst, FT0);
            if e.func.ty(*dst) == Type::F64 {
                e.iconst(T0, *bits as i64)?;
                e.emit("fmv.d.x {f}, {r}", &[rd, T0])?;
            } else {
                let b = (f64::from_bits(*bits) as f32).to_bits() as i64;
                e.iconst(T0, b)?;
                e.emit("fmv.w.x {f}, {r}", &[rd, T0])?;
            }
            e.finish(*dst, rd)
        }
        Inst::Bin { op, dst, lhs, rhs } if op.is_float() => {
            let d64 = e.func.ty(*dst) == Type::F64;
            let rl = e.src_reg(*lhs, FT0)?;
            let rr = e.src_reg(*rhs, FT1)?;
            let rd = e.dst_reg(*dst, FT0);
            let t = match (op, d64) {
                (BinOp::FAdd, true) => "fadd.d {f}, {f}, {f}",
                (BinOp::FSub, true) => "fsub.d {f}, {f}, {f}",
                (BinOp::FMul, true) => "fmul.d {f}, {f}, {f}",
                (BinOp::FDiv, true) => "fdiv.d {f}, {f}, {f}",
                (BinOp::FAdd, false) => "fadd.s {f}, {f}, {f}",
                (BinOp::FSub, false) => "fsub.s {f}, {f}, {f}",
                (BinOp::FMul, false) => "fmul.s {f}, {f}, {f}",
                (BinOp::FDiv, false) => "fdiv.s {f}, {f}, {f}",
                _ => unreachable!(),
            };
            e.emit(t, &[rd, rl, rr])?;
            e.finish(*dst, rd)
        }
        Inst::FCmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            use crate::ssa::FCond;
            let d64 = e.func.ty(*lhs) == Type::F64;
            let rl = e.src_reg(*lhs, FT0)?;
            let rr = e.src_reg(*rhs, FT1)?;
            let rd = e.dst_reg(*dst, T0);
            // feq/flt/fle write integer registers; gt/ge swap, une negates
            let (base, swap, negate) = match cond {
                FCond::Oeq => (0, false, false),
                FCond::Une => (0, false, true),
                FCond::Olt => (1, false, false),
                FCond::Ogt => (1, true, false),
                FCond::Ole => (2, false, false),
                FCond::Oge => (2, true, false),
            };
            let t = match (base, d64) {
                (0, true) => "feq.d {r}, {f}, {f}",
                (1, true) => "flt.d {r}, {f}, {f}",
                (2, true) => "fle.d {r}, {f}, {f}",
                (0, false) => "feq.s {r}, {f}, {f}",
                (1, false) => "flt.s {r}, {f}, {f}",
                (2, false) => "fle.s {r}, {f}, {f}",
                _ => unreachable!(),
            };
            let (a, b) = if swap { (rr, rl) } else { (rl, rr) };
            e.emit(t, &[rd, a, b])?;
            if negate {
                e.emit("xori {r}, {r}, {i -2048..2047}", &[rd, rd, 1])?;
            }
            e.finish(*dst, rd)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let rl = e.src_reg(*lhs, T0)?;
            let rr = e.src_reg(*rhs, T1)?;
            let rd = e.dst_reg(*dst, T0);
            e.emit(bin_template(*op, e.func.ty(*dst)), &[rd, rl, rr])?;
            e.finish(*dst, rd)
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let ty = e.func.ty(*lhs);
            let rl = e.src_reg(*lhs, T0)?;
            let rr = e.src_reg(*rhs, T1)?;
            let rd = e.dst_reg(*dst, T0);
            // canonical i32/u32 values compare correctly at 64 bits
            let slt = if ty.is_signed() { SLT } else { SLTU };
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
        Inst::Cast { op, dst, src } => {
            let from = e.func.ty(*src);
            let to = e.func.ty(*dst);
            let rs = e.src_reg(*src, T0)?;
            let rd = e.dst_reg(*dst, T1);
            let sw = from.width();
            let dw = to.width();
            match (op, from, to) {
                // width-1 signed value (0/1): sign-extension is negation
                (CastOp::Ext, _, _) if sw == Some(1) && from.is_signed() => {
                    e.emit("sub {r}, {r}, {r}", &[rd, ZERO, rs])?;
                }
                // u1 is 0/1 already; i32 is already sign-extended
                (CastOp::Ext, _, _) if sw == Some(1) => {
                    e.mov(rd, rs)?;
                }
                (CastOp::Ext, _, _) if sw == Some(32) && from.is_signed() => {
                    e.mov(rd, rs)?;
                }
                (CastOp::Ext, _, _) if sw == Some(32) => {
                    e.emit("slli {r}, {r}, {i 0..63}", &[rd, rs, 32])?;
                    e.emit("srli {r}, {r}, {i 0..63}", &[rd, rd, 32])?;
                }
                (CastOp::Trunc, _, _) if dw == Some(32) => {
                    e.emit("addiw {r}, {r}, {i -2048..2047}", &[rd, rs, 0])?;
                }
                (CastOp::Trunc, _, _) if dw == Some(1) => {
                    e.emit("andi {r}, {r}, {i -2048..2047}", &[rd, rs, 1])?;
                }
                // same-width signedness change: same bits, same canonical
                (CastOp::Bitcast, _, _) if !from.is_float() && !to.is_float() => {
                    e.mov(rd, rs)?;
                }
                (CastOp::Itof, _, _) => {
                    let rs = e.src_reg(*src, T0)?;
                    let rdf = e.dst_reg(*dst, FT0);
                    let t = match (from.width(), to, from.is_signed()) {
                        (Some(64), Type::F64, true) => "fcvt.d.l {f}, {r}",
                        (Some(32), Type::F64, true) => "fcvt.d.w {f}, {r}",
                        (Some(64), Type::F32, true) => "fcvt.s.l {f}, {r}",
                        (Some(32), Type::F32, true) => "fcvt.s.w {f}, {r}",
                        (Some(64), Type::F64, false) => "fcvt.d.lu {f}, {r}",
                        (Some(32), Type::F64, false) => "fcvt.d.wu {f}, {r}",
                        (Some(64), Type::F32, false) => "fcvt.s.lu {f}, {r}",
                        (Some(32), Type::F32, false) => "fcvt.s.wu {f}, {r}",
                        _ => unreachable!(),
                    };
                    e.emit(t, &[rdf, rs])?;
                    return e.finish(*dst, rdf);
                }
                (CastOp::Ftoi, _, _) => {
                    let rs = e.src_reg(*src, FT0)?;
                    let rdi = e.dst_reg(*dst, T0);
                    let t = match (from, to.width(), to.is_signed()) {
                        (Type::F64, Some(64), true) => "fcvt.l.d {r}, {f}, rtz",
                        (Type::F64, Some(32), true) => "fcvt.w.d {r}, {f}, rtz",
                        (Type::F32, Some(64), true) => "fcvt.l.s {r}, {f}, rtz",
                        (Type::F32, Some(32), true) => "fcvt.w.s {r}, {f}, rtz",
                        (Type::F64, Some(64), false) => "fcvt.lu.d {r}, {f}, rtz",
                        (Type::F64, Some(32), false) => "fcvt.wu.d {r}, {f}, rtz",
                        (Type::F32, Some(64), false) => "fcvt.lu.s {r}, {f}, rtz",
                        (Type::F32, Some(32), false) => "fcvt.wu.s {r}, {f}, rtz",
                        _ => unreachable!(),
                    };
                    e.emit(t, &[rdi, rs])?;
                    return e.finish(*dst, rdi);
                }
                (CastOp::Fpromote, _, _) => {
                    let rs = e.src_reg(*src, FT0)?;
                    let rdf = e.dst_reg(*dst, FT1);
                    e.emit("fcvt.d.s {f}, {f}", &[rdf, rs])?;
                    return e.finish(*dst, rdf);
                }
                (CastOp::Fdemote, _, _) => {
                    let rs = e.src_reg(*src, FT0)?;
                    let rdf = e.dst_reg(*dst, FT1);
                    e.emit("fcvt.s.d {f}, {f}", &[rdf, rs])?;
                    return e.finish(*dst, rdf);
                }
                (CastOp::Bitcast, _, _) => {
                    let (sf, df) = (from.is_float(), to.is_float());
                    let rs = e.src_reg(*src, if sf { FT0 } else { T0 })?;
                    let rdc = e.dst_reg(*dst, if df { FT0 } else { T0 });
                    let t = match (sf, df, if sf { from } else { to }) {
                        (false, true, Type::F64) => "fmv.d.x {f}, {r}",
                        (true, false, Type::F64) => "fmv.x.d {r}, {f}",
                        (false, true, _) => "fmv.w.x {f}, {r}",
                        (true, false, _) => "fmv.x.w {r}, {f}",
                        _ => unreachable!(),
                    };
                    e.emit(t, &[rdc, rs])?;
                    return e.finish(*dst, rdc);
                }
                _ => return Err(format!("unsupported cast {:?} -> {:?}", from, to)),
            }
            e.finish(*dst, rd)
        }
        Inst::Load { dst, addr } => {
            let ty = e.func.ty(*dst);
            let ra = e.src_reg(*addr, T0)?;
            let rd = e.dst_reg(*dst, if ty.is_float() { FT0 } else { T1 });
            let t = match ty {
                Type::F64 => "fld {f}, {i -2048..2047}({r})",
                Type::F32 => "flw {f}, {i -2048..2047}({r})",
                t if t.width() == Some(32) => "lw {r}, {i -2048..2047}({r})",
                _ => LD,
            };
            e.emit(t, &[rd, 0, ra])?;
            e.finish(*dst, rd)
        }
        Inst::Store { val, addr } => {
            let ty = e.func.ty(*val);
            let rv = e.src_reg(*val, if ty.is_float() { FT0 } else { T1 })?;
            let ra = e.src_reg(*addr, T0)?;
            let t = match ty {
                Type::F64 => "fsd {f}, {i -2048..2047}({r})",
                Type::F32 => "fsw {f}, {i -2048..2047}({r})",
                t if t.width() == Some(32) => "sw {r}, {i -2048..2047}({r})",
                _ => SD,
            };
            e.emit(t, &[rv, 0, ra])?;
            Ok(())
        }
        Inst::PtrAdd { dst, base, off } => {
            let rb = e.src_reg(*base, T0)?;
            let ro = e.src_reg(*off, T1)?;
            let rd = e.dst_reg(*dst, T0);
            e.emit("add {r}, {r}, {r}", &[rd, rb, ro])?;
            e.finish(*dst, rd)
        }
        Inst::Call { dsts, callee, args } => {
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            let (mut gi, mut fi) = (0i64, 0i64);
            for &a in args {
                if e.func.ty(a).is_float() {
                    e.value_to(10 + fi, a)?; // fa0..
                    fi += 1;
                } else {
                    e.value_to(A0 + gi, a)?;
                    gi += 1;
                }
            }
            let at = e.emit(JAL, &[RA, 0])?;
            e.fixups.push(Fixup {
                at,
                values: vec![RA, 0],
                imm_slot: 1,
                target: FixTarget::Func(callee.clone()),
            });
            let (mut gi, mut fi) = (0i64, 0i64);
            for &d in dsts {
                if e.func.ty(d).is_float() {
                    e.value_from(d, 10 + fi)?;
                    fi += 1;
                } else {
                    e.value_from(d, A0 + gi)?;
                    gi += 1;
                }
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
            // cond == 0 hops over the then-side moves + jump; the hop is a
            // local distance (tens of bytes), patched once the then side is
            // emitted, so argument moves only run on the taken path
            let beq_at = e.emit(BEQ, &[rc, ZERO, 0])?;
            e.branch_args(*then_target, then_args)?;
            e.goto(*then_target)?;
            let else_here = e.code.len() as i64 - beq_at as i64;
            let word = e.enc.encode(BEQ, &[rc, ZERO, else_here])?;
            e.code[beq_at..beq_at + 4].copy_from_slice(&word.to_le_bytes());
            e.branch_args(*else_target, else_args)?;
            e.goto(*else_target)
        }
        Inst::Extract { .. } | Inst::Pack { .. } | Inst::Insert { .. } => {
            unreachable!("struct ops are lowered before emission")
        }
        Inst::Ret { vals } => {
            if vals.len() > 8 {
                return Err("more than 8 return values not supported yet".into());
            }
            let (mut gi, mut fi) = (0i64, 0i64);
            for &v in vals {
                if e.func.ty(v).is_float() {
                    e.value_to(10 + fi, v)?;
                    fi += 1;
                } else {
                    e.value_to(A0 + gi, v)?;
                    gi += 1;
                }
            }
            e.epilogue()
        }
    }
}
