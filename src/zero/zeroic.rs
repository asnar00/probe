//! A stream processor read the new way (fm3 question 75, log 124):
//! every line of its body holds for every item that arrives. This
//! file says which bodies are read so (`read`, fm3 question 65), and
//! writes, for each wiring of one, the functions it is made of
//! (`write`), in zero's own tree, as the lowering writes an edge's:
//! a plain function of one item, and, where the input has storage, a
//! sink in the walking form that hands each unread item to it. What
//! lowers any function lowers these, so the zeroic form adds no
//! second set of rules for types, conversions or messages.

use super::lex::{self, Error};
use super::store::Store;
use super::syntax::{Arg, Decl, Expr, ExprKind, FnDecl, Init, NamePart, Param, Part, Stmt, Target, VarDecl};

/// a line that says a stream: `int k$ = kind of (c$)`
#[derive(Clone)]
pub struct Said {
    pub ty: String,
    pub name: String,
    pub value: Expr,
    pub line: usize,
}

/// a push into the processor's own output
#[derive(Clone)]
pub struct Out {
    pub item: Expr,
    pub line: usize,
}

/// a stream a line looks back at, `x$[-1]`: how far back the furthest
/// look goes is how many earlier values the wiring keeps of it
#[derive(Clone)]
pub struct Kept {
    pub name: String,
    pub ty: String,
    pub depth: usize,
    /// the input itself, whose present value is the item; else a said
    /// stream, whose present value the function gives back
    pub input: bool,
}

/// a declaration read the new way
#[derive(Clone)]
pub struct Processor {
    pub line: usize,
    /// the input's name, and its item's type as written
    pub input: String,
    pub item_ty: String,
    pub said: Vec<Said>,
    pub outs: Vec<Out>,
    /// does a line ask `position x$`? Then the wiring counts its items
    pub position: bool,
    /// what is kept (question 75 rule 5): the input first, then the
    /// said streams by name, so the order the lines were written in
    /// does not show in what is emitted
    pub kept: Vec<Kept>,
}

/// the reader's words: a body that applies one to its input walks it
const READER: [&str; 6] = ["peek", "advance", "count", "frame", "latest", "unread"];
const READER_AFTER: [&str; 3] = ["behind", "at", "from"];

fn is_seq(e: &Expr, x: &str) -> bool {
    matches!(&e.kind, ExprKind::Seq(n) if n == x)
}

/// is the part the stream `x`, bare or alone in brackets?
fn names(p: &Part, x: &str) -> bool {
    match p {
        Part::Value(e) => is_seq(e, x),
        Part::Args(a) => a.len() == 1 && a[0].name.is_none() && is_seq(&a[0].value, x),
        Part::Word(_) => false,
    }
}

/// every expression of an expression, itself first; the visitor says
/// whether to go on inside it
fn walk(e: &Expr, f: &mut dyn FnMut(&Expr) -> bool) {
    if !f(e) {
        return;
    }
    match &e.kind {
        ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => walk(x, f),
        ExprKind::List(items) => items.iter().for_each(|x| walk(x, f)),
        ExprKind::Range { from: l, to: r, .. } | ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
            walk(l, f);
            walk(r, f);
        }
        ExprKind::IfElse(c, a, b) => {
            walk(c, f);
            walk(a, f);
            walk(b, f);
        }
        ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
            for p in parts {
                match p {
                    Part::Args(list) => list.iter().for_each(|a| walk(&a.value, f)),
                    Part::Value(x) => walk(x, f),
                    Part::Word(_) => {}
                }
            }
        }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Name(_) | ExprKind::Seq(_) | ExprKind::Acc => {}
    }
}

/// every expression a block holds
fn walk_stmts(stmts: &[Stmt], f: &mut dyn FnMut(&Expr) -> bool) {
    let init = |v: &VarDecl, f: &mut dyn FnMut(&Expr) -> bool| match &v.init {
        Some(Init::Value(e)) => walk(e, f),
        Some(Init::Construct(args)) => args.iter().for_each(|a| walk(&a.value, f)),
        Some(Init::Pushes { items, cond }) => items.iter().chain(cond.iter()).for_each(|e| walk(e, f)),
        None => {}
    };
    for s in stmts {
        match s {
            Stmt::Var(v) => init(v, f),
            Stmt::Multi { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => walk(value, f),
            Stmt::If { cond, then, els, .. } => {
                walk(cond, f);
                walk_stmts(then, f);
                if let Some(e) = els {
                    walk_stmts(e, f);
                }
            }
            Stmt::Loop { vars, cond, body, .. } => {
                vars.iter().for_each(|v| init(v, f));
                cond.iter().for_each(|e| walk(e, f));
                walk_stmts(body, f);
            }
            Stmt::For { seq, body, .. } => {
                walk(seq, f);
                walk_stmts(body, f);
            }
            Stmt::Continue { values, .. } => values.iter().for_each(|e| walk(e, f)),
            Stmt::Break { .. } => {}
            Stmt::Push { target, items, cond, .. } => {
                walk(target, f);
                items.iter().chain(cond.iter()).for_each(|e| walk(e, f));
            }
        }
    }
}

/// the reader's word a phrase applies to `x`, if it applies one
fn reader_word(e: &Expr, x: &str) -> Option<String> {
    let ExprKind::Phrase(parts) = &e.kind else { return None };
    match parts.as_slice() {
        [Part::Word(w), s, ..] if READER.contains(&w.as_str()) && names(s, x) => Some(w.clone()),
        [s, Part::Word(w), ..] if READER_AFTER.contains(&w.as_str()) && names(s, x) => Some(w.clone()),
        [Part::Word(t), Part::Word(of), s] if t == "time" && of == "of" && names(s, x) => Some("time of".into()),
        _ => None,
    }
}

/// the line of a push into the stream `x`, if a block has one
fn pushes_into(stmts: &[Stmt], x: &str) -> Option<usize> {
    stmts.iter().find_map(|s| match s {
        Stmt::Push { target, line, .. } if is_seq(target, x) => Some(*line),
        Stmt::If { then, els, .. } => pushes_into(then, x).or_else(|| els.as_deref().and_then(|e| pushes_into(e, x))),
        _ => None,
    })
}

/// The first mark of the new reading in a block (fm3 question 65), by
/// its line and how it is written: a look back at the input,
/// `x$[-1]`. A body that has one and also walks is written both ways
fn new_mark(stmts: &[Stmt], x: &str) -> Option<(usize, String)> {
    let mut found = None;
    walk_stmts(stmts, &mut |e| {
        if found.is_none() {
            if let ExprKind::Index(base, idx) = &e.kind {
                if let (true, ExprKind::Int(k)) = (is_seq(base, x), &idx.kind) {
                    if *k < 0 {
                        found = Some((e.line, format!("`{}$[{}]`", x, k)));
                    }
                }
            }
        }
        true
    });
    found
}

/// the first thing in a block that walks: a loop, a `for`, an
/// assignment, by its line and what to call it
fn walks(stmts: &[Stmt]) -> Option<(usize, String)> {
    for s in stmts {
        match s {
            Stmt::Loop { line, .. } => return Some((*line, "a loop".into())),
            Stmt::For { line, .. } => return Some((*line, "a `for`".into())),
            Stmt::Assign { line, .. } => return Some((*line, "an assignment".into())),
            Stmt::Continue { line, .. } => return Some((*line, "`continue`".into())),
            Stmt::Break { line } => return Some((*line, "`break`".into())),
            Stmt::If { then, els, .. } => {
                if let Some(w) = walks(then).or_else(|| els.as_deref().and_then(walks)) {
                    return Some(w);
                }
            }
            _ => {}
        }
    }
    None
}

/// The functions of a store that take a stream: each name's key, how
/// many arguments it takes, and which of them are `$` parameters. A
/// body that hands its input to one of these hands the stream on
/// whole, to be walked there
pub type Takers = Vec<(String, Vec<bool>)>;

pub fn takers(store: &Store) -> Takers {
    let mut out = Vec::new();
    for f in store.features.iter().chain(&store.left_out) {
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                let seqs: Vec<bool> = fd.params().map(|p| p.seq).collect();
                if seqs.iter().any(|&s| s) {
                    out.push((key(&fd.name), seqs));
                }
            }
        }
    }
    out
}

/// a name's words, joined as a call's are matched
fn key(parts: &[NamePart]) -> String {
    let words: Vec<&str> = parts.iter().filter_map(|p| if let NamePart::Word(w) = p { Some(w.as_str()) } else { None }).collect();
    words.join("_")
}

/// the function a phrase hands the stream `x` to as a stream, if it
/// names one of the store's that takes a stream there
fn handed_on(e: &Expr, x: &str, takers: &Takers) -> Option<String> {
    let ExprKind::Phrase(parts) = &e.kind else { return None };
    let mut words = Vec::new();
    let mut args: Vec<bool> = Vec::new();
    for p in parts {
        match p {
            Part::Word(w) => words.push(w.as_str()),
            Part::Args(list) => args.extend(list.iter().map(|a| is_seq(&a.value, x))),
            Part::Value(v) => args.push(is_seq(v, x)),
        }
    }
    let k = words.join("_");
    takers.iter().any(|(tk, seqs)| *tk == k && seqs.len() == args.len() && seqs.iter().zip(&args).any(|(s, a)| *s && *a)).then(|| words.join(" "))
}

/// How a declaration's body is read (fm3 question 65). `None` is the
/// reading every task had: it is run over what has arrived, and walks
/// it. A declaration with `<<` and one stream parameter and nothing
/// else is read the new way where its body has no loop, assigns
/// nothing, applies no reader's word to its input, does not push into
/// its input and does not hand it whole to a function that takes a
/// stream; such a body holds lines that say a stream and pushes into
/// its own output, and anything else in it is refused with what to
/// write
pub fn read(fd: &FnDecl, file: &str, takers: &Takers) -> Result<Option<Processor>, Error> {
    if !fd.task || fd.results.len() != 1 || !fd.platform.is_empty() {
        return Ok(None);
    }
    let params: Vec<&Param> = fd.params().collect();
    let streams: Vec<&&Param> = params.iter().filter(|p| p.seq).collect();
    // a task with no input has no items for a line to hold for; one
    // with more than its input keeps the reading it had, and is not
    // built the new way in this hop
    if streams.len() != 1 || params.len() != 1 {
        for s in &streams {
            if let Some((line, how)) = new_mark(&fd.body, &s.name) {
                return Err(lex::error(file, line, format!("{} holds for every item, which is a stream processor with no loop in it; one that takes more than its one input is not built in this hop", how)));
            }
        }
        return Ok(None);
    }
    let input = streams[0];
    let x = input.name.as_str();
    let out = fd.results[0].name.as_str();
    // the marks of the walking form
    let mut old = walks(&fd.body);
    if old.is_none() {
        walk_stmts(&fd.body, &mut |e| {
            if old.is_none() {
                if let Some(w) = reader_word(e, x) {
                    old = Some((e.line, format!("`{}`", w)));
                } else if let Some(f) = handed_on(e, x, takers) {
                    old = Some((e.line, format!("'{}$' handed to '{}', which takes a stream", x, f)));
                }
            }
            true
        });
    }
    if old.is_none() {
        old = pushes_into(&fd.body, x).map(|line| (line, format!("a push into '{}$'", x)));
    }
    if let Some((line, how)) = &old {
        // a body with a mark of each reading is written both ways
        if let Some((nline, nhow)) = new_mark(&fd.body, x) {
            return Err(lex::error(file, nline.max(*line), format!("this body is written both ways: line {} walks its input ({}), and line {} holds for every item ({}). A stream processor either walks what has arrived, with loops and the reader's words, or says each stream once with no loop: write it one way", line, how, nline, nhow)));
        }
        return Ok(None);
    }
    let mut p = Processor { line: fd.line, input: x.to_string(), item_ty: input.ty.clone(), said: Vec::new(), outs: Vec::new(), position: false, kept: Vec::new() };
    for s in &fd.body {
        match s {
            Stmt::Var(v) if v.seq => {
                let Some(Init::Value(e)) = &v.init else {
                    return Err(lex::error(file, v.line, format!("in a stream processor a line says what a stream is, for every item: `{} {}$ = ...`", v.ty, v.name)));
                };
                if v.rate.is_some() {
                    return Err(lex::error(file, v.line, format!("'{}$' is said for each item of '{}$' and has its times: it takes no rate of its own", v.name, x)));
                }
                if v.name == x || v.name == out || p.said.iter().any(|d| d.name == v.name) {
                    return Err(lex::error(file, v.line, format!("'{}$' is said once: a stream processor says each stream in one line", v.name)));
                }
                p.said.push(Said { ty: v.ty.clone(), name: v.name.clone(), value: e.clone(), line: v.line });
            }
            Stmt::Var(v) => return Err(lex::error(file, v.line, format!("in a stream processor every line holds for every item, so each line says a stream: write `{} {}$ = ...`", v.ty, v.name))),
            Stmt::Push { target, items, cond, existing, line } => {
                if *existing {
                    return Err(lex::error(file, *line, "`existing` belongs in a `<<` method, not in a stream processor"));
                }
                if !is_seq(target, out) {
                    return Err(lex::error(file, *line, format!("a stream processor pushes into its own output, '{}$'", out)));
                }
                if cond.is_some() {
                    return Err(lex::error(file, *line, format!("`while` on a push repeats it, which is a loop, and a stream processor has none: say the stream by a line of its own, or write `{}$ << item when (condition)`", out)));
                }
                for e in items {
                    p.outs.push(Out { item: e.clone(), line: *line });
                }
            }
            Stmt::If { line, .. } => return Err(lex::error(file, *line, format!("an `if` round a line of a stream processor: every line holds for every item, so the condition goes on the push, `{}$ << item when (condition)`, or in the value, `if (c) then (a) else (b)`", out))),
            Stmt::Multi { line, .. } => return Err(lex::error(file, *line, "in a stream processor each line says one stream: `int k$ = ...`")),
            Stmt::Expr { line, .. } | Stmt::Check { line, .. } => return Err(lex::error(file, *line, format!("a line of a stream processor says a stream, `int k$ = ...`, or pushes into its output, `{}$ << item`", out))),
            Stmt::Assign { .. } | Stmt::Loop { .. } | Stmt::For { .. } | Stmt::Continue { .. } | Stmt::Break { .. } => unreachable!(),
        }
    }
    // what the lines may say of the input and of each other: the
    // present item by the bare name, an earlier one by `x$[-1]`, and
    // nothing forward (question 75 rule 2)
    let mut bad: Option<Error> = None;
    let said: Vec<String> = p.said.iter().map(|d| d.name.clone()).collect();
    let mut position = false;
    let mut backs: Vec<(String, usize)> = Vec::new();
    let mut check = |e: &Expr| -> bool {
        if bad.is_some() {
            return false;
        }
        match &e.kind {
            ExprKind::Seq(n) if n == out => bad = Some(lex::error(file, e.line, format!("'{}$' is the output: a stream processor pushes into it and does not read it", out))),
            ExprKind::Index(base, idx) => {
                if let ExprKind::Seq(n) = &base.kind {
                    if n == x || said.contains(n) {
                        match &idx.kind {
                            ExprKind::Int(k) if *k < 0 => backs.push((n.clone(), k.unsigned_abs() as usize)),
                            ExprKind::Int(0) => bad = Some(lex::error(file, e.line, format!("'{}$[0]' is the present item: write `{}$`", n, n))),
                            ExprKind::Int(k) => bad = Some(lex::error(file, e.line, format!("'{}$[{}]' would be an item that has not come: a stream processor looks back, `{}$[-1]`, and never forward", n, k, n))),
                            _ => bad = Some(lex::error(file, e.line, format!("the index of '{}$' is worked out: in a stream processor an index is a literal, `{}$[-1]` the item one before; an index that is not a literal is not built in this hop", n, n))),
                        }
                        return false;
                    }
                }
            }
            ExprKind::Phrase(parts) => {
                if let [Part::Word(w), s] = parts.as_slice() {
                    if w == "position" && names(s, x) {
                        position = true;
                        return false;
                    }
                }
            }
            _ => {}
        }
        true
    };
    for d in &p.said {
        walk(&d.value, &mut check);
    }
    for o in &p.outs {
        walk(&o.item, &mut check);
    }
    if let Some(e) = bad {
        return Err(e);
    }
    p.position = position;
    // what is kept: of each stream looked back at, as many earlier
    // values as the furthest look
    let ty_of = |n: &str| if n == x { input.ty.clone() } else { p.said.iter().find(|d| d.name == n).unwrap().ty.clone() };
    let mut kept: Vec<Kept> = Vec::new();
    for (n, k) in backs {
        match kept.iter_mut().find(|c| c.name == n) {
            Some(c) => c.depth = c.depth.max(k),
            None => kept.push(Kept { ty: ty_of(&n), input: n == x, name: n, depth: k }),
        }
    }
    kept.sort_by(|a, b| b.input.cmp(&a.input).then(a.name.cmp(&b.name)));
    p.kept = kept;
    // a line may be said in terms of another's present item, and never
    // of its own: the lines are ordered by what each reads now, by
    // name where nothing orders them, so the order they were written
    // in does not matter and does not show
    let now = |d: &Said| -> Vec<(usize, String)> {
        let mut out = Vec::new();
        walk(&d.value, &mut |e| match &e.kind {
            ExprKind::Index(base, idx) if matches!(&base.kind, ExprKind::Seq(n) if said.contains(n)) && matches!(idx.kind, ExprKind::Int(_)) => false,
            ExprKind::Seq(n) if said.contains(n) => {
                out.push((e.line, n.clone()));
                true
            }
            _ => true,
        });
        out
    };
    let reads: Vec<Vec<(usize, String)>> = p.said.iter().map(now).collect();
    for (d, r) in p.said.iter().zip(&reads) {
        if let Some((line, _)) = r.iter().find(|(_, n)| n == &d.name) {
            return Err(lex::error(file, *line, format!("'{}$' is said in terms of itself at the present item: a stream may look back at itself, `{}$[-1]`, and never at itself now", d.name, d.name)));
        }
    }
    let mut order: Vec<usize> = Vec::new();
    let mut left: Vec<usize> = (0..p.said.len()).collect();
    left.sort_by(|&a, &b| p.said[a].name.cmp(&p.said[b].name));
    while !left.is_empty() {
        let ready = left.iter().position(|&i| reads[i].iter().all(|(_, n)| order.iter().any(|&o| &p.said[o].name == n)));
        let Some(k) = ready else {
            // a circle through present items: name it by the line that
            // comes first in the text
            let &i = left.iter().min_by_key(|&&i| p.said[i].line).unwrap();
            let (line, n) = reads[i].iter().find(|(_, n)| left.iter().any(|&l| &p.said[l].name == n)).unwrap();
            return Err(lex::error(file, *line, format!("'{}$' is said in terms of '{}$' at the present item, and '{}$' in terms of '{}$': a circle. One of them must look back, `{}$[-1]`", p.said[i].name, n, n, p.said[i].name, n)));
        };
        order.push(left.remove(k));
    }
    p.said = order.iter().map(|&i| p.said[i].clone()).collect();
    Ok(Some(p))
}

/// The store without its processors read the new way, and those
/// processors, each with its feature and its file: nothing lowers
/// their bodies as written, and no pass over the store's functions
/// should read them as tasks that walk. `None` where the store has
/// none, which is every store written before hop sixteen
pub fn strip(store: &Store) -> Result<Option<(Store, Vec<(FnDecl, String, String, Processor)>)>, Error> {
    let takers = &takers(store);
    let mut found = Vec::new();
    for f in &store.features {
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                if let Some(p) = read(fd, &f.code.file, takers)? {
                    found.push((fd.clone(), f.name.clone(), f.code.file.clone(), p));
                }
            }
        }
    }
    let mut left = false;
    for f in &store.left_out {
        for d in &f.code.decls {
            if let Decl::Fn(fd) = d {
                left |= read(fd, &f.code.file, takers)?.is_some();
            }
        }
    }
    if found.is_empty() && !left {
        return Ok(None);
    }
    let mut s = store.clone();
    let lines: Vec<(String, usize)> = found.iter().map(|(fd, _, file, _)| (file.clone(), fd.line)).collect();
    for f in s.features.iter_mut() {
        let file = f.code.file.clone();
        f.code.decls.retain(|d| !matches!(d, Decl::Fn(fd) if lines.contains(&(file.clone(), fd.line))));
    }
    for f in s.left_out.iter_mut() {
        let file = f.code.file.clone();
        f.code.decls.retain(|d| !matches!(d, Decl::Fn(fd) if read(fd, &file, takers).map_or(true, |p| p.is_some())));
    }
    Ok(Some((s, found)))
}

fn expr(kind: ExprKind, line: usize) -> Expr {
    Expr { kind, line }
}

fn name(n: &str, line: usize) -> Expr {
    expr(ExprKind::Name(n.to_string()), line)
}

/// the parameter of the function of one item that counts the items
pub const AT: &str = "__at";

/// A processor's own name as its function has it. The function is
/// written into the store beside the store's own names, and its
/// output is one of them, so the item and each said stream take a
/// name no zero program can write: one that starts with `_`
pub fn local(n: &str) -> String {
    if n == "this" { "_this_".to_string() } else { format!("_{}", n) }
}

/// the item `k` before the present one of a stream: a parameter of the function
fn back(n: &str, k: usize) -> String {
    format!("__{}_b{}", n, k)
}

/// the present value of a kept stream: a result of the function
fn result(n: &str) -> String {
    format!("__{}_r", n)
}

impl Processor {
    fn is_stream(&self, n: &str) -> bool {
        n == self.input || self.said.iter().any(|d| d.name == n)
    }

    /// A line's expression as the function of one item reads it: the
    /// input's bare name is the item and a said stream's its value for
    /// this item, both plain names of the function; `position x$` is
    /// the count the wiring keeps. This is the one place a processor's
    /// "of each item" is resolved (`fm3/forms.md` may yet spell it
    /// with square brackets at the call)
    fn each(&self, e: &Expr) -> Expr {
        let line = e.line;
        let kind = match &e.kind {
            ExprKind::Seq(n) if self.is_stream(n) => ExprKind::Name(local(n)),
            ExprKind::Unit(x, u) => ExprKind::Unit(Box::new(self.each(x)), u.clone()),
            ExprKind::Neg(x) => ExprKind::Neg(Box::new(self.each(x))),
            ExprKind::Field(x, f) => ExprKind::Field(Box::new(self.each(x)), f.clone()),
            ExprKind::List(items) => ExprKind::List(items.iter().map(|x| self.each(x)).collect()),
            ExprKind::Range { from, to, inclusive } => ExprKind::Range { from: Box::new(self.each(from)), to: Box::new(self.each(to)), inclusive: *inclusive },
            ExprKind::Bin(op, l, r) => ExprKind::Bin(op.clone(), Box::new(self.each(l)), Box::new(self.each(r))),
            ExprKind::Index(l, r) => match (&l.kind, &r.kind) {
                (ExprKind::Seq(n), ExprKind::Int(k)) if self.is_stream(n) && *k < 0 => ExprKind::Name(back(n, k.unsigned_abs() as usize)),
                _ => ExprKind::Index(Box::new(self.each(l)), Box::new(self.each(r))),
            },
            ExprKind::IfElse(c, a, b) => ExprKind::IfElse(Box::new(self.each(c)), Box::new(self.each(a)), Box::new(self.each(b))),
            ExprKind::Phrase(parts) => {
                if let [Part::Word(w), s] = parts.as_slice() {
                    if w == "position" && names(s, &self.input) {
                        return name(AT, line);
                    }
                }
                ExprKind::Phrase(self.parts(parts))
            }
            ExprKind::Existing(parts) => ExprKind::Existing(self.parts(parts)),
            k => k.clone(),
        };
        expr(kind, line)
    }

    fn parts(&self, parts: &[Part]) -> Vec<Part> {
        parts
            .iter()
            .map(|p| match p {
                Part::Args(list) => Part::Args(list.iter().map(|a| Arg { name: a.name.clone(), value: self.each(&a.value) }).collect()),
                Part::Value(x) => Part::Value(self.each(x)),
                Part::Word(w) => Part::Word(w.clone()),
            })
            .collect()
    }
}

/// what a wiring keeps of one stream: its type, and the context
/// fields of its earlier values, the nearest first
pub struct WKept {
    pub ty: String,
    pub fields: Vec<String>,
    pub input: bool,
}

/// what the front end wrote for one wiring of a processor
pub struct Written {
    /// The function of one item. Its parameters are the item; then the
    /// count of items so far, where a line asks `position`; then, of
    /// each stream a line looks back at, its earlier values, the
    /// nearest first. Its results are the present values of the said
    /// streams among those, in the same order: the wiring moves each
    /// one back a place and keeps it
    pub each: FnDecl,
    /// where the input has storage: the sink that walks it
    pub walker: Option<FnDecl>,
    /// the field that counts the items
    pub at: Option<String>,
    pub kept: Vec<WKept>,
}

impl Written {
    /// the wiring's state, a field of the context each: name and type
    pub fn state(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.at.iter().map(|f| (f.clone(), "index".to_string())).collect();
        for k in &self.kept {
            out.extend(k.fields.iter().map(|f| (f.clone(), k.ty.clone())));
        }
        out
    }
}

fn param(ty: &str, n: &str, seq: bool, line: usize) -> Param {
    Param { ty: ty.to_string(), name: n.to_string(), seq, line }
}

fn call(f: &str, args: Vec<Expr>, line: usize) -> Expr {
    expr(ExprKind::Phrase(vec![Part::Word(f.to_string()), Part::Args(args.into_iter().map(|value| Arg { name: None, value }).collect())]), line)
}

fn assign(to: &str, value: Expr, line: usize) -> Stmt {
    Stmt::Assign { targets: vec![Target { name: to.to_string(), seq: false, line, feature: None }], value, line }
}

/// The functions of wiring `k` of a processor, into the stream `out`.
/// `stored` says the input has storage, so a sink walks it
pub fn write(p: &Processor, k: usize, out: &str, stored: bool) -> Written {
    let line = p.line;
    let each_name = format!("__z{}_each", k);
    let at = p.position.then(|| format!("__z{}_at", k));
    let kept: Vec<WKept> = p.kept.iter().map(|c| WKept { ty: c.ty.clone(), fields: (1..=c.depth).map(|j| format!("__z{}_{}_{}", k, c.name, j)).collect(), input: c.input }).collect();
    let mut params = vec![param(&p.item_ty, &local(&p.input), false, line)];
    if p.position {
        params.push(param("index", AT, false, line));
    }
    let mut results = Vec::new();
    for c in &p.kept {
        for j in 1..=c.depth {
            params.push(param(&c.ty, &back(&c.name, j), false, line));
        }
        if !c.input {
            results.push(param(&c.ty, &result(&c.name), false, line));
        }
    }
    // the lines in their order, each a value of the function; then the
    // pushes; then the present values handed back. A function ends
    // where its last result is given (section 6), so the results are
    // named last, each the line's own value under another name
    let mut body = Vec::new();
    for d in &p.said {
        body.push(Stmt::Var(VarDecl { line: d.line, scope: Vec::new(), ty: d.ty.clone(), name: local(&d.name), seq: false, init: Some(Init::Value(p.each(&d.value))), merge: None, rate: None }));
    }
    for o in &p.outs {
        body.push(Stmt::Push { target: expr(ExprKind::Seq(out.to_string()), o.line), items: vec![p.each(&o.item)], cond: None, existing: false, line: o.line });
    }
    for c in p.kept.iter().filter(|c| !c.input) {
        body.push(assign(&result(&c.name), name(&local(&c.name), line), line));
    }
    let each = FnDecl { line, results, name: vec![NamePart::Word(each_name.clone()), NamePart::Group], groups: vec![params], task: false, body, platform: Vec::new() };
    // the walking form (log 124, decision 7): each unread item handed
    // to the function with what the wiring keeps, each kept stream
    // moved back a place, the count moved on; and the reader moved
    // past what it read, as an edge's sink does (log 75)
    let walker = stored.then(|| {
        let x = p.input.as_str();
        let seq = |n: &str| expr(ExprKind::Seq(n.to_string()), line);
        let mut args = vec![name("__item", line)];
        args.extend(at.iter().map(|f| name(f, line)));
        for c in &kept {
            args.extend(c.fields.iter().map(|f| name(f, line)));
        }
        let given: Vec<Param> = p.kept.iter().filter(|c| !c.input).map(|c| param(&c.ty, &format!("__now_{}", c.name), false, line)).collect();
        let mut inner = Vec::new();
        let made = call(&each_name, args, line);
        match given.as_slice() {
            [] => inner.push(Stmt::Expr { expr: made, line }),
            [one] => inner.push(Stmt::Var(VarDecl { line, scope: Vec::new(), ty: one.ty.clone(), name: one.name.clone(), seq: false, init: Some(Init::Value(made)), merge: None, rate: None })),
            _ => inner.push(Stmt::Multi { vars: given.clone(), value: made, line }),
        }
        for (c, w) in p.kept.iter().zip(&kept) {
            for j in (1..w.fields.len()).rev() {
                inner.push(assign(&w.fields[j], name(&w.fields[j - 1], line), line));
            }
            let present = if c.input { "__item".to_string() } else { format!("__now_{}", c.name) };
            inner.push(assign(&w.fields[0], name(&present, line), line));
        }
        if let Some(f) = &at {
            let next = expr(ExprKind::Bin("+".into(), Box::new(name(f, line)), Box::new(expr(ExprKind::Int(1), line))), line);
            inner.push(assign(f, next, line));
        }
        let count = expr(ExprKind::Phrase(vec![Part::Word("count".into()), Part::Value(seq(x))]), line);
        let advance = expr(ExprKind::Phrase(vec![Part::Word("advance".into()), Part::Value(seq(x)), Part::Word("by".into()), Part::Args(vec![Arg { name: None, value: count }])]), line);
        FnDecl {
            line,
            results: Vec::new(),
            name: vec![NamePart::Word(format!("__z{}", k)), NamePart::Group],
            groups: vec![vec![param(&p.item_ty, x, true, line)]],
            task: false,
            body: vec![Stmt::For { var: "__item".into(), seq: seq(x), body: inner, line }, Stmt::Expr { expr: advance, line }],
            platform: Vec::new(),
        }
    });
    Written { each, walker, at, kept }
}
