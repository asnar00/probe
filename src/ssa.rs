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
    /// a concrete integer, `iN` (signed) or `uN` (unsigned), 1 <= N <= 64
    Int { signed: bool, bits: u8 },
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
            bits: bits as u8,
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
        if (1..=64).contains(&bits) {
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
    /// widen, filling by the *source* type's signedness
    Ext,
    /// narrow to the result type's width
    Trunc,
    /// reinterpret the same number of bits as another type
    Bitcast,
}

impl CastOp {
    pub fn name(self) -> &'static str {
        match self {
            CastOp::Ext => "ext",
            CastOp::Trunc => "trunc",
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
/// unsigned twin `uint`) become on this compilation. Targets supply
/// defaults (their natural width, or a size-oriented choice); the user can
/// override. `float` joins when concrete floats do.
#[derive(Clone, Copy)]
pub struct Policy {
    pub int: Type,
}

impl Policy {
    pub fn new(int: Type) -> Result<Policy, String> {
        match int {
            Type::I32 | Type::I64 => Ok(Policy { int }),
            t => Err(format!("'int' cannot resolve to {}", t.name())),
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
                    let n = lex_int(&mut chars).map_err(|m| err(line, m))?;
                    toks.push((Tok::Int(n.wrapping_neg()), line));
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
                let n = lex_int(&mut chars).map_err(|m| err(line, m))?;
                toks.push((Tok::Int(n), line));
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

fn lex_int(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<i64, String> {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            chars.next();
        } else {
            break;
        }
    }
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
        types: Vec::new(),
        packs: Vec::new(),
        generics: Vec::new(),
        instances: HashMap::new(),
        pending: Vec::new(),
        env: Vec::new(),
    };
    // pass 1: type declarations and generic functions, wherever they
    // appear, so functions can use them regardless of order (a type
    // declaration may only refer to types declared before it)
    let mut funcs = Vec::new();
    let mut aliases: Vec<usize> = Vec::new(); // token positions of `fn x = g(..)`
    p.skip_newlines();
    while !p.at_end() {
        match p.item_kind() {
            Item::Type => p.parse_type_decl()?,
            Item::Generic => p.record_generic()?,
            Item::Alias => {
                aliases.push(p.pos);
                p.skip_line();
            }
            Item::Fn => {
                let (_, hi) = p.function_range()?;
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
        p.env.clear();
        f.instance = Some((generic, args));
        funcs.push(f);
    }
    let packs = std::sync::Arc::new(p.packs.clone());
    for f in &mut funcs {
        f.packs = packs.clone();
    }
    Ok(Module {
        types: p.types,
        funcs,
    })
}

/// a parametric function: its token range, re-parsed per instantiation
struct GenericFn {
    name: String,
    params: Vec<String>,
    lo: usize,
    /// the declared type of its first value parameter, for opcode dispatch
    first_param: Option<TypeExpr>,
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    types: Vec<TypeDef>,
    packs: Vec<PackDef>,
    generics: Vec<GenericFn>,
    /// (generic, args) -> the instance's function name
    instances: HashMap<(String, Vec<i64>), String>,
    /// instances requested but not yet parsed: (generic index, args, name)
    pending: Vec<(usize, Vec<i64>, String)>,
    /// the parameter bindings of the body being parsed (empty outside generics)
    env: Vec<(String, i64)>,
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
        if self.generics.iter().any(|g| g.name == name) {
            return Err(self.err(format!("generic function '{}' is defined more than once", name)));
        }
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        loop {
            params.push(self.expect_ident()?);
            if self.eat(&Tok::RParen) {
                break;
            }
            self.expect(Tok::Comma)?;
        }
        // the first value parameter's type: `(a: float(E, M), ...`
        self.expect(Tok::LParen)?;
        let first_param = match (self.toks.get(self.pos).map(|t| &t.0), self.toks.get(self.pos + 1).map(|t| &t.0)) {
            (Some(Tok::Ident(_)), Some(Tok::Colon)) => Some(self.type_expr_at(self.pos + 2, &params)?.0),
            _ => None,
        };
        self.generics.push(GenericFn {
            name,
            params,
            lo,
            first_param,
        });
        self.pos = hi + 1;
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

    /// the name of generic(args), instantiating it if new
    fn request_instance(&mut self, generic: &str, args: Vec<i64>, name: Option<String>) -> Result<String, ParseError> {
        let g = self
            .generics
            .iter()
            .position(|g| g.name == generic)
            .ok_or_else(|| self.err(format!("'{}' is not a generic function", generic)))?;
        if self.generics[g].params.len() != args.len() {
            return Err(self.err(format!(
                "'{}' takes {} parameter(s), given {}",
                generic,
                self.generics[g].params.len(),
                args.len()
            )));
        }
        let key = (generic.to_string(), args.clone());
        if let Some(existing) = self.instances.get(&key) {
            if let Some(n) = name {
                if *existing != n {
                    return Err(self.err(format!(
                        "'{}' is already instantiated as '{}'",
                        generic, existing
                    )));
                }
            }
            return Ok(existing.clone());
        }
        let name = name.unwrap_or_else(|| {
            let a: Vec<String> = args.iter().map(|v| v.to_string()).collect();
            format!("{}_{}", generic, a.join("_"))
        });
        self.instances.insert(key, name.clone());
        self.pending.push((g, args, name.clone()));
        Ok(name)
    }

    /// skip to the end of the current line (a declaration already parsed)
    fn skip_line(&mut self) {
        while !matches!(self.next(), Ok(Tok::Newline) | Err(_)) {}
    }

    /// `type name = expr` or `type name(P, Q) = expr`, on one line
    fn parse_type_decl(&mut self) -> Result<(), ParseError> {
        self.expect_ident()?; // 'type'
        let name = self.expect_ident()?;
        if Type::from_name(&name).is_some() || self.types.iter().any(|t| t.name == name) {
            self.pos -= 1;
            return Err(self.err(format!("type '{}' is already defined", name)));
        }
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
                if !(1..=64).contains(&n) {
                    return Err(format!("{} has {} bits; widths run from 1 to 64", expr, n));
                }
                Ok(Type::int(*signed, n as u32))
            }
            TypeExpr::Named { name, args } => {
                if args.is_empty() {
                    if let Some(t) = Type::from_name(name) {
                        return Ok(t);
                    }
                }
                let def = self
                    .types
                    .iter()
                    .find(|t| t.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("unknown type '{}'", name))?;
                if def.params.len() != args.len() {
                    return Err(format!(
                        "type '{}' takes {} parameter(s), given {}",
                        name,
                        def.params.len(),
                        args.len()
                    ));
                }
                let mut inner = Vec::new();
                for (p, a) in def.params.iter().zip(args) {
                    inner.push((p.clone(), a.eval(env)?));
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
                if width > 64 {
                    return Err(format!("{} is {} bits wide; packs fit in 64", expr, width));
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

    fn expect_value(&mut self, scope: &FuncScope) -> Result<ValueId, ParseError> {
        match self.next()? {
            Tok::Ident(name) => scope.value_ids.get(&name).copied().ok_or_else(|| {
                self.pos -= 1;
                self.err(format!("use of undefined value '{}'", name))
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

    /// Every value definition in the format is the pattern `name : type` —
    /// function params, block params, and instruction results alike. One scan
    /// over the function's tokens builds the whole value table.
    fn prescan_values(&mut self, lo: usize, hi: usize) -> Result<FuncScope, ParseError> {
        let mut scope = FuncScope {
            values: Vec::new(),
            value_ids: HashMap::new(),
            block_ids: HashMap::new(),
            block_names: Vec::new(),
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
                scope.values.push(ValueData { name, ty });
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

        // A body that opens with statements instead of a label is in
        // structured form; it parses via if/loop constructs and lowers to
        // the same block graph on the fly.
        if self.label_at(self.pos).is_none() && !matches!(self.peek(), Some(Tok::RBrace)) {
            let blocks = self.parse_structured_body(&scope)?;
            return Ok(Function {
                name,
                params,
                rets,
                values: scope.values,
                blocks,
                packs: Default::default(),
                instance: None,
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
                    let block = blocks
                        .last_mut()
                        .ok_or_else(|| self.err("instruction before the first block label"))?;
                    let inst = self.parse_inst(&scope)?;
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
        })
    }

    /// Look up a value being *defined*; the prescan registered it iff the
    /// definition is well-formed (`name: ty`), so a miss is a syntax error.
    fn def_id(&self, scope: &FuncScope, name: &str) -> Result<ValueId, ParseError> {
        scope.value_ids.get(name).copied().ok_or_else(|| {
            self.err(format!(
                "definition of '{}' is missing its ': type' annotation",
                name
            ))
        })
    }

    fn parse_inst(&mut self, scope: &FuncScope) -> Result<Inst, ParseError> {
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
                let inst = if op == "call" {
                    let (callee, args) = self.parse_call_tail(scope)?;
                    Inst::Call { dsts, callee, args }
                } else if op == "unpack" {
                    let src = self.expect_value(scope)?;
                    Inst::Unpack { dsts, src }
                } else if dsts.len() == 1 {
                    self.parse_def_op(&op, dsts[0], scope)?
                } else {
                    return Err(self.err(format!(
                        "only 'call' and 'unpack' can define multiple values, not '{}'",
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

    fn parse_def_op(&mut self, op: &str, dst: ValueId, scope: &FuncScope) -> Result<Inst, ParseError> {
        if let Some((_, bin)) = BINOPS.iter().find(|(n, _)| *n == op) {
            let lhs = self.expect_value(scope)?;
            self.expect(Tok::Comma)?;
            let rhs = self.expect_value(scope)?;
            // on a pack, the opcode is whatever generic function of that
            // name takes the pack's origin type: `add` on a float(8, 23)
            // is a call to add(8, 23) — the library, or the platform's
            // instruction for it
            if let Type::Pack(i) = scope.values[lhs.0 as usize].ty {
                let origin = self.packs[i as usize].origin.clone();
                let callee = self.dispatch(op, origin.as_ref(), &self.packs[i as usize].name.clone())?;
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
        if let Some(cc) = op.strip_prefix("icmp.") {
            let cond = CONDS
                .iter()
                .find(|(n, _)| *n == cc)
                .map(|(_, c)| *c)
                .ok_or_else(|| self.err(format!("unknown comparison condition '{}'", cc)))?;
            let lhs = self.expect_value(scope)?;
            self.expect(Tok::Comma)?;
            let rhs = self.expect_value(scope)?;
            return Ok(Inst::ICmp {
                cond,
                dst,
                lhs,
                rhs,
            });
        }
        match op {
            "iconst" => {
                // a literal, or in a generic an expression over its parameters
                let params: Vec<String> = self.env.iter().map(|(n, _)| n.clone()).collect();
                let (e, next) = self.int_expr_at(self.pos, &params)?;
                self.pos = next;
                let imm = e.eval(&self.env).map_err(|m| self.err(m))?;
                Ok(Inst::IConst { dst, imm })
            }
            "ext" | "trunc" | "bitcast" => {
                let cast = match op {
                    "ext" => CastOp::Ext,
                    "trunc" => CastOp::Trunc,
                    _ => CastOp::Bitcast,
                };
                let src = self.expect_value(scope)?;
                Ok(Inst::Cast { op: cast, dst, src })
            }
            "pack" => {
                let args = self.parse_value_list(scope)?;
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
                let val = self.expect_value(scope)?;
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
                let off = self.expect_value(scope)?;
                Ok(Inst::PtrAdd { dst, base, off })
            }
            _ => Err(self.err(format!("unknown opcode '{}'", op))),
        }
    }

    /// the instance of generic `op` for a pack of the given origin: the
    /// generic named `op` whose first parameter is `origin(P, Q, ...)`
    fn dispatch(&mut self, op: &str, origin: Option<&(String, Vec<i64>)>, tyname: &str) -> Result<String, ParseError> {
        let Some((gname, args)) = origin else {
            return Err(self.err(format!(
                "'{}' on {}: only packs instantiated from a generic type can be operated on",
                op, tyname
            )));
        };
        let found = self.generics.iter().position(|g| {
            g.name == op
                && matches!(&g.first_param, Some(TypeExpr::Named { name, args: a })
                    if name == gname
                        && a.len() == g.params.len()
                        && a.iter().zip(&g.params).all(|(e, p)| *e == IntExpr::Param(p.clone())))
        });
        if found.is_none() {
            return Err(self.err(format!(
                "no '{}' for {}: define fn {}({})(a: {}({}), ...) or a platform op",
                op,
                tyname,
                op,
                (0..args.len()).map(|i| ((b'E' + i as u8) as char).to_string()).collect::<Vec<_>>().join(", "),
                gname,
                (0..args.len()).map(|i| ((b'E' + i as u8) as char).to_string()).collect::<Vec<_>>().join(", "),
            )));
        }
        self.request_instance(op, args.clone(), None)
    }

    /// a field name of the pack-typed value `of`, resolved to its index
    fn expect_field(&mut self, scope: &FuncScope, of: ValueId) -> Result<u32, ParseError> {
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
                if matches!(self.peek(), Some(Tok::Ident(_))) {
                    vals.push(self.expect_value(scope)?);
                    while self.eat(&Tok::Comma) {
                        vals.push(self.expect_value(scope)?);
                    }
                }
                Ok(Inst::Ret { vals })
            }
            _ => Err(self.err(format!(
                "unknown opcode '{}' (or missing 'dst: ty =' before it)",
                op
            ))),
        }
    }

    fn parse_call_tail(&mut self, scope: &FuncScope) -> Result<(String, Vec<ValueId>), ParseError> {
        let mut callee = self.expect_ident()?;
        // `call g(8, 23)(a, b)`: the first group is width arguments when it
        // opens with a literal, a parameter, or a parenthesis
        let is_inst = matches!(self.peek(), Some(Tok::LParen))
            && match self.toks.get(self.pos + 1).map(|t| &t.0) {
                Some(Tok::Int(_)) | Some(Tok::LParen) => true,
                Some(Tok::Ident(n)) => self.env.iter().any(|(p, _)| p == n),
                _ => false,
            };
        if is_inst {
            let args = self.instance_args()?;
            callee = self.request_instance(&callee, args, None)?;
        }
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

    fn parse_structured_body(&mut self, scope: &FuncScope) -> Result<Vec<Block>, ParseError> {
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
    fn parse_struct_stmts(&mut self, scope: &FuncScope, st: &mut StructEmit) -> Result<bool, ParseError> {
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

    fn parse_struct_stmt(&mut self, scope: &FuncScope, st: &mut StructEmit) -> Result<bool, ParseError> {
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
                    "call" => {
                        let (callee, args) = self.parse_call_tail(scope)?;
                        st.push(Inst::Call { dsts, callee, args });
                    }
                    "unpack" => {
                        let src = self.expect_value(scope)?;
                        st.push(Inst::Unpack { dsts, src });
                    }
                    _ => {
                        if dsts.len() > 1 {
                            return Err(self.err(format!(
                                "only 'call' and 'unpack' can define multiple values, not '{}'",
                                op
                            )));
                        }
                        let inst = self.parse_def_op(&op, dsts[0], scope)?;
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
        scope: &FuncScope,
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
        scope: &FuncScope,
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
                self.expect_type()?;
                self.expect(Tok::Equals)?;
                inits.push(self.expect_value(scope)?);
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
        if matches!(self.peek(), Some(Tok::Ident(_))) {
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
        self.value(id).name.clone()
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
            Inst::Cast { op, dst, src } => format!(
                "{} = {} {}",
                self.fmt_def(*dst),
                op.name(),
                self.fmt_value(*src)
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
                format!("store {}, {}", self.fmt_value(*val), self.fmt_value(*addr))
            }
            Inst::PtrAdd { dst, base, off } => format!(
                "{} = ptradd {}, {}",
                self.fmt_def(*dst),
                self.fmt_value(*base),
                self.fmt_value(*off)
            ),
            Inst::Call { dsts, callee, args } => {
                let call = format!("call {}({})", callee, self.fmt_args(args));
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
                && matches!(func.width(t), Some(8) | Some(16) | Some(32) | Some(64)))
    };

    match inst {
        Inst::IConst { dst, imm } => {
            let ty = func.ty(*dst);
            let ok = match ty {
                Type::Int { bits, .. } if bits < 64 => {
                    // either reading of the literal must fit: signed or unsigned
                    let lo = -(1i64 << (bits - 1));
                    let hi = 1i64 << bits;
                    (lo..hi).contains(imm)
                }
                // ptr constants are raw addresses (MMIO, fixed buffers) —
                // meaningful wherever ptr is an address-space index
                Type::Int { .. } | Type::Ptr => true,
                // a pack literal is its bit pattern
                Type::Pack(_) => match func.width(ty) {
                    Some(w) if w < 64 => (0..1i64 << w).contains(imm),
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
                    "icmp.{} result {} must be u1, not {}",
                    cond.name(),
                    name(*dst),
                    tn(td)
                )));
            }
            if tl != tr || !(tl.is_int() || tl == Type::Ptr) {
                errs.push(ctx(format!(
                    "icmp.{}: operands must share an integer or ptr type; got {} and {}",
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
                CastOp::Ext => td.is_int() && ts.is_int() && wd > ws,
                CastOp::Trunc => td.is_int() && ts.is_int() && wd < ws,
                CastOp::Bitcast => {
                    (td.is_int() || td.is_pack() || td == Type::Ptr)
                        && (ts.is_int() || ts.is_pack() || ts == Type::Ptr)
                        && wd == ws
                }
            };
            if !ok {
                let why = match op {
                    CastOp::Ext => "ext widens an integer to a wider integer type",
                    CastOp::Trunc => "trunc narrows an integer to a narrower integer type",
                    CastOp::Bitcast => "bitcast needs two types of the same width",
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
    zero: i64 = iconst 0
    jmp loop(zero, zero)
loop(i: i64, acc: i64):
    done: u1 = icmp.ge i, n
    br done, exit, body
body:
    acc2: i64 = add acc, i
    one: i64 = iconst 1
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
    v: i32 = call get(p)
    w: i64 = ext v
    eight: i64 = iconst 8
    q: ptr = ptradd p, eight
    store w, q
    call touch(q)
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
    zero: i64 = iconst 0
    r: i64 = loop(i: i64 = zero, acc: i64 = zero) {
        done: u1 = icmp.ge i, n
        if done {
            break acc
        }
        one: i64 = iconst 1
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
            "fn f(n: i64) {\n    loop(i: i64 = n) {\n        z: i64 = iconst 0\n    }\n    ret\n}"
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
    x: i64 = iconst 7
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
    w: u16 = bitcast c
    r: u5, g: u6, b: u5 = unpack c
    x: i5 = bitcast r
    y: i7 = ext x
    z: u3 = trunc g
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
        assert!(parse("fn f(a: u65) {\n    ret\n}").is_err());
        assert!(parse("type p = pack { a: u40, b: u25 }\n").is_err()); // 65 bits
        assert!(parse("type p = pack { a: u4 }\nfn f(p: p) -> u4 {\n    x: u4 = get p, nope\n    ret x\n}").is_err());
        // bitcast must preserve width; ext must widen
        let m = parse("fn f(a: i8) -> u16 {\n    b: u16 = bitcast a\n    ret b\n}").unwrap();
        assert!(verify(&m).is_err());
        let m = parse("fn f(a: i8) -> i8 {\n    b: i8 = ext a\n    ret b\n}").unwrap();
        assert!(verify(&m).is_err());
        // memory only at 8/16/32/64
        let m = parse("fn f(p: ptr) -> u5 {\n    b: u5 = load p\n    ret b\n}").unwrap();
        assert!(verify(&m).is_err());
        let m = parse("fn f(p: ptr) -> u16 {\n    b: u16 = load p\n    ret b\n}").unwrap();
        assert!(verify(&m).is_ok());
        // icmp results are u1, and iconst must fit
        let m = parse("fn f(a: u5) -> u1 {\n    k: u5 = iconst 40\n    c: u1 = icmp.lt a, k\n    ret c\n}").unwrap();
        assert!(verify(&m).is_err());
        let m = parse("fn f(a: i5) -> u1 {\n    k: i5 = iconst -16\n    c: u1 = icmp.lt a, k\n    ret c\n}").unwrap();
        assert!(verify(&m).is_ok());
    }

    #[test]
    fn uint_resolves_with_int() {
        let mut m = parse("fn f(a: uint, b: int) -> uint {\n    c: uint = bitcast b\n    ret a\n}").unwrap();
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
    r: bits(8, 23) = bitcast f
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
        assert!(parse("type w(N) = u(N)\nfn f(a: w(70)) {\n    ret\n}").is_err());
        assert!(parse("type w(N) = u(M)\n").is_err());
        assert!(parse("type loop(N) = pack { a: loop(N) }\nfn f(a: loop(1)) {\n    ret\n}").is_err());
        assert!(parse("type big = pack { a: u(64), b: u(1) }\n").is_err());
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
    top: word(N) = iconst (1 << (N - 1)) - 1
    big: u1 = icmp.gt s, top
    r: word(N) = if big {
        d: word(N) = call halve(N)(s)
        yield d
    } else {
        yield s
    }
    ret r
}
fn halve(N)(a: word(N)) -> word(N) {
    one: word(N) = iconst 1
    h: word(N) = shr a, one
    ret h
}
fn wrap8 = wrap(8)
fn use12(a: u12) -> u12 {
    r: u12 = call wrap(4 * 3)(a, a)
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
}
