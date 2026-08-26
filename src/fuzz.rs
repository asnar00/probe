//! The fuzzer: random well-formed programs, every configuration a referee.
//!
//! A program is generated so that it cannot go wrong for a reason we don't
//! care about: divisors are made nonzero, shift amounts are literals under
//! the width, floats come from integers (so no NaN payloads reach the
//! platform, whose NaNs carry payloads the library canonicalizes), loops
//! are bounded. Its results at native -O0 with the platform off are the
//! reference; then it runs at every optimization level, on the platform,
//! on wasm, and (slowly) on the two qemu machines. A disagreement is a bug
//! in one of them, and the program is kept as a suite file that reproduces
//! it: target/fuzz/<seed>-<n>.ssa.

use crate::ssa::Type;
use crate::{emit, platform, ssa, suite};
use std::fmt::Write;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next() % den < num
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Kind {
    Int { signed: bool, bits: u32 },
    Float, // f32 or f64 by name
    Pack,  // the program's one pack type
}

#[derive(Clone, PartialEq, Debug)]
struct Ty {
    name: String,
    kind: Kind,
}

fn int_types() -> Vec<Ty> {
    let mut v = Vec::new();
    for (s, b) in [(true, 1), (false, 1), (true, 5), (false, 5), (true, 8), (false, 8), (true, 12), (false, 12), (true, 16), (false, 16), (true, 32), (false, 32), (true, 40), (false, 40), (true, 64), (false, 64)] {
        v.push(Ty {
            name: format!("{}{}", if s { "i" } else { "u" }, b),
            kind: Kind::Int { signed: s, bits: b },
        });
    }
    v
}

fn int_type(name: &str) -> Ty {
    let (s, b) = name.split_at(1);
    Ty { name: name.to_string(), kind: Kind::Int { signed: s == "i", bits: b.parse().unwrap() } }
}

fn float_types() -> Vec<Ty> {
    ["f32", "f64"].iter().map(|n| Ty { name: n.to_string(), kind: Kind::Float }).collect()
}

const PACK: &str = "pack { a: u5, b: i7, c: u4, d: i16 }";
const PACK_FIELDS: [(&str, &str); 4] = [("a", "u5"), ("b", "i7"), ("c", "u4"), ("d", "i16")];

struct Gen {
    rng: Rng,
    out: String,
    vals: Vec<(String, Ty)>,
    n: usize,
    /// earlier functions: name and parameter types
    funcs: Vec<(String, Vec<Ty>)>,
    depth: usize,
}

impl Gen {
    fn fresh(&mut self) -> String {
        self.n += 1;
        format!("v{}", self.n)
    }

    fn of(&self, kind_ok: impl Fn(&Ty) -> bool) -> Vec<(String, Ty)> {
        self.vals.iter().filter(|(_, t)| kind_ok(t)).cloned().collect()
    }

    /// a value from `same` other than `x` when there is one
    fn other(&mut self, same: &[(String, Ty)], x: &str) -> String {
        let others: Vec<_> = same.iter().filter(|(n, _)| n != x).cloned().collect();
        if others.is_empty() { x.to_string() } else { self.rng.pick(&others).0.clone() }
    }

    fn indent(&self) -> String {
        "    ".repeat(self.depth + 1)
    }

    fn emit(&mut self, line: &str) {
        let ind = self.indent();
        let _ = writeln!(self.out, "{}{}", ind, line);
    }

    fn def(&mut self, ty: &Ty, rhs: &str) -> String {
        let name = self.fresh();
        self.emit(&format!("{}: {} = {}", name, ty.name, rhs));
        self.vals.push((name.clone(), ty.clone()));
        name
    }

    /// a literal that fits an integer type
    fn lit(&mut self, t: &Ty) -> String {
        match t.kind {
            Kind::Int { signed, bits } => {
                let r = self.rng.next();
                let v = if bits >= 64 {
                    r as i64
                } else if signed {
                    let m = 1i64 << (bits - 1);
                    (r as i64).rem_euclid(2 * m) - m
                } else {
                    (r % (1u64 << bits)) as i64
                };
                v.to_string()
            }
            _ => "0".into(),
        }
    }

    fn statement(&mut self) {
        let ints = self.of(|t| matches!(t.kind, Kind::Int { .. }));
        let floats = self.of(|t| t.kind == Kind::Float);
        let packs = self.of(|t| t.kind == Kind::Pack);
        match self.rng.below(16) {
            0..=3 if !ints.is_empty() => {
                // integer binop on two values (or a value and a literal) of one type
                let (x, t) = self.rng.pick(&ints).clone();
                let same: Vec<_> = ints.iter().filter(|(_, u)| *u == t).cloned().collect();
                let y = if self.rng.chance(1, 3) { self.lit(&t) } else { self.other(&same, &x) };
                let op = *self.rng.pick(&["add", "sub", "mul", "and", "or", "xor", "div", "rem", "shl", "shr"]);
                match op {
                    "div" | "rem" => {
                        // a nonzero divisor: y | 1 (never a literal 0 either way)
                        let nz = self.def(&t, &format!("or {}, 1", y));
                        self.def(&t, &format!("{} {}, {}", op, x, nz));
                    }
                    "shl" | "shr" => {
                        let Kind::Int { bits, .. } = t.kind else { unreachable!() };
                        let k = self.rng.below(bits as usize);
                        self.def(&t, &format!("{} {}, {}", op, x, k));
                    }
                    _ => {
                        self.def(&t, &format!("{} {}, {}", op, x, y));
                    }
                }
            }
            4 if !ints.is_empty() => {
                let (x, t) = self.rng.pick(&ints).clone();
                let same: Vec<_> = ints.iter().filter(|(_, u)| *u == t).cloned().collect();
                let y = self.other(&same, &x);
                let c = *self.rng.pick(&["eq", "ne", "lt", "le", "gt", "ge"]);
                let u1 = Ty { name: "u1".into(), kind: Kind::Int { signed: false, bits: 1 } };
                self.def(&u1, &format!("cmp.{} {}, {}", c, x, y));
            }
            5 if !ints.is_empty() => {
                // conv between integer types, cast at equal width
                let (x, t) = self.rng.pick(&ints).clone();
                let all = int_types();
                let to = self.rng.pick(&all).clone();
                let Kind::Int { bits: bt, .. } = to.kind else { unreachable!() };
                let Kind::Int { bits: bs, .. } = t.kind else { unreachable!() };
                let op = if bt == bs && self.rng.chance(1, 2) { "cast" } else { "conv" };
                self.def(&to, &format!("{} {}", op, x));
            }
            6 if !ints.is_empty() => {
                // an integer into a float, and a float op on it
                let (x, _) = self.rng.pick(&ints).clone();
                let ft = self.rng.pick(&float_types()).clone();
                self.def(&ft, &format!("conv {}", x));
            }
            7..=9 if !floats.is_empty() => {
                let (x, t) = self.rng.pick(&floats).clone();
                let same: Vec<_> = floats.iter().filter(|(_, u)| *u == t).cloned().collect();
                let y = self.other(&same, &x);
                let z = self.other(&same, &y);
                match self.rng.below(9) {
                    0 => { self.def(&t, &format!("add {}, {}", x, y)); }
                    1 => { self.def(&t, &format!("sub {}, {}", x, y)); }
                    2 => { self.def(&t, &format!("mul {}, {}", x, y)); }
                    3 => { self.def(&t, &format!("div {}, {}", x, y)); }
                    4 => {
                        let a = self.def(&t, &format!("abs {}", x));
                        self.def(&t, &format!("sqrt {}", a));
                    }
                    5 => { self.def(&t, &format!("fma {}, {}, {}", x, y, z)); }
                    6 => {
                        let op = if self.rng.chance(1, 2) { "min" } else { "max" };
                        self.def(&t, &format!("{} {}, {}", op, x, y));
                    }
                    7 => {
                        let c = *self.rng.pick(&["eq", "ne", "lt", "le", "gt", "ge"]);
                        let u1 = Ty { name: "u1".into(), kind: Kind::Int { signed: false, bits: 1 } };
                        self.def(&u1, &format!("cmp.{} {}, {}", c, x, y));
                    }
                    _ => {
                        // to an integer, or to the other float width
                        if self.rng.chance(1, 2) {
                            let to = self.rng.pick(&int_types()).clone();
                            if matches!(to.kind, Kind::Int { bits, .. } if bits >= 8) {
                                self.def(&to, &format!("conv {}", x));
                            }
                        } else {
                            let other = if t.name == "f32" { "f64" } else { "f32" };
                            self.def(&Ty { name: other.into(), kind: Kind::Float }, &format!("conv {}", x));
                        }
                    }
                }
            }
            10 if !ints.is_empty() => {
                // pack from fields, get, set, unpack
                let pt = Ty { name: "p".into(), kind: Kind::Pack };
                if packs.is_empty() || self.rng.chance(1, 2) {
                    let mut args = Vec::new();
                    for (_, fty) in PACK_FIELDS {
                        let t = int_type(fty);
                        let same: Vec<_> = ints.iter().filter(|(_, u)| *u == t).cloned().collect();
                        args.push(if same.is_empty() || self.rng.chance(1, 3) { self.lit(&t) } else { self.rng.pick(&same).0.clone() });
                    }
                    self.def(&pt, &format!("pack {}", args.join(", ")));
                } else {
                    let (p, _) = self.rng.pick(&packs).clone();
                    let (f, fty) = *self.rng.pick(&PACK_FIELDS);
                    let t = int_type(fty);
                    if self.rng.chance(1, 2) {
                        self.def(&t, &format!("get {}, {}", p, f));
                    } else {
                        let v = self.lit(&t);
                        self.def(&pt, &format!("set {}, {}, {}", p, f, v));
                    }
                }
            }
            11 if !ints.is_empty() && self.depth < 2 => {
                // a value-yielding if
                let (x, t) = self.rng.pick(&ints).clone();
                let u1s = self.of(|t| t.name == "u1");
                let c = if u1s.is_empty() {
                    self.def(&Ty { name: "u1".into(), kind: Kind::Int { signed: false, bits: 1 } }, &format!("cmp.ne {}, 0", x))
                } else {
                    self.rng.pick(&u1s).0.clone()
                };
                let name = self.fresh();
                self.emit(&format!("{}: {} = if {} {{", name, t.name, c));
                let mark = self.vals.len();
                self.depth += 1;
                self.statement();
                let a = self.of(|u| *u == t);
                let ya = self.rng.pick(&a).0.clone();
                self.emit(&format!("yield {}", ya));
                self.depth -= 1;
                self.vals.truncate(mark);
                self.emit("} else {");
                self.depth += 1;
                self.statement();
                let b = self.of(|u| *u == t);
                let yb = self.rng.pick(&b).0.clone();
                self.emit(&format!("yield {}", yb));
                self.depth -= 1;
                self.vals.truncate(mark);
                self.emit("}");
                self.vals.push((name, t));
            }
            12 if !ints.is_empty() && self.depth < 2 => {
                // a bounded loop carrying one integer
                let (x, t) = self.rng.pick(&ints).clone();
                let k = 1 + self.rng.below(6);
                let name = self.fresh();
                let acc = self.fresh();
                let i = self.fresh();
                self.emit(&format!("{}: {} = loop({}: u8 = 0, {}: {} = {}) {{", name, t.name, i, acc, t.name, x));
                let mark = self.vals.len();
                self.depth += 1;
                self.emit(&format!("done: u1 = cmp.ge {}, {}", i, k).replace("done", &format!("d{}", self.n)));
                let dn = format!("d{}", self.n);
                self.emit(&format!("if {} {{", dn));
                self.depth += 1;
                self.emit(&format!("break {}", acc));
                self.depth -= 1;
                self.emit("}");
                self.vals.push((acc.clone(), t.clone()));
                self.vals.push((i.clone(), Ty { name: "u8".into(), kind: Kind::Int { signed: false, bits: 8 } }));
                self.statement();
                let same = self.of(|u| *u == t);
                let nxt = self.rng.pick(&same).0.clone();
                let i2 = self.fresh();
                self.emit(&format!("{}: u8 = add {}, 1", i2, i));
                self.emit(&format!("continue {}, {}", i2, nxt));
                self.depth -= 1;
                self.vals.truncate(mark);
                self.emit("}");
                self.vals.push((name, t));
            }
            13 if !self.funcs.is_empty() => {
                // a call to an earlier function
                let (f, ptys) = self.rng.pick(&self.funcs.clone()).clone();
                let mut args = Vec::new();
                for pt in &ptys {
                    let same = self.of(|u| u == pt);
                    args.push(if same.is_empty() { self.lit(pt) } else { self.rng.pick(&same).0.clone() });
                }
                self.def(&Ty { name: "i64".into(), kind: Kind::Int { signed: true, bits: 64 } }, &format!("{}({})", f, args.join(", ")));
            }
            _ => {
                // a fresh constant
                let t = self.rng.pick(&int_types()).clone();
                let l = self.lit(&t);
                self.def(&t, &format!("const {}", l));
            }
        }
    }

    /// one function: integer parameters, a body, an i64 result
    fn function(&mut self, name: &str) -> Vec<Ty> {
        let nparams = 1 + self.rng.below(3);
        let all = int_types();
        let ptys: Vec<Ty> = (0..nparams).map(|_| self.rng.pick(&all).clone()).collect();
        self.vals.clear();
        self.n = 0;
        let params: Vec<String> = ptys
            .iter()
            .map(|t| {
                let n = self.fresh();
                self.vals.push((n.clone(), t.clone()));
                format!("{}: {}", n, t.name)
            })
            .collect();
        let _ = writeln!(self.out, "fn {}({}) -> i64 {{", name, params.join(", "));
        let len = 4 + self.rng.below(16);
        for _ in 0..len {
            self.statement();
        }
        // the result: some value, as an i64 (floats and packs by their bits)
        let later = self.vals[self.vals.len() / 2..].to_vec();
        let (v, t) = self.rng.pick(&later).clone();
        let r = match t.kind {
            Kind::Int { bits: 64, signed: true } => v,
            Kind::Int { .. } => self.def(&Ty { name: "i64".into(), kind: Kind::Int { signed: true, bits: 64 } }, &format!("conv {}", v)),
            Kind::Float => {
                let w = if t.name == "f32" { "u32" } else { "u64" };
                let bits = self.def(&Ty { name: w.into(), kind: Kind::Int { signed: false, bits: 64 } }, &format!("cast {}", v));
                self.def(&Ty { name: "i64".into(), kind: Kind::Int { signed: true, bits: 64 } }, &format!("conv {}", bits))
            }
            Kind::Pack => {
                let bits = self.def(&Ty { name: "u32".into(), kind: Kind::Int { signed: false, bits: 32 } }, &format!("cast {}", v));
                self.def(&Ty { name: "i64".into(), kind: Kind::Int { signed: true, bits: 64 } }, &format!("conv {}", bits))
            }
        };
        let _ = writeln!(self.out, "    ret {}\n}}", r);
        ptys
    }
}

/// a random program: a pack type and a few functions
fn program(seed: u64) -> (String, Vec<(String, Vec<Ty>)>) {
    let mut g = Gen {
        rng: Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1),
        out: format!("type p = {}\n", PACK),
        vals: Vec::new(),
        n: 0,
        funcs: Vec::new(),
        depth: 0,
    };
    let nf = 1 + g.rng.below(3);
    for k in 0..nf {
        let name = format!("f{}", k);
        let ptys = g.function(&name);
        g.funcs.push((name, ptys));
    }
    (g.out, g.funcs)
}

/// canonical random arguments for a function's parameter types
fn arguments(rng: &mut Rng, ptys: &[Ty]) -> Vec<i64> {
    ptys.iter()
        .map(|t| {
            let Kind::Int { signed, bits } = t.kind else { unreachable!() };
            let r = rng.next();
            // edges often
            let r = match rng.below(6) {
                0 => 0,
                1 => u64::MAX,
                2 => 1,
                3 => 1u64 << (bits - 1),
                _ => r,
            };
            crate::opt::norm(if signed { ssa::Repr::S(bits) } else { ssa::Repr::U(bits) }, r as i64)
        })
        .collect()
}

pub fn fuzz(count: usize, seed: u64, slow: bool) -> Result<usize, String> {
    fuzz_with(count, seed, slow, true)
}

/// `soft_run`: also run the whole thing with the platform off at the top
/// level, which flips the process-wide --soft flag — fine from the
/// command line, not from a test running beside others
pub fn fuzz_with(count: usize, seed: u64, slow: bool, soft_run: bool) -> Result<usize, String> {
    let enc = emit::Encoder::load("targets/arm64.encodings.json")?;
    let dir = std::path::Path::new("target/fuzz");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let work = dir.join("work");
    let mut failures = 0;
    for n in 0..count {
        // program n of a run is seed + n, so a printed seed reproduces
        // that program alone: probe fuzz 1 --seed=<it>
        let s = seed.wrapping_add(n as u64);
        let (src, funcs) = program(s);
        // the reference: native, -O0, the platform off
        let policy = ssa::Policy::new(Type::I64)?;
        let module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| format!("generated program did not parse (seed {}): {}\n{}", s, e, src))?;
        ssa::verify(&module).map_err(|e| format!("generated program did not verify (seed {}): {}\n{}", s, e.join("; "), src))?;
        let compiled = emit::compile_with(&module, &enc, &platform::Platform::none())?;
        let jit = emit::jit::JitCode::new(&compiled)?;
        let mut rng = Rng(s.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xabcd);
        let mut directives = String::new();
        for (f, ptys) in &funcs {
            for _ in 0..4 {
                let args = arguments(&mut rng, ptys);
                let r = jit.call(f, &args)?;
                let _ = writeln!(directives, ";! {} {} -> {}", f, args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" "), r);
            }
        }
        let text = format!("; fuzz seed {}\n{}\n{}", s, directives, src);
        // every other configuration must agree
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        std::fs::write(work.join("fuzz.ssa"), &text).map_err(|e| e.to_string())?;
        let mut runs: Vec<(&str, suite::Backend, usize, bool)> = Vec::new();
        for level in 0..=crate::opt::MAX_LEVEL {
            runs.push(("native", suite::Backend::Native, level, false));
        }
        if soft_run {
            runs.push(("soft -O4", suite::Backend::Native, crate::opt::MAX_LEVEL, true));
        }
        runs.push(("wasm", suite::Backend::Wasm, crate::opt::MAX_LEVEL, false));
        if slow {
            runs.push(("riscv", suite::Backend::Riscv, crate::opt::MAX_LEVEL, false));
            runs.push(("arm-qemu", suite::Backend::ArmQemu, crate::opt::MAX_LEVEL, false));
        }
        let mut bad = Vec::new();
        for (name, backend, level, soft) in runs {
            if soft_run {
                platform::set_soft(soft);
            }
            let report = suite::run_dir_at(work.to_str().unwrap(), backend, level, &|p| p)?;
            if report.failed > 0 {
                bad.push(format!("{} -O{}:\n{}", name, level, report.log.lines().filter(|l| l.starts_with("FAIL")).collect::<Vec<_>>().join("\n")));
            }
        }
        if soft_run {
            platform::set_soft(false);
        }
        if bad.is_empty() {
            println!("  ok  seed {:016x}  ({} fn, {} lines)", s, funcs.len(), src.lines().count());
        } else {
            failures += 1;
            let keep = dir.join(format!("{:016x}.ssa", s));
            std::fs::write(&keep, &text).map_err(|e| e.to_string())?;
            println!("FAIL  seed {:016x}  kept as {}\n{}", s, keep.display(), bad.join("\n"));
        }
    }
    Ok(failures)
}

#[cfg(test)]
mod tests {
    /// a short run: every configuration this machine has without qemu
    #[test]
    fn programs_agree_everywhere() {
        assert_eq!(super::fuzz_with(12, 0x1000, false, false).unwrap(), 0);
    }
}
