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
    pub used_regs: Vec<i64>, // pool registers of class 0 this function touches (sorted)
    /// per class, the pool registers touched (used_regs is class 0's)
    pub used_by_class: Vec<Vec<i64>>,
    pub nslots: usize,
}

pub(crate) fn inst_uses(inst: &Inst, out: &mut Vec<ValueId>) {
    match inst {
        Inst::IConst { .. } | Inst::Addr { .. } | Inst::FnAddr { .. } | Inst::Platform { .. } => {}
        Inst::Bin { lhs, rhs, .. } | Inst::ICmp { lhs, rhs, .. } => {
            out.push(*lhs);
            out.push(*rhs);
        }
        Inst::Cast { src, .. } | Inst::Unpack { src, .. } | Inst::Get { src, .. } => {
            out.push(*src)
        }
        Inst::Pack { args, .. } => out.extend(args),
        Inst::Set { src, val, .. } => {
            out.push(*src);
            out.push(*val);
        }
        Inst::Load { addr, index, .. } => {
            out.push(*addr);
            if let Some((i, _)) = index {
                out.push(*i);
            }
        }
        Inst::Store { val, addr, index, .. } => {
            out.push(*val);
            out.push(*addr);
            if let Some((i, _)) = index {
                out.push(*i);
            }
        }
        Inst::PtrAdd { base, off, .. } => {
            out.push(*base);
            out.push(*off);
        }
        Inst::Call { args, .. } => out.extend(args),
        Inst::CallInd { callee, args, .. } => {
            out.push(*callee);
            out.extend(args);
        }
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
        | Inst::Pack { dst, .. }
        | Inst::Get { dst, .. }
        | Inst::Set { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::PtrAdd { dst, .. }
        | Inst::Addr { dst, .. }
        | Inst::FnAddr { dst, .. }
        | Inst::Platform { dst, .. } => out.push(*dst),
        Inst::Call { dsts, .. } | Inst::CallInd { dsts, .. } | Inst::Unpack { dsts, .. } => out.extend(dsts),
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

/// Register classes: `class_of[v]` indexes `pools`; each class gets its
/// own linear scan over its own pool (a value's class is fixed by its
/// type, so coalesced values always share one). Spill slots are one
/// space for all classes.
pub fn allocate_classes(func: &Function, class_of: &[usize], pools: &[&[i64]]) -> Alloc {
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

    // ---- interference: who is live at each value's definition ----
    // Precise per-point liveness (unlike the conservative spans below):
    // two values interfere iff one is live where the other is defined.
    let mut live_at_def: Vec<HashSet<ValueId>> = vec![HashSet::new(); n];
    for (b, block) in func.blocks.iter().enumerate() {
        let mut live = live_out[b].clone();
        for inst in block.insts.iter().rev() {
            defs.clear();
            inst_defs(inst, &mut defs);
            for &d in &defs {
                live.remove(&d);
            }
            for &d in &defs {
                let set = &mut live_at_def[d.0 as usize];
                set.extend(live.iter().copied());
                set.extend(defs.iter().copied().filter(|&x| x != d));
            }
            uses.clear();
            inst_uses(inst, &mut uses);
            for &u in &uses {
                live.insert(u);
            }
        }
        // block (and, in the entry block, function) parameters are defined
        // simultaneously at the block head, with live_in alongside them
        let mut params: Vec<ValueId> = block.params.clone();
        if b == 0 {
            params.extend(&func.params);
        }
        for &p in &params {
            let set = &mut live_at_def[p.0 as usize];
            set.extend(live_in[b].iter().copied());
            set.extend(params.iter().copied().filter(|&x| x != p));
        }
    }
    let interfere = |u: ValueId, v: ValueId| {
        live_at_def[v.0 as usize].contains(&u) || live_at_def[u.0 as usize].contains(&v)
    };

    // ---- coalescing: merge block parameters with their branch arguments ----
    // Sharing a register turns the parallel move on that edge into a no-op
    // (the loop back-edge case: %i2 = %i + 1; jmp ^loop(%i2) — one register).
    // Union-find; a merge is allowed only if no member of one set interferes
    // with any member of the other.
    let mut uf: Vec<usize> = (0..n).collect();
    fn find(uf: &mut Vec<usize>, mut x: usize) -> usize {
        while uf[x] != x {
            uf[x] = uf[uf[x]];
            x = uf[x];
        }
        x
    }
    let mut members: Vec<Vec<ValueId>> = (0..n).map(|i| vec![ValueId(i as u32)]).collect();
    for block in &func.blocks {
        if let Some(term) = block.insts.last() {
            let mut edges: Vec<(ValueId, ValueId)> = Vec::new();
            match term {
                Inst::Jmp { target, args } => {
                    let params = &func.blocks[target.0 as usize].params;
                    edges.extend(params.iter().copied().zip(args.iter().copied()));
                }
                Inst::Br {
                    then_target,
                    then_args,
                    else_target,
                    else_args,
                    ..
                } => {
                    let tp = &func.blocks[then_target.0 as usize].params;
                    edges.extend(tp.iter().copied().zip(then_args.iter().copied()));
                    let ep = &func.blocks[else_target.0 as usize].params;
                    edges.extend(ep.iter().copied().zip(else_args.iter().copied()));
                }
                _ => {}
            }
            for (p, a) in edges {
                let (rp, ra) = (find(&mut uf, p.0 as usize), find(&mut uf, a.0 as usize));
                if rp == ra {
                    continue;
                }
                let ok = !members[rp]
                    .iter()
                    .any(|&x| members[ra].iter().any(|&y| interfere(x, y)));
                if ok {
                    let moved = std::mem::take(&mut members[ra]);
                    members[rp].extend(moved);
                    uf[ra] = rp;
                }
            }
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

    // fold member intervals into their coalesced root's
    for i in 0..n {
        let r = find(&mut uf, i);
        if r != i && start[i] != u32::MAX {
            start[r] = start[r].min(start[i]);
            end[r] = end[r].max(end[i]);
        }
    }

    // ---- linear scan with furthest-end eviction, over coalesced roots ----
    let mut loc = vec![Loc::Slot(usize::MAX); n];
    for (class, pool) in pools.iter().enumerate() {
    let mut order: Vec<usize> = (0..n)
        .filter(|&i| find(&mut uf, i) == i && start[i] != u32::MAX && class_of[i] == class)
        .collect();
    order.sort_by_key(|&i| start[i]);
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
    }

    // spill slots, shared by spilled values whose lifetimes do not
    // overlap (a root's interval covers its coalesced members): in start
    // order, take a slot whose last occupant has ended, else a new one
    let mut root_slot: Vec<Option<usize>> = vec![None; n];
    let mut nslots = 0;
    let mut spilled: Vec<usize> = (0..n).filter(|&i| find(&mut uf, i) == i && matches!(loc[i], Loc::Slot(_))).collect();
    spilled.sort_by_key(|&i| start[i]);
    let mut free: Vec<(u32, usize)> = Vec::new(); // (end of last occupant, slot)
    for i in spilled {
        let slot = match free.iter().position(|&(e, _)| e < start[i]) {
            Some(k) => free.swap_remove(k).1,
            None => {
                nslots += 1;
                nslots - 1
            }
        };
        root_slot[i] = Some(slot);
        free.push((end[i], slot));
    }
    for i in 0..n {
        let r = find(&mut uf, i);
        loc[i] = match loc[r] {
            Loc::Reg(reg) => Loc::Reg(reg),
            Loc::Slot(_) => Loc::Slot(root_slot[r].unwrap()),
        };
    }

    // ---- collect used registers ----
    let mut used_by_class: Vec<Vec<i64>> = vec![Vec::new(); pools.len()];
    for i in 0..n {
        if let Loc::Reg(r) = loc[i] {
            let used = &mut used_by_class[class_of[i]];
            if !used.contains(&r) {
                used.push(r);
            }
        }
    }
    for used in &mut used_by_class {
        used.sort_unstable();
    }
    Alloc {
        loc,
        used_regs: used_by_class[0].clone(),
        used_by_class,
        nslots,
    }
}
