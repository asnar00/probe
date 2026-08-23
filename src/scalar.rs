// The `scalar -> rat` half of the abstract scalar type. When the policy
// picks a concrete float, `scalar` resolves by substitution like `float`
// does and this module has nothing to do. When it picks `rat`, resolution
// retypes scalar values to the `$rat` struct and this pass rewrites their
// float-opcode operations into rational-library calls — the soften recipe
// with a different runtime. The library itself (lib/rational.ssa) is
// ordinary SSA, linked in textually before parsing so the whole module
// shares one struct table.

use crate::ssa::{self, CastOp, FCond, Inst, Module, Policy, ScalarPolicy, Type, ValueId};

pub const RAT_LIB: &str = include_str!("../lib/rational.ssa");

/// Textually link the rational library into a source file when it is (or
/// may become) needed: the file uses `$rat` / `@rat_*` directly, or the
/// policy resolves `scalar` to rat. Idempotent by construction.
pub fn link(src: &str, policy: &Policy) -> String {
    if src.contains("fn @rat_gcd") {
        return src.to_string(); // library already present
    }
    let uses_rat = src.contains("$rat") || src.contains("@rat_");
    let needs_scalar = policy.scalar == ScalarPolicy::Rat && src.contains("scalar");
    if uses_rat || needs_scalar {
        // prepended: the parser must know $rat before its first use
        format!("{}\n{}", RAT_LIB, src)
    } else {
        src.to_string()
    }
}

/// Convert an f64 to an exact (num, den) pair that fits $rat's fields
/// (hw bits each — half the policy's word). Every finite float is a
/// rational with a power-of-two denominator; scalar constants in
/// portable code are simple dyadics, so most fit easily.
fn exact_rat(v: f64, hw: u32) -> Result<(i64, u64), String> {
    if !v.is_finite() {
        return Err(format!("scalar constant {} is not finite", v));
    }
    let (mut num, mut den) = (v, 1u64);
    while num.fract() != 0.0 {
        num *= 2.0;
        den = den
            .checked_mul(2)
            .ok_or_else(|| format!("scalar constant {} not representable as rat", v))?;
    }
    if num.abs() > (1i64 << 62) as f64 {
        return Err(format!("scalar constant {} not representable as rat", v));
    }
    let mut n = num as i64;
    let mut d = den;
    let g = gcd(n.unsigned_abs().max(d), n.unsigned_abs().min(d).max(1));
    if n != 0 {
        n /= g as i64;
        d /= g;
    } else {
        d = 1;
    }
    let fits = if n < 0 {
        -n <= 1i64 << (hw - 1)
    } else {
        n < 1i64 << (hw - 1)
    };
    if !fits || d >= 1u64 << hw {
        return Err(format!(
            "scalar constant {} not representable as a {}-bit rat",
            v, hw
        ));
    }
    Ok((n, d))
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Rewrite float-opcode operations on `$rat`-typed values into calls to
/// the rational library. Runs after type resolution (so scalar values are
/// already `$rat`) and before soften and verification. Real float ops are
/// untouched — scalar and concrete floats mix freely in one module.
pub fn scalarize(module: &mut Module) -> Result<(), String> {
    let Some(rid) = module.structs.iter().position(|d| d.name == "rat") else {
        return Ok(()); // no library linked, so no rat-typed values exist
    };
    // the library is width-agnostic ($rat = { num: half, den: uhalf });
    // read the RESOLVED field width and derive the word type from it
    let hw = match module.structs[rid].fields[0].1 {
        Type::I(n) => n as u32,
        t => {
            return Err(format!(
                "$rat's num field resolved to {}, expected a signed integer",
                t.name()
            ))
        }
    };
    let rat = Type::Struct(rid as u16);
    let mut errs = Vec::new();
    for func in &mut module.funcs {
        scalarize_function(func, rat, hw, &mut errs);
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

fn scalarize_function(func: &mut ssa::Function, rat: Type, hw: u32, errs: &mut Vec<String>) {
    // the word: twice the field width — what rat_make takes and
    // rat_to_int returns under the current policy
    let wty = Type::I((2 * hw) as u8);
    let mut ntmp = 0u32;
    for b in 0..func.blocks.len() {
        let insts = std::mem::take(&mut func.blocks[b].insts);
        let mut out = Vec::with_capacity(insts.len());
        let mut tmp = |func: &mut ssa::Function, ty: Type| {
            ntmp += 1;
            func.values.push(ssa::ValueData {
                name: format!("sc{}", ntmp),
                ty,
            });
            ValueId(func.values.len() as u32 - 1)
        };
        let call1 = |dst: ValueId, callee: &str, args: Vec<ValueId>| Inst::Call {
            dsts: vec![dst],
            callee: callee.to_string(),
            args,
        };
        for inst in insts {
            match inst {
                Inst::FConst { dst, bits } if func.ty(dst) == rat => {
                    match exact_rat(f64::from_bits(bits), hw) {
                        Ok((n, d)) => {
                            let nv = tmp(func, Type::I(hw as u8));
                            let dv = tmp(func, Type::U(hw as u8));
                            out.push(Inst::IConst { dst: nv, imm: n });
                            out.push(Inst::IConst { dst: dv, imm: d as i64 });
                            out.push(Inst::Pack {
                                dst,
                                args: vec![nv, dv],
                            });
                        }
                        Err(e) => errs.push(format!("{}: {}", func.name, e)),
                    }
                }
                Inst::Bin { op, dst, lhs, rhs } if op.is_float() && func.ty(dst) == rat => {
                    let name = match op {
                        ssa::BinOp::FAdd => "rat_add",
                        ssa::BinOp::FSub => "rat_sub",
                        ssa::BinOp::FMul => "rat_mul",
                        ssa::BinOp::FDiv => "rat_div",
                        _ => unreachable!(),
                    };
                    out.push(call1(dst, name, vec![lhs, rhs]));
                }
                Inst::FCmp {
                    cond,
                    dst,
                    lhs,
                    rhs,
                } if func.ty(lhs) == rat => {
                    // gt/ge are lt/le with the operands swapped
                    let (name, a, b2) = match cond {
                        FCond::Oeq => ("rat_eq", lhs, rhs),
                        FCond::Une => ("rat_ne", lhs, rhs),
                        FCond::Olt => ("rat_lt", lhs, rhs),
                        FCond::Ole => ("rat_le", lhs, rhs),
                        FCond::Ogt => ("rat_lt", rhs, lhs),
                        FCond::Oge => ("rat_le", rhs, lhs),
                    };
                    out.push(call1(dst, name, vec![a, b2]));
                }
                Inst::Cast {
                    op: CastOp::Itof,
                    dst,
                    src,
                } if func.ty(dst) == rat => {
                    // rat_make takes two words: adapt the source's width
                    let sw = func.ty(src);
                    let n64 = if sw == wty {
                        src
                    } else {
                        let t = tmp(func, wty);
                        let op = match (sw.width(), wty.width()) {
                            (Some(a), Some(b)) if a < b => CastOp::Ext,
                            (Some(a), Some(b)) if a > b => CastOp::Trunc,
                            _ => CastOp::Bitcast,
                        };
                        out.push(Inst::Cast { op, dst: t, src });
                        t
                    };
                    let one = tmp(func, wty);
                    out.push(Inst::IConst { dst: one, imm: 1 });
                    out.push(call1(dst, "rat_make", vec![n64, one]));
                }
                Inst::Cast {
                    op: CastOp::Ftoi,
                    dst,
                    src,
                } if func.ty(src) == rat => {
                    // rat_to_int returns a word: adapt to the destination
                    let dt = func.ty(dst);
                    if dt == wty {
                        out.push(call1(dst, "rat_to_int", vec![src]));
                    } else {
                        let t = tmp(func, wty);
                        out.push(call1(t, "rat_to_int", vec![src]));
                        let op = match (wty.width(), dt.width()) {
                            (Some(a), Some(b)) if a < b => CastOp::Ext,
                            (Some(a), Some(b)) if a > b => CastOp::Trunc,
                            _ => CastOp::Bitcast,
                        };
                        out.push(Inst::Cast { op, dst, src: t });
                    }
                }
                other => out.push(other),
            }
        }
        func.blocks[b].insts = out;
    }
}
