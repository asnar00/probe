//! The wasm emitter: SSA -> WebAssembly binary, instruction bytes coming
//! ONLY from the learned encoding table (targets/wasm32.encodings.json).
//!
//! As on arm64, strategy lives here and encodings don't. Every SSA value
//! becomes a typed wasm local; each instruction is local.get operands,
//! apply the learned opcode, local.set the result — the register-machine
//! backend's "everything in a slot" strategy is, on a stack machine, simply
//! idiomatic code.
//!
//! Control flow: wasm has no arbitrary branches, so each function becomes a
//! dispatcher — one loop wrapping N nested blocks, a `label` local
//! selecting the SSA block to run via a chain of br_ifs. A jump sets
//! `label` and branches back to the loop. Ugly output, correct for any
//! CFG, and immune to the relooper problem.
//!
//! The module container (sections, type/function/export tables, body
//! sizes, locals declarations) is written with spec knowledge: that's the
//! file format — the analogue of the Mach-O/mmap layer on arm64 — not the
//! instruction encoding.
//!
//! Types: i64 -> i64; i32/i1 -> i32 (i1 as 0/1); ptr -> i32 (an offset
//! into the module's linear memory).

use crate::ssa::{BinOp, CastOp, Cond, Function, Inst, Module, Type, ValueId};
use crate::wlearn::{encode_pieces, uleb, Piece};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Encoder over the learned byte templates

pub struct WEncoder {
    /// keyed by the template text with its slot replaced by "{}"
    insts: HashMap<String, Vec<Piece>>,
    pub end: u8,
}

impl WEncoder {
    pub fn load(path: &str) -> Result<WEncoder, String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
        // reuse the tiny JSON reader from emit.rs via a local parse
        let root = crate::emit::parse_json_pub(&src)?;
        let end_s = root
            .get("end")
            .and_then(|j| j.s())
            .ok_or("no 'end' opcode in encodings")?;
        let end = u8::from_str_radix(end_s.trim_start_matches("0x"), 16)
            .map_err(|e| format!("bad end byte: {}", e))?;
        let mut insts = HashMap::new();
        for item in root
            .get("instructions")
            .and_then(|j| j.a())
            .ok_or("no 'instructions' array")?
        {
            let template = item
                .get("template")
                .and_then(|j| j.s())
                .ok_or("no template")?;
            let key = normalize_key(template);
            let mut pieces = Vec::new();
            for p in item.get("pieces").and_then(|j| j.a()).unwrap_or(&[]) {
                if let Some(hexs) = p.get("fixed").and_then(|j| j.s()) {
                    let bytes: Result<Vec<u8>, _> = (0..hexs.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&hexs[i..i + 2], 16))
                        .collect();
                    pieces.push(Piece::Fixed(bytes.map_err(|e| e.to_string())?));
                } else if p.get("uleb").is_some() {
                    pieces.push(Piece::ULeb);
                } else if p.get("sleb").is_some() {
                    pieces.push(Piece::SLeb);
                } else if p.get("bits64").is_some() {
                    pieces.push(Piece::Bits64);
                } else if p.get("bits32").is_some() {
                    pieces.push(Piece::Bits32);
                } else {
                    return Err("unknown piece kind".into());
                }
            }
            insts.insert(key, pieces);
        }
        insts.insert("end".into(), vec![Piece::Fixed(vec![end])]);
        Ok(WEncoder { insts, end })
    }

    pub fn op(&self, key: &str, value: Option<i64>, out: &mut Vec<u8>) -> Result<(), String> {
        let pieces = self
            .insts
            .get(key)
            .ok_or_else(|| format!("template not in wasm encoding table: '{}'", key))?;
        out.extend(encode_pieces(pieces, value));
        Ok(())
    }
}

/// "local.get {i 0..16384}" -> "local.get {}", "i64.add" -> "i64.add"
fn normalize_key(template: &str) -> String {
    match template.find('{') {
        Some(open) => {
            let close = template[open..].find('}').unwrap() + open;
            format!("{}{{}}{}", &template[..open], &template[close + 1..])
        }
        None => template.to_string(),
    }
    .trim()
    .to_string()
}

// ---------------------------------------------------------------------------
// Compilation

fn valtype(ty: Type) -> u8 {
    match ty {
        Type::I64 => 0x7E,
        Type::I32 | Type::I1 | Type::Ptr => 0x7F,
        Type::F64 => 0x7C,
        Type::F32 => 0x7D,
        Type::Int | Type::Float => {
            unreachable!("abstract types are resolved before emission")
        }
    }
}

fn is64(ty: Type) -> bool {
    ty == Type::I64
}

pub fn compile(module: &Module, enc: &WEncoder) -> Result<Vec<u8>, String> {
    // function name -> (index, result count), in module order
    let mut findex = HashMap::new();
    for (i, f) in module.funcs.iter().enumerate() {
        findex.insert(f.name.clone(), (i as i64, f.rets.len()));
    }

    // type section entries, deduplicated
    let mut types: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut ftype = Vec::new();
    for f in module.funcs.iter() {
        let params: Vec<u8> = f.params.iter().map(|&p| valtype(f.ty(p))).collect();
        let results: Vec<u8> = f.rets.iter().map(|&t| valtype(t)).collect();
        let sig = (params, results);
        let idx = match types.iter().position(|t| *t == sig) {
            Some(i) => i,
            None => {
                types.push(sig);
                types.len() - 1
            }
        };
        ftype.push(idx);
    }

    let mut bodies = Vec::new();
    for f in &module.funcs {
        bodies.push(
            compile_function(f, enc, &findex).map_err(|e| format!("@{}: {}", f.name, e))?,
        );
    }

    // ---- assemble the container ----
    let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
    let section = |out: &mut Vec<u8>, id: u8, payload: Vec<u8>| {
        out.push(id);
        out.extend(uleb(payload.len() as u64));
        out.extend(payload);
    };

    let mut p = uleb(types.len() as u64);
    for (params, results) in &types {
        p.push(0x60);
        p.extend(uleb(params.len() as u64));
        p.extend(params);
        p.extend(uleb(results.len() as u64));
        p.extend(results);
    }
    section(&mut out, 1, p);

    let mut p = uleb(module.funcs.len() as u64);
    for &t in &ftype {
        p.extend(uleb(t as u64));
    }
    section(&mut out, 3, p);

    // one memory, min 1 page (64KB) — plenty for the suite's buffers
    section(&mut out, 5, vec![0x01, 0x00, 0x01]);

    let mut p = uleb(module.funcs.len() as u64 + 1);
    for (i, f) in module.funcs.iter().enumerate() {
        p.extend(uleb(f.name.len() as u64));
        p.extend(f.name.as_bytes());
        p.push(0x00); // func
        p.extend(uleb(i as u64));
    }
    p.extend(uleb("memory".len() as u64));
    p.extend("memory".as_bytes());
    p.push(0x02); // memory
    p.extend(uleb(0));
    section(&mut out, 7, p);

    let mut p = uleb(bodies.len() as u64);
    for b in bodies {
        p.extend(uleb(b.len() as u64));
        p.extend(b);
    }
    section(&mut out, 10, p);

    Ok(out)
}

struct WEmit<'a> {
    enc: &'a WEncoder,
    func: &'a Function,
    findex: &'a HashMap<String, (i64, usize)>,
    code: Vec<u8>,
    local_of: Vec<i64>, // ValueId -> wasm local index
    label_local: i64,
    nblocks: usize,
}

impl WEmit<'_> {
    fn op(&mut self, key: &str, v: Option<i64>) -> Result<(), String> {
        let mut buf = std::mem::take(&mut self.code);
        let r = self.enc.op(key, v, &mut buf);
        self.code = buf;
        r
    }

    fn get(&mut self, v: ValueId) -> Result<(), String> {
        let idx = self.local_of[v.0 as usize];
        self.op("local.get {}", Some(idx))
    }

    fn set(&mut self, v: ValueId) -> Result<(), String> {
        let idx = self.local_of[v.0 as usize];
        self.op("local.set {}", Some(idx))
    }

    /// jump to SSA block `target`: pass args, set the label, br to the
    /// dispatcher loop. `extra_depth` counts if/else nesting at this point.
    fn jump(
        &mut self,
        target: crate::ssa::BlockId,
        args: &[ValueId],
        block_pos: usize,
        extra_depth: i64,
    ) -> Result<(), String> {
        // stack is the swap-safe staging area: push all args, then pop
        // into the target's params in reverse
        for &a in args {
            self.get(a)?;
        }
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        for &p in params.iter().rev() {
            self.set(p)?;
        }
        self.op("i32.const {}", Some(target.0 as i64))?;
        self.op("local.set {}", Some(self.label_local))?;
        let loop_depth = (self.nblocks - 1 - block_pos) as i64 + extra_depth;
        self.op("br {}", Some(loop_depth))
    }
}

fn compile_function(
    func: &Function,
    enc: &WEncoder,
    findex: &HashMap<String, (i64, usize)>,
) -> Result<Vec<u8>, String> {
    // locals: params first (that's the wasm rule), then the other SSA
    // values in id order, then the dispatcher's label local (i32)
    let n = func.values.len();
    let mut local_of = vec![-1i64; n];
    for (i, &p) in func.params.iter().enumerate() {
        local_of[p.0 as usize] = i as i64;
    }
    let mut next = func.params.len() as i64;
    let mut extra_types = Vec::new(); // declared locals, in order
    for id in 0..n {
        if local_of[id] < 0 {
            local_of[id] = next;
            next += 1;
            extra_types.push(valtype(func.ty(ValueId(id as u32))));
        }
    }
    let label_local = next;
    extra_types.push(0x7F);

    let mut e = WEmit {
        enc,
        func,
        findex,
        code: Vec::new(),
        local_of,
        label_local,
        nblocks: func.blocks.len(),
    };

    // dispatcher skeleton: loop { block^N { chain } code_0 } ... }
    e.op("loop", None)?;
    for _ in 0..e.nblocks {
        e.op("block", None)?;
    }
    for i in 0..e.nblocks {
        e.op("local.get {}", Some(e.label_local))?;
        e.op("i32.const {}", Some(i as i64))?;
        e.op("i32.eq", None)?;
        e.op("br_if {}", Some(i as i64))?;
    }
    e.op("unreachable", None)?;
    for (bi, block) in func.blocks.iter().enumerate() {
        e.op("end", None)?; // close block bi; its code follows
        for inst in &block.insts {
            compile_inst(&mut e, inst, bi)?;
        }
    }
    e.op("end", None)?; // close the loop
    e.op("unreachable", None)?;

    // body = locals declaration + code + end
    let mut body = Vec::new();
    let mut decls: Vec<(u64, u8)> = Vec::new();
    for t in extra_types {
        match decls.last_mut() {
            Some((c, ty)) if *ty == t => *c += 1,
            _ => decls.push((1, t)),
        }
    }
    body.extend(uleb(decls.len() as u64));
    for (c, ty) in decls {
        body.extend(uleb(c));
        body.push(ty);
    }
    body.extend(e.code);
    body.push(enc.end);
    Ok(body)
}

fn binop_key(op: BinOp, ty: Type) -> String {
    if op.is_float() {
        let base = match op {
            BinOp::FAdd => "add",
            BinOp::FSub => "sub",
            BinOp::FMul => "mul",
            BinOp::FDiv => "div",
            _ => unreachable!(),
        };
        return format!("{}.{}", if ty == Type::F64 { "f64" } else { "f32" }, base);
    }
    let base = match op {
        BinOp::IAdd => "add",
        BinOp::ISub => "sub",
        BinOp::IMul => "mul",
        BinOp::SDiv => "div_s",
        BinOp::UDiv => "div_u",
        BinOp::SRem => "rem_s",
        BinOp::URem => "rem_u",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Xor => "xor",
        BinOp::Shl => "shl",
        BinOp::LShr => "shr_u",
        BinOp::AShr => "shr_s",
        _ => unreachable!(),
    };
    format!("{}.{}", if is64(ty) { "i64" } else { "i32" }, base)
}

fn fcmp_key(cond: crate::ssa::FCond, ty: Type) -> String {
    use crate::ssa::FCond;
    let base = match cond {
        FCond::Oeq => "eq",
        FCond::Une => "ne",
        FCond::Olt => "lt",
        FCond::Ole => "le",
        FCond::Ogt => "gt",
        FCond::Oge => "ge",
    };
    format!("{}.{}", if ty == Type::F64 { "f64" } else { "f32" }, base)
}

fn cmp_key(cond: Cond, ty: Type) -> String {
    let base = match cond {
        Cond::Eq => "eq",
        Cond::Ne => "ne",
        Cond::Slt => "lt_s",
        Cond::Sle => "le_s",
        Cond::Sgt => "gt_s",
        Cond::Sge => "ge_s",
        Cond::Ult => "lt_u",
        Cond::Ule => "le_u",
        Cond::Ugt => "gt_u",
        Cond::Uge => "ge_u",
    };
    format!("{}.{}", if is64(ty) { "i64" } else { "i32" }, base)
}

fn compile_inst(e: &mut WEmit, inst: &Inst, block_pos: usize) -> Result<(), String> {
    match inst {
        Inst::IConst { dst, imm } => {
            if is64(e.func.ty(*dst)) {
                e.op("i64.const {}", Some(*imm))?;
            } else {
                e.op("i32.const {}", Some(*imm as i32 as i64))?;
            }
            e.set(*dst)
        }
        Inst::FConst { dst, bits } => {
            if e.func.ty(*dst) == Type::F64 {
                e.op("f64.const {}", Some(*bits as i64))?;
            } else {
                let b = (f64::from_bits(*bits) as f32).to_bits() as i64;
                e.op("f32.const {}", Some(b))?;
            }
            e.set(*dst)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            e.get(*lhs)?;
            e.get(*rhs)?;
            e.op(&binop_key(*op, e.func.ty(*dst)), None)?;
            e.set(*dst)
        }
        Inst::FCmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            e.get(*lhs)?;
            e.get(*rhs)?;
            e.op(&fcmp_key(*cond, e.func.ty(*lhs)), None)?;
            e.set(*dst)
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            e.get(*lhs)?;
            e.get(*rhs)?;
            e.op(&cmp_key(*cond, e.func.ty(*lhs)), None)?;
            e.set(*dst)
        }
        Inst::Cast { op, dst, src } => {
            let from = e.func.ty(*src);
            let to = e.func.ty(*dst);
            match (op, from, to) {
                // i1 sign-extension: 0/1 -> 0/-1, computed as 0 - v
                (CastOp::Sext, Type::I1, Type::I64) => {
                    e.op("i64.const {}", Some(0))?;
                    e.get(*src)?;
                    e.op("i64.extend_i32_u", None)?;
                    e.op("i64.sub", None)?;
                }
                (CastOp::Sext, Type::I1, Type::I32) => {
                    e.op("i32.const {}", Some(0))?;
                    e.get(*src)?;
                    e.op("i32.sub", None)?;
                }
                (CastOp::Sext, Type::I32, Type::I64) => {
                    e.get(*src)?;
                    e.op("i64.extend_i32_s", None)?;
                }
                (CastOp::Zext, Type::I1, Type::I64) | (CastOp::Zext, Type::I32, Type::I64) => {
                    e.get(*src)?;
                    e.op("i64.extend_i32_u", None)?;
                }
                (CastOp::Zext, Type::I1, Type::I32) => {
                    e.get(*src)?; // already a 0/1 i32
                }
                (CastOp::Trunc, Type::I64, Type::I32) => {
                    e.get(*src)?;
                    e.op("i32.wrap_i64", None)?;
                }
                (CastOp::Trunc, Type::I64, Type::I1) => {
                    e.get(*src)?;
                    e.op("i32.wrap_i64", None)?;
                    e.op("i32.const {}", Some(1))?;
                    e.op("i32.and", None)?;
                }
                (CastOp::Trunc, Type::I32, Type::I1) => {
                    e.get(*src)?;
                    e.op("i32.const {}", Some(1))?;
                    e.op("i32.and", None)?;
                }
                (CastOp::Sitofp, _, _) | (CastOp::Uitofp, _, _) => {
                    e.get(*src)?;
                    let u = if matches!(op, CastOp::Uitofp) { "u" } else { "s" };
                    let f = if to == Type::F64 { "f64" } else { "f32" };
                    let i = if from == Type::I64 { "i64" } else { "i32" };
                    e.op(&format!("{}.convert_{}_{}", f, i, u), None)?;
                }
                (CastOp::Fptosi, _, _) | (CastOp::Fptoui, _, _) => {
                    e.get(*src)?;
                    let u = if matches!(op, CastOp::Fptoui) { "u" } else { "s" };
                    let f = if from == Type::F64 { "f64" } else { "f32" };
                    let i = if to == Type::I64 { "i64" } else { "i32" };
                    e.op(&format!("{}.trunc_{}_{}", i, f, u), None)?;
                }
                (CastOp::Fpromote, _, _) => {
                    e.get(*src)?;
                    e.op("f64.promote_f32", None)?;
                }
                (CastOp::Fdemote, _, _) => {
                    e.get(*src)?;
                    e.op("f32.demote_f64", None)?;
                }
                (CastOp::Bitcast, _, _) => {
                    e.get(*src)?;
                    let k = match (from, to) {
                        (Type::I64, Type::F64) => "f64.reinterpret_i64",
                        (Type::F64, Type::I64) => "i64.reinterpret_f64",
                        (Type::I32, Type::F32) => "f32.reinterpret_i32",
                        (Type::F32, Type::I32) => "i32.reinterpret_f32",
                        _ => unreachable!(),
                    };
                    e.op(k, None)?;
                }
                _ => return Err(format!("unsupported cast {:?} -> {:?}", from, to)),
            }
            e.set(*dst)
        }
        Inst::Load { dst, addr } => {
            e.get(*addr)?;
            let k = match e.func.ty(*dst) {
                Type::F64 => "f64.load offset={}",
                Type::F32 => "f32.load offset={}",
                Type::I64 => "i64.load offset={}",
                _ => "i32.load offset={}",
            };
            e.op(k, Some(0))?;
            e.set(*dst)
        }
        Inst::Store { val, addr } => {
            e.get(*addr)?;
            e.get(*val)?;
            let k = match e.func.ty(*val) {
                Type::F64 => "f64.store offset={}",
                Type::F32 => "f32.store offset={}",
                Type::I64 => "i64.store offset={}",
                _ => "i32.store offset={}",
            };
            e.op(k, Some(0))
        }
        Inst::PtrAdd { dst, base, off } => {
            e.get(*base)?;
            e.get(*off)?;
            e.op("i32.wrap_i64", None)?;
            e.op("i32.add", None)?;
            e.set(*dst)
        }
        Inst::Call { dsts, callee, args } => {
            for &a in args {
                e.get(a)?;
            }
            let (idx, nrets) = *e
                .findex
                .get(callee)
                .ok_or_else(|| format!("call to undefined function @{}", callee))?;
            e.op("call {}", Some(idx))?;
            if dsts.is_empty() {
                for _ in 0..nrets {
                    e.op("drop", None)?; // results ignored at the call site
                }
            }
            for &d in dsts.iter().rev() {
                e.set(d)?;
            }
            Ok(())
        }
        Inst::Jmp { target, args } => e.jump(*target, args, block_pos, 0),
        Inst::Br {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            e.get(*cond)?;
            e.op("if", None)?;
            e.jump(*then_target, then_args, block_pos, 1)?;
            e.op("else", None)?;
            e.jump(*else_target, else_args, block_pos, 1)?;
            e.op("end", None)
        }
        Inst::Ret { vals } => {
            for &v in vals {
                e.get(v)?;
            }
            e.op("return", None)
        }
    }
}
