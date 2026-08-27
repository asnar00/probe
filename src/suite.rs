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

use crate::{emit, emit_rv, emit_wasm, opt, ssa};
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
    /// an array argument: element width in bytes, values (canonical)
    Arr { bytes: usize, vals: Vec<i64> },
}

impl ArgSpec {
    /// the array's bytes, little-endian
    fn to_bytes(&self) -> Vec<u8> {
        match self {
            ArgSpec::Int(_) => Vec::new(),
            ArgSpec::Arr { bytes, vals } => vals
                .iter()
                .flat_map(|v| v.to_le_bytes()[..*bytes].to_vec())
                .collect(),
        }
    }
}

struct Case {
    func: String,
    args: Vec<ArgSpec>,
    expected: Vec<i64>, // one entry per return value
    text: String,       // the directive as written, for reporting
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
    let expected: Vec<i64> = expect
        .split(',')
        .map(|v| parse_int(v.trim()))
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
        // typed arrays: i8[..] u8[..] i16[..] u16[..] i32[..] u32[..] i64[..] u64[..]
        let arr = tok.split_once('[').and_then(|(ty, rest)| {
            let body = rest.strip_suffix(']')?;
            let bytes = match ty {
                "i8" | "u8" => 1,
                "i16" | "u16" => 2,
                "i32" | "u32" => 4,
                "i64" | "u64" => 8,
                _ => return None,
            };
            Some((bytes, body))
        });
        if let Some((bytes, body)) = arr {
            // an element may be `v*n`: v repeated n times (u8[0*256])
            let mut vals = Vec::new();
            for v in body.split(',') {
                match v.trim().split_once('*') {
                    Some((x, n)) => {
                        let (x, n) = (parse_int(x.trim())?, parse_int(n.trim())?);
                        vals.extend(std::iter::repeat_n(x, n.max(0) as usize));
                    }
                    None => vals.push(parse_int(v.trim())?),
                }
            }
            args.push(ArgSpec::Arr { bytes, vals });
        } else {
            args.push(ArgSpec::Int(parse_int(tok)?));
        }
    }
    Ok(Case {
        func,
        args,
        expected,
        text: line.trim().to_string(),
    })
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
    run_dir_at(dir, backend, opt::MAX_LEVEL, &|p| p)
}

/// Each target's default replacement policy for the abstract 'int' type:
/// the native 64-bit width on the register machines; i32 on wasm32, where
/// encodings are smaller and memory indices are 32-bit anyway.
fn default_int(backend: Backend) -> ssa::Type {
    match backend {
        Backend::Wasm => ssa::Type::I32,
        _ => ssa::Type::I64,
    }
}

/// and for the abstract 'float': f64 on the register machines, f32 on wasm32
fn default_float(backend: Backend) -> (u32, u32) {
    match backend {
        Backend::Wasm => (8, 23),
        _ => (11, 52),
    }
}

pub fn run_dir_at(
    dir: &str,
    backend: Backend,
    level: usize,
    adjust: &dyn Fn(ssa::Policy) -> ssa::Policy,
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

    let (fe, fm) = default_float(backend);
    // the platform (the selected variant of the backend's target, if any)
    // may lack integer instructions the parser would otherwise assume
    let target = match backend {
        Backend::Native | Backend::ArmQemu => "arm64",
        Backend::Riscv => "riscv64",
        Backend::Wasm => "wasm32",
    };
    let platform = crate::platform::Platform::load(target)?;
    let policy = adjust(platform.adjust(ssa::Policy::new(default_int(backend))?.with_float(fe, fm)));
    let mut report = Report {
        passed: 0,
        failed: 0,
        log: String::new(),
    };
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", name, e))?;

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

        let module = (|| -> Result<ssa::Module, String> {
            if let Some(e) = bad_directive {
                return Err(e);
            }
            let mut module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| e.to_string())?;
            ssa::resolve_types(&mut module, &policy);
            ssa::verify(&module).map_err(|errs| errs.join("; "))?;
            opt::optimize(&mut module, level);
            ssa::verify(&module)
                .map_err(|errs| format!("after optimization: {}", errs.join("; ")))?;
            Ok(module)
        })();
        let module = match module {
            Ok(m) => m,
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
        let mut bufs: Vec<Vec<u64>> = Vec::new(); // u64-aligned backing store
        let mut argv = Vec::new();
        for a in &case.args {
            match a {
                ArgSpec::Int(v) => argv.push(*v),
                arr => {
                    let bytes = arr.to_bytes();
                    let mut words = vec![0u64; bytes.len().div_ceil(8).max(1)];
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            words.as_mut_ptr() as *mut u8,
                            bytes.len(),
                        );
                    }
                    bufs.push(words);
                    argv.push(bufs.last().unwrap().as_ptr() as i64);
                }
            }
        }
        // a register holds the canonical value only up to the type's
        // container: read x0/x1 through the declared return types
        let rets: Vec<ssa::Repr> = module
            .func(&case.func)
            .map(|f| f.rets.iter().map(|&t| f.repr(t)).collect())
            .unwrap_or_default();
        let fix = |i: usize, x: i64| match rets.get(i) {
            Some(r) if r.container() == 32 => opt::norm(*r, x as u32 as i64),
            _ => x,
        };
        let got: Result<Vec<i64>, String> = match case.expected.len() {
            1 => jit.call(&case.func, &argv).map(|v| vec![fix(0, v)]),
            2 => jit
                .call2(&case.func, &argv)
                .map(|(a, b)| vec![fix(0, a), fix(1, b)]),
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
            let pty = func.params.get(j).map(|&p| emit_wasm::wrepr(func, func.ty(p)));
            match a {
                ArgSpec::Int(v) => {
                    let t = match pty {
                        Some(r) if r.container() == 64 => "i64",
                        _ => "i32",
                    };
                    spec.push_str(&format!("{{\"t\":\"{}\",\"v\":\"{}\"}}", t, v));
                }
                ArgSpec::Arr { bytes, vals } => {
                    let vs: Vec<String> = vals.iter().map(|v| format!("\"{}\"", v)).collect();
                    spec.push_str(&format!(
                        "{{\"t\":\"ptr\",\"a{}\":[{}]}}",
                        bytes * 8,
                        vs.join(",")
                    ));
                }
            }
        }
        spec.push_str("],\"rets\":[");
        for (j, &t) in func.rets.iter().enumerate() {
            if j > 0 {
                spec.push(',');
            }
            // how the driver prints an i32 result: sign- or zero-extended
            let r = emit_wasm::wrepr(func, t);
            spec.push_str(match (r.container(), r.signed()) {
                (64, _) => "\"i64\"",
                (_, true) => "\"i32\"",
                _ => "\"u32\"",
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
fn __pch(c: u64) {{
entry:
    u: ptr = const {}
    c32: u32 = conv c
    store c32, u
    ret
}}
",
        uart
    ) + PHEX
}

const PHEX: &str = r"
fn __phex(v: u64) {
entry:
    sh0: u64 = const 60
    jmp loop(sh0)
loop(sh: u64):
    t: u64 = shr v, sh
    m: u64 = const 15
    n: u64 = and t, m
    nine: u64 = const 9
    big: u1 = cmp.gt n, nine
    bigi: u64 = conv big
    gap: u64 = const 39
    adj: u64 = mul bigi, gap
    z: u64 = const 48
    c1: u64 = add n, z
    c: u64 = add c1, adj
    __pch(c)
    zero: u64 = const 0
    done: u1 = cmp.eq sh, zero
    four: u64 = const 4
    sh2: u64 = sub sh, four
    br done, exit, loop(sh2)
exit:
    ret
}
";

/// Generate __start: run every case, print each result as hex, then run
/// the target-specific exit statements.
fn tmp_in(s: &mut String, n: &std::cell::Cell<u32>, ty: &str, init: String) -> String {
    n.set(n.get() + 1);
    let name = format!("t{}", n.get());
    s.push_str(&format!("    {}: {} = {}\n", name, ty, init));
    name
}

/// one argument of the written type from the directive's numbers
fn materialize(
    func: &ssa::Function,
    ty: ssa::Type,
    args: &mut std::slice::Iter<ArgSpec>,
    s: &mut String,
    n: &std::cell::Cell<u32>,
    heap: &mut u64,
    text: &str,
) -> Result<String, String> {
    if ty.is_struct() {
        let p = func.pack(ty).unwrap().clone();
        let mut fields = Vec::new();
        for (_, fty) in &p.fields {
            fields.push(materialize(func, *fty, args, s, n, heap, text)?);
        }
        return Ok(tmp_in(s, n, &func.tyname(ty), format!("pack {}", fields.join(", "))));
    }
    let w = func.width(ty).unwrap_or(64);
    if w > 64 {
        let words = w.div_ceil(64);
        // a pack is assembled as its bits, then cast
        let tn = if func.pack(ty).is_some() { format!("u{}", w) } else { func.tyname(ty) };
        let mut acc: Option<String> = None;
        for k in 0..words {
            let Some(ArgSpec::Int(v)) = args.next() else {
                return Err(format!("'{}': a {} takes {} words", text, tn, words));
            };
            let word = tmp_in(s, n, "u64", format!("const {}", v));
            let wide = tmp_in(s, n, &tn, format!("conv {}", word));
            let placed = if k == 0 { wide } else { tmp_in(s, n, &tn, format!("shl {}, {}", wide, 64 * k)) };
            acc = Some(match acc {
                None => placed,
                Some(a) => tmp_in(s, n, &tn, format!("or {}, {}", a, placed)),
            });
        }
        let bits = acc.unwrap();
        return Ok(if func.pack(ty).is_some() { tmp_in(s, n, &func.tyname(ty), format!("cast {}", bits)) } else { bits });
    }
    let a = args.next().ok_or_else(|| format!("too few args in '{}'", text))?;
    Ok(match a {
        ArgSpec::Int(v) => {
            if func.pack(ty).is_some() {
                // packs have no literals: build the bits, then bitcast
                let bits = tmp_in(s, n, &format!("u{}", w), format!("const {}", v));
                tmp_in(s, n, &func.tyname(ty), format!("cast {}", bits))
            } else {
                tmp_in(s, n, &ty.name(), format!("const {}", v))
            }
        }
        ArgSpec::Arr { bytes, vals } => {
            let b = *bytes as u64;
            *heap = (*heap + b - 1) & !(b - 1);
            let base = *heap;
            let ety = format!("i{}", b * 8);
            // the machine's RAM starts zeroed and the heap is never
            // reused, so only nonzero elements need a store (a big
            // zero buffer would otherwise be a store per element)
            for (k, v) in vals.iter().enumerate().filter(|(_, v)| **v != 0) {
                let d = tmp_in(s, n, &ety, format!("const {}", opt::norm(ssa::Repr::S(b as u32 * 8), *v)));
                let p = tmp_in(s, n, "ptr", format!("const {}", base + b * k as u64));
                s.push_str(&format!("    store {}, {}\n", d, p));
            }
            *heap += b * vals.len() as u64;
            tmp_in(s, n, "ptr", format!("const {}", base))
        }
    })
}

/// a result of the written type as the values to print: a struct's
/// fields, a wide value's words, else itself
fn flatten(func: &ssa::Function, ty: ssa::Type, name: String, s: &mut String, n: &std::cell::Cell<u32>, out: &mut Vec<(String, ssa::Type)>) {
    if ty.is_struct() {
        let p = func.pack(ty).unwrap().clone();
        let names: Vec<String> = p.fields.iter().map(|_| { n.set(n.get() + 1); format!("t{}", n.get()) }).collect();
        let defs: Vec<String> = names.iter().zip(&p.fields).map(|(nm, (_, t))| format!("{}: {}", nm, func.tyname(*t))).collect();
        s.push_str(&format!("    {} = unpack {}\n", defs.join(", "), name));
        for (nm, (_, fty)) in names.into_iter().zip(&p.fields) {
            flatten(func, *fty, nm, s, n, out);
        }
        return;
    }
    let w = func.width(ty).unwrap_or(64);
    if w > 64 {
        let (tn, r) = if func.pack(ty).is_some() {
            (format!("u{}", w), tmp_in(s, n, &format!("u{}", w), format!("cast {}", name)))
        } else {
            (func.tyname(ty), name.clone())
        };
        for k in 0..w.div_ceil(64) {
            let sh = if k == 0 { r.clone() } else { tmp_in(s, n, &tn, format!("shr {}, {}", r, 64 * k)) };
            out.push((tmp_in(s, n, "u64", format!("conv {}", sh)), ssa::Type::U64));
        }
        return;
    }
    out.push((name, ty));
}

fn gen_driver(
    module: &ssa::Module,
    cases: &[Case],
    heap_base: u64,
    exit_ssa: &str,
) -> Result<String, String> {
    // one function per case (a driver's frame would otherwise outgrow
    // what a single function may spill), called in order from __start
    let mut s = String::new();
    let mut start = String::from("fn __start() {\nentry:\n");
    let n = std::cell::Cell::new(0u32);
    let mut heap = heap_base;
    let tmp_name = || {
        n.set(n.get() + 1);
        format!("t{}", n.get())
    };
    let tmp = |s: &mut String, ty: &str, init: String| -> String {
        let name = tmp_name();
        s.push_str(&format!("    {}: {} = {}\n", name, ty, init));
        name
    };
    for (ci, case) in cases.iter().enumerate() {
        s.push_str(&format!("fn __case{}() {{\nentry:\n", ci));
        start.push_str(&format!("    __case{}()\n", ci));
        let func = module
            .func(&case.func)
            .ok_or_else(|| format!("no function {} for directive '{}'", case.func, case.text))?;
        if func.rets.is_empty() {
            return Err(format!("{} returns nothing; directives need results", case.func));
        }
        // the signature as written: a value wider than a word is passed
        // and returned as words (see wide.rs), so a directive gives one
        // number per word, low first, and the driver assembles them
        let (ptys, rtys): (Vec<ssa::Type>, Vec<ssa::Type>) = match &func.wide_sig {
            Some((p, r)) => (p.clone(), r.clone()),
            None => (func.params.iter().map(|&p| func.ty(p)).collect(), func.rets.clone()),
        };
        // materialize arguments: a struct from its fields, a wide value
        // from its words, a pack from its bits, an array into the heap
        let mut argv = Vec::new();
        let mut args = case.args.iter();
        for &pty in &ptys {
            argv.push(materialize(func, pty, &mut args, &mut s, &n, &mut heap, &case.text)?);
        }
        if args.next().is_some() {
            return Err(format!("too many args in '{}'", case.text));
        }
        // call, binding every result
        let mut rets = Vec::new();
        for &rt in &rtys {
            let r = tmp_name();
            rets.push((r, rt));
        }
        let defs: Vec<String> = rets
            .iter()
            .map(|(r, t)| format!("{}: {}", r, func.tyname(*t)))
            .collect();
        s.push_str(&format!(
            "    {} = {}({})\n",
            defs.join(", "),
            case.func,
            argv.join(", ")
        ));
        // a struct result prints as its fields, a wide one as its words
        let mut printed: Vec<(String, ssa::Type)> = Vec::new();
        for (r, rt) in &rets {
            flatten(func, *rt, r.clone(), &mut s, &n, &mut printed);
        }
        let rets = printed;
        // print results: 16 hex digits each, space-separated, newline after
        for (i, (r, rt)) in rets.iter().enumerate() {
            if i > 0 {
                let sp = tmp(&mut s, "u64", "const 32".into());
                s.push_str(&format!("    __pch({})\n", sp));
            }
            // the canonical 64-bit value of the result, as a u64 for printing:
            // signed types sign-extend, everything else zero-extends
            let repr = func.repr(*rt);
            let mut v = r.clone();
            if func.pack(*rt).is_some() {
                v = tmp(&mut s, &format!("u{}", repr.bits()), format!("cast {}", v));
            }
            if repr.signed() {
                if repr.bits() < 64 {
                    v = tmp(&mut s, "i64", format!("conv {}", v));
                }
                v = tmp(&mut s, "u64", format!("cast {}", v));
            } else if repr.bits() < 64 {
                v = tmp(&mut s, "u64", format!("conv {}", v));
            } else if *rt == ssa::Type::Ptr {
                v = tmp(&mut s, "u64", format!("cast {}", v));
            }
            s.push_str(&format!("    __phex({})\n", v));
        }
        let nl = tmp(&mut s, "u64", "const 10".into());
        s.push_str(&format!("    __pch({})\n", nl));
        s.push_str("    ret\n}\n");
    }
    start.push_str(exit_ssa);
    start.push_str("    ret\n}\n");
    // __start first: the bare-metal preamble falls into the first function
    Ok(format!("{}{}", start, s))
}

/// Run a qemu child with a hard deadline (a wrong branch in emitted code
/// means an infinite loop); returns captured stdout.
/// a bare-metal riscv64 image: sp = 0x80800000 (mid-RAM), the FPU on,
/// then fall into the first function
const RV_PREAMBLE_WORDS: usize = 6;

pub fn rv_image(compiled: &emit::Compiled, enc: &emit::Encoder) -> Result<Vec<u8>, String> {
    let mut bin = Vec::new();
    const ADDI: &str = "addi {r}, {r}, {i -2048..2047}";
    const SLLI: &str = "slli {r}, {r}, {i 0..63}";
    for (t, v) in [
        (ADDI, [2i64, 0, 0x80]),
        (SLLI, [2, 2, 8]),
        (ADDI, [2, 2, 0x80]),
        (SLLI, [2, 2, 16]),
        // enable the FPU (mstatus.FS = initial) for the platform's fadd
        ("lui {r}, {i 0..1048575}", [5, 0x2, 0]),
        ("csrrs {r}, {i 0..4095}, {r}", [0, 0x300, 5]),
    ] {
        let n = if t.starts_with("lui") { 2 } else { 3 };
        bin.extend(enc.encode(t, &v[..n])?.to_le_bytes());
    }
    debug_assert_eq!(bin.len(), RV_PREAMBLE_WORDS * 4);
    bin.extend(&compiled.code);
    Ok(bin)
}

const ARM_PREAMBLE_WORDS: usize = 6;

/// a bare-metal aarch64 image: the FPU on (cpacr_el1.FPEN), sp =
/// 0x41000000 via x29 (sp itself is not a movz target), then fall into
/// the first function
pub fn arm_image(compiled: &emit::Compiled, enc: &emit::Encoder) -> Result<Vec<u8>, String> {
    let mut bin = Vec::new();
    bin.extend(enc.encode("movz {x}, #{i 0..65535}, lsl #16", &[0, 0x0030])?.to_le_bytes());
    bin.extend(enc.encode("msr cpacr_el1, {x}", &[0])?.to_le_bytes());
    bin.extend(enc.encode("isb", &[])?.to_le_bytes());
    bin.extend(enc.encode("movz {x}, #{i 0..65535}, lsl #16", &[29, 0x4100])?.to_le_bytes());
    bin.extend(enc.encode("mov sp, x29", &[])?.to_le_bytes());
    // a sixth word keeps the code (and the data after it) 8-aligned:
    // with the MMU off every access is to device memory, and a 64-bit
    // load from a 4-aligned address faults
    bin.extend(enc.encode("mov {x}, {x}", &[0, 0])?.to_le_bytes());
    debug_assert_eq!(bin.len(), ARM_PREAMBLE_WORDS * 4);
    bin.extend(&compiled.code);
    Ok(bin)
}

/// qemu's command line for a raw image on the virt machine
pub fn qemu_command(target: &str, bin_path: &std::path::Path) -> Command {
    let mut cmd;
    if target == "riscv64" {
        cmd = Command::new("qemu-system-riscv64");
        cmd.args(["-machine", "virt", "-bios", "none", "-nographic", "-m", "128M"])
            .arg("-device")
            .arg(format!("loader,file={},addr=0x80000000", bin_path.display()));
    } else {
        cmd = Command::new("qemu-system-aarch64");
        cmd.args(["-machine", "virt", "-cpu", "cortex-a57", "-nographic", "-m", "128M"])
            .arg("-device")
            .arg(format!("loader,file={},addr=0x40200000,cpu-num=0", bin_path.display()));
    }
    cmd
}

/// Boot a program on bare-metal qemu: its `__start` runs first (the
/// preamble falls into the first function, so it is moved there); the
/// output is what it wrote to the UART. The platform's constants
/// (`platform uart`, `platform finisher`) are the board's addresses.
pub fn boot(path: &str, target: &str, level: usize, policy: ssa::Policy, input: Option<&[u8]>) -> Result<String, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let platform = crate::platform::Platform::load(target)?;
    let policy = platform.adjust(policy);
    let mut module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| e.to_string())?;
    ssa::resolve_types(&mut module, &policy);
    ssa::verify(&module).map_err(|e| e.join("; "))?;
    let start = module.funcs.iter().position(|f| f.name == "__start").ok_or("a bootable program needs fn __start()")?;
    let f = module.funcs.remove(start);
    module.funcs.insert(0, f);
    opt::optimize(&mut module, level);
    let enc = emit::Encoder::load(&format!("targets/{}.encodings.json", target))?;
    // the code follows the preamble, which is where a vector table's
    // alignment is measured from
    let bin = if target == "riscv64" {
        rv_image(&emit_rv::compile_image(&module, &enc, &platform, RV_PREAMBLE_WORDS * 4)?, &enc)?
    } else {
        arm_image(&emit::compile_image(&module, &enc, &platform, ARM_PREAMBLE_WORDS * 4)?, &enc)?
    };
    let scratch = std::env::temp_dir().join("probe-boot");
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let bin_path = scratch.join(format!("{}.{}.bin", std::path::Path::new(path).file_stem().unwrap().to_string_lossy(), target));
    std::fs::write(&bin_path, &bin).map_err(|e| e.to_string())?;
    match input {
        // the machine's serial port reads what it is given
        Some(bytes) => exec_qemu_with(qemu_command(target, &bin_path), 30, Some(bytes)),
        // or our own stdin: a pipe, or the terminal for as long as the
        // program runs
        None => {
            use std::io::IsTerminal;
            let secs = if std::io::stdin().is_terminal() { 24 * 3600 } else { 30 };
            exec_qemu_with(qemu_command(target, &bin_path), secs, None)
        }
    }
}

pub fn exec_qemu(cmd: Command, secs: u64) -> Result<String, String> {
    exec_qemu_with(cmd, secs, Some(&[]))
}

/// run qemu with its serial port's input: bytes (piped, then closed),
/// or None for the terminal
pub fn exec_qemu_with(mut cmd: Command, secs: u64, input: Option<&[u8]>) -> Result<String, String> {
    let child = cmd
        .stdin(if input.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::inherit() })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut child = child.map_err(|e| format!("spawn qemu: {}", e))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        use std::io::Write;
        let _ = stdin.write_all(bytes);
        // stdin drops here: closed after the bytes
    }
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
        return Err(format!("qemu timed out (runaway emitted code?); output so far:\n{}", out));
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
    report: &mut Report,
) {
    let fail_all = |report: &mut Report, msg: String| {
        report.failed += cases.len().max(1);
        report.log.push_str(&format!("FAIL  {:<16} {}\n", name, msg));
    };

    let prepared = (|| -> Result<Vec<u8>, String> {
        let exit_ssa = format!(
            "    __f1: i32 = const 21845\n    __f2: ptr = const {}\n    store __f1, __f2\n",
            RV_FINISHER
        );
        let driver = gen_driver(module, cases, RV_HEAP, &exit_ssa)?;
        let full = format!("{}\n{}\n{}", driver, helpers(RV_UART), ssa::with_prelude(src));
        let mut m2 = ssa::parse_with(&full, policy).map_err(|e| format!("driver: {}", e))?;
        ssa::resolve_types(&mut m2, policy);
        ssa::verify(&m2).map_err(|e| format!("driver: {}", e.join("; ")))?;
        opt::optimize(&mut m2, level);
        let compiled = emit_rv::compile(&m2, enc)?;
        rv_image(&compiled, enc)
    })();
    let bin = match prepared {
        Ok(b) => b,
        Err(e) => return fail_all(report, e),
    };
    let bin_path = scratch.join(format!("{}.rv.bin", name));
    if let Err(e) = std::fs::write(&bin_path, &bin) {
        return fail_all(report, e.to_string());
    }

    let cmd = qemu_command("riscv64", &bin_path);
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
    report: &mut Report,
) {
    let fail_all = |report: &mut Report, msg: String| {
        report.failed += cases.len().max(1);
        report.log.push_str(&format!("FAIL  {:<16} {}\n", name, msg));
    };

    let prepared = (|| -> Result<Vec<u8>, String> {
        // exit through a stub whose body is patched below into a PSCI
        // SYSTEM_OFF hypervisor call (x0 = 0x84000008; hvc #0)
        let driver = gen_driver(module, cases, ARM_HEAP, "    __qemu_exit()\n")?;
        let stub = "fn __qemu_exit() {\nentry:\n    ret\n}\n";
        let full = format!("{}\n{}\n{}\n{}", driver, helpers(ARM_UART), stub, ssa::with_prelude(src));
        let mut m2 = ssa::parse_with(&full, policy).map_err(|e| format!("driver: {}", e))?;
        ssa::resolve_types(&mut m2, policy);
        ssa::verify(&m2).map_err(|e| format!("driver: {}", e.join("; ")))?;
        opt::optimize(&mut m2, level);
        let compiled = emit::compile(&m2, enc)?;
        let mut bin = arm_image(&compiled, enc)?;
        let preamble = ARM_PREAMBLE_WORDS * 4;
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

    let cmd = qemu_command("arm64", &bin_path);
    match exec_qemu(cmd, 30) {
        Ok(out) => check_hex_lines(&out, cases, name, report),
        Err(e) => fail_all(report, e),
    }
}

fn finish_case(report: &mut Report, name: &str, case: &Case, got: Result<Vec<i64>, String>) {
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
            let report = super::run_dir_at("suite", super::Backend::Native, level, &|p| p)
                .expect("suite runs");
            assert_eq!(report.failed, 0, "at level {}:\n{}", level, report.log);
        }
    }

    #[test]
    fn regression_suite_both_int_policies() {
        // abstract-typed programs must behave identically under every
        // replacement policy (concrete-typed programs are unaffected)
        for int in [crate::ssa::Type::I32, crate::ssa::Type::I64] {
            let report = super::run_dir_at("suite", super::Backend::Native, crate::opt::MAX_LEVEL, &|mut p| {
                p.int = int;
                p
            })
            .expect("suite runs");
            assert_eq!(report.failed, 0, "with int={}:\n{}", int.name(), report.log);
        }
    }

    #[test]
    fn regression_suite_both_float_policies() {
        // programs over the abstract 'float' must behave the same at
        // every width the policy can choose (concrete ones are unaffected)
        for (e, m) in [(8u32, 23u32), (11, 52)] {
            let report = super::run_dir_at("suite", super::Backend::Native, crate::opt::MAX_LEVEL, &|p| p.with_float(e, m))
                .expect("suite runs");
            assert_eq!(report.failed, 0, "with float=({}, {}):\n{}", e, m, report.log);
        }
    }

    #[test]
    fn regression_suite_both_fixed_policies() {
        for (i, f) in [(16u32, 16u32), (32, 32), (24, 8)] {
            let report = super::run_dir_at("suite", super::Backend::Native, crate::opt::MAX_LEVEL, &|p| p.with_fixed(i, f))
                .expect("suite runs");
            assert_eq!(report.failed, 0, "with fixed=({}, {}):\n{}", i, f, report.log);
        }
    }

    #[test]
    fn regression_suite_scalar_families() {
        for family in crate::ssa::Policy::SCALARS {
            let report = super::run_dir_at("suite", super::Backend::Native, crate::opt::MAX_LEVEL, &|p| p.with_scalar(family).unwrap())
                .expect("suite runs");
            assert_eq!(report.failed, 0, "with scalar={}:\n{}", family, report.log);
        }
    }

    /// the machines' timing is some tests' subject, so everything that
    /// runs a machine takes turns: a boot's lateness figures must not
    /// be the host's load
    fn boot_turn() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// the first operating system: hello world on both bare-metal machines
    #[test]
    fn hello_world_boots() {
        let _turn = boot_turn();
        for target in ["riscv64", "arm64"] {
            let out = super::boot("os/hello.ssa", target, crate::opt::MAX_LEVEL, crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap(), Some(b"")).unwrap();
            assert!(out.contains("hello world ᕦ(ツ)ᕤ"), "{}: {:?}", target, out);
        }
    }

    /// the third: the timer interrupt, ten ticks a tenth of a second
    /// apart, and the elapsed count of the machine's counter as exact
    /// milliseconds — a second, plus what qemu's timers are late by
    #[test]
    fn clock_boots() {
        let _turn = boot_turn();
        for target in ["riscv64", "arm64"] {
            let out = super::boot("os/clock.ssa", target, crate::opt::MAX_LEVEL, crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap(), Some(b"")).unwrap();
            let ticks = out.matches("tick\n").count();
            let report = out.lines().last().unwrap_or("");
            let ms: i64 = report.strip_prefix("10 ticks in ").and_then(|r| r.strip_suffix(" ms")).and_then(|n| n.parse().ok()).unwrap_or(-1);
            assert_eq!(ticks, 10, "{}: {:?}", target, out);
            assert!((1000..1500).contains(&ms), "{}: {:?}", target, out);
        }
    }

    /// the fifth: three tasks sleeping until exact times — every 1/10,
    /// 1/3 and 1/7 of a second — must wake in the order the fractions
    /// say (ties at 1 s in index order), never early; two callbacks at
    /// 13/25 s and 77/100 s, the first spawning a one-shot task for
    /// 63/100 s that gives its stack back; a frame every tenth of a
    /// second (flipped at 0.05, 0.15, ...) whose record — the switches
    /// to tasks in it — is printed by the idle task. The wakes, the
    /// frames' contents and the stacks left out are exact; the lateness
    /// figures, and whether two events within the machine's wake-up
    /// latency of each other (c at 0.857 and the flip at 0.85) share an
    /// interrupt and so which prints first, are qemu's.
    #[test]
    fn sleep_boots() {
        let _turn = boot_turn();
        let mut wakes: Vec<(i64, i64, char)> = Vec::new();
        for (letter, den, count) in [('a', 10, 10), ('b', 3, 3), ('c', 7, 7)] {
            for k in 1..=count {
                wakes.push((k, den, letter));
            }
        }
        wakes.push((63, 100, '*'));
        wakes.sort_by(|x, y| (x.0 * y.1).cmp(&(y.0 * x.1)).then(x.2.cmp(&y.2)));
        let mut timed: Vec<(i64, i64, String)> = wakes.iter().map(|(k, d, l)| (*k, *d, format!("{} {}", l, k * 1_000_000 / d))).collect();
        timed.push((13, 25, "! 520000".into()));
        timed.push((77, 100, "!! 770000".into()));
        timed.sort_by(|x, y| (x.0 * y.1).cmp(&(y.0 * x.1)));
        let want_timed: Vec<String> = timed.into_iter().map(|(_, _, t)| t).collect();
        // frame k, flipped at (2k - 1)/20, records the switches in the
        // frame before: the three starts, the one-shot's first dispatch
        // at 13/25 (frame 6), and the wakes in each tenth
        let mut want_frames = Vec::new();
        for k in 1..=11i64 {
            let (lo, hi) = (2 * k - 3, 2 * k - 1); // twentieths: (lo/20, hi/20]
            let mut switches: Vec<(i64, i64, char)> = if k == 1 { vec![(0, 1, 'a'), (0, 1, 'b'), (0, 1, 'c')] } else { Vec::new() };
            for (n, d, l) in wakes.iter().chain([(13, 25, '*')].iter()) {
                if n * 20 > lo * d && n * 20 <= hi * d {
                    switches.push((*n, *d, *l));
                }
            }
            switches.sort_by(|x, y| (x.0 * y.1).cmp(&(y.0 * x.1)).then(x.2.cmp(&y.2)));
            let letters: Vec<String> = switches.iter().map(|s| s.2.to_string()).collect();
            want_frames.push(format!("frame {}: {}", k, letters.join(" ")));
        }
        for target in ["riscv64", "arm64"] {
            let out = super::boot("os/sleep.ssa", target, crate::opt::MAX_LEVEL, crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap(), Some(b"")).unwrap();
            let lines: Vec<&str> = out.lines().collect();
            let n = lines.len();
            assert_eq!(n, want_timed.len() + want_frames.len() + 3, "{}: {:?}", target, out);
            let mut got_timed = Vec::new();
            let mut got_frames = Vec::new();
            for line in &lines[..n - 3] {
                if line.starts_with("frame ") {
                    got_frames.push(line.to_string());
                } else {
                    let (event, late) = line.split_once(" +").unwrap_or((line, "x"));
                    let late: i64 = late.parse().unwrap_or(-1);
                    assert!((0..20_000).contains(&late), "{}: {:?}", target, line);
                    got_timed.push(event.to_string());
                }
            }
            assert_eq!(got_timed, want_timed, "{}: {:?}", target, out);
            assert_eq!(got_frames, want_frames, "{}: {:?}", target, out);
            let interrupts: i64 = lines[n - 3].split_once(" interrupts").and_then(|(k, _)| k.parse().ok()).unwrap_or(-1);
            assert!((30..=32).contains(&interrupts), "{}: {:?}", target, out);
            assert_eq!(lines[n - 2], "3 stacks out", "{}: {:?}", target, out);
            // the stacks' block and the two frame arenas (4 KB units); the
            // one-shot's region went back
            assert_eq!(lines[n - 1], "73728 heap bytes out", "{}: {:?}", target, out);
        }
    }

    /// a failed check is a breakpoint trap the kernel reports, with the
    /// address of the check; what follows it never runs
    #[test]
    fn check_boots() {
        let _turn = boot_turn();
        for target in ["riscv64", "arm64"] {
            let out = super::boot("os/check.ssa", target, crate::opt::MAX_LEVEL, crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap(), Some(b"")).unwrap();
            let rest = out.strip_prefix("acheck failed at 0x").unwrap_or_else(|| panic!("{}: {:?}", target, out));
            assert_eq!(rest.trim_end().len(), 16, "{}: {:?}", target, out);
            assert!(!out.contains('b'), "{}: {:?}", target, out);
        }
    }

    /// the fourth: two tasks preempted by the timer, each printing its
    /// letter once per slice it runs in
    #[test]
    fn tasks_boots() {
        let _turn = boot_turn();
        for target in ["riscv64", "arm64"] {
            let out = super::boot("os/tasks.ssa", target, crate::opt::MAX_LEVEL, crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap(), Some(b"")).unwrap();
            assert_eq!(out, "ababababab\n", "{}", target);
        }
    }

    /// the second: a trap handler, system calls through a table of
    /// function values, and input from the serial port, on both machines
    #[test]
    fn echo_boots() {
        let _turn = boot_turn();
        for target in ["riscv64", "arm64"] {
            let out = super::boot("os/echo.ssa", target, crate::opt::MAX_LEVEL, crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap(), Some(b"hello\n\nsyscalls\nbye\n")).unwrap();
            assert_eq!(out, "> echo: hello\n> echo: \n> echo: syscalls\n> ", "{}", target);
        }
    }

    /// the suite on the RISC-V cores without F/D and without M: floats,
    /// multiplies and divides all in the library, results unchanged
    #[test]
    fn regression_suite_riscv_variants() {
        let _turn = boot_turn(); // a machine at a time: the boot tests are timed
        for variant in ["rv64im", "rv64i"] {
            crate::platform::select(variant);
            let report = super::run_dir_at("suite", super::Backend::Riscv, crate::opt::MAX_LEVEL, &|p| p).expect("suite runs");
            assert_eq!(report.failed, 0, "{}:\n{}", variant, report.log);
        }
    }

    #[test]
    fn regression_suite_unit_policies() {
        for n in [8u32, 16, 32] {
            let report = super::run_dir_at("suite", super::Backend::Native, crate::opt::MAX_LEVEL, &|p| p.with_unit(n).with_sunit(n))
                .expect("suite runs");
            assert_eq!(report.failed, 0, "with unit={}:\n{}", n, report.log);
        }
    }

    #[test]
    fn regression_suite_wasm() {
        let report = super::run_dir("suite", super::Backend::Wasm).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }

    #[test]
    fn regression_suite_riscv() {
        let _turn = boot_turn(); // a machine at a time: the boot tests are timed
        let report = super::run_dir("suite", super::Backend::Riscv).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }

    #[test]
    fn regression_suite_arm_qemu() {
        let _turn = boot_turn(); // a machine at a time: the boot tests are timed
        let report = super::run_dir("suite", super::Backend::ArmQemu).expect("suite runs");
        assert_eq!(report.failed, 0, "\n{}", report.log);
    }
}
