//! The riscv64 emitter: SSA -> RV64IM machine code, every instruction word
//! encoded from the learned table (targets/riscv64.encodings.json) via the
//! same Encoder the arm64 backend uses — the JSON format is identical.
//!
//! Strategy mirrors the arm64 backend: every SSA value gets a stack slot;
//! each instruction loads operands into scratch registers, computes, and
//! stores back. Differences that are genuinely RISC-V:
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
//! x10-x17 (a0-a7) arguments, results, and branch-argument staging.
//!
//! Frame: sp+0 saved ra, sp+16+8i value slots; sp fixed for the body.

use crate::emit::{Compiled, Encoder};
use crate::ssa::{BinOp, BlockId, CastOp, Cond, Function, Inst, Module, Type, ValueId};

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

    fn slot(&self, v: ValueId) -> i64 {
        16 + 8 * v.0 as i64
    }

    fn load(&mut self, reg: i64, v: ValueId) -> Result<(), String> {
        let off = self.slot(v);
        self.emit(LD, &[reg, off, SP]).map(|_| ())
    }

    fn store(&mut self, reg: i64, v: ValueId) -> Result<(), String> {
        let off = self.slot(v);
        self.emit(SD, &[reg, off, SP]).map(|_| ())
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

    /// branch arguments: load all into a0.. then store — swap-safe
    fn branch_args(&mut self, target: BlockId, args: &[ValueId]) -> Result<(), String> {
        if args.len() > 8 {
            return Err("more than 8 branch arguments not supported yet".into());
        }
        for (j, &a) in args.iter().enumerate() {
            self.load(A0 + j as i64, a)?;
        }
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        for (j, &p) in params.iter().enumerate() {
            self.store(A0 + j as i64, p)?;
        }
        Ok(())
    }

    fn epilogue(&mut self) -> Result<(), String> {
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
    let nslots = func.values.len() as i64;
    let frame = (16 + 8 * nslots + 15) & !15;
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
        block_offsets: vec![None; func.blocks.len()],
        fixups: Vec::new(),
    };

    e.emit(ADDI, &[SP, SP, -frame])?;
    e.emit(SD, &[RA, 0, SP])?;
    for (i, &p) in func.params.iter().enumerate() {
        e.store(A0 + i as i64, p)?;
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
            e.iconst(T0, v)?;
            e.store(T0, *dst)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            e.load(T0, *lhs)?;
            e.load(T1, *rhs)?;
            e.emit(bin_template(*op, e.func.ty(*dst)), &[T0, T0, T1])?;
            e.store(T0, *dst)
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            e.load(T0, *lhs)?;
            e.load(T1, *rhs)?;
            // sign-extended i32 slots compare correctly with 64-bit slt/sltu
            match cond {
                Cond::Slt => {
                    e.emit(SLT, &[T0, T0, T1])?;
                }
                Cond::Ult => {
                    e.emit(SLTU, &[T0, T0, T1])?;
                }
                Cond::Sgt => {
                    e.emit(SLT, &[T0, T1, T0])?;
                }
                Cond::Ugt => {
                    e.emit(SLTU, &[T0, T1, T0])?;
                }
                Cond::Sge => {
                    e.emit(SLT, &[T0, T0, T1])?;
                    e.emit(XORI, &[T0, T0, 1])?;
                }
                Cond::Uge => {
                    e.emit(SLTU, &[T0, T0, T1])?;
                    e.emit(XORI, &[T0, T0, 1])?;
                }
                Cond::Sle => {
                    e.emit(SLT, &[T0, T1, T0])?;
                    e.emit(XORI, &[T0, T0, 1])?;
                }
                Cond::Ule => {
                    e.emit(SLTU, &[T0, T1, T0])?;
                    e.emit(XORI, &[T0, T0, 1])?;
                }
                Cond::Eq => {
                    e.emit("xor {r}, {r}, {r}", &[T0, T0, T1])?;
                    e.emit("sltiu {r}, {r}, {i -2048..2047}", &[T0, T0, 1])?;
                }
                Cond::Ne => {
                    e.emit("xor {r}, {r}, {r}", &[T0, T0, T1])?;
                    e.emit(SLTU, &[T0, ZERO, T0])?;
                }
            }
            e.store(T0, *dst)
        }
        Inst::Cast { op, dst, src } => {
            let from = e.func.ty(*src);
            let to = e.func.ty(*dst);
            e.load(T0, *src)?;
            match (op, from, to) {
                // i1 is 0/1; sign-extension is negation
                (CastOp::Sext, Type::I1, _) => {
                    e.emit("sub {r}, {r}, {r}", &[T0, ZERO, T0])?;
                }
                // i32 slots are already sign-extended
                (CastOp::Sext, Type::I32, Type::I64) => {}
                (CastOp::Zext, Type::I1, _) => {}
                (CastOp::Zext, Type::I32, Type::I64) => {
                    e.emit("slli {r}, {r}, {i 0..63}", &[T0, T0, 32])?;
                    e.emit("srli {r}, {r}, {i 0..63}", &[T0, T0, 32])?;
                }
                (CastOp::Trunc, Type::I64, Type::I32) => {
                    e.emit("addiw {r}, {r}, {i -2048..2047}", &[T0, T0, 0])?;
                }
                (CastOp::Trunc, _, Type::I1) => {
                    e.emit("andi {r}, {r}, {i -2048..2047}", &[T0, T0, 1])?;
                }
                _ => return Err(format!("unsupported cast {:?} -> {:?}", from, to)),
            }
            e.store(T0, *dst)
        }
        Inst::Load { dst, addr } => {
            e.load(T0, *addr)?;
            if e.func.ty(*dst) == Type::I32 {
                e.emit("lw {r}, {i -2048..2047}({r})", &[T0, 0, T0])?;
            } else {
                e.emit(LD, &[T0, 0, T0])?;
            }
            e.store(T0, *dst)
        }
        Inst::Store { val, addr } => {
            e.load(T1, *val)?;
            e.load(T0, *addr)?;
            if e.func.ty(*val) == Type::I32 {
                e.emit("sw {r}, {i -2048..2047}({r})", &[T1, 0, T0])?;
            } else {
                e.emit(SD, &[T1, 0, T0])?;
            }
            Ok(())
        }
        Inst::PtrAdd { dst, base, off } => {
            e.load(T0, *base)?;
            e.load(T1, *off)?;
            e.emit("add {r}, {r}, {r}", &[T0, T0, T1])?;
            e.store(T0, *dst)
        }
        Inst::Call { dsts, callee, args } => {
            if args.len() > 8 {
                return Err("more than 8 call arguments not supported yet".into());
            }
            for (j, &a) in args.iter().enumerate() {
                e.load(A0 + j as i64, a)?;
            }
            let at = e.emit(JAL, &[RA, 0])?;
            e.fixups.push(Fixup {
                at,
                values: vec![RA, 0],
                imm_slot: 1,
                target: FixTarget::Func(callee.clone()),
            });
            for (j, &d) in dsts.iter().enumerate() {
                e.store(A0 + j as i64, d)?;
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
            e.load(T0, *cond)?;
            // cond == 0 hops over the then-side moves + jump; the hop is a
            // local distance (tens of bytes), patched once the then side is
            // emitted, so argument moves only run on the taken path
            let beq_at = e.emit(BEQ, &[T0, ZERO, 0])?;
            e.branch_args(*then_target, then_args)?;
            e.goto(*then_target)?;
            let else_here = e.code.len() as i64 - beq_at as i64;
            let word = e.enc.encode(BEQ, &[T0, ZERO, else_here])?;
            e.code[beq_at..beq_at + 4].copy_from_slice(&word.to_le_bytes());
            e.branch_args(*else_target, else_args)?;
            e.goto(*else_target)
        }
        Inst::Ret { vals } => {
            if vals.len() > 8 {
                return Err("more than 8 return values not supported yet".into());
            }
            for (j, &v) in vals.iter().enumerate() {
                e.load(A0 + j as i64, v)?;
            }
            e.epilogue()
        }
    }
}
