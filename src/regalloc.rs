//! Linear-scan register allocation over the SSA block graph.
//!
//! Target-independent: the caller supplies a register pool and gets back a
//! location per value — a pool register or a (compacted) spill slot. The
//! pool is expected to be *callee-saved* registers, which makes call sites
//! trivial for the emitters: allocated values survive calls by the ABI
//! contract, which the emitted prologue upholds by saving exactly the
//! registers the function uses.
//!
//! Liveness is a standard backward fixpoint over blocks (block parameters
//! are definitions at the block head; branch arguments are uses at the
//! terminator). Live intervals are single [start, end] spans over a linear
//! numbering of blocks in layout order — conservative across lifetime
//! holes, which costs registers, never correctness. Allocation is the
//! classic linear scan with furthest-end eviction.

use crate::ssa::{Function, Inst, ValueId};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Loc {
    Reg(i64),
    Slot(usize), // compacted spill index
}

pub struct Alloc {
    pub loc: Vec<Loc>,       // per ValueId
    pub used_regs: Vec<i64>, // pool registers this function touches (sorted)
    pub nslots: usize,
}

pub(crate) fn inst_uses(inst: &Inst, out: &mut Vec<ValueId>) {
    match inst {
        Inst::IConst { .. } => {}
        Inst::Bin { lhs, rhs, .. } | Inst::ICmp { lhs, rhs, .. } => {
            out.push(*lhs);
            out.push(*rhs);
        }
        Inst::Cast { src, .. } => out.push(*src),
        Inst::Load { addr, .. } => out.push(*addr),
        Inst::Store { val, addr } => {
            out.push(*val);
            out.push(*addr);
        }
        Inst::PtrAdd { base, off, .. } => {
            out.push(*base);
            out.push(*off);
        }
        Inst::Call { args, .. } => out.extend(args),
        Inst::Jmp { args, .. } => out.extend(args),
        Inst::Br {
            cond,
            then_args,
            else_args,
            ..
        } => {
            out.push(*cond);
            out.extend(then_args);
            out.extend(else_args);
        }
        Inst::Ret { vals } => out.extend(vals),
    }
}

pub(crate) fn inst_defs(inst: &Inst, out: &mut Vec<ValueId>) {
    match inst {
        Inst::IConst { dst, .. }
        | Inst::Bin { dst, .. }
        | Inst::ICmp { dst, .. }
        | Inst::Cast { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::PtrAdd { dst, .. } => out.push(*dst),
        Inst::Call { dsts, .. } => out.extend(dsts),
        _ => {}
    }
}

fn successors(inst: &Inst) -> Vec<usize> {
    match inst {
        Inst::Jmp { target, .. } => vec![target.0 as usize],
        Inst::Br {
            then_target,
            else_target,
            ..
        } => vec![then_target.0 as usize, else_target.0 as usize],
        _ => vec![],
    }
}

pub fn allocate(func: &Function, pool: &[i64]) -> Alloc {
    let n = func.values.len();
    let nb = func.blocks.len();

    // ---- liveness: backward fixpoint ----
    let mut live_in: Vec<HashSet<ValueId>> = vec![HashSet::new(); nb];
    let mut live_out: Vec<HashSet<ValueId>> = vec![HashSet::new(); nb];
    let mut uses = Vec::new();
    let mut defs = Vec::new();
    loop {
        let mut changed = false;
        for b in (0..nb).rev() {
            let block = &func.blocks[b];
            let mut out: HashSet<ValueId> = HashSet::new();
            if let Some(last) = block.insts.last() {
                for s in successors(last) {
                    out.extend(live_in[s].iter().copied());
                }
            }
            let mut live = out.clone();
            for inst in block.insts.iter().rev() {
                defs.clear();
                inst_defs(inst, &mut defs);
                for &d in &defs {
                    live.remove(&d);
                }
                uses.clear();
                inst_uses(inst, &mut uses);
                for &u in &uses {
                    live.insert(u);
                }
            }
            for &p in &block.params {
                live.remove(&p);
            }
            if b == 0 {
                for &p in &func.params {
                    live.remove(&p);
                }
            }
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
            if out != live_out[b] {
                live_out[b] = out;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // ---- intervals over a linear numbering ----
    let mut start = vec![u32::MAX; n];
    let mut end = vec![0u32; n];
    let touch = |v: ValueId, pos: u32, start: &mut Vec<u32>, end: &mut Vec<u32>| {
        let i = v.0 as usize;
        start[i] = start[i].min(pos);
        end[i] = end[i].max(pos);
    };
    let mut pos = 0u32;
    for (b, block) in func.blocks.iter().enumerate() {
        let block_start = pos;
        for &v in &live_in[b] {
            touch(v, block_start, &mut start, &mut end);
        }
        for &p in &block.params {
            touch(p, block_start, &mut start, &mut end);
        }
        if b == 0 {
            for &p in &func.params {
                touch(p, block_start, &mut start, &mut end);
            }
        }
        for inst in &block.insts {
            pos += 1;
            uses.clear();
            inst_uses(inst, &mut uses);
            for &u in &uses {
                touch(u, pos, &mut start, &mut end);
            }
            defs.clear();
            inst_defs(inst, &mut defs);
            for &d in &defs {
                touch(d, pos, &mut start, &mut end);
            }
        }
        let block_end = pos + 1;
        for &v in &live_out[b] {
            touch(v, block_end, &mut start, &mut end);
        }
        pos = block_end;
    }

    // ---- linear scan with furthest-end eviction ----
    let mut order: Vec<usize> = (0..n).filter(|&i| start[i] != u32::MAX).collect();
    order.sort_by_key(|&i| start[i]);
    let mut loc = vec![Loc::Slot(usize::MAX); n];
    let mut free: Vec<i64> = pool.to_vec();
    free.reverse(); // pop from the front of the pool first
    let mut active: Vec<(u32, usize, i64)> = Vec::new(); // (end, value, reg)

    for &v in &order {
        let s = start[v];
        let mut i = 0;
        while i < active.len() {
            if active[i].0 < s {
                free.push(active[i].2);
                active.swap_remove(i);
            } else {
                i += 1;
            }
        }
        if let Some(r) = free.pop() {
            loc[v] = Loc::Reg(r);
            active.push((end[v], v, r));
        } else if let Some(idx) = active
            .iter()
            .enumerate()
            .max_by_key(|(_, a)| a.0)
            .filter(|(_, a)| a.0 > end[v])
            .map(|(i, _)| i)
        {
            // evict the interval that ends furthest away; it spills instead
            let (_, evicted, r) = active.swap_remove(idx);
            loc[evicted] = Loc::Slot(usize::MAX);
            loc[v] = Loc::Reg(r);
            active.push((end[v], v, r));
        }
        // else: v stays spilled
    }

    // ---- compact spill slots; collect used registers ----
    let mut nslots = 0;
    let mut used: Vec<i64> = Vec::new();
    for i in 0..n {
        match loc[i] {
            Loc::Slot(_) => {
                loc[i] = Loc::Slot(nslots);
                nslots += 1;
            }
            Loc::Reg(r) => {
                if !used.contains(&r) {
                    used.push(r);
                }
            }
        }
    }
    used.sort_unstable();
    Alloc {
        loc,
        used_regs: used,
        nslots,
    }
}
