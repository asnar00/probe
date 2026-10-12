//! The placed build (fm3 cut.md, log 261): one unchanged program at two
//! places in one process. `screen` is the store's first context and has
//! the devices, the output, the input and the list of things going on;
//! `far` is its second and has none. Each has memory of its own. A cut
//! says, for each unit, a function or a line that stands, which of the
//! two it runs at, and the cut is data: a table, `__cut`, that the
//! runner fills before a case, so one compiled program runs at every
//! cut.
//!
//! This file rewrites the finished text, as `turns.rs` does. A unit's
//! function asks the wire for its own place where it begins and for the
//! place it was called from where it returns. The platform's writes ask
//! for the screen. A push into a stored stream is made again at the
//! other place where the stream is read there. And the wire, `__wire`,
//! the one function that changes the current context, carries the time
//! and each field of the context that the cut says crosses: one written
//! at the place left and read or written at the place arrived at. Which
//! those are is read off the text, function by function, and given to
//! the runner as the calls that fill the table.

use super::turns::{called, parse, split_top, tokens, Func, POOL};
use std::collections::{BTreeSet, HashMap, HashSet};

/// A unit: what a cut places. A function the program declares, each
/// definition of a chain its own, or a line that stands
#[derive(Clone, Debug)]
pub struct Unit {
    /// where it is written, `countdown.zero:2`, which is its name
    pub name: String,
    /// a function's words, `count down`
    pub words: Option<String>,
    /// a line as the program wrote it
    pub said: Option<String>,
    /// the functions of the text that are it
    pub irs: Vec<String>,
}

impl Unit {
    /// as a person is shown it: a function's words, or a line
    pub fn shown(&self) -> String {
        self.words.clone().or_else(|| self.said.clone()).unwrap_or_else(|| self.name.clone())
    }
}

/// the context's own output and clock (fm3 question 107): never
/// carried as fields. The time is carried by the wire itself
const OWN: [&str; 4] = ["__out_n", "__clock", "__out_p", "__out_m"];

/// the words that put something into a stream's storage, or end it
const PUSHES: [&str; 7] = ["push_queue", "push_queue_open", "push_queue_few", "push", "__push", "end", "open"];

/// the platform's functions that are the screen's and may be called
/// from either place: the two writes of the output device, the input's
/// push, the list of things going on, the scheduler of the lines that
/// walk a stored stream, and the start of a function left at its steps
fn screens(name: &str) -> bool {
    matches!(name, "__out_ch" | "__out_block" | "__in_ch" | "__turns" | "__run" | "__run_copy") || name.starts_with("__act_start_")
}

/// the runner's own entries, called at the screen
fn runners(name: &str) -> bool {
    matches!(name, "__zero_start" | "__zero_reset" | "__zero_new" | "__zero_context" | "__out_len" | "__out_byte" | "__out_mark" | "__in_at") || name.starts_with("__set___enabled_") || name.starts_with("__whole_") || name.starts_with("__said_")
}

/// what one function of the text does to the context
#[derive(Default, Clone, Debug)]
struct Facts {
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    ring_reads: BTreeSet<usize>,
    ring_pushes: BTreeSet<usize>,
    /// the fields that hold a stream through which it is read here:
    /// one reader, whose place in the stream is that field's
    through: BTreeSet<String>,
    /// ... and those of them whose reader takes what it reads here: it
    /// moves on, gives its slots back, or is handed to a function that
    /// may
    taken: BTreeSet<String>,
    calls: BTreeSet<String>,
}

impl Facts {
    fn add(&mut self, o: &Facts) {
        self.reads.extend(o.reads.iter().cloned());
        self.writes.extend(o.writes.iter().cloned());
        self.ring_reads.extend(o.ring_reads.iter().copied());
        self.ring_pushes.extend(o.ring_pushes.iter().copied());
        self.through.extend(o.through.iter().cloned());
        self.taken.extend(o.taken.iter().cloned());
    }
}

/// a push the rewrite makes again at the other place: the line of the
/// text, the handle it pushes through, the field that handle was read
/// from, the handle's type and the stream
struct Site {
    at: usize,
    handle: String,
    field: String,
    ty: String,
    ring: usize,
}

/// a call from one entity's functions into another's
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Crossing {
    from: usize,
    caller: String,
    to: usize,
    callee: String,
}

/// A store's placed build as the runner needs it: the units, and what
/// each does, from which any cut's table is worked out
#[derive(Clone, Debug)]
pub struct Placed {
    pub units: Vec<Unit>,
    /// what each entity does: a row a unit, and a last row the screen's
    facts: Vec<Facts>,
    crossings: Vec<Crossing>,
    /// the fields the wire can carry, in the order of their flags, each
    /// with its type and whether a value of that type holds no address
    fields: Vec<(String, String, bool)>,
    /// the stored streams: the fields that hold each one's storage, and
    /// whether every push into it is one the rewrite can make again
    rings: Vec<(Vec<String>, bool)>,
    /// each function's parameters' and results' types
    sigs: HashMap<String, Vec<String>>,
}

/// a cut, as the runner uses it
#[derive(Clone, Debug)]
pub struct Cut {
    /// the calls that fill the table, made after the reset and before
    /// anything else
    pub before: Vec<(String, Vec<i64>)>,
    /// `far: count down`, for a case's line
    pub label: String,
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn names(s: &str, name: &str) -> bool {
    tokens(s).iter().any(|(_, t)| *t == name)
}

/// a line's own indentation
fn pad(l: &str) -> &str {
    &l[..l.len() - l.trim_start().len()]
}

/// `a: T = rhs` as (a, T, rhs), for a line that defines one name
fn def(t: &str) -> Option<(&str, &str, &str)> {
    let (l, r) = t.split_once(" = ")?;
    let (n, ty) = l.split_once(": ")?;
    (!n.is_empty() && n.chars().all(is_ident)).then_some((n, ty, r))
}

/// every name a line defines with its type, `a: T, b: U = rhs`
fn defs(t: &str) -> Vec<(String, String)> {
    let Some((l, _)) = t.split_once(" = ") else { return Vec::new() };
    if ["loop(", "if ", "store ", "check ", "ret", "break", "continue", "yield"].iter().any(|w| t.starts_with(w)) {
        return Vec::new();
    }
    split_top(l).iter().filter_map(|part| part.split_once(": ").map(|(n, ty)| (n.to_string(), ty.to_string()))).filter(|(n, _)| !n.is_empty() && n.chars().all(is_ident)).collect()
}

/// is the line one of the stream's pushes, a statement, through this
/// handle: `push_queue(s, v)`, `push_queue<token>(s, v)`, `push s, v`
fn push_stmt(t: &str, s: &str) -> bool {
    if t.contains(" = ") {
        return false;
    }
    if let Some(rest) = t.strip_prefix("push ") {
        return rest.split(',').next().map(str::trim) == Some(s);
    }
    let Some(open) = t.find('(') else { return false };
    let word = t[..open].split('<').next().unwrap_or("");
    if !PUSHES.contains(&word) || !t.ends_with(')') {
        return false;
    }
    split_top(&t[open + 1..t.len() - 1]).first().map(String::as_str) == Some(s)
}

/// does a value of this type hold no address? A number, a bit, and a
/// structure of them; not a stream, an array, a string or a pointer
fn carried(ty: &str, structs: &HashMap<String, Vec<String>>) -> bool {
    if ty.contains('$') || ty.contains('[') || ty.contains("ptr") {
        return false;
    }
    match structs.get(ty) {
        Some(fields) => fields.iter().all(|f| carried(f, structs)),
        None => true,
    }
}

/// what an address is, in a refusal's words
fn address(ty: &str) -> &'static str {
    if ty == "u8$" || ty == "u8[]" {
        "a string"
    } else if ty.contains('$') || ty.contains('[') {
        "an array or a stream"
    } else {
        "an address"
    }
}

struct Reader<'a> {
    fns: &'a [Func],
    by: HashMap<&'a str, &'a Func>,
    /// the fields that hold a stream's storage, and which stream
    handles: HashMap<String, usize>,
    /// (function, parameter) pairs known to be pushed through
    pushed: HashMap<(String, usize), bool>,
    unsplit: BTreeSet<usize>,
    sites: Vec<Site>,
}

impl<'a> Reader<'a> {
    /// Is a stream handed to this function as its i-th parameter pushed
    /// into through it, there or in what it hands it on to?
    fn param_pushes(&mut self, g: &str, i: usize, seen: &mut HashSet<(String, usize)>) -> bool {
        if let Some(&p) = self.pushed.get(&(g.to_string(), i)) {
            return p;
        }
        if !seen.insert((g.to_string(), i)) {
            return false;
        }
        let Some(f) = self.by.get(g).copied() else { return false };
        let Some((p, _)) = f.params.get(i) else { return false };
        let mut held: Vec<String> = vec![p.clone()];
        let mut found = false;
        for l in &f.body {
            let t = l.trim();
            let named: Vec<String> = held.iter().filter(|s| names(t, s)).cloned().collect();
            if named.is_empty() {
                continue;
            }
            if named.iter().any(|s| push_stmt(t, s)) {
                found = true;
                continue;
            }
            for (n, ty) in defs(t) {
                if ty.ends_with('$') && !held.iter().any(|h| *h == n) {
                    held.push(n);
                }
            }
            for s in named {
                found |= self.handed(t, &s, seen);
            }
        }
        self.pushed.insert((g.to_string(), i), found);
        found
    }

    /// a line that hands a stream to a function of the text: is it
    /// pushed into there?
    fn handed(&mut self, t: &str, s: &str, seen: &mut HashSet<(String, usize)>) -> bool {
        let mut found = false;
        for (at, name) in tokens(t) {
            if !self.by.contains_key(name) || !t[at + name.len()..].starts_with('(') {
                continue;
            }
            let rest = &t[at + name.len() + 1..];
            let close = rest.rfind(')').unwrap_or(rest.len());
            for (i, a) in split_top(&rest[..close]).iter().enumerate() {
                if a == s {
                    found |= self.param_pushes(name, i, seen);
                }
            }
        }
        found
    }

    /// what one function does to the context, and the pushes in it the
    /// rewrite makes again
    fn facts(&mut self, f: &Func) -> Facts {
        let mut out = Facts::default();
        let ctx: BTreeSet<&str> = f.body.iter().filter_map(|l| def(l.trim())).filter(|(_, ty, _)| *ty == "__ctx").map(|(n, _, _)| n).collect();
        // the handles read from the context, and the handles made of them
        let mut direct: HashMap<String, (String, String)> = HashMap::new();
        let mut held: Vec<(String, usize, String)> = Vec::new();
        for (k, l) in f.body.iter().enumerate() {
            let t = l.trim();
            out.calls.extend(called(t).into_iter().filter(|c| self.by.contains_key(c)).map(str::to_string));
            if let Some((n, ty, rhs)) = def(t) {
                if let Some(args) = rhs.strip_prefix("get ") {
                    if let [v, field] = split_top(args).as_slice() {
                        if ctx.contains(v.as_str()) && !OWN.contains(&field.as_str()) {
                            match self.handles.get(field) {
                                Some(&r) => {
                                    direct.insert(n.to_string(), (field.clone(), ty.to_string()));
                                    held.push((n.to_string(), r, field.clone()));
                                }
                                None => {
                                    out.reads.insert(field.clone());
                                }
                            }
                            continue;
                        }
                    }
                }
                if let Some(args) = rhs.strip_prefix("set ") {
                    if let [v, field, _] = split_top(args).as_slice() {
                        if ty == "__ctx" && ctx.contains(v.as_str()) && !OWN.contains(&field.as_str()) {
                            match self.handles.get(field) {
                                // a reader's own place in the stream, moved on
                                Some(&r) => {
                                    out.ring_reads.insert(r);
                                    out.through.insert(field.clone());
                                    out.taken.insert(field.clone());
                                }
                                None => {
                                    out.writes.insert(field.clone());
                                }
                            }
                            continue;
                        }
                    }
                }
            }
            // every handle the line names: a push through it, a read
            // of it, or its being handed on
            let named: Vec<(String, usize, String)> = held.iter().filter(|(s, _, _)| names(t, s)).cloned().collect();
            for (s, r, through) in named {
                if push_stmt(t, &s) {
                    out.ring_pushes.insert(r);
                    match direct.get(&s) {
                        Some((field, ty)) => self.sites.push(Site { at: f.at.0 + 1 + k, handle: s.clone(), field: field.clone(), ty: ty.clone(), ring: r }),
                        None => {
                            self.unsplit.insert(r);
                        }
                    }
                    continue;
                }
                // any other word that would push, the compiler does not know
                if PUSHES.iter().any(|w| names(t, w)) && !t.starts_with("store ") {
                    out.ring_pushes.insert(r);
                    self.unsplit.insert(r);
                }
                out.ring_reads.insert(r);
                out.through.insert(through.clone());
                if ["advance", "free_queue"].iter().any(|w| names(t, w)) || called(t).iter().any(|c| self.by.contains_key(c)) {
                    out.taken.insert(through.clone());
                }
                for (n, ty) in defs(t) {
                    if ty.ends_with('$') && !held.iter().any(|(h, hr, _)| *h == n && *hr == r) {
                        held.push((n, r, through.clone()));
                    }
                }
                if self.handed(t, &s, &mut HashSet::new()) {
                    out.ring_pushes.insert(r);
                    self.unsplit.insert(r);
                }
            }
        }
        out
    }
}

/// The rewrite: the text of a placed build, and what the runner needs
/// to say a cut. `units` are the lowering's; one with no function in
/// the text is not placed and is left out
pub fn rewrite(ir: &str, units: &[Unit]) -> Result<(String, Placed), String> {
    let (lines, fns) = parse(ir);
    let present: HashSet<&str> = fns.iter().map(|f| f.name.as_str()).collect();
    if !lines.iter().any(|l| l.starts_with("data __ctx_mem: array(__ctx, 2)")) {
        return Err("the store keeps no context: nothing in it can be placed".into());
    }
    // a function with a body for a machine is the machine's, and stays
    // where it is called
    let machine: HashSet<&str> = fns.iter().filter(|f| lines.get(f.at.1).is_some_and(|l| l.starts_with("platform "))).map(|f| f.name.as_str()).collect();
    // a continuation of a unit's function (`turns.rs`), `f__2`, is the unit's
    let base = |name: &str| -> String {
        match name.rsplit_once("__") {
            Some((b, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && units.iter().any(|u| u.irs.iter().any(|i| i == b)) && present.contains(format!("{}__0", b).as_str()) => b.to_string(),
            _ => name.to_string(),
        }
    };
    let units: Vec<Unit> = units.iter().filter(|u| fns.iter().any(|f| !machine.contains(f.name.as_str()) && u.irs.contains(&base(&f.name)))).cloned().collect();
    let nu = units.len();
    let screen = nu;
    let mut owner: HashMap<String, usize> = HashMap::new();
    for f in &fns {
        if machine.contains(f.name.as_str()) {
            continue;
        }
        let b = base(&f.name);
        if let Some(u) = units.iter().position(|u| u.irs.contains(&b)) {
            owner.insert(f.name.clone(), u);
        } else if screens(&f.name) || runners(&f.name) {
            owner.insert(f.name.clone(), screen);
        }
    }
    // the context's fields, and the structures a field may be
    let mut structs: HashMap<String, Vec<String>> = HashMap::new();
    let mut ctx_fields: Vec<(String, String)> = Vec::new();
    {
        let mut k = 0;
        while k < lines.len() {
            if let Some(name) = lines[k].strip_prefix("type ").and_then(|l| l.strip_suffix(" = struct")) {
                let mut fields = Vec::new();
                k += 1;
                while k < lines.len() && lines[k].starts_with("    ") {
                    if let Some((n, ty)) = lines[k].trim().split_once(": ") {
                        fields.push((n.to_string(), ty.to_string()));
                    }
                    k += 1;
                }
                if name == "__ctx" {
                    ctx_fields = fields.clone();
                }
                structs.insert(name.to_string(), fields.into_iter().map(|(_, t)| t).collect());
            } else {
                k += 1;
            }
        }
    }
    // the stored streams: a field whose type is a stream holds one's
    // storage, and two fields the reset gives one value hold the same
    // one (a stream and a reader's own place in it)
    let mut handles: HashMap<String, usize> = HashMap::new();
    let mut rings: Vec<Vec<String>> = Vec::new();
    {
        let new = fns.iter().find(|f| f.name == "__zero_new").ok_or("the store has no reset")?;
        let packed = new.body.iter().filter_map(|l| def(l.trim())).find(|(_, ty, rhs)| *ty == "__ctx" && rhs.starts_with("pack ")).map(|(_, _, rhs)| split_top(&rhs["pack ".len()..])).unwrap_or_default();
        let mut made: HashMap<String, usize> = HashMap::new();
        for (k, (name, ty)) in ctx_fields.iter().enumerate() {
            if !ty.ends_with('$') {
                continue;
            }
            let value = packed.get(k).cloned().unwrap_or_else(|| name.clone());
            let r = *made.entry(value).or_insert_with(|| {
                rings.push(Vec::new());
                rings.len() - 1
            });
            rings[r].push(name.clone());
            handles.insert(name.clone(), r);
        }
    }
    // ... but a field of a stream's type that is given a new value
    // whole, `open_tool$ << "edit"` where the stream is kept as its
    // latest and that is a string, holds a value and not a stream's
    // storage: it is a field like any other, and what it holds is an
    // address
    for f in &fns {
        let ctx: BTreeSet<&str> = f.body.iter().filter_map(|l| def(l.trim())).filter(|(_, ty, _)| *ty == "__ctx").map(|(n, _, _)| n).collect();
        // the names of this function that are a field's stream, as it
        // was read or moved on
        let mut of: Vec<(String, String)> = Vec::new();
        for l in &f.body {
            let t = l.trim();
            let ds = defs(t);
            let rhs = t.split_once(" = ").map(|(_, r)| r).unwrap_or("");
            if let ([(n, ty)], Some(args)) = (ds.as_slice(), rhs.strip_prefix("get ")) {
                if let [v, field] = split_top(args).as_slice() {
                    if ctx.contains(v.as_str()) && handles.contains_key(field) {
                        let _ = ty;
                        of.push((n.clone(), field.clone()));
                        continue;
                    }
                }
            }
            if let ([(_, ty)], Some(args)) = (ds.as_slice(), rhs.strip_prefix("set ")) {
                if let ([v, field, value], "__ctx") = (split_top(args).as_slice(), ty.as_str()) {
                    if ctx.contains(v.as_str()) && handles.contains_key(field) && !of.iter().any(|(h, hf)| h == value && hf == field) {
                        handles.remove(field);
                    }
                    continue;
                }
            }
            // (a line may give several things, `v, n, s2 = frame_queue(s)`:
            // the stream among them is the one it was handed, moved on)
            if let Some((_, field)) = of.iter().find(|(h, _)| names(rhs, h)).cloned() {
                of.extend(ds.into_iter().filter(|(_, ty)| ty.ends_with('$')).map(|(n, _)| (n, field.clone())));
            }
        }
    }
    for names in rings.iter_mut() {
        names.retain(|n| handles.contains_key(n));
    }
    let by: HashMap<&str, &Func> = fns.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut reader = Reader { fns: &fns, by, handles, pushed: HashMap::new(), unsplit: BTreeSet::new(), sites: Vec::new() };
    let mut own: HashMap<String, Facts> = HashMap::new();
    for f in reader.fns {
        let facts = reader.facts(f);
        own.insert(f.name.clone(), facts);
    }
    // what each entity does: its own functions and every function of
    // nobody's that they reach, which runs where it is called; and a
    // function nothing reaches is the runner's to call, at the screen
    let mut facts: Vec<Facts> = vec![Facts::default(); nu + 1];
    let mut crossings: BTreeSet<Crossing> = BTreeSet::new();
    let mut reached: HashSet<String> = HashSet::new();
    let mut walk = |e: usize, roots: Vec<String>, owner: &HashMap<String, usize>, facts: &mut Vec<Facts>, reached: &mut HashSet<String>| {
        let mut seen: HashSet<String> = HashSet::new();
        let mut todo = roots;
        while let Some(g) = todo.pop() {
            if !seen.insert(g.clone()) {
                continue;
            }
            reached.insert(g.clone());
            let f = &own[&g];
            facts[e].add(f);
            for c in &f.calls {
                match owner.get(c) {
                    Some(&o) if o != e => {
                        crossings.insert(Crossing { from: e, caller: g.clone(), to: o, callee: c.clone() });
                    }
                    _ => todo.push(c.clone()),
                }
            }
        }
    };
    for e in 0..=nu {
        let roots: Vec<String> = fns.iter().filter(|f| owner.get(&f.name) == Some(&e)).map(|f| f.name.clone()).collect();
        walk(e, roots, &owner, &mut facts, &mut reached);
    }
    loop {
        let Some(f) = fns.iter().find(|f| !reached.contains(&f.name)) else { break };
        owner.insert(f.name.clone(), screen);
        walk(screen, vec![f.name.clone()], &owner, &mut facts, &mut reached);
    }
    // the fields the wire may carry: written somewhere, and reached by
    // more than one entity
    let mut fields: Vec<(String, String, bool)> = Vec::new();
    for (name, ty) in &ctx_fields {
        if OWN.contains(&name.as_str()) || reader.handles.contains_key(name) {
            continue;
        }
        let writers = facts.iter().filter(|f| f.writes.contains(name)).count();
        let users = facts.iter().filter(|f| f.writes.contains(name) || f.reads.contains(name)).count();
        if writers > 0 && users > 1 {
            fields.push((name.clone(), ty.clone(), carried(ty, &structs)));
        }
    }
    let rings: Vec<(Vec<String>, bool)> = rings.into_iter().enumerate().map(|(r, names)| (names, !reader.unsplit.contains(&r))).collect();
    let sigs: HashMap<String, Vec<String>> = fns.iter().map(|f| (f.name.clone(), f.params.iter().map(|(_, t)| t.clone()).chain(f.rets.iter().cloned()).collect())).collect();
    let (fbase, rbase) = (nu, nu + 2 * fields.len());
    let size = (rbase + 4 * rings.len()).max(1);
    let sites: HashMap<usize, &Site> = reader.sites.iter().filter(|s| rings[s.ring].1).map(|s| (s.at, s)).collect();
    let marks = ctx_fields.iter().any(|(n, _)| n == "__out_m");
    let pooled = lines.iter().any(|l| l.starts_with("data __act_due: "));

    // the text, a function at a time
    let mut out = String::new();
    let mut n = 0;
    let mut k = 0;
    while k < lines.len() {
        let Some(f) = fns.iter().find(|f| f.at.0 == k) else {
            if lines[k].starts_with("data __ctx_mem: ") {
                out.push_str(&format!("; the cut (fm3 cut.md): where each unit runs, 0 the screen and 1 far, a byte each, {} of them; then for each field of the context the wire may carry, whether it is carried from the screen and whether from far; then for each stored stream, at each place, whether a push there is left out, the stream being read only at the other, and whether it is made again at the other. The runner fills it before a case; all zero, every unit is at the screen and nothing crosses\ndata __cut: array(u8, {})\n", nu, size));
            }
            out.push_str(&lines[k]);
            out.push('\n');
            k += 1;
            continue;
        };
        let who = owner.get(&f.name).copied().filter(|_| !machine.contains(f.name.as_str()));
        let unit = who.filter(|&o| o < nu);
        let wrapped = unit.is_some() || screens(&f.name);
        // a continuation's place in the pool is its caller's, and it is
        // given its own
        let cont = unit.is_some() && pooled && base(&f.name) != f.name && f.params.len() == 1 && f.params[0].0 == "__a";
        out.push_str(&if cont { lines[k].replacen("(__a: index)", "(__a0: index)", 1) } else { lines[k].clone() });
        out.push('\n');
        if let Some(u) = unit {
            out.push_str(&format!("    __pw: i64 = __wire_unit({})\n", u));
        } else if wrapped {
            out.push_str("    __pw: i64 = __wire(0)\n");
        }
        if cont {
            out.push_str("    __a: index = __act_in(__a0, __pw)\n");
        }
        for j in f.at.0 + 1..f.at.1 {
            let l = &lines[j];
            let t = l.trim();
            let p = pad(l);
            if f.name == "__zero_reset" && t == "__zero_new()" {
                // both places made new, and the cut put back: every
                // unit at the screen
                out.push_str(&format!("{p}__zero_context(1)\n{p}__zero_new()\n{p}__zero_context(0)\n{p}__zero_new()\n{p}__cp: ptr = addr __cut\n{p}loop(__ci: index = 0) bound {size}\n{p}    __cd: u1 = cmp.ge __ci, {size}\n{p}    if __cd\n{p}        break\n{p}    store 0: u8, __cp, __ci, 1\n{p}    __ci2: index = add __ci, 1\n{p}    continue __ci2\n", p = p, size = size));
                continue;
            }
            if let Some(s) = sites.get(&j) {
                n += 1;
                let again = {
                    let at = tokens(t).into_iter().find(|(_, w)| *w == s.handle).map(|(at, _)| at).unwrap();
                    format!("{}__rs{}{}", &t[..at], n, &t[at + s.handle.len()..])
                };
                out.push_str(&format!("{p}__ro{n}: u1 = __ring_here({r})\n{p}if __ro{n}\n{p}    {t}\n{p}__rc{n}: i64 = __ring_far({r})\n{p}__rk{n}: u1 = cmp.ne __rc{n}, 2\n{p}if __rk{n}\n{p}    __rp{n}: ptr = context()\n{p}    __rx{n}: __ctx = load __rp{n}\n{p}    __rs{n}: {ty} = get __rx{n}, {field}\n{p}    {again}\n{p}    __wire_back(__rc{n})\n", p = p, n = n, r = s.ring, t = t, ty = s.ty, field = s.field, again = again));
                continue;
            }
            if wrapped && (t == "ret" || t.starts_with("ret ")) {
                if cont {
                    out.push_str(&format!("{}__act_out(__a0, __a)\n", p));
                }
                out.push_str(&format!("{}__wire_back(__pw)\n", p));
            }
            out.push_str(l);
            out.push('\n');
        }
        k = f.at.1;
    }
    // the wire
    out.push_str("\n; the wire (fm3 cut.md): every crossing goes through here, and nothing else changes the current context. Place `to` becomes the current one and the place left is given back. Where the two are one place it is a test and no more. The time is carried: the place arrived at moves its clock up to the clock of the place left, and marks its output there. Then each field the cut says crosses in this direction is copied from the context left to the context arrived at\n");
    out.push_str("fn __wire(to: i64) -> i64\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    from: i64 = conv second\n    same: u1 = cmp.eq from, to\n    if same\n        ret from\n    x: __ctx = load a\n    t: i64 = get x, __clock\n    k: i32 = conv to\n    __zero_context(k)\n    b: ptr = context()\n    y0: __ctx = load b\n    c: i64 = get y0, __clock\n    late: u1 = cmp.gt t, c\n");
    if marks {
        out.push_str("    if late\n        mn: index = get y0, __out_n\n        mm: ptr = get y0, __out_m\n        store t, mm, mn, 8\n");
    }
    out.push_str("    now: i64 = if late\n        yield t\n    else\n        yield c\n    y1: __ctx = set y0, __clock, now\n");
    if !fields.is_empty() {
        out.push_str("    cp: ptr = addr __cut\n    fi: index = conv from\n");
    }
    for (j, (name, ty, _)) in fields.iter().enumerate() {
        out.push_str(&format!("    i{j}: index = add fi, {at}\n    h{j}: u8 = load cp, i{j}, 1\n    g{j}: u1 = cmp.ne h{j}, 0\n    a{j}: {ty} = get x, {name}\n    b{j}: {ty} = get y{y}, {name}\n    v{j}: {ty} = if g{j}\n        yield a{j}\n    else\n        yield b{j}\n    y{z}: __ctx = set y{y}, {name}, v{j}\n", j = j, at = fbase + 2 * j, ty = ty, name = name, y = j + 1, z = j + 2));
    }
    out.push_str(&format!("    store y{}, b\n    ret from\n", fields.len() + 1));
    out.push_str("\n; back over the wire, to the place a crossing left\nfn __wire_back(to: i64)\n    r: i64 = __wire(to)\n    ret\n");
    out.push_str("\n; a unit asks the wire for its own place, which is a byte of the cut\nfn __wire_unit(u: index) -> i64\n    p: ptr = addr __cut\n    c: u8 = load p, u, 1\n    to: i64 = conv c\n    w: i64 = __wire(to)\n    ret w\n");
    out.push_str("\n; the runner's: a byte of the cut\nfn __cut_set(i: index, v: u8)\n    p: ptr = addr __cut\n    store v, p, i, 1\n    ret\n");
    if !sites.is_empty() {
        out.push_str(&format!("\n; a push into a stored stream, at this place: made here unless the stream is read only at the other\nfn __ring_here(r: index) -> u1\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: index = conv second\n    r4: index = mul r, 4\n    i0: index = add r4, {rbase}\n    i: index = add i0, here\n    p: ptr = addr __cut\n    c: u8 = load p, i, 1\n    made: u1 = cmp.eq c, 0\n    ret made\n", rbase = rbase));
        out.push_str(&format!("\n; ... and made again at the other place where the stream is read there: over the wire, giving the place left, or 2 where it is not\nfn __ring_far(r: index) -> i64\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: index = conv second\n    r4: index = mul r, 4\n    i0: index = add r4, {rbase2}\n    i: index = add i0, here\n    p: ptr = addr __cut\n    c: u8 = load p, i, 1\n    stays: u1 = cmp.eq c, 0\n    if stays\n        ret 2\n    hw: i64 = conv second\n    other: i64 = sub 1, hw\n    w: i64 = __wire(other)\n    ret w\n", rbase2 = rbase + 2));
    }
    if pooled {
        let cols: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix("data __act_")).filter_map(|l| l.split(':').next()).collect();
        out.push_str(&format!("\n; a function left at its steps (fm3 log 232), gone on with at the other place: its record goes with its turn. The place in the pool it is handed is its caller's, `w` the place called from; its own is the same place of this context's half, and the record is copied there\nfn __act_in(a0: index, w: i64) -> index\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: i64 = conv second\n    same: u1 = cmp.eq here, w\n    if same\n        ret a0\n    hi: index = conv here\n    wi: index = conv w\n    hb: index = mul hi, {pool}\n    wb: index = mul wi, {pool}\n    n: index = sub a0, wb\n    l: index = add hb, n\n    __act_carry(a0, l)\n    ret l\n\n; ... and copied back where its turn ends\nfn __act_out(a0: index, a: index)\n    same: u1 = cmp.eq a0, a\n    if same\n        ret\n    __act_carry(a, a0)\n    ret\n\n; one record of the pool, every column of it, from one place to another\nfn __act_carry(from: index, to: index)\n", pool = POOL));
        for (j, c) in cols.iter().enumerate() {
            out.push_str(&format!("    p{j}: ptr = addr __act_{c}\n    v{j}: i64 = load p{j}, from, 8\n    store v{j}, p{j}, to, 8\n", j = j, c = c));
        }
        out.push_str("    ret\n");
    }
    Ok((out, Placed { units, facts, crossings: crossings.into_iter().collect(), fields, rings, sigs }))
}

impl Placed {
    fn screen(&self) -> usize {
        self.units.len()
    }

    fn at(&self, e: usize, far: &BTreeSet<usize>) -> usize {
        far.contains(&e) as usize
    }

    fn who(&self, e: usize) -> String {
        if e == self.screen() { "the screen's own".to_string() } else { format!("'{}'", self.units[e].shown()) }
    }

    /// every unit far
    pub fn all(&self) -> BTreeSet<usize> {
        (0..self.units.len()).collect()
    }

    /// The units a person names, `count down,countdown.zero:2` or
    /// `all`: each by where it is written, or a function by its words
    /// where one definition has them
    pub fn named(&self, text: &str) -> Result<BTreeSet<usize>, String> {
        let mut out = BTreeSet::new();
        for name in text.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if name == "all" {
                out.extend(self.all());
                continue;
            }
            let found: Vec<usize> = (0..self.units.len()).filter(|&u| self.units[u].name == name || self.units[u].words.as_deref() == Some(name)).collect();
            match found.as_slice() {
                [u] => {
                    out.insert(*u);
                }
                [] => return Err(format!("'{}' is no unit of the store. A unit is a function or a line that stands, named by where it is written: {}", name, self.units.iter().map(|u| match &u.words { Some(w) => format!("{} ({})", u.name, w), None => u.name.clone() }).collect::<Vec<_>>().join(", "))),
                several => return Err(format!("'{}' is {} units, one a definition: say which by where it is written, {}", name, several.len(), several.iter().map(|&u| self.units[u].name.clone()).collect::<Vec<_>>().join(", "))),
            }
        }
        Ok(out)
    }

    /// a cut as a case's line says it
    pub fn label(&self, far: &BTreeSet<usize>) -> String {
        if far.is_empty() {
            "far: nothing".to_string()
        } else if far.len() == self.units.len() && far.len() > 1 {
            "far: all".to_string()
        } else {
            format!("far: {}", far.iter().map(|&u| match &self.units[u].words { Some(w) if self.units.iter().filter(|o| o.words.as_ref() == Some(w)).count() == 1 => w.clone(), _ => self.units[u].name.clone() }).collect::<Vec<_>>().join(", "))
        }
    }

    /// which fields the wire carries from each place at this cut, and
    /// where each stored stream is read
    fn carries(&self, far: &BTreeSet<usize>) -> (Vec<[bool; 2]>, Vec<[bool; 2]>, Vec<[bool; 2]>) {
        let n = self.facts.len();
        let fields = self.fields.iter().map(|(name, _, _)| {
            let mut c = [false; 2];
            for p in 0..2 {
                let written = (0..n).any(|e| self.at(e, far) == p && self.facts[e].writes.contains(name));
                let lives = (0..n).any(|e| self.at(e, far) != p && (self.facts[e].writes.contains(name) || self.facts[e].reads.contains(name)));
                c[p] = written && lives;
            }
            c
        }).collect();
        let read: Vec<[bool; 2]> = (0..self.rings.len()).map(|r| [0, 1].map(|p| (0..n).any(|e| self.at(e, far) == p && self.facts[e].ring_reads.contains(&r)))).collect();
        let pushed = (0..self.rings.len()).map(|r| [0, 1].map(|p| (0..n).any(|e| self.at(e, far) == p && self.facts[e].ring_pushes.contains(&r)))).collect();
        (fields, read, pushed)
    }

    /// Why a cut cannot be run yet (fm3 question 152): it would carry
    /// an address from one place's memory to the other
    pub fn refused(&self, far: &BTreeSet<usize>) -> Option<String> {
        for c in &self.crossings {
            if self.at(c.from, far) == self.at(c.to, far) || c.to == self.screen() {
                continue;
            }
            if let Some(ty) = self.sigs.get(&c.callee).and_then(|s| s.iter().find(|t| t.contains('$') || t.contains('[') || t.contains("ptr"))) {
                let (far_one, other) = if far.contains(&c.to) { (c.to, c.from) } else { (c.from, c.to) };
                return Some(format!("{} cannot run apart from {} yet: {} is handed, or gives, {}, which is an address in the memory of the place it was made at", self.who(far_one), self.who(other), self.who(c.to), address(ty)));
            }
        }
        let (fields, read, pushed) = self.carries(far);
        for ((name, ty, plain), c) in self.fields.iter().zip(&fields) {
            if (c[0] || c[1]) && !plain {
                return Some(format!("'{}{}' is written at one place and read at the other, and it holds {}, which is an address in the memory of the place it was made at", name.trim_start_matches("__"), if ty.ends_with('$') { "$" } else { "" }, address(ty)));
            }
        }
        // one reader of a stored stream at both places: its place in
        // the stream is one, and each place has a stream of its own
        for (names, _) in self.rings.iter().filter(|(n, _)| !n.is_empty()) {
            for field in names {
                let reads = |p: usize| (0..self.facts.len()).any(|e| self.at(e, far) == p && self.facts[e].through.contains(field));
                let takes = |p: usize| (0..self.facts.len()).any(|e| self.at(e, far) == p && self.facts[e].taken.contains(field));
                if takes(0) && reads(1) || takes(1) && reads(0) {
                    return Some(format!("'{}$' is stored, and one reader of it would be at both places, taking what it reads: what it has taken at one cannot be taken at the other too yet", names[0]));
                }
            }
        }
        for (r, (names, splits)) in self.rings.iter().enumerate().filter(|(_, (n, _))| !n.is_empty()) {
            let touched = |p: usize| read[r][p] || pushed[r][p];
            if !splits && touched(0) && touched(1) {
                return Some(format!("'{}$' is pushed into through a function it is handed to, and is read or pushed at both places: such a push cannot be made again at the other place yet", names[0]));
            }
        }
        None
    }

    /// A cut: the calls that fill the table
    pub fn cut(&self, far: &BTreeSet<usize>) -> Cut {
        self.cut_but(far, None)
    }

    /// ... and the same with one field or one stream deliberately not
    /// carried, which only a test asks for: the proof that a place has
    /// memory of its own is that the program then goes wrong
    pub fn cut_but(&self, far: &BTreeSet<usize>, dropped: Option<&str>) -> Cut {
        let set = |i: usize| ("__cut_set".to_string(), vec![i as i64, 1]);
        let mut before: Vec<(String, Vec<i64>)> = far.iter().map(|&u| set(u)).collect();
        let (fields, read, _) = self.carries(far);
        let (fbase, rbase) = (self.units.len(), self.units.len() + 2 * self.fields.len());
        for (j, c) in fields.iter().enumerate() {
            if dropped == Some(self.fields[j].0.as_str()) {
                continue;
            }
            before.extend((0..2).filter(|&p| c[p]).map(|p| set(fbase + 2 * j + p)));
        }
        for (r, rd) in read.iter().enumerate() {
            if dropped.is_some_and(|d| self.rings[r].0.iter().any(|n| n == d)) {
                continue;
            }
            for p in 0..2 {
                // a push at p is left out where the stream is read
                // only at the other place, and made again there where
                // it is read there
                if rd[1 - p] && !rd[p] {
                    before.push(set(rbase + 4 * r + p));
                }
                if rd[1 - p] {
                    before.push(set(rbase + 4 * r + 2 + p));
                }
            }
        }
        Cut { before, label: self.label(far) }
    }

    /// what a field of the context is, to a person
    fn field_said(&self, name: &str) -> String {
        if let Some(f) = name.strip_prefix("__enabled_").or_else(|| name.strip_prefix("__on_")) {
            format!("the switch of feature {}", f)
        } else if let Some(n) = name.strip_prefix("__due") {
            if n.chars().all(|c| c.is_ascii_digit()) { format!("when clock {} is next due", n) } else { format!("when the list's {} is next due", n.trim_start_matches('_')) }
        } else if let Some(line) = name.strip_prefix("__slot_").and_then(|f| self.units.iter().find(|u| u.irs.iter().any(|i| i == f))) {
            format!("that {} is to run at the close of the slot", line.name)
        } else if name == "__ended" {
            "that a `restart` ended the function that was partway".to_string()
        } else if let Some(x) = name.strip_prefix("__run_") {
            format!("which run of {}$ this is, a `restart` beginning another", x)
        } else if let Some(x) = name.strip_prefix("__phase_") {
            format!("when the beat of {}$ last began again", x)
        } else if let Some(line) = name.strip_prefix("__node").and_then(|r| r.split('_').next()).and_then(|k| self.units.iter().find(|u| u.irs.iter().any(|i| *i == format!("__node{}", k)))) {
            format!("what the line at {} keeps: {}", line.name, name)
        } else if let Some(line) = name.strip_prefix("__z").and_then(|r| r.split('_').next()).and_then(|k| self.units.iter().find(|u| u.irs.iter().any(|i| *i == format!("__z{}_each", k)))) {
            format!("what the line at {} keeps: {}", line.name, name)
        } else if !name.starts_with("__") {
            format!("the latest of {}$", name)
        } else {
            format!("what the compiler keeps as {}", name)
        }
    }

    /// The placement (fm3 distribution.md, "what the compiler does",
    /// 7): every unit and where it runs, and every crossing, what kind
    /// it is and what is carried
    pub fn table(&self, store: &str, far: &BTreeSet<usize>, cases: &[String]) -> String {
        let place = |p: usize| if p == 0 { "screen" } else { "far" };
        let mut out = format!("{}: the cut, {}\n\n", store, self.label(far));
        let w1 = self.units.iter().map(|u| u.name.len()).max().unwrap_or(4).max(4);
        out.push_str(&format!("{:<w1$}  {:<6}  {}\n", "unit", "runs", "what it is", w1 = w1));
        for (u, unit) in self.units.iter().enumerate() {
            out.push_str(&format!("{:<w1$}  {:<6}  {}\n", unit.name, place(self.at(u, far)), match (&unit.words, &unit.said) { (Some(w), _) => format!("the function `{}`", w), (None, Some(s)) => format!("the line `{}`", s), _ => "a function".to_string() }, w1 = w1));
        }
        out.push_str(&format!("{:<w1$}  {:<6}  {}\n", "", "screen", "the devices: the output, the input, the list of things going on; and a case's call", w1 = w1));
        if let Some(why) = self.refused(far) {
            out.push_str(&format!("\nrefused: {}\n", why));
            return out;
        }
        let mut rows: Vec<(String, String, String, String)> = Vec::new();
        let name = |e: usize| if e == self.screen() { "the screen".to_string() } else { self.units[e].name.clone() };
        for c in &self.crossings {
            let (a, b) = (self.at(c.from, far), self.at(c.to, far));
            if a == b {
                continue;
            }
            let types = self.sigs.get(&c.callee).cloned().unwrap_or_default();
            let (kind, what) = if c.to == self.screen() {
                match c.callee.as_str() {
                    "__out_ch" | "__out_block" => ("text".to_string(), "characters for out$, written at the screen at their time".to_string()),
                    "__turns" => ("a call".to_string(), "a step asks the list: everything due before it has its turn first".to_string()),
                    "__run" => ("a call".to_string(), "the lines that walk a stored stream are given their turn".to_string()),
                    f if f.starts_with("__act_start_") => ("a call".to_string(), "a function begun that is left at its steps: its record is the list's".to_string()),
                    _ => ("a call".to_string(), format!("`{}`, the platform's", c.callee)),
                }
            } else if self.units[c.to].words.is_none() {
                match c.caller.as_str() {
                    "__turns" => ("a call".to_string(), "the line's turn, when it is due; the time it is next due comes back".to_string()),
                    "__close" => ("a call".to_string(), "the line's turn, at the close of a slot in which a stream it reads had an item".to_string()),
                    _ if c.callee.starts_with("__node") => ("a call".to_string(), "the line's turn, when a stored stream it reads has more".to_string()),
                    "__act_go" => ("a call".to_string(), "a function gone on with, its record with it".to_string()),
                    _ if types.len() == 1 => ("an item".to_string(), format!("one `{}`, with its time, to the line that reads it", types[0])),
                    _ if types.is_empty() => ("an item".to_string(), "that there is one, with its time: the line reads it by its stream's name".to_string()),
                    _ => ("an item".to_string(), format!("with what the line keeps: {}", types.join(", "))),
                }
            } else if c.caller == "__act_go" {
                ("a call".to_string(), "a function gone on with, its record with it; where it stopped comes back".to_string())
            } else {
                ("a call".to_string(), if types.is_empty() { "nothing over, its end back".to_string() } else { format!("{} over and back, and its end", types.join(", ")) })
            };
            rows.push((kind, format!("{} → {}", name(c.from), name(c.to)), format!("{} → {}", place(a), place(b)), what));
        }
        // a case's own call, made at the screen, of a function placed far
        for (u, unit) in self.units.iter().enumerate().filter(|(u, _)| far.contains(u)) {
            if unit.irs.iter().any(|i| cases.contains(i)) && !self.crossings.iter().any(|c| c.from == self.screen() && c.to == u) {
                let types = unit.irs.iter().find_map(|i| self.sigs.get(i)).cloned().unwrap_or_default();
                rows.push(("a call".to_string(), format!("a case's own → {}", unit.name), "screen → far".to_string(), if types.is_empty() { "nothing over, its end back".to_string() } else { format!("{} over and back, and its end", types.join(", ")) }));
            }
        }
        let (fields, read, pushed) = self.carries(far);
        for ((fname, _, _), c) in self.fields.iter().zip(&fields) {
            for p in 0..2 {
                if c[p] {
                    let kind = if fname.starts_with("__enabled_") || fname.starts_with("__on_") { "a switch" } else if fname.starts_with("__") { "kept" } else { "an item" };
                    rows.push((kind.to_string(), self.field_said(fname), format!("{} → {}", place(p), place(1 - p)), "a field of the context, copied as the wire is crossed".to_string()));
                }
            }
        }
        for (r, (names, _)) in self.rings.iter().enumerate().filter(|(_, (n, _))| !n.is_empty()) {
            for p in 0..2 {
                if pushed[r][p] && read[r][1 - p] {
                    rows.push(("an item".to_string(), format!("of {}$, stored", names[0]), format!("{} → {}", place(p), place(1 - p)), "each push made again at the reader's place, with its time".to_string()));
                }
            }
        }
        rows.dedup();
        if rows.is_empty() {
            out.push_str("\nthe wire carries nothing\n");
            return out;
        }
        let w: Vec<usize> = (0..3).map(|k| rows.iter().map(|r| [&r.0, &r.1, &r.2][k].chars().count()).max().unwrap_or(0)).collect();
        out.push_str("\nthe wire carries\n");
        for (a, b, c, d) in &rows {
            let padded = |s: &str, n: usize| format!("{}{}", s, " ".repeat(n.saturating_sub(s.chars().count())));
            out.push_str(&format!("{}  {}  {}  {}\n", padded(a, w[0]), padded(b, w[1]), padded(c, w[2]), d));
        }
        out
    }
}

/// the cuts a runner tries of a store (fm3 touchstones.md, "hello, in
/// two places", step 1): every unit far, and each unit alone far
pub fn cuts(p: &Placed) -> Vec<BTreeSet<usize>> {
    let mut out: Vec<BTreeSet<usize>> = Vec::new();
    if p.units.len() > 1 {
        out.push(p.all());
    }
    out.extend((0..p.units.len()).map(|u| [u].into_iter().collect::<BTreeSet<usize>>()));
    out
}
