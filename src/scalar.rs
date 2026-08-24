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
pub const FLOAT_LIB: &str = include_str!("../lib/float.ssa");

/// the non-native float(E, M) pairs a source mentions, by text scan —
/// each needs conversion-function instances, forced by a linked trailer
fn small_float_pairs(src: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while let Some(p) = src[i..].find("float") {
        let mut j = i + p + 5;
        while j < b.len() && (b[j] == b' ') {
            j += 1;
        }
        if j >= b.len() || b[j] != b'(' {
            i += p + 5;
            continue;
        }
        j += 1;
        let num = |j: &mut usize| -> Option<u32> {
            while *j < b.len() && b[*j] == b' ' {
                *j += 1;
            }
            let s = *j;
            while *j < b.len() && b[*j].is_ascii_digit() {
                *j += 1;
            }
            if *j == s {
                return None;
            }
            src[s..*j].parse().ok()
        };
        let e = num(&mut j);
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        if e.is_some() && j < b.len() && b[j] == b',' {
            j += 1;
            let m = num(&mut j);
            if let (Some(e), Some(m)) = (e, m) {
                if (e, m) != (8, 23) && (e, m) != (11, 52) && !out.contains(&(e, m)) {
                    out.push((e, m));
                }
            }
        }
        i += p + 5;
    }
    out
}

/// Textually link the rational library into a source file when it is (or
/// may become) needed: the file uses `$rat` / `@rat_*` directly, or the
/// policy resolves `scalar` to rat. Idempotent by construction.
pub fn link(src: &str, policy: &Policy) -> String {
    if src.contains("fn @rat_gcd") {
        return src.to_string(); // library already present
    }
    let uses_rat = src.contains("$rat") || src.contains("@rat_");
    let needs_scalar = policy.scalar == ScalarPolicy::Rat && src.contains("scalar");
    let mut out = if uses_rat || needs_scalar {
        // prepended: the parser must know $rat before its first use
        format!("{}\n{}", RAT_LIB, src)
    } else {
        src.to_string()
    };
    // small floats: link the generic conversion library and force the
    // instances for every float(E, M) pair the source mentions
    let pairs = small_float_pairs(&out);
    let direct = ["softfloat_to_f64", "softfloat_from_f64", "softfloat_add", "softfloat_sub", "softfloat_mul", "softfloat_qnan"]
        .iter()
        .any(|n| out.contains(n));
    if (!pairs.is_empty() || direct) && !out.contains("group softfloat") {
        out.push('\n');
        out.push_str(FLOAT_LIB);
        for (e, m) in pairs {
            out.push_str(&format!(
                "\nfn __fp_force_{e}_{m}(x: f64) -> f64 {{\n    \
                 b: u{t} = softfloat_from_f64({e}, {m})(x)\n    \
                 xf: float({e}, {m}) = bitcast b\n    \
                 c: float({e}, {m}) = softfloat_add(xf, xf)\n    \
                 d: float({e}, {m}) = softfloat_sub(c, xf)\n    \
                 g: float({e}, {m}) = softfloat_mul(d, xf)\n    \
                 gb: u{t} = bitcast g\n    \
                 ret softfloat_to_f64({e}, {m})(gb)\n}}\n",
                e = e,
                m = m,
                t = e + m + 1
            ));
        }
    }
    out
}

/// reference conversions in Rust, mirroring lib/float.ssa: used for
/// small-float constants at lowering time and as the independent
/// referee in the exhaustive tests
pub fn rust_softfloat_to_f64(bits: u64, e: u32, m: u32) -> f64 {
    let s = (bits >> (e + m)) & 1;
    let ex = (bits >> m) & ((1 << e) - 1);
    let f = bits & ((1 << m) - 1);
    let sd = s << 63;
    let emax = (1u64 << e) - 1;
    let bias = (1u64 << (e - 1)) - 1;
    let r = if ex == emax {
        sd | 0x7ff0000000000000 | (f << (52 - m))
    } else if ex == 0 {
        if f == 0 {
            sd
        } else {
            let mut k = 0u64;
            let mut ff = f;
            while (ff >> m) != 1 {
                ff <<= 1;
                k += 1;
            }
            let ed = 1024 - bias - k;
            sd | (ed << 52) | ((ff & ((1 << m) - 1)) << (52 - m))
        }
    } else {
        sd | ((ex + (1023 - bias)) << 52) | (f << (52 - m))
    };
    f64::from_bits(r)
}

pub fn rust_softfloat_from_f64(x: f64, e: u32, m: u32) -> u64 {
    let b = x.to_bits();
    let s = (b >> 63) & 1;
    let ed = (b >> 52) & 0x7ff;
    let fd = b & 0xfffffffffffff;
    let st = s << (e + m);
    let emaxt = (1u64 << e) - 1;
    if ed == 0x7ff {
        let ff = if fd == 0 { 0 } else { 1 << (m - 1) };
        return st | (emaxt << m) | ff;
    }
    if ed == 0 {
        return st; // f64 subnormals: below every target's range
    }
    let bias = (1i64 << (e - 1)) - 1;
    let et = ed as i64 - 1023 + bias;
    if et >= emaxt as i64 {
        return st | (emaxt << m);
    }
    let m53 = fd | (1 << 52);
    let rne = |keep: u64, dropped: u64, half: u64| -> u64 {
        keep + (u64::from(dropped > half) | (u64::from(dropped == half) & keep & 1))
    };
    if et >= 1 {
        let sh = 52 - m;
        let keep2 = rne(m53 >> sh, m53 & ((1 << sh) - 1), 1 << (sh - 1));
        return st + ((et as u64 - 1) << m) + keep2;
    }
    let sh = (52 - m) as i64 + (1 - et);
    if sh >= 54 {
        return st;
    }
    let sh = sh as u64;
    let keep2 = rne(m53 >> sh, m53 & ((1 << sh) - 1), 1 << (sh - 1));
    st | keep2
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

/// Lower small-float (float(E, M)) values: arithmetic promotes to f64,
/// computes natively, and demotes back through the width-generic
/// conversion instances the linker forced; constants demote at compile
/// time via the Rust reference; values retype to their bit patterns.
/// Runs before soften, so on FPU-less targets the f64 ops soften in turn.
pub fn lower_small_floats(module: &mut Module) -> Result<(), String> {
    let any = module.funcs.iter().any(|f| {
        f.values.iter().any(|v| matches!(v.ty, Type::FP(..)))
            || f.rets.iter().any(|t| matches!(t, Type::FP(..)))
    });
    if !any {
        return Ok(());
    }
    let mut errs = Vec::new();
    for func in &mut module.funcs {
        for b in &func.blocks {
            for inst in &b.insts {
                if let Inst::Bin { op, dst, .. } = inst {
                    if !op.is_float() && matches!(func.ty(*dst), Type::FP(..)) {
                        errs.push(format!(
                            "@{}: {} on a small-float value (float types take                              float operations)",
                            func.name,
                            op.name()
                        ));
                    }
                }
            }
        }
    }
    if !errs.is_empty() {
        return Err(errs.join("; "));
    }
    for func in &mut module.funcs {
        lower_fp_function(func);
    }
    // the conversion instances must have been linked in
    if !module.funcs.iter().any(|f| f.name.starts_with("softfloat_to_f64__")) {
        return Err(
            "small floats used, but the conversion library was not linked              (float(E, M) must appear literally in the source text)"
                .into(),
        );
    }
    Ok(())
}

fn lower_fp_function(func: &mut ssa::Function) {
    use std::collections::HashMap;
    let mut subst: HashMap<ValueId, ValueId> = HashMap::new();
    let mut ntmp = 0u32;
    let is_fp = |t: Type| match t {
        Type::FP(e, m) => Some((e as u32, m as u32)),
        _ => None,
    };
    for b in 0..func.blocks.len() {
        let insts = std::mem::take(&mut func.blocks[b].insts);
        let mut out = Vec::with_capacity(insts.len());
        fn tmp(func: &mut ssa::Function, ntmp: &mut u32, ty: Type) -> ValueId {
            *ntmp += 1;
            func.values.push(ssa::ValueData {
                name: format!("fp{}", ntmp),
                ty,
            });
            ValueId(func.values.len() as u32 - 1)
        }
        // promote an FP-typed value to a fresh f64 temp
        fn promote(
            func: &mut ssa::Function,
            out: &mut Vec<Inst>,
            ntmp: &mut u32,
            v: ValueId,
            (e, m): (u32, u32),
        ) -> ValueId {
            let t = tmp(func, ntmp, Type::F64);
            out.push(Inst::Call {
                dsts: vec![t],
                callee: format!("softfloat_to_f64__{}_{}", e, m),
                args: vec![v],
            });
            t
        }
        for inst in insts {
            match inst {
                Inst::FConst { dst, bits } => match is_fp(func.ty(dst)) {
                    Some((e, m)) => out.push(Inst::IConst {
                        dst,
                        imm: rust_softfloat_from_f64(f64::from_bits(bits), e, m) as i64,
                    }),
                    None => out.push(Inst::FConst { dst, bits }),
                },
                Inst::Bin { op, dst, lhs, rhs }
                    if op.is_float() && is_fp(func.ty(dst)).is_some() =>
                {
                    let em = is_fp(func.ty(dst)).unwrap();
                    // add/sub/mul go to the DIY generic library directly:
                    // pure integer code, no f64 detour, full subnormals.
                    // (div still promotes; its DIY algorithm is future
                    // work.) This is the platform fallthrough: a target
                    // exposing a native instruction for the format would
                    // shadow these calls.
                    let diy = match op {
                        ssa::BinOp::FAdd => Some("softfloat_add"),
                        ssa::BinOp::FSub => Some("softfloat_sub"),
                        ssa::BinOp::FMul => Some("softfloat_mul"),
                        _ => None,
                    };
                    if let Some(name) = diy {
                        out.push(Inst::Call {
                            dsts: vec![dst],
                            callee: format!("{}__{}_{}", name, em.0, em.1),
                            args: vec![lhs, rhs],
                        });
                    } else {
                        let ta = promote(func, &mut out, &mut ntmp, lhs, em);
                        let tb = promote(func, &mut out, &mut ntmp, rhs, em);
                        let tr = tmp(func, &mut ntmp, Type::F64);
                        out.push(Inst::Bin {
                            op,
                            dst: tr,
                            lhs: ta,
                            rhs: tb,
                        });
                        out.push(Inst::Call {
                            dsts: vec![dst],
                            callee: format!("softfloat_from_f64__{}_{}", em.0, em.1),
                            args: vec![tr],
                        });
                    }
                }
                Inst::FCmp {
                    cond,
                    dst,
                    lhs,
                    rhs,
                } if is_fp(func.ty(lhs)).is_some() => {
                    let em = is_fp(func.ty(lhs)).unwrap();
                    let ta = promote(func, &mut out, &mut ntmp, lhs, em);
                    let tb = promote(func, &mut out, &mut ntmp, rhs, em);
                    out.push(Inst::FCmp {
                        cond,
                        dst,
                        lhs: ta,
                        rhs: tb,
                    });
                }
                Inst::Cast { op, dst, src } => {
                    let (sf, df) = (is_fp(func.ty(src)), is_fp(func.ty(dst)));
                    match (op, sf, df) {
                        // promote/demote ladders through f64
                        (CastOp::Fpromote | CastOp::Fdemote, Some(em), None) => {
                            let t = promote(func, &mut out, &mut ntmp, src, em);
                            if func.ty(dst) == Type::F64 {
                                subst.insert(dst, t);
                            } else {
                                out.push(Inst::Cast {
                                    op: CastOp::Fdemote,
                                    dst,
                                    src: t,
                                });
                            }
                        }
                        (CastOp::Fpromote | CastOp::Fdemote, None, Some(em)) => {
                            let t = if func.ty(src) == Type::F64 {
                                src
                            } else {
                                let t = tmp(func, &mut ntmp, Type::F64);
                                out.push(Inst::Cast {
                                    op: CastOp::Fpromote,
                                    dst: t,
                                    src,
                                });
                                t
                            };
                            out.push(Inst::Call {
                                dsts: vec![dst],
                                callee: format!("softfloat_from_f64__{}_{}", em.0, em.1),
                                args: vec![t],
                            });
                        }
                        (CastOp::Fpromote | CastOp::Fdemote, Some(sem), Some(dem)) => {
                            let t = promote(func, &mut out, &mut ntmp, src, sem);
                            out.push(Inst::Call {
                                dsts: vec![dst],
                                callee: format!("softfloat_from_f64__{}_{}", dem.0, dem.1),
                                args: vec![t],
                            });
                        }
                        (CastOp::Itof, _, Some(em)) => {
                            let t = tmp(func, &mut ntmp, Type::F64);
                            out.push(Inst::Cast {
                                op: CastOp::Itof,
                                dst: t,
                                src,
                            });
                            out.push(Inst::Call {
                                dsts: vec![dst],
                                callee: format!("softfloat_from_f64__{}_{}", em.0, em.1),
                                args: vec![t],
                            });
                        }
                        (CastOp::Ftoi, Some(em), _) => {
                            let t = promote(func, &mut out, &mut ntmp, src, em);
                            out.push(Inst::Cast {
                                op: CastOp::Ftoi,
                                dst,
                                src: t,
                            });
                        }
                        (CastOp::Bitcast, Some((e, m)), _) | (CastOp::Bitcast, _, Some((e, m))) => {
                            // after retyping, an FP<->uN bitcast may become
                            // an identity
                            let post = |t: Type| match t {
                                Type::FP(e2, m2) => Type::U(e2 + m2 + 1),
                                t => t,
                            };
                            let _ = (e, m);
                            if post(func.ty(src)) == post(func.ty(dst)) {
                                subst.insert(dst, src);
                            } else {
                                out.push(Inst::Cast { op, dst, src });
                            }
                        }
                        _ => out.push(Inst::Cast { op, dst, src }),
                    }
                }
                other => out.push(other),
            }
        }
        func.blocks[b].insts = out;
    }
    if !subst.is_empty() {
        crate::lower::substitute(func, &subst);
    }
    for v in &mut func.values {
        if let Type::FP(e, m) = v.ty {
            v.ty = Type::U(e + m + 1);
        }
    }
    for r in &mut func.rets {
        if let Type::FP(e, m) = *r {
            *r = Type::U(e + m + 1);
        }
    }
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

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn jit(src: &str) -> crate::emit::jit::JitCode {
        let policy = Policy::new(Type::I(64), Type::F64).unwrap();
        let src = link(src, &policy);
        let mut m = ssa::parse(&src).expect("parses");
        ssa::resolve_types(&mut m, &policy);
        scalarize(&mut m).expect("scalarizes");
        lower_small_floats(&mut m).expect("small floats lower");
        ssa::verify(&m).expect("verifies");
        crate::lower::lower_native(&mut m);
        crate::opt::optimize(&mut m, crate::opt::MAX_LEVEL);
        ssa::verify(&m).expect("verifies post-lower");
        let enc = crate::emit::Encoder::load("targets/arm64.encodings.json").unwrap();
        let c = crate::emit::compile(&m, &enc).expect("compiles");
        crate::emit::jit::JitCode::new(&c).expect("maps")
    }

    /// every fp8 e4m3 addition and multiplication, exhaustively, against
    /// the independent Rust reference (promote to f64, compute, demote
    /// RNE) — 65536 pairs per op, bit-exact
    #[test]
    fn fp8_exhaustive_add_mul() {
        let j = jit(
            "fn @add8(%a: u8, %b: u8) -> u8 {\n\
                 %xa: float(4, 3) = bitcast %a\n\
                 %xb: float(4, 3) = bitcast %b\n\
                 %s: float(4, 3) = %xa + %xb\n\
                 %r: u8 = bitcast %s\n\
                 ret %r\n\
             }\n\
             fn @mul8(%a: u8, %b: u8) -> u8 {\n\
                 %xa: float(4, 3) = bitcast %a\n\
                 %xb: float(4, 3) = bitcast %b\n\
                 %s: float(4, 3) = %xa * %xb\n\
                 %r: u8 = bitcast %s\n\
                 ret %r\n\
             }\n",
        );
        // NaN sign/payload is outside the spec (ours canonicalize):
        // NaNs compare as a class, everything else bit-exact
        let is_nan8 = |v: u64| (v >> 3) & 0xf == 0xf && v & 7 != 0;
        let same = |got: u64, want: u64| got == want || (is_nan8(got) && is_nan8(want));
        for a in 0..256u64 {
            for b in 0..256u64 {
                let xa = rust_softfloat_to_f64(a, 4, 3);
                let xb = rust_softfloat_to_f64(b, 4, 3);
                let want_add = rust_softfloat_from_f64(xa + xb, 4, 3);
                let want_mul = rust_softfloat_from_f64(xa * xb, 4, 3);
                let got_add = j.call("add8", &[a as i64, b as i64]).unwrap() as u64 & 0xff;
                let got_mul = j.call("mul8", &[a as i64, b as i64]).unwrap() as u64 & 0xff;
                assert!(
                    same(got_add, want_add),
                    "fp8 {:#04x} + {:#04x}: got {:#04x}, want {:#04x}",
                    a, b, got_add, want_add
                );
                assert!(
                    same(got_mul, want_mul),
                    "fp8 {:#04x} * {:#04x}: got {:#04x}, want {:#04x}",
                    a, b, got_mul, want_mul
                );
            }
        }
    }

    /// the DIY generic add/mul at (8, 23) against the real f32 hardware:
    /// the same generic SSA that runs fp8 on integer-only cores must
    /// reproduce the M1's FPU bit-for-bit at f32. Random pairs plus
    /// directed edges (subnormals, cancellation, overflow, zeros, inf).
    #[test]
    fn f32_diy_vs_fpu() {
        let j = jit(
            "fn a32(a: u32, b: u32) -> u32 {\n\
                 xa: f32 = bitcast a\n\
                 xb: f32 = bitcast b\n\
                 s: f32 = softfloat_add(xa, xb)\n\
                 r: u32 = bitcast s\n\
                 ret r\n\
             }\n\
             fn m32(a: u32, b: u32) -> u32 {\n\
                 xa: f32 = bitcast a\n\
                 xb: f32 = bitcast b\n\
                 s: f32 = softfloat_mul(xa, xb)\n\
                 r: u32 = bitcast s\n\
                 ret r\n\
             }\n",
        );
        let is_nan = |v: u32| (v >> 23) & 0xff == 0xff && v & 0x7fffff != 0;
        let same = |g: u32, w: u32| g == w || (is_nan(g) && is_nan(w));
        let mut check = |a: u32, b: u32| {
            let wa = (f32::from_bits(a) + f32::from_bits(b)).to_bits();
            let wm = (f32::from_bits(a) * f32::from_bits(b)).to_bits();
            let ga = j.call("a32", &[a as i64, b as i64]).unwrap() as u64 as u32;
            let gm = j.call("m32", &[a as i64, b as i64]).unwrap() as u64 as u32;
            assert!(
                same(ga, wa),
                "f32 {:#010x} + {:#010x}: DIY {:#010x}, FPU {:#010x}",
                a, b, ga, wa
            );
            assert!(
                same(gm, wm),
                "f32 {:#010x} * {:#010x}: DIY {:#010x}, FPU {:#010x}",
                a, b, gm, wm
            );
        };
        // directed edges: zeros, subnormal min/max, normal min/max, one,
        // inf, NaN, and sign variants of each
        let edges: Vec<u32> = [
            0x00000000u32, 0x00000001, 0x007fffff, 0x00800000, 0x00800001,
            0x3f800000, 0x3f800001, 0x7f7fffff, 0x7f800000, 0x7fc00001,
            0x34000000, 0x33ffffff, 0x4b800000,
        ]
        .iter()
        .flat_map(|&v| [v, v | 0x8000_0000])
        .collect();
        for &a in &edges {
            for &b in &edges {
                check(a, b);
            }
        }
        // cancellation ladders: a + (-a +/- k ulps)
        for k in 0..8u32 {
            let a = 0x3f80_0000u32 + k;
            check(a, (a ^ 0x8000_0000).wrapping_add(1));
            check(a, (a ^ 0x8000_0000).wrapping_sub(1));
            check(a, a ^ 0x8000_0000);
        }
        // random sweep
        let mut x = 0x243F6A8885A308D3u64;
        let mut rnd = move || {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545F4914F6CDD1D) >> 32) as u32
        };
        for _ in 0..200_000 {
            check(rnd(), rnd());
        }
    }

    /// every fp8 and fp16 value roundtrips f64 exactly (NaN payloads
    /// collapse to the quiet NaN by design and are skipped)
    #[test]
    fn small_float_roundtrip_exhaustive() {
        for &(e, m) in &[(4u32, 3u32), (5, 10), (8, 7), (5, 2)] {
            let total = e + m + 1;
            let emax = (1u64 << e) - 1;
            for bits in 0..(1u64 << total) {
                let ef = (bits >> m) & emax;
                let ff = bits & ((1 << m) - 1);
                if ef == emax && ff != 0 {
                    continue; // NaN payloads quiet by design
                }
                let back = rust_softfloat_from_f64(rust_softfloat_to_f64(bits, e, m), e, m);
                assert_eq!(back, bits, "float({},{}) roundtrip of {:#x}", e, m, bits);
            }
        }
    }
}
