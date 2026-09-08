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
    /// a string: a view of bytes, `u8[]`
    Str,
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
            Ty::Str => "u8[]".into(),
            Ty::Struct(n) | Ty::Enum(n) => n.clone(),
            Ty::None => String::new(),
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
        "string" => return Some(Ty::Str),
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
/// two functions the runner reads it back with, `__print` itself,
/// `__str` (a string literal's view) and `__zero_reset`, which the
/// runner calls before each case
const PRELUDE: &str = r#"
; the print buffer: what `print` wrote, read back by the runner
data __out: array(u8, 4096)
data __out_n: array(i64, 1)
data __nul: array(u8, 1)

fn __zero_reset() {
    q: ptr = addr __out_n
    store 0: i64, q
    ret
}

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

fn __empty_str() -> u8[] {
    p: ptr = addr __nul
    v: u8[] = __str(p, 0)
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
    let mut l = Lowerer { funcs: Vec::new(), types: HashMap::new(), type_lines: Vec::new(), data: Vec::new(), out: String::new(), nstr: 0 };
    // the front end's builtins, until item 11 declares them as platform functions
    l.funcs.push(FnInfo {
        key: "print".into(),
        ir: "__print".into(),
        parts: vec![NamePart::Word("print".into()), NamePart::Group],
        params: vec![("s".into(), Ty::Str)],
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
                Decl::Var(v) => return Err(lex::error(&f.code.file, v.line, "feature-scope variables are not in this item yet")),
            }
        }
    }
    for f in &store.features {
        writeln!(l.out, "\n; feature {}", f.name).unwrap();
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                l.lower_fn(fd, &f.code.file)?;
            }
        }
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
    let (info, args) = find_function(&lowered.funcs, parts, &HashMap::new(), file, case.line)?;
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

/// the function a phrase names, and its arguments in order: a word that
/// is a variable in scope is an argument, every other word is part of
/// the name, and the bracket groups and bare values are arguments
fn find_function<'a>(funcs: &'a [FnInfo], parts: &[Part], vars: &HashMap<String, Var>, file: &str, line: usize) -> Result<(&'a FnInfo, Vec<Expr>), Error> {
    let mut name = Vec::new();
    let mut args = Vec::new();
    for p in parts {
        match p {
            Part::Word(w) if vars.contains_key(w) => {
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
    let key = mangle(&name);
    let found: Vec<&FnInfo> = funcs.iter().filter(|f| f.key == key && !f.parts.iter().any(|p| matches!(p, NamePart::Sym(_)))).collect();
    let Some(info) = found.into_iter().last() else {
        let words: Vec<String> = name.iter().filter_map(|p| if let NamePart::Word(w) = p { Some(w.clone()) } else { None }).collect();
        return Err(lex::error(file, line, format!("no function named '{}'", words.join(" "))));
    };
    if info.params.len() != args.len() {
        return Err(lex::error(file, line, format!("'{}' takes {} argument(s), given {}", key, info.params.len(), args.len())));
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
}

/// a variable in a function's scope: its current IR value and type
#[derive(Clone, Debug)]
struct Var {
    ir: String,
    ty: Ty,
    /// how many times it has been defined: the next version's suffix
    versions: usize,
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
    vars: HashMap<String, Var>,
    results: Vec<(String, Ty)>,
    file: String,
    /// how deep in structured blocks the next line is
    depth: usize,
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

    /// the IR name for a new definition of `name`
    fn define(&mut self, name: &str, ty: Ty) -> String {
        let v = self.vars.entry(name.to_string()).or_insert(Var { ir: name.to_string(), ty: ty.clone(), versions: 0 });
        v.versions += 1;
        v.ty = ty;
        v.ir = if v.versions == 1 { name.to_string() } else { format!("{}_{}", name, v.versions) };
        v.ir.clone()
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
        if seq {
            return Err(lex::error(file, line, "sequences are not in this item yet"));
        }
        if let Some(t) = builtin_type(name) {
            return Ok(t);
        }
        match self.types.get(name) {
            Some(TypeInfo::Struct(_)) => Ok(Ty::Struct(name.to_string())),
            Some(TypeInfo::Enum(_)) => Ok(Ty::Enum(name.to_string())),
            None => Err(lex::error(file, line, format!("'{}' is not a type", name))),
        }
    }

    fn declare_type(&mut self, t: &super::syntax::TypeDecl, file: &str) -> Result<(), Error> {
        if self.types.contains_key(&t.name) || builtin_type(&t.name).is_some() {
            return Err(lex::error(file, t.line, format!("type '{}' is already declared", t.name)));
        }
        match &t.kind {
            TypeKind::Enum(cases) => {
                let mut bits = 1;
                while (1u64 << bits) < cases.len() as u64 {
                    bits += 1;
                }
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

    fn lower_fn(&mut self, f: &FnDecl, file: &str) -> Result<(), Error> {
        let key = mangle(&f.name);
        let info = self.funcs.iter().find(|g| g.key == key && g.parts == f.name && g.params.len() == f.params().count()).unwrap().clone();
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), results: info.results.clone(), file: file.to_string(), depth: 0 };
        let mut sig = format!("fn {}(", info.ir);
        for (i, (n, t)) in info.params.iter().enumerate() {
            if i > 0 {
                sig.push_str(", ");
            }
            write!(sig, "{}: {}", n, t.ir()).unwrap();
            if b.vars.contains_key(n) {
                return Err(lex::error(file, f.line, format!("parameter '{}' is named twice", n)));
            }
            b.vars.insert(n.clone(), Var { ir: n.clone(), ty: t.clone(), versions: 1 });
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
            b.vars.insert(n.clone(), Var { ir: String::new(), ty: t.clone(), versions: 0 });
        }
        writeln!(self.out, "{} {{", sig).unwrap();
        self.lower_block(&f.body, &mut b)?;
        // the results' current values; one never assigned is its type's zero
        let mut rets = Vec::new();
        for (n, t) in &b.results.clone() {
            let v = b.vars[n].clone();
            if v.versions == 0 {
                rets.push(self.zero_val(t, &mut b).text);
            } else {
                rets.push(v.ir);
            }
        }
        b.line(format!("ret {}", rets.join(", ")).trim_end());
        self.out.push_str(&b.out);
        self.out.push_str("}\n");
        Ok(())
    }

    fn lower_block(&mut self, stmts: &[Stmt], b: &mut Body) -> Result<(), Error> {
        for s in stmts {
            self.lower_stmt(s, b)?;
        }
        Ok(())
    }

    /// a type's zero: a literal for a number, a bool or an enumeration,
    /// the empty string, a struct of its defaults
    fn zero_val(&mut self, t: &Ty, b: &mut Body) -> Val {
        match t {
            Ty::Bool | Ty::Num(_) | Ty::Enum(_) => Val { text: "0".into(), ty: t.clone(), literal: true },
            Ty::Str => {
                let n = b.tmp();
                b.line(&format!("{}: u8[] = __empty_str()", n));
                Val { text: n, ty: Ty::Str, literal: false }
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
            b.define(name, var.ty.clone());
            b.vars.get_mut(name).unwrap().ir = v.text;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, s: &Stmt, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        match s {
            Stmt::Assign { targets, value, line } => {
                for t in targets {
                    if !b.vars.contains_key(&t.name) {
                        return Err(lex::error(&file, t.line, format!("'{}' is not declared: a variable is its type then its name", t.name)));
                    }
                }
                if targets.len() == 1 {
                    let t = &targets[0];
                    let ty = b.vars[&t.name].ty.clone();
                    let v = self.lower_expr(value, Some(&ty), b, Some(&t.name))?;
                    return self.assign(&t.name, v, b, *line);
                }
                let names: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();
                let tys: Vec<Ty> = names.iter().map(|n| b.vars[n].ty.clone()).collect();
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
                    b.vars.insert(p.name.clone(), Var { ir: String::new(), ty: ty.clone(), versions: 0 });
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
                b.vars.insert(v.name.clone(), Var { ir: String::new(), ty: ty.clone(), versions: 0 });
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
            Stmt::If { line, .. } | Stmt::Loop { line, .. } | Stmt::For { line, .. } | Stmt::Continue { line, .. } | Stmt::Break { line } => {
                Err(lex::error(&file, *line, "control flow is not in this item yet"))
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
        let (info, args) = find_function(&self.funcs, parts, &b.vars, &file, line)?;
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
        let defs: Vec<String> = names.iter().zip(tys).map(|(n, t)| format!("{}: {}", b.define(n, t.clone()), t.ir())).collect();
        b.line(&format!("{} = {}({})", defs.join(", "), info.ir, ops.join(", ")));
        Ok(())
    }

    /// a call's arguments lowered against its parameters, and its result
    /// types with the tower's abstract names bound by the arguments
    fn lower_args(&mut self, info: &FnInfo, args: &[Expr], b: &mut Body) -> Result<(Vec<String>, Vec<Ty>), Error> {
        let file = b.file.clone();
        let mut ops = Vec::new();
        let mut bound: HashMap<String, Ty> = HashMap::new();
        for (a, (_, ty)) in args.iter().zip(&info.params) {
            let mut v = self.lower_expr(a, Some(ty), b, None)?;
            if v.literal && fits_literal(&v, ty) {
                v.ty = ty.clone();
            }
            if !fits(&v.ty, ty) {
                return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given a {}", info.key, ty.ir(), v.ty.ir())));
            }
            if let Some(name) = ty.abstract_name() {
                if &v.ty != ty {
                    bound.entry(name.to_string()).or_insert(v.ty.clone());
                }
            }
            ops.push(v.text);
        }
        let rtys = info
            .results
            .iter()
            .map(|(_, t)| match t.abstract_name().and_then(|n| bound.get(n)) {
                Some(bt) => bt.clone(),
                None => t.clone(),
            })
            .collect();
        Ok((ops, rtys))
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
            ExprKind::Str(s) => {
                self.nstr += 1;
                let name = format!("__s{}", self.nstr);
                self.data.push(format!("data {} = \"{}\"", name, s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")));
                let p = b.tmp();
                b.line(&format!("{}: ptr = addr {}", p, name));
                let n = b.tmp();
                b.line(&format!("{}: i64 = len {}", n, name));
                let out = name_for(dst, &Ty::Str, b);
                b.line(&format!("{}: u8[] = __str({}, {})", out, p, n));
                Ok(Val { text: out, ty: Ty::Str, literal: false })
            }
            ExprKind::Name(n) => match b.vars.get(n) {
                Some(v) if v.versions > 0 => Ok(Val { text: v.ir.clone(), ty: v.ty.clone(), literal: false }),
                Some(_) => Err(lex::error(&file, e.line, format!("'{}' is read before it is assigned", n))),
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
                let cmp = is_comparison(op);
                let operand_want = if cmp { None } else { want.filter(|t| matches!(t, Ty::Num(_))) };
                let mut lv = self.lower_expr(l, operand_want, b, None)?;
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
                let mut rv = self.lower_expr(r, if lv.literal { operand_want } else { Some(&lv.ty) }, b, None)?;
                // a literal takes the other side's type; two literals
                // make one of them a value first
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
                    return Err(lex::error(&file, e.line, format!("'{}' on a {} and a {}: both sides must have one type", op, lv.ty.ir(), rv.ty.ir())));
                }
                let equality = cmp && matches!(op.as_str(), "==" | "!=");
                match &lv.ty {
                    Ty::Num(_) => {}
                    Ty::Bool | Ty::Enum(_) if equality => {}
                    Ty::Enum(_) => return Err(lex::error(&file, e.line, format!("'{}' on an enumeration: only '==' and '!=' apply", op))),
                    t => return Err(lex::error(&file, e.line, format!("'{}' takes numbers, not a {}", op, t.ir()))),
                }
                let ty = if cmp { Ty::Bool } else { lv.ty.clone() };
                let name = name_for(dst, &ty, b);
                b.line(&format!("{}: {} = {} {}, {}", name, ty.ir(), op_name(op), lv.text, rv.text));
                Ok(Val { text: name, ty, literal: false })
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
                    if b.vars.contains_key(w) {
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
                            Ty::Str | Ty::None => Err(lex::error(&file, e.line, format!("{} cannot be constructed", w))),
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
                let (info, args) = find_function(&self.funcs, parts, &b.vars, &file, e.line)?;
                let info = info.clone();
                let (ops, rtys) = self.lower_args(&info, &args, b)?;
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

/// may a literal be assigned to a variable of this type?
fn fits_literal(v: &Val, ty: &Ty) -> bool {
    match (&v.ty, ty) {
        (Ty::Num(_), Ty::Num(_)) => true,
        (a, b) => a == b,
    }
}
