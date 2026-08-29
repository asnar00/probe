//! Structs: `type point = struct { x: f32, y: f32, z: f32 }` is fields
//! side by side — in memory at their natural offsets, in registers as
//! separate values — and never a bit pattern: no `cast` to or from one,
//! no literal, no arithmetic dispatch. That is what tells it from a
//! `pack`, and what lets the compiler own its layout (an array of them
//! may be laid out field-major one day; a program cannot tell, since it
//! never takes a field's address).
//!
//! Right after parsing, every struct value is dissolved into its fields
//! (scalar replacement): parameters, block parameters and results expand
//! to field lists; `pack`, `unpack`, `get` and `set` become names for
//! values that already exist; a `load` or `store` of a struct becomes one
//! per field at its byte offset. Nested structs flatten; a wide field is
//! then lowered by wide.rs like any other.

use crate::ssa::{CastOp, Function, Inst, Module, Type, ValueData, ValueId, Vectors};
use std::collections::HashMap;

pub fn has_structs(m: &Module) -> bool {
    m.funcs.iter().any(|f| f.values.iter().any(|v| v.ty.is_struct()) || f.rets.iter().any(|t| t.is_struct()))
}

/// `keep_vectors`: a vector (a struct of lanes) stays a value where the
/// platform takes it whole
pub fn lower(m: &mut Module, keep_vectors: Vectors) -> Result<(), String> {
    for f in &mut m.funcs {
        if f.values.iter().any(|v| v.ty.is_struct()) || f.rets.iter().any(|t| t.is_struct()) {
            lower_function(f, keep_vectors).map_err(|e| format!("{}: {}", f.name, e))?;
        }
    }
    Ok(())
}

/// does a type come apart here? a struct, unless it is a vector being kept
fn dissolves(f: &Function, ty: Type, keep_vectors: Vectors) -> bool {
    ty.is_struct() && !(f.vector(ty).is_some() && keep_vectors.keeps(&f.tyname(ty)))
}

/// the scalar leaves of a type: (type, byte offset), nested structs
/// flattened
fn leaves(f: &Function, ty: Type, base: u32, out: &mut Vec<(Type, u32)>, keep_vectors: Vectors) {
    match ty {
        Type::Struct(_) if dissolves(f, ty, keep_vectors) => {
            let p = f.pack(ty).unwrap().clone();
            for (k, (_, fty)) in p.fields.iter().enumerate() {
                leaves(f, *fty, base + p.offsets[k], out, keep_vectors);
            }
        }
        t => out.push((t, base)),
    }
}

struct Lower<'a> {
    f: &'a mut Function,
    /// struct value -> its leaf values (leaf 0 is the value itself)
    rows: HashMap<u32, Vec<ValueId>>,
    /// a value that is another value under a new name
    alias: HashMap<u32, ValueId>,
    /// the struct types of the values that were retyped to their first leaf
    orig: HashMap<u32, Type>,
    out: Vec<Inst>,
    keep: Vectors,
}

fn lower_function(f: &mut Function, keep_vectors: Vectors) -> Result<(), String> {
    // the signature as written, for callers from outside (the harness)
    f.wide_sig = Some((f.params.iter().map(|&p| f.ty(p)).collect(), f.rets.clone()));
    let mut rets = Vec::new();
    for &t in &f.rets {
        let mut ls = Vec::new();
        leaves(f, t, 0, &mut ls, keep_vectors);
        rets.extend(ls.into_iter().map(|(t, _)| t));
    }
    f.rets = rets;
    let mut lo = Lower { f, rows: HashMap::new(), alias: HashMap::new(), orig: HashMap::new(), out: Vec::new(), keep: keep_vectors };
    let n = lo.f.values.len();
    for i in 0..n {
        let id = ValueId(i as u32);
        let ty = lo.f.ty(id);
        if !dissolves(lo.f, ty, keep_vectors) {
            continue;
        }
        let mut ls = Vec::new();
        leaves(lo.f, ty, 0, &mut ls, keep_vectors);
        let name = lo.f.values[i].name.clone();
        let mut row = vec![id];
        for (k, (lt, _)) in ls.iter().enumerate().skip(1) {
            lo.f.values.push(ValueData { name: format!("{}.{}", name, k), ty: *lt, literal: None });
            row.push(ValueId(lo.f.values.len() as u32 - 1));
        }
        lo.f.values[i].ty = ls[0].0;
        lo.f.values[i].literal = None;
        lo.orig.insert(id.0, ty);
        lo.rows.insert(id.0, row);
    }
    let params = std::mem::take(&mut lo.f.params);
    lo.f.params = lo.expand_list(&params);
    let nblocks = lo.f.blocks.len();
    for b in 0..nblocks {
        let ps = std::mem::take(&mut lo.f.blocks[b].params);
        lo.f.blocks[b].params = lo.expand_list(&ps);
    }
    // pass 1: what pack/unpack/get/set name
    for b in 0..nblocks {
        let insts = lo.f.blocks[b].insts.clone();
        for inst in &insts {
            lo.name(inst)?;
        }
    }
    // pass 2: rewrite everything else through the names
    for b in 0..nblocks {
        let insts = std::mem::take(&mut lo.f.blocks[b].insts);
        for inst in insts {
            lo.inst(inst)?;
        }
        lo.f.blocks[b].insts = std::mem::take(&mut lo.out);
    }
    Ok(())
}

impl Lower<'_> {
    /// a u8 value named after a u1 lane, for its byte in memory
    fn byte_temp(&mut self, lane: ValueId) -> ValueId {
        let name = format!("{}_byte", self.f.value(lane).name);
        self.f.values.push(ValueData { name, ty: Type::Int { signed: false, bits: 8 }, literal: None });
        ValueId(self.f.values.len() as u32 - 1)
    }

    fn is_struct(&self, v: ValueId) -> bool {
        self.rows.contains_key(&v.0)
    }

    fn resolve(&self, mut v: ValueId) -> ValueId {
        let mut guard = 0;
        while let Some(&a) = self.alias.get(&v.0) {
            v = a;
            guard += 1;
            if guard > 100_000 {
                break;
            }
        }
        v
    }

    /// the leaves of a value, resolved: a struct's row, or the value
    fn row(&self, v: ValueId) -> Vec<ValueId> {
        match self.rows.get(&v.0) {
            Some(r) => r.iter().map(|&x| self.resolve(x)).collect(),
            None => vec![self.resolve(v)],
        }
    }

    fn expand_list(&self, vs: &[ValueId]) -> Vec<ValueId> {
        vs.iter().flat_map(|&v| self.row(v)).collect()
    }

    /// the leaf range of field `field` within a struct value's row
    fn field_range(&self, v: ValueId, field: u32) -> (usize, usize) {
        let ty = self.struct_ty(v);
        let p = self.f.pack(ty).unwrap();
        let mut start = 0;
        for (k, (_, fty)) in p.fields.iter().enumerate() {
            let mut ls = Vec::new();
            leaves(self.f, *fty, 0, &mut ls, self.keep);
            if k as u32 == field {
                return (start, start + ls.len());
            }
            start += ls.len();
        }
        unreachable!()
    }

    /// the struct type a row was made for (its own type was retyped)
    fn struct_ty(&self, v: ValueId) -> Type {
        self.orig.get(&v.0).copied().unwrap()
    }

    fn name(&mut self, inst: &Inst) -> Result<(), String> {
        match inst {
            Inst::Pack { dst, args } if self.is_struct(*dst) => {
                let row = self.rows[&dst.0].clone();
                let mut k = 0;
                for &a in args {
                    for leaf in self.row(a) {
                        self.alias.insert(row[k].0, leaf);
                        k += 1;
                    }
                }
            }
            Inst::Unpack { dsts, src } if self.is_struct(*src) => {
                let srow = self.row(*src);
                let mut k = 0;
                for &d in dsts {
                    let drow: Vec<ValueId> = self.rows.get(&d.0).cloned().unwrap_or_else(|| vec![d]);
                    for dl in drow {
                        self.alias.insert(dl.0, srow[k]);
                        k += 1;
                    }
                }
            }
            Inst::Get { dst, src, field } if self.is_struct(*src) => {
                let srow = self.row(*src);
                let (s, e) = self.field_range(*src, *field);
                let drow: Vec<ValueId> = self.rows.get(&dst.0).cloned().unwrap_or_else(|| vec![*dst]);
                for (dl, sl) in drow.iter().zip(&srow[s..e]) {
                    self.alias.insert(dl.0, *sl);
                }
            }
            Inst::Set { dst, src, field, val } if self.is_struct(*dst) => {
                let srow = self.row(*src);
                let vrow = self.row(*val);
                let (s, e) = self.field_range(*src, *field);
                let drow = self.rows[&dst.0].clone();
                for k in 0..drow.len() {
                    let from = if k >= s && k < e { vrow[k - s] } else { srow[k] };
                    self.alias.insert(drow[k].0, from);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn inst(&mut self, inst: Inst) -> Result<(), String> {
        let r = |lo: &Self, v: ValueId| lo.resolve(v);
        match inst {
            Inst::Pack { dst, .. } | Inst::Set { dst, .. } if self.is_struct(dst) => {}
            Inst::Unpack { src, .. } | Inst::Get { src, .. } if self.is_struct(src) => {}
            Inst::Load { dst, addr, off, index } if self.is_struct(dst) => {
                let mut ls = Vec::new();
                leaves(self.f, self.struct_ty(dst), 0, &mut ls, self.keep);
                let row = self.rows[&dst.0].clone();
                let addr = r(self, addr);
                let index = index.map(|(i, s)| (r(self, i), s));
                for (leaf, (lty, foff)) in row.iter().zip(&ls) {
                    // a u1 lane is a byte in memory (as the vector
                    // registers and the GPU have it): loaded as one, converted
                    if matches!(lty, Type::Int { bits: 1, .. }) {
                        let byte = self.byte_temp(*leaf);
                        self.out.push(Inst::Load { dst: byte, addr, off: off + *foff as i64, index });
                        self.out.push(Inst::Cast { op: CastOp::Conv, dst: *leaf, src: byte });
                    } else {
                        self.out.push(Inst::Load { dst: *leaf, addr, off: off + *foff as i64, index });
                    }
                }
            }
            Inst::Store { val, addr, off, index } if self.is_struct(val) => {
                let mut ls = Vec::new();
                leaves(self.f, self.struct_ty(val), 0, &mut ls, self.keep);
                let row = self.row(val);
                let addr = r(self, addr);
                let index = index.map(|(i, s)| (r(self, i), s));
                for (leaf, (lty, foff)) in row.iter().zip(&ls) {
                    if matches!(lty, Type::Int { bits: 1, .. }) {
                        let byte = self.byte_temp(*leaf);
                        self.out.push(Inst::Cast { op: CastOp::Conv, dst: byte, src: *leaf });
                        self.out.push(Inst::Store { val: byte, addr, off: off + *foff as i64, index });
                    } else {
                        self.out.push(Inst::Store { val: *leaf, addr, off: off + *foff as i64, index });
                    }
                }
            }
            Inst::Call { dsts, callee, args } => {
                let dsts = self.expand_list(&dsts);
                let args = self.expand_list(&args);
                self.out.push(Inst::Call { dsts, callee, args });
            }
            Inst::CallInd { dsts, callee, args } => {
                let dsts = self.expand_list(&dsts);
                let args = self.expand_list(&args);
                self.out.push(Inst::CallInd { dsts, callee: r(self, callee), args });
            }
            Inst::FnAddr { dst, name } => self.out.push(Inst::FnAddr { dst, name }),
            Inst::Scratch { dst, bytes } => self.out.push(Inst::Scratch { dst, bytes }),
            Inst::Check { cond } => self.out.push(Inst::Check { cond: r(self, cond) }),
            Inst::Jmp { target, args } => {
                let args = self.expand_list(&args);
                self.out.push(Inst::Jmp { target, args });
            }
            Inst::Br { cond, then_target, then_args, else_target, else_args } => {
                let cond = r(self, cond);
                let then_args = self.expand_list(&then_args);
                let else_args = self.expand_list(&else_args);
                self.out.push(Inst::Br { cond, then_target, then_args, else_target, else_args });
            }
            Inst::Ret { vals } => {
                let vals = self.expand_list(&vals);
                self.out.push(Inst::Ret { vals });
            }
            Inst::IConst { dst, imm } => self.out.push(Inst::IConst { dst, imm }),
            Inst::Bin { op, dst, lhs, rhs } => self.out.push(Inst::Bin { op, dst, lhs: r(self, lhs), rhs: r(self, rhs) }),
            Inst::ICmp { cond, dst, lhs, rhs } => self.out.push(Inst::ICmp { cond, dst, lhs: r(self, lhs), rhs: r(self, rhs) }),
            Inst::Cast { op, dst, src } => self.out.push(Inst::Cast { op, dst, src: r(self, src) }),
            Inst::Pack { dst, args } => {
                let args = args.iter().map(|&a| r(self, a)).collect();
                self.out.push(Inst::Pack { dst, args });
            }
            Inst::Unpack { dsts, src } => self.out.push(Inst::Unpack { dsts, src: r(self, src) }),
            Inst::Get { dst, src, field } => self.out.push(Inst::Get { dst, src: r(self, src), field }),
            Inst::Set { dst, src, field, val } => self.out.push(Inst::Set { dst, src: r(self, src), field, val: r(self, val) }),
            Inst::Load { dst, addr, off, index } => {
                let index = index.map(|(i, s)| (r(self, i), s));
                self.out.push(Inst::Load { dst, addr: r(self, addr), off, index });
            }
            Inst::Store { val, addr, off, index } => {
                let index = index.map(|(i, s)| (r(self, i), s));
                self.out.push(Inst::Store { val: r(self, val), addr: r(self, addr), off, index });
            }
            Inst::PtrAdd { dst, base, off } => self.out.push(Inst::PtrAdd { dst, base: r(self, base), off: r(self, off) }),
            Inst::Addr { dst, name } => self.out.push(Inst::Addr { dst, name }),
            Inst::Platform { dst, name } => self.out.push(Inst::Platform { dst, name }),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::ssa::parse;

    /// a struct is never a bit pattern: no cast, no literal, no arithmetic
    #[test]
    fn structs_are_not_bit_patterns() {
        let ok = "type p = struct { x: i32, y: i32 }\nfn f(a: p) -> i32 {\n    v: i32 = get a, y\n    ret v\n}\n";
        let m = parse(ok).unwrap();
        // dissolved: two i32 parameters, a plain return
        assert_eq!(m.funcs[0].params.len(), 2);
        assert!(parse("type p = struct { x: i32, y: i32 }\nfn f(a: p) -> u64 {\n    v: u64 = cast a\n    ret v\n}\n").is_err());
        assert!(parse("type p = struct { x: i32, y: i32 }\nfn f() -> i32 {\n    a: p = const 5\n    v: i32 = get a, x\n    ret v\n}\n").is_err());
        assert!(parse("type p = struct { x: i32, y: i32 }\nfn f(a: p, b: p) -> p {\n    c: p = add a, b\n    ret c\n}\n").is_err());
        // layout: natural alignment, size a multiple of the largest
        let m = parse("type m = struct { t: u8, n: i64, w: u128 }\nfn f(p: ptr) -> i64 {\n    s: m = load p\n    n: i64 = get s, n\n    ret n\n}\n").unwrap();
        let def = m.funcs[0].packs.iter().find(|p| p.aggregate).unwrap();
        assert_eq!(def.offsets, vec![0, 8, 16]);
        assert_eq!(def.size, 32);
    }
}
