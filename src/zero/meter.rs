//! The meter (fm3 log 129): which lines of a store use a non-zeroic
//! form. zero is in two parts, the zeroic and the non-zeroic, and
//! removing the second is an aspiration (fm3 zero.md section 1): a
//! non-zeroic form is not refused, it is noticed. This reads a store to
//! its tree and no further, counts the lines that use a form on the
//! list `fm3/touchstones.md` keeps under "not yet zeroic", and lists
//! each with its file, its line and the form. It lowers nothing, so it
//! refuses nothing and changes no program's meaning or cost.

use super::store::{self, Store};
use super::syntax::{Decl, Expr, ExprKind, FnDecl, Init, LoopInto, Part, Stmt, VarDecl};
use super::zeroic;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

/// the non-zeroic forms, in the table's order: what each is called in
/// a listing, and the rule that finds it in the text
pub const FORMS: [(&str, &str); 9] = [
    ("a name assigned again", "an assignment to a feature-scope variable, a local, a parameter, or a result already given"),
    ("a loop's variable assigned in its body", "an assignment to a name the header of an enclosing `loop` declares"),
    ("a task that walks its input", "in a declaration with `<<`, `peek`, `advance` or `count` applied to one of its `$` parameters"),
    ("an index or a `peek` forward of now", "`peek x$ at (e)` where `e` is not the literal 0, and `x$[e]` on a task's own input where `e` is not a negative literal"),
    ("an `if` statement with a push under it, in a stream processor", "in a declaration with `<<`, a line that begins `if` with a push anywhere in its block; the zeroic form is the condition on the push's own line, `x$ << item if (c)`"),
    ("`ended` asked inside a loop", "`ended x$` on a line inside a `loop` or a `for`"),
    ("a loop that only computes", "a `loop` no line of which pushes into a stream, ends one, or applies a stream word to one; a call in it may push, unseen"),
    ("`for` over a sequence", "every `for`"),
    ("an index into a sequence that may be too short", "`x$[e]` anywhere it is not forward of a task's now, and not a look back, `x$[-1]`, in a stream processor with no loop"),
];

/// a line that uses a form: the file, the line, which form
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Found {
    pub file: String,
    pub line: usize,
    pub form: usize,
}

/// a store, metered
pub struct Metered {
    pub name: String,
    /// its lines of zero: the lines of its `.zero` files that are not blank
    pub lines: usize,
    pub found: Vec<Found>,
}

impl Metered {
    /// how many lines use a non-zeroic form: a line that uses two counts once
    pub fn count(&self) -> usize {
        self.found.iter().map(|f| (&f.file, f.line)).collect::<BTreeSet<_>>().len()
    }
}

const STREAM_WORDS: [&str; 7] = ["peek", "advance", "count", "frame", "latest", "ended", "position"];

/// what the walk knows of where it stands
struct At<'a> {
    file: &'a str,
    /// the `$` parameters of the declaration, where it has a `<<`
    inputs: Vec<String>,
    task: bool,
    /// a processor read the new way: a look back in it is zeroic
    zeroic: bool,
    results: Vec<String>,
}

struct Walk {
    found: BTreeSet<Found>,
}

fn seq_of(p: &Part) -> Option<&str> {
    match p {
        Part::Value(Expr { kind: ExprKind::Seq(n), .. }) => Some(n),
        Part::Args(a) if a.len() == 1 && a[0].name.is_none() => match &a[0].value.kind {
            ExprKind::Seq(n) => Some(n),
            _ => None,
        },
        _ => None,
    }
}

fn literal(p: &Part) -> Option<i64> {
    let e = match p {
        Part::Value(e) => e,
        Part::Args(a) if a.len() == 1 => &a[0].value,
        _ => return None,
    };
    match e.kind {
        ExprKind::Int(v) => Some(v),
        _ => None,
    }
}

impl Walk {
    fn note(&mut self, at: &At, line: usize, form: usize) {
        self.found.insert(Found { file: at.file.to_string(), line, form });
    }

    /// the forms an expression uses; `in_loop` says a loop encloses it
    fn expr(&mut self, e: &Expr, at: &At, in_loop: bool) {
        match &e.kind {
            ExprKind::Index(base, idx) => {
                if let ExprKind::Seq(n) = &base.kind {
                    let back = matches!(idx.kind, ExprKind::Int(k) if k < 0);
                    if at.task && !at.zeroic && at.inputs.contains(n) {
                        if !back {
                            self.note(at, e.line, 3);
                        }
                    } else if !(at.zeroic && back) {
                        self.note(at, e.line, 8);
                    }
                } else {
                    self.note(at, e.line, 8);
                }
                self.expr(base, at, in_loop);
                self.expr(idx, at, in_loop);
            }
            ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
                if let [Part::Word(w), s, rest @ ..] = parts.as_slice() {
                    if let Some(n) = seq_of(s) {
                        if at.task && at.inputs.iter().any(|i| i == n) && matches!(w.as_str(), "peek" | "advance" | "count") {
                            self.note(at, e.line, 2);
                        }
                        if w == "peek" && !matches!(rest, [Part::Word(a), i] if a == "at" && literal(i) == Some(0)) {
                            self.note(at, e.line, 3);
                        }
                        if w == "ended" && in_loop {
                            self.note(at, e.line, 5);
                        }
                    }
                }
                for p in parts {
                    match p {
                        Part::Args(list) => list.iter().for_each(|a| self.expr(&a.value, at, in_loop)),
                        Part::Value(x) => self.expr(x, at, in_loop),
                        Part::Word(_) => {}
                    }
                }
            }
            ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => self.expr(x, at, in_loop),
            ExprKind::List(items) => items.iter().for_each(|x| self.expr(x, at, in_loop)),
            ExprKind::Range { from: l, to: r, .. } | ExprKind::Bin(_, l, r) => {
                self.expr(l, at, in_loop);
                self.expr(r, at, in_loop);
            }
            ExprKind::IfElse(c, a, b) => {
                self.expr(c, at, in_loop);
                self.expr(a, at, in_loop);
                self.expr(b, at, in_loop);
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Name(_) | ExprKind::Seq(_) | ExprKind::Acc => {}
        }
    }

    fn init(&mut self, v: &VarDecl, at: &At, in_loop: bool) {
        match &v.init {
            Some(Init::Value(e)) => self.expr(e, at, in_loop),
            Some(Init::Construct(args)) => args.iter().for_each(|a| self.expr(&a.value, at, in_loop)),
            Some(Init::Pushes { items, cond, .. }) => items.iter().chain(cond.iter()).for_each(|e| self.expr(e, at, in_loop)),
            None => {}
        }
    }

    /// an assignment's target: a loop's own variable, or a name
    /// assigned again; a result's first assignment is neither
    fn assigned(&mut self, name: &str, line: usize, at: &At, carried: &[String], given: &mut Vec<String>) {
        if carried.iter().any(|c| c == name) {
            self.note(at, line, 1);
        } else if !at.results.iter().any(|r| r == name) || given.iter().any(|g| g == name) {
            self.note(at, line, 0);
        }
        if !given.iter().any(|g| g == name) {
            given.push(name.to_string());
        }
    }

    fn block(&mut self, stmts: &[Stmt], at: &At, carried: &[String], in_loop: bool, given: &mut Vec<String>) {
        for s in stmts {
            match s {
                Stmt::Var(v) => self.init(v, at, in_loop),
                Stmt::Multi { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => self.expr(value, at, in_loop),
                Stmt::Assign { targets, value, line } => {
                    self.expr(value, at, in_loop);
                    for t in targets {
                        self.assigned(&t.name, *line, at, carried, given);
                    }
                }
                Stmt::If { cond, then, els, line, on_push } => {
                    self.expr(cond, at, in_loop);
                    if at.task && !*on_push && (pushes(then) || els.as_deref().is_some_and(pushes)) {
                        self.note(at, *line, 4);
                    }
                    // what either arm gives is given after the `if`
                    let (mut a, mut b) = (given.clone(), given.clone());
                    self.block(then, at, carried, in_loop, &mut a);
                    if let Some(e) = els {
                        self.block(e, at, carried, in_loop, &mut b);
                    }
                    // ... but an arm that gives the function's last
                    // result has ended it (fm3 question 2, log 145), and
                    // gives nothing to what follows the `if`
                    let ends = |g: &[String]| !at.results.is_empty() && at.results.iter().all(|r| g.contains(r));
                    let (a, b) = (if ends(&a) { Vec::new() } else { a }, if els.is_some() && ends(&b) { Vec::new() } else { b });
                    for n in a.into_iter().chain(b) {
                        if !given.contains(&n) {
                            given.push(n);
                        }
                    }
                }
                Stmt::Loop { vars, cond, body, into, line, .. } => {
                    let mut inner = carried.to_vec();
                    for v in vars {
                        self.init(v, at, in_loop);
                        inner.push(v.name.clone());
                    }
                    if let Some(c) = cond {
                        self.expr(c, at, true);
                    }
                    if !touches_a_stream(body) && cond.as_ref().is_none_or(|c| !expr_touches(c)) {
                        self.note(at, *line, 6);
                    }
                    self.block(body, at, &inner, true, given);
                    if let Some(LoopInto::Assign(ts)) = into {
                        for t in ts {
                            self.assigned(&t.name, *line, at, carried, given);
                        }
                    }
                }
                Stmt::For { seq, body, line, .. } => {
                    self.note(at, *line, 7);
                    self.expr(seq, at, in_loop);
                    self.block(body, at, carried, true, given);
                }
                Stmt::Continue { values, .. } | Stmt::Break { values, .. } => values.iter().for_each(|e| self.expr(e, at, in_loop)),
                Stmt::Push { target, items, cond, .. } => {
                    self.expr(target, at, in_loop);
                    items.iter().chain(cond.iter()).for_each(|e| self.expr(e, at, in_loop));
                }
            }
        }
    }

    fn function(&mut self, fd: &FnDecl, file: &str, zeroic: bool) {
        let at = At { file, inputs: fd.params().filter(|p| p.seq).map(|p| p.name.clone()).collect(), task: fd.task, zeroic, results: fd.results.iter().map(|p| p.name.clone()).collect() };
        self.block(&fd.body, &at, &[], false, &mut Vec::new());
    }
}

/// has a block a push anywhere under it?
fn pushes(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Push { .. } => true,
        Stmt::If { then, els, .. } => pushes(then) || els.as_deref().is_some_and(pushes),
        Stmt::Loop { body, .. } | Stmt::For { body, .. } => pushes(body),
        _ => false,
    })
}

/// does an expression apply a stream word to a stream, or end one?
fn expr_touches(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Phrase(parts) | ExprKind::Existing(parts) => {
            let word = match parts.as_slice() {
                [Part::Word(w), s, ..] => seq_of(s).is_some() && (STREAM_WORDS.contains(&w.as_str()) || w == "end"),
                [s, Part::Word(w), ..] => seq_of(s).is_some() && matches!(w.as_str(), "behind" | "at" | "from"),
                _ => false,
            };
            word || parts.iter().any(|p| match p {
                Part::Args(list) => list.iter().any(|a| expr_touches(&a.value)),
                Part::Value(x) => expr_touches(x),
                Part::Word(_) => false,
            })
        }
        ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => expr_touches(x),
        ExprKind::List(items) => items.iter().any(expr_touches),
        ExprKind::Range { from: l, to: r, .. } | ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => expr_touches(l) || expr_touches(r),
        ExprKind::IfElse(c, a, b) => expr_touches(c) || expr_touches(a) || expr_touches(b),
        _ => false,
    }
}

/// does any line of a block push into a stream, end one, or apply a
/// stream word to one? A loop of which none does only computes
fn touches_a_stream(stmts: &[Stmt]) -> bool {
    let init = |v: &VarDecl| match &v.init {
        Some(Init::Value(e)) => expr_touches(e),
        Some(Init::Construct(args)) => args.iter().any(|a| expr_touches(&a.value)),
        Some(Init::Pushes { .. }) => true,
        None => false,
    };
    stmts.iter().any(|s| match s {
        Stmt::Push { .. } => true,
        Stmt::Var(v) => init(v),
        Stmt::Multi { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } | Stmt::Check { cond: value, .. } => expr_touches(value),
        Stmt::If { cond, then, els, .. } => expr_touches(cond) || touches_a_stream(then) || els.as_deref().is_some_and(touches_a_stream),
        Stmt::Loop { vars, cond, body, .. } => vars.iter().any(init) || cond.as_ref().is_some_and(expr_touches) || touches_a_stream(body),
        Stmt::For { seq, body, .. } => expr_touches(seq) || touches_a_stream(body),
        Stmt::Continue { values, .. } | Stmt::Break { values, .. } => values.iter().any(expr_touches),
    })
}

/// a store read and metered: every feature but the compiler's own
pub fn of_store(s: &Store, name: &str) -> Metered {
    let takers = zeroic::takers(s);
    let mut w = Walk { found: BTreeSet::new() };
    let mut lines = 0;
    for f in s.features.iter().chain(&s.left_out).filter(|f| f.name != "platform") {
        lines += std::fs::read_to_string(&f.code.file).map(|t| t.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
        for d in &f.code.decls {
            match d {
                Decl::Fn(fd) => {
                    // a body the compiler would refuse is metered as it stands
                    let zeroic = matches!(zeroic::read(fd, &f.code.file, &takers), Ok(Some(_)));
                    w.function(fd, &f.code.file, zeroic);
                }
                // a line at feature scope: an index in an initial value
                Decl::Var(v) => {
                    let at = At { file: &f.code.file, inputs: Vec::new(), task: false, zeroic: false, results: Vec::new() };
                    w.init(v, &at, false);
                }
                _ => {}
            }
        }
    }
    Metered { name: name.to_string(), lines, found: w.found.into_iter().collect() }
}

fn metered(dir: &Path) -> Result<Metered, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    Ok(of_store(&s, &dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
}

/// is the folder a store: has it a feature folder, `name/name.zero`?
fn is_store(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|d| d.filter_map(|e| e.ok()).any(|e| {
        let p = e.path();
        p.is_dir() && p.join(format!("{}.zero", p.file_name().unwrap().to_string_lossy())).is_file()
    }))
}

/// `probe zero meter <store>`: the store's count, its lines of zero,
/// and every line that uses a non-zeroic form with the form it uses.
/// `probe zero meter <suite>`: a row a store, and the total
pub fn report(dir: &Path) -> Result<String, String> {
    let mut out = String::new();
    if is_store(dir) {
        let m = metered(dir)?;
        writeln!(out, "{}: {} of {} lines of zero use a non-zeroic form", dir.display(), m.count(), m.lines).unwrap();
        let root = format!("{}/", dir.display());
        for f in &m.found {
            writeln!(out, "  {}:{}  {}", f.file.strip_prefix(&root).unwrap_or(&f.file), f.line, FORMS[f.form].0).unwrap();
        }
        let used: BTreeSet<usize> = m.found.iter().map(|f| f.form).collect();
        if !used.is_empty() {
            writeln!(out, "by form (a line that uses two is listed under both and counted once):").unwrap();
            for k in used {
                writeln!(out, "  {:>4}  {}: {}", m.found.iter().filter(|f| f.form == k).count(), FORMS[k].0, FORMS[k].1).unwrap();
            }
        }
        return Ok(out);
    }
    let mut stores: Vec<std::path::PathBuf> = std::fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir() && is_store(p)).collect();
    stores.sort();
    if stores.is_empty() {
        return Err(format!("no stores under {}", dir.display()));
    }
    writeln!(out, "{:<22} {:>10} {:>8}", "store", "non-zeroic", "lines").unwrap();
    let (mut total, mut all) = (0, 0);
    let mut by_form = [0usize; 9];
    for sdir in &stores {
        let m = metered(sdir)?;
        writeln!(out, "{:<22} {:>10} {:>8}", m.name, m.count(), m.lines).unwrap();
        total += m.count();
        all += m.lines;
        for f in &m.found {
            by_form[f.form] += 1;
        }
    }
    writeln!(out, "{:<22} {:>10} {:>8}", "total", total, all).unwrap();
    writeln!(out, "by form (a line that uses two is listed under both and counted once):").unwrap();
    for (k, n) in by_form.iter().enumerate() {
        writeln!(out, "  {:>4}  {}: {}", n, FORMS[k].0, FORMS[k].1).unwrap();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_of(tag: &str, code: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("probe-zero-meter-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 2\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), code).unwrap();
        dir
    }

    /// The meter's count is pinned for two small stores that do the
    /// same thing (fm3 log 129). The one written with no loop, a look
    /// back and `if` on its push uses no form on the list: 0 of its 7 lines. The
    /// one that walks uses six of them on 8 of its 18 lines, a line
    /// that uses two counted once and listed under both. And it
    /// refuses nothing: a store the compiler would refuse is metered
    #[test]
    fn the_meter_counts_non_zeroic_lines() {
        let zeroic = store_of("z", "int x$\nint d$ = rising(x$)\n\non (int d$) << rising (int x$)\n    d$ << x$ if (x$ > x$[-1])\n\non (int n) << f()\n    x$ << 1 << 3 << 2\n    n << count d$\n");
        let m = metered(&zeroic).unwrap();
        assert_eq!((m.count(), m.lines), (0, 7), "{:?}", m.found);
        let walking = store_of("w", "int x$\nint d$ = rising(x$)\nint last = 0\n\non (int d$) << rising (int x$)\n    loop\n        if (count x$ == 0)\n            break\n        int v = peek x$ at (0)\n        if (v > last)\n            d$ << v\n        last = v\n        advance x$ by (1)\n\non (int n) << f()\n    x$ << 1 << 3 << 2\n    int f$ = [4, 5]\n    for (v in f$)\n        n << n + f$[1]\n    n << peek d$ at (1)\n");
        let m = metered(&walking).unwrap();
        let forms: Vec<(usize, usize)> = m.found.iter().map(|f| (f.line, f.form)).collect();
        // 7 `count`, 9 `peek`, 13 `advance`: walking; 10: `if` round a
        // push; 12: a feature-scope name assigned; 18: a `for`; 19: an
        // index, the result's first giving being how a function gives
        // it; 20: the result given again, and a `peek` forward of now
        assert_eq!(forms, vec![(7, 2), (9, 2), (10, 4), (12, 0), (13, 2), (18, 7), (19, 8), (20, 0), (20, 3)], "{:?}", m.found);
        assert_eq!((m.count(), m.lines), (8, 18));
        // a body written both ways, which the compiler refuses, is metered
        let refused = store_of("r", "int x$\nint d$ = rising(x$)\n\non (int d$) << rising (int x$)\n    loop\n        if (count x$ == 0)\n            break\n        d$ << x$[-1]\n        advance x$ by (1)\n");
        assert!(super::super::lower::lower(&store::read(&refused).unwrap()).is_err());
        assert_eq!(metered(&refused).unwrap().count(), 2);
        let text = report(&walking).unwrap();
        assert!(text.contains(": 8 of 18 lines of zero use a non-zeroic form\n") && text.contains("  h/h.zero:20  a name assigned again\n  h/h.zero:20  an index or a `peek` forward of now\n"), "{}", text);
        for d in [zeroic, walking, refused] {
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}
