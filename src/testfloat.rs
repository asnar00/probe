//! Berkeley TestFloat as the oracle for lib/float.ssa (and, on the
//! platform, for the hardware): `testfloat_gen` writes IEEE-754 test
//! vectors — operands, the correctly rounded result, the exception
//! flags — and every one of them is run through the library instance for
//! that width and compared bit for bit. tools/get-testfloat.sh builds the
//! generator, with the RISC-V specialization: one positive canonical NaN,
//! which is what the library produces too.
//!
//! What is not compared: exception flags (the IR has none), and float-to-
//! integer results flagged invalid (NaN, out of range: the library
//! saturates and sends NaN to 0, TestFloat's RISC-V rules differ).

use crate::{emit, opt, platform, ssa};
use std::fmt::Write;
use std::process::Command;

/// testfloat_gen's names for the policy's rounding modes, in order
const ROUND_FLAGS: [&str; 5] = ["-rnear_even", "-rminMag", "-rmin", "-rmax", "-rnear_maxMag"];

pub const GEN: &str = "tools/berkeley-testfloat-3/build/Linux-x86_64-GCC/testfloat_gen";

/// a width TestFloat knows, as the library's float(E, M)
fn width(name: &str) -> Option<(u32, u32)> {
    match name {
        "f16" => Some((5, 10)),
        "f32" => Some((8, 23)),
        "f64" => Some((11, 52)),
        _ => None,
    }
}

fn bits_of(name: &str) -> u32 {
    match name {
        "f16" => 16,
        "f32" | "i32" | "ui32" => 32,
        _ => 64,
    }
}

/// one TestFloat function: the SSA wrapper that computes it, and how
/// many operands it takes
struct Op {
    name: String,      // f32_add
    wrapper: String,   // the SSA function text
    nargs: usize,
    result_bits: u32,
    skip_invalid: bool, // ignore cases flagged invalid
    float_result: Option<(u32, u32)>,
}

fn ops_for(w: &str) -> Vec<Op> {
    let (e, m) = width(w).unwrap();
    let mut ops = Vec::new();
    let mut push = |name: String, body: String, nargs: usize, result_bits: u32, skip_invalid: bool, float_result: Option<(u32, u32)>| {
        ops.push(Op { name, wrapper: body, nargs, result_bits, skip_invalid, float_result })
    };
    for op in ["add", "sub", "mul", "div"] {
        push(
            format!("{}_{}", w, op),
            format!("fn {w}_{op}(a: {w}, b: {w}) -> {w} {{\n    r: {w} = {op} a, b\n    ret r\n}}\n"),
            2, bits_of(w), false, Some((e, m)),
        );
    }
    push(format!("{}_sqrt", w), format!("fn {w}_sqrt(a: {w}) -> {w} {{\n    r: {w} = sqrt a\n    ret r\n}}\n"), 1, bits_of(w), false, Some((e, m)));
    push(format!("{}_mulAdd", w), format!("fn {w}_mulAdd(a: {w}, b: {w}, c: {w}) -> {w} {{\n    r: {w} = fma a, b, c\n    ret r\n}}\n"), 3, bits_of(w), false, Some((e, m)));
    for (tf, cond) in [("eq", "eq"), ("le", "le"), ("lt", "lt")] {
        push(format!("{}_{}", w, tf), format!("fn {w}_{tf}(a: {w}, b: {w}) -> u1 {{\n    r: u1 = cmp.{cond} a, b\n    ret r\n}}\n"), 2, 1, false, None);
    }
    for other in ["f16", "f32", "f64"] {
        if other != w {
            push(format!("{}_to_{}", w, other), format!("fn {w}_to_{other}(a: {w}) -> {other} {{\n    r: {other} = conv a\n    ret r\n}}\n"), 1, bits_of(other), false, width(other));
        }
    }
    for (ti, ty) in [("i32", "i32"), ("i64", "i64"), ("ui32", "u32"), ("ui64", "u64")] {
        push(format!("{}_to_{}", w, ti), format!("fn {w}_to_{ti}(a: {w}) -> {ty} {{\n    r: {ty} = conv a\n    ret r\n}}\n"), 1, bits_of(ti), true, None);
        push(format!("{}_to_{}", ti, w), format!("fn {ti}_to_{w}(a: {ty}) -> {w} {{\n    r: {w} = conv a\n    ret r\n}}\n"), 1, bits_of(w), false, Some((e, m)));
    }
    ops
}

fn is_nan(v: u64, (e, m): (u32, u32)) -> bool {
    let exp = (v >> m) & ((1 << e) - 1);
    exp == (1 << e) - 1 && (v & ((1 << m) - 1)) != 0
}

pub struct Report {
    pub cases: usize,
    pub failed: usize,
    pub log: String,
}

/// every op of every width (or the ones named), the library instance
/// with the platform off and, where the platform has the instruction,
/// on; TestFloat's testing level 1 (about 46k cases per op)
pub fn run(only: &[String], level: usize, policy: ssa::Policy) -> Result<Report, String> {
    run_at(only, level, policy, 1)
}

/// `level` is the optimization level; `tf_level` TestFloat's (1 or 2)
pub fn run_at(only: &[String], level: usize, policy: ssa::Policy, tf_level: u8) -> Result<Report, String> {
    if !std::path::Path::new(GEN).exists() {
        return Err(format!("{} not found; build it with tools/get-testfloat.sh", GEN));
    }
    let enc = emit::Encoder::load("targets/arm64.encodings.json")?;
    let mut report = Report { cases: 0, failed: 0, log: String::new() };
    for w in ["f16", "f32", "f64"] {
        let ops: Vec<Op> = ops_for(w).into_iter().filter(|o| only.is_empty() || only.iter().any(|s| o.name.contains(s.as_str()))).collect();
        if ops.is_empty() {
            continue;
        }
        let src: String = ops.iter().map(|o| o.wrapper.as_str()).collect();
        let mut module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| e.to_string())?;
        ssa::verify(&module).map_err(|e| e.join("; "))?;
        opt::optimize(&mut module, level);
        let soft = emit::jit::JitCode::new(&emit::compile_with(&module, &enc, &platform::Platform::none())?)?;
        let hard = emit::jit::JitCode::new(&emit::compile_with(&module, &enc, &platform::Platform::arm64())?)?;
        let native = platform::Platform::arm64().natives(&module);
        for op in &ops {
            // conv to an integer truncates (round toward zero), which
            // TestFloat calls minMag
            let tf = tf_level.to_string();
            let mut gen_args = vec![op.name.as_str(), "-level", tf.as_str()];
            gen_args.push(if op.skip_invalid { "-rminMag" } else { ROUND_FLAGS[policy.round as usize] });
            let out = Command::new(GEN).args(&gen_args).output().map_err(|e| format!("{}: {}", GEN, e))?;
            if !out.status.success() {
                return Err(format!("testfloat_gen {}: {}", op.name, String::from_utf8_lossy(&out.stderr)));
            }
            let text = String::from_utf8_lossy(&out.stdout);
            // does the platform do this one natively? (its wrapper's callee)
            let f = module.funcs.iter().find(|f| f.name == op.name).unwrap();
            let callee = f.blocks.iter().flat_map(|b| b.insts.iter()).find_map(|i| match i {
                ssa::Inst::Call { callee, .. } => module.funcs.iter().find(|g| &g.name == callee),
                _ => None,
            });
            let on_platform = callee.map_or(false, |g| native.get(&g.name).is_some());
            let mask = if op.result_bits == 64 { u64::MAX } else { (1u64 << op.result_bits) - 1 };
            let (mut n, mut bad_soft, mut bad_hard) = (0, 0, 0);
            let mut shown = 0;
            for line in text.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() != op.nargs + 2 {
                    continue;
                }
                let args: Vec<i64> = f[..op.nargs].iter().map(|s| u64::from_str_radix(s, 16).unwrap() as i64).collect();
                let expected = u64::from_str_radix(f[op.nargs], 16).unwrap();
                let flags = u8::from_str_radix(f[op.nargs + 1], 16).unwrap();
                if op.skip_invalid && flags & 0x10 != 0 {
                    continue;
                }
                n += 1;
                let got = soft.call(&op.name, &args)? as u64 & mask;
                if got != expected {
                    bad_soft += 1;
                    if shown < 4 {
                        shown += 1;
                        let _ = writeln!(report.log, "  {} {}: expected {:x}, library {:x}", op.name, f[..op.nargs].join(" "), expected, got);
                    }
                }
                if on_platform {
                    let got = hard.call(&op.name, &args)? as u64 & mask;
                    // hardware NaNs carry payloads; any NaN is the right answer
                    let same_nan = op.float_result.map_or(false, |fr| is_nan(expected, fr) && is_nan(got, fr));
                    if got != expected && !same_nan {
                        bad_hard += 1;
                        if shown < 4 {
                            shown += 1;
                            let _ = writeln!(report.log, "  {} {}: expected {:x}, platform {:x}", op.name, f[..op.nargs].join(" "), expected, got);
                        }
                    }
                }
            }
            report.cases += n;
            report.failed += bad_soft + bad_hard;
            let _ = writeln!(
                report.log,
                "{:<14} {:>6} cases  library {}{}",
                op.name,
                n,
                if bad_soft == 0 { "ok".to_string() } else { format!("{} wrong", bad_soft) },
                if on_platform { format!("  platform {}", if bad_hard == 0 { "ok".to_string() } else { format!("{} wrong", bad_hard) }) } else { String::new() }
            );
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use crate::ssa::{Policy, Type};

    /// needs tools/get-testfloat.sh to have run: cargo test -- --ignored
    #[test]
    #[ignore]
    fn library_and_platform_match_testfloat_in_every_mode() {
        for round in 0..5 {
            let p = Policy::new(Type::I64).unwrap().with_round(&round.to_string()).unwrap();
            let r = super::run(&["f32_add".into(), "f32_mul".into(), "f16_to_f32".into(), "i32_to_f32".into()], crate::opt::MAX_LEVEL, p).unwrap();
            assert_eq!(r.failed, 0, "mode {}:\n{}", round, r.log);
        }
    }
}
