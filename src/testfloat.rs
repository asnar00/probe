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
    /// on the GPU: results wrong only because a denormal was flushed —
    /// what the hardware does, not what the code got wrong
    pub flushed: usize,
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
    let mut report = Report { cases: 0, failed: 0, flushed: 0, log: String::new() };
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

/// the same vectors on the GPU: per op, a kernel whose thread `id`
/// takes case `id`'s operands from an argument table in the program's
/// memory and puts its result after the table — one dispatch of as many
/// threads as TestFloat has cases. Compared is what runs there: the
/// platform's instruction where `targets/air.platform` has one, the
/// library otherwise (as `probe test air` runs it)
pub fn run_air(only: &[String], level: usize, policy: ssa::Policy, tf_level: u8) -> Result<Report, String> {
    if !std::path::Path::new(GEN).exists() {
        return Err(format!("{} not found; build it with tools/get-testfloat.sh", GEN));
    }
    let platform = platform::Platform::load("air")?;
    let policy = platform.adjust(policy);
    let scratch = std::env::temp_dir().join("probe-testfloat");
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let mut report = Report { cases: 0, failed: 0, flushed: 0, log: String::new() };
    for w in ["f16", "f32", "f64"] {
        let ops: Vec<Op> = ops_for(w).into_iter().filter(|o| only.is_empty() || only.iter().any(|s| o.name.contains(s.as_str()))).collect();
        for op in &ops {
            let tf = tf_level.to_string();
            let mut gen_args = vec![op.name.as_str(), "-level", tf.as_str()];
            gen_args.push(if op.skip_invalid { "-rminMag" } else { ROUND_FLAGS[policy.round as usize] });
            let out = Command::new(GEN).args(&gen_args).output().map_err(|e| format!("{}: {}", GEN, e))?;
            if !out.status.success() {
                return Err(format!("testfloat_gen {}: {}", op.name, String::from_utf8_lossy(&out.stderr)));
            }
            let text = String::from_utf8_lossy(&out.stdout);
            let mut cases: Vec<(Vec<u64>, u64)> = Vec::new();
            for line in text.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() != op.nargs + 2 {
                    continue;
                }
                let flags = u8::from_str_radix(f[op.nargs + 1], 16).unwrap();
                if op.skip_invalid && flags & 0x10 != 0 {
                    continue;
                }
                cases.push((f[..op.nargs].iter().map(|s| u64::from_str_radix(s, 16).unwrap()).collect(), u64::from_str_radix(f[op.nargs], 16).unwrap()));
            }
            let n = cases.len();
            // the kernel: operands as words at area + id * nargs * 8,
            // the result as a word at area + n * nargs * 8 + id * 8
            let (e, m) = width(w).unwrap();
            let wbits = e + m + 1;
            let arg_ty = |i: usize| -> &str {
                // the wrapper's parameter types, in its own words
                let sig = op.wrapper.lines().next().unwrap();
                let inside = &sig[sig.find('(').unwrap() + 1..sig.find(')').unwrap()];
                inside.split(',').nth(i).unwrap().split(':').nth(1).unwrap().trim()
            };
            let mut k = String::from("fn __kernel(mem: ptr, area: ptr, id: i64) {\n");
            let _ = writeln!(k, "    base: i64 = mul id, {}", op.nargs * 8);
            k.push_str("    p: ptr = ptradd area, base\n");
            let mut names = Vec::new();
            for i in 0..op.nargs {
                let t = arg_ty(i);
                let _ = writeln!(k, "    w{i}: u64 = load p, {}", i * 8);
                let bits = match t { "f16" => 16, "f32" | "i32" | "u32" => 32, _ => 64 };
                if t.starts_with('f') {
                    if bits < 64 {
                        let _ = writeln!(k, "    x{i}: u{bits} = conv w{i}\n    a{i}: {t} = cast x{i}");
                    } else {
                        let _ = writeln!(k, "    a{i}: {t} = cast w{i}");
                    }
                } else if bits < 64 {
                    let _ = writeln!(k, "    a{i}: {t} = conv w{i}");
                } else {
                    let _ = writeln!(k, "    a{i}: {t} = cast w{i}");
                }
                names.push(format!("a{i}"));
            }
            let rt = op.wrapper.lines().next().unwrap().rsplit("->").next().unwrap().trim().trim_end_matches('{').trim().to_string();
            let _ = writeln!(k, "    r: {} = {}({})", rt, op.name, names.join(", "));
            if rt.starts_with('f') {
                let rb = if rt == "f16" { 16 } else if rt == "f32" { 32 } else { 64 };
                let _ = writeln!(k, "    rb: u{rb} = cast r");
                k.push_str(if rb < 64 { "    ru: u64 = conv rb\n" } else { "    ru: u64 = cast rb\n" });
            } else if rt == "u64" {
                k.push_str("    ru: u64 = cast r\n");
            } else if rt == "i64" {
                k.push_str("    ru: u64 = cast r\n");
            } else {
                k.push_str("    ru: u64 = conv r\n");
            }
            let _ = writeln!(k, "    q: ptr = ptradd area, {}\n    store ru, q, id, 8\n    ret\n}}", n * op.nargs * 8);
            let _ = wbits;
            let src = format!("{}\n{}", op.wrapper, k);
            let mut module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| format!("{}: {}", op.name, e))?;
            ssa::resolve_types(&mut module, &policy);
            ssa::verify(&module).map_err(|e| format!("{}: {}", op.name, e.join("; ")))?;
            opt::optimize(&mut module, level);
            let native = platform.natives(&module);
            let f = module.funcs.iter().find(|f| f.name == op.name).unwrap();
            let on_platform = f.blocks.iter().flat_map(|b| b.insts.iter()).any(|i| matches!(i, ssa::Inst::Call { callee, .. } if native.get(callee).is_some()));
            let c = crate::emit_air::compile_with(&module, &platform)?;
            if !c.has_kernel {
                return Err(format!("{}: the kernel was left out: {:?}", op.name, c.skipped));
            }
            let data_size = (c.layout.data.len() as u64 + 15) & !15;
            let area_off = (data_size + n as u64 * c.layout.slab + 15) & !15;
            let area_len = (n * op.nargs * 8 + n * 8) as u64;
            let size = area_off + area_len + 16;
            let mut image = c.layout.data.clone();
            image.resize(area_off as usize, 0);
            for (args, _) in &cases {
                for a in args {
                    image.extend_from_slice(&a.to_le_bytes());
                }
            }
            let lib = scratch.join(format!("{}.metallib", op.name));
            let mem = scratch.join(format!("{}.mem", op.name));
            std::fs::write(&lib, &c.metallib).map_err(|e| e.to_string())?;
            std::fs::write(&mem, &image).map_err(|e| e.to_string())?;
            let out = Command::new("python3")
                .args(["tools/driver_metal.py", "--batch"])
                .arg(&lib)
                .arg(&mem)
                .args([size.to_string(), area_off.to_string(), area_len.to_string(), n.to_string()])
                .output()
                .map_err(|e| format!("python3: {}", e))?;
            if !out.status.success() {
                return Err(format!("{}: the driver: {}", op.name, String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
            }
            let results = &out.stdout[n * op.nargs * 8..];
            let mask = if op.result_bits == 64 { u64::MAX } else { (1u64 << op.result_bits) - 1 };
            let (mut bad, mut shown, mut denormal) = (0, 0, 0);
            let is_denormal = |v: u64, (e, m): (u32, u32)| -> bool { (v >> m) & ((1 << e) - 1) == 0 && v & ((1 << m) - 1) != 0 };
            for (i, (args, expected)) in cases.iter().enumerate() {
                let got = u64::from_le_bytes(results[i * 8..i * 8 + 8].try_into().unwrap()) & mask;
                let same_nan = op.float_result.map_or(false, |fr| is_nan(*expected, fr) && is_nan(got, fr));
                if got != *expected && !(on_platform && same_nan) {
                    bad += 1;
                    // a denormal operand or result: what a GPU flushes
                    let in_denormal = op.name.starts_with(w) && args.iter().any(|&a| is_denormal(a, (e, m)));
                    // ... or the least normal, which a flushed intermediate loses too
                    let out_denormal = op.float_result.is_some_and(|fr| is_denormal(*expected, fr) || (*expected >> fr.1) & ((1 << fr.0) - 1) == 1);
                    if in_denormal || out_denormal {
                        denormal += 1;
                        continue; // the others are the ones to show
                    }
                    if shown < 4 {
                        shown += 1;
                        let _ = writeln!(report.log, "  {} {}: expected {:x}, gpu {:x}", op.name, args.iter().map(|a| format!("{:x}", a)).collect::<Vec<_>>().join(" "), expected, got);
                    }
                }
            }
            report.cases += n;
            report.failed += bad - denormal;
            report.flushed += denormal;
            let _ = writeln!(
                report.log,
                "{:<14} {:>8} cases  gpu {} ({})",
                op.name,
                n,
                if bad == 0 { "ok".to_string() } else if denormal == bad { format!("{} denormals flushed", bad) } else { format!("{} wrong, {} denormals flushed", bad - denormal, denormal) },
                if on_platform { "the platform's instruction" } else { "the library" }
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
