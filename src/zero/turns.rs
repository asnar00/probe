//! A function partway through a push at a rate is a thing going on (fm3
//! question 135, log 232). Where a turn of the list can start a function
//! that steps, two such functions may each be partway, and neither may
//! hold the machine's stack: each is rewritten here, in the finished
//! text, as continuations. A continuation takes the activity's place in
//! the pool and returns a number: 0, it stopped at a step and is due later; 1,
//! the activity is over; anything else, the continuation to go on with
//! at once. `__act_run` calls them in a loop, so nothing calls itself.
//! A store where no turn can start such a function is never shown to
//! this file.

use std::collections::{BTreeSet, HashMap};

/// the records of one context: an activity started past this many in
/// one case fails a check
pub(super) const POOL: usize = 64;
const NEVER: i64 = 1 << 62;

pub(super) struct Func {
    pub(super) name: String,
    pub(super) params: Vec<(String, String)>,
    pub(super) rets: Vec<String>,
    /// the body's lines, indentation kept
    pub(super) body: Vec<String>,
    /// the lines of the text it stands at, `fn` line first
    pub(super) at: (usize, usize),
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub(super) fn tokens(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if is_ident(b[i] as char) {
            let st = i;
            while i < b.len() && is_ident(b[i] as char) {
                i += 1;
            }
            out.push((st, &s[st..i]));
        } else {
            i += 1;
        }
    }
    out
}

fn names(s: &str, name: &str) -> bool {
    tokens(s).iter().any(|(_, t)| *t == name)
}

fn renamed(s: &str, map: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (at, t) in tokens(s) {
        if let Some(n) = map.get(t) {
            out.push_str(&s[last..at]);
            out.push_str(n);
            last = at + t.len();
        }
    }
    out.push_str(&s[last..]);
    out
}

/// the functions a line calls: a name with a bracket after it
pub(super) fn called(line: &str) -> Vec<&str> {
    tokens(line).into_iter().filter(|(at, t)| line[at + t.len()..].starts_with('(')).map(|(_, t)| t).collect()
}

pub(super) fn split_top(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '(' || c == '[' {
            depth += 1;
        } else if c == ')' || c == ']' {
            depth -= 1;
        }
        if c == ',' && depth == 0 {
            out.push(cur.trim().to_string());
            cur = String::new();
        } else {
            cur.push(c);
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// `a: T, b: U` as names and types
fn typed(s: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    for part in split_top(s) {
        let (n, t) = part.split_once(": ")?;
        if n.is_empty() || !n.chars().all(is_ident) {
            return None;
        }
        out.push((n.to_string(), t.to_string()));
    }
    Some(out)
}

/// what a line defines: `a: T = ...`, `a: T, b: U = ...`
fn defs(head: &str) -> Vec<(String, String)> {
    if ["loop(", "if ", "else", "store ", "check ", "ret", "break", "continue", "yield"].iter().any(|w| head.starts_with(w)) {
        return Vec::new();
    }
    match head.split_once(" = ") {
        Some((l, _)) => typed(l).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// a line that is one call and nothing else: what it defines, the
/// function, its arguments
fn call_of(head: &str) -> Option<(Vec<(String, String)>, String, Vec<String>)> {
    let d = defs(head);
    let right = if d.is_empty() { head } else { head.split_once(" = ")?.1 };
    let open = right.find('(')?;
    let name = &right[..open];
    if name.is_empty() || !name.chars().all(is_ident) || !right.ends_with(')') {
        return None;
    }
    Some((d, name.to_string(), split_top(&right[open + 1..right.len() - 1])))
}

pub(super) fn parse(ir: &str) -> (Vec<String>, Vec<Func>) {
    let lines: Vec<String> = ir.lines().map(str::to_string).collect();
    let mut fns = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("fn ") {
            let open = rest.find('(').unwrap_or(rest.len());
            let name = rest[..open].to_string();
            // the parameters end at the bracket that closes the first
            let mut depth = 0;
            let mut close = rest.len();
            for (k, c) in rest.char_indices().skip(open) {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        close = k;
                        break;
                    }
                }
            }
            let params = typed(&rest[(open + 1).min(close)..close]).unwrap_or_default();
            let rets = match rest[close..].split_once("-> ") {
                Some((_, r)) => split_top(r.trim().trim_start_matches('(').trim_end_matches(')')),
                None => Vec::new(),
            };
            let mut j = i + 1;
            while j < lines.len() && lines[j].starts_with("    ") {
                j += 1;
            }
            fns.push(Func { name, params, rets, body: lines[i + 1..j].to_vec(), at: (i, j) });
            i = j;
        } else {
            i += 1;
        }
    }
    (lines, fns)
}

/// the functions from which a step of a rate can be reached
fn steppers(fns: &[Func]) -> BTreeSet<String> {
    let known: BTreeSet<&str> = fns.iter().map(|f| f.name.as_str()).collect();
    let calls: HashMap<&str, BTreeSet<&str>> = fns.iter().map(|f| (f.name.as_str(), f.body.iter().flat_map(|l| called(l)).filter(|c| known.contains(c) || *c == "__step").collect())).collect();
    let mut r: BTreeSet<String> = BTreeSet::new();
    loop {
        let more: Vec<String> = fns.iter().filter(|f| f.name != "__step" && !r.contains(&f.name) && calls[f.name.as_str()].iter().any(|c| *c == "__step" || r.contains(*c))).map(|f| f.name.clone()).collect();
        if more.is_empty() {
            return r;
        }
        r.extend(more);
    }
}

/// a function wired over a stream, `__zN`, and the function it calls
/// for each item, `__zN_each`
fn wired(f: &Func) -> Option<String> {
    let n = f.name.strip_prefix("__z")?;
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}_each", f.name))
}

/// The one question (fm3 log 232): can a turn of the list start a
/// function that steps? Today that is a function wired over the input,
/// in a store whose input arrives at a time
pub fn wanted(ir: &str) -> bool {
    if !ir.contains("\nfn __in_turn(") {
        return false;
    }
    let (_, fns) = parse(ir);
    let r0 = steppers(&fns);
    fns.iter().any(|f| wired(f).map_or(false, |e| r0.contains(&e) && f.body.iter().any(|l| called(l).contains(&e.as_str()))))
}

/// a statement: a line, the lines under it, and an `else` with its own
#[derive(Clone)]
struct Stmt {
    lines: Vec<String>,
}

impl Stmt {
    fn head(&self) -> &str {
        self.lines[0].trim()
    }
}

fn depth(l: &str) -> usize {
    (l.len() - l.trim_start().len()) / 4
}

fn stmts(lines: &[String], d: usize) -> Vec<Stmt> {
    let mut out: Vec<Stmt> = Vec::new();
    for l in lines {
        if depth(l) == d && l.trim() != "else" || out.is_empty() {
            out.push(Stmt { lines: vec![l.clone()] });
        } else {
            out.last_mut().unwrap().lines.push(l.clone());
        }
    }
    out
}

#[derive(Clone)]
enum Point {
    /// a step of a rate, its period as written
    Step(String),
    /// the wait for a slot, the time as written
    Wait(String),
    /// a call of another such function
    Call(Vec<(String, String)>, String, Vec<String>),
}

enum Kind {
    Plain,
    At(Point),
    /// a loop left at places of its own body: the body's statements
    Loop(Vec<Stmt>, Vec<Option<Point>>),
    /// an `if` that is a statement, left at places at the top of a
    /// branch (fm3 log 265): the statements of its two branches one
    /// after the other, how many of them are the first's, and whether
    /// an `else` is written
    If(Vec<Stmt>, Vec<Option<Point>>, usize, bool),
}

struct Maker<'a> {
    r: &'a BTreeSet<String>,
    fns: HashMap<String, &'a Func>,
    ids: HashMap<(String, usize), i64>,
    fields: Vec<(String, String)>,
    tmp: usize,
    /// the names this continuation read from the record, which still
    /// holds them: they are not kept again
    held: BTreeSet<String>,
    /// the number of the continuation being written: left again at
    /// its own place, the pool already says where to go on
    here: i64,
    /// a place in a branch of an `if` is held (fm3 log 265)
    ifs: bool,
}

fn point_of(head: &str, r: &BTreeSet<String>) -> Option<Point> {
    let (d, name, args) = call_of(head)?;
    match name.as_str() {
        "__step" if args.len() == 1 => Some(Point::Step(args[0].clone())),
        "__wait" if args.len() == 1 => Some(Point::Wait(args[0].clone())),
        n if r.contains(n) => Some(Point::Call(d, name, args)),
        _ => None,
    }
}

fn leaves(line: &str, r: &BTreeSet<String>) -> bool {
    called(line).iter().any(|c| *c == "__step" || *c == "__wait" || r.contains(*c))
}

fn kind(s: &Stmt, d: usize, r: &BTreeSet<String>, ifs: bool) -> Option<Kind> {
    if let Some(p) = point_of(s.head(), r) {
        return Some(Kind::At(p));
    }
    if !s.lines.iter().any(|l| leaves(l, r)) {
        return Some(Kind::Plain);
    }
    // a place inside a loop, at the top of its body; anywhere else is
    // a shape not held
    if d == 1 && s.head().starts_with("loop(") {
        let body = stmts(&s.lines[1..], 2);
        let mut ps = Vec::new();
        for b in &body {
            match kind(b, 2, r, ifs)? {
                Kind::Plain => ps.push(None),
                Kind::At(p) => ps.push(Some(p)),
                Kind::Loop(..) | Kind::If(..) => return None,
            }
        }
        return Some(Kind::Loop(body, ps));
    }
    // ... and at the top of a branch of an `if` that is a statement at
    // the top of the function: a dynamic feature's gate, `if on / else
    // / <the chain below> / ret`, is this. First held where the cut
    // may be moved (fm3 log 265), and from log 282 wherever the pool
    // is wanted (fm3 question 153's B)
    if ifs && d == 1 && s.head().starts_with("if ") {
        let at = s.lines.iter().position(|l| depth(l) == 1 && l.trim() == "else");
        let (first, second) = match at {
            Some(a) => (&s.lines[1..a], &s.lines[a + 1..]),
            None => (&s.lines[1..], &s.lines[..0]),
        };
        let mut body = stmts(first, 2);
        let split = body.len();
        body.extend(stmts(second, 2));
        let mut ps = Vec::new();
        for b in &body {
            match kind(b, 2, r, ifs)? {
                Kind::Plain => ps.push(None),
                Kind::At(p) => ps.push(Some(p)),
                Kind::Loop(..) | Kind::If(..) => return None,
            }
        }
        return Some(Kind::If(body, ps, split, at.is_some()));
    }
    None
}

/// Can the loop these lines are be left by a `break` of its own? One
/// that cannot is where its function ends: nothing can follow it, and
/// the IR refuses text that does (fm3 log 266). A loop left at a place
/// before its test is such a one in the continuation that begins it
fn exits(lines: &[String]) -> bool {
    let mut inner: Option<usize> = None;
    for l in &lines[1..] {
        let (d, t) = (depth(l), l.trim());
        if inner.map_or(false, |n| d > n) {
            continue;
        }
        inner = if t.starts_with("loop(") || t.contains(" = loop(") { Some(d) } else { None };
        if t == "break" || t.starts_with("break ") {
            return true;
        }
    }
    false
}

fn is_context(s: &Stmt) -> bool {
    s.lines.len() == 1 && s.head().ends_with(": ptr = context()")
}

/// a value a column of the pool can hold: one word or less
pub(super) fn word(ty: &str) -> bool {
    let sized = |p: &str| ty.strip_prefix(p).map_or(false, |n| matches!(n, "1" | "8" | "16" | "32" | "64"));
    matches!(ty, "int" | "uint" | "index" | "size" | "ptr") || sized("i") || sized("u")
}

/// a value as a store writes it: a literal says its type
fn said(v: &str, ty: &str) -> String {
    if v.starts_with(|c: char| c.is_ascii_digit() || c == '-') && !v.contains(": ") {
        format!("{}: {}", v, ty)
    } else {
        v.to_string()
    }
}

impl<'a> Maker<'a> {
    fn t(&mut self) -> String {
        self.tmp += 1;
        format!("__s{}", self.tmp)
    }

    /// the column of a function's variable, made where it is first
    /// asked for
    fn field(&mut self, f: &str, name: &str, ty: &str) -> String {
        let n = format!("__act_{}__{}", f, name);
        if !self.fields.iter().any(|(m, _)| *m == n) {
            self.fields.push((n.clone(), ty.to_string()));
        }
        n
    }

    fn to(&self, f: &str) -> String {
        format!("__act_to_{}", f)
    }

    fn res(&self, f: &str, k: usize) -> String {
        format!("__act_res{}_{}", k, f)
    }

    fn read(&mut self, col: &str, name: &str, ty: &str, pad: &str, out: &mut Vec<String>) {
        let p = self.t();
        out.push(format!("{}{}: ptr = addr {}", pad, p, col));
        // a bit is kept as a byte: memory holds nothing narrower
        if ty == "u1" {
            let b = self.t();
            out.push(format!("{}{}: u8 = load {}, __a, 8", pad, b, p));
            out.push(format!("{}{}: u1 = cmp.ne {}, 0", pad, name, b));
            return;
        }
        out.push(format!("{}{}: {} = load {}, __a, 8", pad, name, ty, p));
    }

    fn write(&mut self, col: &str, val: &str, ty: &str, pad: &str, out: &mut Vec<String>) {
        let p = self.t();
        out.push(format!("{}{}: ptr = addr {}", pad, p, col));
        if ty == "u1" {
            let b = self.t();
            out.push(format!("{}{}: u8 = conv {}", pad, b, val));
            out.push(format!("{}store {}, {}, __a, 8", pad, b, p));
            return;
        }
        out.push(format!("{}store {}, {}, __a, 8", pad, said(val, ty), p));
    }

    /// A name alive across a place is kept (fm3 log 282). A reader of
    /// a stream of the context keeps what is its own, where it has
    /// read to and its two rules: which stream it is the text says,
    /// and the record holds no address
    fn keep(&mut self, f: &Func, n: &str, ty: &str, pad: &str, out: &mut Vec<String>) {
        if ty.ends_with('$') && matches!(prov(f, n), Prov::Field(_)) {
            for (part, pt) in [("pos", "index"), ("rule", "i64"), ("edge", "i64")] {
                let v = self.t();
                out.push(format!("{}{}: {} = get {}, {}", pad, v, pt, n, part));
                let col = self.field(&f.name, &format!("{}__{}", n, part), pt);
                self.write(&col, &v, pt, pad, out);
            }
            return;
        }
        let col = self.field(&f.name, n, ty);
        self.write(&col, n, ty, pad, out);
    }

    /// ... and read again where the function goes on, under the name
    /// `to`: a stream of the context from the context, the place this
    /// continuation runs at
    fn fetch(&mut self, f: &Func, n: &str, to: &str, ty: &str, pad: &str, out: &mut Vec<String>) {
        if ty.ends_with('$') {
            if let Prov::Field(x) = prov(f, n) {
                let (c, cx) = (self.t(), self.t());
                out.push(format!("{}{}: ptr = context()", pad, c));
                out.push(format!("{}{}: __ctx = load {}", pad, cx, c));
                let mut h = self.t();
                out.push(format!("{}{}: {} = get {}, {}", pad, h, ty, cx, x));
                for (k, (part, pt)) in [("pos", "index"), ("rule", "i64"), ("edge", "i64")].into_iter().enumerate() {
                    let v = self.t();
                    let col = self.field(&f.name, &format!("{}__{}", n, part), pt);
                    self.read(&col, &v, pt, pad, out);
                    let h2 = if k == 2 { to.to_string() } else { self.t() };
                    out.push(format!("{}{}: {} = set {}, {}, {}", pad, h2, ty, h, part, v));
                    h = h2;
                }
                return;
            }
        }
        let col = self.field(&f.name, n, ty);
        self.read(&col, to, ty, pad, out);
    }

    /// a `ret`: the results kept, and the caller's place given
    fn ret(&mut self, f: &Func, line: &str, out: &mut Vec<String>) {
        let pad = line[..line.len() - line.trim_start().len()].to_string();
        let vals = split_top(line.trim().strip_prefix("ret").unwrap());
        for (k, v) in vals.iter().enumerate() {
            let (col, ty) = (self.res(&f.name, k), f.rets[k].clone());
            self.write(&col, v, &ty, &pad, out);
        }
        let k = self.t();
        let col = self.to(&f.name);
        self.read(&col, &k, "i64", &pad, out);
        out.push(format!("{}ret {}", pad, k));
    }

    fn plain(&mut self, f: &Func, s: &Stmt, out: &mut Vec<String>) {
        for l in &s.lines {
            let t = l.trim();
            if t == "ret" || t.starts_with("ret ") {
                self.ret(f, l, out);
            } else {
                out.push(l.clone());
            }
        }
    }

    /// the function is left here: what is alive is kept in the pool
    /// with the place to go on from, and the continuation returns
    fn leave(&mut self, f: &Func, p: &Point, id: i64, live: &[(String, String)], pad: &str, out: &mut Vec<String>) {
        for (n, ty) in live {
            if self.held.contains(n) {
                continue;
            }
            self.keep(f, n, ty, pad, out);
        }
        match p {
            Point::Step(_) | Point::Wait(_) => {
                let due = match p {
                    Point::Step(d) => {
                        let (c, x, now, due) = (self.t(), self.t(), self.t(), self.t());
                        out.push(format!("{}{}: ptr = context()", pad, c));
                        out.push(format!("{}{}: __ctx = load {}", pad, x, c));
                        out.push(format!("{}{}: i64 = get {}, __clock", pad, now, x));
                        out.push(format!("{}{}: i64 = add {}, {}", pad, due, now, d));
                        due
                    }
                    Point::Wait(t) => t.clone(),
                    _ => unreachable!(),
                };
                self.write("__act_due", &due, "i64", pad, out);
                if id != self.here {
                    self.write("__act_pc", &id.to_string(), "i64", pad, out);
                }
                out.push(format!("{}ret 0", pad));
            }
            Point::Call(_, g, args) => {
                let callee = self.fns[g.as_str()];
                for ((pn, pt), a) in callee.params.iter().zip(args) {
                    let col = self.field(g, pn, pt);
                    self.write(&col, a, pt, pad, out);
                }
                let col = self.to(g);
                self.write(&col, &id.to_string(), "i64", pad, out);
                out.push(format!("{}ret {}", pad, self.ids[&(g.clone(), 0)]));
            }
        }
    }

    /// the statements of a function from one of them on, to the first
    /// place it is left at
    fn top(&mut self, f: &Func, ss: &[Stmt], kinds: &[Kind], from: usize, out: &mut Vec<String>) {
        for j in from..ss.len() {
            match &kinds[j] {
                Kind::Plain => self.plain(f, &ss[j], out),
                Kind::At(p) => {
                    let live = self.live_top(f, ss, j);
                    let id = self.ids[&(f.name.clone(), self.place(kinds, j, 0))];
                    self.leave(f, p, id, &live, "    ", out);
                    return;
                }
                Kind::Loop(body, ps) => {
                    let at = out.len();
                    out.push(ss[j].lines[0].clone());
                    self.body(f, ss, kinds, j, body, ps, 0, out);
                    if !exits(&out[at..]) {
                        return;
                    }
                }
                Kind::If(body, ps, split, other) => {
                    out.push(ss[j].lines[0].clone());
                    self.branch(f, ss, kinds, j, body, ps, (0, 0, *split), false, out);
                    if *other {
                        out.push("    else".to_string());
                        self.branch(f, ss, kinds, j, body, ps, (*split, *split, body.len()), false, out);
                    }
                }
            }
        }
    }

    /// A branch of an `if` from one of its statements on, to the first
    /// place: `span` is where to begin, and where the branch begins
    /// and ends among the statements of the two. In a continuation the
    /// rest of a branch stands at the top of the function, a level
    /// out. Says whether the branch ended there, left or returned from
    #[allow(clippy::too_many_arguments)]
    fn branch(&mut self, f: &Func, ss: &[Stmt], kinds: &[Kind], j: usize, body: &[Stmt], ps: &[Option<Point>], span: (usize, usize, usize), out_a_level: bool, out: &mut Vec<String>) -> bool {
        let (from, lo, hi) = span;
        for i in from..hi {
            match &ps[i] {
                None => {
                    let s = if out_a_level { Stmt { lines: body[i].lines.iter().map(|l| l[4..].to_string()).collect() } } else { body[i].clone() };
                    self.plain(f, &s, out);
                }
                Some(p) => {
                    let mut live = self.live_top(f, ss, j);
                    live.extend(self.live_within(body, lo, i, hi));
                    let id = self.ids[&(f.name.clone(), self.place(kinds, j, i))];
                    self.leave(f, p, id, &live, if out_a_level { "    " } else { "        " }, out);
                    return true;
                }
            }
        }
        hi > from && body[hi - 1].head().starts_with("ret")
    }

    /// what is alive across a place in a branch, of the branch's own
    fn live_within(&self, body: &[Stmt], lo: usize, i: usize, hi: usize) -> Vec<(String, String)> {
        let cands: Vec<(String, String)> = body[lo..i].iter().flat_map(|s| defs(s.head())).collect();
        cands.into_iter().filter(|(n, _)| body[i + 1..hi].iter().any(|s| s.lines.iter().any(|l| names(l, n)))).collect()
    }

    /// a loop's body from one of its statements on, to the first place
    #[allow(clippy::too_many_arguments)]
    fn body(&mut self, f: &Func, ss: &[Stmt], kinds: &[Kind], j: usize, body: &[Stmt], ps: &[Option<Point>], from: usize, out: &mut Vec<String>) {
        for i in from..body.len() {
            match &ps[i] {
                None => self.plain(f, &body[i], out),
                Some(p) => {
                    let mut live = self.live_top(f, ss, j);
                    live.extend(loop_params(ss[j].head()).into_iter().map(|(n, t, _)| (n, t)));
                    live.extend(self.live_body(body, i));
                    let id = self.ids[&(f.name.clone(), self.place(kinds, j, i))];
                    self.leave(f, p, id, &live, "        ", out);
                    return;
                }
            }
        }
    }

    /// the number of a place within its function: the places counted
    /// in the order written, from 1
    fn place(&self, kinds: &[Kind], j: usize, i: usize) -> usize {
        let mut n = 0;
        for (jj, k) in kinds.iter().enumerate() {
            match k {
                Kind::Plain => {}
                Kind::At(_) => {
                    n += 1;
                    if jj == j {
                        return n;
                    }
                }
                Kind::Loop(_, ps) | Kind::If(_, ps, ..) => {
                    for (ii, p) in ps.iter().enumerate() {
                        if p.is_some() {
                            n += 1;
                            if jj == j && ii == i {
                                return n;
                            }
                        }
                    }
                }
            }
        }
        n
    }

    /// what is alive across a place at the top of a function: every
    /// parameter and every name defined above it that the text from
    /// there on names
    fn live_top(&self, f: &Func, ss: &[Stmt], j: usize) -> Vec<(String, String)> {
        let mut cands: Vec<(String, String)> = f.params.clone();
        for s in &ss[..j] {
            if !is_context(s) {
                cands.extend(defs(s.head()));
            }
        }
        // a loop left inside is run again from its head, so its own
        // text is after the place too
        let after: Vec<&String> = ss[j..].iter().flat_map(|s| s.lines.iter()).skip(if ss[j].head().starts_with("loop(") { 0 } else { 1 }).collect();
        cands.into_iter().filter(|(n, _)| after.iter().any(|l| names(l, n))).collect()
    }

    fn live_body(&self, body: &[Stmt], i: usize) -> Vec<(String, String)> {
        let cands: Vec<(String, String)> = body[..i].iter().flat_map(|s| defs(s.head())).collect();
        cands.into_iter().filter(|(n, _)| body[i + 1..].iter().any(|s| s.lines.iter().any(|l| names(l, n)))).collect()
    }

    fn open(&mut self, f: &Func, place: usize, before: &[Stmt], out: &mut Vec<String>) {
        self.tmp = 0;
        self.here = self.ids[&(f.name.clone(), place)];
        out.push(format!("fn {}__{}(__a: index) -> i64", f.name, place));
        out.extend(before.iter().filter(|s| is_context(s)).map(|s| s.lines[0].clone()));
    }

    /// every continuation of one function, or nothing where its shape
    /// is not held
    fn make(&mut self, f: &Func) -> Option<Vec<String>> {
        let ss = stmts(&f.body, 1);
        let mut kinds = Vec::new();
        for s in &ss {
            kinds.push(kind(s, 1, self.r, self.ifs)?);
        }
        let mut out = Vec::new();
        // the start: the parameters are the pool's
        self.open(f, 0, &[], &mut out);
        self.held = f.params.iter().map(|(n, _)| n.clone()).collect();
        for (n, t) in &f.params {
            let col = self.field(&f.name, n, t);
            self.read(&col, n, t, "    ", &mut out);
        }
        self.top(f, &ss, &kinds, 0, &mut out);
        for (j, k) in kinds.iter().enumerate() {
            match k {
                Kind::Plain => {}
                Kind::At(p) => {
                    let place = self.place(&kinds, j, 0);
                    self.open(f, place, &ss[..j], &mut out);
                    let live = self.live_top(f, &ss, j);
                    self.held = live.iter().map(|(n, _)| n.clone()).collect();
                    for (n, t) in &live {
                        self.fetch(f, n, n, t, "    ", &mut out);
                    }
                    if let Point::Call(d, g, _) = p {
                        for (k, (n, t)) in d.iter().enumerate() {
                            let col = self.res(g, k);
                            self.read(&col, n, t, "    ", &mut out);
                        }
                    }
                    self.top(f, &ss, &kinds, j + 1, &mut out);
                }
                Kind::Loop(body, ps) => {
                    for (i, p) in ps.iter().enumerate() {
                        let Some(p) = p else { continue };
                        let place = self.place(&kinds, j, i);
                        self.open(f, place, &ss[..j], &mut out);
                        let live = self.live_top(f, &ss, j);
                        self.held = live.iter().map(|(n, _)| n.clone()).collect();
                        for (n, t) in &live {
                            self.fetch(f, n, n, t, "    ", &mut out);
                        }
                        // the loop begins again from the number it
                        // was left with, what followed the place run
                        // first, under names of its own
                        let mut heads = Vec::new();
                        for (n, t, _) in loop_params(ss[j].head()) {
                            self.fetch(f, &n, &format!("{}__s", n), &t, "    ", &mut out);
                            heads.push(format!("{}: {} = {}__s", n, t, n));
                        }
                        let mut map = HashMap::new();
                        for (n, t) in self.live_body(body, i) {
                            self.fetch(f, &n, &format!("{}__r", n), &t, "    ", &mut out);
                            map.insert(n.clone(), format!("{}__r", n));
                        }
                        if let Point::Call(d, g, _) = p {
                            for (k, (n, t)) in d.iter().enumerate() {
                                let col = self.res(g, k);
                                self.read(&col, &format!("{}__r", n), t, "    ", &mut out);
                                map.insert(n.clone(), format!("{}__r", n));
                            }
                        }
                        let head = ss[j].head();
                        let close = head.rfind(')').unwrap();
                        heads.push("__q: u1 = 1".to_string());
                        out.push(format!("    loop({}){}", heads.join(", "), &head[close + 1..]));
                        let mut rest = Vec::new();
                        self.body(f, &ss, &kinds, j, body, ps, i + 1, &mut rest);
                        out.push("        if __q".to_string());
                        for l in rest {
                            out.push(format!("    {}", renamed(&l, &map)));
                        }
                        let mut again = Vec::new();
                        self.body(f, &ss, &kinds, j, body, ps, 0, &mut again);
                        out.extend(again);
                        // every `continue` of this loop now says the
                        // tail is done with
                        let at = out.iter().rposition(|l| l.starts_with("    loop(")).unwrap();
                        let mut inner: Option<usize> = None;
                        for l in out[at + 1..].iter_mut() {
                            let (d, t) = (depth(l), l.trim().to_string());
                            if inner.map_or(false, |n| d > n) {
                                continue;
                            }
                            inner = if t.starts_with("loop(") || t.contains(" = loop(") { Some(d) } else { None };
                            if t == "continue" {
                                l.push_str(" 0");
                            } else if t.starts_with("continue ") {
                                l.push_str(", 0");
                            }
                        }
                        if exits(&out[at..]) {
                            self.top(f, &ss, &kinds, j + 1, &mut out);
                        }
                    }
                }
                Kind::If(body, ps, split, _) => {
                    for (i, p) in ps.iter().enumerate() {
                        let Some(p) = p else { continue };
                        let place = self.place(&kinds, j, i);
                        self.open(f, place, &ss[..j], &mut out);
                        let (lo, hi) = if i < *split { (0, *split) } else { (*split, body.len()) };
                        let mut live = self.live_top(f, &ss, j);
                        live.extend(self.live_within(body, lo, i, hi));
                        self.held = live.iter().map(|(n, _)| n.clone()).collect();
                        for (n, t) in &live {
                            self.fetch(f, n, n, t, "    ", &mut out);
                        }
                        if let Point::Call(d, g, _) = p {
                            for (k, (n, t)) in d.iter().enumerate() {
                                let col = self.res(g, k);
                                self.read(&col, n, t, "    ", &mut out);
                            }
                        }
                        // the rest of the branch, and then what
                        // follows the `if`
                        if !self.branch(f, &ss, &kinds, j, body, ps, (i + 1, lo, hi), true, &mut out) {
                            self.top(f, &ss, &kinds, j + 1, &mut out);
                        }
                    }
                }
            }
        }
        Some(out)
    }
}

/// a loop's parameters: name, type, first value
fn loop_params(head: &str) -> Vec<(String, String, String)> {
    let close = head.rfind(')').unwrap();
    split_top(&head["loop(".len()..close]).into_iter().filter_map(|p| {
        let (n, rest) = p.split_once(": ")?;
        let (t, v) = rest.split_once(" = ")?;
        Some((n.to_string(), t.to_string(), v.to_string()))
    }).collect()
}

/// The rewrite (fm3 log 232), or nothing where some function's shape
/// is not held and the store stays as it was
pub fn rewrite(ir: &str, moved: bool) -> Option<String> {
    let (mut lines, fns) = parse(ir);
    let r0 = steppers(&fns);
    let by: HashMap<String, &Func> = fns.iter().map(|f| (f.name.clone(), f)).collect();
    // where an activity starts: a wired function's call for each
    // item, and a case's twin
    let mut starts: Vec<(String, String)> = Vec::new();
    for f in &fns {
        if let Some(e) = wired(f) {
            if r0.contains(&e) && f.body.iter().any(|l| called(l).contains(&e.as_str())) {
                starts.push((f.name.clone(), e));
            }
        } else if f.name.starts_with("__whole_") {
            if let Some(g) = f.body.iter().flat_map(|l| called(l)).find(|c| r0.contains(*c)) {
                starts.push((f.name.clone(), g.to_string()));
            }
        }
    }
    // (where the cut may be moved, fm3 log 265, a case's own function
    // is enough: the move is the other thing going on)
    if starts.is_empty() || !moved && !starts.iter().any(|(f, _)| !f.starts_with("__whole_")) {
        return None;
    }
    // every function such a start reaches that can itself reach a step
    let mut r: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<String> = starts.iter().map(|(_, g)| g.clone()).collect();
    while let Some(g) = todo.pop() {
        if !r0.contains(&g) || !r.insert(g.clone()) {
            continue;
        }
        todo.extend(by[&g].body.iter().flat_map(|l| called(l)).filter(|c| by.contains_key(*c)).map(str::to_string));
    }
    if ir.matches("data __ctx_mem: array(__ctx, 2)").count() != 1 {
        return None;
    }
    let mut m = Maker { r: &r, fns: by.iter().map(|(k, v)| (k.clone(), *v)).collect(), ids: HashMap::new(), fields: Vec::new(), tmp: 0, held: BTreeSet::new(), here: 0, ifs: true };
    // the continuations' numbers, from 2: the places inside loops
    // first, being where an item is, so that the choice among them
    // meets those first
    let mut order: Vec<(bool, String, usize)> = Vec::new();
    for g in &r {
        let f = by[g];
        let ss = stmts(&f.body, 1);
        let mut n = 0;
        order.push((false, g.clone(), 0));
        for s in &ss {
            match kind(s, 1, &r, true)? {
                Kind::Plain => {}
                Kind::At(_) => {
                    n += 1;
                    order.push((false, g.clone(), n));
                }
                Kind::Loop(_, ps) => {
                    for _ in ps.iter().flatten() {
                        n += 1;
                        order.push((true, g.clone(), n));
                    }
                }
                Kind::If(_, ps, ..) => {
                    for _ in ps.iter().flatten() {
                        n += 1;
                        order.push((false, g.clone(), n));
                    }
                }
            }
        }
    }
    order.sort_by_key(|(inner, ..)| !*inner);
    for (k, (_, g, n)) in order.iter().enumerate() {
        m.ids.insert((g.clone(), *n), k as i64 + 2);
    }
    let mut text: Vec<String> = Vec::new();
    for g in &r {
        let f = by[g];
        m.fields.push((m.to(g), "i64".into()));
        for (k, t) in f.rets.iter().enumerate() {
            m.fields.push((m.res(g, k), t.clone()));
        }
        text.extend(m.make(f)?);
    }
    // the choice, the loop, the pool
    let ended = ir.contains("    __ended: u1\n");
    text.push("\n; the continuation of this number (fm3 log 232), which gives the one to go on with: 0, it stopped at a step; 1, the activity is over".into());
    text.push("fn __act_go(__a: index, k: i64) -> i64".into());
    for (_, g, n) in &order {
        let id = m.ids[&(g.clone(), *n)];
        text.push(format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        n{id}: i64 = {g}__{n}(__a)\n        ret n{id}", id = id, g = g, n = n));
    }
    text.push("    ret 1".into());
    let over = if ended { "\n        e: __ctx = load _this\n        gone: u1 = get e, __ended\n        if gone\n            e2: __ctx = set e, __ended, 0\n            store e2, _this\n            store {never}: i64, dp, __a, 8\n            break".replace("{never}", &NEVER.to_string()) } else { String::new() };
    text.push(format!("\n; an activity goes on from a continuation until one stops at a step or the activity is over, which a `restart` that ended it also says; over, it is never due again\nfn __act_run(__a: index, k: i64)\n{this}    dp: ptr = addr __act_due\n    loop(c: i64 = k)\n        n: i64 = __act_go(__a, c){over}\n        on: u1 = cmp.gt n, 1\n        if on\n        else\n            done: u1 = cmp.eq n, 1\n            if done\n                store {never}: i64, dp, __a, 8\n            break\n        continue n\n    ret", this = if ended { "    _this: ptr = context()\n" } else { "" }, over = over, never = NEVER));
    text.push(format!("\n; the pool's word of the list: the least time any activity of this context is due and which that is, the first started of two at one time (fm3 question 121)\nfn __act_next()\n    _this: ptr = context()\n    x: __ctx = load _this\n    n: index = get x, __act_n\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne _this, cm\n    sw: index = conv second\n    base: index = mul sw, {pool}\n    top: index = add base, n\n    dp: ptr = addr __act_due\n    loop(k: index = base, b: i64 = {never}, i: index = base)\n        done: u1 = cmp.ge k, top\n        if done\n            y: __ctx = load _this\n            y2: __ctx = set y, __due_act, b\n            y3: __ctx = set y2, __act_i, i\n            store y3, _this\n            break\n        d: i64 = load dp, k, 8\n        less: u1 = cmp.lt d, b\n        b2: i64 = if less\n            yield d\n        else\n            yield b\n        i2: index = if less\n            yield k\n        else\n            yield i\n        k2: index = add k, 1\n        continue k2, b2, i2\n    ret", never = NEVER, pool = POOL));
    text.push("\n; the pool's turn: the activity that is due goes on\nfn __act_turn()\n    _this: ptr = context()\n    x: __ctx = load _this\n    i: index = get x, __act_i\n    pp: ptr = addr __act_pc\n    k: i64 = load pp, i, 8\n    __act_run(i, k)\n    __act_next()\n    ret".into());
    let mut made: BTreeSet<String> = BTreeSet::new();
    for (caller, g) in &starts {
        let f = by[g];
        if made.insert(g.clone()) {
            let params: Vec<String> = f.params.iter().map(|(n, t)| format!("{}: {}", n, t)).collect();
            // (what the start makes of its own is named as no parameter
            // of a program's can be, fm3 log 260: `count down from (int
            // n)` has an `n`)
            let mut s = format!("\n; an activity starts: a place in the pool, its parameters, and its first turn at once\nfn __act_start_{g}({params}) -> index\n    _this: ptr = context()\n    __x: __ctx = load _this\n    __n: index = get __x, __act_n\n    __room: u1 = cmp.lt __n, {pool}\n    check __room\n    __n2: index = add __n, 1\n    __x2: __ctx = set __x, __act_n, __n2\n    store __x2, _this\n    __cm: ptr = addr __ctx_mem\n    __second: u1 = cmp.ne _this, __cm\n    __sw: index = conv __second\n    __base: index = mul __sw, {pool}\n    __a: index = add __base, __n\n", g = g, params = params.join(", "), pool = POOL);
            let mut lines = Vec::new();
            for (n, t) in &f.params {
                let col = m.field(g, n, t);
                m.write(&col, n, t, "    ", &mut lines);
            }
            let col = m.to(g);
            m.write(&col, "1", "i64", "    ", &mut lines);
            s.push_str(&lines.join("\n"));
            s.push_str(&format!("\n    __act_run(__a, {})\n    __act_next()\n    ret __a", m.ids[&(g.clone(), 0)]));
            text.push(s);
        }
        // the call becomes the start; a twin reads the results back
        // when everything has had its turn
        let c = by[caller];
        let mut k = c.at.0 + 1;
        while k < c.at.1 {
            if let Some((d, name, args)) = call_of(lines[k].trim()) {
                if name == *g {
                    let pad = lines[k][..lines[k].len() - lines[k].trim_start().len()].to_string();
                    lines[k] = format!("{}__act_w: index = __act_start_{}({})", pad, g, args.join(", "));
                    if !d.is_empty() {
                        let t = (k..c.at.1).find(|j| lines[*j].trim().starts_with("__turns("))?;
                        let mut back = lines[t].clone();
                        for (i, (n, ty)) in d.iter().enumerate() {
                            back.push_str(&format!("\n{pad}__act_p{i}: ptr = addr {col}\n{pad}{n}: {ty} = load __act_p{i}, __act_w, 8", pad = pad, i = i, col = m.res(g, i), n = n, ty = ty));
                        }
                        lines[t] = back;
                    }
                    break;
                }
            }
            k += 1;
        }
    }
    if m.fields.iter().any(|(_, t)| !word(t)) {
        return None;
    }
    // a function no longer called as it was written leaves the text
    let kept: BTreeSet<String> = {
        let mut kept = BTreeSet::new();
        let mut todo: Vec<String> = fns.iter().filter(|f| !r.contains(&f.name)).flat_map(|f| lines[f.at.0 + 1..f.at.1].iter().flat_map(|l| called(l)).map(str::to_string).collect::<Vec<_>>()).filter(|c| r.contains(c)).collect();
        while let Some(g) = todo.pop() {
            if kept.insert(g.clone()) {
                todo.extend(by[&g].body.iter().flat_map(|l| called(l)).filter(|c| r.contains(*c)).map(str::to_string));
            }
        }
        kept
    };
    let mut out = String::new();
    let gone: Vec<(usize, usize)> = fns.iter().filter(|f| r.contains(&f.name) && !kept.contains(&f.name)).map(|f| f.at).collect();
    for (k, l) in lines.iter().enumerate() {
        if gone.iter().any(|(a, b)| k >= *a && k < *b) {
            continue;
        }
        if l.starts_with("data __ctx_mem: ") {
            out.push_str(&format!("; the activities (fm3 log 232): a function partway through a push at a rate, and whatever called it. The pool is a column for each thing kept, {} places for each of the two contexts: the time an activity is next due, the continuation it goes on with, and for each function in it the place it returns to, its results, its parameters and what is alive across the places it is left at\ndata __act_due: array(i64, {n})\ndata __act_pc: array(i64, {n})\n", POOL, n = 2 * POOL));
            for (n, t) in &m.fields {
                out.push_str(&format!("data {}: array(i64, {}) ; {}\n", n, 2 * POOL, t));
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("\n; the activities' continuations (fm3 log 232)\n");
    for l in text {
        out.push_str(&l);
        out.push('\n');
    }
    Some(out)
}

/// how many items the wire holds one way (fm3 log 269): the pool has
/// a row for each, where a record in flight is
pub(super) const WIRE_Q: usize = 256;

/// every name a line of a function's body defines, with its type: a
/// plain definition, and a loop's own
fn scope_defs(head: &str) -> Vec<(String, String)> {
    let mut out = defs(head);
    if let Some(at) = head.find("loop(") {
        if head.ends_with(')') || head[at..].contains(") bound ") || head[at..].rfind(')').is_some() {
            out.extend(loop_params(&head[at..]).into_iter().map(|(n, t, _)| (n, t)));
        }
    }
    out
}

/// Why a function of a build for a wire stays on the machine's stack
/// (fm3 log 280): the kind of thing not yet held, `stream`, `value`,
/// `structure`, `float`, `shape`, `wired`, `defs`, and a sentence
#[derive(Clone, Debug)]
pub struct Why {
    pub kind: &'static str,
    pub said: String,
}

/// the text for a wire, and each function left on the stack with why
pub struct Wired {
    pub text: String,
    pub stack: std::collections::BTreeMap<String, Why>,
}

/// where a name of a stream's type came from, read off the lines that
/// define it: a field of the context, which is that field's stream;
/// made in the function; a parameter; or something a function gave
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Prov {
    Field(String),
    Made,
    Param(usize),
    Unknown,
}

pub(super) fn prov(f: &Func, name: &str) -> Prov {
    let mut n = name.to_string();
    for _ in 0..32 {
        let Some(l) = f.body.iter().map(|l| l.trim()).find(|l| defs(l).iter().any(|(d, _)| *d == n)) else {
            return f.params.iter().position(|(p, _)| *p == n).map_or(Prov::Unknown, Prov::Param);
        };
        let Some((_, rhs)) = l.split_once(" = ") else { return Prov::Unknown };
        if let Some(args) = rhs.strip_prefix("get ") {
            let a = split_top(args);
            // (a field of the context: what it is read from is a `__ctx`)
            if a.len() == 2 && f.body.iter().any(|l| l.trim().starts_with(&format!("{}: __ctx = ", a[0]))) {
                return Prov::Field(a[1].clone());
            }
            return Prov::Unknown;
        }
        if let Some(args) = rhs.strip_prefix("set ") {
            let Some(h) = split_top(args).first().cloned() else { return Prov::Unknown };
            n = h;
            continue;
        }
        let Some(open) = rhs.find('(') else { return Prov::Unknown };
        let g = &rhs[..open];
        if ["__queue_", "__copy_queue_", "__copy_", "__regular_", "__stream_"].iter().any(|p| g.starts_with(p)) {
            return Prov::Made;
        }
        // the reader of a stream moved on is a reader of that stream
        if matches!(g, "frame_queue" | "frame" | "advance" | "sampling") && rhs.ends_with(')') {
            let Some(h) = split_top(&rhs[open + 1..rhs.len() - 1]).first().cloned() else { return Prov::Unknown };
            n = h;
            continue;
        }
        return Prov::Unknown;
    }
    Prov::Unknown
}

/// a type wider than a word, to a person, and its kind: `value` says
/// whether a stream's type is known to hold a value, an array or a
/// string, or a stream
fn wide(ty: &str, value: Option<bool>) -> Why {
    if ty.ends_with("[]") || ty.contains('[') {
        return Why { kind: "value", said: "an array".into() };
    }
    if ty.ends_with('$') {
        return match value {
            Some(true) => Why { kind: "value", said: "an array or a string".into() },
            Some(false) => Why { kind: "stream", said: "a stream".into() },
            None => Why { kind: "stream", said: "a stream a function gave it".into() },
        };
    }
    if matches!(ty, "float" | "f16" | "f32" | "f64") {
        return Why { kind: "float", said: "a `float`".into() };
    }
    Why { kind: "structure", said: format!("a structure, `{}`", ty) }
}

/// what a name a function keeps is, where it is wider than a word
fn kept(f: &Func, name: &str, ty: &str, values: &HashMap<String, (Vec<String>, Vec<usize>)>) -> Why {
    if !ty.ends_with('$') {
        return wide(ty, None);
    }
    match prov(f, name) {
        Prov::Field(x) => Why { kind: "stream", said: format!("the stream `{}$`", x.trim_start_matches("__")) },
        Prov::Made => Why { kind: "value", said: "an array, a string or a stream it made".into() },
        Prov::Param(i) => wide(ty, Some(values.get(&parent(&f.name)).is_some_and(|(p, _)| p.contains(&f.params[i].0)))),
        Prov::Unknown => wide(ty, None),
    }
}

/// A call nested where the rewrite holds no place, an `if` in an `if`
/// or an `if` in a loop (fm3 log 269), is put out as a function of its
/// own: handed what it reads, giving what it makes that is read after.
/// The shapes held stay three. One statement of one function at a
/// time: the body with the call of the new function in its place, and
/// the new function's lines; nothing where every statement is held
fn outline(f: &Func, r: &BTreeSet<String>, n: usize, values: &HashMap<String, (Vec<String>, Vec<usize>)>) -> Result<Option<(Vec<String>, Vec<String>, String)>, Why> {
    let shape = |what: &str| Why { kind: "shape", said: format!("it has a call or a step where no place is held yet (fm3 question 139): {}", what) };
    let ss = stmts(&f.body, 1);
    let mut at = 0;
    for s in &ss {
        let first = at;
        at += s.lines.len();
        if kind(s, 1, r, true).is_some() {
            continue;
        }
        let head = s.head();
        if !(head.starts_with("loop(") || head.starts_with("if ")) {
            return Err(shape(head));
        }
        // the statements a level in, of the loop's body or of both branches
        let mut i = 1;
        while i < s.lines.len() {
            if depth(&s.lines[i]) != 2 {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < s.lines.len() && (depth(&s.lines[j]) > 2 || depth(&s.lines[j]) == 2 && s.lines[j].trim() == "else") {
                j += 1;
            }
            let child = Stmt { lines: s.lines[i..j].to_vec() };
            let held = !child.lines.iter().any(|l| leaves(l, r)) || matches!(kind(&child, 2, r, true), Some(Kind::Plain | Kind::At(_)));
            if held {
                i = j;
                continue;
            }
            // what leaves a function cannot be put out of it
            let mut loops: Vec<usize> = Vec::new();
            for l in &child.lines {
                let (d, t) = (depth(l), l.trim());
                loops.retain(|n| *n < d);
                if t == "ret" || t.starts_with("ret ") {
                    return Err(shape("a `ret` beneath it"));
                }
                if (t == "break" || t.starts_with("break ") || t == "continue" || t.starts_with("continue ")) && loops.is_empty() {
                    return Err(shape("a `break` or a `continue` of a loop around it"));
                }
                if t.contains("loop(") {
                    loops.push(d);
                }
            }
            let (a, b) = (first + i, first + j);
            let mut before: Vec<(String, String)> = f.params.clone();
            for l in &f.body[..a] {
                before.extend(scope_defs(l.trim()));
            }
            let own: Vec<(String, String)> = child.lines.iter().flat_map(|l| scope_defs(l.trim())).collect();
            let mut params: Vec<(String, String)> = Vec::new();
            let mut this = false;
            for l in &child.lines {
                for (_, tok) in tokens(l) {
                    if tok == "_this" {
                        this = true;
                    } else if let Some((pn, pt)) = before.iter().find(|(pn, _)| pn == tok) {
                        if !own.iter().any(|(o, _)| o == tok) && !params.iter().any(|(q, _)| q == tok) {
                            params.push((pn.clone(), pt.clone()));
                        }
                    }
                }
            }
            let outs: Vec<(String, String)> = defs(child.head()).into_iter().filter(|(o, _)| f.body[b..].iter().any(|l| names(l, o))).collect();
            // (a reader of a stream of the context is handed as what
            // is its own, three words, and read from the context again
            // in the function put out: fm3 log 282)
            let mut before_call: Vec<String> = Vec::new();
            let mut first: Vec<String> = Vec::new();
            let mut split: Vec<(String, String)> = Vec::new();
            for (pn, pt) in params.drain(..) {
                let field = if pt.ends_with('$') { if let Prov::Field(x) = prov(f, &pn) { Some(x) } else { None } } else { None };
                let Some(x) = field else {
                    split.push((pn, pt));
                    continue;
                };
                first.push(format!("    {}__c: ptr = context()", pn));
                first.push(format!("    {}__x: __ctx = load {}__c", pn, pn));
                first.push(format!("    {}__0: {} = get {}__x, {}", pn, pt, pn, x));
                let mut h = format!("{}__0", pn);
                for (k, (part, wt)) in [("pos", "index"), ("rule", "i64"), ("edge", "i64")].into_iter().enumerate() {
                    before_call.push(format!("        {}__{}: {} = get {}, {}", pn, part, wt, pn, part));
                    split.push((format!("{}__{}", pn, part), wt.to_string()));
                    let h2 = if k == 2 { pn.clone() } else { format!("{}__{}", pn, k + 1) };
                    first.push(format!("    {}: {} = set {}, {}, {}__{}", h2, pt, h, part, pn, part));
                    h = h2;
                }
            }
            let params = split;
            if let Some((pn, pt)) = params.iter().chain(&outs).find(|(_, t)| !word(t)) {
                let w = kept(f, pn, pt, values);
                return Err(Why { kind: w.kind, said: format!("it keeps `{}`, {}, across a call that may cross, and a place of the pool holds a word", pn, w.said) });
            }
            let name = format!("{}__o{}", f.name, n);
            let mut text = vec![format!("fn {}({}){}", name, params.iter().map(|(pn, pt)| format!("{}: {}", pn, pt)).collect::<Vec<_>>().join(", "), match outs.len() {
                0 => String::new(),
                1 => format!(" -> {}", outs[0].1),
                _ => format!(" -> ({})", outs.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>().join(", ")),
            })];
            if this {
                text.push("    _this: ptr = context()".to_string());
            }
            text.extend(first);
            text.extend(child.lines.iter().map(|l| l[4..].to_string()));
            text.push(if outs.is_empty() { "    ret".to_string() } else { format!("    ret {}", outs.iter().map(|(o, _)| o.clone()).collect::<Vec<_>>().join(", ")) });
            let call = format!("        {}{}({})", if outs.is_empty() { String::new() } else { format!("{} = ", outs.iter().map(|(o, t)| format!("{}: {}", o, t)).collect::<Vec<_>>().join(", ")) }, name, params.iter().map(|(pn, _)| pn.clone()).collect::<Vec<_>>().join(", "));
            let mut body = f.body[..a].to_vec();
            body.extend(before_call);
            body.push(call);
            body.extend(f.body[b..].iter().cloned());
            return Ok(Some((body, text, name)));
        }
        return Err(shape(head));
    }
    Ok(None)
}

/// the function a call was put out of, `f` for `f__o3`: it is the one
/// left as written where what was put out is not held either
fn parent(name: &str) -> String {
    let mut b = name;
    while let Some((x, n)) = b.rsplit_once("__o") {
        if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        b = x;
    }
    b.to_string()
}

/// The rewrite for a build whose wire takes time (fm3 cut.md, "a wire
/// that takes time"; log 269). Nothing may wait on the machine's stack
/// for something that crosses, so a call of a unit's function is a
/// place its caller is left at, as a step is: every function a turn
/// can reach that is a unit's and takes and gives words is written as
/// continuations, and so is whatever calls one on the way there. A
/// turn begins where the list calls a clock's line or a line at the
/// close of a slot, where a case's twin calls its function, and where
/// a function wired over a stream is called for an item. A record
/// takes a place of the pool where it begins or arrives and gives it
/// up where it is over or leaves; `__act_here` and `__act_send`, the
/// placed build's, say whether the continuation to go on with is at
/// this place and put the record on the wire where it is not. Why, by
/// name, where a function's shape is not held
pub fn rewrite_wire(ir: &str, units: &BTreeSet<String>, values: &HashMap<String, (Vec<String>, Vec<usize>)>) -> Result<Wired, String> {
    if ir.matches("data __ctx_mem: array(__ctx, 2)").count() != 1 {
        return Err("the store keeps no context: nothing in it can be placed".into());
    }
    let mut ir = ir.to_string();
    let mut helpers: BTreeSet<String> = BTreeSet::new();
    // what is a continuation: found, and found again as calls are put out
    // a function whose shape is not held, or that keeps more than a
    // word across a call, stays on the machine's stack, and what it
    // calls is then bound to its place: found by trying
    let mut left: BTreeSet<String> = BTreeSet::new();
    let mut why: std::collections::BTreeMap<String, Why> = std::collections::BTreeMap::new();
    let candidates = |fns: &[Func], helpers: &BTreeSet<String>, left: &BTreeSet<String>| -> (BTreeSet<String>, Vec<(String, String)>) {
        let by: HashMap<&str, &Func> = fns.iter().map(|f| (f.name.as_str(), f)).collect();
        // (a name with several definitions, one a type, is the IR's to choose among: it stays as written)
        let plain = |f: &Func| f.params.iter().all(|(_, t)| word(t)) && f.rets.iter().all(|t| word(t)) && !left.contains(&f.name) && fns.iter().filter(|g| g.name == f.name).count() == 1;
        // the functions from which a step or a unit's function is reached
        let mut need: BTreeSet<String> = fns.iter().filter(|f| units.contains(&f.name) || helpers.contains(&f.name)).map(|f| f.name.clone()).collect();
        loop {
            let more: Vec<String> = fns.iter().filter(|f| !need.contains(&f.name) && !matches!(f.name.as_str(), "__step" | "__wait" | "__turns") && f.body.iter().any(|l| called(l).iter().any(|c| *c == "__step" || need.contains(*c)))).map(|f| f.name.clone()).collect();
            if more.is_empty() {
                break;
            }
            need.extend(more);
        }
        // (a function wired over a stream is begun for an item where it
        // steps, as it was, fm3 log 232; one that does not is called)
        let mut steps: BTreeSet<String> = BTreeSet::new();
        loop {
            let more: Vec<String> = fns.iter().filter(|f| !steps.contains(&f.name) && !matches!(f.name.as_str(), "__step" | "__wait" | "__list_turn" | "__turns") && f.body.iter().any(|l| called(l).iter().any(|c| *c == "__step" || *c == "__wait" || steps.contains(*c)))).map(|f| f.name.clone()).collect();
            if more.is_empty() {
                break;
            }
            steps.extend(more);
        }
        let mut starts: Vec<(String, String)> = Vec::new();
        for f in fns {
            let targets: Vec<String> = f.body.iter().flat_map(|l| called(l)).map(str::to_string).collect();
            for g in targets {
                let Some(callee) = by.get(g.as_str()) else { continue };
                let begins = f.name.starts_with("__whole_") && g.starts_with("__case_") || f.name == "__list_turn" && need.contains(&g) && g != "__in_turn" || wired(f).is_some_and(|e| e == g) && steps.contains(&g);
                if begins && plain(callee) && !starts.contains(&(f.name.clone(), g.clone())) {
                    starts.push((f.name.clone(), g));
                }
            }
        }
        let mut r: BTreeSet<String> = BTreeSet::new();
        let mut todo: Vec<String> = starts.iter().map(|(_, g)| g.clone()).collect();
        while let Some(g) = todo.pop() {
            let Some(f) = by.get(g.as_str()) else { continue };
            if !need.contains(&g) && !g.starts_with("__case_") || !plain(f) || !r.insert(g.clone()) {
                continue;
            }
            todo.extend(f.body.iter().flat_map(|l| called(l)).map(str::to_string));
        }
        (r, starts)
    };
    let original = ir.clone();
    let (mut lines, fns, r, starts, mut m, order, mut text);
    let mut tries = 0;
    'again: loop {
        tries += 1;
        if tries > 200 {
            return Err("its functions could not be settled".into());
        }
        ir = original.clone();
        helpers.clear();
        for _ in 0..400 {
            let (ls, fs) = parse(&ir);
            let (r0, _) = candidates(&fs, &helpers, &left);
            let mut put: Option<(usize, usize, Vec<String>, Vec<String>, String)> = None;
            for f in fs.iter().filter(|f| r0.contains(&f.name)) {
                match outline(f, &r0, helpers.len() + 1, values) {
                    Ok(Some((body, t, name))) => {
                        put = Some((f.at.0, f.at.1, body, t, name));
                        break;
                    }
                    Ok(None) => {}
                    // (a function put out of another is that other's: it is the one left as written)
                    Err(e) => {
                        why.insert(parent(&f.name), e);
                        left.insert(parent(&f.name));
                        continue 'again;
                    }
                }
            }
            let Some((a, b, body, t, name)) = put else { break };
            helpers.insert(name);
            let mut out: Vec<String> = ls[..=a].to_vec();
            out.extend(body);
            out.extend(t);
            out.extend(ls[b..].iter().cloned());
            ir = out.join("\n");
            ir.push('\n');
        }
        let (ls, fs) = parse(&ir);
        let (r1, st) = candidates(&fs, &helpers, &left);
        if st.is_empty() {
            return Err("no case of it calls a function".into());
        }
        // (the functions are leaked for the life of the rewrite, which is one command)
        let fs: &'static [Func] = Box::leak(fs.into_boxed_slice());
        let r1: &'static BTreeSet<String> = Box::leak(Box::new(r1));
        let by1: HashMap<String, &Func> = fs.iter().map(|f| (f.name.clone(), f)).collect();
        // a function called with a value of another type than its
        // parameter's is one the IR makes anew for that type (`int`
        // handed an `i64`): it stays as written
        for f in fs.iter().filter(|f| r1.contains(&f.name)) {
            let mut types: HashMap<String, String> = f.params.iter().cloned().collect();
            for l in &f.body {
                types.extend(scope_defs(l.trim()));
            }
            for l in &f.body {
                let Some((d, g, args)) = call_of(l.trim()) else { continue };
                let Some(callee) = by1.get(&g).filter(|_| r1.contains(&g)) else { continue };
                let odd = args.iter().zip(&callee.params).any(|(a, (_, pt))| types.get(a).is_some_and(|t| t != pt)) || d.iter().zip(&callee.rets).any(|((_, t), rt)| t != rt) || args.len() != callee.params.len();
                if odd {
                    why.insert(parent(&g), Why { kind: "defs", said: format!("it is called by `{}` with a value of another type than its parameter's, and so is the IR's to make anew for that type", f.name) });
                    left.insert(parent(&g));
                    continue 'again;
                }
            }
        }
        let mut m1 = Maker { r: r1, fns: by1.clone(), ids: HashMap::new(), fields: Vec::new(), tmp: 0, held: BTreeSet::new(), here: 0, ifs: true };
        let mut order1: Vec<(bool, String, usize)> = Vec::new();
        for g in r1.iter() {
            let f = by1[g];
            let mut n = 0;
            order1.push((false, g.clone(), 0));
            for s in &stmts(&f.body, 1) {
                let Some(k) = kind(s, 1, r1, true) else {
                    why.insert(parent(g), Why { kind: "shape", said: "it has a call or a step where no place is held yet (fm3 question 139)".into() });
                    left.insert(parent(g));
                    continue 'again;
                };
                match k {
                    Kind::Plain => {}
                    Kind::At(_) => {
                        n += 1;
                        order1.push((false, g.clone(), n));
                    }
                    Kind::Loop(_, ps) => {
                        for _ in ps.iter().flatten() {
                            n += 1;
                            order1.push((true, g.clone(), n));
                        }
                    }
                    Kind::If(_, ps, ..) => {
                        for _ in ps.iter().flatten() {
                            n += 1;
                            order1.push((false, g.clone(), n));
                        }
                    }
                }
            }
        }
        order1.sort_by_key(|(inner, ..)| !*inner);
        for (k, (_, g, n)) in order1.iter().enumerate() {
            m1.ids.insert((g.clone(), *n), k as i64 + 2);
        }
        let mut text1: Vec<String> = Vec::new();
        for g in r1.iter() {
            let f = by1[g];
            m1.fields.push((m1.to(g), "i64".into()));
            for (k, t) in f.rets.iter().enumerate() {
                m1.fields.push((m1.res(g, k), t.clone()));
            }
            let before = m1.fields.len();
            let made = m1.make(f);
            // more than a word kept across a place: the function is left as written
            if made.is_none() || m1.fields[before..].iter().any(|(_, t)| !word(t)) {
                let prefix = format!("__act_{}__", g);
                let w = m1.fields[before..].iter().find(|(_, t)| !word(t)).map(|(n, t)| (n.strip_prefix(&prefix).unwrap_or(n).to_string(), kept(f, n.strip_prefix(&prefix).unwrap_or(n), t, values)));
                why.insert(parent(g), match w {
                    Some((n, w)) => Why { kind: w.kind, said: format!("it keeps `{}`, {}, across a call that may cross or a step, and a place of the pool holds a word", n, w.said) },
                    None => Why { kind: "shape", said: "it has a call or a step where no place is held yet (fm3 question 139)".into() },
                });
                left.insert(parent(g));
                continue 'again;
            }
            text1.extend(made.unwrap());
        }
        (lines, fns, r, starts, m, order, text) = (ls, fs, r1, st, m1, order1, text1);
        break;
    }
    let by: HashMap<String, &Func> = fns.iter().map(|f| (f.name.clone(), f)).collect();
    let ended = ir.contains("    __ended: u1\n");
    text.push("\n; the continuation of this number (fm3 log 232), which gives the one to go on with: 0, it stopped at a step; 1, the activity is over".into());
    text.push("fn __act_go(__a: index, k: i64) -> i64".into());
    for (_, g, n) in &order {
        let id = m.ids[&(g.clone(), *n)];
        text.push(format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        n{id}: i64 = {g}__{n}(__a)\n        ret n{id}", id = id, g = g, n = n));
    }
    text.push("    ret 1".into());
    let over = if ended { "\n        e: __ctx = load _this\n        gone: u1 = get e, __ended\n        if gone\n            e2: __ctx = set e, __ended, 0\n            store e2, _this\n            store {never}: i64, dp, __a, 8\n            store 0: i64, pp, __a, 8\n            break".replace("{never}", &NEVER.to_string()) } else { String::new() };
    text.push(format!("\n; a function partway goes on from a continuation until one stops at a step, or it is over, which a `restart` that ended it also says, or the one to go on with is at the other place (fm3 log 269): then its record is put on the wire. Over or gone, its place in the pool is free\nfn __act_run(__a: index, k: i64)\n{this}    dp: ptr = addr __act_due\n    pp: ptr = addr __act_pc\n    loop(c: i64 = k)\n        here: u1 = __act_here(c)\n        if here\n        else\n            __act_send(__a, c)\n            break\n        n: i64 = __act_go(__a, c){over}\n        on: u1 = cmp.gt n, 1\n        if on\n        else\n            done: u1 = cmp.eq n, 1\n            if done\n                store {never}: i64, dp, __a, 8\n                store 0: i64, pp, __a, 8\n            break\n        continue n\n    ret", this = if ended { "    _this: ptr = context()\n" } else { "" }, over = over, never = NEVER));
    text.push(format!("\n; the pool's word of the list: the least time any function partway at this place is due and which that is, of two at one time the first begun, wherever that was (fm3 question 155, log 278), and of two begun at one time the first to have begun or arrived here\nfn __act_next()\n    _this: ptr = context()\n    x: __ctx = load _this\n    n: index = get x, __act_n\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne _this, cm\n    sw: index = conv second\n    base: index = mul sw, {pool}\n    top: index = add base, n\n    dp: ptr = addr __act_due\n    bp: ptr = addr __act_born\n    gp: ptr = addr __act_begun\n    loop(k: index = base, b: i64 = {never}, i: index = base, o: i64 = 0, g: i64 = 0)\n        done: u1 = cmp.ge k, top\n        if done\n            y: __ctx = load _this\n            y2: __ctx = set y, __due_act, b\n            y3: __ctx = set y2, __act_i, i\n            store y3, _this\n            break\n        d: i64 = load dp, k, 8\n        w: i64 = load bp, k, 8\n        e: i64 = load gp, k, 8\n        sooner: u1 = cmp.lt d, b\n        same: u1 = cmp.eq d, b\n        elder: u1 = cmp.lt e, g\n        twin: u1 = cmp.eq e, g\n        older: u1 = cmp.lt w, o\n        both: u1 = and twin, older\n        first: u1 = or elder, both\n        tie: u1 = and same, first\n        live: u1 = cmp.lt d, {never}\n        tied: u1 = and tie, live\n        less: u1 = or sooner, tied\n        b2: i64 = if less\n            yield d\n        else\n            yield b\n        i2: index = if less\n            yield k\n        else\n            yield i\n        o2: i64 = if less\n            yield w\n        else\n            yield o\n        g2: i64 = if less\n            yield e\n        else\n            yield g\n        k2: index = add k, 1\n        continue k2, b2, i2, o2, g2\n    ret", never = NEVER, pool = POOL));
    text.push("\n; the pool's turn: the function that is due goes on\nfn __act_turn()\n    _this: ptr = context()\n    x: __ctx = load _this\n    i: index = get x, __act_i\n    pp: ptr = addr __act_pc\n    k: i64 = load pp, i, 8\n    __act_run(i, k)\n    __act_next()\n    ret".into());
    text.push(format!("\n; a place of this place's half of the pool (fm3 log 269): one given up, or the next never used; the order it came in; and the time, which is when a function that begins here was begun: one that arrives has its own, carried with it (fm3 log 278)\nfn __act_slot() -> index\n    _this: ptr = context()\n    x: __ctx = load _this\n    n: index = get x, __act_n\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne _this, cm\n    sw: index = conv second\n    base: index = mul sw, {pool}\n    top: index = add base, n\n    pp: ptr = addr __act_pc\n    found: index = loop(k: index = base)\n        done: u1 = cmp.ge k, top\n        if done\n            break k\n        pc: i64 = load pp, k, 8\n        free: u1 = cmp.eq pc, 0\n        if free\n            break k\n        k2: index = add k, 1\n        continue k2\n    fresh: u1 = cmp.eq found, top\n    if fresh\n        room: u1 = cmp.lt n, {pool}\n        check room\n        n2: index = add n, 1\n        y: __ctx = load _this\n        y2: __ctx = set y, __act_n, n2\n        store y2, _this\n    z: __ctx = load _this\n    b: i64 = get z, __act_b\n    b2: i64 = add b, 1\n    z2: __ctx = set z, __act_b, b2\n    store z2, _this\n    bp: ptr = addr __act_born\n    store b, bp, found, 8\n    gp: ptr = addr __act_begun\n    now: i64 = get z, __clock\n    store now, gp, found, 8\n    ret found", pool = POOL));
    let mut made: BTreeSet<String> = BTreeSet::new();
    for (caller, g) in &starts {
        if !r.contains(g) {
            continue;
        }
        let f = by[g];
        if made.insert(g.clone()) {
            let params: Vec<String> = f.params.iter().map(|(n, t)| format!("{}: {}", n, t)).collect();
            // (begun for an item of a stream by the function wired over
            // it: the one before it in this turn must not have gone to
            // the other place, or the two could be taken out of order)
            let once = if by.get(caller).is_some_and(|c| wired(c).is_some()) { "    __wonce()\n" } else { "" };
            let mut s = format!("\n; a function begins (fm3 log 232, 269): a place in the pool, its parameters, and {}\nfn __act_start_{g}({params}) -> index\n{once}    __a: index = __act_slot()\n", if g.starts_with("__case_") { "its first turn when the world gives it one: it is a case's call, due now" } else { "its first turn at once" }, g = g, params = params.join(", "), once = once);
            let mut ls = Vec::new();
            m.tmp = 0;
            for (n, t) in &f.params {
                let col = m.field(g, n, t);
                m.write(&col, n, t, "    ", &mut ls);
            }
            let col = m.to(g);
            m.write(&col, "1", "i64", "    ", &mut ls);
            m.write("__act_pc", &m.ids[&(g.clone(), 0)].to_string(), "i64", "    ", &mut ls);
            s.push_str(&ls.join("\n"));
            if g.starts_with("__case_") {
                s.push_str("\n    __c: ptr = context()\n    __x: __ctx = load __c\n    __now: i64 = get __x, __clock\n    __dp: ptr = addr __act_due\n    store __now, __dp, __a, 8\n    __act_next()\n    ret __a");
            } else {
                s.push_str(&format!("\n    __act_run(__a, {})\n    __act_next()\n    ret __a", m.ids[&(g.clone(), 0)]));
            }
            text.push(s);
        }
        let c = by[caller];
        let mut n = 0;
        for k in c.at.0 + 1..c.at.1 {
            if let Some((d, name, args)) = call_of(lines[k].trim()) {
                if name == *g {
                    if !d.is_empty() {
                        return Err(format!("`{}` is begun by `{}`, which waits for what it gives", g, caller));
                    }
                    n += 1;
                    let pad = lines[k][..lines[k].len() - lines[k].trim_start().len()].to_string();
                    lines[k] = format!("{}__act_w{}_{}: index = __act_start_{}({})", pad, made.len(), n, g, args.join(", "));
                }
            }
        }
    }
    if let Some((_, t)) = m.fields.iter().find(|(_, t)| !word(t)) {
        return Err(format!("a function keeps a `{}` across a call that may cross or a step, and a place of the pool holds a word", t));
    }
    let kept: BTreeSet<String> = {
        let mut kept = BTreeSet::new();
        let mut todo: Vec<String> = fns.iter().filter(|f| !r.contains(&f.name)).flat_map(|f| lines[f.at.0 + 1..f.at.1].iter().flat_map(|l| called(l)).map(str::to_string).collect::<Vec<_>>()).filter(|c| r.contains(c)).collect();
        while let Some(g) = todo.pop() {
            if kept.insert(g.clone()) {
                todo.extend(by[&g].body.iter().flat_map(|l| called(l)).filter(|c| r.contains(*c)).map(str::to_string));
            }
        }
        kept
    };
    // why each function still on the machine's stack is (fm3 log 280):
    // its own reason, or that of what calls it and waits for it
    let mut stack: std::collections::BTreeMap<String, Why> = std::collections::BTreeMap::new();
    let on_stack = |f: &Func| !r.contains(&f.name) || kept.contains(&f.name);
    for f in fns.iter().filter(|f| on_stack(f)) {
        let vals = values.get(&f.name);
        let own = if fns.iter().filter(|g| g.name == f.name).count() > 1 {
            Some(Why { kind: "defs", said: "it has several definitions, one a type, and is the IR's to choose among".into() })
        } else if let Some((i, (n, t))) = f.params.iter().enumerate().find(|(_, (_, t))| !word(t)) {
            let _ = i;
            let w = wide(t, if t.ends_with('$') { vals.map(|(p, _)| p.contains(n)) } else { None });
            Some(Why { kind: w.kind, said: format!("it is handed {}", if t.ends_with('$') && vals.is_none() { "a stream, an array or a string" } else { w.said.as_str() }) })
        } else if let Some((k, t)) = f.rets.iter().enumerate().find(|(_, t)| !word(t)) {
            let w = wide(t, if t.ends_with('$') { vals.map(|(_, g)| g.contains(&k)) } else { None });
            Some(Why { kind: w.kind, said: format!("it gives {}", if t.ends_with('$') && vals.is_none() { "a stream, an array or a string" } else { w.said.as_str() }) })
        } else if let Some(w) = why.get(&f.name) {
            Some(w.clone())
        } else if f.name.ends_with("_each") && fns.iter().any(|g| wired(g).is_some_and(|e| e == f.name)) {
            Some(Why { kind: "wired", said: "it is the function of a line wired over a stream, which is called for each item, and not begun, where it does not step".into() })
        } else {
            None
        };
        if let Some(w) = own {
            stack.insert(f.name.clone(), w);
        }
    }
    loop {
        let mut more: Vec<(String, Why)> = Vec::new();
        for f in fns.iter().filter(|f| on_stack(f) && !stack.contains_key(&f.name)) {
            let caller = fns.iter().find(|g| stack.contains_key(&g.name) && lines[g.at.0 + 1..g.at.1].iter().any(|l| called(l).contains(&f.name.as_str())));
            if let Some(g) = caller {
                let w = &stack[&g.name];
                more.push((f.name.clone(), Why { kind: w.kind, said: format!("it is called by `{}`, which waits for it on the stack: {}", g.name, w.said.strip_prefix("it is called by ").map_or(w.said.clone(), |_| w.said.clone())) }));
            }
        }
        if more.is_empty() {
            break;
        }
        stack.extend(more);
    }
    // nothing of the program left as it was written may wait on the
    // machine's stack: a unit's function still on it that reaches a
    // step, or a continuation that calls one of the language's own
    // that waits
    let mut waits: BTreeSet<String> = BTreeSet::new();
    loop {
        let more: Vec<String> = fns.iter().filter(|f| !waits.contains(&f.name) && !matches!(f.name.as_str(), "__step" | "__wait" | "__list_turn" | "__turns") && lines[f.at.0 + 1..f.at.1].iter().any(|l| called(l).iter().any(|c| *c == "__step" || *c == "__wait" || waits.contains(*c)))).map(|f| f.name.clone()).collect();
        if more.is_empty() {
            break;
        }
        waits.extend(more);
    }
    for f in fns.iter() {
        if r.contains(&f.name) {
            if let Some(c) = f.body.iter().flat_map(|l| called(l)).find(|c| waits.contains(*c) && !r.contains(*c)) {
                return Err(format!("`{}` calls `{}`, which waits on the machine's stack", f.name, c));
            }
        }
        if (units.contains(&f.name) || f.name.starts_with("__case_")) && (!r.contains(&f.name) || kept.contains(&f.name)) && waits.contains(&f.name) {
            return Err(format!("`{}` keeps the machine's stack across a step of a rate: {}", f.name, stack.get(&f.name).map_or("it is not reached from where a turn begins", |w| w.said.as_str())));
        }
    }
    let mut out = String::new();
    let gone: Vec<(usize, usize)> = fns.iter().filter(|f| r.contains(&f.name) && !kept.contains(&f.name)).map(|f| f.at).collect();
    let rows = 2 * POOL + 2 * WIRE_Q;
    for (k, l) in lines.iter().enumerate() {
        if gone.iter().any(|(a, b)| k >= *a && k < *b) {
            continue;
        }
        if l.starts_with("data __ctx_mem: ") {
            out.push_str(&format!("; the functions partway (fm3 log 232, 269): a function left at a step or at a call that may cross, and whatever called it. The pool is a column for each thing kept, {} places for each of the two places and one for each item the wire may hold: the time a function is next due, the continuation it goes on with (0, the place is free), the order it came in, the time it was begun, and for each function the place it returns to, its results, its parameters and what is alive across the places it is left at\ndata __act_due: array(i64, {n})\ndata __act_pc: array(i64, {n})\ndata __act_born: array(i64, {n})\ndata __act_begun: array(i64, {n})\n", POOL, n = rows));
            for (n, t) in &m.fields {
                out.push_str(&format!("data {}: array(i64, {}) ; {}\n", n, rows, t));
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("\n; the continuations (fm3 log 232, 269)\n");
    for l in text {
        out.push_str(&l);
        out.push('\n');
    }
    Ok(Wired { text: out, stack })
}
