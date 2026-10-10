//! What a host of the compiler needs that is not the runner.
//!
//! The suite's runner (`suite.rs`) holds the native JIT, `fork` and the
//! spawning of node and qemu, none of which builds for a machine with
//! no processes, the compiler compiled for `wasm32` and run in a page
//! say. Two things a host that compiles and does not run still wants
//! lived there beside them: the policy a path compiles under, and how a
//! call into a wasm module is described to its driver. They are here,
//! with the few types that say what a call is and what it gave, and
//! `suite.rs` names them from here. Nothing in this file reads but
//! `Platform::load`, which goes through `vfs`.

use crate::zero::lower::Site;
use crate::{emit_wasm, ssa};

#[derive(Clone, Copy, PartialEq)]
pub enum Backend {
    Native,
    Wasm,
    /// bare-metal on qemu-system-riscv64: a driver generated in our own SSA
    /// prints results over the virt machine's UART and exits via its test
    /// finisher — qemu's decoder and semantics are the independent referee
    Riscv,
    /// Apple's GPU: a driver in our own SSA becomes a kernel, one thread
    /// runs every case and writes the results as text into an area of
    /// the program's memory (see emit_air.rs; tools/driver_metal.py)
    Air,
    /// the arm64 backend, but run bare-metal under qemu-system-aarch64
    /// instead of natively — an independent implementation of the
    /// architecture judging the same bytes the M-series CPU runs
    ArmQemu,
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

/// and for `index`, the width of a count and a position in memory (fm3
/// question 73): 32 bits on wasm32, where an address is a 32-bit offset,
/// and 64 on the register machines. The GPU's path takes 64 too: an
/// address there is a 64-bit offset into the device's buffer as the
/// emitter has it, and `int` is 64 beside it; 32 would suit a GPU and is
/// a product's to say, not a default nobody has measured
fn default_index(backend: Backend) -> u32 {
    match backend {
        Backend::Wasm => 32,
        _ => 64,
    }
}

/// and for the abstract 'float': f64 on the register machines, f32 on wasm32
fn default_float(backend: Backend) -> (u32, u32) {
    match backend {
        Backend::Wasm | Backend::Air => (8, 23),
        _ => (11, 52),
    }
}

/// the platform a backend compiles for
pub fn target_of(backend: Backend) -> &'static str {
    match backend {
        Backend::Native | Backend::ArmQemu => "arm64",
        Backend::Riscv => "riscv64",
        Backend::Wasm => "wasm32",
        Backend::Air => "air",
    }
}

/// a backend's default policy: its `int` and `float` widths, and what
/// its platform (the selected variant of the backend's target, if any)
/// lacks of the integer instructions the parser would otherwise assume
pub fn backend_policy(backend: Backend) -> Result<ssa::Policy, String> {
    let (fe, fm) = default_float(backend);
    let platform = crate::platform::Platform::load(target_of(backend))?;
    let policy = ssa::Policy::new(default_int(backend))?.with_float(fe, fm).with_index(default_index(backend)).unwrap();
    Ok(platform.adjust(policy))
}

pub struct Report {
    pub passed: usize,
    pub failed: usize,
    /// cases a backend cannot run (AIR: recursion), neither passed nor failed
    pub skipped: usize,
    pub log: String,
}

impl Report {
    pub fn case(&mut self, ok: bool, file: &str, text: &str, note: &str) {
        if ok {
            self.passed += 1;
            if note.is_empty() {
                self.log.push_str(&format!("  ok  {:<16} {}\n", file, text));
            } else {
                self.log.push_str(&format!("  ok  {:<16} {}   {}\n", file, text, note));
            }
        } else {
            self.failed += 1;
            self.log
                .push_str(&format!("FAIL  {:<16} {}   {}\n", file, text, note));
        }
    }
}

/// a call the zero front end's runner makes: a function, its integer
/// arguments, how many results it has, whether it must end in a failed
/// check, whether the program's text output is wanted after it, and
/// the calls that set its context between the reset and the start
/// (log 43), each a function and its integer arguments
pub struct Call {
    pub func: String,
    pub args: Vec<i64>,
    pub nrets: usize,
    pub checks: bool,
    pub text: bool,
    pub before: Vec<(String, Vec<i64>)>,
    /// the program's output printed as it lands (log 77): the call on a
    /// thread, the ring read beside it; native only
    pub live: bool,
    /// when the text was written is wanted too (fm3 log 91): the marks
    /// the program's clock left, read after the text
    pub times: bool,
}

/// what a call gave: its results, and the text the program printed
pub struct Got {
    pub values: Vec<i64>,
    pub text: String,
    /// for a call that wants times: the words of the program's marks,
    /// one for each byte of the text and one more for its end, the time
    /// the clock moved to when that many bytes had been written, or
    /// zero where it did not move (fm3 log 95)
    pub marks: Vec<i64>,
}

pub const CHECKED: &str = "a failed check";

/// what a call that ended in a failed check gives back
pub fn checked() -> &'static str {
    CHECKED
}

/// the file of the language's own feature as a row of the table names
/// it: a check that fails there is reported at the program's line
pub const OWN_SITE: &str = "platform.zero";

/// what a stop says where it is the trace build's own (fm3 tracer.md):
/// the trace held all it holds, and the program was stopped there. Its
/// site is -1, no row of the table
pub const TRACE_FULL: &str = "at #ffffffffffffffff,";

/// the most events a trace keeps, and the most bytes of the values'
/// text (`TRACE_WORDS` in `zero/lower.rs`)
pub const TRACE_EVENTS: i64 = 16384;
pub const TRACE_TEXT: i64 = 65536;

/// `a failed check at #<site>,<a>,<b>` as the table reads it: what a
/// stop in the diagnostic build of a zero store says (fm3 log 199), its
/// site a row of `Lowered.sites` counted from 1, turned into the file,
/// the line and what was being asked. Here beside `CHECKED` so that a
/// host that runs the module itself, the page, reads it as the runner
/// does. None where the text names no site or the table has no such row
pub fn site_said(sites: &[Site], said: &str) -> Option<String> {
    let words: Vec<i64> = said.strip_prefix(checked())?.trim().strip_prefix("at #")?.split(',').map(|w| u64::from_str_radix(w.trim(), 16).map(|v| v as i64)).collect::<Result<_, _>>().ok()?;
    let [n, a, b] = words.as_slice() else { return None };
    let site = sites.get((*n as usize).checked_sub(1)?)?;
    // a check of the language's own (fm3 log 228): a person is told
    // their own line, the statement of the program that called in,
    // which is the site the check found current and handed over, with
    // the check's reason; the language's line where there was none
    if site.file == OWN_SITE {
        if let Some(caller) = (*a as usize).checked_sub(1).and_then(|i| sites.get(i)).filter(|c| c.file != OWN_SITE) {
            return Some(format!("{} at {}:{}{}", checked(), caller.file, caller.line, if site.what.is_empty() { String::new() } else { format!(": {}", site.what) }));
        }
        return Some(format!("{} at {}:{}{}", checked(), site.file, site.line, if site.what.is_empty() { String::new() } else { format!(": {}", site.what) }));
    }
    let what = site.what.replace("{a}", &a.to_string()).replace("{b}", &b.to_string());
    Some(format!("{} at {}:{}{}", checked(), site.file, site.line, if what.is_empty() { String::new() } else { format!(": {}", what) }))
}

/// an integer argument list as the wasm driver (`driver.js`) reads it,
/// typed by the function's parameters as the module has them
pub fn wasm_args(func: &ssa::Function, args: &[i64]) -> String {
    let mut out = Vec::new();
    for (j, v) in args.iter().enumerate() {
        let t = match func.params.get(j).map(|&p| emit_wasm::wrepr(func, func.ty(p))) {
            Some(r) if r.container() == 64 => "i64",
            _ => "i32",
        };
        out.push(format!("{{\"t\":\"{}\",\"v\":\"{}\"}}", t, v));
    }
    out.join(",")
}

/// how the driver reads each of a function's results back: `"i64"`,
/// `"i32"` or `"u32"`, quoted as the spec has them
pub fn wasm_rets(func: &ssa::Function) -> Vec<&'static str> {
    func.rets
        .iter()
        .map(|&t| {
            let r = emit_wasm::wrepr(func, t);
            match (r.container(), r.signed()) {
                (64, _) => "\"i64\"",
                (_, true) => "\"i32\"",
                _ => "\"u32\"",
            }
        })
        .collect()
}

/// The spec the wasm driver is handed for the zero runner's calls: for
/// each, the function, that the module is reset first, whether its text
/// and its times are read back, the calls made before it, its arguments
/// and how its results are read. A host that runs the module itself
/// reads the same text, or makes its own of `wasm_args` and `wasm_rets`
pub fn wasm_cases(module: &ssa::Module, calls: &[Call]) -> Result<String, String> {
    let mut spec = String::from("{\"cases\":[");
    for (i, call) in calls.iter().enumerate() {
        if i > 0 {
            spec.push(',');
        }
        let func = module.func(&call.func).ok_or_else(|| format!("no function {} in the module", call.func))?;
        let mut before = Vec::new();
        for (f, args) in &call.before {
            let bf = module.func(f).ok_or_else(|| format!("no function {} in the module", f))?;
            before.push(format!("{{\"func\":\"{}\",\"args\":[{}]}}", f, wasm_args(bf, args)));
        }
        spec.push_str(&format!("{{\"func\":\"{}\",\"reset\":true,\"text\":{},\"times\":{},\"before\":[{}],\"args\":[{}", call.func, call.text, call.times, before.join(","), wasm_args(func, &call.args)));
        spec.push_str("],\"rets\":[");
        spec.push_str(&wasm_rets(func).join(","));
        spec.push_str("]}");
    }
    spec.push_str("]}");
    Ok(spec)
}
