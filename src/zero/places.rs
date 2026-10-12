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

use super::turns::{called, parse, split_top, tokens, word, Func, POOL, WIRE_Q};

/// how many times a cut may be moved in one case (fm3 log 265): the
/// later cuts are tables beside the first, filled before the case
pub const MOVES: usize = 4;
const NEVER: i64 = 1 << 62;
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
    /// those of a function's parameters, by name, and of its results,
    /// by place, that are values holding an address: an array or a
    /// string (fm3 log 267). The wire copies them
    pub values: (Vec<String>, Vec<usize>),
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

/// ... and, where the wire takes time and each place has a list of
/// its own (fm3 log 269), the words of that list: a place's own, never
/// the other's
const OWN_LIST: [&str; 11] = ["__act_n", "__due_act", "__act_i", "__act_b", "__in_n", "__in_k", "__in_t", "__in_c", "__due_in", "__in_busy", "__due_close"];

/// how many of the wire's items its record holds, for a person
const WIRE_LOG: usize = 1024;

/// Working ahead of a key (fm3 cut.md, "working ahead of a key"; log
/// 273): how many copies of itself far keeps, one before each turn it
/// has taken for a time the screen has not told it of; how many places
/// of its half of the pool and how many bytes of its streams a copy
/// holds; how many pieces of text the screen keeps until their time;
/// and how many of its own sendings the screen remembers the times of.
/// Past the first three far does not work ahead, and is late, not wrong
pub const AHEAD: usize = 16;
const AHEAD_ROWS: usize = 16;
const AHEAD_MEM: usize = 8192;
const KEPT: usize = 512;
const SENDS: usize = 256;

/// the functions of a build whose wire takes time that run at
/// whichever place the world is giving a turn: what they read of the
/// context is read at both
fn both(name: &str) -> bool {
    matches!(name, "__list_next" | "__list_turn" | "__act_run" | "__act_next" | "__act_turn" | "__act_slot" | "__act_go" | "__run") || name.starts_with("__act_start_")
}

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
    /// for each function of a unit, which places of its signature,
    /// parameters and then results, are values the wire copies
    copied: HashMap<String, Vec<usize>>,
    /// the cut may be moved while the program runs (fm3 cut.md, "the
    /// cut, moved"; log 265)
    pub moved: bool,
    /// the fields a move may hand across, in the order of their bytes:
    /// those the wire may carry, and after them those only one unit
    /// touches, which no fixed cut carries and a unit that moves needs
    hand_fields: Vec<(String, String, bool)>,
    /// each stored stream's first byte in a move's table: whether its
    /// storage is handed across, and after it a byte for each field
    /// that holds it, a reader's place. None where the rewrite does not
    /// know how the stream is kept
    ring_at: Vec<Option<usize>>,
    /// the units one of whose functions may be on the machine's stack
    /// when the list takes a turn, with that function
    held: Vec<(usize, String)>,
    /// the pool's columns, and the units whose functions are left at
    /// their steps and so keep their place there
    pool: Vec<String>,
    left: BTreeSet<usize>,
    /// the size of the cut's table and of a move's
    sizes: (usize, usize),
    /// the wire takes time (fm3 cut.md, "a wire that takes time"; log
    /// 269): what the functions that run at both places do, the clocks'
    /// own words, and the stored streams a push into which cannot be
    /// an item of the wire
    pub wire: bool,
    glue: Facts,
    unwired: BTreeSet<usize>,
    /// each continuation's number and the unit it is of
    conts: Vec<(i64, Option<usize>)>,
    /// the units a function of which is left at a step of a rate, and
    /// so may be due at its place
    stepping: BTreeSet<usize>,
}

/// one thing a move hands across: its byte in the move's table, the
/// direction, 1 from the screen to far and 2 from far to the screen,
/// what it is to a person, and the field or stream it is
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hand {
    at: usize,
    dir: u8,
    pub what: String,
    name: String,
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
    /// the wire takes time: a call of a function the IR makes for a
    /// type, `width_of__i64`, is a call of the function written
    wire: bool,
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
            if self.wire {
                out.calls.extend(called(t).into_iter().filter(|c| !self.by.contains_key(c)).filter_map(|c| c.rsplit_once("__").map(|(x, _)| x)).filter(|x| self.by.contains_key(x)).map(str::to_string));
            }
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
pub fn rewrite(ir: &str, units: &[Unit], wire: bool) -> Result<(String, Placed), String> {
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
            // (where the wire takes time, fm3 log 269, a call put out
            // of a unit's function as `f__o3` is that unit's, and so
            // are its continuations)
            _ if wire => {
                let mut b = match name.rsplit_once("__") {
                    Some((b, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && present.contains(format!("{}__0", b).as_str()) => b,
                    _ => name,
                };
                while let Some((x, n)) = b.rsplit_once("__o") {
                    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
                        break;
                    }
                    b = x;
                }
                if units.iter().any(|u| u.irs.iter().any(|i| i == b)) { b.to_string() } else { name.to_string() }
            }
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
    // the values the wire copies (fm3 log 267): a unit's parameters
    // and results that are an array or a string, by their place in
    // each of its functions' signatures. A continuation has none: a
    // function that keeps one across a step keeps the stack
    let mut copied: HashMap<String, Vec<usize>> = HashMap::new();
    for f in &fns {
        let Some(&u) = owner.get(&f.name).filter(|&&u| !wire && u < nu && base(&f.name) == f.name) else { continue };
        let (params, results) = &units[u].values;
        let mut at: Vec<usize> = f.params.iter().enumerate().filter(|(_, (n, t))| t.ends_with('$') && params.contains(n)).map(|(i, _)| i).collect();
        at.extend(f.rets.iter().enumerate().filter(|(k, t)| t.ends_with('$') && results.contains(k)).map(|(k, _)| f.params.len() + k));
        if !at.is_empty() {
            copied.insert(f.name.clone(), at);
        }
    }
    let mut carried_types: BTreeSet<String> = BTreeSet::new();
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
    // how each is kept, a queue or a ring, read off what made it: a
    // move hands a stream's storage across by its kind
    let mut kinds: Vec<Option<&'static str>> = Vec::new();
    {
        let new = fns.iter().find(|f| f.name == "__zero_new").ok_or("the store has no reset")?;
        let packed = new.body.iter().filter_map(|l| def(l.trim())).find(|(_, ty, rhs)| *ty == "__ctx" && rhs.starts_with("pack ")).map(|(_, _, rhs)| split_top(&rhs["pack ".len()..])).unwrap_or_default();
        let maker = |value: &str| -> Option<&'static str> {
            let mut v = value.to_string();
            for _ in 0..8 {
                let rhs = new.body.iter().filter_map(|l| def(l.trim())).find(|(n, _, _)| *n == v).map(|(_, _, rhs)| rhs)?;
                if rhs.starts_with("__queue_") || rhs.starts_with("__copy_queue_") {
                    return Some("queue");
                }
                if rhs.starts_with("__regular_") || rhs.starts_with("__stream_") {
                    return Some("ring");
                }
                // a reader's rule set on it: the stream beneath
                v = split_top(rhs.strip_prefix("set ")?).first()?.clone();
            }
            None
        };
        let mut made: HashMap<String, usize> = HashMap::new();
        for (k, (name, ty)) in ctx_fields.iter().enumerate() {
            if !ty.ends_with('$') {
                continue;
            }
            let value = packed.get(k).cloned().unwrap_or_else(|| name.clone());
            let r = *made.entry(value.clone()).or_insert_with(|| {
                rings.push(Vec::new());
                kinds.push(maker(&value));
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
    let mut reader = Reader { fns: &fns, by, handles, pushed: HashMap::new(), unsplit: BTreeSet::new(), sites: Vec::new(), wire };
    let mut own: HashMap<String, Facts> = HashMap::new();
    for f in reader.fns {
        let facts = reader.facts(f);
        own.insert(f.name.clone(), facts);
    }
    // where the wire takes time: what the functions that run at both
    // places do, and the clocks' own words, which the list names
    let mut glue = Facts::default();
    let mut dues: BTreeSet<String> = BTreeSet::new();
    if wire {
        for f in fns.iter().filter(|f| both(&f.name)) {
            glue.add(&own[&f.name]);
        }
        if let Some(f) = fns.iter().find(|f| f.name == "__list_next") {
            dues.extend(f.body.iter().filter_map(|l| def(l.trim())).filter_map(|(_, _, rhs)| rhs.strip_prefix("get x, __due")).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|n| format!("__due{}", n)));
        }
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
                // (a name several units define, each for a type: over a
                // wire that takes time the call is bound to them all)
                if wire {
                    for o in (0..nu).filter(|&o| o != e && units[o].irs.contains(c)) {
                        crossings.insert(Crossing { from: e, caller: g.clone(), to: o, callee: c.clone() });
                    }
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
        if OWN.contains(&name.as_str()) || reader.handles.contains_key(name) || wire && (OWN_LIST.contains(&name.as_str()) || dues.contains(name)) {
            continue;
        }
        let writers = facts.iter().filter(|f| f.writes.contains(name)).count();
        let users = facts.iter().filter(|f| f.writes.contains(name) || f.reads.contains(name)).count();
        if writers > 0 && (users > 1 || wire && (glue.reads.contains(name) || glue.writes.contains(name))) {
            fields.push((name.clone(), ty.clone(), carried(ty, &structs)));
        }
    }
    let rings: Vec<(Vec<String>, bool)> = rings.into_iter().enumerate().map(|(r, names)| (names, !reader.unsplit.contains(&r))).collect();
    if wire && fields.len() > 60 {
        return Err("more than sixty fields of the context would cross: an item of the wire says which with a bit each".into());
    }
    let sigs: HashMap<String, Vec<String>> = fns.iter().map(|f| (f.name.clone(), f.params.iter().map(|(_, t)| t.clone()).chain(f.rets.iter().cloned()).collect())).collect();
    let (fbase, rbase) = (nu, nu + 2 * fields.len());
    let size = (rbase + 4 * rings.len()).max(1);
    // a build whose cut may be moved (fm3 cut.md, log 265) has the
    // move's turn in its list, written by the lowering with no body
    let moved = fns.iter().any(|f| f.name == "__move_turn");
    // what a move may hand across: the wire's fields, and each field
    // only one unit touches. A fixed cut never carries the second
    // kind; a unit that moves leaves it behind
    let mut hand_fields = fields.clone();
    if moved {
        for (name, ty) in &ctx_fields {
            if OWN.contains(&name.as_str()) || reader.handles.contains_key(name) || fields.iter().any(|(n, _, _)| n == name) {
                continue;
            }
            let wrote = facts.iter().any(|f| f.writes.contains(name));
            let by_a_unit = facts[..nu].iter().any(|f| f.writes.contains(name) || f.reads.contains(name));
            if wrote && by_a_unit {
                hand_fields.push((name.clone(), ty.clone(), carried(ty, &structs)));
            }
        }
    }
    let mut ring_at: Vec<Option<usize>> = Vec::new();
    let mut hsize = hand_fields.len();
    for (r, (names, _)) in rings.iter().enumerate() {
        if names.is_empty() || kinds[r].is_none() {
            ring_at.push(None);
        } else {
            ring_at.push(Some(hsize));
            hsize += 1 + names.len();
        }
    }
    let hsize = hsize.max(1);
    // the functions from which the list can be reached: one of them
    // is on the machine's stack when a turn is taken, and its unit
    // cannot be moved in that turn
    let mut reach: BTreeSet<String> = BTreeSet::new();
    if moved {
        loop {
            let more: Vec<String> = fns.iter().filter(|f| !reach.contains(&f.name) && own[&f.name].calls.iter().any(|c| c == "__turns" || reach.contains(c))).map(|f| f.name.clone()).collect();
            if more.is_empty() {
                break;
            }
            reach.extend(more);
        }
    }
    let held: Vec<(usize, String)> = fns.iter().filter(|f| reach.contains(&f.name)).filter_map(|f| owner.get(&f.name).copied().filter(|&o| o < nu).map(|o| (o, f.name.clone()))).collect();
    let sites: HashMap<usize, &Site> = reader.sites.iter().filter(|s| rings[s.ring].1).map(|s| (s.at, s)).collect();
    let marks = ctx_fields.iter().any(|(n, _)| n == "__out_m");
    let pooled = lines.iter().any(|l| l.starts_with("data __act_due: "));
    // (a store with an input that arrives at a time has a bound far
    // may work ahead of, fm3 log 273: only its text has what that needs)
    let ahead = wire && fns.iter().any(|f| f.name == "__in_turn");
    let ncols = lines.iter().filter(|l| l.starts_with("data __act_")).count();

    // (where the wire takes time: the stored streams a push into
    // which is no word, and what an arriving push does)
    let mut unwired: BTreeSet<usize> = BTreeSet::new();
    let mut arrivals = String::new();
    // the text, a function at a time
    let mut out = String::new();
    let mut n = 0;
    let mut k = 0;
    while k < lines.len() {
        let Some(f) = fns.iter().find(|f| f.at.0 == k) else {
            if lines[k].starts_with("data __ctx_mem: ") {
                out.push_str(&format!("; the cut (fm3 cut.md): where each unit runs, 0 the screen and 1 far, a byte each, {} of them; then for each field of the context the wire may carry, whether it is carried from the screen and whether from far; then for each stored stream, at each place, whether a push there is left out, the stream being read only at the other, and whether it is made again at the other. The runner fills it before a case; all zero, every unit is at the screen and nothing crosses\ndata __cut: array(u8, {})\n", nu, size));
                if wire {
                    out.push_str(&format!("; the world and the wire (fm3 cut.md, \"a wire that takes time\"; log 269). `__w` is the world's: its clock, the wire's delay, what the runner asked to read back, and for each place what it has sent, had answered and been sent, what it was last told and what it last told; then where each way of the wire begins and ends. An item of the wire is a row: when it arrives by the world's clock, what it is (1 a character and 2 a block of text for the output, 3 a push into a stored stream, 4 a function partway, 5 a telling), two words of its own, which fields of the context it carries and how much of the other's sending it answers, and its own time; {q} rows each way. A function partway in flight has a row of the pool. `__wq_ctx` is the sender's context as it was, from which the fields an item carries are read; `__wshadow` what each place last sent or was sent of each; `__wq_text` the blocks of text in flight; `__wlog` the wire's record for a person, six words an item. When each character of the capture was shown is kept in far's half of the marks, `__out_t`, from its second word: far has no output of its own, and its first word is where its clock marks\ndata __w: array(i64, 32)\ndata __wq_at: array(i64, {q2})\ndata __wq_kind: array(i64, {q2})\ndata __wq_a: array(i64, {q2})\ndata __wq_b: array(i64, {q2})\ndata __wq_c: array(i64, {q2})\ndata __wq_e: array(i64, {q2})\ndata __wq_t: array(i64, {q2})\ndata __wq_ctx: array(__ctx, {q2})\ndata __wshadow: array(__ctx, 2)\ndata __wq_text: array(u8, 65536)\ndata __wlog: array(i64, {log})\n", q = WIRE_Q, q2 = 2 * WIRE_Q, log = 6 * WIRE_LOG));
                }
                if ahead {
                    out.push_str(&format!("; working ahead of a key (fm3 cut.md, \"working ahead of a key\"; log 273). Far takes turns for times the screen has not told it of, and before each keeps a copy of itself: `__wa_t` and `__wa_r` the time and rank of that turn, `__wa_ctx` its context, `__wa_pool` its half of the pool, {rows} places a column, `__wa_mem` the part of the arena its streams were made in; {n} copies, the oldest first. `__wa` is the words of it: whether far is in a turn taken ahead, whether that turn would have sent more than text, whether it left a function partway here that would have crossed, the turn that may not be tried again until something reaches far, where the copies begin and how many there are, how many turns far took and threw away, the rank of the turn each place is in, where the screen's kept pieces begin and how many there are, and where far's part of the arena is and how long. An item of the wire has a column more, `__wq_r`, the rank of the turn that sent it. The screen keeps text until its time: `__wk_*`, a row a piece, its time and rank, how much of the screen's sending far had had when it wrote it, and what it is; and `__ws_t` and `__ws_r` are the time and rank of each thing the screen has sent\ndata __wa: array(i64, 32)\ndata __wa_t: array(i64, {n})\ndata __wa_r: array(i64, {n})\ndata __wa_ctx: array(__ctx, {n})\ndata __wa_keep: array(__ctx, 1)\ndata __wa_pool: array(i64, {pool})\ndata __wa_mem: array(u8, {mem})\ndata __wq_r: array(i64, {q2})\ndata __wk_t: array(i64, {k})\ndata __wk_r: array(i64, {k})\ndata __wk_e: array(i64, {k})\ndata __wk_kind: array(i64, {k})\ndata __wk_a: array(i64, {k})\ndata __wk_b: array(i64, {k})\ndata __ws_t: array(i64, {s})\ndata __ws_r: array(i64, {s})\n", n = AHEAD, rows = AHEAD_ROWS, pool = (AHEAD * AHEAD_ROWS * ncols).max(1), mem = AHEAD * AHEAD_MEM, q2 = 2 * WIRE_Q, k = KEPT, s = SENDS));
                }
                if moved {
                    out.push_str(&format!("; the cut's moves (fm3 cut.md, log 265), {m} at most: the time each is due; the table of the cut after each, as `__cut` is laid out; and what each hands across, a byte for each field of the context that some move may ({nf} of them: the wire's, and those only one unit touches) and then for each stored stream one for its storage and one for the place of each field that reads it: 0 nothing, 1 from the screen to far, 2 from far to the screen. The runner fills them before a case\ndata __move_t: array(i64, {mt})\ndata __cuts: array(u8, {cs})\ndata __hands: array(u8, {hs})\n", m = MOVES, nf = hand_fields.len(), mt = MOVES + 1, cs = MOVES * size, hs = MOVES * hsize));
                }
            }
            out.push_str(&lines[k]);
            out.push('\n');
            k += 1;
            continue;
        };
        if moved && f.name == "__move_turn" {
            out.push_str(&format!("fn __move_turn()\n    c: ptr = context()\n    x: __ctx = load c\n    k: index = get x, __move_k\n    k2: index = add k, 1\n    tp: ptr = addr __move_t\n    d: i64 = load tp, k2, 8\n    x2: __ctx = set x, __move_k, k2\n    x3: __ctx = set x2, __due_move, d\n    store x3, c\n    cp: ptr = addr __cut\n    np: ptr = addr __cuts\n    base: index = mul k, {size}\n    loop(i: index = 0) bound {size}\n        done: u1 = cmp.ge i, {size}\n        if done\n            break\n        j: index = add base, i\n        b: u8 = load np, j, 1\n        store b, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n    __hand(k)\n    ret\n", size = size));
            k = f.at.1;
            continue;
        }
        let who = owner.get(&f.name).copied().filter(|_| !machine.contains(f.name.as_str()));
        let unit = who.filter(|&o| o < nu);
        let wrapped = !wire && (unit.is_some() || screens(&f.name));
        // a continuation's place in the pool is its caller's, and it is
        // given its own
        let cont = unit.is_some() && pooled && base(&f.name) != f.name && f.params.len() == 1 && f.params[0].0 == "__a";
        // (a continuation where the wire takes time runs at its own
        // place, on its own half of the pool, and asks nothing)
        let (cont, staying) = (cont && !wire, cont && wire);
        // a parameter that is an array or a string is the caller's
        // until the wire is crossed, and a copy after (fm3 log 267)
        let values: Vec<usize> = copied.get(&f.name).cloned().unwrap_or_default();
        let mut head = if cont { lines[k].replacen("(__a: index)", "(__a0: index)", 1) } else { lines[k].clone() };
        for &i in values.iter().filter(|&&i| i < f.params.len()) {
            let (name, ty) = &f.params[i];
            head = head.replacen(&format!("{}: {}", name, ty), &format!("{}__0: {}", name, ty), 1);
        }
        if wire {
            // the output's two writes are the screen's capture, which
            // the wire's own two stand before
            head = head.replacen("fn __out_ch(", "fn __cap_ch(", 1).replacen("fn __out_block(", "fn __cap_block(", 1);
        }
        out.push_str(&head);
        out.push('\n');
        if wire {
            if let (Some(u), false) = (unit, staying) {
                // a function still on the machine's stack runs at its
                // own place or not at all: a line that walks a stored
                // stream is passed over where it is not, and a cut
                // that would call any other from the other place is
                // refused before it is run
                out.push_str(&format!("    __wh: u1 = __unit_here({})\n", u));
                if f.name.starts_with("__node") && f.rets.len() <= 1 {
                    out.push_str(&format!("    if __wh\n    else\n        ret{}\n", if f.rets.is_empty() { "" } else { " 0" }));
                } else {
                    out.push_str("    check __wh\n");
                }
            }
            match f.name.as_str() {
                "__out_mark" => out.push_str(&format!("    __wm: ptr = addr __w\n    __md: i64 = load __wm, 16\n    __m1: u1 = cmp.eq __md, 1\n    if __m1\n        __sp: ptr = addr __out_t\n        __si: i64 = add {i}, 65538\n        __sv: i64 = load __sp, __si, 8\n        ret __sv\n    __m2: u1 = cmp.eq __md, 2\n    if __m2\n        __lp: ptr = addr __wlog\n        __lv: i64 = load __lp, {i}, 8\n        ret __lv\n", i = f.params.first().map(|(n, _)| n.as_str()).unwrap_or("i"))),
                "__out_len" => out.push_str("    __wm: ptr = addr __w\n    __md: i64 = load __wm, 16\n    __m2: u1 = cmp.eq __md, 2\n    if __m2\n        __ln: i64 = load __wm, 32\n        __l6: i64 = mul __ln, 6\n        ret __l6\n"),
                "__out_byte" => out.push_str("    __wm: ptr = addr __w\n    __md: i64 = load __wm, 16\n    __m2: u1 = cmp.eq __md, 2\n    if __m2\n        ret 120\n"),
                _ => {}
            }
        } else if let Some(u) = unit {
            out.push_str(&format!("    __pw: i64 = __wire_unit({})\n", u));
        } else if wrapped {
            out.push_str("    __pw: i64 = __wire(0)\n");
        }
        for &i in values.iter().filter(|&&i| i < f.params.len()) {
            let (name, ty) = &f.params[i];
            let item = ty.trim_end_matches('$');
            carried_types.insert(item.to_string());
            out.push_str(&format!("    {name}: {ty} = __carried_{item}({name}__0, __pw)\n", name = name, ty = ty, item = item));
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
                // (where far may work ahead, fm3 log 273: the part of the arena far's context is made in is what a copy of its streams is)
                let (mark0, mark1) = if ahead { (format!("{p}__wab: ptr = addr __arena\n{p}__wu0: index = load __wab, 16\n", p = p), format!("{p}__wu1: index = load __wab, 16\n", p = p)) } else { (String::new(), String::new()) };
                out.push_str(&format!("{p}__zero_context(1)\n{mark0}{p}__zero_new()\n{mark1}{p}__zero_context(0)\n{p}__zero_new()\n{p}__cp: ptr = addr __cut\n{p}loop(__ci: index = 0) bound {size}\n{p}    __cd: u1 = cmp.ge __ci, {size}\n{p}    if __cd\n{p}        break\n{p}    store 0: u8, __cp, __ci, 1\n{p}    __ci2: index = add __ci, 1\n{p}    continue __ci2\n", p = p, size = size, mark0 = mark0, mark1 = mark1));
                if wire {
                    // ... the world's clock at nothing, the wire empty, nothing said, and every place of the pool free
                    out.push_str(&format!("{p}__wz: ptr = addr __w\n{p}loop(__wi: index = 0) bound 32\n{p}    __wd: u1 = cmp.ge __wi, 32\n{p}    if __wd\n{p}        break\n{p}    store 0: i64, __wz, __wi, 8\n{p}    __wj: index = add __wi, 1\n{p}    continue __wj\n{p}store -1: i64, __wz, 104\n{p}store -1: i64, __wz, 168\n{p}store -1: i64, __wz, 224\n{p}store -1: i64, __wz, 240\n", p = p));
                    if ahead {
                        // ... no copy kept, nothing barred, nothing kept at the screen
                        out.push_str(&format!("{p}__waz: ptr = addr __wa\n{p}loop(__wai: index = 0) bound 32\n{p}    __wad: u1 = cmp.ge __wai, 32\n{p}    if __wad\n{p}        break\n{p}    store 0: i64, __waz, __wai, 8\n{p}    __waj: index = add __wai, 1\n{p}    continue __waj\n{p}store -1: i64, __waz, 24\n{p}__wbase: ptr = load __wab\n{p}__wreg: ptr = ptradd __wbase, __wu0\n{p}__wlen: index = sub __wu1, __wu0\n{p}store __wreg, __waz, 112\n{p}store __wlen, __waz, 120\n", p = p));
                    }
                    if pooled {
                        out.push_str(&format!("{p}__zd: ptr = addr __act_due\n{p}__zc: ptr = addr __act_pc\n{p}loop(__zk: index = 0) bound {n}\n{p}    __ze: u1 = cmp.ge __zk, {n}\n{p}    if __ze\n{p}        break\n{p}    store {never}: i64, __zd, __zk, 8\n{p}    store 0: i64, __zc, __zk, 8\n{p}    __zl: index = add __zk, 1\n{p}    continue __zl\n", p = p, n = 2 * POOL, never = NEVER));
                    }
                }
                if moved {
                    // ... and no move due, the later tables emptied
                    out.push_str(&format!("{p}__mp: ptr = addr __move_t\n", p = p));
                    for m in 0..=MOVES {
                        out.push_str(&format!("{p}store {never}: i64, __mp, {at}\n", p = p, never = NEVER, at = 8 * m));
                    }
                    for (name, n) in [("__cuts", MOVES * size), ("__hands", MOVES * hsize)] {
                        out.push_str(&format!("{p}__zp{name}: ptr = addr {name}\n{p}loop(__zi{name}: index = 0) bound {n}\n{p}    __zd{name}: u1 = cmp.ge __zi{name}, {n}\n{p}    if __zd{name}\n{p}        break\n{p}    store 0: u8, __zp{name}, __zi{name}, 1\n{p}    __zj{name}: index = add __zi{name}, 1\n{p}    continue __zj{name}\n", p = p, name = name, n = n));
                    }
                }
                continue;
            }
            if let (true, Some(s)) = (wire, sites.get(&j)) {
                // a push into a stored stream read at the other place
                // is an item of the wire (fm3 log 269): made here
                // unless the stream is read only there, and put on the
                // wire where it is read there
                n += 1;
                let item = s.ty.trim_end_matches('$').to_string();
                let args: Vec<String> = match (t.strip_prefix("push "), t.find('(')) {
                    (Some(rest), _) => rest.split(',').map(|a| a.trim().to_string()).collect(),
                    (None, Some(open)) => split_top(&t[open + 1..t.len() - 1]),
                    _ => Vec::new(),
                };
                // (one item of one word, or the stream's end: a block of items is not an item of the wire yet)
                let verb = t.split(|c: char| c == '(' || c == ' ' || c == '<').next().unwrap_or("");
                let one = args.get(1).is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit() || c == '-') || f.params.iter().any(|(n, ty)| n == v && *ty == item) || f.body.iter().flat_map(|l| defs(l.trim())).any(|(n, ty)| n == *v && ty == item));
                let held = matches!(verb, "end" | "open") && args.len() == 1 || matches!(verb, "push_queue" | "push_queue_open" | "push" | "__push") && args.len() == 2 && word(&item) && item != "ptr" && one;
                if !held {
                    unwired.insert(s.ring);
                    out.push_str(&format!("{p}__ro{n}: u1 = __ring_here({r})\n{p}if __ro{n}\n{p}    {t}\n", p = p, n = n, r = s.ring, t = t));
                    continue;
                }
                let mut carry = String::from("0");
                let mut lines2 = String::new();
                if let Some(v) = args.get(1) {
                    let v = if v.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
                        lines2.push_str(&format!("{p}    __rt{n}: {item} = const {lit}\n", p = p, n = n, item = item, lit = v.split(':').next().unwrap_or(v).trim()));
                        format!("__rt{}", n)
                    } else {
                        v.clone()
                    };
                    lines2.push_str(&format!("{p}    __rv{n}: i64 = conv {v}\n", p = p, n = n, v = v));
                    carry = format!("__rv{}", n);
                }
                out.push_str(&format!("{p}__ro{n}: u1 = __ring_here({r})\n{p}if __ro{n}\n{p}    {t}\n{p}__rk{n}: u1 = __ring_again({r})\n{p}if __rk{n}\n{lines2}{p}    __wsend_push({n}, {carry})\n", p = p, n = n, r = s.ring, t = t, lines2 = lines2, carry = carry));
                // ... and what the place it arrives at does with it
                let again = match (t.strip_prefix("push "), t.find('(')) {
                    (Some(_), _) => format!("push h{}, v{}", n, n),
                    (None, Some(open)) => format!("{}h{}{})", &t[..open + 1], n, if args.len() == 2 { format!(", v{}", n) } else { String::new() }),
                    _ => String::new(),
                };
                arrivals.push_str(&format!("    is{n}: u1 = cmp.eq site, {n}\n    if is{n}\n{val}        c{n}: ptr = context()\n        x{n}: __ctx = load c{n}\n        h{n}: {ty} = get x{n}, {field}\n        {again}\n", n = n, val = if args.len() == 2 { format!("        v{n}: {item} = conv v\n", n = n, item = item) } else { String::new() }, ty = s.ty, field = s.field, again = again));
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
            // a result that is an array or a string is copied where
            // the wire is crossed back, at the caller's place
            if wrapped && t.starts_with("ret ") && values.iter().any(|&i| i >= f.params.len()) {
                n += 1;
                let mut vals = split_top(&t["ret ".len()..]);
                out.push_str(&format!("{}__pb{}: i64 = __wire(__pw)\n", p, n));
                for &i in values.iter().filter(|&&i| i >= f.params.len()) {
                    let k = i - f.params.len();
                    let ty = &f.rets[k];
                    let item = ty.trim_end_matches('$');
                    carried_types.insert(item.to_string());
                    out.push_str(&format!("{p}__rv{n}_{k}: {ty} = __carried_{item}({v}, __pb{n})\n", p = p, n = n, k = k, ty = ty, item = item, v = vals[k]));
                    vals[k] = format!("__rv{}_{}", n, k);
                }
                out.push_str(&format!("{}ret {}\n", p, vals.join(", ")));
                continue;
            }
            if wire && t == "ret" && matches!(f.name.as_str(), "__out_ch" | "__out_block") {
                // ... and when it is shown, by the world's clock
                out.push_str(&format!("{}__shown(n, {})\n", p, if f.name == "__out_ch" { "1" } else { "k" }));
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
    if !wire {
        out.push_str("\n; the wire (fm3 cut.md): every crossing goes through here, and nothing else changes the current context. Place `to` becomes the current one and the place left is given back. Where the two are one place it is a test and no more. The time is carried: the place arrived at moves its clock up to the clock of the place left, and marks its output there. Then each field the cut says crosses in this direction is copied from the context left to the context arrived at\n");
        out.push_str("fn __wire(to: i64) -> i64\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    from: i64 = conv second\n    same: u1 = cmp.eq from, to\n    if same\n        ret from\n    x: __ctx = load a\n    t: i64 = get x, __clock\n    k: i32 = conv to\n    __zero_context(k)\n    b: ptr = context()\n    y0: __ctx = load b\n    c: i64 = get y0, __clock\n    late: u1 = cmp.gt t, c\n");
        if marks {
            out.push_str("    if late\n        mn: index = get y0, __out_n\n        mm: ptr = get y0, __out_m\n        store t, mm, mn, 8\n");
        }
        out.push_str("    now: i64 = if late\n        yield t\n    else\n        yield c\n    y1: __ctx = set y0, __clock, now\n");
        if !fields.is_empty() {
            out.push_str("    cp: ptr = addr __cut\n    fi: index = conv from\n");
        }
        for (j, (name, ty, plain)) in fields.iter().enumerate() {
            // (a stream kept as its latest value where that is a string
            // or an array, fm3 log 267: the value is copied, the address
            // it holds being the other place's)
            let take = if !plain && ty.ends_with('$') {
                let item = ty.trim_end_matches('$');
                carried_types.insert(item.to_string());
                format!("        c{j}: {ty} = __copied_{item}(a{j})\n        yield c{j}\n", j = j, ty = ty, item = item)
            } else {
                format!("        yield a{j}\n", j = j)
            };
            out.push_str(&format!("    i{j}: index = add fi, {at}\n    h{j}: u8 = load cp, i{j}, 1\n    g{j}: u1 = cmp.ne h{j}, 0\n    a{j}: {ty} = get x, {name}\n    b{j}: {ty} = get y{y}, {name}\n    v{j}: {ty} = if g{j}\n{take}    else\n        yield b{j}\n    y{z}: __ctx = set y{y}, {name}, v{j}\n", j = j, at = fbase + 2 * j, ty = ty, name = name, y = j + 1, z = j + 2, take = take));
        }
        out.push_str(&format!("    store y{}, b\n    ret from\n", fields.len() + 1));
        out.push_str("\n; back over the wire, to the place a crossing left\nfn __wire_back(to: i64)\n    r: i64 = __wire(to)\n    ret\n");
        out.push_str("\n; a unit asks the wire for its own place, which is a byte of the cut\nfn __wire_unit(u: index) -> i64\n    p: ptr = addr __cut\n    c: u8 = load p, u, 1\n    to: i64 = conv c\n    w: i64 = __wire(to)\n    ret w\n");
    }
    out.push_str("\n; the runner's: a byte of the cut\nfn __cut_set(i: index, v: u8)\n    p: ptr = addr __cut\n    store v, p, i, 1\n    ret\n");
    if !sites.is_empty() {
        out.push_str(&format!("\n; a push into a stored stream, at this place: made here unless the stream is read only at the other\nfn __ring_here(r: index) -> u1\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: index = conv second\n    r4: index = mul r, 4\n    i0: index = add r4, {rbase}\n    i: index = add i0, here\n    p: ptr = addr __cut\n    c: u8 = load p, i, 1\n    made: u1 = cmp.eq c, 0\n    ret made\n", rbase = rbase));
        if wire {
            out.push_str(&format!("\n; ... and is it made again at the other place, the stream being read there: then it is an item of the wire (fm3 log 269)\nfn __ring_again(r: index) -> u1\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: index = conv second\n    r4: index = mul r, 4\n    i0: index = add r4, {rbase2}\n    i: index = add i0, here\n    p: ptr = addr __cut\n    c: u8 = load p, i, 1\n    goes: u1 = cmp.ne c, 0\n    ret goes\n", rbase2 = rbase + 2));
        } else {
            out.push_str(&format!("\n; ... and made again at the other place where the stream is read there: over the wire, giving the place left, or 2 where it is not\nfn __ring_far(r: index) -> i64\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: index = conv second\n    r4: index = mul r, 4\n    i0: index = add r4, {rbase2}\n    i: index = add i0, here\n    p: ptr = addr __cut\n    c: u8 = load p, i, 1\n    stays: u1 = cmp.eq c, 0\n    if stays\n        ret 2\n    hw: i64 = conv second\n    other: i64 = sub 1, hw\n    w: i64 = __wire(other)\n    ret w\n", rbase2 = rbase + 2));
        }
    }
    if pooled {
        let cols: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix("data __act_")).filter_map(|l| l.split(':').next()).collect();
        if wire {
            out.push_str("\n; one record of the pool, every column of it, from one place of it to another: a place of a half, or the row of an item in flight\nfn __act_carry(from: index, to: index)\n");
        } else {
        out.push_str(&format!("\n; a function left at its steps (fm3 log 232), gone on with at the other place: its record goes with its turn. The place in the pool it is handed is its caller's, `w` the place called from; its own is the same place of this context's half, and the record is copied there\nfn __act_in(a0: index, w: i64) -> index\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: i64 = conv second\n    same: u1 = cmp.eq here, w\n    if same\n        ret a0\n    hi: index = conv here\n    wi: index = conv w\n    hb: index = mul hi, {pool}\n    wb: index = mul wi, {pool}\n    n: index = sub a0, wb\n    l: index = add hb, n\n    __act_carry(a0, l)\n    ret l\n\n; ... and copied back where its turn ends\nfn __act_out(a0: index, a: index)\n    same: u1 = cmp.eq a0, a\n    if same\n        ret\n    __act_carry(a, a0)\n    ret\n\n; one record of the pool, every column of it, from one place to another\nfn __act_carry(from: index, to: index)\n", pool = POOL));
        }
        for (j, c) in cols.iter().enumerate() {
            out.push_str(&format!("    p{j}: ptr = addr __act_{c}\n    v{j}: i64 = load p{j}, from, 8\n    store v{j}, p{j}, to, 8\n", j = j, c = c));
        }
        out.push_str("    ret\n");
    }
    let pool: Vec<String> = lines.iter().filter_map(|l| l.strip_prefix("data __act_")).filter_map(|l| l.split(':').next()).map(|c| format!("__act_{}", c)).collect();
    let left: BTreeSet<usize> = if pooled { fns.iter().filter(|f| base(&f.name) != f.name).filter_map(|f| owner.get(&f.name).copied().filter(|&o| o < nu)).collect() } else { BTreeSet::new() };
    if moved {
        out.push_str("\n; the runner's: when the k-th move is due, a byte of the table of the cut after it, and a byte of what it hands across\nfn __move_at(k: index, t: i64)\n    p: ptr = addr __move_t\n    store t, p, k, 8\n    first: u1 = cmp.eq k, 0\n    if first\n        c: ptr = context()\n        x: __ctx = load c\n        x2: __ctx = set x, __due_move, t\n        store x2, c\n    ret\n");
        out.push_str(&format!("fn __move_set(k: index, i: index, v: u8)\n    p: ptr = addr __cuts\n    b: index = mul k, {size}\n    j: index = add b, i\n    store v, p, j, 1\n    ret\nfn __hand_set(k: index, i: index, v: u8)\n    p: ptr = addr __hands\n    b: index = mul k, {hsize}\n    j: index = add b, i\n    store v, p, j, 1\n    ret\n", size = size, hsize = hsize));
        // what a move hands across, once, in its turn: the wire's own
        // work, done for both places at the screen
        let mut h = String::from("\n; what the k-th move hands across (fm3 cut.md, \"the cut, moved\"), once, in the move's turn: each field of the context its table says, copied from the place that has it to the place that will use it; each stored stream's storage, where it will be read at a place that was not being given its items; and the place of each reader that moved. The table of the cut says what the wire carries from then on\nfn __hand(k: index)\n    cm: ptr(array(__ctx, 2)) = addr __ctx_mem\n    z0: index = const 0\n    z1: index = const 1\n    e0: ptr(__ctx) = index cm, z0\n    e1: ptr(__ctx) = index cm, z1\n    p0: ptr = cast e0\n    p1: ptr = cast e1\n    s0: __ctx = load p0\n    f0: __ctx = load p1\n    hp: ptr = addr __hands\n");
        h.push_str(&format!("    hb: index = mul k, {}\n", hsize));
        let mut c = 0;
        for (j, (name, ty, plain)) in hand_fields.iter().enumerate() {
            // (a field that holds an address is never handed: such a
            // move is refused; but a stream kept as a value that is a
            // string or an array is copied, fm3 log 267)
            if !plain && !ty.ends_with('$') {
                continue;
            }
            let (to_far, to_screen) = if *plain {
                (format!("        yield a{}\n", j), format!("        yield b{}\n", j))
            } else {
                let item = ty.trim_end_matches('$');
                carried_types.insert(item.to_string());
                (format!("        ca{j}: {ty} = __copied_{item}(a{j})\n        yield ca{j}\n", j = j, ty = ty, item = item), format!("        cb{j}: {ty} = __copied_{item}(b{j})\n        yield cb{j}\n", j = j, ty = ty, item = item))
            };
            h.push_str(&format!("    i{j}: index = add hb, {j}\n    h{j}: u8 = load hp, i{j}, 1\n    u{j}: u1 = cmp.eq h{j}, 1\n    d{j}: u1 = cmp.eq h{j}, 2\n    a{j}: {ty} = get s{c}, {name}\n    b{j}: {ty} = get f{c}, {name}\n    nb{j}: {ty} = if u{j}\n{to_far}    else\n        yield b{j}\n    na{j}: {ty} = if d{j}\n{to_screen}    else\n        yield a{j}\n    f{c2}: __ctx = set f{c}, {name}, nb{j}\n    s{c2}: __ctx = set s{c}, {name}, na{j}\n", j = j, ty = ty, name = name, c = c, c2 = c + 1, to_far = to_far, to_screen = to_screen));
            c += 1;
        }
        let mut used: BTreeSet<&'static str> = BTreeSet::new();
        for (r, (names, _)) in rings.iter().enumerate() {
            let (Some(at), Some(kind)) = (ring_at[r], kinds[r]) else { continue };
            used.insert(kind);
            let ty = ctx_fields.iter().find(|(n, _)| *n == names[0]).map(|(_, t)| t.clone()).unwrap_or_default();
            h.push_str(&format!("    ri{r}: index = add hb, {at}\n    rh{r}: u8 = load hp, ri{r}, 1\n    ra{r}: {ty} = get s{c}, {first}\n    rb{r}: {ty} = get f{c}, {first}\n    ru{r}: u1 = cmp.eq rh{r}, 1\n    if ru{r}\n        __hand_{kind}(ra{r}, rb{r})\n    rd{r}: u1 = cmp.eq rh{r}, 2\n    if rd{r}\n        __hand_{kind}(rb{r}, ra{r})\n", r = r, at = at, ty = ty, first = names[0], c = c, kind = kind));
            for (m, name) in names.iter().enumerate() {
                h.push_str(&format!("    qi{r}_{m}: index = add hb, {at}\n    qh{r}_{m}: u8 = load hp, qi{r}_{m}, 1\n    qa{r}_{m}: {ty} = get s{c}, {name}\n    qb{r}_{m}: {ty} = get f{c}, {name}\n    pa{r}_{m}: index = get qa{r}_{m}, pos\n    pb{r}_{m}: index = get qb{r}_{m}, pos\n    qu{r}_{m}: u1 = cmp.eq qh{r}_{m}, 1\n    qd{r}_{m}: u1 = cmp.eq qh{r}_{m}, 2\n    nq{r}_{m}: index = if qu{r}_{m}\n        yield pa{r}_{m}\n    else\n        yield pb{r}_{m}\n    np{r}_{m}: index = if qd{r}_{m}\n        yield pb{r}_{m}\n    else\n        yield pa{r}_{m}\n    qy{r}_{m}: {ty} = set qb{r}_{m}, pos, nq{r}_{m}\n    qx{r}_{m}: {ty} = set qa{r}_{m}, pos, np{r}_{m}\n    f{c2}: __ctx = set f{c}, {name}, qy{r}_{m}\n    s{c2}: __ctx = set s{c}, {name}, qx{r}_{m}\n", r = r, m = m, at = at + 1 + m, ty = ty, name = name, c = c, c2 = c + 1));
                c += 1;
            }
        }
        h.push_str(&format!("    store s{c}, p0\n    store f{c}, p1\n    ret\n", c = c));
        out.push_str(&h);
        if used.contains("queue") {
            out.push_str("\n; a queue handed across (lib/stream.ssa, the queue): the items not yet given back, in their slots, and how many it has had, where its first slot stands, and whether it has ended\nfn __hand_queue(a: any$, b: any$)\n    ra: ptr = get a, ring\n    rb: ptr = get b, ring\n    n: index = load ra\n    o: index = load ra, 8\n    k: index = sub n, o\n    qa: ptr(any) = load ra, 16\n    qb: ptr(any) = load rb, 16\n    s: any[] = pack qa, k, 1\n    d: any[] = pack qb, k, 1\n    copy d, s\n    store n, rb\n    store o, rb, 8\n    e: i64 = load ra, 56\n    store e, rb, 56\n    ret\n");
        }
        if used.contains("ring") {
            out.push_str("\n; a ring handed across (lib/stream.ssa, the ring): its items with their times, how many it has had, its beat and whether it has ended\nfn __hand_ring(a: any$, b: any$)\n    ra: ptr = get a, ring\n    rb: ptr = get b, ring\n    n: index = load ra\n    store n, rb\n    hz: i64 = load ra, 32\n    store hz, rb, 32\n    st: i64 = load ra, 40\n    store st, rb, 40\n    t0: i64 = load ra, 48\n    store t0, rb, 48\n    e: i64 = load ra, 56\n    store e, rb, 56\n    wa: ptr = load ra, 16\n    wb: ptr = load rb, 16\n    va: any[] = slice wa\n    vb: any[] = slice wb\n    copy vb, va\n    ta: ptr = load ra, 24\n    tb: ptr = load rb, 24\n    none: ptr = const 0\n    timed: u1 = cmp.ne ta, none\n    if timed\n        ka: i64[] = slice ta\n        kb: i64[] = slice tb\n        copy kb, ka\n    ret\n");
        }
    }
    let mut conts: Vec<(i64, Option<usize>)> = Vec::new();
    // (a continuation that says when it is next due is left at a step)
    let stepping: BTreeSet<usize> = if wire { fns.iter().filter(|f| f.body.iter().any(|l| l.trim().ends_with(": ptr = addr __act_due"))).filter_map(|f| owner.get(&f.name).copied().filter(|&o| o < nu)).collect() } else { BTreeSet::new() };
    if wire {
        let ids: Vec<(i64, Option<usize>)> = fns.iter().find(|f| f.name == "__act_go").map(|f| f.body.iter().filter_map(|l| def(l.trim())).filter_map(|(n, _, rhs)| Some((n.strip_prefix('n')?.parse::<i64>().ok()?, rhs.split('(').next()?.to_string()))).map(|(id, g)| (id, owner.get(&g).copied().filter(|&o| o < nu))).collect()).unwrap_or_default();
        let heres: BTreeSet<String> = lines.iter().flat_map(|l| called(l)).filter_map(|c| c.strip_prefix("__here__")).map(str::to_string).collect();
        // (the function the list names is the one written, which may be continuations now: its unit is found by its name)
        let heres: Vec<(String, Option<usize>)> = heres.into_iter().map(|g| { let o = units.iter().position(|u| u.irs.contains(&g)); (g, o) }).collect();
        let carried_fields: Vec<(usize, String, String)> = fields.iter().enumerate().filter(|(_, (_, ty, plain))| *plain && word(ty) && ty != "ptr").map(|(j, (n, t, _))| (j, n.clone(), t.clone())).collect();
        conts = ids.clone();
        out.push_str(&wire_text(&World { fbase, fields: carried_fields, ids, heres, arrivals: arrivals.clone(), pooled, input: fns.iter().any(|f| f.name == "__in_turn"), runs: fns.iter().any(|f| f.name == "__run"), writes: (present.contains("__out_ch"), present.contains("__out_block")), cols: pool.clone() }));
    }
    for item in &carried_types {
        out.push_str(&format!("\n; a value that holds an address, an array or a string, copied (fm3 log 267): its storage as the library lays it out, a queue's resident items in a buffer of their number or a ring's buffers whole, with its counts, its beat and its end, carved from the arena; the reader's place and its rules go with the value. A value is never modified, so the copy is the value\nfn __copied_{t}(v: {t}$) -> {t}$\n    r: ptr = get v, ring\n    ar: ptr = addr __arena\n    n: ptr = arena_alloc(ar, 64)\n    pushed: index = load r\n    o: index = load r, 8\n    hz: i64 = load r, 32\n    st: i64 = load r, 40\n    t0: i64 = load r, 48\n    e: i64 = load r, 56\n    w: index = load r, 24\n    store pushed, n\n    store o, n, 8\n    store hz, n, 32\n    store st, n, 40\n    store t0, n, 48\n    store e, n, 56\n    sz: index = sizeof {t}\n    stepped: u1 = cmp.ne st, 0\n    worded: u1 = cmp.ne w, 0\n    queue: u1 = and stepped, worded\n    if queue\n        k: index = sub pushed, o\n        one: index = const 1\n        cap: index = max(k, one)\n        bytes: index = mul sz, cap\n        total: index = add bytes, 16\n        vb: ptr = arena_alloc(ar, total)\n        buffer_init(vb, sz, cap)\n        slots: ptr = ptradd vb, 16\n        qa: ptr({t}) = load r, 16\n        qb: ptr({t}) = cast slots\n        s: {t}[] = pack qa, k, 1\n        d: {t}[] = pack qb, k, 1\n        copy d, s\n        store slots, n, 16\n        store cap, n, 24\n    else\n        wa: ptr = load r, 16\n        cap2: index = load wa, 8\n        bytes2: index = mul sz, cap2\n        total2: index = add bytes2, 16\n        wb: ptr = arena_alloc(ar, total2)\n        buffer_init(wb, sz, cap2)\n        va: {t}[] = slice wa\n        vc: {t}[] = slice wb\n        copy vc, va\n        store wb, n, 16\n        ta: ptr = load r, 24\n        if worded\n            tcap: index = load ta, 8\n            tbytes: index = mul tcap, 8\n            ttotal: index = add tbytes, 16\n            tb: ptr = arena_alloc(ar, ttotal)\n            eight: index = const 8\n            buffer_init(tb, eight, tcap)\n            ka: i64[] = slice ta\n            kb: i64[] = slice tb\n            copy kb, ka\n            store tb, n, 24\n        else\n            store ta, n, 24\n    v2: {t}$ = set v, ring, n\n    ret v2\n\n; ... where the wire was crossed to bring it: called at its own place, a function is handed the value itself\nfn __carried_{t}(v: {t}$, w: i64) -> {t}$\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: i64 = conv second\n    same: u1 = cmp.eq here, w\n    if same\n        ret v\n    c: {t}$ = __copied_{t}(v)\n    ret c\n", t = item));
    }
    Ok((out, Placed { units, facts, crossings: crossings.into_iter().collect(), fields, rings, sigs, copied, moved, hand_fields, ring_at, held, pool, left, sizes: (size, hsize), wire, glue, unwired, conts, stepping }))
}

/// what the world's text is written from
struct World {
    /// where the fields' flags begin in the cut's table
    fbase: usize,
    /// the fields an item may carry: its place among the flags, its name, its type
    fields: Vec<(usize, String, String)>,
    /// each continuation's number and its unit, or none where it is the screen's own
    ids: Vec<(i64, Option<usize>)>,
    /// the functions the list asks of, each with its unit
    heres: Vec<(String, Option<usize>)>,
    /// what an arriving push does, a block a site
    arrivals: String,
    pooled: bool,
    /// the store has an input that arrives at a time
    input: bool,
    /// ... and lines that walk a stored stream
    runs: bool,
    /// the output's two writes, where the text has them
    writes: (bool, bool),
    /// the pool's columns, for a copy of far's half (fm3 log 273)
    cols: Vec<String>,
}

/// The world and the wire of a build whose wire takes time (fm3 cut.md,
/// "a wire that takes time"; log 269), as text. `__world` is the only
/// thing that makes a place the current one. It hands over what the
/// wire holds when it is due, lets a place take the earliest turn it
/// has where the other has said it will begin nothing before it, has
/// each place say where it stands when it can do no more, and moves
/// its own clock on to the next time anything can happen
fn wire_text(w: &World) -> String {
    let (q, pool) = (WIRE_Q, POOL);
    let mut t = String::new();
    t.push_str("\n; which place this is: 0 the screen, 1 far\nfn __wplace() -> index\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    p: index = conv second\n    ret p\n");
    t.push_str("\n; is this unit at this place: a byte of the cut\nfn __unit_here(u: index) -> u1\n    cp: ptr = addr __cut\n    c: u8 = load cp, u, 1\n    at: index = conv c\n    p: index = __wplace()\n    h: u1 = cmp.eq at, p\n    ret h\n");
    for (g, o) in &w.heres {
        match o {
            Some(u) => t.push_str(&format!("fn __here__{g}() -> u1\n    h: u1 = __unit_here({u})\n    ret h\n", g = g, u = u)),
            None => t.push_str(&format!("fn __here__{g}() -> u1\n    p: index = __wplace()\n    h: u1 = cmp.eq p, 0\n    ret h\n", g = g)),
        }
    }
    if w.pooled {
        t.push_str("\n; is the continuation of this number at this place (fm3 log 269): its unit's byte of the cut, or the screen where it is a case's own call\nfn __act_here(k: i64) -> u1\n");
        for (id, o) in &w.ids {
            match o {
                Some(u) => t.push_str(&format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        h{id}: u1 = __unit_here({u})\n        ret h{id}\n", id = id, u = u)),
                None => t.push_str(&format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        p{id}: index = __wplace()\n        h{id}: u1 = cmp.eq p{id}, 0\n        ret h{id}\n", id = id)),
            }
        }
        t.push_str("    ret 1\n");
    }
    // an item put on the wire
    t.push_str(&format!("\n; an item put on the wire toward the other place: it arrives the wire's delay after the world's clock now, and carries its sender's own time, which a wire adds nothing to. Gives its row\nfn __wput(kind: i64, a: i64, b: i64, said: i64) -> index\n    p: index = __wplace()\n    to: index = sub 1, p\n    wp: ptr = addr __w\n    t2: index = mul to, 2\n    hi: index = add t2, 24\n    ti: index = add t2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    used: i64 = sub tail, head\n    room: u1 = cmp.lt used, {q}\n    check room\n    m: i64 = rem tail, {q}\n    mi: index = conv m\n    qb: index = mul to, {q}\n    row: index = add qb, mi\n    now: i64 = load wp\n    d: i64 = load wp, 8\n    at: i64 = add now, d\n    c: ptr = context()\n    x: __ctx = load c\n    stamp: i64 = get x, __clock\n    p1: ptr = addr __wq_at\n    store at, p1, row, 8\n    p2: ptr = addr __wq_kind\n    store kind, p2, row, 8\n    p3: ptr = addr __wq_a\n    store a, p3, row, 8\n    p4: ptr = addr __wq_b\n    store b, p4, row, 8\n    p5: ptr = addr __wq_t\n    store stamp, p5, row, 8\n{rank}    tail2: i64 = add tail, 1\n    store tail2, wp, ti, 8\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        tw: i64 = conv to\n        store tw, lp, l0, 8\n        l1: index = add l0, 1\n        store kind, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store at, lp, l3, 8\n        l4: index = add l0, 4\n        store stamp, lp, l4, 8\n        l5: index = add l0, 5\n        store said, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret row\n", q = q, log = WIRE_LOG, rank = if w.input { format!("    ap: ptr = addr __wa\n    ri: index = add p, 10\n    rk: i64 = load ap, ri, 8\n    p6: ptr = addr __wq_r\n    store rk, p6, row, 8\n    md: i64 = load ap\n    ah: u1 = cmp.ne md, 0\n    b8: index = mul p, 8\n    rci: index = add b8, 10\n    rc: i64 = load wp, rci, 8\n    ev: i64 = if ah\n        yield rc\n    else\n        yield {never}\n    p7: ptr = addr __wq_e\n    store ev, p7, row, 8\n", never = NEVER) } else { String::new() }));
    t.push_str("\n; one more thing this place has sent that the other may answer, and one more it has been sent\nfn __wsent()\n    p: index = __wplace()\n    wp: ptr = addr __w\n    b: index = mul p, 8\n    i: index = add b, 8\n    n: i64 = load wp, i, 8\n    n2: i64 = add n, 1\n    store n2, wp, i, 8\n");
    if w.input {
        t.push_str(&format!("    scr: u1 = cmp.eq p, 0\n    if scr\n        c: ptr = context()\n        x: __ctx = load c\n        st: i64 = get x, __clock\n        ap: ptr = addr __wa\n        sr: i64 = load ap, 80\n        m: i64 = rem n2, {sends}\n        mi: index = conv m\n        tp: ptr = addr __ws_t\n        store st, tp, mi, 8\n        rp: ptr = addr __ws_r\n        store sr, rp, mi, 8\n        __wvoid(st, sr, n2)\n", sends = SENDS));
    }
    t.push_str("    ret\nfn __wrecv()\n    p: index = __wplace()\n    wp: ptr = addr __w\n    b: index = mul p, 8\n    i: index = add b, 10\n    n: i64 = load wp, i, 8\n    n2: i64 = add n, 1\n    store n2, wp, i, 8\n    ret\n");
    // text
    if w.writes.0 {
        t.push_str("\n; the output's two writes (fm3 log 269): at the screen, the capture; far, an item of the wire, which needs nothing back\nfn __out_ch(c: u8)\n    p: index = __wplace()\n    far: u1 = cmp.ne p, 0\n    if far\n        a: i64 = conv c\n        row: index = __wput(1, a, 0, a)\n        ret\n    __cap_ch(c)\n    ret\n");
    }
    if w.writes.1 {
        t.push_str("fn __out_block(v: u8[])\n    p: index = __wplace()\n    far: u1 = cmp.ne p, 0\n    if far\n        k: index = len v\n        wp: ptr = addr __w\n        o: i64 = load wp, 40\n        oi: index = conv o\n        e: index = add oi, k\n        fits: u1 = cmp.le e, 65536\n        check fits\n        tp: ptr = addr __wq_text\n        qd: ptr(u8) = ptradd tp, oi\n        dst: u8[] = pack qd, k, 1\n        copy dst, v\n        o2: i64 = conv e\n        store o2, wp, 40\n        kk: i64 = conv k\n        w7: i64 = loop(i: index = 0, acc: i64 = kk) bound 8\n            end: u1 = cmp.ge i, k\n            full: u1 = cmp.ge i, 7\n            stop: u1 = or end, full\n            if stop\n                break acc\n            by: u8 = load v, i\n            bw: i64 = conv by\n            sh: i64 = mul acc, 256\n            acc2: i64 = add sh, bw\n            i2: index = add i, 1\n            continue i2, acc2\n        row: index = __wput(2, o, kk, w7)\n        ret\n    __cap_block(v)\n    ret\n");
    }
    t.push_str("\n; when what was just put in the capture is shown: the later of its own time and the world's clock. A piece that arrives before its time is kept until it; one that arrives after is late by the difference. A word for each character, in far's half of the marks\nfn __shown(n: index, k: index)\n    wp: ptr = addr __w\n    now: i64 = load wp\n    c: ptr = context()\n    x: __ctx = load c\n    own: i64 = get x, __clock\n    s: i64 = max(own, now)\n    sp: ptr = addr __out_t\n    top: index = add n, k\n    loop(i: index = n) bound 65536\n        done: u1 = cmp.ge i, top\n        if done\n            break\n        j: index = add i, 65538\n        store s, sp, j, 8\n        i2: index = add i, 1\n        continue i2\n    ret\n");
    t.push_str("\n; the time an item carries: the place it arrives at moves its clock up to it, and marks its output there\nfn __wclock(t: i64)\n    c: ptr = context()\n    y: __ctx = load c\n    was: i64 = get y, __clock\n    late: u1 = cmp.gt t, was\n    if late\n        mn: index = get y, __out_n\n        mm: ptr = get y, __out_m\n        store t, mm, mn, 8\n        y2: __ctx = set y, __clock, t\n        store y2, c\n    ret\n");
    // the fields an item carries
    let mut fo = format!("\n; the fields of the context an item carries: each the cut says crosses from this place that this place has changed since it last sent or was sent it. The context as it is goes in the item's row, and which of its fields are meant is a bit each\nfn __wfields_out(row: index) -> i64\n    p: index = __wplace()\n    c: ptr = context()\n    x: __ctx = load c\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    se: ptr(__ctx) = index sm, p\n    sp: ptr = cast se\n    s0: __ctx = load sp\n    cp: ptr = addr __cut\n    m0: i64 = const 0\n");
    let mut fi = format!("\n; ... and the place it arrives at takes them\nfn __wfields_in(row: index, mask: i64)\n    p: index = __wplace()\n    c: ptr = context()\n    y0: __ctx = load c\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    se: ptr(__ctx) = index sm, p\n    sp: ptr = cast se\n    s0: __ctx = load sp\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    x: __ctx = load qp\n", q2 = 2 * q);
    for (k, (j, name, ty)) in w.fields.iter().enumerate() {
        fo.push_str(&format!("    i{k}: index = add p, {at}\n    h{k}: u8 = load cp, i{k}, 1\n    f{k}: u1 = cmp.ne h{k}, 0\n    a{k}: {ty} = get x, {name}\n    b{k}: {ty} = get s{k}, {name}\n    n{k}: u1 = cmp.ne a{k}, b{k}\n    g{k}: u1 = and f{k}, n{k}\n    v{k}: {ty} = if g{k}\n        yield a{k}\n    else\n        yield b{k}\n    s{k1}: __ctx = set s{k}, {name}, v{k}\n    w{k}: i64 = if g{k}\n        yield {bit}\n    else\n        yield 0\n    m{k1}: i64 = or m{k}, w{k}\n", k = k, k1 = k + 1, at = w.fbase + 2 * j, ty = ty, name = name, bit = 1i64 << k));
        fi.push_str(&format!("    k{k}: i64 = and mask, {bit}\n    g{k}: u1 = cmp.ne k{k}, 0\n    a{k}: {ty} = get x, {name}\n    b{k}: {ty} = get y{k}, {name}\n    v{k}: {ty} = if g{k}\n        yield a{k}\n    else\n        yield b{k}\n    y{k1}: __ctx = set y{k}, {name}, v{k}\n    e{k}: {ty} = get s{k}, {name}\n    u{k}: {ty} = if g{k}\n        yield a{k}\n    else\n        yield e{k}\n    s{k1}: __ctx = set s{k}, {name}, u{k}\n", k = k, k1 = k + 1, ty = ty, name = name, bit = 1i64 << k));
    }
    let mut fd = String::from("\n; has this place changed a field that crosses since it last sent it or was sent it: then it has something to say\nfn __wfields_dirty() -> u1\n    p: index = __wplace()\n    c: ptr = context()\n    x: __ctx = load c\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    se: ptr(__ctx) = index sm, p\n    sp: ptr = cast se\n    s: __ctx = load sp\n    cp: ptr = addr __cut\n    m0: u1 = cmp.ne p, p\n");
    for (k, (j, name, ty)) in w.fields.iter().enumerate() {
        fd.push_str(&format!("    i{k}: index = add p, {at}\n    h{k}: u8 = load cp, i{k}, 1\n    f{k}: u1 = cmp.ne h{k}, 0\n    a{k}: {ty} = get x, {name}\n    b{k}: {ty} = get s, {name}\n    n{k}: u1 = cmp.ne a{k}, b{k}\n    g{k}: u1 = and f{k}, n{k}\n    m{k1}: u1 = or m{k}, g{k}\n", k = k, k1 = k + 1, at = w.fbase + 2 * j, ty = ty, name = name));
    }
    fd.push_str(&format!("    ret m{}\n", w.fields.len()));
    t.push_str(&fd);
    fo.push_str(&format!("    store s{n}, sp\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    store x, qp\n    ret m{n}\n", n = w.fields.len(), q2 = 2 * q));
    fi.push_str(&format!("    store y{n}, c\n    store s{n}, sp\n    ret\n", n = w.fields.len()));
    t.push_str(&fo);
    t.push_str(&fi);
    if w.pooled {
        t.push_str(&format!("\n; a function partway whose next continuation is at the other place (fm3 log 269): its record is put on the wire, with the fields that cross, and its place here is free. That is a call that crosses, and its end coming back\nfn __act_send(a: index, pc: i64)\n{park}    row: index = __wput(4, pc, 0, pc)\n    m: i64 = __wfields_out(row)\n    cp: ptr = addr __wq_c\n    store m, cp, row, 8\n    fl: index = add row, {flight}\n    __act_carry(a, fl)\n    dp: ptr = addr __act_due\n    store {never}: i64, dp, a, 8\n    pp: ptr = addr __act_pc\n    store 0: i64, pp, a, 8\n    __wsent()\n    p: index = __wplace()\n    li: index = add p, 6\n    wp: ptr = addr __w\n    store 1: i64, wp, li, 8\n    ret\n\n; a function begun for an item of a stream, by the function wired over it: none begun before it in this turn may have gone to the other place, where what it does there would come after what this one does here. Two characters that arrive together at a line whose function crosses stop here (fm3 question 156)\nfn __wonce()\n    p: index = __wplace()\n    li: index = add p, 6\n    wp: ptr = addr __w\n    gone: i64 = load wp, li, 8\n    none: u1 = cmp.eq gone, 0\n    check none\n    ret\n", flight = 2 * pool, never = NEVER, park = if w.input { "    ap: ptr = addr __wa\n    md: i64 = load ap\n    ah: u1 = cmp.ne md, 0\n    if ah\n        pq: ptr = addr __act_pc\n        store pc, pq, a, 8\n        c0: ptr = context()\n        x0: __ctx = load c0\n        nw: i64 = get x0, __clock\n        dq: ptr = addr __act_due\n        store nw, dq, a, 8\n        a1: i64 = conv a\n        a2: i64 = add a1, 1\n        store a2, ap, 16\n        p0: index = __wplace()\n        l0: index = add p0, 6\n        w0: ptr = addr __w\n        store 1: i64, w0, l0, 8\n        ret\n" } else { "" }));
    }
    t.push_str(&format!("\n; a push into a stored stream read at the other place, put on the wire, and what the place it arrives at does with it\nfn __wsend_push(site: i64, v: i64)\n{spoil}    row: index = __wput(3, site, v, site)\n    __wsent()\n    ret\nfn __wire_push(site: i64, v: i64)\n{arrivals}    ret\n", arrivals = w.arrivals, spoil = if w.input { "    ap: ptr = addr __wa\n    md: i64 = load ap\n    ah: u1 = cmp.ne md, 0\n    if ah\n        store 1: i64, ap, 8\n        ret\n" } else { "" }));
    // a telling
    let bound = if w.input { "    now: i64 = load wp\n    lim: i64 = add now, 1\n    screen: u1 = cmp.eq p, 0\n    sooner: u1 = cmp.lt lim, t0\n    same: u1 = cmp.eq lim, t0\n    first: u1 = cmp.lt 1, r0\n    tie: u1 = and same, first\n    under: u1 = or sooner, tie\n    use: u1 = and screen, under\n    t: i64 = if use\n        yield lim\n    else\n        yield t0\n    r: i64 = if use\n        yield 1\n    else\n        yield r0\n" } else { "    t: i64 = add t0, 0\n    r: i64 = add r0, 0\n" };
    t.push_str(&format!("\n; a place says where it stands (fm3 log 269): the time and the rank of the earliest thing it has, before which it will begin nothing unless it is sent something, and how much of what it was sent it has had. The screen can say nothing of a time the world's clock has not reached where a key may yet be pressed: its bound is then the moment after now, at the input's rank, which is the first. Said where this place's own earliest thing has changed, or it has been sent something since it last said; and, where only the world's clock has moved its bound on, when that frees the other to take the turn it is waiting for and not before. An item of the wire, carrying the fields that cross\nfn __wsay() -> u1\n    p: index = __wplace()\n{next}    wp: ptr = addr __w\n{bound}    b: index = mul p, 8\n    i2: index = add b, 10\n    recv: i64 = load wp, i2, 8\n    i5: index = add b, 13\n    st: i64 = load wp, i5, 8\n    i6: index = add b, 14\n    sr: i64 = load wp, i6, 8\n    i7: index = add b, 15\n    sa: i64 = load wp, i7, 8\n    e1: u1 = cmp.eq st, t\n    e2: u1 = cmp.eq sr, r\n    e3: u1 = cmp.eq sa, recv\n    e12: u1 = and e1, e2\n    p2: index = mul p, 2\n    i8: index = add p2, 28\n    ot: i64 = load wp, i8, 8\n    i9: index = add p2, 29\n    or0: i64 = load wp, i9, 8\n    o1: u1 = cmp.eq ot, t0\n    o2: u1 = cmp.eq or0, r0\n    o12: u1 = and o1, o2\n    j3: index = add b, 11\n    pt: i64 = load wp, j3, 8\n    j4: index = add b, 12\n    pr: i64 = load wp, j4, 8\n    olt: u1 = cmp.lt pt, st\n    oeq: u1 = cmp.eq pt, st\n    orl: u1 = cmp.lt pr, sr\n    otie: u1 = and oeq, orl\n    past: u1 = or olt, otie\n    nv: i64 = const {never}\n    never: u1 = cmp.ge pt, nv\n    unheld: u1 = or past, never\n    ult: u1 = cmp.lt pt, t\n    ueq: u1 = cmp.eq pt, t\n    url: u1 = cmp.lt pr, r\n    utie: u1 = and ueq, url\n    frees: u1 = or ult, utie\n    stays: u1 = xor frees, 1\n    idle: u1 = or unheld, stays\n    creep: u1 = or e12, idle\n    q1: u1 = and e3, o12\n    q2: u1 = and q1, creep\n    dirty: u1 = __wfields_dirty()\n    clean: u1 = xor dirty, 1\n    quiet: u1 = and q2, clean\n    if quiet\n        ret 0\n    row: index = __wput(5, t, r, t)\n    m: i64 = __wfields_out(row)\n    cp: ptr = addr __wq_c\n    store m, cp, row, 8\n    ep: ptr = addr __wq_e\n    store recv, ep, row, 8\n    store t, wp, i5, 8\n    store r, wp, i6, 8\n    store recv, wp, i7, 8\n    store t0, wp, i8, 8\n    store r0, wp, i9, 8\n    carries: u1 = cmp.ne m, 0\n    if carries\n        __wsent()\n    ret 1\n", bound = bound, never = NEVER, next = if w.input { "    t9: i64, r9: i64 = __list_next()\n    wa: ptr = addr __wa\n    pk: i64 = load wa, 16\n    pkd: u1 = cmp.ne pk, 0\n    isfar: u1 = cmp.ne p, 0\n    held: u1 = and pkd, isfar\n    bt: i64 = load wa, 24\n    br: i64 = load wa, 32\n    t0: i64 = if held\n        yield bt\n    else\n        yield t9\n    r0: i64 = if held\n        yield br\n    else\n        yield r9\n" } else { "    t0: i64, r0: i64 = __list_next()\n" }));
    // delivery
    let take = if w.pooled { format!("    is4: u1 = cmp.eq kind, 4\n    if is4\n        __wrecv()\n        cq: ptr = addr __wq_c\n        cm4: i64 = load cq, row, 8\n        __wfields_in(row, cm4)\n        slot: index = __act_slot()\n        bp: ptr = addr __act_born\n        born: i64 = load bp, slot, 8\n        fl: index = add row, {flight}\n        __act_carry(fl, slot)\n        store born, bp, slot, 8\n        pp: ptr = addr __act_pc\n        store a, pp, slot, 8\n        __act_run(slot, a)\n        __act_next()\n", flight = 2 * pool) } else { String::new() };
    t.push_str(&format!("\n; the wire's earliest item toward a place is handed over, where the world's clock has reached the time it arrives: text into the capture; a push made again, and the lines that walk the stream given their turn; a function partway given a place of the pool and gone on with at once, being part of the turn that sent it; a telling kept\nfn __wdeliver(pi: i32) -> u1\n    __zero_context(pi)\n    p: index = conv pi\n    wp: ptr = addr __w\n    p2: index = mul p, 2\n    hi: index = add p2, 24\n    ti: index = add p2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    none: u1 = cmp.ge head, tail\n    if none\n        ret 0\n    m: i64 = rem head, {q}\n    mi: index = conv m\n    qb: index = mul p, {q}\n    row: index = add qb, mi\n    ap: ptr = addr __wq_at\n    at: i64 = load ap, row, 8\n    now: i64 = load wp\n    notyet: u1 = cmp.gt at, now\n    if notyet\n        ret 0\n    head2: i64 = add head, 1\n    store head2, wp, hi, 8\n    gi: index = add p, 6\n    store 0: i64, wp, gi, 8\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    aq: ptr = addr __wq_a\n    a: i64 = load aq, row, 8\n    bq: ptr = addr __wq_b\n    b: i64 = load bq, row, 8\n    tq: ptr = addr __wq_t\n    stamp: i64 = load tq, row, 8\n{head}    is3: u1 = cmp.eq kind, 3\n    if is3\n        __wrecv()\n        __wire_push(a, b)\n{run}{take}    is5: u1 = cmp.eq kind, 5\n    if is5\n        cq5: ptr = addr __wq_c\n        cm5: i64 = load cq5, row, 8\n        __wfields_in(row, cm5)\n        fed: u1 = cmp.ne cm5, 0\n        if fed\n            __wrecv()\n        b8: index = mul p, 8\n        j1: index = add b8, 11\n        store a, wp, j1, 8\n        j2: index = add b8, 12\n        store b, wp, j2, 8\n        eq: ptr = addr __wq_e\n        ack: i64 = load eq, row, 8\n        j3: index = add b8, 9\n        store ack, wp, j3, 8\n{unpark}    ret 1\n", q = q, unpark = if w.input && w.pooled { "        ua: ptr = addr __wa\n        upk: i64 = load ua, 16\n        uon: u1 = cmp.ne upk, 0\n        ufar: u1 = cmp.ne p, 0\n        uu: u1 = and uon, ufar\n        if uu\n            ubt: i64 = load ua, 24\n            ubr: i64 = load ua, 32\n            ult: u1 = cmp.lt ubt, a\n            ueq: u1 = cmp.eq ubt, a\n            url: u1 = cmp.lt ubr, b\n            utie: u1 = and ueq, url\n            upast: u1 = or ult, utie\n            if upast\n                __wunpark()\n" } else { "" }, run = if w.runs { "        __run()\n" } else { "" }, take = take, head = if w.input { "    rq: ptr = addr __wq_r\n    rk: i64 = load rq, row, 8\n    eq0: ptr = addr __wq_e\n    ev: i64 = load eq0, row, 8\n    cq0: ptr = addr __wq_c\n    cm0: i64 = load cq0, row, 8\n    k1: u1 = cmp.eq kind, 1\n    k2: u1 = cmp.eq kind, 2\n    text: u1 = or k1, k2\n    if text\n        __wkeep(kind, a, b, stamp, rk, ev)\n        ret 1\n    k5: u1 = cmp.eq kind, 5\n    nof: u1 = cmp.eq cm0, 0\n    plain: u1 = and k5, nof\n    scr: u1 = cmp.eq p, 0\n    if plain\n        if scr\n        else\n            __wclock(stamp)\n            __wpassed(a, b)\n    else\n        if scr\n            __wflush(stamp, rk)\n        else\n            __wback(stamp, rk)\n        wa: ptr = addr __wa\n        ri: index = add p, 10\n        store rk, wa, ri, 8\n        __wclock(stamp)\n".to_string() } else { format!("    __wclock(stamp)\n{}{}", if w.writes.0 { "    is1: u1 = cmp.eq kind, 1\n    if is1\n        ch: u8 = conv a\n        __cap_ch(ch)\n" } else { "" }, if w.writes.1 { "    is2: u1 = cmp.eq kind, 2\n    if is2\n        ao: index = conv a\n        bk: index = conv b\n        tp: ptr = addr __wq_text\n        qs: ptr(u8) = ptradd tp, ao\n        v: u8[] = pack qs, bk, 1\n        __cap_block(v)\n" } else { "" }) }));
    // a turn
    // (where far may work ahead, fm3 log 273: the screen shows a piece it is keeping where that comes before its own earliest thing and its time has come; and far, not told, takes its turn ahead)
    let next = if w.input { "    t1: i64, r1: i64 = __list_next()\n    ap: ptr = addr __wa\n    pk0: i64 = load ap, 16\n    pkn: u1 = cmp.ne pk0, 0\n    isf: u1 = cmp.ne p, 0\n    pkd: u1 = and pkn, isf\n    bt0: i64 = load ap, 24\n    br0: i64 = load ap, 32\n    t: i64 = if pkd\n        yield bt0\n    else\n        yield t1\n    r: i64 = if pkd\n        yield br0\n    else\n        yield r1\n" } else { "    t: i64, r: i64 = __list_next()\n" };
    let kept = if w.input { "    kn: i64 = load ap, 104\n    khas: u1 = cmp.gt kn, 0\n    kq: u1 = and screen, khas\n    if kq\n        kh: i64 = load ap, 96\n        khi: index = conv kh\n        ktp: ptr = addr __wk_t\n        kt: i64 = load ktp, khi, 8\n        krp: ptr = addr __wk_r\n        kr: i64 = load krp, khi, 8\n        klt: u1 = cmp.lt kt, t\n        keq: u1 = cmp.eq kt, t\n        krl: u1 = cmp.lt kr, r\n        ktie: u1 = and keq, krl\n        kfirst: u1 = or klt, ktie\n        kdue: u1 = cmp.le kt, now\n        kgo: u1 = and kfirst, kdue\n        if kgo\n            __wshow()\n            ret 1\n" } else { "" };
    let ahead = if w.input {
        format!("    unfree: u1 = xor free, 1\n    a0: u1 = and g1, unfree\n    a1: u1 = and a0, far\n    cn: i64 = load ap, 48\n    croom: u1 = cmp.lt cn, {n}\n    a2: u1 = and a1, croom\n    mlen: index = load ap, 120\n    fits: u1 = cmp.le mlen, {mem}\n    a3: u1 = and a2, fits\n    bt: i64 = load ap, 24\n    br: i64 = load ap, 32\n    bte: u1 = cmp.eq bt, t\n    bre: u1 = cmp.eq br, r\n    barred: u1 = and bte, bre\n    unbarred: u1 = xor barred, 1\n    a4: u1 = and a3, unbarred\n    hd: i64 = load wp, 192\n    tl: i64 = load wp, 200\n    usd: i64 = sub tl, hd\n    wroom: u1 = cmp.lt usd, {half}\n    a5: u1 = and a4, wroom\n{rows}    if a6\n        k: index = __wcopy(t, r)\n        tail0: i64 = load wp, 200\n        sent0: i64 = load wp, 128\n        left0: i64 = load wp, 56\n        ln0: i64 = load wp, 32\n        off0: i64 = load wp, 40\n        sm: ptr(array(__ctx, 2)) = addr __wshadow\n        one: index = const 1\n        se: ptr(__ctx) = index sm, one\n        sp: ptr = cast se\n        sh: __ctx = load sp\n        kp: ptr = addr __wa_keep\n        store sh, kp\n        store 1: i64, ap\n        store 0: i64, ap, 8\n        store 0: i64, ap, 16\n        store r, ap, 88\n        store 0: i64, wp, 56\n        __list_turn()\n        store 0: i64, ap\n        sp1: i64 = load ap, 8\n        spoiled: u1 = cmp.ne sp1, 0\n        dirty2: u1 = __wfields_dirty()\n        bad: u1 = or spoiled, dirty2\n        if bad\n            __wrestore(k)\n            store tail0, wp, 200\n            store sent0, wp, 128\n            store left0, wp, 56\n            store ln0, wp, 32\n            store off0, wp, 40\n            sh2: __ctx = load kp\n            store sh2, sp\n            store cn, ap, 48\n            store 0: i64, ap, 16\n            store t, ap, 24\n            store r, ap, 32\n            th: i64 = load ap, 56\n            th2: i64 = add th, 1\n            store th2, ap, 56\n            __wnote(9, t, 0)\n            ret 1\n        pk: i64 = load ap, 16\n        parked: u1 = cmp.ne pk, 0\n        if parked\n            store t, ap, 24\n            store r, ap, 32\n        ret 1\n", n = AHEAD, mem = AHEAD_MEM, half = q / 2, rows = if w.pooled { format!("    cx: ptr = context()\n    xx: __ctx = load cx\n    an: index = get xx, __act_n\n    arow: u1 = cmp.le an, {}\n    a6: u1 = and a5, arow\n", AHEAD_ROWS) } else { "    a6: u1 = and a5, a5\n".to_string() })
    } else {
        String::new()
    };
    t.push_str(&format!("\n; a place takes the earliest turn it has (fm3 log 269), where the other has had everything this place sent it, a field of the context this place has changed that the other reads among it, and has said it will begin nothing before that turn: of two of one rank due together at the two places the screen's is first (fm3 question 155). The screen takes a turn for a time when the world's clock has reached it; far has no clock but its own. `__w`'s fourth word is a test's: a place that does not wait to be told\nfn __wturn(pi: i32) -> u1\n    __zero_context(pi)\n    p: index = conv pi\n{next}    nv: i64 = const {never}\n    some: u1 = cmp.lt t, nv\n    wp: ptr = addr __w\n    b: index = mul p, 8\n    i0: index = add b, 8\n    sent: i64 = load wp, i0, 8\n    i1: index = add b, 9\n    acked: i64 = load wp, i1, 8\n    i3: index = add b, 11\n    pt: i64 = load wp, i3, 8\n    i4: index = add b, 12\n    pr: i64 = load wp, i4, 8\n    quiet: u1 = cmp.eq sent, acked\n    lt: u1 = cmp.lt t, pt\n    eq: u1 = cmp.eq t, pt\n    rlt: u1 = cmp.lt r, pr\n    rle: u1 = cmp.le r, pr\n    screen: u1 = cmp.eq p, 0\n    rok: u1 = if screen\n        yield rle\n    else\n        yield rlt\n    tie: u1 = and eq, rok\n    told: u1 = or lt, tie\n    but: i64 = load wp, 24\n    bit: i64 = if screen\n        yield 1\n    else\n        yield 2\n    bm: i64 = and but, bit\n    blind: u1 = cmp.ne bm, 0\n    free: u1 = or told, blind\n    now: i64 = load wp\n    intime: u1 = cmp.le t, now\n    far: u1 = cmp.ne p, 0\n    timely: u1 = or far, intime\n    dirty: u1 = __wfields_dirty()\n    clean: u1 = xor dirty, 1\n    g0: u1 = and some, quiet\n    g1: u1 = and g0, clean\n    g2: u1 = and g1, free\n    go: u1 = and g2, timely\n{kept}    if go\n        gi: index = add p, 6\n        store 0: i64, wp, gi, 8\n{rank}        ret 1\n{ahead}    ret 0\n", never = NEVER, next = next, kept = kept, rank = if w.input && w.pooled { "        ri: index = add p, 10\n        store r, ap, ri, 8\n        store 0: i64, ap\n        if pkd\n            __wunpark()\n        else\n            __list_turn()\n" } else if w.input { "        ri: index = add p, 10\n        store r, ap, ri, 8\n        store 0: i64, ap\n        __list_turn()\n" } else { "        __list_turn()\n" }, ahead = ahead));
    t.push_str(&format!("\n; nothing can happen now: the world's clock moves on to the next time something can, an item's arrival or the time of a place's earliest thing, or stays, and says so, where there is none\nfn __wnext() -> u1\n    wp: ptr = addr __w\n    now: i64 = load wp\n    nv: i64 = const {never}\n    h0: i64 = load wp, 192\n    t0: i64 = load wp, 200\n    has0: u1 = cmp.lt h0, t0\n    a0: i64 = if has0\n        m0: i64 = rem h0, {q}\n        r0: index = conv m0\n        p0: ptr = addr __wq_at\n        v0: i64 = load p0, r0, 8\n        yield v0\n    else\n        yield nv\n    h1: i64 = load wp, 208\n    t1: i64 = load wp, 216\n    has1: u1 = cmp.lt h1, t1\n    a1: i64 = if has1\n        m1: i64 = rem h1, {q}\n        r1: index = conv m1\n        r1q: index = add r1, {q}\n        p1: ptr = addr __wq_at\n        v1: i64 = load p1, r1q, 8\n        yield v1\n    else\n        yield nv\n    __zero_context(0)\n    s: i64, sr: i64 = __list_next()\n    sl: u1 = cmp.gt s, now\n    s2: i64 = if sl\n        yield s\n    else\n        yield nv\n    __zero_context(1)\n    f: i64, fr: i64 = __list_next()\n    fl: u1 = cmp.gt f, now\n    f2: i64 = if fl\n        yield f\n    else\n        yield nv\n    n1: i64 = min(a0, a1)\n    n2: i64 = min(n1, s2)\n{kept}    none: u1 = cmp.ge n3, nv\n    if none\n        ret 0\n    store n3, wp\n    ret 1\n", never = NEVER, q = q, kept = if w.input { "    n3a: i64 = min(n2, f2)\n    ap: ptr = addr __wa\n    kn: i64 = load ap, 104\n    kh: i64 = load ap, 96\n    khi: index = conv kh\n    ktp: ptr = addr __wk_t\n    kt: i64 = load ktp, khi, 8\n    khas: u1 = cmp.gt kn, 0\n    klate: u1 = cmp.gt kt, now\n    kok: u1 = and khas, klate\n    k2: i64 = if kok\n        yield kt\n    else\n        yield nv\n    n3: i64 = min(n3a, k2)\n" } else { "    n3: i64 = min(n2, f2)\n" }));
    t.push_str(&format!("\n; the world (fm3 cut.md, \"a wire that takes time\"; log 269): called by a case's twin when its call has been begun at the screen. Before its clock moves the lines far that walk a stored stream are given their turn, as the screen's were at the start, and each place tells the other where it stands, which takes no time: the two were joined before the case began. Then, until nothing is left: what the wire holds that is due is handed over; a place that may takes a turn; a place that can do no more says where it stands; and the clock moves on. A place left with something it was never free to do is a fault, and stops\nfn __world()\n    wp: ptr = addr __w\n    __zero_context(1)\n    fc: ptr = context()\n    fx: __ctx = load fc\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    z0: index = const 0\n    z1: index = const 1\n    e0: ptr(__ctx) = index sm, z0\n    e1: ptr(__ctx) = index sm, z1\n    q0: ptr = cast e0\n    q1: ptr = cast e1\n    store fx, q0\n    store fx, q1\n{start}    d: i64 = load wp, 8\n    store 0: i64, wp, 8\n    loop(hn: i64 = 0) bound 64\n        hd: u1 = cmp.ge hn, 64\n        if hd\n            break\n        hn2: i64 = add hn, 1\n        __zero_context(0)\n        y0: u1 = __wsay()\n        __zero_context(1)\n        y1: u1 = __wsay()\n        y2: u1 = __wdeliver(0)\n        y3: u1 = __wdeliver(1)\n        y01: u1 = or y0, y1\n        y23: u1 = or y2, y3\n        any: u1 = or y01, y23\n        if any\n            continue hn2\n        break\n    store d, wp, 8\n    loop(n: i64 = 0)\n        some: u1 = cmp.lt n, 4000000\n        check some\n        n2: i64 = add n, 1\n        d0: u1 = __wdeliver(0)\n        if d0\n            continue n2\n        d1: u1 = __wdeliver(1)\n        if d1\n            continue n2\n        g0: u1 = __wturn(0)\n        if g0\n            continue n2\n        g1: u1 = __wturn(1)\n        if g1\n            continue n2\n        __zero_context(0)\n        s0: u1 = __wsay()\n        if s0\n            continue n2\n        __zero_context(1)\n        s1: u1 = __wsay()\n        if s1\n            continue n2\n        more: u1 = __wnext()\n        if more\n            continue n2\n        break\n    nv: i64 = const {never}\n    __zero_context(1)\n    ft: i64, fr: i64 = __list_next()\n    fdone: u1 = cmp.ge ft, nv\n    check fdone\n    __zero_context(0)\n    st: i64, sr: i64 = __list_next()\n    sdone: u1 = cmp.ge st, nv\n    check sdone\n{end}    ret\n", never = NEVER, start = if w.runs { "    __run()\n" } else { "" }, end = if w.input { "    ap: ptr = addr __wa\n    th: i64 = load ap, 56\n    sc: ptr = context()\n    sx: __ctx = load sc\n    on: index = get sx, __out_n\n    ok: u1 = cmp.lt on, 65536\n    if ok\n        oj: index = add on, 65538\n        op: ptr = addr __out_t\n        store th, op, oj, 8\n" } else { "" }));
    if w.input {
        t.push_str(&ahead_text(w));
    }
    t.push_str("\n; the runner's: the wire's delay; what `__out_mark` reads back, 0 a piece's own time, 1 when it was shown, 2 the wire's record; and a test's switch\nfn __wire_set(d: i64)\n    wp: ptr = addr __w\n    store d, wp, 8\n    ret\nfn __wire_mode(m: i64)\n    wp: ptr = addr __w\n    store m, wp, 16\n    ret\nfn __wire_but(s: i64)\n    wp: ptr = addr __w\n    store s, wp, 24\n    ret\n");
    t
}

/// Working ahead of a key (fm3 cut.md, "working ahead of a key"; log
/// 273), as text, for a store with an input that arrives at a time.
/// Far's copies of itself: made before a turn taken ahead, put back
/// when something from the screen says that turn was wrong, dropped
/// when the screen's telling has passed it. And the screen's queue of
/// kept text: a piece that arrives is kept until its time, shown in
/// the order of the turns that wrote it, and dropped unshown where it
/// was written in ignorance of something the screen has sent since
fn ahead_text(w: &World) -> String {
    let mut t = String::new();
    t.push_str(&format!("\n; a line of the wire's record that is no item: 6 a character and 7 a block of text dropped at the screen unshown, 8 far gone back, with the time it went back to and how many turns it threw away, 9 a turn far took ahead and took back, because it would have sent more than text\nfn __wnote(kind: i64, stamp: i64, said: i64)\n    wp: ptr = addr __w\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        now: i64 = load wp\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        store 0: i64, lp, l0, 8\n        l1: index = add l0, 1\n        store kind, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store now, lp, l3, 8\n        l4: index = add l0, 4\n        store stamp, lp, l4, 8\n        l5: index = add l0, 5\n        store said, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret\n", log = WIRE_LOG));
    if w.pooled {
        t.push_str(&format!("\n; far's half of the pool, its first n places, into the k-th copy or back out of it: every column\nfn __wpool(k: index, n: index, back: u1)\n    sp: ptr = addr __wa_pool\n    kb: index = mul k, {per}\n    loop(i: index = 0) bound {rows}\n        done: u1 = cmp.ge i, n\n        if done\n            break\n        f: index = add i, {pool}\n        s0: index = add kb, i\n", per = AHEAD_ROWS * w.cols.len(), rows = AHEAD_ROWS, pool = POOL));
        for (j, c) in w.cols.iter().enumerate() {
            t.push_str(&format!("        p{j}: ptr = addr {c}\n        d{j}: index = add s0, {at}\n        if back\n            v{j}: i64 = load sp, d{j}, 8\n            store v{j}, p{j}, f, 8\n        else\n            w{j}: i64 = load p{j}, f, 8\n            store w{j}, sp, d{j}, 8\n", j = j, c = c, at = j * AHEAD_ROWS));
        }
        t.push_str("        i2: index = add i, 1\n        continue i2\n    ret\n");
    }
    t.push_str(&format!("\n; the part of the arena far's context was made in, every stream of far's with its counts and its items, into the k-th copy or back out of it\nfn __wmem(k: index, back: u1)\n    ap: ptr = addr __wa\n    base: ptr = load ap, 112\n    n: index = load ap, 120\n    lq: ptr(u8) = cast base\n    live: u8[] = pack lq, n, 1\n    mp: ptr = addr __wa_mem\n    off: index = mul k, {mem}\n    mq: ptr(u8) = ptradd mp, off\n    held: u8[] = pack mq, n, 1\n    if back\n        copy live, held\n    else\n        copy held, live\n    ret\n", mem = AHEAD_MEM));
    let pool_out = if w.pooled { "    an: index = get x, __act_n\n    no: u1 = cmp.ne k, k\n    __wpool(k, an, no)\n" } else { "    no: u1 = cmp.ne k, k\n" };
    let pool_in = if w.pooled { "    an: index = get x, __act_n\n    __wpool(k, an, yes)\n" } else { "" };
    t.push_str(&format!("\n; far keeps a copy of itself before a turn it takes ahead, for time t at rank r: its context, its half of the pool and its streams. Gives which copy it is\nfn __wcopy(t: i64, r: i64) -> index\n    ap: ptr = addr __wa\n    h: i64 = load ap, 40\n    n: i64 = load ap, 48\n    s0: i64 = add h, n\n    k0: i64 = rem s0, {n}\n    k: index = conv k0\n    tp: ptr = addr __wa_t\n    store t, tp, k, 8\n    rp: ptr = addr __wa_r\n    store r, rp, k, 8\n    n2: i64 = add n, 1\n    store n2, ap, 48\n    c: ptr = context()\n    x: __ctx = load c\n    sm: ptr(array(__ctx, {n})) = addr __wa_ctx\n    se: ptr(__ctx) = index sm, k\n    sp: ptr = cast se\n    store x, sp\n{pool_out}    __wmem(k, no)\n    ret k\n\n; ... and is put back to it. `__w`'s fourth word, 16, is a test's: its streams are not\nfn __wrestore(k: index)\n    sm: ptr(array(__ctx, {n})) = addr __wa_ctx\n    se: ptr(__ctx) = index sm, k\n    sp: ptr = cast se\n    x: __ctx = load sp\n    c: ptr = context()\n    store x, c\n    yes: u1 = cmp.eq k, k\n{pool_in}    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 16\n    whole: u1 = cmp.eq bm, 0\n    if whole\n        __wmem(k, yes)\n    ret\n", n = AHEAD, pool_out = pool_out, pool_in = pool_in));
    t.push_str(&format!("\n; something has reached far from the screen that is not a plain telling, for time s at rank r: far goes back to the copy it made before its first turn taken ahead for then or later, of two alike the screen's being first, and those turns are thrown away. Every copy is then gone, the earlier ones having been passed. `__w`'s fourth word, 4, is a test's: far is not put back\nfn __wback(s: i64, r: i64)\n    ap: ptr = addr __wa\n    h: i64 = load ap, 40\n    n: i64 = load ap, 48\n    tp: ptr = addr __wa_t\n    rp: ptr = addr __wa_r\n    j: i64 = loop(i: i64 = 0) bound {n1}\n        done: u1 = cmp.ge i, n\n        if done\n            break i\n        k0: i64 = add h, i\n        k1: i64 = rem k0, {n}\n        ki: index = conv k1\n        t: i64 = load tp, ki, 8\n        q: i64 = load rp, ki, 8\n        gt: u1 = cmp.gt t, s\n        eq: u1 = cmp.eq t, s\n        rge: u1 = cmp.ge q, r\n        tie: u1 = and eq, rge\n        after: u1 = or gt, tie\n        if after\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    some: u1 = cmp.lt j, n\n    if some\n        g0: i64 = add h, j\n        g1: i64 = rem g0, {n}\n        gi: index = conv g1\n        wp: ptr = addr __w\n        but: i64 = load wp, 24\n        bm: i64 = and but, 4\n        put: u1 = cmp.eq bm, 0\n        if put\n            __wrestore(gi)\n        undone: i64 = sub n, j\n        th: i64 = load ap, 56\n        th2: i64 = add th, undone\n        store th2, ap, 56\n        c: ptr = context()\n        x: __ctx = load c\n        to: i64 = get x, __clock\n        __wnote(8, to, undone)\n        store 0: i64, ap, 16\n    pk: i64 = load ap, 16\n    loose: u1 = cmp.eq pk, 0\n    if loose\n        store -1: i64, ap, 24\n    store 0: i64, ap, 40\n    store 0: i64, ap, 48\n    ret\n\n; a plain telling has reached far: the copies made before turns the screen's telling has passed are dropped\nfn __wpassed(pt: i64, pr: i64)\n    ap: ptr = addr __wa\n    h: i64 = load ap, 40\n    n: i64 = load ap, 48\n    tp: ptr = addr __wa_t\n    rp: ptr = addr __wa_r\n    d: i64 = loop(i: i64 = 0) bound {n1}\n        done: u1 = cmp.ge i, n\n        if done\n            break i\n        k0: i64 = add h, i\n        k1: i64 = rem k0, {n}\n        ki: index = conv k1\n        t: i64 = load tp, ki, 8\n        q: i64 = load rp, ki, 8\n        lt: u1 = cmp.lt t, pt\n        eq: u1 = cmp.eq t, pt\n        rlt: u1 = cmp.lt q, pr\n        tie: u1 = and eq, rlt\n        before: u1 = or lt, tie\n        if before\n        else\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    h1: i64 = add h, d\n    h2: i64 = rem h1, {n}\n    n2: i64 = sub n, d\n    store h2, ap, 40\n    store n2, ap, 48\n    ret\n", n = AHEAD, n1 = AHEAD + 1));
    if w.pooled {
        t.push_str("\n; the screen's telling has passed the turn far took ahead as far as a call that crosses: the function it left here is put on the wire now, as part of that turn, and far is free of it\nfn __wunpark()\n    ap: ptr = addr __wa\n    pk: i64 = load ap, 16\n    s1: i64 = sub pk, 1\n    slot: index = conv s1\n    br: i64 = load ap, 32\n    store br, ap, 88\n    store 0: i64, ap, 16\n    store -1: i64, ap, 24\n    store 0: i64, ap\n    wp: ptr = addr __w\n    store 0: i64, wp, 56\n    pp: ptr = addr __act_pc\n    pc: i64 = load pp, slot, 8\n    __act_run(slot, pc)\n    __act_next()\n    ret\n");
    }
    // the screen's kept text
    t.push_str(&format!("\n; a piece of text has reached the screen: kept until its time, in the order it came, which is the order of the turns that wrote it. But one written in a turn taken ahead says how much of the screen's sending far had had, and where that is behind and the piece is for the time of the first sending it did not know of, or later, it is void and is dropped unshown. `__w`'s fourth word, 8, is a test's: the screen does not drop\nfn __wkeep(kind: i64, a: i64, b: i64, t: i64, r: i64, e: i64)\n    wp: ptr = addr __w\n    sent: i64 = load wp, 64\n    behind: u1 = cmp.lt e, sent\n    if behind\n        e1: i64 = add e, 1\n        m: i64 = rem e1, {sends}\n        mi: index = conv m\n        stp: ptr = addr __ws_t\n        srp: ptr = addr __ws_r\n        st: i64 = load stp, mi, 8\n        sr: i64 = load srp, mi, 8\n        gt: u1 = cmp.gt t, st\n        eq: u1 = cmp.eq t, st\n        rge: u1 = cmp.ge r, sr\n        tie: u1 = and eq, rge\n        after: u1 = or gt, tie\n        but: i64 = load wp, 24\n        bm: i64 = and but, 8\n        drops: u1 = cmp.eq bm, 0\n        stale: u1 = and after, drops\n        if stale\n            __wdropped(kind, a, b, t)\n            ret\n    ap: ptr = addr __wa\n    h: i64 = load ap, 96\n    n: i64 = load ap, 104\n    room: u1 = cmp.lt n, {kept}\n    check room\n    k0: i64 = add h, n\n    k1: i64 = rem k0, {kept}\n    ki: index = conv k1\n    p1: ptr = addr __wk_t\n    store t, p1, ki, 8\n    p2: ptr = addr __wk_r\n    store r, p2, ki, 8\n    p3: ptr = addr __wk_e\n    store e, p3, ki, 8\n    p4: ptr = addr __wk_kind\n    store kind, p4, ki, 8\n    p5: ptr = addr __wk_a\n    store a, p5, ki, 8\n    p6: ptr = addr __wk_b\n    store b, p6, ki, 8\n    n2: i64 = add n, 1\n    store n2, ap, 104\n    ret\n\n; a piece dropped unshown, written in the wire's record for a person: a character, or a block as its length and its first seven characters\nfn __wdropped(kind: i64, a: i64, b: i64, t: i64)\n    block: u1 = cmp.eq kind, 2\n    if block\n        tp: ptr = addr __wq_text\n        ao: index = conv a\n        k: index = conv b\n        qs: ptr(u8) = ptradd tp, ao\n        v: u8[] = pack qs, k, 1\n        w7: i64 = loop(i: index = 0, acc: i64 = b) bound 8\n            end: u1 = cmp.ge i, k\n            full: u1 = cmp.ge i, 7\n            stop: u1 = or end, full\n            if stop\n                break acc\n            by: u8 = load v, i\n            bw: i64 = conv by\n            sh: i64 = mul acc, 256\n            acc2: i64 = add sh, bw\n            i2: index = add i, 1\n            continue i2, acc2\n        __wnote(7, t, w7)\n        ret\n    __wnote(6, t, a)\n    ret\n\n; the screen has sent something, its `sent`-th, for time s at rank r: every piece it is keeping that was written in ignorance of it and is for then or later is void, and is dropped now, with nothing more on the wire\nfn __wvoid(s: i64, r: i64, sent: i64)\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 8\n    off: u1 = cmp.ne bm, 0\n    if off\n        ret\n    ap: ptr = addr __wa\n    h: i64 = load ap, 96\n    n: i64 = load ap, 104\n    p1: ptr = addr __wk_t\n    p2: ptr = addr __wk_r\n    p3: ptr = addr __wk_e\n    p4: ptr = addr __wk_kind\n    p5: ptr = addr __wk_a\n    p6: ptr = addr __wk_b\n    left: i64 = loop(i: i64 = 0, o: i64 = 0) bound {kept1}\n        done: u1 = cmp.ge i, n\n        if done\n            break o\n        k0: i64 = add h, i\n        k1: i64 = rem k0, {kept}\n        ki: index = conv k1\n        t: i64 = load p1, ki, 8\n        q: i64 = load p2, ki, 8\n        e: i64 = load p3, ki, 8\n        kind: i64 = load p4, ki, 8\n        a: i64 = load p5, ki, 8\n        b: i64 = load p6, ki, 8\n        behind: u1 = cmp.lt e, sent\n        gt: u1 = cmp.gt t, s\n        eq: u1 = cmp.eq t, s\n        rge: u1 = cmp.ge q, r\n        tie: u1 = and eq, rge\n        after: u1 = or gt, tie\n        stale: u1 = and behind, after\n        i2: i64 = add i, 1\n        if stale\n            __wdropped(kind, a, b, t)\n            continue i2, o\n        j0: i64 = add h, o\n        j1: i64 = rem j0, {kept}\n        ji: index = conv j1\n        store t, p1, ji, 8\n        store q, p2, ji, 8\n        store e, p3, ji, 8\n        store kind, p4, ji, 8\n        store a, p5, ji, 8\n        store b, p6, ji, 8\n        o2: i64 = add o, 1\n        continue i2, o2\n    store left, ap, 104\n    ret\n", sends = SENDS, kept = KEPT, kept1 = KEPT + 1));
    t.push_str(&format!("\n; the piece the screen has kept longest is shown: the screen's clock moves up to its time and marks its output there, and it goes into the capture, where `__shown` writes down when by the world's clock\nfn __wshow()\n    ap: ptr = addr __wa\n    h: i64 = load ap, 96\n    n: i64 = load ap, 104\n    hi: index = conv h\n    p1: ptr = addr __wk_t\n    t: i64 = load p1, hi, 8\n    p4: ptr = addr __wk_kind\n    kind: i64 = load p4, hi, 8\n    p5: ptr = addr __wk_a\n    a: i64 = load p5, hi, 8\n    p6: ptr = addr __wk_b\n    b: i64 = load p6, hi, 8\n    h1: i64 = add h, 1\n    h2: i64 = rem h1, {kept}\n    n2: i64 = sub n, 1\n    store h2, ap, 96\n    store n2, ap, 104\n    __wclock(t)\n{ch}{block}    ret\n\n; something that is not text has reached the screen from far, for time s at rank r: every piece kept that is for then or earlier is shown first, being before it in the order of far's turns\nfn __wflush(s: i64, r: i64)\n    ap: ptr = addr __wa\n    p1: ptr = addr __wk_t\n    p2: ptr = addr __wk_r\n    loop(i: i64 = 0) bound {kept1}\n        n: i64 = load ap, 104\n        some: u1 = cmp.gt n, 0\n        if some\n        else\n            break\n        h: i64 = load ap, 96\n        hi: index = conv h\n        t: i64 = load p1, hi, 8\n        q: i64 = load p2, hi, 8\n        lt: u1 = cmp.lt t, s\n        eq: u1 = cmp.eq t, s\n        rle: u1 = cmp.le q, r\n        tie: u1 = and eq, rle\n        due: u1 = or lt, tie\n        if due\n        else\n            break\n        __wshow()\n        i2: i64 = add i, 1\n        continue i2\n    ret\n", kept = KEPT, kept1 = KEPT + 1, ch = if w.writes.0 { "    is1: u1 = cmp.eq kind, 1\n    if is1\n        ch: u8 = conv a\n        __cap_ch(ch)\n" } else { "" }, block = if w.writes.1 { "    is2: u1 = cmp.eq kind, 2\n    if is2\n        ao: index = conv a\n        bk: index = conv b\n        tp: ptr = addr __wq_text\n        qs: ptr(u8) = ptradd tp, ao\n        v: u8[] = pack qs, bk, 1\n        __cap_block(v)\n" } else { "" }));
    t
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
            // (every unit at the screen, which a move may go to)
            if name == "none" {
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
                // (where the wire takes time the list and the pool run
                // at both places, and what they read is read far too)
                let written = (0..n).any(|e| self.at(e, far) == p && self.facts[e].writes.contains(name)) || self.wire && p == 1 && self.glue.writes.contains(name);
                let lives = (0..n).any(|e| self.at(e, far) != p && (self.facts[e].writes.contains(name) || self.facts[e].reads.contains(name))) || self.wire && p == 0 && (self.glue.writes.contains(name) || self.glue.reads.contains(name));
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
        if let Some(why) = self.refused_wire(far) {
            return Some(why);
        }
        for c in &self.crossings {
            if self.at(c.from, far) == self.at(c.to, far) || c.to == self.screen() {
                continue;
            }
            // (an array or a string is copied at the crossing, fm3 log
            // 267: what is left is a stream, which is its storage)
            let values = self.copied.get(&c.callee);
            if let Some(ty) = self.sigs.get(&c.callee).and_then(|s| s.iter().enumerate().find(|(i, t)| (t.contains('$') || t.contains('[') || t.contains("ptr")) && !values.is_some_and(|v| v.contains(i))).map(|(_, t)| t)) {
                let (far_one, other) = if far.contains(&c.to) { (c.to, c.from) } else { (c.from, c.to) };
                return Some(format!("{} cannot run apart from {} yet: {} is handed, or gives, {}, which is an address in the memory of the place it was made at", self.who(far_one), self.who(other), self.who(c.to), if ty.contains('$') { "a stream" } else { address(ty) }));
            }
        }
        let (fields, read, pushed) = self.carries(far);
        for ((name, ty, plain), c) in self.fields.iter().zip(&fields) {
            if (c[0] || c[1]) && !plain && !ty.ends_with('$') {
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

    /// Why a cut cannot be run over a wire that takes time (fm3 log
    /// 269): nothing may wait on the machine's stack for what crosses,
    /// so a function the rewrite left on the stack runs at the place
    /// of what calls it; an item carries words; and a push into a
    /// stored stream is an item only where it is of one word
    fn refused_wire(&self, far: &BTreeSet<usize>) -> Option<String> {
        if !self.wire {
            return None;
        }
        for c in &self.crossings {
            if c.to == self.screen() {
                continue;
            }
            if c.from == self.screen() {
                if matches!(c.caller.as_str(), "__run" | "__act_go") || self.at(c.to, far) == 0 {
                    continue;
                }
                return Some(format!("{} cannot run far over a wire that takes time yet: its function `{}` is called by `{}`, the screen's own, which waits for it on the machine's stack, the function being handed, or giving, more than a word", self.who(c.to), c.callee, c.caller));
            }
            if self.at(c.from, far) != self.at(c.to, far) {
                let (far_one, other) = if far.contains(&c.to) { (c.to, c.from) } else { (c.from, c.to) };
                return Some(format!("{} cannot run apart from {} over a wire that takes time yet: `{}` calls `{}` and waits for it on the machine's stack, not being left at its calls yet: it is handed, gives or keeps more than a word, or has a call where no place is held (fm3 question 139)", self.who(far_one), self.who(other), c.caller, c.callee));
            }
        }
        // two functions partway at two places and due at one time: the
        // order they were started in is known to neither (fm3 question 155)
        if let (Some(&a), Some(&b)) = (self.stepping.iter().find(|u| !far.contains(u)), self.stepping.iter().find(|u| far.contains(u))) {
            return Some(format!("{} and {} each step at a rate and would be at different places: of two functions partway at two places and due at one time, which was started first is known to neither place yet (fm3 question 155)", self.who(b), self.who(a)));
        }
        let (fields, read, pushed) = self.carries(far);
        for ((name, ty, plain), c) in self.fields.iter().zip(&fields) {
            if (c[0] || c[1]) && !(*plain && word(ty) && ty != "ptr") {
                return Some(format!("'{}{}' is written at one place and read at the other, and over a wire that takes time an item carries a number or a bit and nothing wider yet", name.trim_start_matches("__"), if ty.ends_with('$') { "$" } else { "" }));
            }
        }
        for &r in &self.unwired {
            if (0..2).any(|p| pushed[r][p] && read[r][1 - p]) {
                return Some(format!("'{}$' is pushed into at one place and read at the other, and over a wire that takes time such a push is an item only where it is of one word", self.rings[r].0.first().cloned().unwrap_or_default()));
            }
        }
        None
    }

    /// what the continuation of this number is, to a person: the unit
    /// it is of, or a case's own call
    pub fn continued(&self, id: i64) -> String {
        match self.conts.iter().find(|(k, _)| *k == id) {
            Some((_, Some(u))) => format!("`{}`", self.units[*u].shown()),
            _ => "a case's own call".to_string(),
        }
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

    /// What a move from one cut to another hands across, once (fm3
    /// cut.md, "the cut, moved"; log 265). The wire's carrying is lazy,
    /// a field copied as the wire is crossed, and the move is made at
    /// the screen. So a field goes from far to the screen where no
    /// unit at the screen wrote it or was being given it before and one
    /// uses it there after; and from the screen to far where a unit at
    /// the screen wrote it before, one at far uses it after, and none
    /// at the screen writes it after, when the wire would carry it at
    /// the next crossing anyway. A stored stream's storage goes to a
    /// place where it is read after and was not before, a stream being
    /// whole only where it is read; and the place of a reader that
    /// takes goes with its unit
    pub fn hands(&self, a: &BTreeSet<usize>, b: &BTreeSet<usize>) -> Vec<Hand> {
        let n = self.facts.len();
        let w = |name: &str, cut: &BTreeSet<usize>, p: usize| (0..n).any(|e| self.at(e, cut) == p && self.facts[e].writes.contains(name));
        let u = |name: &str, cut: &BTreeSet<usize>, p: usize| (0..n).any(|e| self.at(e, cut) == p && (self.facts[e].writes.contains(name) || self.facts[e].reads.contains(name)));
        let mut out = Vec::new();
        for (j, (name, _, _)) in self.hand_fields.iter().enumerate() {
            let dir = if !w(name, a, 0) && !u(name, a, 0) && u(name, b, 0) {
                2
            } else if w(name, a, 0) && u(name, b, 1) && !w(name, b, 0) {
                1
            } else {
                continue;
            };
            out.push(Hand { at: j, dir, what: self.field_said(name), name: name.clone() });
        }
        let (_, read_a, _) = self.carries(a);
        let (_, read_b, _) = self.carries(b);
        for (r, (names, _)) in self.rings.iter().enumerate().filter(|(_, (n, _))| !n.is_empty()) {
            let at = self.ring_at[r].unwrap_or(usize::MAX);
            for q in 0..2 {
                if read_b[r][q] && !read_a[r][q] && read_a[r][1 - q] {
                    out.push(Hand { at, dir: if q == 1 { 1 } else { 2 }, what: format!("the items of {}$ that {} still holds, with their times, and its counts", names[0], if q == 1 { "the screen" } else { "far" }), name: names[0].clone() });
                }
            }
            for (m, field) in names.iter().enumerate() {
                let takes = |cut: &BTreeSet<usize>, p: usize| (0..n).any(|e| self.at(e, cut) == p && self.facts[e].taken.contains(field));
                for q in 0..2 {
                    if takes(b, q) && !takes(a, q) && takes(a, 1 - q) {
                        out.push(Hand { at: at.saturating_add(1 + m), dir: if q == 1 { 1 } else { 2 }, what: format!("where the reader of {}$ has read to{}", names[0], if *field == names[0] { String::new() } else { format!(" ({})", field) }), name: field.clone() });
                    }
                }
            }
        }
        out
    }

    /// Why a move cannot be made (fm3 log 265): one of its cuts is
    /// itself refused; a unit that moves may be on the machine's stack
    /// when the move is due; or something it must hand across holds an
    /// address, or is a stream the rewrite does not know how to hand
    pub fn move_refused(&self, a: &BTreeSet<usize>, b: &BTreeSet<usize>) -> Option<String> {
        if let Some(why) = self.refused(a).or_else(|| self.refused(b)) {
            return Some(why);
        }
        if let Some((u, f)) = self.held.iter().find(|(u, _)| a.contains(u) != b.contains(u)) {
            return Some(format!("{} cannot be moved while it runs: its function `{}` keeps the machine's stack across a step of a rate, in a shape the compiler does not yet leave at its steps (fm3 question 139), so it may be partway when the move is due", self.who(*u), f));
        }
        for h in self.hands(a, b) {
            if let Some((_, ty, false)) = self.hand_fields.get(h.at).filter(|(n, ty, _)| *n == h.name && !ty.ends_with('$')) {
                return Some(format!("'{}{}' would be handed across at the move, and it holds {}, which is an address in the memory of the place it was made at", h.name.trim_start_matches("__"), if ty.ends_with('$') { "$" } else { "" }, address(ty)));
            }
            if h.at >= self.sizes.1 {
                return Some(format!("'{}$' would be handed across at the move, and how it is kept is not one the move knows how to hand yet", h.name));
            }
        }
        None
    }

    /// A cut and its moves: the calls that fill the table, and after
    /// them, for each move, when it is due, the table of the cut it
    /// goes to and what it hands across
    pub fn moves(&self, first: &BTreeSet<usize>, then: &[(BTreeSet<usize>, i64)]) -> Cut {
        self.moves_but(first, then, None)
    }

    /// ... and the same with one thing deliberately not handed across,
    /// which only a test asks for: the proof that a move hands across
    /// what it must is that the program then goes wrong
    pub fn moves_but(&self, first: &BTreeSet<usize>, then: &[(BTreeSet<usize>, i64)], dropped: Option<&str>) -> Cut {
        let Cut { mut before, mut label } = self.cut(first);
        let mut was = first;
        for (k, (next, t)) in then.iter().enumerate() {
            let k = k as i64;
            before.push(("__move_at".to_string(), vec![k, *t]));
            before.extend(self.cut(next).before.into_iter().map(|(_, args)| ("__move_set".to_string(), vec![k, args[0], args[1]])));
            before.extend(self.hands(was, next).into_iter().filter(|h| dropped != Some(h.name.as_str())).map(|h| ("__hand_set".to_string(), vec![k, h.at as i64, h.dir as i64])));
            label.push_str(&format!(", then {} at {}", self.label(next).trim_start_matches("far: "), super::store::spell_time(*t)));
            was = next;
        }
        Cut { before, label }
    }

    /// The placement of a cut that moves: the first cut's table, and
    /// for each move when it is, which units change place, what is
    /// handed across, read off the same facts that fill the move's
    /// table, and the table of the cut it goes to
    pub fn table_moved(&self, store: &str, first: &BTreeSet<usize>, then: &[(BTreeSet<usize>, i64)], cases: &[String]) -> String {
        let place = |p: usize| if p == 0 { "screen" } else { "far" };
        let mut out = self.table(store, first, cases);
        let mut was = first;
        for (next, t) in then {
            out.push_str(&format!("\nat {} the cut moves: {}, the first thing due at that time\n\n", super::store::spell_time(*t), self.label(next)));
            if let Some(why) = self.move_refused(was, next) {
                out.push_str(&format!("refused: {}\n", why));
                return out;
            }
            let w1 = self.units.iter().map(|u| u.name.len()).max().unwrap_or(4).max(4);
            out.push_str(&format!("{:<w1$}  {:<6}  {}\n", "unit", "was", "is", w1 = w1));
            for (u, unit) in self.units.iter().enumerate().filter(|(u, _)| was.contains(u) != next.contains(u)) {
                out.push_str(&format!("{:<w1$}  {:<6}  {}\n", unit.name, place(self.at(u, was)), place(self.at(u, next)), w1 = w1));
            }
            let mut rows: Vec<(String, String, String, String)> = vec![("the table".to_string(), format!("{} bytes of the cut", self.sizes.0), String::new(), "rewritten in the move's turn: where each unit runs, and what the wire carries from then on".to_string())];
            let hands = self.hands(was, next);
            for h in &hands {
                let kind = if self.rings.iter().any(|(n, _)| n.contains(&h.name)) { "a stream" } else if h.name.starts_with("__enabled_") || h.name.starts_with("__on_") { "a switch" } else if h.name.starts_with("__") { "kept" } else { "an item" };
                rows.push((kind.to_string(), h.what.clone(), if h.dir == 1 { "screen → far" } else { "far → screen" }.to_string(), "copied once, in the move's turn".to_string()));
            }
            if hands.is_empty() {
                rows.push(("nothing else".to_string(), "no field of the context, and no stored stream".to_string(), String::new(), "none that a unit that moves reads or writes is at the place it leaves and not at the place it goes to".to_string()));
            }
            let partway: Vec<String> = self.units.iter().enumerate().filter(|(u, _)| self.left.contains(u)).map(|(_, unit)| format!("`{}`", unit.shown())).collect::<BTreeSet<_>>().into_iter().collect();
            if !partway.is_empty() {
                rows.push(("not handed".to_string(), format!("a function partway: {}", partway.join(", ")), String::new(), format!("its record is a place of the pool, {} columns, which is the screen's between its turns: when it is next due, where it goes on from, what it keeps", self.pool.len())));
            }
            let w: Vec<usize> = (0..3).map(|k| rows.iter().map(|r| [&r.0, &r.1, &r.2][k].chars().count()).max().unwrap_or(0)).collect();
            out.push_str("\nhanded across at the move\n");
            for (a, b, c, d) in &rows {
                let padded = |s: &str, n: usize| format!("{}{}", s, " ".repeat(n.saturating_sub(s.chars().count())));
                out.push_str(&format!("{}  {}  {}  {}\n", padded(a, w[0]), padded(b, w[1]), padded(c, w[2]), d));
            }
            out.push('\n');
            out.push_str(&self.table(store, next, cases));
            was = next;
        }
        out
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
        // (of the screen's own, a case's twin is the case's call, and
        // the pool's turn is the list's)
        let from = |c: &Crossing| if c.from != self.screen() { name(c.from) } else if c.caller.starts_with("__whole_") || c.caller.starts_with("__said_") { "a case's own".to_string() } else if matches!(c.caller.as_str(), "__act_go" | "__turns" | "__close") { "the list".to_string() } else { name(c.from) };
        for c in &self.crossings {
            let (a, b) = (self.at(c.from, far), self.at(c.to, far));
            if a == b {
                continue;
            }
            // (an array or a string is copied at the crossing, fm3 log 267)
            let values = self.copied.get(&c.callee);
            let types: Vec<String> = self.sigs.get(&c.callee).cloned().unwrap_or_default().into_iter().enumerate().map(|(i, t)| if !values.is_some_and(|v| v.contains(&i)) { t } else if t == "u8$" { "a string, copied".to_string() } else { format!("an array of `{}`, copied", t.trim_end_matches('$')) }).collect();
            let (kind, what) = if c.to == self.screen() {
                match c.callee.as_str() {
                    "__out_ch" | "__out_block" if self.wire => ("text".to_string(), "characters for out$, with their time: shown at the later of it and when they arrive".to_string()),
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
            rows.push((kind, format!("{} → {}", from(c), name(c.to)), format!("{} → {}", place(a), place(b)), what));
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
                    rows.push((kind.to_string(), self.field_said(fname), format!("{} → {}", place(p), place(1 - p)), if self.wire { "a field of the context, sent with a function partway or a telling where it has changed" } else { "a field of the context, copied as the wire is crossed" }.to_string()));
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
