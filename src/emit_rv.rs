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

/// pool for the allocator: callee-saved s2..s11 (x18..x27) — values placed
/// here survive calls by construction
const REG_POOL: &[i64] = &[18, 19, 20, 21, 22, 23, 24, 25, 26, 27];

const ZERO: i64 = 0; // x0
const RA: i64 = 1;
const SP: i64 = 2;
const T0: i64 = 5;
const T1: i64 = 6;
const A0: i64 = 10;

const ADDI: &str = "addi {r}, {r}, {i -2048..2047}";
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

    fn src_reg(&mut self, v: ValueId, scratch: i64) -> Result<i64, String> {
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
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => r,
            Loc::Slot(_) => scratch,
        }
    }

    fn finish(&mut self, v: ValueId, reg: i64) -> Result<(), String> {
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
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => self.mov(target, r),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(LD, &[target, off, SP]).map(|_| ())
            }
        }
    }

    fn value_from(&mut self, v: ValueId, source: i64) -> Result<(), String> {
        match self.alloc.loc[v.0 as usize] {
            Loc::Reg(r) => self.mov(r, source),
            Loc::Slot(i) => {
                let off = self.slot_off(i);
                self.emit(SD, &[source, off, SP]).map(|_| ())
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

    /// branch arguments: two phases through a0..a7 — all sources read
    /// before any target parameter is written, so swaps can't clobber
    fn branch_args(&mut self, target: BlockId, args: &[ValueId]) -> Result<(), String> {
        if args.len() > 8 {
            return Err("more than 8 branch arguments not supported yet".into());
        }
        for (j, &a) in args.iter().enumerate() {
            self.value_to(A0 + j as i64, a)?;
        }
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        for (j, &p) in params.iter().enumerate() {
            self.value_from(p, A0 + j as i64)?;
        }
        Ok(())
    }

    fn epilogue(&mut self) -> Result<(), String> {
        for (k, &r) in self.alloc.used_regs.clone().iter().enumerate() {
            self.emit(LD, &[r, 16 + 8 * k as i64, SP])?;
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
    let alloc = regalloc::allocate(func, REG_POOL);
    let nsaved = alloc.used_regs.len() as i64;
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
    for (k, &r) in alloc.used_regs.iter().enumerate() {
        e.emit(SD, &[r, 16 + 8 * k as i64, SP])?;
    }
    for (i, &p) in func.params.iter().enumerate() {
        e.value_from(p, A0 + i as i64)?;
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
    let w = ty == Type::I32;
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
        BinOp::SDiv => {
            if w {
                "divw {r}, {r}, {r}"
            } else {
                "div {r}, {r}, {r}"
            }
        }
        BinOp::UDiv => {
            if w {
                "divuw {r}, {r}, {r}"
            } else {
                "divu {r}, {r}, {r}"
            }
        }
        BinOp::SRem => {
            if w {
                "remw {r}, {r}, {r}"
            } else {
                "rem {r}, {r}, {r}"
            }
        }
        BinOp::URem => {
            if w {
                "remuw {r}, {r}, {r}"
            } else {
                "remu {r}, {r}, {r}"
            }
        }
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
        BinOp::LShr => {
            if w {
                "srlw {r}, {r}, {r}"
            } else {
                "srl {r}, {r}, {r}"
            }
        }
        BinOp::AShr => {
            if w {
                "sraw {r}, {r}, {r}"
            } else {
                "sra {r}, {r}, {r}"
            }
        }
    }
}

fn compile_inst(e: &mut RvEmit, inst: &Inst) -> Result<(), String> {
    const SLT: &str = "slt {r}, {r}, {r}";
    const SLTU: &str = "sltu {r}, {r}, {r}";
    const XORI: &str = "xori {r}, {r}, {i -2048..2047}";
    match inst {
        Inst::IConst { dst, imm } => {
            // i32 constants live sign-extended, per the W convention
            let v = if e.func.ty(*dst) == Type::I32 {
                *imm as i32 as i64
            } else {
                *imm
            };
            let rd = e.dst_reg(*dst, T0);
            e.iconst(rd, v)?;
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
            let rl = e.src_reg(*lhs, T0)?;
            let rr = e.src_reg(*rhs, T1)?;
            let rd = e.dst_reg(*dst, T0);
            // sign-extended i32 values compare correctly with 64-bit slt/sltu
            match cond {
                Cond::Slt => {
                    e.emit(SLT, &[rd, rl, rr])?;
                }
                Cond::Ult => {
                    e.emit(SLTU, &[rd, rl, rr])?;
                }
                Cond::Sgt => {
                    e.emit(SLT, &[rd, rr, rl])?;
                }
                Cond::Ugt => {
                    e.emit(SLTU, &[rd, rr, rl])?;
                }
                Cond::Sge => {
                    e.emit(SLT, &[rd, rl, rr])?;
                    e.emit(XORI, &[rd, rd, 1])?;
                }
                Cond::Uge => {
                    e.emit(SLTU, &[rd, rl, rr])?;
                    e.emit(XORI, &[rd, rd, 1])?;
                }
                Cond::Sle => {
                    e.emit(SLT, &[rd, rr, rl])?;
                    e.emit(XORI, &[rd, rd, 1])?;
                }
                Cond::Ule => {
                    e.emit(SLTU, &[rd, rr, rl])?;
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
            match (op, from, to) {
                // i1 is 0/1; sign-extension is negation
                (CastOp::Sext, Type::I1, _) => {
                    e.emit("sub {r}, {r}, {r}", &[rd, ZERO, rs])?;
                }
                // i32 values are already sign-extended
                (CastOp::Sext, Type::I32, Type::I64) | (CastOp::Zext, Type::I1, _) => {
                    e.mov(rd, rs)?;
                }
                (CastOp::Zext, Type::I32, Type::I64) => {
                    e.emit("slli {r}, {r}, {i 0..63}", &[rd, rs, 32])?;
                    e.emit("srli {r}, {r}, {i 0..63}", &[rd, rd, 32])?;
                }
                (CastOp::Trunc, Type::I64, Type::I32) => {
                    e.emit("addiw {r}, {r}, {i -2048..2047}", &[rd, rs, 0])?;
                }
                (CastOp::Trunc, _, Type::I1) => {
                    e.emit("andi {r}, {r}, {i -2048..2047}", &[rd, rs, 1])?;
                }
                _ => return Err(format!("unsupported cast {:?} -> {:?}", from, to)),
            }
            e.finish(*dst, rd)
        }
        Inst::Load { dst, addr } => {
            let ra = e.src_reg(*addr, T0)?;
            let rd = e.dst_reg(*dst, T1);
            if e.func.ty(*dst) == Type::I32 {
                e.emit("lw {r}, {i -2048..2047}({r})", &[rd, 0, ra])?;
            } else {
                e.emit(LD, &[rd, 0, ra])?;
            }
            e.finish(*dst, rd)
        }
        Inst::Store { val, addr } => {
            let rv = e.src_reg(*val, T1)?;
            let ra = e.src_reg(*addr, T0)?;
            if e.func.ty(*val) == Type::I32 {
                e.emit("sw {r}, {i -2048..2047}({r})", &[rv, 0, ra])?;
            } else {
                e.emit(SD, &[rv, 0, ra])?;
            }
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
