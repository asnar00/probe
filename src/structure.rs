//! Structure for a stack machine: wasm has no goto, only nested
//! `block`/`loop`/`if` and `br` to an enclosing label, so a control-flow
//! graph has to be expressed as nesting. For a reducible graph — every
//! loop entered only at its header, which is all the parser and the
//! passes ever produce — the dominator tree gives the nesting outright
//! (after Ramsey, "Beyond Relooper"): a loop header becomes a `loop`, a
//! block with several forward predecessors becomes a `block` ending
//! where it starts, and everything else is emitted in place where its
//! one predecessor branches to it. An irreducible graph is reported and
//! the emitter falls back to its dispatcher loop.

use crate::ssa::{Function, Inst};

pub struct Cfg {
    /// children in the dominator tree, each list in rpo order
    pub dom_children: Vec<Vec<usize>>,
    /// the target of a back edge
    pub loop_header: Vec<bool>,
    /// a block with more than one forward-edge predecessor
    pub merge: Vec<bool>,
}

fn successors(inst: &Inst) -> Vec<usize> {
    match inst {
        Inst::Jmp { target, .. } => vec![target.0 as usize],
        Inst::Br { then_target, else_target, .. } => vec![then_target.0 as usize, else_target.0 as usize],
        _ => vec![],
    }
}

/// The dominator tree of a function's blocks: what the verifier's
/// dominance rule and the wasm structuring both start from
pub struct Dom {
    /// reachable blocks in reverse postorder
    pub rpo: Vec<usize>,
    /// a block's position in `rpo`; usize::MAX when unreachable
    pub rpo_index: Vec<usize>,
    pub preds: Vec<Vec<usize>>,
    /// the immediate dominator of each block; None when unreachable (the
    /// entry is its own)
    pub idom: Vec<Option<usize>>,
}

impl Dom {
    pub fn compute(f: &Function) -> Dom {
        let nb = f.blocks.len();
        let succs: Vec<Vec<usize>> = f.blocks.iter().map(|b| b.insts.last().map(successors).unwrap_or_default()).collect();
        // reverse postorder by an explicit DFS
        let mut visited = vec![false; nb];
        let mut post = Vec::new();
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        visited[0] = true;
        while let Some(&mut (b, ref mut i)) = stack.last_mut() {
            if *i < succs[b].len() {
                let s = succs[b][*i];
                *i += 1;
                if !visited[s] {
                    visited[s] = true;
                    stack.push((s, 0));
                }
            } else {
                post.push(b);
                stack.pop();
            }
        }
        let rpo: Vec<usize> = post.iter().rev().copied().collect();
        let mut rpo_index = vec![usize::MAX; nb];
        for (i, &b) in rpo.iter().enumerate() {
            rpo_index[b] = i;
        }
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
        for &b in &rpo {
            for &s in &succs[b] {
                preds[s].push(b);
            }
        }
        // dominators (Cooper, Harvey, Kennedy), over rpo positions
        let n = rpo.len();
        let mut idom_pos: Vec<Option<usize>> = vec![None; n];
        idom_pos[0] = Some(0);
        let intersect = |idom: &Vec<Option<usize>>, mut a: usize, mut b: usize| {
            while a != b {
                while a > b {
                    a = idom[a].unwrap();
                }
                while b > a {
                    b = idom[b].unwrap();
                }
            }
            a
        };
        loop {
            let mut changed = false;
            for i in 1..n {
                let b = rpo[i];
                let mut new_idom: Option<usize> = None;
                for &p in &preds[b] {
                    let pi = rpo_index[p];
                    if idom_pos[pi].is_none() {
                        continue;
                    }
                    new_idom = Some(match new_idom {
                        None => pi,
                        Some(cur) => intersect(&idom_pos, pi, cur),
                    });
                }
                if new_idom != idom_pos[i] {
                    idom_pos[i] = new_idom;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut idom = vec![None; nb];
        for i in 0..n {
            idom[rpo[i]] = idom_pos[i].map(|p| rpo[p]);
        }
        Dom { rpo, rpo_index, preds, idom }
    }

    /// does block a dominate block b? (every block dominates itself)
    pub fn dominates(&self, a: usize, mut b: usize) -> bool {
        loop {
            if a == b {
                return true;
            }
            if b == 0 {
                return false;
            }
            match self.idom[b] {
                Some(d) => b = d,
                None => return false,
            }
        }
    }
}

impl Cfg {
    /// None when the graph is irreducible
    pub fn analyze(f: &Function) -> Option<Cfg> {
        let nb = f.blocks.len();
        let Dom { rpo, rpo_index, preds, idom } = Dom::compute(f);
        let idom: Vec<usize> = idom.iter().map(|d| d.unwrap_or(0)).collect();
        let dominates = |a: usize, mut b: usize| {
            loop {
                if a == b {
                    return true;
                }
                if b == 0 {
                    return false;
                }
                b = idom[b];
            }
        };
        // back edges must go to a dominator (reducibility); merges count
        // forward predecessors
        let mut loop_header = vec![false; nb];
        let mut merge = vec![false; nb];
        for &b in &rpo {
            let mut forward = 0;
            for &p in &preds[b] {
                if rpo_index[p] >= rpo_index[b] {
                    if !dominates(b, p) {
                        return None;
                    }
                    loop_header[b] = true;
                } else {
                    forward += 1;
                }
            }
            merge[b] = forward > 1;
        }
        let mut dom_children: Vec<Vec<usize>> = vec![Vec::new(); nb];
        for &b in rpo.iter().skip(1) {
            dom_children[idom[b]].push(b);
        }
        Some(Cfg { dom_children, loop_header, merge })
    }
}

#[cfg(test)]
mod tests {
    use super::Cfg;

    /// everything the parser and the passes produce is reducible, so the
    /// suite never falls back to the dispatcher
    #[test]
    fn suite_graphs_are_reducible() {
        for entry in std::fs::read_dir("suite").unwrap().flatten() {
            let src = std::fs::read_to_string(entry.path()).unwrap();
            let policy = crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap();
            let mut m = crate::ssa::parse_with(&crate::ssa::with_prelude(&src), &policy).unwrap();
            crate::ssa::resolve_types(&mut m, &policy);
            for level in 0..=crate::opt::MAX_LEVEL {
                let mut m2 = m.clone();
                crate::opt::optimize(&mut m2, level);
                for f in &m2.funcs {
                    assert!(Cfg::analyze(f).is_some(), "{}: {} irreducible at -O{}", entry.path().display(), f.name, level);
                }
            }
        }
    }

    /// a loop entered from two sides has no header: irreducible
    #[test]
    fn irreducible_graph_is_reported() {
        let src = "fn f(a: i64) -> i64 {
entry:
    c: u1 = cmp.eq a, 0
    br c, x(a), y(a)
x(v: i64):
    w: i64 = add v, 1
    d: u1 = cmp.gt w, 10
    br d, done(w), y(w)
y(u: i64):
    t: i64 = sub u, 1
    e: u1 = cmp.lt t, -10
    br e, done(t), x(t)
done(r: i64):
    ret r
}
";
        let m = crate::ssa::parse(src).unwrap();
        assert!(Cfg::analyze(&m.funcs[0]).is_none());
    }
}
