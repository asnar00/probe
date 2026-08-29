//! The cost of a function, two ways. **SSA time** is a count over the
//! IR as the parser produced it — one per instruction, a vector
//! operation one, a call one plus its callee — along the longest path
//! through the function, each loop multiplied by its trip count: the
//! bound the program declares (`loop(...) bound N {`), or one the loop
//! shows (a parameter stepped by a constant and compared with one), or
//! an assumed one, or — reported — none. It compares two programs with
//! no target in sight. **Hardware time** is the same walk with every
//! instruction weighted by the function's K: the platform's costs of
//! the machine code emitted for the function (a `cost` line per
//! mnemonic in the platform file; 1 without one), per IR instruction.
//! K is a constant per function per platform, and where it is far from
//! the platform's usual value the IR's idea of cost and the machine's
//! disagree — a lane-by-lane vector, a divide the ISA lacks.

use crate::emit;
use crate::platform::{Natives, Platform};
use crate::ssa::{BinOp, Cond, Function, Inst, Module, Type, ValueId};
use crate::structure::Dom;
use std::collections::{BTreeSet, HashMap};

/// what one function costs
pub struct Report {
    pub name: String,
    pub ssa: f64,
    /// with a target: hardware units, and K
    pub hw: Option<f64>,
    pub k: Option<f64>,
    /// one line per loop met on the walk, callees included
    pub loops: Vec<String>,
    /// what could not be costed
    pub notes: Vec<String>,
}

/// K per function for a fixed-width target: the platform's cost of the
/// function's emitted code over its IR instruction count
pub fn ks(module: &Module, target: &str, platform: &Platform) -> Result<HashMap<String, f64>, String> {
    let enc = emit::Encoder::load(&format!("targets/{}.encodings.json", target))?;
    let compiled = match target {
        "arm64" => emit::compile_with(module, &enc, platform)?,
        "riscv64" => crate::emit_rv::compile_with(module, &enc, platform)?,
        t => return Err(format!("no costs for {} yet (fixed-width targets only)", t)),
    };
    let mut starts: Vec<(usize, &String)> = compiled.funcs.iter().map(|(n, &o)| (o, n)).collect();
    starts.sort();
    let mut out = HashMap::new();
    for (i, &(start, name)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map(|&(o, _)| o).unwrap_or(compiled.code_end).min(compiled.code_end);
        let mut weight = 0i64;
        for chunk in compiled.code[start..end].chunks_exact(4) {
            let word = u32::from_le_bytes(chunk.try_into().unwrap());
            let mnemonic = enc.decode(word).first().and_then(|t| t.split_whitespace().next()).unwrap_or("?").to_string();
            weight += platform.cost_of(&mnemonic);
        }
        let Some(f) = module.funcs.iter().find(|f| &f.name == name) else { continue };
        let n = ssa_count(f).max(1);
        out.insert(name.clone(), weight as f64 / n as f64);
    }
    Ok(out)
}

/// the IR instructions of a function, jumps aside
fn ssa_count(f: &Function) -> usize {
    f.blocks.iter().flat_map(|b| b.insts.iter()).filter(|i| !matches!(i, Inst::Jmp { .. })).count()
}

/// the operations the IR counts as one on any number, whichever
/// library implements them: a call to an instance of one of these
/// generics over plain numbers costs 1 in SSA time and is not descended
const ARITHMETIC: &[&str] = &["add", "sub", "mul", "div", "rem", "neg", "abs", "min", "max", "sqrt", "fma", "conv", "lt", "le", "gt", "ge", "eq", "ne"];

/// ... over the numbers some machine has instructions for: integers
/// and floats. A rational, a fixed, a decimal is a library everywhere,
/// and its arithmetic is counted as the code it is.
fn is_arithmetic(f: &Function) -> bool {
    let Some((g, _)) = &f.instance else { return false };
    if !ARITHMETIC.contains(&g.as_str()) {
        return false;
    }
    // the types as written, before a wide value was split into words
    let tys: Vec<Type> = match &f.wide_sig {
        Some((ps, _)) => ps.clone(),
        None => f.params.iter().map(|&p| f.values[p.0 as usize].ty).collect(),
    };
    tys.iter().all(|&t| match t {
        Type::Int { bits, .. } => bits <= 64,
        Type::Pack(i) => f.packs[i as usize].origin.as_ref().is_some_and(|(o, _)| o == "float") && f.packs[i as usize].width <= 64,
        _ => false,
    })
}

pub struct Coster<'a> {
    module: &'a Module,
    ks: Option<&'a HashMap<String, f64>>,
    /// with a target: what the platform replaces by an instruction
    natives: Option<Natives>,
    /// a trip count to assume for a loop that shows none
    assume: Option<i64>,
    memo: HashMap<String, (f64, f64, Vec<String>, Vec<String>)>,
    stack: Vec<String>,
}

#[derive(Clone)]
struct Node {
    ssa: f64,
    hw: f64,
    succs: Vec<usize>,
}

impl<'a> Coster<'a> {
    pub fn new(module: &'a Module, ks: Option<&'a HashMap<String, f64>>, natives: Option<Natives>, assume: Option<i64>) -> Coster<'a> {
        Coster { module, ks, natives, assume, memo: HashMap::new(), stack: Vec::new() }
    }

    pub fn report(&mut self, name: &str) -> Option<Report> {
        self.module.funcs.iter().find(|f| f.name == name)?;
        let (ssa, hw, loops, notes) = self.cost(name);
        let k = self.ks.map(|ks| ks.get(name).copied().unwrap_or(1.0));
        Some(Report { name: name.to_string(), ssa, hw: self.ks.map(|_| hw), k, loops, notes })
    }

    /// (ssa, hw, loop lines, notes) of a function, callees included
    fn cost(&mut self, name: &str) -> (f64, f64, Vec<String>, Vec<String>) {
        if let Some(r) = self.memo.get(name) {
            return r.clone();
        }
        if self.stack.iter().any(|n| n == name) {
            return (0.0, 0.0, Vec::new(), vec![format!("{}: recursive, the cycle not costed", name)]);
        }
        let Some(f) = self.module.funcs.iter().find(|f| f.name == name) else {
            return (0.0, 0.0, Vec::new(), vec![format!("{}: not found", name)]);
        };
        self.stack.push(name.to_string());
        let k = self.ks.map(|ks| ks.get(name).copied().unwrap_or(1.0)).unwrap_or(1.0);
        let mut loops = Vec::new();
        let mut notes = Vec::new();
        // each block a node
        let mut nodes: Vec<Node> = Vec::new();
        for b in &f.blocks {
            let (mut ssa, mut hw) = (0.0, 0.0);
            for inst in &b.insts {
                match inst {
                    Inst::Jmp { .. } => {}
                    Inst::Call { callee, .. } => {
                        let callee = callee.clone();
                        let arithmetic = self.module.funcs.iter().find(|g| g.name == callee).is_some_and(is_arithmetic);
                        let native = self.natives.as_ref().is_some_and(|n| n.get(&callee).is_some());
                        if arithmetic && (native || self.ks.is_none()) {
                            // one operation, an instruction where the platform has one
                            ssa += 1.0;
                            hw += k;
                        } else {
                            let (cs, ch, cl, cn) = self.cost(&callee);
                            ssa += if arithmetic { 1.0 } else { 1.0 + cs };
                            hw += k + ch;
                            loops.extend(cl);
                            notes.extend(cn);
                        }
                    }
                    Inst::CallInd { .. } => {
                        ssa += 1.0;
                        hw += k;
                        notes.push(format!("{}: an indirect call, its callee not costed", name));
                    }
                    _ => {
                        ssa += 1.0;
                        hw += k;
                    }
                }
            }
            let succs = match b.insts.last() {
                Some(Inst::Jmp { target, .. }) => vec![target.0 as usize],
                Some(Inst::Br { then_target, else_target, .. }) => vec![then_target.0 as usize, else_target.0 as usize],
                _ => vec![],
            };
            nodes.push(Node { ssa, hw, succs });
        }
        // the loops, innermost first
        let dom = Dom::compute(f);
        let mut found: Vec<(usize, Vec<usize>, BTreeSet<usize>)> = Vec::new();
        for &h in &dom.rpo {
            let latches: Vec<usize> = dom.preds[h].iter().copied().filter(|&p| dom.rpo_index[p] != usize::MAX && dom.rpo_index[p] >= dom.rpo_index[h]).collect();
            if latches.is_empty() {
                continue;
            }
            let mut members: BTreeSet<usize> = BTreeSet::new();
            members.insert(h);
            let mut work = latches.clone();
            while let Some(b) = work.pop() {
                if members.insert(b) {
                    work.extend(dom.preds[b].iter().copied());
                }
            }
            found.push((h, latches, members));
        }
        found.sort_by_key(|(_, _, m)| m.len());
        let mut node_of: Vec<usize> = (0..f.blocks.len()).collect();
        for (h, latches, members) in &found {
            let hn = node_of[*h];
            let mset: BTreeSet<usize> = members.iter().map(|&m| node_of[m]).collect();
            let lset: BTreeSet<usize> = latches.iter().map(|&l| node_of[l]).collect();
            let (trip, how) = match f.blocks[*h].bound {
                Some(n) => (n as f64, "declared".to_string()),
                None => match infer_trip(f, *h, members, latches) {
                    Some((n, why)) => (n as f64, why),
                    None => match self.assume {
                        Some(n) => (n as f64, "assumed".to_string()),
                        None => {
                            notes.push(format!("{}: loop at {} has no bound (declare `bound N`, or --assume=N)", name, f.blocks[*h].name));
                            (1.0, "unbounded, counted once".to_string())
                        }
                    },
                },
            };
            // the longest path from the header to a latch, inside the loop
            let mut memo: HashMap<usize, (f64, f64)> = HashMap::new();
            let body = longest(&nodes, hn, &|n| lset.contains(&n), &|n| mset.contains(&n), hn, &mut memo);
            loops.push(format!("{}: loop at {} x{} ({}), body {} ssa", name, f.blocks[*h].name, trip, how, body.0));
            let mut exits: Vec<usize> = Vec::new();
            for &m in &mset {
                for &s in &nodes[m].succs {
                    if !mset.contains(&s) && !exits.contains(&s) {
                        exits.push(s);
                    }
                }
            }
            let idx = nodes.len();
            nodes.push(Node { ssa: trip * body.0, hw: trip * body.1, succs: exits });
            for n in &mut nodes[..idx] {
                for s in &mut n.succs {
                    if mset.contains(s) {
                        *s = idx;
                    }
                }
            }
            for m in members {
                node_of[*m] = idx;
            }
        }
        let mut memo: HashMap<usize, (f64, f64)> = HashMap::new();
        let entry = node_of[0];
        let total = longest(&nodes, entry, &|_| false, &|_| true, usize::MAX, &mut memo);
        self.stack.pop();
        let r = (total.0, total.1, loops, notes);
        self.memo.insert(name.to_string(), r.clone());
        r
    }
}

/// the longest path from `from` over the nodes `inside` allows, never
/// re-entering `header`, to a node `stop` accepts (or a dead end)
fn longest(nodes: &[Node], from: usize, stop: &dyn Fn(usize) -> bool, inside: &dyn Fn(usize) -> bool, header: usize, memo: &mut HashMap<usize, (f64, f64)>) -> (f64, f64) {
    if let Some(&r) = memo.get(&from) {
        return r;
    }
    let here = (nodes[from].ssa, nodes[from].hw);
    let mut best = (0.0f64, 0.0f64);
    if !stop(from) {
        for &s in &nodes[from].succs {
            if s == header || !inside(s) {
                continue;
            }
            let r = longest(nodes, s, stop, inside, header, memo);
            if r.0 > best.0 {
                best = r;
            }
        }
    }
    let r = (here.0 + best.0, here.1 + best.1);
    memo.insert(from, r);
    r
}

/// a trip count the loop shows: a header parameter set by a constant,
/// stepped by a constant on every latch, compared with a constant by
/// the one branch that leaves — the loop is run in the small
fn infer_trip(f: &Function, h: usize, members: &BTreeSet<usize>, latches: &[usize]) -> Option<(i64, String)> {
    let params = &f.blocks[h].params;
    let iconst = |v: ValueId| -> Option<i128> {
        f.blocks.iter().flat_map(|b| b.insts.iter()).find_map(|i| match i {
            Inst::IConst { dst, imm } if *dst == v => Some(*imm),
            _ => None,
        })
    };
    // the one exit
    let mut exit: Option<(ValueId, bool)> = None; // (cond, leaves when true)
    for &m in members {
        if let Some(Inst::Br { cond, then_target, else_target, .. }) = f.blocks[m].insts.last() {
            let t_out = !members.contains(&(then_target.0 as usize));
            let e_out = !members.contains(&(else_target.0 as usize));
            if t_out || e_out {
                if exit.is_some() || (t_out && e_out) {
                    return None;
                }
                exit = Some((*cond, t_out));
            }
        }
    }
    let (cond, leaves_when_true) = exit?;
    let (cc, lhs, rhs) = f.blocks.iter().flat_map(|b| b.insts.iter()).find_map(|i| match i {
        Inst::ICmp { cond: c, dst, lhs, rhs } if *dst == cond => Some((*c, *lhs, *rhs)),
        _ => None,
    })?;
    let (pi, param_left, limit) = if let Some(pi) = params.iter().position(|&p| p == lhs) {
        (pi, true, iconst(rhs)?)
    } else if let Some(pi) = params.iter().position(|&p| p == rhs) {
        (pi, false, iconst(lhs)?)
    } else {
        return None;
    };
    let p = params[pi];
    let args_to_h = |b: usize| -> Option<ValueId> {
        match f.blocks[b].insts.last()? {
            Inst::Jmp { target, args } if target.0 as usize == h => args.get(pi).copied(),
            Inst::Br { then_target, then_args, else_target, else_args, .. } => {
                if then_target.0 as usize == h {
                    then_args.get(pi).copied()
                } else if else_target.0 as usize == h {
                    else_args.get(pi).copied()
                } else {
                    None
                }
            }
            _ => None,
        }
    };
    // the initial value, from every entry edge
    let mut init: Option<i128> = None;
    for b in 0..f.blocks.len() {
        if members.contains(&b) {
            continue;
        }
        if let Some(v) = args_to_h(b) {
            let c = iconst(v)?;
            if init.is_some_and(|i| i != c) {
                return None;
            }
            init = Some(c);
        }
    }
    let init = init?;
    // the step, the same on every latch
    let mut step: Option<i128> = None;
    for &l in latches {
        let v = args_to_h(l)?;
        let s = f.blocks.iter().flat_map(|b| b.insts.iter()).find_map(|i| match i {
            Inst::Bin { op: BinOp::IAdd, dst, lhs, rhs } if *dst == v && *lhs == p => iconst(*rhs),
            Inst::Bin { op: BinOp::IAdd, dst, lhs, rhs } if *dst == v && *rhs == p => iconst(*lhs),
            Inst::Bin { op: BinOp::ISub, dst, lhs, rhs } if *dst == v && *lhs == p => iconst(*rhs).map(|c| -c),
            _ => None,
        })?;
        if step.is_some_and(|t| t != s) {
            return None;
        }
        step = Some(s);
    }
    let step = step?;
    if step == 0 {
        return None;
    }
    let mut i = init;
    let mut n: i64 = 0;
    loop {
        let (a, b) = if param_left { (i, limit) } else { (limit, i) };
        let holds = match cc {
            Cond::Eq => a == b,
            Cond::Ne => a != b,
            Cond::Lt => a < b,
            Cond::Le => a <= b,
            Cond::Gt => a > b,
            Cond::Ge => a >= b,
        };
        if holds == leaves_when_true {
            break;
        }
        n += 1;
        if n > 1 << 24 {
            return None;
        }
        i += step;
    }
    Some((n, format!("{} from {} by {} to {}", f.values[p.0 as usize].name, init, step, limit)))
}

#[cfg(test)]
mod tests {
    use crate::ssa::{self, Policy, Type};

    fn module(src: &str) -> ssa::Module {
        let policy = Policy::new(Type::I64).unwrap();
        let mut m = ssa::parse_with(&ssa::with_prelude(src), &policy).unwrap();
        ssa::resolve_types(&mut m, &policy);
        crate::opt::optimize(&mut m, crate::opt::MAX_LEVEL);
        m
    }

    /// a loop stepping a parameter by a constant to a constant shows
    /// its trip count; a declared bound is taken as written; one that
    /// shows nothing is reported
    #[test]
    fn loops_are_counted() {
        let src = "fn counted() -> i64 {
    r: i64 = loop(i: i64 = 0, acc: i64 = 0) {
        done: u1 = cmp.ge i, 5
        if done {
            break acc
        }
        acc2: i64 = add acc, i
        i2: i64 = add i, 1
        continue i2, acc2
    }
    ret r
}
fn declared(n: i64) -> i64 {
    r: i64 = loop(i: i64 = 0) bound 16 {
        done: u1 = cmp.ge i, n
        if done {
            break i
        }
        i2: i64 = add i, 1
        continue i2
    }
    ret r
}
fn unknown(n: i64) -> i64 {
    r: i64 = loop(i: i64 = 0) {
        done: u1 = cmp.ge i, n
        if done {
            break i
        }
        i2: i64 = add i, 1
        continue i2
    }
    ret r
}
";
        let m = module(src);
        let mut c = super::Coster::new(&m, None, None, None);
        let counted = c.report("counted").unwrap();
        assert!(counted.loops.iter().any(|l| l.contains("x5 (i from 0 by 1 to 5)")), "{:?}", counted.loops);
        assert!(counted.notes.is_empty());
        let declared = c.report("declared").unwrap();
        assert!(declared.loops.iter().any(|l| l.contains("x16 (declared)")), "{:?}", declared.loops);
        let unknown = c.report("unknown").unwrap();
        assert!(unknown.notes.iter().any(|n| n.contains("no bound")), "{:?}", unknown.notes);
        // the declared loop costs its body sixteen times, the unknown one once
        assert!(declared.ssa > unknown.ssa * 8.0, "{} vs {}", declared.ssa, unknown.ssa);
        let mut assumed = super::Coster::new(&m, None, None, Some(16));
        let u16 = assumed.report("unknown").unwrap();
        assert!(u16.notes.is_empty());
        assert_eq!(u16.ssa, declared.ssa);
    }

    /// arithmetic on machine numbers is one operation whichever library
    /// implements it; on a rational it is the library's code
    #[test]
    fn arithmetic_is_one_on_machine_numbers() {
        let src = "fn fl(a: f32, b: f32) -> f32 {
    c: f32 = mul a, b
    ret c
}
fn ra(a: rational(64, 64), b: rational(64, 64)) -> rational(64, 64) {
    c: rational(64, 64) = mul a, b
    ret c
}
";
        let m = module(src);
        let mut c = super::Coster::new(&m, None, None, Some(4));
        let fl = c.report("fl").unwrap();
        let ra = c.report("ra").unwrap();
        assert!(fl.ssa <= 3.0, "{}", fl.ssa);
        assert!(ra.ssa > 100.0, "{}", ra.ssa);
    }
}
