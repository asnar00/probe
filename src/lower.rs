//! Mandatory lowering passes that erase the richer type-system features
//! before emission, so backends only ever see the core types
//! (i1/i32/i64/ptr/f32/f64).
//!
//! `lower_widths`: arbitrary-width integers (i5, i11, i52...) become
//! masked i64 operations. The canonical representation of an iN value is
//! zero-extended in its container: closed under and/or/xor/udiv/urem/
//! unsigned compares as-is; add/sub/mul/shl re-mask their result; signed
//! operations sign-extend their operands into temporaries first (shift
//! left then arithmetic shift right); shift amounts reduce mod N. Because
//! types live on variables, the final step is just retyping the value
//! tables — instructions never carry widths.

use crate::ssa::{BinOp, CastOp, Cond, Function, Inst, Module, Type, ValueId};
use std::collections::HashMap;

/// The full mandatory lowering: structs first (they produce iN carriers and
/// operations), then arbitrary widths.
pub fn lower(module: &mut Module) {
    lower_structs(module);
    lower_widths(module);
}

// ---------------------------------------------------------------------------
// lower_structs: packed bitfield structs -> carrier integers.
//
// A struct's carrier is the integer of its total width (i64/i32 for the
// named widths, iN otherwise — lower_widths finishes those). `extract` is
// shift+truncate, `pack` is zext+shift+or, `insert` is mask+or. Whole-
// width identities (single-field structs, struct<->int bitcasts) collapse
// to value substitutions and cost nothing.

fn carrier(total: u32) -> Type {
    match total {
        64 => Type::I64,
        32 => Type::I32,
        1 => Type::I1,
        n => Type::IN(n as u8),
    }
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

impl Lw<'_> {
    fn cconst(&mut self, c: Type, v: i64) -> ValueId {
        let d = self.tmp(c);
        self.out.push(Inst::IConst { dst: d, imm: v });
        d
    }

    fn cbin(&mut self, c: Type, op: BinOp, lhs: ValueId, rhs: ValueId) -> ValueId {
        let d = self.tmp(c);
        self.out.push(Inst::Bin {
            op,
            dst: d,
            lhs,
            rhs,
        });
        d
    }
}

fn struct_of(func: &Function, v: ValueId) -> Option<u16> {
    match func.ty(v) {
        Type::Struct(i) => Some(i),
        _ => None,
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
                subst.insert(dst, src);
                return;
            }
            let shifted = if off > 0 {
                let k = lw.cconst(c, off as i64);
                lw.cbin(c, BinOp::LShr, src, k)
            } else {
                src
            };
            lw.out.push(Inst::Cast {
                op: CastOp::Trunc,
                dst,
                src: shifted,
            });
        }
        Inst::Pack { dst, args } => {
            let def = &structs[struct_of(lw.func, dst).unwrap() as usize];
            let total = def.total_bits();
            let c = carrier(total);
            if def.fields.len() == 1 {
                subst.insert(dst, args[0]);
                return;
            }
            let mut acc: Option<ValueId> = None;
            let n = def.fields.len();
            for (i, (&arg, (_, fty))) in args.iter().zip(&def.fields).enumerate() {
                let w = fty.width().unwrap();
                let off = def.offset(i);
                let widened = if w == total {
                    arg
                } else {
                    let t = lw.tmp(c);
                    lw.out.push(Inst::Cast {
                        op: CastOp::Zext,
                        dst: t,
                        src: arg,
                    });
                    t
                };
                let shifted = if off > 0 {
                    let k = lw.cconst(c, off as i64);
                    lw.cbin(c, BinOp::Shl, widened, k)
                } else {
                    widened
                };
                acc = Some(match acc {
                    None => shifted,
                    Some(a) => {
                        if i == n - 1 {
                            lw.out.push(Inst::Bin {
                                op: BinOp::Or,
                                dst,
                                lhs: a,
                                rhs: shifted,
                            });
                            dst
                        } else {
                            lw.cbin(c, BinOp::Or, a, shifted)
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
                subst.insert(dst, val);
                return;
            }
            let field_mask = ((1u64 << w) - 1) << off;
            let keep = if total == 64 {
                !field_mask
            } else {
                !field_mask & ((1u64 << total) - 1)
            };
            let km = lw.cconst(c, keep as i64);
            let cleared = lw.cbin(c, BinOp::And, src, km);
            let widened = {
                let t = lw.tmp(c);
                lw.out.push(Inst::Cast {
                    op: CastOp::Zext,
                    dst: t,
                    src: val,
                });
                t
            };
            let shifted = if off > 0 {
                let k = lw.cconst(c, off as i64);
                lw.cbin(c, BinOp::Shl, widened, k)
            } else {
                widened
            };
            lw.out.push(Inst::Bin {
                op: BinOp::Or,
                dst,
                lhs: cleared,
                rhs: shifted,
            });
        }
        Inst::Cast {
            op: CastOp::Bitcast,
            dst,
            src,
        } => {
            // post-retype view of each side: same type -> pure identity
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

pub fn lower_widths(module: &mut Module) {
    for func in &mut module.funcs {
        let has_odd = func.values.iter().any(|v| matches!(v.ty, Type::IN(_)));
        if has_odd {
            lower_function(func);
        }
    }
}

fn width_of(func: &Function, v: ValueId) -> Option<u8> {
    match func.ty(v) {
        Type::IN(n) => Some(n),
        _ => None,
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

    fn iconst(&mut self, v: i64) -> ValueId {
        let d = self.tmp(Type::I64);
        self.out.push(Inst::IConst { dst: d, imm: v });
        d
    }

    fn bin(&mut self, op: BinOp, lhs: ValueId, rhs: ValueId) -> ValueId {
        let d = self.tmp(Type::I64);
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

    /// canonical-form mask: keep the low n bits
    fn mask_into(&mut self, dst: ValueId, src: ValueId, n: u8) {
        let m = self.iconst(((1u64 << n) - 1) as i64);
        self.bin_into(BinOp::And, dst, src, m);
    }

    /// sign-extend the low n bits of a canonical value into a temp
    fn sx(&mut self, src: ValueId, n: u8) -> ValueId {
        let k = self.iconst(64 - n as i64);
        let hi = self.bin(BinOp::Shl, src, k);
        self.bin(BinOp::AShr, hi, k)
    }

    /// shift amounts are taken mod the bit width
    fn amt_mod(&mut self, amt: ValueId, n: u8) -> ValueId {
        let nn = self.iconst(n as i64);
        self.bin(BinOp::URem, amt, nn)
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

    // identity rewrites (zext iN -> wider) collapse to their source
    if !subst.is_empty() {
        substitute(func, &subst);
    }

    // the payoff of types-on-variables: retyping IS the lowering finale
    for v in &mut func.values {
        if matches!(v.ty, Type::IN(_)) {
            v.ty = Type::I64;
        }
    }
    for r in &mut func.rets {
        if matches!(r, Type::IN(_)) {
            *r = Type::I64;
        }
    }
}

fn lower_inst(lw: &mut Lw, inst: Inst, subst: &mut HashMap<ValueId, ValueId>) {
    match inst {
        Inst::IConst { dst, imm } => {
            let imm = match width_of(lw.func, dst) {
                Some(n) => (imm as u64 & ((1u64 << n) - 1)) as i64,
                None => imm,
            };
            lw.out.push(Inst::IConst { dst, imm });
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let Some(n) = width_of(lw.func, dst).or_else(|| width_of(lw.func, lhs)) else {
                lw.out.push(Inst::Bin { op, dst, lhs, rhs });
                return;
            };
            match op {
                BinOp::And | BinOp::Or | BinOp::Xor | BinOp::UDiv | BinOp::URem => {
                    lw.out.push(Inst::Bin { op, dst, lhs, rhs });
                }
                BinOp::IAdd | BinOp::ISub | BinOp::IMul => {
                    let t = lw.bin(op, lhs, rhs);
                    lw.mask_into(dst, t, n);
                }
                BinOp::Shl => {
                    let a = lw.amt_mod(rhs, n);
                    let t = lw.bin(BinOp::Shl, lhs, a);
                    lw.mask_into(dst, t, n);
                }
                BinOp::LShr => {
                    let a = lw.amt_mod(rhs, n);
                    lw.bin_into(BinOp::LShr, dst, lhs, a);
                }
                BinOp::AShr => {
                    let sl = lw.sx(lhs, n);
                    let a = lw.amt_mod(rhs, n);
                    let t = lw.bin(BinOp::AShr, sl, a);
                    lw.mask_into(dst, t, n);
                }
                BinOp::SDiv | BinOp::SRem => {
                    let sl = lw.sx(lhs, n);
                    let sr = lw.sx(rhs, n);
                    let t = lw.bin(op, sl, sr);
                    lw.mask_into(dst, t, n);
                }
                _ => unreachable!("float ops have no iN operands"),
            }
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let Some(n) = width_of(lw.func, lhs) else {
                lw.out.push(Inst::ICmp {
                    cond,
                    dst,
                    lhs,
                    rhs,
                });
                return;
            };
            let signed = matches!(cond, Cond::Slt | Cond::Sle | Cond::Sgt | Cond::Sge);
            let (l, r) = if signed {
                (lw.sx(lhs, n), lw.sx(rhs, n))
            } else {
                (lhs, rhs) // canonical zero-extension preserves unsigned order
            };
            lw.out.push(Inst::ICmp {
                cond,
                dst,
                lhs: l,
                rhs: r,
            });
        }
        Inst::Cast { op, dst, src } => {
            let sn = width_of(lw.func, src);
            let dn = width_of(lw.func, dst);
            if sn.is_none() && dn.is_none() {
                lw.out.push(Inst::Cast { op, dst, src });
                return;
            }
            let sty = lw.func.ty(src);
            let dty = lw.func.ty(dst);
            match op {
                CastOp::Zext => {
                    match (sn, dty) {
                        // iN -> i64: canonical form IS the answer
                        (Some(_), Type::I64) => {
                            subst.insert(dst, src);
                        }
                        // iN -> i32 / iM: take low bits of the canonical value
                        (Some(_), Type::I32) => {
                            lw.out.push(Inst::Cast {
                                op: CastOp::Trunc,
                                dst,
                                src,
                            });
                        }
                        (Some(_), Type::IN(_)) => {
                            subst.insert(dst, src); // wider iM: still canonical
                        }
                        // i1/i32 -> iN: widen through the core cast
                        (None, _) => {
                            if sty == Type::I32 {
                                lw.out.push(Inst::Cast {
                                    op: CastOp::Zext,
                                    dst,
                                    src,
                                });
                            } else {
                                // i1: 0/1, already canonical at any width
                                lw.out.push(Inst::Cast {
                                    op: CastOp::Zext,
                                    dst,
                                    src,
                                });
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                CastOp::Sext => {
                    // source is iN: materialize the sign, then narrow to dst
                    if let Some(n) = sn {
                        let sxv = lw.sx(src, n);
                        match dty {
                            Type::I64 => {
                                subst.insert(dst, sxv);
                            }
                            Type::I32 => lw.out.push(Inst::Cast {
                                op: CastOp::Trunc,
                                dst,
                                src: sxv,
                            }),
                            Type::IN(m) => lw.mask_into(dst, sxv, m),
                            _ => unreachable!(),
                        }
                    } else {
                        // i1/i32 -> iN(m): core sign-extend to 64, mask to m
                        let m = dn.unwrap();
                        let wide = lw.tmp(Type::I64);
                        lw.out.push(Inst::Cast {
                            op: CastOp::Sext,
                            dst: wide,
                            src,
                        });
                        lw.mask_into(dst, wide, m);
                    }
                }
                CastOp::Trunc => {
                    if let Some(m) = dn {
                        // any wider integer -> iN(m): mask the canonical bits
                        let wide = if sty == Type::I32 {
                            let w = lw.tmp(Type::I64);
                            lw.out.push(Inst::Cast {
                                op: CastOp::Zext,
                                dst: w,
                                src,
                            });
                            w
                        } else {
                            src
                        };
                        lw.mask_into(dst, wide, m);
                    } else {
                        // iN -> i32/i1: the core trunc takes low bits, which
                        // is exactly right for canonical values
                        lw.out.push(Inst::Cast { op, dst, src });
                    }
                }
                _ => unreachable!("float casts are restricted to i32/i64"),
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
