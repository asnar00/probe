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

pub struct Feature {
    pub name: String,
    pub file: String,
    pub decls: Vec<Decl>,
}

pub enum Decl {
    Fn(FnDecl),
    Type(TypeDecl),
    Var(VarDecl),
    /// a bare phrase at feature scope, `write(out$)`: a sink wired to
    /// the streams it reads, with no stream to fill (log 57)
    Wire(Expr),
    /// `out$ << i$ << "\n"` at feature scope: an edge (log 72), a
    /// standing connection the scheduler moves items along, the rest
    /// of the chain pushed after each
    Edge { target: Expr, items: Vec<Expr>, cond: Option<Expr>, line: usize },
}

pub struct FnDecl {
    pub line: usize,
    pub results: Vec<Param>,
    /// the words, symbols and parameter groups, in order
    pub name: Vec<NamePart>,
    /// the parameter groups, in the order their `Group` parts appear
    pub groups: Vec<Vec<Param>>,
    /// declared with `<<`: a task producing its result over time
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
    pub line: usize,
}

pub struct TypeDecl {
    pub line: usize,
    pub name: String,
    pub kind: TypeKind,
}

pub enum TypeKind {
    Enum(Vec<String>),
    Struct(Vec<Field>),
}

pub struct Field {
    pub ty: String,
    pub name: String,
    pub seq: bool,
    pub default: Option<Expr>,
    pub line: usize,
}

pub struct VarDecl {
    pub line: usize,
    /// the scope words in front: `static`, `device`, `group`
    pub scope: Vec<String>,
    pub ty: String,
    pub name: String,
    pub seq: bool,
    pub init: Option<Init>,
    /// `merge sum`: how two writes combine
    pub merge: Option<String>,
    /// `at (48000 hz)`: a stream at a rate
    pub rate: Option<Expr>,
}

pub enum Init {
    Value(Expr),
    /// `Vec v(1, 2, 3)`, `Vec v(z = 3, x = 1)`
    Construct(Vec<Arg>),
    /// `int i$ << 1 << (i$ + 1) while (i$ < 5)`
    Pushes { items: Vec<Expr>, cond: Option<Expr> },
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

pub enum Stmt {
    Var(VarDecl),
    /// `int q, int r = divide (a) by (b)`: several declared at once from one call
    Multi { vars: Vec<Param>, value: Expr, line: usize },
    Assign { targets: Vec<Target>, value: Expr, line: usize },
    If { cond: Expr, then: Vec<Stmt>, els: Option<Vec<Stmt>>, line: usize },
    /// `loop (vars) while (c) yields x, y` (log 40, 48): `yields` names the
    /// carried variables that leave, into declared or existing names
    Loop { vars: Vec<VarDecl>, cond: Option<Expr>, body: Vec<Stmt>, yields: Vec<String>, into: Option<LoopInto>, line: usize },
    For { var: String, seq: Expr, body: Vec<Stmt>, line: usize },
    Continue { values: Vec<Expr>, line: usize },
    Break { line: usize },
    Check { cond: Expr, line: usize },
    /// `x$ << a << b while (c)`; `existing` on it calls the link below
    /// this body in its chain, which is how a feature extends a `<<`
    /// method — the one shape `existing name(...)` cannot spell
    Push { target: Expr, items: Vec<Expr>, cond: Option<Expr>, existing: bool, line: usize },
    Expr { expr: Expr, line: usize },
}

/// where a loop's given values go: `int total = loop ...` declares,
/// `total = loop ...` assigns
pub enum LoopInto {
    Declare(Vec<Param>),
    Assign(Vec<Target>),
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub seq: bool,
    pub line: usize,
    /// `countdown.enabled = false`: a feature's implicit variable (log 28)
    pub feature: Option<String>,
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
pub const BUILTIN_TYPES: [&str; 33] = [
    "bool", "int", "uint", "float", "number", "scalar", "fixed", "unit", "sunit", "rational", "decimal", "time", "string",
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
}

pub fn parse_feature(name: &str, src: &str, file: &str, types: &HashSet<String>) -> Result<Feature, Error> {
    let toks = lex::lex(src, file)?;
    let mut p = Parser { toks, pos: 0, file, types, ranges: 0, header: 0 };
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
    let mut p = Parser { toks, pos: 0, file, types, ranges: 0, header: 0 };
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

    fn parse_decl(&mut self) -> Result<Decl, Error> {
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
            Some(Tok::Seq(_)) if matches!(self.peek_at(1), Some(Tok::Sym("<<"))) => {
                let line = self.line();
                let target = self.parse_primary()?;
                let (items, cond) = self.parse_pushes()?;
                if items.is_empty() {
                    return Err(self.err("nothing to push: an edge is `out$ << i$`"));
                }
                self.expect_newline()?;
                Ok(Decl::Edge { target, items, cond, line })
            }
            _ => Err(self.err(format!("expected 'on', 'type', a variable declaration or a wiring at the top of the feature, found {}", self.found()))),
        }
    }

    fn parse_fn(&mut self) -> Result<FnDecl, Error> {
        let line = self.line();
        self.expect_word()?; // on
        let mut results = Vec::new();
        let mut task = false;
        // `on (results) = name` or `on (results) << name`, or `on name`
        // with no results — told apart by what follows the first group
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
                    task = true;
                }
            } else {
                self.expect_sym("=")?;
            }
        }
        while !self.at(&Tok::Newline) {
            match self.peek().cloned() {
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
        let body = if self.at(&Tok::Indent) { self.parse_block()? } else { Vec::new() };
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
            match self.next()? {
                Tok::Word(w) if self.is_type(&w) || ty.is_none() => {
                    if !self.is_type(&w) {
                        self.pos -= 1;
                        return Err(self.err(format!("'{}' is not a type: a parameter is its type then its name", w)));
                    }
                    ty = Some(w);
                    let (name, seq) = self.expect_name()?;
                    out.push(Param { ty: ty.clone().unwrap(), name, seq, line });
                }
                Tok::Word(w) => out.push(Param { ty: ty.clone().unwrap(), name: w, seq: false, line }),
                Tok::Seq(w) => out.push(Param { ty: ty.clone().unwrap(), name: w, seq: true, line }),
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

    fn expect_name(&mut self) -> Result<(String, bool), Error> {
        match self.next()? {
            Tok::Word(w) => Ok((w, false)),
            Tok::Seq(w) => Ok((w, true)),
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
            let (items, cond) = self.parse_pushes()?;
            Some(Init::Pushes { items, cond })
        } else {
            None
        };
        let merge = if self.eat_word("merge") { Some(self.expect_word()?) } else { None };
        Ok(VarDecl { line, scope, ty, name, seq, init, merge, rate })
    }

    /// `<< a << b [while (c)]`; a bare `<<` at the end of the line is
    /// refused by the lowering (log 38). A trip count is never written
    /// here: a bound is a product setting (log 41)
    fn parse_pushes(&mut self) -> Result<(Vec<Expr>, Option<Expr>), Error> {
        let mut items = Vec::new();
        while self.eat_sym("<<") {
            if self.at(&Tok::Newline) {
                break;
            }
            items.push(self.parse_expr()?);
        }
        let cond = if self.eat_word("while") { Some(self.parse_expr()?) } else { None };
        Ok((items, cond))
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
                Ok(Stmt::If { cond, then, els, line })
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
                self.expect_newline()?;
                Ok(Stmt::Break { line })
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
                    vars.push(Param { ty, name, seq, line: pline });
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym("=")?;
                self.expect_word()?;
                self.parse_loop(Some(LoopInto::Declare(vars)), line)
            }
            Some(Tok::Word(w)) if self.is_type(&w) && matches!(self.peek_at(1), Some(Tok::Word(_)) | Some(Tok::Seq(_))) => {
                let v = self.parse_var()?;
                // `int q, int r = ...`: several typed names, one initializer
                if self.at_sym(",") && v.init.is_none() {
                    let mut vars = vec![Param { ty: v.ty.clone(), name: v.name.clone(), seq: v.seq, line: v.line }];
                    while self.eat_sym(",") {
                        let pline = self.line();
                        let ty = self.expect_word()?;
                        if !self.is_type(&ty) {
                            self.pos -= 1;
                            return Err(self.err(format!("'{}' is not a type: each of several results is its type then its name", ty)));
                        }
                        let (name, seq) = self.expect_name()?;
                        vars.push(Param { ty, name, seq, line: pline });
                    }
                    self.expect_sym("=")?;
                    let value = self.parse_expr()?;
                    self.expect_newline()?;
                    return Ok(Stmt::Multi { vars, value, line });
                }
                self.expect_newline()?;
                Ok(Stmt::Var(v))
            }
            Some(Tok::Word(_)) | Some(Tok::Seq(_)) if self.assignment_ahead() => {
                let mut targets = Vec::new();
                loop {
                    let tline = self.line();
                    let (mut name, seq) = self.expect_name()?;
                    let mut feature = None;
                    if !seq && self.eat_sym(".") {
                        feature = Some(name);
                        name = self.expect_word()?;
                    }
                    targets.push(Target { name, seq, line: tline, feature });
                    if !self.eat_sym(",") {
                        break;
                    }
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
            Some(Tok::Seq(_)) if matches!(self.peek_at(1), Some(Tok::Sym("<<"))) => {
                let target = self.parse_primary()?;
                let (items, cond) = self.parse_pushes()?;
                if items.is_empty() {
                    return Err(self.err("nothing to push: `x$ << item`"));
                }
                self.expect_newline()?;
                Ok(Stmt::Push { target, items, cond, existing: false, line })
            }
            // `existing o$ << x` inside a `<<` method: the definition
            // below this one in the chain, which no `existing name(...)`
            // can name, the method's name being an operator
            Some(Tok::Word(w)) if w == "existing" && matches!(self.peek_at(1), Some(Tok::Seq(_))) && matches!(self.peek_at(2), Some(Tok::Sym("<<"))) => {
                self.pos += 1;
                let target = self.parse_primary()?;
                let (items, cond) = self.parse_pushes()?;
                if cond.is_some() {
                    return Err(self.err("`existing x$ << item` takes no `while`: it calls the definition below once"));
                }
                if items.len() != 1 {
                    return Err(self.err("`existing x$ << item` passes one item to the definition below"));
                }
                self.expect_newline()?;
                Ok(Stmt::Push { target, items, cond, existing: true, line })
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
            let (Some(Tok::Word(_)), Some(Tok::Word(_) | Tok::Seq(_))) = (self.toks.get(i).map(|t| &t.tok), self.toks.get(i + 1).map(|t| &t.tok)) else {
                return false;
            };
            match self.toks.get(i + 2).map(|t| &t.tok) {
                Some(Tok::Sym("=")) => return matches!(self.toks.get(i + 3).map(|t| &t.tok), Some(Tok::Word(w)) if w == "loop"),
                Some(Tok::Sym(",")) => i += 3,
                _ => return false,
            }
        }
    }

    /// `a = ...`, `a, b = ...` or `f.enabled = ...` ahead on this line
    fn assignment_ahead(&self) -> bool {
        let mut i = self.pos;
        loop {
            match self.toks.get(i).map(|t| &t.tok) {
                Some(Tok::Word(_)) | Some(Tok::Seq(_)) => {}
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
        self.parse_compare()
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
                if matches!(self.peek(), Some(Tok::Word(v)) if !self.ends_phrase(v) && !UNITS.contains(&v.as_str())) {
                    let mut parts = vec![Part::Value(Expr { kind: ExprKind::Seq(w), line })];
                    parts.extend(self.parse_parts()?);
                    return Ok(Expr { kind: ExprKind::Phrase(parts), line });
                }
                ExprKind::Seq(w)
            }
            Tok::Sym("_") => ExprKind::Acc,
            Tok::Sym("(") => {
                // a phrase may begin with a bracket group — `(3) is less
                // than (4)` — when a word follows it; otherwise these are
                // parentheses around an expression
                self.pos -= 1;
                let at = self.pos;
                if let Ok(args) = self.parse_args() {
                    if matches!(self.peek(), Some(Tok::Word(w)) if !self.ends_phrase(w) && !UNITS.contains(&w.as_str())) {
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
        matches!(w, "then" | "else" | "while" | "merge" | "in") || (self.ranges > 0 && matches!(w, "through" | "to")) || (self.header > 0 && w == "yields")
    }

    /// the parts of a phrase: words, bracketed argument groups, and bare
    /// arguments (a literal, a sequence name, a list), up to whatever
    /// ends an expression
    fn parse_parts(&mut self) -> Result<Vec<Part>, Error> {
        let mut parts = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Tok::Word(w)) if !self.ends_phrase(&w) => {
                    self.pos += 1;
                    parts.push(Part::Word(w));
                }
                Some(Tok::Sym("(")) => parts.push(Part::Args(self.parse_args()?)),
                Some(Tok::Seq(w)) => {
                    // a bare sequence argument, not the start of a phrase
                    let line = self.line();
                    self.pos += 1;
                    let e = self.postfix_of(Expr { kind: ExprKind::Seq(w), line })?;
                    parts.push(Part::Value(e));
                }
                Some(Tok::Int(_)) | Some(Tok::Float(_)) | Some(Tok::Str(_)) | Some(Tok::Sym("[")) | Some(Tok::Sym("_")) => {
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
        ["Vec".to_string()].into_iter().collect()
    }

    #[test]
    fn a_function_with_results_and_groups() {
        let src = "on (number n) = smaller of (number a) and (number b)\n    n = if (a < b) then (a) else (b)\n";
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
