//! The regression suite runner.
//!
//! Test programs live in `suite/*.ssa` and are target-independent: they
//! express SSA semantics plus expected results. Expectations are embedded
//! as directives:
//!
//! ```text
//! ;! gcd 48 36 -> 12
//! ;! sum4 i32[10,20,30,40] -> 100
//! ;! divmod 17 5 -> 3, 2
//! ```
//!
//! Arguments are integers (decimal or 0x hex, negatives allowed) or arrays,
//! which are materialized as real buffers and passed as pointers. The same
//! suite evaluates every backend; only the compile/execute step differs:
//! the native backend JITs learned arm64 encodings in-process, the wasm
//! backend emits a module from learned wasm encodings and runs it in node
//! (arrays are copied into the module's linear memory, pointers become
//! offsets).

use crate::{emit, emit_rv, emit_wasm, lower, opt, softfloat, ssa};
use std::process::Command;

#[derive(Clone, Copy, PartialEq)]
pub enum Backend {
    Native,
    Wasm,
    /// bare-metal on qemu-system-riscv64: a driver generated in our own SSA
    /// prints results over the virt machine's UART and exits via its test
    /// finisher — qemu's decoder and semantics are the independent referee
    Riscv,
    /// the arm64 backend, but run bare-metal under qemu-system-aarch64
    /// instead of natively — an independent implementation of the
    /// architecture judging the same bytes the M-series CPU runs
    ArmQemu,
}

enum ArgSpec {
    Int(i64),
    Float(f64),
    ArrI64(Vec<i64>),
    ArrI32(Vec<i32>),
}

#[derive(Clone, Copy)]
enum ExpVal {
    Int(i64),
    Float(f64),
}

struct Case {
    func: String,
    args: Vec<ArgSpec>,
    raw_expected: Vec<ExpVal>, // as written in the directive
    expected: Vec<i64>,        // resolved comparison values (bits for floats)
    /// per-result normalization: sub-64 integer results read naturally —
    /// signed sign-extended, unsigned zero-extended — whatever the
    /// execution path handed back
    norms: Vec<Option<(u8, bool)>>,
    text: String,              // the directive as written, for reporting
}

fn parse_int(tok: &str) -> Result<i64, String> {
    let (neg, t) = match tok.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, tok),
    };
    let v = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map(|v| v as i64)
    } else {
        t.parse::<u64>().map(|v| v as i64)
    }
    .map_err(|_| format!("bad integer '{}'", tok))?;
    Ok(if neg { v.wrapping_neg() } else { v })
}

fn parse_case(line: &str) -> Result<Case, String> {
    let (call, expect) = line
        .split_once("->")
        .ok_or("directive needs '-> expected'")?;
    let raw_expected: Vec<ExpVal> = expect
        .split(',')
        .map(|v| {
            let t = v.trim();
            if t.contains('.') {
                t.parse::<f64>()
                    .map(ExpVal::Float)
                    .map_err(|_| format!("bad float '{}'", t))
            } else {
                parse_int(t).map(ExpVal::Int)
            }
        })
        .collect::<Result<_, _>>()?;
    let mut toks = Vec::new();
    let mut rest = call.trim();
    while !rest.is_empty() {
        let end = if rest.contains('[')
            && rest.find('[').unwrap() < rest.find(' ').unwrap_or(usize::MAX)
        {
            rest.find(']').ok_or("unclosed '['")? + 1
        } else {
            rest.find(' ').unwrap_or(rest.len())
        };
        toks.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    let func = toks
        .first()
        .ok_or("directive needs a function name")?
        .to_string();
    let mut args = Vec::new();
    for tok in &toks[1..] {
        if let Some(body) = tok.strip_prefix("i64[").and_then(|t| t.strip_suffix(']')) {
            let vals: Result<Vec<i64>, _> = body.split(',').map(|v| parse_int(v.trim())).collect();
            args.push(ArgSpec::ArrI64(vals?));
        } else if let Some(body) = tok.strip_prefix("i32[").and_then(|t| t.strip_suffix(']')) {
            let vals: Result<Vec<i32>, _> = body
                .split(',')
                .map(|v| parse_int(v.trim()).map(|x| x as i32))
                .collect();
            args.push(ArgSpec::ArrI32(vals?));
        } else if tok.contains('.') {
            args.push(ArgSpec::Float(
                tok.parse::<f64>().map_err(|_| format!("bad float '{}'", tok))?,
            ));
        } else {
            args.push(ArgSpec::Int(parse_int(tok)?));
        }
    }
    Ok(Case {
        func,
        args,
        raw_expected,
        expected: Vec::new(),
        norms: Vec::new(),
        text: line.trim().to_string(),
    })
}

/// Emit SSA turning result `r` (type rt) into a zero-extended u64 bit
/// pattern named `%b<i>`; appends the statements to `w` with 4-space
/// indent. Returns the value name to use.
fn convert_ret_to_bits(w: &mut String, rt: ssa::Type, r: &str, i: usize) -> String {
    use ssa::Type;
    match rt {
        Type::I(64) | Type::U(64) => r.to_string(),
        Type::F64 => {
            w.push_str(&format!("    %b{}: u64 = bitcast {}\n", i, r));
            format!("%b{}", i)
        }
        Type::F32 => {
            w.push_str(&format!("    %c{}: u32 = bitcast {}\n", i, r));
            w.push_str(&format!("    %b{}: u64 = ext %c{}\n", i, i));
            format!("%b{}", i)
        }
        // unsigned values extend zero-filled by their own type
        Type::U(_) => {
            w.push_str(&format!("    %b{}: u64 = ext {}\n", i, r));
            format!("%b{}", i)
        }
        // signed sub-64: reinterpret unsigned first so the extension is
        // zero-filled (the suite's zero-extended convention)
        Type::I(n) => {
            w.push_str(&format!("    %c{}: u{} = bitcast {}\n", i, n, r));
            w.push_str(&format!("    %b{}: u64 = ext %c{}\n", i, i));
            format!("%b{}", i)
        }
        _ => r.to_string(),
    }
}

pub struct Report {
    pub passed: usize,
    pub failed: usize,
    pub log: String,
}

impl Report {
    fn case(&mut self, ok: bool, file: &str, text: &str, note: &str) {
        if ok {
            self.passed += 1;
            self.log.push_str(&format!("  ok  {:<16} {}\n", file, text));
        } else {
            self.failed += 1;
            self.log
                .push_str(&format!("FAIL  {:<16} {}   {}\n", file, text, note));
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn run_dir(dir: &str, backend: Backend) -> Result<Report, String> {
    run_dir_at(dir, backend, opt::MAX_LEVEL, None, None, None, false)
}

/// Each target's default replacement policy for the abstract 'int' type:
/// the native 64-bit width on the register machines; i32 on wasm32, where
/// encodings are smaller and memory indices are 32-bit anyway.
fn default_int(backend: Backend) -> ssa::Type {
    match backend {
        Backend::Wasm => ssa::Type::I(32),
        _ => ssa::Type::I(64),
    }
}

/// Cases touching floats run through a generated wrapper: it fconsts the
/// arguments, calls the target, and returns every result as integer bits
/// (bitcast, f32 zero-extended) — so every execution path (native FFI,
/// node, both qemu drivers) stays integer-only, and float comparison is
/// exact-bits on all of them. Returns extra source to append, and fills
/// each case's comparison values.
fn prepare_cases(module: &ssa::Module, cases: &mut [Case]) -> Result<String, String> {
    let mut wrappers = String::new();
    for (n, case) in cases.iter_mut().enumerate() {
        let func = module
            .func(&case.func)
            .ok_or_else(|| format!("no function @{} for '{}'", case.func, case.text))?;
        let param_tys: Vec<ssa::Type> = func.params.iter().map(|&p| func.ty(p)).collect();
        let needs_wrap = param_tys.iter().any(|t| t.is_float())
            || func.rets.iter().any(|t| t.is_float())
            || case.args.iter().any(|a| matches!(a, ArgSpec::Float(_)));

        // comparison values: float expectations become IEEE bits
        case.expected = case
            .raw_expected
            .iter()
            .zip(&func.rets)
            .map(|(e, &rt)| match (e, rt.is_float()) {
                (ExpVal::Int(v), false) => Ok(*v),
                (ExpVal::Float(_), false) => {
                    Err(format!("float expectation for integer result in '{}'", case.text))
                }
                (e, true) => {
                    let x = match e {
                        ExpVal::Float(v) => *v,
                        ExpVal::Int(v) => *v as f64,
                    };
                        Ok(if rt == ssa::Type::F32 {
                        (x as f32).to_bits() as i64
                    } else {
                        x.to_bits() as i64
                    })
                }
            })
            .collect::<Result<_, _>>()?;
        case.norms = func
            .rets
            .iter()
            .map(|t| match t.width() {
                Some(n) if n < 64 => Some((n as u8, t.is_signed())),
                _ => None,
            })
            .collect();
        if case.expected.len() != case.raw_expected.len() {
            return Err(format!(
                "'{}' expects {} values but @{} returns {}",
                case.text,
                case.raw_expected.len(),
                case.func,
                func.rets.len()
            ));
        }

        if !needs_wrap {
            continue;
        }
        if case
            .args
            .iter()
            .any(|a| matches!(a, ArgSpec::ArrI64(_) | ArgSpec::ArrI32(_)))
        {
            return Err(format!("float cases with array args not supported: '{}'", case.text));
        }
        let mut w = format!("fn @__w{}() -> (", n);
        let ret_tys: Vec<&str> = func
            .rets
            .iter()
            .map(|t| if *t == ssa::Type::I(64) { "i64" } else { "u64" })
            .collect();
        w.push_str(&ret_tys.join(", "));
        w.push_str(") {
^entry:
");
        let mut argv = Vec::new();
        for (j, a) in case.args.iter().enumerate() {
            let pt = *param_tys
                .get(j)
                .ok_or_else(|| format!("too many args in '{}'", case.text))?;
            let name = format!("%a{}", j);
            match (a, pt.is_float()) {
                (ArgSpec::Int(v), false) => {
                    w.push_str(&format!("    {}: {} = iconst {}
", name, pt.name(), v))
                }
                (ArgSpec::Float(v), true) => {
                    w.push_str(&format!("    {}: {} = fconst {:?}
", name, pt.name(), v))
                }
                (ArgSpec::Int(v), true) => {
                    w.push_str(&format!("    {}: {} = fconst {:?}
", name, pt.name(), *v as f64))
                }
                _ => return Err(format!("argument {} type mismatch in '{}'", j, case.text)),
            }
            argv.push(name);
        }
        let rets: Vec<String> = (0..func.rets.len()).map(|i| format!("%r{}", i)).collect();
        let defs: Vec<String> = rets
            .iter()
            .zip(&func.rets)
            .map(|(r, t)| format!("{}: {}", r, t.name()))
            .collect();
        w.push_str(&format!(
            "    {} = call @{}({})
",
            defs.join(", "),
            case.func,
            argv.join(", ")
        ));
        let mut outs = Vec::new();
        for (i, (&rt, r)) in func.rets.iter().zip(&rets).enumerate() {
            outs.push(convert_ret_to_bits(&mut w, rt, r, i));
        }
        w.push_str(&format!("    ret {}
}}
", outs.join(", ")));
        wrappers.push_str(&w);
        case.func = format!("__w{}", n);
        case.args.clear();
    }
    Ok(wrappers)
}

pub fn run_dir_at(
    dir: &str,
    backend: Backend,
    level: usize,
    int_override: Option<ssa::Type>,
    float_override: Option<ssa::Type>,
    scalar_override: Option<ssa::ScalarPolicy>,
    soft: bool,
) -> Result<Report, String> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ssa"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no .ssa files in {}", dir));
    }

    let native_enc = match backend {
        Backend::Native | Backend::ArmQemu => {
            Some(emit::Encoder::load("targets/arm64.encodings.json")?)
        }
        _ => None,
    };
    let wasm_enc = match backend {
        Backend::Wasm => Some(emit_wasm::WEncoder::load("targets/wasm32.encodings.json")?),
        _ => None,
    };
    let rv_enc = match backend {
        Backend::Riscv => Some(emit::Encoder::load("targets/riscv64.encodings.json")?),
        _ => None,
    };
    let scratch = std::env::temp_dir().join("probe-suite");
    if backend != Backend::Native {
        std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    }
    if backend == Backend::Wasm {
        std::fs::write(scratch.join("driver.js"), include_str!("driver.js"))
            .map_err(|e| e.to_string())?;
    }

    let mut policy = ssa::Policy::new(
        int_override.unwrap_or(default_int(backend)),
        float_override.unwrap_or(ssa::Type::F64),
    )?;
    if let Some(sc) = scalar_override {
        policy.scalar = sc;
    }
    let mut report = Report {
        passed: 0,
        failed: 0,
        log: String::new(),
    };
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", name, e))?;
        let src = crate::scalar::link(&src, &policy);

        let mut cases = Vec::new();
        let mut bad_directive = None;
        for (ln, line) in src.lines().enumerate() {
            if let Some(d) = line.trim().strip_prefix(";!") {
                match parse_case(d) {
                    Ok(c) => cases.push(c),
                    Err(e) => bad_directive = Some(format!("line {}: {}", ln + 1, e)),
                }
            }
        }

        let prepared = (|| -> Result<(ssa::Module, String), String> {
            if let Some(e) = bad_directive {
                return Err(e);
            }
            let mut module = ssa::parse(&src).map_err(|e| e.to_string())?;
            ssa::resolve_types(&mut module, &policy);
            crate::scalar::scalarize(&mut module)?;
            ssa::verify(&module).map_err(|errs| errs.join("; "))?;
            let wrappers = prepare_cases(&module, &mut cases)?;
            let full_src = if wrappers.is_empty() {
                src.clone()
            } else {
                format!("{}\n{}", src, wrappers)
            };
            let mut module = ssa::parse(&full_src).map_err(|e| e.to_string())?;
            ssa::resolve_types(&mut module, &policy);
            crate::scalar::scalarize(&mut module)?;
            if soft {
                softfloat::soften(&mut module)?;
            }
            ssa::verify(&module).map_err(|errs| errs.join("; "))?;
            lower::lower(&mut module);
            opt::optimize(&mut module, level);
            ssa::verify(&module)
                .map_err(|errs| format!("after optimization: {}", errs.join("; ")))?;
            Ok((module, full_src))
        })();
        let (module, src) = match prepared {
            Ok(v) => v,
            Err(e) => {
                report.failed += cases.len().max(1);
                report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
                continue;
            }
        };

        match backend {
            Backend::Native => {
                run_native(&module, native_enc.as_ref().unwrap(), &cases, &name, &mut report)
            }
            Backend::Wasm => run_wasm(
                &module,
                wasm_enc.as_ref().unwrap(),
                &cases,
                &name,
                &scratch,
                &mut report,
            ),
            Backend::Riscv => run_riscv(
                &module,
                &policy,
                &src,
                rv_enc.as_ref().unwrap(),
                &cases,
                &name,
                &scratch,
                level,
                soft,
                &mut report,
            ),
            Backend::ArmQemu => run_arm_qemu(
                &module,
                &policy,
                &src,
                native_enc.as_ref().unwrap(),
                &cases,
                &name,
                &scratch,
                level,
                soft,
                &mut report,
            ),
        }
    }
    report.log.push_str(&format!(
        "\n{}/{} cases passed\n",
        report.passed,
        report.passed + report.failed
    ));
    Ok(report)
}

// ---------------------------------------------------------------------------
// Native (arm64 JIT) execution

fn run_native(
    module: &ssa::Module,
    enc: &emit::Encoder,
    cases: &[Case],
    name: &str,
    report: &mut Report,
) {
    let jit = match emit::compile(module, enc).and_then(|c| emit::jit::JitCode::new(&c)) {
        Ok(j) => j,
        Err(e) => {
            report.failed += cases.len().max(1);
            report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
            return;
        }
    };
    for case in cases {
        // materialize array args as live buffers, kept alive through the call
        let mut bufs64: Vec<Vec<i64>> = Vec::new();
        let mut bufs32: Vec<Vec<i32>> = Vec::new();
        let mut argv = Vec::new();
        for a in &case.args {
            match a {
                ArgSpec::Int(v) => argv.push(*v),
                ArgSpec::Float(_) => unreachable!("float args are wrapped"),
                ArgSpec::ArrI64(vals) => {
                    bufs64.push(vals.clone());
                    argv.push(bufs64.last().unwrap().as_ptr() as i64);
                }
                ArgSpec::ArrI32(vals) => {
                    bufs32.push(vals.clone());
                    argv.push(bufs32.last().unwrap().as_ptr() as i64);
                }
            }
        }
        let got: Result<Vec<i64>, String> = match case.expected.len() {
            1 => jit.call(&case.func, &argv).map(|v| vec![v]),
            2 => jit.call2(&case.func, &argv).map(|(a, b)| vec![a, b]),
            n => Err(format!("{} expected values not supported by the runner", n)),
        };
        finish_case(report, name, case, got);
    }
}

// ---------------------------------------------------------------------------
// Wasm (node) execution

fn run_wasm(
    module: &ssa::Module,
    enc: &emit_wasm::WEncoder,
    cases: &[Case],
    name: &str,
    scratch: &std::path::Path,
    report: &mut Report,
) {
    let wasm = match emit_wasm::compile(module, enc) {
        Ok(b) => b,
        Err(e) => {
            report.failed += cases.len().max(1);
            report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
            return;
        }
    };
    let wasm_path = scratch.join(format!("{}.wasm", name));
    if let Err(e) = std::fs::write(&wasm_path, &wasm) {
        report.failed += cases.len().max(1);
        report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
        return;
    }

    // build the case spec JSON, typing each argument from the SSA signature
    let mut spec = String::from("{\"cases\":[");
    for (i, case) in cases.iter().enumerate() {
        if i > 0 {
            spec.push(',');
        }
        let func = match module.func(&case.func) {
            Some(f) => f,
            None => {
                spec.push_str(&format!("{{\"func\":\"{}\",\"args\":[],\"rets\":[]}}", case.func));
                continue;
            }
        };
        spec.push_str(&format!("{{\"func\":\"{}\",\"args\":[", case.func));
        for (j, a) in case.args.iter().enumerate() {
            if j > 0 {
                spec.push(',');
            }
            let pty = func.params.get(j).map(|&p| func.ty(p));
            match a {
                ArgSpec::Int(v) => {
                    let t = if pty.map(|t| t.width()) == Some(Some(64)) || pty == Some(ssa::Type::Ptr)
                    {
                        "i64".to_string()
                    } else {
                        "i32".to_string()
                    };
                    spec.push_str(&format!("{{\"t\":\"{}\",\"v\":\"{}\"}}", t, v));
                }
                ArgSpec::Float(_) => unreachable!("float args are wrapped"),
                ArgSpec::ArrI64(vals) => {
                    let vs: Vec<String> = vals.iter().map(|v| format!("\"{}\"", v)).collect();
                    spec.push_str(&format!("{{\"t\":\"ptr\",\"a64\":[{}]}}", vs.join(",")));
                }
                ArgSpec::ArrI32(vals) => {
                    let vs: Vec<String> = vals.iter().map(|v| format!("\"{}\"", v)).collect();
                    spec.push_str(&format!("{{\"t\":\"ptr\",\"a32\":[{}]}}", vs.join(",")));
                }
            }
        }
        spec.push_str("],\"rets\":[");
        for (j, &t) in func.rets.iter().enumerate() {
            if j > 0 {
                spec.push(',');
            }
            spec.push_str(if t.width() == Some(64) {
                "\"i64\""
            } else {
                "\"i32\""
            });
        }
        spec.push_str("]}");
    }
    spec.push_str("]}");

    let out = Command::new("node")
        .arg(scratch.join("driver.js"))
        .arg(&wasm_path)
        .arg(&spec)
        .output();
    let out = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => {
            report.failed += cases.len().max(1);
            report.log.push_str(&format!(
                "FAIL  {:<16} node: {}\n",
                name,
                String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("")
            ));
            return;
        }
        Err(e) => {
            report.failed += cases.len().max(1);
            report
                .log
                .push_str(&format!("FAIL  {:<16} spawn node: {}\n", name, e));
            return;
        }
    };
    let lines: Vec<&str> = out.lines().collect();
    for (i, case) in cases.iter().enumerate() {
        let got: Result<Vec<i64>, String> = match lines.get(i) {
            Some(l) if l.starts_with("trap:") => Err(l.to_string()),
            Some(l) => l
                .split(',')
                .map(|v| {
                    v.trim()
                        .parse::<i64>()
                        .map_err(|_| format!("bad output '{}'", v))
                })
                .collect(),
            None => Err("no output from node".into()),
        };
        finish_case(report, name, case, got);
    }
}

// ---------------------------------------------------------------------------
// RISC-V (bare-metal qemu) execution

const RV_FINISHER: u64 = 0x0010_0000; // riscv virt: store 0x5555 here to exit
const RV_UART: u64 = 0x1000_0000; // riscv virt: 16550
const RV_HEAP: u64 = 0x8040_0000;
const ARM_UART: u64 = 0x0900_0000; // aarch64 virt: PL011
const ARM_HEAP: u64 = 0x4140_0000;

/// The runtime harness, generated in our own SSA: helpers to print a char
/// (a 32-bit UART store; on both virt machines' UARTs the high bytes land
/// in benign registers) and a value as 16 hex digits. Only the UART
/// address is target-specific; the hex printer is pure SSA.
fn helpers(uart: u64) -> String {
    format!(
        r"
fn @__pch(%c: u64) {{
^entry:
    %u: ptr = iconst {}
    %c32: i32 = trunc %c
    store %c32, %u
    ret
}}
",
        uart
    ) + PHEX
}

const PHEX: &str = r"
fn @__phex(%v: u64) {
^entry:
    %sh0: u64 = iconst 60
    jmp ^loop(%sh0)
^loop(%sh: u64):
    %t: u64 = shr %v, %sh
    %m: u64 = iconst 15
    %n: u64 = and %t, %m
    %nine: u64 = iconst 9
    %big: u1 = icmp.gt %n, %nine
    %bigi: u64 = ext %big
    %gap: u64 = iconst 39
    %adj: u64 = imul %bigi, %gap
    %z: u64 = iconst 48
    %c1: u64 = iadd %n, %z
    %c: u64 = iadd %c1, %adj
    call @__pch(%c)
    %zero: u64 = iconst 0
    %done: u1 = icmp.eq %sh, %zero
    %four: u64 = iconst 4
    %sh2: u64 = isub %sh, %four
    br %done, ^exit, ^loop(%sh2)
^exit:
    ret
}
";

/// Generate @__start: run every case, print each result as hex, then run
/// the target-specific exit statements.
fn gen_driver(
    module: &ssa::Module,
    cases: &[Case],
    heap_base: u64,
    exit_ssa: &str,
) -> Result<String, String> {
    let mut s = String::from("fn @__start() {\n^entry:\n");
    let n = std::cell::Cell::new(0u32);
    let mut heap = heap_base;
    let tmp_name = || {
        n.set(n.get() + 1);
        format!("%t{}", n.get())
    };
    let tmp = |s: &mut String, ty: &str, init: String| -> String {
        let name = tmp_name();
        s.push_str(&format!("    {}: {} = {}\n", name, ty, init));
        name
    };
    for case in cases {
        let func = module
            .func(&case.func)
            .ok_or_else(|| format!("no function @{} for directive '{}'", case.func, case.text))?;
        if func.rets.is_empty() {
            return Err(format!("@{} returns nothing; directives need results", case.func));
        }
        // materialize arguments
        let mut argv = Vec::new();
        for (j, a) in case.args.iter().enumerate() {
            let pty = func
                .params
                .get(j)
                .map(|&p| func.ty(p))
                .ok_or_else(|| format!("too many args in '{}'", case.text))?;
            match a {
                ArgSpec::Int(v) => {
                    let ty = pty.name();
                    argv.push(tmp(&mut s, &ty, format!("iconst {}", v)));
                }
                ArgSpec::Float(_) => unreachable!("float args are wrapped"),
                ArgSpec::ArrI64(vals) => {
                    heap = (heap + 7) & !7;
                    let base = heap;
                    for (k, v) in vals.iter().enumerate() {
                        let d = tmp(&mut s, "i64", format!("iconst {}", v));
                        let p = tmp(&mut s, "ptr", format!("iconst {}", base + 8 * k as u64));
                        s.push_str(&format!("    store {}, {}\n", d, p));
                    }
                    heap += 8 * vals.len() as u64;
                    argv.push(tmp(&mut s, "ptr", format!("iconst {}", base)));
                }
                ArgSpec::ArrI32(vals) => {
                    heap = (heap + 3) & !3;
                    let base = heap;
                    for (k, v) in vals.iter().enumerate() {
                        let d = tmp(&mut s, "i32", format!("iconst {}", v));
                        let p = tmp(&mut s, "ptr", format!("iconst {}", base + 4 * k as u64));
                        s.push_str(&format!("    store {}, {}\n", d, p));
                    }
                    heap += 4 * vals.len() as u64;
                    argv.push(tmp(&mut s, "ptr", format!("iconst {}", base)));
                }
            }
        }
        // call, binding every result
        let mut rets = Vec::new();
        for &rt in &func.rets {
            let r = tmp_name();
            rets.push((r, rt));
        }
        let defs: Vec<String> = rets
            .iter()
            .map(|(r, t)| format!("{}: {}", r, t.name()))
            .collect();
        s.push_str(&format!(
            "    {} = call @{}({})\n",
            defs.join(", "),
            case.func,
            argv.join(", ")
        ));
        // print results: 16 hex digits each, space-separated, newline after
        for (i, (r, rt)) in rets.iter().enumerate() {
            if i > 0 {
                let sp = tmp(&mut s, "u64", "iconst 32".into());
                s.push_str(&format!("    call @__pch({})\n", sp));
            }
            let printable = if *rt == ssa::Type::U(64) {
                r.clone()
            } else if *rt == ssa::Type::I(64) {
                let b = tmp_name();
                s.push_str(&format!("    {}: u64 = bitcast {}\n", b, r));
                b
            } else {
                n.set(n.get() + 1);
                let uniq = 100000 + n.get() as usize;
                let mut conv = String::new();
                let name = convert_ret_to_bits(&mut conv, *rt, r, uniq);
                s.push_str(&conv);
                name
            };
            s.push_str(&format!("    call @__phex({})\n", printable));
        }
        let nl = tmp(&mut s, "u64", "iconst 10".into());
        s.push_str(&format!("    call @__pch({})\n", nl));
    }
    s.push_str(exit_ssa);
    s.push_str("    ret\n}\n");
    Ok(s)
}

/// Run a qemu child with a hard deadline (a wrong branch in emitted code
/// means an infinite loop); returns captured stdout.
fn exec_qemu(mut cmd: Command, secs: u64) -> Result<String, String> {
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut child = child.map_err(|e| format!("spawn qemu: {}", e))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(e) => return Err(format!("wait for qemu: {}", e)),
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout.read_to_string(&mut out);
    }
    let _ = child.wait();
    if timed_out {
        return Err("qemu timed out (runaway emitted code?)".into());
    }
    Ok(out)
}

/// Compare one hex-groups-per-line qemu output against the cases.
fn check_hex_lines(out: &str, cases: &[Case], name: &str, report: &mut Report) {
    let lines: Vec<&str> = out.lines().collect();
    for (i, case) in cases.iter().enumerate() {
        let got: Result<Vec<i64>, String> = match lines.get(i) {
            Some(l) => l
                .split_whitespace()
                .map(|h| {
                    u64::from_str_radix(h, 16)
                        .map(|v| v as i64)
                        .map_err(|_| format!("bad output '{}'", h))
                })
                .collect(),
            None => Err("no output from qemu".into()),
        };
        finish_case(report, name, case, got);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_riscv(
    module: &ssa::Module,
    policy: &ssa::Policy,
    src: &str,
    enc: &emit::Encoder,
    cases: &[Case],
    name: &str,
    scratch: &std::path::Path,
    level: usize,
    soft: bool,
    report: &mut Report,
) {
    let fail_all = |report: &mut Report, msg: String| {
        report.failed += cases.len().max(1);
        report.log.push_str(&format!("FAIL  {:<16} {}\n", name, msg));
    };

    let prepared = (|| -> Result<Vec<u8>, String> {
        let exit_ssa = format!(
            "    %__f1: i32 = iconst 21845\n    %__f2: ptr = iconst {}\n    store %__f1, %__f2\n",
            RV_FINISHER
        );
        let driver = gen_driver(module, cases, RV_HEAP, &exit_ssa)?;
        let full = format!("{}\n{}\n{}", driver, helpers(RV_UART), src);
        let mut m2 = ssa::parse(&full).map_err(|e| format!("driver: {}", e))?;
        ssa::resolve_types(&mut m2, policy);
        crate::scalar::scalarize(&mut m2)?;
        if soft {
            softfloat::soften(&mut m2)?;
        }
        // the driver is generated against the lowered module, so lower
        // before verifying the combined source
        lower::lower(&mut m2);
        ssa::verify(&m2).map_err(|e| format!("driver: {}", e.join("; ")))?;
        opt::optimize(&mut m2, level);
        let compiled = emit_rv::compile(&m2, enc)?;
        // preamble: sp = 0x80800000 (mid-RAM), then fall into @__start
        let mut bin = Vec::new();
        const ADDI: &str = "addi {r}, {r}, {i -2048..2047}";
        const SLLI: &str = "slli {r}, {r}, {i 0..63}";
        for (t, v) in [
            (ADDI, [2i64, 0, 0x80]),
            (SLLI, [2, 2, 8]),
            (ADDI, [2, 2, 0x80]),
            (SLLI, [2, 2, 16]),
            // FPUs power up off: set mstatus.FS (bits 13-14) via t0
            (ADDI, [5, 0, 0x600]),
            (SLLI, [5, 5, 4]),
        ] {
            bin.extend(enc.encode(t, &v)?.to_le_bytes());
        }
        bin.extend(
            enc.encode("csrrs {r}, mstatus, {r}", &[0, 5])?.to_le_bytes(),
        );
        bin.extend(&compiled.code);
        Ok(bin)
    })();
    let bin = match prepared {
        Ok(b) => b,
        Err(e) => return fail_all(report, e),
    };
    let bin_path = scratch.join(format!("{}.rv.bin", name));
    if let Err(e) = std::fs::write(&bin_path, &bin) {
        return fail_all(report, e.to_string());
    }

    let mut cmd = Command::new("qemu-system-riscv64");
    cmd.args(["-machine", "virt", "-bios", "none", "-nographic", "-m", "128M"])
        .arg("-device")
        .arg(format!("loader,file={},addr=0x80000000", bin_path.display()));
    match exec_qemu(cmd, 30) {
        Ok(out) => check_hex_lines(&out, cases, name, report),
        Err(e) => fail_all(report, e),
    }
}

// ---------------------------------------------------------------------------
// arm64 under qemu (bare-metal aarch64 virt machine)

#[allow(clippy::too_many_arguments)]
fn run_arm_qemu(
    module: &ssa::Module,
    policy: &ssa::Policy,
    src: &str,
    enc: &emit::Encoder,
    cases: &[Case],
    name: &str,
    scratch: &std::path::Path,
    level: usize,
    soft: bool,
    report: &mut Report,
) {
    let fail_all = |report: &mut Report, msg: String| {
        report.failed += cases.len().max(1);
        report.log.push_str(&format!("FAIL  {:<16} {}\n", name, msg));
    };

    let prepared = (|| -> Result<Vec<u8>, String> {
        // exit through a stub whose body is patched below into a PSCI
        // SYSTEM_OFF hypervisor call (x0 = 0x84000008; hvc #0)
        let driver = gen_driver(module, cases, ARM_HEAP, "    call @__qemu_exit()\n")?;
        let stub = "fn @__qemu_exit() {\n^entry:\n    ret\n}\n";
        let full = format!("{}\n{}\n{}\n{}", driver, helpers(ARM_UART), stub, src);
        let mut m2 = ssa::parse(&full).map_err(|e| format!("driver: {}", e))?;
        ssa::resolve_types(&mut m2, policy);
        crate::scalar::scalarize(&mut m2)?;
        if soft {
            softfloat::soften(&mut m2)?;
        }
        // the driver is generated against the lowered module, so lower
        // before verifying the combined source
        lower::lower(&mut m2);
        ssa::verify(&m2).map_err(|e| format!("driver: {}", e.join("; ")))?;
        opt::optimize(&mut m2, level);
        let compiled = emit::compile(&m2, enc)?;
        // preamble: sp = 0x41000000 via x29 (sp itself isn't a movz target)
        let mut bin = Vec::new();
        bin.extend(
            enc.encode("movz {x}, #{i 0..65535}, lsl #16", &[29, 0x4100])?
                .to_le_bytes(),
        );
        bin.extend(enc.encode("mov sp, x29", &[])?.to_le_bytes());
        // FPUs power up off: CPACR_EL1.FPEN = 0b11 (bits 20-21), then isb
        bin.extend(
            enc.encode("movz {x}, #{i 0..65535}, lsl #16", &[9, 0x30])?
                .to_le_bytes(),
        );
        bin.extend(enc.encode("msr cpacr_el1, {x}", &[9])?.to_le_bytes());
        bin.extend(enc.encode("isb", &[])?.to_le_bytes());
        let preamble = bin.len();
        bin.extend(&compiled.code);
        // patch the exit stub
        let off = preamble
            + compiled
                .funcs
                .get("__qemu_exit")
                .copied()
                .ok_or("no __qemu_exit stub")?;
        let words = [
            enc.encode("movz {x}, #{i 0..65535}", &[0, 0x0008])?,
            enc.encode("movk {x}, #{i 0..65535}, lsl #16", &[0, 0x8400])?,
            enc.encode("hvc #{i 0..65535}", &[0])?,
        ];
        for (i, w) in words.iter().enumerate() {
            bin[off + 4 * i..off + 4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        Ok(bin)
    })();
    let bin = match prepared {
        Ok(b) => b,
        Err(e) => return fail_all(report, e),
    };
    let bin_path = scratch.join(format!("{}.a64.bin", name));
    if let Err(e) = std::fs::write(&bin_path, &bin) {
        return fail_all(report, e.to_string());
    }

    let mut cmd = Command::new("qemu-system-aarch64");
    cmd.args(["-machine", "virt", "-cpu", "cortex-a57", "-nographic", "-m", "128M"])
        .arg("-device")
        .arg(format!(
            "loader,file={},addr=0x40200000,cpu-num=0",
            bin_path.display()
        ));
    match exec_qemu(cmd, 30) {
        Ok(out) => check_hex_lines(&out, cases, name, report),
        Err(e) => fail_all(report, e),
    }
}

fn finish_case(report: &mut Report, name: &str, case: &Case, got: Result<Vec<i64>, String>) {
    let got = got.map(|vs| {
        vs.iter()
            .enumerate()
            .map(|(i, &v)| match case.norms.get(i).copied().flatten() {
                Some((n, true)) => (v << (64 - n)) >> (64 - n),
                Some((n, false)) => (v as u64 & ((1u128 << n) - 1) as u64) as i64,
                None => v,
            })
            .collect::<Vec<i64>>()
    });
    match got {
        Ok(got) if got == case.expected => report.case(true, name, &case.text, ""),
        Ok(got) => {
            let gs: Vec<String> = got.iter().map(|v| v.to_string()).collect();
            report.case(
                false,
                name,
                &case.text,
                &format!("(got {})", gs.join(", ")),
            );
        }
        Err(e) => report.case(false, name, &case.text, &format!("({})", e)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn regression_suite_native() {
        let report = super::run_dir("suite", super::Backend::Native).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }

    #[test]
    fn regression_suite_every_level() {
        // any prefix of the pass pipeline must be a correct stopping point
        for level in 0..=crate::opt::MAX_LEVEL {
            let report =
                super::run_dir_at("suite", super::Backend::Native, level, None, None, None, false)
                    .expect("suite runs");
            assert_eq!(report.failed, 0, "at level {}:\n{}", level, report.log);
        }
    }

    #[test]
    fn regression_suite_all_policies() {
        // abstract-typed programs must behave identically under every
        // replacement policy (concrete-typed programs are unaffected)
        use crate::ssa::Type;
        for (int, float) in [
            (Type::I(32), Type::F32),
            (Type::I(32), Type::F64),
            (Type::I(64), Type::F32),
            (Type::I(64), Type::F64),
        ] {
            let report = super::run_dir_at(
                "suite",
                super::Backend::Native,
                crate::opt::MAX_LEVEL,
                Some(int),
                Some(float),
                None,
                false,
            )
            .expect("suite runs");
            assert_eq!(
                report.failed,
                0,
                "with int={} float={}:\n{}",
                int.name(),
                float.name(),
                report.log
            );
        }
    }

    #[test]
    fn regression_suite_scalar_rat() {
        // the same abstract-scalar programs, computed in exact rational
        // arithmetic instead of floating point
        let report = super::run_dir_at(
            "suite",
            super::Backend::Native,
            crate::opt::MAX_LEVEL,
            None,
            None,
            Some(crate::ssa::ScalarPolicy::Rat),
            false,
        )
        .expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }

    #[test]
    fn regression_suite_wasm() {
        let report = super::run_dir("suite", super::Backend::Wasm).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }

    #[test]
    fn regression_suite_riscv() {
        let report = super::run_dir("suite", super::Backend::Riscv).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }

    #[test]
    fn regression_suite_softfloat() {
        // the SSA softfloat runtime against hardware-derived expectations:
        // identical results, bit for bit, with no FPU instructions emitted
        // for user code
        let report = super::run_dir_at(
            "suite",
            super::Backend::Native,
            crate::opt::MAX_LEVEL,
            None,
            None,
            None,
            true,
        )
        .expect("suite runs");
        assert_eq!(report.failed, 0, "softfloat:\n{}", report.log);
    }

    #[test]
    fn regression_suite_arm_qemu() {
        let report = super::run_dir("suite", super::Backend::ArmQemu).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }
}
