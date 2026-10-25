//! The kernel form (hop sixty-two, fm3 question 211, `triangle.md` step
//! two): a function applied across an array, run as a GPU kernel, and
//! the frame it leaves compared word for word with the one the CPU's
//! loop makes. Nothing in the program says GPU: `probe zero <store>
//! kernel 'frame (64, 0.5)' --wgsl` asks for it from outside.
//!
//! The store is lowered as for any build, and its text read
//! (`lower::kernel_plan`) for the function applied to each item of the
//! range in the called function's one line. The kernel is that lowered
//! IR with one function more, `__kernel(mem, area, id)`, which reads the
//! call's values from a data item the kernel form adds, calls the
//! applied function for item `start + id`, and leaves its result as
//! word `id` of the area; it is built under the air platform's policy
//! (so `float` is 32 bits), printed by `emit_wgsl.rs` and run by
//! `tools/driver_webgpu.js` with a thread a word, or compiled by
//! `emit_air.rs` and run by `tools/driver_metal.py`. The CPU's frame is
//! the called function itself, as `lower.rs` makes it today, run on the
//! JIT with an arena large enough for the frame, its items read back
//! one by one.

use super::lower::{self, KernelArg, KernelPlan, KernelValue};
use super::{run, store};
use crate::{emit, ssa, suite};
use std::fmt::Write;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// the CPU's arena under the kernel form: the bootstrap's 64 KB holds a
/// frame of a few thousand items, and this one a frame of a million
const ARENA: usize = 1 << 24;
/// ... and how many items it copies out
const OUT: usize = 1 << 20;
/// how many times a driver dispatches the kernel, the least time kept
const TIMES: usize = 5;

/// which GPU path the kernel takes
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gpu {
    Wgsl,
    Air,
}

impl Gpu {
    fn name(self) -> &'static str {
        match self {
            Gpu::Wgsl => "WebGPU",
            Gpu::Air => "Metal",
        }
    }
}

/// what one run of the kernel form found
pub struct Compared {
    pub call: String,
    pub gpu: Gpu,
    pub words: usize,
    /// the first word that differs: its place, the GPU's word, the CPU's
    pub first_differing: Option<(usize, i64, i64)>,
    pub same: usize,
    /// milliseconds: the kernel's dispatch as the driver timed it, the
    /// driver's whole run (Chrome's start included on WebGPU), and the
    /// CPU's call of the function
    pub dispatch_ms: f64,
    pub whole_ms: f64,
    pub cpu_ms: f64,
    pub wgsl_lines: usize,
}

impl Compared {
    pub fn said(&self) -> String {
        let mut s = match self.first_differing {
            None => format!("{}: {} pixels on {}, {} the same as the CPU's\n", self.call, self.words, self.gpu.name(), self.same),
            Some((k, g, c)) => format!("{}: {} pixels on {}; pixel {} differs: {} on {}, {} on the CPU ({} the same)\n", self.call, self.words, self.gpu.name(), k, g, self.gpu.name(), c, self.same),
        };
        let module = if self.wgsl_lines > 0 { format!(" ({} lines of WGSL)", self.wgsl_lines) } else { String::new() };
        let _ = writeln!(s, "  {}{}: the kernel's dispatch {:.2} ms (the least of five), the whole run {:.0} ms{}; the CPU's loop {:.2} ms (the least of five)", self.gpu.name(), module, self.dispatch_ms, self.whole_ms, if self.gpu == Gpu::Wgsl { " with Chrome's start" } else { " with the driver's start" }, self.cpu_ms);
        s
    }
}

/// a value as the word the kernel's data item holds for a parameter of
/// IR type `ty` under the GPU's policy, where `float` is 32 bits
fn word(v: KernelValue, ty: &str) -> Result<i64, String> {
    let float32 = matches!(ty, "float" | "f32" | "scalar" | "number");
    Ok(match (v, ty) {
        (KernelValue::Float(f), _) if float32 => (f as f32).to_bits() as i64,
        (KernelValue::Int(i), _) if float32 => (i as f32).to_bits() as i64,
        (KernelValue::Float(f), "f64") => f.to_bits() as i64,
        (KernelValue::Int(i), "f64") => (i as f64).to_bits() as i64,
        (KernelValue::Int(i), _) => i,
        (KernelValue::Float(_), t) => return Err(format!("a number with a point given for a parameter of type {}", t)),
    })
}

/// ... and as the CPU's call takes it, where `float` is 64 bits
fn native_word(v: KernelValue, ty: &str) -> i64 {
    let float = matches!(ty, "float" | "f64" | "scalar" | "number");
    match v {
        KernelValue::Float(f) if ty == "f32" => (f as f32).to_bits() as i64,
        KernelValue::Int(i) if ty == "f32" => (i as f32).to_bits() as i64,
        KernelValue::Float(f) => f.to_bits() as i64,
        KernelValue::Int(i) if float => (i as f64).to_bits() as i64,
        KernelValue::Int(i) => i,
    }
}

fn literal(v: KernelValue) -> String {
    match v {
        KernelValue::Int(i) => i.to_string(),
        KernelValue::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{:.1}", f),
        KernelValue::Float(f) => format!("{}", f),
    }
}

/// The IR of the kernel: the call's values in a data item, a word each,
/// and `__kernel`, which calls the applied function for item `start +
/// id` and leaves its result as word `id` of the area
pub fn kernel_ir(p: &KernelPlan) -> Result<String, String> {
    let mut t = String::new();
    let words: Vec<String> = p.params.iter().zip(&p.values).map(|((_, ty), v)| word(*v, ty).map(|w| w.to_string())).collect::<Result<_, _>>()?;
    let _ = writeln!(t, "\n; the kernel form (hop sixty-two): `{}` applied to item start + id, the call's values a word each", p.callee_said);
    let _ = writeln!(t, "data __kernel_args: array(i64, {}) = {}", words.len().max(1), if words.is_empty() { "0".to_string() } else { words.join(", ") });
    let _ = writeln!(t, "fn __kernel(mem: ptr, area: ptr, id: i64)");
    let _ = writeln!(t, "    a: ptr = addr __kernel_args");
    for (j, (_, ty)) in p.params.iter().enumerate() {
        let _ = writeln!(t, "    p{}: {} = load a, {}", j, ty, 8 * j);
    }
    let mut args = Vec::new();
    for (i, h) in p.hands.iter().enumerate() {
        match h {
            KernelArg::Place { start, ty } => {
                let _ = writeln!(t, "    k0: i64 = add id, {}", start);
                let _ = writeln!(t, "    k: {} = conv k0", ty);
                args.push("k".to_string());
            }
            KernelArg::Param(j) => args.push(format!("p{}", j)),
            KernelArg::Lit(v, ty) => {
                let _ = writeln!(t, "    l{}: {} = const {}", i, ty, literal(*v));
                args.push(format!("l{}", i));
            }
        }
    }
    let _ = writeln!(t, "    c: {} = {}({})", p.gives, p.callee, args.join(", "));
    let _ = writeln!(t, "    w: i64 = conv c");
    let _ = writeln!(t, "    store w, area, id, 8");
    let _ = writeln!(t, "    ret");
    Ok(t)
}

/// The CPU's build: the store's IR with an arena large enough for the
/// frame, and two functions of the called function's parameters: one
/// that calls it and gives how many items it made, which is what is
/// timed, and one that calls it and copies its items out, a word each,
/// for the runner to read back (a stream is four words and does not
/// cross the JIT's call)
fn native_ir(ir: &str, p: &KernelPlan) -> Result<String, String> {
    let heap = "data __heap: array(u8, 65536)";
    let init = "arena_init(a, h, 65536)";
    if !ir.contains(heap) || !ir.contains(init) {
        return Err("the store's IR has no arena where the kernel form looks for it".into());
    }
    let mut t = ir.replacen(heap, &format!("data __heap: array(u8, {})", ARENA), 1).replacen(init, &format!("arena_init(a, h, {})", ARENA), 1);
    let unread = if p.queue { "unread_queue" } else { "unread" };
    let e = &p.item;
    let params: Vec<String> = p.params.iter().map(|(n, ty)| format!("{}: {}", n, ty)).collect();
    let names: Vec<&str> = p.params.iter().map(|(n, _)| n.as_str()).collect();
    let (params, names, caller) = (params.join(", "), names.join(", "), &p.caller);
    let _ = write!(
        t,
        "\n; the kernel form (hop sixty-two): the frame made and counted, and made and copied out a word each\ndata __kernel_out: array(i64, {OUT})\nfn __kernel_count({params}) -> i64\n    x: {e}$ = {caller}({names})\n    v: {e}[] = {unread}(x)\n    n: index = len v\n    w: i64 = conv n\n    ret w\nfn __kernel_copy({params}) -> i64\n    x: {e}$ = {caller}({names})\n    v: {e}[] = {unread}(x)\n    n: index = len v\n    fits: u1 = cmp.le n, {OUT}\n    check fits\n    o: ptr = addr __kernel_out\n    loop(k: index = 0)\n        done: u1 = cmp.ge k, n\n        if done\n            break\n        y: {e} = load v, k\n        w: i64 = conv y\n        store w, o, k, 8\n        k2: index = add k, 1\n        continue k2\n    m: i64 = conv n\n    ret m\nfn __kernel_word(i: i64) -> i64\n    o: ptr = addr __kernel_out\n    w: i64 = load o, i, 8\n    ret w\n",
        OUT = OUT
    );
    Ok(t)
}

/// The CPU's frame: the called function on the JIT, in a child as every
/// call of the runner's is, and its milliseconds, the least of five calls
fn cpu_frame(module: &ssa::Module, p: &KernelPlan) -> Result<(Vec<i64>, f64), String> {
    let enc = emit::Encoder::load("targets/arm64.encodings.json")?;
    let jit = emit::compile(module, &enc).and_then(|c| emit::jit::JitCode::new(&c))?;
    let args: Vec<i64> = p.params.iter().zip(&p.values).map(|((_, ty), v)| native_word(*v, ty)).collect();
    let reset = || -> Result<(), String> {
        if module.func("__zero_context").is_some() {
            jit.call("__zero_context", &[0])?;
        }
        if module.func("__zero_reset").is_some() {
            jit.call("__zero_reset", &[])?;
        }
        if module.func("__zero_start").is_some() {
            jit.call("__zero_start", &[])?;
        }
        Ok(())
    };
    let got = suite::forked(
        || {
            // (the least of as many calls as the GPU's dispatches)
            let mut ns = i64::MAX;
            for _ in 0..TIMES {
                reset()?;
                let t0 = Instant::now();
                jit.call("__kernel_count", &args)?;
                ns = ns.min(t0.elapsed().as_nanos() as i64);
            }
            reset()?;
            let n = jit.call("__kernel_copy", &args)?;
            let mut values = vec![ns];
            for k in 0..n {
                values.push(jit.call("__kernel_word", &[k])?);
            }
            Ok(suite::Got { values, text: String::new(), marks: Vec::new() })
        },
        Some(&jit),
    )
    .map_err(|e| if e == suite::checked() { format!("the CPU's '{}' failed a check: a frame larger than the kernel form's arena of {} MB or its {} words, or a check of the program's", p.caller_said, ARENA >> 20, OUT) } else { e })?;
    Ok((got.values[1..].to_vec(), got.values[0] as f64 / 1e6))
}

/// the driver's `area: [...]` and `time: ... ms` lines
fn read_driver(stdout: &str, stderr: &str, ok: bool) -> Result<(Vec<i64>, f64), String> {
    let area = stdout.lines().find_map(|l| l.trim().strip_prefix("area: [").and_then(|t| t.strip_suffix(']')));
    let time = stdout.lines().find_map(|l| l.trim().strip_prefix("time: ").and_then(|t| t.strip_suffix(" ms")).and_then(|t| t.parse::<f64>().ok()));
    match (area, ok) {
        (Some(words), true) => {
            let all: Vec<i64> = words.split(',').map(|w| w.trim().parse::<i64>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
            Ok((all, time.unwrap_or(f64::NAN)))
        }
        _ => {
            let why: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
            Err(format!("the driver: {}", if why.is_empty() { "no output".to_string() } else { why.join(" | ") }))
        }
    }
}

/// The GPU's frame: the kernel built under the air platform's policy,
/// printed as WGSL and run in headless Chrome, or compiled to a metallib
/// and run through Metal; `n` threads. The WGSL text and its layout are
/// left in `target/wgsl/<store>-kernel.wgsl`, the metallib in
/// `target/air/`
fn gpu_frame(ir: &str, s: &store::Store, name: &str, n: usize, gpu: Gpu, level: usize) -> Result<(Vec<i64>, f64, f64, usize), String> {
    let platform = crate::platform::Platform::load("air")?;
    let policy = run::store_policy(s, &crate::host::backend_policy(suite::Backend::Air)?);
    let module = run::build(ir, &policy, level)?;
    let (cmd_out, lines) = match gpu {
        Gpu::Wgsl => {
            let c = crate::emit_wgsl::compile(&module, &platform)?;
            let dir = Path::new("target/wgsl");
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let text = dir.join(format!("{}-kernel.wgsl", name));
            let json = dir.join(format!("{}-kernel.wgsl.json", name));
            std::fs::write(&text, &c.text).map_err(|e| e.to_string())?;
            let data_hex: String = c.layout.data.iter().map(|b| format!("{:02x}", b)).collect();
            std::fs::write(&json, format!("{{\"data\":\"{}\",\"slab\":{},\"kernel\":true}}\n", data_hex, c.layout.slab)).map_err(|e| e.to_string())?;
            let t0 = Instant::now();
            let out = Command::new("node").arg("tools/driver_webgpu.js").arg("--kernel").arg(&text).arg(&json).arg(n.to_string()).env("PROBE_TIME", TIMES.to_string()).output().map_err(|e| format!("node: {}", e))?;
            ((out, t0.elapsed().as_secs_f64() * 1e3), c.text.lines().count())
        }
        Gpu::Air => {
            let c = crate::emit_air::compile_with(&module, &platform)?;
            if !c.has_kernel {
                return Err(format!("the kernel was left out of the metallib: {:?}", c.skipped));
            }
            let dir = Path::new("target/air");
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let lib = dir.join(format!("{}-kernel.metallib", name));
            let json = dir.join(format!("{}-kernel.air.json", name));
            std::fs::write(&lib, &c.metallib).map_err(|e| e.to_string())?;
            let data_hex: String = c.layout.data.iter().map(|b| format!("{:02x}", b)).collect();
            std::fs::write(&json, format!("{{\"data\":\"{}\",\"slab\":{},\"kernel\":true}}", data_hex, c.layout.slab)).map_err(|e| e.to_string())?;
            let t0 = Instant::now();
            let out = Command::new("python3").args(["tools/driver_metal.py", "--kernel"]).arg(&lib).arg(&json).arg(n.to_string()).env("PROBE_TIME", TIMES.to_string()).output().map_err(|e| format!("python3: {}", e))?;
            ((out, t0.elapsed().as_secs_f64() * 1e3), 0)
        }
    };
    let (out, whole) = cmd_out;
    let (words, dispatch) = read_driver(&String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr), out.status.success())?;
    Ok((words, dispatch, whole, lines))
}

/// `probe zero <store> kernel '<call>' --wgsl|--air`: the call's frame
/// on the GPU and on the CPU, compared word for word
pub fn compare(dir: &Path, text: &str, gpu: Gpu, policy: &ssa::Policy, level: usize) -> Result<Compared, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let p = lower::kernel_plan(&s, &l, text)?;
    let native = run::build(&native_ir(&l.ir, &p)?, &run::store_policy(&s, policy), level)?;
    let (cpu, cpu_ms) = cpu_frame(&native, &p)?;
    if cpu.is_empty() {
        return Err(format!("{}: the CPU's frame has no items, and a kernel of no threads runs nothing", text.trim()));
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "store".into());
    let (got, dispatch_ms, whole_ms, wgsl_lines) = gpu_frame(&format!("{}{}", l.ir, kernel_ir(&p)?), &s, &name, cpu.len(), gpu, level)?;
    let first_differing = (0..cpu.len()).find(|&k| got.get(k) != Some(&cpu[k])).map(|k| (k, got.get(k).copied().unwrap_or(0), cpu[k]));
    let same = (0..cpu.len()).filter(|&k| got.get(k) == Some(&cpu[k])).count();
    Ok(Compared { call: text.trim().to_string(), gpu, words: got.len(), first_differing, same, dispatch_ms, whole_ms, cpu_ms, wgsl_lines })
}

#[cfg(test)]
mod tests {
    /// the triangle's frame at 16 a side made by its kernel on WebGPU in
    /// headless Chrome, and the same as the CPU's loop, word for word
    #[test]
    fn triangle_on_webgpu_is_the_cpus() {
        let policy = crate::host::backend_policy(crate::suite::Backend::Native).unwrap();
        let c = super::compare(std::path::Path::new("suite/zero/triangle"), "frame (16, 0.5)", super::Gpu::Wgsl, &policy, crate::opt::MAX_LEVEL).expect("the kernel form runs");
        assert_eq!((c.words, c.same, c.first_differing), (256, 256, None), "\n{}", c.said());
    }
}
