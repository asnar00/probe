//! The cost of a function, two ways. **SSA time** is a count over the
//! IR as the parser produced it — one per instruction, a vector
//! operation one, a call one plus its callee — along the longest path
//! through the function, each loop charged its trip count times the
//! longest path from its header to a latch, and once the longest path
//! from its header to the block that leaves: the bound the program
//! declares (`loop(...) bound N {`, N traversals of the back edge), or
//! one the loop shows (a parameter stepped by a constant and compared
//! with a value whose range is known), or an assumed one, or —
//! reported — none, counted as one traversal and the leaving pass.
//! A callee is costed with what its call site knows: a range per
//! integer argument, carried through the function by a forward pass
//! (constants, arithmetic, `min`/`max`, a loop parameter's start and the
//! sign of its step, a `check` or a branch refining what it compares),
//! so a string literal's length bounds the copy inside the library that
//! moves it, and a function met with different constants is costed once
//! per constant. It compares two programs with no target in sight.
//! **Hardware time** is the same walk with every instruction weighted by
//! the function's K: the platform's costs of the machine code emitted
//! for the function (a `cost` line per mnemonic in the platform file; 1
//! without one), per IR instruction. K is a constant per function per
//! platform, and where it is far from the platform's usual value the
//! IR's idea of cost and the machine's disagree — a lane-by-lane vector,
//! a divide the ISA lacks.

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

/// what is known of an integer value: the closed interval it lies in,
/// either end open when nothing says
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Range {
    pub lo: Option<i128>,
    pub hi: Option<i128>,
}

fn max_opt(a: Option<i128>, b: Option<i128>) -> Option<i128> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        _ => None,
    }
}

fn min_opt(a: Option<i128>, b: Option<i128>) -> Option<i128> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        _ => None,
    }
}

/// the values a type holds, for one a machine word carries
fn type_bounds(ty: Type) -> Option<(i128, i128)> {
    match ty {
        Type::Int { signed, bits } if (1..=64).contains(&bits) => Some(if signed { (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1) } else { (0, (1i128 << bits) - 1) }),
        _ => None,
    }
}

impl Range {
    pub const UNKNOWN: Range = Range { lo: None, hi: None };

    fn point(c: i128) -> Range {
        Range { lo: Some(c), hi: Some(c) }
    }

    fn is_point(&self) -> Option<i128> {
        match (self.lo, self.hi) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        }
    }

    /// either of two: the union
    fn join(a: Range, b: Range) -> Range {
        Range { lo: a.lo.zip(b.lo).map(|(x, y)| x.min(y)), hi: a.hi.zip(b.hi).map(|(x, y)| x.max(y)) }
    }

    /// both of two: the intersection
    fn meet(a: Range, b: Range) -> Range {
        Range { lo: max_opt(a.lo, b.lo), hi: min_opt(a.hi, b.hi) }
    }

    fn add(a: Range, b: Range) -> Range {
        Range { lo: a.lo.zip(b.lo).and_then(|(x, y)| x.checked_add(y)), hi: a.hi.zip(b.hi).and_then(|(x, y)| x.checked_add(y)) }
    }

    fn sub(a: Range, b: Range) -> Range {
        Range { lo: a.lo.zip(b.hi).and_then(|(x, y)| x.checked_sub(y)), hi: a.hi.zip(b.lo).and_then(|(x, y)| x.checked_sub(y)) }
    }

    fn scale(a: Range, c: i128) -> Range {
        let (lo, hi) = if c >= 0 { (a.lo, a.hi) } else { (a.hi, a.lo) };
        Range { lo: lo.and_then(|x| x.checked_mul(c)), hi: hi.and_then(|x| x.checked_mul(c)) }
    }

    fn mul(a: Range, b: Range) -> Range {
        if let Some(c) = b.is_point() {
            return Range::scale(a, c);
        }
        if let Some(c) = a.is_point() {
            return Range::scale(b, c);
        }
        match (a.lo, a.hi, b.lo, b.hi) {
            (Some(al), Some(ah), Some(bl), Some(bh)) => {
                let ps = [al.checked_mul(bl), al.checked_mul(bh), ah.checked_mul(bl), ah.checked_mul(bh)];
                if ps.iter().any(|p| p.is_none()) {
                    return Range::UNKNOWN;
                }
                Range { lo: ps.iter().flatten().copied().min(), hi: ps.iter().flatten().copied().max() }
            }
            _ => Range::UNKNOWN,
        }
    }

    /// division by a positive constant truncates toward zero, which is
    /// monotone, so the ends divide
    fn div(a: Range, c: i128) -> Range {
        Range { lo: a.lo.map(|x| x / c), hi: a.hi.map(|x| x / c) }
    }

    /// the remainder by a positive constant: within (-c, c), and within
    /// [0, x] for a non-negative x
    fn rem(a: Range, c: i128, signed: bool) -> Range {
        if !signed || a.lo.is_some_and(|l| l >= 0) {
            Range { lo: Some(0), hi: Some(min_opt(a.hi, Some(c - 1)).unwrap()) }
        } else {
            Range { lo: Some(-(c - 1)), hi: Some(c - 1) }
        }
    }

    fn min(a: Range, b: Range) -> Range {
        Range { lo: a.lo.zip(b.lo).map(|(x, y)| x.min(y)), hi: min_opt(a.hi, b.hi) }
    }

    fn max(a: Range, b: Range) -> Range {
        Range { lo: max_opt(a.lo, b.lo), hi: a.hi.zip(b.hi).map(|(x, y)| x.max(y)) }
    }

    /// the range of an arithmetic result held in `ty`: an end past what
    /// the type holds would have wrapped, and is not known
    fn within(&self, ty: Type) -> Range {
        let Some((tlo, thi)) = type_bounds(ty) else { return Range::UNKNOWN };
        Range { lo: self.lo.filter(|&l| l >= tlo), hi: self.hi.filter(|&h| h <= thi) }
    }

    /// the range after a conversion into `ty`: the same when it fits,
    /// else whatever the type holds
    fn converted(&self, ty: Type) -> Range {
        let Some((tlo, thi)) = type_bounds(ty) else { return Range::UNKNOWN };
        match (self.lo, self.hi) {
            (Some(l), Some(h)) if l >= tlo && h <= thi => *self,
            _ => Range { lo: Some(tlo), hi: Some(thi) },
        }
    }
}

fn negate(c: Cond) -> Cond {
    match c {
        Cond::Eq => Cond::Ne,
        Cond::Ne => Cond::Eq,
        Cond::Lt => Cond::Ge,
        Cond::Ge => Cond::Lt,
        Cond::Le => Cond::Gt,
        Cond::Gt => Cond::Le,
    }
}

/// the comparison with its sides swapped
fn flip(c: Cond) -> Cond {
    match c {
        Cond::Lt => Cond::Gt,
        Cond::Gt => Cond::Lt,
        Cond::Le => Cond::Ge,
        Cond::Ge => Cond::Le,
        c => c,
    }
}

fn holds(c: Cond, a: i128, b: i128) -> bool {
    match c {
        Cond::Eq => a == b,
        Cond::Ne => a != b,
        Cond::Lt => a < b,
        Cond::Le => a <= b,
        Cond::Gt => a > b,
        Cond::Ge => a >= b,
    }
}

/// what a function's text says once, looked up while it is walked
struct Facts<'f> {
    f: &'f Function,
    consts: HashMap<ValueId, i128>,
    cmps: HashMap<ValueId, (Cond, ValueId, ValueId)>,
    bins: HashMap<ValueId, (BinOp, ValueId, ValueId)>,
}

impl<'f> Facts<'f> {
    fn new(f: &'f Function) -> Facts<'f> {
        let mut consts = HashMap::new();
        let mut cmps = HashMap::new();
        let mut bins = HashMap::new();
        for inst in f.blocks.iter().flat_map(|b| b.insts.iter()) {
            match inst {
                Inst::IConst { dst, imm } => {
                    consts.insert(*dst, *imm);
                }
                Inst::ICmp { cond, dst, lhs, rhs } => {
                    cmps.insert(*dst, (*cond, *lhs, *rhs));
                }
                Inst::Bin { op, dst, lhs, rhs } => {
                    bins.insert(*dst, (*op, *lhs, *rhs));
                }
                _ => {}
            }
        }
        Facts { f, consts, cmps, bins }
    }

    /// an integer a word holds: signed, bits
    fn int_type(&self, v: ValueId) -> Option<(bool, u16)> {
        match self.f.values[v.0 as usize].ty {
            Type::Int { signed, bits } if bits <= 64 => Some((signed, bits)),
            _ => None,
        }
    }

    /// a value that is a constant wherever it stands: an IConst, or a
    /// literal written inline
    fn const_of(&self, v: ValueId) -> Option<i128> {
        if let Some(&c) = self.consts.get(&v) {
            return Some(c);
        }
        match self.f.values[v.0 as usize].literal {
            Some((Type::Int { .. }, bits)) => Some(bits as i128),
            _ => None,
        }
    }

    fn rng(&self, env: &HashMap<ValueId, Range>, v: ValueId) -> Range {
        if let Some(r) = env.get(&v) {
            return *r;
        }
        match self.const_of(v) {
            Some(c) => Range::point(c),
            None => Range::UNKNOWN,
        }
    }

    /// the constant a latch adds to a loop parameter on the way back
    fn step_of(&self, v: ValueId, p: ValueId) -> Option<i128> {
        if v == p {
            return Some(0);
        }
        match self.bins.get(&v) {
            Some((BinOp::IAdd, l, r)) if *l == p => self.const_of(*r),
            Some((BinOp::IAdd, l, r)) if *r == p => self.const_of(*l),
            Some((BinOp::ISub, l, r)) if *l == p => self.const_of(*r).map(|c| -c),
            _ => None,
        }
    }

    /// a comparison known to hold (or not) from here on narrows both
    /// sides
    fn refine(&self, env: &mut HashMap<ValueId, Range>, cond: ValueId, holds: bool) {
        let Some(&(cc, l, r)) = self.cmps.get(&cond) else { return };
        if self.int_type(l).is_none() || self.int_type(r).is_none() {
            return;
        }
        let cc = if holds { cc } else { negate(cc) };
        let (a, b) = (self.rng(env, l), self.rng(env, r));
        let (a2, b2) = match cc {
            Cond::Ge => (Range { lo: max_opt(a.lo, b.lo), hi: a.hi }, Range { lo: b.lo, hi: min_opt(b.hi, a.hi) }),
            Cond::Gt => (Range { lo: max_opt(a.lo, b.lo.map(|x| x + 1)), hi: a.hi }, Range { lo: b.lo, hi: min_opt(b.hi, a.hi.map(|x| x - 1)) }),
            Cond::Le => (Range { lo: a.lo, hi: min_opt(a.hi, b.hi) }, Range { lo: max_opt(b.lo, a.lo), hi: b.hi }),
            Cond::Lt => (Range { lo: a.lo, hi: min_opt(a.hi, b.hi.map(|x| x - 1)) }, Range { lo: max_opt(b.lo, a.lo.map(|x| x + 1)), hi: b.hi }),
            Cond::Eq => {
                let m = Range::meet(a, b);
                (m, m)
            }
            Cond::Ne => (a, b),
        };
        if a2 != a {
            env.insert(l, a2);
        }
        if b2 != b {
            env.insert(r, b2);
        }
    }
}

/// the arguments each edge from `p` to `b` passes
fn args_to(f: &Function, p: usize, b: usize) -> Vec<&Vec<ValueId>> {
    match f.blocks[p].insts.last() {
        Some(Inst::Jmp { target, args }) if target.0 as usize == b => vec![args],
        Some(Inst::Br { then_target, then_args, else_target, else_args, .. }) => {
            let mut v = Vec::new();
            if then_target.0 as usize == b {
                v.push(then_args);
            }
            if else_target.0 as usize == b {
                v.push(else_args);
            }
            v
        }
        _ => Vec::new(),
    }
}

/// what a call to a function with these argument ranges costs
#[derive(Clone)]
struct Costed {
    ssa: f64,
    hw: f64,
    loops: Vec<String>,
    notes: Vec<String>,
    /// the ranges of the results, for the caller
    rets: Vec<Range>,
}

pub struct Coster<'a> {
    module: &'a Module,
    ks: Option<&'a HashMap<String, f64>>,
    /// with a target: what the platform replaces by an instruction
    natives: Option<Natives>,
    /// a trip count to assume for a loop that shows none
    assume: Option<i64>,
    /// per function and what its call site knew
    memo: HashMap<(String, Vec<Range>), Costed>,
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
        let c = self.cost(name, &[]);
        let k = self.ks.map(|ks| ks.get(name).copied().unwrap_or(1.0));
        Some(Report { name: name.to_string(), ssa: c.ssa, hw: self.ks.map(|_| c.hw), k, loops: c.loops, notes: c.notes })
    }

    /// the cost of a function, callees included, given what its call
    /// site knows of its integer arguments (missing ones unknown)
    fn cost(&mut self, name: &str, args: &[Range]) -> Costed {
        let key = (name.to_string(), args.to_vec());
        if let Some(r) = self.memo.get(&key) {
            return r.clone();
        }
        let none = |notes: Vec<String>| Costed { ssa: 0.0, hw: 0.0, loops: Vec::new(), notes, rets: Vec::new() };
        if self.stack.iter().any(|n| n == name) {
            return none(vec![format!("{}: recursive, the cycle not costed", name)]);
        }
        let Some(f) = self.module.funcs.iter().find(|f| f.name == name) else {
            return none(vec![format!("{}: not found", name)]);
        };
        self.stack.push(name.to_string());
        let k = self.ks.map(|ks| ks.get(name).copied().unwrap_or(1.0)).unwrap_or(1.0);
        let facts = Facts::new(f);
        let dom = Dom::compute(f);
        let nb = f.blocks.len();
        let mut loops = Vec::new();
        let mut notes = Vec::new();
        let mut rets: Option<Vec<Range>> = None;
        // each block a node, walked in reverse postorder with what its
        // dominators established: the environment of ranges
        let mut env: Vec<HashMap<ValueId, Range>> = vec![HashMap::new(); nb];
        let mut nodes: Vec<Node> = (0..nb).map(|_| Node { ssa: 0.0, hw: 0.0, succs: Vec::new() }).collect();
        let entry = dom.rpo[0];
        for &b in &dom.rpo {
            let mut e = match dom.idom[b] {
                Some(d) if d != b => env[d].clone(),
                _ => HashMap::new(),
            };
            if b == entry {
                for (i, &p) in f.params.iter().enumerate() {
                    if let (Some(r), Some(_)) = (args.get(i), facts.int_type(p)) {
                        e.insert(p, *r);
                    }
                }
            } else {
                // a block's parameters: the union of what its entries
                // pass; a loop header's from its entries and the sign of
                // its step — a latch that only adds keeps the start as
                // the floor, one that only subtracts keeps it as the
                // ceiling
                let ps = f.blocks[b].params.clone();
                for (pi, &p) in ps.iter().enumerate() {
                    let mut init: Option<Range> = None;
                    let mut steps: Vec<Option<i128>> = Vec::new();
                    for &q in &dom.preds[b] {
                        if dom.rpo_index[q] == usize::MAX {
                            continue;
                        }
                        for a in args_to(f, q, b) {
                            let Some(&v) = a.get(pi) else { continue };
                            if dom.rpo_index[q] >= dom.rpo_index[b] {
                                steps.push(facts.step_of(v, p));
                            } else {
                                let r = facts.rng(&env[q], v);
                                init = Some(match init {
                                    None => r,
                                    Some(i) => Range::join(i, r),
                                });
                            }
                        }
                    }
                    let init = init.unwrap_or(Range::UNKNOWN);
                    let r = if steps.is_empty() {
                        init
                    } else if steps.iter().all(|s| s.is_some_and(|s| s >= 0)) {
                        Range { lo: init.lo, hi: if steps.iter().all(|s| *s == Some(0)) { init.hi } else { None } }
                    } else if steps.iter().all(|s| s.is_some_and(|s| s <= 0)) {
                        Range { lo: None, hi: init.hi }
                    } else {
                        Range::UNKNOWN
                    };
                    if facts.int_type(p).is_some() {
                        e.insert(p, r);
                    }
                }
                // a branch that is the only way in says which way it went
                if let [q] = dom.preds[b].as_slice() {
                    if let Some(Inst::Br { cond, then_target, else_target, .. }) = f.blocks[*q].insts.last() {
                        let (t, el) = (then_target.0 as usize, else_target.0 as usize);
                        if (t == b) != (el == b) {
                            facts.refine(&mut e, *cond, t == b);
                        }
                    }
                }
            }
            let (mut ssa, mut hw) = (0.0, 0.0);
            for inst in &f.blocks[b].insts {
                match inst {
                    Inst::Jmp { .. } => continue,
                    Inst::IConst { dst, imm } => {
                        e.insert(*dst, Range::point(*imm));
                    }
                    Inst::Bin { op, dst, lhs, rhs } => {
                        if let Some((signed, _)) = facts.int_type(*dst) {
                            let (a, c) = (facts.rng(&e, *lhs), facts.rng(&e, *rhs));
                            let r = match op {
                                BinOp::IAdd => Range::add(a, c),
                                BinOp::ISub => Range::sub(a, c),
                                BinOp::IMul => Range::mul(a, c),
                                BinOp::Div => match c.is_point() {
                                    Some(d) if d > 0 => Range::div(a, d),
                                    _ => Range::UNKNOWN,
                                },
                                BinOp::Rem => match c.is_point() {
                                    Some(d) if d > 0 => Range::rem(a, d, signed),
                                    _ => Range::UNKNOWN,
                                },
                                BinOp::And if a.lo.is_some_and(|l| l >= 0) && c.lo.is_some_and(|l| l >= 0) => Range { lo: Some(0), hi: min_opt(a.hi, c.hi) },
                                _ => Range::UNKNOWN,
                            };
                            let r = r.within(f.values[dst.0 as usize].ty);
                            if r != Range::UNKNOWN {
                                e.insert(*dst, r);
                            }
                        }
                    }
                    Inst::Cast { dst, src, .. } => {
                        if facts.int_type(*dst).is_some() && facts.int_type(*src).is_some() {
                            let r = facts.rng(&e, *src).converted(f.values[dst.0 as usize].ty);
                            e.insert(*dst, r);
                        }
                    }
                    Inst::Check { cond } => facts.refine(&mut e, *cond, true),
                    Inst::Call { dsts, callee, args: cargs } => {
                        let callee = callee.clone();
                        let g = self.module.funcs.iter().find(|g| g.name == callee);
                        let arithmetic = g.is_some_and(is_arithmetic);
                        let native = self.natives.as_ref().is_some_and(|n| n.get(&callee).is_some());
                        let ranges: Vec<Range> = cargs.iter().map(|&a| if facts.int_type(a).is_some() { facts.rng(&e, a) } else { Range::UNKNOWN }).collect();
                        if arithmetic {
                            // the few whose result's range follows
                            if let (Some(g), [d]) = (g, dsts.as_slice()) {
                                let r = match (g.instance.as_ref().map(|(n, _)| n.as_str()), ranges.as_slice()) {
                                    (Some("min"), [a, c]) => Range::min(*a, *c),
                                    (Some("max"), [a, c]) => Range::max(*a, *c),
                                    (Some("abs"), [a]) if a.lo.is_some_and(|l| l >= 0) => *a,
                                    _ => Range::UNKNOWN,
                                };
                                if r != Range::UNKNOWN && facts.int_type(*d).is_some() {
                                    e.insert(*d, r);
                                }
                            }
                        }
                        if arithmetic && (native || self.ks.is_none()) {
                            // one operation, an instruction where the platform has one
                            ssa += 1.0;
                            hw += k;
                            continue;
                        }
                        let c = self.cost(&callee, &ranges);
                        ssa += if arithmetic { 1.0 } else { 1.0 + c.ssa };
                        hw += k + c.hw;
                        loops.extend(c.loops);
                        notes.extend(c.notes);
                        if !arithmetic && c.rets.len() == dsts.len() {
                            for (d, r) in dsts.iter().zip(&c.rets) {
                                if *r != Range::UNKNOWN && facts.int_type(*d).is_some() {
                                    e.insert(*d, *r);
                                }
                            }
                        }
                        continue;
                    }
                    Inst::CallInd { .. } => {
                        notes.push(format!("{}: an indirect call, its callee not costed", name));
                    }
                    Inst::Ret { vals } => {
                        let rs: Vec<Range> = vals.iter().map(|&v| if facts.int_type(v).is_some() { facts.rng(&e, v) } else { Range::UNKNOWN }).collect();
                        rets = Some(match rets.take() {
                            None => rs,
                            Some(prev) => prev.iter().zip(&rs).map(|(a, b)| Range::join(*a, *b)).collect(),
                        });
                    }
                    _ => {}
                }
                ssa += 1.0;
                hw += k;
            }
            let succs = match f.blocks[b].insts.last() {
                Some(Inst::Jmp { target, .. }) => vec![target.0 as usize],
                Some(Inst::Br { then_target, else_target, .. }) => vec![then_target.0 as usize, else_target.0 as usize],
                _ => vec![],
            };
            nodes[b] = Node { ssa, hw, succs };
            env[b] = e;
        }
        // the loops, innermost first
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
        let mut node_of: Vec<usize> = (0..nb).collect();
        for (h, latches, members) in &found {
            let hn = node_of[*h];
            let mset: BTreeSet<usize> = members.iter().map(|&m| node_of[m]).collect();
            let lset: BTreeSet<usize> = latches.iter().map(|&l| node_of[l]).collect();
            let (trip, how) = match f.blocks[*h].bound {
                Some(n) => (n as f64, "declared".to_string()),
                None => match infer_trip(&facts, &dom, &env, *h, members, latches) {
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
            // the longest path from the header to a latch, inside the
            // loop, trip times; and once the longest to a block that
            // leaves — the pass that ends the loop
            let mut memo: HashMap<usize, (f64, f64)> = HashMap::new();
            let body = longest(&nodes, hn, &|n| lset.contains(&n), &|n| mset.contains(&n), hn, &mut memo);
            let leaving: BTreeSet<usize> = mset.iter().copied().filter(|&m| nodes[m].succs.iter().any(|s| !mset.contains(s))).collect();
            let mut memo: HashMap<usize, (f64, f64)> = HashMap::new();
            let leave = longest(&nodes, hn, &|n| leaving.contains(&n), &|n| mset.contains(&n), hn, &mut memo);
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
            nodes.push(Node { ssa: trip * body.0 + leave.0, hw: trip * body.1 + leave.1, succs: exits });
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
        let total = longest(&nodes, node_of[entry], &|_| false, &|_| true, usize::MAX, &mut memo);
        self.stack.pop();
        let r = Costed { ssa: total.0, hw: total.1, loops, notes, rets: rets.unwrap_or_default() };
        self.memo.insert(key, r.clone());
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

fn ceil_div(a: i128, b: i128) -> i128 {
    (a + b - 1) / b
}

/// how many times the back edge is taken by a count from `start` in
/// steps of `step` that leaves when `p exit limit` holds
fn trips(exit: Cond, start: i128, limit: i128, step: i128) -> Option<i64> {
    let n: i128 = match exit {
        Cond::Ge if step > 0 => ceil_div((limit - start).max(0), step),
        Cond::Gt if step > 0 => {
            if limit < start {
                0
            } else {
                (limit - start) / step + 1
            }
        }
        Cond::Le if step < 0 => ceil_div((start - limit).max(0), -step),
        Cond::Lt if step < 0 => {
            if start < limit {
                0
            } else {
                (start - limit) / -step + 1
            }
        }
        Cond::Eq | Cond::Ne => {
            // run in the small
            let mut i = start;
            let mut n: i128 = 0;
            loop {
                if holds(exit, i, limit) {
                    break;
                }
                n += 1;
                if n > 1 << 24 {
                    return None;
                }
                i += step;
            }
            n
        }
        _ => return None,
    };
    if n > 1 << 24 {
        return None;
    }
    Some(n as i64)
}

/// a trip count the loop shows: a header parameter set by a value whose
/// range is known, stepped by a constant on every latch, compared with a
/// value whose range is known by the one branch that leaves. Two
/// constants give the count exactly; two ranges give the most the loop
/// can run, from the lowest start to the highest limit for a rising
/// count, the reverse for a falling one
fn infer_trip(facts: &Facts, dom: &Dom, env: &[HashMap<ValueId, Range>], h: usize, members: &BTreeSet<usize>, latches: &[usize]) -> Option<(i64, String)> {
    let f = facts.f;
    let params = &f.blocks[h].params;
    // the one exit
    let mut exit: Option<(ValueId, bool, usize)> = None; // (cond, leaves when true, its block)
    for &m in members {
        if let Some(Inst::Br { cond, then_target, else_target, .. }) = f.blocks[m].insts.last() {
            let t_out = !members.contains(&(then_target.0 as usize));
            let e_out = !members.contains(&(else_target.0 as usize));
            if t_out || e_out {
                if exit.is_some() || (t_out && e_out) {
                    return None;
                }
                exit = Some((*cond, t_out, m));
            }
        }
    }
    let (cond, leaves_when_true, xb) = exit?;
    let &(cc, lhs, rhs) = facts.cmps.get(&cond)?;
    let (pi, cc, limit_v) = if let Some(pi) = params.iter().position(|&p| p == lhs) {
        (pi, cc, rhs)
    } else if let Some(pi) = params.iter().position(|&p| p == rhs) {
        (pi, flip(cc), lhs)
    } else {
        return None;
    };
    let p = params[pi];
    let limit = facts.rng(&env[xb], limit_v);
    // the initial value, from every entry edge
    let mut init: Option<Range> = None;
    for &b in &dom.preds[h] {
        if members.contains(&b) || dom.rpo_index[b] == usize::MAX {
            continue;
        }
        for a in args_to(f, b, h) {
            let r = facts.rng(&env[b], *a.get(pi)?);
            init = Some(match init {
                None => r,
                Some(i) => Range::join(i, r),
            });
        }
    }
    let init = init?;
    // the step, the same on every latch
    let mut step: Option<i128> = None;
    for &l in latches {
        for a in args_to(f, l, h) {
            let s = facts.step_of(*a.get(pi)?, p)?;
            if step.is_some_and(|t| t != s) {
                return None;
            }
            step = Some(s);
        }
    }
    let step = step?;
    if step == 0 {
        return None;
    }
    // the loop leaves when `p exit limit` holds
    let exit = if leaves_when_true { cc } else { negate(cc) };
    let name = &f.values[p.0 as usize].name;
    let (start, lim, how) = match (init.is_point(), limit.is_point()) {
        (Some(a), Some(z)) => (a, z, format!("{} from {} by {} to {}", name, a, step, z)),
        _ => {
            let rising = step > 0 && matches!(exit, Cond::Ge | Cond::Gt);
            let falling = step < 0 && matches!(exit, Cond::Le | Cond::Lt);
            let (a, z) = if rising {
                (init.lo?, limit.hi?)
            } else if falling {
                (init.hi?, limit.lo?)
            } else {
                return None;
            };
            let at = |r: Range, v: i128, least: bool| if r.is_point().is_some() { format!("{}", v) } else if least { format!("at least {}", v) } else { format!("at most {}", v) };
            (a, z, format!("{} from {} by {} to {}", name, at(init, a, rising), step, at(limit, z, !rising)))
        }
    };
    Some((trips(exit, start, lim, step)?, how))
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

    /// a callee's loop is bounded by what its call site passes: a
    /// constant exactly, a `min` against one or a `check` as a ceiling
    #[test]
    fn a_bound_comes_through_the_call() {
        let src = "fn fill(n: i64) -> i64 {
    r: i64 = loop(i: i64 = 0, acc: i64 = 0) {
        done: u1 = cmp.ge i, n
        if done {
            break acc
        }
        acc2: i64 = add acc, i
        i2: i64 = add i, 1
        continue i2, acc2
    }
    ret r
}
fn seven() -> i64 {
    r: i64 = fill(7: i64)
    ret r
}
fn clipped(n: i64) -> i64 {
    m: i64 = min(n, 5: i64)
    r: i64 = fill(m)
    ret r
}
fn guarded(n: i64) -> i64 {
    ok: u1 = cmp.le n, 3: i64
    check ok
    r: i64 = fill(n)
    ret r
}
";
        let m = module(src);
        let mut c = super::Coster::new(&m, None, None, None);
        let fill = c.report("fill").unwrap();
        assert!(fill.notes.iter().any(|n| n.contains("no bound")), "{:?}", fill.notes);
        let seven = c.report("seven").unwrap();
        assert!(seven.loops.iter().any(|l| l.contains("x7 (i from 0 by 1 to 7)")), "{:?}", seven.loops);
        assert!(seven.notes.is_empty(), "{:?}", seven.notes);
        assert!(seven.ssa > fill.ssa * 3.0, "{} vs {}", seven.ssa, fill.ssa);
        let clipped = c.report("clipped").unwrap();
        assert!(clipped.loops.iter().any(|l| l.contains("x5 (i from 0 by 1 to at most 5)")), "{:?}", clipped.loops);
        let guarded = c.report("guarded").unwrap();
        assert!(guarded.loops.iter().any(|l| l.contains("x3 (i from 0 by 1 to at most 3)")), "{:?}", guarded.loops);
    }

    /// the pass that leaves a loop is charged once: a loop testing at
    /// the bottom runs its body a tenth time on the way out
    #[test]
    fn the_leaving_pass_is_charged() {
        let src = "fn tenfold() -> i64 {
    r: i64 = loop(i: i64 = 10, acc: i64 = 0) {
        acc2: i64 = add acc, i
        last: u1 = cmp.eq i, 1
        if last {
            break acc2
        }
        i2: i64 = sub i, 1
        continue i2, acc2
    }
    ret r
}
";
        let m = module(src);
        let mut c = super::Coster::new(&m, None, None, None);
        let r = c.report("tenfold").unwrap();
        assert!(r.loops.iter().any(|l| l.contains("x9 (i from 10 by -1 to 1)")), "{:?}", r.loops);
        // nine traversals of add, const 1, cmp, br, const 1, sub; the
        // tenth add, const, cmp, br; the two consts before, the ret
        assert_eq!(r.ssa, 9.0 * 6.0 + 4.0 + 2.0 + 1.0);
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
