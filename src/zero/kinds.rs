//! The two kinds of sequence (fm3 question 90): an array, `int a[]`,
//! whose items are all there, and a stream, `int x$`, whose items
//! arrive. This file is the one walk over a parsed store that knows
//! what every name is where it is written: feature scope, a function's
//! parameters and results, a local from its declaration to the end of
//! its block, as the lowering scopes them. `probe zero names <store>`
//! prints what it found, every name declared with a mark, how it was
//! declared and each form it is used in (fm3 log 158): the census the
//! respelling was made from. And `settle` is what makes the mark part
//! of a name (log 159): every name is held to the mark it was
//! declared with, and an array's name is then written as the `Seq`
//! the lowering reads, so an array lowers to the lines a sequence
//! given whole always did.

use super::lex::{self, Error};
use super::store::Store;
use super::syntax::{Arg, Decl, Expr, ExprKind, FnDecl, Init, LoopInto, NamePart, Part, Stmt, VarDecl};
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

/// the mark a name is declared with, or written with
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    Plain,
    Stream,
    Array,
}

impl Mark {
    pub fn of(seq: bool, arr: bool) -> Mark {
        match (seq, arr) {
            (_, true) => Mark::Array,
            (true, false) => Mark::Stream,
            (false, false) => Mark::Plain,
        }
    }

    pub fn on(&self, name: &str) -> String {
        match self {
            Mark::Plain => name.to_string(),
            Mark::Stream => format!("{}$", name),
            Mark::Array => format!("{}[]", name),
        }
    }
}

/// a name in scope: its mark, and where it was declared
#[derive(Clone)]
struct Entry {
    at: Option<usize>,
    mark: Mark,
    ty: String,
    file: String,
    line: usize,
}

/// one name declared with a mark, and every use of it
#[derive(Clone, Debug)]
pub struct Named {
    pub mark: Mark,
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
    /// the scopes, innermost last
    scopes: Vec<HashMap<String, Entry>>,
    /// write each array's name as the `Seq` the lowering reads, and
    /// hold each name to its mark: the first refusal is kept
    settle: bool,
    refused: Option<Error>,
    file: String,
    /// the tasks of the store by their words, for a wiring
    tasks: Vec<String>,
    /// ... and those of them that are stream processors with no loop
    /// in them, which a function may hand an array (fm3 log 189)
    procs: Vec<String>,
    takers: super::zeroic::Takers,
    /// the functions a feature wires as sinks, `write(out$)`, by
    /// their words: what such a function is handed is a stream
    sinks: Vec<String>,
    /// in a stream processor's body, where every line says a stream
    processor: bool,
}

impl Walk {
    fn declare(&mut self, name: &str, mark: Mark, ty: &str, line: usize, place: Place, how: String) {
        self.declare_from(name, mark, ty, line, place, how, Vec::new())
    }

    fn declare_from(&mut self, name: &str, mark: Mark, ty: &str, line: usize, place: Place, how: String, from: Vec<usize>) {
        let at = if mark != Mark::Plain {
            self.names.push(Named { mark, file: self.file.clone(), line, ty: ty.to_string(), name: name.to_string(), place, how, uses: Vec::new(), from });
            Some(self.names.len() - 1)
        } else {
            None
        };
        self.scopes.last_mut().unwrap().insert(name.to_string(), Entry { at, mark, ty: ty.to_string(), file: self.file.clone(), line });
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.entry(name).and_then(|e| e.at)
    }

    fn refuse(&mut self, line: usize, msg: String) {
        if self.settle && self.refused.is_none() {
            self.refused = Some(lex::error(&self.file, line, msg));
        }
    }

    /// a line of the file being walked, as written, for a refusal to show
    fn source(&self, line: usize) -> Option<String> {
        let text = crate::vfs::read_to_string(&self.file).ok()?;
        text.lines().nth(line.checked_sub(1)?).map(|l| l.trim().to_string())
    }

    /// `text` with the stream `name$` written as the array `name[]`
    fn as_array(text: &str, name: &str) -> String {
        let from = format!("{}$", name);
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find(&from) {
            let joined = rest[..i].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_');
            out.push_str(&rest[..i]);
            let after = &rest[i + from.len()..];
            if joined {
                out.push_str(&from);
            } else if after.starts_with('[') {
                out.push_str(name);
            } else {
                out.push_str(&format!("{}[]", name));
            }
            rest = after;
        }
        out + rest
    }

    /// where a name was declared, for a refusal: `int a[]` on line 3
    fn declared(&self, name: &str, e: &Entry) -> String {
        let at = if e.file == self.file { format!("on line {}", e.line) } else { format!("at {}:{}", e.file, e.line) };
        format!("declared `{} {}` {}", e.ty, e.mark.on(name), at)
    }

    /// a name written with a mark: held to the one it was declared
    /// with (fm3 question 90, "the mark travels"), and its use noted
    fn used(&mut self, name: &str, written: Mark, line: usize, form: String, wants: &'static str) {
        let Some(e) = self.entry(name).cloned() else {
            if written == Mark::Array {
                self.refuse(line, format!("'{}[]' is not declared: an array is declared with its type, `int {}[] = [1, 2, 3]`", name, name));
            }
            return;
        };
        match (e.mark, written) {
            (Mark::Array, Mark::Stream) => self.refuse(line, format!("'{}$': '{}' is an array, {}, and the mark is part of its name wherever it is written (fm3 question 90): write `{}[]`, or `{}[k]` for one item", name, name, self.declared(name, &e), name, name)),
            (Mark::Stream, Mark::Array) => self.refuse(line, format!("'{}[]': '{}' is a stream, {}, and the mark is part of its name wherever it is written (fm3 question 90): write `{}$`", name, name, self.declared(name, &e), name)),
            (Mark::Plain, Mark::Array) => self.refuse(line, format!("'{}[]': '{}' is one value, {}, and has no items", name, name, self.declared(name, &e))),
            // an array where one value is wanted (fm3 question 96, log
            // 161): the name itself, or an operand, has no latest item
            // `if (a[] == b[])`: a bool for each pair where one is
            // wanted; the question meant is `[==]` (fm3 log 164)
            (Mark::Array, Mark::Array) if wants == "one value" && form.ends_with(", between two arrays") => {
                let op = if form.contains("`==`") { "==" } else { "!=" };
                let shown = self.source(line).filter(|l| l.matches(&format!(" {} ", op)).count() == 1);
                let (said, write) = match shown {
                    Some(l) => (format!("`{}`: ", l), format!("write `{}`", l.replace(&format!(" {} ", op), &format!(" [{}] ", op)))),
                    None => (String::new(), format!("write `a[] [{}] b[]`", op)),
                };
                self.refuse(line, format!("{}`{}` is applied to each pair of items and gives a bool for each, and one is wanted here (fm3 question 77). Whether the two arrays {} is `[{}]`: {}", said, op, if op == "==" { "are the same" } else { "differ" }, op, write));
            }
            (Mark::Array, Mark::Array) if wants == "one value" && (form == "the value" || form.starts_with("an operand of") || form.starts_with("an arm of")) => {
                self.refuse(line, format!("'{}[]' is an array, and one value is wanted here: an array has no latest item, as a stream has (fm3 question 90). Its last item is `{}[count {}[] - 1]`, one item `{}[k]`, and its sum `{}[] + _`", name, name, name, name, name));
            }
            (Mark::Array, Mark::Array) if form == "pushed into" => {
                self.refuse(line, format!("'{}[] << ...': an array never changes: its items are all there where it is declared, `{} {}[] = [...]` (fm3 question 90). What is pushed into is a stream, `{} {}$`", name, e.ty, name, e.ty, name));
            }
            (Mark::Array, Mark::Array) if form.starts_with("a look back") => {
                self.refuse(line, format!("'{}[-k]': a look back is a stream's, `x$[-1]`, the item before the present one (fm3 question 90). An array's last item is `{}[count {}[] - 1]`", name, name, name));
            }
            (Mark::Array, Mark::Array) if form.starts_with("the word `") && form != "the word `count`" => {
                let w = form.trim_start_matches("the word `").trim_end_matches('`');
                let instead = match w {
                    "peek" => format!(": one item of an array is `{}[k]`", name),
                    "latest" => format!(": an array's last item is `{}[count {}[] - 1]`", name, name),
                    "frame" | "behind" | "from ... to" => ": it makes an array of what a stream holds, and this is one already".to_string(),
                    _ => String::new(),
                };
                self.refuse(line, format!("`{}` is a stream's word, asked of what arrives over time, and '{}[]' is an array, all there (fm3 question 90){}", w, name, instead));
            }
            (Mark::Stream, Mark::Stream) if form.starts_with("an item by its place") && !self.processor => {
                self.refuse(line, format!("'{}$[k]': an item by its place is an array's, and '{}$' is a stream (fm3 question 90). The item k on from where this reader stands is `peek {}$ at (k)`; the array of what has arrived is `frame {}$`, and one back is `{}$[-1]`", name, name, name, name, name));
            }
            // a look back outside a stream processor (fm3 log 162): a
            // function has no present item to look back from, and the
            // line was `peek x$ at (-1)`, unchecked (question 61)
            (Mark::Stream, Mark::Stream) if form.starts_with("a look back") && !self.processor => {
                self.refuse(line, format!("'{}$[-1]' is a look back, the item before the present one, and only a stream processor has a present item (fm3 question 75). In a function a stream's latest item is its name, `{}$`; the items its reader has passed are `{}$ behind (k)`", name, name, name));
            }
            (Mark::Stream, Mark::Stream) if form == "walked by `for`" => {
                self.refuse(line, format!("`for` walks an array, and '{}$' is a stream (fm3 question 90): the array of what has arrived is `frame {}$`, `for (x in frame {}$)`", name, name, name));
            }
            (Mark::Stream, Mark::Stream) if form.starts_with("reduced") && !self.processor => {
                self.refuse(line, format!("a reduce with `_` gives one answer of a whole array, and '{}$' is a stream (fm3 question 90): the array of what has arrived is `frame {}$`; a running total is a line that stands, `sum$ << sum$ + {}$ forever`", name, name, name));
            }
            _ => {}
        }
        if let Some(i) = e.at {
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
            ExprKind::Arr(n) => format!("given `{}[]`", n),
            ExprKind::Bin(op, ..) => format!("given an operator's sequence, `{}`", op),
            ExprKind::Phrase(parts) => match parts.as_slice() {
                [Part::Word(w), Part::Value(Expr { kind: ExprKind::Seq(_) | ExprKind::Arr(_), .. })] if w == "frame" => "given a `frame`".into(),
                [Part::Value(Expr { kind: ExprKind::Seq(_) | ExprKind::Arr(_), .. }), Part::Word(w), ..] if w == "behind" || w == "from" => format!("given `{}`", w),
                _ => {
                    let words = spoken(parts);
                    if self.is_task(&words) && !self.applied(parts) {
                        format!("wired to the task `{}`", words)
                    } else {
                        format!("given what `{}` gives", words)
                    }
                }
            },
            _ => "given a value".into(),
        }
    }

    fn var(&mut self, v: &mut VarDecl, place: Place) {
        let how = match (&v.init, &v.rate) {
            (Some(Init::Value(e)), _) => self.given(e),
            (Some(Init::Pushes { .. }), Some(_)) => "at a rate, first items".into(),
            (Some(Init::Pushes { .. }), None) => "first items".into(),
            (Some(Init::Construct(_)), _) => "constructed".into(),
            (None, Some(_)) => "at a rate".into(),
            (None, None) => "bare".into(),
        };
        let mark = Mark::of(v.seq, v.arr);
        self.declared_right(v, mark, &place, &how);
        // the value is worked out before the name exists, but a
        // stream's first items may read the stream itself
        let wants = if v.seq { "a sequence" } else { "one value" };
        match &mut v.init {
            Some(Init::Value(e)) => {
                let mut named = Vec::new();
                // what a word makes of a stream that is over, a `frame`,
                // `behind`, a window, is all there; what a function
                // gives is its own to say
                if !matches!(e.kind, ExprKind::Phrase(_)) {
                    super::syntax::seqs_in(e, &mut named);
                }
                let from = named.iter().filter_map(|n| self.find(n)).collect();
                self.expr(e, wants, "the value");
                self.declare_from(&v.name, mark, &v.ty, v.line, place, how, from);
            }
            Some(Init::Construct(args)) => {
                self.args(args, "one value", "an argument of a constructor");
                self.declare(&v.name, mark, &v.ty, v.line, place, how);
            }
            Some(Init::Pushes { items, cond, .. }) => {
                self.declare(&v.name, mark, &v.ty, v.line, place, how);
                for e in items {
                    self.expr(e, "an item of a push", "the item");
                }
                if let Some(c) = cond {
                    self.expr(c, "one value", "the value");
                }
            }
            None => self.declare(&v.name, mark, &v.ty, v.line, place, how),
        }
        if let Some(r) = &mut v.rate {
            self.expr(r, "one value", "the value");
        }
    }

    /// A declaration held to its kind (fm3 question 90, log 161): what
    /// `=` gives a `$` name is a task's stream or it is an array, and
    /// an array is given whole where it is declared
    fn declared_right(&mut self, v: &VarDecl, mark: Mark, place: &Place, how: &str) {
        let line = v.line;
        let shown = self.source(line).unwrap_or_else(|| format!("{} {} ...", v.ty, mark.on(&v.name)));
        match (mark, &v.init) {
            (Mark::Stream, Some(Init::Value(e))) if *place != Place::Said && !how.starts_with("wired") => {
                let begins = match e.kind {
                    ExprKind::List(_) | ExprKind::Range { .. } | ExprKind::Str(_) => format!("; or, for a stream that begins with these items, `{}`", shown.replacen(" = ", " << ", 1)),
                    _ => String::new(),
                };
                self.refuse(line, format!("`{}`: what `=` gives here is an array, all there, and `$` is a stream's mark (fm3 question 90). Write `{}`{}", shown, Walk::as_array(&shown, &v.name), begins));
            }
            (Mark::Array, Some(Init::Value(_))) if how.starts_with("wired") => {
                self.refuse(line, format!("`{}`: a task gives a stream, its items arriving, and '{}[]' is an array (fm3 question 90). Write `{} {}$ = ...`; the array of what has arrived in it is `frame {}$`", shown, v.name, v.ty, v.name, v.name));
            }
            (Mark::Array, Some(Init::Pushes { .. })) => {
                self.refuse(line, format!("`{}`: an array is given whole where it is declared, by `=`, and never pushed into (fm3 question 90). Write `{} {}[] = [...]`; what has first items and more to come is a stream, `{} {}$ << ...`", shown, v.ty, v.name, v.ty, v.name));
            }
            (Mark::Array, _) if v.rate.is_some() => {
                self.refuse(line, format!("`{}`: a rate is a stream's, and '{}[]' is an array, all there (fm3 question 90)", shown, v.name));
            }
            (Mark::Array, None) => {
                self.refuse(line, format!("`{}`: an array is given whole where it is declared, `{} {}[] = [1, 2, 3]`, and never changes (fm3 question 90); an empty one is `{} {}[] = []`. What is declared bare and filled later is a stream, `{} {}$`", shown, v.ty, v.name, v.ty, v.name, v.ty, v.name));
            }
            _ => {}
        }
    }

    fn args(&mut self, args: &mut [Arg], wants: &'static str, form: &str) {
        for a in args {
            self.expr(&mut a.value, wants, form);
        }
    }

    /// `form` is how the expression itself stands in what is round it,
    /// and matters only where the expression is a name
    fn expr(&mut self, e: &mut Expr, wants: &'static str, form: &str) {
        let line = e.line;
        match &mut e.kind {
            ExprKind::Seq(n) => {
                let n = n.clone();
                self.used(&n, Mark::Stream, line, form.to_string(), wants)
            }
            ExprKind::Arr(n) => {
                let n = n.clone();
                self.used(&n, Mark::Array, line, form.to_string(), wants);
                if self.settle {
                    e.kind = ExprKind::Seq(n);
                }
            }
            // a marked name written as a bare word, alone
            ExprKind::Phrase(parts) if matches!(parts.as_slice(), [Part::Word(_)]) => {
                let [Part::Word(w)] = parts.as_slice() else { unreachable!() };
                if let Some(en) = self.entry(w).filter(|en| en.mark != Mark::Plain).cloned() {
                    let msg = format!("'{}' is written without its mark: it is {}, and the mark is part of its name wherever it is written (fm3 question 90): write `{}`", w, self.declared(w, &en), en.mark.on(w));
                    self.refuse(line, msg);
                }
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Name(_) | ExprKind::Acc => {}
            ExprKind::Unit(x, _) => self.expr(x, wants, "the value"),
            ExprKind::Neg(x) => self.expr(x, wants, "an operand of `-`"),
            ExprKind::Field(x, _) => self.expr(x, "one value", "the base of a field"),
            ExprKind::List(items) => items.iter_mut().for_each(|x| self.expr(x, "one value", "an item of a list")),
            ExprKind::Range { from, to, .. } => {
                self.expr(from, "one value", "a bound of a range");
                self.expr(to, "one value", "a bound of a range");
            }
            ExprKind::Bin(op, l, r) => {
                let acc = matches!(l.kind, ExprKind::Acc) || matches!(r.kind, ExprKind::Acc);
                // an operator in square brackets takes its two sides
                // whole (fm3 question 77, log 164)
                let whole = op.starts_with('[');
                let written = |x: &Expr| matches!(x.kind, ExprKind::Arr(_) | ExprKind::List(_) | ExprKind::Range { .. });
                let form = if acc {
                    format!("reduced, `{} _`", op)
                } else if whole {
                    format!("a side of `{}`", op)
                } else if matches!(op.as_str(), "==" | "!=") && written(l) && written(r) {
                    // both sides written as arrays: what is meant is
                    // the bracketed one, and the refusal says so
                    format!("an operand of `{}`, between two arrays", op)
                } else {
                    format!("an operand of `{}`", op)
                };
                let wants = if whole { "a sequence" } else { wants };
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
                self.expr(i, "one value", "the value");
                // `s[0]` on a plain `string s`, or on a word that is no
                // variable: the name and a list of one item, the tree
                // the parser made before an array had a mark (log 159)
                if let ExprKind::Arr(n) = &base.kind {
                    if !self.entry(n).is_some_and(|en| en.mark != Mark::Plain) {
                        if self.settle {
                            let list = Expr { kind: ExprKind::List(vec![(**i).clone()]), line };
                            e.kind = ExprKind::Phrase(vec![Part::Word(n.clone()), Part::Value(list)]);
                        }
                        return;
                    }
                }
                self.expr(base, wants, form);
            }
            ExprKind::Phrase(parts) | ExprKind::Existing(parts) => self.phrase(parts, line, wants),
        }
    }

    fn phrase(&mut self, parts: &mut [Part], line: usize, wants: &'static str) {
        let seq = |p: &Part| -> Option<(String, Mark)> {
            let of = |e: &Expr| match &e.kind {
                ExprKind::Seq(n) => Some((n.clone(), Mark::Stream)),
                ExprKind::Arr(n) => Some((n.clone(), Mark::Array)),
                _ => None,
            };
            match p {
                Part::Value(e) => of(e),
                Part::Args(a) if a.len() == 1 && a[0].name.is_none() => of(&a[0].value),
                _ => None,
            }
        };
        // a stream's words, as `stream_word` reads them: the word, the
        // part that names the stream, and where the rest begins
        let word: Option<(String, usize, usize)> = match &*parts {
            [Part::Word(t), Part::Word(of), x] if t == "time" && of == "of" && seq(x).is_some() => Some(("time of".to_string(), 2, 3)),
            [Part::Word(w), x] if WORDS.contains(&w.as_str()) && seq(x).is_some() => Some((w.clone(), 1, 2)),
            [Part::Word(w), x, Part::Word(k), _] if ((w == "peek" && k == "at") || (w == "advance" && k == "by")) && seq(x).is_some() => Some((w.clone(), 1, 3)),
            [x @ Part::Value(_), Part::Word(w), _] if (w == "behind" || w == "at") && seq(x).is_some() => Some((w.clone(), 0, 2)),
            [x @ Part::Value(_), Part::Word(w), _, Part::Word(to), _] if w == "from" && to == "to" && seq(x).is_some() => Some(("from ... to".to_string(), 0, 2)),
            _ => None,
        };
        if let Some((w, at, rest)) = word {
            let (n, mark) = seq(&parts[at]).unwrap();
            if self.entry(&n).is_some() {
                self.used(&n, mark, line, format!("the word `{}`", w), wants);
                if self.settle {
                    match &mut parts[at] {
                        Part::Value(e) => e.kind = ExprKind::Seq(n),
                        Part::Args(a) => a[0].value.kind = ExprKind::Seq(n),
                        Part::Word(_) | Part::Whole => {}
                    }
                }
                for p in &mut parts[rest..] {
                    match p {
                        Part::Args(a) => self.args(a, "one value", "the value"),
                        Part::Value(x) => self.expr(x, "one value", "the value"),
                        Part::Word(_) | Part::Whole => {}
                    }
                }
                return;
            }
        }
        let words = spoken(parts);
        let acc = parts.iter().any(|p| match p {
            Part::Args(a) => a.iter().any(|a| matches!(a.value.kind, ExprKind::Acc)),
            Part::Value(x) => matches!(x.kind, ExprKind::Acc),
            Part::Word(_) | Part::Whole => false,
        });
        let task = self.is_task(&words) && !self.applied(parts);
        let form = if task {
            format!("handed to the task `{}`", words)
        } else if acc {
            format!("reduced by `{}` with `_`", words)
        } else {
            format!("an argument of `{}`", words)
        };
        // what a function wants of an argument is its declaration's to
        // say; a task is handed a stream
        let wants = if task || wants == "a sink" { wants } else { "an argument" };
        for p in parts {
            match p {
                Part::Args(a) => self.args(a, wants, &form),
                Part::Value(x) => self.expr(x, wants, &form),
                Part::Word(_) | Part::Whole => {}
            }
        }
    }

    fn block(&mut self, stmts: &mut [Stmt]) {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Var(v) => {
                let place = if self.processor { Place::Said } else { Place::Local };
                self.var(v, place)
            }
            Stmt::Multi { vars, value, .. } => {
                self.expr(value, "one value", "the value");
                for p in vars {
                    self.declare(&p.name, Mark::of(p.seq, p.arr), &p.ty, p.line, Place::Local, "one of several from a call".into());
                }
            }
            Stmt::Assign { targets, value, line } => {
                let wants = if targets.iter().any(|t| t.seq) { "a sequence" } else { "one value" };
                self.expr(value, wants, "the value");
                for t in targets.iter().filter(|t| t.seq) {
                    let form = if t.pushed { "given by `<<`, a result" } else { "given by `=`" };
                    self.used(&t.name, Mark::of(t.seq, t.arr), *line, form.into(), "a sequence");
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
                        self.declare(&p.name, Mark::of(p.seq, p.arr), &p.ty, p.line, Place::Local, "what a loop gives".into());
                    }
                }
            }
            Stmt::For { var, seq, body, line } => {
                self.expr(seq, "a sequence", "walked by `for`");
                self.scopes.push(HashMap::new());
                self.declare(var, Mark::Plain, "", *line, Place::Local, String::new());
                self.block(body);
                self.scopes.pop();
            }
            Stmt::Continue { values, .. } | Stmt::Break { values, .. } => values.iter_mut().for_each(|v| self.expr(v, "one value", "the value")),
            Stmt::Check { cond, .. } => self.expr(cond, "one value", "the value"),
            Stmt::Push { target, items, cond, forever, .. } => {
                self.push(target, items, cond.as_mut(), *forever);
            }
            Stmt::Expr { expr, .. } => self.expr(expr, "nothing", "the value"),
        }
    }

    fn push(&mut self, target: &mut Expr, items: &mut [Expr], cond: Option<&mut Expr>, stands: bool) {
        self.expr(target, "a sequence", "pushed into");
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

    /// A stream processor with no loop in it handed an array (fm3
    /// question 113, log 189): its one argument is anything but a
    /// stream's own name. It gives an array, as a function does, and
    /// is no wiring
    fn applied(&self, parts: &[Part]) -> bool {
        if !self.procs.iter().any(|t| *t == spoken(parts)) {
            return false;
        }
        let args: Vec<&Expr> = parts.iter().flat_map(|p| match p {
            Part::Args(a) => a.iter().map(|a| &a.value).collect::<Vec<_>>(),
            Part::Value(x) => vec![x],
            Part::Word(_) | Part::Whole => Vec::new(),
        }).collect();
        matches!(args.as_slice(), [x] if !matches!(x.kind, ExprKind::Seq(_)))
    }

    fn function(&mut self, f: &mut FnDecl) {
        let method = matches!(f.name.as_slice(), [NamePart::Group, NamePart::Sym(s), ..] if s == "<<");
        // a body `read` refuses is a processor with a mistake in it,
        // and the refusal is the lowering's to make
        let processor = f.task && !matches!(super::zeroic::read(f, &self.file, &self.takers), Ok(None));
        self.processor = processor;
        let kind = if processor {
            "a stream processor"
        } else if f.task && f.params().any(|p| p.seq) {
            "a task that walks"
        } else if f.task {
            "a task"
        } else if method {
            "a `<<` method"
        } else if self.sinks.contains(&said(&f.name)) {
            "a sink"
        } else {
            "a function"
        };
        self.scopes.push(HashMap::new());
        for r in &f.results {
            self.declare(&r.name, Mark::of(r.seq, r.arr), &r.ty, f.line, Place::Result(kind), String::new());
        }
        for (g, group) in f.groups.iter().enumerate() {
            for p in group {
                let how = if method && g == 0 { "the stream pushed into" } else { "" };
                self.declare(&p.name, Mark::of(p.seq, p.arr), &p.ty, f.line, Place::Param(kind), how.to_string());
            }
        }
        self.block(&mut f.body);
        self.scopes.pop();
        self.processor = false;
    }

    /// the whole store: feature scope first, which is the store's,
    /// then each feature's functions, wirings and cases
    fn store(&mut self, features: &mut [&mut super::store::FeatureDoc]) {
        for f in features.iter() {
            for d in &f.code.decls {
                if let Decl::Wire(Expr { kind: ExprKind::Phrase(parts), .. }) = d {
                    self.sinks.push(spoken(parts));
                }
                if let Decl::Fn(fd) = d {
                    if fd.task {
                        self.tasks.push(said(&fd.name));
                        if matches!(super::zeroic::read(fd, &f.code.file, &self.takers), Ok(Some(_))) {
                            self.procs.push(said(&fd.name));
                        }
                    }
                }
            }
        }
        for f in features.iter_mut() {
            self.file = f.code.file.clone();
            for d in &mut f.code.decls {
                if let Decl::Var(v) = d {
                    self.var(v, Place::Feature);
                }
            }
        }
        for f in features.iter_mut() {
            self.file = f.code.file.clone();
            for d in &mut f.code.decls {
                match d {
                    Decl::Fn(fd) => self.function(fd),
                    Decl::Wire(e) => self.expr(e, "a sink", "the value"),
                    Decl::Edge { target, items, first, cond, only, .. } => {
                        // a `<<` at feature scope with a stream on its right
                        // stands, by `forever`, a count or an `until`
                        self.push(target, items, cond.as_mut(), true);
                        if let Some(c) = only {
                            self.expr(c, "one value", "the value");
                        }
                        if let Some(x) = first {
                            self.expr(x, "an item of a push that stands", "the value");
                        }
                    }
                    Decl::Var(_) | Decl::Type(_) => {}
                }
            }
            self.file = f.md_file.clone();
            for c in &mut f.cases {
                self.expr(&mut c.call, "a case", "the value");
            }
        }
    }
}

/// the words of a phrase, as a reader says them
fn spoken(parts: &[Part]) -> String {
    parts.iter().filter_map(|p| if let Part::Word(w) = p { Some(w.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
}

/// the words of a declared name
fn said(name: &[NamePart]) -> String {
    name.iter().filter_map(|p| if let NamePart::Word(x) = p { Some(x.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
}

fn walk(store: &Store, settle: bool) -> Walk {
    Walk { names: Vec::new(), scopes: vec![HashMap::new()], settle, refused: None, file: String::new(), tasks: Vec::new(), procs: Vec::new(), takers: super::zeroic::takers(store), sinks: Vec::new(), processor: false }
}

/// Hold every name of a store to the mark it was declared with, and
/// write each array's name as the `Seq` the lowering reads (fm3 log
/// 159). After this no tree of the store has an `Arr` in it
pub fn settle(store: &mut Store) -> Result<(), Error> {
    let mut w = walk(store, true);
    let mut features: Vec<&mut super::store::FeatureDoc> = store.features.iter_mut().chain(store.left_out.iter_mut()).collect();
    w.store(&mut features);
    match w.refused {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// every name of a store declared with a mark, with its uses
pub fn names(store: &Store) -> Vec<Named> {
    let mut w = walk(store, false);
    let mut copy: Vec<super::store::FeatureDoc> = store.features.iter().chain(store.left_out.iter()).cloned().collect();
    let mut features: Vec<&mut super::store::FeatureDoc> = copy.iter_mut().collect();
    w.store(&mut features);
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
        // `PROBE_ZERO_USES`: a line for each use as well, for a list
        // of every place a form stands (the handover's, fm3 log 161)
        if std::env::var("PROBE_ZERO_USES").is_ok() {
            for u in &n.uses {
                out.push_str(&format!("use\t{}:{}\t{} {}\t{}\t{}\t{}\t{}\n", u.0, u.1, n.ty, n.mark.on(&n.name), kind, place, u.2, u.3));
            }
            continue;
        }
        out.push_str(&format!("{}\t{}:{}\t{} {}\t{}{}{}\tstream: {}\tarray: {}\tat: {}\n", kind, n.file, n.line, n.ty, n.mark.on(&n.name), place, if n.how.is_empty() { "" } else { ", " }, n.how, stream.join("; "), array.join("; "), lines.join(" ")));
    }
    Ok(out)
}
