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
    lower_structs(module);
    lower_widths(module);
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
