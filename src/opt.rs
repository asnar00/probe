//! Within-block instruction scheduling to reduce live ranges.
//!
//! Consumers can't generally move up past what they depend on, so this
//! sinks *producers* down toward their consumers instead: each block is
//! rebuilt bottom-up, and at every step the instruction placed above the
//! current suffix is preferably one whose result the just-placed
//! instruction consumes. Chains of dependent instructions cluster
//! together, definitions land next to their uses, and live intervals
//! shrink — which is exactly what the linear-scan allocator prices in
//! registers.
//!
//! Legality: data dependencies are respected by construction (an
//! instruction is only placeable once all its in-block consumers are
//! placed), and memory/observable order is preserved conservatively —
//! loads, stores, and calls keep their original relative order (loads
//! could legally swap with loads; not worth the analysis yet).

use crate::regalloc::{inst_defs, inst_uses};
use crate::ssa::{Function, Inst, Module, ValueId};

enum Class {
    Pure,
    Memory, // load/store/call: mutual order preserved
}

fn class(inst: &Inst) -> Class {
    match inst {
        Inst::Load { .. } | Inst::Store { .. } | Inst::Call { .. } => Class::Memory,
        _ => Class::Pure,
    }
}

pub fn sink_module(module: &mut Module) {
    for func in &mut module.funcs {
        for b in 0..func.blocks.len() {
            sink_block(func, b);
        }
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

    // producer index per value defined in this body
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

    // dependency edges: consumer -> its in-block producers (one per use
    // occurrence, so the pending-consumer counts stay balanced)
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

    // memory instructions must be picked in reverse original order
    let mem_order: Vec<usize> = (0..m)
        .filter(|&i| matches!(class(&body[i]), Class::Memory))
        .collect();
    let mut mem_next = mem_order.len();

    // values the most recently placed instruction consumes: placing their
    // producers directly above it is the whole point
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
        // prefer a producer of the just-placed instruction; otherwise the
        // latest-original ready instruction (stability)
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
