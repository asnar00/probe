//! probe SSA — data structures, text parser, printer, and verifier.
//! See ssa.md for the format specification.

use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Core types

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ValueId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    /// a concrete integer, `iN` (signed) or `uN` (unsigned), 1 <= N <= 256
    /// (above 64, a row of words: see wide.rs)
    Int { signed: bool, bits: u16 },
    /// pointer (64-bit on our native targets, a 32-bit offset on wasm)
    Ptr,
    /// a pack: bitfields laid out lowest-bits-first, by index into the
    /// module's pack table (see `PackDef`)
    Pack(u32),
    /// abstract integers: resolved to a concrete width by the target's
    /// replacement policy before verification (see `resolve_types`)
    AInt,
    AUInt,
}

impl Type {
    pub const I32: Type = Type::Int { signed: true, bits: 32 };
    pub const I64: Type = Type::Int { signed: true, bits: 64 };
    pub const U1: Type = Type::Int { signed: false, bits: 1 };
    pub const U64: Type = Type::Int { signed: false, bits: 64 };

    pub fn int(signed: bool, bits: u32) -> Type {
        Type::Int {
            signed,
            bits: bits as u16,
        }
    }

    /// The type's spelling; packs print as `$#index` here — `Function::tyname`
    /// has the declared name.
    pub fn name(self) -> String {
        match self {
            Type::Int { signed: true, bits } => format!("i{}", bits),
            Type::Int { signed: false, bits } => format!("u{}", bits),
            Type::Ptr => "ptr".into(),
            Type::Pack(i) => format!("#{}", i),
            Type::AInt => "int".into(),
            Type::AUInt => "uint".into(),
        }
    }

    /// public alias for CLI flag parsing
    pub fn from_name_pub(s: &str) -> Option<Type> {
        Type::from_name(s)
    }

    /// `iN`, `uN`, `ptr`, `int`, `uint`; pack names resolve in the parser
    fn from_name(s: &str) -> Option<Type> {
        match s {
            "ptr" => return Some(Type::Ptr),
            "int" => return Some(Type::AInt),
            "uint" => return Some(Type::AUInt),
            _ => {}
        }
        let (signed, rest) = match s.as_bytes().first()? {
            b'i' => (true, &s[1..]),
            b'u' => (false, &s[1..]),
            _ => return None,
        };
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) || rest.starts_with('0') {
            return None;
        }
        let bits: u32 = rest.parse().ok()?;
        if (1..=crate::wide::MAX_BITS).contains(&bits) {
            Some(Type::int(signed, bits))
        } else {
            None
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self, Type::Int { .. })
    }

    pub fn is_pack(self) -> bool {
        matches!(self, Type::Pack(_))
    }

    pub fn is_abstract(self) -> bool {
        matches!(self, Type::AInt | Type::AUInt)
    }

    /// width in bits, when it doesn't depend on the pack table
    pub fn int_bits(self) -> Option<u32> {
        match self {
            Type::Int { bits, .. } => Some(bits as u32),
            Type::Ptr => Some(64),
            _ => None,
        }
    }
}

/// A pack declaration: `pack name { f: ty, ... }`. Fields occupy
/// consecutive bits from bit 0 upward; the whole pack is at most 64 bits
/// and is carried around as the unsigned integer of that width.
#[derive(Clone, Debug, PartialEq)]
pub struct PackDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub offsets: Vec<u32>,
    pub width: u32,
    /// the generic type and arguments this pack was instantiated from, if
    /// any — what an opcode on the pack dispatches on (`add` on a
    /// `float(8, 23)` finds `add(E, M)`)
    pub origin: Option<(String, Vec<i64>)>,
}

impl PackDef {
    pub fn field(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|(n, _)| n == name)
    }
}

/// A width expression inside a type declaration: literals, the
/// declaration's parameters, and `+ - *`.
#[derive(Clone, Debug, PartialEq)]
pub enum IntExpr {
    Lit(i64),
    Param(String),
    Add(Box<IntExpr>, Box<IntExpr>),
    Sub(Box<IntExpr>, Box<IntExpr>),
    Mul(Box<IntExpr>, Box<IntExpr>),
    Shl(Box<IntExpr>, Box<IntExpr>),
    Shr(Box<IntExpr>, Box<IntExpr>),
    And(Box<IntExpr>, Box<IntExpr>),
    Or(Box<IntExpr>, Box<IntExpr>),
}

impl IntExpr {
    fn eval(&self, env: &[(String, i64)]) -> Result<i64, String> {
        Ok(match self {
            IntExpr::Lit(v) => *v,
            IntExpr::Param(p) => env
                .iter()
                .find(|(n, _)| n == p)
                .map(|(_, v)| *v)
                .ok_or_else(|| format!("unknown type parameter '{}'", p))?,
            IntExpr::Add(a, b) => a.eval(env)?.wrapping_add(b.eval(env)?),
            IntExpr::Sub(a, b) => a.eval(env)?.wrapping_sub(b.eval(env)?),
            IntExpr::Mul(a, b) => a.eval(env)?.wrapping_mul(b.eval(env)?),
            IntExpr::Shl(a, b) => a.eval(env)?.wrapping_shl(b.eval(env)? as u32),
            IntExpr::Shr(a, b) => a.eval(env)?.wrapping_shr(b.eval(env)? as u32),
            IntExpr::And(a, b) => a.eval(env)? & b.eval(env)?,
            IntExpr::Or(a, b) => a.eval(env)? | b.eval(env)?,
        })
    }

    /// the same, at 128 bits: for a `const` on a wide value
    fn eval128(&self, env: &[(String, i64)]) -> Result<i128, String> {
        Ok(match self {
            IntExpr::Lit(v) => *v as i128,
            IntExpr::Param(p) => env
                .iter()
                .find(|(n, _)| n == p)
                .map(|(_, v)| *v as i128)
                .ok_or_else(|| format!("unknown type parameter '{}'", p))?,
            IntExpr::Add(a, b) => a.eval128(env)?.wrapping_add(b.eval128(env)?),
            IntExpr::Sub(a, b) => a.eval128(env)?.wrapping_sub(b.eval128(env)?),
            IntExpr::Mul(a, b) => a.eval128(env)?.wrapping_mul(b.eval128(env)?),
            IntExpr::Shl(a, b) => a.eval128(env)?.wrapping_shl(b.eval128(env)? as u32),
            IntExpr::Shr(a, b) => a.eval128(env)?.wrapping_shr(b.eval128(env)? as u32),
            IntExpr::And(a, b) => a.eval128(env)? & b.eval128(env)?,
            IntExpr::Or(a, b) => a.eval128(env)? | b.eval128(env)?,
        })
    }

    fn prec(&self) -> u8 {
        match self {
            IntExpr::Lit(_) | IntExpr::Param(_) => 4,
            IntExpr::Mul(..) => 3,
            IntExpr::Add(..) | IntExpr::Sub(..) => 2,
            IntExpr::Shl(..) | IntExpr::Shr(..) => 1,
            IntExpr::And(..) | IntExpr::Or(..) => 0,
        }
    }
}

impl fmt::Display for IntExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let side = |e: &IntExpr, min: u8| {
            if e.prec() < min {
                format!("({})", e)
            } else {
                e.to_string()
            }
        };
        match self {
            IntExpr::Lit(v) => write!(f, "{}", v),
            IntExpr::Param(p) => write!(f, "{}", p),
            IntExpr::Add(a, b) => write!(f, "{} + {}", side(a, 2), side(b, 3)),
            IntExpr::Sub(a, b) => write!(f, "{} - {}", side(a, 2), side(b, 3)),
            IntExpr::Mul(a, b) => write!(f, "{} * {}", side(a, 3), side(b, 4)),
            IntExpr::Shl(a, b) => write!(f, "{} << {}", side(a, 1), side(b, 2)),
            IntExpr::Shr(a, b) => write!(f, "{} >> {}", side(a, 1), side(b, 2)),
            IntExpr::And(a, b) => write!(f, "{} & {}", side(a, 0), side(b, 1)),
            IntExpr::Or(a, b) => write!(f, "{} | {}", side(a, 0), side(b, 1)),
        }
    }
}

/// The right-hand side of a `type` declaration, before instantiation.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    /// `i(expr)` / `u(expr)`
    Int { signed: bool, bits: IntExpr },
    /// a builtin (`i5`, `ptr`), a declared type, or an instantiation `float(8, 23)`
    Named { name: String, args: Vec<IntExpr> },
    Pack(Vec<(String, TypeExpr)>),
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TypeExpr::Int { signed, bits } => {
                write!(f, "{}({})", if *signed { "i" } else { "u" }, bits)
            }
            TypeExpr::Named { name, args } if args.is_empty() => write!(f, "{}", name),
            TypeExpr::Named { name, args } => {
                let a: Vec<String> = args.iter().map(|e| e.to_string()).collect();
                write!(f, "{}({})", name, a.join(", "))
            }
            TypeExpr::Pack(fields) => {
                let fs: Vec<String> = fields.iter().map(|(n, t)| format!("{}: {}", n, t)).collect();
                write!(f, "pack {{ {} }}", fs.join(", "))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Literals: exact conversion of a decimal or integer to a float(E, M)

/// a natural number of any size, little-endian base 2^32 — enough bignum
/// to round a decimal literal correctly, and nothing more
#[derive(Clone, Debug)]
struct Big(Vec<u32>);

impl Big {
    fn from_u64(v: u64) -> Big {
        let mut b = Big(vec![v as u32, (v >> 32) as u32]);
        b.trim();
        b
    }
    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }
    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
    fn mul_small(&mut self, m: u32) {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let v = *limb as u64 * m as u64 + carry;
            *limb = v as u32;
            carry = v >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
    }
    fn add_small(&mut self, a: u32) {
        let mut carry = a as u64;
        for limb in &mut self.0 {
            if carry == 0 {
                break;
            }
            let v = *limb as u64 + carry;
            *limb = v as u32;
            carry = v >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
    }
    fn bits(&self) -> u64 {
        match self.0.last() {
            None => 0,
            Some(&top) => (self.0.len() as u64 - 1) * 32 + (32 - top.leading_zeros() as u64),
        }
    }
    fn bit(&self, i: u64) -> bool {
        self.0.get((i / 32) as usize).is_some_and(|l| l >> (i % 32) & 1 == 1)
    }
    fn shl1_or(&mut self, bit: bool) {
        let mut carry = bit as u32;
        for limb in &mut self.0 {
            let next = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if carry > 0 {
            self.0.push(carry);
        }
    }
    fn cmp(&self, other: &Big) -> std::cmp::Ordering {
        self.0.len().cmp(&other.0.len()).then_with(|| {
            for i in (0..self.0.len()).rev() {
                match self.0[i].cmp(&other.0[i]) {
                    std::cmp::Ordering::Equal => continue,
                    o => return o,
                }
            }
            std::cmp::Ordering::Equal
        })
    }
    fn sub_assign(&mut self, other: &Big) {
        let mut borrow = 0i64;
        for i in 0..self.0.len() {
            let v = self.0[i] as i64 - *other.0.get(i).unwrap_or(&0) as i64 - borrow;
            if v < 0 {
                self.0[i] = (v + (1 << 32)) as u32;
                borrow = 1;
            } else {
                self.0[i] = v as u32;
                borrow = 0;
            }
        }
        self.trim();
    }
}

/// a decimal literal as (negative, digits, power of ten): `-1.5e-3` is
/// (true, 15, -4)
fn parse_decimal(text: &str) -> Result<(bool, Big, i64), String> {
    let (neg, t) = match text.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, text),
    };
    let (mant, exp) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], t[i + 1..].parse::<i64>().map_err(|_| format!("bad exponent in '{}'", text))?),
        None => (t, 0),
    };
    let (int, frac) = match mant.split_once('.') {
        Some((a, b)) => (a, b),
        None => (mant, ""),
    };
    if (int.is_empty() && frac.is_empty()) || !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("bad float literal '{}'", text));
    }
    let mut digits = Big(Vec::new());
    for b in int.bytes().chain(frac.bytes()) {
        digits.mul_small(10);
        digits.add_small((b - b'0') as u32);
    }
    Ok((neg, digits, exp - frac.len() as i64))
}

/// the bits of the float(E, M) nearest (ties to even) to sign * mant * 2^exp,
/// where `sticky` says something nonzero below mant's unit was dropped
pub fn float_bits(e: u32, m: u32, sign: bool, mut mant: u128, exp: i64, mut sticky: bool) -> u64 {
    let emax = (1u64 << e) - 1;
    let bias = (1i64 << (e - 1)) - 1;
    let s = sign as u64;
    if mant == 0 {
        return s << (e + m);
    }
    let top = 127 - mant.leading_zeros() as i64;
    let mut shift = top - m as i64;
    let mut x = exp + shift + m as i64 + bias; // biased exponent of the unit at bit M
    if x < 1 {
        shift += 1 - x;
        x = 1;
    }
    let mut round = false;
    if shift > 0 {
        if shift >= 128 {
            sticky |= mant != 0;
            mant = 0;
        } else {
            let dropped = mant & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            round = dropped & half != 0;
            sticky |= dropped & (half - 1) != 0;
            mant >>= shift;
        }
    } else if shift < 0 {
        mant <<= -shift;
    }
    if round && (sticky || mant & 1 == 1) {
        mant += 1;
    }
    if mant >> (m + 1) != 0 {
        mant >>= 1;
        x += 1;
    }
    let (fexp, fmant) = if mant >> m != 0 {
        (x as u64, (mant as u64) & ((1u64 << m) - 1))
    } else {
        (0, mant as u64)
    };
    if fexp >= emax {
        return (s << (e + m)) | (emax << m);
    }
    (s << (e + m)) | (fexp << m) | fmant
}

/// a decimal literal, correctly rounded to float(E, M)
pub fn decimal_to_float(text: &str, e: u32, m: u32) -> Result<u64, String> {
    let (neg, digits, pow10) = parse_decimal(text)?;
    if digits.is_zero() {
        return Ok((neg as u64) << (e + m));
    }
    // value = num / den, one of them a power of ten
    let mut num = digits;
    let mut den = Big::from_u64(1);
    for _ in 0..pow10.max(0) {
        num.mul_small(10);
    }
    for _ in 0..(-pow10).max(0) {
        den.mul_small(10);
    }
    // long division to about M + 6 quotient bits: the numerator is scaled
    // up (more bits) or down (low bits dropped into the sticky) so the
    // quotient lands in that range; the remainder is the sticky
    let want = m as i64 + 6;
    let scale = want + den.bits() as i64 - num.bits() as i64 + 1; // may be negative
    let nbits = num.bits() as i64 + scale; // quotient bits to produce
    let mut rem = Big(Vec::new());
    let mut q: u128 = 0;
    let mut sticky = false;
    for i in (0..nbits).rev() {
        let src = i - scale; // the numerator bit feeding this step
        let bit = src >= 0 && num.bit(src as u64);
        rem.shl1_or(bit);
        q <<= 1;
        if rem.cmp(&den) != std::cmp::Ordering::Less {
            rem.sub_assign(&den);
            q |= 1;
        }
    }
    // numerator bits below the ones fed in
    for i in 0..(-scale).max(0) {
        sticky |= num.bit(i as u64);
    }
    Ok(float_bits(e, m, neg, q, -scale, sticky || !rem.is_zero()))
}

/// an integer literal as a float(E, M), rounded to nearest even
pub fn int_to_float(v: i64, e: u32, m: u32) -> u64 {
    float_bits(e, m, v < 0, v.unsigned_abs() as u128, 0, false)
}

/// a literal as written
#[derive(Clone, Debug, PartialEq)]
enum Lit {
    Int(i64),
    Dec(String),
    Inf(bool),
    NaN,
}

/// `type name(params) = expr`
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: TypeExpr,
}

impl fmt::Display for TypeDef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.params.is_empty() {
            write!(f, "type {} = {}", self.name, self.body)
        } else {
            write!(f, "type {}({}) = {}", self.name, self.params.join(", "), self.body)
        }
    }
}

/// Bit width of a type given the pack table (abstract types have none).
pub fn type_width(ty: Type, packs: &[PackDef]) -> Option<u32> {
    match ty {
        Type::Pack(i) => packs.get(i as usize).map(|p| p.width),
        t => t.int_bits(),
    }
}

/// How a value of a type is held in a register or local: as an N-bit
/// quantity that is sign-extended (`S`) or zero-extended (`U`) to the
/// container it lives in. Pointers and packs are unsigned bit patterns.
/// Every emitter keeps every value in this canonical form, which is what
/// lets compares, divides, and calls stay one instruction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repr {
    S(u32),
    U(u32),
}

impl Repr {
    pub fn bits(self) -> u32 {
        match self {
            Repr::S(n) | Repr::U(n) => n,
        }
    }
    pub fn signed(self) -> bool {
        matches!(self, Repr::S(_))
    }
    /// 32 for types up to 32 bits, else 64 — the register/local width a
    /// backend with two integer sizes uses
    pub fn container(self) -> u32 {
        if self.bits() <= 32 {
            32
        } else {
            64
        }
    }
    /// is a canonical value of `self` already a canonical value of `to`,
    /// with no re-normalization? (widening within one signedness; an
    /// unsigned value widening into a strictly larger signed type)
    pub fn fits_in(self, to: Repr) -> bool {
        match (self, to) {
            (Repr::S(a), Repr::S(b)) | (Repr::U(a), Repr::U(b)) => a <= b,
            (Repr::U(a), Repr::S(b)) => a < b,
            (Repr::S(_), Repr::U(_)) => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    IAdd,
    ISub,
    IMul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

const BINOPS: &[(&str, BinOp)] = &[
    ("add", BinOp::IAdd),
    ("sub", BinOp::ISub),
    ("mul", BinOp::IMul),
    ("div", BinOp::Div),
    ("rem", BinOp::Rem),
    ("and", BinOp::And),
    ("or", BinOp::Or),
    ("xor", BinOp::Xor),
    ("shl", BinOp::Shl),
    ("shr", BinOp::Shr),
];

impl BinOp {
    pub fn name(self) -> &'static str {
        BINOPS.iter().find(|(_, op)| *op == self).unwrap().0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cond {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

const CONDS: &[(&str, Cond)] = &[
    ("eq", Cond::Eq),
    ("ne", Cond::Ne),
    ("lt", Cond::Lt),
    ("le", Cond::Le),
    ("gt", Cond::Gt),
    ("ge", Cond::Ge),
];

impl Cond {
    pub fn name(self) -> &'static str {
        CONDS.iter().find(|(_, c)| *c == self).unwrap().0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CastOp {
    /// the value crosses over: widen by the source's signedness, narrow to
    /// the low bits, re-read at the same width (between packs, or a pack
    /// and an integer, a library generic named `conv` does it)
    Conv,
    /// the bits stay, the reading changes: same width, any types
    Cast,
}

impl CastOp {
    pub fn name(self) -> &'static str {
        match self {
            CastOp::Conv => "conv",
            CastOp::Cast => "cast",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Inst {
    IConst {
        dst: ValueId,
        /// 128 bits, so a wide value's constants (`const 1 << 112`) fit
        imm: i128,
    },
    Bin {
        op: BinOp,
        dst: ValueId,
        lhs: ValueId,
        rhs: ValueId,
    },
    ICmp {
        cond: Cond,
        dst: ValueId,
        lhs: ValueId,
        rhs: ValueId,
    },
    Cast {
        op: CastOp,
        dst: ValueId,
        src: ValueId,
    },
    /// build a pack from one value per field, in declaration order
    Pack {
        dst: ValueId,
        args: Vec<ValueId>,
    },
    /// split a pack into one value per field, in declaration order
    Unpack {
        dsts: Vec<ValueId>,
        src: ValueId,
    },
    /// read one field of a pack
    Get {
        dst: ValueId,
        src: ValueId,
        field: u32,
    },
    /// a copy of a pack with one field replaced
    Set {
        dst: ValueId,
        src: ValueId,
        field: u32,
        val: ValueId,
    },
    Load {
        dst: ValueId,
        addr: ValueId,
    },
    Store {
        val: ValueId,
        addr: ValueId,
    },
    PtrAdd {
        dst: ValueId,
        base: ValueId,
        off: ValueId,
    },
    Call {
        dsts: Vec<ValueId>,
        callee: String,
        args: Vec<ValueId>,
    },
    Jmp {
        target: BlockId,
        args: Vec<ValueId>,
    },
    Br {
        cond: ValueId,
        then_target: BlockId,
        then_args: Vec<ValueId>,
        else_target: BlockId,
        else_args: Vec<ValueId>,
    },
    Ret {
        vals: Vec<ValueId>,
    },
}

impl Inst {
    pub fn is_terminator(&self) -> bool {
        matches!(self, Inst::Jmp { .. } | Inst::Br { .. } | Inst::Ret { .. })
    }
}

#[derive(Clone, Debug)]
pub struct ValueData {
    pub name: String,
    pub ty: Type,
    /// a literal written inline as an operand: defined by hidden
    /// instructions and printed back as the literal — the type it was
    /// read in (its own, or i64 / f64 when it was converted into a
    /// library type) and the bits in that type
    pub literal: Option<(Type, i64)>,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,
    pub params: Vec<ValueId>,
    pub insts: Vec<Inst>,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<ValueId>,
    pub rets: Vec<Type>,
    pub values: Vec<ValueData>,
    pub blocks: Vec<Block>,
    /// the module's pack table, shared so a function can be compiled alone
    pub packs: std::sync::Arc<Vec<PackDef>>,
    /// for an instantiated generic: (generic name, width arguments) — what
    /// a platform matches on to substitute a native instruction
    pub instance: Option<(String, Vec<i64>)>,
    /// the generic's parameter names, one per width argument (`E`, `M`,
    /// `round`), so a platform can tell which arguments came from types
    pub instance_names: Vec<String>,
    /// for a function that took or returned values wider than a word:
    /// the (parameter types, result types) as written, before they were
    /// lowered to words (see wide.rs) — what a caller from outside sees
    pub wide_sig: Option<(Vec<Type>, Vec<Type>)>,
}

impl Function {
    pub fn value(&self, id: ValueId) -> &ValueData {
        &self.values[id.0 as usize]
    }

    pub fn ty(&self, id: ValueId) -> Type {
        self.value(id).ty
    }

    pub fn pack(&self, ty: Type) -> Option<&PackDef> {
        match ty {
            Type::Pack(i) => self.packs.get(i as usize),
            _ => None,
        }
    }

    /// the type's spelling with pack names resolved
    pub fn tyname(&self, ty: Type) -> String {
        match self.pack(ty) {
            Some(p) => p.name.clone(),
            None => ty.name(),
        }
    }

    pub fn width(&self, ty: Type) -> Option<u32> {
        type_width(ty, &self.packs)
    }

    /// canonical register form of a (concrete) type
    pub fn repr(&self, ty: Type) -> Repr {
        match ty {
            Type::Int { signed: true, bits } => Repr::S(bits as u32),
            Type::Int { signed: false, bits } => Repr::U(bits as u32),
            Type::Ptr => Repr::U(64),
            Type::Pack(_) => Repr::U(self.width(ty).unwrap_or(64)),
            Type::AInt | Type::AUInt => unreachable!("abstract types are resolved before use"),
        }
    }

    /// (offset, type) of field `i` of a pack-typed value
    pub fn field(&self, ty: Type, i: u32) -> Option<(u32, Type)> {
        let p = self.pack(ty)?;
        Some((p.offsets[i as usize], p.fields[i as usize].1))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Module {
    /// declarations as written (every instantiated pack lives in each
    /// function's shared `packs` table)
    pub types: Vec<TypeDef>,
    pub funcs: Vec<Function>,
}

impl Module {
    pub fn func(&self, name: &str) -> Option<&Function> {
        self.funcs.iter().find(|f| f.name == name)
    }
}

/// The replacement policy for abstract numeric types: what `int` (and its
/// unsigned twin `uint`) and `float` become on this compilation. Targets
/// supply defaults (their natural width, or a size-oriented choice); the
/// user can override. `int` is a builtin the verifier's pre-pass rewrites;
/// `float` is the library's `float(E, M)`, so the parser instantiates a
/// bare `float` with the policy's (E, M) as it meets it.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub int: Type,
    /// (E, M) for a bare `float`
    pub float: (u32, u32),
    /// (I, F) for a bare `fixed`
    pub fixed: (u32, u32),
    /// N for a bare `unit` and `sunit`
    pub unit: u32,
    pub sunit: u32,
    /// (N, D) for a bare `rational`
    pub rational: (u32, u32),
    /// the family a bare `scalar` means: float, fixed, rational, unit, sunit
    pub scalar: &'static str,
    /// the value of a generic parameter named `round` that nothing binds:
    /// 0 nearest even, 1 toward zero, 2 down, 3 up, 4 nearest away
    pub round: i64,
}

pub const ROUNDS: [&str; 5] = ["even", "zero", "down", "up", "away"];

impl Policy {
    pub fn new(int: Type) -> Result<Policy, String> {
        match int {
            // the float of the same class as the integer: f32 with i32, f64 with i64
            Type::I32 => Ok(Policy { int, float: (8, 23), fixed: (16, 16), unit: 16, sunit: 16, rational: (16, 16), scalar: "float", round: 0 }),
            Type::I64 => Ok(Policy { int, float: (11, 52), fixed: (32, 32), unit: 32, sunit: 32, rational: (32, 32), scalar: "float", round: 0 }),
            t => Err(format!("'int' cannot resolve to {}", t.name())),
        }
    }

    /// `--round=even|zero|down|up|away` (or the number)
    pub fn with_round(mut self, arg: &str) -> Result<Policy, String> {
        self.round = match ROUNDS.iter().position(|r| *r == arg) {
            Some(i) => i as i64,
            None => arg.parse::<i64>().ok().filter(|v| (0..5).contains(v)).ok_or_else(|| format!("--round= wants one of {}", ROUNDS.join("|")))?,
        };
        Ok(self)
    }

    /// a generic parameter the policy supplies when nothing binds it —
    /// unless the generic being instantiated has one of the same name,
    /// which is then passed on (`sub`'s `add a, nb` rounds as `sub` does)
    pub fn named(&self, param: &str) -> Option<i64> {
        match param {
            "round" => Some(self.round),
            _ => None,
        }
    }

    pub fn with_float(mut self, e: u32, m: u32) -> Policy {
        self.float = (e, m);
        self
    }

    pub fn with_fixed(mut self, i: u32, f: u32) -> Policy {
        self.fixed = (i, f);
        self
    }

    pub fn with_unit(mut self, n: u32) -> Policy {
        self.unit = n;
        self
    }

    pub fn with_sunit(mut self, n: u32) -> Policy {
        self.sunit = n;
        self
    }

    pub fn with_rational(mut self, n: u32, d: u32) -> Policy {
        self.rational = (n, d);
        self
    }

    pub const SCALARS: [&'static str; 5] = ["float", "fixed", "rational", "unit", "sunit"];

    pub fn with_scalar(mut self, family: &str) -> Option<Policy> {
        let f = Policy::SCALARS.iter().find(|f| **f == family)?;
        self.scalar = f;
        Some(self)
    }

    /// a `--fixed=` argument: `I,F`
    pub fn fixed_from_arg(s: &str) -> Option<(u32, u32)> {
        let (i, f) = s.split_once(',')?;
        Some((i.trim().parse().ok()?, f.trim().parse().ok()?))
    }

    /// a `--float=` argument: f16, bf16, f32, f64, or `E,M`
    pub fn float_from_arg(s: &str) -> Option<(u32, u32)> {
        match s {
            "f16" => Some((5, 10)),
            "bf16" => Some((8, 7)),
            "f32" => Some((8, 23)),
            "f64" => Some((11, 52)),
            _ => {
                let (e, m) = s.split_once(',')?;
                Some((e.trim().parse().ok()?, m.trim().parse().ok()?))
            }
        }
    }

    /// the arguments a bare parametric type name takes under this policy
    fn default_args(&self, name: &str) -> Option<Vec<i64>> {
        match name {
            "float" => Some(vec![self.float.0 as i64, self.float.1 as i64]),
            "fixed" => Some(vec![self.fixed.0 as i64, self.fixed.1 as i64]),
            "unit" => Some(vec![self.unit as i64]),
            "sunit" => Some(vec![self.sunit as i64]),
            "rational" => Some(vec![self.rational.0 as i64, self.rational.1 as i64]),
            _ => None,
        }
    }

    fn resolve(&self, ty: Type) -> Type {
        match ty {
            Type::AInt => self.int,
            Type::AUInt => Type::int(false, self.int.int_bits().unwrap()),
            t => t,
        }
    }
}

/// Resolve abstract types to concrete ones. Because types live on values,
/// not opcodes, this is one sweep over the value tables and signatures —
/// no instruction ever changes.
pub fn resolve_types(module: &mut Module, policy: &Policy) {
    for func in &mut module.funcs {
        for v in &mut func.values {
            v.ty = policy.resolve(v.ty);
        }
        for r in &mut func.rets {
            *r = policy.resolve(*r);
        }
    }
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Newline,
    Ident(String), // every word: keywords, opcodes, values, blocks, functions, types
    Int(i64),
    Float(String), // 1.5, 2e10, -1.0e-3: kept as text, converted by the type it lands in
    Colon,
    Comma,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Arrow,
    Equals,
    Plus,
    Minus,
    Star,
    ShiftL,
    ShiftR,
    Amp,
    Pipe,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tok::Newline => write!(f, "end of line"),
            Tok::Ident(s) => write!(f, "'{}'", s),
            Tok::Int(n) => write!(f, "'{}'", n),
            Tok::Float(s) => write!(f, "'{}'", s),
            Tok::Colon => write!(f, "':'"),
            Tok::Comma => write!(f, "','"),
            Tok::LParen => write!(f, "'('"),
            Tok::RParen => write!(f, "')'"),
            Tok::LBrace => write!(f, "'{{'"),
            Tok::RBrace => write!(f, "'}}'"),
            Tok::Arrow => write!(f, "'->'"),
            Tok::Equals => write!(f, "'='"),
            Tok::Plus => write!(f, "'+'"),
            Tok::Minus => write!(f, "'-'"),
            Tok::Star => write!(f, "'*'"),
            Tok::ShiftL => write!(f, "'<<'"),
            Tok::ShiftR => write!(f, "'>>'"),
            Tok::Amp => write!(f, "'&'"),
            Tok::Pipe => write!(f, "'|'"),
        }
    }
}

#[derive(Debug)]
pub struct ParseError {
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

impl std::error::Error for ParseError {}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let mut toks = Vec::new();
    let mut line = 1;
    let mut chars = src.chars().peekable();

    let err = |line: usize, msg: String| ParseError { line, msg };

    while let Some(&c) = chars.peek() {
        match c {
            '\n' => {
                chars.next();
                // collapse runs of newlines; skip a leading newline entirely
                if !matches!(toks.last(), Some((Tok::Newline, _)) | None) {
                    toks.push((Tok::Newline, line));
                }
                line += 1;
            }
            ' ' | '\t' | '\r' => {
                chars.next();
            }
            ';' => {
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '%' | '^' | '@' | '$' => {
                return Err(err(
                    line,
                    format!("'{}' prefixes are gone: values, blocks, functions, and types are plain names", c),
                ));
            }
            ':' => {
                chars.next();
                toks.push((Tok::Colon, line));
            }
            ',' => {
                chars.next();
                toks.push((Tok::Comma, line));
            }
            '(' => {
                chars.next();
                toks.push((Tok::LParen, line));
            }
            ')' => {
                chars.next();
                toks.push((Tok::RParen, line));
            }
            '{' => {
                chars.next();
                toks.push((Tok::LBrace, line));
            }
            '}' => {
                chars.next();
                toks.push((Tok::RBrace, line));
            }
            '=' => {
                chars.next();
                toks.push((Tok::Equals, line));
            }
            '-' => {
                chars.next();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    toks.push((Tok::Arrow, line));
                } else if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    match lex_number(&mut chars).map_err(|m| err(line, m))? {
                        Tok::Int(n) => toks.push((Tok::Int(n.wrapping_neg()), line)),
                        Tok::Float(s) => toks.push((Tok::Float(format!("-{}", s)), line)),
                        _ => unreachable!(),
                    }
                } else {
                    toks.push((Tok::Minus, line));
                }
            }
            '+' => {
                chars.next();
                toks.push((Tok::Plus, line));
            }
            '*' => {
                chars.next();
                toks.push((Tok::Star, line));
            }
            '&' => {
                chars.next();
                toks.push((Tok::Amp, line));
            }
            '|' => {
                chars.next();
                toks.push((Tok::Pipe, line));
            }
            '<' | '>' => {
                chars.next();
                if chars.peek() != Some(&c) {
                    return Err(err(line, format!("expected '{}{}'", c, c)));
                }
                chars.next();
                toks.push((if c == '<' { Tok::ShiftL } else { Tok::ShiftR }, line));
            }
            '0'..='9' => {
                let t = lex_number(&mut chars).map_err(|m| err(line, m))?;
                toks.push((t, line));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = lex_name(&mut chars);
                // opcode suffixes like cmp.slt lex as one identifier
                while chars.peek() == Some(&'.') {
                    chars.next();
                    s.push('.');
                    s.push_str(&lex_name(&mut chars));
                }
                toks.push((Tok::Ident(s), line));
            }
            _ => return Err(err(line, format!("unexpected character '{}'", c))),
        }
    }
    if !matches!(toks.last(), Some((Tok::Newline, _)) | None) {
        toks.push((Tok::Newline, line));
    }
    Ok(toks)
}

fn lex_name(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c);
            chars.next();
        } else {
            break;
        }
    }
    s
}

/// an integer (decimal or 0x hex) or a decimal float: digits with a
/// fraction (`1.5`), an exponent (`2e10`, `1e-3`), or both
fn lex_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<Tok, String> {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if s.starts_with("0x") || s.starts_with("0X") {
        return lex_int_text(&s).map(Tok::Int);
    }
    let mut is_float = false;
    if chars.peek() == Some(&'.') {
        let mut look = chars.clone();
        look.next();
        if look.peek().is_some_and(|c| c.is_ascii_digit()) {
            chars.next();
            s.push('.');
            while let Some(&c) = chars.peek() {
                if c.is_ascii_alphanumeric() {
                    s.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            is_float = true;
        }
    }
    // an exponent's sign: `1e-3` stops the word at the '-'
    if (s.ends_with('e') || s.ends_with('E')) && matches!(chars.peek(), Some('-') | Some('+')) {
        s.push(chars.next().unwrap());
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                chars.next();
            } else {
                break;
            }
        }
    }
    if is_float || s.contains('e') || s.contains('E') {
        parse_decimal(&s)?; // validate now; the value depends on the type
        return Ok(Tok::Float(s));
    }
    lex_int_text(&s).map(Tok::Int)
}

fn lex_int_text(s: &str) -> Result<i64, String> {
    // parse as u64 so full-width bit patterns (and i64::MIN's magnitude
    // before negation) are representable; const semantics are bit-level
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse::<u64>()
    };
    v.map(|v| v as i64)
        .map_err(|_| format!("bad integer literal '{}'", s))
}

// ---------------------------------------------------------------------------
// Parser

/// the prelude: every `lib/*.ssa`, in name order, appended to a program
/// so its types and generics are always available (and appended, not
/// prepended, so the program's own line numbers hold)
pub fn with_prelude(src: &str) -> String {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir("lib")
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "ssa")).collect())
        .unwrap_or_default();
    files.sort();
    let mut out = String::from(src);
    for f in files {
        if let Ok(t) = std::fs::read_to_string(&f) {
            out.push('\n');
            out.push_str(&t);
        }
    }
    out
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn parse(src: &str) -> Result<Module, ParseError> {
    parse_with(src, &Policy::new(Type::I64).unwrap())
}

/// parse under a policy: a bare `float` is `float(E, M)` for the policy's
/// (E, M) (the policy's `int` is applied afterwards by `resolve_types`)
pub fn parse_with(src: &str, policy: &Policy) -> Result<Module, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        policy: *policy,
        types: Vec::new(),
        packs: Vec::new(),
        generics: Vec::new(),
        instances: HashMap::new(),
        pending: Vec::new(),
        env: Vec::new(),
        consts: Vec::new(),
        cur_rets: Vec::new(),
        sigs: HashMap::new(),
    };
    // pass 0: type declarations, wherever they appear (a declaration may
    // only refer to types declared before it), so that everything after
    // — signatures, generics, bodies — can name any type regardless of
    // order; the prelude is appended, so this matters
    let mut funcs = Vec::new();
    let mut aliases: Vec<usize> = Vec::new(); // token positions of `fn x = g(..)`
    let mut decls: Vec<usize> = Vec::new(); // token positions of `type ...`
    p.skip_newlines();
    while !p.at_end() {
        match p.item_kind() {
            Item::Type => {
                decls.push(p.pos);
                p.skip_line();
            }
            Item::Alias => p.skip_line(),
            _ => {
                let (_, hi) = p.function_range()?;
                p.pos = hi + 1;
            }
        }
        p.skip_newlines();
    }
    // declarations may name types declared later (a file's `type q8 =
    // fixed(8, 8)` precedes the appended prelude's `fixed`): keep trying
    // the ones that failed on an unknown type while any succeeds
    let mut remaining = decls;
    loop {
        let mut failed: Vec<(usize, ParseError)> = Vec::new();
        for &at in &remaining {
            p.pos = at;
            if let Err(e) = p.parse_type_decl() {
                if e.msg.starts_with("unknown type") || e.msg.starts_with("unknown pack type") {
                    failed.push((at, e));
                } else {
                    return Err(e);
                }
            }
        }
        if failed.is_empty() {
            break;
        }
        if failed.len() == remaining.len() {
            return Err(failed.remove(0).1);
        }
        remaining = failed.into_iter().map(|(at, _)| at).collect();
    }
    // pass 1: generic functions and plain signatures
    p.pos = 0;
    p.skip_newlines();
    while !p.at_end() {
        match p.item_kind() {
            Item::Type => p.skip_line(),
            Item::Generic => p.record_generic()?,
            Item::Alias => {
                aliases.push(p.pos);
                p.skip_line();
            }
            Item::Fn => {
                let (_, hi) = p.function_range()?;
                p.record_signature()?;
                p.pos = hi + 1;
            }
        }
        p.skip_newlines();
    }
    // named instantiations first, so call sites reuse them
    for at in aliases {
        p.pos = at;
        p.parse_alias()?;
    }
    // pass 2: functions
    p.pos = 0;
    p.skip_newlines();
    while !p.at_end() {
        match p.item_kind() {
            Item::Type | Item::Alias => p.skip_line(),
            Item::Generic => {
                let (_, hi) = p.function_range()?;
                p.pos = hi + 1;
            }
            Item::Fn => funcs.push(p.parse_function(None)?),
        }
        p.skip_newlines();
    }
    // pass 3: instantiate what was asked for, including what those
    // instantiations ask for in turn
    while let Some((g, args, name)) = p.pending.pop() {
        let (lo, params) = (p.generics[g].lo, p.generics[g].params.clone());
        let generic = p.generics[g].name.clone();
        p.env = params.into_iter().zip(args.iter().copied()).collect();
        p.pos = lo;
        let mut f = p.parse_function(Some(name))?;
        f.instance_names = p.env.iter().map(|(n, _)| n.clone()).collect();
        p.env.clear();
        f.instance = Some((generic, args));
        funcs.push(f);
    }
    let packs = std::sync::Arc::new(p.packs.clone());
    for f in &mut funcs {
        f.packs = packs.clone();
    }
    let mut module = Module {
        types: p.types,
        funcs,
    };
    // values wider than a word: checked as written, then lowered to words
    // so that no backend ever sees one
    if crate::wide::has_wide(&module) {
        if let Err(errs) = verify(&module) {
            return Err(ParseError { line: 0, msg: errs.join("; ") });
        }
        crate::wide::lower(&mut module).map_err(|m| ParseError { line: 0, msg: m })?;
    }
    Ok(module)
}

/// a parametric function: its token range, re-parsed per instantiation
struct GenericFn {
    name: String,
    params: Vec<String>,
    lo: usize,
    /// the declared types of its value parameters and of its first result,
    /// for opcode dispatch (several generics may share a name and differ
    /// here: `conv` from i(W) and from u(W)) and for typing literal
    /// arguments at call sites
    param_types: Vec<TypeExpr>,
    first_param: Option<TypeExpr>,
    ret: Option<TypeExpr>,
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    policy: Policy,
    types: Vec<TypeDef>,
    packs: Vec<PackDef>,
    generics: Vec<GenericFn>,
    /// (generic index, args) -> the instance's function name
    instances: HashMap<(usize, Vec<i64>), String>,
    /// instances requested but not yet parsed: (generic index, args, name)
    pending: Vec<(usize, Vec<i64>, String)>,
    /// the parameter bindings of the body being parsed (empty outside generics)
    env: Vec<(String, i64)>,
    /// hidden `const`s for literal operands of the instruction being parsed,
    /// emitted just before it
    consts: Vec<Inst>,
    /// the return types of the function being parsed, to type `ret 0`
    cur_rets: Vec<Type>,
    /// parameter types of plain functions (from pass 1) and of instances
    /// (as they are requested), to type literal call arguments
    sigs: HashMap<String, Vec<Type>>,
}

/// nesting guard for type declarations that instantiate themselves
const MAX_TYPE_DEPTH: usize = 32;

enum Item {
    Type,
    Generic,
    Alias,
    Fn,
}

/// Placeholder branch target inside structured constructs; every one is
/// patched to the real join/exit block before parsing of the construct ends.
const DUMMY_BLOCK: BlockId = BlockId(u32::MAX);

struct LoopFrame {
    header: BlockId,
    breaks: Vec<(usize, usize)>, // (block, inst) of Jmps to patch to the exit
    rets: Vec<Type>,             // what `break` yields
}

/// Block-graph builder for structured bodies.
struct StructEmit {
    blocks: Vec<Block>,
    cur: usize,
    loop_stack: Vec<LoopFrame>,
    /// One frame per enclosing value-yielding position: edges waiting to be
    /// patched to an if's join block, and the types it yields
    yield_stack: Vec<(Vec<(usize, usize)>, Vec<Type>)>,
}

impl StructEmit {
    fn new_block(&mut self, params: Vec<ValueId>) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        let name = if self.blocks.is_empty() {
            "entry".to_string()
        } else {
            format!("b{}", self.blocks.len())
        };
        self.blocks.push(Block {
            name,
            params,
            insts: Vec::new(),
        });
        id
    }

    fn push(&mut self, inst: Inst) -> (usize, usize) {
        let b = self.cur;
        self.blocks[b].insts.push(inst);
        (b, self.blocks[b].insts.len() - 1)
    }

    fn patch_target(&mut self, at: (usize, usize), target: BlockId) {
        match &mut self.blocks[at.0].insts[at.1] {
            Inst::Jmp { target: t, .. } => *t = target,
            Inst::Br { else_target, .. } => *else_target = target,
            _ => unreachable!("only branches get patched"),
        }
    }
}

/// Per-function parsing state: the value/block name tables.
struct FuncScope {
    values: Vec<ValueData>,
    value_ids: HashMap<String, ValueId>,
    block_ids: HashMap<String, BlockId>,
    block_names: Vec<String>,
    /// each block's parameters, from the prescan, so a branch to a later
    /// block can type its literal arguments
    block_params: Vec<Vec<ValueId>>,
}

impl FuncScope {
    /// a hidden value for a literal operand, recording the literal as
    /// (type it was read in, bits)
    fn synth(&mut self, ty: Type, lit: (Type, i64)) -> ValueId {
        let id = ValueId(self.values.len() as u32);
        self.values.push(ValueData {
            name: format!("#{}", self.values.len()),
            ty,
            literal: Some(lit),
        });
        id
    }
}

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn line(&self) -> usize {
        self.toks
            .get(self.pos.min(self.toks.len().saturating_sub(1)))
            .map_or(0, |(_, l)| *l)
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line(),
            msg: msg.into(),
        }
    }

    fn next(&mut self) -> Result<Tok, ParseError> {
        let t = self
            .toks
            .get(self.pos)
            .map(|(t, _)| t.clone())
            .ok_or_else(|| self.err("unexpected end of input"))?;
        self.pos += 1;
        Ok(t)
    }

    fn expect(&mut self, want: Tok) -> Result<(), ParseError> {
        let got = self.next()?;
        if got == want {
            Ok(())
        } else {
            self.pos -= 1;
            Err(self.err(format!("expected {}, found {}", want, got)))
        }
    }

    fn eat(&mut self, want: &Tok) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_newlines(&mut self) {
        while self.eat(&Tok::Newline) {}
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.next()? {
            Tok::Ident(s) => Ok(s),
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected an identifier, found {}", t)))
            }
        }
    }

    fn expect_type(&mut self) -> Result<Type, ParseError> {
        let env = self.env.clone();
        let (ty, next) = self.type_at(self.pos, &env, 0)?;
        self.pos = next;
        Ok(ty)
    }

    /// what starts at the current position: `type ...`, `fn g(P, Q)(...)`,
    /// `fn x = g(...)`, or a plain `fn`
    fn item_kind(&self) -> Item {
        let at = |k: usize| self.toks.get(self.pos + k).map(|t| &t.0);
        if matches!(at(0), Some(Tok::Ident(k)) if k == "type") {
            return Item::Type;
        }
        if at(2) == Some(&Tok::Equals) {
            return Item::Alias;
        }
        // fn name ( idents , ... ) (   -- a parameter list followed by another
        if at(2) == Some(&Tok::LParen) {
            let mut j = self.pos + 3;
            loop {
                match at(j - self.pos) {
                    Some(Tok::Ident(_)) | Some(Tok::Comma) => j += 1,
                    Some(Tok::RParen) => {
                        return if at(j + 1 - self.pos) == Some(&Tok::LParen) {
                            Item::Generic
                        } else {
                            Item::Fn
                        };
                    }
                    _ => return Item::Fn,
                }
            }
        }
        Item::Fn
    }

    /// `fn name(P, Q)(...) { ... }`: remember the range for later
    fn record_generic(&mut self) -> Result<(), ParseError> {
        let (lo, hi) = self.function_range()?;
        self.expect_ident()?; // fn
        let name = self.expect_ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        loop {
            params.push(self.expect_ident()?);
            if self.eat(&Tok::RParen) {
                break;
            }
            self.expect(Tok::Comma)?;
        }
        // the value parameters' types: `(a: float(E, M), b: u(W))`
        self.expect(Tok::LParen)?;
        let mut param_types = Vec::new();
        let mut j = self.pos;
        loop {
            match (self.toks.get(j).map(|t| &t.0), self.toks.get(j + 1).map(|t| &t.0)) {
                (Some(Tok::Ident(_)), Some(Tok::Colon)) => {
                    let (te, next) = self.type_expr_at(j + 2, &params)?;
                    param_types.push(te);
                    j = next;
                    if self.toks.get(j).map(|t| &t.0) == Some(&Tok::Comma) {
                        j += 1;
                    }
                }
                (Some(Tok::RParen), _) => {
                    j += 1;
                    break;
                }
                _ => return Err(self.err("bad parameter list".to_string())),
            }
        }
        let first_param = param_types.first().cloned();
        // and the first result's: `-> ty` or `-> (ty, ...)`
        let ret = if self.toks.get(j).map(|t| &t.0) == Some(&Tok::Arrow) {
            let at = if self.toks.get(j + 1).map(|t| &t.0) == Some(&Tok::LParen) { j + 2 } else { j + 1 };
            Some(self.type_expr_at(at, &params)?.0)
        } else {
            None
        };
        self.generics.push(GenericFn {
            name,
            params,
            lo,
            param_types,
            first_param,
            ret,
        });
        self.pos = hi + 1;
        Ok(())
    }

    /// pass 1, a plain function: remember its parameter types under its
    /// name, so a literal argument at a call site knows its type
    fn record_signature(&mut self) -> Result<(), ParseError> {
        let at = self.pos;
        self.expect_ident()?; // fn
        let name = self.expect_ident()?;
        self.expect(Tok::LParen)?;
        let mut tys = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                self.expect_ident()?;
                self.expect(Tok::Colon)?;
                tys.push(self.expect_type()?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        self.sigs.insert(name, tys);
        self.pos = at;
        Ok(())
    }

    /// `fn name = generic(args)`: a named instantiation
    fn parse_alias(&mut self) -> Result<(), ParseError> {
        self.expect_ident()?; // fn
        let name = self.expect_ident()?;
        self.expect(Tok::Equals)?;
        let generic = self.expect_ident()?;
        let args = self.instance_args()?;
        self.expect(Tok::Newline)?;
        self.request_instance(&generic, args, Some(name))?;
        Ok(())
    }

    /// `(expr, expr)` of a generic's arguments, evaluated in the current env
    fn instance_args(&mut self) -> Result<Vec<i64>, ParseError> {
        self.expect(Tok::LParen)?;
        let params: Vec<String> = self.env.iter().map(|(n, _)| n.clone()).collect();
        let mut args = Vec::new();
        loop {
            let (e, next) = self.int_expr_at(self.pos, &params)?;
            self.pos = next;
            args.push(e.eval(&self.env).map_err(|m| self.err(m))?);
            if self.eat(&Tok::RParen) {
                break;
            }
            self.expect(Tok::Comma)?;
        }
        Ok(args)
    }

    /// the name of generic(args), instantiating it if new; by name, the
    /// generic must be the only one of that name taking that many arguments
    /// the value for an unbound generic parameter: the enclosing
    /// instantiation's parameter of that name, else the policy's
    fn named(&self, param: &str) -> Option<i64> {
        self.env.iter().find(|(n, _)| n == param).map(|(_, v)| *v).or_else(|| self.policy.named(param))
    }

    fn request_instance(&mut self, generic: &str, args: Vec<i64>, name: Option<String>) -> Result<String, ParseError> {
        // `add(8, 23)` for a `fn add(E, M, round)`: the trailing
        // parameters the policy names are supplied by it
        let fits = |p: &Parser, g: usize| {
            let params = &p.generics[g].params;
            params.len() >= args.len() && params[args.len()..].iter().all(|q| p.named(q).is_some())
        };
        let candidates: Vec<usize> = (0..self.generics.len())
            .filter(|&g| self.generics[g].name == generic && fits(self, g))
            .collect();
        if self.generics.iter().all(|g| g.name != generic) {
            return Err(self.err(format!("'{}' is not a generic function", generic)));
        }
        match candidates.len() {
            0 => Err(self.err(format!("no '{}' takes {} parameter(s)", generic, args.len()))),
            1 => {
                let g = candidates[0];
                let mut args = args;
                for q in &self.generics[g].params.clone()[args.len()..] {
                    args.push(self.named(q).unwrap());
                }
                self.request_instance_of(g, args, name)
            }
            _ => Err(self.err(format!(
                "'{}' has several forms taking {} parameter(s); apply it as an operation so the types choose",
                generic,
                args.len()
            ))),
        }
    }

    fn request_instance_of(&mut self, g: usize, args: Vec<i64>, name: Option<String>) -> Result<String, ParseError> {
        let key = (g, args.clone());
        if let Some(existing) = self.instances.get(&key) {
            if let Some(n) = name {
                if *existing != n {
                    return Err(self.err(format!(
                        "'{}' is already instantiated as '{}'",
                        self.generics[g].name, existing
                    )));
                }
            }
            return Ok(existing.clone());
        }
        let name = name.unwrap_or_else(|| {
            let a: Vec<String> = args.iter().map(|v| v.to_string()).collect();
            let base = format!("{}_{}", self.generics[g].name, a.join("_"));
            if self.instances.values().any(|n| *n == base) {
                format!("{}_{}", base, g) // another form of the same name got it
            } else {
                base
            }
        });
        self.instances.insert(key, name.clone());
        // its parameter types, for literal arguments at call sites
        let env: Vec<(String, i64)> = self.generics[g].params.iter().cloned().zip(args.iter().copied()).collect();
        let ptys = self.generics[g].param_types.clone();
        let mut tys = Vec::new();
        for te in &ptys {
            tys.push(self.instantiate(te, &env, 0).map_err(|m| self.err(m))?);
        }
        self.sigs.insert(name.clone(), tys);
        self.pending.push((g, args, name.clone()));
        Ok(name)
    }

    /// Match a declared type against a concrete one, binding width
    /// parameters: `float(E, M)` against a pack from float(8, 23) binds E
    /// and M; `i(W)` against i32 binds W; builtins must be equal.
    fn unify(&self, expr: &TypeExpr, ty: Type, binds: &mut Vec<(String, i64)>) -> bool {
        let bind = |p: &str, v: i64, binds: &mut Vec<(String, i64)>| match binds.iter().find(|(n, _)| n == p) {
            Some((_, w)) => *w == v,
            None => {
                binds.push((p.to_string(), v));
                true
            }
        };
        match expr {
            TypeExpr::Named { name, args } if args.is_empty() => match Type::from_name(name) {
                Some(t) => t == ty,
                None => matches!(ty, Type::Pack(i) if self.packs[i as usize].name == *name),
            },
            TypeExpr::Named { name, args } => {
                let Type::Pack(i) = ty else {
                    return false;
                };
                let Some((oname, vals)) = &self.packs[i as usize].origin else {
                    return false;
                };
                if oname != name || vals.len() != args.len() {
                    return false;
                }
                args.iter().zip(vals).all(|(e, &v)| match e {
                    IntExpr::Param(p) => bind(p, v, binds),
                    IntExpr::Lit(l) => *l == v,
                    _ => false,
                })
            }
            TypeExpr::Int { signed, bits } => {
                let Type::Int { signed: s, bits: b } = ty else {
                    return false;
                };
                if s != *signed {
                    return false;
                }
                match bits {
                    IntExpr::Param(p) => bind(p, b as i64, binds),
                    IntExpr::Lit(l) => *l == b as i64,
                    _ => false,
                }
            }
            TypeExpr::Pack(_) => false,
        }
    }

    /// skip to the end of the current line (a declaration already parsed)
    fn skip_line(&mut self) {
        while !matches!(self.next(), Ok(Tok::Newline) | Err(_)) {}
    }

    /// `type name = expr` or `type name(P, Q) = expr`, on one line
    fn parse_type_decl(&mut self) -> Result<(), ParseError> {
        self.expect_ident()?; // 'type'
        let name = self.expect_ident()?;
        if Type::from_name(&name).is_some() {
            self.pos -= 1;
            return Err(self.err(format!("type '{}' is already defined", name)));
        }
        let redeclared = self.types.iter().position(|t| t.name == name);
        let mut params = Vec::new();
        if self.eat(&Tok::LParen) {
            loop {
                params.push(self.expect_ident()?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        self.expect(Tok::Equals)?;
        let (body, next) = self.type_expr_at(self.pos, &params)?;
        self.pos = next;
        self.expect(Tok::Newline)?;
        // saying the same thing twice is fine (a file and the prelude may
        // both declare float(E, M)); saying something else is not
        if let Some(i) = redeclared {
            if self.types[i].params == params && self.types[i].body == body {
                return Ok(());
            }
            return Err(self.err(format!("type '{}' is already defined differently", name)));
        }
        // a plain alias is instantiated now, so its errors surface here and
        // a fresh pack takes the alias as its name
        if params.is_empty() {
            let before = self.packs.len();
            let ty = self.instantiate(&body, &[], 0).map_err(|m| self.err(m))?;
            if let Type::Pack(i) = ty {
                if i as usize >= before {
                    self.packs[i as usize].name = name.clone();
                }
            }
        }
        self.types.push(TypeDef { name, params, body });
        Ok(())
    }

    /// Parse a type expression starting at token `i`; `params` are the
    /// names allowed in width expressions. Returns the expression and the
    /// index after it.
    fn type_expr_at(&self, i: usize, params: &[String]) -> Result<(TypeExpr, usize), ParseError> {
        let at = |i: usize| self.toks.get(i).map(|t| &t.0);
        let line = self.toks.get(i).map(|t| t.1).unwrap_or(0);
        let err = |msg: String| ParseError { line, msg };
        let Some(Tok::Ident(name)) = at(i) else {
            return Err(err(format!(
                "expected a type, found {}",
                at(i).map(|t| t.to_string()).unwrap_or("end of input".into())
            )));
        };
        if name == "pack" {
            if at(i + 1) != Some(&Tok::LBrace) {
                return Err(err("expected '{' after 'pack'".into()));
            }
            let mut j = i + 2;
            let mut fields: Vec<(String, TypeExpr)> = Vec::new();
            loop {
                while at(j) == Some(&Tok::Newline) {
                    j += 1;
                }
                if at(j) == Some(&Tok::RBrace) {
                    j += 1;
                    break;
                }
                let Some(Tok::Ident(fname)) = at(j) else {
                    return Err(err("expected a field name".into()));
                };
                if fields.iter().any(|(n, _)| n == fname) {
                    return Err(err(format!("field '{}' appears twice", fname)));
                }
                if at(j + 1) != Some(&Tok::Colon) {
                    return Err(err(format!("expected ':' after field '{}'", fname)));
                }
                let (fty, next) = self.type_expr_at(j + 2, params)?;
                fields.push((fname.clone(), fty));
                j = next;
                while at(j) == Some(&Tok::Newline) {
                    j += 1;
                }
                if at(j) == Some(&Tok::Comma) {
                    j += 1;
                } else if at(j) != Some(&Tok::RBrace) {
                    return Err(err("expected ',' or '}' after a field".into()));
                }
            }
            if fields.is_empty() {
                return Err(err("a pack needs at least one field".into()));
            }
            return Ok((TypeExpr::Pack(fields), j));
        }
        // name, name(args), i(expr), u(expr)
        if at(i + 1) == Some(&Tok::LParen) {
            let mut args = Vec::new();
            let mut j = i + 2;
            loop {
                let (e, next) = self.int_expr_at(j, params)?;
                args.push(e);
                j = next;
                match at(j) {
                    Some(Tok::Comma) => j += 1,
                    Some(Tok::RParen) => {
                        j += 1;
                        break;
                    }
                    _ => return Err(err("expected ',' or ')' in a type's arguments".into())),
                }
            }
            if (name == "i" || name == "u") && args.len() == 1 {
                return Ok((
                    TypeExpr::Int {
                        signed: name == "i",
                        bits: args.pop().unwrap(),
                    },
                    j,
                ));
            }
            return Ok((
                TypeExpr::Named {
                    name: name.clone(),
                    args,
                },
                j,
            ));
        }
        Ok((
            TypeExpr::Named {
                name: name.clone(),
                args: Vec::new(),
            },
            i + 1,
        ))
    }

    /// width expression, lowest precedence first: `& |`, then `<< >>`,
    /// then `+ -`, then `*`, then atoms
    fn int_expr_at(&self, i: usize, params: &[String]) -> Result<(IntExpr, usize), ParseError> {
        let (mut lhs, mut j) = self.int_shift_at(i, params)?;
        loop {
            match self.toks.get(j).map(|t| &t.0) {
                Some(Tok::Amp) => {
                    let (rhs, next) = self.int_shift_at(j + 1, params)?;
                    lhs = IntExpr::And(Box::new(lhs), Box::new(rhs));
                    j = next;
                }
                Some(Tok::Pipe) => {
                    let (rhs, next) = self.int_shift_at(j + 1, params)?;
                    lhs = IntExpr::Or(Box::new(lhs), Box::new(rhs));
                    j = next;
                }
                _ => return Ok((lhs, j)),
            }
        }
    }

    fn int_shift_at(&self, i: usize, params: &[String]) -> Result<(IntExpr, usize), ParseError> {
        let (mut lhs, mut j) = self.int_sum_at(i, params)?;
        loop {
            match self.toks.get(j).map(|t| &t.0) {
                Some(Tok::ShiftL) => {
                    let (rhs, next) = self.int_sum_at(j + 1, params)?;
                    lhs = IntExpr::Shl(Box::new(lhs), Box::new(rhs));
                    j = next;
                }
                Some(Tok::ShiftR) => {
                    let (rhs, next) = self.int_sum_at(j + 1, params)?;
                    lhs = IntExpr::Shr(Box::new(lhs), Box::new(rhs));
                    j = next;
                }
                _ => return Ok((lhs, j)),
            }
        }
    }

    fn int_sum_at(&self, i: usize, params: &[String]) -> Result<(IntExpr, usize), ParseError> {
        let (mut lhs, mut j) = self.int_term_at(i, params)?;
        loop {
            match self.toks.get(j).map(|t| &t.0) {
                Some(Tok::Plus) => {
                    let (rhs, next) = self.int_term_at(j + 1, params)?;
                    lhs = IntExpr::Add(Box::new(lhs), Box::new(rhs));
                    j = next;
                }
                Some(Tok::Minus) => {
                    let (rhs, next) = self.int_term_at(j + 1, params)?;
                    lhs = IntExpr::Sub(Box::new(lhs), Box::new(rhs));
                    j = next;
                }
                // `E-1` lexes as E then the literal -1
                Some(Tok::Int(v)) if *v < 0 => {
                    lhs = IntExpr::Sub(Box::new(lhs), Box::new(IntExpr::Lit(-*v)));
                    j += 1;
                }
                _ => return Ok((lhs, j)),
            }
        }
    }

    fn int_term_at(&self, i: usize, params: &[String]) -> Result<(IntExpr, usize), ParseError> {
        let (mut lhs, mut j) = self.int_atom_at(i, params)?;
        while self.toks.get(j).map(|t| &t.0) == Some(&Tok::Star) {
            let (rhs, next) = self.int_atom_at(j + 1, params)?;
            lhs = IntExpr::Mul(Box::new(lhs), Box::new(rhs));
            j = next;
        }
        Ok((lhs, j))
    }

    fn int_atom_at(&self, i: usize, params: &[String]) -> Result<(IntExpr, usize), ParseError> {
        let line = self.toks.get(i).map(|t| t.1).unwrap_or(0);
        match self.toks.get(i).map(|t| &t.0) {
            Some(Tok::Int(v)) => Ok((IntExpr::Lit(*v), i + 1)),
            Some(Tok::Ident(p)) if params.contains(p) => Ok((IntExpr::Param(p.clone()), i + 1)),
            Some(Tok::Ident(p)) => Err(ParseError {
                line,
                msg: format!("'{}' is not a parameter of this type", p),
            }),
            Some(Tok::LParen) => {
                let (e, j) = self.int_expr_at(i + 1, params)?;
                if self.toks.get(j).map(|t| &t.0) != Some(&Tok::RParen) {
                    return Err(ParseError {
                        line,
                        msg: "expected ')' in a width expression".into(),
                    });
                }
                Ok((e, j + 1))
            }
            t => Err(ParseError {
                line,
                msg: format!(
                    "expected a width, found {}",
                    t.map(|t| t.to_string()).unwrap_or("end of input".into())
                ),
            }),
        }
    }

    /// parse and instantiate a type at token `i`; `env` binds the width
    /// parameters in scope (a generic function's, while its body parses)
    fn type_at(&mut self, i: usize, env: &[(String, i64)], depth: usize) -> Result<(Type, usize), ParseError> {
        let line = self.toks.get(i).map(|t| t.1).unwrap_or(0);
        let params: Vec<String> = env.iter().map(|(n, _)| n.clone()).collect();
        let (expr, next) = self.type_expr_at(i, &params)?;
        let ty = self.instantiate(&expr, env, depth).map_err(|msg| ParseError { line, msg })?;
        Ok((ty, next))
    }

    /// Evaluate a type expression to a concrete `Type`, creating (or
    /// finding) the packs it denotes. Packs are interned structurally, so
    /// every spelling of the same layout is the same type.
    fn instantiate(&mut self, expr: &TypeExpr, env: &[(String, i64)], depth: usize) -> Result<Type, String> {
        if depth > MAX_TYPE_DEPTH {
            return Err("type declarations nest too deeply (is a type defined in terms of itself?)".into());
        }
        match expr {
            TypeExpr::Int { signed, bits } => {
                let n = bits.eval(env)?;
                if !(1..=crate::wide::MAX_BITS as i64).contains(&n) {
                    return Err(format!("{} has {} bits; widths run from 1 to {}", expr, n, crate::wide::MAX_BITS));
                }
                Ok(Type::int(*signed, n as u32))
            }
            TypeExpr::Named { name, args } => {
                if args.is_empty() {
                    if let Some(t) = Type::from_name(name) {
                        return Ok(t);
                    }
                    // `scalar` is whichever family the policy says, itself
                    // bare, so that family's policy width applies
                    if name == "scalar" && !self.types.iter().any(|t| t.name == "scalar") {
                        let family = self.policy.scalar.to_string();
                        return self.instantiate(
                            &TypeExpr::Named {
                                name: family,
                                args: Vec::new(),
                            },
                            env,
                            depth + 1,
                        );
                    }
                }
                let def = self
                    .types
                    .iter()
                    .find(|t| t.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("unknown type '{}'", name))?;
                // a bare parametric name is abstract: the policy supplies
                // its arguments (`float` is float(E, M) for the target's
                // E, M)
                let policy_args = if args.is_empty() && !def.params.is_empty() {
                    self.policy.default_args(name)
                } else {
                    None
                };
                let mut inner = Vec::new();
                match &policy_args {
                    Some(vals) => {
                        if vals.len() != def.params.len() {
                            return Err(format!("the policy gives '{}' {} argument(s); it takes {}", name, vals.len(), def.params.len()));
                        }
                        for (p, v) in def.params.iter().zip(vals) {
                            inner.push((p.clone(), *v));
                        }
                    }
                    None => {
                        if def.params.len() != args.len() {
                            return Err(format!(
                                "type '{}' takes {} parameter(s), given {}",
                                name,
                                def.params.len(),
                                args.len()
                            ));
                        }
                        for (p, a) in def.params.iter().zip(args) {
                            inner.push((p.clone(), a.eval(env)?));
                        }
                    }
                }
                let before = self.packs.len();
                let ty = self.instantiate(&def.body, &inner, depth + 1)?;
                // a pack born from this instantiation is named by it and
                // remembers where it came from
                if let Type::Pack(i) = ty {
                    let vals: Vec<i64> = inner.iter().map(|(_, v)| *v).collect();
                    let p = &mut self.packs[i as usize];
                    if i as usize >= before {
                        p.name = if vals.is_empty() {
                            name.clone()
                        } else {
                            let a: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
                            format!("{}({})", name, a.join(", "))
                        };
                    }
                    if p.origin.is_none() && !def.params.is_empty() {
                        p.origin = Some((name.clone(), vals));
                    }
                }
                Ok(ty)
            }
            TypeExpr::Pack(fields) => {
                let mut out: Vec<(String, Type)> = Vec::new();
                let mut offsets = Vec::new();
                let mut width = 0u32;
                for (fname, fexpr) in fields {
                    let fty = self.instantiate(fexpr, env, depth + 1)?;
                    let w = match fty {
                        Type::Int { bits, .. } => bits as u32,
                        Type::Pack(i) => self.packs[i as usize].width,
                        t => {
                            return Err(format!(
                                "pack field '{}' must be an integer or pack type, not {}",
                                fname,
                                t.name()
                            ))
                        }
                    };
                    offsets.push(width);
                    width += w;
                    out.push((fname.clone(), fty));
                }
                if width > crate::wide::MAX_BITS {
                    return Err(format!("{} is {} bits wide; packs fit in {}", expr, width, crate::wide::MAX_BITS));
                }
                if let Some(i) = self.packs.iter().position(|p| p.fields == out) {
                    return Ok(Type::Pack(i as u32));
                }
                let id = self.packs.len() as u32;
                // the structural spelling; an enclosing alias or
                // instantiation renames it
                let name = TypeExpr::Pack(
                    out.iter()
                        .map(|(n, t)| {
                            (
                                n.clone(),
                                TypeExpr::Named {
                                    name: match t {
                                        Type::Pack(i) => self.packs[*i as usize].name.clone(),
                                        t => t.name(),
                                    },
                                    args: Vec::new(),
                                },
                            )
                        })
                        .collect(),
                )
                .to_string();
                self.packs.push(PackDef {
                    name,
                    fields: out,
                    offsets,
                    width,
                    origin: None,
                });
                Ok(Type::Pack(id))
            }
        }
    }

    fn expect_value(&mut self, scope: &mut FuncScope) -> Result<ValueId, ParseError> {
        self.parse_operand(scope, None)
    }

    /// A value, or a literal standing in for one. A literal takes the type
    /// the context wants, or the one written after it (`1: u8`); it becomes
    /// a hidden value defined by a `const` emitted just before the
    /// instruction.
    fn parse_operand(&mut self, scope: &mut FuncScope, want: Option<Type>) -> Result<ValueId, ParseError> {
        if let Some(Tok::Ident(name)) = self.peek() {
            if let Some(&id) = scope.value_ids.get(name) {
                self.pos += 1;
                return Ok(id);
            }
        }
        let at = self.pos;
        let Some(lit) = self.parse_lit()? else {
            let t = self.next()?;
            self.pos -= 1;
            return Err(self.err(match t {
                Tok::Ident(n) => format!("use of undefined value '{}'", n),
                t => format!("expected a value, found {}", t),
            }));
        };
        let ty = if self.eat(&Tok::Colon) {
            self.expect_type()?
        } else {
            match want {
                Some(t) => t,
                None => {
                    self.pos = at;
                    return Err(self.err("a literal needs a type here: write it as `1: i64`".to_string()));
                }
            }
        };
        self.make_literal(scope, &lit, ty).map_err(|m| {
            self.pos = at;
            self.err(m)
        })
    }

    /// The hidden instructions for a literal of type `ty`: a `const` when
    /// the type reads literals itself (integers, pointers, floats, plain
    /// packs by bit pattern); for a library number type (fixed, rational,
    /// unit, ...) a `const` in i64 or f64 followed by the library's own
    /// `conv` into the type — so every family gets literals through its
    /// conversion, and `x: scalar = const 0.5` means 0.5 whatever scalar is.
    fn make_literal(&mut self, scope: &mut FuncScope, lit: &Lit, ty: Type) -> Result<ValueId, String> {
        let direct = self.float_params(ty).is_some() || !ty.is_pack() || matches!(lit, Lit::Int(_)) && self.conv_from(ty, Type::I64).is_none();
        if direct {
            let bits = self.literal_bits(lit, ty)?;
            let id = scope.synth(ty, (ty, bits));
            self.consts.push(Inst::IConst { dst: id, imm: bits as i128 });
            return Ok(id);
        }
        let src_ty = match lit {
            Lit::Int(_) => Type::I64,
            _ => self.instantiate(
                &TypeExpr::Named {
                    name: "float".into(),
                    args: vec![IntExpr::Lit(11), IntExpr::Lit(52)],
                },
                &[],
                0,
            )?,
        };
        let callee = self
            .conv_from(ty, src_ty)
            .ok_or_else(|| format!("no conversion from a literal to {}", self.tyname_of(ty)))?;
        let bits = self.literal_bits(lit, src_ty)?;
        let src = scope.synth(src_ty, (src_ty, bits));
        self.consts.push(Inst::IConst { dst: src, imm: bits as i128 });
        let id = scope.synth(ty, (src_ty, bits));
        self.consts.push(Inst::Call {
            dsts: vec![id],
            callee,
            args: vec![src],
        });
        Ok(id)
    }

    /// the library's `conv` into `ty` from `from`, if there is one
    fn conv_from(&mut self, ty: Type, from: Type) -> Option<String> {
        let at = self.pos;
        let r = self.dispatch("conv", from, ty).ok();
        self.pos = at;
        r
    }

    /// a literal token, if one is next: an integer, a decimal, `inf`,
    /// `-inf`, `nan`
    fn parse_lit(&mut self) -> Result<Option<Lit>, ParseError> {
        Ok(match self.peek().cloned() {
            Some(Tok::Int(v)) => {
                self.pos += 1;
                Some(Lit::Int(v))
            }
            Some(Tok::Float(t)) => {
                self.pos += 1;
                Some(Lit::Dec(t))
            }
            Some(Tok::Ident(n)) if n == "inf" || n == "nan" => {
                self.pos += 1;
                Some(if n == "inf" { Lit::Inf(false) } else { Lit::NaN })
            }
            Some(Tok::Minus) if matches!(self.toks.get(self.pos + 1).map(|t| &t.0), Some(Tok::Ident(n)) if n == "inf") => {
                self.pos += 2;
                Some(Lit::Inf(true))
            }
            _ => None,
        })
    }

    /// the bits of a literal in a type: an integer as itself (or the bit
    /// pattern of a pack); on a float, the nearest float to the number
    fn literal_bits(&self, lit: &Lit, ty: Type) -> Result<i64, String> {
        if let Some((e, m)) = self.float_params(ty) {
            return Ok(match lit {
                Lit::Int(v) => int_to_float(*v, e, m) as i64,
                Lit::Dec(t) => decimal_to_float(t, e, m)? as i64,
                Lit::Inf(neg) => (((*neg as u64) << (e + m)) | (((1u64 << e) - 1) << m)) as i64,
                Lit::NaN => ((((1u64 << e) - 1) << m) | (1u64 << (m - 1))) as i64,
            });
        }
        match lit {
            Lit::Int(v) => Ok(*v),
            _ => Err(format!("a float literal needs a float type, not {}", self.tyname_of(ty))),
        }
    }

    /// (E, M) if the type is a `float(E, M)` pack
    fn float_params(&self, ty: Type) -> Option<(u32, u32)> {
        let Type::Pack(i) = ty else {
            return None;
        };
        match &self.packs[i as usize].origin {
            Some((name, args)) if name == "float" && args.len() == 2 => Some((args[0] as u32, args[1] as u32)),
            _ => None,
        }
    }

    fn tyname_of(&self, ty: Type) -> String {
        match ty {
            Type::Pack(i) => self.packs[i as usize].name.clone(),
            t => t.name(),
        }
    }

    /// two operands where a literal takes its type from the other operand
    /// (or from `want`, when the instruction fixes it)
    fn parse_pair(&mut self, scope: &mut FuncScope, want: Option<Type>) -> Result<(ValueId, ValueId), ParseError> {
        // a literal first: read the second operand first to learn its type
        let first_is_lit = !matches!(self.peek(), Some(Tok::Ident(n)) if scope.value_ids.contains_key(n));
        if first_is_lit && want.is_none() {
            let at = self.pos;
            let Some(lit) = self.parse_lit()? else {
                return Err(self.err("expected a value".to_string()));
            };
            if self.eat(&Tok::Colon) {
                let ty = self.expect_type()?;
                let lhs = self.make_literal(scope, &lit, ty).map_err(|m| self.err(m))?;
                self.expect(Tok::Comma)?;
                let rhs = self.parse_operand(scope, Some(ty))?;
                return Ok((lhs, rhs));
            }
            self.expect(Tok::Comma)?;
            let rhs = self.parse_operand(scope, None)?;
            let ty = scope.values[rhs.0 as usize].ty;
            let lhs = self.make_literal(scope, &lit, ty).map_err(|m| {
                self.pos = at;
                self.err(m)
            })?;
            return Ok((lhs, rhs));
        }
        let lhs = self.parse_operand(scope, want)?;
        self.expect(Tok::Comma)?;
        let rhs = self.parse_operand(scope, Some(scope.values[lhs.0 as usize].ty))?;
        Ok((lhs, rhs))
    }

    /// structured form: the pending consts, then the instruction
    fn emit(&mut self, st: &mut StructEmit, inst: Inst) -> (usize, usize) {
        self.flush(st);
        st.push(inst)
    }

    fn flush(&mut self, st: &mut StructEmit) {
        for c in self.consts.drain(..) {
            st.push(c);
        }
    }

    // -- pre-scans -----------------------------------------------------------

    /// Find the token range of the current function: from `fn` to its
    /// closing `}` (inclusive). Assumes we're positioned at `fn`.
    fn function_range(&self) -> Result<(usize, usize), ParseError> {
        let start = self.pos;
        let mut depth = 0usize;
        for i in start..self.toks.len() {
            match self.toks[i].0 {
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((start, i));
                    }
                }
                _ => {}
            }
        }
        Err(self.err("unterminated function: missing '}'"))
    }

    /// Every value definition in the format is the pattern `name : type` —
    /// function params, block params, and instruction results alike. One scan
    /// over the function's tokens builds the whole value table.
    fn prescan_values(&mut self, lo: usize, hi: usize) -> Result<FuncScope, ParseError> {
        let mut scope = FuncScope {
            values: Vec::new(),
            value_ids: HashMap::new(),
            block_ids: HashMap::new(),
            block_names: Vec::new(),
            block_params: Vec::new(),
        };
        let mut i = lo;
        while i + 2 <= hi {
            if let (Tok::Ident(name), Tok::Colon, Tok::Ident(_)) =
                (&self.toks[i].0, &self.toks[i + 1].0, &self.toks[i + 2].0)
            {
                let name = name.clone();
                let line = self.toks[i].1;
                let env = self.env.clone();
                let (ty, next) = self.type_at(i + 2, &env, 0)?;
                if scope.value_ids.contains_key(&name) {
                    return Err(ParseError {
                        line,
                        msg: format!("value '{}' is defined more than once", name),
                    });
                }
                let id = ValueId(scope.values.len() as u32);
                scope.value_ids.insert(name.clone(), id);
                scope.values.push(ValueData {
                    name,
                    ty,
                    literal: None,
                });
                i = next;
            } else {
                i += 1;
            }
        }
        Ok(scope)
    }

    /// Block headers are `name` at the start of a line (branch targets never
    /// are). Collect them in order so branches can reference blocks forward.
    fn prescan_blocks(
        &self,
        lo: usize,
        hi: usize,
        scope: &mut FuncScope,
    ) -> Result<(), ParseError> {
        for i in lo..=hi {
            if let Some(name) = self.label_at(i) {
                if scope.block_ids.contains_key(&name) {
                    return Err(ParseError {
                        line: self.toks[i].1,
                        msg: format!("block '{}' is defined more than once", name),
                    });
                }
                let id = BlockId(scope.block_names.len() as u32);
                scope.block_ids.insert(name.clone(), id);
                scope.block_names.push(name);
                // its parameters: `name(a: ty, b: ty):`
                let mut params = Vec::new();
                if self.toks.get(i + 1).map(|t| &t.0) == Some(&Tok::LParen) {
                    let mut j = i + 2;
                    while let (Some(Tok::Ident(n)), Some(Tok::Colon)) =
                        (self.toks.get(j).map(|t| &t.0), self.toks.get(j + 1).map(|t| &t.0))
                    {
                        if let Some(&id) = scope.value_ids.get(n) {
                            params.push(id);
                        }
                        // skip the type: to the next comma at depth 0, or the close
                        let mut depth = 0;
                        j += 2;
                        loop {
                            match self.toks.get(j).map(|t| &t.0) {
                                Some(Tok::LParen) => depth += 1,
                                Some(Tok::RParen) if depth > 0 => depth -= 1,
                                Some(Tok::RParen) | Some(Tok::Newline) | None => break,
                                Some(Tok::Comma) if depth == 0 => {
                                    j += 1;
                                    break;
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                    }
                }
                scope.block_params.push(params);
            }
        }
        Ok(())
    }

    /// A block label starts a line: `name:` followed by a newline, or
    /// `name(params):`. Definitions (`name: type = ...`) and structured
    /// statements (`loop(...) {`) never look like that.
    fn label_at(&self, i: usize) -> Option<String> {
        let at_line_start = i == 0 || matches!(self.toks[i - 1].0, Tok::Newline);
        let Tok::Ident(name) = &self.toks[i].0 else {
            return None;
        };
        if !at_line_start {
            return None;
        }
        match self.toks.get(i + 1).map(|t| &t.0) {
            Some(Tok::Colon) => matches!(self.toks.get(i + 2).map(|t| &t.0), Some(Tok::Newline))
                .then(|| name.clone()),
            Some(Tok::LParen) => {
                let mut j = i + 2;
                let mut depth = 1;
                while let Some((t, _)) = self.toks.get(j) {
                    match t {
                        Tok::LParen => depth += 1,
                        Tok::RParen => {
                            depth -= 1;
                            if depth == 0 {
                                return matches!(self.toks.get(j + 1).map(|t| &t.0), Some(Tok::Colon))
                                    .then(|| name.clone());
                            }
                        }
                        Tok::Newline | Tok::LBrace | Tok::Equals => return None,
                        _ => {}
                    }
                    j += 1;
                }
                None
            }
            _ => None,
        }
    }

    // -- grammar -------------------------------------------------------------

    /// A function; for an instantiation of a generic, `instance` is the
    /// name it gets and the env is already bound
    fn parse_function(&mut self, instance: Option<String>) -> Result<Function, ParseError> {
        let kw = self.expect_ident()?;
        if kw != "fn" {
            self.pos -= 1;
            return Err(self.err(format!("expected 'fn', found '{}'", kw)));
        }
        self.pos -= 1; // rewind so the range starts at `fn`
        let (lo, hi) = self.function_range()?;
        let mut scope = self.prescan_values(lo, hi)?;
        self.prescan_blocks(lo, hi, &mut scope)?;
        self.pos += 1; // past `fn`

        let mut name = self.expect_ident()?;
        if let Some(n) = instance {
            // skip the (P, Q) group; the env already binds them
            self.expect(Tok::LParen)?;
            while !matches!(self.next()?, Tok::RParen) {}
            name = n;
        }

        // parameters: names resolve via the prescan; re-parse for order
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                let id = self.expect_value(&mut scope)?;
                self.expect(Tok::Colon)?;
                self.expect_type()?; // already recorded by the prescan
                params.push(id);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }

        // `-> ty` is shorthand for the one-element tuple `-> (ty)`
        let mut rets = Vec::new();
        if self.eat(&Tok::Arrow) {
            if self.eat(&Tok::LParen) {
                loop {
                    rets.push(self.expect_type()?);
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    self.expect(Tok::Comma)?;
                }
            } else {
                rets.push(self.expect_type()?);
            }
        }

        self.expect(Tok::LBrace)?;
        self.skip_newlines();
        self.cur_rets = rets.clone();

        // A body that opens with statements instead of a label is in
        // structured form; it parses via if/loop constructs and lowers to
        // the same block graph on the fly.
        if self.label_at(self.pos).is_none() && !matches!(self.peek(), Some(Tok::RBrace)) {
            let blocks = self.parse_structured_body(&mut scope)?;
            return Ok(Function {
                name,
                params,
                rets,
                values: scope.values,
                blocks,
                packs: Default::default(),
                instance: None,
                instance_names: Vec::new(),
                wide_sig: None,
            });
        }

        let mut blocks: Vec<Block> = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RBrace) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::Ident(_)) if self.label_at(self.pos).is_some() => {
                    let bname = self.expect_ident()?;
                    let mut bparams = Vec::new();
                    if self.eat(&Tok::LParen) && !self.eat(&Tok::RParen) {
                        loop {
                            let id = self.expect_value(&mut scope)?;
                            self.expect(Tok::Colon)?;
                            self.expect_type()?;
                            bparams.push(id);
                            if self.eat(&Tok::RParen) {
                                break;
                            }
                            self.expect(Tok::Comma)?;
                        }
                    }
                    self.expect(Tok::Colon)?;
                    self.expect(Tok::Newline)?;
                    self.skip_newlines();
                    blocks.push(Block {
                        name: bname,
                        params: bparams,
                        insts: Vec::new(),
                    });
                }
                Some(_) => {
                    let block = blocks
                        .last_mut()
                        .ok_or_else(|| self.err("instruction before the first block label"))?;
                    let inst = self.parse_inst(&mut scope)?;
                    block.insts.append(&mut self.consts);
                    block.insts.push(inst);
                    self.skip_newlines();
                }
                None => return Err(self.err("unexpected end of input inside a function")),
            }
        }

        Ok(Function {
            name,
            params,
            rets,
            values: scope.values,
            blocks,
            packs: Default::default(),
            instance: None,
                instance_names: Vec::new(),
                wide_sig: None,
        })
    }

    /// Look up a value being *defined*; the prescan registered it iff the
    /// definition is well-formed (`name: ty`), so a miss is a syntax error.
    fn def_id(&self, scope: &mut FuncScope, name: &str) -> Result<ValueId, ParseError> {
        scope.value_ids.get(name).copied().ok_or_else(|| {
            self.err(format!(
                "definition of '{}' is missing its ': type' annotation",
                name
            ))
        })
    }

    fn parse_inst(&mut self, scope: &mut FuncScope) -> Result<Inst, ParseError> {
        match self.next()? {
            // dst: ty [, dst2: ty ...] = op ...
            Tok::Ident(name) if matches!(self.peek(), Some(Tok::Colon)) => {
                let mut dsts = vec![self.def_id(scope, &name)?];
                self.expect(Tok::Colon)?;
                self.expect_type()?;
                while self.eat(&Tok::Comma) {
                    let n = self.expect_ident()?;
                    dsts.push(self.def_id(scope, &n)?);
                    self.expect(Tok::Colon)?;
                    self.expect_type()?;
                }
                self.expect(Tok::Equals)?;
                let op = self.expect_ident()?;
                let inst = if self.is_call(&op) {
                    let (callee, args) = self.parse_call_tail(op, scope)?;
                    Inst::Call { dsts, callee, args }
                } else if op == "unpack" {
                    let src = self.expect_value(scope)?;
                    Inst::Unpack { dsts, src }
                } else if dsts.len() == 1 {
                    self.parse_def_op(&op, dsts[0], scope)?
                } else {
                    return Err(self.err(format!(
                        "only calls and 'unpack' can define multiple values, not '{}'",
                        op
                    )));
                };
                self.expect(Tok::Newline)?;
                Ok(inst)
            }
            // op with no result
            Tok::Ident(op) => {
                let inst = self.parse_plain_op(&op, scope)?;
                self.expect(Tok::Newline)?;
                Ok(inst)
            }
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected an instruction, found {}", t)))
            }
        }
    }

    fn parse_def_op(&mut self, op: &str, dst: ValueId, scope: &mut FuncScope) -> Result<Inst, ParseError> {
        if let Some((_, bin)) = BINOPS.iter().find(|(n, _)| *n == op) {
            let dty = scope.values[dst.0 as usize].ty;
            let (lhs, rhs) = self.parse_pair(scope, Some(dty))?;
            // on a pack, the opcode is whatever generic function of that
            // name takes the pack's origin type: `add` on a float(8, 23)
            // is a call to add(8, 23) — the library, or the platform's
            // instruction for it
            // ... and so are `div`/`rem` on an integer wider than a word:
            // lib/wide.ssa's div(W)/rem(W), loops the lowering unrolls
            // into words like everything else
            let lty = scope.values[lhs.0 as usize].ty;
            let wide_divide = matches!(*bin, BinOp::Div | BinOp::Rem) && matches!(lty, Type::Int { bits, .. } if bits > 64);
            if lty.is_pack() || wide_divide {
                let callee = self.dispatch(op, scope.values[lhs.0 as usize].ty, scope.values[dst.0 as usize].ty)?;
                return Ok(Inst::Call {
                    dsts: vec![dst],
                    callee,
                    args: vec![lhs, rhs],
                });
            }
            return Ok(Inst::Bin {
                op: *bin,
                dst,
                lhs,
                rhs,
            });
        }
        if let Some(cc) = op.strip_prefix("cmp.") {
            let cond = CONDS
                .iter()
                .find(|(n, _)| *n == cc)
                .map(|(_, c)| *c)
                .ok_or_else(|| self.err(format!("unknown comparison condition '{}'", cc)))?;
            let (lhs, rhs) = self.parse_pair(scope, None)?;
            // on a pack, `cmp.lt` is the library's `lt` for that type
            if scope.values[lhs.0 as usize].ty.is_pack() {
                let callee = self.dispatch(cond.name(), scope.values[lhs.0 as usize].ty, scope.values[dst.0 as usize].ty)?;
                return Ok(Inst::Call {
                    dsts: vec![dst],
                    callee,
                    args: vec![lhs, rhs],
                });
            }
            return Ok(Inst::ICmp {
                cond,
                dst,
                lhs,
                rhs,
            });
        }
        match op {
            "const" => {
                let dty = scope.values[dst.0 as usize].ty;
                // a library number type: the literal through its conv
                if dty.is_pack() && self.float_params(dty).is_none() && self.conv_from(dty, Type::I64).is_some() {
                    let at = self.pos;
                    let Some(lit) = self.parse_lit()? else {
                        return Err(self.err("expected a number".to_string()));
                    };
                    let tmp = self.make_literal(scope, &lit, dty).map_err(|m| {
                        self.pos = at;
                        self.err(m)
                    })?;
                    // the last hidden instruction defines tmp; make it define dst instead
                    let Some(Inst::Call { callee, args, .. }) = self.consts.pop() else {
                        unreachable!()
                    };
                    let _ = tmp;
                    return Ok(Inst::Call {
                        dsts: vec![dst],
                        callee,
                        args,
                    });
                }
                if self.float_params(dty).is_some() {
                    // a number: 1.5, 2, -inf, nan — rounded to the float type
                    let at = self.pos;
                    let Some(lit) = self.parse_lit()? else {
                        return Err(self.err("expected a number".to_string()));
                    };
                    let imm = self.literal_bits(&lit, dty).map_err(|m| {
                        self.pos = at;
                        self.err(m)
                    })?;
                    return Ok(Inst::IConst { dst, imm: imm as i128 });
                }
                // an integer, or in a generic an expression over its parameters
                let params: Vec<String> = self.env.iter().map(|(n, _)| n.clone()).collect();
                let (e, next) = self.int_expr_at(self.pos, &params)?;
                self.pos = next;
                let imm = e.eval128(&self.env).map_err(|m| self.err(m))?;
                Ok(Inst::IConst { dst, imm })
            }
            "iconst" => Err(self.err("'iconst' is spelled 'const' (and takes 1.5, inf, nan on a float)".to_string())),
            "conv" | "cast" => {
                let src = self.expect_value(scope)?;
                let (ts, td) = (scope.values[src.0 as usize].ty, scope.values[dst.0 as usize].ty);
                // a conversion touching a pack is the library's: conv from
                // float(E, M) to i(W), from u(W) to float(E, M), ...
                if op == "conv" && (ts.is_pack() || td.is_pack()) {
                    let callee = self.dispatch(op, ts, td)?;
                    return Ok(Inst::Call {
                        dsts: vec![dst],
                        callee,
                        args: vec![src],
                    });
                }
                let cast = if op == "conv" { CastOp::Conv } else { CastOp::Cast };
                Ok(Inst::Cast { op: cast, dst, src })
            }
            "pack" => {
                let wants: Vec<Type> = match scope.values[dst.0 as usize].ty {
                    Type::Pack(i) => self.packs[i as usize].fields.iter().map(|(_, t)| *t).collect(),
                    _ => Vec::new(),
                };
                let args = self.parse_value_list(scope, &wants)?;
                Ok(Inst::Pack { dst, args })
            }
            "unpack" => {
                let src = self.expect_value(scope)?;
                Ok(Inst::Unpack {
                    dsts: vec![dst],
                    src,
                })
            }
            "get" => {
                let src = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let field = self.expect_field(scope, src)?;
                Ok(Inst::Get { dst, src, field })
            }
            "set" => {
                let src = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let field = self.expect_field(scope, src)?;
                self.expect(Tok::Comma)?;
                let fty = match scope.values[src.0 as usize].ty {
                    Type::Pack(i) => Some(self.packs[i as usize].fields[field as usize].1),
                    _ => None,
                };
                let val = self.parse_operand(scope, fty)?;
                Ok(Inst::Set {
                    dst,
                    src,
                    field,
                    val,
                })
            }
            "load" => {
                let addr = self.expect_value(scope)?;
                Ok(Inst::Load { dst, addr })
            }
            "ptradd" => {
                let base = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let off = self.parse_operand(scope, Some(Type::I64))?;
                Ok(Inst::PtrAdd { dst, base, off })
            }
            "call" => Err(self.err("'call' is implied: write name(args)".to_string())),
            _ => {
                // a library-defined operation: `sqrt a` on a float is the
                // generic sqrt(E, M) taking float(E, M), any arity; the
                // first operand is a value, and later literals take its type
                // (`fma x, y, 0.0`)
                if !matches!(self.peek(), Some(Tok::Ident(n)) if scope.value_ids.contains_key(n)) {
                    return Err(self.err(format!("unknown opcode '{}'", op)));
                }
                let first = self.parse_operand(scope, None)?;
                let fty = scope.values[first.0 as usize].ty;
                let mut args = vec![first];
                while self.eat(&Tok::Comma) {
                    args.push(self.parse_operand(scope, Some(fty))?);
                }
                if !scope.values[first.0 as usize].ty.is_pack() {
                    return Err(self.err(format!("unknown opcode '{}'", op)));
                }
                let callee = self.dispatch(op, scope.values[first.0 as usize].ty, scope.values[dst.0 as usize].ty)?;
                Ok(Inst::Call {
                    dsts: vec![dst],
                    callee,
                    args,
                })
            }
        }
    }

    /// The instance of operation `op` for a source type and a result type:
    /// the generic named `op` whose first parameter matches the source and
    /// whose first result matches the destination, with the width
    /// parameters those matches bind.
    fn dispatch(&mut self, op: &str, src: Type, dst: Type) -> Result<String, ParseError> {
        let tyname = |p: &Parser, t: Type| match t {
            Type::Pack(i) => p.packs[i as usize].name.clone(),
            t => t.name(),
        };
        for g in 0..self.generics.len() {
            if self.generics[g].name != op {
                continue;
            }
            let (Some(first), ret) = (self.generics[g].first_param.clone(), self.generics[g].ret.clone()) else {
                continue;
            };
            let mut binds = Vec::new();
            if !self.unify(&first, src, &mut binds) {
                continue;
            }
            if let Some(r) = ret {
                if !self.unify(&r, dst, &mut binds) {
                    continue;
                }
            }
            let params = self.generics[g].params.clone();
            let args: Option<Vec<i64>> = params
                .iter()
                .map(|p| binds.iter().find(|(n, _)| n == p).map(|(_, v)| *v).or_else(|| self.named(p)))
                .collect();
            if let Some(args) = args {
                return self.request_instance_of(g, args, None);
            }
        }
        Err(self.err(format!(
            "no '{}' from {} to {}: define a generic fn {} whose first parameter and result match",
            op,
            tyname(self, src),
            tyname(self, dst),
            op
        )))
    }

    /// a field name of the pack-typed value `of`, resolved to its index
    fn expect_field(&mut self, scope: &mut FuncScope, of: ValueId) -> Result<u32, ParseError> {
        let fname = self.expect_ident()?;
        let ty = scope.values[of.0 as usize].ty;
        let Type::Pack(i) = ty else {
            self.pos -= 1;
            return Err(self.err(format!(
                "'{}' is {}, not a pack; it has no field '{}'",
                scope.values[of.0 as usize].name,
                ty.name(),
                fname
            )));
        };
        let (found, pname) = {
            let def = &self.packs[i as usize];
            (def.field(&fname), def.name.clone())
        };
        found.map(|k| k as u32).ok_or_else(|| {
            self.pos -= 1;
            self.err(format!("pack '{}' has no field '{}'", pname, fname))
        })
    }

    fn parse_plain_op(&mut self, op: &str, scope: &mut FuncScope) -> Result<Inst, ParseError> {
        match op {
            "store" => {
                let val = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let addr = self.expect_value(scope)?;
                Ok(Inst::Store { val, addr })
            }
            _ if self.is_call(op) => {
                let (callee, args) = self.parse_call_tail(op.to_string(), scope)?;
                Ok(Inst::Call {
                    dsts: Vec::new(),
                    callee,
                    args,
                })
            }
            "jmp" => {
                let (target, args) = self.parse_branch_target(scope)?;
                Ok(Inst::Jmp { target, args })
            }
            "br" => {
                let cond = self.parse_operand(scope, Some(Type::U1))?;
                self.expect(Tok::Comma)?;
                let (then_target, then_args) = self.parse_branch_target(scope)?;
                self.expect(Tok::Comma)?;
                let (else_target, else_args) = self.parse_branch_target(scope)?;
                Ok(Inst::Br {
                    cond,
                    then_target,
                    then_args,
                    else_target,
                    else_args,
                })
            }
            "ret" => {
                let wants = self.cur_rets.clone();
                let vals = self.parse_value_list(scope, &wants)?;
                Ok(Inst::Ret { vals })
            }
            "call" => Err(self.err("'call' is implied: write name(args)".to_string())),
            _ => Err(self.err(format!(
                "unknown opcode '{}' (or missing 'dst: ty =' before it)",
                op
            ))),
        }
    }

    /// a name followed by '(' in operation position is a call — even one
    /// spelled like an opcode, as in `add(8, 23)(x, y)`. Only `iconst
    /// (expr)` and `loop(...)` take a parenthesis and mean something else.
    fn is_call(&self, op: &str) -> bool {
        !matches!(op, "const" | "iconst" | "loop" | "call") && matches!(self.peek(), Some(Tok::LParen))
    }

    fn parse_call_tail(&mut self, callee: String, scope: &mut FuncScope) -> Result<(String, Vec<ValueId>), ParseError> {
        let mut callee = callee;
        // `g(8, 23)(a, b)`: the first group is width arguments when a
        // second group follows it
        let is_inst = matches!(self.peek(), Some(Tok::LParen)) && {
            let mut j = self.pos + 1;
            let mut depth = 1;
            while depth > 0 {
                match self.toks.get(j).map(|t| &t.0) {
                    Some(Tok::LParen) => depth += 1,
                    Some(Tok::RParen) => depth -= 1,
                    Some(Tok::Newline) | None => break,
                    _ => {}
                }
                j += 1;
            }
            depth == 0 && self.toks.get(j).map(|t| &t.0) == Some(&Tok::LParen)
        };
        if is_inst {
            let args = self.instance_args()?;
            callee = self.request_instance(&callee, args, None)?;
        }
        self.expect(Tok::LParen)?;
        let wants: Vec<Type> = self.sigs.get(&callee).cloned().unwrap_or_default();
        let mut args = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                let want = wants.get(args.len()).copied();
                args.push(self.parse_operand(scope, want)?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        Ok((callee, args))
    }

    // -- structured control flow --------------------------------------------
    // Structured bodies (if / loop / break / continue / yield / ret) are
    // sugar: they lower to the flat block graph during parsing. This is the
    // easy direction — the reverse (CFG -> structured, the "relooper"
    // problem) is what structured-only targets like wasm force on you.

    fn parse_structured_body(&mut self, scope: &mut FuncScope) -> Result<Vec<Block>, ParseError> {
        let mut st = StructEmit {
            blocks: Vec::new(),
            cur: 0,
            loop_stack: Vec::new(),
            yield_stack: Vec::new(),
        };
        st.new_block(Vec::new()); // entry
        self.parse_struct_stmts(scope, &mut st)?;
        self.expect(Tok::RBrace)?;
        Ok(st.blocks)
    }

    /// Parse statements up to (not consuming) the closing '}'. Returns
    /// whether every path through them terminated (ret/break/continue/yield,
    /// or an if whose arms all terminate).
    fn parse_struct_stmts(&mut self, scope: &mut FuncScope, st: &mut StructEmit) -> Result<bool, ParseError> {
        self.skip_newlines();
        loop {
            if matches!(self.peek(), Some(Tok::RBrace)) {
                return Ok(false);
            }
            let terminated = self.parse_struct_stmt(scope, st)?;
            self.skip_newlines();
            if terminated {
                if !matches!(self.peek(), Some(Tok::RBrace)) {
                    return Err(self.err("unreachable code after a terminating statement"));
                }
                return Ok(true);
            }
        }
    }

    fn parse_struct_stmt(&mut self, scope: &mut FuncScope, st: &mut StructEmit) -> Result<bool, ParseError> {
        match self.next()? {
            Tok::Ident(name) if matches!(self.peek(), Some(Tok::Colon)) => {
                let mut dsts = vec![self.def_id(scope, &name)?];
                self.expect(Tok::Colon)?;
                self.expect_type()?;
                while self.eat(&Tok::Comma) {
                    let n = self.expect_ident()?;
                    dsts.push(self.def_id(scope, &n)?);
                    self.expect(Tok::Colon)?;
                    self.expect_type()?;
                }
                self.expect(Tok::Equals)?;
                let op = self.expect_ident()?;
                match op.as_str() {
                    "if" => return self.parse_struct_if(scope, st, dsts),
                    "loop" => return self.parse_struct_loop(scope, st, dsts),
                    _ if self.is_call(&op) => {
                        let (callee, args) = self.parse_call_tail(op, scope)?;
                        self.emit(st, Inst::Call { dsts, callee, args });
                    }
                    "unpack" => {
                        let src = self.expect_value(scope)?;
                        self.emit(st, Inst::Unpack { dsts, src });
                    }
                    _ => {
                        if dsts.len() > 1 {
                            return Err(self.err(format!(
                                "only calls and 'unpack' can define multiple values, not '{}'",
                                op
                            )));
                        }
                        let inst = self.parse_def_op(&op, dsts[0], scope)?;
                        self.emit(st, inst);
                    }
                }
                self.expect(Tok::Newline)?;
                Ok(false)
            }
            Tok::Ident(op) => match op.as_str() {
                "if" => self.parse_struct_if(scope, st, Vec::new()),
                "loop" => self.parse_struct_loop(scope, st, Vec::new()),
                "break" => {
                    let Some(frame) = st.loop_stack.last() else {
                        return Err(self.err("'break' outside a loop"));
                    };
                    let wants = frame.rets.clone();
                    let vals = self.parse_value_list(scope, &wants)?;
                    let at = self.emit(st, Inst::Jmp {
                        target: DUMMY_BLOCK,
                        args: vals,
                    });
                    st.loop_stack.last_mut().unwrap().breaks.push(at);
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "continue" => {
                    let Some(frame) = st.loop_stack.last() else {
                        return Err(self.err("'continue' outside a loop"));
                    };
                    let header = frame.header;
                    let wants: Vec<Type> = st.blocks[header.0 as usize]
                        .params
                        .iter()
                        .map(|&p| scope.values[p.0 as usize].ty)
                        .collect();
                    let vals = self.parse_value_list(scope, &wants)?;
                    self.emit(st, Inst::Jmp {
                        target: header,
                        args: vals,
                    });
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "yield" => {
                    let Some(frame) = st.yield_stack.last() else {
                        return Err(self.err("'yield' outside an if"));
                    };
                    let wants = frame.1.clone();
                    let vals = self.parse_value_list(scope, &wants)?;
                    let at = self.emit(st, Inst::Jmp {
                        target: DUMMY_BLOCK,
                        args: vals,
                    });
                    st.yield_stack.last_mut().unwrap().0.push(at);
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "ret" => {
                    let wants = self.cur_rets.clone();
                    let vals = self.parse_value_list(scope, &wants)?;
                    self.emit(st, Inst::Ret { vals });
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "store" => {
                    let inst = self.parse_plain_op(&op, scope)?;
                    self.emit(st, inst);
                    self.expect(Tok::Newline)?;
                    Ok(false)
                }
                _ if self.is_call(&op) => {
                    let inst = self.parse_plain_op(&op, scope)?;
                    self.emit(st, inst);
                    self.expect(Tok::Newline)?;
                    Ok(false)
                }
                "jmp" | "br" => Err(self.err(
                    "jmp/br are not allowed in a structured function; use if/loop/break/continue",
                )),
                _ => Err(self.err(format!("unknown opcode '{}'", op))),
            },
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a statement, found {}", t)))
            }
        }
    }

    fn parse_struct_if(
        &mut self,
        scope: &mut FuncScope,
        st: &mut StructEmit,
        dsts: Vec<ValueId>,
    ) -> Result<bool, ParseError> {
        let cond = self.parse_operand(scope, Some(Type::U1))?;
        self.flush(st); // the condition's consts belong before the branch
        self.expect(Tok::LBrace)?;
        self.expect(Tok::Newline)?;

        let before = st.cur;
        let then_b = st.new_block(Vec::new());
        let dst_types: Vec<Type> = dsts.iter().map(|&d| scope.values[d.0 as usize].ty).collect();
        st.yield_stack.push((Vec::new(), dst_types)); // collects edges into the join

        st.cur = then_b.0 as usize;
        let t_then = self.parse_struct_stmts(scope, st)?;
        self.expect(Tok::RBrace)?;
        if !t_then {
            if !dsts.is_empty() {
                return Err(self.err("this if yields values, so each arm must end with 'yield'"));
            }
            let at = st.push(Inst::Jmp {
                target: DUMMY_BLOCK,
                args: Vec::new(),
            });
            st.yield_stack.last_mut().unwrap().0.push(at);
        }

        if matches!(self.peek(), Some(Tok::Ident(k)) if k == "else") {
            self.pos += 1;
            self.expect(Tok::LBrace)?;
            self.expect(Tok::Newline)?;
            let else_b = st.new_block(Vec::new());
            st.cur = else_b.0 as usize;
            let t_else = self.parse_struct_stmts(scope, st)?;
            self.expect(Tok::RBrace)?;
            if !t_else {
                if !dsts.is_empty() {
                    return Err(
                        self.err("this if yields values, so each arm must end with 'yield'")
                    );
                }
                let at = st.push(Inst::Jmp {
                    target: DUMMY_BLOCK,
                    args: Vec::new(),
                });
                st.yield_stack.last_mut().unwrap().0.push(at);
            }
            st.blocks[before].insts.push(Inst::Br {
                cond,
                then_target: then_b,
                then_args: Vec::new(),
                else_target: else_b,
                else_args: Vec::new(),
            });
        } else {
            // no else arm: the false edge goes straight to the join
            if !dsts.is_empty() {
                return Err(self.err("an if that yields values needs an else arm"));
            }
            st.blocks[before].insts.push(Inst::Br {
                cond,
                then_target: then_b,
                then_args: Vec::new(),
                else_target: DUMMY_BLOCK,
                else_args: Vec::new(),
            });
            let at = (before, st.blocks[before].insts.len() - 1);
            st.yield_stack.last_mut().unwrap().0.push(at);
        }
        self.expect(Tok::Newline)?;

        let (pending, _) = st.yield_stack.pop().unwrap();
        if pending.is_empty() {
            Ok(true) // every arm terminated; nothing can follow
        } else {
            let join = st.new_block(dsts);
            for at in pending {
                st.patch_target(at, join);
            }
            st.cur = join.0 as usize;
            Ok(false)
        }
    }

    fn parse_struct_loop(
        &mut self,
        scope: &mut FuncScope,
        st: &mut StructEmit,
        dsts: Vec<ValueId>,
    ) -> Result<bool, ParseError> {
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        let mut inits = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                let n = self.expect_ident()?;
                params.push(self.def_id(scope, &n)?);
                self.expect(Tok::Colon)?;
                let pty = self.expect_type()?;
                self.expect(Tok::Equals)?;
                inits.push(self.parse_operand(scope, Some(pty))?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        self.expect(Tok::LBrace)?;
        self.expect(Tok::Newline)?;

        let header = st.new_block(params);
        self.emit(st, Inst::Jmp {
            target: header,
            args: inits,
        });
        let rets: Vec<Type> = dsts.iter().map(|&d| scope.values[d.0 as usize].ty).collect();
        st.loop_stack.push(LoopFrame {
            header,
            breaks: Vec::new(),
            rets,
        });
        st.cur = header.0 as usize;
        let terminated = self.parse_struct_stmts(scope, st)?;
        self.expect(Tok::RBrace)?;
        self.expect(Tok::Newline)?;
        if !terminated {
            return Err(self.err("loop body must end with break, continue, or ret"));
        }
        let frame = st.loop_stack.pop().unwrap();
        if frame.breaks.is_empty() {
            if !dsts.is_empty() {
                return Err(self.err("loop yields values but contains no 'break'"));
            }
            Ok(true) // no exit: nothing can follow
        } else {
            let exit = st.new_block(dsts);
            for at in frame.breaks {
                st.patch_target(at, exit);
            }
            st.cur = exit.0 as usize;
            Ok(false)
        }
    }

    /// comma-separated operands, each literal typed by position from `wants`
    fn parse_value_list(&mut self, scope: &mut FuncScope, wants: &[Type]) -> Result<Vec<ValueId>, ParseError> {
        let mut vals = Vec::new();
        if matches!(self.peek(), Some(Tok::Ident(_)) | Some(Tok::Int(_)) | Some(Tok::Float(_)) | Some(Tok::Minus)) {
            loop {
                let want = wants.get(vals.len()).copied();
                vals.push(self.parse_operand(scope, want)?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        Ok(vals)
    }

    fn parse_branch_target(
        &mut self,
        scope: &mut FuncScope,
    ) -> Result<(BlockId, Vec<ValueId>), ParseError> {
        let target = match self.next()? {
            Tok::Ident(name) => scope.block_ids.get(&name).copied().ok_or_else(|| {
                self.pos -= 1;
                self.err(format!("branch to undefined block '{}'", name))
            })?,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a block, found {}", t)));
            }
        };
        let mut args = Vec::new();
        if self.eat(&Tok::LParen) && !self.eat(&Tok::RParen) {
            let wants: Vec<Type> = scope.block_params[target.0 as usize]
                .iter()
                .map(|&p| scope.values[p.0 as usize].ty)
                .collect();
            loop {
                let want = wants.get(args.len()).copied();
                args.push(self.parse_operand(scope, want)?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        Ok((target, args))
    }
}

// ---------------------------------------------------------------------------
// Printer

impl fmt::Display for Module {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for t in &self.types {
            writeln!(f, "{}", t)?;
        }
        for (i, func) in self.funcs.iter().enumerate() {
            if i > 0 || !self.types.is_empty() {
                writeln!(f)?;
            }
            write!(f, "{}", func)?;
        }
        Ok(())
    }
}

impl Function {
    fn fmt_value(&self, id: ValueId) -> String {
        let v = self.value(id);
        match v.literal {
            Some((lt, bits)) => self.literal_text(lt, bits as i128),
            None => v.name.clone(),
        }
    }

    fn is_hidden(&self, id: ValueId) -> bool {
        self.value(id).literal.is_some()
    }

    /// a value where a literal would have nothing to type it: printed
    /// with its type, `200: u8`
    fn fmt_value_typed(&self, id: ValueId) -> String {
        let v = self.value(id);
        match v.literal {
            Some((lt, bits)) => format!("{}: {}", self.literal_text(lt, bits as i128), self.tyname(v.ty)),
            None => v.name.clone(),
        }
    }

    /// (E, M) if the type is a float(E, M) pack
    pub fn float_params(&self, ty: Type) -> Option<(u32, u32)> {
        match &self.pack(ty)?.origin {
            Some((name, args)) if name == "float" && args.len() == 2 => Some((args[0] as u32, args[1] as u32)),
            _ => None,
        }
    }

    /// a constant as source text: a float as the shortest decimal that
    /// reads back to the same value, anything else as its integer
    fn literal_text(&self, ty: Type, bits: i128) -> String {
        let Some((e, m)) = self.float_params(ty) else {
            return bits.to_string();
        };
        let bits = bits as i64;
        let bits = bits as u64;
        let emax = (1u64 << e) - 1;
        let (sign, exp, mant) = (bits >> (e + m) & 1, (bits >> m) & emax, bits & ((1u64 << m) - 1));
        if exp == emax {
            return if mant != 0 { "nan".into() } else if sign == 1 { "-inf".into() } else { "inf".into() };
        }
        // the value is exact in f64 for every E <= 11, M <= 52; print the
        // shortest decimal (fixed or exponent form) that rounds back to
        // these bits at this width
        let bias = (1i64 << (e - 1)) - 1;
        let mag = if exp == 0 {
            mant as f64 * 2f64.powi((1 - bias - m as i64) as i32)
        } else {
            (mant | (1u64 << m)) as f64 * 2f64.powi((exp as i64 - bias - m as i64) as i32)
        };
        let v = if sign == 1 { -mag } else { mag };
        let back = |s: &str| decimal_to_float(s, e, m).ok() == Some(bits);
        let fixed = (0..=40usize)
            .map(|p| {
                let mut s = format!("{:.*}", p, v);
                if !s.contains('.') {
                    s.push_str(".0");
                }
                s
            })
            .find(|s| back(s));
        let sci = (0..=40usize).map(|p| format!("{:.*e}", p, v)).find(|s| back(s));
        match (fixed, sci) {
            (Some(f), Some(s)) => {
                if f.len() <= s.len() + 2 {
                    f
                } else {
                    s
                }
            }
            (Some(f), None) => f,
            (None, Some(s)) => s,
            (None, None) => format!("{:?}", v),
        }
    }

    fn fmt_def(&self, id: ValueId) -> String {
        let v = self.value(id);
        format!("{}: {}", v.name, self.tyname(v.ty))
    }

    fn fmt_field(&self, of: ValueId, field: u32) -> String {
        match self.pack(self.ty(of)) {
            Some(p) => p.fields[field as usize].0.clone(),
            None => format!("#{}", field),
        }
    }

    fn fmt_args(&self, args: &[ValueId]) -> String {
        args.iter()
            .map(|&a| self.fmt_value(a))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn fmt_target(&self, target: BlockId, args: &[ValueId]) -> String {
        let name = &self.blocks[target.0 as usize].name;
        if args.is_empty() {
            name.clone()
        } else {
            format!("{}({})", name, self.fmt_args(args))
        }
    }
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let params = self
            .params
            .iter()
            .map(|&p| self.fmt_def(p))
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "fn {}({})", self.name, params)?;
        match self.rets.len() {
            0 => {}
            1 => write!(f, " -> {}", self.tyname(self.rets[0]))?,
            _ => {
                let ts: Vec<String> = self.rets.iter().map(|&t| self.tyname(t)).collect();
                write!(f, " -> ({})", ts.join(", "))?;
            }
        }
        writeln!(f, " {{")?;
        for block in &self.blocks {
            if block.params.is_empty() {
                writeln!(f, "{}:", block.name)?;
            } else {
                let ps = block
                    .params
                    .iter()
                    .map(|&p| self.fmt_def(p))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(f, "{}({}):", block.name, ps)?;
            }
            for inst in &block.insts {
                // a literal's hidden instructions print at its use instead
                let dsts = inst_dsts(inst);
                if !dsts.is_empty() && dsts.iter().all(|&d| self.is_hidden(d)) {
                    continue;
                }
                writeln!(f, "    {}", self.fmt_inst(inst))?;
            }
        }
        writeln!(f, "}}")
    }
}

impl Function {
    fn fmt_inst(&self, inst: &Inst) -> String {
        match inst {
            Inst::IConst { dst, imm } => format!(
                "{} = const {}",
                self.fmt_def(*dst),
                self.literal_text(self.ty(*dst), *imm)
            ),
            Inst::Bin { op, dst, lhs, rhs } => format!(
                "{} = {} {}, {}",
                self.fmt_def(*dst),
                op.name(),
                self.fmt_value(*lhs),
                self.fmt_value(*rhs)
            ),
            Inst::ICmp {
                cond,
                dst,
                lhs,
                rhs,
            } => format!(
                "{} = cmp.{} {}, {}",
                self.fmt_def(*dst),
                cond.name(),
                self.fmt_value(*lhs),
                self.fmt_value(*rhs)
            ),
            Inst::Cast { op, dst, src } => format!(
                "{} = {} {}",
                self.fmt_def(*dst),
                op.name(),
                self.fmt_value_typed(*src)
            ),
            Inst::Pack { dst, args } => {
                format!("{} = pack {}", self.fmt_def(*dst), self.fmt_args(args))
            }
            Inst::Unpack { dsts, src } => {
                let defs: Vec<String> = dsts.iter().map(|&d| self.fmt_def(d)).collect();
                format!("{} = unpack {}", defs.join(", "), self.fmt_value(*src))
            }
            Inst::Get { dst, src, field } => format!(
                "{} = get {}, {}",
                self.fmt_def(*dst),
                self.fmt_value(*src),
                self.fmt_field(*src, *field)
            ),
            Inst::Set {
                dst,
                src,
                field,
                val,
            } => format!(
                "{} = set {}, {}, {}",
                self.fmt_def(*dst),
                self.fmt_value(*src),
                self.fmt_field(*src, *field),
                self.fmt_value(*val)
            ),
            Inst::Load { dst, addr } => {
                format!("{} = load {}", self.fmt_def(*dst), self.fmt_value(*addr))
            }
            Inst::Store { val, addr } => {
                format!("store {}, {}", self.fmt_value_typed(*val), self.fmt_value(*addr))
            }
            Inst::PtrAdd { dst, base, off } => format!(
                "{} = ptradd {}, {}",
                self.fmt_def(*dst),
                self.fmt_value(*base),
                self.fmt_value(*off)
            ),
            Inst::Call { dsts, callee, args } if args.len() == 1 && dsts.len() == 1 && self.is_hidden(args[0]) && callee.starts_with("conv_") => {
                // a library-typed constant: `x: fixed = const 0.5`
                format!("{} = const {}", self.fmt_def(dsts[0]), self.fmt_value(args[0]))
            }
            Inst::Call { dsts, callee, args } => {
                let call = format!("{}({})", callee, self.fmt_args(args));
                if dsts.is_empty() {
                    call
                } else {
                    let defs: Vec<String> = dsts.iter().map(|&d| self.fmt_def(d)).collect();
                    format!("{} = {}", defs.join(", "), call)
                }
            }
            Inst::Jmp { target, args } => format!("jmp {}", self.fmt_target(*target, args)),
            Inst::Br {
                cond,
                then_target,
                then_args,
                else_target,
                else_args,
            } => format!(
                "br {}, {}, {}",
                self.fmt_value(*cond),
                self.fmt_target(*then_target, then_args),
                self.fmt_target(*else_target, else_args)
            ),
            Inst::Ret { vals } => {
                if vals.is_empty() {
                    "ret".into()
                } else {
                    format!("ret {}", self.fmt_args(vals))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Verifier

pub fn verify(module: &Module) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    for func in &module.funcs {
        verify_function(module, func, &mut errs);
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn verify_function(module: &Module, func: &Function, errs: &mut Vec<String>) {
    let ctx = |msg: String| format!("{}: {}", func.name, msg);

    if func.blocks.is_empty() {
        errs.push(ctx("function has no blocks".into()));
        return;
    }

    // rule 0: abstract types are resolved before verification
    for v in &func.values {
        if v.ty.is_abstract() {
            errs.push(ctx(format!(
                "value '{}' has unresolved abstract type '{}' (run type resolution first)",
                v.name,
                v.ty.name()
            )));
            return;
        }
    }
    for r in &func.rets {
        if r.is_abstract() {
            errs.push(ctx(format!(
                "return type '{}' is unresolved (run type resolution first)",
                r.name()
            )));
            return;
        }
    }

    // rule 1: every value defined exactly once
    let mut defined = vec![0u32; func.values.len()];
    let mut define = |id: ValueId, errs: &mut Vec<String>| {
        let d = &mut defined[id.0 as usize];
        *d += 1;
        if *d == 2 {
            errs.push(ctx(format!(
                "value '{}' is defined more than once",
                func.value(id).name
            )));
        }
    };
    for &p in &func.params {
        define(p, errs);
    }
    for block in &func.blocks {
        for &p in &block.params {
            define(p, errs);
        }
        for inst in &block.insts {
            for dst in inst_dsts(inst) {
                define(dst, errs);
            }
        }
    }

    // rule 5: entry block has no parameters and is never a branch target
    if !func.blocks[0].params.is_empty() {
        errs.push(ctx("entry block must not have parameters".into()));
    }

    // rules 2 & 5: exactly one terminator, at the end; entry never targeted
    for block in &func.blocks {
        let bctx = |msg: String| ctx(format!("{}: {}", block.name, msg));
        match block.insts.last() {
            None => errs.push(bctx("block is empty; must end with a terminator".into())),
            Some(last) if !last.is_terminator() => {
                errs.push(bctx("block does not end with a terminator".into()))
            }
            _ => {}
        }
        for inst in block.insts.iter().rev().skip(1) {
            if inst.is_terminator() {
                errs.push(bctx("terminator in the middle of a block".into()));
            }
        }
        for inst in &block.insts {
            for (target, _) in branch_targets(inst) {
                if target.0 == 0 {
                    errs.push(bctx("branch targets the entry block".into()));
                }
            }
        }
    }

    // rules 3, 4, 6: per-instruction typing
    for block in &func.blocks {
        for inst in &block.insts {
            verify_inst(module, func, block, inst, errs);
        }
    }
}

fn inst_dsts(inst: &Inst) -> Vec<ValueId> {
    match inst {
        Inst::IConst { dst, .. }
        | Inst::Bin { dst, .. }
        | Inst::ICmp { dst, .. }
        | Inst::Cast { dst, .. }
        | Inst::Pack { dst, .. }
        | Inst::Get { dst, .. }
        | Inst::Set { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::PtrAdd { dst, .. } => vec![*dst],
        Inst::Call { dsts, .. } | Inst::Unpack { dsts, .. } => dsts.clone(),
        _ => vec![],
    }
}

fn branch_targets(inst: &Inst) -> Vec<(BlockId, &[ValueId])> {
    match inst {
        Inst::Jmp { target, args } => vec![(*target, args.as_slice())],
        Inst::Br {
            then_target,
            then_args,
            else_target,
            else_args,
            ..
        } => vec![
            (*then_target, then_args.as_slice()),
            (*else_target, else_args.as_slice()),
        ],
        _ => vec![],
    }
}

fn verify_inst(module: &Module, func: &Function, block: &Block, inst: &Inst, errs: &mut Vec<String>) {
    let ctx = |msg: String| format!("{}: {}: {}", func.name, block.name, msg);
    let name = |id: ValueId| func.value(id).name.clone();

    let tn = |t: Type| func.tyname(t);
    let is_memory = |t: Type| {
        t == Type::Ptr
            || ((t.is_int() || t.is_pack())
                && matches!(func.width(t), Some(8) | Some(16) | Some(32) | Some(64) | Some(128) | Some(192) | Some(256)))
    };

    match inst {
        Inst::IConst { dst, imm } => {
            let ty = func.ty(*dst);
            let ok = match ty {
                Type::Int { bits, .. } if bits < 64 => {
                    // either reading of the literal must fit: signed or unsigned
                    let lo = -(1i128 << (bits - 1));
                    let hi = 1i128 << bits;
                    (lo..hi).contains(imm)
                }
                // ptr constants are raw addresses (MMIO, fixed buffers) —
                // meaningful wherever ptr is an address-space index
                Type::Int { .. } | Type::Ptr => true,
                // a pack literal is its bit pattern
                Type::Pack(_) => match func.width(ty) {
                    // a 64-bit pack takes any 64-bit pattern (a negative i64
                    // is one); narrower and wide ones fit their width
                    Some(w) if w != 64 && w < 128 => (0..1i128 << w).contains(imm),
                    _ => true,
                },
                Type::AInt | Type::AUInt => unreachable!("rejected by rule 0"),
            };
            if !ok {
                errs.push(ctx(format!(
                    "iconst {} does not fit in type {}",
                    imm,
                    tn(ty)
                )));
            }
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let (td, tl, tr) = (func.ty(*dst), func.ty(*lhs), func.ty(*rhs));
            if !td.is_int() || tl != td || tr != td {
                errs.push(ctx(format!(
                    "{}: operands and result must share an integer type; got {}: {}, {}: {}, {}: {}",
                    op.name(),
                    name(*dst),
                    tn(td),
                    name(*lhs),
                    tn(tl),
                    name(*rhs),
                    tn(tr)
                )));
            }
        }
        Inst::ICmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let (td, tl, tr) = (func.ty(*dst), func.ty(*lhs), func.ty(*rhs));
            if td != Type::U1 {
                errs.push(ctx(format!(
                    "cmp.{} result {} must be u1, not {}",
                    cond.name(),
                    name(*dst),
                    tn(td)
                )));
            }
            if tl != tr || !(tl.is_int() || tl == Type::Ptr) {
                errs.push(ctx(format!(
                    "cmp.{}: operands must share an integer or ptr type; got {} and {}",
                    cond.name(),
                    tn(tl),
                    tn(tr)
                )));
            }
        }
        Inst::Cast { op, dst, src } => {
            let (td, ts) = (func.ty(*dst), func.ty(*src));
            let (wd, ws) = (func.width(td), func.width(ts));
            let ok = match op {
                CastOp::Conv => td.is_int() && ts.is_int(),
                CastOp::Cast => {
                    (td.is_int() || td.is_pack() || td == Type::Ptr)
                        && (ts.is_int() || ts.is_pack() || ts == Type::Ptr)
                        && wd == ws
                }
            };
            if !ok {
                let why = match op {
                    CastOp::Conv => "conv between integers converts the value; other conversions are library operations",
                    CastOp::Cast => "cast needs two types of the same width",
                };
                errs.push(ctx(format!(
                    "{} from {} to {} is not valid: {}",
                    op.name(),
                    tn(ts),
                    tn(td),
                    why
                )));
            }
        }
        Inst::Pack { dst, args } => {
            let td = func.ty(*dst);
            match func.pack(td) {
                None => errs.push(ctx(format!(
                    "pack result {} must have a pack type, not {}",
                    name(*dst),
                    tn(td)
                ))),
                Some(p) => {
                    let want: Vec<Type> = p.fields.iter().map(|(_, t)| *t).collect();
                    let got: Vec<Type> = args.iter().map(|&a| func.ty(a)).collect();
                    if want != got {
                        errs.push(ctx(format!(
                            "pack {}: argument types ({}) do not match its fields ({})",
                            p.name,
                            got.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", "),
                            want.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", ")
                        )));
                    }
                }
            }
        }
        Inst::Unpack { dsts, src } => {
            let ts = func.ty(*src);
            match func.pack(ts) {
                None => errs.push(ctx(format!(
                    "unpack of {} needs a pack, not {}",
                    name(*src),
                    tn(ts)
                ))),
                Some(p) => {
                    let want: Vec<Type> = p.fields.iter().map(|(_, t)| *t).collect();
                    let got: Vec<Type> = dsts.iter().map(|&d| func.ty(d)).collect();
                    if want != got {
                        errs.push(ctx(format!(
                            "unpack {}: result types ({}) do not match its fields ({})",
                            p.name,
                            got.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", "),
                            want.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", ")
                        )));
                    }
                }
            }
        }
        Inst::Get { dst, src, field } => {
            let ts = func.ty(*src);
            match func.field(ts, *field) {
                None => errs.push(ctx(format!(
                    "get: {} is {}, which has no field #{}",
                    name(*src),
                    tn(ts),
                    field
                ))),
                Some((_, ft)) if ft != func.ty(*dst) => errs.push(ctx(format!(
                    "get: field {} of {} is {}, but {} is {}",
                    func.fmt_field(*src, *field),
                    name(*src),
                    tn(ft),
                    name(*dst),
                    tn(func.ty(*dst))
                ))),
                _ => {}
            }
        }
        Inst::Set {
            dst,
            src,
            field,
            val,
        } => {
            let ts = func.ty(*src);
            match func.field(ts, *field) {
                None => errs.push(ctx(format!(
                    "set: {} is {}, which has no field #{}",
                    name(*src),
                    tn(ts),
                    field
                ))),
                Some((_, ft)) => {
                    if ft != func.ty(*val) {
                        errs.push(ctx(format!(
                            "set: field {} of {} is {}, but {} is {}",
                            func.fmt_field(*src, *field),
                            name(*src),
                            tn(ft),
                            name(*val),
                            tn(func.ty(*val))
                        )));
                    }
                    if func.ty(*dst) != ts {
                        errs.push(ctx(format!(
                            "set: result {} must be {} like {}, not {}",
                            name(*dst),
                            tn(ts),
                            name(*src),
                            tn(func.ty(*dst))
                        )));
                    }
                }
            }
        }
        Inst::Load { dst, addr } => {
            if func.ty(*addr) != Type::Ptr {
                errs.push(ctx(format!("load address {} must be ptr", name(*addr))));
            }
            if !is_memory(func.ty(*dst)) {
                errs.push(ctx(format!(
                    "load result must be ptr or an 8/16/32/64-bit integer or pack, not {}",
                    tn(func.ty(*dst))
                )));
            }
        }
        Inst::Store { val, addr } => {
            if func.ty(*addr) != Type::Ptr {
                errs.push(ctx(format!("store address {} must be ptr", name(*addr))));
            }
            if !is_memory(func.ty(*val)) {
                errs.push(ctx(format!(
                    "stored value must be ptr or an 8/16/32/64-bit integer or pack, not {}",
                    tn(func.ty(*val))
                )));
            }
        }
        Inst::PtrAdd { dst, base, off } => {
            if func.ty(*dst) != Type::Ptr
                || func.ty(*base) != Type::Ptr
                || !matches!(func.ty(*off), Type::I64 | Type::U64)
            {
                errs.push(ctx(
                    "ptradd requires result: ptr, base: ptr, offset: i64 or u64".into()
                ));
            }
        }
        Inst::Call { dsts, callee, args } => {
            if let Some(target) = module.func(callee) {
                let want: Vec<Type> = target.params.iter().map(|&p| target.ty(p)).collect();
                let got: Vec<Type> = args.iter().map(|&a| func.ty(a)).collect();
                if want != got {
                    errs.push(ctx(format!(
                        "call {}: argument types ({}) do not match parameters ({})",
                        callee,
                        got.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", "),
                        want.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", ")
                    )));
                }
                // results may bind all of the callee's return values or none
                if !dsts.is_empty() {
                    let dt: Vec<Type> = dsts.iter().map(|&d| func.ty(d)).collect();
                    if dt != target.rets {
                        errs.push(ctx(format!(
                            "call {}: result types ({}) do not match return types ({})",
                            callee,
                            dt.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", "),
                            target
                                .rets
                                .iter()
                                .map(|&t| tn(t))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                }
            }
        }
        Inst::Jmp { .. } | Inst::Br { .. } => {
            if let Inst::Br { cond, .. } = inst {
                if func.ty(*cond) != Type::U1 {
                    errs.push(ctx(format!(
                        "br condition {} must be u1, not {}",
                        name(*cond),
                        tn(func.ty(*cond))
                    )));
                }
            }
            for (target, args) in branch_targets(inst) {
                let tblock = &func.blocks[target.0 as usize];
                let want: Vec<Type> = tblock.params.iter().map(|&p| func.ty(p)).collect();
                let got: Vec<Type> = args.iter().map(|&a| func.ty(a)).collect();
                if want != got {
                    errs.push(ctx(format!(
                        "branch to {}: argument types ({}) do not match block parameters ({})",
                        tblock.name,
                        got.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", "),
                        want.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", ")
                    )));
                }
            }
        }
        Inst::Ret { vals } => {
            let got: Vec<Type> = vals.iter().map(|&v| func.ty(v)).collect();
            if got != func.rets {
                errs.push(ctx(format!(
                    "ret types ({}) do not match the function's return types ({})",
                    got.iter().map(|&t| tn(t)).collect::<Vec<_>>().join(", "),
                    func.rets
                        .iter()
                        .map(|&t| tn(t))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SUM: &str = r"
; sum of 0..n
fn sum(n: i64) -> i64 {
entry:
    zero: i64 = const 0
    jmp loop(zero, zero)
loop(i: i64, acc: i64):
    done: u1 = cmp.ge i, n
    br done, exit, body
body:
    acc2: i64 = add acc, i
    one: i64 = const 1
    i2: i64 = add i, one
    jmp loop(i2, acc2)
exit:
    ret acc
}
";

    #[test]
    fn round_trip() {
        let m1 = parse(SUM).expect("first parse");
        verify(&m1).expect("verify");
        let printed = m1.to_string();
        let m2 = parse(&printed).expect("reparse of printed output");
        assert_eq!(printed, m2.to_string());
    }

    #[test]
    fn parses_structure() {
        let m = parse(SUM).unwrap();
        let f = m.func("sum").unwrap();
        assert_eq!(f.params.len(), 1);
        assert_eq!(f.rets, vec![Type::I64]);
        assert_eq!(f.blocks.len(), 4);
        assert_eq!(f.blocks[1].params.len(), 2);
        assert!(matches!(f.blocks[1].insts.last(), Some(Inst::Br { .. })));
    }

    #[test]
    fn calls_memory_and_casts() {
        let src = r"
fn get(p: ptr) -> i32 {
entry:
    v: i32 = load p
    ret v
}
fn use(p: ptr) -> i64 {
entry:
    v: i32 = get(p)
    w: i64 = conv v
    eight: i64 = const 8
    q: ptr = ptradd p, eight
    store w, q
    touch(q)
    ret w
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        let printed = m.to_string();
        assert_eq!(printed, parse(&printed).unwrap().to_string());
    }

    #[test]
    fn rejects_type_mismatch() {
        let src = r"
fn bad(a: i64, b: i32) -> i64 {
entry:
    c: i64 = add a, b
    ret c
}
";
        let m = parse(src).expect("parse succeeds; verify should fail");
        let errs = verify(&m).unwrap_err();
        assert!(errs[0].contains("add"), "got: {:?}", errs);
    }

    #[test]
    fn rejects_branch_arg_mismatch() {
        let src = r"
fn bad(a: i64) -> i64 {
entry:
    jmp next(a)
next(x: i32):
    ret a
}
";
        let m = parse(src).expect("parse");
        let errs = verify(&m).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("branch to next")), "got: {:?}", errs);
    }

    #[test]
    fn rejects_missing_terminator() {
        let src = r"
fn bad(a: i64) -> i64 {
entry:
    b: i64 = add a, a
}
";
        let m = parse(src).expect("parse");
        let errs = verify(&m).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("terminator")), "got: {:?}", errs);
    }

    #[test]
    fn rejects_double_definition() {
        let src = r"
fn bad(a: i64) -> i64 {
entry:
    b: i64 = add a, a
    b: i64 = add a, a
    ret b
}
";
        assert!(parse(src).is_err());
    }

    #[test]
    fn rejects_undefined_value() {
        let src = r"
fn bad(a: i64) -> i64 {
entry:
    b: i64 = add a, nope
    ret b
}
";
        assert!(parse(src).is_err());
    }

    #[test]
    fn structured_lowers_and_verifies() {
        let src = r"
fn sum(n: i64) -> i64 {
    zero: i64 = const 0
    r: i64 = loop(i: i64 = zero, acc: i64 = zero) {
        done: u1 = cmp.ge i, n
        if done {
            break acc
        }
        one: i64 = const 1
        a2: i64 = add acc, i
        i2: i64 = add i, one
        continue i2, a2
    }
    ret r
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        // the printed (lowered) form must round-trip through the flat parser
        let printed = m.to_string();
        let m2 = parse(&printed).expect("reparse");
        verify(&m2).expect("reverify");
        assert_eq!(printed, m2.to_string());
        // loop header + exit exist: entry, header, then-arm, else-arm... >= 4 blocks
        assert!(m.funcs[0].blocks.len() >= 4);
    }

    #[test]
    fn structured_if_yield_types_checked() {
        let src = r"
fn bad(c: u1, a: i64, b: i32) -> i64 {
    r: i64 = if c {
        yield a
    } else {
        yield b
    }
    ret r
}
";
        let m = parse(src).expect("parse");
        assert!(verify(&m).is_err(), "i32 yield into i64 result must fail");
    }

    #[test]
    fn structured_error_cases() {
        // yield outside an if
        assert!(parse("fn f() {\n    yield\n}").is_err());
        // break outside a loop
        assert!(parse("fn f() {\n    break\n}").is_err());
        // loop body falling through
        assert!(parse(
            "fn f(n: i64) {\n    loop(i: i64 = n) {\n        z: i64 = const 0\n    }\n    ret\n}"
        )
        .is_err());
        // unreachable code after ret
        assert!(parse("fn f() {\n    ret\n    ret\n}").is_err());
        // value-yielding if without else
        assert!(parse(
            "fn f(c: u1, a: i64) -> i64 {\n    r: i64 = if c {\n        yield a\n    }\n    ret r\n}"
        )
        .is_err());
    }

    #[test]
    fn forward_value_reference_across_blocks() {
        // x is defined textually after its use; the prescan makes this fine
        // (dominance is the emitter's problem, per the spec).
        let src = r"
fn fwd(c: u1) -> i64 {
entry:
    br c, a, b
a:
    jmp join(x)
b:
    x: i64 = const 7
    jmp join(x)
join(r: i64):
    ret r
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
    }

    #[test]
    fn narrow_types_and_packs_round_trip() {
        let src = r"
type rgb = pack { r: u5, g: u6, b: u5 }
type pix = pack { c: rgb, a: u8 }

fn mk(r: u5, g: u6, b: u5) -> rgb {
entry:
    c: rgb = pack r, g, b
    ret c
}
fn green(c: rgb) -> u6 {
entry:
    g: u6 = get c, g
    ret g
}
fn fade(p: pix, a: u8) -> (pix, u16) {
entry:
    q: pix = set p, a, a
    c: rgb = get q, c
    w: u16 = cast c
    r: u5, g: u6, b: u5 = unpack c
    x: i5 = cast r
    y: i7 = conv x
    z: u3 = conv g
    ret q, w
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        let packs = &m.funcs[0].packs;
        assert_eq!(packs[0].width, 16);
        assert_eq!(packs[0].offsets, vec![0, 5, 11]);
        assert_eq!(packs[1].width, 24);
        let printed = m.to_string();
        let m2 = parse(&printed).expect("reparse");
        verify(&m2).expect("reverify");
        assert_eq!(printed, m2.to_string());
        assert!(printed.starts_with("type rgb = pack { r: u5, g: u6, b: u5 }\n"), "{}", printed);
    }

    #[test]
    fn rejects_bad_widths_and_fields() {
        assert!(parse("fn f(a: i0) {\n    ret\n}").is_err());
        assert!(parse("fn f(a: u257) {\n    ret\n}").is_err());
        assert!(parse("type p = pack { a: u200, b: u57 }\n").is_err()); // 257 bits
        assert!(parse("fn f(a: u65) {\n    ret\n}").is_ok()); // wide: lowered to words
        assert!(parse("type p = pack { a: u4 }\nfn f(p: p) -> u4 {\n    x: u4 = get p, nope\n    ret x\n}").is_err());
        // cast must preserve width; conv goes any way between integers, and
        // is a library operation as soon as a pack is involved
        let m = parse("fn f(a: i8) -> u16 {\n    b: u16 = cast a\n    ret b\n}").unwrap();
        assert!(verify(&m).is_err());
        let m = parse("fn f(a: i8) -> i8 {\n    b: i8 = conv a\n    ret b\n}").unwrap();
        assert!(verify(&m).is_ok());
        assert!(parse("type p = pack { a: u8 }\nfn f(a: i8) -> p {\n    b: p = conv a\n    ret b\n}").is_err());
        // memory only at 8/16/32/64
        let m = parse("fn f(p: ptr) -> u5 {\n    b: u5 = load p\n    ret b\n}").unwrap();
        assert!(verify(&m).is_err());
        let m = parse("fn f(p: ptr) -> u16 {\n    b: u16 = load p\n    ret b\n}").unwrap();
        assert!(verify(&m).is_ok());
        // icmp results are u1, and const must fit
        let m = parse("fn f(a: u5) -> u1 {\n    k: u5 = const 40\n    c: u1 = cmp.lt a, k\n    ret c\n}").unwrap();
        assert!(verify(&m).is_err());
        let m = parse("fn f(a: i5) -> u1 {\n    k: i5 = const -16\n    c: u1 = cmp.lt a, k\n    ret c\n}").unwrap();
        assert!(verify(&m).is_ok());
    }

    #[test]
    fn uint_resolves_with_int() {
        let mut m = parse("fn f(a: uint, b: int) -> uint {\n    c: uint = cast b\n    ret a\n}").unwrap();
        resolve_types(&mut m, &Policy::new(Type::I32).unwrap());
        verify(&m).expect("verify");
        assert_eq!(m.funcs[0].rets, vec![Type::int(false, 32)]);
    }

    #[test]
    fn parametric_types() {
        let src = r"
type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }
type f32 = float(8, 23)
type f16 = float(5, 10)
type bits(E, M) = u(E + M + 1)
type byte = u8
type word(N) = u(N)

fn exp32(f: f32) -> u8 {
    e: u8 = get f, exponent
    ret e
}
fn same(f: float(8, 23)) -> f32 {
    ret f
}
fn raw(f: f32) -> bits(8, 23) {
    r: bits(8, 23) = cast f
    ret r
}
fn half(f: f16, b: byte, w: word(2 * 6)) -> (u5, u8, u12) {
    e: u5 = get f, exponent
    ret e, b, w
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        // f32 and float(8, 23) are one type, named by the alias
        let f = m.func("same").unwrap();
        assert_eq!(f.ty(f.params[0]), f.rets[0]);
        assert_eq!(f.tyname(f.rets[0]), "f32");
        assert_eq!(m.funcs[0].packs.iter().filter(|p| p.width == 32).count(), 1);
        assert_eq!(m.func("raw").unwrap().rets[0], Type::int(false, 32));
        assert_eq!(m.func("half").unwrap().rets[2], Type::int(false, 12));
        // declarations print as written, and the whole thing round-trips
        let printed = m.to_string();
        assert!(printed.starts_with("type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }\ntype f32 = float(8, 23)\n"), "{}", printed);
        assert!(printed.contains("type bits(E, M) = u(E + M + 1)\n"), "{}", printed);
        assert!(printed.contains("fn half(f: f16, b: byte, w: word(2 * 6)) -> (u5, u8, u12)") || printed.contains("fn half(f: f16, b: u8, w: u12)"), "{}", printed);
        let m2 = parse(&printed).expect("reparse");
        verify(&m2).expect("reverify");
        assert_eq!(printed, m2.to_string());
    }

    #[test]
    fn parametric_type_errors() {
        // wrong arity, bad width, unknown parameter, self-reference
        assert!(parse("type w(N) = u(N)\nfn f(a: w) {\n    ret\n}").is_err());
        assert!(parse("type w(N) = u(N)\nfn f(a: w(300)) {\n    ret\n}").is_err());
        assert!(parse("type w(N) = u(N)\nfn f(a: w(70)) {\n    ret\n}").is_ok()); // wide
        assert!(parse("type w(N) = u(M)\n").is_err());
        assert!(parse("type loop(N) = pack { a: loop(N) }\nfn f(a: loop(1)) {\n    ret\n}").is_err());
        assert!(parse("type big = pack { a: u(200), b: u(57) }\n").is_err()); // 257 bits
        // a block label whose params use parenthesized types still parses
        let m = parse("fn f(n: u(8)) -> u8 {\nentry:\n    jmp next(n)\nnext(x: u(4 + 4)):\n    ret x\n}").expect("parse");
        verify(&m).expect("verify");
    }

    #[test]
    fn generic_functions_instantiate() {
        let src = r"
type word(N) = u(N)
fn wrap(N)(a: word(N), b: word(N)) -> word(N) {
    s: word(N) = add a, b
    top: word(N) = const (1 << (N - 1)) - 1
    big: u1 = cmp.gt s, top
    r: word(N) = if big {
        d: word(N) = halve(N)(s)
        yield d
    } else {
        yield s
    }
    ret r
}
fn halve(N)(a: word(N)) -> word(N) {
    one: word(N) = const 1
    h: word(N) = shr a, one
    ret h
}
fn wrap8 = wrap(8)
fn use12(a: u12) -> u12 {
    r: u12 = wrap(4 * 3)(a, a)
    ret r
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        let names: Vec<&str> = m.funcs.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"wrap8"), "{:?}", names);
        assert!(names.contains(&"wrap_12"), "{:?}", names);
        assert!(names.contains(&"halve_8") && names.contains(&"halve_12"), "{:?}", names);
        assert!(!names.iter().any(|n| *n == "wrap" || *n == "halve"), "{:?}", names);
        let w8 = m.func("wrap8").unwrap();
        assert_eq!(w8.rets, vec![Type::int(false, 8)]);
        // the lowered module is plain and round-trips
        let printed = m.to_string();
        let m2 = parse(&printed).expect("reparse");
        verify(&m2).expect("reverify");
        assert_eq!(printed, m2.to_string());
        // errors: unknown generic, arity, an alias that clashes
        assert!(parse("fn f = nope(1)\n").is_err());
        assert!(parse("fn g(N)(a: u(N)) -> u(N) {\n    ret a\n}\nfn h = g(1, 2)\n").is_err());
    }

    #[test]
    fn literal_operands_take_the_type_from_context() {
        let src = r"
type rgb = pack { r: u5, g: u6, b: u5 }
fn f(a: i64, p: ptr, c: rgb) -> i64 {
entry:
    b: i64 = add a, 1
    lt: u1 = cmp.lt 0, b
    q: ptr = ptradd p, 8
    d: rgb = set c, g, 63
    e: rgb = pack 1, 2, 3
    x: u8 = conv 200: u8
    m: i64 = g(b, 2)
    jmp next(m, 7)
next(v: i64, w: i64):
    br lt, done(v), done(0)
done(z: i64):
    ret z
}
fn g(a: i64, b: i64) -> i64 {
    r: i64 = mul a, b
    ret r
}
fn h(n: i64) -> i64 {
    r: i64 = loop(i: i64 = 0, acc: i64 = 0) {
        done: u1 = cmp.ge i, n
        if done {
            break acc
        }
        acc2: i64 = add acc, i
        i2: i64 = add i, 1
        continue i2, acc2
    }
    nz: u1 = cmp.ne r, 0
    k: i64 = if nz {
        yield 1
    } else {
        yield 0
    }
    ret k
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        // the printed form shows the literals inline and round-trips
        let printed = m.to_string();
        assert!(printed.contains("b: i64 = add a, 1\n"), "{}", printed);
        assert!(printed.contains("jmp next(m, 7)\n"), "{}", printed);
        assert!(printed.contains("x: u8 = conv 200: u8\n"), "{}", printed);
        assert!(!printed.contains('#'), "hidden values must not print: {}", printed);
        let m2 = parse(&printed).expect("reparse");
        verify(&m2).expect("reverify");
        assert_eq!(printed, m2.to_string());
        // a literal with nothing to type it is an error, and a float
        // literal needs a float
        assert!(parse("fn f(p: ptr) {\n    store 1, p\n    ret\n}").is_err());
        assert!(parse("fn f(a: i64) -> i64 {\n    b: i64 = add a, 1.5\n    ret b\n}").is_err());
        let m = parse("fn f(a: i64) -> i64 {\n    b: i64 = add a, 1\n    ret b\n}").unwrap();
        verify(&m).expect("verify");
    }

    #[test]
    fn decimal_literals_round_correctly() {
        // Rust's from_str is correctly rounded: the oracle for f32 and f64
        let texts = [
            "0.1", "0.2", "0.3", "1.5", "2", "3.14159265358979", "1e-40", "1e-45", "1.4e-45", "7e-46", "1e38", "3.4028235e38",
            "3.4028236e38", "1e39", "16777217", "16777219", "0.000000059604645", "5e-324", "2.5e-324", "1e-323", "1.7976931348623157e308",
            "1.8e308", "123456789012345678901234567890", "0.1000000000000000055511151231257827", "-0.0", "-2.5", "9007199254740993",
            "1.00000011920928955078125", "1.000000178813934326171875", "8388609", "4.9e-324", "2.2250738585072014e-308", "2.2250738585072009e-308",
        ];
        for t in texts {
            let want32 = t.parse::<f32>().unwrap().to_bits() as u64;
            let want64 = t.parse::<f64>().unwrap().to_bits();
            assert_eq!(decimal_to_float(t, 8, 23).unwrap(), want32, "f32 {}", t);
            assert_eq!(decimal_to_float(t, 11, 52).unwrap(), want64, "f64 {}", t);
        }
        // and a float type reads them as values, an integer type as bits
        let src = r"
type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }
type f32 = float(8, 23)
fn f() -> (f32, f32, f32, f32, u32) {
    a: f32 = const 0.1
    b: f32 = const -inf
    c: f32 = const nan
    d: f32 = const 3
    e: u32 = const 0x3dcccccd
    ret a, b, c, d, e
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
        let f = &m.funcs[0];
        let consts: Vec<i64> = f.blocks[0].insts.iter().filter_map(|i| match i {
            Inst::IConst { imm, .. } => Some(*imm as i64),
            _ => None,
        }).collect();
        assert_eq!(consts, vec![0x3dcccccd, 0xff800000, 0x7fc00000, 0x40400000, 0x3dcccccd]);
        let printed = m.to_string();
        assert!(printed.contains("a: f32 = const 0.1\n"), "{}", printed);
        assert!(printed.contains("b: f32 = const -inf\n"), "{}", printed);
        assert!(printed.contains("d: f32 = const 3.0\n"), "{}", printed);
        assert_eq!(printed, parse(&printed).unwrap().to_string());
    }

    #[test]
    fn bare_float_follows_the_policy() {
        let src = r"
type float(E, M) = pack { mantissa: u(M), exponent: u(E), sign: u1 }
fn add(E, M)(a: float(E, M), b: float(E, M)) -> float(E, M) {
    ret a
}
fn twice(x: float) -> float {
    r: float = add x, x
    ret r
}
";
        let m = parse_with(src, &Policy::new(Type::I64).unwrap()).expect("parse");
        verify(&m).expect("verify");
        let f = m.func("twice").unwrap();
        assert_eq!(f.tyname(f.rets[0]), "float(11, 52)");
        let m = parse_with(src, &Policy::new(Type::I32).unwrap()).expect("parse");
        assert_eq!(m.func("twice").unwrap().tyname(m.func("twice").unwrap().rets[0]), "float(8, 23)");
        let m = parse_with(src, &Policy::new(Type::I64).unwrap().with_float(5, 10)).expect("parse");
        let f = m.func("twice").unwrap();
        assert_eq!(f.tyname(f.rets[0]), "float(5, 10)");
        assert!(m.funcs.iter().any(|f| f.name == "add_5_10")); // this source's own add(E, M)
        // a parametric type with no policy default still needs its arguments
        assert!(parse("type w(N) = u(N)\nfn f(a: w) {\n    ret\n}").is_err());
        assert_eq!(Policy::float_from_arg("bf16"), Some((8, 7)));
        assert_eq!(Policy::float_from_arg("4,3"), Some((4, 3)));
    }
}
