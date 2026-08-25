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

use crate::ssa::{Cond, Function, Module, Type};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FOp {
    Add,
    Sub,
    Mul,
    Div,
    Sqrt,
    Neg,
    Abs,
}

impl FOp {
    pub fn arity(self) -> usize {
        match self {
            FOp::Sqrt | FOp::Neg | FOp::Abs => 1,
            _ => 2,
        }
    }
}

/// a value kind a conversion goes between
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    F32,
    F64,
    I32,
    U32,
    I64,
    U64,
}

impl Kind {
    pub fn is_float(self) -> bool {
        matches!(self, Kind::F32 | Kind::F64)
    }
    pub fn bits(self) -> u32 {
        match self {
            Kind::F32 | Kind::I32 | Kind::U32 => 32,
            _ => 64,
        }
    }
    pub fn signed(self) -> bool {
        matches!(self, Kind::I32 | Kind::I64)
    }
}

/// something the platform has an instruction for
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Native {
    /// an arithmetic op on 32- or 64-bit floats
    Arith { op: FOp, bits: u32 },
    /// a value conversion between kinds
    Conv { from: Kind, to: Kind },
    /// an IEEE comparison of two floats to a u1 (unordered is false,
    /// except for ne)
    Cmp { cond: Cond, bits: u32 },
}

pub struct Platform {
    ops: Vec<(&'static str, Vec<i64>, Native)>,
    /// conversions the platform has, as (from, to)
    convs: Vec<(Kind, Kind)>,
}

const FLOATS: [Kind; 2] = [Kind::F32, Kind::F64];
const INTS: [Kind; 4] = [Kind::I32, Kind::U32, Kind::I64, Kind::U64];

/// set by `--soft`: every backend's default platform becomes empty
static SOFT: AtomicBool = AtomicBool::new(false);

pub fn set_soft(soft: bool) {
    SOFT.store(soft, Ordering::Relaxed);
}

impl Platform {
    pub fn none() -> Platform {
        Platform {
            ops: Vec::new(),
            convs: Vec::new(),
        }
    }

    /// the float arithmetic every target has, plus the conversions given
    fn with_floats(convs: Vec<(Kind, Kind)>) -> Platform {
        if SOFT.load(Ordering::Relaxed) {
            return Platform::none();
        }
        Platform {
            ops: [
                ("add", FOp::Add),
                ("sub", FOp::Sub),
                ("mul", FOp::Mul),
                ("div", FOp::Div),
                ("sqrt", FOp::Sqrt),
                ("neg", FOp::Neg),
                ("abs", FOp::Abs),
            ]
            .into_iter()
            .flat_map(|(name, op)| {
                [
                    (name, vec![8, 23], Native::Arith { op, bits: 32 }),
                    (name, vec![11, 52], Native::Arith { op, bits: 64 }),
                ]
            })
            .chain(
                [("eq", Cond::Eq), ("ne", Cond::Ne), ("lt", Cond::Lt), ("le", Cond::Le), ("gt", Cond::Gt), ("ge", Cond::Ge)]
                    .into_iter()
                    .flat_map(|(name, cond)| {
                        [
                            (name, vec![8, 23], Native::Cmp { cond, bits: 32 }),
                            (name, vec![11, 52], Native::Cmp { cond, bits: 64 }),
                        ]
                    }),
            )
            .collect(),
            convs,
        }
    }

    /// float <-> float and int -> float
    fn convs_widen_and_from_int() -> Vec<(Kind, Kind)> {
        let mut v = vec![(Kind::F32, Kind::F64), (Kind::F64, Kind::F32)];
        for i in INTS {
            for f in FLOATS {
                v.push((i, f));
            }
        }
        v
    }

    /// ... and float -> int, truncating and saturating with NaN to 0
    fn convs_all() -> Vec<(Kind, Kind)> {
        let mut v = Platform::convs_widen_and_from_int();
        for f in FLOATS {
            for i in INTS {
                v.push((f, i));
            }
        }
        v
    }

    pub fn arm64() -> Platform {
        Platform::with_floats(Platform::convs_all())
    }

    /// riscv64's float -> int gives the maximum integer for NaN where the
    /// library gives 0, so those stay in the library
    pub fn riscv64() -> Platform {
        Platform::with_floats(Platform::convs_widen_and_from_int())
    }

    pub fn wasm32() -> Platform {
        Platform::with_floats(Platform::convs_all())
    }

    /// the kind of a type, if it is one the platform's instructions take:
    /// a 32/64-bit `float(...)` pack, or a 32/64-bit integer
    fn kind(f: &Function, t: Type) -> Option<Kind> {
        match t {
            Type::Int { signed, bits: 32 } => Some(if signed { Kind::I32 } else { Kind::U32 }),
            Type::Int { signed, bits: 64 } => Some(if signed { Kind::I64 } else { Kind::U64 }),
            Type::Pack(_) => {
                let p = f.pack(t)?;
                if !matches!(&p.origin, Some((name, _)) if name == "float") {
                    return None;
                }
                match p.width {
                    32 => Some(Kind::F32),
                    64 => Some(Kind::F64),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// the native op standing in for `f`, if this platform has one and
    /// `f` has the signature the op expects (the library defines the
    /// meaning; the shape must match for the substitution to be sound)
    pub fn lookup(&self, f: &Function) -> Option<Native> {
        let (generic, args) = f.instance.as_ref()?;
        if f.rets.len() != 1 {
            return None;
        }
        if generic == "conv" && f.params.len() == 1 {
            let from = Platform::kind(f, f.ty(f.params[0]))?;
            let to = Platform::kind(f, f.rets[0])?;
            return self.convs.contains(&(from, to)).then_some(Native::Conv { from, to });
        }
        let op = self
            .ops
            .iter()
            .find(|(g, a, _)| g == generic && a == args)
            .map(|(_, _, op)| *op)?;
        let (arity, bits, ret_float) = match op {
            Native::Arith { op: fop, bits } => (fop.arity(), bits, true),
            Native::Cmp { bits, .. } => (2, bits, false),
            Native::Conv { .. } => return None,
        };
        let is_float = |t: Type| matches!(Platform::kind(f, t), Some(k) if k.is_float() && k.bits() == bits);
        let shape_ok = f.params.len() == arity
            && f.params.iter().all(|&p| is_float(f.ty(p)))
            && if ret_float { is_float(f.rets[0]) } else { f.rets[0] == Type::U1 };
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
