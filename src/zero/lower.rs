//! The lowering: a store's syntax trees to IR text. The IR is the
//! meaning (zero.md section 17 is the table this file implements), so
//! every construct here emits the form the section names and nothing
//! cleverer, in probe's structured form, readable by a person and
//! accepted by `probe parse` — which is the oracle for this front end.
//!
//! Names: an open-syntax name mangles to its words joined by `_`;
//! temporaries are `_1`, `_2`, ... (a zero name may not start with `_`);
//! a variable assigned again is `name_2`, `name_3`, ... so the IR stays
//! in SSA and a reader can follow the versions.
//!
//! Types are strict inside a body — two operands share a type or one is
//! a literal — and the tower of abstract numbers binds only at a call:
//! an `int32` argument fits a `number` parameter, and a `number` result
//! comes back as `int32`, which is how the IR instantiates its templates.
//! A struct is the IR's `struct`, an enumeration an unsigned integer
//! with named constants, a string a `u8[]` view of bytes.

use super::lex::{self, Error};
use super::store::{Case, Expect, Store};
use super::syntax::{Arg, Decl, Expr, ExprKind, FnDecl, Init, NamePart, Part, Stmt, TypeKind};
use std::collections::HashMap;
use std::fmt::Write;

/// a zero type, as the lowering sees it
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Bool,
    /// a number, by its IR name: `int`, `u8`, `f32`, `number`, ...
    Num(String),
    /// a sequence `T$`: the IR's rank-1 view `T[]`; a `string` is `u8$`
    Seq(Box<Ty>),
    /// a declared struct, by name
    Struct(String),
    /// a declared enumeration, by name
    Enum(String),
    /// no value: a function with no results
    None,
}

impl Ty {
    fn ir(&self) -> String {
        match self {
            Ty::Bool => "u1".into(),
            Ty::Num(n) => n.clone(),
            Ty::Seq(e) => format!("{}[]", e.ir()),
            Ty::Struct(n) | Ty::Enum(n) => n.clone(),
            Ty::None => String::new(),
        }
    }

    fn string() -> Ty {
        Ty::Seq(Box::new(Ty::Num("u8".into())))
    }

    /// the element type of a sequence, or none
    fn elem(&self) -> Option<&Ty> {
        match self {
            Ty::Seq(e) => Some(e),
            _ => None,
        }
    }

    /// is this an abstract number, one the tower binds at a call?
    fn abstract_name(&self) -> Option<&str> {
        match self {
            Ty::Num(n) if matches!(n.as_str(), "number" | "scalar" | "int" | "uint" | "float" | "fixed" | "unit" | "sunit" | "rational" | "decimal") => Some(n),
            _ => None,
        }
    }
}

/// zero's spelling of a builtin number type to the IR's (section 4)
fn builtin_type(name: &str) -> Option<Ty> {
    let ir = match name {
        "bool" => return Some(Ty::Bool),
        "string" => return Some(Ty::string()),
        "int" | "uint" | "float" | "number" | "scalar" | "fixed" | "unit" | "sunit" | "rational" | "decimal" | "time" => name.to_string(),
        "int8" => "i8".into(),
        "int16" => "i16".into(),
        "int32" => "i32".into(),
        "int64" => "i64".into(),
        "int128" => "i128".into(),
        "uint8" | "byte" | "char" => "u8".into(),
        "uint16" => "u16".into(),
        "uint32" => "u32".into(),
        "uint64" => "u64".into(),
        "uint128" => "u128".into(),
        "float16" => "f16".into(),
        "bfloat16" => "bf16".into(),
        "float32" => "f32".into(),
        "float64" => "f64".into(),
        _ => return None,
    };
    Some(Ty::Num(ir))
}

/// does an argument of type `arg` fit a parameter of type `param`? Its
/// own type does, and so does an abstract type above it in the tower
/// (ssa.md, *Abstract numeric types*): the call instantiates the
/// template with the argument's type
fn fits(arg: &Ty, param: &Ty) -> bool {
    if arg == param {
        return true;
    }
    let (Ty::Num(a), Ty::Num(p)) = (arg, param) else {
        return false;
    };
    let width = |s: &str| s[1..].parse::<u32>().is_ok();
    let is_float = |s: &str| matches!(s, "f16" | "bf16" | "f32" | "f64" | "float");
    let is_scalar = |s: &str| is_float(s) || matches!(s, "fixed" | "unit" | "sunit" | "rational" | "decimal" | "scalar" | "time");
    let is_int = |s: &str| s == "int" || (s.starts_with('i') && width(s));
    let is_uint = |s: &str| s == "uint" || (s.starts_with('u') && width(s));
    match p.as_str() {
        "number" => true,
        "scalar" => is_scalar(a),
        "int" => is_int(a),
        "uint" => is_uint(a),
        "float" => is_float(a),
        _ => false,
    }
}

/// a function the store defines, as the lowering knows it
#[derive(Clone, Debug)]
pub struct FnInfo {
    /// the mangled name a call is matched by
    pub key: String,
    /// the IR function's name: the key, except for an operator on a
    /// declared type (`add_Vec`) and the front end's builtins
    pub ir: String,
    pub parts: Vec<NamePart>,
    pub params: Vec<(String, Ty)>,
    pub results: Vec<(String, Ty)>,
    pub feature: String,
}

/// a declared type
#[derive(Clone, Debug)]
enum TypeInfo {
    /// fields: name, type, default (a literal's text)
    Struct(Vec<(String, Ty, Option<String>)>),
    Enum(Vec<String>),
}

/// a feature-scope variable: a field of the store's context struct
/// (log 16), with the columns section 5 declares
#[derive(Clone, Debug)]
struct FVar {
    name: String,
    ty: Ty,
    /// `user`, `static`, `device` or `group`
    scope: String,
    /// `last`, or the word after `merge`
    merge: String,
    feature: String,
}

pub struct Lowered {
    pub ir: String,
    pub funcs: Vec<FnInfo>,
}

/// what the runner calls for a case: the IR function, its integer
/// arguments, how many results it has, and what the case expects
pub struct Call {
    pub func: String,
    pub args: Vec<i64>,
    pub nrets: usize,
    pub expect: Expect,
}

/// the IR every store gets: the output buffer `print` appends to, the
/// two functions the runner reads it back with, `__print` itself and
/// `__str` (a string literal's view). `__zero_reset`, which the runner
/// calls before each case, is generated per store (log 16)
const PRELUDE: &str = r#"
; the print buffer: what `print` wrote, read back by the runner
data __out: array(u8, 4096)
data __out_n: array(i64, 1)
data __nul: array(u8, 1)

; the arena every sequence is carved from, emptied before every case
data __heap: array(u8, 65536)
data __arena: array(i64, 3)

fn __out_len() -> i64 {
    q: ptr = addr __out_n
    n: i64 = load q
    ret n
}

fn __out_byte(i: i64) -> u8 {
    o: ptr = addr __out
    b: u8 = load o, i, 1
    ret b
}

; a string: a view of n bytes at p
fn __str(p: ptr, n: i64) -> u8[] {
    q: ptr(u8) = cast p
    v: u8[] = pack q, n, 1
    ret v
}

; append a string and a newline; what does not fit is dropped
fn __print(s: u8[]) {
    q: ptr = addr __out_n
    k0: i64 = load q
    o: ptr = addr __out
    p: ptr(u8) = ptr s
    n: i64 = len s
    k1: i64 = loop(i: i64 = 0, k: i64 = k0) {
        done: u1 = cmp.ge i, n
        if done {
            break k
        }
        c: u8 = load p, i
        room: u1 = cmp.lt k, 4095
        if room {
            store c, o, k, 1
        }
        i2: i64 = add i, 1
        k2: i64 = add k, 1
        continue i2, k2
    }
    room2: u1 = cmp.lt k1, 4095
    k4: i64 = if room2 {
        store 10: u8, o, k1, 1
        k3: i64 = add k1, 1
        yield k3
    } else {
        yield k1
    }
    store k4, q
    ret
}
"#;

pub fn mangle(parts: &[NamePart]) -> String {
    let words: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            NamePart::Word(w) => Some(w.clone()),
            NamePart::Sym(s) => Some(op_name(s).trim_start_matches("cmp.").to_string()),
            NamePart::Group => None,
        })
        .collect();
    words.join("_")
}

/// an operator's opcode in the IR (section 6: `+` on a type is `add`)
fn op_name(s: &str) -> &'static str {
    match s {
        "+" => "add",
        "-" => "sub",
        "*" => "mul",
        "/" => "div",
        "%" => "rem",
        "<" => "cmp.lt",
        ">" => "cmp.gt",
        "<=" => "cmp.le",
        ">=" => "cmp.ge",
        "==" => "cmp.eq",
        "!=" => "cmp.ne",
        _ => "op",
    }
}

fn is_comparison(op: &str) -> bool {
    matches!(op, "<" | ">" | "<=" | ">=" | "==" | "!=")
}

pub fn lower(store: &Store) -> Result<Lowered, Error> {
    let mut l = Lowerer { funcs: Vec::new(), types: HashMap::new(), type_lines: Vec::new(), data: Vec::new(), out: String::new(), nstr: 0, fvars: Vec::new(), news: std::collections::BTreeSet::new() };
    // the front end's builtins, until item 11 declares them as platform functions
    l.funcs.push(FnInfo {
        key: "print".into(),
        ir: "__print".into(),
        parts: vec![NamePart::Word("print".into()), NamePart::Group],
        params: vec![("s".into(), Ty::string())],
        results: Vec::new(),
        feature: String::new(),
    });
    // types first, then every signature, so a body may use what a later
    // feature declares
    for f in &store.features {
        for d in &f.code.decls {
            if let Decl::Type(t) = d {
                l.declare_type(t, &f.code.file)?;
            }
        }
    }
    for f in &store.features {
        for d in &f.code.decls {
            match d {
                Decl::Fn(fd) => l.declare(fd, &f.name, &f.code.file)?,
                Decl::Type(_) => {}
                Decl::Var(v) => l.declare_var(v, &f.name, &f.code.file)?,
            }
        }
    }
    l.emit_context(store)?;
    for f in &store.features {
        writeln!(l.out, "\n; feature {}", f.name).unwrap();
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                l.lower_fn(fd, &f.code.file)?;
            }
        }
    }
    if !l.news.is_empty() {
        writeln!(l.out, "\n; a sequence of n items, carved from the arena: a buffer, then the view over it").unwrap();
    }
    for t in &l.news {
        writeln!(l.out, "fn __new_{}(n: i64) -> {}[] {{\n    a: ptr = addr __arena\n    sz: i64 = sizeof {}\n    bytes: i64 = mul n, sz\n    total: i64 = add bytes, 16\n    p: ptr = arena_alloc(a, total)\n    buffer_init(p, sz, n)\n    v: {}[] = slice p\n    ret v\n}}", t, t, t, t).unwrap();
    }
    let mut ir = String::new();
    writeln!(ir, "; lowered from the zero store {}", store.path.display()).unwrap();
    ir.push_str(PRELUDE);
    if !l.type_lines.is_empty() {
        ir.push('\n');
        for t in &l.type_lines {
            ir.push_str(t);
            ir.push('\n');
        }
    }
    if !l.data.is_empty() {
        ir.push('\n');
        for d in &l.data {
            ir.push_str(d);
            ir.push('\n');
        }
    }
    ir.push_str(&l.out);
    Ok(Lowered { ir, funcs: l.funcs })
}

/// resolve a `## testing` case against the lowered store
pub fn resolve_case(lowered: &Lowered, case: &Case, file: &str) -> Result<Call, Error> {
    let ExprKind::Phrase(parts) = &case.call.kind else {
        return Err(lex::error(file, case.line, "a case calls a function"));
    };
    let (info, args) = find_function(&lowered.funcs, parts, &|_| false, file, case.line)?;
    let mut vals = Vec::new();
    for (a, (_, ty)) in args.iter().zip(&info.params) {
        let v = match (&a.kind, ty) {
            (ExprKind::Int(v), Ty::Num(_) | Ty::Enum(_)) => *v,
            (ExprKind::Bool(b), Ty::Bool) => *b as i64,
            _ => return Err(lex::error(file, case.line, "a case's arguments are numbers")),
        };
        vals.push(v);
    }
    Ok(Call { func: info.ir.clone(), args: vals, nrets: info.results.len(), expect: case.expect.clone() })
}

/// The function a phrase names, and its arguments in order: the bracket
/// groups and bare values are arguments, and every word is part of the
/// name — unless no function reads that way, when a word that names a
/// variable in scope is an argument too (log 17)
fn find_function<'a>(funcs: &'a [FnInfo], parts: &[Part], is_var: &dyn Fn(&str) -> bool, file: &str, line: usize) -> Result<(&'a FnInfo, Vec<Expr>), Error> {
    let read = |vars_are_args: bool| -> Result<(Vec<NamePart>, Vec<Expr>), Error> {
        let mut name = Vec::new();
        let mut args = Vec::new();
        for p in parts {
            match p {
                Part::Word(w) if vars_are_args && is_var(w) => {
                    name.push(NamePart::Group);
                    args.push(Expr { kind: ExprKind::Name(w.clone()), line });
                }
                Part::Word(w) => name.push(NamePart::Word(w.clone())),
                Part::Args(list) => {
                    name.push(NamePart::Group);
                    for a in list {
                        if a.name.is_some() {
                            return Err(lex::error(file, line, "a call's arguments are by position"));
                        }
                        args.push(a.value.clone());
                    }
                }
                Part::Value(e) => {
                    name.push(NamePart::Group);
                    args.push(e.clone());
                }
            }
        }
        Ok((name, args))
    };
    let lookup = |name: &[NamePart]| -> Option<&'a FnInfo> {
        let key = mangle(name);
        funcs.iter().filter(|f| f.key == key && !f.parts.iter().any(|p| matches!(p, NamePart::Sym(_)))).last()
    };
    let (name, args) = read(false)?;
    if let Some(info) = lookup(&name) {
        if info.params.len() == args.len() {
            return Ok((info, args));
        }
    }
    let (name, args) = read(true)?;
    let Some(info) = lookup(&name) else {
        let words: Vec<String> = name.iter().filter_map(|p| if let NamePart::Word(w) = p { Some(w.clone()) } else { None }).collect();
        return Err(lex::error(file, line, format!("no function named '{}'", words.join(" "))));
    };
    if info.params.len() != args.len() {
        return Err(lex::error(file, line, format!("'{}' takes {} argument(s), given {}", info.key, info.params.len(), args.len())));
    }
    Ok((info, args))
}

struct Lowerer {
    funcs: Vec<FnInfo>,
    types: HashMap<String, TypeInfo>,
    /// the `type` lines, in declaration order
    type_lines: Vec<String>,
    /// `data` lines for the string literals met so far
    data: Vec<String>,
    out: String,
    nstr: usize,
    /// the feature-scope variables, in composition order
    fvars: Vec<FVar>,
    /// the element types sequences were made of: one `__new_T` each
    news: std::collections::BTreeSet<String>,
}

/// a variable in a function's scope: its current IR value and type
#[derive(Clone, Debug)]
struct Var {
    ir: String,
    ty: Ty,
    /// has it a value yet? A result starts without one
    set: bool,
    /// how many loops enclosed its declaration: a variable may be
    /// assigned only at the depth it was declared, or by the loop that
    /// carries it (log 12)
    loop_depth: usize,
}

/// a loop being lowered: what `break` and `continue` need
struct LoopCtx {
    /// the carried variables, in the header's order; empty for a `for`
    carried: Vec<String>,
    /// a `for`'s stepped variable: its name and the step (`add`/`sub`,
    /// the amount) — the item itself over a range, the index over a
    /// sequence
    item: Option<(String, &'static str, String)>,
    /// over a sequence, the item loaded at the top of each pass
    loaded: Option<String>,
    /// how many `break`s the body has, the `while` test's included
    breaks: usize,
}

/// a value: the text that stands for it as an operand — a name, or a
/// literal where the IR takes one inline — and its type
#[derive(Clone, Debug)]
struct Val {
    text: String,
    ty: Ty,
    literal: bool,
}

/// one function body being lowered
struct Body {
    out: String,
    ntmp: usize,
    /// the variables in scope; a block's declarations leave with it
    vars: HashMap<String, Var>,
    /// how many IR definitions each name has had, for the whole body,
    /// so a name declared again after its block closed is still SSA
    defs: HashMap<String, usize>,
    results: Vec<(String, Ty)>,
    file: String,
    /// how deep in structured blocks the next line is
    depth: usize,
    loops: Vec<LoopCtx>,
}

impl Body {
    fn tmp(&mut self) -> String {
        self.ntmp += 1;
        format!("_{}", self.ntmp)
    }

    fn line(&mut self, s: &str) {
        for _ in 0..=self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// a variable brought into scope, with no value yet
    fn declare(&mut self, name: &str, ty: Ty) {
        let depth = self.loops.len();
        self.vars.insert(name.to_string(), Var { ir: String::new(), ty, set: false, loop_depth: depth });
    }

    /// the IR name for a new definition of `name`: the name itself the
    /// first time, `name_2`, `name_3` after
    fn define(&mut self, name: &str, ty: Ty) -> String {
        let depth = self.loops.len();
        let n = self.defs.entry(name.to_string()).or_insert(0);
        *n += 1;
        let ir = if *n == 1 { name.to_string() } else { format!("{}_{}", name, n) };
        let v = self.vars.entry(name.to_string()).or_insert(Var { ir: String::new(), ty: ty.clone(), set: false, loop_depth: depth });
        v.ty = ty;
        v.set = true;
        v.ir = ir.clone();
        ir
    }

    /// the current IR values of some variables, in order
    fn current(&self, names: &[String]) -> Vec<String> {
        names.iter().map(|n| self.vars[n].ir.clone()).collect()
    }

    /// may `name` be assigned here? Only at the depth it was declared,
    /// which includes a loop's own carried variables; a `for`'s item
    /// and a variable of an enclosing block are not (log 12)
    fn assignable(&self, name: &str, line: usize) -> Result<(), Error> {
        let v = &self.vars[name];
        if let Some(l) = self.loops.last() {
            if l.item.as_ref().map(|(i, _, _)| i.as_str()) == Some(name) || l.loaded.as_deref() == Some(name) {
                return Err(lex::error(&self.file, line, format!("'{}' is the item of the `for`: it steps by itself and is not assigned", name)));
            }
        }
        if v.loop_depth != self.loops.len() {
            return Err(lex::error(&self.file, line, format!("'{}' is declared outside the loop: carry it in the loop's header, `loop ({} {} = ...)`", name, v.ty.ir(), name)));
        }
        Ok(())
    }

    /// a literal made into a value, where the IR wants a name
    fn materialize(&mut self, v: &Val) -> Val {
        if !v.literal {
            return v.clone();
        }
        let t = self.tmp();
        self.line(&format!("{}: {} = const {}", t, v.ty.ir(), v.text));
        Val { text: t, ty: v.ty.clone(), literal: false }
    }
}

impl Lowerer {
    /// a type by zero's name
    fn ty(&self, name: &str, seq: bool, file: &str, line: usize) -> Result<Ty, Error> {
        let t = match builtin_type(name) {
            Some(t) => t,
            None => match self.types.get(name) {
                Some(TypeInfo::Struct(_)) => Ty::Struct(name.to_string()),
                Some(TypeInfo::Enum(_)) => Ty::Enum(name.to_string()),
                None => return Err(lex::error(file, line, format!("'{}' is not a type", name))),
            },
        };
        if !seq {
            return Ok(t);
        }
        // the items of a sequence are numbers and enumerations in this milestone
        match t {
            Ty::Num(_) | Ty::Enum(_) => Ok(Ty::Seq(Box::new(t))),
            _ => Err(lex::error(file, line, format!("a sequence of {}: only numbers and enumerations in this milestone", t.ir()))),
        }
    }

    /// a new sequence of n items in the arena, through the generated
    /// `__new_T` for its element type
    fn new_seq(&mut self, elem: &Ty, n: &str, b: &mut Body, dst: Option<&str>) -> Val {
        let ty = Ty::Seq(Box::new(elem.clone()));
        self.news.insert(elem.ir());
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = __new_{}({})", out, ty.ir(), elem.ir(), n));
        Val { text: out, ty, literal: false }
    }

    fn declare_type(&mut self, t: &super::syntax::TypeDecl, file: &str) -> Result<(), Error> {
        if self.types.contains_key(&t.name) || builtin_type(&t.name).is_some() {
            return Err(lex::error(file, t.line, format!("type '{}' is already declared", t.name)));
        }
        match &t.kind {
            TypeKind::Enum(cases) => {
                // a byte, or the next memory width, so that a variable
                // of the type can be a field of the context (log 18)
                let bits = if cases.len() <= 256 { 8 } else if cases.len() <= 65536 { 16 } else { 32 };
                self.type_lines.push(format!("type {} = u{}", t.name, bits));
                self.types.insert(t.name.clone(), TypeInfo::Enum(cases.clone()));
            }
            TypeKind::Struct(fields) => {
                let mut out = Vec::new();
                let mut ir = Vec::new();
                for f in fields {
                    let ty = self.ty(&f.ty, f.seq, file, f.line)?;
                    if out.iter().any(|(n, _, _)| n == &f.name) {
                        return Err(lex::error(file, f.line, format!("field '{}' is named twice", f.name)));
                    }
                    let default = match &f.default {
                        None => None,
                        Some(Expr { kind: ExprKind::Int(v), .. }) if matches!(ty, Ty::Num(_)) => Some(v.to_string()),
                        Some(Expr { kind: ExprKind::Float(s), .. }) if matches!(ty, Ty::Num(_)) => Some(s.clone()),
                        Some(Expr { kind: ExprKind::Bool(v), .. }) if ty == Ty::Bool => Some((*v as i64).to_string()),
                        Some(e) => return Err(lex::error(file, e.line, format!("a field's default is a literal of its type ({})", ty.ir()))),
                    };
                    ir.push(format!("{}: {}", f.name, ty.ir()));
                    out.push((f.name.clone(), ty, default));
                }
                if out.is_empty() {
                    return Err(lex::error(file, t.line, format!("type {} has no fields", t.name)));
                }
                self.type_lines.push(format!("type {} = struct {{ {} }}", t.name, ir.join(", ")));
                self.types.insert(t.name.clone(), TypeInfo::Struct(out));
            }
        }
        Ok(())
    }

    fn declare(&mut self, f: &FnDecl, feature: &str, file: &str) -> Result<(), Error> {
        if f.task {
            return Err(lex::error(file, f.line, "tasks are not in this item yet"));
        }
        if !f.platform.is_empty() {
            return Err(lex::error(file, f.line, "platform bodies are not in this item yet"));
        }
        let key = mangle(&f.name);
        let mut params = Vec::new();
        for p in f.params() {
            params.push((p.name.clone(), self.ty(&p.ty, p.seq, file, p.line)?));
        }
        let mut results = Vec::new();
        for r in &f.results {
            results.push((r.name.clone(), self.ty(&r.ty, r.seq, file, r.line)?));
        }
        let operator = f.name.iter().any(|p| matches!(p, NamePart::Sym(_)));
        let ir = if operator {
            // an operator on a declared type: named by the opcode and its
            // first operand's type, since the IR does not dispatch
            // arithmetic on structs and plain functions cannot share a name
            if f.name.len() != 3 || !matches!(f.name.as_slice(), [NamePart::Group, NamePart::Sym(_), NamePart::Group]) || params.len() != 2 {
                return Err(lex::error(file, f.line, "an operator is `on (T r) = (T a) op (U b)`"));
            }
            if !matches!(params[0].1, Ty::Struct(_)) {
                return Err(lex::error(file, f.line, "an operator's first operand is a declared struct type; numbers have the IR's operators"));
            }
            format!("{}_{}", key, params[0].1.ir())
        } else {
            key.clone()
        };
        if let Some(other) = self.funcs.iter().find(|g| g.ir == ir) {
            if other.feature.is_empty() {
                return Err(lex::error(file, f.line, format!("'{}' is a builtin of the front end", key)));
            }
            if operator && other.params != params {
                return Err(lex::error(file, f.line, format!("a second operator on {} with this symbol clashes with feature {}'s: the IR names it by its first operand's type only", params[0].1.ir(), other.feature)));
            }
            if other.parts != f.name || other.params.len() != params.len() {
                return Err(lex::error(file, f.line, format!("'{}' clashes with a function of feature {} that mangles to the same name", key, other.feature)));
            }
            return Err(lex::error(file, f.line, "redefinition is not in this item yet"));
        }
        self.funcs.push(FnInfo { key, ir, parts: f.name.clone(), params, results, feature: feature.to_string() });
        Ok(())
    }

    /// a feature-scope variable: a field of the context (log 16)
    fn declare_var(&mut self, v: &super::syntax::VarDecl, feature: &str, file: &str) -> Result<(), Error> {
        if v.name == "enabled" {
            return Err(lex::error(file, v.line, "every feature has 'enabled' already: a feature may not declare its own"));
        }
        if let Some(other) = self.fvars.iter().find(|f| f.name == v.name) {
            return Err(lex::error(file, v.line, format!("'{}' is already a variable of feature {}", v.name, other.feature)));
        }
        if v.scope.len() > 1 {
            return Err(lex::error(file, v.line, "a variable has one scope word: static, device or group"));
        }
        let ty = self.ty(&v.ty, v.seq, file, v.line)?;
        self.fvars.push(FVar {
            name: v.name.clone(),
            ty,
            scope: v.scope.first().cloned().unwrap_or_else(|| "user".into()),
            merge: v.merge.clone().unwrap_or_else(|| "last".into()),
            feature: feature.to_string(),
        });
        Ok(())
    }

    fn fvar(&self, name: &str) -> Option<&FVar> {
        self.fvars.iter().find(|f| f.name == name)
    }

    /// the context struct, its storage, its accessors, and
    /// `__zero_reset`, which puts every variable's initial value in it
    fn emit_context(&mut self, store: &Store) -> Result<(), Error> {
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: String::new(), depth: 0, loops: Vec::new() };
        b.line("q: ptr = addr __out_n");
        b.line("store 0: i64, q");
        b.line("a: ptr = addr __arena");
        b.line("h: ptr = addr __heap");
        b.line("arena_init(a, h, 65536)");
        if !self.fvars.is_empty() {
            let mut fields = Vec::new();
            self.type_lines.push(String::new());
            self.type_lines.push("; the context: one field per feature-scope variable — name: scope, merge (feature)".into());
            for f in &self.fvars {
                self.type_lines.push(format!(";   {}: {}, {} ({})", f.name, f.scope, f.merge, f.feature));
                fields.push(format!("{}: {}", f.name, f.ty.ir()));
            }
            self.type_lines.push(format!("type __ctx = struct {{ {} }}", fields.join(", ")));
            self.data.push("data __ctx_mem: array(__ctx, 1)".into());
            // the initial values, in composition order
            let mut inits = Vec::new();
            for feat in &store.features {
                for d in &feat.code.decls {
                    let Decl::Var(v) = d else { continue };
                    b.file = feat.code.file.clone();
                    let ty = self.fvar(&v.name).unwrap().ty.clone();
                    let val = match &v.init {
                        None => self.zero_val(&ty, &mut b),
                        Some(Init::Value(e)) => {
                            let val = self.lower_expr(e, Some(&ty), &mut b, None)?;
                            if !(val.ty == ty || (val.literal && fits_literal(&val, &ty))) {
                                return Err(lex::error(&b.file, e.line, format!("'{}' is {} but the value is {}", v.name, ty.ir(), val.ty.ir())));
                            }
                            val
                        }
                        Some(Init::Construct(args)) => {
                            let Ty::Struct(name) = &ty else {
                                return Err(lex::error(&b.file, v.line, format!("'{}' is not a struct to construct", v.ty)));
                            };
                            self.construct(&name.clone(), args, &mut b, None, v.line)?
                        }
                        Some(Init::Pushes { .. }) => return Err(lex::error(&b.file, v.line, "streams are not in this item yet")),
                    };
                    inits.push(val.text);
                }
            }
            let c = b.tmp();
            b.line(&format!("{}: __ctx = pack {}", c, inits.join(", ")));
            b.line("p: ptr = addr __ctx_mem");
            b.line(&format!("store {}, p", c));
        }
        b.line("ret");
        writeln!(self.out, "\n; before every case: the print buffer emptied, the variables at their initial values").unwrap();
        writeln!(self.out, "fn __zero_reset() {{").unwrap();
        self.out.push_str(&b.out);
        self.out.push_str("}\n");
        for f in &self.fvars {
            let t = f.ty.ir();
            writeln!(self.out, "\nfn __get_{}() -> {} {{\n    p: ptr = addr __ctx_mem\n    c: __ctx = load p\n    v: {} = get c, {}\n    ret v\n}}", f.name, t, t, f.name).unwrap();
            writeln!(self.out, "\nfn __set_{}(v: {}) {{\n    p: ptr = addr __ctx_mem\n    c: __ctx = load p\n    c2: __ctx = set c, {}, v\n    store c2, p\n    ret\n}}", f.name, t, f.name).unwrap();
        }
        Ok(())
    }

    /// a feature variable read: a call to its getter
    fn read_fvar(&mut self, name: &str, b: &mut Body, dst: Option<&str>) -> Val {
        let ty = self.fvar(name).unwrap().ty.clone();
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = __get_{}()", out, ty.ir(), name));
        Val { text: out, ty, literal: false }
    }

    /// a feature variable written: a call to its setter
    fn write_fvar(&mut self, name: &str, v: Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let ty = self.fvar(name).unwrap().ty.clone();
        if !(v.ty == ty || (v.literal && fits_literal(&v, &ty))) {
            return Err(lex::error(&b.file, line, format!("'{}' is {} but the value is {}", name, ty.ir(), v.ty.ir())));
        }
        b.line(&format!("__set_{}({})", name, v.text));
        Ok(())
    }

    fn lower_fn(&mut self, f: &FnDecl, file: &str) -> Result<(), Error> {
        let key = mangle(&f.name);
        let info = self.funcs.iter().find(|g| g.key == key && g.parts == f.name && g.params.len() == f.params().count()).unwrap().clone();
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: info.results.clone(), file: file.to_string(), depth: 0, loops: Vec::new() };
        let mut sig = format!("fn {}(", info.ir);
        for (i, (n, t)) in info.params.iter().enumerate() {
            if i > 0 {
                sig.push_str(", ");
            }
            write!(sig, "{}: {}", n, t.ir()).unwrap();
            if b.vars.contains_key(n) {
                return Err(lex::error(file, f.line, format!("parameter '{}' is named twice", n)));
            }
            b.define(n, t.clone());
        }
        sig.push(')');
        match info.results.len() {
            0 => {}
            1 => write!(sig, " -> {}", info.results[0].1.ir()).unwrap(),
            _ => {
                let ts: Vec<String> = info.results.iter().map(|(_, t)| t.ir()).collect();
                write!(sig, " -> ({})", ts.join(", ")).unwrap();
            }
        }
        for (n, t) in &info.results {
            if b.vars.contains_key(n) {
                return Err(lex::error(file, f.line, format!("result '{}' is also a parameter, or named twice", n)));
            }
            b.declare(n, t.clone());
        }
        writeln!(self.out, "{} {{", sig).unwrap();
        let terminated = self.lower_block(&f.body, &mut b)?;
        // the results' current values; one never assigned is its type's
        // zero. A body that ends in a loop with no way out never returns
        if !terminated {
            let mut rets = Vec::new();
            for (n, t) in &b.results.clone() {
                let v = b.vars[n].clone();
                if !v.set {
                    rets.push(self.zero_val(t, &mut b).text);
                } else {
                    rets.push(v.ir);
                }
            }
            b.line(format!("ret {}", rets.join(", ")).trim_end());
        }
        self.out.push_str(&b.out);
        self.out.push_str("}\n");
        Ok(())
    }

    /// Lower a block's statements; true when the block ends in `break`
    /// or `continue` (or an `if` or `loop` that always does). A
    /// statement after that would never run, and is refused here with
    /// its zero line rather than by the IR with an IR line (log 14).
    fn lower_block(&mut self, stmts: &[Stmt], b: &mut Body) -> Result<bool, Error> {
        for (i, s) in stmts.iter().enumerate() {
            self.lower_stmt(s, b)?;
            if i + 1 < stmts.len() && terminates(&stmts[..=i]) {
                return Err(lex::error(&b.file, stmt_line(&stmts[i + 1]), "this never runs: the statement before it leaves the block"));
            }
        }
        Ok(terminates(stmts))
    }

    /// `if (c)` as a statement (log 11): the variables an arm assigns
    /// become the results of the IR's value-yielding `if`, each arm
    /// yielding its version, so the code after reads the join's names
    fn lower_if(&mut self, cond: &Expr, then: &[Stmt], els: Option<&[Stmt]>, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let cv = self.lower_expr(cond, Some(&Ty::Bool), b, None)?;
        if cv.ty != Ty::Bool {
            return Err(lex::error(&file, cond.line, "'if' takes a bool"));
        }
        let cv = b.materialize(&cv);
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        let t_term = self.lower_block(then, b)?;
        let then_vars = std::mem::replace(&mut b.vars, before.clone());
        let mut then_lines = b.out.split_off(start);
        let (e_term, else_vars) = match els {
            Some(e) => {
                let t = self.lower_block(e, b)?;
                (t, std::mem::replace(&mut b.vars, before.clone()))
            }
            None => (false, before.clone()),
        };
        let mut else_lines = b.out.split_off(start);
        b.depth -= 1;
        // the join's results: the variables from before that an arm
        // which falls through has given a new value
        let differs = |arm: &HashMap<String, Var>, n: &str| arm[n].ir != before[n].ir || arm[n].set != before[n].set;
        let mut changed: Vec<String> = before.keys().filter(|n| (!t_term && differs(&then_vars, n)) || (!e_term && differs(&else_vars, n))).cloned().collect();
        changed.sort();
        // what each arm yields: its version, or the type's zero for a
        // variable that has none yet on that path
        let mut yields = |lines: &mut String, vars: &HashMap<String, Var>, b: &mut Body| -> Vec<String> {
            let saved = std::mem::replace(&mut b.out, std::mem::take(lines));
            b.depth += 1;
            let mut out = Vec::new();
            for n in &changed {
                let v = &vars[n];
                if v.set {
                    out.push(v.ir.clone());
                } else {
                    let ty = v.ty.clone();
                    out.push(self.zero_val(&ty, b).text);
                }
            }
            b.depth -= 1;
            *lines = std::mem::replace(&mut b.out, saved);
            out
        };
        let then_yields = if t_term { Vec::new() } else { yields(&mut then_lines, &then_vars, b) };
        let else_yields = if e_term { Vec::new() } else { yields(&mut else_lines, &else_vars, b) };
        let head = if changed.is_empty() {
            format!("if {} {{", cv.text)
        } else {
            let defs: Vec<String> = changed.iter().map(|n| {
                let ty = before[n].ty.clone();
                format!("{}: {}", b.define(n, ty.clone()), ty.ir())
            }).collect();
            format!("{} = if {} {{", defs.join(", "), cv.text)
        };
        b.line(&head);
        b.out.push_str(&then_lines);
        if !t_term && !changed.is_empty() {
            b.depth += 1;
            b.line(&format!("yield {}", then_yields.join(", ")));
            b.depth -= 1;
        }
        if els.is_some() || !changed.is_empty() {
            b.line("} else {");
            b.out.push_str(&else_lines);
            if !e_term && !changed.is_empty() {
                b.depth += 1;
                b.line(&format!("yield {}", else_yields.join(", ")));
                b.depth -= 1;
            }
        }
        b.line("}");
        Ok(())
    }

    /// `loop (vars) while (c) bound N` (log 12): the carried variables
    /// are the header's, `while` is tested at the top of every pass, a
    /// body that falls off its end continues with the current versions,
    /// every `break` yields them, and after the loop the variables hold
    /// the values it left with
    fn lower_loop(&mut self, vars: &[super::syntax::VarDecl], cond: Option<&Expr>, bound: Option<i64>, body: &[Stmt], line: usize, b: &mut Body) -> Result<bool, Error> {
        let file = b.file.clone();
        // the initial values, in the block before the loop
        let mut header = Vec::new();
        let mut carried = Vec::new();
        let mut tys = Vec::new();
        for v in vars {
            if !v.scope.is_empty() || v.merge.is_some() {
                return Err(lex::error(&file, v.line, "a scope word or 'merge' belongs on a feature-scope variable"));
            }
            if b.vars.contains_key(&v.name) || carried.contains(&v.name) {
                return Err(lex::error(&file, v.line, format!("'{}' is already declared", v.name)));
            }
            let ty = self.ty(&v.ty, v.seq, &file, v.line)?;
            let init = match &v.init {
                None => self.zero_val(&ty, b),
                Some(Init::Value(e)) => {
                    let val = self.lower_expr(e, Some(&ty), b, None)?;
                    if !(val.ty == ty || (val.literal && fits_literal(&val, &ty))) {
                        return Err(lex::error(&file, e.line, format!("'{}' is {} but the value is {}", v.name, ty.ir(), val.ty.ir())));
                    }
                    val
                }
                Some(Init::Construct(args)) => {
                    let Ty::Struct(name) = &ty else {
                        return Err(lex::error(&file, v.line, format!("'{}' is not a struct to construct", v.ty)));
                    };
                    self.construct(&name.clone(), args, b, None, v.line)?
                }
                Some(Init::Pushes { .. }) => return Err(lex::error(&file, v.line, "streams are not in this item yet")),
            };
            header.push((v.name.clone(), ty.clone(), init.text));
            carried.push(v.name.clone());
            tys.push(ty);
        }
        // the carried variables are declared inside the loop
        b.loops.push(LoopCtx { carried: carried.clone(), item: None, loaded: None, breaks: 0 });
        let mut hdr = Vec::new();
        for (n, ty, init) in &header {
            let ir = b.define(n, ty.clone());
            hdr.push(format!("{}: {} = {}", ir, ty.ir(), init));
        }
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        if let Some(c) = cond {
            let cv = self.lower_expr(c, Some(&Ty::Bool), b, None)?;
            if cv.ty != Ty::Bool {
                return Err(lex::error(&file, c.line, "'while' takes a bool"));
            }
            let cv = b.materialize(&cv);
            b.line(&format!("if {} {{", cv.text));
            b.line("} else {");
            b.depth += 1;
            let vals = b.current(&carried);
            b.line(format!("break {}", vals.join(", ")).trim_end());
            b.depth -= 1;
            b.line("}");
            b.loops.last_mut().unwrap().breaks += 1;
        }
        let terminated = self.lower_block(body, b)?;
        if !terminated {
            let vals = b.current(&carried);
            b.line(format!("continue {}", vals.join(", ")).trim_end());
        }
        b.depth -= 1;
        let body_lines = b.out.split_off(start);
        let ctx = b.loops.pop().unwrap();
        b.vars = before;
        let bound = match bound {
            Some(n) if n > 0 => format!(" bound {}", n),
            Some(_) => return Err(lex::error(&file, line, "'bound' takes a positive number")),
            None => String::new(),
        };
        // a loop with no way out has no results and nothing after it
        if ctx.breaks == 0 {
            b.line(&format!("loop({}){} {{", hdr.join(", "), bound));
            b.out.push_str(&body_lines);
            b.line("}");
            for n in &carried {
                b.vars.remove(n);
            }
            return Ok(true);
        }
        let depth = b.loops.len();
        let mut results = Vec::new();
        for (n, ty) in carried.iter().zip(&tys) {
            let ir = b.define(n, ty.clone());
            b.vars.get_mut(n).unwrap().loop_depth = depth;
            results.push(format!("{}: {}", ir, ty.ir()));
        }
        b.line(&format!("{} = loop({}){} {{", results.join(", "), hdr.join(", "), bound));
        b.out.push_str(&body_lines);
        b.line("}");
        Ok(false)
    }

    /// `for (x in [a through b])`, `[a to b]` (log 13): a loop whose
    /// carried variable is the item, stepped by one each pass. Literal
    /// bounds fix the direction, and the IR is the plain counted loop
    /// `probe cost` reads; otherwise the step is chosen at run time and
    /// the test is on the signed distance left
    fn lower_for(&mut self, var: &str, seq: &Expr, bound: Option<i64>, body: &[Stmt], b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let ExprKind::Range { from, to, inclusive } = &seq.kind else {
            return self.lower_for_seq(var, seq, bound, body, b);
        };
        if b.vars.contains_key(var) {
            return Err(lex::error(&file, seq.line, format!("'{}' is already declared", var)));
        }
        for e in [from, to] {
            if matches!(e.kind, ExprKind::Float(_)) {
                return Err(lex::error(&file, e.line, "a range's bounds are integers"));
            }
        }
        let mut fv = self.lower_expr(from, None, b, None)?;
        let mut tv = self.lower_expr(to, if fv.literal { None } else { Some(&fv.ty) }, b, None)?;
        if fv.literal && !tv.literal {
            fv.ty = tv.ty.clone();
        }
        if tv.literal && !fv.literal {
            tv.ty = fv.ty.clone();
        }
        let ty = fv.ty.clone();
        if !matches!(ty, Ty::Num(_)) || ty != tv.ty {
            return Err(lex::error(&file, seq.line, format!("a range runs over one number type, given {} and {}", fv.ty.ir(), tv.ty.ir())));
        }
        let bound = match bound {
            Some(n) if n > 0 => format!(" bound {}", n),
            Some(_) => return Err(lex::error(&file, seq.line, "'bound' takes a positive number")),
            None => String::new(),
        };
        // the direction, and the test that leaves
        let (op, step, test) = if fv.literal && tv.literal {
            let a: i64 = fv.text.parse().map_err(|_| lex::error(&file, from.line, "a range's bounds are integers"))?;
            let z: i64 = tv.text.parse().map_err(|_| lex::error(&file, to.line, "a range's bounds are integers"))?;
            let up = a <= z;
            let cc = match (up, *inclusive) {
                (true, true) => "cmp.le",
                (true, false) => "cmp.lt",
                (false, true) => "cmp.ge",
                (false, false) => "cmp.gt",
            };
            (if up { "add" } else { "sub" }, "1".to_string(), Some(format!("{} {{}}, {}", cc, tv.text)))
        } else {
            let Ty::Num(n) = &ty else { unreachable!() };
            if !(n == "int" || (n.starts_with('i') && n[1..].parse::<u32>().is_ok())) {
                return Err(lex::error(&file, seq.line, format!("a range with a bound that is not a literal counts over a signed integer, not {}", n)));
            }
            let fv2 = b.materialize(&fv);
            let tv2 = b.materialize(&tv);
            fv = fv2;
            tv = tv2;
            let d = b.tmp();
            b.line(&format!("{}: {} = sub {}, {}", d, ty.ir(), tv.text, fv.text));
            let down = b.tmp();
            b.line(&format!("{}: u1 = cmp.lt {}, 0", down, d));
            let step = b.tmp();
            b.line(&format!("{}: {} = if {} {{", step, ty.ir(), down));
            b.depth += 1;
            b.line("yield -1");
            b.depth -= 1;
            b.line("} else {");
            b.depth += 1;
            b.line("yield 1");
            b.depth -= 1;
            b.line("}");
            ("add", step, None)
        };
        b.loops.push(LoopCtx { carried: Vec::new(), item: Some((var.to_string(), op, step.clone())), loaded: None, breaks: 1 });
        let x = b.define(var, ty.clone());
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        let done = match &test {
            Some(t) => {
                let done = b.tmp();
                b.line(&format!("{}: u1 = {}", done, t.replace("{}", &x)));
                done
            }
            None => {
                let left = b.tmp();
                b.line(&format!("{}: {} = sub {}, {}", left, ty.ir(), tv.text, x));
                let signed = b.tmp();
                b.line(&format!("{}: {} = mul {}, {}", signed, ty.ir(), left, step));
                let done = b.tmp();
                b.line(&format!("{}: u1 = {} {}, 0", done, if *inclusive { "cmp.lt" } else { "cmp.le" }, signed));
                done
            }
        };
        if test.is_some() {
            b.line(&format!("if {} {{", done));
            b.line("} else {");
            b.depth += 1;
            b.line("break");
            b.depth -= 1;
            b.line("}");
        } else {
            b.line(&format!("if {} {{", done));
            b.depth += 1;
            b.line("break");
            b.depth -= 1;
            b.line("}");
        }
        let terminated = self.lower_block(body, b)?;
        if !terminated {
            self.step_for(b);
        }
        b.depth -= 1;
        let body_lines = b.out.split_off(start);
        b.loops.pop();
        b.vars = before;
        b.vars.remove(var);
        b.line(&format!("loop({}: {} = {}){} {{", x, ty.ir(), fv.text, bound));
        b.out.push_str(&body_lines);
        b.line("}");
        Ok(())
    }

    /// `for (x in items$)`: a loop over the index, the item loaded at
    /// the top of each pass and gone after the loop
    fn lower_for_seq(&mut self, var: &str, seq: &Expr, bound: Option<i64>, body: &[Stmt], b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let sv = self.lower_expr(seq, None, b, None)?;
        let Some(e) = sv.ty.elem().cloned() else {
            return Err(lex::error(&file, seq.line, format!("a `for` runs over a sequence or a range, not a {}", sv.ty.ir())));
        };
        if b.vars.contains_key(var) {
            return Err(lex::error(&file, seq.line, format!("'{}' is already declared", var)));
        }
        let bound = match bound {
            Some(n) if n > 0 => format!(" bound {}", n),
            Some(_) => return Err(lex::error(&file, seq.line, "'bound' takes a positive number")),
            None => String::new(),
        };
        let n = b.tmp();
        b.line(&format!("{}: i64 = len {}", n, sv.text));
        let k = b.tmp();
        b.loops.push(LoopCtx { carried: Vec::new(), item: Some((k.clone(), "add", "1".into())), loaded: Some(var.to_string()), breaks: 1 });
        let depth = b.loops.len();
        b.vars.insert(k.clone(), Var { ir: k.clone(), ty: Ty::Num("i64".into()), set: true, loop_depth: depth });
        let before = b.vars.clone();
        let start = b.out.len();
        b.depth += 1;
        let done = b.tmp();
        b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
        b.line(&format!("if {} {{", done));
        b.depth += 1;
        b.line("break");
        b.depth -= 1;
        b.line("}");
        let x = b.define(var, e.clone());
        b.line(&format!("{}: {} = load {}, {}", x, e.ir(), sv.text, k));
        let terminated = self.lower_block(body, b)?;
        if !terminated {
            self.step_for(b);
        }
        b.depth -= 1;
        let body_lines = b.out.split_off(start);
        b.loops.pop();
        b.vars = before;
        b.vars.remove(var);
        b.vars.remove(&k);
        b.line(&format!("loop({}: i64 = 0){} {{", k, bound));
        b.out.push_str(&body_lines);
        b.line("}");
        Ok(())
    }

    /// a `for`'s pass ends: the item stepped, and `continue` with it
    fn step_for(&mut self, b: &mut Body) {
        let (var, op, step) = b.loops.last().unwrap().item.clone().unwrap();
        let v = b.vars[&var].clone();
        let next = b.tmp();
        b.line(&format!("{}: {} = {} {}, {}", next, v.ty.ir(), op, v.ir, step));
        b.line(&format!("continue {}", next));
    }

    /// a type's zero: a literal for a number, a bool or an enumeration,
    /// the empty string, a struct of its defaults
    fn zero_val(&mut self, t: &Ty, b: &mut Body) -> Val {
        match t {
            Ty::Bool | Ty::Num(_) | Ty::Enum(_) => Val { text: "0".into(), ty: t.clone(), literal: true },
            Ty::Seq(e) => {
                // the empty view: nothing at the null byte
                let p = b.tmp();
                b.line(&format!("{}: ptr = addr __nul", p));
                let q = b.tmp();
                b.line(&format!("{}: ptr({}) = cast {}", q, e.ir(), p));
                let n = b.tmp();
                b.line(&format!("{}: {} = pack {}, 0, 1", n, t.ir(), q));
                Val { text: n, ty: t.clone(), literal: false }
            }
            Ty::Struct(name) => self.construct(name, &[], b, None, 0).unwrap_or(Val { text: "0".into(), ty: t.clone(), literal: true }),
            Ty::None => Val { text: String::new(), ty: Ty::None, literal: false },
        }
    }

    /// a struct from its arguments, by position or by name, the rest
    /// from the fields' defaults: `Vec(1, 2, 3)`, `Vec v(z = 3)`, `Vec v`
    fn construct(&mut self, name: &str, args: &[Arg], b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let Some(TypeInfo::Struct(fields)) = self.types.get(name).cloned() else {
            return Err(lex::error(&file, line, format!("'{}' is not a struct", name)));
        };
        let named = args.iter().any(|a| a.name.is_some());
        if named && args.iter().any(|a| a.name.is_none()) {
            return Err(lex::error(&file, line, "arguments are all by position or all by name"));
        }
        if !named && args.len() > fields.len() {
            return Err(lex::error(&file, line, format!("{} has {} field(s), given {}", name, fields.len(), args.len())));
        }
        for a in args.iter().filter_map(|a| a.name.as_ref()) {
            if !fields.iter().any(|(n, _, _)| n == a) {
                return Err(lex::error(&file, line, format!("{} has no field '{}'", name, a)));
            }
        }
        let mut ops = Vec::new();
        for (i, (fname, fty, default)) in fields.iter().enumerate() {
            let given = if named { args.iter().find(|a| a.name.as_deref() == Some(fname)) } else { args.get(i) };
            let v = match given {
                Some(a) => {
                    let v = self.lower_expr(&a.value, Some(fty), b, None)?;
                    if !(v.ty == *fty || (v.literal && fits_literal(&v, fty))) {
                        return Err(lex::error(&file, a.value.line, format!("field '{}' of {} is {}, given a {}", fname, name, fty.ir(), v.ty.ir())));
                    }
                    v
                }
                None => match default {
                    Some(d) => Val { text: d.clone(), ty: fty.clone(), literal: true },
                    None => self.zero_val(fty, b),
                },
            };
            ops.push(v.text);
        }
        let ty = Ty::Struct(name.to_string());
        let out = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = pack {}", out, name, ops.join(", ")));
        Ok(Val { text: out, ty, literal: false })
    }

    /// assign a value to a declared variable: a literal becomes a
    /// `const` under the variable's name; a value that already has a
    /// name is not copied — the variable names it too
    fn assign(&mut self, name: &str, v: Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let var = b.vars[name].clone();
        if v.literal {
            if !fits_literal(&v, &var.ty) {
                return Err(lex::error(&b.file, line, format!("'{}' is {} but the value is {}", name, var.ty.ir(), v.ty.ir())));
            }
            let ir = b.define(name, var.ty.clone());
            b.line(&format!("{}: {} = const {}", ir, var.ty.ir(), v.text));
            return Ok(());
        }
        if v.ty != var.ty {
            return Err(lex::error(&b.file, line, format!("'{}' is {} but the value is {}", name, var.ty.ir(), v.ty.ir())));
        }
        if var.ir != v.text {
            let v2 = b.vars.get_mut(name).unwrap();
            v2.ir = v.text;
            v2.set = true;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, s: &Stmt, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        match s {
            Stmt::Assign { targets, value, line } => {
                // a local is a new SSA version; a feature variable is a
                // call to its setter, allowed anywhere
                for t in targets {
                    if b.vars.contains_key(&t.name) {
                        b.assignable(&t.name, t.line)?;
                    } else if self.fvar(&t.name).is_none() {
                        return Err(lex::error(&file, t.line, format!("'{}' is not declared: a variable is its type then its name", t.name)));
                    }
                }
                if targets.len() == 1 {
                    let t = &targets[0];
                    if !b.vars.contains_key(&t.name) {
                        let ty = self.fvar(&t.name).unwrap().ty.clone();
                        let v = self.lower_expr(value, Some(&ty), b, None)?;
                        return self.write_fvar(&t.name, v, b, *line);
                    }
                    let ty = b.vars[&t.name].ty.clone();
                    let v = self.lower_expr(value, Some(&ty), b, Some(&t.name))?;
                    return self.assign(&t.name, v, b, *line);
                }
                let names: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();
                let tys: Vec<Ty> = names.iter().map(|n| match b.vars.get(n) { Some(v) => v.ty.clone(), None => self.fvar(n).unwrap().ty.clone() }).collect();
                self.lower_multi(value, &names, &tys, b, *line)
            }
            Stmt::Multi { vars, value, line } => {
                let mut names = Vec::new();
                let mut tys = Vec::new();
                for p in vars {
                    if b.vars.contains_key(&p.name) {
                        return Err(lex::error(&file, p.line, format!("'{}' is already declared", p.name)));
                    }
                    let ty = self.ty(&p.ty, p.seq, &file, p.line)?;
                    b.declare(&p.name, ty.clone());
                    names.push(p.name.clone());
                    tys.push(ty);
                }
                self.lower_multi(value, &names, &tys, b, *line)
            }
            Stmt::Var(v) => {
                if !v.scope.is_empty() || v.merge.is_some() {
                    return Err(lex::error(&file, v.line, "a scope word or 'merge' belongs on a feature-scope variable"));
                }
                if b.vars.contains_key(&v.name) {
                    return Err(lex::error(&file, v.line, format!("'{}' is already declared", v.name)));
                }
                let ty = self.ty(&v.ty, v.seq, &file, v.line)?;
                b.declare(&v.name, ty.clone());
                match &v.init {
                    None => {
                        let z = self.zero_val(&ty, b);
                        self.assign(&v.name, z, b, v.line)
                    }
                    Some(Init::Value(e)) => {
                        let val = self.lower_expr(e, Some(&ty), b, Some(&v.name))?;
                        self.assign(&v.name, val, b, v.line)
                    }
                    Some(Init::Construct(args)) => {
                        let Ty::Struct(name) = &ty else {
                            return Err(lex::error(&file, v.line, format!("'{}' is not a struct to construct", v.ty)));
                        };
                        let val = self.construct(&name.clone(), args, b, Some(&v.name), v.line)?;
                        self.assign(&v.name, val, b, v.line)
                    }
                    Some(Init::Pushes { .. }) => Err(lex::error(&file, v.line, "streams are not in this item yet")),
                }
            }
            Stmt::Expr { expr, line } => {
                match &expr.kind {
                    ExprKind::Phrase(_) | ExprKind::Existing(_) => {}
                    _ => return Err(lex::error(&file, *line, "a statement is a call, an assignment or a declaration")),
                }
                self.lower_expr(expr, None, b, None)?;
                Ok(())
            }
            Stmt::If { cond, then, els, .. } => self.lower_if(cond, then, els.as_deref(), b),
            Stmt::Loop { vars, cond, bound, body, line } => self.lower_loop(vars, cond.as_ref(), *bound, body, *line, b).map(|_| ()),
            Stmt::For { var, seq, bound, body, .. } => self.lower_for(var, seq, *bound, body, b),
            Stmt::Break { line } => {
                if b.loops.is_empty() {
                    return Err(lex::error(&file, *line, "'break' outside a loop"));
                }
                let ctx = b.loops.last_mut().unwrap();
                ctx.breaks += 1;
                let carried = if ctx.item.is_some() { Vec::new() } else { ctx.carried.clone() };
                let vals = b.current(&carried);
                b.line(format!("break {}", vals.join(", ")).trim_end());
                Ok(())
            }
            Stmt::Continue { values, line } => {
                let Some(ctx) = b.loops.last() else {
                    return Err(lex::error(&file, *line, "'continue' outside a loop"));
                };
                if ctx.item.is_some() {
                    if !values.is_empty() {
                        return Err(lex::error(&file, *line, "a `for` steps its item by itself: 'continue' takes no values here"));
                    }
                    self.step_for(b);
                    return Ok(());
                }
                let carried = ctx.carried.clone();
                if values.is_empty() {
                    let vals = b.current(&carried);
                    b.line(format!("continue {}", vals.join(", ")).trim_end());
                    return Ok(());
                }
                if values.len() != carried.len() {
                    return Err(lex::error(&file, *line, format!("the loop carries {} variable(s), 'continue' gives {}", carried.len(), values.len())));
                }
                let mut vals = Vec::new();
                for (e, n) in values.iter().zip(&carried) {
                    let ty = b.vars[n].ty.clone();
                    let v = self.lower_expr(e, Some(&ty), b, None)?;
                    if !(v.ty == ty || (v.literal && fits_literal(&v, &ty))) {
                        return Err(lex::error(&file, e.line, format!("'{}' is {} but 'continue' gives a {}", n, ty.ir(), v.ty.ir())));
                    }
                    vals.push(v.text);
                }
                b.line(&format!("continue {}", vals.join(", ")));
                Ok(())
            }
            Stmt::Check { line, .. } => Err(lex::error(&file, *line, "checks are not in this item yet")),
            Stmt::Push { line, .. } => Err(lex::error(&file, *line, "streams are not in this item yet")),
        }
    }

    /// `q, r = f(...)`: a call with several results defines several variables
    fn lower_multi(&mut self, value: &Expr, names: &[String], tys: &[Ty], b: &mut Body, line: usize) -> Result<(), Error> {
        let file = b.file.clone();
        let ExprKind::Phrase(parts) = &value.kind else {
            return Err(lex::error(&file, line, "several variables at once take a call with several results"));
        };
        let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
        let (info, args) = find_function(&self.funcs, parts, &is_var, &file, line)?;
        let info = info.clone();
        if info.results.len() != names.len() {
            return Err(lex::error(&file, line, format!("'{}' gives {} result(s), {} wanted", info.key, info.results.len(), names.len())));
        }
        let (ops, rtys) = self.lower_args(&info, &args, b)?;
        for ((n, want), got) in names.iter().zip(tys).zip(&rtys) {
            if want != got {
                return Err(lex::error(&file, line, format!("'{}' is {} but '{}' gives {}", n, want.ir(), info.key, got.ir())));
            }
        }
        // a feature variable among the targets takes its result through
        // a temporary and its setter
        let mut sets = Vec::new();
        let defs: Vec<String> = names
            .iter()
            .zip(tys)
            .map(|(n, t)| {
                if b.vars.contains_key(n) {
                    format!("{}: {}", b.define(n, t.clone()), t.ir())
                } else {
                    let tmp = b.tmp();
                    sets.push((n.clone(), tmp.clone()));
                    format!("{}: {}", tmp, t.ir())
                }
            })
            .collect();
        b.line(&format!("{} = {}({})", defs.join(", "), info.ir, ops.join(", ")));
        for (n, tmp) in sets {
            b.line(&format!("__set_{}({})", n, tmp));
        }
        Ok(())
    }

    /// a call's arguments lowered against its parameters, and its result
    /// types with the tower's abstract names bound by the arguments; an
    /// argument that is a sequence where the parameter is an item is
    /// marked lifted (a map), and `_` marks the accumulator (a reduce)
    fn lower_call_args(&mut self, info: &FnInfo, args: &[Expr], b: &mut Body) -> Result<(Vec<Val>, Vec<bool>, Option<(usize, Ty)>, Vec<Ty>), Error> {
        let file = b.file.clone();
        let mut vals = Vec::new();
        let mut lifted = Vec::new();
        let mut acc = None;
        let mut bound: HashMap<String, Ty> = HashMap::new();
        for (i, (a, (_, ty))) in args.iter().zip(&info.params).enumerate() {
            if matches!(a.kind, ExprKind::Acc) {
                if acc.is_some() {
                    return Err(lex::error(&file, a.line, "one '_' marks the accumulator"));
                }
                acc = Some(i);
                vals.push(Val { text: "_".into(), ty: ty.clone(), literal: false });
                lifted.push(false);
                continue;
            }
            let mut v = self.lower_expr(a, Some(ty), b, None)?;
            if v.literal && fits_literal(&v, ty) {
                v.ty = ty.clone();
            }
            // the item's type is what the tower sees
            let item = match (v.ty.elem(), ty) {
                (Some(e), Ty::Seq(_)) => {
                    let _ = e;
                    v.ty.clone()
                }
                (Some(e), _) => {
                    lifted.push(true);
                    e.clone()
                }
                _ => v.ty.clone(),
            };
            if lifted.len() < vals.len() + 1 {
                lifted.push(false);
            }
            if !fits(&item, ty) {
                return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given a {}", info.key, ty.ir(), v.ty.ir())));
            }
            if let Some(name) = ty.abstract_name() {
                if &item != ty {
                    bound.entry(name.to_string()).or_insert(item.clone());
                }
            }
            vals.push(v);
        }
        let resolve = |t: &Ty| match t.abstract_name().and_then(|n| bound.get(n)) {
            Some(bt) => bt.clone(),
            None => t.clone(),
        };
        let rtys = info.results.iter().map(|(_, t)| resolve(t)).collect();
        let acc = acc.map(|i| {
            let aty = resolve(&info.params[i].1);
            vals[i].ty = aty.clone();
            (i, aty)
        });
        Ok((vals, lifted, acc, rtys))
    }

    /// a call's arguments where no sequence is lifted: the operand texts
    fn lower_args(&mut self, info: &FnInfo, args: &[Expr], b: &mut Body) -> Result<(Vec<String>, Vec<Ty>), Error> {
        let file = b.file.clone();
        let (vals, lifted, acc, rtys) = self.lower_call_args(info, args, b)?;
        if lifted.iter().any(|&l| l) || acc.is_some() {
            return Err(lex::error(&file, args[0].line, format!("'{}' gives several results: it is not mapped over a sequence", info.key)));
        }
        Ok((vals.into_iter().map(|v| v.text).collect(), rtys))
    }

    /// Map, zip and reduce (log 19): `vals` are an operation's operands,
    /// those marked lifted being sequences whose items the operation
    /// takes one at a time; `op` emits the operation on one set of items.
    /// With no accumulator the results make a new sequence, as long as
    /// the longest input, a shorter one reading as zero past its end;
    /// with one, the operation folds over the one sequence, from its
    /// first item, an empty sequence giving the accumulator's zero.
    fn lift(&mut self, mut vals: Vec<Val>, lifted: Vec<bool>, acc: Option<(usize, Ty)>, b: &mut Body, dst: Option<&str>, line: usize, op: &dyn Fn(&mut Lowerer, &[Val], &mut Body) -> Result<Val, Error>) -> Result<Val, Error> {
        let file = b.file.clone();
        let seqs: Vec<usize> = (0..vals.len()).filter(|&i| lifted[i]).collect();
        if acc.is_some() && seqs.len() != 1 {
            return Err(lex::error(&file, line, "a reduction folds one sequence"));
        }
        let mut lens = Vec::new();
        for &i in &seqs {
            let n = b.tmp();
            b.line(&format!("{}: i64 = len {}", n, vals[i].text));
            lens.push(n);
        }
        let mut n = lens[0].clone();
        for l in &lens[1..] {
            let m = b.tmp();
            b.line(&format!("{}: i64 = max({}, {})", m, n, l));
            n = m;
        }
        let k = b.tmp();
        let zip = seqs.len() > 1;
        // the items of this pass, a shorter sequence's zero past its end
        let load_items = |b: &mut Body, vals: &mut Vec<Val>, from: &str| {
            for (j, &i) in seqs.iter().enumerate() {
                let e = vals[i].ty.elem().unwrap().clone();
                let x = b.tmp();
                if zip {
                    let inside = b.tmp();
                    b.line(&format!("{}: u1 = cmp.lt {}, {}", inside, from, lens[j]));
                    b.line(&format!("{}: {} = if {} {{", x, e.ir(), inside));
                    b.depth += 1;
                    let y = b.tmp();
                    b.line(&format!("{}: {} = load {}, {}", y, e.ir(), vals[i].text, from));
                    b.line(&format!("yield {}", y));
                    b.depth -= 1;
                    b.line("} else {");
                    b.depth += 1;
                    b.line("yield 0");
                    b.depth -= 1;
                    b.line("}");
                } else {
                    b.line(&format!("{}: {} = load {}, {}", x, e.ir(), vals[i].text, from));
                }
                vals[i] = Val { text: x, ty: e, literal: false };
            }
        };
        match acc {
            None => {
                let c = b.tmp();
                let start = b.out.len();
                b.depth += 1;
                let done = b.tmp();
                b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
                b.line(&format!("if {} {{", done));
                b.depth += 1;
                b.line("break");
                b.depth -= 1;
                b.line("}");
                load_items(b, &mut vals, &k);
                let r = op(self, &vals, b)?;
                let r = b.materialize(&r);
                b.line(&format!("store {}, {}, {}", r.text, c, k));
                let k2 = b.tmp();
                b.line(&format!("{}: i64 = add {}, 1", k2, k));
                b.line(&format!("continue {}", k2));
                b.depth -= 1;
                let body = b.out.split_off(start);
                let rty = Ty::Seq(Box::new(r.ty.clone()));
                self.news.insert(r.ty.ir());
                b.line(&format!("{}: {} = __new_{}({})", c, rty.ir(), r.ty.ir(), n));
                b.line(&format!("loop({}: i64 = 0) {{", k));
                b.out.push_str(&body);
                b.line("}");
                Ok(Val { text: c, ty: rty, literal: false })
            }
            Some((ai, aty)) => {
                let si = seqs[0];
                let e = vals[si].ty.elem().unwrap().clone();
                let v = vals[si].text.clone();
                let empty = b.tmp();
                b.line(&format!("{}: u1 = cmp.eq {}, 0", empty, n));
                let seed = b.tmp();
                b.line(&format!("{}: {} = if {} {{", seed, aty.ir(), empty));
                b.depth += 1;
                b.line("yield 0");
                b.depth -= 1;
                b.line("} else {");
                b.depth += 1;
                let first = b.tmp();
                b.line(&format!("{}: {} = load {}, 0", first, e.ir(), v));
                if e != aty {
                    return Err(lex::error(&file, line, format!("the accumulator is a {} but the items are {}", aty.ir(), e.ir())));
                }
                b.line(&format!("yield {}", first));
                b.depth -= 1;
                b.line("}");
                let a = b.tmp();
                let start = b.out.len();
                b.depth += 1;
                let done = b.tmp();
                b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
                b.line(&format!("if {} {{", done));
                b.depth += 1;
                b.line(&format!("break {}", a));
                b.depth -= 1;
                b.line("}");
                load_items(b, &mut vals, &k);
                vals[ai] = Val { text: a.clone(), ty: aty.clone(), literal: false };
                let r = op(self, &vals, b)?;
                let r = b.materialize(&r);
                if r.ty != aty {
                    return Err(lex::error(&file, line, format!("the reduction gives a {} but its accumulator is a {}", r.ty.ir(), aty.ir())));
                }
                let k2 = b.tmp();
                b.line(&format!("{}: i64 = add {}, 1", k2, k));
                b.line(&format!("continue {}, {}", k2, r.text));
                b.depth -= 1;
                let body = b.out.split_off(start);
                let out = name_for(dst, &aty, b);
                b.line(&format!("{}: {} = loop({}: i64 = 1, {}: {} = {}) {{", out, aty.ir(), k, a, aty.ir(), seed));
                b.out.push_str(&body);
                b.line("}");
                Ok(Val { text: out, ty: aty, literal: false })
            }
        }
    }

    /// `[a, b, c]`: a new sequence holding the items
    fn lower_list(&mut self, items: &[Expr], want: Option<&Ty>, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let want_e = want.and_then(|t| t.elem()).cloned();
        let mut vals = Vec::new();
        for it in items {
            vals.push(self.lower_expr(it, want_e.as_ref(), b, None)?);
        }
        let e = want_e.or_else(|| vals.iter().find(|v| !v.literal).map(|v| v.ty.clone())).or_else(|| vals.first().map(|v| v.ty.clone()));
        let Some(e) = e else {
            return Err(lex::error(&file, line, "an empty list needs a type: declare the sequence, `int i$`"));
        };
        if !matches!(e, Ty::Num(_) | Ty::Enum(_)) {
            return Err(lex::error(&file, line, format!("a sequence of {}: only numbers and enumerations in this milestone", e.ir())));
        }
        for (it, v) in items.iter().zip(&vals) {
            if !(v.ty == e || (v.literal && fits_literal(v, &e))) {
                return Err(lex::error(&file, it.line, format!("the items are {}, this one is a {}", e.ir(), v.ty.ir())));
            }
        }
        let c = self.new_seq(&e, &vals.len().to_string(), b, dst);
        for (i, v) in vals.iter().enumerate() {
            // a literal stored through a view takes the item's type
            b.line(&format!("store {}, {}, {}", v.text, c.text, i));
        }
        Ok(c)
    }

    /// `[a through b]`, `[a to b]`: the items counted from the bounds
    /// and filled by a loop, counting down when a > b
    fn lower_range(&mut self, from: &Expr, to: &Expr, inclusive: bool, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        for e in [from, to] {
            if matches!(e.kind, ExprKind::Float(_)) {
                return Err(lex::error(&file, e.line, "a range's bounds are integers"));
            }
        }
        let mut fv = self.lower_expr(from, None, b, None)?;
        let mut tv = self.lower_expr(to, if fv.literal { None } else { Some(&fv.ty) }, b, None)?;
        if fv.literal && !tv.literal {
            fv.ty = tv.ty.clone();
        }
        if tv.literal && !fv.literal {
            tv.ty = fv.ty.clone();
        }
        let ty = fv.ty.clone();
        let signed = matches!(&ty, Ty::Num(n) if n == "int" || (n.starts_with('i') && n[1..].parse::<u32>().is_ok()));
        if !signed || ty != tv.ty {
            return Err(lex::error(&file, line, format!("a range counts over a signed integer, given {} and {}", fv.ty.ir(), tv.ty.ir())));
        }
        let fv = b.materialize(&fv);
        let tv = b.materialize(&tv);
        let d = b.tmp();
        b.line(&format!("{}: {} = sub {}, {}", d, ty.ir(), tv.text, fv.text));
        let down = b.tmp();
        b.line(&format!("{}: u1 = cmp.lt {}, 0", down, d));
        let step = b.tmp();
        b.line(&format!("{}: {} = if {} {{", step, ty.ir(), down));
        b.depth += 1;
        b.line("yield -1");
        b.depth -= 1;
        b.line("} else {");
        b.depth += 1;
        b.line("yield 1");
        b.depth -= 1;
        b.line("}");
        let span = b.tmp();
        b.line(&format!("{}: {} = mul {}, {}", span, ty.ir(), d, step));
        let count = if inclusive {
            let c = b.tmp();
            b.line(&format!("{}: {} = add {}, 1", c, ty.ir(), span));
            c
        } else {
            span
        };
        let n = b.tmp();
        b.line(&format!("{}: i64 = conv {}", n, count));
        let c = self.new_seq(&ty, &n, b, dst);
        let k = b.tmp();
        let x = b.tmp();
        b.line(&format!("loop({}: i64 = 0, {}: {} = {}) {{", k, x, ty.ir(), fv.text));
        b.depth += 1;
        let done = b.tmp();
        b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
        b.line(&format!("if {} {{", done));
        b.depth += 1;
        b.line("break");
        b.depth -= 1;
        b.line("}");
        b.line(&format!("store {}, {}, {}", x, c.text, k));
        let k2 = b.tmp();
        b.line(&format!("{}: i64 = add {}, 1", k2, k));
        let x2 = b.tmp();
        b.line(&format!("{}: {} = add {}, {}", x2, ty.ir(), x, step));
        b.line(&format!("continue {}, {}", k2, x2));
        b.depth -= 1;
        b.line("}");
        Ok(c)
    }

    /// arithmetic or a comparison on two scalars: a literal takes the
    /// other side's type, two literals make one a value first
    fn emit_bin(&mut self, op: &str, mut lv: Val, mut rv: Val, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let cmp = is_comparison(op);
        if lv.literal && !rv.literal {
            lv.ty = rv.ty.clone();
        }
        if rv.literal && !lv.literal {
            rv.ty = lv.ty.clone();
        }
        if lv.literal && rv.literal {
            lv = b.materialize(&lv);
            rv.ty = lv.ty.clone();
        }
        if lv.ty != rv.ty {
            return Err(lex::error(&file, line, format!("'{}' on a {} and a {}: both sides must have one type", op, lv.ty.ir(), rv.ty.ir())));
        }
        let equality = cmp && matches!(op, "==" | "!=");
        match &lv.ty {
            Ty::Num(_) => {}
            Ty::Bool | Ty::Enum(_) if equality => {}
            Ty::Enum(_) => return Err(lex::error(&file, line, format!("'{}' on an enumeration: only '==' and '!=' apply", op))),
            t => return Err(lex::error(&file, line, format!("'{}' takes numbers, not a {}", op, t.ir()))),
        }
        let ty = if cmp { Ty::Bool } else { lv.ty.clone() };
        let name = name_for(dst, &ty, b);
        b.line(&format!("{}: {} = {} {}, {}", name, ty.ir(), op_name(op), lv.text, rv.text));
        Ok(Val { text: name, ty, literal: false })
    }

    /// an operator with a sequence on a side: a map (the slice library's
    /// chunked form for a sequence on the left and a scalar on the
    /// right) or a zip
    fn seq_bin(&mut self, op: &str, lv: Val, rv: Val, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        if is_comparison(op) {
            return Err(lex::error(&file, line, "a comparison over a sequence is not in this milestone"));
        }
        let lifted = vec![lv.ty.elem().is_some(), rv.ty.elem().is_some()];
        if lifted[0] && !lifted[1] && matches!(op, "+" | "-" | "*" | "/") {
            let e = lv.ty.elem().unwrap().clone();
            let mut sv = rv;
            if sv.literal && fits_literal(&sv, &e) {
                sv.ty = e.clone();
            }
            if sv.ty != e {
                return Err(lex::error(&file, line, format!("'{}' on a sequence of {} and a {}", op, e.ir(), sv.ty.ir())));
            }
            if !matches!(e, Ty::Num(_)) {
                return Err(lex::error(&file, line, format!("'{}' takes numbers, not a {}", op, e.ir())));
            }
            let n = b.tmp();
            b.line(&format!("{}: i64 = len {}", n, lv.text));
            let c = self.new_seq(&e, &n, b, dst);
            b.line(&format!("{} {}, {}, {}", op_name(op), c.text, lv.text, sv.text));
            return Ok(c);
        }
        let op = op.to_string();
        let f = move |s: &mut Lowerer, ev: &[Val], b: &mut Body| s.emit_bin(&op, ev[0].clone(), ev[1].clone(), b, None, line);
        self.lift(vec![lv, rv], lifted, None, b, dst, line, &f)
    }

    /// `x$ + _`, `_ * x$`: a reduction by an operator; `+` is the
    /// library's `sum`
    fn reduce_bin(&mut self, op: &str, l: &Expr, r: &Expr, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let file = b.file.clone();
        let acc_left = matches!(l.kind, ExprKind::Acc);
        let seq = if acc_left { r } else { l };
        let sv = self.lower_expr(seq, None, b, None)?;
        let Some(e) = sv.ty.elem().cloned() else {
            return Err(lex::error(&file, line, "'_' goes with a sequence on the other side"));
        };
        if is_comparison(op) || !matches!(e, Ty::Num(_)) {
            return Err(lex::error(&file, line, format!("'{}' does not reduce a sequence of {}", op, e.ir())));
        }
        if op == "+" {
            let name = name_for(dst, &e, b);
            b.line(&format!("{}: {} = sum {}", name, e.ir(), sv.text));
            return Ok(Val { text: name, ty: e, literal: false });
        }
        let acc = Val { text: "_".into(), ty: e.clone(), literal: false };
        let (vals, lifted, ai) = if acc_left { (vec![acc, sv], vec![false, true], 0) } else { (vec![sv, acc], vec![true, false], 1) };
        let op = op.to_string();
        let f = move |s: &mut Lowerer, ev: &[Val], b: &mut Body| s.emit_bin(&op, ev[0].clone(), ev[1].clone(), b, None, line);
        self.lift(vals, lifted, Some((ai, e)), b, dst, line, &f)
    }

    /// an enumeration's case by its bare name, when exactly one has it
    fn enum_case(&self, word: &str) -> Option<Val> {
        let mut found = None;
        for (name, info) in &self.types {
            if let TypeInfo::Enum(cases) = info {
                if let Some(i) = cases.iter().position(|c| c == word) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(Val { text: i.to_string(), ty: Ty::Enum(name.clone()), literal: true });
                }
            }
        }
        found
    }

    /// the user's operator for a struct on the left
    fn find_operator(&self, op: &str, l: &Ty, r: &Ty) -> Option<FnInfo> {
        self.funcs
            .iter()
            .find(|f| matches!(f.parts.as_slice(), [NamePart::Group, NamePart::Sym(s), NamePart::Group] if s == op) && &f.params[0].1 == l && fits(r, &f.params[1].1))
            .cloned()
    }

    /// Lower an expression to a value. `want` is the type the context
    /// fixes, if any; `dst` a name the result should be defined under.
    fn lower_expr(&mut self, e: &Expr, want: Option<&Ty>, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        let file = b.file.clone();
        match &e.kind {
            ExprKind::Int(v) => {
                let ty = match want {
                    Some(Ty::Num(n)) => Ty::Num(n.clone()),
                    Some(Ty::Bool) => return Err(lex::error(&file, e.line, "a number where a bool is wanted")),
                    _ => Ty::Num("int".into()),
                };
                Ok(Val { text: v.to_string(), ty, literal: true })
            }
            ExprKind::Float(s) => {
                let ty = match want {
                    Some(Ty::Num(n)) => Ty::Num(n.clone()),
                    Some(Ty::Bool) => return Err(lex::error(&file, e.line, "a decimal where a bool is wanted")),
                    _ => Ty::Num("float".into()),
                };
                Ok(Val { text: s.clone(), ty, literal: true })
            }
            ExprKind::Bool(v) => Ok(Val { text: (*v as i64).to_string(), ty: Ty::Bool, literal: true }),
            ExprKind::Seq(w) => {
                let v = self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst)?;
                if v.ty.elem().is_none() {
                    return Err(lex::error(&file, e.line, format!("'{}$' is not a sequence: '{}' is a {}", w, w, v.ty.ir())));
                }
                Ok(v)
            }
            ExprKind::Acc => Err(lex::error(&file, e.line, "'_' marks the accumulator of a reduction: it goes with a sequence in an operator or a call")),
            ExprKind::List(items) => self.lower_list(items, want, b, dst, e.line),
            ExprKind::Range { from, to, inclusive } => self.lower_range(from, to, *inclusive, b, dst, e.line),
            ExprKind::Index(base, idx) => {
                let sv = self.lower_expr(base, None, b, None)?;
                let Some(elem) = sv.ty.elem().cloned() else {
                    return Err(lex::error(&file, e.line, format!("an index into a {}, which has no items", sv.ty.ir())));
                };
                let iv = self.lower_expr(idx, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(iv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, idx.line, "an index is an integer"));
                }
                let out = name_for(dst, &elem, b);
                b.line(&format!("{}: {} = load {}, {}", out, elem.ir(), sv.text, iv.text));
                Ok(Val { text: out, ty: elem, literal: false })
            }
            ExprKind::Str(s) => {
                self.nstr += 1;
                let name = format!("__s{}", self.nstr);
                self.data.push(format!("data {} = \"{}\"", name, s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")));
                let p = b.tmp();
                b.line(&format!("{}: ptr = addr {}", p, name));
                let n = b.tmp();
                b.line(&format!("{}: i64 = len {}", n, name));
                let out = name_for(dst, &Ty::string(), b);
                b.line(&format!("{}: u8[] = __str({}, {})", out, p, n));
                Ok(Val { text: out, ty: Ty::string(), literal: false })
            }
            ExprKind::Name(n) => match b.vars.get(n) {
                Some(v) if v.set => Ok(Val { text: v.ir.clone(), ty: v.ty.clone(), literal: false }),
                Some(_) => Err(lex::error(&file, e.line, format!("'{}' is read before it is assigned", n))),
                None if self.fvar(n).is_some() => Ok(self.read_fvar(n, b, dst)),
                None => Err(lex::error(&file, e.line, format!("'{}' is not a variable here", n))),
            },
            ExprKind::Field(base, field) => {
                // `Tristate.yes`: an enumeration's case, qualified
                if let ExprKind::Phrase(parts) = &base.kind {
                    if let [Part::Word(w)] = parts.as_slice() {
                        if let Some(TypeInfo::Enum(cases)) = self.types.get(w) {
                            let Some(i) = cases.iter().position(|c| c == field) else {
                                return Err(lex::error(&file, e.line, format!("{} has no case '{}'", w, field)));
                            };
                            return Ok(Val { text: i.to_string(), ty: Ty::Enum(w.clone()), literal: true });
                        }
                    }
                }
                let v = self.lower_expr(base, None, b, None)?;
                let Ty::Struct(name) = &v.ty else {
                    return Err(lex::error(&file, e.line, format!("'.{}' on a {}, which has no fields", field, v.ty.ir())));
                };
                let Some(TypeInfo::Struct(fields)) = self.types.get(name) else { unreachable!() };
                let Some((_, fty, _)) = fields.iter().find(|(n, _, _)| n == field) else {
                    return Err(lex::error(&file, e.line, format!("{} has no field '{}'", name, field)));
                };
                let fty = fty.clone();
                let out = name_for(dst, &fty, b);
                b.line(&format!("{}: {} = get {}, {}", out, fty.ir(), v.text, field));
                Ok(Val { text: out, ty: fty, literal: false })
            }
            ExprKind::Neg(x) => {
                let v = self.lower_expr(x, want, b, None)?;
                if !matches!(v.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, e.line, "'-' takes a number"));
                }
                let v = b.materialize(&v);
                let name = name_for(dst, &v.ty, b);
                b.line(&format!("{}: {} = neg {}", name, v.ty.ir(), v.text));
                Ok(Val { text: name, ty: v.ty, literal: false })
            }
            ExprKind::Bin(op, l, r) => {
                if matches!(l.kind, ExprKind::Acc) || matches!(r.kind, ExprKind::Acc) {
                    return self.reduce_bin(op, l, r, b, dst, e.line);
                }
                let cmp = is_comparison(op);
                // a wanted sequence types the items; a wanted number, the operands
                let operand_want = if cmp {
                    None
                } else {
                    match want {
                        Some(Ty::Seq(inner)) => Some(inner.as_ref()),
                        Some(t @ Ty::Num(_)) => Some(t),
                        _ => None,
                    }
                };
                let lv = self.lower_expr(l, operand_want, b, None)?;
                // a struct on the left: the program's own operator
                if let Ty::Struct(_) = &lv.ty {
                    let rv = self.lower_expr(r, None, b, None)?;
                    let Some(info) = self.find_operator(op, &lv.ty, &rv.ty) else {
                        return Err(lex::error(&file, e.line, format!("no '{}' is defined on a {} and a {}", op, lv.ty.ir(), rv.ty.ir())));
                    };
                    let ty = info.results[0].1.clone();
                    let name = name_for(dst, &ty, b);
                    b.line(&format!("{}: {} = {}({}, {})", name, ty.ir(), info.ir, lv.text, rv.text));
                    return Ok(Val { text: name, ty, literal: false });
                }
                let lt = lv.ty.clone();
                let rv_want = if lv.literal { operand_want } else { Some(lt.elem().unwrap_or(&lt)) };
                let rv = self.lower_expr(r, rv_want, b, None)?;
                if lv.ty.elem().is_some() || rv.ty.elem().is_some() {
                    return self.seq_bin(op, lv, rv, b, dst, e.line);
                }
                self.emit_bin(op, lv, rv, b, dst, e.line)
            }
            ExprKind::IfElse(c, a, d) => {
                let cv = self.lower_expr(c, Some(&Ty::Bool), b, None)?;
                if cv.ty != Ty::Bool {
                    return Err(lex::error(&file, c.line, "'if' takes a bool"));
                }
                let cv = b.materialize(&cv);
                // each arm is lowered into its own block; a literal arm
                // yields the literal, which the join's parameter types
                let start = b.out.len();
                b.depth += 1;
                let av = self.lower_expr(a, want, b, None)?;
                let a_lines = b.out.split_off(start);
                let dv = self.lower_expr(d, if av.literal { want } else { Some(&av.ty) }, b, None)?;
                let d_lines = b.out.split_off(start);
                b.depth -= 1;
                let ty = match (av.literal, dv.literal) {
                    (true, false) => dv.ty.clone(),
                    (true, true) => want.cloned().filter(|w| fits_literal(&av, w)).unwrap_or(av.ty.clone()),
                    _ => av.ty.clone(),
                };
                if !av.literal && !dv.literal && av.ty != dv.ty {
                    return Err(lex::error(&file, e.line, format!("the arms of 'if' give a {} and a {}", av.ty.ir(), dv.ty.ir())));
                }
                let name = name_for(dst, &ty, b);
                b.line(&format!("{}: {} = if {} {{", name, ty.ir(), cv.text));
                b.out.push_str(&a_lines);
                b.depth += 1;
                b.line(&format!("yield {}", av.text));
                b.depth -= 1;
                b.line("} else {");
                b.out.push_str(&d_lines);
                b.depth += 1;
                b.line(&format!("yield {}", dv.text));
                b.depth -= 1;
                b.line("}");
                Ok(Val { text: name, ty, literal: false })
            }
            ExprKind::Phrase(parts) => {
                // a lone word: a variable, or an enumeration's case
                if let [Part::Word(w)] = parts.as_slice() {
                    if b.vars.contains_key(w) || self.fvar(w).is_some() {
                        return self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst);
                    }
                    if let Some(v) = self.enum_case(w) {
                        return Ok(v);
                    }
                }
                // a type applied to arguments: a struct constructed, or a
                // number converted
                if let [Part::Word(w), Part::Args(args)] = parts.as_slice() {
                    if self.types.contains_key(w) || builtin_type(w).is_some() {
                        return match self.ty(w, false, &file, e.line)? {
                            Ty::Struct(name) => self.construct(&name, args, b, dst, e.line),
                            Ty::Enum(_) => Err(lex::error(&file, e.line, format!("{} is an enumeration: name a case", w))),
                            Ty::Seq(_) | Ty::None => Err(lex::error(&file, e.line, format!("{} cannot be constructed", w))),
                            to => {
                                let [a] = args.as_slice() else {
                                    return Err(lex::error(&file, e.line, format!("a conversion is {}(x)", w)));
                                };
                                let v = self.lower_expr(&a.value, None, b, None)?;
                                if !matches!(v.ty, Ty::Num(_) | Ty::Bool) || v.ty == Ty::Bool && to == Ty::Bool {
                                    return Err(lex::error(&file, e.line, format!("{}(x) converts a number, not a {}", w, v.ty.ir())));
                                }
                                if to == Ty::Bool {
                                    return Err(lex::error(&file, e.line, "a bool is a comparison, not a conversion"));
                                }
                                let v = b.materialize(&v);
                                let name = name_for(dst, &to, b);
                                b.line(&format!("{}: {} = conv {}", name, to.ir(), v.text));
                                Ok(Val { text: name, ty: to, literal: false })
                            }
                        };
                    }
                }
                // `s[0]` on a sequence declared without `$` (a string):
                // the parser saw a word and a one-item list
                if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::List(items), line })] = parts.as_slice() {
                    let is_seq = b.vars.get(w).map(|v| v.ty.elem().is_some()).or_else(|| self.fvar(w).map(|f| f.ty.elem().is_some()));
                    if items.len() == 1 && is_seq == Some(true) {
                        let base = Expr { kind: ExprKind::Name(w.clone()), line: *line };
                        return self.lower_expr(&Expr { kind: ExprKind::Index(Box::new(base), Box::new(items[0].clone())), line: *line }, want, b, dst);
                    }
                }
                // `count x$`: a sequence's length, as an int
                if let [Part::Word(w), rest] = parts.as_slice() {
                    let named;
                    let arg = match rest {
                        Part::Value(a) => Some(a),
                        Part::Args(a) if a.len() == 1 && a[0].name.is_none() => Some(&a[0].value),
                        Part::Word(v) if b.vars.contains_key(v) || self.fvar(v).is_some() => {
                            named = Expr { kind: ExprKind::Name(v.clone()), line: e.line };
                            Some(&named)
                        }
                        _ => None,
                    };
                    if let (true, Some(a)) = (w == "count", arg) {
                        let start = b.out.len();
                        let sv = self.lower_expr(a, None, b, None)?;
                        if sv.ty.elem().is_some() {
                            let n = b.tmp();
                            b.line(&format!("{}: i64 = len {}", n, sv.text));
                            let ty = Ty::Num("int".into());
                            let out = name_for(dst, &ty, b);
                            b.line(&format!("{}: int = conv {}", out, n));
                            return Ok(Val { text: out, ty, literal: false });
                        }
                        b.out.truncate(start);
                    }
                }
                let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
                let (info, args) = find_function(&self.funcs, parts, &is_var, &file, e.line)?;
                let info = info.clone();
                let (vals, lifted, acc, rtys) = self.lower_call_args(&info, &args, b)?;
                if lifted.iter().any(|&l| l) || acc.is_some() {
                    if rtys.len() != 1 {
                        return Err(lex::error(&file, e.line, format!("'{}' over a sequence: the function gives one result", info.key)));
                    }
                    if acc.is_some() && !lifted.iter().any(|&l| l) {
                        return Err(lex::error(&file, e.line, "'_' goes with a sequence among the arguments"));
                    }
                    let ir = info.ir.clone();
                    let rty = rtys[0].clone();
                    let f = move |_: &mut Lowerer, ev: &[Val], b: &mut Body| -> Result<Val, Error> {
                        let ops: Vec<String> = ev.iter().map(|v| v.text.clone()).collect();
                        let name = b.tmp();
                        b.line(&format!("{}: {} = {}({})", name, rty.ir(), ir, ops.join(", ")));
                        Ok(Val { text: name, ty: rty.clone(), literal: false })
                    };
                    return self.lift(vals, lifted, acc, b, dst, e.line, &f);
                }
                let ops: Vec<String> = vals.into_iter().map(|v| v.text).collect();
                let call = format!("{}({})", info.ir, ops.join(", "));
                match rtys.len() {
                    0 => {
                        b.line(&call);
                        Ok(Val { text: String::new(), ty: Ty::None, literal: false })
                    }
                    1 => {
                        let ty = rtys[0].clone();
                        let name = name_for(dst, &ty, b);
                        b.line(&format!("{}: {} = {}", name, ty.ir(), call));
                        Ok(Val { text: name, ty, literal: false })
                    }
                    _ => Err(lex::error(&file, e.line, format!("'{}' gives several results; take them with `a, b = ...`", info.key))),
                }
            }
            ExprKind::Existing(_) => Err(lex::error(&file, e.line, "'existing' is not in this item yet")),
            _ => Err(lex::error(&file, e.line, "this expression is not in this item yet")),
        }
    }
}

/// the name a result is defined under: the destination variable's next
/// version when the value's type is the variable's, a temporary
/// otherwise (the assignment then reports the mismatch)
fn name_for(dst: Option<&str>, ty: &Ty, b: &mut Body) -> String {
    match dst {
        Some(d) if b.vars.get(d).map(|v| &v.ty) == Some(ty) => b.define(d, ty.clone()),
        _ => b.tmp(),
    }
}

/// does a block end by leaving: `break`, `continue`, an `if` whose two
/// arms both do, or a `loop` with no `while` and no `break`? The IR's
/// rule (ssa.md, *Termination rules*), checked on the zero tree
fn terminates(stmts: &[Stmt]) -> bool {
    match stmts.last() {
        Some(Stmt::Break { .. }) | Some(Stmt::Continue { .. }) => true,
        Some(Stmt::If { then, els: Some(e), .. }) => terminates(then) && terminates(e),
        Some(Stmt::Loop { cond: None, body, .. }) => !has_break(body),
        _ => false,
    }
}

/// a `break` of this loop, in the body or in an `if` inside it
fn has_break(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break { .. } => true,
        Stmt::If { then, els, .. } => has_break(then) || els.as_ref().map_or(false, |e| has_break(e)),
        _ => false,
    })
}

fn stmt_line(s: &Stmt) -> usize {
    match s {
        Stmt::Var(v) => v.line,
        Stmt::Multi { line, .. } | Stmt::Assign { line, .. } | Stmt::If { line, .. } | Stmt::Loop { line, .. } | Stmt::For { line, .. } => *line,
        Stmt::Continue { line, .. } | Stmt::Break { line } | Stmt::Check { line, .. } | Stmt::Push { line, .. } | Stmt::Expr { line, .. } => *line,
    }
}

/// may a literal be assigned to a variable of this type?
fn fits_literal(v: &Val, ty: &Ty) -> bool {
    match (&v.ty, ty) {
        (Ty::Num(_), Ty::Num(_)) => true,
        (a, b) => a == b,
    }
}
