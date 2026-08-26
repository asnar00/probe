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
//! Types: anything up to 32 bits is an i32 local, wider is i64; ptr is an
//! i32 offset into the module's linear memory. Values are canonical in
//! their local (`iN` sign-extended, `uN`/ptr/packs zero-extended, see
//! `ssa::Repr`); narrow results re-normalize with a shift pair or a mask.

use crate::platform::{FOp, Kind, Native, Platform};
use crate::ssa::{BinOp, Cond, Function, Inst, Module, Repr, ValueId};
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

/// a value's representation on wasm: as in `Function::repr`, except that
/// pointers are 32-bit offsets into linear memory
pub fn wrepr(f: &Function, ty: crate::ssa::Type) -> Repr {
    if ty == crate::ssa::Type::Ptr {
        Repr::U(32)
    } else {
        f.repr(ty)
    }
}

fn valtype(r: Repr) -> u8 {
    if r.container() == 64 {
        0x7E
    } else {
        0x7F
    }
}

/// "i32" or "i64": the instruction prefix for a value's container
fn pfx(r: Repr) -> &'static str {
    if r.container() == 64 {
        "i64"
    } else {
        "i32"
    }
}

pub fn compile(module: &Module, enc: &WEncoder) -> Result<Vec<u8>, String> {
    compile_with(module, enc, &Platform::wasm32())
}

pub fn compile_with(module: &Module, enc: &WEncoder, platform: &Platform) -> Result<Vec<u8>, String> {
    let natives = platform.natives(module);
    // function name -> (index, result count), in module order
    let mut findex = HashMap::new();
    for (i, f) in module.funcs.iter().enumerate() {
        findex.insert(f.name.clone(), (i as i64, f.rets.len()));
    }

    // type section entries, deduplicated
    let mut types: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut ftype = Vec::new();
    for f in module.funcs.iter() {
        let params: Vec<u8> = f.params.iter().map(|&p| valtype(wrepr(f, f.ty(p)))).collect();
        let results: Vec<u8> = f.rets.iter().map(|&t| valtype(wrepr(f, t))).collect();
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
            compile_function(f, enc, &findex, &natives).map_err(|e| format!("{}: {}", f.name, e))?,
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
    natives: &'a HashMap<String, Native>,
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

    fn repr(&self, v: ValueId) -> Repr {
        wrepr(self.func, self.func.ty(v))
    }

    fn konst(&mut self, r: Repr, v: i64) -> Result<(), String> {
        if r.container() == 64 {
            self.op("i64.const {}", Some(v))
        } else {
            self.op("i32.const {}", Some(v as i32 as i64))
        }
    }

    /// re-normalize the stack top (in r's container) to canonical `r`
    fn norm(&mut self, r: Repr) -> Result<(), String> {
        let n = r.bits();
        let c = r.container();
        if n == c {
            return Ok(());
        }
        let p = pfx(r);
        if r.signed() {
            let k = (c - n) as i64;
            self.konst(r, k)?;
            self.op(&format!("{}.shl", p), None)?;
            self.konst(r, k)?;
            self.op(&format!("{}.shr_s", p), None)
        } else {
            self.konst(r, ((1u64 << n) - 1) as i64)?;
            self.op(&format!("{}.and", p), None)
        }
    }

    /// stack top: a canonical `from` value; leave a canonical `to` value
    fn cast(&mut self, from: Repr, to: Repr) -> Result<(), String> {
        match (from.container(), to.container()) {
            (32, 64) => {
                self.op(
                    if from.signed() { "i64.extend_i32_s" } else { "i64.extend_i32_u" },
                    None,
                )?;
                if !from.fits_in(to) {
                    self.norm(to)?;
                }
                Ok(())
            }
            (64, 32) => {
                self.op("i32.wrap_i64", None)?;
                self.norm(to)
            }
            _ => {
                if from.fits_in(to) {
                    Ok(())
                } else {
                    self.norm(to)
                }
            }
        }
    }

    /// stack top: a pack of container `c`; leave field (off, w) as canonical `fr`
    fn extract(&mut self, c: u32, off: u32, fr: Repr) -> Result<(), String> {
        let p = if c == 64 { "i64" } else { "i32" };
        if off > 0 {
            if c == 64 {
                self.op("i64.const {}", Some(off as i64))?;
            } else {
                self.op("i32.const {}", Some(off as i64))?;
            }
            self.op(&format!("{}.shr_u", p), None)?;
        }
        if c == 64 && fr.container() == 32 {
            self.op("i32.wrap_i64", None)?;
        }
        self.norm(fr)
    }

    /// stack top: a canonical value of `vr`; leave its low `w` bits moved
    /// to `off` in a container of `c`, zero elsewhere
    fn place(&mut self, c: u32, vr: Repr, off: u32, w: u32) -> Result<(), String> {
        let p = if c == 64 { "i64" } else { "i32" };
        if c == 64 && vr.container() == 32 {
            self.op("i64.extend_i32_u", None)?;
        }
        let cr = Repr::U(c);
        if w < c {
            self.konst(cr, ((1u64 << w) - 1) as i64)?;
            self.op(&format!("{}.and", p), None)?;
        }
        if off > 0 {
            self.konst(cr, off as i64)?;
            self.op(&format!("{}.shl", p), None)?;
        }
        Ok(())
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
    natives: &HashMap<String, Native>,
) -> Result<Vec<u8>, String> {
    if let Some(&op) = natives.get(&func.name) {
        // this function *is* a platform instruction: params -> result
        let mut body = vec![0u8]; // no extra locals
        for (key, v) in native_seq(op, 0, 1, 2) {
            enc.op(key, v, &mut body)?;
        }
        body.push(enc.end);
        return Ok(body);
    }
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
            extra_types.push(valtype(wrepr(func, func.ty(ValueId(id as u32)))));
        }
    }
    let label_local = next;
    extra_types.push(0x7F);

    let mut e = WEmit {
        enc,
        func,
        findex,
        natives,
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

/// the ops for a platform op, reading locals l0 (l1, l2), leaving the
/// integer-typed result on the stack
fn native_seq(op: Native, l0: i64, l1: i64, l2: i64) -> Vec<(&'static str, Option<i64>)> {
    match op {
        Native::Arith { op: fop, .. } => {
            let (to_f, t, to_i) = fp_keys(op);
            let mut seq = vec![("local.get {}", Some(l0)), (to_f, None)];
            if fop.arity() >= 2 {
                seq.extend([("local.get {}", Some(l1)), (to_f, None)]);
            }
            if fop.arity() == 3 {
                seq.extend([("local.get {}", Some(l2)), (to_f, None)]);
            }
            seq.extend([(t, None), (to_i, None)]);
            seq
        }
        Native::Cmp { cond, bits } => {
            let reinterp = if bits == 32 { "f32.reinterpret_i32" } else { "f64.reinterpret_i64" };
            let t: &'static str = Box::leak(format!("f{}.{}", bits, cond.name()).into_boxed_str());
            vec![
                ("local.get {}", Some(l0)),
                (reinterp, None),
                ("local.get {}", Some(l1)),
                (reinterp, None),
                (t, None),
            ]
        }
        Native::Conv { from, to } => {
            let reinterp_in = |k: Kind| if k.bits() == 32 { "f32.reinterpret_i32" } else { "f64.reinterpret_i64" };
            let reinterp_out = |k: Kind| if k.bits() == 32 { "i32.reinterpret_f32" } else { "i64.reinterpret_f64" };
            let mut seq = vec![("local.get {}", Some(l0))];
            match (from.is_float(), to.is_float()) {
                (true, true) => {
                    seq.push((reinterp_in(from), None));
                    seq.push((if to == Kind::F64 { "f64.promote_f32" } else { "f32.demote_f64" }, None));
                    seq.push((reinterp_out(to), None));
                }
                (false, true) => {
                    let t: &'static str = Box::leak(
                        format!(
                            "f{}.convert_i{}_{}",
                            to.bits(),
                            from.bits(),
                            if from.signed() { "s" } else { "u" }
                        )
                        .into_boxed_str(),
                    );
                    seq.push((t, None));
                    seq.push((reinterp_out(to), None));
                }
                (true, false) => {
                    seq.push((reinterp_in(from), None));
                    let t: &'static str = Box::leak(
                        format!(
                            "i{}.trunc_sat_f{}_{}",
                            to.bits(),
                            from.bits(),
                            if to.signed() { "s" } else { "u" }
                        )
                        .into_boxed_str(),
                    );
                    seq.push((t, None));
                }
                (false, false) => unreachable!("int to int is not a platform op"),
            }
            seq
        }
    }
}

/// (reinterpret in, the op, reinterpret out) for a platform float arithmetic op
fn fp_keys(op: Native) -> (&'static str, &'static str, &'static str) {
    let Native::Arith { op: fop, bits } = op else {
        unreachable!()
    };
    let name = match fop {
        FOp::Add => "add",
        FOp::Sub => "sub",
        FOp::Mul => "mul",
        FOp::Div => "div",
        FOp::Sqrt => "sqrt",
        FOp::Neg => "neg",
        FOp::Abs => "abs",
        FOp::Min => "min",
        FOp::Max => "max",
        FOp::Fma => unreachable!("not on the wasm platform"),
    };
    let fop: &'static str = Box::leak(format!("f{}.{}", bits, name).into_boxed_str());
    if bits == 32 {
        ("f32.reinterpret_i32", fop, "i32.reinterpret_f32")
    } else {
        ("f64.reinterpret_i64", fop, "i64.reinterpret_f64")
    }
}

fn binop_key(op: BinOp, r: Repr) -> String {
    let base = match (op, r.signed()) {
        (BinOp::IAdd, _) => "add",
        (BinOp::ISub, _) => "sub",
        (BinOp::IMul, _) => "mul",
        (BinOp::Div, true) => "div_s",
        (BinOp::Div, false) => "div_u",
        (BinOp::Rem, true) => "rem_s",
        (BinOp::Rem, false) => "rem_u",
        (BinOp::And, _) => "and",
        (BinOp::Or, _) => "or",
        (BinOp::Xor, _) => "xor",
        (BinOp::Shl, _) => "shl",
        (BinOp::Shr, true) => "shr_s",
        (BinOp::Shr, false) => "shr_u",
    };
    format!("{}.{}", pfx(r), base)
}

fn cmp_key(cond: Cond, r: Repr) -> String {
    let base = match (cond, r.signed()) {
        (Cond::Eq, _) => "eq",
        (Cond::Ne, _) => "ne",
        (Cond::Lt, true) => "lt_s",
        (Cond::Le, true) => "le_s",
        (Cond::Gt, true) => "gt_s",
        (Cond::Ge, true) => "ge_s",
        (Cond::Lt, false) => "lt_u",
        (Cond::Le, false) => "le_u",
        (Cond::Gt, false) => "gt_u",
        (Cond::Ge, false) => "ge_u",
    };
    format!("{}.{}", pfx(r), base)
}

fn compile_inst(e: &mut WEmit, inst: &Inst, block_pos: usize) -> Result<(), String> {
    match inst {
        Inst::IConst { dst, imm } => {
            let r = e.repr(*dst);
            e.konst(r, crate::opt::norm(r, *imm))?;
            e.set(*dst)
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let r = e.repr(*dst);
            let (n, c) = (r.bits(), r.container());
            let full = n == c;
            e.get(*lhs)?;
            // shifts by >= n are unspecified for narrow types: the container
            // shift, then re-normalize what can carry out
            if *op == BinOp::Div && r.signed() && full {
                // wasm traps on MIN / -1 where the IR (and the CPUs) wrap
                // to MIN: divide by rhs + 2m instead, m = (rhs == -1), and
                // conditionally negate the quotient, (q ^ -m) + m
                let p = pfx(r);
                let m = |e: &mut WEmit| -> Result<(), String> {
                    e.get(*rhs)?;
                    e.konst(r, -1)?;
                    e.op(&format!("{}.eq", p), None)?;
                    if c == 64 {
                        e.op("i64.extend_i32_u", None)?;
                    }
                    Ok(())
                };
                m(e)?;
                e.konst(r, 2)?;
                e.op(&format!("{}.mul", p), None)?;
                e.get(*rhs)?;
                e.op(&format!("{}.add", p), None)?;
                e.op(&binop_key(*op, r), None)?;
                e.konst(r, 0)?;
                m(e)?;
                e.op(&format!("{}.sub", p), None)?;
                e.op(&format!("{}.xor", p), None)?;
                m(e)?;
                e.op(&format!("{}.add", p), None)?;
                return e.set(*dst);
            }
            e.get(*rhs)?;
            e.op(&binop_key(*op, r), None)?;
            let carries = matches!(op, BinOp::IAdd | BinOp::ISub | BinOp::IMul | BinOp::Shl)
                || (*op == BinOp::Div && r.signed());
            if !full && carries {
                e.norm(r)?;
            }
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
            e.op(&cmp_key(*cond, e.repr(*lhs)), None)?;
            e.set(*dst)
        }
        Inst::Cast { dst, src, .. } => {
            let from = e.repr(*src);
            let to = e.repr(*dst);
            e.get(*src)?;
            e.cast(from, to)?;
            e.set(*dst)
        }
        Inst::Get { dst, src, field } => {
            let (off, fty) = e.func.field(e.func.ty(*src), *field).unwrap();
            let fr = e.func.repr(fty);
            let c = e.repr(*src).container();
            e.get(*src)?;
            e.extract(c, off, fr)?;
            e.set(*dst)
        }
        Inst::Set {
            dst,
            src,
            field,
            val,
        } => {
            let (off, fty) = e.func.field(e.func.ty(*src), *field).unwrap();
            let w = e.func.width(fty).unwrap();
            let r = e.repr(*src);
            let c = r.container();
            let mask = if w >= 64 { -1i64 } else { ((1u64 << w) - 1) as i64 } << off;
            e.get(*src)?;
            e.konst(r, !mask)?;
            e.op(&format!("{}.and", pfx(r)), None)?;
            e.get(*val)?;
            e.place(c, e.repr(*val), off, w)?;
            e.op(&format!("{}.or", pfx(r)), None)?;
            e.set(*dst)
        }
        Inst::Pack { dst, args } => {
            let ty = e.func.ty(*dst);
            let r = e.repr(*dst);
            let c = r.container();
            for (k, &a) in args.iter().enumerate() {
                let (off, fty) = e.func.field(ty, k as u32).unwrap();
                let w = e.func.width(fty).unwrap();
                e.get(a)?;
                e.place(c, e.repr(a), off, w)?;
                if k > 0 {
                    e.op(&format!("{}.or", pfx(r)), None)?;
                }
            }
            e.set(*dst)
        }
        Inst::Unpack { dsts, src } => {
            let ty = e.func.ty(*src);
            let c = e.repr(*src).container();
            for (k, &d) in dsts.iter().enumerate() {
                let (off, fty) = e.func.field(ty, k as u32).unwrap();
                let fr = e.func.repr(fty);
                e.get(*src)?;
                e.extract(c, off, fr)?;
                e.set(d)?;
            }
            Ok(())
        }
        Inst::Load { dst, addr } => {
            let r = e.repr(*dst);
            e.get(*addr)?;
            let t = match (r.bits(), r.signed()) {
                (8, true) => "i32.load8_s offset={}",
                (8, false) => "i32.load8_u offset={}",
                (16, true) => "i32.load16_s offset={}",
                (16, false) => "i32.load16_u offset={}",
                (32, _) => "i32.load offset={}",
                (64, _) => "i64.load offset={}",
                (n, _) => return Err(format!("no {}-bit memory access", n)),
            };
            e.op(t, Some(0))?;
            e.set(*dst)
        }
        Inst::Store { val, addr } => {
            let r = e.repr(*val);
            e.get(*addr)?;
            e.get(*val)?;
            let t = match r.bits() {
                8 => "i32.store8 offset={}",
                16 => "i32.store16 offset={}",
                32 => "i32.store offset={}",
                64 => "i64.store offset={}",
                n => return Err(format!("no {}-bit memory access", n)),
            };
            e.op(t, Some(0))
        }
        Inst::PtrAdd { dst, base, off } => {
            e.get(*base)?;
            e.get(*off)?;
            e.op("i32.wrap_i64", None)?;
            e.op("i32.add", None)?;
            e.set(*dst)
        }
        Inst::Call { dsts, callee, args } if e.natives.contains_key(callee) => {
            // the platform has this one: the instruction instead of the call
            let op = e.natives[callee];
            let Some(&dst) = dsts.first() else {
                return Ok(());
            };
            let l0 = e.local_of[args[0].0 as usize];
            let l1 = args.get(1).map(|a| e.local_of[a.0 as usize]).unwrap_or(0);
            let l2 = args.get(2).map(|a| e.local_of[a.0 as usize]).unwrap_or(0);
            for (key, v) in native_seq(op, l0, l1, l2) {
                e.op(key, v)?;
            }
            e.set(dst)
        }
        Inst::Call { dsts, callee, args } => {
            for &a in args {
                e.get(a)?;
            }
            let (idx, nrets) = *e
                .findex
                .get(callee)
                .ok_or_else(|| format!("call to undefined function {}", callee))?;
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
