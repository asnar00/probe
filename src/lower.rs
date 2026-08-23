//! Mandatory lowering passes that erase the richer type-system features
//! before emission, so backends only ever see the core types
//! (width-1 bools, i32/u32, i64/u64, ptr, f32, f64).
//!
//! `lower_widths`: arbitrary-width integers become 64-bit container
//! operations. The canonical representation follows the type: `uN` values
//! are zero-extended, `iN` values sign-extended. That choice makes
//! division, remainder, comparisons, and right shifts *direct* on the
//! container (the container op's own type-driven behavior is correct);
//! add/sub/mul/shl re-canonicalize their results, and shift amounts
//! reduce mod N. Width-1 values are the exception: 0/1 for both signs
//! (`ext` interprets). Because types live on variables, the final step is
//! retyping the value tables.
//!
//! `lower_structs`: packed bitfield structs become their carrier
//! (unsigned) integer; extract is shift+trunc, pack/insert mask each
//! field and or it into place. Whole-width identities collapse to value
//! substitutions.

use crate::ssa::{BinOp, CastOp, Function, Inst, Module, Type, ValueId};
use std::collections::HashMap;

pub fn lower(module: &mut Module) {
    lower_vectors(module);
    lower_structs(module);
    lower_widths(module);
}

/// The native (arm64) lowering: vector-preserving. Functions whose vector
/// ops all have NEON mappings keep their vectors whole for the emitter;
/// the rest scalarize their BODIES but keep vector types in their
/// signatures, so the calling convention is uniform module-wide (vector
/// params, arguments, and returns always travel in d registers — a
/// scalarized function pays one fmov per boundary crossing, a kept
/// function pays nothing).
pub fn lower_native(module: &mut Module) {
    lower_vectors_native(module);
    lower_structs(module);
    lower_widths(module);
}

/// Every vector op the NEON emitter can realize directly: elementwise
/// arithmetic minus div/rem (NEON has no integer divide), 8/16/32-bit
/// lanes, and vector<->scalar bitcasts at 32/64 bits total.
fn neon_ok(func: &Function, inst: &crate::ssa::Inst) -> bool {
    use crate::ssa::{BinOp, CastOp, Inst, VecElem};
    let lane_ok = |e: crate::ssa::VecElem| matches!(e.bits(), 8 | 16 | 32);
    let vec_elem = |t: Type| match t {
        Type::Vec(_, e) => Some(e),
        _ => None,
    };
    match inst {
        Inst::Bin { op, dst, .. } => match vec_elem(func.ty(*dst)) {
            None => true,
            Some(e) => {
                lane_ok(e)
                    && match op {
                        BinOp::Div | BinOp::Rem => false,
                        BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv => {
                            e == VecElem::F32
                        }
                        _ => true,
                    }
            }
        },
        Inst::Extract { src, .. } | Inst::Insert { src, .. } => {
            vec_elem(func.ty(*src)).map_or(true, lane_ok)
        }
        Inst::Pack { dst, .. } => vec_elem(func.ty(*dst)).map_or(true, lane_ok),
        Inst::Cast {
            op: CastOp::Bitcast,
            dst,
            src,
        } => {
            let total = |t: Type| match t {
                Type::Vec(n, e) => Some(n as u32 * e.bits()),
                _ => None,
            };
            match total(func.ty(*src)).or(total(func.ty(*dst))) {
                Some(bits) => matches!(bits, 32 | 64),
                None => true,
            }
        }
        _ => true,
    }
}

fn lower_vectors_native(module: &mut Module) {
    use crate::ssa::{CastOp, Inst};
    let has_vec = |f: &Function| f.values.iter().any(|v| matches!(v.ty, Type::Vec(..)));
    let keep: Vec<bool> = module
        .funcs
        .iter()
        .map(|f| {
            !has_vec(f)
                || f.blocks
                    .iter()
                    .flat_map(|b| &b.insts)
                    .all(|i| neon_ok(f, i))
        })
        .collect();
    if keep.iter().all(|&k| k) {
        return; // everything emits directly; vectors survive to the emitter
    }
    // struct table for the scalarized functions
    let sid = register_vec_structs(module);
    for (fi, func) in module.funcs.iter_mut().enumerate() {
        if keep[fi] || !has_vec(func) {
            continue;
        }
        // remember which values must keep their vector type: params, and
        // fresh boundary temps introduced below
        let vec_params: Vec<crate::ssa::ValueId> = func
            .params
            .iter()
            .copied()
            .filter(|&p| matches!(func.ty(p), Type::Vec(..)))
            .collect();
        let mut boundary: Vec<crate::ssa::ValueId> = vec_params.clone();
        let mut ntmp = 0u32;
        // vector call/ret boundaries: calls pass and return vectors (the
        // uniform ABI), rets return them — bridge with bitcasts
        for b in 0..func.blocks.len() {
            let insts = std::mem::take(&mut func.blocks[b].insts);
            let mut out = Vec::with_capacity(insts.len());
            for inst in insts {
                match inst {
                    Inst::Call { dsts, callee, mut args } => {
                        for a in args.iter_mut() {
                            if let Type::Vec(..) = func.ty(*a) {
                                ntmp += 1;
                                func.values.push(crate::ssa::ValueData {
                                    name: format!("vb{}", ntmp),
                                    ty: func.ty(*a),
                                });
                                let t = crate::ssa::ValueId(func.values.len() as u32 - 1);
                                boundary.push(t);
                                out.push(Inst::Cast {
                                    op: CastOp::Bitcast,
                                    dst: t,
                                    src: *a,
                                });
                                *a = t;
                            }
                        }
                        let mut post = Vec::new();
                        let dsts = dsts
                            .into_iter()
                            .map(|d| {
                                if let Type::Vec(..) = func.ty(d) {
                                    ntmp += 1;
                                    func.values.push(crate::ssa::ValueData {
                                        name: format!("vb{}", ntmp),
                                        ty: func.ty(d),
                                    });
                                    let t = crate::ssa::ValueId(func.values.len() as u32 - 1);
                                    boundary.push(t);
                                    post.push(Inst::Cast {
                                        op: CastOp::Bitcast,
                                        dst: d,
                                        src: t,
                                    });
                                    t
                                } else {
                                    d
                                }
                            })
                            .collect();
                        out.push(Inst::Call { dsts, callee, args });
                        out.extend(post);
                    }
                    Inst::Ret { mut vals } => {
                        for v in vals.iter_mut() {
                            if let Type::Vec(..) = func.ty(*v) {
                                ntmp += 1;
                                func.values.push(crate::ssa::ValueData {
                                    name: format!("vb{}", ntmp),
                                    ty: func.ty(*v),
                                });
                                let t = crate::ssa::ValueId(func.values.len() as u32 - 1);
                                boundary.push(t);
                                out.push(Inst::Cast {
                                    op: CastOp::Bitcast,
                                    dst: t,
                                    src: *v,
                                });
                                *v = t;
                            }
                        }
                        out.push(Inst::Ret { vals });
                    }
                    other => out.push(other),
                }
            }
            func.blocks[b].insts = out;
        }
        // vector params: bridge into the scalarized body through a bitcast
        // (substitute first, then prepend, so the bridge isn't rewritten)
        let mut entry_casts = Vec::new();
        let mut subst = HashMap::new();
        for &p in &vec_params {
            ntmp += 1;
            func.values.push(crate::ssa::ValueData {
                name: format!("vb{}", ntmp),
                ty: func.ty(p),
            });
            let c = crate::ssa::ValueId(func.values.len() as u32 - 1);
            subst.insert(p, c);
            entry_casts.push(Inst::Cast {
                op: CastOp::Bitcast,
                dst: c,
                src: p,
            });
        }
        if !subst.is_empty() {
            substitute(func, &subst);
            // the bridge value scalarizes with the body; the param stays
            for (&p, &c) in &subst {
                let _ = (p, c);
            }
            let mut pre = entry_casts;
            pre.append(&mut func.blocks[0].insts);
            func.blocks[0].insts = pre;
        }
        // scalarize the body; boundary values and params keep Vec
        lower_vectors_fn(func, &sid, &boundary_set(&boundary, &vec_params));
    }
}

fn boundary_set(
    boundary: &[crate::ssa::ValueId],
    _params: &[crate::ssa::ValueId],
) -> std::collections::HashSet<crate::ssa::ValueId> {
    boundary.iter().copied().collect()
}

/// register the generated per-vector-type structs in the module table
fn register_vec_structs(
    module: &mut Module,
) -> std::collections::HashMap<(u8, crate::ssa::VecElem), u16> {
    use crate::ssa::VecElem;
    let mut vecs: Vec<(u8, VecElem)> = Vec::new();
    for f in &module.funcs {
        for t in f.values.iter().map(|v| v.ty).chain(f.rets.iter().copied()) {
            if let Type::Vec(n, e) = t {
                if !vecs.contains(&(n, e)) {
                    vecs.push((n, e));
                }
            }
        }
    }
    let mut defs = (*module.structs).clone();
    let mut sid = std::collections::HashMap::new();
    for &(n, e) in &vecs {
        let field_ty = match e {
            VecElem::F32 => Type::U(32), // struct fields are integers
            e => e.ty(),
        };
        let name = format!("__v{}{}", n, e.ty().name());
        if let Some(i) = defs.iter().position(|d| d.name == name) {
            sid.insert((n, e), i as u16);
            continue;
        }
        let fields = (0..n).map(|i| (format!("l{}", i), field_ty)).collect();
        sid.insert((n, e), defs.len() as u16);
        defs.push(crate::ssa::StructDef { name, fields });
    }
    let rc = std::rc::Rc::new(defs);
    module.structs = rc.clone();
    for f in &mut module.funcs {
        f.structs = rc.clone();
    }
    sid
}

/// Scalarize vectors into packed structs: each vector type becomes a
/// generated struct (fields low-first, so lane i IS field i), elementwise
/// ops become per-lane extract/op/pack. Float lanes travel as their bit patterns
/// in the struct, bitcast at the lane boundary. After this pass nothing
/// downstream knows vectors existed — struct lowering turns the rest into
/// carrier-integer code. Idempotent, and also called from soften() so the
/// softfloat rewrite sees scalar float ops, never vector ones.
pub fn lower_vectors(module: &mut Module) {
    use crate::ssa::VecElem;
    // collect the vector types in use and generate their structs
    let mut vecs: Vec<(u8, VecElem)> = Vec::new();
    for f in &module.funcs {
        for t in f.values.iter().map(|v| v.ty).chain(f.rets.iter().copied()) {
            if let Type::Vec(n, e) = t {
                if !vecs.contains(&(n, e)) {
                    vecs.push((n, e));
                }
            }
        }
    }
    if vecs.is_empty() {
        return;
    }
    let sid = register_vec_structs(module);
    let none = std::collections::HashSet::new();
    for f in &mut module.funcs {
        lower_vectors_fn(f, &sid, &none);
    }
}

fn lower_vectors_fn(
    func: &mut Function,
    sid: &std::collections::HashMap<(u8, crate::ssa::VecElem), u16>,
    keep_vec: &std::collections::HashSet<crate::ssa::ValueId>,
) {
    use crate::ssa::{CastOp, Inst, VecElem};
    let mut ntmp = 0u32;
    let mut tmp = |func: &mut Function, ty: Type| {
        ntmp += 1;
        func.values.push(crate::ssa::ValueData {
            name: format!("vl{}", ntmp),
            ty,
        });
        crate::ssa::ValueId(func.values.len() as u32 - 1)
    };
    for b in 0..func.blocks.len() {
        let insts = std::mem::take(&mut func.blocks[b].insts);
        let mut out = Vec::with_capacity(insts.len());
        for inst in insts {
            match inst {
                Inst::Bin { op, dst, lhs, rhs } if matches!(func.ty(dst), Type::Vec(..)) => {
                    let Type::Vec(lanes, elem) = func.ty(dst) else { unreachable!() };
                    let fty = if elem == VecElem::F32 { Type::U(32) } else { elem.ty() };
                    let mut lane_vals = Vec::with_capacity(lanes as usize);
                    for lane in 0..lanes {
                        let field = lane as u16;
                        let e1 = tmp(func, fty);
                        let e2 = tmp(func, fty);
                        out.push(Inst::Extract { dst: e1, src: lhs, field });
                        out.push(Inst::Extract { dst: e2, src: rhs, field });
                        if elem == VecElem::F32 {
                            let (b1, b2) = (tmp(func, Type::F32), tmp(func, Type::F32));
                            out.push(Inst::Cast { op: CastOp::Bitcast, dst: b1, src: e1 });
                            out.push(Inst::Cast { op: CastOp::Bitcast, dst: b2, src: e2 });
                            let r = tmp(func, Type::F32);
                            out.push(Inst::Bin { op, dst: r, lhs: b1, rhs: b2 });
                            let rb = tmp(func, Type::U(32));
                            out.push(Inst::Cast { op: CastOp::Bitcast, dst: rb, src: r });
                            lane_vals.push(rb);
                        } else {
                            let r = tmp(func, fty);
                            out.push(Inst::Bin { op, dst: r, lhs: e1, rhs: e2 });
                            lane_vals.push(r);
                        }
                    }
                    out.push(Inst::Pack { dst, args: lane_vals });
                }
                Inst::Extract { dst, src, field } if matches!(func.ty(src), Type::Vec(..)) => {
                    let Type::Vec(_, elem) = func.ty(src) else { unreachable!() };
                    if elem == VecElem::F32 {
                        let t = tmp(func, Type::U(32));
                        out.push(Inst::Extract { dst: t, src, field });
                        out.push(Inst::Cast { op: CastOp::Bitcast, dst, src: t });
                    } else {
                        out.push(Inst::Extract { dst, src, field });
                    }
                }
                Inst::Insert { dst, src, field, val }
                    if matches!(func.ty(src), Type::Vec(..)) =>
                {
                    let Type::Vec(_, elem) = func.ty(src) else { unreachable!() };
                    if elem == VecElem::F32 {
                        let t = tmp(func, Type::U(32));
                        out.push(Inst::Cast { op: CastOp::Bitcast, dst: t, src: val });
                        out.push(Inst::Insert { dst, src, field, val: t });
                    } else {
                        out.push(Inst::Insert { dst, src, field, val });
                    }
                }
                Inst::Pack { dst, mut args } if matches!(func.ty(dst), Type::Vec(..)) => {
                    let Type::Vec(_, elem) = func.ty(dst) else { unreachable!() };
                    if elem == VecElem::F32 {
                        args = args
                            .into_iter()
                            .map(|a| {
                                let t = tmp(func, Type::U(32));
                                out.push(Inst::Cast { op: CastOp::Bitcast, dst: t, src: a });
                                t
                            })
                            .collect();
                    }
                    out.push(Inst::Pack { dst, args });
                }
                other => out.push(other),
            }
        }
        func.blocks[b].insts = out;
    }
    // retype vector values to their generated structs; everything else
    // (params, branch args, calls, bitcasts) follows automatically. Values
    // in keep_vec are ABI-boundary values (params, call args/results, ret
    // operands) that stay vectors — and so do the return types with them.
    for (i, v) in func.values.iter_mut().enumerate() {
        if keep_vec.contains(&crate::ssa::ValueId(i as u32)) {
            continue;
        }
        if let Type::Vec(n, e) = v.ty {
            v.ty = Type::Struct(sid[&(n, e)]);
        }
    }
    if keep_vec.is_empty() {
        for r in &mut func.rets {
            if let Type::Vec(n, e) = *r {
                *r = Type::Struct(sid[&(n, e)]);
            }
        }
    }
}

fn odd(t: Type) -> Option<(u8, bool)> {
    match t {
        Type::I(n) if n != 1 && n != 32 && n != 64 => Some((n, true)),
        Type::U(n) if n != 1 && n != 32 && n != 64 => Some((n, false)),
        _ => None,
    }
}

fn container(t: Type) -> Type {
    match t {
        Type::I(_) => Type::I(64),
        Type::U(_) => Type::U(64),
        t => t,
    }
}

struct Lw<'a> {
    func: &'a mut Function,
    out: Vec<Inst>,
}

impl Lw<'_> {
    fn tmp(&mut self, ty: Type) -> ValueId {
        let id = ValueId(self.func.values.len() as u32);
        self.func.values.push(crate::ssa::ValueData {
            name: format!("__lw{}", id.0),
            ty,
        });
        id
    }

    fn iconst(&mut self, ty: Type, v: i64) -> ValueId {
        let d = self.tmp(ty);
        self.out.push(Inst::IConst { dst: d, imm: v });
        d
    }

    fn bin(&mut self, ty: Type, op: BinOp, lhs: ValueId, rhs: ValueId) -> ValueId {
        let d = self.tmp(ty);
        self.out.push(Inst::Bin {
            op,
            dst: d,
            lhs,
            rhs,
        });
        d
    }

    fn bin_into(&mut self, op: BinOp, dst: ValueId, lhs: ValueId, rhs: ValueId) {
        self.out.push(Inst::Bin {
            op,
            dst,
            lhs,
            rhs,
        });
    }

    fn cast(&mut self, op: CastOp, dst: ValueId, src: ValueId) {
        self.out.push(Inst::Cast { op, dst, src });
    }

    fn cast_tmp(&mut self, op: CastOp, ty: Type, src: ValueId) -> ValueId {
        let d = self.tmp(ty);
        self.cast(op, d, src);
        d
    }

    /// canonicalize a container value for width n into `dst`:
    /// unsigned masks, signed shifts up and arithmetically back down
    fn canon_into(&mut self, dst: ValueId, src: ValueId, n: u8, signed: bool) {
        let cty = if signed { Type::I(64) } else { Type::U(64) };
        if signed {
            let k = self.iconst(cty, 64 - n as i64);
            let hi = self.bin(cty, BinOp::Shl, src, k);
            self.bin_into(BinOp::Shr, dst, hi, k);
        } else {
            let m = self.iconst(cty, ((1u128 << n) - 1) as u64 as i64);
            self.bin_into(BinOp::And, dst, src, m);
        }
    }

    /// shift amounts are taken mod the bit width
    fn amt_mod(&mut self, ty: Type, amt: ValueId, n: u8) -> ValueId {
        let nn = self.iconst(ty, n as i64);
        self.bin(ty, BinOp::Rem, amt, nn)
    }
}

// ---------------------------------------------------------------------------
// widths

pub fn lower_widths(module: &mut Module) {
    for func in &mut module.funcs {
        let has_odd = func.values.iter().any(|v| odd(v.ty).is_some());
        if has_odd {
            lower_function(func);
        }
    }
}

fn lower_function(func: &mut Function) {
    let mut subst: HashMap<ValueId, ValueId> = HashMap::new();
    for b in 0..func.blocks.len() {
        let insts = std::mem::take(&mut func.blocks[b].insts);
        let out = {
            let mut lw = Lw {
                func,
                out: Vec::with_capacity(insts.len()),
            };
            for inst in insts {
                lower_inst(&mut lw, inst, &mut subst);
            }
            lw.out
        };
        func.blocks[b].insts = out;
    }
    if !subst.is_empty() {
        substitute(func, &subst);
    }
    for v in &mut func.values {
        if odd(v.ty).is_some() {
            v.ty = container(v.ty);
        }
    }
    for r in &mut func.rets {
        if odd(*r).is_some() {
            *r = container(*r);
        }
    }
}

fn lower_inst(lw: &mut Lw, inst: Inst, subst: &mut HashMap<ValueId, ValueId>) {
    match inst {
        Inst::IConst { dst, imm } => {
            let imm = match odd(lw.func.ty(dst)) {
                Some((n, true)) => (imm << (64 - n)) >> (64 - n), // sign-extended canonical
                Some((n, false)) => (imm as u64 & ((1u128 << n) - 1) as u64) as i64,
                None => imm,
            };
            lw.out.push(Inst::IConst { dst, imm });
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let Some((n, signed)) = odd(lw.func.ty(dst)) else {
                lw.out.push(Inst::Bin { op, dst, lhs, rhs });
                return;
            };
            let cty = container(lw.func.ty(dst));
            match op {
                // canonical forms are closed under these
                BinOp::And | BinOp::Or | BinOp::Xor => {
                    lw.out.push(Inst::Bin { op, dst, lhs, rhs });
                }
                // the container's type-driven op is correct on canonical
                // values; signed div can overflow the width (MIN / -1),
                // so re-canonicalize that case
                BinOp::Div | BinOp::Rem => {
                    if signed {
                        let t = lw.bin(cty, op, lhs, rhs);
                        lw.canon_into(dst, t, n, signed);
                    } else {
                        lw.out.push(Inst::Bin { op, dst, lhs, rhs });
                    }
                }
                BinOp::IAdd | BinOp::ISub | BinOp::IMul => {
                    let t = lw.bin(cty, op, lhs, rhs);
                    lw.canon_into(dst, t, n, signed);
                }
                BinOp::Shl => {
                    let a = lw.amt_mod(cty, rhs, n);
                    let t = lw.bin(cty, BinOp::Shl, lhs, a);
                    lw.canon_into(dst, t, n, signed);
                }
                BinOp::Shr => {
                    // canonical values shift correctly by type; results
                    // stay canonical
                    let a = lw.amt_mod(cty, rhs, n);
                    lw.bin_into(BinOp::Shr, dst, lhs, a);
                }
                _ => unreachable!("float ops have no iN operands"),
            }
        }
        // canonical forms compare correctly under the container's
        // type-driven comparison — nothing to change
        Inst::ICmp { .. } => lw.out.push(inst),
        Inst::Cast { op, dst, src } => lower_cast(lw, op, dst, src, subst),
        other => lw.out.push(other),
    }
}

fn lower_cast(
    lw: &mut Lw,
    op: CastOp,
    dst: ValueId,
    src: ValueId,
    subst: &mut HashMap<ValueId, ValueId>,
) {
    let sty = lw.func.ty(src);
    let dty = lw.func.ty(dst);
    let so = odd(sty);
    let dodd = odd(dty);
    if so.is_none() && dodd.is_none() {
        lw.out.push(Inst::Cast { op, dst, src });
        return;
    }
    match op {
        CastOp::Ext => {
            let src_signed = sty.is_signed();
            match (so, dodd) {
                // odd -> 64-bit: canonical bits are already right; same
                // signedness is identity, mixed reinterprets
                (Some((_, ss)), None) if dty.width() == Some(64) => {
                    if ss == dty.is_signed() {
                        subst.insert(dst, src);
                    } else {
                        lw.cast(CastOp::Bitcast, dst, src);
                    }
                }
                // odd -> 32-bit: low bits of the canonical container
                (Some(_), None) => {
                    lw.cast(CastOp::Trunc, dst, src);
                }
                // odd -> odd
                (Some((_, ss)), Some((dn, ds))) => {
                    if ss == ds {
                        subst.insert(dst, src); // canonical form carries over
                    } else if !ss {
                        // unsigned into signed width: top bit is clear, so
                        // the value is already canonical — reinterpret
                        lw.cast(CastOp::Bitcast, dst, src);
                    } else {
                        // signed into unsigned width: re-canonicalize
                        let b = lw.cast_tmp(CastOp::Bitcast, Type::U(64), src);
                        lw.canon_into(dst, b, dn, ds);
                    }
                }
                // core -> odd
                (None, Some((dn, ds))) => {
                    if src_signed == ds || !src_signed {
                        // core ext fills by source sign; canonical for the
                        // destination in these cases
                        lw.cast(CastOp::Ext, dst, src);
                    } else {
                        // signed source into unsigned width: sign-extend,
                        // then mask down to the destination width
                        let w = lw.cast_tmp(CastOp::Ext, Type::I(64), src);
                        let b = lw.cast_tmp(CastOp::Bitcast, Type::U(64), w);
                        lw.canon_into(dst, b, dn, ds);
                    }
                }
                (None, None) => unreachable!(),
            }
        }
        CastOp::Trunc => {
            match dodd {
                Some((dn, ds)) => {
                    // widen 32-bit sources into a container first (fill is
                    // irrelevant: only low bits survive)
                    let wide = if sty.width() == Some(32) {
                        lw.cast_tmp(CastOp::Ext, container(sty), src)
                    } else {
                        src
                    };
                    // match the destination's container signedness; same
                    // sign means the (future) containers already agree
                    let wty = lw.func.ty(wide);
                    let wide = if container(wty) != container(dty) {
                        lw.cast_tmp(CastOp::Bitcast, container(dty), wide)
                    } else {
                        wide
                    };
                    lw.canon_into(dst, wide, dn, ds);
                }
                // odd -> narrower core: container trunc takes low bits
                None => lw.cast(CastOp::Trunc, dst, src),
            }
        }
        CastOp::Bitcast => {
            // odd <-> odd same width: same bits, different canonical form;
            // re-canonicalize into the destination's
            match (so, dodd) {
                (Some(_), Some((dn, ds))) => {
                    let b = if container(sty) != container(dty) {
                        lw.cast_tmp(CastOp::Bitcast, container(dty), src)
                    } else {
                        src
                    };
                    lw.canon_into(dst, b, dn, ds);
                }
                _ => lw.out.push(Inst::Cast { op, dst, src }),
            }
        }
        _ => unreachable!("float casts are restricted to 32/64-bit ints"),
    }
}

// ---------------------------------------------------------------------------
// structs

fn carrier(total: u32) -> Type {
    Type::U(total as u8)
}

fn lower_structs(module: &mut Module) {
    for func in &mut module.funcs {
        let has = func.values.iter().any(|v| matches!(v.ty, Type::Struct(_)));
        if has {
            lower_structs_fn(func);
        }
    }
}

fn lower_structs_fn(func: &mut Function) {
    let structs = func.structs.clone();
    let mut subst: HashMap<ValueId, ValueId> = HashMap::new();
    for b in 0..func.blocks.len() {
        let insts = std::mem::take(&mut func.blocks[b].insts);
        let out = {
            let mut lw = Lw {
                func,
                out: Vec::with_capacity(insts.len()),
            };
            for inst in insts {
                lower_struct_inst(&mut lw, &structs, inst, &mut subst);
            }
            lw.out
        };
        func.blocks[b].insts = out;
    }
    if !subst.is_empty() {
        substitute(func, &subst);
    }
    for v in &mut func.values {
        if let Type::Struct(i) = v.ty {
            v.ty = carrier(structs[i as usize].total_bits());
        }
    }
    for r in &mut func.rets {
        if let Type::Struct(i) = *r {
            *r = carrier(structs[i as usize].total_bits());
        }
    }
}

fn struct_of(func: &Function, v: ValueId) -> Option<u16> {
    match func.ty(v) {
        Type::Struct(i) => Some(i),
        _ => None,
    }
}

impl Lw<'_> {
    /// widen a field value into the carrier and mask to its width —
    /// masking makes sign-extended (signed-field) values safe to place
    fn field_to_carrier(&mut self, c: Type, w: u32, total: u32, v: ValueId) -> ValueId {
        let vty = self.func.ty(v);
        let widened = if vty.width() == Some(total) {
            if vty == c {
                v
            } else {
                self.cast_tmp(CastOp::Bitcast, c, v)
            }
        } else {
            // Ext straight to the carrier type; the width pass sorts out
            // fills and canonical forms, the mask below cleans the sign
            self.cast_tmp(CastOp::Ext, c, v)
        };
        if w == total {
            widened
        } else {
            let m = self.iconst(c, ((1u128 << w) - 1) as u64 as i64);
            self.bin(c, BinOp::And, widened, m)
        }
    }
}

fn lower_struct_inst(
    lw: &mut Lw,
    structs: &[crate::ssa::StructDef],
    inst: Inst,
    subst: &mut HashMap<ValueId, ValueId>,
) {
    match inst {
        Inst::Extract { dst, src, field } => {
            let def = &structs[struct_of(lw.func, src).unwrap() as usize];
            let total = def.total_bits();
            let c = carrier(total);
            let w = def.fields[field as usize].1.width().unwrap();
            let off = def.offset(field as usize);
            if w == total {
                if lw.func.ty(dst) == c {
                    subst.insert(dst, src);
                } else {
                    lw.cast(CastOp::Bitcast, dst, src);
                }
                return;
            }
            let shifted = if off > 0 {
                let k = lw.iconst(c, off as i64);
                lw.bin(c, BinOp::Shr, src, k)
            } else {
                src
            };
            lw.cast(CastOp::Trunc, dst, shifted);
        }
        Inst::Pack { dst, args } => {
            let def = &structs[struct_of(lw.func, dst).unwrap() as usize];
            let total = def.total_bits();
            let c = carrier(total);
            if def.fields.len() == 1 {
                if lw.func.ty(args[0]) == c {
                    subst.insert(dst, args[0]);
                } else {
                    lw.cast(CastOp::Bitcast, dst, args[0]);
                }
                return;
            }
            let mut acc: Option<ValueId> = None;
            let n = def.fields.len();
            for (i, (&arg, (_, fty))) in args.iter().zip(&def.fields).enumerate() {
                let w = fty.width().unwrap();
                let off = def.offset(i);
                let masked = lw.field_to_carrier(c, w, total, arg);
                let shifted = if off > 0 {
                    let k = lw.iconst(c, off as i64);
                    lw.bin(c, BinOp::Shl, masked, k)
                } else {
                    masked
                };
                acc = Some(match acc {
                    None => shifted,
                    Some(a) => {
                        if i == n - 1 {
                            lw.bin_into(BinOp::Or, dst, a, shifted);
                            dst
                        } else {
                            lw.bin(c, BinOp::Or, a, shifted)
                        }
                    }
                });
            }
        }
        Inst::Insert {
            dst,
            src,
            field,
            val,
        } => {
            let def = &structs[struct_of(lw.func, src).unwrap() as usize];
            let total = def.total_bits();
            let c = carrier(total);
            let w = def.fields[field as usize].1.width().unwrap();
            let off = def.offset(field as usize);
            if w == total {
                if lw.func.ty(val) == c {
                    subst.insert(dst, val);
                } else {
                    lw.cast(CastOp::Bitcast, dst, val);
                }
                return;
            }
            let field_mask = (((1u128 << w) - 1) as u64) << off;
            let keep = !field_mask & ((1u128 << total) - 1) as u64;
            let km = lw.iconst(c, keep as i64);
            let cleared = lw.bin(c, BinOp::And, src, km);
            let masked = lw.field_to_carrier(c, w, total, val);
            let shifted = if off > 0 {
                let k = lw.iconst(c, off as i64);
                lw.bin(c, BinOp::Shl, masked, k)
            } else {
                masked
            };
            lw.bin_into(BinOp::Or, dst, cleared, shifted);
        }
        Inst::Cast {
            op: CastOp::Bitcast,
            dst,
            src,
        } => {
            // struct<->scalar: identical bits; drop when the post-retype
            // types will match, keep the (cheap) bitcast otherwise
            let post = |f: &Function, v: ValueId| match f.ty(v) {
                Type::Struct(i) => carrier(structs[i as usize].total_bits()),
                t => t,
            };
            if post(lw.func, src) == post(lw.func, dst) {
                subst.insert(dst, src);
            } else {
                lw.out.push(Inst::Cast {
                    op: CastOp::Bitcast,
                    dst,
                    src,
                });
            }
        }
        other => lw.out.push(other),
    }
}

/// Replace every use of the mapped values (chasing chains).
pub(crate) fn substitute(func: &mut Function, map: &HashMap<ValueId, ValueId>) {
    let resolve = |mut v: ValueId| {
        while let Some(&n) = map.get(&v) {
            v = n;
        }
        v
    };
    let fix = |v: &mut ValueId| *v = resolve(*v);
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            match inst {
                Inst::IConst { .. } | Inst::FConst { .. } => {}
                Inst::Bin { lhs, rhs, .. }
                | Inst::ICmp { lhs, rhs, .. }
                | Inst::FCmp { lhs, rhs, .. } => {
                    fix(lhs);
                    fix(rhs);
                }
                Inst::Cast { src, .. } | Inst::Extract { src, .. } => fix(src),
                Inst::Pack { args, .. } => args.iter_mut().for_each(fix),
                Inst::Insert { src, val, .. } => {
                    fix(src);
                    fix(val);
                }
                Inst::Load { addr, .. } => fix(addr),
                Inst::Store { val, addr } => {
                    fix(val);
                    fix(addr);
                }
                Inst::PtrAdd { base, off, .. } => {
                    fix(base);
                    fix(off);
                }
                Inst::Call { args, .. } => args.iter_mut().for_each(fix),
                Inst::Jmp { args, .. } => args.iter_mut().for_each(fix),
                Inst::Br {
                    cond,
                    then_args,
                    else_args,
                    ..
                } => {
                    fix(cond);
                    then_args.iter_mut().for_each(fix);
                    else_args.iter_mut().for_each(fix);
                }
                Inst::Ret { vals } => vals.iter_mut().for_each(fix),
            }
        }
    }
}
