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

use super::lex::{self, Error};
use super::store::{Case, Expect, Store};
use super::syntax::{Decl, Expr, ExprKind, FnDecl, NamePart, Part, Stmt};
use std::collections::HashMap;
use std::fmt::Write;

/// a zero type, as the lowering sees it
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Bool,
    /// a number, by its IR name: `int`, `u8`, `f32`, `number`, ...
    Num(String),
    /// a string: bytes in `data`, reached as (address, length) for now
    Str,
    /// no value: a function with no results
    None,
}

impl Ty {
    fn ir(&self) -> String {
        match self {
            Ty::Bool => "u1".into(),
            Ty::Num(n) => n.clone(),
            Ty::Str => "ptr".into(),
            Ty::None => String::new(),
        }
    }

    /// zero's spelling of a type to the IR's (section 4)
    fn from_name(name: &str, seq: bool, file: &str, line: usize) -> Result<Ty, Error> {
        if seq {
            return Err(lex::error(file, line, "sequences are not in this item yet"));
        }
        let ir = match name {
            "bool" => return Ok(Ty::Bool),
            "string" => return Ok(Ty::Str),
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
            _ => return Err(lex::error(file, line, format!("'{}' is not a type this item knows", name))),
        };
        Ok(Ty::Num(ir))
    }
}

/// a function the store defines, as the lowering knows it
#[derive(Clone, Debug)]
pub struct FnInfo {
    /// the mangled name, which is also the IR function's name
    pub name: String,
    pub parts: Vec<NamePart>,
    pub params: Vec<(String, Ty)>,
    pub results: Vec<(String, Ty)>,
    pub feature: String,
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
/// two functions the runner reads it back with, `__print` itself, and
/// `__zero_reset`, which the runner calls before each case
const PRELUDE: &str = r#"
; the print buffer: what `print` wrote, read back by the runner
data __out: array(u8, 4096)
data __out_n: array(i64, 1)

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

; append n bytes at s and a newline; what does not fit is dropped
fn __print(s: ptr, n: i64) {
    q: ptr = addr __out_n
    k0: i64 = load q
    o: ptr = addr __out
    k1: i64 = loop(i: i64 = 0, k: i64 = k0) {
        done: u1 = cmp.ge i, n
        if done {
            break k
        }
        c: u8 = load s, i, 1
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
            NamePart::Sym(s) => Some(symbol_name(s).to_string()),
            NamePart::Group => None,
        })
        .collect();
    words.join("_")
}

/// an operator's name in the IR (section 6: `+` on a type is `add`)
fn symbol_name(s: &str) -> &'static str {
    match s {
        "+" => "add",
        "-" => "sub",
        "*" => "mul",
        "/" => "div",
        "<" => "lt",
        ">" => "gt",
        "<=" => "le",
        ">=" => "ge",
        "==" => "eq",
        "!=" => "ne",
        _ => "op",
    }
}

pub fn lower(store: &Store) -> Result<Lowered, Error> {
    let mut l = Lowerer { funcs: Vec::new(), data: Vec::new(), out: String::new(), nstr: 0 };
    // every signature first, so a body may call what a later feature defines
    for f in &store.features {
        for d in &f.code.decls {
            match d {
                Decl::Fn(fd) => l.declare(fd, &f.name, &f.code.file)?,
                Decl::Type(t) => return Err(lex::error(&f.code.file, t.line, "types are not in this item yet")),
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
            (ExprKind::Int(v), Ty::Num(_)) => *v,
            (ExprKind::Bool(b), Ty::Bool) => *b as i64,
            _ => return Err(lex::error(file, case.line, "a case's arguments are numbers")),
        };
        vals.push(v);
    }
    Ok(Call { func: info.name.clone(), args: vals, nrets: info.results.len(), expect: case.expect.clone() })
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
    let found: Vec<&FnInfo> = funcs.iter().filter(|f| f.name == key).collect();
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
}

impl Body {
    fn tmp(&mut self) -> String {
        self.ntmp += 1;
        format!("_{}", self.ntmp)
    }

    fn line(&mut self, s: &str) {
        self.out.push_str("    ");
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
}

impl Lowerer {
    fn declare(&mut self, f: &FnDecl, feature: &str, file: &str) -> Result<(), Error> {
        if f.task {
            return Err(lex::error(file, f.line, "tasks are not in this item yet"));
        }
        if !f.platform.is_empty() {
            return Err(lex::error(file, f.line, "platform bodies are not in this item yet"));
        }
        let name = mangle(&f.name);
        let mut params = Vec::new();
        for p in f.params() {
            params.push((p.name.clone(), Ty::from_name(&p.ty, p.seq, file, p.line)?));
        }
        let mut results = Vec::new();
        for r in &f.results {
            results.push((r.name.clone(), Ty::from_name(&r.ty, r.seq, file, r.line)?));
        }
        if let Some(other) = self.funcs.iter().find(|g| g.name == name) {
            if other.parts != f.name || other.params.len() != params.len() {
                return Err(lex::error(file, f.line, format!("'{}' clashes with a function of feature {} that mangles to the same name", name, other.feature)));
            }
            return Err(lex::error(file, f.line, "redefinition is not in this item yet"));
        }
        self.funcs.push(FnInfo { name, parts: f.name.clone(), params, results, feature: feature.to_string() });
        Ok(())
    }

    fn lower_fn(&mut self, f: &FnDecl, file: &str) -> Result<(), Error> {
        let name = mangle(&f.name);
        let info = self.funcs.iter().find(|g| g.name == name).unwrap().clone();
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), results: info.results.clone(), file: file.to_string() };
        let mut sig = format!("fn {}(", info.name);
        for (i, (n, t)) in info.params.iter().enumerate() {
            if i > 0 {
                sig.push_str(", ");
            }
            write!(sig, "{}: {}", n, t.ir()).unwrap();
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
                return Err(lex::error(file, f.line, format!("result '{}' is also a parameter", n)));
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
                rets.push(zero_of(t));
            } else {
                rets.push(v.ir);
            }
        }
        b.line(&format!("ret {}", rets.join(", ")).trim_end());
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

    fn lower_stmt(&mut self, s: &Stmt, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        match s {
            Stmt::Assign { targets, value, line } => {
                if targets.len() != 1 {
                    return Err(lex::error(&file, *line, "several results at once are not in this item yet"));
                }
                let t = &targets[0];
                let Some(var) = b.vars.get(&t.name).cloned() else {
                    return Err(lex::error(&file, *line, format!("'{}' is not declared: a variable is its type then its name", t.name)));
                };
                let ty = var.ty.clone();
                let v = self.lower_expr(value, Some(&ty), b, Some(&t.name))?;
                if v.ty != ty {
                    return Err(lex::error(&file, *line, format!("'{}' is {} but the value is {}", t.name, ty.ir(), v.ty.ir())));
                }
                // a value that already has a name is not copied: the
                // variable simply names it too
                if b.vars[&t.name].ir != v.text {
                    let ir = b.define(&t.name, ty.clone());
                    if v.literal {
                        b.line(&format!("{}: {} = const {}", ir, ty.ir(), v.text));
                    } else {
                        // the value was defined under a temporary; the
                        // variable takes that name
                        b.vars.get_mut(&t.name).unwrap().ir = v.text;
                    }
                }
                Ok(())
            }
            Stmt::Expr { expr, line } => {
                match &expr.kind {
                    ExprKind::Phrase(_) | ExprKind::Existing(_) => {}
                    _ => return Err(lex::error(&file, *line, "a statement is a call, an assignment or a declaration")),
                }
                self.lower_expr(expr, None, b, None)?;
                Ok(())
            }
            Stmt::Var(v) => Err(lex::error(&file, v.line, "variables inside functions are not in this item yet")),
            Stmt::If { line, .. } | Stmt::Loop { line, .. } | Stmt::For { line, .. } | Stmt::Continue { line, .. } | Stmt::Break { line } => {
                Err(lex::error(&file, *line, "control flow is not in this item yet"))
            }
            Stmt::Check { line, .. } => Err(lex::error(&file, *line, "checks are not in this item yet")),
            Stmt::Push { line, .. } => Err(lex::error(&file, *line, "streams are not in this item yet")),
        }
    }

    /// Lower an expression to a value. `want` is the type the context
    /// fixes, if any; `dst` a name the result should be defined under.
    fn lower_expr(&mut self, e: &Expr, want: Option<&Ty>, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        let file = b.file.clone();
        match &e.kind {
            ExprKind::Int(v) => {
                let ty = match want {
                    Some(Ty::Num(n)) => Ty::Num(n.clone()),
                    Some(t) => return Err(lex::error(&file, e.line, format!("a number where a {} is wanted", t.ir()))),
                    None => Ty::Num("int".into()),
                };
                Ok(Val { text: v.to_string(), ty, literal: true })
            }
            ExprKind::Float(s) => {
                let ty = match want {
                    Some(Ty::Num(n)) => Ty::Num(n.clone()),
                    Some(t) => return Err(lex::error(&file, e.line, format!("a decimal where a {} is wanted", t.ir()))),
                    None => Ty::Num("float".into()),
                };
                Ok(Val { text: s.clone(), ty, literal: true })
            }
            ExprKind::Bool(v) => Ok(Val { text: (*v as i64).to_string(), ty: Ty::Bool, literal: true }),
            ExprKind::Str(_) => Err(lex::error(&file, e.line, "a string is only printed in this item")),
            ExprKind::Name(n) => match b.vars.get(n) {
                Some(v) if v.versions > 0 => Ok(Val { text: v.ir.clone(), ty: v.ty.clone(), literal: false }),
                Some(_) => Err(lex::error(&file, e.line, format!("'{}' is read before it is assigned", n))),
                None => Err(lex::error(&file, e.line, format!("'{}' is not a variable here", n))),
            },
            ExprKind::Phrase(parts) => {
                // a lone word is a variable when there is one
                if let [Part::Word(w)] = parts.as_slice() {
                    if b.vars.contains_key(w) {
                        return self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst);
                    }
                }
                if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Str(s), .. })] = parts.as_slice() {
                    if w == "print" {
                        self.nstr += 1;
                        let name = format!("__s{}", self.nstr);
                        self.data.push(format!("data {} = \"{}\"", name, s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")));
                        let p = b.tmp();
                        b.line(&format!("{}: ptr = addr {}", p, name));
                        let n = b.tmp();
                        b.line(&format!("{}: i64 = len {}", n, name));
                        b.line(&format!("__print({}, {})", p, n));
                        return Ok(Val { text: String::new(), ty: Ty::None, literal: false });
                    }
                }
                let (info, args) = find_function(&self.funcs, parts, &b.vars, &file, e.line)?;
                let info = info.clone();
                let mut ops = Vec::new();
                for (a, (_, ty)) in args.iter().zip(&info.params) {
                    let v = self.lower_expr(a, Some(ty), b, None)?;
                    if &v.ty != ty {
                        return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given a {}", info.name, ty.ir(), v.ty.ir())));
                    }
                    ops.push(v.text);
                }
                let call = format!("{}({})", info.name, ops.join(", "));
                match info.results.len() {
                    0 => {
                        b.line(&call);
                        Ok(Val { text: String::new(), ty: Ty::None, literal: false })
                    }
                    1 => {
                        let ty = info.results[0].1.clone();
                        let name = match dst {
                            Some(d) => b.define(d, ty.clone()),
                            None => b.tmp(),
                        };
                        b.line(&format!("{}: {} = {}", name, ty.ir(), call));
                        Ok(Val { text: name, ty, literal: false })
                    }
                    _ => Err(lex::error(&file, e.line, format!("'{}' gives several results; take them with `a, b = ...`", info.name))),
                }
            }
            ExprKind::Existing(_) => Err(lex::error(&file, e.line, "'existing' is not in this item yet")),
            _ => Err(lex::error(&file, e.line, "this expression is not in this item yet")),
        }
    }
}

/// a type's zero, as a literal the IR takes for a result
fn zero_of(t: &Ty) -> String {
    match t {
        Ty::Bool | Ty::Num(_) | Ty::Str => "0".into(),
        Ty::None => String::new(),
    }
}
