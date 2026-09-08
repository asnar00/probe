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
    /// a stream `T$` (log 23): the IR's `T$`, a reader's view of a ring,
    /// regular (at a rate) or not; a stream of a struct is a generated
    /// struct of one stream per field
    Stream(Box<Ty>, bool),
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
            Ty::Stream(e, _) => match e.as_ref() {
                Ty::Struct(n) => format!("__s_{}", n),
                e => format!("{}$", e.ir()),
            },
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
    /// a task (log 25): declared with `<<`, its result a stream it
    /// produces and its `$` parameters streams it reads; in the IR its
    /// output's reader comes first and `__hz` last, and it returns its
    /// stream parameters moved on
    pub task: bool,
    /// the features that define it, innermost first (log 28): one for a
    /// function defined once; more for a chain of redefinitions, whose
    /// bodies are `key__feature` and whose links `key` and
    /// `key__before_feature` gate on each feature's `enabled`
    pub chain: Vec<String>,
}

/// a wiring at feature scope (log 25): one task call the scheduler runs
/// into a feature-scope stream
struct Node {
    info: FnInfo,
    /// the output stream variable
    out: String,
    /// the arguments in the parameters' order: a stream parameter's is
    /// the name of a feature-scope stream
    args: Vec<Expr>,
    hz: i64,
    feature: String,
    file: String,
    /// the wiring as written, for the IR's comment
    text: String,
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

; the store's virtual clock: an exact time, zero at every case, moved
; by a rated task sleeping between its pushes and by nothing else; a
; push into a stream without a rate is stamped with it in microseconds
data __clock: array(time, 1)
; the scheduler is running: a push from inside a task does not start it again
data __running: array(i64, 1)

fn __now() -> i64 {
    p: ptr = addr __clock
    c: time = load p
    h: time = seconds(1000000)
    x: time = mul c, h
    k: i64 = conv x
    ret k
}

; a task wired at a rate, after each push: the clock moves on one period
fn __sleep(hz: i64) {
    rated: u1 = cmp.gt hz, 0
    if rated {
        p: ptr = addr __clock
        c: time = load p
        d: time = period(hz)
        c2: time = add c, d
        store c2, p
    }
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

; one byte, when there is room
fn __out_ch(c: u8) {
    q: ptr = addr __out_n
    k: i64 = load q
    o: ptr = addr __out
    room: u1 = cmp.lt k, 4095
    if room {
        store c, o, k, 1
        k2: i64 = add k, 1
        store k2, q
    }
    ret
}

; an int in decimal
fn __print_int(x: int) {
    negative: u1 = cmp.lt x, 0
    m: int = if negative {
        __out_ch(45)
        y: int = sub 0, x
        yield y
    } else {
        yield x
    }
    top: int = loop(p: int = 1) {
        q: int = div m, p
        more: u1 = cmp.ge q, 10
        if more {
            p2: int = mul p, 10
            continue p2
        }
        break p
    }
    loop(p3: int = top) {
        done: u1 = cmp.eq p3, 0
        if done {
            break
        }
        d: int = div m, p3
        r: int = rem d, 10
        a: int = add r, 48
        c: u8 = conv a
        __out_ch(c)
        p4: int = div p3, 10
        continue p4
    }
    ret
}

; a sequence of ints on one line, a space between, then a newline
fn __print_ints(v: int[]) {
    n: i64 = len v
    loop(i: i64 = 0) {
        done: u1 = cmp.ge i, n
        if done {
            break
        }
        first: u1 = cmp.eq i, 0
        if first {
        } else {
            __out_ch(32)
        }
        x: int = load v, i
        __print_int(x)
        i2: i64 = add i, 1
        continue i2
    }
    __out_ch(10)
    ret
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

/// a ring's capacity: the front end's number until residency is
/// computed (section 9, log 23); a reader more than half this behind
/// fails the library's check
const RING_ITEMS: usize = 64;

/// the clock of a stream without a rate: microsecond ticks
const CLOCK_HZ: i64 = 1_000_000;

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
    let mut l = Lowerer { funcs: Vec::new(), types: HashMap::new(), type_lines: Vec::new(), data: Vec::new(), out: String::new(), nstr: 0, fvars: Vec::new(), news: std::collections::BTreeSet::new(), rings: std::collections::BTreeSet::new(), sstructs: std::collections::BTreeSet::new(), push_read: None, nodes: Vec::new(), node_inputs: std::collections::HashSet::new(), cur: String::new(), ranks: HashMap::new(), features: Vec::new(), type_feature: HashMap::new() };
    for f in &store.features {
        l.features.push(f.name.clone());
        l.ranks.insert(f.name.clone(), store.rank(f.layer.as_deref().unwrap_or("")));
    }
    // the front end's builtins, until item 11 declares them as platform functions
    l.funcs.push(FnInfo {
        key: "print".into(),
        ir: "__print".into(),
        parts: vec![NamePart::Word("print".into()), NamePart::Group],
        params: vec![("s".into(), Ty::string())],
        results: Vec::new(),
        feature: String::new(),
        task: false,
        chain: Vec::new(),
    });
    // types first, then every signature, then the variables (a wiring
    // names a task), so a body may use what a later feature declares
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Type(t) = d {
                l.declare_type(t, &f.code.file)?;
            }
        }
    }
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                l.declare(fd, &f.name, &f.code.file)?;
            }
        }
    }
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Var(v) = d {
                l.declare_var(v, &f.name, &f.code.file)?;
            }
        }
    }
    for f in &store.features {
        l.cur = f.name.clone();
        for d in &f.code.decls {
            if let Decl::Var(v) = d {
                l.collect_nodes(v, &f.name, &f.code.file)?;
            }
        }
    }
    l.emit_context(store)?;
    for f in &store.features {
        l.cur = f.name.clone();
        writeln!(l.out, "\n; feature {} (layer {})", f.name, f.layer.as_deref().unwrap_or("")).unwrap();
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                l.lower_fn(fd, &f.name, &f.code.file)?;
            }
        }
    }
    l.emit_links();
    if !l.news.is_empty() {
        writeln!(l.out, "\n; a sequence of n items, carved from the arena: a buffer, then the view over it").unwrap();
    }
    for t in &l.news {
        writeln!(l.out, "fn __new_{}(n: i64) -> {}[] {{\n    a: ptr = addr __arena\n    sz: i64 = sizeof {}\n    bytes: i64 = mul n, sz\n    total: i64 = add bytes, 16\n    p: ptr = arena_alloc(a, total)\n    buffer_init(p, sz, n)\n    v: {}[] = slice p\n    ret v\n}}", t, t, t, t).unwrap();
    }
    if !l.rings.is_empty() {
        writeln!(l.out, "\n; a stream's ring, carved from the arena: {} items, and a tick per item unless regular; a reader's view at its start", RING_ITEMS).unwrap();
    }
    for (t, regular) in &l.rings {
        let ticks = if *regular { String::new() } else { format!("    tb: ptr = arena_alloc(a, {})\n    buffer_init(tb, 8, {})\n", RING_ITEMS * 8 + 16, RING_ITEMS) };
        let init = if *regular { "ring_regular(r, vb, hz, 1, 0)".to_string() } else { "ring_init(r, vb, tb, hz)".to_string() };
        let name = if *regular { "regular" } else { "stream" };
        writeln!(l.out, "fn __{}_{}(hz: i64) -> {}$ {{\n    a: ptr = addr __arena\n    r: ptr = arena_alloc(a, 64)\n    sz: i64 = sizeof {}\n    bytes: i64 = mul sz, {}\n    total: i64 = add bytes, 16\n    vb: ptr = arena_alloc(a, total)\n    buffer_init(vb, sz, {})\n{}    {}\n    s: {}$ = stream r\n    ret s\n}}", name, t, t, t, RING_ITEMS, RING_ITEMS, ticks, init, t).unwrap();
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
    if info.task {
        return Err(lex::error(file, case.line, format!("'{}' is a task: a case calls a function that reads its stream", info.key)));
    }
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
    /// the rings made: element type and whether regular, one
    /// `__stream_T` or `__regular_T` each
    rings: std::collections::BTreeSet<(String, bool)>,
    /// the structs a stream was made of: `type __s_T` declared once
    sstructs: std::collections::BTreeSet<String>,
    /// inside a push chain: what the stream's own name reads as
    push_read: Option<(String, PushRead)>,
    /// the wirings at feature scope, in declaration order
    nodes: Vec<Node>,
    /// the feature-scope streams some node reads: a push into one from a
    /// plain function is followed by `__run()`
    node_inputs: std::collections::HashSet<String>,
    /// the feature whose code is being lowered
    cur: String,
    /// every feature's layer height (log 28)
    ranks: HashMap<String, usize>,
    /// the features, in composition order
    features: Vec<String>,
    /// the feature that declared each type
    type_feature: HashMap<String, String>,
}

/// what kind of body is being lowered: the scheduler runs after a push
/// in a plain function only, and a task sleeps after a push into its
/// own output (log 25)
#[derive(Clone, PartialEq)]
enum BodyKind {
    Fn,
    /// the output stream's name, and the IR name of the rate parameter
    Task { out: String, hz: String },
    Reset,
    Node,
}

/// on the right of `<<` a stream's name is its latest item; in the
/// chain's `while` it is the candidate about to be pushed (log 23)
#[derive(Clone)]
enum PushRead {
    Latest(Val, Ty),
    Value(Val),
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
    /// the carried variables, in the header's order, then the streams
    /// the body moves (log 23); empty for a `for`
    carried: Vec<String>,
    /// how many of them the header declares: `continue (...)` gives
    /// those, and a carried stream takes its current reader
    explicit: usize,
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
    kind: BodyKind,
    /// the function this body defines, and the link below it in its
    /// chain, which is what `existing` calls (log 28)
    func: Option<FnInfo>,
    below: Option<String>,
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
    /// may the feature being lowered name something of `owner`'s? Its
    /// own layer or a lower one, never up (log 28); the front end's
    /// builtins belong to no feature
    fn reach(&self, what: &str, owner: &str, file: &str, line: usize) -> Result<(), Error> {
        if owner.is_empty() || owner == self.cur {
            return Ok(());
        }
        let (from, to) = (self.ranks.get(&self.cur).copied().unwrap_or(0), self.ranks.get(owner).copied().unwrap_or(0));
        if to > from {
            return Err(lex::error(file, line, format!("'{}' belongs to feature {}, whose layer is above {}'s: a name reaches down or sideways, never up", what, owner, self.cur)));
        }
        Ok(())
    }

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
        if let Some(owner) = self.type_feature.get(name) {
            self.reach(name, &owner.clone(), file, line)?;
        }
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
                self.type_feature.insert(t.name.clone(), self.cur.clone());
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
                self.type_feature.insert(t.name.clone(), self.cur.clone());
            }
        }
        Ok(())
    }

    fn declare(&mut self, f: &FnDecl, feature: &str, file: &str) -> Result<(), Error> {
        if !f.platform.is_empty() {
            return Err(lex::error(file, f.line, "platform bodies are not in this item yet"));
        }
        let key = mangle(&f.name);
        let mut params = Vec::new();
        for p in f.params() {
            // a task's `$` parameters are the streams it reads (log 25)
            let ty = if f.task && p.seq {
                let elem = self.ty(&p.ty, false, file, p.line)?;
                self.stream_ty(elem, false, file, p.line)?
            } else {
                self.ty(&p.ty, p.seq, file, p.line)?
            };
            params.push((p.name.clone(), ty));
        }
        let mut results = Vec::new();
        if f.task {
            let [r] = f.results.as_slice() else {
                return Err(lex::error(file, f.line, "a task produces one stream: `on (T x$) << name (...)`"));
            };
            if !r.seq {
                return Err(lex::error(file, f.line, format!("a task's result is the stream it produces: `on ({} {}$) << ...`", r.ty, r.name)));
            }
            let elem = self.ty(&r.ty, false, file, r.line)?;
            results.push((r.name.clone(), self.stream_ty(elem, false, file, r.line)?));
        }
        for r in &f.results {
            if f.task {
                break;
            }
            results.push((r.name.clone(), self.ty(&r.ty, r.seq, file, r.line)?));
        }
        let operator = f.name.iter().any(|p| matches!(p, NamePart::Sym(_)));
        if operator && f.task {
            return Err(lex::error(file, f.line, "a task has a name, not a symbol"));
        }
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
            // a redefinition (log 28): the same signature from a later
            // feature joins the chain, and its body will call `existing`
            let last = other.chain.last().cloned().unwrap_or_default();
            if last == feature {
                return Err(lex::error(file, f.line, format!("'{}' is defined twice in feature {}", spoken(other), feature)));
            }
            if other.task || f.task {
                return Err(lex::error(file, f.line, format!("'{}' is a task: a task is not redefined in this milestone", spoken(other))));
            }
            if operator {
                return Err(lex::error(file, f.line, "an operator is not redefined in this milestone"));
            }
            if other.params != params || other.results != results {
                return Err(lex::error(file, f.line, format!("'{}' redefines feature {}'s with different parameters or results: a redefinition keeps the signature", spoken(other), last)));
            }
            self.reach(&spoken(other), &last, file, f.line)?;
            let i = self.funcs.iter().position(|g| g.ir == ir).unwrap();
            // the function stays the first definer's: a lower layer's
            // name, which control may flow up through (section 12)
            self.funcs[i].chain.push(feature.to_string());
            return Ok(());
        }
        self.funcs.push(FnInfo { key, ir, parts: f.name.clone(), params, results, feature: feature.to_string(), task: f.task, chain: vec![feature.to_string()] });
        Ok(())
    }

    /// the task a phrase calls, its arguments, and the rate after it —
    /// `count down from (10) at (1 hz)` — or None when the phrase is not
    /// a task call (the ordinary call path then reports what it is)
    fn task_call(&self, e: &Expr, vars: Option<&HashMap<String, Var>>, file: &str) -> Result<Option<(FnInfo, Vec<Expr>, i64)>, Error> {
        let ExprKind::Phrase(parts) = &e.kind else { return Ok(None) };
        let (parts, hz) = match parts.as_slice() {
            [rest @ .., Part::Word(at), Part::Args(a)] if at == "at" && a.len() == 1 && matches!(&a[0].value.kind, ExprKind::Unit(..)) => (rest, Some(&a[0].value)),
            _ => (parts.as_slice(), None),
        };
        let is_var = |w: &str| vars.is_some_and(|v| v.contains_key(w)) || self.fvar(w).is_some();
        let Ok((info, args)) = find_function(&self.funcs, parts, &is_var, file, e.line) else {
            return Ok(None);
        };
        if !info.task {
            return Ok(None);
        }
        let hz = match hz {
            Some(r) => self.rate_hz(r, file)?,
            None => 0,
        };
        Ok(Some((info.clone(), args, hz)))
    }

    /// the stream variables a task call moves: its arguments at the
    /// task's stream parameters
    fn task_streams(&self, parts: &[Part]) -> Vec<String> {
        let e = Expr { kind: ExprKind::Phrase(parts.to_vec()), line: 0 };
        let Ok(Some((info, args, _))) = self.task_call(&e, None, "") else { return Vec::new() };
        args.iter()
            .zip(&info.params)
            .filter_map(|(a, (_, t))| match (&a.kind, t) {
                (ExprKind::Seq(n), Ty::Stream(..)) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }

    /// Run a task now (log 25): into the stream `out`, with `__hz` the
    /// rate given, the enclosing task's, or none; each stream argument
    /// is a stream variable, and takes the reader the task returns
    fn run_task(&mut self, info: &FnInfo, args: &[Expr], hz: i64, out: &Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let file = b.file.clone();
        self.reach(&spoken(info), &info.feature, &file, line)?;
        let Ty::Stream(want, _) = &info.results[0].1 else { unreachable!() };
        let Ty::Stream(have, _) = &out.ty else { unreachable!() };
        if want != have {
            return Err(lex::error(&file, line, format!("'{}' produces {} and '{}' holds {}", info.key, want.ir(), out.text, have.ir())));
        }
        let mut ops = vec![out.text.clone()];
        let mut moved: Vec<(String, Ty)> = Vec::new();
        for (a, (pname, pty)) in args.iter().zip(&info.params) {
            if let Ty::Stream(pe, _) = pty {
                let ExprKind::Seq(n) = &a.kind else {
                    return Err(lex::error(&file, a.line, format!("'{}' reads '{}$' as a stream: give it a stream variable", info.key, pname)));
                };
                let Some(Ty::Stream(ae, _)) = self.stream_var(n, b) else {
                    return Err(lex::error(&file, a.line, format!("'{}$' is not a stream: '{}' reads one here", n, info.key)));
                };
                if ae != *pe {
                    return Err(lex::error(&file, a.line, format!("'{}' reads a stream of {}, '{}$' holds {}", info.key, pe.ir(), n, ae.ir())));
                }
                let v = self.lower_expr(&Expr { kind: ExprKind::Name(n.clone()), line: a.line }, None, b, None)?;
                if b.vars.contains_key(n) {
                    b.assignable(n, a.line)?;
                }
                ops.push(v.text);
                moved.push((n.clone(), v.ty));
            } else {
                ops.push(self.plain_arg(info, a, pty, b)?);
            }
        }
        ops.push(match (&b.kind, hz) {
            (BodyKind::Task { hz: h, .. }, 0) => h.clone(),
            _ => hz.to_string(),
        });
        // the moved readers: a local's next version, a feature variable's setter
        let mut defs = Vec::new();
        let mut sets = Vec::new();
        for (n, ty) in &moved {
            if b.vars.contains_key(n) {
                let ir = b.define(n, ty.clone());
                defs.push(format!("{}: {}", ir, ty.ir()));
            } else {
                let t = b.tmp();
                sets.push((n.clone(), t.clone()));
                defs.push(format!("{}: {}", t, ty.ir()));
            }
        }
        let call = format!("{}({})", info.ir, ops.join(", "));
        if defs.is_empty() {
            b.line(&call);
        } else {
            b.line(&format!("{} = {}", defs.join(", "), call));
        }
        for (n, t) in sets {
            b.line(&format!("__set_{}({})", n, t));
        }
        Ok(())
    }

    /// The nodes a feature-scope declaration wires (log 25): `T x$ =
    /// task(...)`, or task calls in a `<<` chain after its pushed items
    fn collect_nodes(&mut self, v: &super::syntax::VarDecl, feature: &str, file: &str) -> Result<(), Error> {
        let calls: Vec<&Expr> = match &v.init {
            Some(Init::Value(e)) if self.task_call(e, None, file)?.is_some() => vec![e],
            Some(Init::Pushes { items, cond }) => {
                let mut first = None;
                for (i, e) in items.iter().enumerate() {
                    if self.task_call(e, None, file)?.is_some() {
                        first = Some(i);
                        break;
                    }
                }
                let Some(first) = first else { return Ok(()) };
                if cond.is_some() {
                    return Err(lex::error(file, v.line, "a task call is not repeated with `while`: the task's own chain says when it stops"));
                }
                items[first..].iter().collect()
            }
            _ => return Ok(()),
        };
        for e in calls {
            let Some((info, args, hz)) = self.task_call(e, None, file)? else {
                return Err(lex::error(file, e.line, "a value pushed after a task call at feature scope: a chain's items come before its tasks"));
            };
            self.reach(&spoken(&info), &info.feature, file, e.line)?;
            for (a, (pname, pty)) in args.iter().zip(&info.params) {
                if let Ty::Stream(pe, _) = pty {
                    let ExprKind::Seq(n) = &a.kind else {
                        return Err(lex::error(file, a.line, format!("'{}' reads '{}$' as a stream: wire a feature-scope stream to it", info.key, pname)));
                    };
                    match self.fvar(n).map(|f| f.ty.clone()) {
                        Some(Ty::Stream(ae, _)) if ae == *pe => {}
                        Some(Ty::Stream(ae, _)) => return Err(lex::error(file, a.line, format!("'{}' reads a stream of {}, '{}$' holds {}", info.key, pe.ir(), n, ae.ir()))),
                        Some(t) => return Err(lex::error(file, a.line, format!("'{}$' is a {}, not a stream: a stream is declared with `<<` or `at (n hz)`", n, t.ir()))),
                        None => return Err(lex::error(file, a.line, format!("'{}$' is not a feature-scope stream", n))),
                    }
                    self.reach(&format!("{}$", n), &self.fvar(n).unwrap().feature.clone(), file, a.line)?;
                    self.node_inputs.insert(n.clone());
                }
            }
            let text = format!("{} {}$ {} {}", v.ty, v.name, if matches!(v.init, Some(Init::Value(_))) { "=" } else { "<<" }, phrase_text(e));
            self.nodes.push(Node { info, out: v.name.clone(), args, hz, feature: feature.to_string(), file: file.to_string(), text });
        }
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
        let ty = self.decl_ty(v, None, file)?;
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

    /// the context struct, its storage, its accessors, `__zero_reset`,
    /// which puts every variable's initial value in it, and the
    /// scheduler with a function per node (log 25)
    fn emit_context(&mut self, store: &Store) -> Result<(), Error> {
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: String::new(), depth: 0, loops: Vec::new(), kind: BodyKind::Reset, func: None, below: None };
        b.line("q: ptr = addr __out_n");
        b.line("store 0: i64, q");
        b.line("a: ptr = addr __arena");
        b.line("h: ptr = addr __heap");
        b.line("arena_init(a, h, 65536)");
        b.line("k: ptr = addr __clock");
        b.line("z: time = seconds(0)");
        b.line("store z, k");
        b.line("r: ptr = addr __running");
        b.line("store 0: i64, r");
        // a node's state: its own reader of each input, and whether it
        // has finished, as fields after the variables
        for (k, node) in self.nodes.iter().enumerate() {
            let mut fields = Vec::new();
            for (a, (pname, pty)) in node.args.iter().zip(&node.info.params) {
                if let (ExprKind::Seq(_), Ty::Stream(..)) = (&a.kind, pty) {
                    fields.push((format!("__node{}_{}", k + 1, pname), pty.clone()));
                    // how many items the ring had when the node last ran
                    fields.push((format!("__node{}_{}_seen", k + 1, pname), Ty::Num("i64".into())));
                }
            }
            fields.push((format!("__node{}_fin", k + 1), Ty::Bool));
            for (name, ty) in fields {
                self.fvars.push(FVar { name, ty, scope: "node".into(), merge: "last".into(), feature: node.feature.clone() });
            }
        }
        // every feature's implicit `enabled` (section 5, log 28), first
        for (i, f) in self.features.clone().iter().enumerate() {
            self.fvars.insert(i, FVar { name: format!("__enabled_{}", f), ty: Ty::Bool, scope: "user".into(), merge: "last".into(), feature: f.clone() });
        }
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
            // the initial values, in composition order: every feature on,
            // then the variables, then the nodes' state
            let mut inits: Vec<String> = self.features.iter().map(|_| "1".to_string()).collect();
            let mut init_of: HashMap<String, String> = HashMap::new();
            for feat in &store.features {
                for d in &feat.code.decls {
                    let Decl::Var(v) = d else { continue };
                    b.file = feat.code.file.clone();
                    self.cur = feat.name.clone();
                    let ty = self.fvar(&v.name).unwrap().ty.clone();
                    let val = match &v.init {
                        _ if matches!(ty, Ty::Stream(..)) => {
                            let hz = match &v.rate {
                                Some(r) => self.rate_hz(r, &b.file)?,
                                None => CLOCK_HZ,
                            };
                            let s = self.make_stream(&ty, hz, &mut b, None)?;
                            match &v.init {
                                Some(Init::Pushes { items, cond }) => {
                                    // the items before the first task call
                                    // are pushed here; the calls are nodes
                                    let n = items.iter().position(|e| matches!(self.task_call(e, None, &b.file), Ok(Some(_)))).unwrap_or(items.len());
                                    self.lower_pushes(&v.name, &s, &items[..n], cond.as_ref(), &mut b)?;
                                }
                                None => {}
                                Some(Init::Value(_)) => {}
                                Some(_) => return Err(lex::error(&b.file, v.line, "a stream is filled with `<<`")),
                            }
                            s
                        }
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
                        Some(Init::Pushes { .. }) => unreachable!(),
                    };
                    init_of.insert(v.name.clone(), val.text.clone());
                    inits.push(val.text);
                }
            }
            // a node's readers start where its inputs' rings start (a
            // reader is a value: a copy is its own position)
            for node in &self.nodes {
                for (a, (_, pty)) in node.args.iter().zip(&node.info.params) {
                    if let (ExprKind::Seq(n), Ty::Stream(..)) = (&a.kind, pty) {
                        inits.push(init_of[n].clone());
                        inits.push("0".into());
                    }
                }
                inits.push("0".into());
            }
            let c = b.tmp();
            b.line(&format!("{}: __ctx = pack {}", c, inits.join(", ")));
            b.line("p: ptr = addr __ctx_mem");
            b.line(&format!("store {}, p", c));
        }
        if !self.nodes.is_empty() {
            b.line("__run()");
        }
        b.line("ret");
        writeln!(self.out, "\n; before every case: the print buffer emptied, the variables at their initial values{}", if self.nodes.is_empty() { "" } else { ", the nodes run" }).unwrap();
        writeln!(self.out, "fn __zero_reset() {{").unwrap();
        self.out.push_str(&b.out);
        self.out.push_str("}\n");
        for f in &self.fvars {
            let t = f.ty.ir();
            writeln!(self.out, "\nfn __get_{}() -> {} {{\n    p: ptr = addr __ctx_mem\n    c: __ctx = load p\n    v: {} = get c, {}\n    ret v\n}}", f.name, t, t, f.name).unwrap();
            writeln!(self.out, "\nfn __set_{}(v: {}) {{\n    p: ptr = addr __ctx_mem\n    c: __ctx = load p\n    c2: __ctx = set c, {}, v\n    store c2, p\n    ret\n}}", f.name, t, f.name).unwrap();
        }
        let nodes = std::mem::take(&mut self.nodes);
        for (k, node) in nodes.iter().enumerate() {
            self.emit_node(k + 1, node)?;
        }
        if !nodes.is_empty() {
            writeln!(self.out, "\n; the scheduler (log 25): passes over the nodes in declaration order until a pass runs nothing").unwrap();
            writeln!(self.out, "fn __run() {{\n    p: ptr = addr __running\n    busy: i64 = load p\n    idle: u1 = cmp.eq busy, 0\n    if idle {{\n        store 1: i64, p\n        loop() {{").unwrap();
            let mut any = String::new();
            for k in 1..=nodes.len() {
                writeln!(self.out, "            r{}: u1 = __node{}()", k, k).unwrap();
                if k == 1 {
                    any = "r1".into();
                } else {
                    writeln!(self.out, "            any{}: u1 = or {}, r{}", k, any, k).unwrap();
                    any = format!("any{}", k);
                }
            }
            writeln!(self.out, "            if {} {{\n                continue\n            }} else {{\n                break\n            }}\n        }}\n        store 0: i64, p\n    }}\n    ret\n}}", any).unwrap();
        }
        self.nodes = nodes;
        Ok(())
    }

    /// the links of every chain (log 28): `key` is the outermost, and
    /// `key__before_F` the chain below feature F's body; each reads its
    /// feature's `enabled` and calls the body or the link below; the
    /// innermost, off, gives the results' zeros
    fn emit_links(&mut self) {
        let chains: Vec<FnInfo> = self.funcs.iter().filter(|f| f.chain.len() > 1).cloned().collect();
        for info in chains {
            let n = info.chain.len();
            let params: Vec<String> = info.params.iter().map(|(p, t)| format!("{}: {}", p, t.ir())).collect();
            let args: Vec<String> = info.params.iter().map(|(p, _)| p.clone()).collect();
            let rets: Vec<String> = info.results.iter().map(|(_, t)| t.ir()).collect();
            let sig_ret = match rets.len() {
                0 => String::new(),
                1 => format!(" -> {}", rets[0]),
                _ => format!(" -> ({})", rets.join(", ")),
            };
            writeln!(self.out, "\n; {}: the chain {}, newest outermost; a link whose feature is off falls through", info.ir, info.chain.iter().rev().cloned().collect::<Vec<_>>().join(", ")).unwrap();
            for i in (0..n).rev() {
                let name = if i == n - 1 { info.ir.clone() } else { format!("{}__before_{}", info.ir, info.chain[i + 1]) };
                let body = format!("{}__{}({})", info.ir, info.chain[i], args.join(", "));
                let under = if i == 0 { None } else { Some(format!("{}__before_{}({})", info.ir, info.chain[i], args.join(", "))) };
                let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: String::new(), depth: 0, loops: Vec::new(), kind: BodyKind::Node, func: None, below: None };
                b.line(&format!("on: u1 = __get___enabled_{}()", info.chain[i]));
                if rets.is_empty() {
                    b.line("if on {");
                    b.depth += 1;
                    b.line(&body);
                    b.depth -= 1;
                    if let Some(u) = under {
                        b.line("} else {");
                        b.depth += 1;
                        b.line(&u);
                        b.depth -= 1;
                    }
                    b.line("}");
                    b.line("ret");
                } else {
                    let outs: Vec<String> = (0..rets.len()).map(|_| b.tmp()).collect();
                    let defs: Vec<String> = outs.iter().zip(&rets).map(|(o, t)| format!("{}: {}", o, t)).collect();
                    b.line(&format!("{} = if on {{", defs.join(", ")));
                    b.depth += 1;
                    let vs: Vec<String> = (0..rets.len()).map(|_| b.tmp()).collect();
                    let ds: Vec<String> = vs.iter().zip(&rets).map(|(v, t)| format!("{}: {}", v, t)).collect();
                    b.line(&format!("{} = {}", ds.join(", "), body));
                    b.line(&format!("yield {}", vs.join(", ")));
                    b.depth -= 1;
                    b.line("} else {");
                    b.depth += 1;
                    match under {
                        Some(u) => {
                            let vs: Vec<String> = (0..rets.len()).map(|_| b.tmp()).collect();
                            let ds: Vec<String> = vs.iter().zip(&rets).map(|(v, t)| format!("{}: {}", v, t)).collect();
                            b.line(&format!("{} = {}", ds.join(", "), u));
                            b.line(&format!("yield {}", vs.join(", ")));
                        }
                        None => {
                            let mut zs = Vec::new();
                            for (_, t) in &info.results {
                                zs.push(self.zero_val(t, &mut b).text);
                            }
                            b.line(&format!("yield {}", zs.join(", ")));
                        }
                    }
                    b.depth -= 1;
                    b.line("}");
                    b.line(&format!("ret {}", outs.join(", ")));
                }
                writeln!(self.out, "fn {}({}){} {{", name, params.join(", "), sig_ret).unwrap();
                self.out.push_str(&b.out);
                self.out.push_str("}\n");
            }
        }
    }

    /// `fn __nodeK() -> u1`: run the node when it is pending, and say
    /// whether it ran. Pending: an input has more items than it had
    /// when the node last ran (a node may leave what it cannot take yet,
    /// a partial token, unread), or has ended and the node has not run
    /// since; a node with no inputs, once. After a run the node is
    /// finished when all its inputs have ended.
    fn emit_node(&mut self, k: usize, node: &Node) -> Result<(), Error> {
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: Vec::new(), file: node.file.clone(), depth: 0, loops: Vec::new(), kind: BodyKind::Node, func: None, below: None };
        b.line(&format!("fin: u1 = __get___node{}_fin()", k));
        b.line("notfin: u1 = xor fin, 1");
        let mut readers = Vec::new();
        let mut pending: Option<String> = None;
        for (a, (pname, pty)) in node.args.iter().zip(&node.info.params) {
            let Ty::Stream(..) = pty else { continue };
            let r = b.tmp();
            b.line(&format!("{}: {} = __get___node{}_{}()", r, pty.ir(), k, pname));
            let rv = Val { text: r.clone(), ty: pty.clone(), literal: false };
            let first = self.first_reader(&rv, &mut b);
            let pushed = self.pushed_of(&first, &mut b);
            let seen = b.tmp();
            b.line(&format!("{}: i64 = __get___node{}_{}_seen()", seen, k, pname));
            let some = b.tmp();
            b.line(&format!("{}: u1 = cmp.gt {}, {}", some, pushed, seen));
            let e = b.tmp();
            b.line(&format!("{}: u1 = ended({})", e, first));
            let en = b.tmp();
            b.line(&format!("{}: u1 = and {}, notfin", en, e));
            let p = b.tmp();
            b.line(&format!("{}: u1 = or {}, {}", p, some, en));
            pending = Some(match pending {
                None => p,
                Some(q) => {
                    let pq = b.tmp();
                    b.line(&format!("{}: u1 = or {}, {}", pq, q, p));
                    pq
                }
            });
            readers.push((pname.clone(), r, pty.clone(), a.line));
        }
        let pending = pending.unwrap_or_else(|| "notfin".into());
        // a feature that is off runs no node; its readers keep their place
        b.line(&format!("on: u1 = __get___enabled_{}()", node.feature));
        b.line(&format!("due: u1 = and {}, on", pending));
        b.line("ran: u1 = if due {");
        b.depth += 1;
        let out = self.read_fvar(&node.out, &mut b, None, 0)?;
        let mut ops = vec![out.text.clone()];
        let mut ri = 0;
        for (a, (_, pty)) in node.args.iter().zip(&node.info.params) {
            if let Ty::Stream(..) = pty {
                ops.push(readers[ri].1.clone());
                ri += 1;
            } else {
                ops.push(self.plain_arg(&node.info, a, pty, &mut b)?);
            }
        }
        ops.push(node.hz.to_string());
        let call = format!("{}({})", node.info.ir, ops.join(", "));
        let mut moved = Vec::new();
        for (_, _, ty, _) in &readers {
            let r2 = b.tmp();
            moved.push(format!("{}: {}", r2, ty.ir()));
        }
        if moved.is_empty() {
            b.line(&call);
        } else {
            b.line(&format!("{} = {}", moved.join(", "), call));
        }
        let mut done: Option<String> = None;
        for ((pname, _, ty, _), m) in readers.iter().zip(&moved) {
            let r2 = m.split(':').next().unwrap().to_string();
            b.line(&format!("__set___node{}_{}({})", k, pname, r2));
            let first = self.first_reader(&Val { text: r2, ty: ty.clone(), literal: false }, &mut b);
            let pushed = self.pushed_of(&first, &mut b);
            b.line(&format!("__set___node{}_{}_seen({})", k, pname, pushed));
            let e = b.tmp();
            b.line(&format!("{}: u1 = ended({})", e, first));
            done = Some(match done {
                None => e,
                Some(d) => {
                    let de = b.tmp();
                    b.line(&format!("{}: u1 = and {}, {}", de, d, e));
                    de
                }
            });
        }
        b.line(&format!("__set___node{}_fin({})", k, done.unwrap_or_else(|| "1".into())));
        b.line("yield 1");
        b.depth -= 1;
        b.line("} else {");
        b.depth += 1;
        b.line("yield 0");
        b.depth -= 1;
        b.line("}");
        b.line("ret ran");
        writeln!(self.out, "\n; node {}: {} — run when an input has more than the node has seen, or has ended and the node has not run since", k, node.text).unwrap();
        writeln!(self.out, "fn __node{}() -> u1 {{", k).unwrap();
        self.out.push_str(&b.out);
        self.out.push_str("}\n");
        Ok(())
    }

    /// how many items a reader's ring has received: its position plus
    /// what is unread
    fn pushed_of(&mut self, reader: &str, b: &mut Body) -> String {
        let (p, t) = (b.tmp(), b.tmp());
        b.line(&format!("{}: i64, {}: i64 = position({})", p, t, reader));
        let n = b.tmp();
        b.line(&format!("{}: i64 = count {}", n, reader));
        let pushed = b.tmp();
        b.line(&format!("{}: i64 = add {}, {}", pushed, p, n));
        pushed
    }

    /// a task's plain argument, lowered against its parameter
    fn plain_arg(&mut self, info: &FnInfo, a: &Expr, pty: &Ty, b: &mut Body) -> Result<String, Error> {
        let file = b.file.clone();
        let mut v = self.lower_expr(a, Some(pty), b, None)?;
        if v.literal && fits_literal(&v, pty) {
            v.ty = pty.clone();
        }
        if !fits(&v.ty, pty) {
            return Err(lex::error(&file, a.line, format!("'{}' wants a {} here, given a {}", info.key, pty.ir(), v.ty.ir())));
        }
        Ok(v.text)
    }

    /// a feature variable read: a call to its getter
    fn read_fvar(&mut self, name: &str, b: &mut Body, dst: Option<&str>, line: usize) -> Result<Val, Error> {
        let f = self.fvar(name).unwrap().clone();
        self.reach(name, &f.feature, &b.file, line)?;
        let out = name_for(dst, &f.ty, b);
        b.line(&format!("{}: {} = __get_{}()", out, f.ty.ir(), name));
        Ok(Val { text: out, ty: f.ty, literal: false })
    }

    /// a feature variable written: a call to its setter
    fn write_fvar(&mut self, name: &str, v: Val, b: &mut Body, line: usize) -> Result<(), Error> {
        let f = self.fvar(name).unwrap().clone();
        self.reach(name, &f.feature, &b.file, line)?;
        let ty = f.ty;
        if !(v.ty == ty || (v.literal && fits_literal(&v, &ty))) {
            return Err(lex::error(&b.file, line, format!("'{}' is {} but the value is {}", name, ty.ir(), v.ty.ir())));
        }
        b.line(&format!("__set_{}({})", name, v.text));
        Ok(())
    }

    fn lower_fn(&mut self, f: &FnDecl, feature: &str, file: &str) -> Result<(), Error> {
        let key = mangle(&f.name);
        let info = self.funcs.iter().find(|g| g.key == key && g.parts == f.name && g.params.len() == f.params().count()).unwrap().clone();
        // a task's IR results are its stream parameters, moved on (log 25)
        let results: Vec<(String, Ty)> = if info.task { info.params.iter().filter(|(_, t)| matches!(t, Ty::Stream(..))).cloned().collect() } else { info.results.clone() };
        let kind = if info.task { BodyKind::Task { out: info.results[0].0.clone(), hz: "__hz".into() } } else { BodyKind::Fn };
        // in a chain the body is `key__feature`, and `existing` is the link below
        let (name, below) = if info.chain.len() > 1 {
            let i = info.chain.iter().position(|c| c == feature).unwrap();
            (format!("{}__{}", info.ir, feature), if i == 0 { None } else { Some(format!("{}__before_{}", info.ir, feature)) })
        } else {
            (info.ir.clone(), None)
        };
        let mut b = Body { out: String::new(), ntmp: 0, vars: HashMap::new(), defs: HashMap::new(), results: results.clone(), file: file.to_string(), depth: 0, loops: Vec::new(), kind, func: Some(info.clone()), below };
        let mut sig = format!("fn {}(", name);
        let mut sig_params: Vec<(String, Ty)> = Vec::new();
        if info.task {
            sig_params.push(info.results[0].clone());
        }
        sig_params.extend(info.params.iter().cloned());
        if info.task {
            sig_params.push(("__hz".into(), Ty::Num("i64".into())));
        }
        for (i, (n, t)) in sig_params.iter().enumerate() {
            if i > 0 {
                sig.push_str(", ");
            }
            write!(sig, "{}: {}", n, t.ir()).unwrap();
            if b.vars.contains_key(n) {
                return Err(lex::error(file, f.line, format!("parameter '{}' is named twice{}", n, if info.task { " (a task's result is its first parameter)" } else { "" })));
            }
            b.define(n, t.clone());
        }
        sig.push(')');
        match results.len() {
            0 => {}
            1 => write!(sig, " -> {}", results[0].1.ir()).unwrap(),
            _ => {
                let ts: Vec<String> = results.iter().map(|(_, t)| t.ir()).collect();
                write!(sig, " -> ({})", ts.join(", ")).unwrap();
            }
        }
        if !info.task {
            for (n, t) in &results {
                if b.vars.contains_key(n) {
                    return Err(lex::error(file, f.line, format!("result '{}' is also a parameter, or named twice", n)));
                }
                b.declare(n, t.clone());
            }
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
                Some(Init::Pushes { .. }) => return Err(lex::error(&file, v.line, "a stream is declared before the loop, which then carries it")),
            };
            if v.rate.is_some() {
                return Err(lex::error(&file, v.line, "a stream is declared before the loop, which then carries it"));
            }
            header.push((v.name.clone(), ty.clone(), init.text));
            carried.push(v.name.clone());
            tys.push(ty);
        }
        // a stream the body moves (`advance`, `frame`) is carried too: its
        // position threads through the loop as an ordinary value (log 23)
        let mut moved = Vec::new();
        moved_streams(body, &mut moved, &|parts| self.task_streams(parts));
        for n in moved {
            if carried.contains(&n) {
                continue;
            }
            if let Some(v) = b.vars.get(&n) {
                if matches!(v.ty, Ty::Stream(..)) && v.set {
                    header.push((n.clone(), v.ty.clone(), v.ir.clone()));
                    carried.push(n.clone());
                    tys.push(v.ty.clone());
                }
            }
        }
        // the carried variables are declared inside the loop
        b.loops.push(LoopCtx { carried: carried.clone(), explicit: vars.len(), item: None, loaded: None, breaks: 0 });
        let mut hdr = Vec::new();
        let inner = b.loops.len();
        for (n, ty, init) in &header {
            let ir = b.define(n, ty.clone());
            // a stream carried from outside is the loop's own inside it
            b.vars.get_mut(n).unwrap().loop_depth = inner;
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
        self.no_moved_stream(body, b)?;
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
        b.loops.push(LoopCtx { carried: Vec::new(), explicit: 0, item: Some((var.to_string(), op, step.clone())), loaded: None, breaks: 1 });
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
        b.loops.push(LoopCtx { carried: Vec::new(), explicit: 0, item: Some((k.clone(), "add", "1".into())), loaded: Some(var.to_string()), breaks: 1 });
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
            Ty::Stream(..) => self.make_stream(t, CLOCK_HZ, b, None).unwrap_or(Val { text: "0".into(), ty: t.clone(), literal: true }),
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
                // `countdown.enabled = false`: a feature's switch (log 28)
                if let [t] = targets.as_slice() {
                    if let Some(feat) = &t.feature {
                        if t.name != "enabled" || !self.features.contains(feat) {
                            return Err(lex::error(&file, t.line, format!("'{}.{}': a feature's implicit variable is `{}.enabled`", feat, t.name, feat)));
                        }
                        self.reach(&format!("{}.enabled", feat), feat, &file, t.line)?;
                        let v = self.lower_expr(value, Some(&Ty::Bool), b, None)?;
                        if v.ty != Ty::Bool {
                            return Err(lex::error(&file, *line, format!("'{}.enabled' is a bool, given a {}", feat, v.ty.ir())));
                        }
                        b.line(&format!("__set___enabled_{}({})", feat, v.text));
                        return Ok(());
                    }
                }
                if targets.iter().any(|t| t.feature.is_some()) {
                    return Err(lex::error(&file, *line, "a feature's `enabled` is assigned on its own"));
                }
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
                let ty = self.decl_ty(v, Some(&b.vars), &file)?;
                b.declare(&v.name, ty.clone());
                if let Ty::Stream(..) = &ty {
                    // a stream: its ring, then the chain of pushes, or the
                    // task that fills it, run now (log 25)
                    let hz = match &v.rate {
                        Some(r) => self.rate_hz(r, &file)?,
                        None => CLOCK_HZ,
                    };
                    let s = self.make_stream(&ty, hz, b, Some(&v.name))?;
                    self.assign(&v.name, s.clone(), b, v.line)?;
                    return match &v.init {
                        Some(Init::Pushes { items, cond }) => self.lower_pushes(&v.name, &s, items, cond.as_ref(), b),
                        None => Ok(()),
                        Some(Init::Value(e)) => {
                            let (info, args, hz) = self.task_call(e, Some(&b.vars), &file)?.unwrap();
                            self.run_task(&info, &args, hz, &s, b, e.line)
                        }
                        Some(_) => Err(lex::error(&file, v.line, "a stream is filled with `<<`")),
                    };
                }
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
                    Some(Init::Pushes { .. }) => unreachable!(),
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
                let explicit = ctx.explicit;
                if values.is_empty() {
                    let vals = b.current(&carried);
                    b.line(format!("continue {}", vals.join(", ")).trim_end());
                    return Ok(());
                }
                if values.len() != explicit {
                    return Err(lex::error(&file, *line, format!("the loop declares {} variable(s), 'continue' gives {}", explicit, values.len())));
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
                // a stream the loop carries for the body goes on as it stands
                vals.extend(b.current(&carried[explicit..]));
                b.line(&format!("continue {}", vals.join(", ")));
                Ok(())
            }
            Stmt::Check { cond, line } => {
                // `check (c)` (section 14, log 29): the IR's trap when c
                // does not hold, the site printed first so the runner
                // can name it
                let cv = self.lower_expr(cond, Some(&Ty::Bool), b, None)?;
                if cv.ty != Ty::Bool {
                    return Err(lex::error(&file, *line, "'check' takes a bool"));
                }
                let cv = b.materialize(&cv);
                b.line(&format!("if {} {{", cv.text));
                b.line("} else {");
                b.depth += 1;
                let base = file.rsplit('/').next().unwrap_or(&file).to_string();
                let site = Expr { kind: ExprKind::Str(format!("check at {}:{}", base, line)), line: *line };
                let sv = self.lower_expr(&site, None, b, None)?;
                b.line(&format!("__print({})", sv.text));
                let z = b.tmp();
                b.line(&format!("{}: u1 = const 0", z));
                b.line(&format!("check {}", z));
                b.depth -= 1;
                b.line("}");
                Ok(())
            }
            Stmt::Push { target, items, cond, line } => {
                let ExprKind::Seq(n) = &target.kind else {
                    return Err(lex::error(&file, *line, "`<<` pushes into a stream, named `x$`"));
                };
                if self.stream_var(n, b).is_none() {
                    return Err(match self.seq_or_fvar_ty(n, b) {
                        Some(t) => lex::error(&file, *line, format!("'{}$' is a {}, not a stream: a stream is declared with `<<` or `at (n hz)`", n, t.ir())),
                        None => lex::error(&file, *line, format!("'{}$' is not declared", n)),
                    });
                }
                let s = self.lower_expr(&Expr { kind: ExprKind::Name(n.clone()), line: *line }, None, b, None)?;
                self.lower_pushes(n, &s, items, cond.as_ref(), b)?;
                self.trigger(n, b);
                Ok(())
            }
        }
    }

    /// `existing name(args)` (log 28): the link below this body in its
    /// chain, called with the arguments; the phrase names the enclosing
    /// function
    fn existing_call(&mut self, parts: &[Part], b: &mut Body, line: usize) -> Result<(String, Vec<Ty>), Error> {
        let file = b.file.clone();
        let Some(info) = b.func.clone() else {
            return Err(lex::error(&file, line, "'existing' belongs in a function's body"));
        };
        let Some(below) = b.below.clone() else {
            return Err(lex::error(&file, line, format!("no earlier definition of '{}' for 'existing' to call{}", spoken(&info), if info.chain.len() > 1 { ": this is the first" } else { "" })));
        };
        let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
        let (named, args) = find_function(std::slice::from_ref(&info), parts, &is_var, &file, line).map_err(|_| lex::error(&file, line, format!("'existing' names the function it is in: `existing {}(...)`", spoken(&info))))?;
        let named = named.clone();
        let (ops, rtys) = self.lower_args(&named, &args, b)?;
        Ok((format!("{}({})", below, ops.join(", ")), rtys))
    }

    /// `q, r = f(...)`: a call with several results defines several variables
    fn lower_multi(&mut self, value: &Expr, names: &[String], tys: &[Ty], b: &mut Body, line: usize) -> Result<(), Error> {
        let file = b.file.clone();
        if let ExprKind::Existing(parts) = &value.kind {
            let (call, rtys) = self.existing_call(parts, b, line)?;
            if rtys.len() != names.len() {
                return Err(lex::error(&file, line, format!("'existing' gives {} result(s), {} wanted", rtys.len(), names.len())));
            }
            for ((n, want), got) in names.iter().zip(tys).zip(&rtys) {
                if want != got {
                    return Err(lex::error(&file, line, format!("'{}' is {} but 'existing' gives {}", n, want.ir(), got.ir())));
                }
            }
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
            b.line(&format!("{} = {}", defs.join(", "), call));
            for (n, tmp) in sets {
                b.line(&format!("__set_{}({})", n, tmp));
            }
            return Ok(());
        }
        let ExprKind::Phrase(parts) = &value.kind else {
            return Err(lex::error(&file, line, "several variables at once take a call with several results"));
        };
        // `int i, int t = position x$`: where a stream's reader stands
        if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(n), .. })] = parts.as_slice() {
            if w == "position" && self.stream_var(n, b).is_some() {
                let int = Ty::Num("int".into());
                if names.len() != 2 || tys.iter().any(|t| *t != int) {
                    return Err(lex::error(&file, line, "'position' gives two ints, the index of the next unread item and its tick: `int i, int t = position x$`"));
                }
                let s = self.lower_expr(&Expr { kind: ExprKind::Name(n.clone()), line }, None, b, None)?;
                let first = self.first_reader(&s, b);
                let (k, t) = (b.tmp(), b.tmp());
                b.line(&format!("{}: i64, {}: i64 = position({})", k, t, first));
                for (name, tmp) in names.iter().zip([k, t]) {
                    if b.vars.contains_key(name) {
                        let ir = b.define(name, int.clone());
                        b.line(&format!("{}: int = conv {}", ir, tmp));
                    } else {
                        let v = b.tmp();
                        b.line(&format!("{}: int = conv {}", v, tmp));
                        b.line(&format!("__set_{}({})", name, v));
                    }
                }
                return Ok(());
            }
        }
        let is_var = |w: &str| b.vars.contains_key(w) || self.fvar(w).is_some();
        let (info, args) = find_function(&self.funcs, parts, &is_var, &file, line)?;
        let info = info.clone();
        if info.task {
            return Err(lex::error(&file, line, format!("'{}' is a task: it is wired into a stream, `{} x$ = {}`", spoken(&info), task_elem(&info), phrase_text(value))));
        }
        self.reach(&spoken(&info), &info.feature, &file, line)?;
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
    // --- streams (section 9, log 23) ---

    /// a declaration's type: a stream when it is made with `<<` or has a
    /// rate, a sequence or a plain value otherwise
    fn decl_ty(&mut self, v: &super::syntax::VarDecl, vars: Option<&HashMap<String, Var>>, file: &str) -> Result<Ty, Error> {
        // a wiring, `T x$ = task(...)`, declares a stream too (log 25)
        let wired = match &v.init {
            Some(Init::Value(e)) => self.task_call(e, vars, file)?.is_some(),
            _ => false,
        };
        if wired && !v.seq {
            return Err(lex::error(file, v.line, format!("a task produces a stream: `{} {}$ = ...`", v.ty, v.name)));
        }
        let stream = v.seq && (v.rate.is_some() || wired || matches!(v.init, Some(Init::Pushes { .. })));
        if !stream {
            if v.rate.is_some() {
                return Err(lex::error(file, v.line, "a rate belongs on a stream, `T x$ at (n hz)`"));
            }
            return self.ty(&v.ty, v.seq, file, v.line);
        }
        let elem = self.ty(&v.ty, false, file, v.line)?;
        self.stream_ty(elem, v.rate.is_some(), file, v.line)
    }

    /// a stream of an element type: numbers, enumerations, and structs of
    /// those, the last as a generated struct of one stream per field
    fn stream_ty(&mut self, elem: Ty, regular: bool, file: &str, line: usize) -> Result<Ty, Error> {
        match &elem {
            Ty::Num(_) | Ty::Enum(_) => {}
            Ty::Struct(name) => {
                let Some(TypeInfo::Struct(fields)) = self.types.get(name) else { unreachable!() };
                let mut ir = Vec::new();
                for (f, t, _) in fields {
                    if !matches!(t, Ty::Num(_) | Ty::Enum(_)) {
                        return Err(lex::error(file, line, format!("a stream of {}: field '{}' is a {}, and a stream of structs holds numbers and enumerations in its fields", name, f, t.ir())));
                    }
                    ir.push(format!("{}: {}$", f, t.ir()));
                }
                if self.sstructs.insert(name.clone()) {
                    self.type_lines.push(format!("; a stream of {}: one ring per field
type __s_{} = struct {{ {} }}", name, name, ir.join(", ")));
                }
            }
            _ => return Err(lex::error(file, line, format!("a stream of {}: a stream holds numbers, enumerations or structs of those", elem.ir()))),
        }
        Ok(Ty::Stream(Box::new(elem), regular))
    }

    /// `at (n hz)` as a number of hertz
    fn rate_hz(&self, e: &Expr, file: &str) -> Result<i64, Error> {
        if let ExprKind::Unit(inner, u) = &e.kind {
            if let ExprKind::Int(n) = inner.kind {
                match u.as_str() {
                    "hz" if n > 0 => return Ok(n),
                    "khz" if n > 0 => return Ok(n * 1000),
                    _ => {}
                }
            }
        }
        Err(lex::error(file, e.line, "a rate is `at (n hz)` or `at (n khz)`, n positive"))
    }

    /// a stream of a struct: its fields and their types
    fn stream_fields(&self, ty: &Ty) -> Option<Vec<(String, Ty)>> {
        let Ty::Stream(e, _) = ty else { return None };
        let Ty::Struct(name) = e.as_ref() else { return None };
        let Some(TypeInfo::Struct(fields)) = self.types.get(name) else { unreachable!() };
        Some(fields.iter().map(|(f, t, _)| (f.clone(), t.clone())).collect())
    }

    /// a new stream: its ring in the arena, and a reader at its start
    fn make_stream(&mut self, ty: &Ty, hz: i64, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        let Ty::Stream(elem, regular) = ty else { unreachable!() };
        let maker = if *regular { "regular" } else { "stream" };
        match self.stream_fields(ty) {
            None => {
                self.rings.insert((elem.ir(), *regular));
                let out = name_for(dst, ty, b);
                b.line(&format!("{}: {} = __{}_{}({})", out, ty.ir(), maker, elem.ir(), hz));
                Ok(Val { text: out, ty: ty.clone(), literal: false })
            }
            Some(fields) => {
                let mut parts = Vec::new();
                for (_, t) in &fields {
                    self.rings.insert((t.ir(), *regular));
                    let r = b.tmp();
                    b.line(&format!("{}: {}$ = __{}_{}({})", r, t.ir(), maker, t.ir(), hz));
                    parts.push(r);
                }
                let out = name_for(dst, ty, b);
                b.line(&format!("{}: {} = pack {}", out, ty.ir(), parts.join(", ")));
                Ok(Val { text: out, ty: ty.clone(), literal: false })
            }
        }
    }

    /// the type of a stream variable in scope, local or feature-scope
    fn stream_var(&self, name: &str, b: &Body) -> Option<Ty> {
        let ty = match b.vars.get(name) {
            Some(v) => v.ty.clone(),
            None => self.fvar(name)?.ty.clone(),
        };
        matches!(ty, Ty::Stream(..)).then_some(ty)
    }

    /// the type of any variable in scope, for a message
    fn seq_or_fvar_ty(&self, name: &str, b: &Body) -> Option<Ty> {
        b.vars.get(name).map(|v| v.ty.clone()).or_else(|| self.fvar(name).map(|f| f.ty.clone()))
    }

    /// the reader the ring's counts are read from: the stream itself,
    /// or a struct stream's first field
    fn first_reader(&mut self, s: &Val, b: &mut Body) -> String {
        match self.stream_fields(&s.ty) {
            None => s.text.clone(),
            Some(fields) => {
                let (f, t) = &fields[0];
                let r = b.tmp();
                b.line(&format!("{}: {}$ = get {}, {}", r, t.ir(), s.text, f));
                r
            }
        }
    }

    /// an int as the i64 the library takes
    fn as_i64(&mut self, v: &Val, b: &mut Body) -> String {
        if v.literal || v.ty == Ty::Num("i64".into()) {
            return v.text.clone();
        }
        let t = b.tmp();
        b.line(&format!("{}: i64 = conv {}", t, v.text));
        t
    }

    /// a read of every field's stream, packed: `latest`, `peek`
    fn read_fields(&mut self, s: &Val, fields: &[(String, Ty)], op: &str, arg: &str, b: &mut Body, dst: Option<&str>) -> Val {
        let Ty::Stream(elem, _) = &s.ty else { unreachable!() };
        let mut vals = Vec::new();
        for (f, t) in fields {
            let r = b.tmp();
            b.line(&format!("{}: {}$ = get {}, {}", r, t.ir(), s.text, f));
            let v = b.tmp();
            b.line(&format!("{}: {} = {} {}{}", v, t.ir(), op, r, arg));
            vals.push(v);
        }
        let out = name_for(dst, elem, b);
        b.line(&format!("{}: {} = pack {}", out, elem.ir(), vals.join(", ")));
        Val { text: out, ty: elem.as_ref().clone(), literal: false }
    }

    /// the most recent item of a stream
    fn latest_of(&mut self, s: &Val, ty: &Ty, b: &mut Body, dst: Option<&str>) -> Result<Val, Error> {
        let Ty::Stream(elem, _) = ty else { unreachable!() };
        match self.stream_fields(ty) {
            Some(fields) => Ok(self.read_fields(s, &fields, "latest", "", b, dst)),
            None => {
                let out = name_for(dst, elem, b);
                b.line(&format!("{}: {} = latest {}", out, elem.ir(), s.text));
                Ok(Val { text: out, ty: elem.as_ref().clone(), literal: false })
            }
        }
    }

    /// one push: a tick from the virtual clock unless the ring is
    /// regular; a struct pushed field by field at one tick; a task's
    /// own output sleeps to its next tick after (log 25)
    fn emit_push(&mut self, name: &str, s: &Val, v: &Val, b: &mut Body) {
        self.emit_push_only(s, v, b);
        if let BodyKind::Task { out, hz } = &b.kind {
            if out == name {
                let hz = hz.clone();
                b.line(&format!("__sleep({})", hz));
            }
        }
    }

    fn emit_push_only(&mut self, s: &Val, v: &Val, b: &mut Body) {
        let Ty::Stream(_, regular) = &s.ty else { unreachable!() };
        let tick = if *regular {
            String::new()
        } else {
            let t = b.tmp();
            b.line(&format!("{}: i64 = __now()", t));
            format!("{}, ", t)
        };
        match self.stream_fields(&s.ty) {
            None => b.line(&format!("push {}, {}{}", s.text, tick, v.text)),
            Some(fields) => {
                for (f, t) in &fields {
                    let r = b.tmp();
                    b.line(&format!("{}: {}$ = get {}, {}", r, t.ir(), s.text, f));
                    let x = b.tmp();
                    b.line(&format!("{}: {} = get {}, {}", x, t.ir(), v.text, f));
                    b.line(&format!("push {}, {}{}", r, tick, x));
                }
            }
        }
    }

    /// `x$ << a << b while (c)`: a push per item, the stream's name on
    /// the right reading as its latest item; `while` repeats the last
    /// push for as long as the condition holds of the candidate (log 23)
    fn lower_pushes(&mut self, name: &str, s: &Val, items: &[Expr], cond: Option<&Expr>, b: &mut Body) -> Result<(), Error> {
        let file = b.file.clone();
        let Ty::Stream(elem, regular) = s.ty.clone() else { unreachable!() };
        let elem = *elem;
        let block_ty = Ty::Seq(Box::new(elem.clone()));
        let item = |l: &mut Lowerer, e: &Expr, b: &mut Body| -> Result<Val, Error> {
            let mut v = l.lower_expr(e, Some(&elem), b, None)?;
            if v.literal && fits_literal(&v, &elem) {
                v.ty = elem.clone();
            }
            if v.ty != elem && v.ty != block_ty {
                return Err(lex::error(&b.file, e.line, format!("'{}$' holds {} but the item is {}", name, elem.ir(), v.ty.ir())));
            }
            Ok(v)
        };
        for (i, e) in items.iter().enumerate() {
            let last = i + 1 == items.len();
            // a task call in the chain: the task runs into the stream now
            if let Some((info, args, hz)) = self.task_call(e, Some(&b.vars), &file)? {
                if b.kind == BodyKind::Reset {
                    return Err(lex::error(&file, e.line, "a task call at feature scope before a pushed item: a chain's items come before its tasks"));
                }
                if cond.is_some() && last {
                    return Err(lex::error(&file, e.line, "a task call is not repeated with `while`: the task's own chain says when it stops"));
                }
                self.run_task(&info, &args, hz, s, b, e.line)?;
                continue;
            }
            match cond {
                Some(c) if last => {
                    b.line("loop() {");
                    b.depth += 1;
                    self.push_read = Some((name.to_string(), PushRead::Latest(s.clone(), s.ty.clone())));
                    let v = item(self, e, b);
                    self.push_read = None;
                    let v = b.materialize(&v?);
                    self.push_read = Some((name.to_string(), PushRead::Value(v.clone())));
                    let cv = self.lower_expr(c, Some(&Ty::Bool), b, None);
                    self.push_read = None;
                    let cv = cv?;
                    if cv.ty != Ty::Bool {
                        return Err(lex::error(&file, c.line, "'while' takes a bool"));
                    }
                    let cv = b.materialize(&cv);
                    if v.ty == block_ty {
                        return Err(lex::error(&file, e.line, "a block is pushed once: `while` repeats an item"));
                    }
                    b.line(&format!("if {} {{", cv.text));
                    b.line("} else {");
                    b.depth += 1;
                    b.line("break");
                    b.depth -= 1;
                    b.line("}");
                    self.emit_push(name, s, &v, b);
                    b.line("continue");
                    b.depth -= 1;
                    b.line("}");
                }
                _ => {
                    self.push_read = Some((name.to_string(), PushRead::Latest(s.clone(), s.ty.clone())));
                    let v = item(self, e, b);
                    self.push_read = None;
                    let v = v?;
                    if v.ty == block_ty {
                        // `x$ << block$`: the library's block push on a
                        // regular ring; item by item at the clock's tick
                        // on an irregular one
                        if regular && self.stream_fields(&s.ty).is_none() {
                            b.line(&format!("push {}, {}", s.text, v.text));
                            continue;
                        }
                        let n = b.tmp();
                        b.line(&format!("{}: i64 = len {}", n, v.text));
                        let k = b.tmp();
                        b.line(&format!("loop({}: i64 = 0) {{", k));
                        b.depth += 1;
                        let done = b.tmp();
                        b.line(&format!("{}: u1 = cmp.ge {}, {}", done, k, n));
                        b.line(&format!("if {} {{", done));
                        b.depth += 1;
                        b.line("break");
                        b.depth -= 1;
                        b.line("}");
                        let x = b.tmp();
                        b.line(&format!("{}: {} = load {}, {}", x, elem.ir(), v.text, k));
                        self.emit_push(name, s, &Val { text: x, ty: elem.clone(), literal: false }, b);
                        let k2 = b.tmp();
                        b.line(&format!("{}: i64 = add {}, 1", k2, k));
                        b.line(&format!("continue {}", k2));
                        b.depth -= 1;
                        b.line("}");
                        continue;
                    }
                    self.emit_push(name, s, &v, b);
                }
            }
        }
        Ok(())
    }

    /// after a push or an `end` from a plain function into a stream a
    /// node reads: the scheduler runs (log 25)
    fn trigger(&self, name: &str, b: &mut Body) {
        if b.kind == BodyKind::Fn && self.node_inputs.contains(name) {
            b.line("__run()");
        }
    }

    /// a stream variable takes its moved reader: a new version of a
    /// local, the setter of a feature variable
    fn rebind_stream(&mut self, name: &str, s: &Val, moved: &dyn Fn(&mut Lowerer, &str, &mut Body), b: &mut Body, line: usize) -> Result<(), Error> {
        if b.vars.contains_key(name) {
            b.assignable(name, line)?;
            let ir = b.define(name, s.ty.clone());
            moved(self, &ir, b);
        } else {
            let t = b.tmp();
            moved(self, &t, b);
            b.line(&format!("__set_{}({})", name, t));
        }
        Ok(())
    }

    /// a `for` may not move a local stream: a `loop` carries one
    fn no_moved_stream(&self, body: &[Stmt], b: &Body) -> Result<(), Error> {
        let mut moved = Vec::new();
        moved_streams(body, &mut moved, &|parts| self.task_streams(parts));
        for n in moved {
            if let Some(v) = b.vars.get(&n) {
                if matches!(v.ty, Ty::Stream(..)) {
                    return Err(lex::error(&b.file, stmt_line(&body[0]), format!("'{}$' is moved inside a `for`: move a stream inside a `loop`, which carries it", n)));
                }
            }
        }
        Ok(())
    }

    /// the stream words of section 9 applied to a stream variable —
    /// `peek x$ at (i)`, `latest x$`, `count x$`, `advance x$ by (n)`,
    /// `frame x$`, `ended x$`, `end x$`, `x$ behind (k)`, `x$ at (t)`,
    /// `x$ from (t1) to (t2)` — or None when the phrase is not one
    fn stream_word(&mut self, parts: &[Part], b: &mut Body, dst: Option<&str>, line: usize) -> Result<Option<Val>, Error> {
        let file = b.file.clone();
        let name_of = |p: &Part| -> Option<String> {
            match p {
                Part::Value(Expr { kind: ExprKind::Seq(n), .. }) => Some(n.clone()),
                _ => None,
            }
        };
        let (w, sname, rest, infix) = match parts {
            [Part::Word(w), x, rest @ ..] if name_of(x).is_some_and(|n| self.stream_var(&n, b).is_some()) => (w.clone(), name_of(x).unwrap(), rest, false),
            [x, Part::Word(w), rest @ ..] if name_of(x).is_some_and(|n| self.stream_var(&n, b).is_some()) => (w.clone(), name_of(x).unwrap(), rest, true),
            _ => return Ok(None),
        };
        let one_arg = |p: &Part| -> Option<Expr> {
            match p {
                Part::Args(a) if a.len() == 1 && a[0].name.is_none() => Some(a[0].value.clone()),
                Part::Value(e) => Some(e.clone()),
                _ => None,
            }
        };
        let ty = self.stream_var(&sname, b).unwrap();
        let Ty::Stream(elem, _) = ty.clone() else { unreachable!() };
        let fields = self.stream_fields(&ty);
        let s = self.lower_expr(&Expr { kind: ExprKind::Name(sname.clone()), line }, None, b, None)?;
        let no_struct = |l: &Lowerer, what: &str| -> Result<(), Error> {
            if fields.is_some() {
                let _ = l;
                return Err(lex::error(&file, line, format!("{} on a stream of structs: a sequence of structs is not in this milestone", what)));
            }
            Ok(())
        };
        let none = Val { text: String::new(), ty: Ty::None, literal: false };
        let seq_ty = Ty::Seq(elem.clone());
        match (w.as_str(), infix, rest) {
            ("count", false, []) => {
                let first = self.first_reader(&s, b);
                let n = b.tmp();
                b.line(&format!("{}: i64 = count {}", n, first));
                let ty = Ty::Num("int".into());
                let out = name_for(dst, &ty, b);
                b.line(&format!("{}: int = conv {}", out, n));
                Ok(Some(Val { text: out, ty, literal: false }))
            }
            ("latest", false, []) => Ok(Some(self.latest_of(&s, &ty, b, dst)?)),
            ("peek", false, [Part::Word(at), arg]) if at == "at" => {
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let iv = self.lower_expr(&a, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(iv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, a.line, "'peek' takes an integer index"));
                }
                let i = self.as_i64(&iv, b);
                match fields {
                    Some(f) => Ok(Some(self.read_fields(&s, &f, "peek", &format!(", {}", i), b, dst))),
                    None => {
                        let out = name_for(dst, &elem, b);
                        b.line(&format!("{}: {} = peek {}, {}", out, elem.ir(), s.text, i));
                        Ok(Some(Val { text: out, ty: *elem, literal: false }))
                    }
                }
            }
            ("advance", false, [Part::Word(by), arg]) if by == "by" => {
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let nv = self.lower_expr(&a, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(nv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, a.line, "'advance' takes an integer count"));
                }
                let n = self.as_i64(&nv, b);
                let sty = ty.clone();
                let moved = |l: &mut Lowerer, out: &str, b: &mut Body| match l.stream_fields(&sty) {
                    None => b.line(&format!("{}: {} = advance({}, {})", out, sty.ir(), s.text, n)),
                    Some(fields) => {
                        let mut parts = Vec::new();
                        for (f, t) in &fields {
                            let r = b.tmp();
                            b.line(&format!("{}: {}$ = get {}, {}", r, t.ir(), s.text, f));
                            let r2 = b.tmp();
                            b.line(&format!("{}: {}$ = advance({}, {})", r2, t.ir(), r, n));
                            parts.push(r2);
                        }
                        b.line(&format!("{}: {} = pack {}", out, sty.ir(), parts.join(", ")));
                    }
                };
                self.rebind_stream(&sname, &s, &moved, b, line)?;
                Ok(Some(none))
            }
            ("frame", false, []) => {
                no_struct(self, "'frame'")?;
                let f = name_for(dst, &seq_ty, b);
                let k = b.tmp();
                let sty = ty.clone();
                let ft = f.clone();
                let moved = |_: &mut Lowerer, out: &str, b: &mut Body| b.line(&format!("{}: {}, {}: i64, {}: {} = frame({})", ft, seq_ty.ir(), k, out, sty.ir(), s.text));
                self.rebind_stream(&sname, &s, &moved, b, line)?;
                Ok(Some(Val { text: f, ty: Ty::Seq(elem), literal: false }))
            }
            ("ended", false, []) => {
                let first = self.first_reader(&s, b);
                let out = name_for(dst, &Ty::Bool, b);
                b.line(&format!("{}: u1 = ended({})", out, first));
                Ok(Some(Val { text: out, ty: Ty::Bool, literal: false }))
            }
            ("end", false, []) => {
                match fields {
                    None => b.line(&format!("end({})", s.text)),
                    Some(fields) => {
                        for (f, t) in &fields {
                            let r = b.tmp();
                            b.line(&format!("{}: {}$ = get {}, {}", r, t.ir(), s.text, f));
                            b.line(&format!("end({})", r));
                        }
                    }
                }
                self.trigger(&sname, b);
                Ok(Some(none))
            }
            ("position", false, []) => Err(lex::error(&file, line, "'position' gives the index of the next unread item and its tick: `int i, int t = position x$`")),
            ("behind", true, [arg]) => {
                no_struct(self, "'behind'")?;
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let kv = self.lower_expr(&a, Some(&Ty::Num("int".into())), b, None)?;
                if !matches!(kv.ty, Ty::Num(_)) {
                    return Err(lex::error(&file, a.line, "'behind' takes an integer count"));
                }
                let k = self.as_i64(&kv, b);
                let out = name_for(dst, &seq_ty, b);
                b.line(&format!("{}: {} = behind {}, {}", out, seq_ty.ir(), s.text, k));
                Ok(Some(Val { text: out, ty: seq_ty, literal: false }))
            }
            ("at", true, [arg]) => {
                no_struct(self, "'at'")?;
                let Some(a) = one_arg(arg) else { return Ok(None) };
                let tv = self.lower_expr(&a, Some(&Ty::Num("time".into())), b, None)?;
                if tv.ty != Ty::Num("time".into()) {
                    return Err(lex::error(&file, a.line, "'x$ at (t)' takes a time, `1500 us`, `2 ms`, `1 s`"));
                }
                let out = name_for(dst, &elem, b);
                b.line(&format!("{}: {} = sample {}, {}", out, elem.ir(), s.text, tv.text));
                Ok(Some(Val { text: out, ty: *elem, literal: false }))
            }
            ("from", true, [a1, Part::Word(to), a2]) if to == "to" => {
                no_struct(self, "'from'")?;
                let (Some(e1), Some(e2)) = (one_arg(a1), one_arg(a2)) else { return Ok(None) };
                let time = Ty::Num("time".into());
                let t1 = self.lower_expr(&e1, Some(&time), b, None)?;
                let t2 = self.lower_expr(&e2, Some(&time), b, None)?;
                if t1.ty != time || t2.ty != time {
                    return Err(lex::error(&file, line, "'x$ from (t1) to (t2)' takes two times"));
                }
                let out = name_for(dst, &seq_ty, b);
                b.line(&format!("{}: {} = window {}, {}, {}", out, seq_ty.ir(), s.text, t1.text, t2.text));
                Ok(Some(Val { text: out, ty: seq_ty, literal: false }))
            }
            _ => Ok(None),
        }
    }

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
                // in a push chain the stream's own name is an item (log 23)
                if let Some((n, read)) = self.push_read.clone() {
                    if &n == w {
                        return match read {
                            PushRead::Latest(s, ty) => self.latest_of(&s, &ty, b, dst),
                            PushRead::Value(v) => Ok(v),
                        };
                    }
                }
                let v = self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst)?;
                if v.ty.elem().is_none() && !matches!(v.ty, Ty::Stream(..)) {
                    return Err(lex::error(&file, e.line, format!("'{}$' is not a sequence: '{}' is a {}", w, w, v.ty.ir())));
                }
                Ok(v)
            }
            ExprKind::Unit(inner, u) => {
                // a time literal is the IR's exact `time`; a rate belongs
                // on a stream's declaration
                let f = match u.as_str() {
                    "s" => "seconds",
                    "ms" => "millis",
                    "us" => "micros",
                    "ns" => "nanos",
                    "hz" | "khz" => return Err(lex::error(&file, e.line, "a rate belongs on a stream's declaration: `T x$ at (n hz)`")),
                    _ => return Err(lex::error(&file, e.line, format!("'{}' is not a unit of time here: s, ms, us or ns", u))),
                };
                let ExprKind::Int(n) = inner.kind else {
                    return Err(lex::error(&file, e.line, "a time is a whole number of s, ms, us or ns"));
                };
                let ty = Ty::Num("time".into());
                let out = name_for(dst, &ty, b);
                b.line(&format!("{}: time = {}({})", out, f, n));
                Ok(Val { text: out, ty, literal: false })
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
                None if self.fvar(n).is_some() => self.read_fvar(n, b, dst, e.line),
                None => Err(lex::error(&file, e.line, format!("'{}' is not a variable here", n))),
            },
            ExprKind::Field(base, field) => {
                // `Tristate.yes`: an enumeration's case, qualified;
                // `countdown.enabled`: a feature's switch (log 28)
                if let ExprKind::Phrase(parts) = &base.kind {
                    if let [Part::Word(w)] = parts.as_slice() {
                        if field == "enabled" && self.features.contains(w) {
                            self.reach(&format!("{}.enabled", w), w, &file, e.line)?;
                            return self.read_fvar(&format!("__enabled_{}", w), b, dst, e.line);
                        }
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
                // a lone word: a variable, the feature's `enabled`, or an
                // enumeration's case
                if let [Part::Word(w)] = parts.as_slice() {
                    if b.vars.contains_key(w) || self.fvar(w).is_some() {
                        return self.lower_expr(&Expr { kind: ExprKind::Name(w.clone()), line: e.line }, want, b, dst);
                    }
                    if w == "enabled" {
                        let cur = self.cur.clone();
                        return self.read_fvar(&format!("__enabled_{}", cur), b, dst, e.line);
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
                // the stream words of section 9, on a stream
                if let Some(v) = self.stream_word(parts, b, dst, e.line)? {
                    return Ok(v);
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
                    // `print x$` on a sequence of ints: one line, spaces between
                    if let (true, Some(a)) = (w == "print", arg) {
                        let start = b.out.len();
                        let sv = self.lower_expr(a, None, b, None)?;
                        if sv.ty == Ty::Seq(Box::new(Ty::Num("int".into()))) {
                            b.line(&format!("__print_ints({})", sv.text));
                            return Ok(Val { text: String::new(), ty: Ty::None, literal: false });
                        }
                        b.out.truncate(start);
                    }
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
                if info.task {
                    return Err(lex::error(&file, e.line, format!("'{}' is a task: it is wired into a stream, `{} x$ = {}`", spoken(&info), task_elem(&info), phrase_text(e))));
                }
                self.reach(&spoken(&info), &info.feature, &file, e.line)?;
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
            ExprKind::Existing(parts) => {
                let (call, rtys) = self.existing_call(parts, b, e.line)?;
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
                    _ => Err(lex::error(&file, e.line, "'existing' gives several results; take them with `a, b = existing ...`")),
                }
            }
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
/// the streams a block moves: the names `advance x$ by (n)` and
/// `frame x$` are applied to, anywhere in it, and the stream arguments
/// of a task call (`task` says which, log 25)
fn moved_streams(stmts: &[Stmt], out: &mut Vec<String>, task: &dyn Fn(&[Part]) -> Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Var(v) => match &v.init {
                Some(Init::Value(e)) => moved_in(e, out, task),
                Some(Init::Construct(args)) => args.iter().for_each(|a| moved_in(&a.value, out, task)),
                Some(Init::Pushes { items, cond }) => {
                    items.iter().for_each(|e| moved_in(e, out, task));
                    cond.iter().for_each(|e| moved_in(e, out, task));
                }
                None => {}
            },
            Stmt::Multi { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => moved_in(value, out, task),
            Stmt::If { cond, then, els, .. } => {
                moved_in(cond, out, task);
                moved_streams(then, out, task);
                if let Some(e) = els {
                    moved_streams(e, out, task);
                }
            }
            Stmt::Loop { vars, cond, body, .. } => {
                for v in vars {
                    if let Some(Init::Value(e)) = &v.init {
                        moved_in(e, out, task);
                    }
                }
                cond.iter().for_each(|e| moved_in(e, out, task));
                moved_streams(body, out, task);
            }
            Stmt::For { seq, body, .. } => {
                moved_in(seq, out, task);
                moved_streams(body, out, task);
            }
            Stmt::Continue { values, .. } => values.iter().for_each(|e| moved_in(e, out, task)),
            Stmt::Push { items, cond, .. } => {
                items.iter().for_each(|e| moved_in(e, out, task));
                cond.iter().for_each(|e| moved_in(e, out, task));
            }
            Stmt::Break { .. } => {}
        }
    }
}

fn moved_in(e: &Expr, out: &mut Vec<String>, task: &dyn Fn(&[Part]) -> Vec<String>) {
    let parts_in = |parts: &[Part], out: &mut Vec<String>| {
        if let [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(n), .. }), ..] = parts {
            if (w == "advance" || w == "frame") && !out.contains(n) {
                out.push(n.clone());
            }
        }
        for n in task(parts) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
        for p in parts {
            match p {
                Part::Args(list) => list.iter().for_each(|a| moved_in(&a.value, out, task)),
                Part::Value(e) => moved_in(e, out, task),
                Part::Word(_) => {}
            }
        }
    };
    match &e.kind {
        ExprKind::Phrase(parts) | ExprKind::Existing(parts) => parts_in(parts, out),
        ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) | ExprKind::Range { from: l, to: r, .. } => {
            moved_in(l, out, task);
            moved_in(r, out, task);
        }
        ExprKind::Neg(x) | ExprKind::Unit(x, _) | ExprKind::Field(x, _) => moved_in(x, out, task),
        ExprKind::IfElse(c, a, d) => {
            moved_in(c, out, task);
            moved_in(a, out, task);
            moved_in(d, out, task);
        }
        ExprKind::List(items) => items.iter().for_each(|e| moved_in(e, out, task)),
        _ => {}
    }
}

/// a phrase as text, for a comment in the IR
fn phrase_text(e: &Expr) -> String {
    fn expr(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Float(s) => s.clone(),
            ExprKind::Str(s) => format!("{:?}", s),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Name(n) => n.clone(),
            ExprKind::Seq(n) => format!("{}$", n),
            ExprKind::Unit(x, u) => format!("{} {}", expr(x), u),
            ExprKind::Phrase(parts) => parts
                .iter()
                .map(|p| match p {
                    Part::Word(w) => w.clone(),
                    Part::Args(a) => format!("({})", a.iter().map(|a| expr(&a.value)).collect::<Vec<_>>().join(", ")),
                    Part::Value(v) => expr(v),
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => "...".into(),
        }
    }
    expr(e)
}

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

/// a function's name as spoken: its words
fn spoken(info: &FnInfo) -> String {
    info.parts.iter().filter_map(|p| if let NamePart::Word(w) = p { Some(w.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
}

/// a task's element type, in zero's spelling where it has one
fn task_elem(info: &FnInfo) -> String {
    match &info.results[0].1 {
        Ty::Stream(e, _) => match e.as_ref() {
            Ty::Num(n) if n == "u8" => "uint8".into(),
            Ty::Num(n) => match n.as_str() {
                "i8" | "i16" | "i32" | "i64" => format!("int{}", &n[1..]),
                "u16" | "u32" | "u64" => format!("uint{}", &n[1..]),
                "f32" | "f64" | "f16" => format!("float{}", &n[1..]),
                _ => n.clone(),
            },
            e => e.ir(),
        },
        t => t.ir(),
    }
}

/// may a literal be assigned to a variable of this type?
fn fits_literal(v: &Val, ty: &Ty) -> bool {
    match (&v.ty, ty) {
        (Ty::Num(_), Ty::Num(_)) => true,
        (a, b) => a == b,
    }
}
