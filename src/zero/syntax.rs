//! The syntax tree and the parser: tokens to a small tree per feature.
//! The parser knows nothing about meaning — an open-syntax call is kept
//! as a phrase of words and arguments, and the lowering resolves it
//! against the store's functions and the scope's variables. It knows
//! the store's type names (collected before parsing, since a type may be
//! declared by another feature), because `Vec v(1, 2, 3)` and
//! `count down()` are told apart only by whether the first word is one.
// the tree carries every construct of zero.md; the plan items lower them one at a time
#![allow(dead_code)]

use super::lex::{self, Error, Tok, Token};
use std::collections::HashSet;

#[derive(Clone)]
pub struct Feature {
    pub name: String,
    pub file: String,
    pub decls: Vec<Decl>,
}

#[derive(Clone)]
pub enum Decl {
    Fn(FnDecl),
    Type(TypeDecl),
    Var(VarDecl),
    /// a bare phrase at feature scope, `write(out$)`: a sink wired to
    /// the streams it reads, with no stream to fill (log 57)
    Wire(Expr),
    /// `out$ << (i$ << "\n") forever` at feature scope: an edge (log 72),
    /// a standing connection the scheduler moves items along, the rest
    /// of the chain pushed after each. `forever` is what makes it stand
    /// (fm3 question 79): the line is kept without it so the lowering
    /// can say what it would mean. `only` is `if (c)` on it, a standing
    /// filter, with the source's own name already read as the item
    /// `first` is what is pushed for each item where the line's first
    /// item was an expression, `sum$ << sum$ + x$ forever` (fm3
    /// question 80, log 149): the one stream it names other than the
    /// target paces the line and is kept as `items[0]`, and `first` is
    /// the expression with that stream's name read as the item
    /// `group` is how many of the line's last items its word applies
    /// to (fm3 question 84, log 155): 1 with nothing bracketed, and
    /// all of them in `out$ << (i$ << "\n") forever`. Where the word
    /// has several items and no brackets, `shown` is the line as its
    /// text would be with brackets round them all, for the refusal to
    /// show (log 156)
    Edge { target: Expr, items: Vec<Expr>, group: usize, shown: Option<String>, first: Option<Expr>, cond: Option<Expr>, word: Repeat, only: Option<Expr>, forever: bool, line: usize },
}

/// Which word a push's `cond` goes with (fm3 question 79, log 147):
/// `while (c)`, tested before each push of the last item; `(n) times`,
/// a count worked out once; `until (c)`, tested after each push. One
/// field holds the expression for all three, so a pass that only asks
/// what a push mentions reads it as it read a `while`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repeat {
    While,
    Times,
    Until,
}

#[derive(Clone)]
pub struct FnDecl {
    pub line: usize,
    pub results: Vec<Param>,
    /// the words, symbols and parameter groups, in order
    pub name: Vec<NamePart>,
    /// the parameter groups, in the order their `Group` parts appear
    pub groups: Vec<Vec<Param>>,
    /// declared with `<<` and a `$` on its result: a task producing its
    /// result over time, or a stream processor. Every function is
    /// declared with `<<` (fm3 question 77), and one whose results
    /// have no `$` is a plain function, which gives each by pushing it
    /// once (log 151)
    pub task: bool,
    pub body: Vec<Stmt>,
    /// `platform <kind> [<kind>...]` bodies: the kinds and the lines
    pub platform: Vec<(Vec<String>, Vec<String>)>,
}

impl FnDecl {
    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.groups.iter().flatten()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NamePart {
    Word(String),
    Sym(String),
    Group,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub ty: String,
    pub name: String,
    pub seq: bool,
    /// written `a[]`, an array (fm3 question 90); `seq` is true of it
    /// too, so what asks "is this a sequence" reads as it did
    pub arr: bool,
    pub line: usize,
}

#[derive(Clone)]
pub struct TypeDecl {
    pub line: usize,
    pub name: String,
    pub kind: TypeKind,
}

#[derive(Clone)]
pub enum TypeKind {
    Enum(Vec<String>),
    Struct(Vec<Field>),
}

#[derive(Clone)]
pub struct Field {
    pub ty: String,
    pub name: String,
    pub seq: bool,
    pub default: Option<Expr>,
    pub line: usize,
}

#[derive(Clone)]
pub struct VarDecl {
    pub line: usize,
    /// the scope words in front: `static`, `device`, `group`
    pub scope: Vec<String>,
    pub ty: String,
    pub name: String,
    pub seq: bool,
    /// declared `int a[]`, an array (fm3 question 90)
    pub arr: bool,
    pub init: Option<Init>,
    /// `merge sum`: how two writes combine
    pub merge: Option<String>,
    /// `at (48000 hz)`: a stream at a rate
    pub rate: Option<Expr>,
}

#[derive(Clone)]
pub enum Init {
    Value(Expr),
    /// `Vec v(1, 2, 3)`, `Vec v(z = 3, x = 1)`
    Construct(Vec<Arg>),
    /// `int i$ << 1 << (i$ + 1) while (i$ < 5)`; `group` as on a push
    Pushes { items: Vec<Expr>, group: usize, cond: Option<Expr>, word: Repeat },
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone)]
pub enum Stmt {
    Var(VarDecl),
    /// `int q, int r = divide (a) by (b)`: several declared at once from one call
    Multi { vars: Vec<Param>, value: Expr, line: usize },
    Assign { targets: Vec<Target>, value: Expr, line: usize },
    /// `on_push` says the `if` was written on a push's own line,
    /// `x$ << item if (c)` (fm3 question 75 rule 3, respelled from
    /// `when` by question 79): the push under its condition
    If { cond: Expr, then: Vec<Stmt>, els: Option<Vec<Stmt>>, line: usize, on_push: bool },
    /// `loop (vars) while (c) yields x, y` (log 40, 48): `yields` names the
    /// carried variables that leave, into declared or existing names
    Loop { vars: Vec<VarDecl>, cond: Option<Expr>, body: Vec<Stmt>, yields: Vec<String>, into: Option<LoopInto>, line: usize },
    For { var: String, seq: Expr, body: Vec<Stmt>, line: usize },
    Continue { values: Vec<Expr>, line: usize },
    /// `break`, or `break (values)`: the names the loop yields given
    /// their values where it leaves (fm3 question 81)
    Break { values: Vec<Expr>, line: usize },
    Check { cond: Expr, line: usize },
    /// `x$ << a << b while (c)`; `existing` on it calls the link below
    /// this body in its chain, which is how a feature extends a `<<`
    /// method — the one shape `existing name(...)` cannot spell
    /// `forever` is the word on it (fm3 question 79), which only a
    /// line at feature scope may take: kept so the refusal can say why.
    /// A word applies to the last item of the chain, and brackets
    /// round several items make them the one it applies to, `x$ <<
    /// (a << b) (3) times` (fm3 question 84, log 155): the items stay
    /// one flat list, and `group` is how many of its last the word
    /// covers, 1 where nothing is bracketed
    Push { target: Expr, items: Vec<Expr>, group: usize, cond: Option<Expr>, word: Repeat, existing: bool, forever: bool, line: usize },
    Expr { expr: Expr, line: usize },
}

/// where a loop's given values go: `int total = loop ...` declares,
/// `total = loop ...` assigns
#[derive(Clone)]
pub enum LoopInto {
    Declare(Vec<Param>),
    Assign(Vec<Target>),
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub seq: bool,
    /// written `a[]`
    pub arr: bool,
    pub line: usize,
    /// `countdown.enabled = false`: a feature's implicit variable (log 28)
    pub feature: Option<String>,
    /// written `y << value`: a function's result given by pushing it
    /// (fm3 question 77, log 151). The statement is the giving of a
    /// result an assignment was, and lowers as one; the mark is for
    /// the refusals, a name that is no result not being pushed into
    pub pushed: bool,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(String),
    Str(String),
    Bool(bool),
    Name(String),
    Seq(String),
    /// `a[]`: an array's name, whole (fm3 question 90). `kinds::settle`
    /// holds it to its declaration and writes it as the `Seq` the
    /// lowering reads, so nothing past the store's reading meets one
    Arr(String),
    /// `_`: the accumulator of a reduction
    Acc,
    /// `1 hz`, `500 ms`
    Unit(Box<Expr>, String),
    List(Vec<Expr>),
    Range { from: Box<Expr>, to: Box<Expr>, inclusive: bool },
    Bin(String, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    /// `x$[i]`: an element of a sequence
    Index(Box<Expr>, Box<Expr>),
    /// an open-syntax call: words and arguments, resolved by the lowering
    Phrase(Vec<Part>),
    /// `existing ...`: the previous definition of the enclosing function
    Existing(Vec<Part>),
}

#[derive(Clone, Debug)]
pub enum Part {
    Word(String),
    /// a bracketed group of arguments
    Args(Vec<Arg>),
    /// a bare argument: a literal, a sequence name, a list
    Value(Expr),
}

/// the types every store has
pub const BUILTIN_TYPES: [&str; 34] = [
    "bool", "int", "uint", "index", "float", "number", "scalar", "fixed", "unit", "sunit", "rational", "decimal", "time", "string",
    "int8", "int16", "int32", "int64", "int128", "uint8", "uint16", "uint32", "uint64", "uint128", "float16", "float32", "float64",
    "bfloat16", "int256", "uint256", "int1", "uint1", "char", "byte",
];

const SCOPES: [&str; 3] = ["static", "device", "group"];
const UNITS: [&str; 7] = ["hz", "khz", "s", "ms", "us", "ns", "min"];

/// the type names a file declares, found before any file is parsed
pub fn declared_types(src: &str) -> Vec<String> {
    src.lines()
        .filter_map(|l| l.strip_prefix("type "))
        .filter_map(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .collect()
}

/// The words that stand before `and` and `or` in the names of the
/// functions a file declares (fm3 question 66): `smaller of (a) and
/// (b)` gives "and" after "smaller of". `and` and `or` join two
/// conditions everywhere but where a declared name has them, and the
/// parser, which knows no functions, is handed these with the type
/// names, each spelled with a first character no type name has
pub fn declared_joins(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in src.lines().filter(|l| l.starts_with("on ")) {
        let mut toks = Vec::new();
        if lex::lex_line(l, 0, "", &mut toks).is_err() {
            continue;
        }
        // past `on`, and past the results where the first group is
        // followed by `=` or `<<`
        let mut i = 1;
        if matches!(toks.get(1).map(|t| &t.tok), Some(Tok::Sym("("))) {
            let mut depth = 0;
            let mut k = 1;
            while k < toks.len() {
                match &toks[k].tok {
                    Tok::Sym("(") => depth += 1,
                    Tok::Sym(")") => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            if matches!(toks.get(k + 1).map(|t| &t.tok), Some(Tok::Sym("=")) | Some(Tok::Sym("<<"))) {
                i = k + 2;
            }
        }
        let mut words: Vec<String> = Vec::new();
        let mut depth = 0;
        for t in &toks[i.min(toks.len())..] {
            match &t.tok {
                Tok::Sym("(") => depth += 1,
                Tok::Sym(")") => depth -= 1,
                Tok::Word(w) if depth == 0 => {
                    if JOINERS.contains(&w.as_str()) {
                        out.push(join_key(w, &words));
                    }
                    words.push(w.clone());
                }
                _ => {}
            }
        }
    }
    out
}

/// An expression with a stream's own name read as one item of it, the
/// name `__item`: the condition of a standing filter, `e$ << x$ if
/// (x$ > 0) forever`, as the edge's function of one item reads it
pub fn as_item(e: &Expr, stream: &str) -> Expr {
    renamed(e, stream, "__item")
}

/// an expression with a stream's name read as the value of the name `to`
pub fn renamed(e: &Expr, stream: &str, to: &str) -> Expr {
    let f = |x: &Expr| Box::new(renamed(x, stream, to));
    let parts = |ps: &[Part]| -> Vec<Part> {
        ps.iter()
            .map(|p| match p {
                Part::Args(list) => Part::Args(list.iter().map(|a| Arg { name: a.name.clone(), value: renamed(&a.value, stream, to) }).collect()),
                Part::Value(x) => Part::Value(renamed(x, stream, to)),
                Part::Word(w) => Part::Word(w.clone()),
            })
            .collect()
    };
    let kind = match &e.kind {
        ExprKind::Seq(n) if n == stream => ExprKind::Name(to.into()),
        ExprKind::Unit(x, u) => ExprKind::Unit(f(x), u.clone()),
        ExprKind::Neg(x) => ExprKind::Neg(f(x)),
        ExprKind::Field(x, n) => ExprKind::Field(f(x), n.clone()),
        ExprKind::List(items) => ExprKind::List(items.iter().map(|x| renamed(x, stream, to)).collect()),
        ExprKind::Range { from, to: end, inclusive } => ExprKind::Range { from: f(from), to: f(end), inclusive: *inclusive },
        ExprKind::Bin(op, l, r) => ExprKind::Bin(op.clone(), f(l), f(r)),
        ExprKind::Index(l, r) => ExprKind::Index(f(l), f(r)),
        ExprKind::IfElse(c, a, b) => ExprKind::IfElse(f(c), f(a), f(b)),
        ExprKind::Phrase(ps) => ExprKind::Phrase(parts(ps)),
        ExprKind::Existing(ps) => ExprKind::Existing(parts(ps)),
        k => k.clone(),
    };
    Expr { kind, line: e.line }
}

/// the streams an expression names, each once, in the order met
pub fn seqs_in(e: &Expr, out: &mut Vec<String>) {
    let mut each = |x: &Expr| seqs_in(x, out);
    match &e.kind {
        ExprKind::Seq(n) | ExprKind::Arr(n) => {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
        ExprKind::Unit(x, _) | ExprKind::Neg(x) | ExprKind::Field(x, _) => each(x),
        ExprKind::List(items) => items.iter().for_each(each),
        ExprKind::Range { from: l, to: r, .. } | ExprKind::Bin(_, l, r) | ExprKind::Index(l, r) => {
            each(l);
            each(r);
        }
        ExprKind::IfElse(c, a, b) => {
            each(c);
            each(a);
            each(b);
        }
        ExprKind::Phrase(ps) | ExprKind::Existing(ps) => {
            for p in ps {
                match p {
                    Part::Args(list) => list.iter().for_each(|a| each(&a.value)),
                    Part::Value(x) => each(x),
                    Part::Word(_) => {}
                }
            }
        }
        _ => {}
    }
}

/// the words a phrase stops at unless a declared name has them there.
/// `times` is one since fm3 log 147: a push's count, `x$ << item (n)
/// times`, and a word of a name only where a declared name has it
const JOINERS: [&str; 4] = ["and", "or", "when", "times"];

/// the words that end a phrase wherever they stand: a statement's own.
/// `if` is one since fm3 question 79, the word a push takes after its
/// items, `x$ << item if (c)`, and `until` since log 148
const ENDS_PHRASE: [&str; 8] = ["then", "else", "while", "until", "if", "forever", "merge", "in"];

const COUNT_FORM: &str = "a push's count is the bracketed group before `times`, after the item: `x$ << item (n) times`";

const NO_WHEN: &str = "`when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`";

fn join_key(w: &str, before: &[String]) -> String {
    format!("\u{0}{} {}", w, before.join(" "))
}

/// the words after a push's items: `if (c)`, and how often
struct PushWords {
    only: Option<Expr>,
    cond: Option<Expr>,
    word: Repeat,
    forever: bool,
}

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    file: &'a str,
    types: &'a HashSet<String>,
    /// how deep in `[ ]` the parser is: only there do `to` and
    /// `through` end a phrase (log 15)
    ranges: usize,
    /// inside a loop's header line, where `yields` ends a phrase
    header: usize,
    /// the source's lines, for a refusal that shows the line as it is
    /// to be written; none for a case line
    lines: Vec<&'a str>,
    /// the results, those with no `$`, of the function whose body is
    /// being read: each is given by pushing it, and `y = value` on
    /// one is refused (fm3 question 77, log 153)
    results: Vec<String>,
    /// ... and its results that are arrays, `on (int r[]) << ...`:
    /// `r[] << value` gives one, as `y << value` gives a plain one
    arr_results: Vec<String>,
    /// was the name `expect_name` last read written `a[]`?
    arr: bool,
}

pub fn parse_feature<'a>(name: &str, src: &'a str, file: &'a str, types: &'a HashSet<String>) -> Result<Feature, Error> {
    let toks = lex::lex(src, file)?;
    let mut p = Parser { toks, pos: 0, file, types, ranges: 0, header: 0, lines: src.lines().collect(), results: Vec::new(), arr_results: Vec::new(), arr: false };
    let mut decls = Vec::new();
    while !p.at_end() {
        decls.push(p.parse_decl()?);
    }
    Ok(Feature { name: name.to_string(), file: file.to_string(), decls })
}

/// a `## testing` line's call, or the argument of `probe zero ... run`
pub fn parse_call(text: &str, file: &str, line: usize, types: &HashSet<String>) -> Result<Expr, Error> {
    let mut toks = Vec::new();
    lex::lex_line(text, line, file, &mut toks)?;
    toks.push(Token { tok: Tok::Newline, line });
    let mut p = Parser { toks, pos: 0, file, types, ranges: 0, header: 0, lines: Vec::new(), results: Vec::new(), arr_results: Vec::new(), arr: false };
    let e = p.parse_expr()?;
    if !p.at(&Tok::Newline) {
        return Err(p.err("the call has something after it"));
    }
    Ok(e)
}

impl<'a> Parser<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).or(self.toks.last()).map(|t| t.line).unwrap_or(0)
    }

    fn err(&self, msg: impl Into<String>) -> Error {
        lex::error(self.file, self.line(), msg)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.pos + n).map(|t| &t.tok)
    }

    fn at(&self, t: &Tok) -> bool {
        self.peek() == Some(t)
    }

    fn at_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x == w)
    }

    fn at_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if *x == s)
    }

    fn next(&mut self) -> Result<Tok, Error> {
        let t = self.peek().cloned().ok_or_else(|| self.err("the file ended early"))?;
        self.pos += 1;
        Ok(t)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        if self.at_sym(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.at_word(w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, s: &str) -> Result<(), Error> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            Err(self.err(format!("expected '{}', found {}", s, self.found())))
        }
    }

    fn expect_word(&mut self) -> Result<String, Error> {
        match self.next()? {
            Tok::Word(w) => Ok(w),
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a name, found {}", t)))
            }
        }
    }

    fn expect_newline(&mut self) -> Result<(), Error> {
        if self.eat(&Tok::Newline) {
            Ok(())
        } else if self.at_word("when") {
            // the word a push took before fm3 question 79
            Err(self.err(NO_WHEN))
        } else if self.at_word("times") || self.count_ahead(None) {
            Err(self.err("`times` is a word a push takes, `x$ << item (n) times`"))
        } else {
            Err(self.err(format!("expected the end of the line, found {}", self.found())))
        }
    }

    fn found(&self) -> String {
        match self.peek() {
            Some(t) => format!("{}", t),
            None => "the end of the file".into(),
        }
    }

    fn is_type(&self, w: &str) -> bool {
        BUILTIN_TYPES.contains(&w) || self.types.contains(w)
    }

    // --- declarations ---

    /// `int[] a`: the mark written on the type
    fn mark_on_type(&self, ty: &str) -> Error {
        let name = match self.peek_at(1) {
            Some(Tok::Word(n)) => n.clone(),
            _ => "a".to_string(),
        };
        self.err(format!("'{}[]': the mark is part of the name and not of the type, wherever the name is written (fm3 question 90): write `{} {}[]`", ty, ty, name))
    }

    fn parse_decl(&mut self) -> Result<Decl, Error> {
        if let Some(Tok::Arr(w)) = self.peek().cloned() {
            if self.is_type(&w) {
                return Err(self.mark_on_type(&w));
            }
        }
        match self.peek() {
            Some(Tok::Word(w)) if w == "on" => self.parse_fn().map(Decl::Fn),
            Some(Tok::Word(w)) if w == "type" => self.parse_type().map(Decl::Type),
            Some(Tok::Word(w)) if SCOPES.contains(&w.as_str()) || self.is_type(w) => {
                let v = self.parse_var()?;
                self.expect_newline()?;
                Ok(Decl::Var(v))
            }
            Some(Tok::Word(w)) if w == "feature" => Err(self.err("a feature's name and parent are in its .md, not its code")),
            Some(Tok::Word(w)) if w == "platform" => Err(self.err("a platform body follows the function it gives a body to")),
            Some(Tok::Indent) => Err(self.err("an indented line outside any declaration")),
            // `write(out$)`: a sink wired at feature scope (log 57)
            Some(Tok::Word(_)) => {
                let e = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Decl::Wire(e))
            }
            // `out$ << i$`: an edge (log 72)
            Some(Tok::Seq(_)) | Some(Tok::Arr(_)) if matches!(self.peek_at(1), Some(Tok::Sym("<<"))) => {
                let line = self.line();
                let began = self.pos;
                let target = self.parse_primary()?;
                let (items, group) = self.parse_pushes()?;
                if items.is_empty() {
                    return Err(self.err("nothing to push: an edge is `out$ << i$`"));
                }
                let shown = if group == 1 && items.len() > 1 { self.bracketed(line, self.pos - began) } else { None };
                let PushWords { only, cond, word, forever } = self.push_words()?;
                self.expect_newline()?;
                // a line that stands whose first item is an expression
                // (fm3 question 80, log 149): the one stream it names
                // other than its own target paces it, and stays the
                // first item; the expression is kept beside it, that
                // stream's name read as the item that has arrived
                let mut items = items;
                let mut first = None;
                if (forever || matches!(word, Repeat::Times | Repeat::Until) && cond.is_some()) && !matches!(items[0].kind, ExprKind::Seq(_)) {
                    let mut named = Vec::new();
                    seqs_in(&items[0], &mut named);
                    named.retain(|n| !matches!(&target.kind, ExprKind::Seq(t) if t == n));
                    if let [x] = named.as_slice() {
                        first = Some(as_item(&items[0], x));
                        items[0] = Expr { kind: ExprKind::Seq(x.clone()), line: items[0].line };
                    }
                }
                // in the condition of a standing filter the source's
                // own name is the item that has arrived
                let only = match (only, items.first().map(|e| &e.kind)) {
                    (Some(c), Some(ExprKind::Seq(s))) => Some(as_item(&c, s)),
                    (c, _) => c,
                };
                // ... and so it is in the `until` of a line that
                // stands, asked after the item has gone out (log 148)
                let cond = match (cond, word, items.first().map(|e| &e.kind)) {
                    (Some(c), Repeat::Until, Some(ExprKind::Seq(s))) => Some(as_item(&c, s)),
                    (c, _, _) => c,
                };
                Ok(Decl::Edge { target, items, group, shown, first, cond, word, only, forever, line })
            }
            _ => Err(self.err(format!("expected 'on', 'type', a variable declaration or a wiring at the top of the feature, found {}", self.found()))),
        }
    }

    fn parse_fn(&mut self) -> Result<FnDecl, Error> {
        let line = self.line();
        self.expect_word()?; // on
        let mut results = Vec::new();
        let mut task = false;
        // `on (results) << name`, or the form before fm3 question 77,
        // `on (results) = name`, or `on name` with no results — told
        // apart by what follows the first group
        let has_results = self.at_sym("(") && {
            let mut depth = 0;
            let mut i = self.pos;
            loop {
                match self.toks.get(i).map(|t| &t.tok) {
                    Some(Tok::Sym("(")) => depth += 1,
                    Some(Tok::Sym(")")) => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Some(Tok::Newline) | None => break,
                    _ => {}
                }
                i += 1;
            }
            matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Sym("=")) | Some(Tok::Sym("<<")))
        };
        let mut name = Vec::new();
        let mut groups = Vec::new();
        if has_results {
            results = self.parse_params()?;
            if self.eat_sym("<<") {
                // a `<<` method (log 59): `on (uint8 o$) << (int x)`, one
                // bracketed group after the `<<` and nothing else, where
                // a task has its name word; the stream is a parameter
                // and there is no result, a push moving no reader
                if self.at_sym("(") && self.group_ends_line() {
                    groups.push(std::mem::take(&mut results));
                    name.push(NamePart::Group);
                    name.push(NamePart::Sym("<<".into()));
                } else {
                    // the names' own marks say which (fm3 log 151):
                    // a `$` on a result is a stream produced over
                    // time, and no `$` a value given once
                    task = results.iter().any(|r| r.seq && !r.arr);
                }
            } else {
                // the form before fm3 question 77 is refused (log 153),
                // and since an array has its mark (question 87, log
                // 161) so is the one shape that kept it, a function
                // that gives a sequence whole
                let shown = self.respelt(line, ") = ", ") << ", "on (results) << name (parameters)");
                if let Some(r) = results.iter().find(|r| r.seq) {
                    let shown = shown.replace(&format!("{}$", r.name), &format!("{}[]", r.name));
                    return Err(self.err(format!("a function that gives an array says so on its result, `{} {}[]`, and gives it by pushing it, once; `=` says what a name is (fm3 questions 77, 87 and 90). Write `{}`", r.ty, r.name, shown)));
                }
                return Err(self.err(format!("a function is declared with `<<` and gives its result by pushing it; `=` says what a name is (fm3 question 77). Write `{}`", shown)));
            }
        }
        while !self.at(&Tok::Newline) {
            match self.peek().cloned() {
                // a word that ends a phrase wherever it stands would
                // end every call of this name at itself (fm3 log 126)
                Some(Tok::Word(w)) if ENDS_PHRASE.contains(&w.as_str()) => {
                    return Err(self.err(format!("'{}' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written", w)));
                }
                Some(Tok::Word(w)) => {
                    self.pos += 1;
                    name.push(NamePart::Word(w));
                }
                Some(Tok::Sym("(")) => {
                    groups.push(self.parse_params()?);
                    name.push(NamePart::Group);
                }
                Some(Tok::Sym(s)) if !matches!(s, ")" | "," | "=" | "<<") => {
                    self.pos += 1;
                    name.push(NamePart::Sym(s.to_string()));
                }
                _ => return Err(self.err(format!("unexpected {} in a function's name", self.found()))),
            }
        }
        if name.is_empty() {
            return Err(self.err("a function needs a name"));
        }
        if !name.iter().any(|p| matches!(p, NamePart::Word(_) | NamePart::Sym(_))) {
            return Err(self.err("a function's name needs a word or a symbol"));
        }
        self.expect_newline()?;
        self.results = results.iter().filter(|r| !r.seq).map(|r| r.name.clone()).collect();
        self.arr_results = if task { Vec::new() } else { results.iter().filter(|r| r.arr).map(|r| r.name.clone()).collect() };
        let body = if self.at(&Tok::Indent) { self.parse_block() } else { Ok(Vec::new()) };
        self.results.clear();
        self.arr_results.clear();
        let body = body?;
        let mut platform = Vec::new();
        while self.at_word("platform") {
            self.pos += 1;
            let mut kinds = Vec::new();
            while let Some(Tok::Word(k)) = self.peek().cloned() {
                self.pos += 1;
                kinds.push(k);
            }
            if kinds.is_empty() {
                return Err(self.err("'platform' needs the kind of place the body is for"));
            }
            self.expect_newline()?;
            let mut lines = Vec::new();
            // a platform body is foreign text: the lexer hands its lines
            // over as written (log 31)
            if self.eat(&Tok::Indent) {
                loop {
                    match self.next()? {
                        Tok::Raw(l) => {
                            lines.push(l);
                            self.expect_newline()?;
                        }
                        Tok::Dedent => break,
                        t => return Err(self.err(format!("unexpected {} in a platform body", t))),
                    }
                }
            }
            platform.push((kinds, lines));
        }
        Ok(FnDecl { line, results, name, groups, task, body, platform })
    }

    /// a line of the source with its `=` written `<<`, for a refusal to
    /// show; `or` where the source is not to hand
    fn respelt(&self, line: usize, from: &str, to: &str, or: &str) -> String {
        match line.checked_sub(1).and_then(|i| self.lines.get(i)) {
            Some(l) if l.contains(from) => l.trim().replacen(from, to, 1),
            _ => or.to_string(),
        }
    }

    /// A line at feature scope as its text would be with brackets
    /// round all its items, `out$ << (i$ << "\n") forever` (fm3
    /// question 84, log 156): the items are the line's first `k`
    /// tokens less the target and its `<<`, found in the source's own
    /// text as the longest start of it that is `k` tokens. None where
    /// the text is not to hand
    fn bracketed(&self, line: usize, k: usize) -> Option<String> {
        let text = line.checked_sub(1).and_then(|i| self.lines.get(i))?.trim_end();
        let open = text.find("<<")? + 2;
        let cut = (open..=text.len()).rev().filter(|&c| text.is_char_boundary(c)).find(|&c| {
            let mut toks = Vec::new();
            lex::lex_line(&text[..c], line, self.file, &mut toks).is_ok() && toks.len() == k
        })?;
        Some(format!("{} ({}) {}", &text[..open], text[open..cut].trim(), text[cut..].trim_start()).trim_end().to_string())
    }

    /// is the bracketed group at the cursor the last thing on the line?
    fn group_ends_line(&self) -> bool {
        let mut depth = 0;
        let mut i = self.pos;
        loop {
            match self.toks.get(i).map(|t| &t.tok) {
                Some(Tok::Sym("(")) => depth += 1,
                Some(Tok::Sym(")")) => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Newline) | None);
                    }
                }
                Some(Tok::Newline) | None => return false,
                _ => {}
            }
            i += 1;
        }
    }

    /// `(T a, T b)`, `(T a, b)`, `(T a$)`, `()`
    fn parse_params(&mut self) -> Result<Vec<Param>, Error> {
        self.expect_sym("(")?;
        let mut out = Vec::new();
        let mut ty: Option<String> = None;
        while !self.at_sym(")") {
            let line = self.line();
            if let Some(Tok::Arr(w)) = self.peek().cloned() {
                if self.is_type(&w) {
                    return Err(self.mark_on_type(&w));
                }
            }
            match self.next()? {
                Tok::Word(w) if self.is_type(&w) || ty.is_none() => {
                    if !self.is_type(&w) {
                        self.pos -= 1;
                        return Err(self.err(format!("'{}' is not a type: a parameter is its type then its name", w)));
                    }
                    ty = Some(w);
                    let (name, seq) = self.expect_name()?;
                    out.push(Param { ty: ty.clone().unwrap(), name, seq, arr: self.arr, line });
                }
                Tok::Word(w) => out.push(Param { ty: ty.clone().unwrap(), name: w, seq: false, arr: false, line }),
                Tok::Seq(w) => out.push(Param { ty: ty.clone().unwrap(), name: w, seq: true, arr: false, line }),
                Tok::Arr(w) => out.push(Param { ty: ty.clone().unwrap(), name: w, seq: true, arr: true, line }),
                t => {
                    self.pos -= 1;
                    return Err(self.err(format!("expected a parameter, found {}", t)));
                }
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(out)
    }

    /// a name and whether it is a sequence's; `self.arr` says whether
    /// it was written with an array's mark
    fn expect_name(&mut self) -> Result<(String, bool), Error> {
        self.arr = false;
        match self.next()? {
            Tok::Word(w) => Ok((w, false)),
            Tok::Seq(w) => Ok((w, true)),
            Tok::Arr(w) => {
                self.arr = true;
                Ok((w, true))
            }
            t => {
                self.pos -= 1;
                Err(self.err(format!("expected a name, found {}", t)))
            }
        }
    }

    fn parse_type(&mut self) -> Result<TypeDecl, Error> {
        let line = self.line();
        self.expect_word()?; // type
        let name = self.expect_word()?;
        if self.eat_sym("+=") {
            return Err(self.err("type extension ('+=') is not in this milestone"));
        }
        self.expect_sym("=")?;
        if self.at_word("pack") {
            return Err(self.err("packs are not in this milestone"));
        }
        // an enumeration: words separated by `|`
        if matches!(self.peek(), Some(Tok::Word(_))) && matches!(self.peek_at(1), Some(Tok::Sym("|"))) {
            let mut cases = vec![self.expect_word()?];
            while self.eat_sym("|") {
                cases.push(self.expect_word()?);
            }
            self.expect_newline()?;
            return Ok(TypeDecl { line, name, kind: TypeKind::Enum(cases) });
        }
        let mut fields = Vec::new();
        if self.eat(&Tok::Newline) {
            if !self.eat(&Tok::Indent) {
                return Err(self.err(format!("type {} needs its fields, indented", name)));
            }
            while !self.eat(&Tok::Dedent) {
                self.parse_fields(&mut fields)?;
                self.expect_newline()?;
            }
        } else {
            self.parse_fields(&mut fields)?;
            self.expect_newline()?;
        }
        Ok(TypeDecl { line, name, kind: TypeKind::Struct(fields) })
    }

    /// one line of fields: `float x, y, z = 0`
    fn parse_fields(&mut self, out: &mut Vec<Field>) -> Result<(), Error> {
        let line = self.line();
        let ty = self.expect_word()?;
        if !self.is_type(&ty) {
            self.pos -= 1;
            return Err(self.err(format!("'{}' is not a type: a field is its type then its names", ty)));
        }
        let mut names = Vec::new();
        loop {
            names.push(self.expect_name()?);
            if self.arr {
                self.pos -= 1;
                return Err(self.err("an array as a field of a struct is not built"));
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        let default = if self.eat_sym("=") { Some(self.parse_expr()?) } else { None };
        for (name, seq) in names {
            out.push(Field { ty: ty.clone(), name, seq, default: default.clone(), line });
        }
        Ok(())
    }

    /// `[static|device|group] T name [= expr | (args) | << ...] [merge word]`
    fn parse_var(&mut self) -> Result<VarDecl, Error> {
        let line = self.line();
        let mut scope = Vec::new();
        while let Some(Tok::Word(w)) = self.peek() {
            if SCOPES.contains(&w.as_str()) {
                scope.push(w.clone());
                self.pos += 1;
            } else {
                break;
            }
        }
        let ty = self.expect_word()?;
        if !self.is_type(&ty) {
            self.pos -= 1;
            return Err(self.err(format!("'{}' is not a type", ty)));
        }
        let (name, seq) = self.expect_name()?;
        let arr = self.arr;
        // `T x$ at (n hz)`: a stream at a rate, section 9
        let rate = if seq && self.eat_word("at") {
            let args = self.parse_args()?;
            let [Arg { name: None, value }] = args.as_slice() else {
                return Err(self.err("a rate is `at (n hz)`"));
            };
            Some(value.clone())
        } else {
            None
        };
        let init = if self.eat_sym("=") {
            Some(Init::Value(self.parse_expr()?))
        } else if self.at_sym("(") {
            Some(Init::Construct(self.parse_args()?))
        } else if self.at_sym("<<") {
            let (items, group) = self.parse_pushes()?;
            if self.at_word("forever") {
                return Err(self.err(format!("`forever` on a declaration is not built: declare the stream and wire it on a line of its own, `{}$ << x$ forever`", name)));
            }
            if self.at_word("if") {
                return Err(self.err(format!("a declaration's items take no `if`: declare the stream, `{} {}$`, and push on a line of its own", ty, name)));
            }
            let PushWords { cond, word, .. } = self.push_words()?;
            Some(Init::Pushes { items, group, cond, word })
        } else {
            None
        };
        let merge = if self.eat_word("merge") { Some(self.expect_word()?) } else { None };
        Ok(VarDecl { line, scope, ty, name, seq, arr, init, merge, rate })
    }

    /// `<< a << b`; a bare `<<` at the end of the line is refused by
    /// the lowering (log 38). The words after the items are
    /// `push_words`'. A bracket with a `<<` directly inside it is a
    /// group of items, the one item the push's word applies to (fm3
    /// question 84, log 155): `x$ << (a << b) (3) times`. Its items
    /// join the one list, and the number given back with the list is
    /// how many of its last the word covers, 1 with nothing bracketed.
    /// A group stands last in its chain
    fn parse_pushes(&mut self) -> Result<(Vec<Expr>, usize), Error> {
        let mut items = Vec::new();
        let mut group = 1;
        while self.eat_sym("<<") {
            if self.at(&Tok::Newline) {
                break;
            }
            if !self.group_ahead() {
                items.push(self.parse_expr()?);
                continue;
            }
            self.pos += 1;
            let before = items.len();
            loop {
                if self.group_ahead() {
                    return Err(self.err("a group of items inside a group: one pair of brackets says it, `x$ << (a << b << c) (3) times`"));
                }
                items.push(self.parse_expr()?);
                if !self.eat_sym("<<") {
                    break;
                }
            }
            self.expect_sym(")")?;
            group = items.len() - before;
            if self.at_sym("<<") {
                return Err(self.err("brackets round several items of a push make them the one item its word applies to, and they stand last in the chain (fm3 question 84): `x$ << a << (b << c) (3) times`. Before the last item a group would be its items in order and nothing more: write them without the brackets"));
            }
            let follows = match self.peek() {
                Some(Tok::Newline) | Some(Tok::Sym(")")) | Some(Tok::Sym(",")) | None => true,
                Some(Tok::Word(w)) => matches!(w.as_str(), "if" | "while" | "until" | "forever" | "times" | "merge"),
                Some(Tok::Sym("(")) => self.count_ahead(None),
                _ => false,
            };
            if !follows {
                return Err(self.err("brackets round several items of a push make them one item for the word that follows, `x$ << (a << b) (3) times`: a group is not a value, and what may follow it is `if`, `(n) times`, `while`, `until` or `forever`"));
            }
        }
        Ok((items, group))
    }

    /// Is the bracket at the cursor a group of a push's items (fm3 log
    /// 155)? It is where a `<<` stands inside it at its own depth, in
    /// no deeper bracket: `<<` is no operator of an expression, so a
    /// bracketed value never has one there
    fn group_ahead(&self) -> bool {
        let mut depth = 0;
        let mut i = self.pos;
        loop {
            match self.toks.get(i).map(|t| &t.tok) {
                Some(Tok::Sym("(")) | Some(Tok::Sym("[")) => depth += 1,
                Some(Tok::Sym(")")) | Some(Tok::Sym("]")) => {
                    depth -= 1;
                    if depth <= 0 {
                        return false;
                    }
                }
                Some(Tok::Sym("<<")) if depth == 1 => return matches!(self.peek(), Some(Tok::Sym("("))),
                Some(Tok::Newline) | None => return false,
                _ if depth == 0 => return false,
                _ => {}
            }
            i += 1;
        }
    }

    /// The words after a push's items (fm3 question 79): `if (c)`, the
    /// push made where the condition holds, and then one of `(n)
    /// times`, `while (c)`, `until (c)` and `forever`, how often. `if`
    /// comes first and goes with any of them; two of the four on one
    /// push are refused, each pair by name
    fn push_words(&mut self) -> Result<PushWords, Error> {
        // a `times` with no bracket before it, the item's own included
        if self.at_word("times") {
            return Err(self.err(COUNT_FORM));
        }
        // the item ended, outside any bracket of its own, in a group
        // and a declared name's `times`, and no count follows: the call
        // pushed once, or what stands before the group pushed that
        // often (fm3 log 147)
        let ended = |k: usize| self.pos.checked_sub(k).and_then(|i| self.toks.get(i)).map(|t| &t.tok);
        if matches!(ended(1), Some(Tok::Word(w)) if w == "times") && matches!(ended(2), Some(Tok::Sym(")"))) && !self.count_ahead(None) {
            return Err(self.err("'... (k) times' at the end of a push reads two ways: a function whose name ends `(...) times`, called and pushed once, or what stands before the bracket pushed that many times. For the call put it in brackets, `x$ << (name (k) times)`; for the count put the item in brackets, `x$ << (item) (k) times`"));
        }
        let mut only = None;
        if self.eat_word("if") {
            only = Some(self.parse_expr()?);
            if self.at_word("then") {
                return Err(self.err("an `if` after a push's items says whether the push happens, and takes no `then`: the value that is one thing or another is written first, `x$ << if (c) then (a) else (b)`"));
            }
        }
        let (mut cond, mut word, mut forever) = (None, Repeat::While, false);
        let first = self.repeat_ahead();
        match first {
            Some("`(n) times`") => {
                let args = self.parse_args()?;
                let [Arg { name: None, value }] = args.as_slice() else {
                    return Err(self.err(COUNT_FORM));
                };
                self.pos += 1;
                (cond, word) = (Some(value.clone()), Repeat::Times);
            }
            Some("`while`") => {
                self.pos += 1;
                cond = Some(self.parse_expr()?);
            }
            Some("`until`") => {
                self.pos += 1;
                (cond, word) = (Some(self.parse_expr()?), Repeat::Until);
            }
            Some(_) => {
                self.pos += 1;
                forever = true;
            }
            None => {}
        }
        if let Some(a) = first {
            if self.at_word("times") {
                return Err(self.err(COUNT_FORM));
            }
            if let Some(b) = self.repeat_ahead() {
                return Err(self.err(match (a, b) {
                    ("`while`", "`forever`") | ("`forever`", "`while`") => "a push takes `while` or `forever`, not both: `while` is `forever` with an end".to_string(),
                    _ if a == b => format!("a push takes one {}", a),
                    _ => format!("a push takes {} or {}, not both", a, b),
                }));
            }
            if self.at_word("if") {
                return Err(self.err(match a {
                    "`forever`" => "`forever` is the last word of its line: `x$ << item if (condition) forever`".to_string(),
                    "`(n) times`" => "`if` comes first on a push, then how often: `x$ << item if (condition) (n) times`".to_string(),
                    _ => format!("`if` comes first on a push, then how often: `x$ << item if (condition) {} (...)`", a.trim_matches('`')),
                }));
            }
        }
        Ok(PushWords { only, cond, word, forever })
    }

    /// which of a push's words of how often stands at the cursor
    fn repeat_ahead(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Sym("(")) if self.count_ahead(None) => Some("`(n) times`"),
            Some(Tok::Word(w)) if w == "while" => Some("`while`"),
            Some(Tok::Word(w)) if w == "until" => Some("`until`"),
            Some(Tok::Word(w)) if w == "forever" => Some("`forever`"),
            _ => None,
        }
    }

    /// Is the bracketed group at the cursor a push's count (fm3 log
    /// 147)? It is where the word after its `)` is `times`, unless a
    /// declared name has `times` after the words `before` of the
    /// phrase the group would belong to
    fn count_ahead(&self, before: Option<&[String]>) -> bool {
        let mut depth = 0;
        let mut i = self.pos;
        loop {
            match self.toks.get(i).map(|t| &t.tok) {
                Some(Tok::Sym("(")) => depth += 1,
                Some(Tok::Sym(")")) => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Word(w)) if w == "times") && !before.is_some_and(|b| self.joins("times", b));
                    }
                    if depth < 0 {
                        return false;
                    }
                }
                Some(Tok::Newline) | None => return false,
                _ if depth == 0 => return false,
                _ => {}
            }
            i += 1;
        }
    }

    // --- statements ---

    fn parse_block(&mut self) -> Result<Vec<Stmt>, Error> {
        if !self.eat(&Tok::Indent) {
            return Err(self.err(format!("expected an indented block, found {}", self.found())));
        }
        let mut out = Vec::new();
        while !self.eat(&Tok::Dedent) {
            if self.at_end() {
                return Err(self.err("the file ended inside a block"));
            }
            out.push(self.parse_stmt()?);
        }
        Ok(out)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Error> {
        let line = self.line();
        let first = self.peek().cloned();
        match first {
            Some(Tok::Arr(w)) if self.is_type(&w) => Err(self.mark_on_type(&w)),
            Some(Tok::Word(w)) if w == "if" => {
                self.pos += 1;
                let cond = self.parse_expr()?;
                self.expect_newline()?;
                let then = self.parse_block()?;
                let els = if self.eat_word("else") {
                    if self.at_word("if") {
                        Some(vec![self.parse_stmt()?])
                    } else {
                        self.expect_newline()?;
                        Some(self.parse_block()?)
                    }
                } else {
                    None
                };
                Ok(Stmt::If { cond, then, els, line, on_push: false })
            }
            Some(Tok::Word(w)) if w == "loop" => {
                self.pos += 1;
                self.parse_loop(None, line)
            }
            Some(Tok::Word(w)) if w == "for" => {
                self.pos += 1;
                self.expect_sym("(")?;
                let var = self.expect_word()?;
                if !self.eat_word("in") {
                    return Err(self.err("'for' is 'for (x in items$)'"));
                }
                let seq = self.parse_expr()?;
                self.expect_sym(")")?;
                self.expect_newline()?;
                let body = self.parse_block()?;
                Ok(Stmt::For { var, seq, body, line })
            }
            Some(Tok::Word(w)) if w == "continue" => {
                self.pos += 1;
                let mut values = Vec::new();
                if self.eat_sym("(") {
                    while !self.at_sym(")") {
                        values.push(self.parse_expr()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                }
                self.expect_newline()?;
                Ok(Stmt::Continue { values, line })
            }
            Some(Tok::Word(w)) if w == "break" => {
                self.pos += 1;
                // `break (values)` (fm3 question 81): the loop's result
                // given where it leaves, as `continue (values)` gives
                // the next pass its own
                let mut values = Vec::new();
                if self.eat_sym("(") {
                    while !self.at_sym(")") {
                        values.push(self.parse_expr()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                }
                self.expect_newline()?;
                Ok(Stmt::Break { values, line })
            }
            Some(Tok::Word(w)) if w == "check" => {
                self.pos += 1;
                let cond = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt::Check { cond, line })
            }
            Some(Tok::Word(w)) if SCOPES.contains(&w.as_str()) => Err(self.err(format!("'{}' belongs on a feature-scope variable, outside any function", w))),
            // `int total = loop (...) ... yields acc`: a loop's results declared
            Some(Tok::Word(w)) if self.is_type(&w) && self.loop_ahead() => {
                let mut vars = Vec::new();
                loop {
                    let pline = self.line();
                    let ty = self.expect_word()?;
                    if !self.is_type(&ty) {
                        self.pos -= 1;
                        return Err(self.err(format!("'{}' is not a type: each of a loop's results is its type then its name", ty)));
                    }
                    let (name, seq) = self.expect_name()?;
                    vars.push(Param { ty, name, seq, arr: self.arr, line: pline });
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym("=")?;
                self.expect_word()?;
                self.parse_loop(Some(LoopInto::Declare(vars)), line)
            }
            Some(Tok::Word(w)) if self.is_type(&w) && matches!(self.peek_at(1), Some(Tok::Word(_)) | Some(Tok::Seq(_)) | Some(Tok::Arr(_))) => {
                let v = self.parse_var()?;
                // `int q, int r = ...`: several typed names, one initializer
                if self.at_sym(",") && v.init.is_none() {
                    let mut vars = vec![Param { ty: v.ty.clone(), name: v.name.clone(), seq: v.seq, arr: v.arr, line: v.line }];
                    while self.eat_sym(",") {
                        let pline = self.line();
                        let ty = self.expect_word()?;
                        if !self.is_type(&ty) {
                            self.pos -= 1;
                            return Err(self.err(format!("'{}' is not a type: each of several results is its type then its name", ty)));
                        }
                        let (name, seq) = self.expect_name()?;
                        vars.push(Param { ty, name, seq, arr: self.arr, line: pline });
                    }
                    self.expect_sym("=")?;
                    let value = self.parse_expr()?;
                    self.expect_newline()?;
                    return Ok(Stmt::Multi { vars, value, line });
                }
                self.expect_newline()?;
                Ok(Stmt::Var(v))
            }
            // `y << value`, `q, r << call`, `n << loop (...)`: a
            // function's results given by pushing them (fm3 question
            // 77, log 151). Whether each name is a result is the
            // lowering's to say; one line gives each once
            Some(Tok::Word(_)) if self.gives_ahead() => {
                let mut targets = Vec::new();
                loop {
                    let tline = self.line();
                    let name = self.expect_word()?;
                    targets.push(Target { name, seq: false, arr: false, line: tline, feature: None, pushed: true });
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym("<<")?;
                if self.eat_word("loop") {
                    return self.parse_loop(Some(LoopInto::Assign(targets)), line);
                }
                let said = targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ");
                let one = if targets.len() == 1 { format!("'{}' is one value", said) } else { format!("'{}' are one value each", said) };
                let stream = format!("`{}$`", targets[0].name);
                let value = self.parse_expr()?;
                if self.at_sym("<<") {
                    return Err(self.err(format!("{}, given once: this line pushes it twice. What takes more than one item is a stream, {}", one, stream)));
                }
                let mut only = None;
                if self.eat_word("if") {
                    only = Some(self.parse_expr()?);
                    if self.at_word("then") {
                        return Err(self.err("an `if` after a pushed value says whether the push happens, and takes no `then`: the value that is one thing or another is written first, `y << if (c) then (a) else (b)`"));
                    }
                }
                let often = if self.at_word("times") { Some("`(n) times`") } else { self.repeat_ahead() };
                if let Some(w) = often {
                    let does = if w == "`forever`" { "make it stand and give it again and again" } else { "give it more than once" };
                    return Err(self.err(format!("{} on the push of '{}' would {}, and {}, given once: what takes more than one item is a stream, {}", w, said, does, one, stream)));
                }
                self.expect_newline()?;
                let give = Stmt::Assign { targets, value, line };
                return Ok(match only {
                    Some(cond) => Stmt::If { cond, then: vec![give], els: None, line, on_push: true },
                    None => give,
                });
            }
            // `r[] << value` where `r[]` is a result of this function:
            // the array given, once, as `y << value` gives a plain
            // result (fm3 question 87, log 159)
            Some(Tok::Arr(w)) if matches!(self.peek_at(1), Some(Tok::Sym("<<"))) && self.arr_results.contains(&w) => {
                self.pos += 2;
                let value = self.parse_expr()?;
                if self.at_sym("<<") {
                    return Err(self.err(format!("'{}[]' is one array, given once: this line pushes it twice. What takes items one after another is a stream, `{}$`", w, w)));
                }
                if let Some(word) = if self.at_word("times") { Some("`(n) times`") } else if self.at_word("if") { Some("`if`") } else { self.repeat_ahead() } {
                    return Err(self.err(format!("{} on the push of '{}[]' is not built: an array that is a function's result is given whole, once", word, w)));
                }
                self.expect_newline()?;
                Ok(Stmt::Assign { targets: vec![Target { name: w, seq: true, arr: true, line, feature: None, pushed: true }], value, line })
            }
            Some(Tok::Word(_)) | Some(Tok::Seq(_)) | Some(Tok::Arr(_)) if self.assignment_ahead() => {
                let mut targets = Vec::new();
                loop {
                    let tline = self.line();
                    let (mut name, seq) = self.expect_name()?;
                    let arr = self.arr;
                    let mut feature = None;
                    if !seq && self.eat_sym(".") {
                        feature = Some(name);
                        name = self.expect_word()?;
                    }
                    targets.push(Target { name, seq, arr, line: tline, feature, pushed: false });
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                // a result is given by pushing it (fm3 question 77,
                // log 153): no local or parameter may have a result's
                // name, so the name says it
                if let Some(t) = targets.iter().find(|t| t.feature.is_none() && t.arr && self.arr_results.contains(&t.name)) {
                    return Err(self.err(format!("'{}[]' is a result, and a result is given by pushing it, an array as any other; `=` says what a name is (fm3 questions 77 and 90). Write `{}`", t.name, self.respelt(line, " = ", " << ", &format!("{}[] << ...", t.name)))));
                }
                if let Some(t) = targets.iter().find(|t| t.feature.is_none() && !t.seq && self.results.contains(&t.name)) {
                    return Err(self.err(format!("'{}' is a result, and a result is given by pushing it; `=` says what a name is (fm3 question 77). Write `{}`", t.name, self.respelt(line, " = ", " << ", &format!("{} << ...", t.name)))));
                }
                self.expect_sym("=")?;
                // `total = loop (...) ... yields acc`: a loop's results assigned
                if self.eat_word("loop") {
                    return self.parse_loop(Some(LoopInto::Assign(targets)), line);
                }
                let value = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt::Assign { targets, value, line })
            }
            Some(Tok::Seq(_)) | Some(Tok::Arr(_)) if matches!(self.peek_at(1), Some(Tok::Sym("<<"))) => {
                let target = self.parse_primary()?;
                let (items, group) = self.parse_pushes()?;
                if items.is_empty() {
                    return Err(self.err("nothing to push: `x$ << item`"));
                }
                // `x$ << item if (c)`: the push where the condition
                // holds (fm3 question 75 rule 3; `when` until question
                // 79). An `if` here stands where a value has ended, so
                // it is the push's word; one that begins an item is the
                // expression, and `parse_primary` has taken it
                let PushWords { only, cond, word, forever } = self.push_words()?;
                self.expect_newline()?;
                if let Some(c) = only {
                    return Ok(Stmt::If { cond: c, then: vec![Stmt::Push { target, items, group, cond, word, existing: false, forever, line }], els: None, line, on_push: true });
                }
                Ok(Stmt::Push { target, items, group, cond, word, existing: false, forever, line })
            }
            // `existing o$ << x` inside a `<<` method: the definition
            // below this one in the chain, which no `existing name(...)`
            // can name, the method's name being an operator
            Some(Tok::Word(w)) if w == "existing" && matches!(self.peek_at(1), Some(Tok::Seq(_))) && matches!(self.peek_at(2), Some(Tok::Sym("<<"))) => {
                self.pos += 1;
                let target = self.parse_primary()?;
                let (items, _) = self.parse_pushes()?;
                if self.at_word("while") {
                    return Err(self.err("`existing x$ << item` takes no `while`: it calls the definition below once"));
                }
                if items.len() != 1 {
                    return Err(self.err("`existing x$ << item` passes one item to the definition below"));
                }
                self.expect_newline()?;
                Ok(Stmt::Push { target, items, group: 1, cond: None, word: Repeat::While, existing: true, forever: false, line })
            }
            Some(Tok::Indent) => Err(self.err("an indented line with nothing to belong to")),
            _ => {
                let expr = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt::Expr { expr, line })
            }
        }
    }

    /// `loop (vars) [while (c)] [yields x, y]`, then the body;
    /// `into` says where the given values go (log 40)
    fn parse_loop(&mut self, into: Option<LoopInto>, line: usize) -> Result<Stmt, Error> {
        let mut vars = Vec::new();
        if self.eat_sym("(") {
            while !self.at_sym(")") {
                vars.push(self.parse_var()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.expect_sym(")")?;
        }
        let mut cond = None;
        let mut yields = Vec::new();
        self.header += 1;
        loop {
            if self.eat_word("while") {
                cond = Some(self.parse_expr()?);
            } else if self.eat_word("yields") {
                loop {
                    yields.push(self.expect_word()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
            } else {
                break;
            }
        }
        self.header -= 1;
        self.expect_newline()?;
        let body = self.parse_block()?;
        Ok(Stmt::Loop { vars, cond, body, yields, into, line })
    }

    /// `T a[, T b] = loop` ahead on this line: a loop's results declared
    fn loop_ahead(&self) -> bool {
        let mut i = self.pos;
        loop {
            let (Some(Tok::Word(_)), Some(Tok::Word(_) | Tok::Seq(_) | Tok::Arr(_))) = (self.toks.get(i).map(|t| &t.tok), self.toks.get(i + 1).map(|t| &t.tok)) else {
                return false;
            };
            match self.toks.get(i + 2).map(|t| &t.tok) {
                Some(Tok::Sym("=")) => return matches!(self.toks.get(i + 3).map(|t| &t.tok), Some(Tok::Word(w)) if w == "loop"),
                Some(Tok::Sym(",")) => i += 3,
                _ => return false,
            }
        }
    }

    /// `y << ...` or `q, r << ...` ahead on this line: names with no
    /// `$`, then the push
    fn gives_ahead(&self) -> bool {
        let mut i = self.pos;
        loop {
            if !matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Word(_))) {
                return false;
            }
            match self.toks.get(i + 1).map(|t| &t.tok) {
                Some(Tok::Sym("<<")) => return true,
                Some(Tok::Sym(",")) => i += 2,
                _ => return false,
            }
        }
    }

    /// `a = ...`, `a, b = ...` or `f.enabled = ...` ahead on this line
    fn assignment_ahead(&self) -> bool {
        let mut i = self.pos;
        loop {
            match self.toks.get(i).map(|t| &t.tok) {
                Some(Tok::Word(_)) | Some(Tok::Seq(_)) | Some(Tok::Arr(_)) => {}
                _ => return false,
            }
            if matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Sym("."))) && matches!(self.toks.get(i + 2).map(|t| &t.tok), Some(Tok::Word(_))) {
                i += 2;
            }
            match self.toks.get(i + 1).map(|t| &t.tok) {
                Some(Tok::Sym("=")) => return true,
                Some(Tok::Sym(",")) => i += 2,
                _ => return false,
            }
        }
    }

    // --- expressions ---

    pub fn parse_expr(&mut self) -> Result<Expr, Error> {
        self.parse_or()
    }

    /// `a or b`, then `a and b`, both looser than a comparison (fm3
    /// question 66): two conditions joined, both always worked out
    fn parse_or(&mut self) -> Result<Expr, Error> {
        let mut l = self.parse_and()?;
        while self.at_word("or") {
            let line = self.line();
            self.pos += 1;
            let r = self.parse_and()?;
            l = Expr { kind: ExprKind::Bin("or".to_string(), Box::new(l), Box::new(r)), line };
        }
        Ok(l)
    }

    fn parse_and(&mut self) -> Result<Expr, Error> {
        let mut l = self.parse_compare()?;
        while self.at_word("and") {
            let line = self.line();
            self.pos += 1;
            let r = self.parse_compare()?;
            l = Expr { kind: ExprKind::Bin("and".to_string(), Box::new(l), Box::new(r)), line };
        }
        Ok(l)
    }

    /// is `w`, met in a phrase whose words so far are `before`, a word
    /// of some declared function's name there, and not the operator?
    fn joins(&self, w: &str, before: &[String]) -> bool {
        self.types.contains(&join_key(w, before))
    }

    /// does a word end a phrase as `and` or `or` between two conditions?
    /// `when` ends one the same way, so that a push written with it is
    /// met by its refusal and not read as a call that does not exist;
    /// it is a word of a name where a declared name has it (question 79)
    fn joiner(&self, w: &str, before: &[String]) -> bool {
        JOINERS.contains(&w) && !self.joins(w, before)
    }

    fn parse_compare(&mut self) -> Result<Expr, Error> {
        let mut l = self.parse_sum()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(s)) if matches!(*s, "<" | ">" | "<=" | ">=" | "==" | "!=") => *s,
                _ => break,
            };
            let line = self.line();
            self.pos += 1;
            let r = self.parse_sum()?;
            l = Expr { kind: ExprKind::Bin(op.to_string(), Box::new(l), Box::new(r)), line };
        }
        Ok(l)
    }

    fn parse_sum(&mut self) -> Result<Expr, Error> {
        let mut l = self.parse_product()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(s)) if matches!(*s, "+" | "-") => *s,
                _ => break,
            };
            let line = self.line();
            self.pos += 1;
            let r = self.parse_product()?;
            l = Expr { kind: ExprKind::Bin(op.to_string(), Box::new(l), Box::new(r)), line };
        }
        Ok(l)
    }

    fn parse_product(&mut self) -> Result<Expr, Error> {
        let mut l = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(s)) if matches!(*s, "*" | "/" | "%") => *s,
                _ => break,
            };
            let line = self.line();
            self.pos += 1;
            let r = self.parse_unary()?;
            l = Expr { kind: ExprKind::Bin(op.to_string(), Box::new(l), Box::new(r)), line };
        }
        Ok(l)
    }

    fn parse_unary(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        if self.eat_sym("-") {
            let e = self.parse_unary()?;
            return Ok(match e.kind {
                ExprKind::Int(v) => Expr { kind: ExprKind::Int(v.wrapping_neg()), line },
                ExprKind::Float(s) => Expr { kind: ExprKind::Float(format!("-{}", s)), line },
                _ => Expr { kind: ExprKind::Neg(Box::new(e)), line },
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, Error> {
        let e = self.parse_primary()?;
        self.postfix_of(e)
    }

    /// a value's postfixes: `.field`, `[i]` on a sequence, a unit
    fn postfix_of(&mut self, mut e: Expr) -> Result<Expr, Error> {
        loop {
            if self.at_sym(".") && matches!(self.peek_at(1), Some(Tok::Word(_))) {
                let line = self.line();
                self.pos += 1;
                let f = self.expect_word()?;
                e = Expr { kind: ExprKind::Field(Box::new(e), f), line };
            } else if self.at_sym("[") && matches!(e.kind, ExprKind::Seq(_) | ExprKind::Index(..)) {
                // an index, only straight after a sequence's name: a `[`
                // after anything else starts a list
                let line = self.line();
                self.pos += 1;
                let i = self.parse_expr()?;
                self.expect_sym("]")?;
                e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(i)), line };
            } else if let Some(Tok::Word(u)) = self.peek() {
                if UNITS.contains(&u.as_str()) && matches!(e.kind, ExprKind::Int(_) | ExprKind::Float(_)) {
                    let line = self.line();
                    let u = u.clone();
                    self.pos += 1;
                    e = Expr { kind: ExprKind::Unit(Box::new(e), u), line };
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let kind = match self.next()? {
            Tok::Int(v) => ExprKind::Int(v),
            Tok::Float(s) => ExprKind::Float(s),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Seq(w) => {
                // a phrase may begin with a sequence name when a word
                // follows it: `x$ behind (2)`, `x$ at (t)` (section 9)
                if matches!(self.peek(), Some(Tok::Word(v)) if !self.ends_phrase(v) && !self.joiner(v, &[]) && !UNITS.contains(&v.as_str())) {
                    let mut parts = vec![Part::Value(Expr { kind: ExprKind::Seq(w), line })];
                    parts.extend(self.parse_parts()?);
                    return Ok(Expr { kind: ExprKind::Phrase(parts), line });
                }
                ExprKind::Seq(w)
            }
            // an array's name, whole; a stream's word after it is
            // parsed as it is after a stream's and refused by its kind
            Tok::Arr(w) => {
                if matches!(self.peek(), Some(Tok::Word(v)) if !self.ends_phrase(v) && !self.joiner(v, &[]) && !UNITS.contains(&v.as_str())) {
                    let mut parts = vec![Part::Value(Expr { kind: ExprKind::Arr(w), line })];
                    parts.extend(self.parse_parts()?);
                    return Ok(Expr { kind: ExprKind::Phrase(parts), line });
                }
                ExprKind::Arr(w)
            }
            // `a[k]`: one item of an array, by its place
            Tok::At(w) => {
                self.expect_sym("[")?;
                let i = self.parse_expr()?;
                self.expect_sym("]")?;
                ExprKind::Index(Box::new(Expr { kind: ExprKind::Arr(w), line }), Box::new(i))
            }
            Tok::Sym("_") => ExprKind::Acc,
            Tok::Sym("(") => {
                // a phrase may begin with a bracket group — `(3) is less
                // than (4)` — when a word follows it; otherwise these are
                // parentheses around an expression
                self.pos -= 1;
                let at = self.pos;
                if let Ok(args) = self.parse_args() {
                    if matches!(self.peek(), Some(Tok::Word(w)) if !self.ends_phrase(w) && !self.joiner(w, &[]) && !UNITS.contains(&w.as_str())) {
                        let mut parts = vec![Part::Args(args)];
                        parts.extend(self.parse_parts()?);
                        return Ok(Expr { kind: ExprKind::Phrase(parts), line });
                    }
                }
                self.pos = at;
                self.expect_sym("(")?;
                let e = self.parse_expr()?;
                self.expect_sym(")")?;
                return Ok(e);
            }
            Tok::Sym("[") => {
                let mut items = Vec::new();
                if !self.at_sym("]") {
                    self.ranges += 1;
                    let first = self.parse_expr();
                    self.ranges -= 1;
                    let first = first?;
                    if self.at_word("through") || self.at_word("to") {
                        let inclusive = self.eat_word("through") || !self.eat_word("to");
                        let to = self.parse_expr()?;
                        self.expect_sym("]")?;
                        return Ok(Expr { kind: ExprKind::Range { from: Box::new(first), to: Box::new(to), inclusive }, line });
                    }
                    items.push(first);
                    while self.eat_sym(",") {
                        items.push(self.parse_expr()?);
                    }
                }
                self.expect_sym("]")?;
                ExprKind::List(items)
            }
            Tok::Word(w) => match w.as_str() {
                "true" => ExprKind::Bool(true),
                "false" => ExprKind::Bool(false),
                "if" => {
                    let c = self.parse_expr()?;
                    if !self.eat_word("then") {
                        return Err(self.err("'if' as an expression is 'if (c) then (a) else (b)'"));
                    }
                    let a = self.parse_expr()?;
                    if !self.eat_word("else") {
                        return Err(self.err("'if' as an expression needs its 'else'"));
                    }
                    let b = self.parse_expr()?;
                    ExprKind::IfElse(Box::new(c), Box::new(a), Box::new(b))
                }
                "existing" => ExprKind::Existing(self.parse_parts()?),
                _ => {
                    self.pos -= 1;
                    ExprKind::Phrase(self.parse_parts()?)
                }
            },
            t => {
                self.pos -= 1;
                return Err(self.err(format!("expected a value, found {}", t)));
            }
        };
        Ok(Expr { kind, line })
    }

    /// a word that ends a phrase: a statement's own word, or a range's
    /// `to` and `through` inside `[ ]`
    fn ends_phrase(&self, w: &str) -> bool {
        ENDS_PHRASE.contains(&w) || (self.ranges > 0 && matches!(w, "through" | "to")) || (self.header > 0 && w == "yields")
    }

    /// the parts of a phrase: words, bracketed argument groups, and bare
    /// arguments (a literal, a sequence name, a list), up to whatever
    /// ends an expression
    fn parse_parts(&mut self) -> Result<Vec<Part>, Error> {
        let mut parts = Vec::new();
        // the words of the phrase so far: `and` and `or` are words of
        // it only where a declared name has them after these
        let mut words: Vec<String> = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Tok::Word(w)) if !self.ends_phrase(&w) && !self.joiner(&w, &words) => {
                    self.pos += 1;
                    words.push(w.clone());
                    parts.push(Part::Word(w));
                }
                // a bracket before `times` is a push's count, not this
                // phrase's argument, unless a declared name has the
                // word here (fm3 log 147)
                Some(Tok::Sym("(")) if !self.count_ahead(Some(&words)) => parts.push(Part::Args(self.parse_args()?)),
                Some(Tok::Seq(w)) => {
                    // a bare sequence argument, not the start of a phrase
                    let line = self.line();
                    self.pos += 1;
                    let e = self.postfix_of(Expr { kind: ExprKind::Seq(w), line })?;
                    parts.push(Part::Value(e));
                }
                Some(Tok::Arr(w)) => {
                    let line = self.line();
                    self.pos += 1;
                    parts.push(Part::Value(Expr { kind: ExprKind::Arr(w), line }));
                }
                Some(Tok::Int(_)) | Some(Tok::Float(_)) | Some(Tok::Str(_)) | Some(Tok::Sym("[")) | Some(Tok::Sym("_")) | Some(Tok::At(_)) => {
                    let e = self.parse_postfix()?;
                    parts.push(Part::Value(e));
                }
                _ => break,
            }
        }
        if parts.is_empty() {
            return Err(self.err(format!("expected a value, found {}", self.found())));
        }
        Ok(parts)
    }

    /// `(a, b)`, `(z = 3, x = 1)`, `()`
    fn parse_args(&mut self) -> Result<Vec<Arg>, Error> {
        self.expect_sym("(")?;
        let mut out = Vec::new();
        while !self.at_sym(")") {
            let name = if matches!(self.peek(), Some(Tok::Word(_))) && matches!(self.peek_at(1), Some(Tok::Sym("="))) {
                let n = self.expect_word()?;
                self.pos += 1;
                Some(n)
            } else {
                None
            };
            out.push(Arg { name, value: self.parse_expr()? });
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> HashSet<String> {
        let mut t: HashSet<String> = ["Vec".to_string()].into_iter().collect();
        t.extend(declared_joins("on (number n) << smaller of (number a) and (number b)\n"));
        t
    }

    #[test]
    fn a_function_with_results_and_groups() {
        let src = "on (number n) << smaller of (number a) and (number b)\n    n << if (a < b) then (a) else (b)\n";
        let f = parse_feature("t", src, "t.zero", &types()).unwrap();
        let Decl::Fn(f) = &f.decls[0] else { panic!() };
        assert_eq!(f.results.len(), 1);
        assert_eq!(f.name, vec![NamePart::Word("smaller".into()), NamePart::Word("of".into()), NamePart::Group, NamePart::Word("and".into()), NamePart::Group]);
        assert_eq!(f.groups.len(), 2);
        assert_eq!(f.body.len(), 1);
    }

    /// a `<<` method (log 59) is an operator with the stream as its first
    /// parameter and no result, told from a task by the group after `<<`
    #[test]
    fn a_push_method_and_a_task() {
        let src = "on (uint8 o$) << (int x)\n    o$ << \"?\"\n\non (int t$) << count up (int n)\n    t$ << 1\n";
        let f = parse_feature("t", src, "t.zero", &types()).unwrap();
        let Decl::Fn(m) = &f.decls[0] else { panic!() };
        assert!(!m.task && m.results.is_empty());
        assert_eq!(m.name, vec![NamePart::Group, NamePart::Sym("<<".into()), NamePart::Group]);
        assert_eq!(m.groups.len(), 2);
        assert!(m.groups[0][0].seq && m.groups[0][0].ty == "uint8" && m.groups[1][0].name == "x");
        let Decl::Fn(t) = &f.decls[1] else { panic!() };
        assert!(t.task && t.results.len() == 1 && t.name[0] == NamePart::Word("count".into()));
    }

    #[test]
    fn declarations_calls_and_pushes() {
        let src = "on run()\n    Vec v(1, 2, 3)\n    int i = 1\n    count down()\n    existing run()\n    x$ << 1 << (x$ + 1) while (x$ < 5)\n    print \"hi\"\n";
        let f = parse_feature("t", src, "t.zero", &types()).unwrap();
        let Decl::Fn(f) = &f.decls[0] else { panic!() };
        assert!(matches!(f.body[0], Stmt::Var(_)));
        assert!(matches!(f.body[1], Stmt::Var(_)));
        assert!(matches!(&f.body[2], Stmt::Expr { expr: Expr { kind: ExprKind::Phrase(p), .. }, .. } if p.len() == 3));
        assert!(matches!(&f.body[3], Stmt::Expr { expr: Expr { kind: ExprKind::Existing(p), .. }, .. } if p.len() == 2));
        assert!(matches!(&f.body[4], Stmt::Push { items, cond: Some(_), existing: false, .. } if items.len() == 2));
        assert!(matches!(&f.body[5], Stmt::Expr { expr: Expr { kind: ExprKind::Phrase(p), .. }, .. } if matches!(p[1], Part::Value(_))));
    }

    #[test]
    fn a_phrase_may_begin_with_a_group() {
        let e = parse_call("(3) is less than (4)", "t.md", 1, &types()).unwrap();
        let ExprKind::Phrase(p) = e.kind else { panic!() };
        assert_eq!(p.len(), 5);
        let e = parse_call("(3 + 4) * 2", "t.md", 1, &types()).unwrap();
        assert!(matches!(e.kind, ExprKind::Bin(ref op, _, _) if op == "*"));
    }

    #[test]
    fn to_is_a_word_outside_a_range() {
        let e = parse_call("sum to (10)", "t.md", 1, &types()).unwrap();
        let ExprKind::Phrase(p) = e.kind else { panic!() };
        assert_eq!(p.len(), 3);
        let e = parse_call("[1 to n]", "t.md", 1, &types()).unwrap();
        assert!(matches!(e.kind, ExprKind::Range { inclusive: false, .. }));
        // `bound` is a word like any other since log 41
        let src = "on f (int n)\n    for (i in [1 through n])\n        bound (i)\n";
        let f = parse_feature("t", src, "t.zero", &types()).unwrap();
        let Decl::Fn(f) = &f.decls[0] else { panic!() };
        assert!(matches!(&f.body[0], Stmt::For { .. }));
    }

    #[test]
    fn a_case_line_is_a_call() {
        let e = parse_call("smaller of (3) and (4)", "t.md", 1, &types()).unwrap();
        let ExprKind::Phrase(p) = e.kind else { panic!() };
        assert_eq!(p.len(), 5);
    }
}
