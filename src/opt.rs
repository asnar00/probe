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
use crate::ssa::{BinOp, BlockId, CastOp, Cond, Function, Inst, Module, Type, ValueId};

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
// Lowered `if` arms are often just `jmp ^join(...)`; threading each
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
                    consts[dst.0 as usize] = Some(*imm);
                }
            }
        }
        let get = |v: ValueId| consts[v.0 as usize];
        let mut changed = false;
        for block in &mut func.blocks {
            for inst in &mut block.insts {
                let folded: Option<(ValueId, i64)> = match inst {
                    Inst::Bin { op, dst, lhs, rhs } => match (get(*lhs), get(*rhs)) {
                        (Some(a), Some(b)) => {
                            fold_bin(*op, func.values[dst.0 as usize].ty, a, b)
                                .map(|v| (*dst, v))
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
                            let ty = func.values[lhs.0 as usize].ty;
                            Some((*dst, fold_cmp(*cond, ty, a, b) as i64))
                        }
                        _ => None,
                    },
                    Inst::Cast { op, dst, src } => get(*src).map(|a| {
                        let from = func.values[src.0 as usize].ty;
                        let to = func.values[dst.0 as usize].ty;
                        (*dst, fold_cast(*op, from, to, a))
                    }),
                    _ => None,
                };
                if let Some((dst, imm)) = folded {
                    *inst = Inst::IConst { dst, imm };
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn norm(ty: Type, v: i64) -> i64 {
    match ty {
        Type::I32 => (v as u32) as i64, // stored zero-extended
        Type::I1 => v & 1,
        _ => v,
    }
}

fn fold_bin(op: BinOp, ty: Type, a: i64, b: i64) -> Option<i64> {
    if op.is_float() {
        return None; // float folding: not yet (rounding-mode care needed)
    }
    let v = if ty == Type::I32 {
        let (a, b) = (a as i32, b as i32);
        let r: i32 = match op {
            BinOp::IAdd => a.wrapping_add(b),
            BinOp::ISub => a.wrapping_sub(b),
            BinOp::IMul => a.wrapping_mul(b),
            BinOp::SDiv | BinOp::SRem if b == 0 || (a == i32::MIN && b == -1) => return None,
            BinOp::UDiv | BinOp::URem if b == 0 => return None,
            BinOp::SDiv => a.wrapping_div(b),
            BinOp::UDiv => ((a as u32) / (b as u32)) as i32,
            BinOp::SRem => a.wrapping_rem(b),
            BinOp::URem => ((a as u32) % (b as u32)) as i32,
            BinOp::And => a & b,
            BinOp::Or => a | b,
            BinOp::Xor => a ^ b,
            BinOp::Shl => a.wrapping_shl(b as u32 & 31),
            BinOp::LShr => ((a as u32) >> (b as u32 & 31)) as i32,
            BinOp::AShr => a >> (b as u32 & 31),
            _ => unreachable!(),
        };
        r as i64
    } else {
        match op {
            BinOp::IAdd => a.wrapping_add(b),
            BinOp::ISub => a.wrapping_sub(b),
            BinOp::IMul => a.wrapping_mul(b),
            BinOp::SDiv | BinOp::SRem if b == 0 || (a == i64::MIN && b == -1) => return None,
            BinOp::UDiv | BinOp::URem if b == 0 => return None,
            BinOp::SDiv => a.wrapping_div(b),
            BinOp::UDiv => ((a as u64) / (b as u64)) as i64,
            BinOp::SRem => a.wrapping_rem(b),
            BinOp::URem => ((a as u64) % (b as u64)) as i64,
            BinOp::And => a & b,
            BinOp::Or => a | b,
            BinOp::Xor => a ^ b,
            BinOp::Shl => a.wrapping_shl(b as u32 & 63),
            BinOp::LShr => ((a as u64) >> (b as u64 & 63)) as i64,
            BinOp::AShr => a >> (b as u64 & 63),
            _ => unreachable!(),
        }
    };
    Some(norm(ty, v))
}

fn fold_cmp(cond: Cond, ty: Type, a: i64, b: i64) -> bool {
    let (sa, sb, ua, ub) = if ty == Type::I32 {
        (a as i32 as i64, b as i32 as i64, (a as u32) as u64, (b as u32) as u64)
    } else {
        (a, b, a as u64, b as u64)
    };
    match cond {
        Cond::Eq => ua == ub,
        Cond::Ne => ua != ub,
        Cond::Slt => sa < sb,
        Cond::Sle => sa <= sb,
        Cond::Sgt => sa > sb,
        Cond::Sge => sa >= sb,
        Cond::Ult => ua < ub,
        Cond::Ule => ua <= ub,
        Cond::Ugt => ua > ub,
        Cond::Uge => ua >= ub,
    }
}

fn fold_cast(op: CastOp, from: Type, to: Type, a: i64) -> i64 {
    let v = match (op, from) {
        (CastOp::Sext, Type::I1) => -(a & 1),
        (CastOp::Sext, Type::I32) => a as i32 as i64,
        (CastOp::Zext, Type::I1) => a & 1,
        (CastOp::Zext, Type::I32) => (a as u32) as i64,
        (CastOp::Trunc, _) => a,
        _ => a,
    };
    norm(to, v)
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
                consts[dst.0 as usize] = Some(*imm);
            }
        }
    }
    let removable = |inst: &Inst| match inst {
        Inst::IConst { .. }
        | Inst::FConst { .. }
        | Inst::ICmp { .. }
        | Inst::FCmp { .. }
        | Inst::Cast { .. }
        | Inst::PtrAdd { .. }
        | Inst::Load { .. } => true,
        Inst::Bin { op, rhs, .. } => match op {
            BinOp::SDiv | BinOp::UDiv | BinOp::SRem | BinOp::URem => {
                matches!(consts[rhs.0 as usize], Some(v) if v != 0)
            }
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
        Inst::Load { .. } | Inst::Store { .. } | Inst::Call { .. } => Class::Memory,
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
fn @f() -> i64 {
^entry:
    %a: i64 = iconst 6
    %b: i64 = iconst 7
    %c: i64 = imul %a, %b
    %d: i1 = icmp.slt %a, %b
    %e: i64 = sext %d
    %r: i64 = iadd %c, %e
    ret %r
}
";
        let m = opt_at(src, super::MAX_LEVEL);
        // everything folds to iconst 41, dead consts removed
        let insts = &m.funcs[0].blocks[0].insts;
        assert_eq!(insts.len(), 2, "{}", m);
        assert!(matches!(insts[0], ssa::Inst::IConst { imm: 41, .. }), "{}", m);
    }

    #[test]
    fn threads_forwarding_blocks() {
        // the structured lowering of `if %c { break }` produces arms that
        // are pure forwarders; simplify-cfg must thread and drop them
        let src = r"
fn @count(%n: i64) -> i64 {
    %zero: i64 = iconst 0
    %r: i64 = loop(%i: i64 = %zero) {
        %done: i1 = icmp.sge %i, %n
        if %done {
            break %i
        }
        %one: i64 = iconst 1
        %i2: i64 = iadd %i, %one
        continue %i2
    }
    ret %r
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
fn @g(%a: i64, %b: i64) -> i64 {
^entry:
    %two: i64 = iconst 2
    %c: i1 = icmp.slt %a, %b
    br %c, ^x, ^y
^x:
    jmp ^join(%two)
^y:
    %d: i64 = imul %a, %two
    jmp ^join(%d)
^join(%v: i64):
    ret %v
}
";
        for level in 0..=super::MAX_LEVEL {
            opt_at(src, level);
        }
    }
}
