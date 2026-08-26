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
//! dispatcher (the fallback for an irreducible graph; a reducible one,
//! which is all the parser and the passes produce, is emitted as nested
//! block/loop/if from its dominator tree, see structure.rs) — one loop
//! wrapping N nested blocks, a `label` local
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

use crate::platform::{Native, Natives, Operand, Platform};
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
    natives: &'a Natives,
    code: Vec<u8>,
    /// per value: the platform's local class (`f32`/`f64`), if any
    classes: Vec<Option<String>>,
    local_of: Vec<i64>, // ValueId -> wasm local index
    label_local: i64,
    nblocks: usize,
    /// structured emission: the enclosing labels, innermost last
    ctx: Vec<Label>,
}

/// a wasm label in scope, for `br`
#[derive(Clone, Copy, PartialEq, Debug)]
enum Label {
    /// a `block` that ends where SSA block n starts
    Block(usize),
    /// a `loop` whose start is SSA block n
    Loop(usize),
    /// an `if` arm (one label, never a branch target)
    If,
}

impl WEmit<'_> {
    fn op(&mut self, key: &str, v: Option<i64>) -> Result<(), String> {
        let mut buf = std::mem::take(&mut self.code);
        let r = self.enc.op(key, v, &mut buf);
        self.code = buf;
        r
    }

    fn is_f(&self, v: ValueId) -> bool {
        self.classes[v.0 as usize].is_some()
    }

    /// push the local as it is (a float stays a float)
    fn get_raw(&mut self, v: ValueId) -> Result<(), String> {
        let idx = self.local_of[v.0 as usize];
        self.op("local.get {}", Some(idx))
    }

    fn set_raw(&mut self, v: ValueId) -> Result<(), String> {
        let idx = self.local_of[v.0 as usize];
        self.op("local.set {}", Some(idx))
    }

    /// push v's bits: a float local reinterpreted to its integer form
    fn get(&mut self, v: ValueId) -> Result<(), String> {
        self.get_raw(v)?;
        if self.is_f(v) {
            let t = if self.repr(v).container() == 32 { "i32.reinterpret_f32" } else { "i64.reinterpret_f64" };
            self.op(t, None)?;
        }
        Ok(())
    }

    /// pop v's bits into its local (reinterpreted to a float if it is one)
    fn set(&mut self, v: ValueId) -> Result<(), String> {
        if self.is_f(v) {
            let t = if self.repr(v).container() == 32 { "f32.reinterpret_i32" } else { "f64.reinterpret_i64" };
            self.op(t, None)?;
        }
        self.set_raw(v)
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

    /// push the address base + index * step (+ a negative off), and give
    /// the memarg offset: a non-negative off rides in the instruction
    fn address(&mut self, base: ValueId, off: i64, index: Option<(ValueId, u8)>) -> Result<i64, String> {
        self.get(base)?;
        if let Some((i, step)) = index {
            self.get(i)?;
            self.op("i32.wrap_i64", None)?;
            if step > 1 {
                self.op("i32.const {}", Some(step.trailing_zeros() as i64))?;
                self.op("i32.shl", None)?;
            }
            self.op("i32.add", None)?;
        }
        if off < 0 {
            self.op("i32.const {}", Some(off as i32 as i64))?;
            self.op("i32.add", None)?;
            return Ok(0);
        }
        Ok(off)
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

    /// pass branch arguments to a block's parameters: the stack is the
    /// swap-safe staging area — push all, pop into the params in reverse
    fn pass_args(&mut self, target: crate::ssa::BlockId, args: &[ValueId]) -> Result<(), String> {
        for &a in args {
            self.get(a)?;
        }
        let params: Vec<ValueId> = self.func.blocks[target.0 as usize].params.clone();
        for &p in params.iter().rev() {
            self.set(p)?;
        }
        Ok(())
    }

    /// emit SSA block `x` and everything it dominates, nested (see
    /// structure.rs): a loop header inside a `loop`, its merge-node
    /// children each inside a `block` that ends where the child begins
    fn do_tree(&mut self, cfg: &crate::structure::Cfg, x: usize) -> Result<(), String> {
        if cfg.loop_header[x] {
            self.op("loop", None)?;
            self.ctx.push(Label::Loop(x));
            let merges: Vec<usize> = cfg.dom_children[x].iter().copied().filter(|&c| cfg.merge[c]).collect();
            self.node_within(cfg, x, &merges)?;
            self.ctx.pop();
            self.op("end", None)?;
            // a loop is left only by br to an enclosing label; after `end`
            // the stack machine still wants an unreachable path closed
            self.op("unreachable", None)
        } else {
            let merges: Vec<usize> = cfg.dom_children[x].iter().copied().filter(|&c| cfg.merge[c]).collect();
            self.node_within(cfg, x, &merges)
        }
    }

    /// `merges` in rpo order: the last (latest) is the outermost block
    fn node_within(&mut self, cfg: &crate::structure::Cfg, x: usize, merges: &[usize]) -> Result<(), String> {
        if let Some((&y, rest)) = merges.split_last() {
            self.op("block", None)?;
            self.ctx.push(Label::Block(y));
            self.node_within(cfg, x, rest)?;
            self.ctx.pop();
            self.op("end", None)?;
            return self.do_tree(cfg, y);
        }
        let block = &self.func.blocks[x];
        let (last, body) = block.insts.split_last().ok_or("empty block")?;
        for inst in body {
            compile_inst(self, inst, x)?;
        }
        match last {
            Inst::Jmp { target, args } => self.do_branch(cfg, *target, args),
            Inst::Br { cond, then_target, then_args, else_target, else_args } => {
                self.get(*cond)?;
                self.op("if", None)?;
                self.ctx.push(Label::If);
                self.do_branch(cfg, *then_target, then_args)?;
                self.op("else", None)?;
                self.do_branch(cfg, *else_target, else_args)?;
                self.ctx.pop();
                self.op("end", None)?;
                self.op("unreachable", None)
            }
            Inst::Ret { vals } => {
                for &v in vals {
                    self.get(v)?;
                }
                self.op("return", None)
            }
            other => compile_inst(self, other, x),
        }
    }

    /// a branch to `target`: a back edge or a merge node is a `br` to its
    /// label; anything else is emitted right here
    fn do_branch(&mut self, cfg: &crate::structure::Cfg, target: crate::ssa::BlockId, args: &[ValueId]) -> Result<(), String> {
        self.pass_args(target, args)?;
        let t = target.0 as usize;
        let want = if cfg.loop_header[t] && self.ctx.contains(&Label::Loop(t)) { Some(Label::Loop(t)) } else if cfg.merge[t] { Some(Label::Block(t)) } else { None };
        match want {
            Some(label) => {
                let depth = self.ctx.iter().rev().position(|l| *l == label).ok_or_else(|| format!("no label for block {} in scope", t))?;
                self.op("br {}", Some(depth as i64))
            }
            None => self.do_tree(cfg, t),
        }
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
    natives: &Natives,
) -> Result<Vec<u8>, String> {
    if let Some(native) = natives.get(&func.name) {
        // this function *is* a platform instruction: parameters arrive as
        // bits (the calling convention), float ones are reinterpreted on
        // the way in and the result on the way out
        let mut body = vec![0u8]; // no extra locals
        for (i, class) in native.arg_class.iter().enumerate() {
            enc.op("local.get {}", Some(i as i64), &mut body)?;
            if class.is_some() {
                enc.op(if native.arg_bits[i] <= 32 { "f32.reinterpret_i32" } else { "f64.reinterpret_i64" }, None, &mut body)?;
            }
        }
        for (key, v) in rule_seq(native)? {
            enc.op(&key, v, &mut body)?;
        }
        if native.ret_class.is_some() {
            enc.op(if native.ret_bits <= 32 { "i32.reinterpret_f32" } else { "i64.reinterpret_f64" }, None, &mut body)?;
        }
        body.push(enc.end);
        return Ok(body);
    }
    // locals: params first (that's the wasm rule), then the other SSA
    // values in id order, then the dispatcher's label local (i32). A
    // value of a float class gets a float local; a parameter of one gets
    // a second local, filled from its integer parameter at entry.
    let classes: Vec<Option<String>> = func.values.iter().map(|v| natives.class_of(func, v.ty).map(str::to_string)).collect();
    let n = func.values.len();
    let mut local_of = vec![-1i64; n];
    for (i, &p) in func.params.iter().enumerate() {
        if classes[p.0 as usize].is_none() {
            local_of[p.0 as usize] = i as i64;
        }
    }
    let mut next = func.params.len() as i64;
    let mut extra_types = Vec::new(); // declared locals, in order
    for id in 0..n {
        if local_of[id] < 0 {
            local_of[id] = next;
            next += 1;
            let r = wrepr(func, func.ty(ValueId(id as u32)));
            extra_types.push(match &classes[id] {
                Some(_) if r.container() == 32 => 0x7D,
                Some(_) => 0x7C,
                None => valtype(r),
            });
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
        classes,
        local_of,
        label_local,
        nblocks: func.blocks.len(),
        ctx: Vec::new(),
    };
    for (i, &p) in func.params.iter().enumerate() {
        if e.is_f(p) {
            e.op("local.get {}", Some(i as i64))?;
            e.set(p)?;
        }
    }

    match crate::structure::Cfg::analyze(func) {
        Some(cfg) => {
            // the graph as nesting: loops, blocks, ifs, and br
            e.do_tree(&cfg, 0)?;
            e.op("unreachable", None)?;
        }
        None => {
            // irreducible: a dispatcher loop { block^N { chain } code_0 } ... }
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
        }
    }

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

/// the ops for a platform rule, the arguments already on the stack in
/// order and the result left there: a line is an op key, with a
/// literal as its one immediate at most
fn rule_seq(native: &Native) -> Result<Vec<(String, Option<i64>)>, String> {
    let mut seq = Vec::new();
    for line in &native.rule.lines {
        let key = line.template.clone().unwrap_or_else(|| line.mnemonic.clone());
        match line.operands.as_slice() {
            [] => seq.push((key, None)),
            [Operand::Lit(l)] => {
                let v = l.parse::<i64>().map_err(|_| format!("rule '{}': bad literal '{}'", native.sig, l))?;
                seq.push((format!("{} {{}}", line.mnemonic), Some(v)))
            }
            _ => return Err(format!("rule '{}': a wasm rule line is one op, with a literal at most", native.sig)),
        }
    }
    Ok(seq)
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
            e.konst(r, crate::opt::norm(r, *imm as i64))?;
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
        Inst::Load { dst, addr, off, index } => {
            let r = e.repr(*dst);
            let imm = e.address(*addr, *off, *index)?;
            let t = match (r.bits(), r.signed()) {
                (8, true) => "i32.load8_s offset={}",
                (8, false) => "i32.load8_u offset={}",
                (16, true) => "i32.load16_s offset={}",
                (16, false) => "i32.load16_u offset={}",
                (32, _) => "i32.load offset={}",
                (64, _) => "i64.load offset={}",
                (n, _) => return Err(format!("no {}-bit memory access", n)),
            };
            e.op(t, Some(imm))?;
            e.set(*dst)
        }
        Inst::Store { val, addr, off, index } => {
            let r = e.repr(*val);
            let imm = e.address(*addr, *off, *index)?;
            e.get(*val)?;
            let t = match r.bits() {
                8 => "i32.store8 offset={}",
                16 => "i32.store16 offset={}",
                32 => "i32.store offset={}",
                64 => "i64.store offset={}",
                n => return Err(format!("no {}-bit memory access", n)),
            };
            e.op(t, Some(imm))
        }
        Inst::PtrAdd { dst, base, off } => {
            e.get(*base)?;
            e.get(*off)?;
            e.op("i32.wrap_i64", None)?;
            e.op("i32.add", None)?;
            e.set(*dst)
        }
        Inst::Call { dsts, callee, args } if e.natives.get(callee).is_some() => {
            // the platform has this one: the rule's ops instead of the
            // call, floats staying floats on the stack
            let natives: &Natives = e.natives;
            let native = natives.get(callee).unwrap();
            let Some(&dst) = dsts.first() else {
                return Ok(());
            };
            for &a in args {
                if e.is_f(a) { e.get_raw(a)? } else { e.get(a)? }
            }
            for (key, v) in rule_seq(native)? {
                e.op(&key, v)?;
            }
            if e.is_f(dst) { e.set_raw(dst) } else { e.set(dst) }
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
