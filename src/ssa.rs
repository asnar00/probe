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
    /// declared MSB-first: the first field occupies the top bits
    pub fields: Vec<(String, Type)>,
}

impl StructDef {
    pub fn total_bits(&self) -> u32 {
        self.fields.iter().filter_map(|(_, t)| t.width()).sum()
    }

    /// LSB offset of field i (sum of the widths of later fields)
    pub fn offset(&self, i: usize) -> u32 {
        self.fields[i + 1..]
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
            Type::Struct(_) => "$struct".into(), // callers with a table print the name
            Type::Vec(n, e) => format!("{}x{}", e.ty().name(), n),
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
        matches!(self, Type::I(_) | Type::Int)
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
        matches!(self, Type::F32 | Type::F64)
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
        BINOPS.iter().find(|(_, op)| *op == self).unwrap().0
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
                if chars.peek() == Some(&'<') {
                    chars.next();
                    toks.push((Tok::Op("<<"), line));
                } else {
                    return Err(err(line, "expected '<<'".into()));
                }
            }
            '>' => {
                chars.next();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    toks.push((Tok::Op(">>"), line));
                } else {
                    return Err(err(line, "expected '>>'".into()));
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
    };
    let mut module = Module::default();
    p.skip_newlines();
    while !p.at_end() {
        if matches!(p.peek(), Some(Tok::Ident(k)) if k == "type") {
            p.parse_type_decl()?;
        } else {
            module.funcs.push(p.parse_function()?);
        }
        p.skip_newlines();
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
}

/// expression-sugar AST: parsed first, then desugared to flat temps so
/// the root operation can define the declared value directly
enum EAst {
    V(ValueId),
    Lit(Tok),
    Bin(BinOp, Box<EAst>, Box<EAst>),
}

/// Placeholder branch target inside structured constructs; every one is
/// patched to the real join/exit block before parsing of the construct ends.
const DUMMY_BLOCK: BlockId = BlockId(u32::MAX);

struct LoopFrame {
    header: BlockId,
    breaks: Vec<(usize, usize)>, // (block, inst) of Jmps to patch to the exit
}

/// Block-graph builder for structured bodies.
struct StructEmit {
    blocks: Vec<Block>,
    cur: usize,
    loop_stack: Vec<LoopFrame>,
    /// One frame per enclosing value-yielding position: edges waiting to be
    /// patched to an if's join block.
    yield_stack: Vec<Vec<(usize, usize)>>,
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
            return match self.structs.iter().position(|d| d.name == n) {
                Some(i) => Ok(Type::Struct(i as u16)),
                None => {
                    self.pos -= 1;
                    Err(self.err(format!("unknown struct type '${}'", n)))
                }
            };
        }
        let s = self.expect_ident()?;
        Type::from_name(&s).ok_or_else(|| {
            self.pos -= 1;
            self.err(format!("unknown type '{}'", s))
        })
    }

    /// `type $name = { field: iN, ... }` — fields are declared MSB-first
    /// and must be integer widths; the total may not exceed 64 bits
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
        self.expect(Tok::Equals)?;
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        loop {
            let fname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.expect_type()?;
            if ty.width().is_none() {
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
        let def = StructDef { name, fields };
        if def.total_bits() == 0 || def.total_bits() > 64 {
            return Err(self.err(format!(
                "struct '${}' is {} bits; 1..=64 required",
                def.name,
                def.total_bits()
            )));
        }
        self.expect(Tok::Newline)?;
        self.structs.push(def);
        Ok(())
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
        let floatish = matches!(ty, Type::F32 | Type::F64 | Type::Float | Type::Scalar);
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
            _ => self.expect_value(scope),
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
        let ast = self.expr_level(scope, ty, 0)?;
        match ast {
            EAst::Bin(op, l, r) => {
                let lhs = self.emit_expr(scope, ty, *l)?;
                let rhs = self.emit_expr(scope, ty, *r)?;
                Ok(Inst::Bin { op, dst, lhs, rhs })
            }
            EAst::Lit(tok) => self.lit_inst(ty, dst, &tok),
            EAst::V(_) => Err(self.err(
                "an expression must compute something; there is no copy opcode".to_string(),
            )),
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
            EAst::Bin(op, l, r) => {
                let lhs = self.emit_expr(scope, ty, *l)?;
                let rhs = self.emit_expr(scope, ty, *r)?;
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
                self.pending.push(Inst::Bin { op, dst: id, lhs, rhs });
                Ok(id)
            }
        }
    }

    /// operator for the level's symbol, chosen by the result type
    fn expr_op(&self, ty: Type, sym: &str) -> Result<BinOp, ParseError> {
        let floatish = matches!(ty, Type::F32 | Type::F64 | Type::Float | Type::Scalar)
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

    /// precedence-climbing over the C levels: | ^ & <<>> +- */%
    fn expr_level(
        &mut self,
        scope: &mut FuncScope,
        ty: Type,
        level: usize,
    ) -> Result<EAst, ParseError> {
        const LEVELS: &[&[&str]] = &[
            &["|"],
            &["^"],
            &["&"],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.expr_atom(scope, ty);
        }
        let mut lhs = self.expr_level(scope, ty, level + 1)?;
        loop {
            // "%x - 3" lexes the literal as Int(-3): absorb the sign as
            // a subtraction at the +/- level
            if LEVELS[level].contains(&"+") {
                match self.peek() {
                    Some(&Tok::Int(n)) if n < 0 && n != i64::MIN => {
                        self.next()?;
                        let op = self.expr_op(ty, "-")?;
                        lhs = EAst::Bin(op, Box::new(lhs), Box::new(EAst::Lit(Tok::Int(-n))));
                        continue;
                    }
                    Some(&Tok::FloatLit(x)) if x < 0.0 => {
                        self.next()?;
                        let op = self.expr_op(ty, "-")?;
                        lhs = EAst::Bin(
                            op,
                            Box::new(lhs),
                            Box::new(EAst::Lit(Tok::FloatLit(-x))),
                        );
                        continue;
                    }
                    _ => {}
                }
            }
            let Some(&Tok::Op(sym)) = self.peek() else { break };
            if !LEVELS[level].contains(&sym) {
                break;
            }
            self.next()?;
            let rhs = self.expr_level(scope, ty, level + 1)?;
            let op = self.expr_op(ty, sym)?;
            lhs = EAst::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn expr_atom(&mut self, scope: &mut FuncScope, ty: Type) -> Result<EAst, ParseError> {
        match self.next()? {
            Tok::Value(name) => {
                let id = scope.value_ids.get(&name).copied().ok_or_else(|| {
                    self.pos -= 1;
                    self.err(format!("use of undefined value '%{}'", name))
                })?;
                Ok(EAst::V(id))
            }
            t @ (Tok::Int(_) | Tok::FloatLit(_)) => Ok(EAst::Lit(t)),
            Tok::LParen => {
                let e = self.expr_level(scope, ty, 0)?;
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
        let floatish = matches!(ty, Type::F32 | Type::F64 | Type::Float | Type::Scalar);
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
                let rhs = self.expect_value(scope)?;
                let lhs = self.synth_lit(scope, scope.values[rhs.0 as usize].ty, &tok)?;
                Ok((lhs, rhs))
            }
            _ => {
                let lhs = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let rhs = self.operand(scope, scope.values[lhs.0 as usize].ty)?;
                Ok((lhs, rhs))
            }
        }
    }

    fn expect_value(&mut self, scope: &FuncScope) -> Result<ValueId, ParseError> {
        match self.next()? {
            Tok::Value(name) => scope.value_ids.get(&name).copied().ok_or_else(|| {
                self.pos -= 1;
                self.err(format!("use of undefined value '%{}'", name))
            }),
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a value, found {}", t)))
            }
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

    /// Every value definition in the format is the pattern `%name : type` —
    /// function params, block params, and instruction results alike. One scan
    /// over the function's tokens builds the whole value table.
    fn prescan_values(&self, lo: usize, hi: usize) -> Result<FuncScope, ParseError> {
        let mut scope = FuncScope {
            values: Vec::new(),
            value_ids: HashMap::new(),
            block_ids: HashMap::new(),
            block_names: Vec::new(),
        };
        let mut i = lo;
        while i + 2 <= hi {
            if let (Tok::Value(name), Tok::Colon) = (&self.toks[i].0, &self.toks[i + 1].0) {
                let line = self.toks[i].1;
                let ty = match &self.toks[i + 2].0 {
                    Tok::Ident(tyname) => Type::from_name(tyname).ok_or_else(|| ParseError {
                        line,
                        msg: format!("unknown type '{}'", tyname),
                    })?,
                    Tok::TyName(tn) => {
                        let idx = self.structs.iter().position(|d| &d.name == tn).ok_or_else(
                            || ParseError {
                                line,
                                msg: format!("unknown struct type '${}'", tn),
                            },
                        )?;
                        Type::Struct(idx as u16)
                    }
                    _ => {
                        return Err(ParseError {
                            line,
                            msg: format!("expected a type after '%{}:'", name),
                        })
                    }
                };
                if scope.value_ids.contains_key(name) {
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
                i += 3;
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

        let name = match self.next()? {
            Tok::Global(n) => n,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a function name, found {}", t)));
            }
        };

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
        match self.next()? {
            // %dst: ty [, %dst2: ty ...] = op ...
            Tok::Value(name) => {
                let mut dsts = vec![self.def_id(scope, &name)?];
                self.expect(Tok::Colon)?;
                self.expect_type()?;
                while self.eat(&Tok::Comma) {
                    match self.next()? {
                        Tok::Value(n) => dsts.push(self.def_id(scope, &n)?),
                        t => {
                            self.pos -= 1;
                            return Err(self.err(format!("expected a value, found {}", t)));
                        }
                    }
                    self.expect(Tok::Colon)?;
                    self.expect_type()?;
                }
                self.expect(Tok::Equals)?;
                let inst = if !matches!(self.peek(), Some(Tok::Ident(_))) {
                    // expression sugar: %v: ty = %a * %b + 1
                    if dsts.len() != 1 {
                        return Err(self.err("an expression defines exactly one value"));
                    }
                    self.parse_expr_into(dsts[0], scope)?
                } else {
                    let op = self.expect_ident()?;
                    if op == "call" {
                        let (callee, args) = self.parse_call_tail(scope)?;
                        Inst::Call { dsts, callee, args }
                    } else if dsts.len() == 1 {
                        self.parse_def_op(&op, dsts[0], scope)?
                    } else {
                        return Err(self.err(format!(
                            "only 'call' can define multiple values, not '{}'",
                            op
                        )));
                    }
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
                let src = self.expect_value(scope)?;
                Ok(Inst::Cast { op: cast, dst, src })
            }
            "load" => {
                let addr = self.expect_value(scope)?;
                Ok(Inst::Load { dst, addr })
            }
            "ptradd" => {
                let base = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let off = self.expect_value(scope)?;
                Ok(Inst::PtrAdd { dst, base, off })
            }
            "extract" => {
                let src = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let field = self.field_or_lane(scope, src)?;
                Ok(Inst::Extract { dst, src, field })
            }
            "pack" => {
                let mut args = vec![self.expect_value(scope)?];
                while self.eat(&Tok::Comma) {
                    args.push(self.expect_value(scope)?);
                }
                Ok(Inst::Pack { dst, args })
            }
            "insert" => {
                let src = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let field = self.field_or_lane(scope, src)?;
                self.expect(Tok::Comma)?;
                let val = self.expect_value(scope)?;
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

    fn parse_plain_op(&mut self, op: &str, scope: &FuncScope) -> Result<Inst, ParseError> {
        match op {
            "store" => {
                let val = self.expect_value(scope)?;
                self.expect(Tok::Comma)?;
                let addr = self.expect_value(scope)?;
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
                let cond = self.expect_value(scope)?;
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
                let mut vals = Vec::new();
                if matches!(self.peek(), Some(Tok::Value(_))) {
                    vals.push(self.expect_value(scope)?);
                    while self.eat(&Tok::Comma) {
                        vals.push(self.expect_value(scope)?);
                    }
                }
                Ok(Inst::Ret { vals })
            }
            _ => Err(self.err(format!(
                "unknown opcode '{}' (or missing '%dst: ty =' before it)",
                op
            ))),
        }
    }

    fn parse_call_tail(&mut self, scope: &FuncScope) -> Result<(String, Vec<ValueId>), ParseError> {
        let callee = match self.next()? {
            Tok::Global(n) => n,
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a function name, found {}", t)));
            }
        };
        self.expect(Tok::LParen)?;
        let mut args = Vec::new();
        if !self.eat(&Tok::RParen) {
            loop {
                args.push(self.expect_value(scope)?);
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
        match self.next()? {
            Tok::Value(name) => {
                let mut dsts = vec![self.def_id(scope, &name)?];
                self.expect(Tok::Colon)?;
                self.expect_type()?;
                while self.eat(&Tok::Comma) {
                    match self.next()? {
                        Tok::Value(n) => dsts.push(self.def_id(scope, &n)?),
                        t => {
                            self.pos -= 1;
                            return Err(self.err(format!("expected a value, found {}", t)));
                        }
                    }
                    self.expect(Tok::Colon)?;
                    self.expect_type()?;
                }
                self.expect(Tok::Equals)?;
                if !matches!(self.peek(), Some(Tok::Ident(_))) {
                    if dsts.len() != 1 {
                        return Err(self.err("an expression defines exactly one value"));
                    }
                    let inst = self.parse_expr_into(dsts[0], scope)?;
                    for p in self.pending.drain(..) {
                        st.push(p);
                    }
                    st.push(inst);
                    self.expect(Tok::Newline)?;
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
                self.expect(Tok::Newline)?;
                Ok(false)
            }
            Tok::Ident(op) => match op.as_str() {
                "if" => self.parse_struct_if(scope, st, Vec::new()),
                "loop" => self.parse_struct_loop(scope, st, Vec::new()),
                "break" => {
                    let vals = self.parse_value_list(scope)?;
                    if st.loop_stack.is_empty() {
                        return Err(self.err("'break' outside a loop"));
                    }
                    let at = st.push(Inst::Jmp {
                        target: DUMMY_BLOCK,
                        args: vals,
                    });
                    st.loop_stack.last_mut().unwrap().breaks.push(at);
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "continue" => {
                    let vals = self.parse_value_list(scope)?;
                    let Some(frame) = st.loop_stack.last() else {
                        return Err(self.err("'continue' outside a loop"));
                    };
                    let header = frame.header;
                    st.push(Inst::Jmp {
                        target: header,
                        args: vals,
                    });
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "yield" => {
                    let vals = self.parse_value_list(scope)?;
                    if st.yield_stack.is_empty() {
                        return Err(self.err("'yield' outside an if"));
                    }
                    let at = st.push(Inst::Jmp {
                        target: DUMMY_BLOCK,
                        args: vals,
                    });
                    st.yield_stack.last_mut().unwrap().push(at);
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "ret" => {
                    let vals = self.parse_value_list(scope)?;
                    st.push(Inst::Ret { vals });
                    self.expect(Tok::Newline)?;
                    Ok(true)
                }
                "store" | "call" => {
                    let inst = self.parse_plain_op(&op, scope)?;
                    st.push(inst);
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
        let cond = self.expect_value(scope)?;
        self.expect(Tok::LBrace)?;
        self.expect(Tok::Newline)?;

        let before = st.cur;
        let then_b = st.new_block(Vec::new());
        st.yield_stack.push(Vec::new()); // collects edges into the join

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
        self.expect(Tok::Newline)?;

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
        if !self.eat(&Tok::RParen) {
            loop {
                match self.next()? {
                    Tok::Value(n) => params.push(self.def_id(scope, &n)?),
                    t => {
                        self.pos -= 1;
                        return Err(self.err(format!("expected a loop variable, found {}", t)));
                    }
                }
                self.expect(Tok::Colon)?;
                let vty = self.expect_type()?;
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
        self.expect(Tok::Newline)?;

        let header = st.new_block(params);
        st.push(Inst::Jmp {
            target: header,
            args: inits,
        });
        st.loop_stack.push(LoopFrame {
            header,
            breaks: Vec::new(),
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

    fn parse_value_list(&mut self, scope: &FuncScope) -> Result<Vec<ValueId>, ParseError> {
        let mut vals = Vec::new();
        if matches!(self.peek(), Some(Tok::Value(_))) {
            vals.push(self.expect_value(scope)?);
            while self.eat(&Tok::Comma) {
                vals.push(self.expect_value(scope)?);
            }
        }
        Ok(vals)
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
        format!("%{}", self.value(id).name)
    }

    fn fmt_def(&self, id: ValueId) -> String {
        let v = self.value(id);
        format!("%{}: {}", v.name, self.ty_name(v.ty))
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
        write!(f, "fn @{}({})", self.name, params)?;
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
                let call = format!("call @{}({})", callee, self.fmt_args(args));
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
    let ctx = |msg: String| format!("@{}: {}", func.name, msg);

    if func.blocks.is_empty() {
        errs.push(ctx("function has no blocks".into()));
        return;
    }

    // rule 0: abstract types are resolved before verification
    for v in &func.values {
        if matches!(v.ty, Type::Int | Type::Uint | Type::Float | Type::Scalar) {
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
                Type::F32 | Type::F64 => false, // use fconst
                Type::Struct(_) | Type::Vec(..) => false, // use pack
                Type::Int | Type::Uint | Type::Float | Type::Scalar => {
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
            if tl != tr || !(tl.is_arith() || tl == Type::Ptr) {
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
                CastOp::Fpromote => ts == Type::F32 && td == Type::F64,
                CastOp::Fdemote => ts == Type::F64 && td == Type::F32,
                CastOp::Bitcast => {
                    // same total width, different type: signedness flips,
                    // int<->float, int<->struct
                    let w = |t: Type| match t {
                        Type::F32 => Some(32),
                        Type::F64 => Some(64),
                        Type::Struct(i) => Some(func.structs[i as usize].total_bits()),
                        Type::Vec(n, e) => Some(n as u32 * e.bits()),
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
        assert!(errs[0].contains("iadd"), "got: {:?}", errs);
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
