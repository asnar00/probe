//! WGSL: the SSA as text for WebGPU, out of the same pruned module the AIR
//! emitter turns into Metal bitcode, so that the same data image, slab and
//! area the Metal driver builds run unchanged (tools/driver_webgpu.js runs
//! it in headless Chrome). What it shares with AIR, it calls there: which
//! functions recursion leaves out, what the kernel cannot reach, the
//! kernel's signature, which values are threadgroup addresses.
//!
//! What WGSL has and the SSA does not, and how it is given:
//!
//! - one memory, as on AIR: `mem`, a `var<storage, read_write>` array of
//!   32-bit words, pointers offsets into it in bytes; the driver's
//!   parameters a uniform whose first two words are the area's offset. A
//!   32- or 64-bit access takes the word at offset / 4 (the IR's data and
//!   scratch are laid out aligned to their sizes); a load of 8 or 16 bits
//!   is the byte or half shifted out of its word. A store narrower than a
//!   word is refused: it would be a read and a write of a word another
//!   thread may be writing the rest of.
//! - no 64-bit integer. An `i64`, a `u64`, a pointer, any integer of 33
//!   to 64 bits is a pair of words, `vec2<u32>`, low first, the high word
//!   extended as the type's canonical form says; add and subtract carry,
//!   multiply is the low word's 32 x 32 product plus the cross terms,
//!   shifts and comparisons are written out (the `w_` functions at the
//!   head of a module, printed when used), the bitwise three are the
//!   vector's own. `div` and `rem` on a pair are refused by name. An
//!   address is computed in `u32`: memory is under 4 GiB.
//! - an integer of 2 to 32 bits is an `i32` or a `u32`, normalized as
//!   AIR's containers are; a `u1` is a `bool`.
//! - floats are their bits, as on AIR: a platform rule (targets/air.platform,
//!   the same file) is printed as WGSL's operation, its arguments
//!   `bitcast<f32>` and its result `bitcast<u32>`. WGSL has no f64, and
//!   f16 needs a feature this path does not ask for: both refused.
//! - threadgroups: a `group` item is in one `var<workgroup>` array of words,
//!   `group_sync()` is `workgroupBarrier()`. The workgroup's size is an
//!   `override`, set per dispatch from the directive's group.
//! - the simdgroup: the `simd_*` rules are subgroup operations behind
//!   `enable subgroups;`, the thread's index in its subgroup and the width
//!   left in its header as AIR leaves them.
//! - structure: WGSL has `loop`, `if` and a `break` that leaves one loop.
//!   The walk is the wasm emitter's over `Cfg` (Ramsey's): a loop header a
//!   `loop`, a merge a region that ends where the merge's code begins; a
//!   branch to a merge at the end of an `if` arm falls out of it, a back
//!   edge to the innermost loop is `continue`, a branch out of the
//!   innermost loop to the merge right after it is `break`, and any other
//!   branch sets `brk` to its target and leaves, each region and loop it
//!   crosses guarded by it. Block parameters are `var`s, values `let`s.
//! - recursion, indirect calls and vectors: refused by name.

use crate::emit_air::{self, Layout, HEADER};
use crate::platform::{Natives, Platform};
use crate::ssa::{BinOp, CastOp, Cond, Function, Inst, Module, Type, ValueId};
use crate::structure::Cfg;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

pub struct Compiled {
    pub text: String,
    pub layout: Layout,
    /// functions left out, with why
    pub skipped: Vec<(String, String)>,
    /// does the text need the device's `subgroups`?
    pub subgroups: bool,
}

/// how a value is held
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum W {
    Bool,
    I32,
    U32,
    Pair,
}

impl W {
    fn name(self) -> &'static str {
        match self {
            W::Bool => "bool",
            W::I32 => "i32",
            W::U32 => "u32",
            W::Pair => "vec2<u32>",
        }
    }
}

fn wty(f: &Function, t: Type) -> Result<W, String> {
    if f.vector(t).is_some() {
        return Err(format!("not yet printed for WGSL: a vector, {}", f.tyname(t)));
    }
    match t {
        Type::Ptr | Type::TPtr(_) | Type::Fn(_) => Ok(W::Pair),
        Type::Int { .. } | Type::Pack(_) => {
            let r = f.repr(t);
            match r.bits() {
                1 if !r.signed() => Ok(W::Bool),
                1 => Err("not yet printed for WGSL: i1".into()),
                2..=32 => Ok(if r.signed() { W::I32 } else { W::U32 }),
                33..=64 => Ok(W::Pair),
                _ => Err(format!("not printed for WGSL: {} (wider than a word pair)", f.tyname(t))),
            }
        }
        _ => Err(format!("not printed for WGSL: a value of type {}", f.tyname(t))),
    }
}

/// the pair arithmetic, printed once each where used: (name, needs, text)
const HELPERS: &[(&str, &[&str], &str)] = &[
    ("w_add", &[], "fn w_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {\n    let lo = a.x + b.x;\n    return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x));\n}\n"),
    ("w_sub", &[], "fn w_sub(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {\n    return vec2<u32>(a.x - b.x, a.y - b.y - select(0u, 1u, a.x < b.x));\n}\n"),
    (
        "w_mul32",
        &[],
        "// the whole product of two words, from their halves\nfn w_mul32(a: u32, b: u32) -> vec2<u32> {\n    let a0 = a & 0xffffu;\n    let a1 = a >> 16u;\n    let b0 = b & 0xffffu;\n    let b1 = b >> 16u;\n    let p00 = a0 * b0;\n    let p01 = a0 * b1;\n    let p10 = a1 * b0;\n    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);\n    return vec2<u32>((p00 & 0xffffu) | (mid << 16u), a1 * b1 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));\n}\n",
    ),
    ("w_mul", &["w_mul32"], "fn w_mul(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {\n    let p = w_mul32(a.x, b.x);\n    return vec2<u32>(p.x, p.y + a.x * b.y + a.y * b.x);\n}\n"),
    (
        "w_shl",
        &[],
        "// shifts take the amount mod 64, as the IR's do\nfn w_shl(a: vec2<u32>, n: u32) -> vec2<u32> {\n    let k = n & 63u;\n    if k == 0u {\n        return a;\n    }\n    if k < 32u {\n        return vec2<u32>(a.x << k, (a.y << k) | (a.x >> (32u - k)));\n    }\n    return vec2<u32>(0u, a.x << (k - 32u));\n}\n",
    ),
    (
        "w_shr",
        &[],
        "fn w_shr(a: vec2<u32>, n: u32) -> vec2<u32> {\n    let k = n & 63u;\n    if k == 0u {\n        return a;\n    }\n    if k < 32u {\n        return vec2<u32>((a.x >> k) | (a.y << (32u - k)), a.y >> k);\n    }\n    return vec2<u32>(a.y >> (k - 32u), 0u);\n}\n",
    ),
    (
        "w_sar",
        &[],
        "fn w_sar(a: vec2<u32>, n: u32) -> vec2<u32> {\n    let k = n & 63u;\n    let hi = bitcast<i32>(a.y);\n    if k == 0u {\n        return a;\n    }\n    if k < 32u {\n        return vec2<u32>((a.x >> k) | (a.y << (32u - k)), bitcast<u32>(hi >> k));\n    }\n    return vec2<u32>(bitcast<u32>(hi >> (k - 32u)), bitcast<u32>(hi >> 31u));\n}\n",
    ),
    ("w_ult", &[], "fn w_ult(a: vec2<u32>, b: vec2<u32>) -> bool {\n    return a.y < b.y || (a.y == b.y && a.x < b.x);\n}\n"),
    ("w_slt", &[], "fn w_slt(a: vec2<u32>, b: vec2<u32>) -> bool {\n    return bitcast<i32>(a.y) < bitcast<i32>(b.y) || (a.y == b.y && a.x < b.x);\n}\n"),
];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Label {
    Loop(usize),
    Block(usize),
    If,
}

impl Label {
    /// what `brk` holds for a branch to it
    fn id(self) -> usize {
        match self {
            Label::Block(b) => 2 * b + 1,
            Label::Loop(b) => 2 * b + 2,
            Label::If => 0,
        }
    }
}

/// a WGSL identifier from an SSA name: letters, digits and underscores,
/// never two underscores first (WGSL reserves those)
fn ident(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

struct Cx<'a> {
    module: &'a Module,
    natives: &'a Natives,
    out: &'a [bool],
    /// SSA function -> WGSL name
    names: HashMap<String, String>,
    data_offsets: HashMap<String, usize>,
    group_offsets: HashMap<String, i64>,
    scratch_of: Vec<i64>,
    helpers: BTreeSet<&'static str>,
    /// pairs of fewer than 64 bits made canonical: (signed, bits)
    ext: BTreeSet<(bool, u32)>,
    subgroups: bool,
    /// result structs, by WGSL function name: the struct's text
    structs: Vec<String>,
}

pub fn compile(module: &Module, platform: &Platform) -> Result<Compiled, String> {
    let natives = platform.natives(module);
    let (data, data_offsets) = crate::ssa::layout_data(module);
    let data_size = ((data.len() as u64) + 15) & !15;
    // no function values: what the kernel reaches by its calls, a
    // function value met there refused by name (below)
    let taken: Vec<String> = Vec::new();
    let (mut out, skipped) = emit_air::left_out(module, &taken);
    let kernel = module.funcs.iter().position(|f| f.name == "__kernel").ok_or("WGSL prints a module with a __kernel, and this one has none")?;
    let arity = emit_air::kernel_arity(module)?;
    if out[kernel] {
        let why = skipped.iter().find(|(n, _)| n == "__kernel").map(|(_, w)| w.clone()).unwrap_or_default();
        return Err(format!("the kernel is left out: {}", why.replace("Metal", "WGSL")));
    }
    for (i, u) in emit_air::unreachable_from_kernel(module, &taken, &natives).into_iter().enumerate() {
        out[i] |= u;
    }
    // per-thread scratch, as AIR has it: any call chain's frames fit in the sum of all
    let scratch_of: Vec<i64> = module.funcs.iter().map(|f| crate::emit::scratch_layout(f, 0).1).collect();
    let slab: u64 = scratch_of.iter().sum::<i64>() as u64 + 16 + HEADER;
    let mut group_offsets = HashMap::new();
    let mut group_bytes = 0u64;
    for d in module.data.iter().filter(|d| d.shared) {
        group_bytes = (group_bytes + 15) & !15;
        group_offsets.insert(d.name.clone(), group_bytes as i64);
        group_bytes += d.bytes.len() as u64;
    }
    group_bytes = (group_bytes + 15) & !15;
    let mut names = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for (i, f) in module.funcs.iter().enumerate() {
        if out[i] {
            continue;
        }
        let base = if f.name == "__kernel" { "p_kernel".to_string() } else { format!("p_{}", ident(&f.name)) };
        let mut name = base.clone();
        let mut k = 2;
        while used.contains(&name) {
            name = format!("{}_{}", base, k);
            k += 1;
        }
        used.insert(name.clone());
        names.insert(f.name.clone(), name);
    }
    let mut cx = Cx { module, natives: &natives, out: &out, names, data_offsets, group_offsets, scratch_of, helpers: BTreeSet::new(), ext: BTreeSet::new(), subgroups: false, structs: Vec::new() };
    let mut bodies = String::new();
    for (i, f) in module.funcs.iter().enumerate() {
        if out[i] {
            continue;
        }
        let text = cx.function(f, i).map_err(|e| format!("{}: {}", f.name, e))?;
        bodies.push_str(&text);
        bodies.push('\n');
    }

    let mut t = String::new();
    t.push_str("// printed by probe (src/emit_wgsl.rs): the program's kernel for WebGPU.\n// buffer 0 is the program's memory, its data image, then a slab per\n// thread, then the area; the uniform's first two words are the area's\n// offset (tools/driver_webgpu.js lays them out as tools/driver_metal.py does)\n");
    if cx.subgroups {
        t.push_str("enable subgroups;\n");
    }
    t.push('\n');
    t.push_str("@group(0) @binding(0) var<storage, read_write> mem: array<u32>;\n");
    t.push_str("@group(0) @binding(1) var<uniform> params: vec4<u32>;\n");
    if group_bytes > 0 {
        let _ = writeln!(t, "var<workgroup> grp: array<u32, {}>;", group_bytes / 4);
    }
    t.push_str("override group_size: u32 = 64u;\n\n");
    // the helpers, each after what it needs
    let mut printed: Vec<&str> = Vec::new();
    fn add(name: &'static str, printed: &mut Vec<&'static str>, t: &mut String) {
        if printed.contains(&name) {
            return;
        }
        let (_, needs, text) = HELPERS.iter().find(|(n, _, _)| *n == name).unwrap();
        for &d in needs.iter() {
            add(d, printed, t);
        }
        printed.push(name);
        t.push_str(text);
        t.push('\n');
    }
    for &h in &cx.helpers.clone() {
        add(h, &mut printed, &mut t);
    }
    for &(signed, bits) in &cx.ext {
        if signed {
            let _ = writeln!(t, "fn w_sx{}(a: vec2<u32>) -> vec2<u32> {{\n    return vec2<u32>(a.x, bitcast<u32>((bitcast<i32>(a.y) << {}u) >> {}u));\n}}\n", bits, 64 - bits, 64 - bits);
        } else {
            let _ = writeln!(t, "fn w_zx{}(a: vec2<u32>) -> vec2<u32> {{\n    return vec2<u32>(a.x, a.y & 0x{:x}u);\n}}\n", bits, (1u64 << (bits - 32)) - 1);
        }
    }
    for s in &cx.structs {
        t.push_str(s);
        t.push('\n');
    }
    t.push_str(&bodies);
    // the entry: the header (the thread's index in its subgroup and the
    // width), then the kernel's body with the slab after the header
    let sg = if cx.subgroups { ", @builtin(subgroup_invocation_id) sg_id: u32, @builtin(subgroup_size) sg_size: u32" } else { "" };
    let _ = writeln!(t, "@compute @workgroup_size(group_size)");
    let _ = writeln!(t, "fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>{}) {{", sg);
    let _ = writeln!(t, "    let id = gid.x;");
    let _ = writeln!(t, "    let tls = {}u + id * {}u;", data_size, slab);
    let _ = writeln!(t, "    let h = tls >> 2u;");
    if cx.subgroups {
        let _ = writeln!(t, "    mem[h] = sg_id;\n    mem[h + 1u] = 0u;\n    mem[h + 2u] = sg_size;\n    mem[h + 3u] = 0u;");
    } else {
        // no subgroup operation reads them: a thread is its own subgroup
        let _ = writeln!(t, "    mem[h] = 0u;\n    mem[h + 1u] = 0u;\n    mem[h + 2u] = 1u;\n    mem[h + 3u] = 0u;");
    }
    let mut args = format!("tls + {}u, tls, vec2<u32>(0u, 0u), params.xy, vec2<u32>(id, 0u)", HEADER);
    if arity == 5 {
        args.push_str(", vec2<u32>(lid.x, 0u), vec2<u32>(wid.x, 0u)");
    }
    let _ = writeln!(t, "    p_kernel({});", args);
    t.push_str("}\n");
    Ok(Compiled { text: t, layout: Layout { data, slab }, skipped, subgroups: cx.subgroups })
}

/// a function's body being printed
struct Fx<'b> {
    f: &'b Function,
    cfg: Cfg,
    out: String,
    depth: usize,
    ctx: Vec<Label>,
    /// value -> its name, or the literal it is
    vals: HashMap<u32, String>,
    /// the values that are constants, canonical
    lits: HashMap<u32, i64>,
    /// labels whose region or loop a branch through `brk` crosses or
    /// targets: from the first printing, guarded in the second
    guards: HashSet<Label>,
    found: HashSet<Label>,
    brk: bool,
    tmp: usize,
    group_vals: HashSet<u32>,
    scratch: HashMap<ValueId, i64>,
    my_scratch: i64,
}

impl Fx<'_> {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn val(&self, v: ValueId) -> Result<String, String> {
        self.vals.get(&v.0).cloned().ok_or_else(|| format!("value {} used before it is defined", self.f.value(v).name))
    }
    fn fresh(&mut self, stem: &str) -> String {
        self.tmp += 1;
        format!("{}_t{}", stem, self.tmp)
    }
}

fn vname(f: &Function, v: ValueId) -> String {
    let s = ident(&f.value(v).name);
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) { format!("v_{}", v.0) } else { format!("{}_{}", s, v.0) }
}

/// a literal of a held type, from the canonical value
fn literal(w: W, v: i64) -> String {
    match w {
        W::Bool => (if v & 1 != 0 { "true" } else { "false" }).to_string(),
        W::I32 => {
            let x = v as i32;
            if x == i32::MIN { "bitcast<i32>(0x80000000u)".to_string() } else { format!("{}i", x) }
        }
        W::U32 => {
            let x = v as u32;
            if x < 65536 { format!("{}u", x) } else { format!("0x{:x}u", x) }
        }
        W::Pair => {
            let (lo, hi) = (v as u64 as u32, ((v as u64) >> 32) as u32);
            let w = |x: u32| if x < 65536 { format!("{}u", x) } else { format!("0x{:x}u", x) };
            format!("vec2<u32>({}, {})", w(lo), w(hi))
        }
    }
}

impl Cx<'_> {
    fn need(&mut self, h: &'static str) {
        self.helpers.insert(h);
    }

    /// an expression of type t back in its canonical form
    fn norm(&mut self, f: &Function, e: String, t: Type) -> Result<String, String> {
        let w = wty(f, t)?;
        let r = f.repr(t);
        let bits = r.bits();
        Ok(match w {
            W::Bool => e,
            W::I32 if bits < 32 => format!("(({}) << {}u) >> {}u", e, 32 - bits, 32 - bits),
            W::U32 if bits < 32 => format!("({}) & 0x{:x}u", e, (1u64 << bits) - 1),
            W::Pair if bits < 64 => {
                self.ext.insert((r.signed(), bits));
                format!("w_{}x{}({})", if r.signed() { "s" } else { "z" }, bits, e)
            }
            _ => e,
        })
    }

    fn function(&mut self, f: &Function, fi: usize) -> Result<String, String> {
        let cfg = Cfg::analyze(f).ok_or("not printed for WGSL: an irreducible graph")?;
        if !f.blocks[0].params.is_empty() {
            return Err("not printed for WGSL: an entry block with parameters".into());
        }
        let (scratch, _) = crate::emit::scratch_layout(f, 0);
        let group_vals = emit_air::group_addresses(f, &self.group_offsets)?;
        let mut vals = HashMap::new();
        for &p in &f.params {
            vals.insert(p.0, vname(f, p));
        }
        for b in &f.blocks {
            for &p in &b.params {
                vals.insert(p.0, vname(f, p));
            }
        }
        let mut fx = Fx { f, cfg, out: String::new(), depth: 1, ctx: Vec::new(), vals: vals.clone(), lits: HashMap::new(), guards: HashSet::new(), found: HashSet::new(), brk: false, tmp: 0, group_vals, scratch, my_scratch: self.scratch_of[fi] };
        // twice: the first printing finds which regions a branch through
        // `brk` crosses, the second guards them
        self.do_tree(&mut fx, 0)?;
        let found = std::mem::take(&mut fx.found);
        fx.guards = found;
        fx.out.clear();
        fx.vals = vals;
        fx.lits.clear();
        fx.tmp = 0;
        fx.brk = false;
        self.do_tree(&mut fx, 0)?;
        debug_assert_eq!(fx.found, fx.guards);

        let wname = self.names[&f.name].clone();
        let mut params = vec!["slab: u32".to_string(), "tls: u32".to_string()];
        for &p in &f.params {
            params.push(format!("{}: {}", vname(f, p), wty(f, f.ty(p))?.name()));
        }
        let ret = match f.rets.len() {
            0 => String::new(),
            1 => format!(" -> {}", wty(f, f.rets[0])?.name()),
            _ => {
                let fields: Vec<String> = f.rets.iter().enumerate().map(|(i, &t)| wty(f, t).map(|w| format!("    r{}: {},", i, w.name()))).collect::<Result<_, _>>()?;
                self.structs.push(format!("struct R_{} {{\n{}\n}}\n", wname, fields.join("\n")));
                format!(" -> R_{}", wname)
            }
        };
        let mut s = format!("fn {}({}){} {{\n", wname, params.join(", "), ret);
        for b in &f.blocks[1..] {
            for &p in &b.params {
                let _ = writeln!(s, "    var {}: {};", vname(f, p), wty(f, f.ty(p))?.name());
            }
        }
        if fx.brk {
            s.push_str("    var brk: u32 = 0u;\n");
        }
        s.push_str(&fx.out);
        s.push_str("}\n");
        Ok(s)
    }

    /// block x and everything it dominates (structure.rs; the wasm
    /// emitter's `do_tree`)
    fn do_tree(&mut self, fx: &mut Fx, x: usize) -> Result<(), String> {
        let merges: Vec<usize> = fx.cfg.dom_children[x].iter().copied().filter(|&c| fx.cfg.merge[c]).collect();
        if fx.cfg.loop_header[x] {
            fx.line("loop {");
            fx.depth += 1;
            fx.ctx.push(Label::Loop(x));
            self.node_within(fx, x, &merges)?;
            if fx.guards.contains(&Label::Loop(x)) {
                // a branch through `brk` fell to the end of the body: this
                // loop's turn again, or on outwards
                fx.line(&format!("if brk == {}u {{", Label::Loop(x).id()));
                fx.line("    brk = 0u;");
                fx.line("    continue;");
                fx.line("}");
                fx.line("break;");
            }
            fx.ctx.pop();
            fx.depth -= 1;
            fx.line("}");
            Ok(())
        } else {
            self.node_within(fx, x, &merges)
        }
    }

    /// `merges` in rpo order: the last is the outermost region
    fn node_within(&mut self, fx: &mut Fx, x: usize, merges: &[usize]) -> Result<(), String> {
        if let Some((&y, rest)) = merges.split_last() {
            fx.ctx.push(Label::Block(y));
            self.node_within(fx, x, rest)?;
            fx.ctx.pop();
            if fx.guards.contains(&Label::Block(y)) {
                fx.line(&format!("if brk == {}u {{", Label::Block(y).id()));
                fx.line("    brk = 0u;");
                fx.line("}");
                fx.line("if brk == 0u {");
                fx.depth += 1;
                self.do_tree(fx, y)?;
                fx.depth -= 1;
                fx.line("}");
                return Ok(());
            }
            return self.do_tree(fx, y);
        }
        let f = fx.f;
        let block = &f.blocks[x];
        let (last, body) = block.insts.split_last().ok_or("an empty block")?;
        for inst in body {
            self.inst(fx, inst)?;
        }
        match last {
            Inst::Jmp { target, args } => self.do_branch(fx, target.0 as usize, args),
            Inst::Br { cond, then_target, then_args, else_target, else_args } => {
                let c = fx.val(*cond)?;
                let at = fx.out.len();
                fx.line(&format!("if {} {{", c));
                fx.depth += 1;
                fx.ctx.push(Label::If);
                self.do_branch(fx, then_target.0 as usize, then_args)?;
                fx.ctx.pop();
                fx.depth -= 1;
                let mid = fx.out.len();
                fx.line("} else {");
                fx.depth += 1;
                fx.ctx.push(Label::If);
                self.do_branch(fx, else_target.0 as usize, else_args)?;
                fx.ctx.pop();
                fx.depth -= 1;
                let end = fx.out.len();
                let else_line = fx.out[mid..].lines().next().map_or(0, |l| l.len() + 1);
                if end == mid + else_line {
                    // an empty else
                    fx.out.truncate(mid);
                } else if fx.out[at..mid].lines().count() == 1 {
                    // an empty then: the condition turned round
                    let rest = fx.out[mid + else_line..].to_string();
                    fx.out.truncate(at);
                    fx.line(&format!("if !{} {{", c));
                    fx.out.push_str(&rest);
                }
                fx.line("}");
                Ok(())
            }
            Inst::Ret { vals } => {
                match vals.len() {
                    0 => fx.line("return;"),
                    1 => {
                        let v = fx.val(vals[0])?;
                        fx.line(&format!("return {};", v));
                    }
                    _ => {
                        let vs: Vec<String> = vals.iter().map(|&v| fx.val(v)).collect::<Result<_, _>>()?;
                        let wname = &self.names[&f.name];
                        fx.line(&format!("return R_{}({});", wname, vs.join(", ")));
                    }
                }
                Ok(())
            }
            other => Err(format!("a block ending in {:?}", other)),
        }
    }

    /// a branch to block t: its parameters set, then a fall out of an arm,
    /// `continue`, `break`, the block itself printed here, or `brk`
    fn do_branch(&mut self, fx: &mut Fx, t: usize, args: &[ValueId]) -> Result<(), String> {
        let f = fx.f;
        let params = f.blocks[t].params.clone();
        // the parameters set all at once: through temporaries when an
        // argument is another of the same block's parameters
        let pairs: Vec<(ValueId, ValueId)> = params.iter().copied().zip(args.iter().copied()).filter(|(p, a)| p != a).collect();
        let clash = pairs.iter().any(|(_, a)| params.contains(a));
        let mut staged = Vec::new();
        for &(p, a) in &pairs {
            let v = fx.val(a)?;
            if clash {
                let tmp = fx.fresh(&vname(f, p));
                fx.line(&format!("let {} = {};", tmp, v));
                staged.push((p, tmp));
            } else {
                staged.push((p, v));
            }
        }
        for (p, v) in staged {
            let name = vname(f, p);
            fx.line(&format!("{} = {};", name, v));
        }
        let label = if fx.cfg.loop_header[t] && fx.ctx.contains(&Label::Loop(t)) {
            Label::Loop(t)
        } else if fx.cfg.merge[t] {
            Label::Block(t)
        } else {
            return self.do_tree(fx, t);
        };
        let at = fx.ctx.iter().rposition(|l| *l == label).ok_or_else(|| format!("no region for block {} in scope", f.blocks[t].name))?;
        // what lies between here and the label, innermost first
        let crossed: Vec<Label> = fx.ctx[at + 1..].iter().rev().copied().filter(|l| *l != Label::If).collect();
        let loops = crossed.iter().filter(|l| matches!(l, Label::Loop(_))).count();
        match label {
            Label::Block(_) if crossed.is_empty() => {}
            Label::Block(_) if crossed.len() == 1 && loops == 1 => fx.line("break;"),
            Label::Loop(_) if loops == 0 => fx.line("continue;"),
            _ => {
                fx.brk = true;
                fx.line(&format!("brk = {}u;", label.id()));
                if matches!(crossed[0], Label::Loop(_)) {
                    fx.line("break;");
                } else {
                    fx.found.insert(crossed[0]);
                }
                for &l in &crossed[1..] {
                    fx.found.insert(l);
                }
                fx.found.insert(label);
            }
        }
        Ok(())
    }

    fn def(&mut self, fx: &mut Fx, dst: ValueId, e: String) -> Result<(), String> {
        let f = fx.f;
        let name = vname(f, dst);
        let w = wty(f, f.ty(dst))?;
        fx.line(&format!("let {}: {} = {};", name, w.name(), e));
        fx.vals.insert(dst.0, name);
        Ok(())
    }

    fn defn(&mut self, fx: &mut Fx, dst: ValueId, e: String) -> Result<(), String> {
        let t = fx.f.ty(dst);
        let e = self.norm(fx.f, e, t)?;
        self.def(fx, dst, e)
    }

    /// an index or offset as a u32
    fn as_u32(&self, fx: &Fx, v: ValueId) -> Result<String, String> {
        if let Some(&k) = fx.lits.get(&v.0) {
            return Ok(literal(W::U32, k));
        }
        let e = fx.val(v)?;
        Ok(match wty(fx.f, fx.f.ty(v))? {
            W::Pair => format!("{}.x", e),
            W::I32 => format!("bitcast<u32>({})", e),
            W::U32 => e,
            W::Bool => format!("select(0u, 1u, {})", e),
        })
    }

    /// the byte offset of an access, as a u32 `let`
    fn address(&mut self, fx: &mut Fx, addr: ValueId, off: i64, index: Option<(ValueId, u32)>) -> Result<String, String> {
        // the constant part summed, the rest as written
        let mut k = off as u32;
        let mut terms = Vec::new();
        match fx.lits.get(&addr.0) {
            Some(&b) => k = k.wrapping_add(b as u32),
            None => terms.push(format!("{}.x", fx.val(addr)?)),
        }
        if let Some((i, step)) = index {
            match fx.lits.get(&i.0) {
                Some(&c) => k = k.wrapping_add((c as u32).wrapping_mul(step)),
                None => {
                    let iv = self.as_u32(fx, i)?;
                    terms.push(if step == 1 { iv } else { format!("{} * {}u", iv, step) });
                }
            }
        }
        let mut e = terms.join(" + ");
        if e.is_empty() {
            e = format!("{}u", k);
        } else if (k as i32) > 0 {
            e = format!("{} + {}u", e, k);
        } else if (k as i32) < 0 {
            e = format!("{} - {}u", e, (k as i32).unsigned_abs());
        }
        let name = fx.fresh("at");
        fx.line(&format!("let {}: u32 = {};", name, e));
        Ok(name)
    }

    fn inst(&mut self, fx: &mut Fx, inst: &Inst) -> Result<(), String> {
        let f = fx.f;
        match inst {
            Inst::IConst { dst, imm } => {
                let t = f.ty(*dst);
                let v = crate::opt::norm(f.repr(t), *imm as i64);
                fx.vals.insert(dst.0, literal(wty(f, t)?, v));
                fx.lits.insert(dst.0, v);
            }
            Inst::Addr { dst, name } => {
                let off = match self.group_offsets.get(name.as_str()) {
                    Some(&g) => g,
                    None => *self.data_offsets.get(name.as_str()).ok_or_else(|| format!("no data named {}", name))? as i64,
                };
                fx.vals.insert(dst.0, literal(W::Pair, off));
                fx.lits.insert(dst.0, off);
            }
            Inst::Platform { dst, name } => {
                let v = *self.natives.consts.get(name).ok_or_else(|| format!("the platform has no constant '{}'", name))?;
                let t = f.ty(*dst);
                fx.vals.insert(dst.0, literal(wty(f, t)?, v));
                fx.lits.insert(dst.0, v);
            }
            Inst::Scratch { dst, .. } => {
                let off = fx.scratch[dst];
                self.def(fx, *dst, format!("vec2<u32>(slab + {}u, 0u)", off))?;
            }
            Inst::Bin { op, dst, lhs, rhs } => {
                let t = f.ty(*dst);
                let w = wty(f, t)?;
                let signed = f.repr(t).signed();
                let (a, b) = (fx.val(*lhs)?, fx.val(*rhs)?);
                let opname = format!("{:?}", op).to_lowercase();
                let e = match w {
                    W::Bool => match op {
                        BinOp::IAdd | BinOp::ISub | BinOp::Xor => format!("{} != {}", a, b),
                        BinOp::And | BinOp::IMul => format!("{} & {}", a, b),
                        BinOp::Or => format!("{} | {}", a, b),
                        _ => return Err(format!("not yet printed for WGSL: `{}` on u1", opname)),
                    },
                    W::I32 | W::U32 => {
                        let sym = match op {
                            BinOp::IAdd => "+",
                            BinOp::ISub => "-",
                            BinOp::IMul => "*",
                            BinOp::Div => "/",
                            BinOp::Rem => "%",
                            BinOp::And => "&",
                            BinOp::Or => "|",
                            BinOp::Xor => "^",
                            BinOp::Shl => "<<",
                            BinOp::Shr => ">>",
                        };
                        if matches!(op, BinOp::Shl | BinOp::Shr) {
                            // the amount mod 32, as the IR has it
                            let amt = if w == W::I32 { format!("(bitcast<u32>({}) & 31u)", b) } else { format!("({} & 31u)", b) };
                            format!("{} {} {}", a, sym, amt)
                        } else {
                            format!("{} {} {}", a, sym, b)
                        }
                    }
                    W::Pair => match op {
                        BinOp::IAdd => {
                            self.need("w_add");
                            format!("w_add({}, {})", a, b)
                        }
                        BinOp::ISub => {
                            self.need("w_sub");
                            format!("w_sub({}, {})", a, b)
                        }
                        BinOp::IMul => {
                            self.need("w_mul");
                            format!("w_mul({}, {})", a, b)
                        }
                        BinOp::And => format!("{} & {}", a, b),
                        BinOp::Or => format!("{} | {}", a, b),
                        BinOp::Xor => format!("{} ^ {}", a, b),
                        BinOp::Shl => {
                            self.need("w_shl");
                            format!("w_shl({}, {})", a, self.as_u32(fx, *rhs)?)
                        }
                        BinOp::Shr => {
                            let h = if signed { "w_sar" } else { "w_shr" };
                            self.need(h);
                            format!("{}({}, {})", h, a, self.as_u32(fx, *rhs)?)
                        }
                        BinOp::Div | BinOp::Rem => return Err(format!("not yet printed for WGSL: `{}` on {}", opname, f.tyname(t))),
                    },
                };
                if matches!(op, BinOp::IAdd | BinOp::ISub | BinOp::IMul | BinOp::Div | BinOp::Rem | BinOp::Shl) {
                    self.defn(fx, *dst, e)?;
                } else {
                    self.def(fx, *dst, e)?;
                }
            }
            Inst::ICmp { cond, dst, lhs, rhs } => {
                let t = f.ty(*lhs);
                let w = wty(f, t)?;
                let signed = t.is_int() && f.repr(t).signed();
                let (a, b) = (fx.val(*lhs)?, fx.val(*rhs)?);
                let e = match w {
                    W::Bool => match cond {
                        Cond::Eq => format!("{} == {}", a, b),
                        Cond::Ne => format!("{} != {}", a, b),
                        Cond::Lt => format!("!{} & {}", a, b),
                        Cond::Gt => format!("{} & !{}", a, b),
                        Cond::Le => format!("!{} | {}", a, b),
                        Cond::Ge => format!("{} | !{}", a, b),
                    },
                    W::I32 | W::U32 => {
                        let sym = match cond {
                            Cond::Eq => "==",
                            Cond::Ne => "!=",
                            Cond::Lt => "<",
                            Cond::Le => "<=",
                            Cond::Gt => ">",
                            Cond::Ge => ">=",
                        };
                        format!("{} {} {}", a, sym, b)
                    }
                    W::Pair => {
                        let lt = if signed { "w_slt" } else { "w_ult" };
                        match cond {
                            Cond::Eq => format!("all({} == {})", a, b),
                            Cond::Ne => format!("any({} != {})", a, b),
                            _ => {
                                self.need(lt);
                                match cond {
                                    Cond::Lt => format!("{}({}, {})", lt, a, b),
                                    Cond::Gt => format!("{}({}, {})", lt, b, a),
                                    Cond::Le => format!("!{}({}, {})", lt, b, a),
                                    _ => format!("!{}({}, {})", lt, a, b),
                                }
                            }
                        }
                    }
                };
                self.def(fx, *dst, e)?;
            }
            Inst::Cast { op, dst, src } => {
                let (td, ts) = (f.ty(*dst), f.ty(*src));
                let (wd, ws) = (wty(f, td)?, wty(f, ts)?);
                let s = fx.val(*src)?;
                let src_signed = f.repr(ts).signed();
                let e = match (ws, wd) {
                    (W::Bool, W::Bool) => s,
                    (W::Bool, W::I32) => format!("select(0i, 1i, {})", s),
                    (W::Bool, W::U32) => format!("select(0u, 1u, {})", s),
                    (W::Bool, W::Pair) => format!("vec2<u32>(select(0u, 1u, {}), 0u)", s),
                    (W::I32, W::Bool) => format!("({} & 1i) != 0i", s),
                    (W::U32, W::Bool) => format!("({} & 1u) != 0u", s),
                    (W::Pair, W::Bool) => format!("({}.x & 1u) != 0u", s),
                    (W::I32, W::I32) | (W::U32, W::U32) | (W::Pair, W::Pair) => s,
                    (W::I32, W::U32) => format!("bitcast<u32>({})", s),
                    (W::U32, W::I32) => format!("bitcast<i32>({})", s),
                    (W::Pair, W::U32) => format!("{}.x", s),
                    (W::Pair, W::I32) => format!("bitcast<i32>({}.x)", s),
                    (W::I32, W::Pair) if *op == CastOp::Conv && src_signed => format!("vec2<u32>(bitcast<u32>({}), bitcast<u32>({} >> 31u))", s, s),
                    (W::I32, W::Pair) => format!("vec2<u32>(bitcast<u32>({}), 0u)", s),
                    (W::U32, W::Pair) => format!("vec2<u32>({}, 0u)", s),
                };
                let same = e == fx.val(*src)?;
                let e = self.norm(f, e, td)?;
                if same && e == fx.val(*src)? {
                    // the same word or words: the source under a new name
                    fx.vals.insert(dst.0, e);
                    if let Some(&k) = fx.lits.get(&src.0) {
                        fx.lits.insert(dst.0, k);
                    }
                } else {
                    self.def(fx, *dst, e)?;
                }
            }
            Inst::Pack { dst, args } => {
                let t = f.ty(*dst);
                if wty(f, t)? != W::U32 {
                    return Err(format!("not yet printed for WGSL: a pack of {} bits", f.width(t).unwrap_or(0)));
                }
                let p = f.pack(t).ok_or("pack of a non-pack")?.clone();
                let mut parts = Vec::new();
                for (k, &a) in args.iter().enumerate() {
                    let (off, fty) = (p.offsets[k], p.fields[k].1);
                    let fw = f.width(fty).unwrap_or(32);
                    let v = self.field_bits(fx, a)?;
                    let v = if fw < 32 { format!("({} & 0x{:x}u)", v, (1u64 << fw) - 1) } else { v };
                    parts.push(if off > 0 { format!("({} << {}u)", v, off) } else { v });
                }
                let e = if parts.is_empty() { "0u".to_string() } else { parts.join(" | ") };
                self.def(fx, *dst, e)?;
            }
            Inst::Unpack { dsts, src } => {
                for (k, &d) in dsts.iter().enumerate() {
                    let e = self.field_of(fx, *src, k as u32)?;
                    self.defn(fx, d, e)?;
                }
            }
            Inst::Get { dst, src, field } => {
                let e = self.field_of(fx, *src, *field)?;
                self.defn(fx, *dst, e)?;
            }
            Inst::Set { dst, src, field, val } => {
                let t = f.ty(*src);
                if wty(f, t)? != W::U32 {
                    return Err(format!("not yet printed for WGSL: a pack of {} bits", f.width(t).unwrap_or(0)));
                }
                let p = f.pack(t).ok_or("set on a non-pack")?.clone();
                let (off, fty) = (p.offsets[*field as usize], p.fields[*field as usize].1);
                let fw = f.width(fty).unwrap_or(32);
                let mask: u64 = if fw >= 32 { 0xffff_ffff } else { (1u64 << fw) - 1 };
                let hole = !((mask << off) as u32) & (if p.width >= 32 { u32::MAX } else { ((1u64 << p.width) - 1) as u32 });
                let s = fx.val(*src)?;
                let v = self.field_bits(fx, *val)?;
                let e = format!("({} & 0x{:x}u) | (({} & 0x{:x}u) << {}u)", s, hole, v, mask, off);
                self.def(fx, *dst, e)?;
            }
            Inst::Load { dst, addr, off, index } => {
                let t = f.ty(*dst);
                let w = wty(f, t)?;
                let bytes = (f.width(t).ok_or_else(|| format!("no memory width for {}", f.tyname(t)))?.max(8) / 8).next_power_of_two();
                let at = self.address(fx, *addr, *off, *index)?;
                let m = if fx.group_vals.contains(&addr.0) { "grp" } else { "mem" };
                let word = format!("{}[{} >> 2u]", m, at);
                let e = match bytes {
                    8 => format!("vec2<u32>({}, {}[({} >> 2u) + 1u])", word, m, at),
                    4 => match w {
                        W::I32 => format!("bitcast<i32>({})", word),
                        _ => word,
                    },
                    2 | 1 => {
                        let mask = if bytes == 2 { "0xffffu" } else { "0xffu" };
                        let shift = if bytes == 2 { format!("(({} & 2u) * 8u)", at) } else { format!("(({} & 3u) * 8u)", at) };
                        let u = format!("(({} >> {}) & {})", word, shift, mask);
                        match w {
                            W::Bool => format!("{} != 0u", u),
                            W::I32 => format!("bitcast<i32>({})", u),
                            _ => u,
                        }
                    }
                    _ => return Err(format!("not printed for WGSL: a load of {} bytes", bytes)),
                };
                // memory holds the container; what was put there by
                // something else may not be canonical
                self.defn(fx, *dst, e)?;
            }
            Inst::Store { val, addr, off, index } => {
                let t = f.ty(*val);
                let w = wty(f, t)?;
                let bits = f.width(t).ok_or_else(|| format!("no memory width for {}", f.tyname(t)))?;
                let bytes = (bits.max(8) / 8).next_power_of_two();
                if bytes < 4 {
                    return Err(format!("not yet printed for WGSL: a store of {} ({} bits): memory is 32-bit words, and a part of a word stored beside another thread's would race", f.tyname(t), bits));
                }
                let at = self.address(fx, *addr, *off, *index)?;
                let m = if fx.group_vals.contains(&addr.0) { "grp" } else { "mem" };
                let v = fx.val(*val)?;
                match (bytes, w) {
                    (8, _) => {
                        fx.line(&format!("{}[{} >> 2u] = {}.x;", m, at, v));
                        fx.line(&format!("{}[({} >> 2u) + 1u] = {}.y;", m, at, v));
                    }
                    (4, W::I32) => fx.line(&format!("{}[{} >> 2u] = bitcast<u32>({});", m, at, v)),
                    (4, _) => fx.line(&format!("{}[{} >> 2u] = {};", m, at, v)),
                    _ => return Err(format!("not printed for WGSL: a store of {} bytes", bytes)),
                }
            }
            Inst::PtrAdd { dst, base, off } => {
                let b = fx.val(*base)?;
                let o = fx.val(*off)?;
                let o = match wty(f, f.ty(*off))? {
                    W::Pair => o,
                    W::I32 => format!("vec2<u32>(bitcast<u32>({}), bitcast<u32>({} >> 31u))", o, o),
                    W::U32 => format!("vec2<u32>({}, 0u)", o),
                    W::Bool => return Err("not printed for WGSL: a u1 offset".into()),
                };
                self.need("w_add");
                self.def(fx, *dst, format!("w_add({}, {})", b, o))?;
            }
            Inst::FnAddr { name, .. } => return Err(format!("not yet printed for WGSL: a function value (`addr {}`)", name)),
            Inst::CallInd { .. } => return Err("not yet printed for WGSL: a call through a function value".into()),
            Inst::Call { dsts, callee, args } if self.natives.get(callee).is_some() => self.rule(fx, dsts, callee, args)?,
            Inst::Call { dsts, callee, args } => {
                if callee.starts_with("simd_") {
                    return Err(format!("{} has no rule for these types: its one-thread form would be wrong here", callee));
                }
                let Some(wname) = self.names.get(callee).cloned() else {
                    let i = self.module.funcs.iter().position(|g| &g.name == callee);
                    return Err(match i {
                        Some(i) if self.out[i] => format!("a call to {}, which recursion leaves out: WGSL has no call stack", callee),
                        _ => format!("a call to {}, which is not in this module", callee),
                    });
                };
                let mut a = vec![format!("slab + {}u", fx.my_scratch), "tls".to_string()];
                for &x in args {
                    a.push(fx.val(x)?);
                }
                let call = format!("{}({})", wname, a.join(", "));
                match dsts.len() {
                    0 => fx.line(&format!("{};", call)),
                    1 => self.def(fx, dsts[0], call)?,
                    _ => {
                        let r = fx.fresh("r");
                        fx.line(&format!("let {} = {};", r, call));
                        for (i, &d) in dsts.iter().enumerate() {
                            self.def(fx, d, format!("{}.r{}", r, i))?;
                        }
                    }
                }
            }
            Inst::Check { cond } => {
                let c = fx.val(*cond)?;
                fx.line(&format!("// check {}: the GPU does not stop at a failed check", c));
            }
            Inst::Jmp { .. } | Inst::Br { .. } | Inst::Ret { .. } => return Err("a terminator inside a block".into()),
        }
        Ok(())
    }

    /// a pack field's value as u32 bits
    fn field_bits(&self, fx: &Fx, v: ValueId) -> Result<String, String> {
        let e = fx.val(v)?;
        Ok(match wty(fx.f, fx.f.ty(v))? {
            W::Bool => format!("select(0u, 1u, {})", e),
            W::I32 => format!("bitcast<u32>({})", e),
            W::U32 => e,
            W::Pair => return Err("not yet printed for WGSL: a pack field of more than 32 bits".into()),
        })
    }

    /// field k of a pack held in a u32, as the field's held type
    fn field_of(&self, fx: &Fx, src: ValueId, k: u32) -> Result<String, String> {
        let f = fx.f;
        let t = f.ty(src);
        if wty(f, t)? != W::U32 {
            return Err(format!("not yet printed for WGSL: a pack of {} bits", f.width(t).unwrap_or(0)));
        }
        let p = f.pack(t).ok_or("get on a non-pack")?.clone();
        let (off, fty) = (p.offsets[k as usize], p.fields[k as usize].1);
        let fw = f.width(fty).unwrap_or(32);
        let s = fx.val(src)?;
        let shifted = if off > 0 { format!("({} >> {}u)", s, off) } else { s };
        let bits = if fw < 32 { format!("({} & 0x{:x}u)", shifted, (1u64 << fw) - 1) } else { shifted };
        Ok(match wty(f, fty)? {
            W::Bool => format!("{} != 0u", bits),
            W::I32 => format!("bitcast<i32>({})", bits),
            W::U32 => bits,
            W::Pair => return Err("not yet printed for WGSL: a pack field of more than 32 bits".into()),
        })
    }

    /// a platform rule (targets/air.platform) as WGSL's operation
    fn rule(&mut self, fx: &mut Fx, dsts: &[ValueId], callee: &str, args: &[ValueId]) -> Result<(), String> {
        let native = self.natives.get(callee).unwrap();
        let key = emit_air::rule_key(native);
        if key == "air.wg.barrier" {
            fx.line("workgroupBarrier();");
            return Ok(());
        }
        let Some(&dst) = dsts.first() else { return Err(format!("rule '{}': not printed for WGSL", key)) };
        match key.as_str() {
            "air.thread" => return self.def(fx, dst, "vec2<u32>(tls, 0u)".into()),
            "air.simd_thread" | "air.simd_size" => {
                self.subgroups = true;
                let k = if key == "air.simd_thread" { 0 } else { 2 };
                return self.def(fx, dst, format!("vec2<u32>(mem[(tls >> 2u) + {}u], mem[(tls >> 2u) + {}u])", k, k + 1));
            }
            _ => {}
        }
        let mut a = Vec::new();
        for (j, &x) in args.iter().enumerate() {
            let v = fx.val(x)?;
            let bits = native.arg_bits[j];
            a.push(if native.arg_class[j].is_some() {
                match bits {
                    32 => format!("bitcast<f32>({})", v),
                    16 => return Err(format!("not yet printed for WGSL: `{}` on f16 (the device's shader-f16 is not asked for)", callee)),
                    _ => return Err(format!("not printed for WGSL: `{}` on f{} (WGSL has no f64)", callee, bits)),
                }
            } else if wty(fx.f, fx.f.ty(x))? == W::Pair && key.starts_with("air.simd_") {
                // a thread index or a delta: a word
                self.as_u32(fx, x)?
            } else {
                v
            });
        }
        if native.ret_class.is_some() && native.ret_bits != 32 {
            return Err(format!("not yet printed for WGSL: `{}` on f{}", callee, native.ret_bits));
        }
        let e = if let Some(pred) = key.strip_prefix("fcmp.") {
            let sym = match pred {
                "oeq" => "==",
                "une" => "!=",
                "olt" => "<",
                "ole" => "<=",
                "ogt" => ">",
                "oge" => ">=",
                _ => return Err(format!("not yet printed for WGSL: the float comparison '{}'", pred)),
            };
            format!("{} {} {}", a[0], sym, a[1])
        } else if let Some(rest) = key.strip_prefix("air.simd_") {
            self.subgroups = true;
            match rest {
                "is_first" => "subgroupElect()".to_string(),
                "all" => format!("subgroupAll({})", a[0]),
                "any" => format!("subgroupAny({})", a[0]),
                "ballot.i64" => {
                    let b = fx.fresh("ballot");
                    fx.line(&format!("let {} = subgroupBallot({});", b, a[0]));
                    format!("vec2<u32>({}.x, {}.y)", b, b)
                }
                _ => {
                    let (op, ty) = rest.split_once('.').ok_or_else(|| format!("rule '{}': not printed for WGSL", key))?;
                    if ty == "f16" {
                        return Err(format!("not yet printed for WGSL: `{}` on f16 (the device's shader-f16 is not asked for)", callee));
                    }
                    let name = match op {
                        "sum" => "subgroupAdd",
                        "product" => "subgroupMul",
                        "max" => "subgroupMax",
                        "min" => "subgroupMin",
                        "prefix_inclusive_sum" => "subgroupInclusiveAdd",
                        // a broadcast's thread need not be a constant here
                        "shuffle" | "broadcast" => "subgroupShuffle",
                        "shuffle_xor" => "subgroupShuffleXor",
                        "shuffle_down" => "subgroupShuffleDown",
                        "shuffle_up" => "subgroupShuffleUp",
                        _ => return Err(format!("rule '{}': not printed for WGSL", key)),
                    };
                    format!("{}({})", name, a.join(", "))
                }
            }
        } else {
            match key.as_str() {
                "fadd" | "add" => format!("{} + {}", a[0], a[1]),
                "fsub" | "sub" => format!("{} - {}", a[0], a[1]),
                "fmul" | "mul" => format!("{} * {}", a[0], a[1]),
                "fdiv" => format!("{} / {}", a[0], a[1]),
                "fneg" => format!("-{}", a[0]),
                "air.sqrt.f32" => format!("sqrt({})", a[0]),
                "air.fma.f32" => format!("fma({}, {}, {})", a[0], a[1], a[2]),
                "air.fabs.f32" => format!("abs({})", a[0]),
                other => return Err(format!("rule '{}': not printed for WGSL", other)),
            }
        };
        let e = if native.ret_class.is_some() { format!("bitcast<u32>({})", e) } else { e };
        self.def(fx, dst, e)
    }
}
