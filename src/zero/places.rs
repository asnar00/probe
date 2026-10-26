//! The placed build (fm3 cut.md, log 261): one unchanged program at two
//! places in one process. `client` is the store's first context and has
//! the devices, the output, the input and the list of things going on;
//! `server` is its second and has none. Each has memory of its own. A cut
//! says, for each unit, a function or a line that stands, which of the
//! two it runs at, and the cut is data: a table, `__cut`, that the
//! runner fills before a case, so one compiled program runs at every
//! cut.
//!
//! This file rewrites the finished text, as `turns.rs` does. A unit's
//! function asks the wire for its own place where it begins and for the
//! place it was called from where it returns. The platform's writes ask
//! for the client. A push into a stored stream is made again at the
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
const OWN_LIST: [&str; 13] = ["__act_n", "__due_act", "__act_i", "__act_b", "__in_n", "__in_k", "__in_t", "__in_c", "__due_in", "__in_busy", "__due_close", "__due_move", "__move_k"];

/// how many of the wire's items its record holds, for a person
const WIRE_LOG: usize = 1024;

/// Working ahead of a key (fm3 cut.md, "working ahead of a key"; log
/// 273): how many copies of itself the server keeps, one before each turn it
/// has taken for a time the client has not told it of; how many places
/// of its half of the pool and how many bytes of its streams a copy
/// holds; how many pieces of text the client keeps until their time;
/// and how many of its own sendings the client remembers the times of.
/// Past the first three the server does not work ahead, and is late, not wrong
pub const AHEAD: usize = 16;
const AHEAD_ROWS: usize = 16;
const AHEAD_MEM: usize = 8192;
const KEPT: usize = 512;
const SENDS: usize = 256;
/// the client's log of what it sent the server, items (fm3 log 292)
const AGAIN_LOG: usize = 256;
/// a rank below every rank but a move's own: a move's (fm3 log 278)
const RANK: i64 = 1 << 44;

/// how many times a wire's delay may change in one case (fm3 log 289)
pub const CHANGES: usize = 8;
/// the head every item of a wire with a speed carries, in bytes (fm3
/// log 294): its kind and flags, its own time, its counts
pub const PACE_HEAD: usize = 16;

/// the functions of a build whose wire takes time that run at
/// whichever place the world is giving a turn: what they read of the
/// context is read at both
fn both(name: &str) -> bool {
    matches!(name, "__list_next" | "__list_turn" | "__act_run" | "__act_next" | "__act_turn" | "__act_slot" | "__act_go" | "__run") || name.starts_with("__act_start_")
}

/// the words that put something into a stream's storage, or end it
const PUSHES: [&str; 7] = ["push_queue", "push_queue_open", "push_queue_few", "push", "__push", "end", "open"];

/// the platform's functions that are the client's and may be called
/// from either place: the two writes of the output device, the input's
/// push, the list of things going on, the scheduler of the lines that
/// walk a stored stream, and the start of a function left at its steps
fn clients(name: &str) -> bool {
    matches!(name, "__out_ch" | "__out_block" | "__in_ch" | "__turns" | "__run" | "__run_copy") || name.starts_with("__act_start_")
}

/// the runner's own entries, called at the client
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
    /// what each entity does: a row a unit, and a last row the client's
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
    /// where the wire takes time, the word of each clock, when it is
    /// next due, and the unit its line is: the word is the line's
    /// place's, and a move hands it across with the line (fm3 log 276)
    clock_dues: Vec<(String, usize)>,
    /// the types that are another type's name, as an enumeration is a
    /// `u8`: a field of one is a word (fm3 log 284)
    named: HashMap<String, String>,
    /// where the wire takes time, why each function still on the
    /// machine's stack is (fm3 log 280)
    pub stack: std::collections::BTreeMap<String, super::turns::Why>,
    /// whose each stream is that has a scope word (hop fifty-eight): the
    /// field and `user`, `group` or `global`
    pub homes: Vec<(String, String)>,
    /// a push into a group's or everyone's stored stream is stamped by
    /// its home, and the record may say so (`__push_home`)
    pub stamps: bool,
    /// (hop fifty-nine) a pair of contexts a person: each person a client
    /// of their own, and the server a context for each person's part
    pub pairs: bool,
    /// where the wire takes time, the fields an item may carry, each with
    /// its bit's place, its name and its type; and the group's and
    /// everyone's streams kept as one word, each with its place among
    /// `homes` (hop sixty: what the real wire's harness reads and writes)
    carried: Vec<(usize, String, String)>,
    home_words: Vec<(usize, String, String)>,
}

/// one thing a move hands across: its byte in the move's table, the
/// direction, 1 from the client to the server and 2 from the server to the client,
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
    /// `the server: count down`, for a case's line
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
pub fn rewrite(ir: &str, units: &[Unit], wire: bool, changes: bool, choose: bool, paced: bool) -> Result<(String, Placed), String> {
    let (lines, fns) = parse(ir);
    let present: HashSet<&str> = fns.iter().map(|f| f.name.as_str()).collect();
    // (hop fifty-nine) a store built for people over wires that take no
    // time has a pair of contexts a person, their client and the
    // server's context for their part: the wire is between the two of
    // the current person, `__zero_place`, and which place a context is
    // is told from the person's first, `__pair_base`
    let pairs = lines.iter().any(|l| l.starts_with("data __share: "));
    let zc = if pairs { "__zero_place" } else { "__zero_context" };
    if !lines.iter().any(|l| l.starts_with("data __ctx_mem: array(__ctx, 2)")) && !pairs {
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
    let client = nu;
    let mut owner: HashMap<String, usize> = HashMap::new();
    for f in &fns {
        if machine.contains(f.name.as_str()) {
            continue;
        }
        let b = base(&f.name);
        if let Some(u) = units.iter().position(|u| u.irs.contains(&b)) {
            owner.insert(f.name.clone(), u);
        } else if clients(&f.name) || runners(&f.name) {
            owner.insert(f.name.clone(), client);
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
    // (a type that is another's name, `type kind = u8`)
    let named: HashMap<String, String> = lines.iter().filter_map(|l| l.strip_prefix("type ")).filter_map(|l| l.split_once(" = ")).filter(|(_, d)| *d != "struct").map(|(n, d)| (n.to_string(), d.to_string())).collect();
    let mut carried_types: BTreeSet<String> = BTreeSet::new();
    // (a record that arrives has its values copied, fm3 log 284:
    // `__act_land`, written with the pool, calls the copy of each type)
    let lands = present.contains("__act_land");
    carried_types.extend(lines.iter().flat_map(|l| called(l)).filter_map(|c| c.strip_prefix("__copied_")).map(str::to_string));
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
    // whose each stream is (hop fifty-eight), from the context's comment
    // rows, `;   all: group (counter), ...`: a field a user's, a group's or
    // everyone's. The group's and everyone's have a home, the server
    // where the cut puts anything there, and the client a copy; a user's
    // has one device this hop and is placed as the device's own is
    let homes: Vec<(String, String)> = lines.iter().filter_map(|l| l.strip_prefix(";   ")).filter_map(|l| l.split_once(": ")).filter_map(|(n, rest)| {
        let w = rest.split(' ').next()?;
        (matches!(w, "user" | "group" | "global") && !n.starts_with("__") && ctx_fields.iter().any(|(f, _)| f == n)).then(|| (n.to_string(), w.to_string()))
    }).collect();
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
    // (where each person is a pair, a reader of a shared stream made for a
    // person after the first starts where the stream starts, `__zero_new`:
    // the same storage)
    for f in fns.iter().filter(|f| !(pairs && matches!(f.name.as_str(), "__share_out" | "__share_in" | "__zero_new"))) {
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
        // (hop fifty-nine) the copies of a person's shared streams taken
        // and written back where the current context changes are the
        // layout's, as the context's own output is: no line's read or push
        let layout = pairs && matches!(f.name.as_str(), "__share_out" | "__share_in" | "__zero_new");
        let facts = if layout { Facts { calls: reader.facts(f).calls, ..Facts::default() } } else { reader.facts(f) };
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
    // (the word of each clock is its line's: a move hands it across
    // with the line, fm3 log 276)
    let mut clock_dues: Vec<(String, usize)> = Vec::new();
    if let Some(f) = fns.iter().find(|f| wire && f.name == "__list_next") {
        for pair in f.body.windows(2) {
            let (Some((_, _, a)), Some((_, _, b))) = (def(pair[0].trim()), def(pair[1].trim())) else { continue };
            let (Some(due), Some(g)) = (a.strip_prefix("get x, ").filter(|d| dues.contains(*d)), b.strip_prefix("__here__").and_then(|g| g.strip_suffix("()"))) else { continue };
            if let Some(u) = units.iter().position(|u| u.irs.iter().any(|i| i == g)) {
                clock_dues.push((due.to_string(), u));
            }
        }
    }
    // what each entity does: its own functions and every function of
    // nobody's that they reach, which runs where it is called; and a
    // function nothing reaches is the runner's to call, at the client
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
        owner.insert(f.name.clone(), client);
        walk(client, vec![f.name.clone()], &owner, &mut facts, &mut reached);
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
        let homed = homes.iter().any(|(n, w)| n == name && w != "user");
        if writers > 0 && (users > 1 || homed || wire && (glue.reads.contains(name) || glue.writes.contains(name))) {
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
            if OWN.contains(&name.as_str()) || reader.handles.contains_key(name) || fields.iter().any(|(n, _, _)| n == name) || wire && OWN_LIST.contains(&name.as_str()) {
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
    // (a store with an input that arrives at a time has a bound the server
    // may work ahead of, fm3 log 273: only its text has what that needs)
    // (and where the runner chooses the cut, fm3 log 290: a move may come at any time, as a key may)
    let ahead = wire && (choose || fns.iter().any(|f| f.name == "__in_turn"));
    let ncols = lines.iter().filter(|l| l.starts_with("data __act_")).count();

    // (where the wire takes time: the stored streams a push into
    // which is no word, and what an arriving push does)
    let mut unwired: BTreeSet<usize> = BTreeSet::new();
    // (hop fifty-eight) a push into a group's or everyone's stored stream
    // is stamped by its home: such a push is written `__push_home`
    let homed_ring = |r: usize| rings[r].0.iter().any(|f| homes.iter().any(|(n, w)| n == f && w != "user"));
    let mut stamps = false;
    let mut arrivals = String::new();
    // the text, a function at a time
    let mut out = String::new();
    let mut n = 0;
    let mut k = 0;
    while k < lines.len() {
        let Some(f) = fns.iter().find(|f| f.at.0 == k) else {
            if lines[k].starts_with("data __ctx_mem: ") {
                out.push_str(&format!("; the cut (fm3 cut.md): where each unit runs, 0 the client and 1 the server, a byte each, {} of them; then for each field of the context the wire may carry, whether it is carried from the client and whether from the server; then for each stored stream, at each place, whether a push there is left out, the stream being read only at the other, and whether it is made again at the other. The runner fills it before a case; all zero, every unit is at the client and nothing crosses\ndata __cut: array(u8, {})\n", nu, size));
                if wire {
                    out.push_str(&format!("; the world and the wire (fm3 cut.md, \"a wire that takes time\"; log 269). `__w` is the world's: its clock, the wire's delay, what the runner asked to read back, and for each place what it has sent, had answered and been sent, what it was last told and what it last told; then where each way of the wire begins and ends. An item of the wire is a row: when it arrives by the world's clock, what it is (1 a character and 2 a block of text for the output, 3 a push into a stored stream, 4 a function partway, 5 a telling), two words of its own, which fields of the context it carries and how much of the other's sending it answers, and its own time; {q} rows each way. A function partway in flight has a row of the pool. `__wq_ctx` is the sender's context as it was, from which the fields an item carries are read; `__wshadow` what each place last sent or was sent of each; `__wq_text` the blocks of text in flight; `__wlog` the wire's record for a person, six words an item. When each character of the capture was shown is kept in the server's half of the marks, `__out_t`, from its second word: the server has no output of its own, and its first word is where its clock marks\ndata __w: array(i64, 32)\ndata __wq_at: array(i64, {q2})\ndata __wq_kind: array(i64, {q2})\ndata __wq_a: array(i64, {q2})\ndata __wq_b: array(i64, {q2})\ndata __wq_c: array(i64, {q2})\ndata __wq_e: array(i64, {q2})\ndata __wq_t: array(i64, {q2})\ndata __wq_ctx: array(__ctx, {q2})\ndata __wshadow: array(__ctx, 2)\ndata __wq_text: array(u8, 65536)\ndata __wlog: array(i64, {log})\n", q = WIRE_Q, q2 = 2 * WIRE_Q, log = 6 * WIRE_LOG));
                    if choose {
                        out.push_str("; the chooser (fm3 cut.md, \"the cut chosen\"; log 290), the client's: the bound, how many round trips of its own it has begun, when the last was sent, whether it is out, since when every round trip has come back within the bound, the last round trip, which cut it holds (0 the one given, 1 nothing on the server), how many moves it has made, the longest round trip, and how many have come back; then what it measures of the wire (fm3 log 295): the speed it last saw, 0 for none, when the burst of what arrives now arrived, its bytes, whether it came one behind the burst before, when that one arrived, the least time an item has taken from its sending, when the burst's first item was sent, when the speed was last seen, when the last answer to a ping arrived and its number, since when the wire has been gone, how many gaps it has seen, since when the cut given has been held and the bytes put on the wire and arrived from the server meanwhile, the rate the cut given put on the wire, and its last eight gaps, each from when to when\ndata __wc: array(i64, 48)\n");
                        out.push_str(&format!("; what each item of the wire weighs and when its sender put it on, for the chooser's measures (fm3 log 295)\ndata __wq_sz: array(i64, {q2})\ndata __wq_s: array(i64, {q2})\n", q2 = 2 * WIRE_Q));
                        // (the client running the server's part again, fm3 log 292)
                        out.push_str(&format!("; the client running the server's part again (fm3 cut.md, \"the client runs the server's part again\"; log 292). `__wg` is its words: whether the client is running the server's part now, how many items it has logged, how many pieces of the server's text it has shown, how many of them the part run again has written, how many moves it has made by running the server's part (its cut number), the most the server has said it has heard of, the server's own count, how many pushes and functions partway it has taken from the server, how many of them the part run again has made, how many of those it had not had, the turns run again, the items dropped by their count, the pieces of the part run again it had not shown, whether the log ran out, how many pieces from the server it was keeping when it took the server's part, the row of the wire it runs through, and whether it holds the server's part so now. `__wg_cut` is the cut's table as it was given and `__wg_now` the table as it is while the client runs the server's part; `__wg_shadow` the server's notes of the fields at its start and as they are, and `__wg_ctx` the server's context at its start and as it is; their halves of the pool and parts of the arena are the two copies after working ahead's. `__wq_n` is the count of moves made by running the server's part that the sender of an item had heard of. The log, `__wl_*`, is an item a row: what it is, its two words, its own time and rank, which fields it carried, the context it carried them from and the record it carried, {l} rows\ndata __wg: array(i64, 32)\ndata __wg_cut: array(u8, {size})\ndata __wg_now: array(u8, {size})\ndata __wg_shadow: array(__ctx, 2)\ndata __wg_ctx: array(__ctx, 2)\ndata __wq_n: array(i64, {q2})\ndata __wl_kind: array(i64, {l})\ndata __wl_a: array(i64, {l})\ndata __wl_b: array(i64, {l})\ndata __wl_t: array(i64, {l})\ndata __wl_r: array(i64, {l})\ndata __wl_c: array(i64, {l})\ndata __wl_ctx: array(__ctx, {l})\n{lp}", l = AGAIN_LOG, size = size, q2 = 2 * WIRE_Q, lp = if pooled { format!("data __wl_pool: array(i64, {})\n", AGAIN_LOG * ncols) } else { String::new() }));
                    }
                    if changes {
                        out.push_str(&format!("; a wire whose delay changes (fm3 cut.md, \"the cut chosen\"; log 289): how many changes, and then each as the time it takes effect and the delay from then, a gone wire's the never-time; {m} at most\ndata __wsch: array(i64, {n})\n", m = CHANGES, n = 1 + 2 * CHANGES));
                        if paced {
                            out.push_str("; a wire with a speed, and one that comes and goes (fm3 cut.md, \"the cut chosen by the wire's speed and reliability\"; log 294): the speed in bytes a second at first and after each change, 0 for as fast as the wire can be; when the coming and going begins, its period and how long each gap is; and for each way, when it is next free, the kind of what was put on it last, its own time and the moment it was put, and where the piece of text being put on began, in the wire and in the record\ndata __wsp: array(i64, 32)\n");
                        }
                    }
                }
                if ahead {
                    // (two copies more where the runner chooses: the server's start, and the server's real state while the client runs the server's part, fm3 log 292)
                    let slots = if choose { AHEAD + 2 } else { AHEAD };
                    out.push_str(&format!("; working ahead of a key (fm3 cut.md, \"working ahead of a key\"; log 273). The server takes turns for times the client has not told it of, and before each keeps a copy of itself: `__wa_t` and `__wa_r` the time and rank of that turn, `__wa_ctx` its context, `__wa_pool` its half of the pool, {rows} places a column, `__wa_mem` the part of the arena its streams were made in; {n} copies, the oldest first. `__wa` is the words of it: whether the server is in a turn taken ahead, whether that turn would have sent more than text, whether it left a function partway here that would have crossed, the turn that may not be tried again until something reaches the server, where the copies begin and how many there are, how many turns the server took and threw away, the rank of the turn each place is in, where the client's kept pieces begin and how many there are, and where the server's part of the arena is and how long. An item of the wire has a column more, `__wq_r`, the rank of the turn that sent it. The client keeps text until its time: `__wk_*`, a row a piece, its time and rank, how much of the client's sending the server had had when it wrote it, and what it is; and `__ws_t` and `__ws_r` are the time and rank of each thing the client has sent\ndata __wa: array(i64, 32)\ndata __wa_t: array(i64, {n})\ndata __wa_r: array(i64, {n})\ndata __wa_ctx: array(__ctx, {n})\ndata __wa_keep: array(__ctx, 1)\ndata __wa_pool: array(i64, {pool})\ndata __wa_mem: array(u8, {mem})\ndata __wq_r: array(i64, {q2})\ndata __wk_t: array(i64, {k})\ndata __wk_r: array(i64, {k})\ndata __wk_e: array(i64, {k})\ndata __wk_kind: array(i64, {k})\ndata __wk_a: array(i64, {k})\ndata __wk_b: array(i64, {k})\ndata __ws_t: array(i64, {s})\ndata __ws_r: array(i64, {s})\n", n = AHEAD, rows = AHEAD_ROWS, pool = (slots * AHEAD_ROWS * ncols).max(1), mem = slots * AHEAD_MEM, q2 = 2 * WIRE_Q, k = KEPT, s = SENDS));
                }
                if moved {
                    out.push_str(&format!("; the cut's moves (fm3 cut.md, log 265), {m} at most: the time each is due; the table of the cut after each, as `__cut` is laid out; and what each hands across, a byte for each field of the context that some move may ({nf} of them: the wire's, and those only one unit touches) and then for each stored stream one for its storage and one for the place of each field that reads it: 0 nothing, 1 from the client to the server, 2 from the server to the client. The runner fills them before a case\ndata __move_t: array(i64, {mt})\ndata __cuts: array(u8, {cs})\ndata __hands: array(u8, {hs})\n", m = MOVES, nf = hand_fields.len(), mt = MOVES + 1, cs = MOVES * size, hs = MOVES * hsize));
                }
            }
            out.push_str(&lines[k]);
            out.push('\n');
            k += 1;
            continue;
        };
        if moved && wire && f.name == "__move_turn" {
            // (over a wire that takes time, fm3 log 276: the client's
            // own bytes of the table, each unit that leaves it marked
            // nowhere, and the move's word put on the wire with the
            // functions partway that go with it)
            out.push_str(&format!("fn __move_turn()\n    c: ptr = context()\n    x: __ctx = load c\n    k: index = get x, __move_k\n    t: i64 = get x, __due_move\n    k2: index = add k, 1\n    tp: ptr = addr __move_t\n    d: i64 = load tp, k2, 8\n    x2: __ctx = set x, __move_k, k2\n    x3: __ctx = set x2, __due_move, d\n    store x3, c\n    cp: ptr = addr __cut\n    np: ptr = addr __cuts\n    base: index = mul k, {size}\n    loop(i: index = 0) bound {nu}\n        done: u1 = cmp.ge i, {nu}\n        if done\n            break\n        j: index = add base, i\n        nb: u8 = load np, j, 1\n        ob: u8 = load cp, i, 1\n{was}        to: u1 = cmp.eq nb, 1\n        goes: u1 = and was, to\n        if goes\n            store 2: u8, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n    loop(m: index = {nu}) bound {size}\n        done2: u1 = cmp.ge m, {size}\n        if done2\n            break\n        j2: index = add base, m\n        b2: u8 = load np, j2, 1\n        store b2, cp, m, 1\n        m2: index = add m, 2\n        continue m2\n    kw: i64 = conv k\n    row: index = __wput(10, kw, t, t)\n    __wstamp(row, t)\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    store x3, qp\n{log}{ship}{keep}    __wsent_move(t)\n    ret\n", size = size, nu = nu, q2 = 2 * WIRE_Q, ship = if pooled { "    __wship()\n" } else { "" }, keep = if choose { "    __wagain_keep(t)\n" } else { "" }, was = if choose { "        w0: u1 = cmp.eq ob, 0\n        w3: u1 = cmp.eq ob, 3\n        was: u1 = or w0, w3\n" } else { "        was: u1 = cmp.eq ob, 0\n" }, log = if choose { "    __wlogged(row)\n" } else { "" }));
            k = f.at.1;
            continue;
        }
        if moved && f.name == "__move_turn" {
            out.push_str(&format!("fn __move_turn()\n    c: ptr = context()\n    x: __ctx = load c\n    k: index = get x, __move_k\n    k2: index = add k, 1\n    tp: ptr = addr __move_t\n    d: i64 = load tp, k2, 8\n    x2: __ctx = set x, __move_k, k2\n    x3: __ctx = set x2, __due_move, d\n    store x3, c\n    cp: ptr = addr __cut\n    np: ptr = addr __cuts\n    base: index = mul k, {size}\n    loop(i: index = 0) bound {size}\n        done: u1 = cmp.ge i, {size}\n        if done\n            break\n        j: index = add base, i\n        b: u8 = load np, j, 1\n        store b, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n    __hand(k)\n    ret\n", size = size));
            k = f.at.1;
            continue;
        }
        let who = owner.get(&f.name).copied().filter(|_| !machine.contains(f.name.as_str()));
        let unit = who.filter(|&o| o < nu);
        let wrapped = !wire && (unit.is_some() || clients(&f.name));
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
            // the output's two writes are the client's capture, which
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
                // unit at the client
                // (where the server may work ahead, fm3 log 273: the part of the arena the server's context is made in is what a copy of its streams is)
                let (mark0, mark1) = if ahead { (format!("{p}__wab: ptr = addr __arena\n{p}__wu0: index = load __wab, 16\n", p = p), format!("{p}__wu1: index = load __wab, 16\n", p = p)) } else { (String::new(), String::new()) };
                out.push_str(&format!("{p}{zc}(1)\n{mark0}{p}__zero_new()\n{mark1}{p}{zc}(0)\n{p}__zero_new()\n{p}__cp: ptr = addr __cut\n{p}loop(__ci: index = 0) bound {size}\n{p}    __cd: u1 = cmp.ge __ci, {size}\n{p}    if __cd\n{p}        break\n{p}    store 0: u8, __cp, __ci, 1\n{p}    __ci2: index = add __ci, 1\n{p}    continue __ci2\n", p = p, size = size, mark0 = mark0, mark1 = mark1, zc = zc));
                if wire {
                    // ... the world's clock at nothing, the wire empty, nothing said, and every place of the pool free
                    out.push_str(&format!("{p}__wz: ptr = addr __w\n{p}loop(__wi: index = 0) bound 32\n{p}    __wd: u1 = cmp.ge __wi, 32\n{p}    if __wd\n{p}        break\n{p}    store 0: i64, __wz, __wi, 8\n{p}    __wj: index = add __wi, 1\n{p}    continue __wj\n{p}store -1: i64, __wz, 104\n{p}store -1: i64, __wz, 168\n{p}store -1: i64, __wz, 224\n{p}store -1: i64, __wz, 240\n", p = p));
                    if ahead {
                        // ... no copy kept, nothing barred, nothing kept at the client
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
                // (the home's time stamps an item of a group's or
                // everyone's stream, fm3 places.md point 3: at both
                // places the push is `__push_home`, which stamps the
                // later of the place's own time and the world's clock
                // where the place is the server)
                let homed = verb == "__push" && homed_ring(s.ring);
                stamps |= homed;
                let t = if homed { t.replacen("__push(", "__push_home(", 1) } else { t.to_string() };
                let t = t.as_str();
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
                out.push_str(&format!("{p}__ro{n}: u1 = __ring_here({r})\n{p}if __ro{n}\n{p}    {t}\n{p}__rc{n}: i64 = __ring_there({r})\n{p}__rk{n}: u1 = cmp.ne __rc{n}, 2\n{p}if __rk{n}\n{p}    __rp{n}: ptr = context()\n{p}    __rx{n}: __ctx = load __rp{n}\n{p}    __rs{n}: {ty} = get __rx{n}, {field}\n{p}    {again}\n{p}    __wire_back(__rc{n})\n", p = p, n = n, r = s.ring, t = t, ty = s.ty, field = s.field, again = again));
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
        out.push_str(&"fn __wire(to: i64) -> i64\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    from: i64 = conv second\n    same: u1 = cmp.eq from, to\n    if same\n        ret from\n    x: __ctx = load a\n    t: i64 = get x, __clock\n    k: i32 = conv to\n    ZC(k)\n    b: ptr = context()\n    y0: __ctx = load b\n    c: i64 = get y0, __clock\n    late: u1 = cmp.gt t, c\n".replace("ZC", zc));
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
            out.push_str(&format!("\n; ... and made again at the other place where the stream is read there: over the wire, giving the place left, or 2 where it is not\nfn __ring_there(r: index) -> i64\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: index = conv second\n    r4: index = mul r, 4\n    i0: index = add r4, {rbase2}\n    i: index = add i0, here\n    p: ptr = addr __cut\n    c: u8 = load p, i, 1\n    stays: u1 = cmp.eq c, 0\n    if stays\n        ret 2\n    hw: i64 = conv second\n    other: i64 = sub 1, hw\n    w: i64 = __wire(other)\n    ret w\n", rbase2 = rbase + 2));
        }
    }
    if pooled {
        let cols: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix("data __act_")).filter_map(|l| l.split(':').next()).collect();
        if wire {
            out.push_str("\n; one record of the pool, every column of it, from one place of it to another: a place of a half, or the row of an item in flight\nfn __act_carry(from: index, to: index)\n");
        } else {
        out.push_str(&format!("\n; a function left at its steps (fm3 log 232), gone on with at the other place: its record goes with its turn. The place in the pool it is handed is its caller's, `w` the place called from; its own is the same place of this context's half, and the record is copied there\nfn __act_in(a0: index, w: i64) -> index\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: i64 = conv second\n    same: u1 = cmp.eq here, w\n    if same\n        ret a0\n    hi: index = conv here\n    wi: index = conv w\n    hb: index = mul hi, {pool}\n    wb: index = mul wi, {pool}\n    n: index = sub a0, wb\n    l: index = add hb, n\n    __act_carry(a0, l)\n{land_in}    ret l\n\n; ... and copied back where its turn ends\nfn __act_out(a0: index, a: index)\n    same: u1 = cmp.eq a0, a\n    if same\n        ret\n    __act_carry(a, a0)\n{land_out}    ret\n\n; one record of the pool, every column of it, from one place to another\nfn __act_carry(from: index, to: index)\n", pool = POOL, land_in = if lands { "    __act_land(l)\n" } else { "" }, land_out = if lands { "    __act_land(a0)\n" } else { "" }));
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
        let mut used: BTreeSet<&'static str> = BTreeSet::new();
        if wire {
            // what a move hands across over a wire that takes time (fm3
            // log 276): taken at the place it reaches, from the sender's
            // context as it was when it sent, which the item keeps
            let mut h = format!("\n; what the k-th move hands across over a wire that takes time (fm3 cut.md, \"the cut moved over a wire that takes time\"; log 276), taken here, at the place it has reached, when its item arrives: `dir` is 1 for what the client sent with the move's word and 2 for what the server sent back. Each field of the context the move's table says, from the sender's context as it was when it sent, which the item's row keeps; and both places' note of what was last sent of it, so that it is not said again. Each stored stream's storage, read where it lies at the sender, which no longer reads it; and the place of each reader that moved\nfn __whand(row: index, k: index, dir: u8)\n    c: ptr = context()\n    y0: __ctx = load c\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    x: __ctx = load qp\n    hp: ptr = addr __hands\n    hb: index = mul k, {hsize}\n    __wnoted(row, k, dir)\n", q2 = 2 * WIRE_Q, hsize = hsize);
            // (... and both places' note of what was last sent of each field the wire carries, a function of its own: a context is wide, and one function with three of them a field is a frame too deep for a machine with a small stack)
            let mut note = format!("\n; both places' note of what was last sent of a field the wire carries, set to what a move handed of it\nfn __wnoted(row: index, k: index, dir: u8)\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    x: __ctx = load qp\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    z0: index = const 0\n    z1: index = const 1\n    e0: ptr(__ctx) = index sm, z0\n    e1: ptr(__ctx) = index sm, z1\n    p0: ptr = cast e0\n    p1: ptr = cast e1\n    s0: __ctx = load p0\n    t0: __ctx = load p1\n    hp: ptr = addr __hands\n    hb: index = mul k, {hsize}\n", q2 = 2 * WIRE_Q, hsize = hsize);
            let (mut c, mut sc) = (0, 0);
            for (j, (name, ty, plain)) in hand_fields.iter().enumerate() {
                if !plain && !ty.ends_with('$') {
                    continue;
                }
                let take = if *plain {
                    format!("        yield a{}\n", j)
                } else {
                    let item = ty.trim_end_matches('$');
                    carried_types.insert(item.to_string());
                    format!("        ca{j}: {ty} = __copied_{item}(a{j})\n        yield ca{j}\n", j = j, ty = ty, item = item)
                };
                h.push_str(&format!("    i{j}: index = add hb, {j}\n    h{j}: u8 = load hp, i{j}, 1\n    g{j}: u1 = cmp.eq h{j}, dir\n    a{j}: {ty} = get x, {name}\n    b{j}: {ty} = get y{c}, {name}\n    v{j}: {ty} = if g{j}\n{take}    else\n        yield b{j}\n    y{c2}: __ctx = set y{c}, {name}, v{j}\n", j = j, ty = ty, name = name, c = c, c2 = c + 1, take = take));
                c += 1;
                if *plain && j < fields.len() {
                    note.push_str(&format!("    i{j}: index = add hb, {j}\n    h{j}: u8 = load hp, i{j}, 1\n    g{j}: u1 = cmp.eq h{j}, dir\n    a{j}: {ty} = get x, {name}\n    sa{j}: {ty} = get s{sc}, {name}\n    su{j}: {ty} = if g{j}\n        yield a{j}\n    else\n        yield sa{j}\n    s{sc2}: __ctx = set s{sc}, {name}, su{j}\n    ta{j}: {ty} = get t{sc}, {name}\n    tu{j}: {ty} = if g{j}\n        yield a{j}\n    else\n        yield ta{j}\n    t{sc2}: __ctx = set t{sc}, {name}, tu{j}\n", j = j, ty = ty, name = name, sc = sc, sc2 = sc + 1));
                    sc += 1;
                }
            }
            for (r, (names, _)) in rings.iter().enumerate() {
                let (Some(at), Some(kind)) = (ring_at[r], kinds[r]) else { continue };
                used.insert(kind);
                let ty = ctx_fields.iter().find(|(n, _)| *n == names[0]).map(|(_, t)| t.clone()).unwrap_or_default();
                h.push_str(&format!("    ri{r}: index = add hb, {at}\n    rh{r}: u8 = load hp, ri{r}, 1\n    rg{r}: u1 = cmp.eq rh{r}, dir\n    ra{r}: {ty} = get x, {first}\n    rb{r}: {ty} = get y{c}, {first}\n    if rg{r}\n        __hand_{kind}(ra{r}, rb{r})\n", r = r, at = at, ty = ty, first = names[0], c = c, kind = kind));
                for (m, name) in names.iter().enumerate() {
                    h.push_str(&format!("    qi{r}_{m}: index = add hb, {at}\n    qh{r}_{m}: u8 = load hp, qi{r}_{m}, 1\n    qg{r}_{m}: u1 = cmp.eq qh{r}_{m}, dir\n    qa{r}_{m}: {ty} = get x, {name}\n    qb{r}_{m}: {ty} = get y{c}, {name}\n    pa{r}_{m}: index = get qa{r}_{m}, pos\n    pb{r}_{m}: index = get qb{r}_{m}, pos\n    nq{r}_{m}: index = if qg{r}_{m}\n        yield pa{r}_{m}\n    else\n        yield pb{r}_{m}\n    qy{r}_{m}: {ty} = set qb{r}_{m}, pos, nq{r}_{m}\n    y{c2}: __ctx = set y{c}, {name}, qy{r}_{m}\n", r = r, m = m, at = at + 1 + m, ty = ty, name = name, c = c, c2 = c + 1));
                    c += 1;
                }
            }
            h.push_str(&format!("    store y{c}, c\n    ret\n", c = c));
            note.push_str(&format!("    store s{sc}, p0\n    store t{sc}, p1\n    ret\n", sc = sc));
            out.push_str(&h);
            out.push_str(&note);
        } else {
            // what a move hands across, once, in its turn: the wire's own
            // work, done for both places at the client
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
                let (to_server, to_client) = if *plain {
                    (format!("        yield a{}\n", j), format!("        yield b{}\n", j))
                } else {
                    let item = ty.trim_end_matches('$');
                    carried_types.insert(item.to_string());
                    (format!("        ca{j}: {ty} = __copied_{item}(a{j})\n        yield ca{j}\n", j = j, ty = ty, item = item), format!("        cb{j}: {ty} = __copied_{item}(b{j})\n        yield cb{j}\n", j = j, ty = ty, item = item))
                };
                h.push_str(&format!("    i{j}: index = add hb, {j}\n    h{j}: u8 = load hp, i{j}, 1\n    u{j}: u1 = cmp.eq h{j}, 1\n    d{j}: u1 = cmp.eq h{j}, 2\n    a{j}: {ty} = get s{c}, {name}\n    b{j}: {ty} = get f{c}, {name}\n    nb{j}: {ty} = if u{j}\n{to_server}    else\n        yield b{j}\n    na{j}: {ty} = if d{j}\n{to_client}    else\n        yield a{j}\n    f{c2}: __ctx = set f{c}, {name}, nb{j}\n    s{c2}: __ctx = set s{c}, {name}, na{j}\n", j = j, ty = ty, name = name, c = c, c2 = c + 1, to_server = to_server, to_client = to_client));
                c += 1;
            }
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
        }
        if used.contains("queue") {
            out.push_str("\n; a queue handed across (lib/stream.ssa, the queue): the items not yet given back, in their slots, and how many it has had, where its first slot stands, and whether it has ended\nfn __hand_queue(a: any$, b: any$)\n    ra: ptr = get a, ring\n    rb: ptr = get b, ring\n    n: index = load ra\n    o: index = load ra, 8\n    k: index = sub n, o\n    qa: ptr(any) = load ra, 16\n    qb: ptr(any) = load rb, 16\n    s: any[] = pack qa, k, 1\n    d: any[] = pack qb, k, 1\n    copy d, s\n    store n, rb\n    store o, rb, 8\n    e: i64 = load ra, 56\n    store e, rb, 56\n    ret\n");
        }
        if used.contains("ring") {
            out.push_str("\n; a ring handed across (lib/stream.ssa, the ring): its items with their times, how many it has had, its beat and whether it has ended\nfn __hand_ring(a: any$, b: any$)\n    ra: ptr = get a, ring\n    rb: ptr = get b, ring\n    n: index = load ra\n    store n, rb\n    hz: i64 = load ra, 32\n    store hz, rb, 32\n    st: i64 = load ra, 40\n    store st, rb, 40\n    t0: i64 = load ra, 48\n    store t0, rb, 48\n    e: i64 = load ra, 56\n    store e, rb, 56\n    wa: ptr = load ra, 16\n    wb: ptr = load rb, 16\n    va: any[] = slice wa\n    vb: any[] = slice wb\n    copy vb, va\n    ta: ptr = load ra, 24\n    tb: ptr = load rb, 24\n    none: ptr = const 0\n    timed: u1 = cmp.ne ta, none\n    if timed\n        ka: i64[] = slice ta\n        kb: i64[] = slice tb\n        copy kb, ka\n    ret\n");
        }
    }
    let mut conts: Vec<(i64, Option<usize>)> = Vec::new();
    let (mut real_carried, mut real_homes) = (Vec::new(), Vec::new());
    if wire {
        let ids: Vec<(i64, Option<usize>)> = fns.iter().find(|f| f.name == "__act_go").map(|f| f.body.iter().filter_map(|l| def(l.trim())).filter_map(|(n, _, rhs)| Some((n.strip_prefix('n')?.parse::<i64>().ok()?, rhs.split('(').next()?.to_string()))).map(|(id, g)| (id, owner.get(&g).copied().filter(|&o| o < nu))).collect()).unwrap_or_default();
        let heres: BTreeSet<String> = lines.iter().flat_map(|l| called(l)).filter_map(|c| c.strip_prefix("__here__")).map(str::to_string).collect();
        // (the function the list names is the one written, which may be continuations now: its unit is found by its name)
        let heres: Vec<(String, Option<usize>)> = heres.into_iter().map(|g| { let o = units.iter().position(|u| u.irs.contains(&g)); (g, o) }).collect();
        let home_words: Vec<(usize, String, String)> = homes.iter().enumerate().filter(|(_, (_, w))| w != "user").filter_map(|(k, (n, _))| ctx_fields.iter().find(|(f, _)| f == n).filter(|(_, ty)| carried(ty, &structs) && word(named.get(ty).map_or(ty.as_str(), |t| t.as_str())) && ty != "ptr").map(|(_, ty)| (k, n.clone(), ty.clone()))).collect();
        let carried_fields: Vec<(usize, String, String)> = fields.iter().enumerate().filter(|(_, (_, ty, plain))| *plain && word(named.get(ty).map_or(ty.as_str(), |t| t.as_str())) && ty != "ptr").map(|(j, (n, t, _))| (j, n.clone(), t.clone())).collect();
        conts = ids.clone();
        real_carried = carried_fields.clone();
        real_homes = home_words.clone();
        if stamps {
            out.push_str(&format!("\n; (hop fifty-eight) a push into a group's or everyone's stream that keeps a tick an item: its home stamps it (fm3 places.md, \"whose clock stamps a shared item\"). The home is the server, and its time for a push is the later of its own and the world's clock, when the push reached it; a line of the record, 28, says where that is the later. At the client, and while the client runs the server's part again, a push is stamped with the place's own time\nfn __push_home(s: any$, v: any)\n    r: ptr = get s, ring\n    step: i64 = load r, 40\n    regular: u1 = cmp.gt step, 0\n    if regular\n        push(s, v)\n    else\n        t: i64 = __home_now()\n        push(s, t, v)\n    ret\nfn __push_home(s: any$, block: any[])\n    r: ptr = get s, ring\n    step: i64 = load r, 40\n    regular: u1 = cmp.gt step, 0\n    if regular\n        push(s, block)\n    else\n        t: i64 = __home_now()\n        push(s, t, block)\n    ret\nfn __home_now() -> i64\n    t: i64 = __now()\n    p: index = __wplace()\n    srv: u1 = cmp.eq p, 1\n    if srv\n    else\n        ret t\n{again}    wp: ptr = addr __w\n    w: i64 = load wp\n    late: u1 = cmp.gt w, t\n    if late\n    else\n        ret t\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        store 1: i64, lp, l0, 8\n        l1: index = add l0, 1\n        store 28: i64, lp, l1, 8\n        l2: index = add l0, 2\n        store w, lp, l2, 8\n        l3: index = add l0, 3\n        store 0: i64, lp, l3, 8\n        l4: index = add l0, 4\n        store t, lp, l4, 8\n        l5: index = add l0, 5\n        store 0: i64, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret w\n", log = WIRE_LOG, again = if choose { "    gw: ptr = addr __wg\n    g: i64 = load gw\n    ag: u1 = cmp.ne g, 0\n    if ag\n        ret t\n" } else { "" }));
        }
        out.push_str(&wire_text(&World { fbase, fields: carried_fields, ids, heres, arrivals: arrivals.clone(), pooled, input: choose || fns.iter().any(|f| f.name == "__in_turn"), runs: fns.iter().any(|f| f.name == "__run"), writes: (present.contains("__out_ch"), present.contains("__out_block")), cols: pool.clone(), moved, nu, size, hsize, lands, land_at: present.contains("__act_land_at"), changes, choose, paced, homes: home_words }));
    }
    for item in &carried_types {
        out.push_str(&format!("\n; a value that holds an address, an array or a string, copied (fm3 log 267): its storage as the library lays it out, a queue's resident items in a buffer of their number or a ring's buffers whole, with its counts, its beat and its end, carved from the arena; the reader's place and its rules go with the value. A value is never modified, so the copy is the value\nfn __copied_{t}(v: {t}$) -> {t}$\n    r: ptr = get v, ring\n    ar: ptr = addr __arena\n    n: ptr = arena_alloc(ar, 64)\n    pushed: index = load r\n    o: index = load r, 8\n    hz: i64 = load r, 32\n    st: i64 = load r, 40\n    t0: i64 = load r, 48\n    e: i64 = load r, 56\n    w: index = load r, 24\n    store pushed, n\n    store o, n, 8\n    store hz, n, 32\n    store st, n, 40\n    store t0, n, 48\n    store e, n, 56\n    sz: index = sizeof {t}\n    stepped: u1 = cmp.ne st, 0\n    worded: u1 = cmp.ne w, 0\n    queue: u1 = and stepped, worded\n    if queue\n        k: index = sub pushed, o\n        one: index = const 1\n        cap: index = max(k, one)\n        bytes: index = mul sz, cap\n        total: index = add bytes, 16\n        vb: ptr = arena_alloc(ar, total)\n        buffer_init(vb, sz, cap)\n        slots: ptr = ptradd vb, 16\n        qa: ptr({t}) = load r, 16\n        qb: ptr({t}) = cast slots\n        s: {t}[] = pack qa, k, 1\n        d: {t}[] = pack qb, k, 1\n        copy d, s\n        store slots, n, 16\n        store cap, n, 24\n    else\n        wa: ptr = load r, 16\n        cap2: index = load wa, 8\n        bytes2: index = mul sz, cap2\n        total2: index = add bytes2, 16\n        wb: ptr = arena_alloc(ar, total2)\n        buffer_init(wb, sz, cap2)\n        va: {t}[] = slice wa\n        vc: {t}[] = slice wb\n        copy vc, va\n        store wb, n, 16\n        ta: ptr = load r, 24\n        if worded\n            tcap: index = load ta, 8\n            tbytes: index = mul tcap, 8\n            ttotal: index = add tbytes, 16\n            tb: ptr = arena_alloc(ar, ttotal)\n            eight: index = const 8\n            buffer_init(tb, eight, tcap)\n            ka: i64[] = slice ta\n            kb: i64[] = slice tb\n            copy kb, ka\n            store tb, n, 24\n        else\n            store ta, n, 24\n    v2: {t}$ = set v, ring, n\n    ret v2\n\n; ... where the wire was crossed to bring it: called at its own place, a function is handed the value itself\nfn __carried_{t}(v: {t}$, w: i64) -> {t}$\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    here: i64 = conv second\n    same: u1 = cmp.eq here, w\n    if same\n        ret v\n    c: {t}$ = __copied_{t}(v)\n    ret c\n", t = item));
    }
    // (hop fifty-nine) a person's pair: which place a context is is told
    // from the person's first context, and a move hands across between
    // the two of the current person
    let out = if pairs {
        out.replace("    cm: ptr = addr __ctx_mem\n", "    cm: ptr = __pair_base()\n").replace("    cm: ptr(array(__ctx, 2)) = addr __ctx_mem\n", "    cmb: ptr = __pair_base()\n    cm: ptr(array(__ctx, 2)) = cast cmb\n")
    } else {
        out
    };
    Ok((out, Placed { units, facts, crossings: crossings.into_iter().collect(), fields, rings, sigs, copied, moved, hand_fields, ring_at, held, pool, left, sizes: (size, hsize), wire, glue, unwired, conts, clock_dues, named, stack: Default::default(), homes, stamps, pairs, carried: real_carried, home_words: real_homes }))
}

/// what the world's text is written from
struct World {
    /// where the fields' flags begin in the cut's table
    fbase: usize,
    /// the fields an item may carry: its place among the flags, its name, its type
    fields: Vec<(usize, String, String)>,
    /// each continuation's number and its unit, or none where it is the client's own
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
    /// the pool's columns, for a copy of the server's half (fm3 log 273)
    cols: Vec<String>,
    /// the cut may be moved (fm3 log 276), and how many units there are
    /// and how long the cut's table is
    moved: bool,
    nu: usize,
    size: usize,
    /// some function keeps a value, which is copied where its record
    /// arrives (fm3 log 284)
    lands: bool,
    /// ... and where a record arrives in a call or its end only what
    /// the continuation reads is copied (hop sixty-three)
    land_at: bool,
    /// the wire's delay changes while the program runs, and it may go
    /// (fm3 log 289)
    changes: bool,
    /// the runner chooses the cut (fm3 log 290)
    choose: bool,
    /// how many things a move may hand (fm3 log 276); and the wire has a
    /// speed or comes and goes (fm3 log 294)
    hsize: usize,
    paced: bool,
    /// the group's and everyone's streams kept as one word (hop
    /// fifty-eight): their place among `Placed::homes`, name and type.
    /// At the world's end a line of the record says each at both places
    homes: Vec<(usize, String, String)>,
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
    if w.lands {
        t.push_str("\n; a record has arrived: its values are copied into this place's memory (fm3 log 284). `__w`'s fourth word, 64, is a test's: the copy is not made\nfn __wland(row: index)\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 64\n    whole: u1 = cmp.eq bm, 0\n    if whole\n        __act_land(row)\n    ret\n");
        if w.land_at {
            t.push_str("\n; ... and where a record arrives in a call or its end, over a wire whose cut does not move (hop sixty-three): only what the continuation it goes on with reads that the other place wrote\nfn __wland_at(row: index, k: i64)\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 64\n    whole: u1 = cmp.eq bm, 0\n    if whole\n        __act_land_at(row, k)\n    ret\n");
        }
    }
    t.push_str("\n; which place this is: 0 the client, 1 the server\nfn __wplace() -> index\n    a: ptr = context()\n    cm: ptr = addr __ctx_mem\n    second: u1 = cmp.ne a, cm\n    p: index = conv second\n    ret p\n");
    t.push_str("\n; is this unit at this place: a byte of the cut\nfn __unit_here(u: index) -> u1\n    cp: ptr = addr __cut\n    c: u8 = load cp, u, 1\n    at: index = conv c\n    p: index = __wplace()\n    h: u1 = cmp.eq at, p\n    ret h\n");
    for (g, o) in &w.heres {
        match o {
            Some(u) => t.push_str(&format!("fn __here__{g}() -> u1\n    h: u1 = __unit_here({u})\n    ret h\n", g = g, u = u)),
            None => t.push_str(&format!("fn __here__{g}() -> u1\n    p: index = __wplace()\n    h: u1 = cmp.eq p, 0\n    ret h\n", g = g)),
        }
    }
    if w.pooled {
        t.push_str("\n; is the continuation of this number at this place (fm3 log 269): its unit's byte of the cut, or the client where it is a case's own call\nfn __act_here(k: i64) -> u1\n");
        for (id, o) in &w.ids {
            match o {
                Some(u) => t.push_str(&format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        h{id}: u1 = __unit_here({u})\n        ret h{id}\n", id = id, u = u)),
                None => t.push_str(&format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        p{id}: index = __wplace()\n        h{id}: u1 = cmp.eq p{id}, 0\n        ret h{id}\n", id = id)),
            }
        }
        t.push_str("    ret 1\n");
    }
    // an item put on the wire
    t.push_str(&format!("\n; an item put on the wire toward the other place: it arrives the wire's delay after the world's clock now, and carries its sender's own time, which a wire adds nothing to. Gives its row\nfn __wput(kind: i64, a: i64, b: i64, said: i64) -> index\n    p: index = __wplace()\n    to: index = sub 1, p\n    wp: ptr = addr __w\n    t2: index = mul to, 2\n    hi: index = add t2, 24\n    ti: index = add t2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    used: i64 = sub tail, head\n    room: u1 = cmp.lt used, {q}\n    check room\n    m: i64 = rem tail, {q}\n    mi: index = conv m\n    qb: index = mul to, {q}\n    row: index = add qb, mi\n    now: i64 = load wp\n    d: i64 = load wp, 8\n{at}    c: ptr = context()\n    x: __ctx = load c\n    stamp: i64 = get x, __clock\n    p1: ptr = addr __wq_at\n    store at, p1, row, 8\n    p2: ptr = addr __wq_kind\n    store kind, p2, row, 8\n    p3: ptr = addr __wq_a\n    store a, p3, row, 8\n    p4: ptr = addr __wq_b\n    store b, p4, row, 8\n    p5: ptr = addr __wq_t\n    store stamp, p5, row, 8\n{rank}    tail2: i64 = add tail, 1\n    store tail2, wp, ti, 8\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        tw: i64 = conv to\n        store tw, lp, l0, 8\n        l1: index = add l0, 1\n        store kind, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store at, lp, l3, 8\n        l4: index = add l0, 4\n        store stamp, lp, l4, 8\n        l5: index = add l0, 5\n        store said, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n{plog}    ret row\n", q = q, log = WIRE_LOG, plog = if w.paced { "        __wsized(to, dn, sz)\n" } else { "" }, at = if w.paced { format!("    cz: ptr = context()\n    xz: __ctx = load cz\n    sz0: i64 = get xz, __clock\n    a0: i64, dn: i64, sz: i64 = __wpace(kind, a, b, to, now, d, sz0, tail)\n    some: u1 = cmp.gt used, 0\n    at: i64 = if some\n        pt: i64 = sub tail, 1\n        pm: i64 = rem pt, {q}\n        pmi: index = conv pm\n        prow: index = add qb, pmi\n        pa: ptr = addr __wq_at\n        pat: i64 = load pa, prow, 8\n        later: i64 = max(a0, pat)\n        yield later\n    else\n        yield a0\n", q = q) } else if w.changes { format!("    a0: i64 = __wat(now, d)\n    some: u1 = cmp.gt used, 0\n    at: i64 = if some\n        pt: i64 = sub tail, 1\n        pm: i64 = rem pt, {q}\n        pmi: index = conv pm\n        prow: index = add qb, pmi\n        pa: ptr = addr __wq_at\n        pat: i64 = load pa, prow, 8\n        later: i64 = max(a0, pat)\n        yield later\n    else\n        yield a0\n", q = q) } else { "    at: i64 = add now, d\n".to_string() }, rank = if w.input { format!("    ap: ptr = addr __wa\n    ri: index = add p, 10\n    rk: i64 = load ap, ri, 8\n    p6: ptr = addr __wq_r\n    store rk, p6, row, 8\n    md: i64 = load ap\n    ah: u1 = cmp.ne md, 0\n    b8: index = mul p, 8\n    rci: index = add b8, 10\n    rc: i64 = load wp, rci, 8\n    ev: i64 = if ah\n        yield rc\n    else\n        yield {never}\n    p7: ptr = addr __wq_e\n    store ev, p7, row, 8\n", never = NEVER) } else { String::new() }));
    t.push_str("\n; one more thing this place has sent that the other may answer, and one more it has been sent\nfn __wsent()\n    p: index = __wplace()\n    wp: ptr = addr __w\n    b: index = mul p, 8\n    i: index = add b, 8\n    n: i64 = load wp, i, 8\n    n2: i64 = add n, 1\n    store n2, wp, i, 8\n");
    if w.input {
        t.push_str(&format!("    cli: u1 = cmp.eq p, 0\n    if cli\n        c: ptr = context()\n        x: __ctx = load c\n        st: i64 = get x, __clock\n        ap: ptr = addr __wa\n        sr: i64 = load ap, 80\n        m: i64 = rem n2, {sends}\n        mi: index = conv m\n        tp: ptr = addr __ws_t\n        store st, tp, mi, 8\n        rp: ptr = addr __ws_r\n        store sr, rp, mi, 8\n        __wvoid(st, sr, n2)\n", sends = SENDS));
    }
    t.push_str("    ret\nfn __wrecv()\n    p: index = __wplace()\n    wp: ptr = addr __w\n    b: index = mul p, 8\n    i: index = add b, 10\n    n: i64 = load wp, i, 8\n    n2: i64 = add n, 1\n    store n2, wp, i, 8\n    ret\n");
    // text
    // (hop sixty-three) where the wire toward the client is half full, a
    // character is added to the text row before it rather than put in a
    // row of its own: `said frame()` with everything on the server writes
    // 704 characters in one turn, and the wire holds 256 rows. A row that
    // is not yet taken, for the same time and arriving at the same moment,
    // is one piece of text however many characters it has. Not in a
    // build whose rows carry a rank or a size, nor below half full, so
    // that no build that ran before sends anything otherwise
    let joins = w.writes.0 && !w.input && !w.paced && !w.changes;
    if joins {
        t.push_str(&format!("\n; a character for the client where the wire toward it is half full or more (hop sixty-three): added to the row before it, where that row is text not yet taken, for the same time and arriving at the same moment, and its characters end where the blocks in flight do. Says whether it was. The record for a person says a block of 127 characters or more as 127, its first seven and how many more being one word\nfn __wjoin(c: u8) -> u1\n    wp: ptr = addr __w\n    head: i64 = load wp, 192\n    tail: i64 = load wp, 200\n    used: i64 = sub tail, head\n    full: u1 = cmp.ge used, {half}\n    if full\n    else\n        ret 0\n    lt: i64 = sub tail, 1\n    m: i64 = rem lt, {q}\n    row: index = conv m\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    tq: ptr = addr __wq_t\n    st: i64 = load tq, row, 8\n    atq: ptr = addr __wq_at\n    at: i64 = load atq, row, 8\n    now: i64 = load wp\n    d: i64 = load wp, 8\n    at2: i64 = add now, d\n    cx: ptr = context()\n    x: __ctx = load cx\n    clk: i64 = get x, __clock\n    k1: u1 = cmp.eq kind, 1\n    k2: u1 = cmp.eq kind, 2\n    txt: u1 = or k1, k2\n    st1: u1 = cmp.eq st, clk\n    at1: u1 = cmp.eq at, at2\n    j0: u1 = and txt, st1\n    j1: u1 = and j0, at1\n    if j1\n    else\n        ret 0\n    aq: ptr = addr __wq_a\n    a: i64 = load aq, row, 8\n    bq: ptr = addr __wq_b\n    b: i64 = load bq, row, 8\n    o: i64 = load wp, 40\n    tp: ptr = addr __wq_text\n    oi: index = conv o\n    if k1\n        e1: i64 = add o, 2\n        f1: u1 = cmp.le e1, 65536\n        if f1\n        else\n            ret 0\n        c0: u8 = conv a\n        store c0, tp, oi, 1\n        o1: index = add oi, 1\n        store c, tp, o1, 1\n        store 2: i64, kp, row, 8\n        store o, aq, row, 8\n        store 2: i64, bq, row, 8\n        store e1, wp, 40\n    else\n        ab: i64 = add a, b\n        on: u1 = cmp.eq ab, o\n        e2: i64 = add o, 1\n        f2: u1 = cmp.le e2, 65536\n        g2: u1 = and on, f2\n        if g2\n        else\n            ret 0\n        store c, tp, oi, 1\n        b2: i64 = add b, 1\n        store b2, bq, row, 8\n        store e2, wp, 40\n    na: i64 = load aq, row, 8\n    nb: i64 = load bq, row, 8\n    nai: index = conv na\n    nbi: index = conv nb\n    cap: i64 = const 127\n    nc: i64 = min(nb, cap)\n    w7: i64 = loop(i: index = 0, acc: i64 = nc) bound 8\n        end: u1 = cmp.ge i, nbi\n        seven: u1 = cmp.ge i, 7\n        stop: u1 = or end, seven\n        if stop\n            break acc\n        ii: index = add nai, i\n        by: u8 = load tp, ii, 1\n        bw: i64 = conv by\n        sh: i64 = mul acc, 256\n        acc2: i64 = add sh, bw\n        i2: index = add i, 1\n        continue i2, acc2\n    ln: i64 = load wp, 32\n    some: u1 = cmp.gt ln, 0\n    held: u1 = cmp.le ln, {log}\n    lg: u1 = and some, held\n    if lg\n        lj: i64 = sub ln, 1\n        li: index = conv lj\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        lw: i64 = load lp, l0, 8\n        l1: index = add l0, 1\n        lk: i64 = load lp, l1, 8\n        l3: index = add l0, 3\n        la: i64 = load lp, l3, 8\n        l4: index = add l0, 4\n        ls: i64 = load lp, l4, 8\n        lc0: u1 = cmp.eq lw, 0\n        lk1: u1 = cmp.eq lk, 1\n        lk2: u1 = cmp.eq lk, 2\n        lkt: u1 = or lk1, lk2\n        la1: u1 = cmp.eq la, at\n        ls1: u1 = cmp.eq ls, st\n        q0: u1 = and lc0, lkt\n        q1: u1 = and q0, la1\n        q2: u1 = and q1, ls1\n        if q2\n            store 2: i64, lp, l1, 8\n            l5: index = add l0, 5\n            store w7, lp, l5, 8\n    ret 1\n", half = q / 2, q = q, log = WIRE_LOG));
    }
    if w.writes.0 {
        t.push_str(&format!("\n; the output's two writes (fm3 log 269): at the client, the capture; the server, an item of the wire, which needs nothing back\nfn __out_ch(c: u8)\n    p: index = __wplace()\n    server: u1 = cmp.ne p, 0\n    if server\n{join}        a: i64 = conv c\n        row: index = __wput(1, a, 0, a)\n        ret\n    __cap_ch(c)\n    ret\n", join = if joins { "        j: u1 = __wjoin(c)\n        if j\n            ret\n" } else { "" }));
    }
    if w.writes.1 {
        t.push_str("fn __out_block(v: u8[])\n    p: index = __wplace()\n    server: u1 = cmp.ne p, 0\n    if server\n        k: index = len v\n        wp: ptr = addr __w\n        o: i64 = load wp, 40\n        oi: index = conv o\n        e: index = add oi, k\n        fits: u1 = cmp.le e, 65536\n        check fits\n        tp: ptr = addr __wq_text\n        qd: ptr(u8) = ptradd tp, oi\n        dst: u8[] = pack qd, k, 1\n        copy dst, v\n        o2: i64 = conv e\n        store o2, wp, 40\n        kk: i64 = conv k\n        w7: i64 = loop(i: index = 0, acc: i64 = kk) bound 8\n            end: u1 = cmp.ge i, k\n            full: u1 = cmp.ge i, 7\n            stop: u1 = or end, full\n            if stop\n                break acc\n            by: u8 = load v, i\n            bw: i64 = conv by\n            sh: i64 = mul acc, 256\n            acc2: i64 = add sh, bw\n            i2: index = add i, 1\n            continue i2, acc2\n        row: index = __wput(2, o, kk, w7)\n        ret\n    __cap_block(v)\n    ret\n");
    }
    t.push_str("\n; when what was just put in the capture is shown: the later of its own time and the world's clock. A piece that arrives before its time is kept until it; one that arrives after is late by the difference. A word for each character, in the server's half of the marks\nfn __shown(n: index, k: index)\n    wp: ptr = addr __w\n    now: i64 = load wp\n    c: ptr = context()\n    x: __ctx = load c\n    own: i64 = get x, __clock\n    s: i64 = max(own, now)\n    sp: ptr = addr __out_t\n    top: index = add n, k\n    loop(i: index = n) bound 65536\n        done: u1 = cmp.ge i, top\n        if done\n            break\n        j: index = add i, 65538\n        store s, sp, j, 8\n        i2: index = add i, 1\n        continue i2\n    ret\n");
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
    let bound = if w.input { "    now: i64 = load wp\n    lim: i64 = add now, 1\n    client: u1 = cmp.eq p, 0\n    sooner: u1 = cmp.lt lim, t0\n    same: u1 = cmp.eq lim, t0\n    first: u1 = cmp.lt 1, r0\n    tie: u1 = and same, first\n    under: u1 = or sooner, tie\n    use: u1 = and client, under\n    t: i64 = if use\n        yield lim\n    else\n        yield t0\n    r: i64 = if use\n        yield 1\n    else\n        yield r0\n" } else { "    t: i64 = add t0, 0\n    r: i64 = add r0, 0\n" };
    t.push_str(&format!("\n; a place says where it stands (fm3 log 269): the time and the rank of the earliest thing it has, before which it will begin nothing unless it is sent something, and how much of what it was sent it has had. The client can say nothing of a time the world's clock has not reached where a key may yet be pressed: its bound is then the moment after now, at the input's rank, which is the first. Said where this place's own earliest thing has changed, or it has been sent something since it last said; and, where only the world's clock has moved its bound on, when that frees the other to take the turn it is waiting for and not before. An item of the wire, carrying the fields that cross\nfn __wsay() -> u1\n    p: index = __wplace()\n{next}    wp: ptr = addr __w\n{bound}    b: index = mul p, 8\n    i2: index = add b, 10\n    recv: i64 = load wp, i2, 8\n    i5: index = add b, 13\n    st: i64 = load wp, i5, 8\n    i6: index = add b, 14\n    sr: i64 = load wp, i6, 8\n    i7: index = add b, 15\n    sa: i64 = load wp, i7, 8\n    e1: u1 = cmp.eq st, t\n    e2: u1 = cmp.eq sr, r\n    e3: u1 = cmp.eq sa, recv\n    e12: u1 = and e1, e2\n    p2: index = mul p, 2\n    i8: index = add p2, 28\n    ot: i64 = load wp, i8, 8\n    i9: index = add p2, 29\n    or0: i64 = load wp, i9, 8\n    o1: u1 = cmp.eq ot, t0\n    o2: u1 = cmp.eq or0, r0\n    o12: u1 = and o1, o2\n    j3: index = add b, 11\n    pt: i64 = load wp, j3, 8\n    j4: index = add b, 12\n    pr: i64 = load wp, j4, 8\n    olt: u1 = cmp.lt pt, st\n    oeq: u1 = cmp.eq pt, st\n    orl: u1 = cmp.lt pr, sr\n    otie: u1 = and oeq, orl\n    past: u1 = or olt, otie\n    nv: i64 = const {never}\n    never: u1 = cmp.ge pt, nv\n    unheld: u1 = or past, never\n    ult: u1 = cmp.lt pt, t\n    ueq: u1 = cmp.eq pt, t\n    url: u1 = cmp.lt pr, r\n    utie: u1 = and ueq, url\n    frees: u1 = or ult, utie\n    stays: u1 = xor frees, 1\n    idle: u1 = or unheld, stays\n    creep: u1 = or e12, idle\n    q1: u1 = and e3, o12\n    q2: u1 = and q1, creep\n    dirty: u1 = __wfields_dirty()\n    clean: u1 = xor dirty, 1\n    quiet: u1 = and q2, clean\n    if quiet\n        ret 0\n    row: index = __wput(5, t, r, t)\n    m: i64 = __wfields_out(row)\n    cp: ptr = addr __wq_c\n    store m, cp, row, 8\n    ep: ptr = addr __wq_e\n    store recv, ep, row, 8\n    store t, wp, i5, 8\n    store r, wp, i6, 8\n    store recv, wp, i7, 8\n    store t0, wp, i8, 8\n    store r0, wp, i9, 8\n    carries: u1 = cmp.ne m, 0\n    if carries\n        __wsent()\n    ret 1\n", bound = bound, never = NEVER, next = if w.input { "    t9: i64, r9: i64 = __list_next()\n    wa: ptr = addr __wa\n    pk: i64 = load wa, 16\n    pkd: u1 = cmp.ne pk, 0\n    issrv: u1 = cmp.ne p, 0\n    held: u1 = and pkd, issrv\n    bt: i64 = load wa, 24\n    br: i64 = load wa, 32\n    t0: i64 = if held\n        yield bt\n    else\n        yield t9\n    r0: i64 = if held\n        yield br\n    else\n        yield r9\n" } else { "    t0: i64, r0: i64 = __list_next()\n" }));
    // delivery
    // (the three items of a move, fm3 log 276: the move's word, at the server, which first goes back to the move's time where it was ahead; the server's state, at the client; and a function partway handed across, which is given a place and not gone on with)
    // (the chooser's round trips, fm3 log 290: the server answers a ping at once; the client notes how long it took)
    let pings = if w.choose { format!("    is13: u1 = cmp.eq kind, 13\n    if is13\n        prow: index = __wput(14, a, b, a)\n        ret 1\n    is14: u1 = cmp.eq kind, 14\n    if is14\n        __wpinged(a, b)\n        ret 1\n") } else { String::new() };
    let moves = if w.moved {
        format!("    is10: u1 = cmp.eq kind, 10\n    if is10\n        __wrecv()\n{back}        ka: index = conv a\n        __wmove_server(row, ka, b)\n        ret 1\n    is11: u1 = cmp.eq kind, 11\n    if is11\n        __wrecv()\n        kb: index = conv a\n        __wmove_client(row, kb)\n        ret 1\n{take}", back = if w.input { "        __wback(b, 1)\n" } else { "" }, take = if w.pooled { "    is12: u1 = cmp.eq kind, 12\n    if is12\n        __wtake(row)\n        ret 1\n" } else { "" })
    } else {
        String::new()
    };
    let take = if w.pooled { format!("    is4: u1 = cmp.eq kind, 4\n    if is4\n        __wrecv()\n        cq: ptr = addr __wq_c\n        cm4: i64 = load cq, row, 8\n        __wfields_in(row, cm4)\n        slot: index = __act_slot()\n        bp: ptr = addr __act_born\n        born: i64 = load bp, slot, 8\n        fl: index = add row, {flight}\n        __act_carry(fl, slot)\n{land}        store born, bp, slot, 8\n        pp: ptr = addr __act_pc\n        store a, pp, slot, 8\n        __act_run(slot, a)\n        __act_next()\n", flight = 2 * pool, land = if w.lands && w.land_at { "        __wland_at(slot, a)\n" } else if w.lands { "        __wland(slot)\n" } else { "" }) } else { String::new() };
    let moves = format!("{}{}", pings, moves);
    t.push_str(&format!("\n; the wire's earliest item toward a place is handed over, where the world's clock has reached the time it arrives: text into the capture; a push made again, and the lines that walk the stream given their turn; a function partway given a place of the pool and gone on with at once, being part of the turn that sent it; a telling kept\nfn __wdeliver(pi: i32) -> u1\n    __zero_context(pi)\n    p: index = conv pi\n    wp: ptr = addr __w\n    p2: index = mul p, 2\n    hi: index = add p2, 24\n    ti: index = add p2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    none: u1 = cmp.ge head, tail\n    if none\n        ret 0\n    m: i64 = rem head, {q}\n    mi: index = conv m\n    qb: index = mul p, {q}\n    row: index = add qb, mi\n    ap: ptr = addr __wq_at\n    at: i64 = load ap, row, 8\n    now: i64 = load wp\n    notyet: u1 = cmp.gt at, now\n    if notyet\n        ret 0\n    head2: i64 = add head, 1\n    store head2, wp, hi, 8\n    gi: index = add p, 6\n    store 0: i64, wp, gi, 8\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    aq: ptr = addr __wq_a\n    a: i64 = load aq, row, 8\n    bq: ptr = addr __wq_b\n    b: i64 = load bq, row, 8\n    tq: ptr = addr __wq_t\n    stamp: i64 = load tq, row, 8\n{moves}{head}    is3: u1 = cmp.eq kind, 3\n    if is3\n        __wrecv()\n        __wire_push(a, b)\n{run}{take}    is5: u1 = cmp.eq kind, 5\n    if is5\n        cq5: ptr = addr __wq_c\n        cm5: i64 = load cq5, row, 8\n        __wfields_in(row, cm5)\n        fed: u1 = cmp.ne cm5, 0\n        if fed\n            __wrecv()\n        b8: index = mul p, 8\n        j1: index = add b8, 11\n        store a, wp, j1, 8\n        j2: index = add b8, 12\n        store b, wp, j2, 8\n        eq: ptr = addr __wq_e\n        ack: i64 = load eq, row, 8\n        j3: index = add b8, 9\n        store ack, wp, j3, 8\n{unpark}    ret 1\n", q = q, unpark = if w.input && w.pooled { "        ua: ptr = addr __wa\n        upk: i64 = load ua, 16\n        uon: u1 = cmp.ne upk, 0\n        usrv: u1 = cmp.ne p, 0\n        uu: u1 = and uon, usrv\n        if uu\n            ubt: i64 = load ua, 24\n            ubr: i64 = load ua, 32\n            ult: u1 = cmp.lt ubt, a\n            ueq: u1 = cmp.eq ubt, a\n            url: u1 = cmp.lt ubr, b\n            utie: u1 = and ueq, url\n            upast: u1 = or ult, utie\n            if upast\n                __wunpark()\n" } else { "" }, moves = moves, run = if w.runs { "        __run()\n" } else { "" }, take = take, head = if w.input { "    rq: ptr = addr __wq_r\n    rk: i64 = load rq, row, 8\n    eq0: ptr = addr __wq_e\n    ev: i64 = load eq0, row, 8\n    cq0: ptr = addr __wq_c\n    cm0: i64 = load cq0, row, 8\n    k1: u1 = cmp.eq kind, 1\n    k2: u1 = cmp.eq kind, 2\n    text: u1 = or k1, k2\n    if text\n        __wkeep(kind, a, b, stamp, rk, ev)\n        ret 1\n    k5: u1 = cmp.eq kind, 5\n    nof: u1 = cmp.eq cm0, 0\n    plain: u1 = and k5, nof\n    cli: u1 = cmp.eq p, 0\n    if plain\n        if cli\n        else\n            __wclock(stamp)\n            __wpassed(a, b)\n    else\n        if cli\n            __wflush(stamp, rk)\n        else\n            __wback(stamp, rk)\n        wa: ptr = addr __wa\n        ri: index = add p, 10\n        store rk, wa, ri, 8\n        __wclock(stamp)\n".to_string() } else { format!("    __wclock(stamp)\n{}{}", if w.writes.0 { "    is1: u1 = cmp.eq kind, 1\n    if is1\n        ch: u8 = conv a\n        __cap_ch(ch)\n" } else { "" }, if w.writes.1 { "    is2: u1 = cmp.eq kind, 2\n    if is2\n        ao: index = conv a\n        bk: index = conv b\n        tp: ptr = addr __wq_text\n        qs: ptr(u8) = ptradd tp, ao\n        v: u8[] = pack qs, bk, 1\n        __cap_block(v)\n" } else if joins { "    is2: u1 = cmp.eq kind, 2\n    if is2\n        ao: index = conv a\n        bk: index = conv b\n        tp: ptr = addr __wq_text\n        loop(ci: index = 0)\n            cd: u1 = cmp.ge ci, bk\n            if cd\n                break\n            cj: index = add ao, ci\n            cc: u8 = load tp, cj, 1\n            __cap_ch(cc)\n            ci2: index = add ci, 1\n            continue ci2\n" } else { "" }) }));
    // a turn
    // (where the server may work ahead, fm3 log 273: the client shows a piece it is keeping where that comes before its own earliest thing and its time has come; and the server, not told, takes its turn ahead)
    let next = if w.input { "    t1: i64, r1: i64 = __list_next()\n    ap: ptr = addr __wa\n    pk0: i64 = load ap, 16\n    pkn: u1 = cmp.ne pk0, 0\n    isf: u1 = cmp.ne p, 0\n    pkd: u1 = and pkn, isf\n    bt0: i64 = load ap, 24\n    br0: i64 = load ap, 32\n    t: i64 = if pkd\n        yield bt0\n    else\n        yield t1\n    r: i64 = if pkd\n        yield br0\n    else\n        yield r1\n" } else { "    t: i64, r: i64 = __list_next()\n" };
    let kept = if w.input { "    kn: i64 = load ap, 104\n    khas: u1 = cmp.gt kn, 0\n    kq: u1 = and client, khas\n    if kq\n        kh: i64 = load ap, 96\n        khi: index = conv kh\n        ktp: ptr = addr __wk_t\n        kt: i64 = load ktp, khi, 8\n        krp: ptr = addr __wk_r\n        kr: i64 = load krp, khi, 8\n        klt: u1 = cmp.lt kt, t\n        keq: u1 = cmp.eq kt, t\n        krl: u1 = cmp.lt kr, r\n        ktie: u1 = and keq, krl\n        kfirst: u1 = or klt, ktie\n        kdue: u1 = cmp.le kt, now\n        kgo: u1 = and kfirst, kdue\n        if kgo\n            __wshow()\n            ret 1\n" } else { "" };
    let ahead = if w.input {
        format!("    unfree: u1 = xor free, 1\n    a0: u1 = and g1, unfree\n    a1: u1 = and a0, server\n    cn: i64 = load ap, 48\n    croom: u1 = cmp.lt cn, {n}\n    a2: u1 = and a1, croom\n    mlen: index = load ap, 120\n    fits: u1 = cmp.le mlen, {mem}\n    a3: u1 = and a2, fits\n    bt: i64 = load ap, 24\n    br: i64 = load ap, 32\n    bte: u1 = cmp.eq bt, t\n    bre: u1 = cmp.eq br, r\n    barred: u1 = and bte, bre\n    unbarred: u1 = xor barred, 1\n    a4: u1 = and a3, unbarred\n    hd: i64 = load wp, 192\n    tl: i64 = load wp, 200\n    usd: i64 = sub tl, hd\n    wroom: u1 = cmp.lt usd, {half}\n    a5: u1 = and a4, wroom\n{rows}    if a6\n        k: index = __wcopy(t, r)\n        tail0: i64 = load wp, 200\n        sent0: i64 = load wp, 128\n        left0: i64 = load wp, 56\n        ln0: i64 = load wp, 32\n        off0: i64 = load wp, 40\n        sm: ptr(array(__ctx, 2)) = addr __wshadow\n        one: index = const 1\n        se: ptr(__ctx) = index sm, one\n        sp: ptr = cast se\n        sh: __ctx = load sp\n        kp: ptr = addr __wa_keep\n        store sh, kp\n        store 1: i64, ap\n        store 0: i64, ap, 8\n        store 0: i64, ap, 16\n        store r, ap, 88\n        store 0: i64, wp, 56\n        __list_turn()\n        store 0: i64, ap\n        sp1: i64 = load ap, 8\n        spoiled: u1 = cmp.ne sp1, 0\n        dirty2: u1 = __wfields_dirty()\n        bad: u1 = or spoiled, dirty2\n        if bad\n            __wrestore(k)\n            store tail0, wp, 200\n            store sent0, wp, 128\n            store left0, wp, 56\n            store ln0, wp, 32\n            store off0, wp, 40\n            sh2: __ctx = load kp\n            store sh2, sp\n            store cn, ap, 48\n            store 0: i64, ap, 16\n            store t, ap, 24\n            store r, ap, 32\n            th: i64 = load ap, 56\n            th2: i64 = add th, 1\n            store th2, ap, 56\n            __wnote(9, t, 0)\n            ret 1\n        pk: i64 = load ap, 16\n        parked: u1 = cmp.ne pk, 0\n        if parked\n            store t, ap, 24\n            store r, ap, 32\n        ret 1\n", n = AHEAD, mem = AHEAD_MEM, half = q / 2, rows = if w.pooled { format!("    cx: ptr = context()\n    xx: __ctx = load cx\n    an: index = get xx, __act_n\n    arow: u1 = cmp.le an, {}\n    a6: u1 = and a5, arow\n", AHEAD_ROWS) } else { "    a6: u1 = and a5, a5\n".to_string() })
    } else {
        String::new()
    };
    t.push_str(&format!("\n; a place takes the earliest turn it has (fm3 log 269), where the other has had everything this place sent it, a field of the context this place has changed that the other reads among it, and has said it will begin nothing before that turn: of two of one rank due together at the two places the client's is first (fm3 question 155). The client takes a turn for a time when the world's clock has reached it; the server has no clock but its own. `__w`'s fourth word is a test's: a place that does not wait to be told\nfn __wturn(pi: i32) -> u1\n    __zero_context(pi)\n    p: index = conv pi\n{next}    nv: i64 = const {never}\n    some: u1 = cmp.lt t, nv\n    wp: ptr = addr __w\n    b: index = mul p, 8\n    i0: index = add b, 8\n    sent: i64 = load wp, i0, 8\n    i1: index = add b, 9\n    acked: i64 = load wp, i1, 8\n    i3: index = add b, 11\n    pt: i64 = load wp, i3, 8\n    i4: index = add b, 12\n    pr: i64 = load wp, i4, 8\n    quiet: u1 = cmp.eq sent, acked\n    lt: u1 = cmp.lt t, pt\n    eq: u1 = cmp.eq t, pt\n    rlt: u1 = cmp.lt r, pr\n    rle: u1 = cmp.le r, pr\n    client: u1 = cmp.eq p, 0\n    rok: u1 = if client\n        yield rle\n    else\n        yield rlt\n    tie: u1 = and eq, rok\n    told: u1 = or lt, tie\n    but: i64 = load wp, 24\n    bit: i64 = if client\n        yield 1\n    else\n        yield 2\n    bm: i64 = and but, bit\n    blind: u1 = cmp.ne bm, 0\n    free: u1 = or told, blind\n    now: i64 = load wp\n    intime: u1 = cmp.le t, now\n    server: u1 = cmp.ne p, 0\n    timely: u1 = or server, intime\n    dirty: u1 = __wfields_dirty()\n    clean: u1 = xor dirty, 1\n    g0: u1 = and some, quiet\n    g1: u1 = and g0, clean\n    g2: u1 = and g1, free\n    go: u1 = and g2, timely\n{kept}    if go\n        gi: index = add p, 6\n        store 0: i64, wp, gi, 8\n{rank}        ret 1\n{ahead}    ret 0\n", never = NEVER, next = next, kept = kept, rank = if w.input && w.pooled { "        ri: index = add p, 10\n        store r, ap, ri, 8\n        store 0: i64, ap\n        if pkd\n            __wunpark()\n        else\n            __list_turn()\n" } else if w.input { "        ri: index = add p, 10\n        store r, ap, ri, 8\n        store 0: i64, ap\n        __list_turn()\n" } else { "        __list_turn()\n" }, ahead = ahead));
    t.push_str(&format!("\n; nothing can happen now: the world's clock moves on to the next time something can, an item's arrival or the time of a place's earliest thing, or stays, and says so, where there is none\nfn __wnext() -> u1\n    wp: ptr = addr __w\n    now: i64 = load wp\n    nv: i64 = const {never}\n    h0: i64 = load wp, 192\n    t0: i64 = load wp, 200\n    has0: u1 = cmp.lt h0, t0\n    a0: i64 = if has0\n        m0: i64 = rem h0, {q}\n        r0: index = conv m0\n        p0: ptr = addr __wq_at\n        v0: i64 = load p0, r0, 8\n        yield v0\n    else\n        yield nv\n    h1: i64 = load wp, 208\n    t1: i64 = load wp, 216\n    has1: u1 = cmp.lt h1, t1\n    a1: i64 = if has1\n        m1: i64 = rem h1, {q}\n        r1: index = conv m1\n        r1q: index = add r1, {q}\n        p1: ptr = addr __wq_at\n        v1: i64 = load p1, r1q, 8\n        yield v1\n    else\n        yield nv\n    __zero_context(0)\n    s: i64, sr: i64 = __list_next()\n    sl: u1 = cmp.gt s, now\n    s2: i64 = if sl\n        yield s\n    else\n        yield nv\n    __zero_context(1)\n    f: i64, fr: i64 = __list_next()\n    fl: u1 = cmp.gt f, now\n    f2: i64 = if fl\n        yield f\n    else\n        yield nv\n    n1: i64 = min(a0, a1)\n    n2: i64 = min(n1, s2)\n{kept}    none: u1 = cmp.ge n3, nv\n    if none\n        ret 0\n    store n3, wp\n    ret 1\n", never = NEVER, q = q, kept = if w.input { "    n3a: i64 = min(n2, f2)\n    ap: ptr = addr __wa\n    kn: i64 = load ap, 104\n    kh: i64 = load ap, 96\n    khi: index = conv kh\n    ktp: ptr = addr __wk_t\n    kt: i64 = load ktp, khi, 8\n    khas: u1 = cmp.gt kn, 0\n    klate: u1 = cmp.gt kt, now\n    kok: u1 = and khas, klate\n    k2: i64 = if kok\n        yield kt\n    else\n        yield nv\n    n3: i64 = min(n3a, k2)\n" } else { "    n3: i64 = min(n2, f2)\n" }));
    t.push_str(&format!("\n; the world (fm3 cut.md, \"a wire that takes time\"; log 269): called by a case's twin when its call has been begun at the client. Before its clock moves the lines on the server that walk a stored stream are given their turn, as the client's were at the start, and each place tells the other where it stands, which takes no time: the two were joined before the case began. Then, until nothing is left: what the wire holds that is due is handed over; a place that may takes a turn; a place that can do no more says where it stands; and the clock moves on. A place left with something it was never free to do is a fault, and stops\nfn __world()\n    wp: ptr = addr __w\n    __zero_context(1)\n    fc: ptr = context()\n    fx: __ctx = load fc\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    z0: index = const 0\n    z1: index = const 1\n    e0: ptr(__ctx) = index sm, z0\n    e1: ptr(__ctx) = index sm, z1\n    q0: ptr = cast e0\n    q1: ptr = cast e1\n    store fx, q0\n    store fx, q1\n{start}    d: i64 = load wp, 8\n    store 0: i64, wp, 8\n    loop(hn: i64 = 0) bound 64\n        hd: u1 = cmp.ge hn, 64\n        if hd\n            break\n        hn2: i64 = add hn, 1\n        __zero_context(0)\n        y0: u1 = __wsay()\n        __zero_context(1)\n        y1: u1 = __wsay()\n        y2: u1 = __wdeliver(0)\n        y3: u1 = __wdeliver(1)\n        y01: u1 = or y0, y1\n        y23: u1 = or y2, y3\n        any: u1 = or y01, y23\n        if any\n            continue hn2\n        break\n    store d, wp, 8\n    loop(n: i64 = 0)\n        some: u1 = cmp.lt n, 4000000\n        check some\n        n2: i64 = add n, 1\n        d0: u1 = __wdeliver(0)\n        if d0\n            continue n2\n        d1: u1 = __wdeliver(1)\n        if d1\n            continue n2\n        g0: u1 = __wturn(0)\n        if g0\n            continue n2\n        g1: u1 = __wturn(1)\n        if g1\n            continue n2\n{choose}        __zero_context(0)\n        s0: u1 = __wsay()\n        if s0\n            continue n2\n        __zero_context(1)\n        s1: u1 = __wsay()\n        if s1\n            continue n2\n        more: u1 = __wnext()\n        if more\n            continue n2\n        break\n    nv: i64 = const {never}\n    __zero_context(1)\n    ft: i64, fr: i64 = __list_next()\n    fdone: u1 = cmp.ge ft, nv\n    check fdone\n    __zero_context(0)\n    st: i64, sr: i64 = __list_next()\n    sdone: u1 = cmp.ge st, nv\n    check sdone\n{end}    ret\n", never = NEVER, choose = if w.choose { "        c0: u1 = __wchoose()\n        if c0\n            continue n2\n" } else { "" }, start = if w.runs { "    __run()\n" } else { "" }, end = if w.input { "    ap: ptr = addr __wa\n    th: i64 = load ap, 56\n    sc: ptr = context()\n    sx: __ctx = load sc\n    on: index = get sx, __out_n\n    ok: u1 = cmp.lt on, 65536\n    if ok\n        oj: index = add on, 65538\n        op: ptr = addr __out_t\n        store th, op, oj, 8\n" } else { "" }));
    if w.input {
        t.push_str(&ahead_text(w));
    }
    if w.moved {
        t.push_str(&move_text(w));
    }
    if w.choose {
        t.push_str(&choose_text(w));
        t.push_str(&again_text(w));
        // (the client running the server's part again, fm3 log 292: each
        // place below is changed only where the runner chooses)
        let hook = |t: &mut String, find: &str, with: &str| {
            assert!(t.matches(find).count() == 1, "{}", find);
            *t = t.replacen(find, with, 1);
        };
        // an item put on the wire while the client runs the server's part goes nowhere; and every item carries the count of the moves made by running the server's part that its sender has heard of
        hook(&mut t, "fn __wput(kind: i64, a: i64, b: i64, said: i64) -> index\n    p: index = __wplace()\n", "fn __wput(kind: i64, a: i64, b: i64, said: i64) -> index\n    p: index = __wplace()\n    gwp: ptr = addr __wg\n    grp: i64 = load gwp\n    gop: u1 = cmp.ne grp, 0\n    if gop\n        grow: index = __wagain_put(kind, a, b)\n        ret grow\n");
        hook(&mut t, "    tail2: i64 = add tail, 1\n    store tail2, wp, ti, 8\n", "    gnp: index = mul p, 2\n    gnq: index = add gnp, 4\n    gnv: i64 = load gwp, gnq, 8\n    gnc: ptr = addr __wq_n\n    store gnv, gnc, row, 8\n    tail2: i64 = add tail, 1\n    store tail2, wp, ti, 8\n");
        // ... and nothing is counted as sent or had, and the record is not stamped
        hook(&mut t, "fn __wsent()\n    p: index = __wplace()\n", "fn __wsent()\n    p: index = __wplace()\n    gws: ptr = addr __wg\n    grs: i64 = load gws\n    gos: u1 = cmp.ne grs, 0\n    if gos\n        ret\n");
        hook(&mut t, "fn __wrecv()\n    p: index = __wplace()\n", "fn __wrecv()\n    p: index = __wplace()\n    gws: ptr = addr __wg\n    grs: i64 = load gws\n    gos: u1 = cmp.ne grs, 0\n    if gos\n        ret\n");
        hook(&mut t, "fn __wstamp(row: index, t: i64)\n    tq: ptr = addr __wq_t\n    store t, tq, row, 8\n", "fn __wstamp(row: index, t: i64)\n    tq: ptr = addr __wq_t\n    store t, tq, row, 8\n    gws: ptr = addr __wg\n    grs: i64 = load gws\n    gos: u1 = cmp.ne grs, 0\n    if gos\n        ret\n");
        // the pieces of the server's text the client has shown, counted
        hook(&mut t, "fn __wshow()\n", "fn __wshow()\n    gwh: ptr = addr __wg\n    gsh: i64 = load gwh, 16\n    gsh2: i64 = add gsh, 1\n    store gsh2, gwh, 16\n");
        // the client's log: each place an item is put on the wire that the server may need again
        hook(&mut t, "    __act_carry(a, fl)\n    dp: ptr = addr __act_due\n", "    __act_carry(a, fl)\n    __wlogged(row)\n    dp: ptr = addr __act_due\n");
        hook(&mut t, "    row: index = __wput(3, site, v, site)\n", "    row: index = __wput(3, site, v, site)\n    __wlogged(row)\n");
        hook(&mut t, "    ep: ptr = addr __wq_e\n    store recv, ep, row, 8\n", "    ep: ptr = addr __wq_e\n    store recv, ep, row, 8\n    __wlogged(row)\n");
        if w.pooled {
            hook(&mut t, "            __act_carry(best, fl)\n", "            __act_carry(best, fl)\n            __wlogged(row)\n");
        }
        // the server's start, as the world begins
        hook(&mut t, "    store fx, q0\n    store fx, q1\n", "    store fx, q0\n    store fx, q1\n    __wstart()\n");
        // a unit the client has taken by running the server's part is at both places until the server hears of it
        hook(&mut t, "    p: index = __wplace()\n    h: u1 = cmp.eq at, p\n    ret h\n", "    p: index = __wplace()\n    h0: u1 = cmp.eq at, p\n    h3: u1 = cmp.eq at, 3\n    h: u1 = or h0, h3\n    ret h\n");
        // what arrives: at the client, an item from the server made before it heard of the move is dropped, and a ping's answer never; at the server, the move's word where the client has taken its part
        hook(&mut t, "    tq: ptr = addr __wq_t\n    stamp: i64 = load tq, row, 8\n", &format!("    tq: ptr = addr __wq_t\n    stamp: i64 = load tq, row, 8\n    gwd: ptr = addr __wg\n    nqd: ptr = addr __wq_n\n    gnd: i64 = load nqd, row, 8\n    gsd: u1 = cmp.eq p, 0\n    if gsd\n        ghd: i64 = load gwd, 40\n        ghm: i64 = max(ghd, gnd)\n        store ghm, gwd, 40\n        gmd: i64 = load gwd, 32\n        gold: u1 = cmp.lt gnd, gmd\n        gk13: u1 = cmp.eq kind, 13\n        gk14: u1 = cmp.eq kind, 14\n        gping: u1 = or gk13, gk14\n        gprog: u1 = xor gping, 1\n        gbt: i64 = load wp, 24\n        gbm: i64 = and gbt, 256\n        gkeep: u1 = cmp.eq gbm, 0\n        g1: u1 = and gold, gprog\n        gdrop: u1 = and g1, gkeep\n        if gdrop\n            __wold(row, kind, a, stamp)\n            ret 1\n        g3: u1 = cmp.eq kind, 3\n        g4: u1 = cmp.eq kind, 4\n        g34: u1 = or g3, g4\n        if g34\n            gtk: i64 = load gwd, 56\n            gtk2: i64 = add gtk, 1\n            store gtk2, gwd, 56\n    is17: u1 = cmp.eq kind, 17\n    if is17\n        __wrecv()\n        __wback(b, 1)\n        ka17: index = conv a\n        __wagain_server(row, ka17)\n        ret 1\n"));
        // the chooser takes the server's part by running it again where it can, and gives it back only once the server has heard of that
        hook(&mut t, "    if go1\n        __wtaken(now)\n        x1: __ctx = set x, __due_move, now\n", "    if go1\n        __wtaken(now)\n        ag: u1 = __wagain_ok()\n        if ag\n            store 1: i64, cw, 48\n            ka1: i64 = add k, 1\n            store ka1, cw, 56\n            __wnote(15, now, b)\n            __wagain_move()\n            ret 1\n        x1: __ctx = set x, __due_move, now\n");
        // ... and while it holds the server's part so, nothing it sends the server is waited for: the server's answer can change nothing the client does then
        hook(&mut t, "    quiet: u1 = cmp.eq sent, acked\n", "    qeq: u1 = cmp.eq sent, acked\n    qgw: ptr = addr __wg\n    qhd: i64 = load qgw, 128\n    qho: u1 = cmp.ne qhd, 0\n    qsc: u1 = cmp.eq p, 0\n    qal: u1 = and qho, qsc\n    quiet: u1 = or qeq, qal\n");
        hook(&mut t, "            store x2, c\n            store 0: i64, cw, 48\n", "            store x2, c\n            store 0: i64, cw, 48\n            store 0: i64, gwc, 128\n");
        // ... and a piece kept for the very time and rank of the client's next turn was written by an earlier turn of that rank, the part run again having taken it, and is shown first
        hook(&mut t, "        krl: u1 = cmp.lt kr, r\n", "        krl: u1 = cmp.le kr, r\n");
        hook(&mut t, "    h2: u1 = and h1, free\n", "    h2a: u1 = and h1, free\n    gwc: ptr = addr __wg\n    ghc: i64 = load gwc, 40\n    gmc: i64 = load gwc, 32\n    gok: u1 = cmp.ge ghc, gmc\n    h2: u1 = and h2a, gok\n");
        // (the world goes on to the chooser's next moment only while something else is still to happen)
        let ends = "    none: u1 = cmp.ge n3, nv\n    if none\n        ret 0\n    store n3, wp\n";
        assert!(t.contains(ends));
        // (and, from fm3 log 292, while the client waits on the server at the cut given with a move left: a wire gone for good holds what it is waiting for, and only the chooser can end the wait)
        // (the chooser's measures, fm3 log 295: each item's size and the moment it was put on; what arrives at the client measured; and at the world's end, what was measured last)
        hook(&mut t, "    p5: ptr = addr __wq_t\n    store stamp, p5, row, 8\n    ap: ptr = addr __wa\n    ri: index = add p, 10\n", &format!("    p5: ptr = addr __wq_t\n    store stamp, p5, row, 8\n    ap: ptr = addr __wa\n    ri: index = add p, 10\n{sz}    pzq: ptr = addr __wq_sz\n    store wsz, pzq, row, 8\n    psq: ptr = addr __wq_s\n    store now, psq, row, 8\n    wsc: u1 = cmp.eq p, 0\n    if wsc\n        __wtraffic(0, kind, wsz)\n", sz = if w.paced { "    wsz: i64 = add sz, 0\n".to_string() } else { format!("    wsb: i64 = __wsize(kind, a, b)\n    wsz: i64 = add wsb, {}\n", PACE_HEAD) }));
        hook(&mut t, "    gsd: u1 = cmp.eq p, 0\n    if gsd\n", "    gsd: u1 = cmp.eq p, 0\n    if gsd\n        __wmeasure(row, kind)\n");
        hook(&mut t, "            continue n2\n        break\n    nv: i64 = const 4611686018427387904\n    __zero_context(1)\n", "            continue n2\n        break\n    __wmeasured()\n    nv: i64 = const 4611686018427387904\n    __zero_context(1)\n");
        t = t.replacen(ends, "    cn: i64 = __wchoose_next()\n    cwt: u1 = __wchoose_waits()\n    cnw: i64 = if cwt\n        yield cn\n    else\n        yield nv\n    n3w: i64 = min(n3, cnw)\n    none: u1 = cmp.ge n3w, nv\n    if none\n        ret 0\n    n4: i64 = min(n3, cn)\n    store n4, wp\n", 1);
    }
    if w.choose || w.paced {
        t.push_str(&size_text(w));
    }
    if w.paced && t.contains("        off0: i64 = load wp, 40\n") {
        // (a turn the server took ahead and took back put nothing on the wire,
        // fm3 log 294: the way toward the client is free when it was)
        t = t.replacen("        off0: i64 = load wp, 40\n", "        off0: i64 = load wp, 40\n        pzp: ptr = addr __wsp\n        pz16: i64 = load pzp, 128\n        pz18: i64 = load pzp, 144\n        pz20: i64 = load pzp, 160\n        pz22: i64 = load pzp, 176\n        pz24: i64 = load pzp, 192\n        pz26: i64 = load pzp, 208\n", 1);
        assert!(t.matches("            store off0, wp, 40\n").count() == 1);
        t = t.replacen("            store off0, wp, 40\n", "            store off0, wp, 40\n            store pz16, pzp, 128\n            store pz18, pzp, 144\n            store pz20, pzp, 160\n            store pz22, pzp, 176\n            store pz24, pzp, 192\n            store pz26, pzp, 208\n", 1);
    }
    if w.changes {
        t.push_str(&if w.paced { paced_text() } else { changes_text() });
        // (what the wire still holds where it has gone for good is the wire's: the two places are not asked to have finished)
        let ends = "    fdone: u1 = cmp.ge ft, nv\n    check fdone\n    __zero_context(0)\n    st: i64, sr: i64 = __list_next()\n    sdone: u1 = cmp.ge st, nv\n    check sdone\n";
        assert!(t.contains(ends));
        t = t.replacen(ends, "    wh0: i64 = load wp, 192\n    wt0: i64 = load wp, 200\n    wh1: i64 = load wp, 208\n    wt1: i64 = load wp, 216\n    held0: u1 = cmp.lt wh0, wt0\n    held1: u1 = cmp.lt wh1, wt1\n    held: u1 = or held0, held1\n    fd0: u1 = cmp.ge ft, nv\n    fdone: u1 = or fd0, held\n    check fdone\n    __zero_context(0)\n    st: i64, sr: i64 = __list_next()\n    sd0: u1 = cmp.ge st, nv\n    sdone: u1 = or sd0, held\n    check sdone\n", 1);
    }
    t.push_str("\n; the runner's: the wire's delay; what `__out_mark` reads back, 0 a piece's own time, 1 when it was shown, 2 the wire's record; and a test's switch\nfn __wire_set(d: i64)\n    wp: ptr = addr __w\n    store d, wp, 8\n    ret\nfn __wire_mode(m: i64)\n    wp: ptr = addr __w\n    store m, wp, 16\n    ret\nfn __wire_but(s: i64)\n    wp: ptr = addr __w\n    store s, wp, 24\n    ret\n");
    // (hop fifty-eight) at the world's end, a line of the record, 27, for
    // each group's or everyone's stream kept as a word: its value at
    // each place, the home's and the copy's
    if !w.homes.is_empty() {
        let end = "    nv: i64 = const 4611686018427387904\n    __zero_context(1)\n    ft: i64, fr: i64 = __list_next()\n";
        assert!(t.matches(end).count() == 1);
        t = t.replacen(end, &format!("    __whomes()\n{}", end), 1);
        let mut f = "\n; at the world's end (hop fifty-eight), a line of the record, 27, for each group's or everyone's stream kept as a word: the place, the world's clock, which stream, and its value there\nfn __whomes()\n    wp: ptr = addr __w\n    now: i64 = load wp\n    cm: ptr(array(__ctx, 2)) = addr __ctx_mem\n    lp: ptr = addr __wlog\n".to_string();
        let mut n = 0;
        for (k, name, ty) in &w.homes {
            for p in 0..2 {
                f.push_str(&format!("    e{n}: ptr(__ctx) = index cm, {p}\n    q{n}: ptr = cast e{n}\n    x{n}: __ctx = load q{n}\n    v{n}: {ty} = get x{n}, {name}\n    w{n}: i64 = conv v{n}\n    ln{n}: i64 = load wp, 32\n    r{n}: u1 = cmp.lt ln{n}, {log}\n    if r{n}\n        li{n}: index = conv ln{n}\n        l{n}: index = mul li{n}, 6\n        store {p}: i64, lp, l{n}, 8\n        a{n}: index = add l{n}, 1\n        store 27: i64, lp, a{n}, 8\n        b{n}: index = add l{n}, 2\n        store now, lp, b{n}, 8\n        c{n}: index = add l{n}, 3\n        store {k}: i64, lp, c{n}, 8\n        d{n}: index = add l{n}, 4\n        store 0: i64, lp, d{n}, 8\n        f{n}: index = add l{n}, 5\n        store w{n}, lp, f{n}, 8\n        m{n}: i64 = add ln{n}, 1\n        store m{n}, wp, 32\n", n = n, p = p, ty = ty, name = name, k = k, log = WIRE_LOG));
                n += 1;
            }
        }
        f.push_str("    ret\n");
        t.push_str(&f);
    }
    t
}

/// The client running the server's part again (fm3 cut.md, "the client runs
/// the server's part again"; log 292), as text, where the runner chooses the
/// cut. The server's start, kept as the world begins; the client's log of
/// what it sent the server; the server's part run from its start to now in the server's own
/// slot, the server's real state set aside and put back; its units handed to
/// the client; the move's word, which the server takes without answering; and
/// what the server sent before it heard of it, dropped by its count
fn again_text(w: &World) -> String {
    let (q, pool, nu, size, l) = (WIRE_Q, POOL, w.nu, w.size, AGAIN_LOG);
    let (flight, mem, rows) = (2 * pool, AHEAD_MEM, AHEAD_ROWS);
    let ncols = w.cols.len();
    let mut t = String::new();
    // the fields that cross from the server, as the server's part run again has them
    let mut fl = String::from("\n; the fields of the context that cross from the server, as the part run again has them, taken by the client: what the server's tellings would have brought it\nfn __wagain_fields(row: index)\n    cp: ptr = addr __cut\n    m0: i64 = const 0\n");
    for (k, (j, _, _)) in w.fields.iter().enumerate() {
        fl.push_str(&format!("    f{k}: u8 = load cp, {at}\n    c{k}: u1 = cmp.ne f{k}, 0\n    w{k}: i64 = if c{k}\n        yield {bit}\n    else\n        yield 0\n    m{k1}: i64 = or m{k}, w{k}\n", k = k, k1 = k + 1, at = w.fbase + 2 * j + 1, bit = 1i64 << k));
    }
    fl.push_str(&format!("    __wfields_in(row, m{})\n    ret\n", w.fields.len()));
    t.push_str(&fl);
    // the chooser waits
    t.push_str("\n; is the client waiting on the server at the cut given, with a move left to make: then the chooser's next moment is a moment of the world's, though nothing else is to happen, a wire gone for good holding what the client waits for\nfn __wchoose_waits() -> u1\n    cw: ptr = addr __wc\n    b: i64 = load cw\n    on: u1 = cmp.gt b, 0\n    whr: i64 = load cw, 48\n    given: u1 = cmp.eq whr, 0\n    k: i64 = load cw, 56\n    roomk: u1 = cmp.lt k, 4\n    wp: ptr = addr __w\n    sent: i64 = load wp, 64\n    acked: i64 = load wp, 72\n    told: i64 = load wp, 88\n    nv: i64 = const 4611686018427387904\n    owes: u1 = cmp.lt told, nv\n    unans: u1 = cmp.ne sent, acked\n    waiting: u1 = or owes, unans\n    a0: u1 = and on, given\n    a1: u1 = and a0, roomk\n    w: u1 = and a1, waiting\n    ret w\n");
    // the log, a row in and out
    let mut put = format!("\n; an item of the wire, the row it is in, into the j-th row of the client's log, or back out of it into a row of the wire: what it is, its two words, its own time and rank, the fields it carries, the context they are read from, and the record it carries\nfn __wlog_put(row: index, j: index)\n");
    let mut get = format!("fn __wlog_get(j: index, row: index)\n");
    for (k, (col, lc)) in [("__wq_kind", "__wl_kind"), ("__wq_a", "__wl_a"), ("__wq_b", "__wl_b"), ("__wq_t", "__wl_t"), ("__wq_r", "__wl_r"), ("__wq_c", "__wl_c")].iter().enumerate() {
        put.push_str(&format!("    q{k}: ptr = addr {col}\n    v{k}: i64 = load q{k}, row, 8\n    l{k}: ptr = addr {lc}\n    store v{k}, l{k}, j, 8\n", k = k, col = col, lc = lc));
        get.push_str(&format!("    l{k}: ptr = addr {lc}\n    v{k}: i64 = load l{k}, j, 8\n    q{k}: ptr = addr {col}\n    store v{k}, q{k}, row, 8\n", k = k, col = col, lc = lc));
    }
    let ctx = format!("    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    lm: ptr(array(__ctx, {l})) = addr __wl_ctx\n    le: ptr(__ctx) = index lm, j\n    lp: ptr = cast le\n", q2 = 2 * q, l = l);
    put.push_str(&format!("{ctx}    x: __ctx = load qp\n    store x, lp\n", ctx = ctx));
    get.push_str(&format!("{ctx}    x: __ctx = load lp\n    store x, qp\n", ctx = ctx));
    if w.pooled {
        let head = format!("    fl: index = add row, {flight}\n    lb: index = mul j, {n}\n    lq: ptr = addr __wl_pool\n", flight = flight, n = ncols);
        put.push_str(&head);
        get.push_str(&head);
        for (k, c) in w.cols.iter().enumerate() {
            put.push_str(&format!("    c{k}: ptr = addr {c}\n    w{k}: i64 = load c{k}, fl, 8\n    d{k}: index = add lb, {k}\n    store w{k}, lq, d{k}, 8\n", k = k, c = c));
            get.push_str(&format!("    c{k}: ptr = addr {c}\n    d{k}: index = add lb, {k}\n    w{k}: i64 = load lq, d{k}, 8\n    store w{k}, c{k}, fl, 8\n", k = k, c = c));
        }
    }
    put.push_str("    ret\n");
    get.push_str("    ret\n");
    t.push_str(&put);
    t.push_str(&get);
    // what is logged
    t.push_str(&format!("\n; an item has been put on the wire: at the client, toward the server, it is logged, but a ping and a telling that carries no field; the log full, the client cannot run the server's part again. While the client runs the server's part, what that part sends is looked at instead\nfn __wlogged(row: index)\n    gw: ptr = addr __wg\n    gr: i64 = load gw\n    gon: u1 = cmp.ne gr, 0\n    if gon\n        __wagain_out(row)\n        ret\n    p: index = __wplace()\n    server: u1 = cmp.ne p, 0\n    if server\n        ret\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    cq: ptr = addr __wq_c\n    c: i64 = load cq, row, 8\n    k5: u1 = cmp.eq kind, 5\n    nof: u1 = cmp.eq c, 0\n    plain: u1 = and k5, nof\n    if plain\n        ret\n    n: i64 = load gw, 8\n    room: u1 = cmp.lt n, {l}\n    if room\n        ni: index = conv n\n        __wlog_put(row, ni)\n        n2: i64 = add n, 1\n        store n2, gw, 8\n        ret\n    store 1: i64, gw, 104\n    ret\n", l = l));
    t.push_str(&format!("\n; while the client runs the server's part: a push or a function partway that part sends the client is counted, and one the client has not had from the server is kept after the log, to be taken when the part is handed\nfn __wagain_out(row: index)\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    k3: u1 = cmp.eq kind, 3\n    k4: u1 = cmp.eq kind, 4\n    k34: u1 = or k3, k4\n    if k34\n    else\n        ret\n    gw: ptr = addr __wg\n    gr: i64 = load gw\n    keeping: u1 = cmp.eq gr, 2\n    if keeping\n        ret\n    c: i64 = load gw, 64\n    c2: i64 = add c, 1\n    store c2, gw, 64\n    m: i64 = load gw, 56\n    had: u1 = cmp.le c2, m\n    if had\n        ret\n    n: i64 = load gw, 8\n    o: i64 = load gw, 72\n    j: i64 = add n, o\n    room: u1 = cmp.lt j, {l}\n    if room\n        ji: index = conv j\n        __wlog_put(row, ji)\n        o2: i64 = add o, 1\n        store o2, gw, 72\n        ret\n    store 1: i64, gw, 104\n    ret\n", l = l));
    // what the part run again puts on the wire
    t.push_str(&format!("\n; while the client runs the server's part, an item that part would put on the wire goes nowhere: it is written into a row no one reads, the tail of the way toward the server, which is free, so that what follows can fill it as it would. A piece of text is counted, and kept at the client to be shown where the client has not shown it\nfn __wagain_put(kind: i64, a: i64, b: i64) -> index\n    gw: ptr = addr __wg\n    rw: i64 = load gw, 120\n    row: index = conv rw\n    c: ptr = context()\n    x: __ctx = load c\n    stamp: i64 = get x, __clock\n    ap: ptr = addr __wa\n    rk: i64 = load ap, 88\n    p2: ptr = addr __wq_kind\n    store kind, p2, row, 8\n    p3: ptr = addr __wq_a\n    store a, p3, row, 8\n    p4: ptr = addr __wq_b\n    store b, p4, row, 8\n    p5: ptr = addr __wq_t\n    store stamp, p5, row, 8\n    p6: ptr = addr __wq_r\n    store rk, p6, row, 8\n    p7: ptr = addr __wq_c\n    store 0: i64, p7, row, 8\n    k1: u1 = cmp.eq kind, 1\n    k2: u1 = cmp.eq kind, 2\n    tx: u1 = or k1, k2\n    if tx\n        __wagain_text(kind, a, b, stamp, rk)\n    ret row\n\n; a piece of text the part run again writes: one of those the client has shown already is not shown again, and a block of it gives its room back; one it has not shown is kept, at its own time, and shown when the client comes to it, late where it is\nfn __wagain_text(kind: i64, a: i64, b: i64, t: i64, r: i64)\n    gw: ptr = addr __wg\n    gr: i64 = load gw\n    early: u1 = cmp.eq gr, 2\n    if early\n        eb: u1 = cmp.eq kind, 2\n        if eb\n            ewp: ptr = addr __w\n            eo: i64 = load ewp, 40\n            eo2: i64 = sub eo, b\n            store eo2, ewp, 40\n        ret\n    c: i64 = load gw, 24\n    c2: i64 = add c, 1\n    store c2, gw, 24\n    m: i64 = load gw, 16\n    had: u1 = cmp.le c2, m\n    if had\n        blk: u1 = cmp.eq kind, 2\n        if blk\n            wp: ptr = addr __w\n            o: i64 = load wp, 40\n            o2: i64 = sub o, b\n            store o2, wp, 40\n        ret\n    nv: i64 = const {never}\n    __wkeep(kind, a, b, t, r, nv)\n    n: i64 = load gw, 96\n    n2: i64 = add n, 1\n    store n2, gw, 96\n    ret\n", never = NEVER));
    // the server's start
    let start_pool = if w.pooled { format!("    an: index = get x, __act_n\n    few: u1 = cmp.le an, {rows}\n    ok: u1 = and fits, few\n", rows = rows) } else { "    ok: u1 = and fits, fits\n".to_string() };
    t.push_str(&format!("\n; the server's start (fm3 log 292), as the world begins and before anything has crossed: its context, its half of the pool and its part of the arena into the copy after working ahead's, and the cut's table as it is given. The server's state is a function of this and of what the client sends it. Where it does not fit a copy, the client cannot run the server's part\nfn __wstart()\n    gw: ptr = addr __wg\n    loop(i: index = 0) bound 32\n        done: u1 = cmp.ge i, 32\n        if done\n            break\n        store 0: i64, gw, i, 8\n        i2: index = add i, 1\n        continue i2\n    c: ptr = context()\n    x: __ctx = load c\n    sm: ptr(array(__ctx, 2)) = addr __wg_ctx\n    s16: index = const {a}\n    z0: index = const 0\n    se: ptr(__ctx) = index sm, z0\n    sp: ptr = cast se\n    store x, sp\n    shm: ptr(array(__ctx, 2)) = addr __wg_shadow\n    she: ptr(__ctx) = index shm, z0\n    shp: ptr = cast she\n    store x, shp\n    ap: ptr = addr __wa\n    mlen: index = load ap, 120\n    fits: u1 = cmp.le mlen, {mem}\n{start_pool}    if ok\n        no: u1 = cmp.ne s16, s16\n{pool_out}        __wmem(s16, no)\n    else\n        store 1: i64, gw, 104\n    cp: ptr = addr __cut\n    gc: ptr = addr __wg_cut\n    loop(j: index = 0) bound {size}\n        jd: u1 = cmp.ge j, {size}\n        if jd\n            break\n        b: u8 = load cp, j, 1\n        store b, gc, j, 1\n        j2: index = add j, 1\n        continue j2\n    ret\n", a = AHEAD, mem = mem, size = size, start_pool = start_pool, pool_out = if w.pooled { "        __wpool(s16, an, no)\n" } else { "" }));
    // can it
    let rows_ok = if w.pooled { format!("    cm: ptr(array(__ctx, 2)) = addr __ctx_mem\n    o1: index = const 1\n    fe: ptr(__ctx) = index cm, o1\n    fp: ptr = cast fe\n    fx: __ctx = load fp\n    fan: index = get fx, __act_n\n    few: u1 = cmp.le fan, {rows}\n", rows = rows) } else { "    few: u1 = and fits, fits\n".to_string() };
    t.push_str(&format!("\n; can the client run the server's part now: its log whole, the server's part of the arena and its places of the pool within a copy, and a row of the way toward the server free to run it through\nfn __wagain_ok() -> u1\n    gw: ptr = addr __wg\n    ov: i64 = load gw, 104\n    whole: u1 = cmp.eq ov, 0\n    ap: ptr = addr __wa\n    mlen: index = load ap, 120\n    fits: u1 = cmp.le mlen, {mem}\n    wp: ptr = addr __w\n    h1: i64 = load wp, 208\n    t1: i64 = load wp, 216\n    used: i64 = sub t1, h1\n    roomw: u1 = cmp.lt used, {q}\n{rows_ok}    a0: u1 = and whole, fits\n    a1: u1 = and a0, roomw\n    ok: u1 = and a1, few\n    ret ok\n", mem = mem, q = q, rows_ok = rows_ok));
    // taking a logged item, as the server
    let take4 = if w.pooled { format!("    is4: u1 = cmp.eq kind, 4\n    if is4\n        __wfields_in(row, c)\n        slot: index = __act_slot()\n        bp: ptr = addr __act_born\n        born: i64 = load bp, slot, 8\n        fl: index = add row, {flight}\n        __act_carry(fl, slot)\n        store born, bp, slot, 8\n        pp: ptr = addr __act_pc\n        store a, pp, slot, 8\n        __act_run(slot, a)\n        __act_next()\n        ret\n", flight = flight) } else { String::new() };
    let take12 = if w.pooled { "    is12: u1 = cmp.eq kind, 12\n    if is12\n        __wtake(row)\n        ret\n" } else { "" };
    let run = if w.runs { "        __run()\n" } else { "" };
    t.push_str(&format!("\n; while the client runs the server's part, the j-th item of its log is given to it, through the row, as `__wdeliver` gives an item to the server: a move's word with the table as the client's move turn left it, a function partway handed, and a push, a function partway or a telling with its fields at its own time and rank\nfn __wagain_take(j: index, row: index)\n    __wlog_get(j, row)\n    lk: ptr = addr __wl_kind\n    kind: i64 = load lk, j, 8\n    la: ptr = addr __wl_a\n    a: i64 = load la, j, 8\n    lb: ptr = addr __wl_b\n    b: i64 = load lb, j, 8\n    lt: ptr = addr __wl_t\n    stamp: i64 = load lt, j, 8\n    lr: ptr = addr __wl_r\n    rk: i64 = load lr, j, 8\n    lc: ptr = addr __wl_c\n    c: i64 = load lc, j, 8\n    wp: ptr = addr __w\n    store 0: i64, wp, 56\n    is10: u1 = cmp.eq kind, 10\n    if is10\n        ka: index = conv a\n        cp: ptr = addr __cut\n        np: ptr = addr __cuts\n        base: index = mul ka, {size}\n        loop(i: index = 0) bound {nu}\n            done: u1 = cmp.ge i, {nu}\n            if done\n                break\n            ji: index = add base, i\n            nb: u8 = load np, ji, 1\n            ob: u8 = load cp, i, 1\n            was: u1 = cmp.eq ob, 0\n            to: u1 = cmp.eq nb, 1\n            goes: u1 = and was, to\n            if goes\n                store 2: u8, cp, i, 1\n            i2: index = add i, 1\n            continue i2\n        __wmove_server(row, ka, b)\n        ret\n{take12}    is17: u1 = cmp.eq kind, 17\n    if is17\n        kb: index = conv a\n        __wagain_server(row, kb)\n        ret\n    ap: ptr = addr __wa\n    store rk, ap, 88\n    __wclock(stamp)\n    is3: u1 = cmp.eq kind, 3\n    if is3\n        __wire_push(a, b)\n{run}        ret\n{take4}    is5: u1 = cmp.eq kind, 5\n    if is5\n        __wfields_in(row, c)\n    ret\n", size = size, nu = nu, take12 = take12, take4 = take4, run = run));
    // the server hears of the move
    let drop = if w.pooled { "    __wdrop_moving()\n" } else { "" };
    t.push_str(&format!("\n; the move's word where the client has taken the server's part has reached the server (fm3 log 292): each unit the server had that the k-th table gives the client is nowhere for a moment, the server's records of it are dropped, and it is the client's; the server's own bytes of the table are the new one's; and the server's count of such moves is one more. The server sends nothing back: the client has its state\nfn __wagain_server(row: index, k: index)\n    cp: ptr = addr __cut\n    np: ptr = addr __cuts\n    base: index = mul k, {size}\n    loop(i: index = 0) bound {nu}\n        done: u1 = cmp.ge i, {nu}\n        if done\n            break\n        j: index = add base, i\n        nb: u8 = load np, j, 1\n        ob: u8 = load cp, i, 1\n        o1: u1 = cmp.eq ob, 1\n        o3: u1 = cmp.eq ob, 3\n        was: u1 = or o1, o3\n        to: u1 = cmp.eq nb, 0\n        goes: u1 = and was, to\n        if goes\n            store 2: u8, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n{drop}    loop(g: index = 0) bound {nu}\n        gd: u1 = cmp.ge g, {nu}\n        if gd\n            break\n        gb: u8 = load cp, g, 1\n        two: u1 = cmp.eq gb, 2\n        if two\n            store 0: u8, cp, g, 1\n        g2: index = add g, 1\n        continue g2\n    loop(m: index = {nu1}) bound {size}\n        done2: u1 = cmp.ge m, {size}\n        if done2\n            break\n        j2: index = add base, m\n        b2: u8 = load np, j2, 1\n        store b2, cp, m, 1\n        m2: index = add m, 2\n        continue m2\n    gw: ptr = addr __wg\n    f: i64 = load gw, 48\n    f2: i64 = add f, 1\n    store f2, gw, 48\n    ret\n", size = size, nu = nu, nu1 = nu + 1, drop = drop));
    if w.pooled {
        t.push_str(&format!("\n; the server's records of the units it no longer has are dropped, and its place for each is free\nfn __wdrop_moving()\n    c: ptr = context()\n    x: __ctx = load c\n    n: index = get x, __act_n\n    p: index = __wplace()\n    base: index = mul p, {pool}\n    top: index = add base, n\n    pp: ptr = addr __act_pc\n    dp: ptr = addr __act_due\n    ap: ptr = addr __wa\n    loop(s: index = base) bound {pool1}\n        done: u1 = cmp.ge s, top\n        if done\n            break\n        pc: i64 = load pp, s, 8\n        mv: u1 = __wmoving(pc)\n        if mv\n            store {never}: i64, dp, s, 8\n            store 0: i64, pp, s, 8\n            pk: i64 = load ap, 16\n            sw: i64 = conv s\n            s1: i64 = add sw, 1\n            hit: u1 = cmp.eq pk, s1\n            if hit\n                store 0: i64, ap, 16\n                store -1: i64, ap, 24\n        s2: index = add s, 1\n        continue s2\n    __act_next()\n    ret\n\n; the part run again handed to the client: each record of the server's half whose unit leaves the server, the oldest first, into a place of the client's half, as a move with no wire hands one\nfn __wagain_hand()\n    cm: ptr(array(__ctx, 2)) = addr __ctx_mem\n    o1: index = const 1\n    fe: ptr(__ctx) = index cm, o1\n    fp: ptr = cast fe\n    fx: __ctx = load fp\n    n: index = get fx, __act_n\n    top: index = add n, {pool}\n    pp: ptr = addr __act_pc\n    bp: ptr = addr __act_born\n    dp: ptr = addr __act_due\n    loop(j: index = 0) bound {pool1}\n        best: index = loop(s: index = {pool}, i: index = top, o: i64 = {never}) bound {pool1}\n            done: u1 = cmp.ge s, top\n            if done\n                break i\n            pc: i64 = load pp, s, 8\n            mv: u1 = __wmoving(pc)\n            w: i64 = load bp, s, 8\n            older: u1 = cmp.lt w, o\n            take: u1 = and mv, older\n            i2: index = if take\n                yield s\n            else\n                yield i\n            o2: i64 = if take\n                yield w\n            else\n                yield o\n            s2: index = add s, 1\n            continue s2, i2, o2\n        none: u1 = cmp.eq best, top\n        if none\n            break\n        __zero_context(0)\n        slot: index = __act_slot()\n        born: i64 = load bp, slot, 8\n        __act_carry(best, slot)\n        store born, bp, slot, 8\n        store {never}: i64, dp, best, 8\n        store 0: i64, pp, best, 8\n        j2: index = add j, 1\n        continue j2\n    __zero_context(0)\n    __act_next()\n    ret\n", pool = pool, pool1 = pool + 1, never = NEVER));
    }
    // what the server's part sent the client that it had not had
    let out4 = if w.pooled { format!("        is4: u1 = cmp.eq kind, 4\n        if is4\n            slot: index = __act_slot()\n            bp: ptr = addr __act_born\n            born: i64 = load bp, slot, 8\n            fl: index = add row, {flight}\n            __act_carry(fl, slot)\n            store born, bp, slot, 8\n            pp: ptr = addr __act_pc\n            store a, pp, slot, 8\n            __act_run(slot, a)\n            __act_next()\n", flight = flight) } else { String::new() };
    t.push_str(&format!("\n; what the part run again sent the client, a push or a function partway, that the client had not had from the server: taken now, at the client, as `__wdeliver` takes one, in order, but for the fields it carried, which the hand has given the client as they are now\nfn __wagain_outbox(row: index)\n    gw: ptr = addr __wg\n    n: i64 = load gw, 8\n    o: i64 = load gw, 72\n    e: i64 = add n, o\n    loop(j: i64 = n) bound {l}\n        done: u1 = cmp.ge j, e\n        if done\n            break\n        ji: index = conv j\n        __wlog_get(ji, row)\n        lk: ptr = addr __wl_kind\n        kind: i64 = load lk, ji, 8\n        la: ptr = addr __wl_a\n        a: i64 = load la, ji, 8\n        lb: ptr = addr __wl_b\n        b: i64 = load lb, ji, 8\n        lt: ptr = addr __wl_t\n        stamp: i64 = load lt, ji, 8\n        lr: ptr = addr __wl_r\n        rk: i64 = load lr, ji, 8\n        lc: ptr = addr __wl_c\n        c: i64 = load lc, ji, 8\n        __zero_context(0)\n        __wflush(stamp, rk)\n        ap: ptr = addr __wa\n        store rk, ap, 80\n        __wclock(stamp)\n        is3: u1 = cmp.eq kind, 3\n        if is3\n            __wire_push(a, b)\n{run}{out4}        tk: i64 = load gw, 56\n        tk2: i64 = add tk, 1\n        store tk2, gw, 56\n        j2: i64 = add j, 1\n        continue j2\n    ret\n", l = l, run = if w.runs { "            __run()\n" } else { "" }, out4 = out4));
    // an item the server sent before it heard of the move
    t.push_str("\n; an item from the server made before the server heard of the last move the client made by running the server's part (fm3 log 292): dropped where it arrives, and a line of the wire's record says so. It is counted as had where it would have been, so that the server is not left waiting for its answer\nfn __wold(row: index, kind: i64, a: i64, stamp: i64)\n    k3: u1 = cmp.eq kind, 3\n    k4: u1 = cmp.eq kind, 4\n    k11: u1 = cmp.eq kind, 11\n    k5: u1 = cmp.eq kind, 5\n    cq: ptr = addr __wq_c\n    c: i64 = load cq, row, 8\n    fed: u1 = cmp.ne c, 0\n    k5f: u1 = and k5, fed\n    k34: u1 = or k3, k4\n    k3411: u1 = or k34, k11\n    counted: u1 = or k3411, k5f\n    if counted\n        __wrecv()\n    gw: ptr = addr __wg\n    d: i64 = load gw, 88\n    d2: i64 = add d, 1\n    store d2, gw, 88\n    k1: u1 = cmp.eq kind, 1\n    a16: i64 = mul a, 16\n    ch: i64 = add a16, kind\n    said: i64 = if k1\n        yield ch\n    else\n        yield kind\n    __wnote(19, stamp, said)\n    ret\n");
    // the move
    let set_aside = if w.pooled { "    fan: index = get fx, __act_n\n    __wpool(s17, fan, no)\n" } else { "" };
    let put_start = if w.pooled { "        an16: index = get x16, __act_n\n        __wpool(s16, an16, yes)\n" } else { "" };
    let put_back = if w.pooled { "    __wpool(s17, fan, yes)\n" } else { "" };
    let hand = if w.pooled { "    __wagain_hand()\n" } else { "" };
    let start_run = if w.runs { "    sw: i64 = load gw, 152\n    wor: u1 = cmp.eq sw, 0\n    rw0: u1 = and right, wor\n    if rw0\n        __run()\n" } else { "" };
    t.push_str(&format!("\n; the client takes the server's part by running it again (fm3 log 292), at the chooser's moment: the pieces it keeps from the server are its own turns' to write now; the server's real state is set aside, the server's start put in its place, and the server's turns taken from its start to now, or to the client's own earliest thing where that is sooner, each logged item given to it at its own time; what the part run again has is handed to the client as a move from the server hands it; the server's real state is put back as it was; the units are the client's, and the server's too until it hears of it; and the move's word is put on the wire, which the server takes without answering. The client counts it as answered, and the server as owing nothing. `__w`'s fourth word, 128, is a test's: the log is not given; 512: the server's part is run again from where the server is, not from its start\nfn __wagain_move()\n    gw: ptr = addr __wg\n    ap: ptr = addr __wa\n    wp: ptr = addr __w\n    now: i64 = load wp\n    __zero_context(0)\n    sc: ptr = context()\n    sx: __ctx = load sc\n    k: index = get sx, __move_k\n    sbt: i64, sbr: i64 = __list_next()\n    nlt: u1 = cmp.lt now, sbt\n    neq: u1 = cmp.eq now, sbt\n    nrl: u1 = cmp.lt {rank}, sbr\n    nti: u1 = and neq, nrl\n    nfirst: u1 = or nlt, nti\n    bt: i64 = if nfirst\n        yield now\n    else\n        yield sbt\n    br: i64 = if nfirst\n        yield {rank}\n    else\n        yield sbr\n    t1: i64 = load wp, 216\n    m1: i64 = rem t1, {q}\n    mi: index = conv m1\n    row: index = add mi, {q}\n    rw: i64 = conv row\n    store rw, gw, 120\n    kn0: i64 = load ap, 104\n    store kn0, gw, 112\n    store 0: i64, ap, 104\n    __zero_context(1)\n    fc: ptr = context()\n    fx: __ctx = load fc\n    sm: ptr(array(__ctx, 2)) = addr __wg_ctx\n    s16: index = const {a}\n    s17: index = const {a1}\n    c0: index = const 0\n    c1: index = const 1\n    e17: ptr(__ctx) = index sm, c1\n    p17: ptr = cast e17\n    store fx, p17\n    no: u1 = cmp.ne s16, s16\n    yes: u1 = cmp.eq s16, s16\n{set_aside}    __wmem(s17, no)\n    cp: ptr = addr __cut\n    gn: ptr = addr __wg_now\n    gc: ptr = addr __wg_cut\n    loop(i: index = 0) bound {size}\n        idone: u1 = cmp.ge i, {size}\n        if idone\n            break\n        ib: u8 = load cp, i, 1\n        store ib, gn, i, 1\n        ig: u8 = load gc, i, 1\n        store ig, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n    shm: ptr(array(__ctx, 2)) = addr __wshadow\n    z0: index = const 0\n    z1: index = const 1\n    sh0e: ptr(__ctx) = index shm, z0\n    sh0p: ptr = cast sh0e\n    sh0: __ctx = load sh0p\n    sh1e: ptr(__ctx) = index shm, z1\n    sh1p: ptr = cast sh1e\n    sh1: __ctx = load sh1p\n    ksm: ptr(array(__ctx, 2)) = addr __wg_shadow\n    k1e: ptr(__ctx) = index ksm, z1\n    k1p: ptr = cast k1e\n    store sh1, k1p\n    k0e: ptr(__ctx) = index ksm, z0\n    k0p: ptr = cast k0e\n    st0: __ctx = load k0p\n    store st0, sh1p\n    g7: i64 = load wp, 56\n    r88: i64 = load ap, 88\n    f48: i64 = load gw, 48\n    but: i64 = load wp, 24\n    bw: i64 = and but, 512\n    right: u1 = cmp.eq bw, 0\n    if right\n        e16: ptr(__ctx) = index sm, c0\n        p16: ptr = cast e16\n        x16: __ctx = load p16\n        store x16, fc\n{put_start}        __wmem(s16, yes)\n    store 1: i64, gw\n    store 0: i64, gw, 24\n    store 0: i64, gw, 64\n    store 0: i64, gw, 72\n    store 0: i64, gw, 96\n    store 0: i64, ap\n{start_run}    nl: i64 = load gw, 8\n    bl: i64 = and but, 128\n    blind: u1 = cmp.ne bl, 0\n    n: i64 = if blind\n        yield 0\n    else\n        yield nl\n    lt: ptr = addr __wl_t\n    lr: ptr = addr __wl_r\n    lmax: i64 = const {l1}\n    turns: i64 = loop(j: i64 = 0, tn: i64 = 0)\n        lim: u1 = cmp.lt tn, 4000000\n        check lim\n        __zero_context(1)\n        t: i64, r: i64 = __list_next()\n        has: u1 = cmp.lt j, n\n        jl: i64 = min(j, lmax)\n        jc: index = conv jl\n        st: i64 = load lt, jc, 8\n        sr: i64 = load lr, jc, 8\n        sb: u1 = cmp.lt st, t\n        sq: u1 = cmp.eq st, t\n        sle: u1 = cmp.le sr, r\n        stie: u1 = and sq, sle\n        sfirst: u1 = or sb, stie\n        item: u1 = and has, sfirst\n        if item\n            __wagain_take(jc, row)\n            j2: i64 = add j, 1\n            continue j2, tn\n        dlt: u1 = cmp.lt t, bt\n        deq: u1 = cmp.eq t, bt\n        drl: u1 = cmp.lt r, br\n        dti: u1 = and deq, drl\n        due: u1 = or dlt, dti\n        if due\n            store 0: i64, wp, 56\n            store r, ap, 88\n            __list_turn()\n            tn2: i64 = add tn, 1\n            continue j, tn2\n        if has\n            __wagain_take(jc, row)\n            j3: i64 = add j, 1\n            continue j3, tn\n        break tn\n    store turns, gw, 80\n    np: ptr = addr __cuts\n    base: index = mul k, {size}\n    loop(u: index = 0) bound {nu}\n        udone: u1 = cmp.ge u, {nu}\n        if udone\n            break\n        uj: index = add base, u\n        unb: u8 = load np, uj, 1\n        uob: u8 = load cp, u, 1\n        uf: u1 = cmp.eq uob, 1\n        us: u1 = cmp.eq unb, 0\n        ugo: u1 = and uf, us\n        if ugo\n            store 2: u8, cp, u, 1\n        u2: index = add u, 1\n        continue u2\n    __zero_context(1)\n    hx: __ctx = load fc\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    store hx, qp\n    store sh0, sh0p\n    __zero_context(0)\n    __whand(row, k, 2)\n    __wagain_fields(row)\n{hand}    __zero_context(1)\n    x17: __ctx = load p17\n    store x17, fc\n{put_back}    __wmem(s17, yes)\n    loop(v: index = 0) bound {size}\n        vdone: u1 = cmp.ge v, {size}\n        if vdone\n            break\n        vb: u8 = load gn, v, 1\n        store vb, cp, v, 1\n        v2: index = add v, 1\n        continue v2\n    sh1b: __ctx = load k1p\n    store sh1b, sh1p\n    store g7, wp, 56\n    store r88, ap, 88\n    store f48, gw, 48\n    store 0: i64, gw\n    loop(y: index = 0) bound {nu}\n        ydone: u1 = cmp.ge y, {nu}\n        if ydone\n            break\n        yj: index = add base, y\n        ynb: u8 = load np, yj, 1\n        yob: u8 = load cp, y, 1\n        yf1: u1 = cmp.eq yob, 1\n        yf2: u1 = cmp.eq yob, 2\n        yf: u1 = or yf1, yf2\n        ys: u1 = cmp.eq ynb, 0\n        ygo: u1 = and yf, ys\n        if ygo\n            store 3: u8, cp, y, 1\n        y2: index = add y, 1\n        continue y2\n    loop(m: index = {nu}) bound {size}\n        mdone: u1 = cmp.ge m, {size}\n        if mdone\n            break\n        mj: index = add base, m\n        mb: u8 = load np, mj, 1\n        store mb, cp, m, 1\n        m2: index = add m, 2\n        continue m2\n    __zero_context(0)\n    __wagain_outbox(row)\n    __zero_context(0)\n    z: __ctx = load sc\n    k2: index = add k, 1\n    tp: ptr = addr __move_t\n    d: i64 = load tp, k2, 8\n    z2: __ctx = set z, __move_k, k2\n    z3: __ctx = set z2, __due_move, d\n    store z3, sc\n    store {rank}: i64, ap, 80\n    kw: i64 = conv k\n    row2: index = __wput(17, kw, now, kw)\n    __wstamp(row2, now)\n    rq: ptr = addr __wq_r\n    store {rank}: i64, rq, row2, 8\n    qe2: ptr(__ctx) = index qm, row2\n    qp2: ptr = cast qe2\n    store z3, qp2\n    __wlogged(row2)\n    __wsent_move(now)\n    sn: i64 = load wp, 64\n    store sn, wp, 72\n    nv: i64 = const {never}\n    store nv, wp, 88\n    store nv, wp, 96\n    cn: i64 = load gw, 32\n    cn2: i64 = add cn, 1\n    store cn2, gw, 32\n    store 1: i64, gw, 128\n    o: i64 = load gw, 72\n    kept: i64 = load gw, 96\n    s1: i64 = mul n, 1048576\n    s3: i64 = mul o, 1099511627776\n    sa: i64 = add turns, s1\n    said: i64 = add sa, s3\n    __wnote(18, now, said)\n    kc: i64 = load gw, 112\n    s2: i64 = mul kc, 1048576\n    said2: i64 = add kept, s2\n    __wnote(20, now, said2)\n    __act_next()\n    ret\n", q = q, q2 = 2 * q, a = AHEAD, a1 = AHEAD + 1, size = size, nu = nu, l1 = l - 1, set_aside = set_aside, put_start = put_start, put_back = put_back, hand = hand, start_run = start_run, rank = RANK, never = NEVER));
    // the server's new start at a giving back (fm3 log 298, question 168)
    let (k_aside, k_start, k_keep, k_back) = if w.pooled {
        ("    fan: index = get fx, __act_n\n    __wpool(s17, fan, no)\n", "    an16: index = get x16, __act_n\n    __wpool(s16, an16, yes)\n", "        __wpool(s16, kan, no)\n", "    __wpool(s17, fan, yes)\n")
    } else {
        ("", "", "", "")
    };
    let k_fits = if w.pooled { format!("    kan: index = get kx, __act_n\n    few: u1 = cmp.le kan, {rows}\n    ok: u1 = and fits, few\n", rows = rows) } else { "    ok: u1 = and fits, fits\n".to_string() };
    let k_run = if w.runs { "    sw: i64 = load gw, 152\n    wor: u1 = cmp.eq sw, 0\n    if wor\n        __run()\n" } else { "" };
    t.push_str(&format!("\n; the server's new start at a giving back (fm3 log 298, question 168): the server's part is run again in the server's own slot, from the server's start through the whole log, which ends with what this move hands, and the server's turns taken for times before the move's. What the server's slot then holds is the server's state as the server will have it once it takes the hand, and it is the server's start: its context, its half of the pool, its part of the arena, the table and its notes of the fields. The log is emptied, and the client's counts of the server's text and items are from here. Nothing the part run so writes is shown, sent or counted (`__wg`'s first word 2), and the server's real state is put back byte for byte. Where the log has run out, the start would not fit a copy, or no row toward the server is free, the client cannot run the server's part again. `__w`'s fourth word, 4096, is a test's: the server's start is not moved\nfn __wagain_keep(t: i64)\n    gw: ptr = addr __wg\n    ap: ptr = addr __wa\n    wp: ptr = addr __w\n    kq: i64 = load gw, 144\n    asked: u1 = cmp.ne kq, 0\n    if asked\n    else\n        ret\n    store 0: i64, gw, 144\n    but: i64 = load wp, 24\n    bk: i64 = and but, 4096\n    skip: u1 = cmp.ne bk, 0\n    if skip\n        ret\n    ov: i64 = load gw, 104\n    lost: u1 = cmp.ne ov, 0\n    if lost\n        ret\n    h1: i64 = load wp, 208\n    t1: i64 = load wp, 216\n    used: i64 = sub t1, h1\n    roomw: u1 = cmp.lt used, {q}\n    if roomw\n    else\n        store 1: i64, gw, 104\n        ret\n    m1: i64 = rem t1, {q}\n    mi: index = conv m1\n    row: index = add mi, {q}\n    rw: i64 = conv row\n    store rw, gw, 120\n    __zero_context(1)\n    fc: ptr = context()\n    fx: __ctx = load fc\n    sm: ptr(array(__ctx, 2)) = addr __wg_ctx\n    s16: index = const {a}\n    s17: index = const {a1}\n    c0: index = const 0\n    c1: index = const 1\n    e17: ptr(__ctx) = index sm, c1\n    p17: ptr = cast e17\n    store fx, p17\n    e16: ptr(__ctx) = index sm, c0\n    p16: ptr = cast e16\n    no: u1 = cmp.ne s16, s16\n    yes: u1 = cmp.eq s16, s16\n{k_aside}    __wmem(s17, no)\n    cp: ptr = addr __cut\n    gn: ptr = addr __wg_now\n    gc: ptr = addr __wg_cut\n    loop(i: index = 0) bound {size}\n        idone: u1 = cmp.ge i, {size}\n        if idone\n            break\n        ib: u8 = load cp, i, 1\n        store ib, gn, i, 1\n        ig: u8 = load gc, i, 1\n        store ig, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n    shm: ptr(array(__ctx, 2)) = addr __wshadow\n    z0: index = const 0\n    z1: index = const 1\n    sh0e: ptr(__ctx) = index shm, z0\n    sh0p: ptr = cast sh0e\n    sh0: __ctx = load sh0p\n    sh1e: ptr(__ctx) = index shm, z1\n    sh1p: ptr = cast sh1e\n    sh1: __ctx = load sh1p\n    ksm: ptr(array(__ctx, 2)) = addr __wg_shadow\n    k1e: ptr(__ctx) = index ksm, z1\n    k1p: ptr = cast k1e\n    store sh1, k1p\n    k0e: ptr(__ctx) = index ksm, z0\n    k0p: ptr = cast k0e\n    st0: __ctx = load k0p\n    store st0, sh1p\n    g7: i64 = load wp, 56\n    r88: i64 = load ap, 88\n    f48: i64 = load gw, 48\n    a0: i64 = load ap\n    x16: __ctx = load p16\n    store x16, fc\n{k_start}    __wmem(s16, yes)\n    store 2: i64, gw\n    store 0: i64, ap\n{k_run}    n: i64 = load gw, 8\n    lt: ptr = addr __wl_t\n    lr: ptr = addr __wl_r\n    lmax: i64 = const {l1}\n    turns: i64 = loop(j: i64 = 0, tn: i64 = 0)\n        lim: u1 = cmp.lt tn, 4000000\n        check lim\n        __zero_context(1)\n        ft: i64, fr: i64 = __list_next()\n        has: u1 = cmp.lt j, n\n        jl: i64 = min(j, lmax)\n        jc: index = conv jl\n        st: i64 = load lt, jc, 8\n        sr: i64 = load lr, jc, 8\n        sb: u1 = cmp.lt st, ft\n        sq: u1 = cmp.eq st, ft\n        sle: u1 = cmp.le sr, fr\n        stie: u1 = and sq, sle\n        sfirst: u1 = or sb, stie\n        item: u1 = and has, sfirst\n        if item\n            __wagain_take(jc, row)\n            j2: i64 = add j, 1\n            continue j2, tn\n        due: u1 = cmp.lt ft, t\n        if due\n            store 0: i64, wp, 56\n            store fr, ap, 88\n            __list_turn()\n            tn2: i64 = add tn, 1\n            continue j, tn2\n        if has\n            __wagain_take(jc, row)\n            j3: i64 = add j, 1\n            continue j3, tn\n        break tn\n    __zero_context(1)\n    kx: __ctx = load fc\n    mlen: index = load ap, 120\n    fits: u1 = cmp.le mlen, {mem}\n{k_fits}    if ok\n        store kx, p16\n{k_keep}        __wmem(s16, no)\n        loop(u: index = 0) bound {size}\n            udone: u1 = cmp.ge u, {size}\n            if udone\n                break\n            ub: u8 = load cp, u, 1\n            store ub, gc, u, 1\n            u2: index = add u, 1\n            continue u2\n        ks: __ctx = load sh1p\n        store ks, k0p\n        store 0: i64, gw, 8\n        store 0: i64, gw, 16\n        store 0: i64, gw, 56\n        store 1: i64, gw, 152\n    else\n        store 1: i64, gw, 104\n    store sh0, sh0p\n    x17: __ctx = load p17\n    store x17, fc\n{k_back}    __wmem(s17, yes)\n    loop(v: index = 0) bound {size}\n        vdone: u1 = cmp.ge v, {size}\n        if vdone\n            break\n        vb: u8 = load gn, v, 1\n        store vb, cp, v, 1\n        v2: index = add v, 1\n        continue v2\n    sh1b: __ctx = load k1p\n    store sh1b, sh1p\n    store g7, wp, 56\n    store r88, ap, 88\n    store f48, gw, 48\n    store a0, ap\n    store 0: i64, gw\n    __zero_context(0)\n    s1: i64 = mul n, 1048576\n    said: i64 = add turns, s1\n    kept: u1 = cmp.eq ok, 1\n    if kept\n        __wnote(26, t, said)\n    ret\n", q = q, a = AHEAD, a1 = AHEAD + 1, size = size, l1 = l - 1, mem = mem, k_aside = k_aside, k_start = k_start, k_keep = k_keep, k_back = k_back, k_fits = k_fits, k_run = k_run));
    t
}

/// The chooser (fm3 cut.md, "the cut chosen"; log 290), as text: the
/// client's, between the cut given and nothing on the server, by round trips of
/// its own and what the server's last telling said, and never the wire's
/// table. It makes a move due now, and the move is hop forty-nine's
fn choose_text(w: &World) -> String {
    format!("\n; the chooser, at the client (fm3 log 290). The server's part could be later than the bound where the client is waiting on the server, the server having told it of something still to do or not having answered what it sent, and a round trip of its own has been out longer than the bound: then nothing on the server, a move made due now. It could not, where every round trip begun in the last whole second came back within the bound: then the cut given back, where nothing on the client's list is due within the last round trip measured. And a round trip of its own, one out at a time, a bound after the last, where the client is waiting on the server at the cut given, or has something still to do at nothing on the server. Four moves at most\nfn __wchoose() -> u1\n    __zero_context(0)\n    cw: ptr = addr __wc\n    b: i64 = load cw\n    on: u1 = cmp.gt b, 0\n    if on\n    else\n        ret 0\n    wp: ptr = addr __w\n    now: i64 = load wp\n    out: i64 = load cw, 24\n    ls: i64 = load cw, 16\n    dl: i64 = add ls, b\n    over: u1 = cmp.ge now, dl\n    isout: u1 = cmp.ne out, 0\n    late: u1 = and isout, over\n    nv: i64 = const {never}\n    if late\n        store nv, cw, 32\n        __wgap_begin(ls)\n    whr: i64 = load cw, 48\n    k: i64 = load cw, 56\n    roomk: u1 = cmp.lt k, {moves}\n    c: ptr = context()\n    x: __ctx = load c\n    dm: i64 = get x, __due_move\n    idle: u1 = cmp.ge dm, nv\n    free: u1 = and roomk, idle\n    sent: i64 = load wp, 64\n    acked: i64 = load wp, 72\n    told: i64 = load wp, 88\n    owes: u1 = cmp.lt told, nv\n    unans: u1 = cmp.ne sent, acked\n    waiting: u1 = or owes, unans\n    given: u1 = cmp.eq whr, 0\n    g1: u1 = and late, given\n    g2: u1 = and g1, waiting\n    go1: u1 = and g2, free\n    if go1\n        __wtaken(now)\n        x1: __ctx = set x, __due_move, now\n        store x1, c\n        store 1: i64, cw, 48\n        k1: i64 = add k, 1\n        store k1, cw, 56\n        __wnote(15, now, b)\n        ret 1\n    gs: i64 = load cw, 32\n    sty: i64 = __wsteady(now)\n    gsec: i64 = add gs, sty\n    good0: u1 = cmp.lt gs, nv\n    good1: u1 = cmp.ge now, gsec\n    good: u1 = and good0, good1\n    away: u1 = cmp.eq whr, 1\n    h1: u1 = and good, away\n    h2: u1 = and h1, free\n    t9: i64, r9: i64 = __list_next()\n    __wkept(now, gs, h2)\n    if h2\n        rt: i64 = load cw, 40\n        mt: i64 = __wmove_time()\n        lr: i64 = add rt, mt\n        lim: i64 = add now, lr\n        clr: u1 = cmp.gt t9, lim\n        spok: u1 = __wspeed_ok()\n        clear: u1 = and clr, spok\n        slow: u1 = xor spok, 1\n        held: u1 = and clr, slow\n        if held\n            __wkept_slow(now)\n        if clear\n            __wgiven(now)\n            x2: __ctx = set x, __due_move, now\n            store x2, c\n            store 0: i64, cw, 48\n            k2: i64 = add k, 1\n            store k2, cw, 56\n            __wnote(16, sty, lr)\n            ret 1\n    mine: u1 = cmp.lt t9, nv\n    keep: u1 = and away, mine\n    hold: u1 = and given, waiting\n    need: u1 = or keep, hold\n    none: u1 = xor isout, 1\n    p1: u1 = and need, none\n    p2: u1 = and p1, over\n    if p2\n        n: i64 = load cw, 8\n        n2: i64 = add n, 1\n        row: index = __wput(13, n2, now, n2)\n        if away\n            m2: i64 = sub 0, n2\n            row2: index = __wput(13, m2, now, m2)\n        store n2, cw, 8\n        store now, cw, 16\n        store 1: i64, cw, 24\n        ret 1\n    ret 0\n\n; a round trip of the client's own has come back, sent at s: how long it took; and since when every round trip has come back within the bound, from the sending of the first of them\nfn __wpinged(pn: i64, s: i64)\n    cw: ptr = addr __wc\n    wp: ptr = addr __w\n    now: i64 = load wp\n    neg: u1 = cmp.lt pn, 0\n    if neg\n        fa: i64 = load cw, 144\n        fnb: i64 = load cw, 152\n        mn: i64 = sub 0, pn\n        mine: u1 = cmp.eq fnb, mn\n        gap: i64 = sub now, fa\n        pos: u1 = cmp.gt gap, 0\n        ok: u1 = and mine, pos\n        if ok\n            v: i64 = div {head_us}, gap\n            store v, cw, 80\n            store now, cw, 136\n        ret\n    store now, cw, 144\n    store pn, cw, 152\n    rt: i64 = sub now, s\n    store rt, cw, 40\n    store 0: i64, cw, 24\n    lg: i64 = load cw, 64\n    lg2: i64 = max(lg, rt)\n    store lg2, cw, 64\n    na: i64 = load cw, 72\n    na2: i64 = add na, 1\n    store na2, cw, 72\n    b: i64 = load cw\n    gs: i64 = load cw, 32\n    nv: i64 = const {never}\n    within: u1 = cmp.le rt, b\n    fresh: u1 = cmp.ge gs, nv\n    st: u1 = and within, fresh\n    g1: i64 = if st\n        yield s\n    else\n        yield gs\n    g2: i64 = if within\n        yield g1\n    else\n        yield nv\n    store g2, cw, 32\n    go: i64 = load cw, 160\n    open: u1 = cmp.lt go, nv\n    cl: u1 = and within, open\n    if cl\n        __wgap_end(s)\n    ret\n\n; the chooser's next moment, after now: a round trip due to be begun or due back, or a second of them within the bound\nfn __wchoose_next() -> i64\n    cw: ptr = addr __wc\n    b: i64 = load cw\n    nv: i64 = const {never}\n    on: u1 = cmp.gt b, 0\n    if on\n    else\n        ret nv\n    wp: ptr = addr __w\n    now: i64 = load wp\n    ls: i64 = load cw, 16\n    dl: i64 = add ls, b\n    gs: i64 = load cw, 32\n    whr: i64 = load cw, 48\n    g: i64 = add gs, 1000000\n    away: u1 = cmp.eq whr, 1\n    gok: u1 = cmp.lt gs, nv\n    ga: u1 = and away, gok\n    gt: i64 = if ga\n        yield g\n    else\n        yield nv\n    m: i64 = min(dl, gt)\n    fut: u1 = cmp.gt m, now\n    r: i64 = if fut\n        yield m\n    else\n        yield nv\n    ret r\n\n; the runner's: the chooser's bound, 0 for none, and its words as a case begins\nfn __wire_choose(b: i64)\n    cw: ptr = addr __wc\n    store b, cw\n    store 0: i64, cw, 8\n    nb: i64 = sub 0, b\n    store nb, cw, 16\n    store 0: i64, cw, 24\n    store {never}: i64, cw, 32\n    store 0: i64, cw, 40\n    store 0: i64, cw, 48\n    store 0: i64, cw, 56\n    store 0: i64, cw, 64\n    store 0: i64, cw, 72\n    loop(i: index = 10) bound 48\n        done: u1 = cmp.ge i, 48\n        if done\n            break\n        store 0: i64, cw, i, 8\n        i2: index = add i, 1\n        continue i2\n    m1: i64 = sub 0, 1\n    store m1, cw, 88\n    store m1, cw, 112\n    store {never}: i64, cw, 120\n    store {never}: i64, cw, 160\n    ret\n{measures}{weighs}", never = NEVER, moves = MOVES, head_us = PACE_HEAD as i64 * 1_000_000, measures = measures_text(), weighs = weighs_text(w))
}

/// What the chooser measures of the wire (fm3 log 295), as text: the
/// speed from what arrives at the client spaced by the wire, the gaps
/// from its own round trips, the given cut's traffic, and at the world's
/// end a line of the record with what it measured last
fn measures_text() -> String {
    format!("\n; an item from the server has arrived at the client (fm3 log 295). Items that arrive at one moment are a burst, their bytes summed. A burst came one behind the one before where its first item was put on before that one could have gone on, and the arrivals are further apart than the sendings: then the wire spaced them, and its speed is the burst's bytes over the gap. And what the server puts on the wire while the cut given is held, but a ping's answer and a move's items, is the cut's traffic\nfn __wmeasure(row: index, kind: i64)\n    cw: ptr = addr __wc\n    wp: ptr = addr __w\n    now: i64 = load wp\n    qa: ptr = addr __wq_at\n    at: i64 = load qa, row, 8\n    qs: ptr = addr __wq_s\n    s: i64 = load qs, row, 8\n    qz: ptr = addr __wq_sz\n    sz: i64 = load qz, row, 8\n    __wtraffic(1, kind, sz)\n    ow: i64 = sub at, s\n    dm0: i64 = load cw, 120\n    dm: i64 = min(dm0, ow)\n    store dm, cw, 120\n    ca: i64 = load cw, 88\n    same: u1 = cmp.eq at, ca\n    if same\n        cb: i64 = load cw, 96\n        cb2: i64 = add cb, sz\n        store cb2, cw, 96\n    else\n        ps: i64 = load cw, 128\n        lim: i64 = sub ca, dm\n        early: u1 = cmp.lt s, lim\n        ga: i64 = sub at, ca\n        gs: i64 = sub s, ps\n        spread: u1 = cmp.gt ga, gs\n        had: u1 = cmp.ge ca, 0\n        q1: u1 = and early, spread\n        q2: u1 = and q1, had\n        qv: i64 = if q2\n            yield 1\n        else\n            yield 0\n        store qv, cw, 104\n        store ca, cw, 112\n        store at, cw, 88\n        store sz, cw, 96\n        store s, cw, 128\n    qq: i64 = load cw, 104\n    qon: u1 = cmp.ne qq, 0\n    if qon\n        pa: i64 = load cw, 112\n        gap: i64 = sub at, pa\n        pos: u1 = cmp.gt gap, 0\n        if pos\n            byt: i64 = load cw, 96\n            bm: i64 = mul byt, 1000000\n            v: i64 = div bm, gap\n            store v, cw, 80\n            store now, cw, 136\n    ret\n\n; what the cut given puts on the wire while it is held (fm3 log 295): at the client what it sends, from the server what arrives, in bytes; not a ping or its answer, nor a move's items\nfn __wtraffic(p: index, kind: i64, sz: i64)\n    cw: ptr = addr __wc\n    whr: i64 = load cw, 48\n    given: u1 = cmp.eq whr, 0\n    k13: u1 = cmp.eq kind, 13\n    k14: u1 = cmp.eq kind, 14\n    k10: u1 = cmp.eq kind, 10\n    k11: u1 = cmp.eq kind, 11\n    k12: u1 = cmp.eq kind, 12\n    k17: u1 = cmp.eq kind, 17\n    o1: u1 = or k13, k14\n    o2: u1 = or o1, k10\n    o3: u1 = or o2, k11\n    o4: u1 = or o3, k12\n    o5: u1 = or o4, k17\n    keep: u1 = xor o5, 1\n    c: u1 = and given, keep\n    if c\n        ix: index = add p, 23\n        n: i64 = load cw, ix, 8\n        n2: i64 = add n, sz\n        store n2, cw, ix, 8\n    ret\n\n; a gap of the wire begins (fm3 log 295): a round trip of the client's own, sent at t, has been out longer than the bound. One of the last eight\nfn __wgap_begin(t: i64)\n    cw: ptr = addr __wc\n    go: i64 = load cw, 160\n    nv: i64 = const {never}\n    open: u1 = cmp.lt go, nv\n    if open\n        ret\n    store t, cw, 160\n    k: i64 = load cw, 168\n    m: i64 = rem k, 8\n    m2: i64 = mul m, 2\n    i: i64 = add m2, 26\n    ix: index = conv i\n    store t, cw, ix, 8\n    j: index = add ix, 1\n    store nv, cw, j, 8\n    k2: i64 = add k, 1\n    store k2, cw, 168\n    ret\n\n; ... and ends at the sending of a round trip that came back within it\nfn __wgap_end(t: i64)\n    cw: ptr = addr __wc\n    k: i64 = load cw, 168\n    k1: i64 = sub k, 1\n    m: i64 = rem k1, 8\n    m2: i64 = mul m, 2\n    i: i64 = add m2, 27\n    ix: index = conv i\n    store t, cw, ix, 8\n    store {never}: i64, cw, 160\n    ret\n\n; how often the wire has gone in the ten seconds to now, and for how long in them: the count times 2^40 and the time\nfn __wgaps(now: i64) -> i64\n    cw: ptr = addr __wc\n    k: i64 = load cw, 168\n    eight: i64 = const 8\n    n: i64 = min(k, eight)\n    w0: i64 = sub now, 10000000\n    r: i64 = loop(i: i64 = 0, g: i64 = 0) bound 9\n        done: u1 = cmp.ge i, n\n        if done\n            break g\n        i2: i64 = mul i, 2\n        a: i64 = add i2, 26\n        ax: index = conv a\n        st: i64 = load cw, ax, 8\n        bx: index = add ax, 1\n        en: i64 = load cw, bx, 8\n        e2: i64 = min(en, now)\n        s2: i64 = max(st, w0)\n        len: i64 = sub e2, s2\n        inw: u1 = cmp.gt en, w0\n        z: i64 = const 0\n        c1: i64 = if inw\n            yield 1099511627776\n        else\n            yield z\n        pos: u1 = cmp.gt len, 0\n        l2: i64 = if pos\n            yield len\n        else\n            yield z\n        g1: i64 = add g, c1\n        g2: i64 = add g1, l2\n        i3: i64 = add i, 1\n        continue i3, g2\n    ret r\n\n; at the world's end, a line of the record, 22: what the chooser measured last: the delay, half its last round trip less a head's time at the speed it saw; the speed, 0 for none seen; and how often the wire went in the last ten seconds, and for how long\nfn __wmeasured()\n    cw: ptr = addr __wc\n    b: i64 = load cw\n    on: u1 = cmp.gt b, 0\n    if on\n    else\n        ret\n    wp: ptr = addr __w\n    now: i64 = load wp\n    rt: i64 = load cw, 40\n    half: i64 = div rt, 2\n    spd: i64 = load cw, 80\n    has: u1 = cmp.gt spd, 0\n    one: i64 = const 1\n    sp1: i64 = max(spd, one)\n    hu: i64 = div {head_us}, sp1\n    hs: i64 = if has\n        yield hu\n    else\n        yield 0\n    dl0: i64 = sub half, hs\n    zero: i64 = const 0\n    dl: i64 = max(dl0, zero)\n    packed: i64 = __wgaps(now)\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        store 0: i64, lp, l0, 8\n        l1: index = add l0, 1\n        store 22: i64, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store dl, lp, l3, 8\n        l4: index = add l0, 4\n        store spd, lp, l4, 8\n        l5: index = add l0, 5\n        store packed, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret\n", never = NEVER, head_us = PACE_HEAD as i64 * 1_000_000, log = WIRE_LOG)
}

/// What the chooser weighs before it gives the server its part back (fm3 log
/// 296), as text: the steady time the wire's history asks, the speed
/// against the rate the cut given put on the wire, the move's own
/// bytes; each a line of the record where it is what keeps the part
fn weighs_text(w: &World) -> String {
    let word = (w.size + 8 * w.hsize + PACE_HEAD) as i64;
    let fun = (8 * w.cols.len() + PACE_HEAD) as i64;
    let rows = if w.pooled { format!("    c: ptr = context()\n    x: __ctx = load c\n    an: index = get x, __act_n\n    n: i64 = conv an\n    fb: i64 = mul n, {fun}\n    m: i64 = add fb, {word}\n", fun = fun, word = word) } else { format!("    m: i64 = const {word}\n", word = word) };
    format!("\n; how long the wire must have stayed steady before the server is given its part back (fm3 log 296): a second where it has gone at most once in the last ten seconds; otherwise a second for each time it went in them and the time it was gone in them. `__w`'s fourth word 1024 is a test's: a second always\nfn __wsteady(now: i64) -> i64\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 1024\n    ig: u1 = cmp.ne bm, 0\n    if ig\n        ret 1000000\n    pk: i64 = __wgaps(now)\n    n: i64 = div pk, 1099511627776\n    g: i64 = rem pk, 1099511627776\n    once: u1 = cmp.le n, 1\n    if once\n        ret 1000000\n    ns: i64 = mul n, 1000000\n    s: i64 = add ns, g\n    ret s\n\n; the bytes giving the server its part back puts on the wire: the move's word, and a function partway for each place of the client's pool\nfn __wmove_bytes() -> i64\n{rows}    ret m\n\n; how long the move's own bytes take to go on at the speed last seen; none where no speed has been seen. `__w`'s fourth word 2048 is a test's: the speed not weighed\nfn __wmove_time() -> i64\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 2048\n    ig: u1 = cmp.ne bm, 0\n    if ig\n        ret 0\n    cw: ptr = addr __wc\n    spd: i64 = load cw, 80\n    none: u1 = cmp.le spd, 0\n    if none\n        ret 0\n    m: i64 = __wmove_bytes()\n    bm6: i64 = mul m, 1000000\n    sp1: i64 = sub spd, 1\n    b2: i64 = add bm6, sp1\n    t: i64 = div b2, spd\n    ret t\n\n; does the speed last seen carry what the cut given put on the wire, with room to spare: twice it. Where no speed has been seen, it does\nfn __wspeed_ok() -> u1\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 2048\n    ig: u1 = cmp.ne bm, 0\n    if ig\n        ret 1\n    cw: ptr = addr __wc\n    spd: i64 = load cw, 80\n    none: u1 = cmp.le spd, 0\n    if none\n        ret 1\n    r: i64 = load cw, 200\n    r2: i64 = mul r, 2\n    ok: u1 = cmp.ge spd, r2\n    ret ok\n\n; the server's part taken: the rate the cut given put on the wire while it was held, the larger way's bytes over the time, remembered; and what keeps the part, said once a hold\nfn __wtaken(now: i64)\n    cw: ptr = addr __wc\n    hs: i64 = load cw, 176\n    up: i64 = load cw, 184\n    dn: i64 = load cw, 192\n    byx: i64 = max(up, dn)\n    el0: i64 = sub now, hs\n    one: i64 = const 1\n    el: i64 = max(el0, one)\n    b6: i64 = mul byx, 1000000\n    r: i64 = div b6, el\n    store r, cw, 200\n    store 0: i64, cw, 352\n    ret\n\n; the server's part given back: the cut given held from now, its traffic counted again; the server's start to be kept at the move's turn (fm3 log 298); and what was weighed, a line of the record, 23: how often and how long the wire went, the move's bytes, the speed seen and the rate remembered\nfn __wgiven(now: i64)\n    gw: ptr = addr __wg\n    store 1: i64, gw, 144\n    cw: ptr = addr __wc\n    store now, cw, 176\n    store 0: i64, cw, 184\n    store 0: i64, cw, 192\n    pk: i64 = __wgaps(now)\n    m: i64 = __wmove_bytes()\n    sp0: i64 = load cw, 80\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 2048\n    ig: u1 = cmp.ne bm, 0\n    spd: i64 = if ig\n        yield 0\n    else\n        yield sp0\n    r: i64 = load cw, 200\n    __wnote3(pk, 23, m, spd, r)\n    ret\n\n; the round trips would give the server its part back and the speed seen does not carry the cut given's rate with room to spare: said once a hold, 24\nfn __wkept_slow(now: i64)\n    cw: ptr = addr __wc\n    f: i64 = load cw, 352\n    f1: i64 = and f, 1\n    said: u1 = cmp.ne f1, 0\n    if said\n        ret\n    f2: i64 = or f, 1\n    store f2, cw, 352\n    spd: i64 = load cw, 80\n    r: i64 = load cw, 200\n    __wnote(24, spd, r)\n    ret\n\n; a second of round trips within the bound would give the server its part back and the wire's history asks longer: said once a hold, 25\nfn __wkept(now: i64, gs: i64, h2: u1)\n    cw: ptr = addr __wc\n    whr: i64 = load cw, 48\n    away: u1 = cmp.eq whr, 1\n    nv: i64 = const {never}\n    g0: u1 = cmp.lt gs, nv\n    g1s: i64 = add gs, 1000000\n    g1: u1 = cmp.ge now, g1s\n    a: u1 = and away, g0\n    b: u1 = and a, g1\n    nh: u1 = xor h2, 1\n    c: u1 = and b, nh\n    if c\n        sty: i64 = __wsteady(now)\n        lng: u1 = cmp.gt sty, 1000000\n        f: i64 = load cw, 352\n        f1: i64 = and f, 2\n        fresh: u1 = cmp.eq f1, 0\n        d: u1 = and lng, fresh\n        if d\n            f2: i64 = or f, 2\n            store f2, cw, 352\n            pk: i64 = __wgaps(now)\n            __wnote(25, sty, pk)\n    ret\n\n; a line of the wire's record with three words of its own and one in its first\nfn __wnote3(w0: i64, kind: i64, a: i64, b: i64, c: i64)\n    wp: ptr = addr __w\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        now: i64 = load wp\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        store w0, lp, l0, 8\n        l1: index = add l0, 1\n        store kind, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store a, lp, l3, 8\n        l4: index = add l0, 4\n        store b, lp, l4, 8\n        l5: index = add l0, 5\n        store c, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret\n", rows = rows, never = NEVER, log = WIRE_LOG)
}

/// A wire whose delay changes while the program runs, and that may go
/// and come back (fm3 log 289), as text: when an item put on the wire
/// now arrives, and the runner's word that fills the table of changes
fn changes_text() -> String {
    format!("\n; when an item put on the wire now arrives (fm3 log 289): now and the delay of the last change made by now, or the first delay where none is; where the wire is gone now, when it comes back and the delay it comes back with, or never. An item keeps the delay it left with, whatever changes while it is on the wire\nfn __wat(now: i64, d: i64) -> i64\n    sp: ptr = addr __wsch\n    n: i64 = load sp\n    dd: i64 = loop(i: i64 = 0, v: i64 = d) bound {b}\n        done: u1 = cmp.ge i, n\n        if done\n            break v\n        i2: i64 = mul i, 2\n        fi: i64 = add i2, 1\n        fx: index = conv fi\n        f: i64 = load sp, fx, 8\n        ex: index = add fx, 1\n        e: i64 = load sp, ex, 8\n        due: u1 = cmp.le f, now\n        v2: i64 = if due\n            yield e\n        else\n            yield v\n        i3: i64 = add i, 1\n        continue i3, v2\n    nv: i64 = const {never}\n    gone: u1 = cmp.ge dd, nv\n    if gone\n        back: i64 = loop(j: i64 = 0) bound {b}\n            jd: u1 = cmp.ge j, n\n            if jd\n                break nv\n            j2: i64 = mul j, 2\n            gi: i64 = add j2, 1\n            gx: index = conv gi\n            g: i64 = load sp, gx, 8\n            hx: index = add gx, 1\n            h: i64 = load sp, hx, 8\n            after: u1 = cmp.gt g, now\n            up: u1 = cmp.lt h, nv\n            ok: u1 = and after, up\n            if ok\n                bk: i64 = add g, h\n                break bk\n            j3: i64 = add j, 1\n            continue j3\n        ret back\n    at: i64 = add now, dd\n    ret at\n\n; the runner's: the k-th change of the wire, the time it takes effect and the delay from then, a gone wire's the never-time. Said in order from the first, and the k-th said is the last\nfn __wire_at(k: i64, f: i64, v: i64)\n    sp: ptr = addr __wsch\n    k2: i64 = mul k, 2\n    fi: i64 = add k2, 1\n    fx: index = conv fi\n    store f, sp, fx, 8\n    vx: index = add fx, 1\n    store v, sp, vx, 8\n    k1: i64 = add k, 1\n    store k1, sp\n    ret\n", b = CHANGES + 1, never = NEVER)
}

/// An item's size in bytes, its head aside (fm3 log 294), as text: in
/// a build whose wire has a speed, and in one that chooses, whose
/// chooser measures the wire by what arrives (fm3 log 295)
fn size_text(w: &World) -> String {
    let record = 8 * (w.cols.len() + w.fields.len());
    let word = w.size + 8 * w.hsize;
    let state = 8 * w.hsize;
    let fun = 8 * w.cols.len();
    format!("\n; an item's size in bytes, its head aside (fm3 log 294): a character 1, a block its characters, a push its value, a function partway its row of the pool and the fields the wire may carry with it, a telling its time and rank, a ping and its answer nothing more; at a move, the word the cut's table and what a move may hand, the server's state what a move may hand, a function partway its row\nfn __wsize(kind: i64, a: i64, b: i64) -> i64\n    k1: u1 = cmp.eq kind, 1\n    if k1\n        ret 1\n    k2: u1 = cmp.eq kind, 2\n    if k2\n        ret b\n    k3: u1 = cmp.eq kind, 3\n    if k3\n        ret 8\n    k4: u1 = cmp.eq kind, 4\n    if k4\n        ret {record}\n    k5: u1 = cmp.eq kind, 5\n    if k5\n        ret 16\n    k10: u1 = cmp.eq kind, 10\n    if k10\n        ret {word}\n    k11: u1 = cmp.eq kind, 11\n    if k11\n        ret {state}\n    k12: u1 = cmp.eq kind, 12\n    if k12\n        ret {fun}\n    k17: u1 = cmp.eq kind, 17\n    if k17\n        ret {size}\n    ret 0\n", record = record, word = word, state = state, fun = fun, size = w.size)
}

/// A wire with a speed, or one that comes and goes (fm3 log 294), as
/// text, in place of `changes_text`'s `__wat`: an item goes on its way
/// of the wire when the wire is free and up, takes its size over the
/// speed of that moment to go on, and then the delay of the moment it
/// has gone on. A piece of text, characters and blocks put on one way
/// at one moment for one time, is one item: the head paid once, and
/// every character arriving when the last has gone on and travelled
fn paced_text() -> String {
    let (q, b, never) = (WIRE_Q, CHANGES + 1, NEVER);
    let head = PACE_HEAD;
    let mut t = String::new();
    t.push_str(&format!("\n; the wire's speed at a moment: the first, or that of the last change made by then\nfn __wspeed(u: i64) -> i64\n    sp: ptr = addr __wsp\n    sc: ptr = addr __wsch\n    n: i64 = load sc\n    s0: i64 = load sp\n    vr: i64 = loop(i: i64 = 0, v: i64 = s0) bound {b}\n        done: u1 = cmp.ge i, n\n        if done\n            break v\n        i2: i64 = mul i, 2\n        fi: i64 = add i2, 1\n        fx: index = conv fi\n        f: i64 = load sc, fx, 8\n        due: u1 = cmp.le f, u\n        ix: i64 = add i, 1\n        ii: index = conv ix\n        e: i64 = load sp, ii, 8\n        v2: i64 = if due\n            yield e\n        else\n            yield v\n        continue ix, v2\n    ret vr\n", b = b));
    t.push_str(&format!("\n; the first moment from u that the wire is up: not gone by its changes, and not in a gap of its coming and going; never where it has gone for good\nfn __wup(u0: i64) -> i64\n    sp: ptr = addr __wsp\n    sc: ptr = addr __wsch\n    n: i64 = load sc\n    fr: i64 = load sp, 80\n    pe: i64 = load sp, 88\n    gp: i64 = load sp, 96\n    nv: i64 = const {never}\n    one: i64 = const 1\n    pe1: i64 = max(pe, one)\n    per: u1 = cmp.gt pe, 0\n    r: i64 = loop(k: i64 = 0, u: i64 = u0) bound 64\n        fin: u1 = cmp.ge u, nv\n        if fin\n            break u\n        g: i64 = loop(i: i64 = 0, v: i64 = 0) bound {b}\n            done: u1 = cmp.ge i, n\n            if done\n                break v\n            i2: i64 = mul i, 2\n            fi: i64 = add i2, 1\n            fx: index = conv fi\n            f: i64 = load sc, fx, 8\n            ex: index = add fx, 1\n            e: i64 = load sc, ex, 8\n            due: u1 = cmp.le f, u\n            v2: i64 = if due\n                yield e\n            else\n                yield v\n            i3: i64 = add i, 1\n            continue i3, v2\n        back: i64 = loop(j: i64 = 0) bound {b}\n            jd: u1 = cmp.ge j, n\n            if jd\n                break nv\n            j2: i64 = mul j, 2\n            gi: i64 = add j2, 1\n            gx: index = conv gi\n            h: i64 = load sc, gx, 8\n            hx: index = add gx, 1\n            hv: i64 = load sc, hx, 8\n            after: u1 = cmp.gt h, u\n            up: u1 = cmp.lt hv, nv\n            ok: u1 = and after, up\n            if ok\n                break h\n            j3: i64 = add j, 1\n            continue j3\n        tgone: u1 = cmp.ge g, nv\n        u1v: i64 = if tgone\n            yield back\n        else\n            yield u\n        aft: u1 = cmp.ge u1v, fr\n        fin2: u1 = cmp.lt u1v, nv\n        pa: u1 = and per, aft\n        pb: u1 = and pa, fin2\n        off: i64 = sub u1v, fr\n        ph: i64 = rem off, pe1\n        inn: u1 = cmp.lt ph, gp\n        pc: u1 = and pb, inn\n        st: i64 = sub u1v, ph\n        en: i64 = add st, gp\n        u2: i64 = if pc\n            yield en\n        else\n            yield u1v\n        same: u1 = cmp.eq u2, u\n        if same\n            break u\n        k2: i64 = add k, 1\n        continue k2, u2\n    ret r\n", never = never, b = b));
    t.push_str(&format!("\n; when an item put on the wire now arrives (fm3 log 289, 294): the first moment from then that the wire is up, and the delay of the last change made by that moment, or the first delay where none is; never where the wire has gone for good\nfn __wat(now: i64, d: i64) -> i64\n    sc: ptr = addr __wsch\n    n: i64 = load sc\n    u: i64 = __wup(now)\n    nv: i64 = const {never}\n    fin: u1 = cmp.ge u, nv\n    if fin\n        ret nv\n    dd: i64 = loop(i: i64 = 0, v: i64 = d) bound {b}\n        done: u1 = cmp.ge i, n\n        if done\n            break v\n        i2: i64 = mul i, 2\n        fi: i64 = add i2, 1\n        fx: index = conv fi\n        f: i64 = load sc, fx, 8\n        ex: index = add fx, 1\n        e: i64 = load sc, ex, 8\n        due: u1 = cmp.le f, u\n        v2: i64 = if due\n            yield e\n        else\n            yield v\n        i3: i64 = add i, 1\n        continue i3, v2\n    at: i64 = add u, dd\n    ret at\n", never = never, b = b));
    // (`__wsp`'s words: 0 to 8 the speeds, 10 to 12 the coming and going, then a pair a way: 16 free, 18 the last kind, 20 its time, 22 its moment, 24 where the piece began in the wire, 26 in the record)
    t.push_str(&format!("\n; an item put on the wire toward `to` now (fm3 log 294): when it has gone on, and when it arrives, and its size. It goes on when the way is free and the wire up, and takes its size over the speed of that moment; then the delay. A character or a block put on just after one of a piece of text on this way, at this moment and for the same time, is more of that piece: no head, and every character of the piece already on the wire, and its line in the record, arrives with it. A function partway handed at a move, put on just after the move's word, the server's state or another such, on this way at this moment, is more of the move in the same way, with its own head (fm3 log 298): a move arrives whole. While the world begins (the delay 0) the wire takes no time\nfn __wpace(kind: i64, a: i64, b: i64, to: index, now: i64, d: i64, stamp: i64, tail: i64) -> (i64, i64, i64)\n    sp: ptr = addr __wsp\n    wp: ptr = addr __w\n    body: i64 = __wsize(kind, a, b)\n    ik: index = add to, 18\n    lk: i64 = load sp, ik, 8\n    ist: index = add to, 20\n    lst: i64 = load sp, ist, 8\n    im: index = add to, 22\n    lnw: i64 = load sp, im, 8\n    ip: index = add to, 24\n    pf: i64 = load sp, ip, 8\n    il: index = add to, 26\n    lf: i64 = load sp, il, 8\n    t2: index = mul to, 2\n    hi: index = add t2, 24\n    qh: i64 = load wp, hi, 8\n    x1: u1 = cmp.eq kind, 1\n    x2: u1 = cmp.eq kind, 2\n    tx: u1 = or x1, x2\n    y1: u1 = cmp.eq lk, 1\n    y2: u1 = cmp.eq lk, 2\n    ty: u1 = or y1, y2\n    se: u1 = cmp.eq lst, stamp\n    sm: u1 = cmp.eq lnw, now\n    qd: u1 = cmp.le qh, pf\n    live: u1 = cmp.gt d, 0\n    c1: u1 = and tx, ty\n    m10: u1 = cmp.eq lk, 10\n    m11: u1 = cmp.eq lk, 11\n    m12: u1 = cmp.eq lk, 12\n    mw: u1 = or m10, m11\n    mv: u1 = or mw, m12\n    x12: u1 = cmp.eq kind, 12\n    mc: u1 = and x12, mv\n    c1m: u1 = or c1, mc\n    c2: u1 = and c1m, se\n    c3: u1 = and c2, sm\n    c4: u1 = and c3, qd\n    cont: u1 = and c4, live\n    store kind, sp, ik, 8\n    store stamp, sp, ist, 8\n    store now, sp, im, 8\n    ct: u1 = and cont, tx\n    hd: i64 = if ct\n        yield 0\n    else\n        yield {head}\n    sz: i64 = add body, hd\n    ifr: index = add to, 16\n    free: i64 = load sp, ifr, 8\n    st0: i64 = max(now, free)\n    st: i64 = __wup(st0)\n    spd: i64 = __wspeed(st)\n    has: u1 = cmp.gt spd, 0\n    one: i64 = const 1\n    sp1: i64 = max(spd, one)\n    byt: i64 = mul sz, 1000000\n    sp2: i64 = sub sp1, 1\n    by2: i64 = add byt, sp2\n    du: i64 = div by2, sp1\n    dur: i64 = if has\n        yield du\n    else\n        yield 0\n    dn0: i64 = add st, dur\n    dn: i64 = if live\n        yield dn0\n    else\n        yield now\n    if live\n        store dn, sp, ifr, 8\n    at: i64 = __wat(dn, d)\n    if cont\n        tq: i64 = conv to\n        qb: i64 = mul tq, {q}\n        loop(j: i64 = pf) bound {q}\n            jd: u1 = cmp.ge j, tail\n            if jd\n                break\n            jm: i64 = rem j, {q}\n            jr: i64 = add qb, jm\n            jx: index = conv jr\n            qa: ptr = addr __wq_at\n            store at, qa, jx, 8\n            j2: i64 = add j, 1\n            continue j2\n        ln: i64 = load wp, 32\n        tw: i64 = conv to\n        loop(li: i64 = lf) bound {log}\n            ld: u1 = cmp.ge li, ln\n            if ld\n                break\n            lp: ptr = addr __wlog\n            l6: i64 = mul li, 6\n            l0: index = conv l6\n            lt: i64 = load lp, l0, 8\n            l1: index = add l0, 1\n            lkd: i64 = load lp, l1, 8\n            l4: index = add l0, 4\n            lsm: i64 = load lp, l4, 8\n            w1: u1 = cmp.eq lt, tw\n            w2: u1 = cmp.eq lkd, 1\n            w3: u1 = cmp.eq lkd, 2\n            w23: u1 = or w2, w3\n            w4: u1 = cmp.eq lsm, stamp\n            w12: u1 = and w1, w23\n            w5t: u1 = and w12, w4\n            v10: u1 = cmp.eq lkd, 10\n            v11: u1 = cmp.eq lkd, 11\n            v12: u1 = cmp.eq lkd, 12\n            vm1: u1 = or v10, v11\n            vm: u1 = or vm1, v12\n            vmc: u1 = and vm, mc\n            vmw: u1 = and w1, vmc\n            w5: u1 = or w5t, vmw\n            if w5\n                l3: index = add l0, 3\n                store at, lp, l3, 8\n            li2: i64 = add li, 1\n            continue li2\n    else\n        store tail, sp, ip, 8\n        lnow: i64 = load wp, 32\n        store lnow, sp, il, 8\n    ret at, dn, sz\n\n; beneath an item's line of the wire's record, a line of its own, 21: its size and when it finished going on\nfn __wsized(to: index, dn: i64, sz: i64)\n    wp: ptr = addr __w\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        now: i64 = load wp\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        tw: i64 = conv to\n        store tw, lp, l0, 8\n        l1: index = add l0, 1\n        store 21: i64, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store dn, lp, l3, 8\n        l4: index = add l0, 4\n        store sz, lp, l4, 8\n        l5: index = add l0, 5\n        store 0: i64, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret\n", head = head, q = q, log = WIRE_LOG));
    t.push_str("\n; the runner's: the wire's first speed, 0 for as fast as it can be, and its coming and going, from when, how often and for how long, a period of 0 for none; each way of the wire empty. Said before every case\nfn __wire_pace(s: i64, from: i64, every: i64, gap: i64)\n    sp: ptr = addr __wsp\n    store s, sp\n    store from, sp, 80\n    store every, sp, 88\n    store gap, sp, 96\n    loop(i: index = 16) bound 32\n        done: u1 = cmp.ge i, 32\n        if done\n            break\n        store 0: i64, sp, i, 8\n        i2: index = add i, 1\n        continue i2\n    ret\n\n; ... and the speed from the k-th change, said after the change itself\nfn __wire_speed(k: i64, s: i64)\n    sp: ptr = addr __wsp\n    k1: i64 = add k, 1\n    kx: index = conv k1\n    store s, sp, kx, 8\n    ret\n");
    // `changes_text`'s runner's word for the changes, after its `__wat`
    let ct = changes_text();
    let at = ct.find("\n; the runner's: the k-th change").expect("the runner's word for a change");
    t.push_str(&ct[at..]);
    t
}

/// The cut moved over a wire that takes time (fm3 cut.md, "the cut
/// moved over a wire that takes time"; log 276), as text. The move's
/// turn, `__move_turn`, is the client's and is written with the table.
/// Here: what the server does when the move's word reaches it, what the client
/// does when the server's state reaches it, the functions partway that go
/// with a unit, and the client's count of the word as a thing sent
fn move_text(w: &World) -> String {
    let (q, pool, nu, size) = (WIRE_Q, POOL, w.nu, w.size);
    let mut t = String::new();
    t.push_str(&format!("\n; the time an item of a move is for: the move's own, or when a function partway is next due, and not its sender's clock. In the item's row and in the wire's record\nfn __wstamp(row: index, t: i64)\n    tq: ptr = addr __wq_t\n    store t, tq, row, 8\n    wp: ptr = addr __w\n    ln: i64 = load wp, 32\n    some: u1 = cmp.gt ln, 0\n    room: u1 = cmp.lt ln, {log}\n    mine: u1 = and some, room\n    if mine\n{last}        l6: i64 = mul l1, 6\n        l4: i64 = add l6, 4\n        li: index = conv l4\n        lp: ptr = addr __wlog\n        store t, lp, li, 8\n    ret\n", log = WIRE_LOG, last = if w.paced { "        lz: i64 = sub ln, 1\n        lz6: i64 = mul lz, 6\n        lz1: i64 = add lz6, 1\n        lzi: index = conv lz1\n        lzp: ptr = addr __wlog\n        lzk: i64 = load lzp, lzi, 8\n        lzs: u1 = cmp.eq lzk, 21\n        lzb: u1 = cmp.gt lz, 0\n        lzc: u1 = and lzs, lzb\n        lzd: i64 = if lzc\n            yield 1\n        else\n            yield 0\n        l1: i64 = sub lz, lzd\n" } else { "        l1: i64 = sub ln, 1\n" }));
    // the client's count of the word
    t.push_str(&format!("\n; the move's word is a thing the client has sent, which the server answers: and, where the server works ahead, it is for the move's time at the move's rank, so that every piece the client is keeping that was written in ignorance of it, for then or later, is dropped\nfn __wsent_move(t: i64)\n    wp: ptr = addr __w\n    n: i64 = load wp, 64\n    n2: i64 = add n, 1\n    store n2, wp, 64\n{void}    ret\n", void = if w.input { format!("    ap: ptr = addr __wa\n    sr: i64 = load ap, 80\n    m: i64 = rem n2, {sends}\n    mi: index = conv m\n    tp: ptr = addr __ws_t\n    store t, tp, mi, 8\n    rp: ptr = addr __ws_r\n    store sr, rp, mi, 8\n    __wvoid(t, sr, n2)\n", sends = SENDS) } else { String::new() }));
    if w.pooled {
        t.push_str("\n; is the continuation of this number a unit's that is nowhere: between the place that left it and the place that takes it\nfn __wmoving(pc: i64) -> u1\n    cp: ptr = addr __cut\n");
        for (id, o) in &w.ids {
            if let Some(u) = o {
                t.push_str(&format!("    is{id}: u1 = cmp.eq pc, {id}\n    if is{id}\n        c{id}: u8 = load cp, {u}\n        m{id}: u1 = cmp.eq c{id}, 2\n        ret m{id}\n", id = id, u = u));
            }
        }
        t.push_str("    ret 0\n");
        t.push_str(&format!("\n; the functions partway at this place that go on in a unit that is leaving it: each is put on the wire, a record an item, with the time it is next due, the oldest first, so that the order they were begun in is theirs at the place they reach; and its place here is free. `__w`'s fourth word, 32, is a test's: they are not sent\nfn __wship()\n    c: ptr = context()\n    x: __ctx = load c\n    n: index = get x, __act_n\n    p: index = __wplace()\n    base: index = mul p, {pool}\n    top: index = add base, n\n    pp: ptr = addr __act_pc\n    bp: ptr = addr __act_born\n    dp: ptr = addr __act_due\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 32\n    sends: u1 = cmp.eq bm, 0\n    loop(j: index = 0) bound {pool1}\n        best: index = loop(s: index = base, i: index = top, o: i64 = {never}) bound {pool1}\n            done: u1 = cmp.ge s, top\n            if done\n                break i\n            pc: i64 = load pp, s, 8\n            mv: u1 = __wmoving(pc)\n            w: i64 = load bp, s, 8\n            older: u1 = cmp.lt w, o\n            take: u1 = and mv, older\n            i2: index = if take\n                yield s\n            else\n                yield i\n            o2: i64 = if take\n                yield w\n            else\n                yield o\n            s2: index = add s, 1\n            continue s2, i2, o2\n        none: u1 = cmp.eq best, top\n        if none\n            break\n        if sends\n            pc2: i64 = load pp, best, 8\n            due: i64 = load dp, best, 8\n            row: index = __wput(12, pc2, due, pc2)\n            __wstamp(row, due)\n            fl: index = add row, {flight}\n            __act_carry(best, fl)\n        store {never}: i64, dp, best, 8\n        store 0: i64, pp, best, 8\n        j2: index = add j, 1\n        continue j2\n    __act_next()\n    ret\n\n; ... and one that arrives is given a place of this half of the pool, as it is: when it is next due and where it goes on are the record's own, and the list takes its turn then. It is not gone on with at once, as a function partway that crosses in a call is\nfn __wtake(row: index)\n    slot: index = __act_slot()\n    bp: ptr = addr __act_born\n    born: i64 = load bp, slot, 8\n    fl: index = add row, {flight}\n    __act_carry(fl, slot)\n{land}    store born, bp, slot, 8\n    __act_next()\n    ret\n", pool = pool, pool1 = pool + 1, never = NEVER, flight = 2 * pool, land = if w.lands { "    __wland(slot)\n" } else { "" }));
    }
    let (ship, next) = if w.pooled { ("        __wship()\n", "    __act_next()\n") } else { ("", "") };
    t.push_str(&format!("\n; the move's word has reached the server (fm3 log 276): the k-th move, for time t. Each unit the client left is the server's now, with what the client sent of it, which is taken from the word's row; each unit that leaves the server is nowhere until the client has it; the server's own bytes of the table are the new cut's. And where anything leaves the server, the server sends its state, as it is, and the functions partway that go with it, and waits for the client's answer before it takes a turn\nfn __wmove_server(row: index, k: index, t: i64)\n    cp: ptr = addr __cut\n    np: ptr = addr __cuts\n    base: index = mul k, {size}\n    back: i64 = loop(i: index = 0, n: i64 = 0) bound {nu1}\n        done: u1 = cmp.ge i, {nu}\n        if done\n            break n\n        j: index = add base, i\n        nb: u8 = load np, j, 1\n        ob: u8 = load cp, i, 1\n        comes: u1 = cmp.eq ob, 2\n        was: u1 = cmp.eq ob, 1\n        to: u1 = cmp.eq nb, 0\n        goes: u1 = and was, to\n        u0: u8 = if goes\n            yield 2\n        else\n            yield ob\n        ub: u8 = if comes\n            yield 1\n        else\n            yield u0\n        store ub, cp, i, 1\n        g: i64 = conv goes\n        n2: i64 = add n, g\n        i2: index = add i, 1\n        continue i2, n2\n    loop(m: index = {nu1}) bound {size}\n        done2: u1 = cmp.ge m, {size}\n        if done2\n            break\n        j2: index = add base, m\n        b2: u8 = load np, j2, 1\n        store b2, cp, m, 1\n        m2: index = add m, 2\n        continue m2\n    __whand(row, k, 1)\n    some: u1 = cmp.gt back, 0\n    if some\n        c: ptr = context()\n        x: __ctx = load c\n        kw: i64 = conv k\n        row2: index = __wput(11, kw, t, t)\n        __wstamp(row2, t)\n        qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n        qe: ptr(__ctx) = index qm, row2\n        qp: ptr = cast qe\n        store x, qp\n{ship}        __wsent()\n{next}    ret\n\n; the server's state has reached the client: each unit on the server left is the client's now, with what the server sent of it\nfn __wmove_client(row: index, k: index)\n    cp: ptr = addr __cut\n    loop(i: index = 0) bound {nu1}\n        done: u1 = cmp.ge i, {nu}\n        if done\n            break\n        ob: u8 = load cp, i, 1\n        comes: u1 = cmp.eq ob, 2\n        if comes\n            store 0: u8, cp, i, 1\n        i2: index = add i, 1\n        continue i2\n    __whand(row, k, 2)\n{next}    ret\n", size = size, nu = nu, nu1 = nu + 1, q2 = 2 * q, ship = ship, next = next));
    t
}

/// Working ahead of a key (fm3 cut.md, "working ahead of a key"; log
/// 273), as text, for a store with an input that arrives at a time.
/// The server's copies of itself: made before a turn taken ahead, put back
/// when something from the client says that turn was wrong, dropped
/// when the client's telling has passed it. And the client's queue of
/// kept text: a piece that arrives is kept until its time, shown in
/// the order of the turns that wrote it, and dropped unshown where it
/// was written in ignorance of something the client has sent since
fn ahead_text(w: &World) -> String {
    let mut t = String::new();
    t.push_str(&format!("\n; a line of the wire's record that is no item: 6 a character and 7 a block of text dropped at the client unshown, 8 the server gone back, with the time it went back to and how many turns it threw away, 9 a turn the server took ahead and took back, because it would have sent more than text\nfn __wnote(kind: i64, stamp: i64, said: i64)\n    wp: ptr = addr __w\n    ln: i64 = load wp, 32\n    lroom: u1 = cmp.lt ln, {log}\n    if lroom\n        now: i64 = load wp\n        li: index = conv ln\n        l0: index = mul li, 6\n        lp: ptr = addr __wlog\n        store 0: i64, lp, l0, 8\n        l1: index = add l0, 1\n        store kind, lp, l1, 8\n        l2: index = add l0, 2\n        store now, lp, l2, 8\n        l3: index = add l0, 3\n        store now, lp, l3, 8\n        l4: index = add l0, 4\n        store stamp, lp, l4, 8\n        l5: index = add l0, 5\n        store said, lp, l5, 8\n        ln2: i64 = add ln, 1\n        store ln2, wp, 32\n    ret\n", log = WIRE_LOG));
    if w.pooled {
        t.push_str(&format!("\n; the server's half of the pool, its first n places, into the k-th copy or back out of it: every column\nfn __wpool(k: index, n: index, back: u1)\n    sp: ptr = addr __wa_pool\n    kb: index = mul k, {per}\n    loop(i: index = 0) bound {rows}\n        done: u1 = cmp.ge i, n\n        if done\n            break\n        f: index = add i, {pool}\n        s0: index = add kb, i\n", per = AHEAD_ROWS * w.cols.len(), rows = AHEAD_ROWS, pool = POOL));
        for (j, c) in w.cols.iter().enumerate() {
            t.push_str(&format!("        p{j}: ptr = addr {c}\n        d{j}: index = add s0, {at}\n        if back\n            v{j}: i64 = load sp, d{j}, 8\n            store v{j}, p{j}, f, 8\n        else\n            w{j}: i64 = load p{j}, f, 8\n            store w{j}, sp, d{j}, 8\n", j = j, c = c, at = j * AHEAD_ROWS));
        }
        t.push_str("        i2: index = add i, 1\n        continue i2\n    ret\n");
    }
    t.push_str(&format!("\n; the part of the arena the server's context was made in, every stream of the server's with its counts and its items, into the k-th copy or back out of it\nfn __wmem(k: index, back: u1)\n    ap: ptr = addr __wa\n    base: ptr = load ap, 112\n    n: index = load ap, 120\n    lq: ptr(u8) = cast base\n    live: u8[] = pack lq, n, 1\n    mp: ptr = addr __wa_mem\n    off: index = mul k, {mem}\n    mq: ptr(u8) = ptradd mp, off\n    held: u8[] = pack mq, n, 1\n    if back\n        copy live, held\n    else\n        copy held, live\n    ret\n", mem = AHEAD_MEM));
    // (the copy keeps each value's storage as it was, and the server's own
    // records are given storage of their own: fm3 log 284)
    let pool_out = if w.pooled && w.lands { "    an: index = get x, __act_n\n    no: u1 = cmp.ne k, k\n    __wpool(k, an, no)\n    __wland_all(an)\n" } else if w.pooled { "    an: index = get x, __act_n\n    no: u1 = cmp.ne k, k\n    __wpool(k, an, no)\n" } else { "    no: u1 = cmp.ne k, k\n" };
    if w.pooled && w.lands {
        t.push_str(&format!("\n; before a turn taken ahead, each of the server's records is given values of its own, the copy just made keeping the storage as it was (fm3 log 284)\nfn __wland_all(n: index)\n    loop(i: index = 0)\n        done: u1 = cmp.ge i, n\n        if done\n            break\n        row: index = add i, {pool}\n        __wland(row)\n        i2: index = add i, 1\n        continue i2\n    ret\n", pool = POOL));
    }
    let pool_in = if w.pooled { "    an: index = get x, __act_n\n    __wpool(k, an, yes)\n" } else { "" };
    t.push_str(&format!("\n; the server keeps a copy of itself before a turn it takes ahead, for time t at rank r: its context, its half of the pool and its streams. Gives which copy it is\nfn __wcopy(t: i64, r: i64) -> index\n    ap: ptr = addr __wa\n    h: i64 = load ap, 40\n    n: i64 = load ap, 48\n    s0: i64 = add h, n\n    k0: i64 = rem s0, {n}\n    k: index = conv k0\n    tp: ptr = addr __wa_t\n    store t, tp, k, 8\n    rp: ptr = addr __wa_r\n    store r, rp, k, 8\n    n2: i64 = add n, 1\n    store n2, ap, 48\n    c: ptr = context()\n    x: __ctx = load c\n    sm: ptr(array(__ctx, {n})) = addr __wa_ctx\n    se: ptr(__ctx) = index sm, k\n    sp: ptr = cast se\n    store x, sp\n{pool_out}    __wmem(k, no)\n    ret k\n\n; ... and is put back to it. `__w`'s fourth word, 16, is a test's: its streams are not\nfn __wrestore(k: index)\n    sm: ptr(array(__ctx, {n})) = addr __wa_ctx\n    se: ptr(__ctx) = index sm, k\n    sp: ptr = cast se\n    x: __ctx = load sp\n    c: ptr = context()\n    store x, c\n    yes: u1 = cmp.eq k, k\n{pool_in}    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 16\n    whole: u1 = cmp.eq bm, 0\n    if whole\n        __wmem(k, yes)\n    ret\n", n = AHEAD, pool_out = pool_out, pool_in = pool_in));
    t.push_str(&format!("\n; something has reached the server from the client that is not a plain telling, for time s at rank r: the server goes back to the copy it made before its first turn taken ahead for then or later, of two alike the client's being first, and those turns are thrown away. Every copy is then gone, the earlier ones having been passed. `__w`'s fourth word, 4, is a test's: the server is not put back\nfn __wback(s: i64, r: i64)\n    ap: ptr = addr __wa\n    h: i64 = load ap, 40\n    n: i64 = load ap, 48\n    tp: ptr = addr __wa_t\n    rp: ptr = addr __wa_r\n    j: i64 = loop(i: i64 = 0) bound {n1}\n        done: u1 = cmp.ge i, n\n        if done\n            break i\n        k0: i64 = add h, i\n        k1: i64 = rem k0, {n}\n        ki: index = conv k1\n        t: i64 = load tp, ki, 8\n        q: i64 = load rp, ki, 8\n        gt: u1 = cmp.gt t, s\n        eq: u1 = cmp.eq t, s\n        rge: u1 = cmp.ge q, r\n        tie: u1 = and eq, rge\n        after: u1 = or gt, tie\n        if after\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    some: u1 = cmp.lt j, n\n    if some\n        g0: i64 = add h, j\n        g1: i64 = rem g0, {n}\n        gi: index = conv g1\n        wp: ptr = addr __w\n        but: i64 = load wp, 24\n        bm: i64 = and but, 4\n        put: u1 = cmp.eq bm, 0\n        if put\n            __wrestore(gi)\n        undone: i64 = sub n, j\n        th: i64 = load ap, 56\n        th2: i64 = add th, undone\n        store th2, ap, 56\n        c: ptr = context()\n        x: __ctx = load c\n        to: i64 = get x, __clock\n        __wnote(8, to, undone)\n        store 0: i64, ap, 16\n    pk: i64 = load ap, 16\n    loose: u1 = cmp.eq pk, 0\n    if loose\n        store -1: i64, ap, 24\n    store 0: i64, ap, 40\n    store 0: i64, ap, 48\n    ret\n\n; a plain telling has reached the server: the copies made before turns the client's telling has passed are dropped\nfn __wpassed(pt: i64, pr: i64)\n    ap: ptr = addr __wa\n    h: i64 = load ap, 40\n    n: i64 = load ap, 48\n    tp: ptr = addr __wa_t\n    rp: ptr = addr __wa_r\n    d: i64 = loop(i: i64 = 0) bound {n1}\n        done: u1 = cmp.ge i, n\n        if done\n            break i\n        k0: i64 = add h, i\n        k1: i64 = rem k0, {n}\n        ki: index = conv k1\n        t: i64 = load tp, ki, 8\n        q: i64 = load rp, ki, 8\n        lt: u1 = cmp.lt t, pt\n        eq: u1 = cmp.eq t, pt\n        rlt: u1 = cmp.lt q, pr\n        tie: u1 = and eq, rlt\n        before: u1 = or lt, tie\n        if before\n        else\n            break i\n        i2: i64 = add i, 1\n        continue i2\n    h1: i64 = add h, d\n    h2: i64 = rem h1, {n}\n    n2: i64 = sub n, d\n    store h2, ap, 40\n    store n2, ap, 48\n    ret\n", n = AHEAD, n1 = AHEAD + 1));
    if w.pooled {
        t.push_str("\n; the client's telling has passed the turn the server took ahead as far as a call that crosses: the function it left here is put on the wire now, as part of that turn, and the server is free of it\nfn __wunpark()\n    ap: ptr = addr __wa\n    pk: i64 = load ap, 16\n    s1: i64 = sub pk, 1\n    slot: index = conv s1\n    br: i64 = load ap, 32\n    store br, ap, 88\n    store 0: i64, ap, 16\n    store -1: i64, ap, 24\n    store 0: i64, ap\n    wp: ptr = addr __w\n    store 0: i64, wp, 56\n    pp: ptr = addr __act_pc\n    pc: i64 = load pp, slot, 8\n    __act_run(slot, pc)\n    __act_next()\n    ret\n");
    }
    // the client's kept text
    t.push_str(&format!("\n; a piece of text has reached the client: kept until its time, in the order it came, which is the order of the turns that wrote it. But one written in a turn taken ahead says how much of the client's sending the server had had, and where that is behind and the piece is for the time of the first sending it did not know of, or later, it is void and is dropped unshown. `__w`'s fourth word, 8, is a test's: the client does not drop\nfn __wkeep(kind: i64, a: i64, b: i64, t: i64, r: i64, e: i64)\n    wp: ptr = addr __w\n    sent: i64 = load wp, 64\n    behind: u1 = cmp.lt e, sent\n    if behind\n        e1: i64 = add e, 1\n        m: i64 = rem e1, {sends}\n        mi: index = conv m\n        stp: ptr = addr __ws_t\n        srp: ptr = addr __ws_r\n        st: i64 = load stp, mi, 8\n        sr: i64 = load srp, mi, 8\n        gt: u1 = cmp.gt t, st\n        eq: u1 = cmp.eq t, st\n        rge: u1 = cmp.ge r, sr\n        tie: u1 = and eq, rge\n        after: u1 = or gt, tie\n        but: i64 = load wp, 24\n        bm: i64 = and but, 8\n        drops: u1 = cmp.eq bm, 0\n        stale: u1 = and after, drops\n        if stale\n            __wdropped(kind, a, b, t)\n            ret\n    ap: ptr = addr __wa\n    h: i64 = load ap, 96\n    n: i64 = load ap, 104\n    room: u1 = cmp.lt n, {kept}\n    check room\n    k0: i64 = add h, n\n    k1: i64 = rem k0, {kept}\n    ki: index = conv k1\n    p1: ptr = addr __wk_t\n    store t, p1, ki, 8\n    p2: ptr = addr __wk_r\n    store r, p2, ki, 8\n    p3: ptr = addr __wk_e\n    store e, p3, ki, 8\n    p4: ptr = addr __wk_kind\n    store kind, p4, ki, 8\n    p5: ptr = addr __wk_a\n    store a, p5, ki, 8\n    p6: ptr = addr __wk_b\n    store b, p6, ki, 8\n    n2: i64 = add n, 1\n    store n2, ap, 104\n    ret\n\n; a piece dropped unshown, written in the wire's record for a person: a character, or a block as its length and its first seven characters\nfn __wdropped(kind: i64, a: i64, b: i64, t: i64)\n    block: u1 = cmp.eq kind, 2\n    if block\n        tp: ptr = addr __wq_text\n        ao: index = conv a\n        k: index = conv b\n        qs: ptr(u8) = ptradd tp, ao\n        v: u8[] = pack qs, k, 1\n        w7: i64 = loop(i: index = 0, acc: i64 = b) bound 8\n            end: u1 = cmp.ge i, k\n            full: u1 = cmp.ge i, 7\n            stop: u1 = or end, full\n            if stop\n                break acc\n            by: u8 = load v, i\n            bw: i64 = conv by\n            sh: i64 = mul acc, 256\n            acc2: i64 = add sh, bw\n            i2: index = add i, 1\n            continue i2, acc2\n        __wnote(7, t, w7)\n        ret\n    __wnote(6, t, a)\n    ret\n\n; the client has sent something, its `sent`-th, for time s at rank r: every piece it is keeping that was written in ignorance of it and is for then or later is void, and is dropped now, with nothing more on the wire\nfn __wvoid(s: i64, r: i64, sent: i64)\n    wp: ptr = addr __w\n    but: i64 = load wp, 24\n    bm: i64 = and but, 8\n    off: u1 = cmp.ne bm, 0\n    if off\n        ret\n    ap: ptr = addr __wa\n    h: i64 = load ap, 96\n    n: i64 = load ap, 104\n    p1: ptr = addr __wk_t\n    p2: ptr = addr __wk_r\n    p3: ptr = addr __wk_e\n    p4: ptr = addr __wk_kind\n    p5: ptr = addr __wk_a\n    p6: ptr = addr __wk_b\n    left: i64 = loop(i: i64 = 0, o: i64 = 0) bound {kept1}\n        done: u1 = cmp.ge i, n\n        if done\n            break o\n        k0: i64 = add h, i\n        k1: i64 = rem k0, {kept}\n        ki: index = conv k1\n        t: i64 = load p1, ki, 8\n        q: i64 = load p2, ki, 8\n        e: i64 = load p3, ki, 8\n        kind: i64 = load p4, ki, 8\n        a: i64 = load p5, ki, 8\n        b: i64 = load p6, ki, 8\n        behind: u1 = cmp.lt e, sent\n        gt: u1 = cmp.gt t, s\n        eq: u1 = cmp.eq t, s\n        rge: u1 = cmp.ge q, r\n        tie: u1 = and eq, rge\n        after: u1 = or gt, tie\n        stale: u1 = and behind, after\n        i2: i64 = add i, 1\n        if stale\n            __wdropped(kind, a, b, t)\n            continue i2, o\n        j0: i64 = add h, o\n        j1: i64 = rem j0, {kept}\n        ji: index = conv j1\n        store t, p1, ji, 8\n        store q, p2, ji, 8\n        store e, p3, ji, 8\n        store kind, p4, ji, 8\n        store a, p5, ji, 8\n        store b, p6, ji, 8\n        o2: i64 = add o, 1\n        continue i2, o2\n    store left, ap, 104\n    ret\n", sends = SENDS, kept = KEPT, kept1 = KEPT + 1));
    t.push_str(&format!("\n; the piece the client has kept longest is shown: the client's clock moves up to its time and marks its output there, and it goes into the capture, where `__shown` writes down when by the world's clock\nfn __wshow()\n    ap: ptr = addr __wa\n    h: i64 = load ap, 96\n    n: i64 = load ap, 104\n    hi: index = conv h\n    p1: ptr = addr __wk_t\n    t: i64 = load p1, hi, 8\n    p4: ptr = addr __wk_kind\n    kind: i64 = load p4, hi, 8\n    p5: ptr = addr __wk_a\n    a: i64 = load p5, hi, 8\n    p6: ptr = addr __wk_b\n    b: i64 = load p6, hi, 8\n    h1: i64 = add h, 1\n    h2: i64 = rem h1, {kept}\n    n2: i64 = sub n, 1\n    store h2, ap, 96\n    store n2, ap, 104\n    __wclock(t)\n{ch}{block}    ret\n\n; something that is not text has reached the client from the server, for time s at rank r: every piece kept that is for then or earlier is shown first, being before it in the order of the server's turns\nfn __wflush(s: i64, r: i64)\n    ap: ptr = addr __wa\n    p1: ptr = addr __wk_t\n    p2: ptr = addr __wk_r\n    loop(i: i64 = 0) bound {kept1}\n        n: i64 = load ap, 104\n        some: u1 = cmp.gt n, 0\n        if some\n        else\n            break\n        h: i64 = load ap, 96\n        hi: index = conv h\n        t: i64 = load p1, hi, 8\n        q: i64 = load p2, hi, 8\n        lt: u1 = cmp.lt t, s\n        eq: u1 = cmp.eq t, s\n        rle: u1 = cmp.le q, r\n        tie: u1 = and eq, rle\n        due: u1 = or lt, tie\n        if due\n        else\n            break\n        __wshow()\n        i2: i64 = add i, 1\n        continue i2\n    ret\n", kept = KEPT, kept1 = KEPT + 1, ch = if w.writes.0 { "    is1: u1 = cmp.eq kind, 1\n    if is1\n        ch: u8 = conv a\n        __cap_ch(ch)\n" } else { "" }, block = if w.writes.1 { "    is2: u1 = cmp.eq kind, 2\n    if is2\n        ao: index = conv a\n        bk: index = conv b\n        tp: ptr = addr __wq_text\n        qs: ptr(u8) = ptradd tp, ao\n        v: u8[] = pack qs, bk, 1\n        __cap_block(v)\n" } else { "" }));
    t
}

impl Placed {
    fn client(&self) -> usize {
        self.units.len()
    }

    fn at(&self, e: usize, server: &BTreeSet<usize>) -> usize {
        server.contains(&e) as usize
    }

    fn who(&self, e: usize) -> String {
        if e == self.client() { "the client's own".to_string() } else { format!("'{}'", self.units[e].shown()) }
    }

    /// every unit on the server
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
            // (every unit at the client, which a move may go to)
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
    pub fn label(&self, server: &BTreeSet<usize>) -> String {
        if server.is_empty() {
            "server: nothing".to_string()
        } else if server.len() == self.units.len() && server.len() > 1 {
            "server: all".to_string()
        } else {
            format!("server: {}", server.iter().map(|&u| match &self.units[u].words { Some(w) if self.units.iter().filter(|o| o.words.as_ref() == Some(w)).count() == 1 => w.clone(), _ => self.units[u].name.clone() }).collect::<Vec<_>>().join(", "))
        }
    }

    /// Is this field a group's or everyone's stream whose home is the
    /// server at this cut (hop fifty-eight)? The home of a shared scope
    /// is the place every other place has a wire to (fm3 places.md, "what
    /// the product says"): of two, the server, the place without
    /// devices. Where the cut puts nothing on the server the program is at
    /// one place, and the client is the home
    fn homed(&self, name: &str, server: &BTreeSet<usize>) -> bool {
        !server.is_empty() && self.homes.iter().any(|(n, w)| n == name && w != "user")
    }

    /// which fields the wire carries from each place at this cut, and
    /// where each stored stream is read
    fn carries(&self, server: &BTreeSet<usize>) -> (Vec<[bool; 2]>, Vec<[bool; 2]>, Vec<[bool; 2]>) {
        let n = self.facts.len();
        let fields = self.fields.iter().map(|(name, _, _)| {
            let mut c = [false; 2];
            for p in 0..2 {
                // (where the wire takes time the list and the pool run
                // at both places, and what they read is read on the server too)
                let written = (0..n).any(|e| self.at(e, server) == p && self.facts[e].writes.contains(name)) || self.wire && p == 1 && self.glue.writes.contains(name);
                let lives = (0..n).any(|e| self.at(e, server) != p && (self.facts[e].writes.contains(name) || self.facts[e].reads.contains(name))) || self.wire && p == 0 && (self.glue.writes.contains(name) || self.glue.reads.contains(name));
                // (a group's or everyone's stream lives at its home too,
                // the server, where the cut puts anything there: what the
                // client writes of it is carried there, hop fifty-eight)
                let home = p == 0 && self.homed(name, server);
                c[p] = written && (lives || home);
            }
            c
        }).collect();
        // (... and a push into a stored one is made again at the home)
        let read: Vec<[bool; 2]> = (0..self.rings.len()).map(|r| [0, 1].map(|p| (0..n).any(|e| self.at(e, server) == p && self.facts[e].ring_reads.contains(&r)) || p == 1 && self.rings[r].0.iter().any(|f| self.homed(f, server)))).collect();
        let pushed = (0..self.rings.len()).map(|r| [0, 1].map(|p| (0..n).any(|e| self.at(e, server) == p && self.facts[e].ring_pushes.contains(&r)))).collect();
        (fields, read, pushed)
    }

    /// Why a cut cannot be run yet (fm3 question 152): it would carry
    /// an address from one place's memory to the other
    pub fn refused(&self, server: &BTreeSet<usize>) -> Option<String> {
        if let Some(why) = self.refused_wire(server) {
            return Some(why);
        }
        for c in &self.crossings {
            if self.at(c.from, server) == self.at(c.to, server) || c.to == self.client() {
                continue;
            }
            // (an array or a string is copied at the crossing, fm3 log
            // 267: what is left is a stream, which is its storage)
            let values = self.copied.get(&c.callee);
            if let Some(ty) = self.sigs.get(&c.callee).and_then(|s| s.iter().enumerate().find(|(i, t)| (t.contains('$') || t.contains('[') || t.contains("ptr")) && !values.is_some_and(|v| v.contains(i))).map(|(_, t)| t)) {
                let (server_one, other) = if server.contains(&c.to) { (c.to, c.from) } else { (c.from, c.to) };
                return Some(format!("{} cannot run apart from {} yet: {} is handed, or gives, {}, which is an address in the memory of the place it was made at", self.who(server_one), self.who(other), self.who(c.to), if ty.contains('$') { "a stream" } else { address(ty) }));
            }
        }
        let (fields, read, pushed) = self.carries(server);
        for ((name, ty, plain), c) in self.fields.iter().zip(&fields) {
            if (c[0] || c[1]) && !plain && !ty.ends_with('$') {
                return Some(format!("'{}{}' is written at one place and read at the other, and it holds {}, which is an address in the memory of the place it was made at", name.trim_start_matches("__"), if ty.ends_with('$') { "$" } else { "" }, address(ty)));
            }
        }
        // one reader of a stored stream at both places: its place in
        // the stream is one, and each place has a stream of its own
        for (names, _) in self.rings.iter().filter(|(n, _)| !n.is_empty()) {
            for field in names {
                let reads = |p: usize| (0..self.facts.len()).any(|e| self.at(e, server) == p && self.facts[e].through.contains(field));
                let takes = |p: usize| (0..self.facts.len()).any(|e| self.at(e, server) == p && self.facts[e].taken.contains(field));
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
    fn refused_wire(&self, server: &BTreeSet<usize>) -> Option<String> {
        if !self.wire {
            return None;
        }
        for c in &self.crossings {
            if c.to == self.client() {
                continue;
            }
            if c.from == self.client() {
                if matches!(c.caller.as_str(), "__run" | "__act_go") || self.at(c.to, server) == 0 {
                    continue;
                }
                return Some(format!("{} cannot run on the server over a wire that takes time yet: its function `{}` is called by `{}`, the client's own, which waits for it on the machine's stack: {}", self.who(c.to), c.callee, c.caller, self.why(&c.callee)));
            }
            if self.at(c.from, server) != self.at(c.to, server) {
                let (server_one, other) = if server.contains(&c.to) { (c.to, c.from) } else { (c.from, c.to) };
                // (the caller's reason where it is the caller that is
                // still on the stack; the callee's where a function left
                // at its calls calls one that is not, fm3 log 280)
                return Some(match self.stack.get(&c.caller) {
                    Some(w) => format!("{} cannot run apart from {} over a wire that takes time yet: `{}` calls `{}` and waits for it on the machine's stack, not being left at its calls yet: {}", self.who(server_one), self.who(other), c.caller, c.callee, w.said),
                    None => format!("{} cannot run apart from {} over a wire that takes time yet: `{}` calls `{}`, which is not left at its calls yet, and waits for it on the machine's stack: {}", self.who(server_one), self.who(other), c.caller, c.callee, self.why(&c.callee)),
                });
            }
        }
        // (two functions partway at two places and due at one time are
        // taken first begun first, each record carrying when it was
        // begun and a telling's rank that of its sender's earliest:
        // fm3 question 155, log 278. Such a cut was refused)
        let (fields, read, pushed) = self.carries(server);
        for ((name, ty, plain), c) in self.fields.iter().zip(&fields) {
            if (c[0] || c[1]) && !(*plain && word(self.named.get(ty).map_or(ty.as_str(), |t| t.as_str())) && ty != "ptr") {
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

    /// why a function is still on the machine's stack, as a sentence
    fn why(&self, f: &str) -> String {
        self.stack.get(f).map_or("it is not reached from where a turn begins".to_string(), |w| w.said.clone())
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
    pub fn cut(&self, server: &BTreeSet<usize>) -> Cut {
        self.cut_but(server, None)
    }

    /// ... and the same with one field or one stream deliberately not
    /// carried, which only a test asks for: the proof that a place has
    /// memory of its own is that the program then goes wrong
    pub fn cut_but(&self, server: &BTreeSet<usize>, dropped: Option<&str>) -> Cut {
        let set = |i: usize| ("__cut_set".to_string(), vec![i as i64, 1]);
        let mut before: Vec<(String, Vec<i64>)> = server.iter().map(|&u| set(u)).collect();
        let (fields, read, _) = self.carries(server);
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
        Cut { before, label: self.label(server) }
    }

    /// What a move from one cut to another hands across, once (fm3
    /// cut.md, "the cut, moved"; log 265). The wire's carrying is lazy,
    /// a field copied as the wire is crossed, and the move is made at
    /// the client. So a field goes from the server to the client where no
    /// unit at the client wrote it or was being given it before and one
    /// uses it there after; and from the client to the server where a unit at
    /// the client wrote it before, one at the server uses it after, and none
    /// at the client writes it after, when the wire would carry it at
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
            // (where the wire takes time a clock's word is at the place
            // its line is at, and goes where the line goes, fm3 log 276)
            if let Some((_, line)) = self.clock_dues.iter().find(|(d, _)| d == name) {
                let dir = match (a.contains(line), b.contains(line)) {
                    (false, true) => 1,
                    (true, false) => 2,
                    _ => continue,
                };
                out.push(Hand { at: j, dir, what: self.field_said(name), name: name.clone() });
                continue;
            }
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
                    out.push(Hand { at, dir: if q == 1 { 1 } else { 2 }, what: format!("the items of {}$ that {} still holds, with their times, and its counts", names[0], if q == 1 { "the client" } else { "the server" }), name: names[0].clone() });
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
        // over a wire that takes time (fm3 log 276) a stream's storage
        // is read where it lies when the move's item arrives, which is
        // what was sent only where the place that sent it reads the
        // stream no longer
        if self.wire {
            let (_, read_a, _) = self.carries(a);
            let (_, read_b, _) = self.carries(b);
            for (r, (names, _)) in self.rings.iter().enumerate().filter(|(_, (n, _))| !n.is_empty()) {
                if (0..2).any(|q| read_b[r][q] && !read_a[r][q] && read_a[r][1 - q] && read_b[r][1 - q]) {
                    return Some(format!("'{}$' would be read at both places after the move, and newly at one: over a wire that takes time what a stream holds is handed across where the place that sends it reads it no longer, and not yet where it goes on reading it", names[0]));
                }
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
            label.push_str(&format!(", then {} at {}", self.label(next).trim_start_matches("server: "), super::store::spell_time(*t)));
            was = next;
        }
        Cut { before, label }
    }

    /// The placement of a cut that moves: the first cut's table, and
    /// for each move when it is, which units change place, what is
    /// handed across, read off the same facts that fill the move's
    /// table, and the table of the cut it goes to
    pub fn table_moved(&self, store: &str, first: &BTreeSet<usize>, then: &[(BTreeSet<usize>, i64)], cases: &[String]) -> String {
        let place = |p: usize| if p == 0 { "client" } else { "server" };
        let mut out = self.table(store, first, cases);
        let mut was = first;
        for (next, t) in then {
            out.push_str(&format!("\nat {} the cut moves: {}, the first thing due at that time{}\n\n", super::store::spell_time(*t), self.label(next), if self.wire { ", said at the client" } else { "" }));
            if let Some(why) = self.move_refused(was, next) {
                out.push_str(&format!("refused: {}\n", why));
                return out;
            }
            let w1 = self.units.iter().map(|u| u.name.len()).max().unwrap_or(4).max(4);
            out.push_str(&format!("{:<w1$}  {:<6}  {}\n", "unit", "was", "is", w1 = w1));
            for (u, unit) in self.units.iter().enumerate().filter(|(u, _)| was.contains(u) != next.contains(u)) {
                out.push_str(&format!("{:<w1$}  {:<6}  {}\n", unit.name, place(self.at(u, was)), place(self.at(u, next)), w1 = w1));
            }
            let how = |dir: u8| if !self.wire { "copied once, in the move's turn" } else if dir == 1 { "sent with the move's word, and the server's when that arrives" } else { "sent by the server when the move's word reaches it, and the client's when that arrives" };
            let mut rows: Vec<(String, String, String, String)> = vec![("the table".to_string(), format!("{} bytes of the cut", self.sizes.0), String::new(), if self.wire { "each place's own bytes rewritten when the move reaches it: the client's in the move's turn, the server's when the word arrives; a unit between the two is at neither" } else { "rewritten in the move's turn: where each unit runs, and what the wire carries from then on" }.to_string())];
            let hands = self.hands(was, next);
            for h in &hands {
                let kind = if self.rings.iter().any(|(n, _)| n.contains(&h.name)) { "a stream" } else if h.name.starts_with("__enabled_") || h.name.starts_with("__on_") { "a switch" } else if h.name.starts_with("__") { "kept" } else { "an item" };
                rows.push((kind.to_string(), h.what.clone(), if h.dir == 1 { "client → server" } else { "server → client" }.to_string(), how(h.dir).to_string()));
            }
            if hands.is_empty() {
                rows.push(("nothing else".to_string(), "no field of the context, and no stored stream".to_string(), String::new(), "none that a unit that moves reads or writes is at the place it leaves and not at the place it goes to".to_string()));
            }
            let partway: Vec<String> = self.units.iter().enumerate().filter(|(u, _)| self.left.contains(u)).map(|(_, unit)| format!("`{}`", unit.shown())).collect::<BTreeSet<_>>().into_iter().collect();
            let partway: Vec<String> = if self.wire { self.units.iter().enumerate().filter(|(u, _)| self.left.contains(u) && was.contains(u) != next.contains(u)).map(|(_, unit)| format!("`{}`", unit.shown())).collect::<BTreeSet<_>>().into_iter().collect() } else { partway };
            if !partway.is_empty() && self.wire {
                rows.push(("a function".to_string(), format!("partway, of a unit that moves: {}", partway.join(", ")), "with its unit".to_string(), format!("its record is a place of the pool, {} columns, at the place its function is: an item of the wire each, with when it is next due, where it goes on from and what it keeps", self.pool.len())));
            } else if !partway.is_empty() {
                rows.push(("not handed".to_string(), format!("a function partway: {}", partway.join(", ")), String::new(), format!("its record is a place of the pool, {} columns, which is the client's between its turns: when it is next due, where it goes on from, what it keeps", self.pool.len())));
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

    /// Where each stream with a scope word is, as the placement says it
    /// (hop fifty-eight; fm3 places.md, "what the product says": "the
    /// compiler prints a placement: every stream's home"). Nothing for a
    /// store that names no scope word
    pub fn homes_said(&self, server: &BTreeSet<usize>) -> String {
        if self.homes.is_empty() {
            return String::new();
        }
        let w = self.homes.iter().map(|(n, _)| n.len() + 1).max().unwrap_or(0).max(6);
        let mut out = "\nwhose each stream is, and where\n".to_string();
        for (n, scope) in &self.homes {
            let whose = match scope.as_str() { "user" => "a user's", "group" => "a group's", _ => "everyone's" };
            let said = if scope == "user" {
                "one person with one device: placed as the device's own is, at the place of the lines that use it".to_string()
            } else if server.is_empty() && self.pairs {
                "its home the client with one person: nothing is on the server, so the program is at one place; with two people or more the server, which every person's device has a wire to, there being none between two devices".to_string()
            } else if server.is_empty() {
                "its home the client: nothing is on the server, so the program is at one place".to_string()
            } else {
                "its home the server, the place without devices, which every other place has a wire to; a copy at the client".to_string()
            };
            out.push_str(&format!("{:<w$}  {:<10}  {}\n", format!("{}$", n), whose, said, w = w));
        }
        out
    }

    /// The places of a build with a pair of contexts a person (hop
    /// fifty-nine, three places): a client each, and the server
    pub fn places_said(&self) -> String {
        if !self.pairs {
            return String::new();
        }
        format!("\nthe places, for a case that names people\neach person's device  a client of their own: their output, their input, their list of things going on, and what the cut puts at the client, in a context of theirs; {} people at most\nthe server            what the cut puts on the server, in a context for each person, and the home of every group's and everyone's stream\nthe wires             one from each person's device to the server, none between two devices; they take no time\n", super::lower::PEOPLE)
    }

    /// The placement (fm3 distribution.md, "what the compiler does",
    /// 7): every unit and where it runs, and every crossing, what kind
    /// it is and what is carried
    pub fn table(&self, store: &str, server: &BTreeSet<usize>, cases: &[String]) -> String {
        let place = |p: usize| if p == 0 { "client" } else { "server" };
        let mut out = format!("{}: the cut, {}\n\n", store, self.label(server));
        let w1 = self.units.iter().map(|u| u.name.len()).max().unwrap_or(4).max(4);
        out.push_str(&format!("{:<w1$}  {:<6}  {}\n", "unit", "runs", "what it is", w1 = w1));
        for (u, unit) in self.units.iter().enumerate() {
            out.push_str(&format!("{:<w1$}  {:<6}  {}\n", unit.name, place(self.at(u, server)), match (&unit.words, &unit.said) { (Some(w), _) => format!("the function `{}`", w), (None, Some(s)) => format!("the line `{}`", s), _ => "a function".to_string() }, w1 = w1));
        }
        out.push_str(&format!("{:<w1$}  {:<6}  {}\n", "", "client", "the devices: the output, the input, the list of things going on; and a case's call", w1 = w1));
        out.push_str(&self.homes_said(server));
        out.push_str(&self.places_said());
        if let Some(why) = self.refused(server) {
            out.push_str(&format!("\nrefused: {}\n", why));
            return out;
        }
        let mut rows: Vec<(String, String, String, String)> = Vec::new();
        let name = |e: usize| if e == self.client() { "the client".to_string() } else { self.units[e].name.clone() };
        // (of the client's own, a case's twin is the case's call, and
        // the pool's turn is the list's)
        let from = |c: &Crossing| if c.from != self.client() { name(c.from) } else if c.caller.starts_with("__whole_") || c.caller.starts_with("__said_") { "a case's own".to_string() } else if matches!(c.caller.as_str(), "__act_go" | "__turns" | "__close") { "the list".to_string() } else { name(c.from) };
        for c in &self.crossings {
            let (a, b) = (self.at(c.from, server), self.at(c.to, server));
            if a == b {
                continue;
            }
            // (an array or a string is copied at the crossing, fm3 log 267)
            let values = self.copied.get(&c.callee);
            let types: Vec<String> = self.sigs.get(&c.callee).cloned().unwrap_or_default().into_iter().enumerate().map(|(i, t)| if !values.is_some_and(|v| v.contains(&i)) { t } else if t == "u8$" { "a string, copied".to_string() } else { format!("an array of `{}`, copied", t.trim_end_matches('$')) }).collect();
            let (kind, what) = if c.to == self.client() {
                match c.callee.as_str() {
                    "__out_ch" | "__out_block" if self.wire => ("text".to_string(), "characters for out$, with their time: shown at the later of it and when they arrive".to_string()),
                    "__out_ch" | "__out_block" => ("text".to_string(), "characters for out$, written at the client at their time".to_string()),
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
        // a case's own call, made at the client, of a function placed on the server
        for (u, unit) in self.units.iter().enumerate().filter(|(u, _)| server.contains(u)) {
            if unit.irs.iter().any(|i| cases.contains(i)) && !self.crossings.iter().any(|c| c.from == self.client() && c.to == u) {
                let types = unit.irs.iter().find_map(|i| self.sigs.get(i)).cloned().unwrap_or_default();
                rows.push(("a call".to_string(), format!("a case's own → {}", unit.name), "client → server".to_string(), if types.is_empty() { "nothing over, its end back".to_string() } else { format!("{} over and back, and its end", types.join(", ")) }));
            }
        }
        let (fields, read, pushed) = self.carries(server);
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
/// two places", step 1): every unit on the server, and each unit alone on the server
/// What a record arriving in a call or its end copies (`__act_land_at`,
/// hop sixty-three), read back from the build's text: for each
/// continuation, each column, the type of its items, and whether it is
/// a view
fn landed_at(ir: &str) -> Vec<(i64, Vec<(String, String, bool)>)> {
    let lines: Vec<&str> = ir.lines().collect();
    // the item's type of each copy, from its own `sizeof`
    let item = |f: &str| -> Option<String> {
        let at = lines.iter().position(|l| l.starts_with(&format!("fn {}(", f)))?;
        lines[at + 1..].iter().take_while(|l| l.starts_with("    ")).find_map(|l| l.trim().strip_prefix("sz: index = sizeof ").map(str::to_string))
    };
    let Some(at) = lines.iter().position(|l| l.starts_with("fn __act_land_at(")) else { return Vec::new() };
    let mut out: Vec<(i64, Vec<(String, String, bool)>)> = Vec::new();
    let mut col: Option<String> = None;
    for l in lines[at + 1..].iter().take_while(|l| l.starts_with("    ")) {
        let t = l.trim();
        if let Some(id) = t.split_once(": u1 = cmp.eq k, ").and_then(|(_, n)| n.parse::<i64>().ok()) {
            out.push((id, Vec::new()));
        } else if let Some((_, a)) = t.split_once(": ptr = addr ") {
            if let Some(c) = a.strip_suffix("__ring").or_else(|| a.strip_suffix("__ptr")) {
                col = Some(c.to_string());
            }
        } else if let Some(call) = t.strip_prefix("__act_land_") {
            let (view, f) = match call.strip_prefix("view_") {
                Some(_) => (true, format!("__act_land_{}", call.split('(').next().unwrap_or(""))),
                None => (false, format!("__act_land_{}", call.split('(').next().unwrap_or(""))),
            };
            if let (Some(c), Some(e), Some(last)) = (col.take(), item(&f), out.last_mut()) {
                last.1.push((c, e, view));
            }
        }
    }
    out.retain(|(_, cs)| !cs.is_empty());
    out
}

/// (hop sixty-three) the values a record carries over a real wire: where
/// a row is a function partway, each array or string the continuation it
/// goes on with reads that the sending place wrote, the words of its
/// storage after the record's columns and fields, length first, so that
/// the receiver lays the copy out in its own arena and the record points
/// into it, as `__act_land_at` copies it over the simulated wire. A
/// column that holds what an earlier column of the row holds says so,
/// and the receiver gives it the same copy. None where nothing is landed
fn land_vals_text(ir: &str, flight: usize) -> Option<String> {
    let landed = landed_at(ir);
    if landed.is_empty() {
        return None;
    }
    const VALS: usize = 8192;
    let mut t = format!("\n; (hop sixty-three) the arrays and strings a function partway carries over a real wire, the words of their storage after its record and fields: packed at the sender when the row is taken, and laid out at the receiver when it is put on. `__wq_vals` holds a row's, and its first word how many\ndata __wq_vals: array(i64, {n})\n", n = VALS + 1);
    let mut items: BTreeSet<(String, bool)> = BTreeSet::new();
    for (_, cs) in &landed {
        for (_, e, v) in cs {
            items.insert((e.clone(), *v));
        }
    }
    for (e, view) in &items {
        let x = e.replace(|c: char| !is_ident(c), "_");
        if *view {
            t.push_str(&format!("\n; a view's items, packed from word o: 0 where it holds none, else its length and its items\nfn __wq_pack_view_{x}(c: ptr, lc: ptr, sc: ptr, fl: index, o: index) -> index\n    sp: ptr = addr __wq_vals\n    r: i64 = load c, fl, 8\n    none: u1 = cmp.eq r, 0\n    if none\n        store 0: i64, sp, o, 8\n        o1: index = add o, 1\n        ret o1\n    q0: ptr = load c, fl, 8\n    n: index = load lc, fl, 8\n    st: index = load sc, fl, 8\n    store 1: i64, sp, o, 8\n    o1: index = add o, 1\n    nw: i64 = conv n\n    store nw, sp, o1, 8\n    o2: index = add o, 2\n    sz: index = sizeof {e}\n    bytes: index = mul sz, n\n    b7: index = add bytes, 7\n    words: index = div b7, 8\n    end: index = add o2, words\n    fits: u1 = cmp.le end, {vals}\n    check fits\n    q: ptr({e}) = cast q0\n    s: {e}[] = pack q, n, st\n    o8: index = mul o2, 8\n    dp: ptr = ptradd sp, o8\n    dq: ptr({e}) = cast dp\n    d: {e}[] = pack dq, n, 1\n    copy d, s\n    ret end\n\n; ... and laid out in this place's arena from word o, the column pointing into it\nfn __wq_unpack_view_{x}(c: ptr, lc: ptr, sc: ptr, fl: index, o: index) -> index\n    sp: ptr = addr __wq_vals\n    m: i64 = load sp, o, 8\n    none: u1 = cmp.eq m, 0\n    if none\n        store 0: i64, c, fl, 8\n        o1: index = add o, 1\n        ret o1\n    o1: index = add o, 1\n    nw: i64 = load sp, o1, 8\n    n: index = conv nw\n    o2: index = add o, 2\n    sz: index = sizeof {e}\n    one: index = const 1\n    cap: index = max(n, one)\n    cb: index = mul sz, cap\n    total: index = add cb, 16\n    ar: ptr = addr __arena\n    vb: ptr = arena_alloc(ar, total)\n    buffer_init(vb, sz, cap)\n    slots: ptr = ptradd vb, 16\n    qb: ptr({e}) = cast slots\n    d: {e}[] = pack qb, n, 1\n    o8: index = mul o2, 8\n    sp2: ptr = ptradd sp, o8\n    sq: ptr({e}) = cast sp2\n    s: {e}[] = pack sq, n, 1\n    copy d, s\n    store slots, c, fl, 8\n    store n, lc, fl, 8\n    store one, sc, fl, 8\n    bytes: index = mul sz, n\n    b7: index = add bytes, 7\n    words: index = div b7, 8\n    end: index = add o2, words\n    ret end\n", x = x, e = e, vals = VALS));
        } else {
            t.push_str(&format!("\n; an array's or a string's storage, packed from word o: 0 where the column holds none; else 1, the eight words of its head, and a queue's resident items, or a ring's buffer whole, its room first, and where it has one its buffer of ticks, its room first\nfn __wq_pack_ring_{x}(c: ptr, fl: index, o: index) -> index\n    sp: ptr = addr __wq_vals\n    r0: i64 = load c, fl, 8\n    none: u1 = cmp.eq r0, 0\n    if none\n        store 0: i64, sp, o, 8\n        o1: index = add o, 1\n        ret o1\n    r: ptr = load c, fl, 8\n    store 1: i64, sp, o, 8\n    loop(i: index = 0) bound 9\n        hd: u1 = cmp.ge i, 8\n        if hd\n            break\n        hv: i64 = load r, i, 8\n        oi: index = add o, i\n        oj: index = add oi, 1\n        store hv, sp, oj, 8\n        i2: index = add i, 1\n        continue i2\n    o2: index = add o, 9\n    st: i64 = load r, 40\n    w: index = load r, 24\n    stepped: u1 = cmp.ne st, 0\n    worded: u1 = cmp.ne w, 0\n    queue: u1 = and stepped, worded\n    sz: index = sizeof {e}\n    if queue\n        pushed: index = load r\n        qo: index = load r, 8\n        k: index = sub pushed, qo\n        kb: index = mul sz, k\n        k7: index = add kb, 7\n        kw: index = div k7, 8\n        end: index = add o2, kw\n        fits: u1 = cmp.le end, {vals}\n        check fits\n        qa: ptr({e}) = load r, 16\n        s: {e}[] = pack qa, k, 1\n        o8: index = mul o2, 8\n        dp: ptr = ptradd sp, o8\n        dq: ptr({e}) = cast dp\n        d: {e}[] = pack dq, k, 1\n        copy d, s\n        ret end\n    wa: ptr = load r, 16\n    va: {e}[] = slice wa\n    cap: index = len va\n    cw: i64 = conv cap\n    store cw, sp, o2, 8\n    o3: index = add o2, 1\n    cb: index = mul sz, cap\n    c7: index = add cb, 7\n    cwn: index = div c7, 8\n    e1: index = add o3, cwn\n    f1: u1 = cmp.le e1, {vals}\n    check f1\n    o8r: index = mul o3, 8\n    dpr: ptr = ptradd sp, o8r\n    dqr: ptr({e}) = cast dpr\n    dr: {e}[] = pack dqr, cap, 1\n    copy dr, va\n    if worded\n    else\n        ret e1\n    ta: ptr = load r, 24\n    ka: i64[] = slice ta\n    tcap: index = len ka\n    tw: i64 = conv tcap\n    store tw, sp, e1, 8\n    o4: index = add e1, 1\n    e2: index = add o4, tcap\n    f2: u1 = cmp.le e2, {vals}\n    check f2\n    o48: index = mul o4, 8\n    tp: ptr = ptradd sp, o48\n    tq: ptr(i64) = cast tp\n    td: i64[] = pack tq, tcap, 1\n    copy td, ka\n    ret e2\n\n; ... and laid out in this place's arena from word o, the column pointing into it: a queue with the room it was made with, a ring with its buffers\nfn __wq_unpack_ring_{x}(c: ptr, fl: index, o: index) -> index\n    sp: ptr = addr __wq_vals\n    m: i64 = load sp, o, 8\n    none: u1 = cmp.eq m, 0\n    if none\n        store 0: i64, c, fl, 8\n        o1: index = add o, 1\n        ret o1\n    ar: ptr = addr __arena\n    n: ptr = arena_alloc(ar, 64)\n    loop(i: index = 0) bound 9\n        hd: u1 = cmp.ge i, 8\n        if hd\n            break\n        oi: index = add o, i\n        oj: index = add oi, 1\n        hv: i64 = load sp, oj, 8\n        store hv, n, i, 8\n        i2: index = add i, 1\n        continue i2\n    o2: index = add o, 9\n    st: i64 = load n, 40\n    w: index = load n, 24\n    stepped: u1 = cmp.ne st, 0\n    worded: u1 = cmp.ne w, 0\n    queue: u1 = and stepped, worded\n    sz: index = sizeof {e}\n    if queue\n        pushed: index = load n\n        qo: index = load n, 8\n        k: index = sub pushed, qo\n        wb0: index = mul sz, w\n        total: index = add wb0, 16\n        vb: ptr = arena_alloc(ar, total)\n        buffer_init(vb, sz, w)\n        slots: ptr = ptradd vb, 16\n        qb: ptr({e}) = cast slots\n        d: {e}[] = pack qb, k, 1\n        o8: index = mul o2, 8\n        sp2: ptr = ptradd sp, o8\n        sq: ptr({e}) = cast sp2\n        s: {e}[] = pack sq, k, 1\n        copy d, s\n        store slots, n, 16\n        store n, c, fl, 8\n        kb: index = mul sz, k\n        k7: index = add kb, 7\n        kw: index = div k7, 8\n        end: index = add o2, kw\n        ret end\n    cw: i64 = load sp, o2, 8\n    cap: index = conv cw\n    o3: index = add o2, 1\n    cb: index = mul sz, cap\n    total2: index = add cb, 16\n    wb: ptr = arena_alloc(ar, total2)\n    buffer_init(wb, sz, cap)\n    vc: {e}[] = slice wb\n    o8r: index = mul o3, 8\n    sp3: ptr = ptradd sp, o8r\n    sq3: ptr({e}) = cast sp3\n    vs: {e}[] = pack sq3, cap, 1\n    copy vc, vs\n    store wb, n, 16\n    c7: index = add cb, 7\n    cwn: index = div c7, 8\n    e1: index = add o3, cwn\n    store n, c, fl, 8\n    if worded\n    else\n        ret e1\n    tw: i64 = load sp, e1, 8\n    tcap: index = conv tw\n    o4: index = add e1, 1\n    tb0: index = mul tcap, 8\n    ttotal: index = add tb0, 16\n    tb: ptr = arena_alloc(ar, ttotal)\n    eight: index = const 8\n    buffer_init(tb, eight, tcap)\n    kb2: i64[] = slice tb\n    o48: index = mul o4, 8\n    tp: ptr = ptradd sp, o48\n    tq: ptr(i64) = cast tp\n    ks: i64[] = pack tq, tcap, 1\n    copy kb2, ks\n    store tb, n, 24\n    e2: index = add o4, tcap\n    ret e2\n", x = x, e = e, vals = VALS));
        }
    }
    // by the continuation the row goes on with
    let mut pack = format!("\n; a row toward the other place that is a function partway: its values packed, by the continuation it goes on with; how many words\nfn __wq_vals_pack(row: index) -> i64\n    sp: ptr = addr __wq_vals\n    fl: index = add row, {flight}\n    ap: ptr = addr __wq_a\n    k: i64 = load ap, row, 8\n", flight = flight);
    let mut unpack = format!("\n; ... and a row from it put on here: its values laid out, by the same\nfn __wq_vals_unpack(row: index)\n    sp: ptr = addr __wq_vals\n    fl: index = add row, {flight}\n    ap: ptr = addr __wq_a\n    k: i64 = load ap, row, 8\n", flight = flight);
    for (id, cs) in &landed {
        pack.push_str(&format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        o{id}_0: index = const 1\n", id = id));
        unpack.push_str(&format!("    is{id}: u1 = cmp.eq k, {id}\n    if is{id}\n        o{id}_0: index = const 0\n", id = id));
        for (j, (col, e, view)) in cs.iter().enumerate() {
            let x = e.replace(|c: char| !is_ident(c), "_");
            let (o, o2) = (format!("o{}_{}", id, j), format!("o{}_{}", id, j + 1));
            if *view {
                pack.push_str(&format!("        c{id}_{j}: ptr = addr {col}__ptr\n        l{id}_{j}: ptr = addr {col}__len\n        s{id}_{j}: ptr = addr {col}__stride\n        {o2}: index = __wq_pack_view_{x}(c{id}_{j}, l{id}_{j}, s{id}_{j}, fl, {o})\n", id = id, j = j, col = col, x = x, o = o, o2 = o2));
                unpack.push_str(&format!("        c{id}_{j}: ptr = addr {col}__ptr\n        l{id}_{j}: ptr = addr {col}__len\n        s{id}_{j}: ptr = addr {col}__stride\n        {o2}: index = __wq_unpack_view_{x}(c{id}_{j}, l{id}_{j}, s{id}_{j}, fl, {o})\n", id = id, j = j, col = col, x = x, o = o, o2 = o2));
                continue;
            }
            // (a column holding what an earlier one of the row holds: its number, less than zero)
            let earlier: Vec<(usize, &String)> = cs[..j].iter().enumerate().filter(|(_, (_, _, v))| !*v).map(|(i, (c, _, _))| (i, c)).collect();
            pack.push_str(&format!("        c{id}_{j}: ptr = addr {col}__ring\n        v{id}_{j}: i64 = load c{id}_{j}, fl, 8\n        z{id}_{j}: u1 = cmp.ne v{id}_{j}, 0\n", id = id, j = j, col = col));
            let mut same = format!("        d{id}_{j}: i64 = const 0\n", id = id, j = j);
            for (i, c) in &earlier {
                same.push_str(&format!("        e{id}_{j}_{i}: ptr = addr {c}__ring\n        w{id}_{j}_{i}: i64 = load e{id}_{j}_{i}, fl, 8\n        h{id}_{j}_{i}: u1 = cmp.eq w{id}_{j}_{i}, v{id}_{j}\n        g{id}_{j}_{i}: u1 = and h{id}_{j}_{i}, z{id}_{j}\n        d{id}_{j}_{i}: i64 = if g{id}_{j}_{i}\n            yield -{n}\n        else\n            yield d{id}_{j}{prev}\n", id = id, j = j, i = i, c = c, n = i + 1, prev = if *i == earlier[0].0 { String::new() } else { format!("_{}", earlier.iter().take_while(|(k, _)| k < i).last().unwrap().0) }));
            }
            let dname = if let Some((last, _)) = earlier.last() { format!("d{}_{}_{}", id, j, last) } else { format!("d{}_{}", id, j) };
            pack.push_str(&same);
            pack.push_str(&format!("        r{id}_{j}: u1 = cmp.lt {dn}, 0\n        {o2}: index = if r{id}_{j}\n            store {dn}, sp, {o}, 8\n            q{id}_{j}: index = add {o}, 1\n            yield q{id}_{j}\n        else\n            p{id}_{j}: index = __wq_pack_ring_{x}(c{id}_{j}, fl, {o})\n            yield p{id}_{j}\n", id = id, j = j, dn = dname, o = o, o2 = o2, x = x));
            unpack.push_str(&format!("        c{id}_{j}: ptr = addr {col}__ring\n        m{id}_{j}: i64 = load sp, {o}, 8\n        r{id}_{j}: u1 = cmp.lt m{id}_{j}, 0\n        {o2}: index = if r{id}_{j}\n            b{id}_{j}: i64 = sub 0, m{id}_{j}\n", id = id, j = j, col = col, o = o, o2 = o2));
            for (i, c) in &earlier {
                unpack.push_str(&format!("            t{id}_{j}_{i}: u1 = cmp.eq b{id}_{j}, {n}\n            if t{id}_{j}_{i}\n                u{id}_{j}_{i}: ptr = addr {c}__ring\n                y{id}_{j}_{i}: i64 = load u{id}_{j}_{i}, fl, 8\n                store y{id}_{j}_{i}, c{id}_{j}, fl, 8\n", id = id, j = j, i = i, c = c, n = i + 1));
            }
            unpack.push_str(&format!("            q{id}_{j}: index = add {o}, 1\n            yield q{id}_{j}\n        else\n            p{id}_{j}: index = __wq_unpack_ring_{x}(c{id}_{j}, fl, {o})\n            yield p{id}_{j}\n", id = id, j = j, o = o, x = x));
        }
        let last = format!("o{}_{}", id, cs.len());
        pack.push_str(&format!("        n{id}: i64 = conv {last}\n        store n{id}, sp\n        w{id}: i64 = sub n{id}, 1\n        ret w{id}\n", id = id, last = last));
        unpack.push_str("        ret\n");
    }
    pack.push_str("    store 1: i64, sp\n    ret 0\n");
    unpack.push_str("    ret\n");
    t.push_str(&pack);
    t.push_str(&unpack);
    Some(t)
}

pub fn cuts(p: &Placed) -> Vec<BTreeSet<usize>> {
    let mut out: Vec<BTreeSet<usize>> = Vec::new();
    if p.units.len() > 1 {
        out.push(p.all());
    }
    out.extend((0..p.units.len()).map(|u| [u].into_iter().collect::<BTreeSet<usize>>()));
    out
}

impl Placed {
    /// The real wire, step one (fm3 wire.md; hop sixty): what a build for a
    /// wire that takes time gains so that each place can run in a process
    /// of its own, the harness being the world. The module's queue as the
    /// harness reads and writes it, a row as words, and the few words of
    /// the world it needs; each case's twin in two, its call begun and
    /// what it gave read back; and at each place its own line of the
    /// record for each home kept as a word. Nothing in it knows what a
    /// socket is, and nothing of a build without it changes: the text is
    /// added after the build's own. `ir` is that build's text
    pub fn real_text(&self, ir: &str) -> Result<String, String> {
        if !self.wire {
            return Err("the real wire runs a build for a wire that takes time".into());
        }
        let (q, flight) = (WIRE_Q, 2 * POOL);
        let ranked = ir.contains("\ndata __wq_r: ");
        let ahead = ir.contains("\ndata __wa: ");
        let runs = ir.contains("\nfn __run()\n");
        let cols: Vec<String> = if ir.contains("\ndata __act_due: ") { self.pool.clone() } else { Vec::new() };
        let nf = self.carried.len();
        let mut t = String::from("\n; the real wire (fm3 wire.md, step one; hop sixty): each place in a process of its own, and the harness, which is the world there, carrying the rows that cross as bytes. What follows is the module's queue as the harness reads and writes it, a row as words, and the few words of the world it needs. Nothing here knows what a socket is\n");
        // the row the harness takes next, and the row an arriving one goes into
        t.push_str(&format!("\n; how many rows this place has put on the wire toward the other that the harness has not taken\nfn __wq_out() -> i64\n    p: index = __wplace()\n    to: index = sub 1, p\n    wp: ptr = addr __w\n    t2: index = mul to, 2\n    hi: index = add t2, 24\n    ti: index = add t2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    n: i64 = sub tail, head\n    ret n\n\n; the earliest of them, its row in the queue\nfn __wq_head_out() -> index\n    p: index = __wplace()\n    to: index = sub 1, p\n    wp: ptr = addr __w\n    t2: index = mul to, 2\n    hi: index = add t2, 24\n    head: i64 = load wp, hi, 8\n    m: i64 = rem head, {q}\n    mi: index = conv m\n    qb: index = mul to, {q}\n    row: index = add qb, mi\n    ret row\n\n; ... and the harness has taken it\nfn __wq_row_done()\n    p: index = __wplace()\n    to: index = sub 1, p\n    wp: ptr = addr __w\n    t2: index = mul to, 2\n    hi: index = add t2, 24\n    head: i64 = load wp, hi, 8\n    h2: i64 = add head, 1\n    store h2, wp, hi, 8\n    ret\n\n; the row toward this place an arriving one goes into: the next of its queue, which must have room\nfn __wq_tail_in() -> index\n    p: index = __wplace()\n    wp: ptr = addr __w\n    p2: index = mul p, 2\n    hi: index = add p2, 24\n    ti: index = add p2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    used: i64 = sub tail, head\n    room: u1 = cmp.lt used, {q}\n    check room\n    m: i64 = rem tail, {q}\n    mi: index = conv m\n    qb: index = mul p, {q}\n    row: index = add qb, mi\n    ret row\n", q = q));
        // a field of the context an item carries, as a word, by the place of its bit
        let mut get = format!("\n; a field of the context a row carries, as a word, by the place of its bit among the fields the wire may carry\nfn __wq_field_get(row: index, f: i64) -> i64\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    x: __ctx = load qp\n", q2 = 2 * q);
        let mut set = format!("\n; ... and one set from a word, in the row an arriving item has\nfn __wq_field_set(row: index, f: i64, v: i64)\n    qm: ptr(array(__ctx, {q2})) = addr __wq_ctx\n    qe: ptr(__ctx) = index qm, row\n    qp: ptr = cast qe\n    x: __ctx = load qp\n", q2 = 2 * q);
        for (k, (_, name, ty)) in self.carried.iter().enumerate() {
            let base = self.named.get(ty).map_or(ty.as_str(), |b| b.as_str());
            let from = if base == "u1" { format!("        b{k}: u1 = cmp.ne v, 0\n", k = k) } else { format!("        b{k}: {ty} = conv v\n", k = k, ty = ty) };
            get.push_str(&format!("    is{k}: u1 = cmp.eq f, {k}\n    if is{k}\n        a{k}: {ty} = get x, {name}\n        w{k}: i64 = conv a{k}\n        ret w{k}\n", k = k, ty = ty, name = name));
            set.push_str(&format!("    is{k}: u1 = cmp.eq f, {k}\n    if is{k}\n{from}        y{k}: __ctx = set x, {name}, b{k}\n        store y{k}, qp\n        ret\n", k = k, name = name, from = from));
        }
        get.push_str("    ret 0\n");
        set.push_str("    ret\n");
        t.push_str(&get);
        t.push_str(&set);
        // the k-th field a mask carries, by the order of their bits
        t.push_str(&format!("\n; the place of the bit of the k-th field a mask says a row carries, in the order of their bits\nfn __wq_nth(c: i64, k: i64) -> i64\n    f: i64 = loop(i: i64 = 0, n: i64 = 0) bound {b}\n        done: u1 = cmp.ge i, {nf}\n        if done\n            break -1\n        one: i64 = shl 1, i\n        bit: i64 = and c, one\n        has: u1 = cmp.ne bit, 0\n        hit: u1 = cmp.eq n, k\n        found: u1 = and has, hit\n        if found\n            break i\n        n1: i64 = add n, 1\n        n2: i64 = if has\n            yield n1\n        else\n            yield n\n        i2: i64 = add i, 1\n        continue i2, n2\n    ret f\n\n; ... and how many fields a mask says\nfn __wq_count(c: i64) -> i64\n    cnt: i64 = loop(i: i64 = 0, n: i64 = 0) bound {b}\n        done: u1 = cmp.ge i, {nf}\n        if done\n            break n\n        one: i64 = shl 1, i\n        bit: i64 = and c, one\n        has: u1 = cmp.ne bit, 0\n        n1: i64 = add n, 1\n        n2: i64 = if has\n            yield n1\n        else\n            yield n\n        i2: i64 = add i, 1\n        continue i2, n2\n    ret cnt\n", b = nf + 1, nf = nf));
        // a column of a record in flight, by its place
        let mut colget = format!("\n; a column of the record a row carries, a function partway's, by its place among the pool's columns\nfn __wq_col_get(row: index, k: i64) -> i64\n    fl: index = add row, {flight}\n", flight = flight);
        let mut colset = format!("\n; ... and one set, in the row an arriving item has\nfn __wq_col_set(row: index, k: i64, v: i64)\n    fl: index = add row, {flight}\n", flight = flight);
        for (k, c) in cols.iter().enumerate() {
            colget.push_str(&format!("    is{k}: u1 = cmp.eq k, {k}\n    if is{k}\n        p{k}: ptr = addr {c}\n        v{k}: i64 = load p{k}, fl, 8\n        ret v{k}\n", k = k, c = c));
            colset.push_str(&format!("    is{k}: u1 = cmp.eq k, {k}\n    if is{k}\n        p{k}: ptr = addr {c}\n        store v, p{k}, fl, 8\n        ret\n", k = k, c = c));
        }
        colget.push_str("    ret 0\n");
        colset.push_str("    ret\n");
        t.push_str(&colget);
        t.push_str(&colset);
        let nc = cols.len();
        // (hop sixty-three) the arrays and strings a function partway carries
        let vals = land_vals_text(ir, flight);
        let valued = vals.is_some();
        let vals_size = if valued { "        nv: i64 = __wq_vals_pack(row)\n        n4v: i64 = add n4, nv\n        ret n4v\n" } else { "        ret n4\n" };
        let vals_out = if valued { format!("        nf4: i64 = __wq_count(c)\n        kv: i64 = sub k, {nc}\n        pv: u1 = cmp.ge kv, nf4\n        if pv\n            kx: i64 = sub kv, nf4\n            ky: i64 = add kx, 1\n            kyi: index = conv ky\n            vsp: ptr = addr __wq_vals\n            vv: i64 = load vsp, kyi, 8\n            ret vv\n", nc = nc) } else { String::new() };
        let vals_in = if valued { format!("        cq4: ptr = addr __wq_c\n        c4: i64 = load cq4, row, 8\n        nf4: i64 = __wq_count(c4)\n        kv: i64 = sub k, {nc}\n        pv: u1 = cmp.ge kv, nf4\n        if pv\n            kx: i64 = sub kv, nf4\n            kxi: index = conv kx\n            vsp: ptr = addr __wq_vals\n            store v, vsp, kxi, 8\n            ret\n", nc = nc) } else { String::new() };
        let vals_put = if valued { "    t4: u1 = cmp.eq kind, 4\n    if t4\n        __wq_vals_unpack(row)\n" } else { "" };
        if let Some(v) = &vals {
            t.push_str(v);
        }
        // the row out, as words
        t.push_str(&format!("\n; how many words the earliest row toward the other place carries after its eight (fm3 wire.md, \"what a row is on the wire\"): a block of text its bytes, eight a word; a function partway its record, a word a column of the pool, and the fields of the context it carries; a telling the fields it carries\nfn __wq_row_size() -> i64\n    row: index = __wq_head_out()\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    cp: ptr = addr __wq_c\n    c: i64 = load cp, row, 8\n    is2: u1 = cmp.eq kind, 2\n    if is2\n        bp: ptr = addr __wq_b\n        b: i64 = load bp, row, 8\n        b7: i64 = add b, 7\n        w: i64 = div b7, 8\n        ret w\n    n: i64 = __wq_count(c)\n    is4: u1 = cmp.eq kind, 4\n    if is4\n        n4: i64 = add n, {nc}\n{vals_size}    is5: u1 = cmp.eq kind, 5\n    if is5\n        ret n\n    ret 0\n\n; word j of the earliest row toward the other place: from 0 its eight words, when it arrives by the sender's clock, what it is, its three values, which fields it carries, how much of the other's sending it answers, its sender's own time and the rank of the turn that sent it; then, from 8, the words `__wq_row_size` counts\nfn __wq_row_out(j: i64) -> i64\n    row: index = __wq_head_out()\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    is0: u1 = cmp.eq j, 0\n    if is0\n        p0: ptr = addr __wq_at\n        v0: i64 = load p0, row, 8\n        ret v0\n    is1: u1 = cmp.eq j, 1\n    if is1\n        ret kind\n    ap: ptr = addr __wq_a\n    a: i64 = load ap, row, 8\n    is2: u1 = cmp.eq j, 2\n    if is2\n        ret a\n    bp: ptr = addr __wq_b\n    b: i64 = load bp, row, 8\n    is3: u1 = cmp.eq j, 3\n    if is3\n        ret b\n    cp: ptr = addr __wq_c\n    c: i64 = load cp, row, 8\n    is4: u1 = cmp.eq j, 4\n    if is4\n        ret c\n    is5: u1 = cmp.eq j, 5\n    if is5\n        p5: ptr = addr __wq_e\n        v5: i64 = load p5, row, 8\n        ret v5\n    is6: u1 = cmp.eq j, 6\n    if is6\n        p6: ptr = addr __wq_t\n        v6: i64 = load p6, row, 8\n        ret v6\n    is7: u1 = cmp.eq j, 7\n    if is7\n{rank}    k: i64 = sub j, 8\n    t2: u1 = cmp.eq kind, 2\n    if t2\n        tp: ptr = addr __wq_text\n        o: i64 = mul k, 8\n        w: i64 = loop(i: i64 = 0, acc: i64 = 0) bound 9\n            done: u1 = cmp.ge i, 8\n            if done\n                break acc\n            oi: i64 = add o, i\n            past: u1 = cmp.ge oi, b\n            if past\n                break acc\n            at: i64 = add a, oi\n            ai: index = conv at\n            by: u8 = load tp, ai, 1\n            bw: i64 = conv by\n            sh: i64 = mul i, 8\n            bs: i64 = shl bw, sh\n            acc2: i64 = or acc, bs\n            i2: i64 = add i, 1\n            continue i2, acc2\n        ret w\n    t4: u1 = cmp.eq kind, 4\n    if t4\n        inrec: u1 = cmp.lt k, {nc}\n        if inrec\n            cv: i64 = __wq_col_get(row, k)\n            ret cv\n{vals_out}    off: i64 = if t4\n        yield {nc}\n    else\n        yield 0\n    kf: i64 = sub k, off\n    f: i64 = __wq_nth(c, kf)\n    fv: i64 = __wq_field_get(row, f)\n    ret fv\n", nc = nc, vals_size = vals_size, vals_out = vals_out, rank = if ranked { "        p7: ptr = addr __wq_r\n        v7: i64 = load p7, row, 8\n        ret v7\n" } else { "        ret 0\n" }));
        // the row in, as words
        t.push_str(&format!("\n; word j of a row arriving at this place from the other, as `__wq_row_out` gave it there, into the next row of its queue toward this place: its words but the first, when it arrives, which is this place's to say; then the words after them, a block of text's bytes into this place's own text in flight, a record's columns into the row's record, a field into the row's context\nfn __wq_row_in(j: i64, v: i64)\n    row: index = __wq_tail_in()\n    is1: u1 = cmp.eq j, 1\n    if is1\n        p1: ptr = addr __wq_kind\n        store v, p1, row, 8\n        ret\n    is2: u1 = cmp.eq j, 2\n    if is2\n        p2: ptr = addr __wq_a\n        store v, p2, row, 8\n        ret\n    is3: u1 = cmp.eq j, 3\n    if is3\n        p3: ptr = addr __wq_b\n        store v, p3, row, 8\n        ret\n    is4: u1 = cmp.eq j, 4\n    if is4\n        p4: ptr = addr __wq_c\n        store v, p4, row, 8\n        ret\n    is5: u1 = cmp.eq j, 5\n    if is5\n        p5: ptr = addr __wq_e\n        store v, p5, row, 8\n        ret\n    is6: u1 = cmp.eq j, 6\n    if is6\n        p6: ptr = addr __wq_t\n        store v, p6, row, 8\n        ret\n    is7: u1 = cmp.eq j, 7\n    if is7\n{rank}        ret\n    low: u1 = cmp.lt j, 8\n    if low\n        ret\n    k: i64 = sub j, 8\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    t2: u1 = cmp.eq kind, 2\n    if t2\n        wp: ptr = addr __w\n        o: i64 = load wp, 40\n        o8: i64 = mul k, 8\n        base: i64 = add o, o8\n        e: i64 = add base, 8\n        fits: u1 = cmp.le e, 65536\n        check fits\n        tp: ptr = addr __wq_text\n        loop(i: i64 = 0) bound 9\n            done: u1 = cmp.ge i, 8\n            if done\n                break\n            sh: i64 = mul i, 8\n            bv: i64 = shr v, sh\n            bm: i64 = and bv, 255\n            by: u8 = conv bm\n            at: i64 = add base, i\n            ai: index = conv at\n            store by, tp, ai, 1\n            i2: i64 = add i, 1\n            continue i2\n        ret\n    cp: ptr = addr __wq_c\n    c: i64 = load cp, row, 8\n    t4: u1 = cmp.eq kind, 4\n    if t4\n        inrec: u1 = cmp.lt k, {nc}\n        if inrec\n            __wq_col_set(row, k, v)\n            ret\n{vals_in}    off: i64 = if t4\n        yield {nc}\n    else\n        yield 0\n    kf: i64 = sub k, off\n    f: i64 = __wq_nth(c, kf)\n    __wq_field_set(row, f, v)\n    ret\n\n; ... and it is put on: when it arrives by this place's clock, which the harness says, and for a block of text its place in this place's own text in flight\nfn __wq_row_put(at: i64)\n    row: index = __wq_tail_in()\n    p0: ptr = addr __wq_at\n    store at, p0, row, 8\n    kp: ptr = addr __wq_kind\n    kind: i64 = load kp, row, 8\n    t2: u1 = cmp.eq kind, 2\n    if t2\n        wp: ptr = addr __w\n        o: i64 = load wp, 40\n        ap: ptr = addr __wq_a\n        store o, ap, row, 8\n        bp: ptr = addr __wq_b\n        b: i64 = load bp, row, 8\n        o2: i64 = add o, b\n        store o2, wp, 40\n{vals_put}    p: index = __wplace()\n    wq: ptr = addr __w\n    p2: index = mul p, 2\n    ti: index = add p2, 25\n    tail: i64 = load wq, ti, 8\n    tl2: i64 = add tail, 1\n    store tl2, wq, ti, 8\n    ret\n", nc = nc, vals_in = vals_in, vals_put = vals_put, rank = if ranked { "        p7: ptr = addr __wq_r\n        store v, p7, row, 8\n" } else { "" }));
        // the world's words the harness needs
        let kept = if ahead { "    ap: ptr = addr __wa\n    kn: i64 = load ap, 104\n    kh: i64 = load ap, 96\n    khi: index = conv kh\n    ktp: ptr = addr __wk_t\n    kt: i64 = load ktp, khi, 8\n    khas: u1 = cmp.gt kn, 0\n    klate: u1 = cmp.gt kt, now\n    kok: u1 = and khas, klate\n    k2: i64 = if kok\n        yield kt\n    else\n        yield nv\n" } else { "    k2: i64 = add nv, 0\n" };
        let still = if ahead { "    sa: ptr = addr __wa\n    pk: i64 = load sa, 16\n    unparked: u1 = cmp.eq pk, 0\n    kn: i64 = load sa, 104\n    nokept: u1 = cmp.eq kn, 0\n    g4: u1 = and g3, unparked\n    g5: u1 = and g4, nokept\n" } else { "    g5: u1 = and g3, 1\n" };
        t.push_str(&format!("\n; the world's clock at this place, which the harness says: its own, the real one, from when its case began\nfn __wreal_now(t: i64)\n    wp: ptr = addr __w\n    store t, wp\n    ret\n\n; the world begins at this place (`__world`'s first lines, at one place): its note of what was last sent or was sent of each field is its own context as it was made, before anything is set at it, as each field's first value is at both places; and at the server the lines that walk a stored stream are given their turn\nfn __wreal_begin(pi: i32)\n    __zero_context(pi)\n    pl: index = conv pi\n    fc: ptr = context()\n    fx: __ctx = load fc\n    sm: ptr(array(__ctx, 2)) = addr __wshadow\n    e0: ptr(__ctx) = index sm, pl\n    q0: ptr = cast e0\n    store fx, q0\n{run}    ret\n\n; the next time something can happen at this place by its clock, the never-time where nothing can but what arrives: a row held until its time, and at the client its own earliest thing, a piece it keeps until its time, and the time the server said its earliest thing is, which the client's word of where it stands may free when the clock reaches it\nfn __wreal_due(pi: i32) -> i64\n    __zero_context(pi)\n    p: index = conv pi\n    wp: ptr = addr __w\n    now: i64 = load wp\n    nv: i64 = const {never}\n    p2: index = mul p, 2\n    hi: index = add p2, 24\n    ti: index = add p2, 25\n    head: i64 = load wp, hi, 8\n    tail: i64 = load wp, ti, 8\n    has: u1 = cmp.lt head, tail\n    a0: i64 = if has\n        m: i64 = rem head, {q}\n        mi: index = conv m\n        qb: index = mul p, {q}\n        row: index = add qb, mi\n        atp: ptr = addr __wq_at\n        v: i64 = load atp, row, 8\n        yield v\n    else\n        yield nv\n    al: u1 = cmp.gt a0, now\n    a1: i64 = if al\n        yield a0\n    else\n        yield nv\n    srv: u1 = cmp.ne p, 0\n    if srv\n        ret a1\n    t: i64, r: i64 = __list_next()\n    tl: u1 = cmp.gt t, now\n    t2: i64 = if tl\n        yield t\n    else\n        yield nv\n    b8: index = mul p, 8\n    j3: index = add b8, 11\n    pt: i64 = load wp, j3, 8\n    pl: u1 = cmp.gt pt, now\n    pt2: i64 = if pl\n        yield pt\n    else\n        yield nv\n{kept}    n1: i64 = min(a1, t2)\n    n2: i64 = min(n1, pt2)\n    n3: i64 = min(n2, k2)\n    ret n3\n\n; has this place nothing left to do but what may yet arrive: nothing on its list, nothing in either way of its queue, an answer to everything it sent, no field changed that crosses, and where the server works ahead nothing held at it and nothing kept at the client. The harness ends the case when both places have nothing left and each has had all the other sent\nfn __wreal_idle(pi: i32) -> u1\n    __zero_context(pi)\n    p: index = conv pi\n    t: i64, r: i64 = __list_next()\n    nv: i64 = const {never}\n    empty: u1 = cmp.ge t, nv\n    wp: ptr = addr __w\n    h0: i64 = load wp, 192\n    t0: i64 = load wp, 200\n    h1: i64 = load wp, 208\n    t1: i64 = load wp, 216\n    e0: u1 = cmp.ge h0, t0\n    e1: u1 = cmp.ge h1, t1\n    b: index = mul p, 8\n    i0: index = add b, 8\n    sent: i64 = load wp, i0, 8\n    i1: index = add b, 9\n    acked: i64 = load wp, i1, 8\n    quiet: u1 = cmp.eq sent, acked\n    dirty: u1 = __wfields_dirty()\n    clean: u1 = xor dirty, 1\n    g0: u1 = and empty, e0\n    g1: u1 = and g0, e1\n    g2: u1 = and g1, quiet\n    g3: u1 = and g2, clean\n{still}    ret g5\n", q = q, never = NEVER, run = if runs { "    pw: i64 = conv pi\n    atsrv: u1 = cmp.ne pw, 0\n    if atsrv\n        __run()\n" } else { "" }, kept = kept, still = still));
        if ahead {
            t.push_str("\n; how many turns the server took ahead and threw away, which it says at the end\nfn __wreal_thrown() -> i64\n    ap: ptr = addr __wa\n    th: i64 = load ap, 56\n    ret th\n");
        }
        // each home kept as a word, at this place alone
        if !self.home_words.is_empty() {
            let mut f = format!("\n; at the world's end, a line of the record, 27, for each group's or everyone's stream kept as a word, at this place alone (`__whomes`, each place in a process of its own)\nfn __wreal_homes(pi: i32)\n    wp: ptr = addr __w\n    now: i64 = load wp\n    pw: i64 = conv pi\n    pl: index = conv pi\n    cm: ptr(array(__ctx, 2)) = addr __ctx_mem\n    lp: ptr = addr __wlog\n    e: ptr(__ctx) = index cm, pl\n    qx: ptr = cast e\n    x: __ctx = load qx\n");
            for (n, (k, name, ty)) in self.home_words.iter().enumerate() {
                f.push_str(&format!("    v{n}: {ty} = get x, {name}\n    w{n}: i64 = conv v{n}\n    ln{n}: i64 = load wp, 32\n    r{n}: u1 = cmp.lt ln{n}, {log}\n    if r{n}\n        li{n}: index = conv ln{n}\n        l{n}: index = mul li{n}, 6\n        store pw, lp, l{n}, 8\n        a{n}: index = add l{n}, 1\n        store 27: i64, lp, a{n}, 8\n        b{n}: index = add l{n}, 2\n        store now, lp, b{n}, 8\n        c{n}: index = add l{n}, 3\n        store {k}: i64, lp, c{n}, 8\n        d{n}: index = add l{n}, 4\n        store 0: i64, lp, d{n}, 8\n        f{n}: index = add l{n}, 5\n        store w{n}, lp, f{n}, 8\n        m{n}: i64 = add ln{n}, 1\n        store m{n}, wp, 32\n", n = n, ty = ty, name = name, k = k, log = WIRE_LOG));
            }
            f.push_str("    ret\n");
            t.push_str(&f);
        }
        // each case's twin in two: its call begun, and what it gave
        let lines: Vec<&str> = ir.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let head = lines[i];
            if !head.starts_with("fn ") {
                i += 1;
                continue;
            }
            let end = (i + 1..lines.len()).find(|&j| !lines[j].starts_with("    ")).unwrap_or(lines.len());
            if let Some(w) = (i + 1..end).find(|&j| lines[j] == "    __world()") {
                let name = head["fn ".len()..].split('(').next().unwrap_or("");
                let params = &head[head.find('(').unwrap_or(0)..head.find(')').map_or(head.len(), |c| c + 1)];
                let rets = head.find(") -> ").map(|r| &head[r + 1..]).unwrap_or("");
                t.push_str(&format!("\n; `{name}` in two (hop sixty): its call begun at the client, and what it gave read back once the world has ended\nfn {name}__begin{params}\n", name = name, params = params));
                for l in &lines[i + 1..w] {
                    t.push_str(l);
                    t.push('\n');
                }
                t.push_str(&format!("    ret\nfn {name}__end(){rets}\n", name = name, rets = rets));
                for l in &lines[w + 1..end] {
                    t.push_str(l);
                    t.push('\n');
                }
            }
            i = end;
        }
        Ok(t)
    }
}
