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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Type {
    /// signed integer, 1..=64 bits. Signedness lives in the TYPE: `div`,
    /// `rem`, `shr`, ordered `icmp`, `ext`, `itof`/`ftoi` all take their
    /// behavior from operand types, so there is one opcode per operation.
    /// Odd widths lower to sign-extended values in 64-bit containers.
    I(u8),
    /// unsigned integer, 1..=64 bits; `u1` is the boolean (icmp results).
    /// Odd widths lower to zero-extended values in 64-bit containers.
    U(u8),
    Ptr,
    F32,
    F64,
    /// abstract integers/float: resolved to a concrete type by the
    /// target's replacement policy before verification
    Int,
    Uint,
    Float,
    /// a small IEEE-style float: 1 sign bit, `e` exponent bits, `m`
    /// mantissa bits. float(8, 23) IS F32 and float(11, 52) IS F64
    /// (canonicalized at parse); other instances — float(5, 10) = fp16,
    /// float(8, 7) = bf16, float(4, 3) = fp8 e4m3 — are storage formats
    /// whose arithmetic lowers to promote -> f64 op -> demote (correctly
    /// rounded for m <= 24), with conversions from the width-generic
    /// lib/float.ssa. Lowered away before emission.
    FP(u8, u8),
    /// abstract half-word integers: resolved to HALF the `int` policy's
    /// width (i32/u32 under int=i64, i16/u16 under int=i32). The point is
    /// stating a width *relationship*: a struct of `half` fields with
    /// `int` intermediates keeps "intermediates are twice the fields"
    /// true under every policy — the invariant exact-rational arithmetic
    /// (and fixed-point, and anything product-shaped) rests on.
    Half,
    UHalf,
    /// abstract scalar — parent of float, rational, (future) fixed-point.
    /// Resolved by policy either to a concrete float (a substitution, like
    /// `float`) or to the `$rat` struct (a rewrite of its float-opcode
    /// operations into rational-library calls — see scalar.rs).
    Scalar,
    /// packed bitfield struct; index into the module's struct table
    /// (each Function carries a copy). Total width <= 64; lowered to its
    /// carrier integer before emission.
    Struct(u16),
    /// short vector: `i16x4`, `f32x2`, `u8x8` — lanes x element, lane 0
    /// in the low bits, total width <= 64 for now (the SIMD tier lifts
    /// this). Elementwise arithmetic uses the ordinary opcodes — the type
    /// alone makes it a vector op — and extract/insert/pack take a lane
    /// index where structs take a field name. Lowered to a packed struct
    /// (then to its carrier integer) before anything downstream looks.
    Vec(u8, VecElem),
}

/// A vector's element type: the scalar core types that fit in lanes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VecElem {
    I(u8),
    U(u8),
    F32,
}

impl VecElem {
    pub fn ty(self) -> Type {
        match self {
            VecElem::I(n) => Type::I(n),
            VecElem::U(n) => Type::U(n),
            VecElem::F32 => Type::F32,
        }
    }

    pub fn bits(self) -> u32 {
        match self {
            VecElem::I(n) | VecElem::U(n) => n as u32,
            VecElem::F32 => 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructDef {
    pub name: String,
    /// declared low-first: the first field occupies the LOW bits — the
    /// same convention as vector lanes (lane 0 low) and C layout (first
    /// field at offset 0, viewed little-endian)
    pub fields: Vec<(String, Type)>,
}

impl StructDef {
    pub fn total_bits(&self) -> u32 {
        self.fields.iter().filter_map(|(_, t)| t.width()).sum()
    }

    /// C-like multi-word layout: fields pack low-first and never straddle
    /// a 64-bit word (a field that would cross starts the next word).
    /// Returns (word count, per-field (word index, bit offset in word)).
    pub fn word_layout(&self) -> (u32, Vec<(u32, u32)>) {
        let mut pos = 0u32;
        let mut map = Vec::with_capacity(self.fields.len());
        for (_, t) in &self.fields {
            let w = t.width().unwrap_or(0);
            if pos % 64 + w > 64 {
                pos = (pos / 64 + 1) * 64;
            }
            map.push((pos / 64, pos % 64));
            pos += w;
        }
        (pos.div_ceil(64).max(1), map)
    }

    /// LSB offset of field i (sum of the widths of earlier fields)
    pub fn offset(&self, i: usize) -> u32 {
        self.fields[..i]
            .iter()
            .filter_map(|(_, t)| t.width())
            .sum()
    }
}

impl Type {
    pub fn name(self) -> String {
        match self {
            Type::I(n) => format!("i{}", n),
            Type::U(n) => format!("u{}", n),
            Type::Ptr => "ptr".into(),
            Type::Int => "int".into(),
            Type::Uint => "uint".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Float => "float".into(),
            Type::Scalar => "scalar".into(),
            Type::Half => "half".into(),
            Type::UHalf => "uhalf".into(),
            Type::Struct(_) => "$struct".into(), // callers with a table print the name
            Type::Vec(n, e) => format!("{}x{}", e.ty().name(), n),
            Type::FP(e, m) => format!("float({}, {})", e, m),
        }
    }

    /// public alias for CLI flag parsing
    pub fn from_name_pub(s: &str) -> Option<Type> {
        Type::from_name(s)
    }

    fn from_name(s: &str) -> Option<Type> {
        match s {
            "ptr" => Some(Type::Ptr),
            "int" => Some(Type::Int),
            "uint" => Some(Type::Uint),
            "f32" => Some(Type::F32),
            "f64" => Some(Type::F64),
            "float" => Some(Type::Float),
            "scalar" => Some(Type::Scalar),
            "half" => Some(Type::Half),
            "uhalf" => Some(Type::UHalf),
            _ => {
                // vector names: <elem>x<lanes>, e.g. i16x4, f32x2, u8x8
                if let Some((el, ln)) = s.rsplit_once('x') {
                    if let (Some(elem), Ok(lanes)) = (Type::from_name(el), ln.parse::<u8>()) {
                        let elem = match elem {
                            Type::I(n) => Some(VecElem::I(n)),
                            Type::U(n) => Some(VecElem::U(n)),
                            Type::F32 => Some(VecElem::F32),
                            _ => None,
                        };
                        if let Some(elem) = elem {
                            if lanes >= 2
                                && !ln.starts_with('0')
                                && lanes as u32 * elem.bits() <= 64
                            {
                                return Some(Type::Vec(lanes, elem));
                            }
                        }
                        return None;
                    }
                }
                let (ctor, rest): (fn(u8) -> Type, &str) =
                    if let Some(r) = s.strip_prefix('i') {
                        (Type::I, r)
                    } else if let Some(r) = s.strip_prefix('u') {
                        (Type::U, r)
                    } else {
                        return None;
                    };
                let n: u8 = rest.parse().ok()?;
                // reject a leading zero or empty ("i", "i0", "i07")
                if (1..=64).contains(&n) && !rest.starts_with('0') {
                    Some(ctor(n))
                } else {
                    None
                }
            }
        }
    }

    /// Integer bit width, for ext/trunc rules; ptr and floats take no
    /// part in width changes.
    pub fn width(self) -> Option<u32> {
        match self {
            Type::I(n) | Type::U(n) => Some(n as u32),
            _ => None,
        }
    }

    /// Signedness — the property the opcodes read.
    pub fn is_signed(self) -> bool {
        matches!(self, Type::I(_) | Type::Int | Type::Half)
    }

    fn is_arith(self) -> bool {
        matches!(self, Type::I(n) | Type::U(n) if n >= 2)
    }

    /// A vector's element type, or the type itself: what the elementwise
    /// op-class rules look at.
    fn elem_or_self(self) -> Type {
        match self {
            Type::Vec(_, e) => e.ty(),
            t => t,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Type::F32 | Type::F64 | Type::FP(..))
    }

    /// Register-class choice for allocation, staging, and calls: floats
    /// and vectors travel in the d registers (our vectors are <= 64 bits,
    /// exactly one d register). Type rules never use this.
    pub fn uses_float_reg(self) -> bool {
        matches!(self, Type::F32 | Type::F64 | Type::Vec(..))
    }

    fn is_memory(self) -> bool {
        matches!(
            self,
            Type::I(32) | Type::U(32) | Type::I(64) | Type::U(64) | Type::Ptr | Type::F32 | Type::F64
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    FAdd,
    FSub,
    FMul,
    FDiv,
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
    ("fadd", BinOp::FAdd),
    ("fsub", BinOp::FSub),
    ("fmul", BinOp::FMul),
    ("fdiv", BinOp::FDiv),
    ("iadd", BinOp::IAdd),
    ("isub", BinOp::ISub),
    ("imul", BinOp::IMul),
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
        // one name per operation; the types make it unambiguous
        match self {
            BinOp::IAdd | BinOp::FAdd => "add",
            BinOp::ISub | BinOp::FSub => "sub",
            BinOp::IMul | BinOp::FMul => "mul",
            BinOp::Div | BinOp::FDiv => "div",
            BinOp::Rem => "rem",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Xor => "xor",
            BinOp::Shl => "shl",
            BinOp::Shr => "shr",
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv)
    }
}

/// Ordered float comparisons (false when either operand is NaN), plus
/// `une` (true on NaN) as the negation of `oeq`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FCond {
    Oeq,
    Une,
    Olt,
    Ole,
    Ogt,
    Oge,
}

const FCONDS: &[(&str, FCond)] = &[
    ("oeq", FCond::Oeq),
    ("une", FCond::Une),
    ("olt", FCond::Olt),
    ("ole", FCond::Ole),
    ("ogt", FCond::Ogt),
    ("oge", FCond::Oge),
];

impl FCond {
    pub fn name(self) -> &'static str {
        FCONDS.iter().find(|(_, c)| *c == self).unwrap().0
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
    /// widen; the fill follows the SOURCE's signedness
    Ext,
    Trunc,
    /// int -> float; signedness from the int side's type
    Itof,
    /// float -> int, rounding toward zero; signedness from the int side
    Ftoi,
    Fpromote,
    Fdemote,
    /// same-width reinterpretation: between int signedness, int<->float,
    /// int<->struct
    Bitcast,
}

impl CastOp {
    pub fn name(self) -> &'static str {
        match self {
            CastOp::Ext => "ext",
            CastOp::Trunc => "trunc",
            CastOp::Itof => "itof",
            CastOp::Ftoi => "ftoi",
            CastOp::Fpromote => "fpromote",
            CastOp::Fdemote => "fdemote",
            CastOp::Bitcast => "bitcast",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Inst {
    IConst {
        dst: ValueId,
        imm: i64,
    },
    /// float constant; `bits` is always the f64 bit pattern of the value —
    /// an f32 destination narrows at emission, so abstract `float` needs no
    /// constant rewriting at resolution time
    FConst {
        dst: ValueId,
        bits: u64,
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
    FCmp {
        cond: FCond,
        dst: ValueId,
        lhs: ValueId,
        rhs: ValueId,
    },
    Cast {
        op: CastOp,
        dst: ValueId,
        src: ValueId,
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
    /// read one bitfield out of a struct value
    Extract {
        dst: ValueId,
        src: ValueId,
        field: u16,
    },
    /// build a struct value from all its fields, in declaration order
    Pack {
        dst: ValueId,
        args: Vec<ValueId>,
    },
    /// a copy of `src` with one field replaced
    Insert {
        dst: ValueId,
        src: ValueId,
        field: u16,
        val: ValueId,
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
    pub structs: std::rc::Rc<Vec<StructDef>>,
}

impl Function {
    pub fn value(&self, id: ValueId) -> &ValueData {
        &self.values[id.0 as usize]
    }

    pub fn ty(&self, id: ValueId) -> Type {
        self.value(id).ty
    }
}

#[derive(Clone, Debug, Default)]
pub struct Module {
    pub funcs: Vec<Function>,
    pub structs: std::rc::Rc<Vec<StructDef>>,
}

impl Module {
    pub fn func(&self, name: &str) -> Option<&Function> {
        self.funcs.iter().find(|f| f.name == name)
    }
}

/// The replacement policy for abstract numeric types: what `int` becomes
/// on this compilation. Targets supply defaults (their natural width, or a
/// size-oriented choice); the user can override. `float` joins when
/// concrete floats do.
#[derive(Clone, Copy)]
pub struct Policy {
    pub int: Type,
    pub uint: Type,
    pub float: Type,
    pub scalar: ScalarPolicy,
}

/// What the abstract `scalar` type becomes: a concrete float (pure type
/// substitution) or the rational library's `$rat` struct (scalar.rs then
/// rewrites the float opcodes into calls).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScalarPolicy {
    Float(Type),
    Rat,
}

impl Policy {
    pub fn new(int: Type, float: Type) -> Result<Policy, String> {
        if !matches!(int, Type::I(32) | Type::I(64)) {
            return Err(format!("'int' cannot resolve to {}", int.name()));
        }
        if !matches!(float, Type::F32 | Type::F64) {
            return Err(format!("'float' cannot resolve to {}", float.name()));
        }
        // 'uint' follows 'int' at the same width
        let uint = match int {
            Type::I(n) => Type::U(n),
            _ => unreachable!(),
        };
        // 'scalar' follows 'float' unless overridden
        Ok(Policy {
            int,
            uint,
            float,
            scalar: ScalarPolicy::Float(float),
        })
    }
}

/// A scalar crossing a vector lane boundary: the element type itself, or
/// the canonical forms width-lowering leaves behind — the 64-bit container
/// for sub-32-bit integer lanes, and the u32 bit pattern for f32 lanes.
fn lane_scalar_ok(elem: VecElem, t: Type) -> bool {
    if t == elem.ty() {
        return true;
    }
    match elem {
        VecElem::I(n) if n < 32 => t == Type::I(64),
        VecElem::U(n) if n < 32 => t == Type::U(64),
        VecElem::F32 => t == Type::U(32),
        _ => false,
    }
}

/// Resolve abstract types to concrete ones. Because types live on values,
/// not opcodes, this is one sweep over the value tables and signatures —
/// no instruction ever changes.
pub fn resolve_types(module: &mut Module, policy: &Policy) {
    let hw = match policy.int {
        Type::I(n) => n / 2,
        _ => 32,
    };
    // struct fields resolve too (layouts may be policy-parametric); the
    // shared table is rebuilt and re-distributed
    if module.structs.iter().any(|d| {
        d.fields
            .iter()
            .any(|(_, t)| matches!(t, Type::Int | Type::Uint | Type::Half | Type::UHalf))
    }) {
        let mut defs = (*module.structs).clone();
        for d in &mut defs {
            for (_, t) in &mut d.fields {
                match *t {
                    Type::Int => *t = policy.int,
                    Type::Uint => *t = policy.uint,
                    Type::Half => *t = Type::I(hw),
                    Type::UHalf => *t = Type::U(hw),
                    _ => {}
                }
            }
        }
        let rc = std::rc::Rc::new(defs);
        module.structs = rc.clone();
        for f in &mut module.funcs {
            f.structs = rc.clone();
        }
    }
    // scalar -> $rat needs the rational library's struct in the module's
    // table (the load paths link it in textually); if it is absent, Scalar
    // stays unresolved and rule 0 reports it honestly
    let scalar = match policy.scalar {
        ScalarPolicy::Float(t) => Some(t),
        ScalarPolicy::Rat => module
            .structs
            .iter()
            .position(|d| d.name == "rat")
            .map(|i| Type::Struct(i as u16)),
    };
    for func in &mut module.funcs {
        let subst = |t: &mut Type| match *t {
            Type::Int => *t = policy.int,
            Type::Uint => *t = policy.uint,
            Type::Float => *t = policy.float,
            Type::Half => *t = Type::I(hw),
            Type::UHalf => *t = Type::U(hw),
            Type::Scalar => {
                if let Some(sc) = scalar {
                    *t = sc
                }
            }
            _ => {}
        };
        for v in &mut func.values {
            subst(&mut v.ty);
        }
        for r in &mut func.rets {
            subst(r);
        }
    }
}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Newline,
    Ident(String),  // fn, ret, iadd, icmp.slt, i64, ...
    Value(String),  // %x
    Block(String),  // ^x
    Global(String), // @x
    TyName(String), // $x
    Int(i64),
    FloatLit(f64),
    Colon,
    Comma,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Arrow,
    Equals,
    /// expression operator: + - * / % & | ^ << >>
    Op(&'static str),
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tok::Op(o) => write!(f, "'{}'", o),
            Tok::Newline => write!(f, "end of line"),
            Tok::Ident(s) => write!(f, "'{}'", s),
            Tok::Value(s) => write!(f, "'%{}'", s),
            Tok::Block(s) => write!(f, "'^{}'", s),
            Tok::Global(s) => write!(f, "'@{}'", s),
            Tok::TyName(s) => write!(f, "'${}'", s),
            Tok::Int(n) => write!(f, "'{}'", n),
            Tok::FloatLit(x) => write!(f, "'{:?}'", x),
            Tok::Colon => write!(f, "':'"),
            Tok::Comma => write!(f, "','"),
            Tok::LParen => write!(f, "'('"),
            Tok::RParen => write!(f, "')'"),
            Tok::LBrace => write!(f, "'{{'"),
            Tok::RBrace => write!(f, "'}}'"),
            Tok::Arrow => write!(f, "'->'"),
            Tok::Equals => write!(f, "'='"),
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
                chars.next();
                let name = lex_name(&mut chars);
                if name.is_empty() {
                    // a bare sigil with no name is an expression operator
                    match c {
                        '%' => toks.push((Tok::Op("%"), line)),
                        '^' => toks.push((Tok::Op("^"), line)),
                        _ => return Err(err(line, format!("expected a name after '{}'", c))),
                    }
                    continue;
                }
                toks.push((
                    match c {
                        '%' => Tok::Value(name),
                        '^' => Tok::Block(name),
                        '@' => Tok::Global(name),
                        _ => Tok::TyName(name),
                    },
                    line,
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
                if chars.peek() == Some(&'=') {
                    chars.next();
                    toks.push((Tok::Op("=="), line));
                } else {
                    toks.push((Tok::Equals, line));
                }
            }
            '!' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    toks.push((Tok::Op("!="), line));
                } else {
                    return Err(err(line, "expected '!='".into()));
                }
            }
            '-' => {
                chars.next();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    toks.push((Tok::Arrow, line));
                } else if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    match lex_number(&mut chars).map_err(|m| err(line, m))? {
                        Tok::Int(n) => toks.push((Tok::Int(n.wrapping_neg()), line)),
                        Tok::FloatLit(x) => toks.push((Tok::FloatLit(-x), line)),
                        _ => unreachable!(),
                    }
                } else {
                    toks.push((Tok::Op("-"), line));
                }
            }
            '+' => {
                chars.next();
                toks.push((Tok::Op("+"), line));
            }
            '*' => {
                chars.next();
                toks.push((Tok::Op("*"), line));
            }
            '/' => {
                chars.next();
                toks.push((Tok::Op("/"), line));
            }
            '&' => {
                chars.next();
                toks.push((Tok::Op("&"), line));
            }
            '|' => {
                chars.next();
                toks.push((Tok::Op("|"), line));
            }
            '<' => {
                chars.next();
                match chars.peek() {
                    Some(&'<') => {
                        chars.next();
                        toks.push((Tok::Op("<<"), line));
                    }
                    Some(&'=') => {
                        chars.next();
                        toks.push((Tok::Op("<="), line));
                    }
                    _ => toks.push((Tok::Op("<"), line)),
                }
            }
            '>' => {
                chars.next();
                match chars.peek() {
                    Some(&'>') => {
                        chars.next();
                        toks.push((Tok::Op(">>"), line));
                    }
                    Some(&'=') => {
                        chars.next();
                        toks.push((Tok::Op(">="), line));
                    }
                    _ => toks.push((Tok::Op(">"), line)),
                }
            }
            '0'..='9' => {
                let t = lex_number(&mut chars).map_err(|m| err(line, m))?;
                toks.push((t, line));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = lex_name(&mut chars);
                // opcode suffixes like icmp.slt lex as one identifier
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

/// integer, or float when a '.' / exponent follows (hex stays integer)
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
    let is_hex = s.starts_with("0x") || s.starts_with("0X");
    let mut float = false;
    if !is_hex {
        if chars.peek() == Some(&'.') {
            float = true;
            s.push('.');
            chars.next();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
        }
        // exponent: "1e10", "2.5e-3" ("e" already consumed into s if no dot)
        if s.ends_with(['e', 'E']) || (float && matches!(chars.peek(), Some(&'e') | Some(&'E'))) {
            float = true;
            if !s.ends_with(['e', 'E']) {
                s.push('e');
                chars.next();
            }
            if matches!(chars.peek(), Some(&'+') | Some(&'-')) {
                s.push(*chars.peek().unwrap());
                chars.next();
            }
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
        }
    }
    if float {
        return s
            .parse::<f64>()
            .map(Tok::FloatLit)
            .map_err(|_| format!("bad float literal '{}'", s));
    }
    lex_int_str(&s).map(Tok::Int)
}

fn lex_int_str(s: &str) -> Result<i64, String> {
    // parse as u64 so full-width bit patterns (and i64::MIN's magnitude
    // before negation) are representable; iconst semantics are bit-level
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

pub fn parse(src: &str) -> Result<Module, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        structs: Vec::new(),
        pending: Vec::new(),
        nlit: 0,
        sigs: HashMap::new(),
        gfns: HashMap::new(),
        gstructs: HashMap::new(),
        wenv: HashMap::new(),
        inst_name: None,
        struct_insts: HashMap::new(),
        struct_inst_rev: HashMap::new(),
        fn_insts: HashMap::new(),
        fn_worklist: Vec::new(),
    };
    let mut module = Module::default();
    // phase 1: type declarations and function signatures, so call sugar
    // in any body knows every callee's types regardless of order
    p.skip_newlines();
    while !p.at_end() {
        if matches!(p.peek(), Some(Tok::Ident(k)) if k == "type") {
            p.parse_type_decl()?;
        } else {
            p.prescan_signature()?;
        }
        p.skip_newlines();
    }
    p.pos = 0;
    // phase 2: bodies (type declarations were fully handled in phase 1;
    // width-generic templates are skipped — instances parse on demand)
    p.skip_newlines();
    while !p.at_end() {
        if matches!(p.peek(), Some(Tok::Ident(k)) if k == "type") {
            p.skip_line();
        } else if matches!(p.toks.get(p.pos + 1).map(|(t, _)| t), Some(Tok::Global(n) | Tok::Ident(n)) if p.gfns.contains_key(n))
        {
            let (_, hi) = p.function_range()?;
            p.pos = hi + 1;
        } else {
            module.funcs.push(p.parse_function()?);
        }
        p.skip_newlines();
    }
    // monomorphization: parse queued instances (which may queue more)
    let mut spins = 0;
    while let Some((gname, vals, mangled)) = p.fn_worklist.pop() {
        spins += 1;
        if spins > 4096 {
            return Err(ParseError {
                line: 0,
                msg: "instantiation explosion (over 4096 width-generic instances)".into(),
            });
        }
        let g = &p.gfns[&gname];
        let (lo, params) = (g.lo, g.params.clone());
        let save_env = std::mem::take(&mut p.wenv);
        for (par, &v) in params.iter().zip(&vals) {
            p.wenv.insert(par.clone(), v);
        }
        p.inst_name = Some(mangled.clone());
        let save_pos = p.pos;
        p.pos = lo;
        let f = p.parse_function()?;
        module.funcs.push(f);
        p.pos = save_pos;
        p.wenv = save_env;
        p.inst_name = None;
        p.fn_insts.insert(mangled, true);
    }
    let rc = std::rc::Rc::new(p.structs);
    module.structs = rc.clone();
    for f in &mut module.funcs {
        f.structs = rc.clone();
    }
    Ok(module)
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    structs: Vec<StructDef>,
    /// constant/temp definitions synthesized by literal-operand and
    /// expression sugar; drained ahead of the instruction that uses them
    pending: Vec<Inst>,
    nlit: u32,
    /// every function signature in the module, collected up front so
    /// call sugar (literal args, call-in-expression) knows callee types
    sigs: HashMap<String, (Vec<Type>, Vec<Type>)>,
    /// width-generic templates and their instantiation state
    gfns: HashMap<String, GenericFn>,
    gstructs: HashMap<String, GenericStruct>,
    wenv: HashMap<String, i64>,
    inst_name: Option<String>,
    struct_insts: HashMap<(String, Vec<i64>), u16>,
    /// instantiated struct id -> (generic name, argument values), for
    /// solving parameters from argument types at call sites
    struct_inst_rev: HashMap<u16, (String, Vec<i64>)>,
    fn_insts: HashMap<String, bool>, // mangled -> body parsed yet
    fn_worklist: Vec<(String, Vec<i64>, String)>,
}

/// a width expression inside a parametric type: `i(N)`, `u(2*N)`,
/// `$fp(E+1, M)` — literals, parameters, + - * /, parens
#[derive(Clone, Debug)]
enum WExpr {
    Lit(i64),
    Par(String),
    Bin(char, Box<WExpr>, Box<WExpr>),
}

impl WExpr {
    fn eval(&self, env: &HashMap<String, i64>) -> Result<i64, String> {
        match self {
            WExpr::Lit(n) => Ok(*n),
            WExpr::Par(p) => env
                .get(p)
                .copied()
                .ok_or_else(|| format!("unbound width parameter '{}'", p)),
            WExpr::Bin(op, l, r) => {
                let (a, b) = (l.eval(env)?, r.eval(env)?);
                Ok(match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    _ => {
                        if b == 0 {
                            return Err("division by zero in a width expression".into());
                        }
                        a / b
                    }
                })
            }
        }
    }

    fn params(&self, out: &mut Vec<String>) {
        match self {
            WExpr::Lit(_) => {}
            WExpr::Par(p) => {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
            WExpr::Bin(_, l, r) => {
                l.params(out);
                r.params(out);
            }
        }
    }
}

/// bare names are the canonical spelling; the legacy '%' sigil remains
/// as the escape for names that collide with reserved words (i2, add...)
fn fmt_name(name: &str) -> String {
    if Parser::is_reserved(name) {
        format!("%{}", name)
    } else {
        name.to_string()
    }
}

/// like fmt_name, but with '@' as the function-name escape
fn fmt_fn_name(name: &str) -> String {
    if Parser::is_reserved(name) {
        format!("@{}", name)
    } else {
        name.to_string()
    }
}

/// point an instruction's destination at a different value
fn rebind_dst(inst: &mut Inst, from: ValueId, to: ValueId) {
    match inst {
        Inst::IConst { dst, .. }
        | Inst::FConst { dst, .. }
        | Inst::Bin { dst, .. }
        | Inst::ICmp { dst, .. }
        | Inst::FCmp { dst, .. }
        | Inst::Cast { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::PtrAdd { dst, .. }
        | Inst::Extract { dst, .. }
        | Inst::Pack { dst, .. }
        | Inst::Insert { dst, .. } => {
            if *dst == from {
                *dst = to;
            }
        }
        Inst::Call { dsts, .. } => {
            for d in dsts {
                if *d == from {
                    *d = to;
                }
            }
        }
        _ => {}
    }
}

fn symty_params(st: &SymTy, out: &mut Vec<String>) {
    match st {
        SymTy::C(_) => {}
        SymTy::IW(e) | SymTy::UW(e) => e.params(out),
        SymTy::GS(_, es) => {
            for e in es {
                e.params(out);
            }
        }
        SymTy::FPW(e, m) => {
            e.params(out);
            m.params(out);
        }
    }
}

/// the (E, M) of any float-family member
fn float_em(t: Type) -> Option<(i64, i64)> {
    match t {
        Type::F32 => Some((8, 23)),
        Type::F64 => Some((11, 52)),
        Type::FP(e, m) => Some((e as i64, m as i64)),
        _ => None,
    }
}

/// a type in a width-generic signature, held symbolically until the
/// call site's argument types solve the parameters
#[derive(Clone, Debug)]
enum SymTy {
    C(Type),
    IW(WExpr),
    UW(WExpr),
    GS(String, Vec<WExpr>),
    /// float(E, M) with free parameters — infers from FP/F32/F64 args
    FPW(WExpr, WExpr),
}

/// a width-generic function: a token-range template, instantiated by
/// re-parsing under a parameter environment (monomorphization)
struct GenericFn {
    params: Vec<String>,
    lo: usize,
    sig_params: Vec<SymTy>,
    sig_rets: Vec<SymTy>,
}

struct GenericStruct {
    params: Vec<String>,
    /// token range of the field list, from '{'
    fields_at: usize,
}

/// expression-sugar AST: parsed first, then desugared to flat temps so
/// the root operation can define the declared value directly
enum EAst {
    V(ValueId),
    Lit(Tok),
    Bin(&'static str, Box<EAst>, Box<EAst>),
    Cmp(&'static str, Box<EAst>, Box<EAst>),
}

/// Placeholder branch target inside structured constructs; every one is
/// patched to the real join/exit block before parsing of the construct ends.
const DUMMY_BLOCK: BlockId = BlockId(u32::MAX);

struct LoopFrame {
    header: BlockId,
    breaks: Vec<(usize, usize)>, // (block, inst) of Jmps to patch to the exit
    var_tys: Vec<Type>,          // loop variable types, for continue literals
    res_tys: Vec<Type>,          // bound result types, for break literals
}

/// Block-graph builder for structured bodies.
struct StructEmit {
    blocks: Vec<Block>,
    cur: usize,
    loop_stack: Vec<LoopFrame>,
    /// One frame per enclosing value-yielding position: edges waiting to be
    /// patched to an if's join block.
    yield_stack: Vec<Vec<(usize, usize)>>,
    /// result types of each enclosing value-yielding if, for yield literals
    yield_tys: Vec<Vec<Type>>,
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
    /// the function's return types, for literal/call sugar in `ret`
    rets: Vec<Type>,
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
        if let Some(Tok::TyName(_)) = self.peek() {
            let Tok::TyName(n) = self.next()? else {
                unreachable!()
            };
            // parametric struct instantiation: $fp(4, 3) or, inside a
            // generic body, $rat(N)
            if matches!(self.peek(), Some(Tok::LParen)) {
                let args = self.parse_width_args()?;
                return self.instantiate_struct(&n, &args).map(Type::Struct);
            }
            return match self.structs.iter().position(|d| d.name == n) {
                Some(i) => Ok(Type::Struct(i as u16)),
                None => {
                    self.pos -= 1;
                    Err(self.err(format!("unknown struct type '${}'", n)))
                }
            };
        }
        let s = self.expect_ident()?;
        // the float family: float(8, 23) is f32, float(11, 52) is f64,
        // anything else a small-float storage format
        if s == "float" && matches!(self.peek(), Some(Tok::LParen)) {
            let args = self.parse_width_args()?;
            if args.len() != 2 {
                return Err(self.err("float(E, M) takes two parameters".to_string()));
            }
            let (e, m) = (args[0], args[1]);
            return match (e, m) {
                (8, 23) => Ok(Type::F32),
                (11, 52) => Ok(Type::F64),
                (e, m) if (2..=11).contains(&e) && (1..=24).contains(&m) => {
                    Ok(Type::FP(e as u8, m as u8))
                }
                _ => Err(self.err(format!(
                    "float({}, {}) is out of range (E in 2..=11, M in 1..=24,                      or the native (8, 23) / (11, 52))",
                    e, m
                ))),
            };
        }
        // parametric integer widths: i(N), u(2*N), evaluated in the
        // current instantiation's environment
        if (s == "i" || s == "u") && matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            let e = self.parse_wexpr()?;
            self.expect(Tok::RParen)?;
            let w = e
                .eval(&self.wenv)
                .map_err(|m| self.err(m))?;
            if !(1..=64).contains(&w) {
                return Err(self.err(format!(
                    "width expression evaluates to {}; 1..=64 required",
                    w
                )));
            }
            return Ok(if s == "i" {
                Type::I(w as u8)
            } else {
                Type::U(w as u8)
            });
        }
        Type::from_name(&s).ok_or_else(|| {
            self.pos -= 1;
            self.err(format!("unknown type '{}'", s))
        })
    }

    /// the `{ field: ty, ... }` list of a struct declaration (shared by
    /// concrete declarations and generic instantiation)
    fn parse_field_list(&mut self) -> Result<Vec<(String, Type)>, ParseError> {
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        loop {
            let fname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.expect_type()?;
            let abstract_int =
                matches!(ty, Type::Int | Type::Uint | Type::Half | Type::UHalf);
            if ty.width().is_none() && !abstract_int {
                return Err(self.err(format!(
                    "struct field '{}' must have an integer width",
                    fname
                )));
            }
            fields.push((fname, ty));
            if self.eat(&Tok::RBrace) {
                break;
            }
            self.expect(Tok::Comma)?;
        }
        Ok(fields)
    }

    /// the float family member for evaluated (E, M) arguments
    fn make_float_type(&self, args: &[i64]) -> Result<Type, ParseError> {
        if args.len() != 2 {
            return Err(self.err("float(E, M) takes two parameters".to_string()));
        }
        let (e, m) = (args[0], args[1]);
        match (e, m) {
            (8, 23) => Ok(Type::F32),
            (11, 52) => Ok(Type::F64),
            (e, m) if (2..=8).contains(&e) && (1..=24).contains(&m) => {
                Ok(Type::FP(e as u8, m as u8))
            }
            _ => Err(self.err(format!(
                "float({}, {}) is out of range (E in 2..=8, M in 1..=24 for \
                 small formats — every value must be an f64 normal — or \
                 the native (8, 23) / (11, 52))",
                e, m
            ))),
        }
    }

    /// `( expr, expr, ... )` — evaluated width arguments
    fn parse_width_args(&mut self) -> Result<Vec<i64>, ParseError> {
        self.expect(Tok::LParen)?;
        let mut out = Vec::new();
        loop {
            let e = self.parse_wexpr()?;
            out.push(e.eval(&self.wenv).map_err(|m| self.err(m))?);
            if self.eat(&Tok::RParen) {
                break;
            }
            self.expect(Tok::Comma)?;
        }
        Ok(out)
    }

    /// width expression: + - over * /, literals, parameters, parens
    fn parse_wexpr(&mut self) -> Result<WExpr, ParseError> {
        let mut lhs = self.parse_wterm()?;
        loop {
            match self.peek() {
                Some(&Tok::Op(o @ ("+" | "-"))) => {
                    self.pos += 1;
                    let rhs = self.parse_wterm()?;
                    lhs = WExpr::Bin(o.chars().next().unwrap(), Box::new(lhs), Box::new(rhs));
                }
                // "N-1" lexes the 1 as Int(-1): absorb the sign
                Some(&Tok::Int(n)) if n < 0 => {
                    self.pos += 1;
                    lhs = WExpr::Bin('-', Box::new(lhs), Box::new(WExpr::Lit(-n)));
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_wterm(&mut self) -> Result<WExpr, ParseError> {
        let mut lhs = self.parse_wfactor()?;
        while let Some(&Tok::Op(o @ ("*" | "/"))) = self.peek() {
            self.pos += 1;
            let rhs = self.parse_wfactor()?;
            lhs = WExpr::Bin(o.chars().next().unwrap(), Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_wfactor(&mut self) -> Result<WExpr, ParseError> {
        match self.next()? {
            Tok::Int(n) => Ok(WExpr::Lit(n)),
            Tok::Ident(p) => Ok(WExpr::Par(p)),
            Tok::LParen => {
                let e = self.parse_wexpr()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a width expression, found {}", t)))
            }
        }
    }

    /// instantiate a generic struct with concrete width arguments,
    /// memoized; the instance is an ordinary struct named e.g. fp__4_3
    fn instantiate_struct(&mut self, gname: &str, vals: &[i64]) -> Result<u16, ParseError> {
        if let Some(&id) = self.struct_insts.get(&(gname.to_string(), vals.to_vec())) {
            return Ok(id);
        }
        let Some(g) = self.gstructs.get(gname) else {
            return Err(self.err(format!("'${}' is not a parametric struct type", gname)));
        };
        if g.params.len() != vals.len() {
            return Err(self.err(format!(
                "'${}' takes {} width parameters, {} given",
                gname,
                g.params.len(),
                vals.len()
            )));
        }
        let (params, fields_at) = (g.params.clone(), g.fields_at);
        let save_pos = self.pos;
        let save_env = std::mem::take(&mut self.wenv);
        for (p, &v) in params.iter().zip(vals) {
            self.wenv.insert(p.clone(), v);
        }
        self.pos = fields_at;
        let fields = self.parse_field_list();
        self.pos = save_pos;
        self.wenv = save_env;
        let fields = fields?;
        let mangled = format!(
            "{}__{}",
            gname,
            vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("_")
        );
        let def = StructDef {
            name: mangled,
            fields,
        };
        if def.total_bits() == 0 || def.word_layout().0 > 8 {
            return Err(self.err(format!(
                "'${}' instantiated at {:?} is {} bits; 1 bit to 8 words required",
                gname,
                vals,
                def.total_bits()
            )));
        }
        let id = self.structs.len() as u16;
        self.structs.push(def);
        self.struct_insts
            .insert((gname.to_string(), vals.to_vec()), id);
        self.struct_inst_rev
            .insert(id, (gname.to_string(), vals.to_vec()));
        Ok(id)
    }

    /// `type $name = { field: iN, ... }` — fields are declared low-first
    /// and must be integer widths; the total may not exceed 64 bits
    /// words a bare (sigil-less) value name may not shadow
    fn is_reserved(name: &str) -> bool {
        Parser::is_opcode(name)
            || matches!(
                name,
                "fn" | "type"
                    | "ret"
                    | "if"
                    | "else"
                    | "loop"
                    | "break"
                    | "continue"
                    | "yield"
                    | "call"
                    | "jmp"
                    | "br"
                    | "float"
            )
            || Type::from_name(name).is_some()
    }

    fn is_opcode(name: &str) -> bool {
        name.starts_with("icmp.")
            || name.starts_with("fcmp.")
            || matches!(
                name,
                "b.eq" | "b.ne" | "b.lt" | "b.le" | "b.gt" | "b.ge" | "b.lo" | "b.ls"
                    | "b.hi" | "b.hs"
            )
            || matches!(
                name,
                "add" | "sub" | "mul" | "div" | "rem"
                    | "iconst" | "fconst"
                    | "ext" | "trunc" | "itof" | "ftoi"
                    | "fpromote" | "fdemote" | "bitcast"
                    | "load" | "store" | "ptradd"
                    | "extract" | "pack" | "insert"
            )
            || BINOPS.iter().any(|(n, _)| *n == name)
    }

    /// does the rest of this line contain an expression operator? Used to
    /// route `%r: u1 = call @f(%x) == 0` to the expression parser while
    /// keeping plain call bindings on the direct path.
    fn line_has_op(&self) -> bool {
        for (t, _) in &self.toks[self.pos..] {
            match t {
                Tok::Newline => return false,
                Tok::Op(_) => return true,
                _ => {}
            }
        }
        false
    }

    fn skip_line(&mut self) {
        while !self.at_end() {
            let (t, _) = &self.toks[self.pos];
            self.pos += 1;
            if matches!(t, Tok::Newline) {
                break;
            }
        }
    }

    /// read one function's signature into `sigs` and skip its body
    /// read one function's signature — SYMBOLICALLY, so width-generic
    /// functions (any i(N)/u(N)/$g(N) with free parameters) register as
    /// templates; concrete functions register their sigs as before.
    fn prescan_signature(&mut self) -> Result<(), ParseError> {
        let kw = self.expect_ident()?;
        if kw != "fn" {
            self.pos -= 1;
            return Err(self.err(format!("expected 'fn', found '{}'", kw)));
        }
        self.pos -= 1;
        let (lo, hi) = self.function_range()?;
        self.pos += 1; // past `fn`
        let name = match self.next()? {
            Tok::Global(n) | Tok::Ident(n) => n,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a function name, found {}", t)));
            }
        };
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                match self.next()? {
                    Tok::Value(_) | Tok::Ident(_) => {}
                    t => {
                        self.pos -= 1;
                        return Err(self.err(format!("expected a parameter, found {}", t)));
                    }
                }
                self.expect(Tok::Colon)?;
                params.push(self.parse_symty()?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        let mut rets = Vec::new();
        if self.eat(&Tok::Arrow) {
            if self.eat(&Tok::LParen) {
                loop {
                    rets.push(self.parse_symty()?);
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    self.expect(Tok::Comma)?;
                }
            } else {
                rets.push(self.parse_symty()?);
            }
        }
        // free width parameters anywhere in the signature make it generic
        let mut free = Vec::new();
        for st in params.iter().chain(&rets) {
            symty_params(st, &mut free);
        }
        if free.is_empty() {
            let empty = HashMap::new();
            let cp: Result<Vec<Type>, _> =
                params.iter().map(|t| self.eval_symty(t, &empty)).collect();
            let cr: Result<Vec<Type>, _> =
                rets.iter().map(|t| self.eval_symty(t, &empty)).collect();
            self.sigs.insert(name, (cp?, cr?));
        } else {
            self.gfns.insert(
                name,
                GenericFn {
                    params: free,
                    lo,
                    sig_params: params,
                    sig_rets: rets,
                },
            );
        }
        self.pos = hi + 1; // past the closing brace
        Ok(())
    }

    /// a signature type, held symbolically: concrete, i(expr), u(expr),
    /// or a generic-struct application $g(expr, ...)
    fn parse_symty(&mut self) -> Result<SymTy, ParseError> {
        if let Some(Tok::TyName(_)) = self.peek() {
            let Tok::TyName(n) = self.next()? else { unreachable!() };
            if matches!(self.peek(), Some(Tok::LParen)) {
                self.pos += 1;
                let mut exprs = Vec::new();
                loop {
                    exprs.push(self.parse_wexpr()?);
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    self.expect(Tok::Comma)?;
                }
                return Ok(SymTy::GS(n, exprs));
            }
            return match self.structs.iter().position(|d| d.name == n) {
                Some(i) => Ok(SymTy::C(Type::Struct(i as u16))),
                None => {
                    self.pos -= 1;
                    Err(self.err(format!("unknown struct type '${}'", n)))
                }
            };
        }
        let s = self.expect_ident()?;
        if (s == "i" || s == "u") && matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            let e = self.parse_wexpr()?;
            self.expect(Tok::RParen)?;
            return Ok(if s == "i" { SymTy::IW(e) } else { SymTy::UW(e) });
        }
        if s == "float" && matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            let e = self.parse_wexpr()?;
            self.expect(Tok::Comma)?;
            let m = self.parse_wexpr()?;
            self.expect(Tok::RParen)?;
            let mut ps = Vec::new();
            e.params(&mut ps);
            m.params(&mut ps);
            if ps.is_empty() {
                let args = vec![
                    e.eval(&HashMap::new()).map_err(|x| self.err(x))?,
                    m.eval(&HashMap::new()).map_err(|x| self.err(x))?,
                ];
                return self.make_float_type(&args).map(SymTy::C);
            }
            return Ok(SymTy::FPW(e, m));
        }
        match Type::from_name(&s) {
            Some(t) => Ok(SymTy::C(t)),
            None => {
                self.pos -= 1;
                Err(self.err(format!("unknown type '{}'", s)))
            }
        }
    }

    /// evaluate a symbolic type under a width environment
    fn eval_symty(
        &mut self,
        st: &SymTy,
        env: &HashMap<String, i64>,
    ) -> Result<Type, ParseError> {
        match st {
            SymTy::C(t) => Ok(*t),
            SymTy::IW(e) => {
                let w = e.eval(env).map_err(|m| self.err(m))?;
                if !(1..=64).contains(&w) {
                    return Err(self.err(format!("width {} out of range 1..=64", w)));
                }
                Ok(Type::I(w as u8))
            }
            SymTy::UW(e) => {
                let w = e.eval(env).map_err(|m| self.err(m))?;
                if !(1..=64).contains(&w) {
                    return Err(self.err(format!("width {} out of range 1..=64", w)));
                }
                Ok(Type::U(w as u8))
            }
            SymTy::GS(n, exprs) => {
                let vals: Result<Vec<i64>, _> =
                    exprs.iter().map(|e| e.eval(env)).collect();
                let vals = vals.map_err(|m| self.err(m))?;
                let (n, vals2) = (n.clone(), vals);
                self.instantiate_struct(&n, &vals2).map(Type::Struct)
            }
            SymTy::FPW(e, m) => {
                let args = vec![
                    e.eval(env).map_err(|x| self.err(x))?,
                    m.eval(env).map_err(|x| self.err(x))?,
                ];
                self.make_float_type(&args)
            }
        }
    }

    /// queue a generic-function instantiation (memoized); its concrete
    /// signature is registered immediately so call sugar works, and its
    /// body is parsed from the worklist after the main pass
    fn request_fn_inst(&mut self, gname: &str, vals: &[i64]) -> Result<String, ParseError> {
        let mangled = format!(
            "{}__{}",
            gname,
            vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("_")
        );
        if self.fn_insts.contains_key(&mangled) {
            return Ok(mangled);
        }
        let g = &self.gfns[gname];
        let (params, sp, sr) = (g.params.clone(), g.sig_params.clone(), g.sig_rets.clone());
        let mut env = HashMap::new();
        for (p, &v) in params.iter().zip(vals) {
            env.insert(p.clone(), v);
        }
        let cp: Result<Vec<Type>, _> = sp.iter().map(|t| self.eval_symty(t, &env)).collect();
        let cr: Result<Vec<Type>, _> = sr.iter().map(|t| self.eval_symty(t, &env)).collect();
        self.sigs.insert(mangled.clone(), (cp?, cr?));
        self.fn_insts.insert(mangled.clone(), false);
        self.fn_worklist
            .push((gname.to_string(), vals.to_vec(), mangled.clone()));
        Ok(mangled)
    }

    /// does `( ... ) (` follow — an explicit width-argument group before
    /// the real argument list? (No %values inside the first group.)
    fn explicit_widths_ahead(&self) -> bool {
        let mut i = self.pos;
        let Some((Tok::LParen, _)) = self.toks.get(i) else {
            return false;
        };
        i += 1;
        let mut depth = 1usize;
        while let Some((t, _)) = self.toks.get(i) {
            match t {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.toks.get(i + 1).map(|(t, _)| t),
                            Some(Tok::LParen)
                        );
                    }
                }
                Tok::Value(_) | Tok::Newline => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// solve a generic call's width parameters by matching argument types
    /// against the symbolic signature, then instantiate
    fn solve_generic_call(
        &mut self,
        gname: &str,
        args: &[ValueId],
        scope: &FuncScope,
    ) -> Result<String, ParseError> {
        let g = &self.gfns[gname];
        let (gparams, sig) = (g.params.clone(), g.sig_params.clone());
        if args.len() != sig.len() {
            return Err(self.err(format!(
                "@{} takes {} arguments, {} given",
                gname,
                sig.len(),
                args.len()
            )));
        }
        let mut env: HashMap<String, i64> = HashMap::new();
        let bind = |env: &mut HashMap<String, i64>, p: &str, v: i64| -> Result<(), String> {
            match env.get(p) {
                Some(&old) if old != v => Err(format!(
                    "width parameter {} is both {} and {}",
                    p, old, v
                )),
                _ => {
                    env.insert(p.to_string(), v);
                    Ok(())
                }
            }
        };
        // pass 1: positions that name a parameter directly
        for (&a, st) in args.iter().zip(&sig) {
            let aty = scope.values[a.0 as usize].ty;
            let r = match (st, aty) {
                (SymTy::IW(WExpr::Par(p)), Type::I(w)) => bind(&mut env, p, w as i64),
                (SymTy::UW(WExpr::Par(p)), Type::U(w)) => bind(&mut env, p, w as i64),
                (SymTy::FPW(ee, me), aty2) => match float_em(aty2) {
                    Some((ev, mv)) => {
                        let mut r = Ok(());
                        if let WExpr::Par(p) = ee {
                            r = bind(&mut env, p, ev);
                        }
                        if r.is_ok() {
                            if let WExpr::Par(p) = me {
                                r = bind(&mut env, p, mv);
                            }
                        }
                        r
                    }
                    None => Ok(()),
                },
                (SymTy::GS(n, exprs), Type::Struct(id)) => {
                    match self.struct_inst_rev.get(&id) {
                        Some((gn, vals)) if gn == n => {
                            let mut r = Ok(());
                            for (e, &v) in exprs.iter().zip(vals) {
                                if let WExpr::Par(p) = e {
                                    if let Err(m) = bind(&mut env, p, v) {
                                        r = Err(m);
                                        break;
                                    }
                                }
                            }
                            r
                        }
                        _ => Ok(()), // pass 2 reports the mismatch
                    }
                }
                _ => Ok(()),
            };
            r.map_err(|m| self.err(m))?;
        }
        for p in &gparams {
            if !env.contains_key(p) {
                return Err(self.err(format!(
                    "cannot infer width parameter {} for @{} from the argument types",
                    p, gname
                )));
            }
        }
        // pass 2: every position must agree once evaluated
        for (i, (&a, st)) in args.iter().zip(&sig).enumerate() {
            let want = self.eval_symty(st, &env)?;
            let got = scope.values[a.0 as usize].ty;
            if want != got {
                return Err(self.err(format!(
                    "@{} argument {}: expected {}, got {}",
                    gname,
                    i + 1,
                    want.name(),
                    got.name()
                )));
            }
        }
        let vals: Vec<i64> = gparams.iter().map(|p| env[p]).collect();
        self.request_fn_inst(gname, &vals)
    }

    fn parse_type_decl(&mut self) -> Result<(), ParseError> {
        self.expect_ident()?; // "type"
        let name = match self.next()? {
            Tok::TyName(n) => n,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a $name, found {}", t)));
            }
        };
        if self.structs.iter().any(|d| d.name == name) {
            return Err(self.err(format!("struct '${}' is defined more than once", name)));
        }
        // parametric declaration: type $rat(N) = { ... } — capture the
        // field list as a template, instantiated on use
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            let mut params = Vec::new();
            loop {
                params.push(self.expect_ident()?);
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
            self.expect(Tok::Equals)?;
            let fields_at = self.pos; // at '{'
            self.gstructs.insert(name, GenericStruct { params, fields_at });
            self.skip_line();
            return Ok(());
        }
        self.expect(Tok::Equals)?;
        let fields = self.parse_field_list()?;
        let def = StructDef { name, fields };
        // abstract fields have no width yet; the resolved layout is
        // checked by the verifier instead
        let all_known = def.fields.iter().all(|(_, t)| t.width().is_some());
        if all_known && (def.total_bits() == 0 || def.word_layout().0 > 8) {
            return Err(self.err(format!(
                "struct '${}' is {} bits; 1 bit to 8 words (512 bits) required",
                def.name,
                def.total_bits()
            )));
        }
        self.expect(Tok::Newline)?;
        self.structs.push(def);
        Ok(())
    }

    /// a statement ends at a newline — or directly before the '}' that
    /// closes its block, so single-line forms like `if %c { ret 0 }` work
    fn end_stmt(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek(), Some(Tok::RBrace)) {
            return Ok(());
        }
        self.expect(Tok::Newline)
    }

    /// a fresh value of the given type, uniquely named (%c1, %c2, ...)
    fn temp_val(&mut self, scope: &mut FuncScope, ty: Type) -> ValueId {
        let mut name;
        loop {
            self.nlit += 1;
            name = format!("c{}", self.nlit);
            if !scope.value_ids.contains_key(&name) {
                break;
            }
        }
        scope.values.push(ValueData { name: name.clone(), ty });
        let id = ValueId(scope.values.len() as u32 - 1);
        scope.value_ids.insert(name, id);
        id
    }

    /// the $fp(E, M) struct view of a float-family type
    fn float_view(&mut self, t: Type) -> Result<u16, ParseError> {
        let Some((e, m)) = float_em(t) else {
            return Err(self.err(format!("{} has no fields", t.name())));
        };
        if !self.gstructs.contains_key("fp") {
            return Err(self.err(
                "float field access needs the fp struct from lib/float.ssa".to_string(),
            ));
        }
        self.instantiate_struct("fp", &[e, m])
    }

    /// value.field — extract from a struct, or from a float through its
    /// bitfield view; emits through `pending`
    fn field_access(
        &mut self,
        scope: &mut FuncScope,
        id: ValueId,
        field: &str,
    ) -> Result<ValueId, ParseError> {
        let ty = scope.values[id.0 as usize].ty;
        let (src, si) = match ty {
            Type::Struct(si) => (id, si),
            t if float_em(t).is_some() => {
                let si = self.float_view(t)?;
                let sv = self.temp_val(scope, Type::Struct(si));
                self.pending.push(Inst::Cast {
                    op: CastOp::Bitcast,
                    dst: sv,
                    src: id,
                });
                (sv, si)
            }
            t => {
                return Err(self.err(format!("{} has no fields", t.name())));
            }
        };
        let def = &self.structs[si as usize];
        let Some(fi) = def.fields.iter().position(|(n, _)| n == field) else {
            return Err(self.err(format!(
                "'${}' has no field '{}'",
                def.name, field
            )));
        };
        let fty = def.fields[fi].1;
        let dst = self.temp_val(scope, fty);
        self.pending.push(Inst::Extract {
            dst,
            src,
            field: fi as u16,
        });
        Ok(dst)
    }

    /// TypeName(fields...) — pack a struct (or a float, through its
    /// bitfield view) from all its fields, in declaration order
    fn construct(&mut self, scope: &mut FuncScope, ty: Type) -> Result<ValueId, ParseError> {
        let si = match ty {
            Type::Struct(si) => si,
            t if float_em(t).is_some() => self.float_view(t)?,
            t => {
                return Err(self.err(format!("{} cannot be constructed", t.name())));
            }
        };
        let ftys: Vec<Type> = self.structs[si as usize]
            .fields
            .iter()
            .map(|(_, t)| *t)
            .collect();
        self.expect(Tok::LParen)?;
        let mut args = Vec::new();
        loop {
            let ast = self.expr_level(scope, 0)?;
            let Some(&fty) = ftys.get(args.len()) else {
                return Err(self.err(format!(
                    "too many fields for '${}'",
                    self.structs[si as usize].name
                )));
            };
            // field expressions compute at full width (so shifts and
            // masks behave naturally) and narrow at the boundary;
            // literals and already-field-typed values pass straight in
            let v = match ast {
                EAst::Lit(tok) => self.synth_lit(scope, fty, &tok)?,
                EAst::V(id) if scope.values[id.0 as usize].ty == fty => id,
                ast => {
                    let wide = self.emit_ast(scope, Type::U(64), ast)?;
                    let wty = scope.values[wide.0 as usize].ty;
                    if wty == fty {
                        wide
                    } else {
                        let t = self.temp_val(scope, fty);
                        let op = match (wty.width(), fty.width()) {
                            (Some(a), Some(b)) if a > b => CastOp::Trunc,
                            (Some(a), Some(b)) if a < b => CastOp::Ext,
                            _ => CastOp::Bitcast,
                        };
                        self.pending.push(Inst::Cast {
                            op,
                            dst: t,
                            src: wide,
                        });
                        t
                    }
                }
            };
            args.push(v);
            if self.eat(&Tok::RParen) {
                break;
            }
            self.expect(Tok::Comma)?;
        }
        if args.len() != ftys.len() {
            return Err(self.err(format!(
                "'${}' has {} fields, {} given",
                self.structs[si as usize].name,
                ftys.len(),
                args.len()
            )));
        }
        let sv = self.temp_val(scope, Type::Struct(si));
        self.pending.push(Inst::Pack { dst: sv, args });
        if matches!(ty, Type::Struct(_)) {
            Ok(sv)
        } else {
            let fv = self.temp_val(scope, ty);
            self.pending.push(Inst::Cast {
                op: CastOp::Bitcast,
                dst: fv,
                src: sv,
            });
            Ok(fv)
        }
    }

    /// synthesize a constant of the given type from a literal token; its
    /// defining instruction goes to `pending`, drained before the user of
    /// the constant. This is the literal-operand sugar: `iadd %i, 1`.
    fn synth_lit(
        &mut self,
        scope: &mut FuncScope,
        ty: Type,
        tok: &Tok,
    ) -> Result<ValueId, ParseError> {
        let floatish = matches!(
            ty,
            Type::F32 | Type::F64 | Type::Float | Type::Scalar | Type::FP(..)
        );
        let inst_for = |dst: ValueId| -> Result<Inst, String> {
            match (tok, floatish) {
                (Tok::Int(n), false) => Ok(Inst::IConst { dst, imm: *n }),
                (Tok::Int(n), true) => Ok(Inst::FConst {
                    dst,
                    bits: (*n as f64).to_bits(),
                }),
                (Tok::FloatLit(x), true) => Ok(Inst::FConst {
                    dst,
                    bits: x.to_bits(),
                }),
                (Tok::FloatLit(_), false) => {
                    Err(format!("float literal needs a float type, not {}", ty.name()))
                }
                _ => Err("expected a literal".into()),
            }
        };
        if matches!(ty, Type::Vec(..) | Type::Struct(_)) {
            return Err(self.err(format!(
                "literal operands cannot have type {} (use pack)",
                ty.name()
            )));
        }
        let id = self.temp_val(scope, ty);
        let inst = inst_for(id).map_err(|m| self.err(m))?;
        self.pending.push(inst);
        Ok(id)
    }

    /// a value operand that may also be a literal of the given type
    fn operand(
        &mut self,
        scope: &mut FuncScope,
        ty: Type,
    ) -> Result<ValueId, ParseError> {
        match self.peek() {
            Some(Tok::Int(_)) | Some(Tok::FloatLit(_)) => {
                let tok = self.next()?.clone();
                self.synth_lit(scope, ty, &tok)
            }
            _ => self.expect_value_mut(scope),
        }
    }

    /// Expression sugar: `%v: ty = %a * %b + 1`. Pure arithmetic with C
    /// precedence (| ^ & << >> + - * / %), parenthesized subexpressions,
    /// and literal operands; every node has the declared result type, and
    /// the opcode family (iadd vs fadd, div's signedness) comes from that
    /// type — exactly the types-on-variables rule, applied to sugar. The
    /// tree desugars to ordinary flat instructions at parse time (temps
    /// %c1, %c2, ...); like structured control flow, it is one-way sugar —
    /// the printer prints flat form.
    fn parse_expr_into(
        &mut self,
        dst: ValueId,
        scope: &mut FuncScope,
    ) -> Result<Inst, ParseError> {
        let ty = scope.values[dst.0 as usize].ty;
        let vlen = scope.values.len() as u32;
        let ast = self.expr_level(scope, 0)?;
        match ast {
            EAst::Bin(sym, l, r) => {
                let op = self.expr_op(ty, sym)?;
                let lhs = self.emit_expr(scope, ty, *l)?;
                let rhs = self.emit_expr(scope, ty, *r)?;
                Ok(Inst::Bin { op, dst, lhs, rhs })
            }
            EAst::Cmp(sym, l, r) => self.emit_cmp(scope, sym, *l, *r, dst),
            EAst::Lit(tok) => self.lit_inst(ty, dst, &tok),
            EAst::V(id) if id.0 >= vlen => {
                // the root is machinery this expression emitted (a field
                // access or constructor): rebind its result to `dst`
                let Some(last) = self.pending.last_mut() else {
                    return Err(self.err("internal: expression temp without a definition".to_string()));
                };
                rebind_dst(last, id, dst);
                Ok(self.pending.pop().unwrap())
            }
            EAst::V(_) => Err(self.err(
                "an expression must compute something; there is no copy opcode".to_string(),
            )),
        }
    }

    /// a comparison's operand type comes from whichever side names a
    /// value; both sides literal is an error (nothing fixes the width)
    fn side_ty(&self, scope: &FuncScope, e: &EAst) -> Option<Type> {
        match e {
            EAst::V(id) => Some(scope.values[id.0 as usize].ty),
            EAst::Lit(_) => None,
            EAst::Bin(_, l, r) | EAst::Cmp(_, l, r) => {
                self.side_ty(scope, l).or_else(|| self.side_ty(scope, r))
            }
        }
    }

    fn emit_cmp(
        &mut self,
        scope: &mut FuncScope,
        sym: &'static str,
        l: EAst,
        r: EAst,
        dst: ValueId,
    ) -> Result<Inst, ParseError> {
        let sty = self
            .side_ty(scope, &l)
            .or_else(|| self.side_ty(scope, &r))
            .ok_or_else(|| {
                self.err("a comparison needs at least one value operand".to_string())
            })?;
        let lhs = self.emit_expr(scope, sty, l)?;
        let rhs = self.emit_expr(scope, sty, r)?;
        let floatish = matches!(
            sty,
            Type::F32 | Type::F64 | Type::Float | Type::Scalar | Type::FP(..)
        );
        if floatish {
            let cond = match sym {
                "<" => FCond::Olt,
                "<=" => FCond::Ole,
                ">" => FCond::Ogt,
                ">=" => FCond::Oge,
                "==" => FCond::Oeq,
                _ => FCond::Une,
            };
            Ok(Inst::FCmp { cond, dst, lhs, rhs })
        } else {
            let cond = match sym {
                "<" => Cond::Lt,
                "<=" => Cond::Le,
                ">" => Cond::Gt,
                ">=" => Cond::Ge,
                "==" => Cond::Eq,
                _ => Cond::Ne,
            };
            Ok(Inst::ICmp { cond, dst, lhs, rhs })
        }
    }

    fn emit_expr(
        &mut self,
        scope: &mut FuncScope,
        ty: Type,
        e: EAst,
    ) -> Result<ValueId, ParseError> {
        match e {
            EAst::V(id) => Ok(id),
            EAst::Lit(tok) => self.synth_lit(scope, ty, &tok),
            EAst::Bin(sym, l, r) => {
                let op = self.expr_op(ty, sym)?;
                let lhs = self.emit_expr(scope, ty, *l)?;
                let rhs = self.emit_expr(scope, ty, *r)?;
                let id = self.temp_val(scope, ty);
                self.pending.push(Inst::Bin { op, dst: id, lhs, rhs });
                Ok(id)
            }
            EAst::Cmp(..) => Err(self.err(
                "comparisons cannot nest inside expressions".to_string(),
            )),
        }
    }

    /// operator for the level's symbol, chosen by the result type
    fn expr_op(&self, ty: Type, sym: &str) -> Result<BinOp, ParseError> {
        let floatish = matches!(
            ty,
            Type::F32 | Type::F64 | Type::Float | Type::Scalar | Type::FP(..)
        )
            || matches!(ty, Type::Vec(_, VecElem::F32));
        let op = match (sym, floatish) {
            ("+", true) => BinOp::FAdd,
            ("-", true) => BinOp::FSub,
            ("*", true) => BinOp::FMul,
            ("/", true) => BinOp::FDiv,
            ("+", false) => BinOp::IAdd,
            ("-", false) => BinOp::ISub,
            ("*", false) => BinOp::IMul,
            ("/", false) => BinOp::Div,
            ("%", false) => BinOp::Rem,
            ("&", false) => BinOp::And,
            ("|", false) => BinOp::Or,
            ("^", false) => BinOp::Xor,
            ("<<", false) => BinOp::Shl,
            (">>", false) => BinOp::Shr,
            (o, true) => {
                return Err(self.err(format!("'{}' is not a float operation", o)))
            }
            _ => unreachable!(),
        };
        Ok(op)
    }

    /// precedence-climbing: a single non-associative comparison on top,
    /// then the C levels | ^ & <<>> +- */%. Parsing is type-free — opcode
    /// resolution happens at emit time, once types are known.
    fn expr_level(&mut self, scope: &mut FuncScope, level: usize) -> Result<EAst, ParseError> {
        const CMPS: &[&str] = &["<", "<=", ">", ">=", "==", "!="];
        const LEVELS: &[&[&str]] = &[
            &["|"],
            &["^"],
            &["&"],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == 0 {
            let lhs = self.expr_level(scope, 1)?;
            if let Some(&Tok::Op(sym)) = self.peek() {
                if CMPS.contains(&sym) {
                    self.next()?;
                    let rhs = self.expr_level(scope, 1)?;
                    return Ok(EAst::Cmp(sym, Box::new(lhs), Box::new(rhs)));
                }
            }
            return Ok(lhs);
        }
        let lv = level - 1;
        if lv == LEVELS.len() {
            return self.expr_atom(scope);
        }
        let mut lhs = self.expr_level(scope, level + 1)?;
        loop {
            // "%x - 3" lexes the literal as Int(-3): absorb the sign as
            // a subtraction at the +/- level
            if LEVELS[lv].contains(&"+") {
                match self.peek() {
                    Some(&Tok::Int(n)) if n < 0 && n != i64::MIN => {
                        self.next()?;
                        lhs = EAst::Bin("-", Box::new(lhs), Box::new(EAst::Lit(Tok::Int(-n))));
                        continue;
                    }
                    Some(&Tok::FloatLit(x)) if x < 0.0 => {
                        self.next()?;
                        lhs = EAst::Bin(
                            "-",
                            Box::new(lhs),
                            Box::new(EAst::Lit(Tok::FloatLit(-x))),
                        );
                        continue;
                    }
                    _ => {}
                }
            }
            let Some(&Tok::Op(sym)) = self.peek() else { break };
            if !LEVELS[lv].contains(&sym) {
                break;
            }
            self.next()?;
            let rhs = self.expr_level(scope, level + 1)?;
            lhs = EAst::Bin(sym, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn expr_atom(&mut self, scope: &mut FuncScope) -> Result<EAst, ParseError> {
        // width parameters are literals inside a generic instance:
        // mask: u64 = (1 << M) - 1
        if let Some(Tok::Ident(p)) = self.peek() {
            if let Some(&v) = self.wenv.get(p) {
                self.pos += 1;
                return Ok(EAst::Lit(Tok::Int(v)));
            }
        }
        // field access: value.field on a struct — or on a float, whose
        // fields are its $fp(E, M) view (frac / exp / sign)
        if let Some(Tok::Ident(name)) = self.peek() {
            if let Some(dot) = name.find('.') {
                let (base, field) = (name[..dot].to_string(), name[dot + 1..].to_string());
                if scope.value_ids.contains_key(&base) {
                    self.pos += 1;
                    let id = scope.value_ids[&base];
                    let v = self.field_access(scope, id, &field)?;
                    return Ok(EAst::V(v));
                }
            }
        }
        // constructors: float(E, M)(fields...) or $name(fields...) pack
        // a value from its fields
        if matches!(self.peek(), Some(Tok::Ident(k)) if k == "float")
            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen))
        {
            self.pos += 1;
            let args = self.parse_width_args()?;
            let ty = self.make_float_type(&args)?;
            let v = self.construct(scope, ty)?;
            return Ok(EAst::V(v));
        }
        if let Some(Tok::TyName(_)) = self.peek() {
            let Some(Tok::TyName(n)) = self.peek().cloned() else { unreachable!() };
            self.pos += 1;
            let ty = if matches!(self.peek(), Some(Tok::LParen))
                && self.explicit_widths_ahead()
            {
                let wargs = self.parse_width_args()?;
                Type::Struct(self.instantiate_struct(&n, &wargs)?)
            } else {
                match self.structs.iter().position(|d| d.name == n) {
                    Some(i) => Type::Struct(i as u16),
                    None => {
                        self.pos -= 1;
                        return Err(self.err(format!("unknown struct type '${}'", n)));
                    }
                }
            };
            let v = self.construct(scope, ty)?;
            return Ok(EAst::V(v));
        }
        // a name directly followed by '(' is a call, not a value
        let callish = matches!(
            (self.peek(), self.toks.get(self.pos + 1).map(|(t, _)| t)),
            (Some(Tok::Ident(k)), Some(Tok::LParen)) if !Parser::is_reserved(k)
        );
        match self.next()? {
            Tok::Value(name) | Tok::Ident(name)
                if !callish || matches!(self.toks[self.pos - 1].0, Tok::Value(_)) =>
            {
                let id = scope.value_ids.get(&name).copied().ok_or_else(|| {
                    self.pos -= 1;
                    self.err(format!("use of undefined value '{}'", name))
                })?;
                Ok(EAst::V(id))
            }
            t @ (Tok::Int(_) | Tok::FloatLit(_)) => Ok(EAst::Lit(t)),
            t2 @ (Tok::Global(_) | Tok::Ident(_))
                if matches!(&t2, Tok::Global(_))
                    || matches!(&t2, Tok::Ident(k) if k == "call")
                    || matches!((&t2, self.peek()), (Tok::Ident(k), Some(Tok::LParen))
                        if !Parser::is_reserved(k)) =>
            {
                if !matches!(&t2, Tok::Ident(k) if k == "call") {
                    self.pos -= 1; // parse_call_tail reads the name itself
                }
                let (callee, args) = self.parse_call_tail(scope)?;
                let rets = match self.sigs.get(&callee) {
                    Some((_, r)) if r.len() == 1 => r.clone(),
                    Some(_) => {
                        return Err(self.err(format!(
                            "@{} in an expression must return exactly one value",
                            callee
                        )))
                    }
                    None => {
                        return Err(self.err(format!(
                            "call in an expression needs @{}'s signature",
                            callee
                        )))
                    }
                };
                let t = self.temp_val(scope, rets[0]);
                self.pending.push(Inst::Call {
                    dsts: vec![t],
                    callee,
                    args,
                });
                Ok(EAst::V(t))
            }
            Tok::LParen => {
                let e = self.expr_level(scope, 0)?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a value, literal, or '(', found {}", t)))
            }
        }
    }

    /// a constant-defining instruction for a literal, typed by `ty`
    fn lit_inst(&self, ty: Type, dst: ValueId, tok: &Tok) -> Result<Inst, ParseError> {
        let floatish = matches!(
            ty,
            Type::F32 | Type::F64 | Type::Float | Type::Scalar | Type::FP(..)
        );
        match (tok, floatish) {
            (Tok::Int(n), false) => Ok(Inst::IConst { dst, imm: *n }),
            (Tok::Int(n), true) => Ok(Inst::FConst {
                dst,
                bits: (*n as f64).to_bits(),
            }),
            (Tok::FloatLit(x), true) => Ok(Inst::FConst {
                dst,
                bits: x.to_bits(),
            }),
            (Tok::FloatLit(_), false) => Err(self.err(format!(
                "float literal needs a float type, not {}",
                ty.name()
            ))),
            _ => Err(self.err("expected a literal".to_string())),
        }
    }

    /// comparison operands: either side may be a literal, typed by the
    /// other side (both literal is an error — nothing fixes the width)
    fn cmp_operands(
        &mut self,
        scope: &mut FuncScope,
    ) -> Result<(ValueId, ValueId), ParseError> {
        match self.peek() {
            Some(Tok::Int(_)) | Some(Tok::FloatLit(_)) => {
                let tok = self.next()?.clone();
                self.expect(Tok::Comma)?;
                let rhs = self.expect_value_mut(scope)?;
                let lhs = self.synth_lit(scope, scope.values[rhs.0 as usize].ty, &tok)?;
                Ok((lhs, rhs))
            }
            _ => {
                let lhs = self.expect_value_mut(scope)?;
                self.expect(Tok::Comma)?;
                let rhs = self.operand(scope, scope.values[lhs.0 as usize].ty)?;
                Ok((lhs, rhs))
            }
        }
    }

    fn expect_value(&mut self, scope: &FuncScope) -> Result<ValueId, ParseError> {
        match self.next()? {
            Tok::Value(name) | Tok::Ident(name) => {
                scope.value_ids.get(&name).copied().ok_or_else(|| {
                    self.pos -= 1;
                    self.err(format!("use of undefined value '{}'", name))
                })
            }
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a value, found {}", t)))
            }
        }
    }

    /// a value operand that may be a field access (value.field): the
    /// extract (and, for floats, the view bitcast) go through `pending`
    fn expect_value_mut(&mut self, scope: &mut FuncScope) -> Result<ValueId, ParseError> {
        if let Some(Tok::Ident(name)) = self.peek() {
            if let Some(dot) = name.find('.') {
                let (base, field) = (name[..dot].to_string(), name[dot + 1..].to_string());
                if scope.value_ids.contains_key(&base) {
                    self.pos += 1;
                    let id = scope.value_ids[&base];
                    return self.field_access(scope, id, &field);
                }
            }
        }
        self.expect_value(scope)
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

    /// Every value definition in the format is the pattern `%name : type` —
    /// function params, block params, and instruction results alike. One scan
    /// over the function's tokens builds the whole value table.
    fn prescan_values(&mut self, lo: usize, hi: usize) -> Result<FuncScope, ParseError> {
        let mut scope = FuncScope {
            values: Vec::new(),
            value_ids: HashMap::new(),
            block_ids: HashMap::new(),
            block_names: Vec::new(),
            rets: Vec::new(),
        };
        let mut i = lo;
        while i + 2 <= hi {
            let is_def = match (&self.toks[i].0, &self.toks[i + 1].0) {
                (Tok::Value(_), Tok::Colon) => true,
                (Tok::Ident(n), Tok::Colon) => !Parser::is_reserved(n),
                _ => false,
            };
            if is_def {
                let name = match &self.toks[i].0 {
                    Tok::Value(n) | Tok::Ident(n) => n.clone(),
                    _ => unreachable!(),
                };
                let line = self.toks[i].1;
                // parse the type through the full type parser so
                // parametric forms (i(N), $fp(4,3)) instantiate here too
                let save = self.pos;
                self.pos = i + 2;
                let ty = self.expect_type().map_err(|mut e| {
                    e.line = line;
                    e
                })?;
                let after = self.pos;
                self.pos = save;
                if scope.value_ids.contains_key(&name) {
                    return Err(ParseError {
                        line,
                        msg: format!("value '%{}' is defined more than once", name),
                    });
                }
                let id = ValueId(scope.values.len() as u32);
                scope.value_ids.insert(name.clone(), id);
                scope.values.push(ValueData {
                    name: name.clone(),
                    ty,
                });
                i = after;
            } else {
                i += 1;
            }
        }
        Ok(scope)
    }

    /// Block headers are `^name` at the start of a line (branch targets never
    /// are). Collect them in order so branches can reference blocks forward.
    fn prescan_blocks(
        &self,
        lo: usize,
        hi: usize,
        scope: &mut FuncScope,
    ) -> Result<(), ParseError> {
        for i in lo..=hi {
            if let Tok::Block(name) = &self.toks[i].0 {
                let at_line_start = i == 0 || matches!(self.toks[i - 1].0, Tok::Newline);
                if at_line_start {
                    if scope.block_ids.contains_key(name) {
                        return Err(ParseError {
                            line: self.toks[i].1,
                            msg: format!("block '^{}' is defined more than once", name),
                        });
                    }
                    let id = BlockId(scope.block_names.len() as u32);
                    scope.block_ids.insert(name.clone(), id);
                    scope.block_names.push(name.clone());
                }
            }
        }
        Ok(())
    }

    // -- grammar -------------------------------------------------------------

    fn parse_function(&mut self) -> Result<Function, ParseError> {
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

        let inst_name = self.inst_name.take();
        let name = match self.next()? {
            Tok::Global(n) | Tok::Ident(n) => n,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a function name, found {}", t)));
            }
        };
        let name = inst_name.unwrap_or(name);

        // parameters: names resolve via the prescan; re-parse for order
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                let id = self.expect_value(&scope)?;
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
        scope.rets = rets.clone();

        self.expect(Tok::LBrace)?;
        self.skip_newlines();

        // A body that opens with statements instead of a ^label is in
        // structured form; it parses via if/loop constructs and lowers to
        // the same block graph on the fly.
        if !matches!(self.peek(), Some(Tok::Block(_)) | Some(Tok::RBrace)) {
            let blocks = self.parse_structured_body(&mut scope)?;
            return Ok(Function {
                name,
                params,
                rets,
                values: scope.values,
                blocks,
                structs: std::rc::Rc::new(Vec::new()), // set by parse()
            });
        }

        let mut blocks: Vec<Block> = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::RBrace) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::Block(_)) => {
                    let Tok::Block(bname) = self.next()? else {
                        unreachable!()
                    };
                    let mut bparams = Vec::new();
                    if self.eat(&Tok::LParen) && !self.eat(&Tok::RParen) {
                        loop {
                            let id = self.expect_value(&scope)?;
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
                    if blocks.is_empty() {
                        return Err(self.err("instruction before the first block label"));
                    }
                    let inst = self.parse_inst(&mut scope)?;
                    let block = blocks.last_mut().unwrap();
                    block.insts.extend(self.pending.drain(..));
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
            structs: std::rc::Rc::new(Vec::new()), // set by parse() at the end
        })
    }

    /// Look up a value being *defined*; the prescan registered it iff the
    /// definition is well-formed (`%name: ty`), so a miss is a syntax error.
    fn def_id(&self, scope: &FuncScope, name: &str) -> Result<ValueId, ParseError> {
        scope.value_ids.get(name).copied().ok_or_else(|| {
            self.err(format!(
                "definition of '%{}' is missing its ': type' annotation",
                name
            ))
        })
    }

    /// A struct field name, or an integer lane index for vectors.
    fn field_or_lane(&mut self, scope: &FuncScope, v: ValueId) -> Result<u16, ParseError> {
        if let Tok::Int(i) = self.next()? {
            if matches!(scope.values[v.0 as usize].ty, Type::Vec(..)) && (0..=u16::MAX as i64).contains(&i) {
                return Ok(i as u16);
            }
            self.pos -= 1;
            return Err(self.err(format!(
                "'%{}' is not a vector value (lane indices index vectors; structs use field names)",
                scope.values[v.0 as usize].name
            )));
        }
        self.pos -= 1;
        let fname = self.expect_ident()?;
        self.field_index(scope, v, &fname)
    }

    fn field_index(
        &self,
        scope: &FuncScope,
        v: ValueId,
        fname: &str,
    ) -> Result<u16, ParseError> {
        let Type::Struct(si) = scope.values[v.0 as usize].ty else {
            return Err(self.err(format!(
                "'%{}' is not a struct value",
                scope.values[v.0 as usize].name
            )));
        };
        let def = &self.structs[si as usize];
        def.fields
            .iter()
            .position(|(n, _)| n == fname)
            .map(|i| i as u16)
            .ok_or_else(|| {
                self.err(format!("struct '${}' has no field '{}'", def.name, fname))
            })
    }

    fn parse_inst(&mut self, scope: &mut FuncScope) -> Result<Inst, ParseError> {
        // a bare identifier followed by ':' opens a definition
        let bare_def = matches!(
            (self.peek(), self.toks.get(self.pos + 1).map(|(t, _)| t)),
            (Some(Tok::Ident(n)), Some(Tok::Colon)) if !Parser::is_reserved(n)
        );
        match self.next()? {
            // dst: ty [, dst2: ty ...] = op ...
            Tok::Value(name) | Tok::Ident(name) if bare_def || matches!(self.toks[self.pos - 1].0, Tok::Value(_)) => {
                let mut dsts = vec![self.def_id(scope, &name)?];
                self.expect(Tok::Colon)?;
                self.expect_type()?;
                while self.eat(&Tok::Comma) {
                    match self.next()? {
                        Tok::Value(n) | Tok::Ident(n) => dsts.push(self.def_id(scope, &n)?),
                        t => {
                            self.pos -= 1;
                            return Err(self.err(format!("expected a value, found {}", t)));
                        }
                    }
                    self.expect(Tok::Colon)?;
                    self.expect_type()?;
                }
                self.expect(Tok::Equals)?;
                // a call is written @f(...); the RHS routes to the direct
                // call form unless operators follow (then it's an
                // expression with a call atom). 'call' stays accepted.
                let callish = matches!(self.peek(), Some(Tok::Global(_)))
                    || matches!(self.peek(), Some(Tok::Ident(k)) if k == "call")
                    || matches!(self.peek(), Some(Tok::Ident(k))
                        if !Parser::is_reserved(k)
                            && !self.wenv.contains_key(k)
                            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen)));
                let wpar =
                    matches!(self.peek(), Some(Tok::Ident(k)) if self.wenv.contains_key(k));
                let bare_val = matches!(self.peek(), Some(Tok::Ident(k))
                    if !Parser::is_reserved(k) && !callish)
                    || matches!(self.peek(), Some(Tok::Ident(k))
                        if k == "float"
                            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen)));
                let expr = (!matches!(self.peek(), Some(Tok::Ident(_))) && !callish)
                    || wpar
                    || bare_val
                    || (callish && dsts.len() == 1 && self.line_has_op());
                let inst = if expr {
                    // expression sugar: %v: ty = %a * %b + 1
                    if dsts.len() != 1 {
                        return Err(self.err("an expression defines exactly one value"));
                    }
                    self.parse_expr_into(dsts[0], scope)?
                } else if callish {
                    if matches!(self.peek(), Some(Tok::Ident(k)) if k == "call") {
                        self.pos += 1; // legacy 'call' keyword
                    }
                    let (callee, args) = self.parse_call_tail(scope)?;
                    Inst::Call { dsts, callee, args }
                } else {
                    let op = self.expect_ident()?;
                    if dsts.len() == 1 {
                        self.parse_def_op(&op, dsts[0], scope)?
                    } else {
                        return Err(self.err(format!(
                            "only a call can define multiple values, not '{}'",
                            op
                        )));
                    }
                };
                self.expect(Tok::Newline)?;
                Ok(inst)
            }
            // op with no result — or a bare call statement name(...)
            Tok::Ident(op) => {
                if !Parser::is_reserved(&op) && matches!(self.peek(), Some(Tok::LParen)) {
                    self.pos -= 1;
                    let (callee, args) = self.parse_call_tail(scope)?;
                    self.expect(Tok::Newline)?;
                    return Ok(Inst::Call {
                        dsts: Vec::new(),
                        callee,
                        args,
                    });
                }
                let inst = self.parse_plain_op(&op, scope)?;
                self.expect(Tok::Newline)?;
                Ok(inst)
            }
            // bare call statement: @f(...) with results ignored
            Tok::Global(_) => {
                self.pos -= 1;
                let (callee, args) = self.parse_call_tail(scope)?;
                self.expect(Tok::Newline)?;
                Ok(Inst::Call {
                    dsts: Vec::new(),
                    callee,
                    args,
                })
            }
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected an instruction, found {}", t)))
            }
        }
    }

    fn parse_def_op(&mut self, op: &str, dst: ValueId, scope: &mut FuncScope) -> Result<Inst, ParseError> {
        // add/sub/mul/div are one opcode each: the result type decides
        // integer or float (rem, and the bitwise ops, are integer-only)
        let poly = {
            let ty = scope.values[dst.0 as usize].ty;
            let floatish = matches!(
                ty,
                Type::F32 | Type::F64 | Type::Float | Type::Scalar | Type::FP(..)
            ) || matches!(ty, Type::Vec(_, VecElem::F32));
            match (op, floatish) {
                ("add", true) => Some(BinOp::FAdd),
                ("add", false) => Some(BinOp::IAdd),
                ("sub", true) => Some(BinOp::FSub),
                ("sub", false) => Some(BinOp::ISub),
                ("mul", true) => Some(BinOp::FMul),
                ("mul", false) => Some(BinOp::IMul),
                ("div", true) => Some(BinOp::FDiv),
                _ => None,
            }
        };
        if let Some(bin) = poly {
            let ty = scope.values[dst.0 as usize].ty;
            let lhs = self.operand(scope, ty)?;
            self.expect(Tok::Comma)?;
            let rhs = self.operand(scope, ty)?;
            return Ok(Inst::Bin {
                op: bin,
                dst,
                lhs,
                rhs,
            });
        }
        if let Some((_, bin)) = BINOPS.iter().find(|(n, _)| *n == op) {
            let ty = scope.values[dst.0 as usize].ty;
            let lhs = self.operand(scope, ty)?;
            self.expect(Tok::Comma)?;
            let rhs = self.operand(scope, ty)?;
            return Ok(Inst::Bin {
                op: *bin,
                dst,
                lhs,
                rhs,
            });
        }
        if let Some(cc) = op.strip_prefix("fcmp.") {
            let cond = FCONDS
                .iter()
                .find(|(n, _)| *n == cc)
                .map(|(_, c)| *c)
                .ok_or_else(|| self.err(format!("unknown float comparison '{}'", cc)))?;
            let (lhs, rhs) = self.cmp_operands(scope)?;
            return Ok(Inst::FCmp {
                cond,
                dst,
                lhs,
                rhs,
            });
        }
        if let Some(cc) = op.strip_prefix("icmp.") {
            let cond = CONDS
                .iter()
                .find(|(n, _)| *n == cc)
                .map(|(_, c)| *c)
                .ok_or_else(|| self.err(format!("unknown comparison condition '{}'", cc)))?;
            let (lhs, rhs) = self.cmp_operands(scope)?;
            return Ok(Inst::ICmp {
                cond,
                dst,
                lhs,
                rhs,
            });
        }
        match op {
            "iconst" => match self.next()? {
                Tok::Int(imm) => Ok(Inst::IConst { dst, imm }),
                t => {
                    self.pos -= 1;
                    Err(self.err(format!("expected an integer literal, found {}", t)))
                }
            },
            "fconst" => match self.next()? {
                Tok::FloatLit(x) => Ok(Inst::FConst {
                    dst,
                    bits: x.to_bits(),
                }),
                Tok::Int(i) => Ok(Inst::FConst {
                    dst,
                    bits: (i as f64).to_bits(),
                }),
                t => {
                    self.pos -= 1;
                    Err(self.err(format!("expected a float literal, found {}", t)))
                }
            },
            "ext" | "trunc" | "itof" | "ftoi" | "fpromote" | "fdemote" | "bitcast" => {
                let cast = match op {
                    "ext" => CastOp::Ext,
                    "trunc" => CastOp::Trunc,
                    "itof" => CastOp::Itof,
                    "ftoi" => CastOp::Ftoi,
                    "fpromote" => CastOp::Fpromote,
                    "fdemote" => CastOp::Fdemote,
                    _ => CastOp::Bitcast,
                };
                let src = self.expect_value_mut(scope)?;
                Ok(Inst::Cast { op: cast, dst, src })
            }
            "load" => {
                let addr = self.expect_value_mut(scope)?;
                Ok(Inst::Load { dst, addr })
            }
            "ptradd" => {
                let base = self.expect_value_mut(scope)?;
                self.expect(Tok::Comma)?;
                let off = self.expect_value_mut(scope)?;
                Ok(Inst::PtrAdd { dst, base, off })
            }
            "extract" => {
                let src = self.expect_value_mut(scope)?;
                self.expect(Tok::Comma)?;
                let field = self.field_or_lane(scope, src)?;
                Ok(Inst::Extract { dst, src, field })
            }
            "pack" => {
                let mut args = vec![self.expect_value_mut(scope)?];
                while self.eat(&Tok::Comma) {
                    args.push(self.expect_value_mut(scope)?);
                }
                Ok(Inst::Pack { dst, args })
            }
            "insert" => {
                let src = self.expect_value_mut(scope)?;
                self.expect(Tok::Comma)?;
                let field = self.field_or_lane(scope, src)?;
                self.expect(Tok::Comma)?;
                let val = self.expect_value_mut(scope)?;
                Ok(Inst::Insert {
                    dst,
                    src,
                    field,
                    val,
                })
            }
            _ => Err(self.err(format!("unknown opcode '{}'", op))),
        }
    }

    fn parse_plain_op(&mut self, op: &str, scope: &mut FuncScope) -> Result<Inst, ParseError> {
        match op {
            "store" => {
                let val = self.expect_value_mut(scope)?;
                self.expect(Tok::Comma)?;
                let addr = self.expect_value_mut(scope)?;
                Ok(Inst::Store { val, addr })
            }
            "call" => {
                let (callee, args) = self.parse_call_tail(scope)?;
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
                let cond = self.expect_value_mut(scope)?;
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
                // `ret @f(...)`: return the callee's results directly
                // (with an operator after, it's an expression instead)
                if (matches!(self.peek(), Some(Tok::Global(_)))
                    || matches!(self.peek(), Some(Tok::Ident(k)) if k == "call")
                    || matches!(self.peek(), Some(Tok::Ident(k))
                        if !Parser::is_reserved(k)
                            && !scope.value_ids.contains_key(k.as_str())
                            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen))))
                    && !self.line_has_op()
                {
                    if matches!(self.peek(), Some(Tok::Ident(k)) if k == "call") {
                        self.pos += 1;
                    }
                    let rets = scope.rets.clone();
                    let (callee, args) = self.parse_call_tail(scope)?;
                    let dsts: Vec<ValueId> =
                        rets.iter().map(|&t| self.temp_val(scope, t)).collect();
                    self.pending.push(Inst::Call {
                        dsts: dsts.clone(),
                        callee,
                        args,
                    });
                    return Ok(Inst::Ret { vals: dsts });
                }
                let rets = scope.rets.clone();
                let vals = self.parse_value_list_t(scope, &rets)?;
                Ok(Inst::Ret { vals })
            }
            _ => Err(self.err(format!(
                "unknown opcode '{}' (or missing '%dst: ty =' before it)",
                op
            ))),
        }
    }

    fn parse_call_tail(
        &mut self,
        scope: &mut FuncScope,
    ) -> Result<(String, Vec<ValueId>), ParseError> {
        let callee = match self.next()? {
            Tok::Global(n) | Tok::Ident(n) => n,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a function name, found {}", t)));
            }
        };
        // width-generic callee: @f(args) infers parameters from argument
        // types; @f(4, 3)(args) states them explicitly (required when the
        // signature can't be inverted, e.g. a u(E+M+1) argument)
        if self.gfns.contains_key(&callee) {
            if self.explicit_widths_ahead() {
                let vals = self.parse_width_args()?;
                let mangled = self.request_fn_inst(&callee, &vals)?;
                let ptys = self.sigs[&mangled].0.clone();
                self.expect(Tok::LParen)?;
                let mut args = Vec::new();
                if !self.eat(&Tok::RParen) {
                    loop {
                        match self.peek() {
                            Some(Tok::Int(_)) | Some(Tok::FloatLit(_)) => {
                                let Some(&ty) = ptys.get(args.len()) else {
                                    return Err(self.err(format!(
                                        "too many arguments to @{}",
                                        mangled
                                    )));
                                };
                                let tok = self.next()?.clone();
                                args.push(self.synth_lit(scope, ty, &tok)?);
                            }
                            _ => args.push(self.expect_value_mut(scope)?),
                        }
                        if self.eat(&Tok::RParen) {
                            break;
                        }
                        self.expect(Tok::Comma)?;
                    }
                }
                return Ok((mangled, args));
            }
            self.expect(Tok::LParen)?;
            let mut args = Vec::new();
            if !self.eat(&Tok::RParen) {
                loop {
                    if matches!(self.peek(), Some(Tok::Int(_) | Tok::FloatLit(_))) {
                        return Err(self.err(format!(
                            "@{} is width-generic: literal arguments cannot drive                              inference; pass typed values",
                            callee
                        )));
                    }
                    args.push(self.expect_value_mut(scope)?);
                    if self.eat(&Tok::RParen) {
                        break;
                    }
                    self.expect(Tok::Comma)?;
                }
            }
            let mangled = self.solve_generic_call(&callee, &args, scope)?;
            return Ok((mangled, args));
        }
        let ptys = self.sigs.get(&callee).map(|(p, _)| p.clone());
        self.expect(Tok::LParen)?;
        let mut args = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                match self.peek() {
                    Some(Tok::Int(_)) | Some(Tok::FloatLit(_)) => {
                        let Some(&ty) = ptys.as_ref().and_then(|p| p.get(args.len())) else {
                            return Err(self.err(format!(
                                "a literal argument needs @{}'s signature (unknown or                                  too few parameters)",
                                callee
                            )));
                        };
                        let tok = self.next()?.clone();
                        args.push(self.synth_lit(scope, ty, &tok)?);
                    }
                    _ => args.push(self.expect_value_mut(scope)?),
                }
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
            yield_tys: Vec::new(),
        };
        st.new_block(Vec::new()); // ^entry
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
        let bare_def = matches!(
            (self.peek(), self.toks.get(self.pos + 1).map(|(t, _)| t)),
            (Some(Tok::Ident(n)), Some(Tok::Colon)) if !Parser::is_reserved(n)
        );
        match self.next()? {
            Tok::Value(name) | Tok::Ident(name)
                if bare_def || matches!(self.toks[self.pos - 1].0, Tok::Value(_)) =>
            {
                let mut dsts = vec![self.def_id(scope, &name)?];
                self.expect(Tok::Colon)?;
                self.expect_type()?;
                while self.eat(&Tok::Comma) {
                    match self.next()? {
                        Tok::Value(n) | Tok::Ident(n) => dsts.push(self.def_id(scope, &n)?),
                        t => {
                            self.pos -= 1;
                            return Err(self.err(format!("expected a value, found {}", t)));
                        }
                    }
                    self.expect(Tok::Colon)?;
                    self.expect_type()?;
                }
                self.expect(Tok::Equals)?;
                let callish = matches!(self.peek(), Some(Tok::Global(_)))
                    || matches!(self.peek(), Some(Tok::Ident(k)) if k == "call")
                    || matches!(self.peek(), Some(Tok::Ident(k))
                        if !Parser::is_reserved(k)
                            && !self.wenv.contains_key(k)
                            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen)));
                let wpar =
                    matches!(self.peek(), Some(Tok::Ident(k)) if self.wenv.contains_key(k));
                let bare_val = matches!(self.peek(), Some(Tok::Ident(k))
                    if !Parser::is_reserved(k) && !callish)
                    || matches!(self.peek(), Some(Tok::Ident(k))
                        if k == "float"
                            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen)));
                let expr = (!matches!(self.peek(), Some(Tok::Ident(_))) && !callish)
                    || wpar
                    || bare_val
                    || (callish && dsts.len() == 1 && self.line_has_op());
                if callish && !expr {
                    if matches!(self.peek(), Some(Tok::Ident(k)) if k == "call") {
                        self.pos += 1; // legacy 'call' keyword
                    }
                    let (callee, args) = self.parse_call_tail(scope)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(Inst::Call { dsts, callee, args });
                    self.end_stmt()?;
                    return Ok(false);
                }
                if expr {
                    if dsts.len() != 1 {
                        return Err(self.err("an expression defines exactly one value"));
                    }
                    let inst = self.parse_expr_into(dsts[0], scope)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(inst);
                    self.end_stmt()?;
                    return Ok(false);
                }
                let op = self.expect_ident()?;
                match op.as_str() {
                    "if" => return self.parse_struct_if(scope, st, dsts),
                    "loop" => return self.parse_struct_loop(scope, st, dsts),
                    "call" => {
                        let (callee, args) = self.parse_call_tail(scope)?;
                        st.push(Inst::Call { dsts, callee, args });
                    }
                    _ => {
                        if dsts.len() > 1 {
                            return Err(self.err(format!(
                                "only 'call' can define multiple values, not '{}'",
                                op
                            )));
                        }
                        let inst = self.parse_def_op(&op, dsts[0], scope)?;
                        for p in self.pending.drain(..) {
                            st.push(p);
                        }
                        st.push(inst);
                    }
                }
                self.end_stmt()?;
                Ok(false)
            }
            // bare call statement: @f(...) with results ignored
            Tok::Global(_) => {
                self.pos -= 1;
                let (callee, args) = self.parse_call_tail(scope)?;
                for p in self.pending.drain(..) {
                    st.push(p);
                }
                st.push(Inst::Call {
                    dsts: Vec::new(),
                    callee,
                    args,
                });
                self.end_stmt()?;
                Ok(false)
            }
            Tok::Ident(op) => match op.as_str() {
                "if" => self.parse_struct_if(scope, st, Vec::new()),
                "loop" => self.parse_struct_loop(scope, st, Vec::new()),
                "break" => {
                    let tys = match st.loop_stack.last() {
                        Some(f) => f.res_tys.clone(),
                        None => return Err(self.err("'break' outside a loop")),
                    };
                    let vals = self.parse_value_list_t(scope, &tys)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    let at = st.push(Inst::Jmp {
                        target: DUMMY_BLOCK,
                        args: vals,
                    });
                    st.loop_stack.last_mut().unwrap().breaks.push(at);
                    self.end_stmt()?;
                    Ok(true)
                }
                "continue" => {
                    let (header, tys) = match st.loop_stack.last() {
                        Some(f) => (f.header, f.var_tys.clone()),
                        None => return Err(self.err("'continue' outside a loop")),
                    };
                    let vals = self.parse_value_list_t(scope, &tys)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(Inst::Jmp {
                        target: header,
                        args: vals,
                    });
                    self.end_stmt()?;
                    Ok(true)
                }
                "yield" => {
                    let tys = match st.yield_tys.last() {
                        Some(t) => t.clone(),
                        None => return Err(self.err("'yield' outside an if")),
                    };
                    let vals = self.parse_value_list_t(scope, &tys)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    let at = st.push(Inst::Jmp {
                        target: DUMMY_BLOCK,
                        args: vals,
                    });
                    st.yield_stack.last_mut().unwrap().push(at);
                    self.end_stmt()?;
                    Ok(true)
                }
                "ret" => {
                    // `ret @f(...)`: return the callee's results
                    // (with an operator after, it's an expression instead)
                    if (matches!(self.peek(), Some(Tok::Global(_)))
                        || matches!(self.peek(), Some(Tok::Ident(k)) if k == "call")
                        || matches!(self.peek(), Some(Tok::Ident(k))
                            if !Parser::is_reserved(k)
                                && !scope.value_ids.contains_key(k.as_str())
                                && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LParen))))
                        && !self.line_has_op()
                    {
                        if matches!(self.peek(), Some(Tok::Ident(k)) if k == "call") {
                            self.pos += 1;
                        }
                        let rets = scope.rets.clone();
                        let (callee, args) = self.parse_call_tail(scope)?;
                        let dsts: Vec<ValueId> =
                            rets.iter().map(|&t| self.temp_val(scope, t)).collect();
                        for p in self.pending.drain(..) {
                            st.push(p);
                        }
                        st.push(Inst::Call {
                            dsts: dsts.clone(),
                            callee,
                            args,
                        });
                        st.push(Inst::Ret { vals: dsts });
                        self.end_stmt()?;
                        return Ok(true);
                    }
                    let rets = scope.rets.clone();
                    let vals = self.parse_value_list_t(scope, &rets)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(Inst::Ret { vals });
                    self.end_stmt()?;
                    Ok(true)
                }
                "store" | "call" => {
                    let inst = self.parse_plain_op(&op, scope)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(inst);
                    self.end_stmt()?;
                    Ok(false)
                }
                "jmp" | "br" => Err(self.err(
                    "jmp/br are not allowed in a structured function; use if/loop/break/continue",
                )),
                _ if matches!(self.peek(), Some(Tok::LParen)) => {
                    // bare call statement: name(...) with results ignored
                    self.pos -= 1;
                    let (callee, args) = self.parse_call_tail(scope)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(Inst::Call {
                        dsts: Vec::new(),
                        callee,
                        args,
                    });
                    self.end_stmt()?;
                    Ok(false)
                }
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
        // `if %c { ... }`, or a comparison condition: `if %i >= %n { ... }`
        let cond = if matches!(self.peek(), Some(Tok::Value(_)))
            && matches!(self.toks.get(self.pos + 1).map(|(t, _)| t), Some(Tok::LBrace))
        {
            self.expect_value_mut(scope)?
        } else {
            match self.expr_level(scope, 0)? {
                EAst::V(id) => id,
                EAst::Cmp(sym, l, r) => {
                    let t = self.temp_val(scope, Type::U(1));
                    let inst = self.emit_cmp(scope, sym, *l, *r, t)?;
                    self.pending.push(inst);
                    t
                }
                _ => {
                    return Err(self.err(
                        "an if condition must be a value or a comparison".to_string(),
                    ))
                }
            }
        };
        for p in self.pending.drain(..) {
            st.push(p);
        }
        self.expect(Tok::LBrace)?;
        self.eat(&Tok::Newline);

        let before = st.cur;
        let then_b = st.new_block(Vec::new());
        st.yield_stack.push(Vec::new()); // collects edges into the join
        st.yield_tys.push(
            dsts.iter()
                .map(|d| scope.values[d.0 as usize].ty)
                .collect(),
        );

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
            st.yield_stack.last_mut().unwrap().push(at);
        }

        if matches!(self.peek(), Some(Tok::Ident(k)) if k == "else") {
            self.pos += 1;
            self.expect(Tok::LBrace)?;
            self.eat(&Tok::Newline);
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
                st.yield_stack.last_mut().unwrap().push(at);
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
            st.yield_stack.last_mut().unwrap().push(at);
        }
        self.end_stmt()?;

        st.yield_tys.pop();
        let pending = st.yield_stack.pop().unwrap();
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
        let mut var_tys = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                match self.next()? {
                    Tok::Value(n) | Tok::Ident(n) => params.push(self.def_id(scope, &n)?),
                    t => {
                        self.pos -= 1;
                        return Err(self.err(format!("expected a loop variable, found {}", t)));
                    }
                }
                self.expect(Tok::Colon)?;
                let vty = self.expect_type()?;
                var_tys.push(vty);
                self.expect(Tok::Equals)?;
                inits.push(self.operand(scope, vty)?);
                for p in self.pending.drain(..) {
                    st.push(p);
                }
                if self.eat(&Tok::RParen) {
                    break;
                }
                self.expect(Tok::Comma)?;
            }
        }
        self.expect(Tok::LBrace)?;
        self.eat(&Tok::Newline);

        let header = st.new_block(params);
        st.push(Inst::Jmp {
            target: header,
            args: inits,
        });
        st.loop_stack.push(LoopFrame {
            header,
            breaks: Vec::new(),
            var_tys,
            res_tys: dsts
                .iter()
                .map(|d| scope.values[d.0 as usize].ty)
                .collect(),
        });
        st.cur = header.0 as usize;
        let terminated = self.parse_struct_stmts(scope, st)?;
        self.expect(Tok::RBrace)?;
        self.end_stmt()?;
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

    /// a value list where literals are allowed, typed positionally (loop
    /// results for break, loop vars for continue, if results for yield,
    /// the function's return types for ret)
    fn parse_value_list_t(
        &mut self,
        scope: &mut FuncScope,
        tys: &[Type],
    ) -> Result<Vec<ValueId>, ParseError> {
        let mut vals = Vec::new();
        loop {
            match self.peek() {
                Some(
                    Tok::Value(_)
                    | Tok::Int(_)
                    | Tok::FloatLit(_)
                    | Tok::LParen
                    | Tok::Ident(_)
                    | Tok::Global(_),
                ) => {
                    // each element is a full expression (a bare value is
                    // the degenerate case); literals and temps take the
                    // positional type
                    match self.expr_level(scope, 0)? {
                        EAst::V(id) => vals.push(id),
                        ast => {
                            let Some(&ty) = tys.get(vals.len()) else {
                                return Err(self.err(
                                    "no declared type for an expression in this position"
                                        .to_string(),
                                ));
                            };
                            let id = self.emit_ast(scope, ty, ast)?;
                            vals.push(id);
                        }
                    }
                }
                _ if vals.is_empty() => break,
                _ => return Err(self.err("expected a value or expression".to_string())),
            }
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(vals)
    }

    /// emit any expression AST to a fresh value of the given type
    fn emit_ast(
        &mut self,
        scope: &mut FuncScope,
        ty: Type,
        ast: EAst,
    ) -> Result<ValueId, ParseError> {
        match ast {
            EAst::Cmp(sym, l, r) => {
                let t = self.temp_val(scope, Type::U(1));
                let inst = self.emit_cmp(scope, sym, *l, *r, t)?;
                self.pending.push(inst);
                Ok(t)
            }
            ast => self.emit_expr(scope, ty, ast),
        }
    }

    fn parse_branch_target(
        &mut self,
        scope: &FuncScope,
    ) -> Result<(BlockId, Vec<ValueId>), ParseError> {
        let target = match self.next()? {
            Tok::Block(name) => scope.block_ids.get(&name).copied().ok_or_else(|| {
                self.pos -= 1;
                self.err(format!("branch to undefined block '^{}'", name))
            })?,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a block, found {}", t)));
            }
        };
        let mut args = Vec::new();
        if self.eat(&Tok::LParen) && !self.eat(&Tok::RParen) {
            loop {
                args.push(self.expect_value(scope)?);
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
        for def in self.structs.iter() {
            let fields: Vec<String> = def
                .fields
                .iter()
                .map(|(n, t)| format!("{}: {}", n, t.name()))
                .collect();
            writeln!(f, "type ${} = {{ {} }}", def.name, fields.join(", "))?;
        }
        if !self.structs.is_empty() {
            writeln!(f)?;
        }
        for (i, func) in self.funcs.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}", func)?;
        }
        Ok(())
    }
}

impl Function {
    /// type name with struct names resolved through this function's table
    pub fn ty_name(&self, ty: Type) -> String {
        match ty {
            Type::Struct(i) => format!("${}", self.structs[i as usize].name),
            t => t.name(),
        }
    }

    fn fmt_value(&self, id: ValueId) -> String {
        fmt_name(&self.value(id).name)
    }

    fn fmt_def(&self, id: ValueId) -> String {
        let v = self.value(id);
        format!("{}: {}", fmt_name(&v.name), self.ty_name(v.ty))
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
            format!("^{}", name)
        } else {
            format!("^{}({})", name, self.fmt_args(args))
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
        write!(f, "fn {}({})", fmt_fn_name(&self.name), params)?;
        match self.rets.len() {
            0 => {}
            1 => write!(f, " -> {}", self.ty_name(self.rets[0]))?,
            _ => {
                let ts: Vec<String> = self.rets.iter().map(|&t| self.ty_name(t)).collect();
                write!(f, " -> ({})", ts.join(", "))?;
            }
        }
        writeln!(f, " {{")?;
        for block in &self.blocks {
            if block.params.is_empty() {
                writeln!(f, "^{}:", block.name)?;
            } else {
                let ps = block
                    .params
                    .iter()
                    .map(|&p| self.fmt_def(p))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(f, "^{}({}):", block.name, ps)?;
            }
            for inst in &block.insts {
                writeln!(f, "    {}", self.fmt_inst(inst))?;
            }
        }
        writeln!(f, "}}")
    }
}

impl Function {
    fn fmt_inst(&self, inst: &Inst) -> String {
        match inst {
            Inst::IConst { dst, imm } => format!("{} = iconst {}", self.fmt_def(*dst), imm),
            Inst::FConst { dst, bits } => {
                format!("{} = fconst {:?}", self.fmt_def(*dst), f64::from_bits(*bits))
            }
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
                "{} = icmp.{} {}, {}",
                self.fmt_def(*dst),
                cond.name(),
                self.fmt_value(*lhs),
                self.fmt_value(*rhs)
            ),
            Inst::FCmp {
                cond,
                dst,
                lhs,
                rhs,
            } => format!(
                "{} = fcmp.{} {}, {}",
                self.fmt_def(*dst),
                cond.name(),
                self.fmt_value(*lhs),
                self.fmt_value(*rhs)
            ),
            Inst::Cast { op, dst, src } => format!(
                "{} = {} {}",
                self.fmt_def(*dst),
                op.name(),
                self.fmt_value(*src)
            ),
            Inst::Load { dst, addr } => {
                format!("{} = load {}", self.fmt_def(*dst), self.fmt_value(*addr))
            }
            Inst::Store { val, addr } => {
                format!("store {}, {}", self.fmt_value(*val), self.fmt_value(*addr))
            }
            Inst::PtrAdd { dst, base, off } => format!(
                "{} = ptradd {}, {}",
                self.fmt_def(*dst),
                self.fmt_value(*base),
                self.fmt_value(*off)
            ),
            Inst::Extract { dst, src, field } => {
                let fname = match self.ty(*src) {
                    Type::Struct(i) => self.structs[i as usize].fields[*field as usize].0.clone(),
                    _ => format!("{}", field),
                };
                format!(
                    "{} = extract {}, {}",
                    self.fmt_def(*dst),
                    self.fmt_value(*src),
                    fname
                )
            }
            Inst::Pack { dst, args } => {
                format!("{} = pack {}", self.fmt_def(*dst), self.fmt_args(args))
            }
            Inst::Insert {
                dst,
                src,
                field,
                val,
            } => {
                let fname = match self.ty(*src) {
                    Type::Struct(i) => self.structs[i as usize].fields[*field as usize].0.clone(),
                    _ => format!("{}", field),
                };
                format!(
                    "{} = insert {}, {}, {}",
                    self.fmt_def(*dst),
                    self.fmt_value(*src),
                    fname,
                    self.fmt_value(*val)
                )
            }
            Inst::Call { dsts, callee, args } => {
                let call = format!("{}({})", fmt_fn_name(callee), self.fmt_args(args));
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
    // struct layouts must be fully resolved and fit a 64-bit carrier
    // (abstract fields defer this check from parse to here)
    for d in module.structs.iter() {
        if d.fields.iter().any(|(_, t)| t.width().is_none()) {
            errs.push(format!(
                "struct '${}' has an unresolved field type (run type resolution first)",
                d.name
            ));
        } else if d.total_bits() == 0 || d.word_layout().0 > 8 {
            errs.push(format!(
                "struct '${}' resolves to {} bits; 1 bit to 8 words (512 bits) required",
                d.name,
                d.total_bits()
            ));
        }
    }
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
    let ctx = |msg: String| format!("@{}: {}", func.name, msg);

    if func.blocks.is_empty() {
        errs.push(ctx("function has no blocks".into()));
        return;
    }

    // rule 0: abstract types are resolved before verification
    for v in &func.values {
        if matches!(
            v.ty,
            Type::Int | Type::Uint | Type::Float | Type::Scalar | Type::Half | Type::UHalf
        ) {
            errs.push(ctx(format!(
                "value '%{}' has unresolved abstract type '{}' (run type resolution first)",
                v.name,
                v.ty.name()
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
                "value '%{}' is defined more than once",
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
        let bctx = |msg: String| ctx(format!("^{}: {}", block.name, msg));
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
        | Inst::Load { dst, .. }
        | Inst::PtrAdd { dst, .. } => vec![*dst],
        Inst::Call { dsts, .. } => dsts.clone(),
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
    let ctx = |msg: String| format!("@{}: ^{}: {}", func.name, block.name, msg);
    let name = |id: ValueId| format!("%{}", func.value(id).name);

    match inst {
        Inst::IConst { dst, imm } => {
            let ty = func.ty(*dst);
            let ok = match ty {
                // ptr constants are raw addresses (MMIO, fixed buffers) —
                // meaningful wherever ptr is an address-space index
                Type::I(64) | Type::U(64) | Type::Ptr => true,
                // accept the value written as signed or as the unsigned
                // bit pattern of the width; canonicalization is lowering's
                Type::I(n) | Type::U(n) => {
                    let n = n as u32;
                    *imm >= -(1i64 << (n - 1)) && *imm < (1i64 << n)
                }
                Type::F32 | Type::F64 | Type::FP(..) => false, // use fconst
                Type::Struct(_) | Type::Vec(..) => false, // use pack
                Type::Int | Type::Uint | Type::Float | Type::Scalar | Type::Half
                | Type::UHalf => {
                    unreachable!("rejected by rule 0")
                }
            };
            if !ok {
                errs.push(ctx(format!(
                    "iconst {} does not fit in type {}",
                    imm,
                    ty.name()
                )));
            }
        }
        Inst::FConst { dst, .. } => {
            if !func.ty(*dst).is_float() {
                errs.push(ctx(format!(
                    "fconst result must be f32/f64, not {}",
                    func.ty(*dst).name()
                )));
            }
        }
        Inst::FCmp {
            cond,
            dst,
            lhs,
            rhs,
        } => {
            let (td, tl, tr) = (func.ty(*dst), func.ty(*lhs), func.ty(*rhs));
            if td != Type::U(1) {
                errs.push(ctx(format!(
                    "fcmp.{} result {} must be u1",
                    cond.name(),
                    name(*dst)
                )));
            }
            if tl != tr || !tl.is_float() {
                errs.push(ctx(format!(
                    "fcmp.{}: operands must share a float type; got {} and {}",
                    cond.name(),
                    tl.name(),
                    tr.name()
                )));
            }
        }
        Inst::Bin { op, dst, lhs, rhs } if op.is_float() => {
            let (td, tl, tr) = (func.ty(*dst), func.ty(*lhs), func.ty(*rhs));
            if !td.elem_or_self().is_float() || tl != td || tr != td {
                errs.push(ctx(format!(
                    "{}: operands and result must share a float type; got {}, {}, {}",
                    op.name(),
                    td.name(),
                    tl.name(),
                    tr.name()
                )));
            }
        }
        Inst::Bin { op, dst, lhs, rhs } => {
            let (td, tl, tr) = (func.ty(*dst), func.ty(*lhs), func.ty(*rhs));
            if !td.elem_or_self().is_arith() || tl != td || tr != td {
                errs.push(ctx(format!(
                    "{}: operands and result must share an arithmetic type; got {}: {}, {}: {}, {}: {}",
                    op.name(),
                    name(*dst),
                    td.name(),
                    name(*lhs),
                    tl.name(),
                    name(*rhs),
                    tr.name()
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
            if td != Type::U(1) {
                errs.push(ctx(format!(
                    "icmp.{} result {} must be u1, not {}",
                    cond.name(),
                    name(*dst),
                    td.name()
                )));
            }
            if tl != tr || !(tl.width().is_some() || tl == Type::Ptr) {
                errs.push(ctx(format!(
                    "icmp.{}: operands must share an integer type or ptr; got {} and {}",
                    cond.name(),
                    tl.name(),
                    tr.name()
                )));
            }
        }
        Inst::Cast { op, dst, src } => {
            let (td, ts) = (func.ty(*dst), func.ty(*src));
            let int_wide = |t: Type| matches!(t.width(), Some(32) | Some(64));
            let ok = match op {
                CastOp::Ext | CastOp::Trunc => match (ts.width(), td.width()) {
                    (Some(ws), Some(wd)) => match op {
                        CastOp::Ext => wd > ws,
                        _ => wd < ws,
                    },
                    _ => false,
                },
                CastOp::Itof => int_wide(ts) && td.is_float(),
                CastOp::Ftoi => ts.is_float() && int_wide(td),
                CastOp::Fpromote => {
                    let em = |t: Type| match t {
                        Type::F32 => Some((8i32, 23i32)),
                        Type::F64 => Some((11, 52)),
                        Type::FP(e, m) => Some((e as i32, m as i32)),
                        _ => None,
                    };
                    match (em(ts), em(td)) {
                        (Some((es, ms)), Some((ed, md))) => {
                            ts != td && es <= ed && ms <= md
                        }
                        _ => false,
                    }
                }
                CastOp::Fdemote => {
                    let em = |t: Type| match t {
                        Type::F32 => Some((8i32, 23i32)),
                        Type::F64 => Some((11, 52)),
                        Type::FP(e, m) => Some((e as i32, m as i32)),
                        _ => None,
                    };
                    match (em(ts), em(td)) {
                        (Some((es, ms)), Some((ed, md))) => {
                            ts != td && es >= ed && ms >= md
                        }
                        _ => false,
                    }
                }
                CastOp::Bitcast => {
                    // same total width, different type: signedness flips,
                    // int<->float, int<->struct
                    let w = |t: Type| match t {
                        Type::F32 => Some(32),
                        Type::F64 => Some(64),
                        Type::Struct(i) => {
                            let d = &func.structs[i as usize];
                            if d.word_layout().0 == 1 {
                                Some(d.total_bits())
                            } else {
                                None // multi-word structs move by parts
                            }
                        }
                        Type::Vec(n, e) => Some(n as u32 * e.bits()),
                        Type::FP(e, m) => Some(e as u32 + m as u32 + 1),
                        t => t.width(),
                    };
                    ts != td && w(ts).is_some() && w(ts) == w(td)
                }
            };
            if !ok {
                errs.push(ctx(format!(
                    "{} from {} to {} is not valid",
                    op.name(),
                    ts.name(),
                    td.name()
                )));
            }
        }
        Inst::Load { dst, addr } => {
            if func.ty(*addr) != Type::Ptr {
                errs.push(ctx(format!("load address {} must be ptr", name(*addr))));
            }
            if !func.ty(*dst).is_memory() {
                errs.push(ctx(format!(
                    "load result must be i32/i64/ptr, not {}",
                    func.ty(*dst).name()
                )));
            }
        }
        Inst::Store { val, addr } => {
            if func.ty(*addr) != Type::Ptr {
                errs.push(ctx(format!("store address {} must be ptr", name(*addr))));
            }
            if !func.ty(*val).is_memory() {
                errs.push(ctx(format!(
                    "stored value must be i32/i64/ptr, not {}",
                    func.ty(*val).name()
                )));
            }
        }
        Inst::PtrAdd { dst, base, off } => {
            let offw = func.ty(*off).width();
            if func.ty(*dst) != Type::Ptr
                || func.ty(*base) != Type::Ptr
                || offw != Some(64)
            {
                errs.push(ctx(
                    "ptradd requires result: ptr, base: ptr, offset: 64-bit int".into(),
                ));
            }
        }
        Inst::Call { dsts, callee, args } => {
            if let Some(target) = module.func(callee) {
                let want: Vec<Type> = target.params.iter().map(|&p| target.ty(p)).collect();
                let got: Vec<Type> = args.iter().map(|&a| func.ty(a)).collect();
                if want != got {
                    errs.push(ctx(format!(
                        "call @{}: argument types ({}) do not match parameters ({})",
                        callee,
                        got.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "),
                        want.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
                    )));
                }
                // results may bind all of the callee's return values or none
                if !dsts.is_empty() {
                    let dt: Vec<Type> = dsts.iter().map(|&d| func.ty(d)).collect();
                    if dt != target.rets {
                        errs.push(ctx(format!(
                            "call @{}: result types ({}) do not match return types ({})",
                            callee,
                            dt.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "),
                            target
                                .rets
                                .iter()
                                .map(|t| t.name())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                }
            }
        }
        Inst::Extract { dst, src, field } => {
            match func.ty(*src) {
                Type::Struct(i) => {
                    let def = &func.structs[i as usize];
                    match def.fields.get(*field as usize) {
                        Some((fname, fty)) => {
                            if func.ty(*dst) != *fty {
                                errs.push(ctx(format!(
                                    "extract {}: result must be {}, not {}",
                                    fname,
                                    func.ty_name(*fty),
                                    func.ty_name(func.ty(*dst))
                                )));
                            }
                        }
                        None => errs.push(ctx("extract: field index out of range".into())),
                    }
                }
                Type::Vec(lanes, elem) => {
                    if *field >= lanes as u16 {
                        errs.push(ctx("extract: lane index out of range".into()));
                    } else if !lane_scalar_ok(elem, func.ty(*dst)) {
                        errs.push(ctx(format!(
                            "extract lane {}: result must be {}, not {}",
                            field,
                            elem.ty().name(),
                            func.ty(*dst).name()
                        )));
                    }
                }
                t => errs.push(ctx(format!(
                    "extract source must be a struct or vector, not {}",
                    func.ty_name(t)
                ))),
            }
        }
        Inst::Pack { dst, args } => match func.ty(*dst) {
            Type::Struct(i) => {
                let def = &func.structs[i as usize];
                if args.len() != def.fields.len() {
                    errs.push(ctx(format!(
                        "pack into ${}: {} fields required, {} given",
                        def.name,
                        def.fields.len(),
                        args.len()
                    )));
                } else {
                    for (a, (fname, fty)) in args.iter().zip(&def.fields) {
                        if func.ty(*a) != *fty {
                            errs.push(ctx(format!(
                                "pack field {}: expected {}, got {}",
                                fname,
                                func.ty_name(*fty),
                                func.ty_name(func.ty(*a))
                            )));
                        }
                    }
                }
            }
            Type::Vec(lanes, elem) => {
                if args.len() != lanes as usize {
                    errs.push(ctx(format!(
                        "pack into {}: {} lanes required, {} given",
                        func.ty(*dst).name(),
                        lanes,
                        args.len()
                    )));
                } else {
                    for (i, a) in args.iter().enumerate() {
                        if !lane_scalar_ok(elem, func.ty(*a)) {
                            errs.push(ctx(format!(
                                "pack lane {}: expected {}, got {}",
                                i,
                                elem.ty().name(),
                                func.ty(*a).name()
                            )));
                        }
                    }
                }
            }
            t => errs.push(ctx(format!(
                "pack result must be a struct or vector, not {}",
                func.ty_name(t)
            ))),
        },
        Inst::Insert {
            dst,
            src,
            field,
            val,
        } => match func.ty(*src) {
            Type::Struct(i) => {
                let def = &func.structs[i as usize];
                if func.ty(*dst) != func.ty(*src) {
                    errs.push(ctx("insert result must have the source's struct type".into()));
                }
                match def.fields.get(*field as usize) {
                    Some((fname, fty)) => {
                        if func.ty(*val) != *fty {
                            errs.push(ctx(format!(
                                "insert {}: value must be {}, not {}",
                                fname,
                                func.ty_name(*fty),
                                func.ty_name(func.ty(*val))
                            )));
                        }
                    }
                    None => errs.push(ctx("insert: field index out of range".into())),
                }
            }
            Type::Vec(lanes, elem) => {
                if func.ty(*dst) != func.ty(*src) {
                    errs.push(ctx("insert result must have the source's vector type".into()));
                }
                if *field >= lanes as u16 {
                    errs.push(ctx("insert: lane index out of range".into()));
                } else if !lane_scalar_ok(elem, func.ty(*val)) {
                    errs.push(ctx(format!(
                        "insert lane {}: value must be {}, not {}",
                        field,
                        elem.ty().name(),
                        func.ty(*val).name()
                    )));
                }
            }
            t => errs.push(ctx(format!(
                "insert source must be a struct or vector, not {}",
                func.ty_name(t)
            ))),
        },
        Inst::Jmp { .. } | Inst::Br { .. } => {
            if let Inst::Br { cond, .. } = inst {
                if func.ty(*cond).width() != Some(1) {
                    errs.push(ctx(format!(
                        "br condition {} must be one bit wide, not {}",
                        name(*cond),
                        func.ty(*cond).name()
                    )));
                }
            }
            for (target, args) in branch_targets(inst) {
                let tblock = &func.blocks[target.0 as usize];
                let want: Vec<Type> = tblock.params.iter().map(|&p| func.ty(p)).collect();
                let got: Vec<Type> = args.iter().map(|&a| func.ty(a)).collect();
                if want != got {
                    errs.push(ctx(format!(
                        "branch to ^{}: argument types ({}) do not match block parameters ({})",
                        tblock.name,
                        got.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "),
                        want.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
                    )));
                }
            }
        }
        Inst::Ret { vals } => {
            let got: Vec<Type> = vals.iter().map(|&v| func.ty(v)).collect();
            if got != func.rets {
                errs.push(ctx(format!(
                    "ret types ({}) do not match the function's return types ({})",
                    got.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "),
                    func.rets
                        .iter()
                        .map(|t| t.name())
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
fn @sum(%n: i64) -> i64 {
^entry:
    %zero: i64 = iconst 0
    jmp ^loop(%zero, %zero)
^loop(%i: i64, %acc: i64):
    %done: u1 = icmp.ge %i, %n
    br %done, ^exit, ^body
^body:
    %acc2: i64 = iadd %acc, %i
    %one: i64 = iconst 1
    %i2: i64 = iadd %i, %one
    jmp ^loop(%i2, %acc2)
^exit:
    ret %acc
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
        assert_eq!(f.rets, vec![Type::I(64)]);
        assert_eq!(f.blocks.len(), 4);
        assert_eq!(f.blocks[1].params.len(), 2);
        assert!(matches!(f.blocks[1].insts.last(), Some(Inst::Br { .. })));
    }

    #[test]
    fn calls_memory_and_casts() {
        let src = r"
fn @get(%p: ptr) -> i32 {
^entry:
    %v: i32 = load %p
    ret %v
}
fn @use(%p: ptr) -> i64 {
^entry:
    %v: i32 = call @get(%p)
    %w: i64 = ext %v
    %eight: i64 = iconst 8
    %q: ptr = ptradd %p, %eight
    store %w, %q
    call @touch(%q)
    ret %w
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
fn @bad(%a: i64, %b: i32) -> i64 {
^entry:
    %c: i64 = iadd %a, %b
    ret %c
}
";
        let m = parse(src).expect("parse succeeds; verify should fail");
        let errs = verify(&m).unwrap_err();
        assert!(errs[0].contains("add"), "got: {:?}", errs);
    }

    #[test]
    fn rejects_branch_arg_mismatch() {
        let src = r"
fn @bad(%a: i64) -> i64 {
^entry:
    jmp ^next(%a)
^next(%x: i32):
    ret %a
}
";
        let m = parse(src).expect("parse");
        let errs = verify(&m).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("branch to ^next")), "got: {:?}", errs);
    }

    #[test]
    fn rejects_missing_terminator() {
        let src = r"
fn @bad(%a: i64) -> i64 {
^entry:
    %b: i64 = iadd %a, %a
}
";
        let m = parse(src).expect("parse");
        let errs = verify(&m).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("terminator")), "got: {:?}", errs);
    }

    #[test]
    fn rejects_double_definition() {
        let src = r"
fn @bad(%a: i64) -> i64 {
^entry:
    %b: i64 = iadd %a, %a
    %b: i64 = iadd %a, %a
    ret %b
}
";
        assert!(parse(src).is_err());
    }

    #[test]
    fn rejects_undefined_value() {
        let src = r"
fn @bad(%a: i64) -> i64 {
^entry:
    %b: i64 = iadd %a, %nope
    ret %b
}
";
        assert!(parse(src).is_err());
    }

    #[test]
    fn structured_lowers_and_verifies() {
        let src = r"
fn @sum(%n: i64) -> i64 {
    %zero: i64 = iconst 0
    %r: i64 = loop(%i: i64 = %zero, %acc: i64 = %zero) {
        %done: u1 = icmp.ge %i, %n
        if %done {
            break %acc
        }
        %one: i64 = iconst 1
        %a2: i64 = iadd %acc, %i
        %i2: i64 = iadd %i, %one
        continue %i2, %a2
    }
    ret %r
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
fn @bad(%c: u1, %a: i64, %b: i32) -> i64 {
    %r: i64 = if %c {
        yield %a
    } else {
        yield %b
    }
    ret %r
}
";
        let m = parse(src).expect("parse");
        assert!(verify(&m).is_err(), "i32 yield into i64 result must fail");
    }

    #[test]
    fn structured_error_cases() {
        // yield outside an if
        assert!(parse("fn @f() {\n    yield\n}").is_err());
        // break outside a loop
        assert!(parse("fn @f() {\n    break\n}").is_err());
        // loop body falling through
        assert!(parse(
            "fn @f(%n: i64) {\n    loop(%i: i64 = %n) {\n        %z: i64 = iconst 0\n    }\n    ret\n}"
        )
        .is_err());
        // unreachable code after ret
        assert!(parse("fn @f() {\n    ret\n    ret\n}").is_err());
        // value-yielding if without else
        assert!(parse(
            "fn @f(%c: u1, %a: i64) -> i64 {\n    %r: i64 = if %c {\n        yield %a\n    }\n    ret %r\n}"
        )
        .is_err());
    }

    #[test]
    fn forward_value_reference_across_blocks() {
        // %x is defined textually after its use; the prescan makes this fine
        // (dominance is the emitter's problem, per the spec).
        let src = r"
fn @fwd(%c: u1) -> i64 {
^entry:
    br %c, ^a, ^b
^a:
    jmp ^join(%x)
^b:
    %x: i64 = iconst 7
    jmp ^join(%x)
^join(%r: i64):
    ret %r
}
";
        let m = parse(src).expect("parse");
        verify(&m).expect("verify");
    }
}
