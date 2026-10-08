//! The two kinds of sequence (fm3 question 90): an array, `int a[]`,
//! whose items are all there, and a stream, `int x$`, whose items
//! arrive. This file is the one walk over a parsed store that knows
//! what every name is where it is written: feature scope, a function's
//! parameters and results, a local from its declaration to the end of
//! its block, as the lowering scopes them. `probe zero names <store>`
//! prints what it found, every name declared with a mark, how it was
//! declared and each form it is used in (fm3 log 158): the census the
//! respelling was made from.

use super::lex::{self, Error};
use super::store::Store;
use super::syntax::{Decl, Expr, ExprKind, FnDecl, Init, LoopInto, NamePart, Part, Stmt, VarDecl};
use std::collections::HashMap;

/// where a name is declared
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Feature,
    Local,
    /// a line of a stream processor, `int k$ = kind of (c$)`
    Said,
    /// a parameter or a result of a function, with what kind of
    /// function it is: "a function", "a task", "a `<<` method"
    Param(&'static str),
    Result(&'static str),
}

/// one name declared with a mark, and every use of it
#[derive(Clone, Debug)]
pub struct Named {
    pub file: String,
    pub line: usize,
    pub ty: String,
    pub name: String,
    pub place: Place,
    /// how it was declared: "bare", "first items", "at a rate",
    /// "wired to a task", "given a list" and the like
    pub how: String,
    /// the file, the line, the form the name stands in, and what the
    /// line it stands on wants
    pub uses: Vec<(String, usize, String, &'static str)>,
    /// the marked names its declaration's value is made from, `int j$
    /// = i$ * 2`: a function of each item of a stream is a stream
    pub from: Vec<usize>,
}

/// the words the lowering applies to a stream by its name (`stream_word`)
const WORDS: [&str; 7] = ["count", "latest", "frame", "ended", "end", "position", "empty"];

struct Walk {
    names: Vec<Named>,
    /// the scopes, innermost last: a name, and its entry in `names`
    /// where it was declared with a mark
    scopes: Vec<HashMap<String, Option<usize>>>,
    file: String,
    /// the tasks of the store by their words, for a wiring
    tasks: Vec<String>,
    takers: super::zeroic::Takers,
    /// the functions a feature wires as sinks, `write(out$)`, by
    /// their words: what such a function is handed is a stream
    sinks: Vec<String>,
    /// in a stream processor's body, where every line says a stream
    processor: bool,
}

impl Walk {
    fn declare(&mut self, name: &str, seq: bool, ty: &str, line: usize, place: Place, how: String) {
        self.declare_from(name, seq, ty, line, place, how, Vec::new())
    }

    fn declare_from(&mut self, name: &str, seq: bool, ty: &str, line: usize, place: Place, how: String, from: Vec<usize>) {
        let at = if seq {
            self.names.push(Named { file: self.file.clone(), line, ty: ty.to_string(), name: name.to_string(), place, how, uses: Vec::new(), from });
            Some(self.names.len() - 1)
        } else {
            None
        };
        self.scopes.last_mut().unwrap().insert(name.to_string(), at);
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| s.get(name)).copied().flatten()
    }

    fn used(&mut self, name: &str, line: usize, form: String, wants: &'static str) {
        if let Some(i) = self.find(name) {
            let file = self.file.clone();
            self.names[i].uses.push((file, line, form, wants));
        }
    }

    /// what kind of value a declaration's `=` gives
    fn given(&self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::List(_) => "given a list".into(),
            ExprKind::Range { .. } => "given a range".into(),
            ExprKind::Str(_) => "given a text".into(),
            ExprKind::Seq(n) => format!("given `{}$`", n),
            ExprKind::Bin(op, ..) => format!("given an operator's sequence, `{}`", op),
            ExprKind::Phrase(parts) => match parts.as_slice() {
                [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(_), .. })] if w == "frame" => "given a `frame`".into(),
                [Part::Value(Expr { kind: ExprKind::Seq(_), .. }), Part::Word(w), ..] if w == "behind" || w == "from" => format!("given `{}`", w),
                _ => {
                    let words = spoken(parts);
                    if self.is_task(&words) {
                        format!("wired to the task `{}`", words)
                    } else {
                        format!("given what `{}` gives", words)
                    }
                }
            },
            _ => "given a value".into(),
        }
    }

    fn var(&mut self, v: &VarDecl, place: Place) {
        let how = match (&v.init, &v.rate) {
            (Some(Init::Value(e)), _) => self.given(e),
            (Some(Init::Pushes { .. }), Some(_)) => "at a rate, first items".into(),
            (Some(Init::Pushes { .. }), None) => "first items".into(),
            (Some(Init::Construct(_)), _) => "constructed".into(),
            (None, Some(_)) => "at a rate".into(),
            (None, None) => "bare".into(),
        };
        // the value is worked out before the name exists, but a
        // stream's first items may read the stream itself
        let wants = if v.seq { "a sequence" } else { "one value" };
        match &v.init {
            Some(Init::Value(e)) => {
                self.expr(e, wants, "the value");
                let mut named = Vec::new();
                // what a word makes of a stream that is over, a `frame`,
                // `behind`, a window, is all there; what a function
                // gives is its own to say
                if !matches!(e.kind, ExprKind::Phrase(_)) {
                    super::syntax::seqs_in(e, &mut named);
                }
                let from = named.iter().filter_map(|n| self.find(n)).collect();
                self.declare_from(&v.name, v.seq, &v.ty, v.line, place, how, from);
            }
            Some(Init::Construct(args)) => {
                for a in args {
                    self.expr(&a.value, "one value", "an argument of a constructor");
                }
                self.declare(&v.name, v.seq, &v.ty, v.line, place, how);
            }
            Some(Init::Pushes { items, cond, .. }) => {
                self.declare(&v.name, v.seq, &v.ty, v.line, place, how);
                for e in items {
                    self.expr(e, "an item of a push", "the item");
                }
                if let Some(c) = cond {
                    self.expr(c, "one value", "the value");
                }
            }
            None => self.declare(&v.name, v.seq, &v.ty, v.line, place, how),
        }
        if let Some(r) = &v.rate {
            self.expr(r, "one value", "the value");
        }
    }

    /// `form` is how the expression itself stands in what is round it,
    /// and matters only where the expression is a name
    fn expr(&mut self, e: &Expr, wants: &'static str, form: &str) {
        match &e.kind {
            ExprKind::Seq(n) => self.used(n, e.line, form.to_string(), wants),
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Name(_) | ExprKind::Acc => {}
            ExprKind::Unit(x, _) => self.expr(x, wants, "the value"),
            ExprKind::Neg(x) => self.expr(x, wants, "an operand of `-`"),
            ExprKind::Field(x, _) => self.expr(x, "one value", "the base of a field"),
            ExprKind::List(items) => items.iter().for_each(|x| self.expr(x, "one value", "an item of a list")),
            ExprKind::Range { from, to, .. } => {
                self.expr(from, "one value", "a bound of a range");
                self.expr(to, "one value", "a bound of a range");
            }
            ExprKind::Bin(op, l, r) => {
                let acc = matches!(l.kind, ExprKind::Acc) || matches!(r.kind, ExprKind::Acc);
                let form = if acc { format!("reduced, `{} _`", op) } else { format!("an operand of `{}`", op) };
                self.expr(l, wants, &form);
                self.expr(r, wants, &form);
            }
            ExprKind::IfElse(c, a, b) => {
                self.expr(c, "one value", "the value");
                self.expr(a, wants, "an arm of `if then else`");
                self.expr(b, wants, "an arm of `if then else`");
            }
            ExprKind::Index(base, i) => {
                let form = match i.kind {
                    ExprKind::Int(k) if k < 0 => "a look back, `[-k]`",
                    ExprKind::Int(_) => "an item by its place, `[k]`",
                    _ => "an item by its place, `[e]`",
                };
                self.expr(base, wants, form);
                self.expr(i, "one value", "the value");
            }
            ExprKind::Phrase(parts) | ExprKind::Existing(parts) => self.phrase(parts, e.line, wants),
        }
    }

    fn phrase(&mut self, parts: &[Part], line: usize, wants: &'static str) {
        let seq = |p: &Part| -> Option<String> {
            match p {
                Part::Value(Expr { kind: ExprKind::Seq(n), .. }) => Some(n.clone()),
                Part::Args(a) if a.len() == 1 && a[0].name.is_none() => match &a[0].value.kind {
                    ExprKind::Seq(n) => Some(n.clone()),
                    _ => None,
                },
                _ => None,
            }
        };
        // a stream's words, as `stream_word` reads them
        let word: Option<(String, String, &[Part])> = match parts {
            [Part::Word(t), Part::Word(of), x] if t == "time" && of == "of" => seq(x).map(|n| ("time of".to_string(), n, &parts[3..])),
            [Part::Word(w), x] if WORDS.contains(&w.as_str()) => seq(x).map(|n| (w.clone(), n, &parts[2..])),
            [Part::Word(w), x, Part::Word(k), _] if (w == "peek" && k == "at") || (w == "advance" && k == "by") => seq(x).map(|n| (w.clone(), n, &parts[3..])),
            [x @ Part::Value(_), Part::Word(w), _] if w == "behind" || w == "at" => seq(x).map(|n| (w.clone(), n, &parts[2..])),
            [x @ Part::Value(_), Part::Word(w), _, Part::Word(to), _] if w == "from" && to == "to" => seq(x).map(|n| ("from ... to".to_string(), n, &parts[2..])),
            _ => None,
        };
        if let Some((w, n, rest)) = word {
            if self.find(&n).is_some() {
                self.used(&n, line, format!("the word `{}`", w), wants);
                for p in rest {
                    match p {
                        Part::Args(a) => a.iter().for_each(|a| self.expr(&a.value, "one value", "the value")),
                        Part::Value(x) => self.expr(x, "one value", "the value"),
                        Part::Word(_) => {}
                    }
                }
                return;
            }
        }
        let words = spoken(parts);
        let acc = parts.iter().any(|p| match p {
            Part::Args(a) => a.iter().any(|a| matches!(a.value.kind, ExprKind::Acc)),
            Part::Value(x) => matches!(x.kind, ExprKind::Acc),
            Part::Word(_) => false,
        });
        let task = self.is_task(&words);
        let form = if task {
            format!("handed to the task `{}`", words)
        } else if acc {
            format!("reduced by `{}` with `_`", words)
        } else {
            format!("an argument of `{}`", words)
        };
        for p in parts {
            match p {
                Part::Args(a) => a.iter().for_each(|a| self.expr(&a.value, wants, &form)),
                Part::Value(x) => self.expr(x, wants, &form),
                Part::Word(_) => {}
            }
        }
    }

    fn block(&mut self, stmts: &[Stmt]) {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Var(v) => {
                let place = if self.processor { Place::Said } else { Place::Local };
                self.var(v, place)
            }
            Stmt::Multi { vars, value, .. } => {
                self.expr(value, "one value", "the value");
                for p in vars {
                    self.declare(&p.name, p.seq, &p.ty, p.line, Place::Local, "one of several from a call".into());
                }
            }
            Stmt::Assign { targets, value, line } => {
                let wants = if targets.iter().any(|t| t.seq) { "a sequence" } else { "one value" };
                self.expr(value, wants, "the value");
                for t in targets.iter().filter(|t| t.seq) {
                    self.used(&t.name, *line, "given by `=`".into(), "a sequence");
                }
            }
            Stmt::If { cond, then, els, .. } => {
                self.expr(cond, "one value", "the value");
                self.block(then);
                if let Some(e) = els {
                    self.block(e);
                }
            }
            Stmt::Loop { vars, cond, body, into, .. } => {
                self.scopes.push(HashMap::new());
                for v in vars {
                    self.var(v, Place::Local);
                }
                if let Some(c) = cond {
                    self.expr(c, "one value", "the value");
                }
                self.block(body);
                self.scopes.pop();
                if let Some(LoopInto::Declare(ps)) = into {
                    for p in ps {
                        self.declare(&p.name, p.seq, &p.ty, p.line, Place::Local, "what a loop gives".into());
                    }
                }
            }
            Stmt::For { var, seq, body, line } => {
                self.expr(seq, "a sequence", "walked by `for`");
                self.scopes.push(HashMap::new());
                self.declare(var, false, "", *line, Place::Local, String::new());
                self.block(body);
                self.scopes.pop();
            }
            Stmt::Continue { values, .. } | Stmt::Break { values, .. } => values.iter().for_each(|v| self.expr(v, "one value", "the value")),
            Stmt::Check { cond, .. } => self.expr(cond, "one value", "the value"),
            Stmt::Push { target, items, cond, forever, line, .. } => {
                self.push(target, items, cond.as_ref(), *forever, *line);
            }
            Stmt::Expr { expr, .. } => self.expr(expr, "nothing", "the value"),
        }
    }

    fn push(&mut self, target: &Expr, items: &[Expr], cond: Option<&Expr>, stands: bool, line: usize) {
        match &target.kind {
            ExprKind::Seq(n) => self.used(n, line, "pushed into".into(), "a sequence"),
            _ => self.expr(target, "a sequence", "pushed into"),
        }
        let wants = if stands { "an item of a push that stands" } else { "an item of a push" };
        for e in items {
            self.expr(e, wants, "the item");
        }
        if let Some(c) = cond {
            self.expr(c, "one value", "the value");
        }
    }

    /// a call's words name a task, with or without its rate, `ticks at`
    fn is_task(&self, words: &str) -> bool {
        self.tasks.iter().any(|t| t == words || words.strip_suffix(" at") == Some(t.as_str()))
    }

    fn function(&mut self, f: &FnDecl) {
        let method = matches!(f.name.as_slice(), [NamePart::Group, NamePart::Sym(s)] if s == "<<");
        let processor = f.task && matches!(super::zeroic::read(f, &self.file, &self.takers), Ok(Some(_)));
        self.processor = processor;
        let kind = if processor {
            "a stream processor"
        } else if f.task && f.params().any(|p| p.seq) {
            "a task that walks"
        } else if f.task {
            "a task"
        } else if method {
            "a `<<` method"
        } else if self.sinks.contains(&f.name.iter().filter_map(|p| if let NamePart::Word(x) = p { Some(x.as_str()) } else { None }).collect::<Vec<_>>().join(" ")) {
            "a sink"
        } else {
            "a function"
        };
        self.scopes.push(HashMap::new());
        for r in &f.results {
            self.declare(&r.name, r.seq, &r.ty, f.line, Place::Result(kind), String::new());
        }
        for (g, group) in f.groups.iter().enumerate() {
            for p in group {
                let how = if method && g == 0 { "the stream pushed into" } else { "" };
                self.declare(&p.name, p.seq, &p.ty, f.line, Place::Param(kind), how.to_string());
            }
        }
        self.block(&f.body);
        self.scopes.pop();
        self.processor = false;
    }
}

/// the words of a phrase or of a declared name, as a reader says them
fn spoken(parts: &[Part]) -> String {
    parts.iter().filter_map(|p| if let Part::Word(w) = p { Some(w.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
}

/// every name of a store declared with a mark, with its uses
pub fn names(store: &Store) -> Vec<Named> {
    let mut w = Walk { names: Vec::new(), scopes: vec![HashMap::new()], file: String::new(), tasks: Vec::new(), takers: super::zeroic::takers(store), sinks: Vec::new(), processor: false };
    let features: Vec<_> = store.features.iter().chain(store.left_out.iter()).collect();
    for f in &features {
        for d in &f.code.decls {
            if let Decl::Wire(Expr { kind: ExprKind::Phrase(parts), .. }) = d {
                w.sinks.push(spoken(parts));
            }
            if let Decl::Fn(fd) = d {
                if fd.task {
                    w.tasks.push(fd.name.iter().filter_map(|p| if let NamePart::Word(x) = p { Some(x.as_str()) } else { None }).collect::<Vec<_>>().join(" "));
                }
            }
        }
    }
    // feature scope is the store's: every feature's variables first
    for f in &features {
        w.file = f.code.file.clone();
        for d in &f.code.decls {
            if let Decl::Var(v) = d {
                w.var(v, Place::Feature);
            }
        }
    }
    for f in &features {
        w.file = f.code.file.clone();
        for d in &f.code.decls {
            match d {
                Decl::Fn(fd) => w.function(fd),
                Decl::Wire(e) => w.expr(e, "a sink", "the value"),
                Decl::Edge { target, items, cond, only, forever, line, .. } => {
                    // a `<<` at feature scope with a stream on its right
                    // stands, by `forever`, a count or an `until`
                    let _ = forever;
                    w.push(target, items, cond.as_ref(), true, *line);
                    if let Some(c) = only {
                        w.expr(c, "one value", "the value");
                    }
                }
                Decl::Var(_) | Decl::Type(_) => {}
            }
        }
        w.file = f.md_file.clone();
        for c in &f.cases {
            w.expr(&c.call, "a case", "the value");
        }
    }
    w.names
}

/// The sort (fm3 log 158): what a name's declaration says it is, what
/// its uses say, and the kind that follows, "array", "stream" or
/// "cannot tell". `kinds` is the sort of the names before this one,
/// which a value made from them follows
pub fn sort(n: &Named, kinds: &[&'static str]) -> (&'static str, Vec<String>, Vec<String>) {
    let mut stream: Vec<String> = Vec::new();
    let mut array: Vec<String> = Vec::new();
    let given = n.how.starts_with("given");
    let cell_only = matches!(n.ty.as_str(), "string" | "bool");
    let tasks = |k: &str| k.contains("task") || k.contains("processor") || k.contains("sink");
    match &n.place {
        Place::Param(k) | Place::Result(k) if tasks(k) => stream.push(format!("{}'s", k)),
        Place::Said => stream.push("a line of a stream processor".into()),
        Place::Param(_) if n.how == "the stream pushed into" => stream.push("a `<<` method's stream".into()),
        Place::Param(k) if *k == "a `<<` method" => array.push("a `<<` method's block".into()),
        Place::Param(_) | Place::Result(_) => {}
        _ if n.how.starts_with("given what") || n.how == "given a value" => {}
        _ if given && n.from.iter().any(|&i| kinds.get(i) == Some(&"stream")) => stream.push(format!("{}, of a stream", n.how)),
        _ if given => array.push(n.how.clone()),
        _ => stream.push(format!("declared {}", n.how)),
    }
    let all = matches!(n.place, Place::Said) || matches!(&n.place, Place::Param(k) | Place::Result(k) if tasks(k));
    for (_, line, form, wants) in &n.uses {
        let at = |s: &str| format!("{} ({})", s, line);
        // a stream's: it is pushed into, or a word that moves a
        // reader or asks about time or the end is applied to it
        if form == "pushed into" || form.starts_with("a look back") || form.starts_with("handed to the task") || *wants == "a sink" || *wants == "an item of a push that stands" {
            stream.push(at(form));
        } else if let Some(w) = form.strip_prefix("the word `") {
            if w != "count`" {
                stream.push(at(form));
            }
        // an array's: an item by its place, walked, reduced, or the
        // whole of it handed on by its name where the line happens once
        } else if form.starts_with("an item by its place") || form == "walked by `for`" || form.starts_with("reduced") || form == "given by `=`" {
            array.push(at(form));
        } else if (form == "the item" || form == "the value") && matches!(*wants, "a sequence" | "an item of a push") && !all && !cell_only {
            array.push(at(&format!("whole, as {}", wants)));
        }
    }
    let kind = match (stream.is_empty(), array.is_empty()) {
        (true, _) => "array",
        (false, true) => "stream",
        (false, false) => "cannot tell",
    };
    (kind, stream, array)
}

/// `probe zero names <store>`: the print
pub fn report(dir: &std::path::Path) -> Result<String, Error> {
    let store = super::store::read(dir).map_err(|e| lex::error(&e.file, e.line, e.msg))?;
    let mut out = String::new();
    let mut kinds: Vec<&'static str> = Vec::new();
    for n in names(&store) {
        let (kind, stream, array) = sort(&n, &kinds);
        kinds.push(kind);
        let place = match &n.place {
            Place::Feature => "feature scope".to_string(),
            Place::Local => "local".to_string(),
            Place::Said => "said in a stream processor".to_string(),
            Place::Param(k) => format!("parameter of {}", k),
            Place::Result(k) => format!("result of {}", k),
        };
        let lines: Vec<String> = {
            let mut l: Vec<String> = n.uses.iter().map(|u| format!("{}:{}", if u.0 == n.file { "" } else { u.0.as_str() }, u.1)).collect();
            l.dedup();
            l
        };
        out.push_str(&format!("{}\t{}:{}\t{} {}$\t{}{}{}\tstream: {}\tarray: {}\tat: {}\n", kind, n.file, n.line, n.ty, n.name, place, if n.how.is_empty() { "" } else { ", " }, n.how, stream.join("; "), array.join("; "), lines.join(" ")));
    }
    Ok(out)
}
