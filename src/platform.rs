//! The platform: what a target implements natively, described as the
//! SSA functions it stands in for.
//!
//! Semantics live in SSA libraries (suite/float.ssa defines what
//! `add(8, 23)` on a float(8, 23) means, with integer instructions). A platform lists the
//! generic instantiations it has hardware for; when an emitter meets a
//! call to one of them it emits the instruction sequence instead of the
//! call — and the instance itself compiles to that sequence, so callers
//! from outside (the harness, a JIT call by name) get the hardware too.
//! The library body is the reference the hardware path is verified
//! against; `--soft` compiles with an empty platform so both paths stay
//! comparable.

use crate::ssa::{Function, Module, Type};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// a floating-point op the platform has, on 32- or 64-bit floats
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Native {
    pub op: FOp,
    pub bits: u32,
}

pub struct Platform {
    ops: Vec<(&'static str, Vec<i64>, Native)>,
}

/// set by `--soft`: every backend's default platform becomes empty
static SOFT: AtomicBool = AtomicBool::new(false);

pub fn set_soft(soft: bool) {
    SOFT.store(soft, Ordering::Relaxed);
}

impl Platform {
    pub fn none() -> Platform {
        Platform { ops: Vec::new() }
    }

    fn with_floats() -> Platform {
        if SOFT.load(Ordering::Relaxed) {
            return Platform::none();
        }
        Platform {
            ops: [("add", FOp::Add), ("sub", FOp::Sub), ("mul", FOp::Mul), ("div", FOp::Div)]
                .into_iter()
                .flat_map(|(name, op)| {
                    [
                        (name, vec![8, 23], Native { op, bits: 32 }),
                        (name, vec![11, 52], Native { op, bits: 64 }),
                    ]
                })
                .collect(),
        }
    }

    pub fn arm64() -> Platform {
        Platform::with_floats()
    }

    pub fn riscv64() -> Platform {
        Platform::with_floats()
    }

    pub fn wasm32() -> Platform {
        Platform::with_floats()
    }

    /// the native op standing in for `f`, if this platform has one and
    /// `f` has the signature the op expects (the library defines the
    /// meaning; the shape must match for the substitution to be sound)
    pub fn lookup(&self, f: &Function) -> Option<Native> {
        let (generic, args) = f.instance.as_ref()?;
        let op = self
            .ops
            .iter()
            .find(|(g, a, _)| g == generic && a == args)
            .map(|(_, _, op)| *op)?;
        let is_float = |t: Type| f.pack(t).is_some() && f.width(t) == Some(op.bits);
        let shape_ok = f.params.len() == 2
            && f.rets.len() == 1
            && f.params.iter().all(|&p| is_float(f.ty(p)))
            && is_float(f.rets[0]);
        shape_ok.then_some(op)
    }

    /// callee name -> native op, for every function of a module
    pub fn natives(&self, m: &Module) -> HashMap<String, Native> {
        m.funcs
            .iter()
            .filter_map(|f| self.lookup(f).map(|op| (f.name.clone(), op)))
            .collect()
    }
}
