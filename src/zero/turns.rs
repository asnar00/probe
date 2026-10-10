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
const POOL: usize = 64;
const NEVER: i64 = 1 << 62;

struct Func {
    name: String,
    params: Vec<(String, String)>,
    rets: Vec<String>,
    /// the body's lines, indentation kept
    body: Vec<String>,
    /// the lines of the text it stands at, `fn` line first
    at: (usize, usize),
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn tokens(s: &str) -> Vec<(usize, &str)> {
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
fn called(line: &str) -> Vec<&str> {
    tokens(line).into_iter().filter(|(at, t)| line[at + t.len()..].starts_with('(')).map(|(_, t)| t).collect()
}

fn split_top(s: &str) -> Vec<String> {
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

fn parse(ir: &str) -> (Vec<String>, Vec<Func>) {
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

fn kind(s: &Stmt, d: usize, r: &BTreeSet<String>) -> Option<Kind> {
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
            match kind(b, 2, r)? {
                Kind::Plain => ps.push(None),
                Kind::At(p) => ps.push(Some(p)),
                Kind::Loop(..) => return None,
            }
        }
        return Some(Kind::Loop(body, ps));
    }
    None
}

fn is_context(s: &Stmt) -> bool {
    s.lines.len() == 1 && s.head().ends_with(": ptr = context()")
}

/// a value a column of the pool can hold: one word or less
fn word(ty: &str) -> bool {
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
            let col = self.field(&f.name, n, ty);
            self.write(&col, n, ty, pad, out);
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
                    out.push(ss[j].lines[0].clone());
                    self.body(f, ss, kinds, j, body, ps, 0, out);
                }
            }
        }
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
                Kind::Loop(_, ps) => {
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
            kinds.push(kind(s, 1, self.r)?);
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
                        let col = self.field(&f.name, n, t);
                        self.read(&col, n, t, "    ", &mut out);
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
                            let col = self.field(&f.name, n, t);
                            self.read(&col, n, t, "    ", &mut out);
                        }
                        // the loop begins again from the number it
                        // was left with, what followed the place run
                        // first, under names of its own
                        let mut heads = Vec::new();
                        for (n, t, _) in loop_params(ss[j].head()) {
                            let col = self.field(&f.name, &n, &t);
                            self.read(&col, &format!("{}__s", n), &t, "    ", &mut out);
                            heads.push(format!("{}: {} = {}__s", n, t, n));
                        }
                        let mut map = HashMap::new();
                        for (n, t) in self.live_body(body, i) {
                            let col = self.field(&f.name, &n, &t);
                            self.read(&col, &format!("{}__r", n), &t, "    ", &mut out);
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
                        self.top(f, &ss, &kinds, j + 1, &mut out);
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
pub fn rewrite(ir: &str) -> Option<String> {
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
    if !starts.iter().any(|(f, _)| !f.starts_with("__whole_")) {
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
    let mut m = Maker { r: &r, fns: by.iter().map(|(k, v)| (k.clone(), *v)).collect(), ids: HashMap::new(), fields: Vec::new(), tmp: 0, held: BTreeSet::new(), here: 0 };
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
            match kind(s, 1, &r)? {
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
            let mut s = format!("\n; an activity starts: a place in the pool, its parameters, and its first turn at once\nfn __act_start_{g}({params}) -> index\n    _this: ptr = context()\n    x: __ctx = load _this\n    n: index = get x, __act_n\n    room: u1 = cmp.lt n, {pool}\n    check room\n    n2: index = add n, 1\n    x2: __ctx = set x, __act_n, n2\n    store x2, _this\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne _this, cm\n    sw: index = conv second\n    base: index = mul sw, {pool}\n    __a: index = add base, n\n", g = g, params = params.join(", "), pool = POOL);
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
