//! The optimization engine: a pipeline of SSA -> SSA passes.
//!
//! Every optimization lives here, at SSA level, because that's one engine
//! serving every backend — and every pass is validated for free by the
//! verifier plus the regression suite running on four independent
//! referees. Emitters may pattern-match harder (instruction selection),
//! but they never transform.
//!
//! The pipeline is an ordered list, and an *optimization level is a prefix
//! of it*: level 0 emits immediately, each further level adds one pass.
//! Since every pass maps valid SSA to valid SSA, any prefix is a correct
//! stopping point — which is what makes gradual optimization possible: a
//! baseline compile can return in microseconds while later passes run
//! whenever there's time, re-emitting as the SSA improves. The unit of
//! gradual work is (function, pass).

use crate::regalloc::{inst_defs, inst_uses};
use crate::ssa::{BinOp, BlockId, Cond, Function, Inst, Module, Repr, ValueId};

pub type PassFn = fn(&mut Function);

/// Ordered pipeline; a level applies the first `level` passes.
pub const PASSES: &[(&str, PassFn)] = &[
    ("simplify-cfg", simplify_cfg),
    ("const-fold", const_fold),
    ("dce", dce),
    ("sink", sink),
];

pub const MAX_LEVEL: usize = PASSES.len();

pub fn optimize(module: &mut Module, level: usize) {
    for func in &mut module.funcs {
        optimize_function(func, level);
    }
}

pub fn optimize_function(func: &mut Function, level: usize) {
    for (_, pass) in &PASSES[..level.min(MAX_LEVEL)] {
        pass(func);
    }
}

// ---------------------------------------------------------------------------
// simplify-cfg: thread branches through empty forwarding blocks
//
// Lowered `if` arms are often just `jmp join(...)`; threading each
// predecessor edge straight to the join (substituting the forwarder's
// parameters with the incoming arguments) removes a taken branch on every
// backend, and the emptied blocks become unreachable and are dropped.

fn simplify_cfg(func: &mut Function) {
    // repeat until no chain is left (chains collapse one hop per round)
    loop {
        let mut forwarding: Vec<Option<(Vec<ValueId>, BlockId, Vec<ValueId>)>> =
            vec![None; func.blocks.len()];
        for (b, block) in func.blocks.iter().enumerate().skip(1) {
            if let [Inst::Jmp { target, args }] = block.insts.as_slice() {
                if target.0 as usize != b {
                    forwarding[b] = Some((block.params.clone(), *target, args.clone()));
                }
            }
        }
        if forwarding.iter().all(Option::is_none) {
            break;
        }
        let thread = |target: &mut BlockId, args: &mut Vec<ValueId>| -> bool {
            let Some((params, fwd_target, fwd_args)) = &forwarding[target.0 as usize] else {
                return false;
            };
            // don't thread into a forwarder's own cycle
            if fwd_target == target {
                return false;
            }
            let new_args: Vec<ValueId> = fwd_args
                .iter()
                .map(|&a| match params.iter().position(|&p| p == a) {
                    Some(i) => args[i],
                    None => a,
                })
                .collect();
            *target = *fwd_target;
            *args = new_args;
            true
        };
        let mut changed = false;
        for block in &mut func.blocks {
            match block.insts.last_mut() {
                Some(Inst::Jmp { target, args }) => {
                    changed |= thread(target, args);
                }
                Some(Inst::Br {
                    then_target,
                    then_args,
                    else_target,
                    else_args,
                    ..
                }) => {
                    changed |= thread(then_target, then_args);
                    changed |= thread(else_target, else_args);
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }

    // drop unreachable blocks and renumber targets
    let nb = func.blocks.len();
    let mut reach = vec![false; nb];
    let mut stack = vec![0usize];
    while let Some(b) = stack.pop() {
        if std::mem::replace(&mut reach[b], true) {
            continue;
        }
        if let Some(term) = func.blocks[b].insts.last() {
            match term {
                Inst::Jmp { target, .. } => stack.push(target.0 as usize),
                Inst::Br {
                    then_target,
                    else_target,
                    ..
                } => {
                    stack.push(then_target.0 as usize);
                    stack.push(else_target.0 as usize);
                }
                _ => {}
            }
        }
    }
    if reach.iter().all(|&r| r) {
        return;
    }
    let mut remap = vec![BlockId(0); nb];
    let mut kept = Vec::new();
    for (b, block) in std::mem::take(&mut func.blocks).into_iter().enumerate() {
        if reach[b] {
            remap[b] = BlockId(kept.len() as u32);
            kept.push(block);
        }
    }
    for block in &mut kept {
        match block.insts.last_mut() {
            Some(Inst::Jmp { target, .. }) => *target = remap[target.0 as usize],
            Some(Inst::Br {
                then_target,
                else_target,
                ..
            }) => {
                *then_target = remap[then_target.0 as usize];
                *else_target = remap[else_target.0 as usize];
            }
            _ => {}
        }
    }
    func.blocks = kept;
}

// ---------------------------------------------------------------------------
// const-fold: evaluate instructions whose operands are all constants
//
// Folded i32 results are stored zero-extended (the value convention the
// backends normalize to); reads of i32 constants interpret the low 32
// bits. Division by a constant zero (and MIN/-1) is left alone — its
// behavior is target-defined (wasm traps, the native ISAs don't).

fn const_fold(func: &mut Function) {
    for _round in 0..10 {
        let mut consts: Vec<Option<i64>> = vec![None; func.values.len()];
        for block in &func.blocks {
            for inst in &block.insts {
                if let Inst::IConst { dst, imm } = inst {
                    consts[dst.0 as usize] = Some(norm(func.repr(func.ty(*dst)), *imm as i64));
                }
            }
        }
        let get = |v: ValueId| consts[v.0 as usize];
        // decide every replacement on the immutable function, then apply
        let mut new_insts: Vec<(usize, usize, Vec<Inst>)> = Vec::new();
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.insts.iter().enumerate() {
                let folded: Option<(ValueId, i64)> = match inst {
                    Inst::Bin { op, dst, lhs, rhs } => match (get(*lhs), get(*rhs)) {
                        (Some(a), Some(b)) => {
                            let r = func.repr(func.values[dst.0 as usize].ty);
                            fold_bin(*op, r, a, b).map(|v| (*dst, v))
                        }
                        _ => None,
                    },
                    Inst::ICmp {
                        cond,
                        dst,
                        lhs,
                        rhs,
                    } => match (get(*lhs), get(*rhs)) {
                        (Some(a), Some(b)) => {
                            let r = func.repr(func.values[lhs.0 as usize].ty);
                            Some((*dst, fold_cmp(*cond, r, a, b) as i64))
                        }
                        _ => None,
                    },
                    // every cast of a canonical value is "re-normalize for
                    // the destination": ext keeps the value (or reinterprets
                    // a negative as unsigned), trunc keeps the low bits,
                    // bitcast keeps all the bits
                    Inst::Cast { dst, src, .. } => get(*src).map(|a| {
                        let to = func.repr(func.values[dst.0 as usize].ty);
                        (*dst, norm(to, a))
                    }),
                    Inst::Get { dst, src, field } => get(*src).map(|a| {
                        let (off, fty) = func.field(func.ty(*src), *field).unwrap();
                        let r = func.repr(fty);
                        (*dst, norm(r, a >> off))
                    }),
                    Inst::Set {
                        dst,
                        src,
                        field,
                        val,
                    } => match (get(*src), get(*val)) {
                        (Some(a), Some(v)) => {
                            let (off, fty) = func.field(func.ty(*src), *field).unwrap();
                            let w = func.width(fty).unwrap();
                            let mask = low_mask(w) << off;
                            let r = func.repr(func.ty(*dst));
                            Some((*dst, norm(r, (a & !mask) | ((v << off) & mask))))
                        }
                        _ => None,
                    },
                    Inst::Pack { dst, args } => {
                        let vals: Option<Vec<i64>> = args.iter().map(|&a| get(a)).collect();
                        vals.map(|vals| {
                            let p = func.pack(func.ty(*dst)).unwrap();
                            let mut acc = 0i64;
                            for (k, v) in vals.iter().enumerate() {
                                let w = func.width(p.fields[k].1).unwrap();
                                acc |= (v & low_mask(w)) << p.offsets[k];
                            }
                            (*dst, norm(func.repr(func.ty(*dst)), acc))
                        })
                    }
                    Inst::Unpack { dsts, src } => {
                        if let Some(a) = get(*src) {
                            let ty = func.ty(*src);
                            let consts: Vec<Inst> = dsts
                                .iter()
                                .enumerate()
                                .map(|(k, &d)| {
                                    let (off, fty) = func.field(ty, k as u32).unwrap();
                                    Inst::IConst {
                                        dst: d,
                                        imm: norm(func.repr(fty), a >> off) as i128,
                                    }
                                })
                                .collect();
                            new_insts.push((bi, ii, consts));
                        }
                        None
                    }
                    _ => None,
                };
                if let Some((dst, imm)) = folded {
                    new_insts.push((bi, ii, vec![Inst::IConst { dst, imm: imm as i128 }]));
                }
            }
        }
        if new_insts.is_empty() {
            break;
        }
        // later indices first, so an unpack's several iconsts don't shift
        // the positions still to be replaced
        for (bi, ii, consts) in new_insts.into_iter().rev() {
            func.blocks[bi].insts.splice(ii..ii + 1, consts);
        }
    }
}

fn low_mask(bits: u32) -> i64 {
    if bits >= 64 {
        -1
    } else {
        ((1u64 << bits) - 1) as i64
    }
}

/// The canonical i64 holding an N-bit value: sign-extended for signed
/// types, zero-extended for unsigned (this is also the value every backend
/// keeps in a register, and what the suite prints).
pub fn norm(r: Repr, v: i64) -> i64 {
    let n = r.bits();
    if n >= 64 {
        return v;
    }
    let shift = 64 - n;
    match r {
        Repr::S(_) => (v << shift) >> shift,
        Repr::U(_) => ((v as u64) << shift >> shift) as i64,
    }
}

/// N-bit arithmetic on canonical values: compute in 64 bits, then
/// re-normalize — exactly the emitters' strategy, so this doubles as the
/// reference model the exhaustive backend tests compare against.
pub fn fold_bin(op: BinOp, r: Repr, a: i64, b: i64) -> Option<i64> {
    let n = r.bits();
    let signed = r.signed();
    let v = match op {
        BinOp::IAdd => a.wrapping_add(b),
        BinOp::ISub => a.wrapping_sub(b),
        BinOp::IMul => a.wrapping_mul(b),
        BinOp::Div | BinOp::Rem if b == 0 => return None,
        // the only 64-bit overflow; narrow MIN/-1 wraps like the hardware
        BinOp::Div | BinOp::Rem if signed && n == 64 && a == i64::MIN && b == -1 => return None,
        BinOp::Div => {
            if signed {
                a.wrapping_div(b)
            } else {
                ((a as u64) / (b as u64)) as i64
            }
        }
        BinOp::Rem => {
            if signed {
                a.wrapping_rem(b)
            } else {
                ((a as u64) % (b as u64)) as i64
            }
        }
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        // the hardware masks 32/64-bit amounts; for narrower types an
        // amount >= n is unspecified, so it is left unfolded
        BinOp::Shl | BinOp::Shr if (n == 32 || n == 64) => {
            let k = ((b as u64) % n as u64) as u32;
            shift(op, signed, a, k)
        }
        BinOp::Shl | BinOp::Shr if (b as u64) >= n as u64 => return None,
        BinOp::Shl | BinOp::Shr => shift(op, signed, a, b as u32),
    };
    Some(norm(r, v))
}

fn shift(op: BinOp, signed: bool, a: i64, k: u32) -> i64 {
    match op {
        BinOp::Shl => a.wrapping_shl(k),
        _ if signed => a >> k,
        _ => ((a as u64) >> k) as i64,
    }
}

pub fn fold_cmp(cond: Cond, r: Repr, a: i64, b: i64) -> bool {
    let (ua, ub) = (a as u64, b as u64);
    let lt = if r.signed() { a < b } else { ua < ub };
    let gt = if r.signed() { a > b } else { ua > ub };
    match cond {
        Cond::Eq => a == b,
        Cond::Ne => a != b,
        Cond::Lt => lt,
        Cond::Le => !gt,
        Cond::Gt => gt,
        Cond::Ge => !lt,
    }
}

// ---------------------------------------------------------------------------
// dce: drop side-effect-free instructions whose results are never used
//
// Stores, calls, and terminators always stay. Divisions stay unless the
// divisor is a nonzero constant (their by-zero behavior is observable on
// wasm). Dead loads go — removing a load never changes a program that
// wasn't already faulting.

fn dce(func: &mut Function) {
    let mut consts: Vec<Option<i64>> = vec![None; func.values.len()];
    for block in &func.blocks {
        for inst in &block.insts {
            if let Inst::IConst { dst, imm } = inst {
                consts[dst.0 as usize] = Some(*imm as i64);
            }
        }
    }
    let removable = |inst: &Inst| match inst {
        Inst::IConst { .. }
        | Inst::ICmp { .. }
        | Inst::Cast { .. }
        | Inst::Pack { .. }
        | Inst::Unpack { .. }
        | Inst::Get { .. }
        | Inst::Set { .. }
        | Inst::PtrAdd { .. }
        | Inst::Addr { .. }
        | Inst::Scratch { .. }
        | Inst::FnAddr { .. }
        | Inst::Platform { .. }
        | Inst::Load { .. } => true,
        Inst::Bin { op, rhs, .. } => match op {
            BinOp::Div | BinOp::Rem => matches!(consts[rhs.0 as usize], Some(v) if v != 0),
            _ => true,
        },
        _ => false,
    };
    loop {
        let mut use_count = vec![0usize; func.values.len()];
        let mut uses = Vec::new();
        for block in &func.blocks {
            for inst in &block.insts {
                uses.clear();
                inst_uses(inst, &mut uses);
                for &u in &uses {
                    use_count[u.0 as usize] += 1;
                }
            }
        }
        let mut changed = false;
        let mut defs = Vec::new();
        for block in &mut func.blocks {
            let n = block.insts.len();
            block.insts.retain(|inst| {
                defs.clear();
                inst_defs(inst, &mut defs);
                let dead = !defs.is_empty()
                    && defs.iter().all(|d| use_count[d.0 as usize] == 0)
                    && removable(inst);
                !dead
            });
            changed |= block.insts.len() != n;
        }
        if !changed {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// sink: within-block scheduling to reduce live ranges
//
// Consumers can't generally move up past what they depend on, so this
// sinks *producers* down toward their consumers: each block is rebuilt
// bottom-up, and at every step the instruction placed above the current
// suffix is preferably one whose result the just-placed instruction
// consumes. Chains cluster, definitions land next to their uses, and live
// intervals shrink — exactly what linear scan prices in registers.
// Loads, stores, and calls keep their original relative order.

enum Class {
    Pure,
    Memory,
}

fn class(inst: &Inst) -> Class {
    match inst {
        Inst::Load { .. } | Inst::Store { .. } | Inst::Call { .. } | Inst::CallInd { .. } => Class::Memory,
        _ => Class::Pure,
    }
}

fn sink(func: &mut Function) {
    for b in 0..func.blocks.len() {
        sink_block(func, b);
    }
}

fn sink_block(func: &mut Function, b: usize) {
    let insts = std::mem::take(&mut func.blocks[b].insts);
    if insts.len() <= 2 {
        func.blocks[b].insts = insts;
        return;
    }
    let m = insts.len() - 1; // the terminator stays last
    let body = &insts[..m];

    let nvals = func.values.len();
    let mut producer: Vec<Option<usize>> = vec![None; nvals];
    let mut defs = Vec::new();
    for (i, inst) in body.iter().enumerate() {
        defs.clear();
        inst_defs(inst, &mut defs);
        for &d in &defs {
            producer[d.0 as usize] = Some(i);
        }
    }

    let mut uses = Vec::new();
    let mut producers_of: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut pending_consumers = vec![0usize; m];
    for (i, inst) in body.iter().enumerate() {
        uses.clear();
        inst_uses(inst, &mut uses);
        for &u in &uses {
            if let Some(p) = producer[u.0 as usize] {
                if p != i {
                    producers_of[i].push(p);
                    pending_consumers[p] += 1;
                }
            }
        }
    }

    let mem_order: Vec<usize> = (0..m)
        .filter(|&i| matches!(class(&body[i]), Class::Memory))
        .collect();
    let mut mem_next = mem_order.len();

    let mut wanted: Vec<ValueId> = Vec::new();
    inst_uses(&insts[m], &mut wanted);

    let mut placed = vec![false; m];
    let mut order_rev: Vec<usize> = Vec::with_capacity(m);
    for _ in 0..m {
        let ready = |i: usize| {
            !placed[i]
                && pending_consumers[i] == 0
                && match class(&body[i]) {
                    Class::Pure => true,
                    Class::Memory => mem_next > 0 && mem_order[mem_next - 1] == i,
                }
        };
        let chosen = wanted
            .iter()
            .filter_map(|u| producer[u.0 as usize])
            .filter(|&p| ready(p))
            .max()
            .or_else(|| (0..m).filter(|&i| ready(i)).max())
            .expect("schedule: no ready instruction (dependency cycle?)");

        placed[chosen] = true;
        order_rev.push(chosen);
        if matches!(class(&body[chosen]), Class::Memory) {
            mem_next -= 1;
        }
        for &p in &producers_of[chosen] {
            pending_consumers[p] -= 1;
        }
        wanted.clear();
        inst_uses(&body[chosen], &mut wanted);
    }

    let mut new_insts: Vec<Inst> = order_rev
        .into_iter()
        .rev()
        .map(|i| body[i].clone())
        .collect();
    new_insts.push(insts[m].clone());
    func.blocks[b].insts = new_insts;
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::ssa;

    fn opt_at(src: &str, level: usize) -> ssa::Module {
        let mut m = ssa::parse(src).expect("parse");
        ssa::verify(&m).expect("verify before");
        super::optimize(&mut m, level);
        ssa::verify(&m).expect("verify after");
        m
    }

    #[test]
    fn folds_and_removes_constants() {
        let src = r"
fn f() -> i64 {
entry:
    a: i64 = const 6
    b: i64 = const 7
    c: i64 = mul a, b
    d: u1 = cmp.lt a, b
    e: i64 = conv d
    r: i64 = add c, e
    ret r
}
";
        let m = opt_at(src, super::MAX_LEVEL);
        // everything folds to const 43 (42 + the u1 comparison), dead consts removed
        let insts = &m.funcs[0].blocks[0].insts;
        assert_eq!(insts.len(), 2, "{}", m);
        assert!(matches!(insts[0], ssa::Inst::IConst { imm: 43, .. }), "{}", m);
    }

    #[test]
    fn threads_forwarding_blocks() {
        // the structured lowering of `if c { break }` produces arms that
        // are pure forwarders; simplify-cfg must thread and drop them
        let src = r"
fn count(n: i64) -> i64 {
    zero: i64 = const 0
    r: i64 = loop(i: i64 = zero) {
        done: u1 = cmp.ge i, n
        if done {
            break i
        }
        one: i64 = const 1
        i2: i64 = add i, one
        continue i2
    }
    ret r
}
";
        let before = {
            let m = ssa::parse(src).unwrap();
            m.funcs[0].blocks.len()
        };
        let m = opt_at(src, super::MAX_LEVEL);
        assert!(
            m.funcs[0].blocks.len() < before,
            "expected fewer blocks: {}",
            m
        );
        // no block may remain a pure forwarder
        for block in &m.funcs[0].blocks {
            assert!(
                !matches!(block.insts.as_slice(), [ssa::Inst::Jmp { .. }]) || block.params.is_empty(),
                "{}",
                m
            );
        }
    }

    #[test]
    fn all_levels_verify() {
        let src = r"
fn g(a: i64, b: i64) -> i64 {
entry:
    two: i64 = const 2
    c: u1 = cmp.lt a, b
    br c, x, y
x:
    jmp join(two)
y:
    d: i64 = mul a, two
    jmp join(d)
join(v: i64):
    ret v
}
";
        for level in 0..=super::MAX_LEVEL {
            opt_at(src, level);
        }
    }
}
