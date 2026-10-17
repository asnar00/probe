//! The zero runner: `probe zero <store> emit`, `probe zero <store> run
//! <case>`, and `probe zero test [dir] [path]`. A store is lowered once
//! to IR text, the text goes through probe's parser and everything after
//! it — the IR is the oracle for the front end — and the cases from the
//! features' `## testing` sections run on the chosen path through the
//! suite's own machinery (`suite::run_calls`).

use super::{lower, store};
use crate::host::site_said;
use crate::suite::{self, Backend, Report};
use crate::{opt, ssa};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// lower a store and print its IR: one text on every path, the widths
/// being the policy's (log 52)
pub fn emit(dir: &Path) -> Result<String, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    Ok(l.ir)
}

/// the diagnostic build of a store (fm3 log 199): the text a failed
/// case is run again from, and after it the table of sites, a row a
/// comment, `; site <n>: <file>:<line>[: <what>]`
pub fn emit_sites(dir: &Path) -> Result<String, String> {
    emit_build(dir, false)
}

/// ... and the trace build (fm3 tracer.md), the diagnostic build that
/// also writes down what it does, with its table: the rows a statement
/// stores and the rows an event names, `= int n`, `< int x$`, `/ x$`
pub fn emit_trace(dir: &Path) -> Result<String, String> {
    emit_build(dir, true)
}

/// ... and the placed build (fm3 cut.md): the text that runs at two
/// places, the wire and the table of the cut in it
///
/// With a cut named, a function `__cut` after the text fills the table
/// as the runner would before a case: `probe count <text> run
/// --after=__zero_reset,__cut` then prices the case at that cut
///
/// ... and with moves (fm3 cut.md, "the cut, moved"), `then` a cut and
/// the time it is gone to, the text is that of a build whose cut may
/// be moved, and `__cut` hands the moves over as well
pub fn emit_placed(dir: &Path, server: Option<&str>, then: &[(String, i64)]) -> Result<String, String> {
    emit_wired(dir, server, then, None)
}

/// ... and with a wire that takes time (fm3 cut.md, "a wire that takes
/// time"), the text of that build, `__cut` setting the delay too
pub fn emit_wired(dir: &Path, server: Option<&str>, then: &[(String, i64)], wire: Option<&Wire>) -> Result<String, String> {
    let mut s = match wire {
        Some(w) => {
            let mut s = wired(dir)?;
            s.changes = !w.then.is_empty() || w.paced();
            s.paced = w.paced();
            s.choose = w.choose.is_some();
            s
        }
        None => store::read(dir).map_err(|e| e.to_string())?,
    };
    s.places = true;
    s.moved = !then.is_empty() || s.choose;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let mut ir = l.ir;
    if let (true, Some(p)) = (server.is_some() || !then.is_empty() || wire.is_some(), &l.placed) {
        let mut cut = match wire.filter(|w| w.choose.is_some()) {
            Some(w) => super::places::Cut { before: chosen_cut(p, server, then, w)?, label: format!("{}, the runner choosing within {}", p.label(&cuts_named(p, server, &[])?.0), spell_late(w.choose.unwrap_or(0))) },
            None => cut_of(p, server, then)?,
        };
        if let Some(w) = wire {
            cut.before.push(("__wire_set".to_string(), vec![w.d]));
            cut.before.extend(w.calls());
            cut.label.push_str(&format!(", a wire of {}", w.said()));
        }
        ir.push_str(&format!("\n; the cut, {}, as the runner fills it before a case\nfn __cut()\n", cut.label));
        for (f, args) in &cut.before {
            ir.push_str(&format!("    {}({})\n", f, args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")));
        }
        ir.push_str("    ret\n");
    }
    Ok(ir)
}

/// The cut a command names, `--server <units>`, and its moves, `--then
/// <units> at <time>`, as the calls that say them to the program; or
/// why it cannot be run. A move goes forward in time, and a store has
/// room for `places::MOVES` of them
fn cut_of(p: &super::places::Placed, server: Option<&str>, then: &[(String, i64)]) -> Result<super::places::Cut, String> {
    let (first, later) = cuts_named(p, server, then)?;
    if !later.is_empty() && !p.moved {
        return Err("the store was not built to have its cut moved".into());
    }
    if let Some(why) = p.refused(&first) {
        return Err(format!("this cut cannot be run yet: {}", why));
    }
    let mut was = &first;
    for (next, _) in &later {
        if let Some(why) = p.move_refused(was, next) {
            return Err(format!("this move cannot be made yet: {}", why));
        }
        was = next;
    }
    Ok(p.moves(&first, &later))
}

fn cuts_named(p: &super::places::Placed, server: Option<&str>, then: &[(String, i64)]) -> Result<(BTreeSet<usize>, Vec<(BTreeSet<usize>, i64)>), String> {
    let first = match server {
        Some(names) => p.named(names)?,
        None => BTreeSet::new(),
    };
    if then.len() > super::places::MOVES {
        return Err(format!("a cut may be moved {} times in one run, and {} moves are said", super::places::MOVES, then.len()));
    }
    let mut later = Vec::new();
    for (k, (names, t)) in then.iter().enumerate() {
        if k > 0 && then[k - 1].1 >= *t {
            return Err(format!("the moves of a cut go forward in time: {} is not after {}", store::spell_time(*t), store::spell_time(then[k - 1].1)));
        }
        later.push((p.named(names)?, *t));
    }
    Ok((first, later))
}

/// A time as a command line says it, `4.5 s`, `4.5s` or `250 ms`, in
/// the clock's steps
pub fn time_said(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let (n, per) = if let Some(n) = t.strip_suffix("ms") { (n, 1000.0) } else if let Some(n) = t.strip_suffix('s') { (n, 1_000_000.0) } else { return Err(format!("'{}' is not a time: a number and `s` or `ms`, `4.5 s`", text)) };
    match n.trim().parse::<f64>() {
        Ok(x) if x >= 0.0 && x.is_finite() => Ok((x * per).round() as i64),
        _ => Err(format!("'{}' is not a time: a number and `s` or `ms`, `4.5 s`", text)),
    }
}

/// The placement of a store at a cut (fm3 cut.md; distribution.md,
/// "what the compiler does", 7): every unit and where it runs, every
/// crossing, what kind it is and what is carried. `server` names the
/// units placed on the server, `count down,countdown.zero:2` or `all`; none,
/// and everything is at the client
///
/// ... and with moves, the table of each cut, and between two what is
/// handed across at the move (fm3 cut.md, "the cut, moved")
pub fn placement(dir: &Path, server: Option<&str>, then: &[(String, i64)]) -> Result<String, String> {
    let mut s = store::read(dir).map_err(|e| e.to_string())?;
    s.clock = store::Clock::Virtual;
    s.places = true;
    s.moved = !then.is_empty();
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let p = l.placed.as_ref().ok_or("the store was not placed")?;
    let (server, later) = cuts_named(p, server, then)?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // the functions a case of the store calls: each is a call made at
    // the client
    let policy = store_policy(&s, &ssa::Policy::new(ssa::Type::I64)?);
    let cases: Vec<String> = calls_of(&s, &l, &policy)?.iter().map(|c| c.call.func.trim_start_matches("__whole_").trim_start_matches("__said_").to_string()).collect();
    if later.is_empty() {
        return Ok(p.table(&name, &server, &cases));
    }
    Ok(p.table_moved(&name, &server, &later, &cases))
}

fn emit_build(dir: &Path, trace: bool) -> Result<String, String> {
    let mut s = store::read(dir).map_err(|e| e.to_string())?;
    s.sites = true;
    s.trace = trace;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let mut ir = l.ir;
    ir.push_str("\n; the sites (fm3 log 199): what `__site_at` stores, and where it is\n");
    for (i, site) in l.sites.iter().enumerate() {
        ir.push_str(&format!("; site {}: {}:{}{}\n", i + 1, site.file, site.line, if site.what.is_empty() { String::new() } else { format!(": {}", site.what) }));
    }
    Ok(ir)
}

/// the policy a store is built under (log 47, 52, fm3 log 120): the
/// path's, with `int`, `float` and `index` at the widths the store's
/// `product.md` sets, if it does
pub fn store_policy(s: &store::Store, policy: &ssa::Policy) -> ssa::Policy {
    let policy = &match s.index_width.and_then(|w| policy.with_index(w)) {
        Some(p) => p,
        None => *policy,
    };
    let policy = match s.int_width {
        Some(32) => ssa::Policy { int: ssa::Type::I32, ..*policy },
        Some(64) => ssa::Policy { int: ssa::Type::I64, ..*policy },
        _ => *policy,
    };
    match s.float_width {
        Some(32) => policy.with_float(8, 23),
        Some(64) => policy.with_float(11, 52),
        _ => policy,
    }
}

/// the width of `int` under a policy, the width a bare integer literal
/// takes between two concrete widths (log 47)
pub fn int_bits(policy: &ssa::Policy) -> u32 {
    match policy.int {
        ssa::Type::I32 => 32,
        _ => 64,
    }
}

/// The IR's refusal of a literal its type cannot hold, in zero's words
/// (fm3 log 173). The front end holds a literal to its type where it
/// is given one and names the line; this is for one that reached the
/// IR unchecked, so that no programmer is shown `iconst`. It names the
/// function, which is all the IR knows
fn unheld(e: &str) -> Option<String> {
    let (head, rest) = e.split_once(": iconst ")?;
    let (n, ty) = rest.split_once(" does not fit in type ")?;
    let ty = ty.trim();
    let bits: u32 = ty.get(1..)?.parse().ok()?;
    let (name, range) = match &ty[..1] {
        "u" => (format!("uint{}", bits), format!("0 to {}", (1u128 << bits) - 1)),
        "i" => (format!("int{}", bits), format!("-{} to {}", 1u128 << (bits - 1), (1u128 << (bits - 1)) - 1)),
        _ => return None,
    };
    // `line 0: f: entry`: the function is the word before the block's
    let func = head.split(": ").nth(1).unwrap_or("?");
    Some(format!("in '{}': {} does not fit {} {}, which holds {}. (The compiler should have named the line: a literal reached the IR unchecked.)", func, n, if name.starts_with('i') { "an" } else { "a" }, name, range))
}

/// the lowered store as a module under a policy: parsed, resolved,
/// verified, optimized, verified again
pub fn build(ir: &str, policy: &ssa::Policy, level: usize) -> Result<ssa::Module, String> {
    let mut module = ssa::parse_with(&ssa::with_prelude(ir), policy).map_err(|e| unheld(&e.to_string()).unwrap_or_else(|| format!("the lowered IR did not parse: {}", e)))?;
    ssa::resolve_types(&mut module, policy);
    ssa::verify(&module).map_err(|errs| format!("the lowered IR did not verify: {}", errs.join("; ")))?;
    opt::optimize(&mut module, level);
    ssa::verify(&module).map_err(|errs| format!("after optimization: {}", errs.join("; ")))?;
    Ok(module)
}

/// a case as the runner places it (log 44): the line as written, the
/// call, the feature it came from and that feature's place in the
/// composition order, and where it was written
pub(super) struct Planned {
    pub(super) text: String,
    pub(super) call: lower::Call,
    feature: String,
    rank: usize,
    file: String,
    line: usize,
}

/// the calls a store's cases make, in the order the features compose;
/// a case's literal arguments are typed at the width the store is
/// built at (log 52)
pub(super) fn calls_of(s: &store::Store, l: &lower::Lowered, policy: &ssa::Policy) -> Result<Vec<Planned>, String> {
    let mut out = Vec::new();
    for (rank, f) in s.features.iter().enumerate() {
        for c in &f.cases {
            let call = lower::resolve_case(l, c, &f.md_file, int_bits(policy)).map_err(|e| e.to_string())?;
            out.push(Planned { text: c.text.clone(), call, feature: f.name.clone(), rank, file: f.md_file.clone(), line: c.line });
        }
    }
    Ok(out)
}

/// A published feature whose code has changed must still pass its own
/// cases (fm3 question 89, log 176): a refactoring is allowed and a
/// change of meaning is not, and the feature's cases are what tell
/// them apart. Its own are the lines of its own `## testing`, each in
/// the context its line names, every other feature on. They are run
/// on the path asked for before anything else of the store is, and
/// the first that fails refuses the store, naming it. A case the path
/// cannot run is passed over, as the suite passes over it
fn held(s: &store::Store, cases: &[&Planned], module: &ssa::Module, l: &lower::Lowered, backend: Backend, name: &str, level: usize) -> Result<(), String> {
    for f in s.features.iter().filter(|f| f.changed.is_some()) {
        let unreached = out_of_reach(module, &l.funcs, kind_of(backend));
        let own: Vec<&Planned> = cases.iter().copied().filter(|p| p.feature == f.name && !unreached.contains_key(&p.call.func)).collect();
        let calls: Vec<suite::Call> = own
            .iter()
            .map(|p| suite::Call {
                func: p.call.func.clone(),
                args: p.call.args.clone(),
                nrets: p.call.nrets,
                checks: checks(&p.call.expect),
                text: true,
                before: setters(&p.call.context, &p.call.input, &p.call.input_at),
                live: false,
                times: matches!(p.call.expect, store::Expect::Timed(_)),
            })
            .collect();
        if calls.is_empty() {
            continue;
        }
        let got = suite::run_calls(module, &l.ir, backend, &calls, name, level)?;
        for (p, got) in own.iter().zip(got) {
            if got.as_ref().err().is_some_and(|e| e.starts_with("skip: ")) {
                continue;
            }
            let (ok, note) = judge(&p.call.expect, got);
            if !ok {
                return Err(format!(
                    "feature {} was published on {} and its code {}: a published feature may be refactored and must still pass its own cases (fm3 question 89), and `{}` does not {}. A change of what a feature means is a sub-feature that redefines what it needs and calls `existing` (structure.md, the lifecycle)",
                    f.name,
                    f.published.as_deref().unwrap_or(""),
                    f.changed.as_deref().unwrap_or(""),
                    p.text,
                    note
                ));
            }
        }
    }
    Ok(())
}

/// The contexts a store's cases run in (section 14, log 44, 51): every
/// feature on, then each feature of the store switched off alone, as a
/// label and the set of features switched. A feature off takes its
/// descendants with it through the gate, not through their fields
/// (`Store::closure` says which are effectively off). The compiler's
/// own `platform` feature is never switched
fn contexts(s: &store::Store) -> Vec<(String, BTreeSet<String>)> {
    let mut out = vec![(String::new(), BTreeSet::new())];
    for f in &s.features {
        if f.name != "platform" && s.marks.get(&f.name) != Some(&store::Mark::StaticOn) {
            out.push((format!("with {} off", f.name), [f.name.clone()].into_iter().collect()));
        }
    }
    out
}

/// The features switched off when a case runs in the runner's context
/// `x`: `x` with the case's own line applied on top, a sequence of
/// switches in order, `off` setting a flag and `on` restoring it (log
/// 51). None when the case does not stand there: its own feature is
/// effectively off in `x`
fn effective(s: &store::Store, x: &BTreeSet<String>, p: &Planned) -> Option<BTreeSet<String>> {
    if s.closure(x).contains(&p.feature) {
        return None;
    }
    let mut off = x.clone();
    for (name, on) in &p.call.context {
        if *on {
            off.remove(name);
        } else {
            off.insert(name.clone());
        }
    }
    Some(off)
}

/// the calls that build a context before a case: each feature
/// switched off, its own `enabled` field set to 0 and no other
/// (everything is on after the reset, log 43); then the case's input,
/// a byte per call into `in$` (log 62), before the program starts
///
/// ... and, from fm3 log 138, the switches are made as the line says
/// them, in order: the runner's own context first, then each `off` and
/// each `on` of the case's line, a setter's call each. A feature's
/// effective state is worked out where a switch is written, so a line
/// that switches a parent off and on again runs the code that must
/// leave its children as they were
pub fn setters(switches: &[(String, bool)], input: &[u8], input_at: &[(u8, i64)]) -> Vec<(String, Vec<i64>)> {
    let mut calls: Vec<(String, Vec<i64>)> = switches.iter().map(|(f, on)| (format!("__set___enabled_{}", f), vec![*on as i64])).collect();
    calls.extend(input.iter().map(|&c| ("__in_ch".to_string(), vec![c as i64])));
    // ... and what arrives at a time is handed over with its time
    // (fm3 log 220), to arrive when the program reaches it
    calls.extend(input_at.iter().map(|&(c, t)| ("__in_at".to_string(), vec![c as i64, t])));
    calls
}

/// a case overridden in one context: the case with the runner's label,
/// the context, and why it does not run there
struct Over {
    text: String,
    context: String,
    why: String,
}

/// one run of a case: in which effective context, under which of the
/// runner's labels
struct Run {
    case: usize,
    off: BTreeSet<String>,
    /// the switches that make it, in order: the runner's context, then
    /// the case's own line
    switches: Vec<(String, bool)>,
    label: String,
}

/// the call of a case as written, `run()`, `counted (10)`, before any
/// `with` clause
fn call_text(p: &Planned) -> String {
    let head = p.text.split('→').next().unwrap_or("").trim();
    match head.find(") with ").map(|i| i + 1).or_else(|| head.rfind(')').map(|i| i + 1)) {
        Some(i) => head[..i].to_string(),
        None => head.to_string(),
    }
}

/// Every run a store's cases make (section 14, log 50). In each of the
/// runner's contexts a feature's cases for a method are its definition
/// of that method's test function, and the definitions compose as the
/// front end composes the method: the newest feature that is on and has
/// cases for it is outermost, and falls through to the next older one
/// only when its testing section says `>existing`; a feature that is
/// off is not in the chain, so the older cases stand again. A case
/// whose feature is not in the chain is overridden there. A standing
/// case runs once per distinct effective context — the runner's with
/// the case's own line applied — and where two lines of one feature
/// make the same call in one effective context, the line naming more
/// switches stands. Gives the runs and a report line per override
fn plan(s: &store::Store, cases: &[Planned]) -> Result<(Vec<Run>, Vec<Over>), String> {
    // `>existing` falls through to an older definition, which must exist
    for (rank, f) in s.features.iter().enumerate().filter(|(_, f)| f.existing_cases) {
        let own: Vec<&Planned> = cases.iter().filter(|p| p.feature == f.name).collect();
        if !own.iter().any(|p| cases.iter().any(|q| q.rank < rank && q.call.func == p.call.func)) {
            let calls: Vec<String> = own.iter().map(|p| call_text(p)).collect();
            return Err(format!("{}: `>existing` in feature {}'s testing, but no older feature has cases for {}: nothing to fall through to", f.md_file, f.name, if calls.is_empty() { "any function".to_string() } else { calls.join(", ") }));
        }
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut over = Vec::new();
    let mut seen: HashSet<(usize, BTreeSet<String>)> = HashSet::new();
    for (label, x) in contexts(s) {
        // the chain of test definitions per method in this context,
        // newest first: the features whose cases stand
        let gone = s.closure(&x);
        let methods: BTreeSet<&str> = cases.iter().map(|p| p.call.func.as_str()).collect();
        let mut chains: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for m in methods {
            let mut chain = Vec::new();
            for f in s.features.iter().rev() {
                if gone.contains(&f.name) || !cases.iter().any(|p| p.feature == f.name && p.call.func == m) {
                    continue;
                }
                chain.push(f.name.as_str());
                if !f.existing_cases {
                    break;
                }
            }
            chains.insert(m, chain);
        }
        for (i, p) in cases.iter().enumerate() {
            let Some(off) = effective(s, &x, p) else { continue };
            let chain = &chains[p.call.func.as_str()];
            if !chain.iter().any(|f| *f == p.feature) {
                over.push(Over { text: labelled(&p.text, &label), context: describe(&x), why: format!("replaced by {}'s cases for {}", chain[0], call_text(p)) });
                continue;
            }
            if seen.insert((i, off.clone())) {
                let switches = x.iter().map(|f| (f.clone(), false)).chain(p.call.context.iter().cloned()).collect();
                runs.push(Run { case: i, off, switches, label: label.clone() });
            }
        }
    }
    // within a feature, one promise per call per effective context:
    // the line naming more switches stands
    let mut standing = vec![true; runs.len()];
    for i in 0..runs.len() {
        if !standing[i] {
            continue;
        }
        // a case's input is part of its call (log 62): two lines
        // with different inputs are two promises
        let same: Vec<usize> = (0..runs.len()).filter(|&j| standing[j] && runs[j].off == runs[i].off && cases[runs[j].case].feature == cases[runs[i].case].feature && cases[runs[j].case].call.func == cases[runs[i].case].call.func && cases[runs[j].case].call.args == cases[runs[i].case].call.args && cases[runs[j].case].call.input == cases[runs[i].case].call.input && cases[runs[j].case].call.input_at == cases[runs[i].case].call.input_at).collect();
        if same.len() < 2 {
            continue;
        }
        let weight = |j: usize| cases[runs[j].case].call.context.len();
        let best = *same.iter().max_by_key(|&&j| weight(j)).unwrap();
        if let Some(&tie) = same.iter().find(|&&j| j != best && weight(j) == weight(best)) {
            let (a, b) = (&cases[runs[best].case], &cases[runs[tie].case]);
            return Err(format!("{}:{} and line {} both claim `{}` in one context ({}): one case per call per context", a.file, a.line, b.line, call_text(a), describe(&runs[best].off)));
        }
        for &j in &same {
            if j != best {
                standing[j] = false;
                let (o, w) = (&cases[runs[j].case], &cases[runs[best].case]);
                over.push(Over { text: labelled(&o.text, &runs[j].label), context: describe(&runs[j].off), why: format!("the line `{}` stands there", w.text) });
            }
        }
    }
    let runs = runs.into_iter().zip(standing).filter_map(|(r, keep)| keep.then_some(r)).collect();
    Ok((runs, over))
}

fn describe(off: &BTreeSet<String>) -> String {
    if off.is_empty() {
        "every feature on".to_string()
    } else {
        format!("{} off", off.iter().cloned().collect::<Vec<_>>().join(", "))
    }
}

/// a case's line with the runner's context after it
fn labelled(text: &str, label: &str) -> String {
    if label.is_empty() {
        text.to_string()
    } else {
        format!("{} [{}]", text, label)
    }
}

/// the kind of place a backend is, as a `platform` body names it (log 31)
fn kind_of(backend: Backend) -> &'static str {
    match backend {
        Backend::Native | Backend::ArmQemu => "arm64",
        Backend::Riscv => "riscv64",
        Backend::Wasm => "wasm32",
        Backend::Air => "air",
    }
}

/// Which of the module's functions are out of reach on a kind of place
/// (section 15): a platform function with no body for it and no `ir`
/// body, and every function that calls one, by name — with the platform
/// function it reaches, for the skip line
fn out_of_reach(module: &ssa::Module, funcs: &[lower::FnInfo], kind: &str) -> std::collections::HashMap<String, String> {
    let mut out: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for f in funcs {
        if let Some(kinds) = &f.platform {
            if !kinds.iter().any(|k| k == "ir" || k == kind) {
                out.insert(f.ir.clone(), f.ir.clone());
            }
        }
    }
    if out.is_empty() {
        return out;
    }
    // callers of what is out of reach, until nothing new is found
    loop {
        let mut more = Vec::new();
        for f in &module.funcs {
            if out.contains_key(&f.name) {
                continue;
            }
            for inst in f.blocks.iter().flat_map(|b| &b.insts) {
                if let ssa::Inst::Call { callee, .. } = inst {
                    if let Some(why) = out.get(callee) {
                        more.push((f.name.clone(), why.clone()));
                        break;
                    }
                }
            }
        }
        if more.is_empty() {
            break;
        }
        out.extend(more);
    }
    out
}

fn skip_note(funcs: &[lower::FnInfo], reaches: &str, kind: &str) -> String {
    let name = funcs.iter().find(|f| f.ir == reaches).map(|f| f.key.replace('_', " ")).unwrap_or_else(|| reaches.to_string());
    format!("'{}' has no platform body for {}", name, kind)
}

/// the case as its line begins, the newest feature's where several
/// begin so, since that is the one that stands with every feature on;
/// or the first case of a function named alone
fn case_at(calls: &[Planned], which: &str) -> Option<usize> {
    let head = |p: &Planned| p.text.split('→').next().unwrap_or("").trim() == which.trim();
    calls.iter().rposition(head).or_else(|| calls.iter().position(|p| p.call.func == which.trim()))
}

pub(super) fn case_named<'a>(calls: &'a [Planned], which: &str) -> Option<&'a Planned> {
    case_at(calls, which).map(|i| &calls[i])
}

/// run one case of a store on the native JIT (log 77): the case line
/// first, the program's output as it lands, then what the case gave
/// after an arrow — the numbers, or the output as the case's quoted
/// text; on the real clock unless `fast`
pub fn run(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, fast: bool) -> Result<String, String> {
    run_cut(dir, which, policy, level, fast, None, &[])
}

/// ... and at a cut (fm3 cut.md): `server` names the units that run at
/// the second place, and the case is run from the store's placed build
///
/// ... and moved while it runs (fm3 cut.md, "the cut, moved"): `then`
/// is each cut gone to and the time, and the build is one whose cut
/// may be moved
pub fn run_cut(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, fast: bool, server: Option<&str>, then: &[(String, i64)]) -> Result<String, String> {
    let mut s = store::read(dir).map_err(|e| e.to_string())?;
    s.clock = if fast { store::Clock::Virtual } else { store::Clock::Real };
    s.places = server.is_some() || !then.is_empty();
    s.moved = !then.is_empty();
    let policy = &store_policy(&s, policy);
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    // the cut's table is filled before anything else of the case, and
    // its moves handed over
    let cut = match &l.placed {
        Some(p) => cut_of(p, server, then)?.before,
        None => Vec::new(),
    };
    let calls = calls_of(&s, &l, policy)?;
    let at = case_at(&calls, which);
    let mut calls = calls;
    let p = at.map(|i| calls.swap_remove(i)).ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let (text, call) = (p.text.clone(), &p.call);
    let module = build(&l.ir, policy, level)?;
    // (the case asked for has been taken out of `calls`: it is put
    // back for the check of a published feature's own cases)
    held(&s, &calls.iter().chain(std::iter::once(&p)).collect::<Vec<_>>(), &module, &l, Backend::Native, "zero-run", level)?;
    let kind = kind_of(Backend::Native);
    if let Some(why) = out_of_reach(&module, &l.funcs, kind).get(&call.func) {
        return Err(format!("{} is out of reach here: {}", text.split('→').next().unwrap_or("").trim(), skip_note(&l.funcs, why, kind)));
    }
    // the real clock leaves no marks (fm3 log 91): the times were
    // watched; on the virtual one a timed case is shown with them
    let timed = fast && matches!(call.expect, store::Expect::Timed(_));
    {
        use std::io::Write;
        println!("{}", text.split('→').next().unwrap_or("").trim());
        let _ = std::io::stdout().flush();
    }
    let sc = suite::Call { func: call.func.clone(), args: call.args.clone(), nrets: call.nrets, checks: checks(&call.expect), text: true, before: cut.into_iter().chain(setters(&call.context, &call.input, &call.input_at)).collect(), live: true, times: timed };
    // a check that fails is traced to its line by the diagnostic build
    // (fm3 log 199): a person is looking
    let got = match suite::run_calls(&module, &l.ir, Backend::Native, std::slice::from_ref(&sc), "zero-run", level)?.remove(0) {
        Err(e) if untraced(&e) => return Err(traced(dir, policy, Backend::Native, level, &sc, "zero-run").unwrap_or(e)),
        g => g?,
    };
    let vals: Vec<String> = got.values.iter().enumerate().map(|(k, v)| if call.times.get(k) == Some(&true) { store::spell_nanos(*v) } else { v.to_string() }).collect();
    let mut out = String::new();
    if !got.text.is_empty() && !got.text.ends_with('\n') {
        out.push('\n');
    }
    let shown = match call.expect {
        store::Expect::Timed(_) if timed => store::spell_timed(&pieces(&got)),
        store::Expect::Text(_) | store::Expect::Timed(_) => format!("{:?}", got.text.strip_suffix('\n').unwrap_or(&got.text)),
        _ => vals.join(", "),
    };
    out.push_str(&format!("→ {}\n", shown));
    Ok(out)
}

/// A wire as a command says it (fm3 log 289): its first delay, and
/// each change with the time it takes effect and the delay from then,
/// `GONE` where the wire goes. `--wire 100 ms then gone at 3 s then
/// 100 ms at 6 s`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wire {
    pub d: i64,
    pub then: Vec<(i64, i64)>,
    /// said beside it, `--choose <bound>` (fm3 log 290): the runner
    /// chooses the cut after the first, between it and nothing on the server
    pub choose: Option<i64>,
    /// its speed in bytes a second, 0 for as fast as it can be (fm3 log
    /// 294); and each change's, `None` where the change keeps the speed
    /// before it
    pub speed: i64,
    pub speeds: Vec<Option<i64>>,
    /// a wire that comes and goes: the gap, the period, and from when
    pub every: Option<(i64, i64, i64)>,
}

/// a gone wire's delay: the never-time of the world's text
pub const GONE: i64 = 1 << 62;

impl From<i64> for Wire {
    fn from(d: i64) -> Wire {
        Wire { d, ..Default::default() }
    }
}

impl From<&Wire> for Wire {
    fn from(w: &Wire) -> Wire {
        w.clone()
    }
}

impl Wire {
    /// as a person reads it: `100 ms then gone at 3 s`
    pub fn said(&self) -> String {
        let mut out = spell_late(self.d);
        if self.speed > 0 {
            out.push_str(&format!(", {} bytes/s", self.speed));
        }
        if let Some((gap, every, from)) = self.every {
            out.push_str(&format!(", gone {} every {} from {}", spell_late(gap), spell_late(every), store::spell_time(from)));
        }
        for (k, (at, d)) in self.then.iter().enumerate() {
            let speed = match self.speeds.get(k) {
                Some(Some(v)) if *d < GONE => format!(", {} bytes/s", v),
                _ => String::new(),
            };
            out.push_str(&format!(" then {}{} at {}", if *d >= GONE { "gone".to_string() } else { spell_late(*d) }, speed, store::spell_time(*at)));
        }
        out
    }
    /// does the wire have a speed or come and go (fm3 log 294): its
    /// build is then one whose items queue on each way
    pub fn paced(&self) -> bool {
        self.speed > 0 || self.speeds.iter().any(|v| v.is_some()) || self.every.is_some()
    }
    /// the calls that fill the build's table of changes, after the cut's
    pub fn calls(&self) -> Vec<(String, Vec<i64>)> {
        let mut out: Vec<(String, Vec<i64>)> = self.then.iter().enumerate().map(|(k, (at, d))| ("__wire_at".to_string(), vec![k as i64, *at, *d])).collect();
        if self.paced() {
            out.extend(self.pace_calls());
        }
        out
    }
    /// the speed at first and after each change, a change that says
    /// none keeping the one before it (fm3 log 294), and the coming and
    /// going; each way of the wire emptied
    fn pace_calls(&self) -> Vec<(String, Vec<i64>)> {
        let (gap, every, from) = self.every.unwrap_or((0, 0, 0));
        let mut out = vec![("__wire_pace".to_string(), vec![self.speed, from, every, gap])];
        let mut now = self.speed;
        for k in 0..self.then.len() {
            if let Some(Some(v)) = self.speeds.get(k) {
                now = *v;
            }
            out.push(("__wire_speed".to_string(), vec![k as i64, now]));
        }
        out
    }
    /// where the wire goes and does not come back, from when
    pub fn gone_for_good(&self) -> Option<i64> {
        match self.then.last() {
            Some((at, d)) if *d >= GONE => Some(self.then.iter().rev().take_while(|(_, d)| *d >= GONE).last().map(|(a, _)| *a).unwrap_or(*at)),
            _ => None,
        }
    }
    /// a wire said with changes is checked as `--then` is: forward in
    /// time, none at 0 s, `places::CHANGES` at most
    pub fn checked(self) -> Result<Wire, String> {
        if let Some((gap, every, from)) = self.every {
            if gap <= 0 || gap >= every {
                return Err(format!("a wire that comes and goes is gone for less than its period: gone {} every {}", spell_late(gap), spell_late(every)));
            }
            if from <= 0 {
                return Err("a wire comes and goes from a time after 0 s: `gone 500 ms every 2 s from 1 s`".to_string());
            }
        }
        if self.speed < 0 || self.speeds.iter().any(|v| v.is_some_and(|v| v <= 0)) {
            return Err("a wire's speed is a whole number of bytes a second above 0: `--wire 100 ms, 40 bytes/s`".to_string());
        }
        if self.then.len() > super::places::CHANGES {
            return Err(format!("a wire's delay may change {} times in one run, and {} changes are said", super::places::CHANGES, self.then.len()));
        }
        for (k, (at, _)) in self.then.iter().enumerate() {
            if *at <= 0 {
                return Err("a wire's change is at a time after 0 s: at 0 s the wire is its first delay, `--wire 2 s`".to_string());
            }
            if k > 0 && self.then[k - 1].0 >= *at {
                return Err(format!("the changes of a wire go forward in time: {} is not after {}", store::spell_time(*at), store::spell_time(self.then[k - 1].0)));
            }
        }
        Ok(self)
    }
}

/// beneath what was late, where the wire went for good and the case's
/// text stops short of what the case says (fm3 log 289): how many of
/// its pieces were never shown, a piece a line as `late:` counts them.
/// Nothing where the wire comes back or the text is not the start of
/// the case's
pub fn never_said(wire: &Wire, expect: &store::Expect, got: &str) -> String {
    let Some(from) = wire.gone_for_good() else { return String::new() };
    let want: String = match expect {
        store::Expect::Timed(p) => p.iter().map(|(t, _)| t.as_str()).collect(),
        store::Expect::Text(t) => t.clone(),
        _ => return String::new(),
    };
    let (want, got) = (want.strip_suffix('\n').unwrap_or(&want).to_string(), got.strip_suffix('\n').unwrap_or(got).to_string());
    if got.len() >= want.len() || !want.starts_with(&got) {
        return String::new();
    }
    let rest = &want[got.len()..];
    let n = rest.split_inclusive('\n').filter(|l| !l.trim_end_matches('\n').is_empty()).count();
    format!("never shown: {} piece{}, the wire gone from {}\n", n, if n == 1 { "" } else { "s" }, store::spell_time(from))
}

/// A store built for a wire that takes time (fm3 cut.md, "a wire that
/// takes time"; log 269), on the runner's clock
fn wired(dir: &Path) -> Result<store::Store, String> {
    let mut s = store::read(dir).map_err(|e| e.to_string())?;
    s.clock = store::Clock::Virtual;
    s.places = true;
    s.wire = true;
    s.times = true;
    Ok(s)
}

/// a time as a lateness is said: `200 ms`, `1.5 s`
pub fn spell_late(us: i64) -> String {
    if us % 1000 == 0 && us.abs() < 1_000_000 {
        format!("{} ms", us / 1000)
    } else {
        store::spell_time(us)
    }
}

/// One piece of what a case wrote over a wire that takes time: its
/// text, the time it is for, which is the program's, and when it was
/// shown, by the world's clock
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub own: i64,
    pub shown: i64,
}

/// What was written, cut where either time changes and after each
/// line: `own` is the capture's marks, a word where the clock moved,
/// and `shown` a word for each character
pub fn shown_pieces(text: &str, own: &[i64], shown: &[i64]) -> Vec<Shown> {
    let bytes = text.as_bytes();
    let mut out: Vec<Shown> = Vec::new();
    let mut now = 0i64;
    let mut cur: Option<(usize, i64, i64)> = None;
    for i in 0..bytes.len() {
        if let Some(t) = own.get(i).filter(|t| **t != 0) {
            now = *t;
        }
        let at = shown.get(i).copied().unwrap_or(0).max(now);
        match cur {
            Some((_, o, s)) if o == now && s == at => {}
            Some((from, o, s)) => {
                out.push(Shown { text: String::from_utf8_lossy(&bytes[from..i]).to_string(), own: o, shown: s });
                cur = Some((i, now, at));
            }
            None => cur = Some((i, now, at)),
        }
        if bytes[i] == b'\n' {
            let (from, o, s) = cur.take().unwrap();
            out.push(Shown { text: String::from_utf8_lossy(&bytes[from..=i]).to_string(), own: o, shown: s });
        }
    }
    if let Some((from, o, s)) = cur {
        out.push(Shown { text: String::from_utf8_lossy(&bytes[from..]).to_string(), own: o, shown: s });
    }
    out
}

/// what was late, as `run` says it beneath a case's line
pub fn late_said(pieces: &[Shown]) -> String {
    let late: Vec<&Shown> = pieces.iter().filter(|p| p.shown > p.own).collect();
    if late.is_empty() {
        return format!("late: nothing, of {} piece{}\n", pieces.len(), if pieces.len() == 1 { "" } else { "s" });
    }
    let most = late.iter().map(|p| p.shown - p.own).max().unwrap_or(0);
    let mut out = format!("late: {} piece{} of {}, the most {}\n", late.len(), if late.len() == 1 { "" } else { "s" }, pieces.len(), spell_late(most));
    for p in late {
        out.push_str(&format!("  {:?} for {}, shown at {}, {} late\n", p.text, store::spell_time(p.own), store::spell_time(p.shown), spell_late(p.shown - p.own)));
    }
    out
}

/// One case at a cut over a wire that takes time: what it gave, by
/// its own times; each piece with when it was shown; and the wire's
/// record, six words an item. The case is run three times, the runner
/// reading back one thing each time by the words it has on every path
/// (fm3 log 269); `but` is a test's switch
#[allow(clippy::too_many_arguments)]
pub(super) fn wire_runs(module: &ssa::Module, l: &lower::Lowered, backend: Backend, level: usize, name: &str, cut: &[(String, Vec<i64>)], wire: i64, but: i64, p: &Planned, switches: &[(String, bool)]) -> Result<(Result<suite::Got, String>, Vec<Shown>, Vec<i64>, Option<i64>), String> {
    let c = &p.call;
    let call = |mode: i64| suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: cut.iter().cloned().chain([("__wire_set".to_string(), vec![wire]), ("__wire_but".to_string(), vec![but]), ("__wire_mode".to_string(), vec![mode])]).chain(setters(switches, &c.input, &c.input_at)).collect(), live: false, times: true };
    let mut got = suite::run_calls(module, &l.ir, backend, &[call(0), call(1), call(2)], name, level)?;
    let log = got.pop().unwrap().map(|g| g.marks).unwrap_or_default();
    let shown = got.pop().unwrap();
    let own = got.pop().unwrap();
    let pieces = match (&own, &shown) {
        (Ok(o), Ok(s)) => shown_pieces(&o.text, &o.marks, &s.marks),
        _ => Vec::new(),
    };
    let thrown = if works_ahead(&l.ir) { shown.as_ref().ok().and_then(|s| s.marks.last().copied()) } else { None };
    Ok((own, pieces, log, thrown))
}

/// Does a build for a wire work ahead of a key (fm3 cut.md, "working
/// ahead of a key"; log 273): its store has an input that arrives at
/// a time, and so a bound the server may work ahead of. Such a build says,
/// in the word after the last of when each character was shown, how
/// many turns the server took and threw away
pub(super) fn works_ahead(ir: &str) -> bool {
    ir.contains("\ndata __wa: ")
}

/// what working ahead cost, as `run` says it beneath what was late
pub fn thrown_said(thrown: Option<i64>) -> String {
    match thrown {
        None => String::new(),
        Some(0) => "the server worked ahead: no turn it took was thrown away\n".to_string(),
        Some(n) => format!("the server worked ahead: {} turn{} it took {} thrown away\n", n, if n == 1 { "" } else { "s" }, if n == 1 { "was" } else { "were" }),
    }
}

/// `probe zero <store> run <case> --server <cut> --wire <time>`: the case
/// by its own times, which is what is compared with the case, and
/// beneath it what was late
#[allow(dead_code)]
pub fn run_wire(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, server: Option<&str>, wire: i64) -> Result<String, String> {
    run_wire_moved(dir, which, policy, level, server, &[], wire)
}

/// ... and the cut moved while it runs (fm3 cut.md, "the cut moved
/// over a wire that takes time"; log 276): `then` is each cut gone to
/// and the time the move is said at the client
pub fn run_wire_moved<W: Into<Wire>>(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, server: Option<&str>, then: &[(String, i64)], wire: W) -> Result<String, String> {
    let wire: Wire = wire.into();
    let ((head, shown, pieces, thrown), expect, log) = wire_moved_case_said(dir, which, policy, level, Backend::Native, server, then, &wire, said_but(), None)?;
    let never = never_said(&wire, &expect, &pieces.iter().map(|p| p.text.as_str()).collect::<String>());
    Ok(format!("{}\n→ {}\n{}{}{}{}", head, shown, late_said(&pieces), never, thrown_said(thrown), chose_said(&wire, &log)))
}

/// the switches a test may set (`__wire_but`), said for `run` and
/// `placement` by `PROBE_ZERO_BUT=<n>` (fm3 log 298): 3072 is hop
/// fifty-two's chooser, as `PROBE_ZERO_CUTS=choose52` has it over the suite
fn said_but() -> i64 {
    std::env::var("PROBE_ZERO_BUT").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// what the chooser chose (fm3 log 290), as `run` says it, from the
/// wire's record: each choice with its time and its reason, and the
/// client's own round trips
pub fn chose_said(wire: &Wire, log: &[i64]) -> String {
    let Some(b) = wire.choose else { return String::new() };
    let mut out = String::new();
    for item in log.chunks(6).filter(|c| c.len() == 6) {
        match item[1] {
            15 => out.push_str(&format!("chose: nothing on the server at {}{}: a round trip of its own out longer than {}, the client waiting on the server\n", store::spell_time(item[2]), if log.chunks(6).any(|c| c.len() == 6 && c[1] == 18 && c[2] == item[2]) { ", by running the server's part again" } else { "" }, spell_late(b))),
            16 => out.push_str(&format!("chose: the cut given at {}: {}\n", store::spell_time(item[2]), weighed_said(item, log, b))),
            24 => out.push_str(&format!("kept: nothing on the server at {}: about {} bytes/s, less than twice the {} bytes/s the cut given put on the wire\n", store::spell_time(item[2]), two_figures(item[4]), two_figures(item[5]))),
            25 => out.push_str(&format!("kept: nothing on the server at {}: {}, so {} of round trips within {} are wanted\n", store::spell_time(item[2]), gaps_said(item[5]), spell_late(item[4]), spell_late(b))),
            _ => {}
        }
    }
    if out.is_empty() {
        out.push_str("chose: nothing; the cut given held throughout\n");
    }
    out.push_str(&measured_said(log));
    let (n, most) = round_trips(log);
    out.push_str(&format!("the client's own round trips: {}{}\n", n, if n > 0 { format!(", the longest back {}", spell_late(most)) } else { String::new() }));
    out
}

/// what a choice to give the server its part back weighed (fm3 log 296): how
/// long the round trips were steady, and why that long; what was due;
/// and, where a speed was seen, the speed against the rate the cut
/// given put on the wire, and the move's own bytes. As hop fifty-one
/// said it where no speed was seen and the wire went once
fn weighed_said(item: &[i64], log: &[i64], b: i64) -> String {
    let w = log.chunks(6).filter(|c| c.len() == 6).find(|c| c[1] == 23 && c[2] == item[2]);
    let (steady, within) = (item[4], item[5]);
    let mut out = if steady == 1_000_000 || w.is_none() { format!("a second of round trips within {}", spell_late(b)) } else { format!("{} of round trips within {}, {}", spell_late(steady), spell_late(b), gaps_said(w.unwrap()[0])) };
    out.push_str(&format!(", nothing due within {}", spell_late(within)));
    if let Some(w) = w.filter(|w| w[4] > 0) {
        let took = (w[3] * 1_000_000 + w[4] - 1) / w[4];
        out.push_str(&format!(", the move's {} bytes {} of it; about {} bytes/s, at least twice the {} bytes/s the cut given put on the wire", w[3], spell_late((took + 500) / 1000 * 1000), two_figures(w[4]), two_figures(w[5])));
    }
    out
}

/// how often and how long the wire went in the last ten seconds, as
/// the chooser's world packs it: the count times 2^40 and the time
fn gaps_said(packed: i64) -> String {
    let (n, gone) = (packed >> 40, packed & ((1 << 40) - 1));
    match n {
        0 => "not gone in the last 10 s".to_string(),
        n => format!("gone {} time{} in the last 10 s, {} in all", n, if n == 1 { "" } else { "s" }, spell_late(gone)),
    }
}

/// what the chooser measured of the wire last (fm3 log 295), from the
/// line of the record its world wrote at its end: the delay, the speed
/// where one was seen, and how often and how long the wire went in the
/// last ten seconds
fn measured_said(log: &[i64]) -> String {
    let Some(m) = log.chunks(6).filter(|c| c.len() == 6).find(|c| c[1] == 22) else { return String::new() };
    let (delay, speed) = (m[3], m[4]);
    let speed = if speed > 0 { format!("about {} bytes/s", two_figures(speed)) } else { "its speed not seen to limit".to_string() };
    let gone = gaps_said(m[5]);
    format!("the wire: about {}, {}, {}\n", spell_late(two_figures(delay)), speed, gone)
}

/// a whole number to two figures: 39 is 39, 9 987 is 10 000
fn two_figures(n: i64) -> i64 {
    let mut unit = 1;
    while n / unit >= 100 {
        unit *= 10;
    }
    (n + unit / 2) / unit * unit
}

/// the client's own round trips in a wire's record: how many were
/// begun, and the longest that came back. The second of a pair, sent
/// to measure the speed (fm3 log 295), is numbered below 0 and is not
/// one
fn round_trips(log: &[i64]) -> (usize, i64) {
    let rows: Vec<&[i64]> = log.chunks(6).filter(|c| c.len() == 6).collect();
    let sent: Vec<(i64, i64)> = rows.iter().filter(|r| r[1] == 13 && r[5] > 0).map(|r| (r[5], r[2])).collect();
    let most = rows.iter().filter(|r| r[1] == 14 && r[3] < GONE).filter_map(|r| sent.iter().find(|(n, _)| *n == r[5]).map(|(_, at)| r[3] - at)).max().unwrap_or(0);
    (sent.len(), most)
}

/// ... as its parts: the case's call as written, what it gave by its
/// own times, and each piece with when it was shown. `but` is a
/// test's switch, a place that does not wait to be told
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(super) fn wire_case(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, backend: Backend, server: Option<&str>, wire: i64, but: i64) -> Result<(String, String, Vec<Shown>), String> {
    wire_case_cost(dir, which, policy, level, backend, server, wire, but).map(|(head, shown, pieces, _)| (head, shown, pieces))
}

/// ... and, where the build works ahead of a key, how many turns the server
/// took and threw away (`but`: 4, the server is not put back; 8, the client
/// does not drop what it is keeping)
#[allow(clippy::too_many_arguments)]
pub(super) fn wire_case_cost(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, backend: Backend, server: Option<&str>, wire: i64, but: i64) -> Result<(String, String, Vec<Shown>, Option<i64>), String> {
    wire_moved_case(dir, which, policy, level, backend, server, &[], wire, but, None)
}

/// ... and with the cut's moves; `dropped` is a test's, one thing a
/// move deliberately does not hand across
#[allow(clippy::too_many_arguments)]
pub(super) fn wire_moved_case<W: Into<Wire>>(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, backend: Backend, server: Option<&str>, then: &[(String, i64)], wire: W, but: i64, dropped: Option<&str>) -> Result<(String, String, Vec<Shown>, Option<i64>), String> {
    wire_moved_case_said(dir, which, policy, level, backend, server, then, wire, but, dropped).map(|(r, _, _)| r)
}

/// ... and what the case says it gives
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn wire_moved_case_said<W: Into<Wire>>(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, backend: Backend, server: Option<&str>, then: &[(String, i64)], wire: W, but: i64, dropped: Option<&str>) -> Result<((String, String, Vec<Shown>, Option<i64>), store::Expect, Vec<i64>), String> {
    let wire: Wire = wire.into();
    let mut s = wired(dir)?;
    s.moved = !then.is_empty() || wire.choose.is_some();
    s.changes = !wire.then.is_empty() || wire.paced();
    s.paced = wire.paced();
    s.choose = wire.choose.is_some();
    let policy = &store_policy(&s, policy);
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let placed = l.placed.as_ref().ok_or("the store was not placed")?;
    let mut cut = match (dropped, wire.choose) {
        (_, Some(_)) => chosen_cut(placed, server, then, &wire)?,
        (None, _) => cut_of(placed, server, then)?.before,
        (Some(_), _) => {
            let (first, later) = cuts_named(placed, server, then)?;
            placed.moves_but(&first, &later, dropped).before
        }
    };
    cut.extend(wire.calls());
    let calls = calls_of(&s, &l, policy)?;
    let p = case_named(&calls, which).ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let module = build(&l.ir, policy, level)?;
    let (own, pieces, log, thrown) = wire_runs(&module, &l, backend, level, "zero-wire", &cut, wire.d, but, p, &p.call.context)?;
    let got = own?;
    let vals: Vec<String> = got.values.iter().enumerate().map(|(k, v)| if p.call.times.get(k) == Some(&true) { store::spell_nanos(*v) } else { v.to_string() }).collect();
    let shown = match p.call.expect {
        store::Expect::Timed(_) => store::spell_timed(&self::pieces(&got)),
        store::Expect::Text(_) => format!("{:?}", got.text.strip_suffix('\n').unwrap_or(&got.text)),
        _ => vals.join(", "),
    };
    Ok(((p.text.split('→').next().unwrap_or("").trim().to_string(), shown, pieces, thrown), p.call.expect.clone(), log))
}

/// The calls that fill a build whose cut the runner chooses (fm3 log
/// 290): the cut given, and the four moves' tables, nothing on the server and the
/// cut given by turns, each due at the never-time until the chooser
/// makes it due; then the chooser's bound, none where the cut given is
/// nothing on the server. `--choose` with `--then` is refused by name
fn chosen_cut(placed: &super::places::Placed, server: Option<&str>, then: &[(String, i64)], wire: &Wire) -> Result<Vec<(String, Vec<i64>)>, String> {
    if !then.is_empty() {
        return Err("`--choose` and `--then` are not said together: with `--choose` the runner chooses each cut after the first, and `--then` would say one".to_string());
    }
    let (first, _) = cuts_named(placed, server, &[])?;
    if let Some(why) = placed.refused(&first) {
        return Err(format!("this cut cannot be run yet: {}", why));
    }
    let none = BTreeSet::new();
    for (a, b) in [(&first, &none), (&none, &first)] {
        if let Some(why) = placed.move_refused(a, b) {
            return Err(format!("the runner cannot choose between this cut and nothing on the server yet: {}", why));
        }
    }
    let later: Vec<(BTreeSet<usize>, i64)> = (0..super::places::MOVES).map(|k| (if k % 2 == 0 { none.clone() } else { first.clone() }, GONE)).collect();
    let mut cut = placed.moves(&first, &later).before;
    cut.push(("__wire_choose".to_string(), vec![if first.is_empty() { 0 } else { wire.choose.unwrap_or(0) }]));
    Ok(cut)
}

/// `probe zero <store> placement --server <cut> --wire <time>`: the table
/// as it is, and then one case run over the wire: each thing the wire
/// carried, when it left and when it arrived by the world's clock, and
/// what was late. The case is the one named, or the store's first
/// that says a time
#[allow(dead_code)]
pub fn placement_wire(dir: &Path, server: Option<&str>, wire: i64, which: Option<&str>, policy: &ssa::Policy, level: usize) -> Result<String, String> {
    placement_wire_moved(dir, server, &[], wire, which, policy, level)
}

/// ... and with the cut's moves (fm3 log 276): the tables of the build
/// for a wire, with what each move hands across in it; and in the list
/// of what the wire carried, the move's own items
pub fn placement_wire_moved<W: Into<Wire>>(dir: &Path, server: Option<&str>, then: &[(String, i64)], wire: W, which: Option<&str>, policy: &ssa::Policy, level: usize) -> Result<String, String> {
    let wire: Wire = wire.into();
    let mut s = wired(dir)?;
    s.moved = !then.is_empty() || wire.choose.is_some();
    s.changes = !wire.then.is_empty() || wire.paced();
    s.paced = wire.paced();
    s.choose = wire.choose.is_some();
    let policy = &store_policy(&s, policy);
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let placed = l.placed.as_ref().ok_or("the store was not placed")?;
    let (first, later) = cuts_named(placed, server, then)?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let calls = calls_of(&s, &l, policy)?;
    let cases: Vec<String> = calls.iter().map(|c| c.call.func.trim_start_matches("__whole_").trim_start_matches("__said_").to_string()).collect();
    // the table as it is, of the build whose wire takes no time: what
    // crosses is the program's and the cut's, whatever the wire
    let mut out = if later.is_empty() { placement(dir, server, &[])? } else { placed.table_moved(&name, &first, &later, &cases) };
    if let Some(why) = placed.refused(&first) {
        if !out.contains("\nrefused: ") {
            out.push_str(&format!("\nover a wire that takes time, refused: {}\n", why));
        }
        return Ok(out);
    }
    if out.contains("\nrefused: ") {
        return Ok(out);
    }
    let p = match which {
        Some(w) => case_named(&calls, w).ok_or_else(|| format!("no case '{}' in the store's ## testing sections", w))?,
        None => calls.iter().find(|c| matches!(c.call.expect, store::Expect::Timed(_))).or(calls.first()).ok_or("the store has no case")?,
    };
    let mut cut = if wire.choose.is_some() { chosen_cut(placed, server, then, &wire)? } else { placed.moves(&first, &later).before };
    cut.extend(wire.calls());
    let module = build(&l.ir, policy, level)?;
    let (own, pieces, log, thrown) = wire_runs(&module, &l, Backend::Native, level, "zero-wire", &cut, wire.d, said_but(), p, &p.call.context)?;
    let got = own?;
    out.push_str(&format!("\na wire of {}, by the world's clock: `{}`\n\n", wire.said(), p.text.split('→').next().unwrap_or("").trim()));
    // the wire's record, text a character an item put together; and,
    // where the server works ahead of a key (fm3 log 273), beside what the
    // wire carried: each piece the client dropped unshown, each time
    // the server went back, and each turn it took ahead and took back
    let mut rows: Vec<[String; 5]> = Vec::new();
    let mut text: Option<(i64, i64, i64, i64, String)> = None;
    // (over a wire with a speed, fm3 log 294: each item's size and when
    // it finished going on, from the line of the record beneath it; a
    // piece of text's summed, a row of the table each)
    let mut sized: HashMap<usize, (i64, i64)> = HashMap::new();
    let mut piece: Option<(i64, i64)> = None;
    // (a ping's line and its answer's are not rows, nor their sizes)
    let mut skip21 = false;
    let flush = |text: &mut Option<(i64, i64, i64, i64, String)>, rows: &mut Vec<[String; 5]>, piece: &mut Option<(i64, i64)>, sized: &mut HashMap<usize, (i64, i64)>| {
        if let Some(z) = piece.take() {
            if text.is_some() {
                sized.insert(rows.len(), z);
            }
        }
        if let Some((to, left, at, stamp, t)) = text.take() {
            if to == 6 {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("dropped unshown: text {:?}", t), format!("for {}", store::spell_time(stamp))]);
            } else if to == 19 {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("dropped, made before the server heard of the move: text {:?}", t), format!("for {}", store::spell_time(stamp))]);
            } else {
                rows.push([store::spell_time(left), store::spell_time(at), "server → client".to_string(), format!("text {:?}", t), format!("for {}", store::spell_time(stamp))]);
            }
        }
    };
    // (a block's record is how many characters and the first seven)
    let block = |a: i64| {
        let (mut w, mut chars) = (a, Vec::new());
        while w > 255 {
            chars.push((w & 255) as u8);
            w >>= 8;
        }
        chars.reverse();
        let n = w as usize;
        format!("text {:?}{}", String::from_utf8_lossy(&chars).to_string(), if n > chars.len() { format!(" and {} more", n - chars.len()) } else { String::new() })
    };
    let mut start: Option<i64> = None;
    for item in log.chunks(6).filter(|c| c.len() == 6) {
        let (to, kind, left, at, stamp, a) = (item[0], item[1], item[2], item[3], item[4], item[5]);
        let way = if to == 1 { "client → server" } else { "server → client" };
        // (text sent, and text dropped, a character a line of the record)
        // (and a character dropped by its count, fm3 log 292)
        let dropped_char = kind == 19 && a & 15 == 1;
        if kind == 21 && skip21 {
            skip21 = false;
            continue;
        }
        if kind == 21 {
            let (n, by) = (item[4], item[3]);
            match (&text, &mut piece) {
                (Some(_), Some((sz, on))) => {
                    *sz += n;
                    *on = (*on).max(by);
                }
                (Some(_), None) => piece = Some((n, by)),
                (None, _) => {
                    if !rows.is_empty() {
                        sized.insert(rows.len() - 1, (n, by));
                    }
                }
            }
            continue;
        }
        if kind == 1 || kind == 6 || dropped_char {
            let tag = if kind == 1 { to } else { kind };
            let ch = if dropped_char { (a >> 4) as u8 as char } else { a as u8 as char };
            match &mut text {
                Some((t, l2, a2, s2, chars)) if (*t, *l2, *a2, *s2) == (tag, left, at, stamp) => chars.push(ch),
                _ => {
                    flush(&mut text, &mut rows, &mut piece, &mut sized);
                    text = Some((tag, left, at, stamp, ch.to_string()));
                }
            }
            continue;
        }
        flush(&mut text, &mut rows, &mut piece, &mut sized);
        match kind {
            7 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("dropped unshown: {}", block(a)), format!("for {}", store::spell_time(stamp))]);
                continue;
            }
            8 => {
                rows.push([store::spell_time(left), String::new(), "the server".to_string(), format!("goes back to {}: {} turn{} thrown away", store::spell_time(stamp), a, if a == 1 { "" } else { "s" }), String::new()]);
                continue;
            }
            9 => {
                rows.push([store::spell_time(left), String::new(), "the server".to_string(), "takes back a turn it took ahead: it would have sent more than text".to_string(), format!("for {}", store::spell_time(stamp))]);
                continue;
            }
            // (the chooser's, fm3 log 290: its round trips are counted beneath, and each choice is a row)
            13 | 14 => {
                skip21 = true;
                continue;
            }
            22 | 23 => continue,
            24 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("keeps nothing on the server: about {} bytes/s, less than twice the {} bytes/s the cut given put on the wire", two_figures(stamp), two_figures(a)), String::new()]);
                continue;
            }
            25 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("keeps nothing on the server: {}, so {} of round trips within the bound are wanted", gaps_said(a), spell_late(stamp)), String::new()]);
                continue;
            }
            15 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("chooses nothing on the server: a round trip of its own out longer than {}", spell_late(a)), String::new()]);
                continue;
            }
            16 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("chooses the cut given: {} of round trips within the bound, nothing due within {}", if stamp == 1_000_000 { "a second".to_string() } else { spell_late(stamp) }, spell_late(a)), String::new()]);
                continue;
            }
            // (the server's new start at a giving back, fm3 log 298)
            26 => {
                start = Some(left);
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("keeps the server's state as the server will have it once it takes the hand as the server's new start, and empties its log: the server's part run again from its start, {} turn{}, {} item{} of its log", a & 0xfffff, if a & 0xfffff == 1 { "" } else { "s" }, (a >> 20) & 0xfffff, if (a >> 20) & 0xfffff == 1 { "" } else { "s" }), String::new()]);
                continue;
            }
            // (the client running the server's part again, fm3 log 292)
            18 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("runs the server's part again from {} to {}: {} turn{}, {} item{} of its log given at their times; takes the server's units{}", start.map(|t| format!("the server's start at {}", store::spell_time(t))).unwrap_or_else(|| "its start".to_string()), store::spell_time(left), a & 0xfffff, if a & 0xfffff == 1 { "" } else { "s" }, (a >> 20) & 0xfffff, if (a >> 20) & 0xfffff == 1 { "" } else { "s" }, match a >> 40 { 0 => String::new(), n => format!(", and {} push{} or function{} partway the server's part sent that it had not had", n, if n == 1 { "" } else { "es" }, if n == 1 { "" } else { "s" }) }), String::new()]);
                continue;
            }
            20 => {
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("{} item{} of the server's text it had not shown, kept to be shown now; {} it was keeping from the server, its own turns' to write now", a & 0xfffff, if a & 0xfffff == 1 { "" } else { "s" }, a >> 20), String::new()]);
                continue;
            }
            19 => {
                let what = match a & 15 {
                    2 => "a block of text".to_string(),
                    3 => "a push into a stored stream".to_string(),
                    4 => "a function partway".to_string(),
                    5 => "a telling".to_string(),
                    11 => "the server's state".to_string(),
                    12 => "a function partway, handed across at a move".to_string(),
                    k => format!("an item of kind {}", k),
                };
                rows.push([store::spell_time(left), String::new(), "at the client".to_string(), format!("dropped, made before the server heard of the move: {}", what), format!("for {}", store::spell_time(stamp))]);
                continue;
            }
            _ => {}
        }
        let what = match kind {
            2 => block(a),
            3 => "a push into a stored stream, made again where it is read".to_string(),
            4 => format!("a function partway, to go on in {}", placed.continued(a)),
            // (the items of a move, fm3 log 276)
            10 => "the move's word: the cut's new table, and what the client hands with it".to_string(),
            11 => "the server's state: what the server hands across at the move".to_string(),
            12 => format!("a function partway, handed across at the move, to go on in {} when it is next due", placed.continued(a)),
            17 => "the move's word, the server's part taken by the client: the cut's new table, and nothing to send back".to_string(),
            // (the client's bound where a key may yet be pressed is the moment after the world's clock)
            _ if a == left + 1 => format!("a telling: nothing for a time up to {}", store::spell_time(left)),
            _ => format!("a telling: nothing before {}", if a >= 1 << 62 { "the end".to_string() } else { store::spell_time(a) }),
        };
        rows.push([store::spell_time(left), store::spell_time(at), way.to_string(), what, format!("for {}", store::spell_time(stamp))]);
    }
    flush(&mut text, &mut rows, &mut piece, &mut sized);
    // (with a speed, two columns more: when it had gone on, and its bytes)
    let mut rows: Vec<Vec<String>> = rows.into_iter().enumerate().map(|(k, r)| {
        let mut v = r.to_vec();
        if wire.paced() {
            let (on, n) = sized.get(&k).map(|(n, by)| (store::spell_time(*by), n.to_string())).unwrap_or_default();
            v.insert(1, on);
            v.insert(2, n);
        }
        v
    }).collect();
    rows.insert(0, if wire.paced() { ["left", "on by", "bytes", "arrived", "", "what", "its own time"] } else { ["left", "arrived", "", "what", "its own time", "", ""] }.iter().take(if wire.paced() { 7 } else { 5 }).map(|s| s.to_string()).collect());
    let cols = rows[0].len();
    let w: Vec<usize> = (0..cols - 1).map(|k| rows.iter().map(|r| r[k].chars().count()).max().unwrap_or(0)).collect();
    for r in &rows {
        let padded = |s: &str, n: usize| format!("{}{}", s, " ".repeat(n.saturating_sub(s.chars().count())));
        let line: Vec<String> = (0..cols - 1).map(|k| padded(&r[k], w[k])).chain([r[cols - 1].clone()]).collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    let shown = match p.call.expect {
        store::Expect::Timed(_) => store::spell_timed(&self::pieces(&got)),
        _ => format!("{:?}", got.text.strip_suffix('\n').unwrap_or(&got.text)),
    };
    let never = never_said(&wire, &p.call.expect, &pieces.iter().map(|p| p.text.as_str()).collect::<String>());
    out.push_str(&format!("\n→ {}\n{}{}{}{}", shown, late_said(&pieces), never, thrown_said(thrown), chose_said(&wire, &log)));
    Ok(out)
}

/// every store under `dir`, its cases run on the backend, reported as the suite does
pub fn test(dir: &Path, backend: Backend, level: usize) -> Result<Report, String> {
    let mut stores: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    stores.sort();
    if stores.is_empty() {
        return Err(format!("no stores under {}", dir.display()));
    }
    let policy = suite::backend_policy(backend)?;
    let mut report = Report { passed: 0, failed: 0, skipped: 0, log: String::new() };
    // the closing line's numbers (log 44): cases, and the runs overridden
    let (mut ncases, mut nover) = (0, 0);
    for sdir in &stores {
        let name = sdir.file_name().unwrap().to_string_lossy().to_string();
        let result = (|| -> Result<(Vec<Planned>, Vec<Run>, Vec<Over>, ssa::Module, lower::Lowered), String> {
            let mut s = store::read(sdir).map_err(|e| e.to_string())?;
            // the suite keeps the virtual clock whatever the product says (log 77)
            s.clock = store::Clock::Virtual;
            // a check of the diagnostic build itself (fm3 log 199): with
            // PROBE_ZERO_SITES set every case is run from it, and gives
            // what it gives from the program (the expected-IR checks
            // aside, the text being another)
            s.sites = std::env::var("PROBE_ZERO_SITES").is_ok();
            // ... and of the trace build (fm3 tracer.md) with
            // PROBE_ZERO_TRACE: what a program gives is not changed by
            // its being watched
            s.trace = std::env::var("PROBE_ZERO_TRACE").is_ok();
            let policy = store_policy(&s, &policy);
            let l = lower::lower(&s).map_err(|e| e.to_string())?;
            let cases = calls_of(&s, &l, &policy)?;
            let (runs, over) = plan(&s, &cases)?;
            let module = build(&l.ir, &policy, level)?;
            held(&s, &cases.iter().collect::<Vec<_>>(), &module, &l, backend, &name, level)?;
            Ok((cases, runs, over, module, l))
        })();
        let (cases, runs, over, module, lowered) = match result {
            Ok(r) => r,
            Err(e) => {
                report.failed += 1;
                report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
                continue;
            }
        };
        // `<name>.expected.ssa` beside the store: the IR the front end
        // emitted when it was accepted, so a lowering change is a diff
        // (log 33); the first line names the store's path and is not compared
        let expected = dir.join(format!("{}.expected.ssa", name));
        if let Ok(want) = std::fs::read_to_string(&expected) {
            let (got_lines, want_lines): (Vec<&str>, Vec<&str>) = (lowered.ir.lines().skip(1).collect(), want.lines().skip(1).collect());
            let file = expected.display().to_string();
            ncases += 1;
            match got_lines.iter().zip(&want_lines).position(|(g, w)| g != w).or_else(|| (got_lines.len() != want_lines.len()).then_some(got_lines.len().min(want_lines.len()))) {
                None => report.case(true, &name, &format!("{} matches the emitted IR", file), ""),
                Some(i) => {
                    let show = |v: &Vec<&str>| v.get(i).map(|l| l.trim().to_string()).unwrap_or_else(|| "(the end)".into());
                    report.case(false, &name, &format!("{} differs from the emitted IR", file), &format!("(line {}: emitted `{}`, expected `{}`; `probe zero {} emit > {}` accepts the change)", i + 2, show(&got_lines), show(&want_lines), sdir.display(), file));
                }
            }
        }
        // a case whose function is out of reach on this kind of place
        // (section 15, log 31) is skipped, saying which platform
        // function has no body for it
        let kind = kind_of(backend);
        let unreached = out_of_reach(&module, &lowered.funcs, kind);
        ncases += cases.len();
        nover += over.len();
        for o in &over {
            report.log.push_str(&format!("over  {:<16} {} ({}): {}\n", name, o.text, o.context, o.why));
        }
        let (runs, skipped): (Vec<&Run>, Vec<&Run>) = runs.iter().partition(|r| !unreached.contains_key(&cases[r.case].call.func));
        for r in skipped {
            report.skipped += 1;
            report.log.push_str(&format!("skip  {:<16} {}: {}\n", name, labelled(&cases[r.case].text, &r.label), skip_note(&lowered.funcs, &unreached[&cases[r.case].call.func], kind)));
        }
        let scalls: Vec<suite::Call> = runs
            .iter()
            .map(|r| {
                let c = &cases[r.case].call;
                suite::Call {
                    func: c.func.clone(),
                    args: c.args.clone(),
                    nrets: c.nrets,
                    checks: checks(&c.expect),
                    // every case reads the text back: a failed check names its site there
                    text: true,
                    before: setters(&r.switches, &c.input, &c.input_at),
                    live: false,
                    times: matches!(c.expect, store::Expect::Timed(_)),
                }
            })
            .collect();
        if scalls.is_empty() {
            continue;
        }
        let got = match suite::run_calls(&module, &lowered.ir, backend, &scalls, &name, level) {
            Ok(g) => g,
            Err(e) => {
                report.failed += runs.len().max(1);
                report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
                continue;
            }
        };
        for ((r, got), sc) in runs.iter().zip(got).zip(&scalls) {
            let text = labelled(&cases[r.case].text, &r.label);
            // a check that failed and named no line is traced to it by
            // the diagnostic build, where the case did not expect one
            // or says which line it expects (fm3 log 199); a case that
            // says `→ check` and no more is run once, as it was
            let got = match got {
                Err(e) if untraced(&e) && cases[r.case].call.expect != store::Expect::Check => Err(traced(sdir, &policy, backend, level, sc, &name).unwrap_or(e)),
                // (the suite run from the trace build, PROBE_ZERO_TRACE: a
                // case that writes down more than the trace holds is
                // stopped by the trace, and is no case of it)
                Err(e) if e.contains(crate::host::TRACE_FULL) => Err(format!("skip: the trace is full: a run of more than {} events", crate::host::TRACE_EVENTS)),
                // (the suite run from the diagnostic build itself,
                // PROBE_ZERO_SITES: the site is already in hand)
                Err(e) if !lowered.sites.is_empty() => Err(site_said(&lowered.sites, &e).unwrap_or(e)),
                g => g,
            };
            let got = if lowered.sites.is_empty() { got } else { got.map(unsited) };
            // a case a path cannot run (air: a failed check, recursion)
            // is skipped, as the suite skips its own
            if let Some(why) = got.as_ref().err().and_then(|e| e.strip_prefix("skip: ")) {
                report.skipped += 1;
                report.log.push_str(&format!("skip  {:<16} {}: {}\n", name, text, why));
                continue;
            }
            let (ok, note) = judge(&cases[r.case].call.expect, got);
            report.case(ok, &name, &text, &note);
        }
    }
    // the suite at every cut (fm3 cut.md), where it is asked for: each
    // store's cases run again from its placed build with every unit
    // the server and with each unit alone on the server, and compared with the case's
    // own. Counted apart, after the line that counts the suite's runs
    let mut cuts = Cuts::default();
    // ... and over a wire that takes time (fm3 cut.md, "a wire that
    // takes time"), `PROBE_ZERO_CUTS=wire` for a wire of no delay and
    // `wire=100` for one of that many milliseconds: each store's build
    // for a wire, every case at every cut, by its own times; and how
    // late each cut shows what it shows. `wire-timed=100`: only the
    // stores that have a case with a time
    let wire_asked = std::env::var("PROBE_ZERO_CUTS").ok().filter(|v| v.starts_with("wire"));
    let mut wires = Wires::default();
    // ... and the cut moved over such a wire (fm3 cut.md, "the cut
    // moved over a wire that takes time"; log 276), `wire-moves=100`:
    // each case with a time moved partway, both ways, at two times.
    // `wire-moves-each=100`, a search for faults: each unit alone
    let mut wmoves = (Cuts::default(), Wires::default());
    let moves_asked = wire_asked.as_ref().filter(|v| v.starts_with("wire-moves")).map(|v| v.split('=').nth(1).and_then(|n| n.parse::<i64>().ok()).unwrap_or(0) * 1000);
    if let Some(d) = moves_asked {
        for sdir in &stores {
            test_wire_moves(sdir, &policy, backend, level, &mut report, &mut wmoves.0, &mut wmoves.1, d, wire_asked.as_ref().is_some_and(|v| v.starts_with("wire-moves-each")));
        }
    }
    if let Some(v) = wire_asked.as_ref().filter(|_| moves_asked.is_none()) {
        let d = v.split('=').nth(1).and_then(|n| n.parse::<i64>().ok()).unwrap_or(0) * 1000;
        for sdir in &stores {
            test_wire(sdir, &policy, backend, level, &mut report, &mut wires, d, v.starts_with("wire-timed"));
        }
    }
    // ... and the cut chosen (fm3 cut.md, "the cut chosen"; log 290),
    // `choose=250`: every case of every store that says a time,
    // everything on the server, the runner choosing, under five wires
    let choose_asked = std::env::var("PROBE_ZERO_CUTS").ok().filter(|v| v.starts_with("choose")).map(|v| v.split('=').nth(1).and_then(|n| n.parse::<i64>().ok()).unwrap_or(250) * 1000);
    let mut chosen = Chosen::default();
    // (`choose52=250`: the chooser as hop fifty-two had it, neither the
    // speed nor the wire's history weighed, fm3 log 296: a measurement
    // to compare with on the new wires)
    let but = if std::env::var("PROBE_ZERO_CUTS").is_ok_and(|v| v.starts_with("choose52")) { 1024 | 2048 } else { 0 };
    if let Some(b) = choose_asked {
        for sdir in &stores {
            test_choose(sdir, &policy, backend, level, &mut report, &mut chosen, b, but);
        }
        report.log.push_str(&format!("chosen, a bound of {}, everything on the server{}: {} stores with a time, {} built, {} refused by name\n", spell_late(b), if but != 0 { ", neither the speed nor the wire's history weighed" } else { "" }, chosen.stores, chosen.built, chosen.refused));
        for (k, w) in choose_wires().iter().enumerate() {
            report.log.push_str(&format!("  a wire of {}: {} runs, {} give the case's own line, {} stop short where the wire went for good; {} of {} pieces for {} or after late, the most {}{}; {} runs chose nothing on the server, {} gave the server its part back; {} moves\n", w.said(), chosen.runs[k], chosen.held[k], chosen.short[k], chosen.late[k], chosen.after[k], store::spell_time(changed_from(w)), spell_late(chosen.most[k]), chosen.worst[k], chosen.away[k], chosen.back[k], chosen.moves[k]));
        }
    }
    if std::env::var("PROBE_ZERO_CUTS").is_ok() && wire_asked.is_none() && choose_asked.is_none() {
        // (`PROBE_ZERO_CUTS=moves`: the moves alone)
        for sdir in stores.iter().filter(|_| !std::env::var("PROBE_ZERO_CUTS").is_ok_and(|v| v.starts_with("moves"))) {
            test_cuts(sdir, &policy, backend, level, &mut report, &mut cuts);
        }
        // ... and each case with a time moved partway (fm3 cut.md,
        // "the cut, moved"), from the build whose cut may be moved
        for sdir in &stores {
            test_moves(sdir, &policy, backend, level, &mut report, &mut cuts);
        }
    }
    // the last line counts runs: every case in every context it stands
    // in (log 44), the expected-IR checks among the cases
    report.log.push_str(&format!(
        "\n{}/{} runs passed ({} cases over their contexts, {} overridden){}\n",
        report.passed,
        report.passed + report.failed,
        ncases,
        nover,
        if report.skipped > 0 { format!(", {} skipped", report.skipped) } else { String::new() }
    ));
    if cuts.stores > 0 {
        report.log.push_str(&format!(
            "cuts: {} of {} stores hold at every cut; {} hold at every cut that is not refused; {} have a cut that does not hold; {} could not be placed ({} cuts run, {} refused; {} of {} runs at a cut hold)\n",
            cuts.hold, cuts.stores, cuts.hold_but, cuts.broken, cuts.unplaced, cuts.cuts, cuts.refused, cuts.passed, cuts.passed + cuts.failed
        ));
        report.passed += cuts.passed;
        report.failed += cuts.failed;
    }
    if wires.stores > 0 {
        report.log.push_str(&format!(
            "wire: {} of {} stores hold at every cut; {} hold at every cut that is not refused; {} have a cut that does not hold; {} are not built for a wire ({} cuts run, {} refused; {} of {} runs at a cut hold by their own times; {} of {} cases in their contexts hold at every cut tried)\n",
            wires.hold, wires.stores, wires.hold_but, wires.broken, wires.unbuilt, wires.cuts, wires.refused, wires.passed, wires.passed + wires.failed, wires.case_hold, wires.cases
        ));
        report.log.push_str(&format!("late: {} of {} cuts show nothing late; the most a piece is late at any cut is {}; {} of {} pieces of text are shown late\n", wires.cuts_on_time, wires.cuts, spell_late(wires.most), wires.late, wires.pieces));
        if wires.ahead_cuts > 0 {
            report.log.push_str(&format!("ahead: at {} cuts of stores with an input that arrives at a time the server works ahead of it; {} turns it took were thrown away\n", wires.ahead_cuts, wires.thrown));
        }
        report.passed += wires.passed;
        report.failed += wires.failed;
    }
    if let (Some(d), (cuts, wires)) = (moves_asked, &wmoves) {
        report.log.push_str(&format!(
            "moves over a wire of {}: {} of {} cases with a time hold by their own times at every move tried; {} hold at every move that is not refused; {} have a move that does not hold; in {} stores, {} of which are not built to move over a wire ({} moves run, {} refused; {} of {} runs of a move hold)\n",
            spell_late(d), cuts.case_hold, cuts.timed, cuts.case_hold_but, cuts.case_broken, cuts.moved_stores, cuts.unmoved, cuts.moves, cuts.moves_refused, cuts.moves_passed, cuts.moves_passed + cuts.moves_failed
        ));
        report.log.push_str(&format!("late: {} of {} moves show nothing late; the most a piece is late at any move is {}; {} of {} pieces of text are shown late\n", wires.cuts_on_time, cuts.moves, spell_late(wires.most), wires.late, wires.pieces));
        if wires.ahead_cuts > 0 {
            report.log.push_str(&format!("ahead: in {} moves of stores with an input that arrives at a time the server works ahead of it; {} turns it took were thrown away\n", wires.ahead_cuts, wires.thrown));
        }
        report.passed += cuts.moves_passed;
        report.failed += cuts.moves_failed;
    }
    if cuts.timed > 0 {
        report.log.push_str(&format!(
            "moves: {} of {} cases with a time hold at every move tried; {} hold at every move that is not refused; {} have a move that does not hold; in {} stores, {} of which could not be built to move ({} moves run, {} refused; {} of {} runs of a move hold)\n",
            cuts.case_hold, cuts.timed, cuts.case_hold_but, cuts.case_broken, cuts.moved_stores, cuts.unmoved, cuts.moves, cuts.moves_refused, cuts.moves_passed, cuts.moves_passed + cuts.moves_failed
        ));
        report.passed += cuts.moves_passed;
        report.failed += cuts.moves_failed;
    }
    Ok(report)
}

/// the wires the suite chooses the cut under (fm3 log 290, the hop's
/// prompt): a steady 100 ms; 2 s from 1 s; gone from 1 s; gone from
/// 1 s and back at 3 s; and 3 s for one second
///
/// ... and two more (fm3 log 296): a narrow wire, 1 000 bytes a second,
/// on which a ping still comes back within the bound but a store's
/// start does not fit; and a wire gone 500 ms every 2 s from 1 s
fn choose_wires() -> Vec<Wire> {
    let w = |then: Vec<(i64, i64)>| Wire { d: 100_000, then, ..Default::default() };
    vec![w(vec![]), w(vec![(1_000_000, 2_000_000)]), w(vec![(1_000_000, GONE)]), w(vec![(1_000_000, GONE), (3_000_000, 100_000)]), w(vec![(1_000_000, 3_000_000), (2_000_000, 100_000)]), Wire { speed: NARROW, ..w(vec![]) }, Wire { every: Some((500_000, 2_000_000, 1_000_000)), ..w(vec![]) }]
}

/// the narrow wire of the suite's chooser, in bytes a second (fm3 log 296)
const NARROW: i64 = 1000;

/// from when a wire is not what it was at first: its first change, or
/// its first gap; 0 s for a wire narrow from the start
fn changed_from(w: &Wire) -> i64 {
    w.then.first().map(|t| t.0).or(w.every.map(|e| e.2)).unwrap_or(0)
}

/// what the suite with the cut chosen found, a row a wire
#[derive(Default)]
struct Chosen {
    stores: usize,
    built: usize,
    refused: usize,
    runs: [usize; 7],
    held: [usize; 7],
    short: [usize; 7],
    most: [i64; 7],
    /// ... the store and case of it
    worst: [String; 7],
    /// pieces for the change's time or after, and of them those late
    after: [usize; 7],
    late: [usize; 7],
    away: [usize; 7],
    back: [usize; 7],
    /// the moves made, both ways, over all runs (fm3 log 296)
    moves: [usize; 7],
}

/// One store with the cut chosen (fm3 log 290): every case, everything
/// the server, the runner choosing between that and nothing on the server, under each
/// of `choose_wires`. A measurement, not a gate: a run that does not
/// give the case's line is listed and counted, not failed
#[allow(clippy::too_many_arguments)]
fn test_choose(sdir: &Path, policy: &ssa::Policy, backend: Backend, level: usize, report: &mut Report, ch: &mut Chosen, bound: i64, but: i64) {
    let name = sdir.file_name().unwrap().to_string_lossy().to_string();
    let Ok(mut s) = wired(sdir) else { return };
    if !s.features.iter().any(|f| f.cases.iter().any(|c| matches!(c.expect, store::Expect::Timed(_)))) {
        return;
    }
    ch.stores += 1;
    s.moved = true;
    s.choose = true;
    s.changes = true;
    s.paced = true;
    let built = (|| -> Result<(Vec<Planned>, Vec<Run>, ssa::Module, lower::Lowered), String> {
        let policy = store_policy(&s, policy);
        let l = lower::lower(&s).map_err(|e| e.to_string())?;
        let cases = calls_of(&s, &l, &policy)?;
        let (runs, _) = plan(&s, &cases)?;
        let module = build(&l.ir, &policy, level)?;
        Ok((cases, runs, module, l))
    })();
    let (cases, runs, module, lowered) = match built {
        Ok(b) => b,
        Err(e) => {
            ch.refused += 1;
            report.log.push_str(&format!("chose {:<16} not built: {}\n", name, e.split(": ").skip(1).collect::<Vec<_>>().join(": ")));
            return;
        }
    };
    let Some(placed) = &lowered.placed else { return };
    let all = placed.all();
    let wires = choose_wires();
    let first = Wire { choose: Some(bound), ..wires[0].clone() };
    let cut = match chosen_cut(placed, Some("all"), &[], &first) {
        Ok(c) => c,
        Err(e) => {
            ch.refused += 1;
            report.log.push_str(&format!("chose {:<16} refused: {}\n", name, e));
            return;
        }
    };
    let _ = all;
    ch.built += 1;
    let unreached = out_of_reach(&module, &lowered.funcs, kind_of(backend));
    let runs: Vec<&Run> = runs.iter().filter(|r| !unreached.contains_key(&cases[r.case].call.func)).collect();
    for (k, w) in wires.iter().enumerate() {
        // (a steady wire in a build for one that changes is a change at the never-time)
        // (and every case's wire said whole in a build whose wire may have a speed, fm3 log 294)
        let mut changes = if w.then.is_empty() { vec![("__wire_at".to_string(), vec![0, GONE, w.d])] } else { w.then.iter().enumerate().map(|(k, (at, d))| ("__wire_at".to_string(), vec![k as i64, *at, *d])).collect() };
        changes.extend(w.pace_calls());
        changes.push(("__wire_but".to_string(), vec![but]));
        let mut scalls: [Vec<suite::Call>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for r in &runs {
            let c = &cases[r.case].call;
            for mode in 0..3 {
                scalls[mode as usize].push(suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: cut.iter().cloned().chain(changes.iter().cloned()).chain([("__wire_set".to_string(), vec![w.d]), ("__wire_mode".to_string(), vec![mode])]).chain(setters(&r.switches, &c.input, &c.input_at)).collect(), live: false, times: true });
            }
        }
        if runs.is_empty() {
            continue;
        }
        let from = changed_from(w);
        let got = [0, 1, 2].map(|mode| suite::run_calls(&module, &lowered.ir, backend, &scalls[mode], &format!("{}-choose", name), level));
        match got {
            [Ok(own), Ok(shown), Ok(logs)] => {
                for (((r, own), shown), log) in runs.iter().zip(own).zip(shown).zip(logs) {
                    ch.runs[k] += 1;
                    let log = log.map(|g| g.marks).unwrap_or_default();
                    ch.away[k] += usize::from(log.chunks(6).any(|c| c.len() == 6 && c[1] == 15));
                    ch.back[k] += usize::from(log.chunks(6).any(|c| c.len() == 6 && c[1] == 16));
                    ch.moves[k] += log.chunks(6).filter(|c| c.len() == 6 && (c[1] == 15 || c[1] == 16)).count();
                    let mut text = String::new();
                    if let (Ok(o), Ok(sh)) = (&own, &shown) {
                        let pieces = shown_pieces(&o.text, &o.marks, &sh.marks);
                        let late = pieces.iter().filter(|p| p.own >= from).map(|p| p.shown - p.own).max().unwrap_or(0);
                        ch.after[k] += pieces.iter().filter(|p| p.own >= from).count();
                        ch.late[k] += pieces.iter().filter(|p| p.own >= from && p.shown > p.own).count();
                        if late > ch.most[k] {
                            ch.most[k] = late;
                            ch.worst[k] = format!(" ({}, `{}`)", name, cases[r.case].text.split('→').next().unwrap_or("").trim());
                        }
                        text = o.text.clone();
                    }
                    let expect = &cases[r.case].call.expect;
                    if !never_said(w, expect, &text).is_empty() {
                        ch.short[k] += 1;
                    }
                    let (ok, note) = judge(expect, own);
                    if ok {
                        ch.held[k] += 1;
                    } else {
                        report.log.push_str(&format!("chose {:<16} {} [a wire of {}]   {}\n", name, labelled(&cases[r.case].text, &r.label), w.said(), note));
                    }
                }
            }
            [Err(e), _, _] | [_, Err(e), _] | [_, _, Err(e)] => {
                ch.runs[k] += runs.len();
                report.log.push_str(&format!("chose {:<16} [a wire of {}] {}\n", name, w.said(), e));
            }
        }
    }
}

/// what the suite at every cut found, over all its stores
#[derive(Default)]
struct Cuts {
    stores: usize,
    /// every cut of the store ran, and every case held at each
    hold: usize,
    /// ... or held at every cut that was not refused
    hold_but: usize,
    broken: usize,
    unplaced: usize,
    cuts: usize,
    refused: usize,
    passed: usize,
    failed: usize,
    /// the cut moved while a case runs: the stores that have a case
    /// with a time, and those of them with no build that moves
    moved_stores: usize,
    unmoved: usize,
    /// such cases, a run in each context: those that hold at every
    /// move tried, at every move not refused, and those with a move
    /// that runs and gives something else
    timed: usize,
    case_hold: usize,
    case_hold_but: usize,
    case_broken: usize,
    /// a move is one case in one context, one direction, one time
    moves: usize,
    moves_refused: usize,
    moves_passed: usize,
    moves_failed: usize,
}

/// what the suite over a wire that takes time found, over its stores
#[derive(Default)]
pub(super) struct Wires {
    pub(super) stores: usize,
    pub(super) hold: usize,
    pub(super) hold_but: usize,
    pub(super) broken: usize,
    /// the stores the rewrite does not hold, each refused by name
    pub(super) unbuilt: usize,
    pub(super) cuts: usize,
    pub(super) refused: usize,
    pub(super) passed: usize,
    pub(super) failed: usize,
    /// a case in a context: how many there are in the stores built,
    /// and how many give their own line at every cut tried
    pub(super) cases: usize,
    pub(super) case_hold: usize,
    /// the cuts at which no piece of any case is shown late, and the
    /// most any piece is
    pub(super) cuts_on_time: usize,
    pub(super) most: i64,
    /// the pieces of text shown at every cut run, and those of them
    /// shown late
    pub(super) pieces: usize,
    pub(super) late: usize,
    /// where the server works ahead of a key (fm3 log 273): the cuts of such
    /// stores, and the turns the server took at them and threw away
    pub(super) ahead_cuts: usize,
    pub(super) thrown: i64,
}

/// One store over a wire that takes time (fm3 cut.md, "a wire that
/// takes time"; log 269): its build for a wire, compiled once, and
/// each case in each of its contexts at every cut the runner tries,
/// every unit on the server and each alone, with a wire of `d`. A case is
/// compared by its own times; when each piece was shown is read back
/// by a second run and its lateness counted. A store or a cut the
/// build does not hold is refused by name, passed over and counted
pub(super) fn test_wire(sdir: &Path, policy: &ssa::Policy, backend: Backend, level: usize, report: &mut Report, wires: &mut Wires, d: i64, timed_only: bool) {
    let name = sdir.file_name().unwrap().to_string_lossy().to_string();
    let Ok(s) = wired(sdir) else { return };
    if timed_only && !s.features.iter().any(|f| f.cases.iter().any(|c| matches!(c.expect, store::Expect::Timed(_)))) {
        return;
    }
    wires.stores += 1;
    let built = (|| -> Result<(Vec<Planned>, Vec<Run>, ssa::Module, lower::Lowered), String> {
        let policy = store_policy(&s, policy);
        let l = lower::lower(&s).map_err(|e| e.to_string())?;
        let cases = calls_of(&s, &l, &policy)?;
        let (runs, _) = plan(&s, &cases)?;
        let module = build(&l.ir, &policy, level)?;
        Ok((cases, runs, module, l))
    })();
    let (cases, runs, module, lowered) = match built {
        Ok(b) => b,
        Err(e) => {
            wires.unbuilt += 1;
            report.log.push_str(&format!("wire  {:<16} not built: {}\n", name, e.split(": ").skip(1).collect::<Vec<_>>().join(": ")));
            return;
        }
    };
    let Some(placed) = &lowered.placed else { return };
    let unreached = out_of_reach(&module, &lowered.funcs, kind_of(backend));
    let runs: Vec<&Run> = runs.iter().filter(|r| !unreached.contains_key(&cases[r.case].call.func)).collect();
    let (mut refused, mut failed, mut ran) = (0, 0, 0);
    let (mut most, mut on_time) = (0i64, 0);
    let mut bad = vec![false; runs.len()];
    for server in super::places::cuts(placed) {
        let label = placed.label(&server);
        if let Some(why) = placed.refused(&server) {
            refused += 1;
            wires.refused += 1;
            report.log.push_str(&format!("wire  {:<16} [{}] refused: {}\n", name, label, why));
            continue;
        }
        wires.cuts += 1;
        ran += 1;
        let cut = placed.cut(&server);
        let call = |r: &Run, mode: i64| {
            let c = &cases[r.case].call;
            suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: cut.before.iter().cloned().chain([("__wire_set".to_string(), vec![d]), ("__wire_mode".to_string(), vec![mode])]).chain(setters(&r.switches, &c.input, &c.input_at)).collect(), live: false, times: true }
        };
        if runs.is_empty() {
            continue;
        }
        // (two batches, the second to read back when each piece was
        // shown: node's argument list holds only so many calls)
        let both = [0, 1].map(|mode| suite::run_calls(&module, &lowered.ir, backend, &runs.iter().map(|r| call(r, mode)).collect::<Vec<_>>(), &format!("{}-wire", name), level));
        let got = match both {
            [Ok(own), Ok(shown)] => Ok(own.into_iter().zip(shown).flat_map(|(o, s)| [o, s]).collect::<Vec<_>>()),
            [Err(e), _] | [_, Err(e)] => Err(e),
        };
        let got: Result<Vec<Result<suite::Got, String>>, String> = got;
        let got = match got {
            Ok(g) => g,
            Err(e) => {
                failed += 1;
                wires.failed += 1;
                bad.iter_mut().for_each(|b| *b = true);
                report.log.push_str(&format!("FAIL  {:<16} [{}] {}\n", name, label, e));
                continue;
            }
        };
        let mut late = 0i64;
        let mut got = got.into_iter();
        for (i, r) in runs.iter().enumerate() {
            let text = format!("{} [{}, a wire of {}]", labelled(&cases[r.case].text, &r.label), label, spell_late(d));
            let (Some(own), shown) = (got.next(), got.next()) else { break };
            if own.as_ref().err().is_some_and(|e| e.starts_with("skip: ")) {
                continue;
            }
            if let (Ok(o), Some(Ok(sh))) = (&own, &shown) {
                let pieces = shown_pieces(&o.text, &o.marks, &sh.marks);
                late = late.max(pieces.iter().map(|p| p.shown - p.own).max().unwrap_or(0));
                wires.pieces += pieces.len();
                wires.late += pieces.iter().filter(|p| p.shown > p.own).count();
                if works_ahead(&lowered.ir) {
                    wires.thrown += sh.marks.last().copied().unwrap_or(0);
                }
            }
            // a check that fails is traced to its line as any is, by
            // running the case again at one place (fm3 question 115)
            let own = match own {
                Err(e) if untraced(&e) && cases[r.case].call.expect != store::Expect::Check => {
                    let c = &cases[r.case].call;
                    // (the build for a wire has a list whatever the store has, and so a twin for every case's function: the store as it is may have none)
                    let plain = |func: &str| suite::Call { func: func.to_string(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: setters(&r.switches, &c.input, &c.input_at), live: false, times: matches!(c.expect, store::Expect::Timed(_)) };
                    Err(traced(sdir, &store_policy(&s, policy), backend, level, &plain(&c.func), &name).or_else(|| traced(sdir, &store_policy(&s, policy), backend, level, &plain(c.func.trim_start_matches("__whole_")), &name)).unwrap_or(e))
                }
                g => g,
            };
            // (a case that gives no times is compared as it is: its marks are asked for only to say what was late)
            let own = own.map(|mut g| {
                if !matches!(cases[r.case].call.expect, store::Expect::Timed(_)) {
                    g.marks.clear();
                }
                g
            });
            let (ok, note) = judge(&cases[r.case].call.expect, own);
            if ok {
                wires.passed += 1;
            } else {
                failed += 1;
                wires.failed += 1;
                bad[i] = true;
                report.log.push_str(&format!("FAIL  {:<16} {}   {}\n", name, text, note));
            }
        }
        wires.most = wires.most.max(late);
        most = most.max(late);
        if works_ahead(&lowered.ir) {
            wires.ahead_cuts += 1;
        }
        if late == 0 {
            wires.cuts_on_time += 1;
            on_time += 1;
        }
    }
    wires.cases += runs.len();
    wires.case_hold += bad.iter().filter(|b| !**b).count();
    match (failed, refused) {
        (0, 0) => wires.hold += 1,
        (0, _) => wires.hold_but += 1,
        _ => wires.broken += 1,
    }
    report.log.push_str(&format!("wires {:<16} {} units, {} cuts run{}{}\n", name, placed.units.len(), ran, if refused > 0 { format!(", {} refused", refused) } else { String::new() }, if failed > 0 { format!(", {} runs do not hold", failed) } else { format!(": every case holds at each by its own times; nothing late at {}, the most {}", on_time, spell_late(most)) }));
}

/// The two times a case with a time is moved at (fm3 log 265): one
/// when a piece of its result is due, the piece nearest the middle of
/// the case, and one halfway from the piece before to that, when
/// nothing of the result is. None where the result has no time after
/// its start
pub(super) fn move_times(expect: &store::Expect) -> Option<(i64, i64)> {
    let store::Expect::Timed(pieces) = expect else { return None };
    let mut times: Vec<i64> = pieces.iter().map(|(_, t)| *t).collect();
    times.sort();
    times.dedup();
    let last = *times.last()?;
    if last <= 0 {
        return None;
    }
    let due = times.iter().copied().filter(|t| *t > 0).min_by_key(|t| ((2 * t - last).abs(), -*t))?;
    let before = times.iter().copied().filter(|t| *t < due).max().unwrap_or(0);
    Some(((before + due) / 2, due))
}

/// One store's cases with a time, each moved partway (fm3 touchstones.md,
/// "hello, in two places", step 2; cut.md, "the cut, moved"): from the
/// build whose cut may be moved, compiled once, everything on the server and
/// then everything at the client, and the other way, at the two times
/// `move_times` gives, and compared with the case's own line. A move
/// that cannot be made is refused by name, passed over and counted
fn test_moves(sdir: &Path, policy: &ssa::Policy, backend: Backend, level: usize, report: &mut Report, cuts: &mut Cuts) {
    let name = sdir.file_name().unwrap().to_string_lossy().to_string();
    let Ok(mut s) = store::read(sdir) else { return };
    if !s.features.iter().any(|f| f.cases.iter().any(|c| move_times(&c.expect).is_some())) {
        return;
    }
    cuts.moved_stores += 1;
    s.clock = store::Clock::Virtual;
    s.places = true;
    s.moved = true;
    let built = (|| -> Result<(Vec<Planned>, Vec<Run>, ssa::Module, lower::Lowered), String> {
        let policy = store_policy(&s, policy);
        let l = lower::lower(&s).map_err(|e| e.to_string())?;
        let cases = calls_of(&s, &l, &policy)?;
        let (runs, _) = plan(&s, &cases)?;
        let module = build(&l.ir, &policy, level)?;
        Ok((cases, runs, module, l))
    })();
    let (cases, runs, module, lowered) = match built {
        Ok(b) => b,
        Err(e) => {
            cuts.unmoved += 1;
            report.log.push_str(&format!("move  {:<16} could not be built to move: {}\n", name, e));
            return;
        }
    };
    let Some(placed) = &lowered.placed else { return };
    let unreached = out_of_reach(&module, &lowered.funcs, kind_of(backend));
    let runs: Vec<(&Run, (i64, i64))> = runs.iter().filter(|r| !unreached.contains_key(&cases[r.case].call.func)).filter_map(|r| move_times(&cases[r.case].call.expect).map(|t| (r, t))).collect();
    let (none, all) = (BTreeSet::new(), placed.all());
    let mut ways = vec![(all.clone(), none.clone()), (none.clone(), all.clone())];
    // (`PROBE_ZERO_CUTS=moves-each`, a search for faults and no part
    // of the count a handover gives: each unit alone moved, to the server and
    // back, from among the others at the client and from among them on the server)
    if std::env::var("PROBE_ZERO_CUTS").as_deref() == Ok("moves-each") {
        for u in 0..placed.units.len() {
            let one: BTreeSet<usize> = [u].into_iter().collect();
            let rest: BTreeSet<usize> = all.iter().copied().filter(|v| *v != u).collect();
            ways.extend([(one.clone(), none.clone()), (none.clone(), one), (all.clone(), rest.clone()), (rest, all.clone())]);
        }
    }
    let refused: Vec<Option<String>> = ways.iter().map(|(a, b)| placed.move_refused(a, b)).collect();
    for ((a, b), why) in ways.iter().zip(&refused) {
        if let Some(why) = why {
            report.log.push_str(&format!("move  {:<16} [{}, then {}] refused: {}\n", name, placed.label(a), placed.label(b).trim_start_matches("server: "), why));
        }
    }
    // every move of every case, one batch
    let mut scalls: Vec<suite::Call> = Vec::new();
    let mut said: Vec<(usize, String)> = Vec::new();
    for (i, (r, (between, due))) in runs.iter().enumerate() {
        let c = &cases[r.case].call;
        for ((a, b), why) in ways.iter().zip(&refused) {
            for t in [*between, *due] {
                if why.is_some() {
                    cuts.moves_refused += 1;
                    continue;
                }
                let cut = placed.moves(a, &[(b.clone(), t)]);
                said.push((i, cut.label));
                scalls.push(suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: cut.before.into_iter().chain(setters(&r.switches, &c.input, &c.input_at)).collect(), live: false, times: true });
            }
        }
    }
    cuts.timed += runs.len();
    cuts.moves += scalls.len();
    let mut failed = vec![0usize; runs.len()];
    let mut total = 0;
    if !scalls.is_empty() {
        match suite::run_calls(&module, &lowered.ir, backend, &scalls, &format!("{}-move", name), level) {
            Ok(got) => {
                for ((i, label), got) in said.iter().zip(got) {
                    let r = runs[*i].0;
                    let (ok, note) = judge(&cases[r.case].call.expect, got);
                    if ok {
                        cuts.moves_passed += 1;
                    } else {
                        failed[*i] += 1;
                        total += 1;
                        cuts.moves_failed += 1;
                        report.log.push_str(&format!("FAIL  {:<16} {} [{}]   {}\n", name, labelled(&cases[r.case].text, &r.label), label, note));
                    }
                }
            }
            Err(e) => {
                total = scalls.len();
                cuts.moves_failed += scalls.len();
                failed.iter_mut().for_each(|f| *f += 1);
                report.log.push_str(&format!("FAIL  {:<16} [moved] {}\n", name, e));
            }
        }
    }
    let any_refused = refused.iter().any(Option::is_some);
    for f in &failed {
        match (*f, any_refused) {
            (0, false) => cuts.case_hold += 1,
            (0, true) => cuts.case_hold_but += 1,
            _ => cuts.case_broken += 1,
        }
    }
    report.log.push_str(&format!("moves {:<16} {} cases with a time, {} moves run{}{}\n", name, runs.len(), scalls.len(), if any_refused { format!(", {} of {} ways refused", refused.iter().flatten().count(), ways.len()) } else { String::new() }, if total > 0 { format!(", {} runs do not hold", total) } else if scalls.is_empty() { String::new() } else { ": every case holds at each".to_string() }));
}

/// One store's cases with a time, each moved partway over a wire that
/// takes time (fm3 cut.md, "the cut moved over a wire that takes time";
/// log 276): from the build for a wire whose cut may be moved, compiled
/// once, everything on the server and then everything at the client, and the
/// other way, at the two times `move_times` gives, with a wire of `d`.
/// A case is compared by its own times; when each piece was shown is
/// read back by a second run and its lateness counted. A store the
/// build does not hold, and a move that cannot be made, is refused by
/// name, passed over and counted
#[allow(clippy::too_many_arguments)]
fn test_wire_moves(sdir: &Path, policy: &ssa::Policy, backend: Backend, level: usize, report: &mut Report, cuts: &mut Cuts, wires: &mut Wires, d: i64, each: bool) {
    let name = sdir.file_name().unwrap().to_string_lossy().to_string();
    let Ok(mut s) = wired(sdir) else { return };
    if !s.features.iter().any(|f| f.cases.iter().any(|c| move_times(&c.expect).is_some())) {
        return;
    }
    cuts.moved_stores += 1;
    s.moved = true;
    let built = (|| -> Result<(Vec<Planned>, Vec<Run>, ssa::Module, lower::Lowered), String> {
        let policy = store_policy(&s, policy);
        let l = lower::lower(&s).map_err(|e| e.to_string())?;
        let cases = calls_of(&s, &l, &policy)?;
        let (runs, _) = plan(&s, &cases)?;
        let module = build(&l.ir, &policy, level)?;
        Ok((cases, runs, module, l))
    })();
    let (cases, runs, module, lowered) = match built {
        Ok(b) => b,
        Err(e) => {
            cuts.unmoved += 1;
            report.log.push_str(&format!("wmove {:<16} not built: {}\n", name, e.split(": ").skip(1).collect::<Vec<_>>().join(": ")));
            return;
        }
    };
    let Some(placed) = &lowered.placed else { return };
    let unreached = out_of_reach(&module, &lowered.funcs, kind_of(backend));
    let runs: Vec<(&Run, (i64, i64))> = runs.iter().filter(|r| !unreached.contains_key(&cases[r.case].call.func)).filter_map(|r| move_times(&cases[r.case].call.expect).map(|t| (r, t))).collect();
    let (none, all) = (BTreeSet::new(), placed.all());
    let mut ways = vec![(all.clone(), none.clone()), (none.clone(), all.clone())];
    if each {
        for u in 0..placed.units.len() {
            let one: BTreeSet<usize> = [u].into_iter().collect();
            let rest: BTreeSet<usize> = all.iter().copied().filter(|v| *v != u).collect();
            ways.extend([(one.clone(), none.clone()), (none.clone(), one), (all.clone(), rest.clone()), (rest, all.clone())]);
        }
    }
    let refused: Vec<Option<String>> = ways.iter().map(|(a, b)| placed.move_refused(a, b)).collect();
    for ((a, b), why) in ways.iter().zip(&refused) {
        if let Some(why) = why {
            report.log.push_str(&format!("wmove {:<16} [{}, then {}] refused: {}\n", name, placed.label(a), placed.label(b).trim_start_matches("server: "), why));
        }
    }
    // every move of every case: a batch for what each gave by its own
    // times, and one for when each piece was shown
    let mut scalls: [Vec<suite::Call>; 2] = [Vec::new(), Vec::new()];
    let mut said: Vec<(usize, String)> = Vec::new();
    for (i, (r, (between, due))) in runs.iter().enumerate() {
        let c = &cases[r.case].call;
        for ((a, b), why) in ways.iter().zip(&refused) {
            for t in [*between, *due] {
                if why.is_some() {
                    cuts.moves_refused += 1;
                    continue;
                }
                let cut = placed.moves(a, &[(b.clone(), t)]);
                said.push((i, format!("{}, a wire of {}", cut.label, spell_late(d))));
                for mode in 0..2 {
                    scalls[mode as usize].push(suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: cut.before.iter().cloned().chain([("__wire_set".to_string(), vec![d]), ("__wire_mode".to_string(), vec![mode])]).chain(setters(&r.switches, &c.input, &c.input_at)).collect(), live: false, times: true });
                }
            }
        }
    }
    cuts.timed += runs.len();
    cuts.moves += said.len();
    let mut failed = vec![0usize; runs.len()];
    let mut total = 0;
    let (mut most, mut on_time) = (0i64, 0);
    if !said.is_empty() {
        let both = [0, 1].map(|mode| suite::run_calls(&module, &lowered.ir, backend, &scalls[mode], &format!("{}-wmove", name), level));
        match both {
            [Ok(own), Ok(shown)] => {
                for (((i, label), own), shown) in said.iter().zip(own).zip(shown) {
                    let r = runs[*i].0;
                    let mut late = 0i64;
                    if let (Ok(o), Ok(sh)) = (&own, &shown) {
                        let pieces = shown_pieces(&o.text, &o.marks, &sh.marks);
                        late = pieces.iter().map(|p| p.shown - p.own).max().unwrap_or(0);
                        wires.pieces += pieces.len();
                        wires.late += pieces.iter().filter(|p| p.shown > p.own).count();
                        if works_ahead(&lowered.ir) {
                            wires.thrown += sh.marks.last().copied().unwrap_or(0);
                            wires.ahead_cuts += 1;
                        }
                    }
                    most = most.max(late);
                    if late == 0 {
                        on_time += 1;
                    }
                    let (ok, note) = judge(&cases[r.case].call.expect, own);
                    if ok {
                        cuts.moves_passed += 1;
                    } else {
                        failed[*i] += 1;
                        total += 1;
                        cuts.moves_failed += 1;
                        report.log.push_str(&format!("FAIL  {:<16} {} [{}]   {}\n", name, labelled(&cases[r.case].text, &r.label), label, note));
                    }
                }
            }
            [Err(e), _] | [_, Err(e)] => {
                total = said.len();
                cuts.moves_failed += said.len();
                failed.iter_mut().for_each(|f| *f += 1);
                report.log.push_str(&format!("FAIL  {:<16} [moved over a wire] {}\n", name, e));
            }
        }
    }
    wires.most = wires.most.max(most);
    wires.cuts_on_time += on_time;
    let any_refused = refused.iter().any(Option::is_some);
    for f in &failed {
        match (*f, any_refused) {
            (0, false) => cuts.case_hold += 1,
            (0, true) => cuts.case_hold_but += 1,
            _ => cuts.case_broken += 1,
        }
    }
    report.log.push_str(&format!("wmoves {:<15} {} cases with a time, {} moves run{}{}\n", name, runs.len(), said.len(), if any_refused { format!(", {} of {} ways refused", refused.iter().flatten().count(), ways.len()) } else { String::new() }, if total > 0 { format!(", {} runs do not hold", total) } else if said.is_empty() { String::new() } else { format!(": every case holds at each by its own times; nothing late at {}, the most {}", on_time, spell_late(most)) }));
}

/// One store at every cut the runner tries (fm3 touchstones.md, "hello,
/// in two places", step 1): the store's placed build, compiled once,
/// and each case in each of its contexts run with every unit on the server and
/// with each unit alone on the server, the cut being the calls that fill its
/// table before the case. A cut that would carry an address is refused
/// by name, passed over and counted (fm3 question 152)
fn test_cuts(sdir: &Path, policy: &ssa::Policy, backend: Backend, level: usize, report: &mut Report, cuts: &mut Cuts) {
    let name = sdir.file_name().unwrap().to_string_lossy().to_string();
    cuts.stores += 1;
    let built = (|| -> Result<(Vec<Planned>, Vec<Run>, ssa::Module, lower::Lowered, ssa::Policy), String> {
        let mut s = store::read(sdir).map_err(|e| e.to_string())?;
        s.clock = store::Clock::Virtual;
        s.places = true;
        let policy = store_policy(&s, policy);
        let l = lower::lower(&s).map_err(|e| e.to_string())?;
        let cases = calls_of(&s, &l, &policy)?;
        let (runs, _) = plan(&s, &cases)?;
        let module = build(&l.ir, &policy, level)?;
        Ok((cases, runs, module, l, policy))
    })();
    let (cases, runs, module, lowered, policy) = match built {
        Ok(b) => b,
        Err(e) => {
            cuts.unplaced += 1;
            report.log.push_str(&format!("cut   {:<16} could not be placed: {}\n", name, e));
            return;
        }
    };
    let Some(placed) = &lowered.placed else { return };
    let unreached = out_of_reach(&module, &lowered.funcs, kind_of(backend));
    let runs: Vec<&Run> = runs.iter().filter(|r| !unreached.contains_key(&cases[r.case].call.func)).collect();
    let (mut refused, mut failed, mut ran) = (0, 0, 0);
    for server in super::places::cuts(placed) {
        let label = placed.label(&server);
        if let Some(why) = placed.refused(&server) {
            refused += 1;
            cuts.refused += 1;
            report.log.push_str(&format!("cut   {:<16} [{}] refused: {}\n", name, label, why));
            continue;
        }
        cuts.cuts += 1;
        ran += 1;
        let cut = placed.cut(&server);
        let label = cut.label.clone();
        let scalls: Vec<suite::Call> = runs
            .iter()
            .map(|r| {
                let c = &cases[r.case].call;
                suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: checks(&c.expect), text: true, before: cut.before.iter().cloned().chain(setters(&r.switches, &c.input, &c.input_at)).collect(), live: false, times: matches!(c.expect, store::Expect::Timed(_)) }
            })
            .collect();
        if scalls.is_empty() {
            continue;
        }
        let got = match suite::run_calls(&module, &lowered.ir, backend, &scalls, &format!("{}-cut", name), level) {
            Ok(g) => g,
            Err(e) => {
                failed += 1;
                cuts.failed += 1;
                report.log.push_str(&format!("FAIL  {:<16} [{}] {}\n", name, label, e));
                continue;
            }
        };
        for ((r, got), sc) in runs.iter().zip(got).zip(&scalls) {
            let text = format!("{} [{}]", labelled(&cases[r.case].text, &r.label), label);
            // a check that fails on the server is traced to its line as any is,
            // by running the case again at one place (fm3 question 115)
            let got = match got {
                Err(e) if untraced(&e) && cases[r.case].call.expect != store::Expect::Check => {
                    let plain = suite::Call { func: sc.func.clone(), args: sc.args.clone(), nrets: sc.nrets, checks: sc.checks, text: sc.text, before: sc.before[cut.before.len()..].to_vec(), live: false, times: sc.times };
                    Err(traced(sdir, &policy, backend, level, &plain, &name).unwrap_or(e))
                }
                g => g,
            };
            if got.as_ref().err().is_some_and(|e| e.starts_with("skip: ")) {
                continue;
            }
            let (ok, note) = judge(&cases[r.case].call.expect, got);
            if ok {
                cuts.passed += 1;
            } else {
                failed += 1;
                cuts.failed += 1;
                report.log.push_str(&format!("FAIL  {:<16} {}   {}\n", name, text, note));
            }
        }
    }
    match (failed, refused) {
        (0, 0) => cuts.hold += 1,
        (0, _) => cuts.hold_but += 1,
        _ => cuts.broken += 1,
    }
    report.log.push_str(&format!("cuts  {:<16} {} units, {} cuts run{}{}\n", name, placed.units.len(), ran, if refused > 0 { format!(", {} refused", refused) } else { String::new() }, if failed > 0 { format!(", {} runs do not hold", failed) } else { ": every case holds at each".to_string() }));
}

/// does the case expect a failed check?
fn checks(expect: &store::Expect) -> bool {
    matches!(expect, store::Expect::Check | store::Expect::CheckAt(_))
}

/// a stop that named no line: a failed check and nothing more, or a
/// trap the machine's own (wasm's remainder by zero)
fn untraced(e: &str) -> bool {
    // (... or named a line of the language's own, which is not the
    // line a person is told, fm3 log 228)
    e == suite::checked() || e.starts_with("trap:") || e.strip_prefix(suite::checked()).is_some_and(|r| r.trim().strip_prefix("at ").is_some_and(|at| at.starts_with(&format!("{}:", crate::host::OWN_SITE))))
}

/// What a case that returned gave, from a diagnostic build or a trace
/// build: the two words that read the text back say the site after it
/// wherever one is still stored, and one is after a run in which
/// something going on was ended partway, by a `restart`, and never put
/// back the site it found. That line is the build's and no part of what
/// the program wrote: it is taken off, and the marks after it
pub fn unsited(mut g: suite::Got) -> suite::Got {
    const LINE: usize = 62;
    let hex = |s: &str| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let at = g.text.len().saturating_sub(LINE);
    let tail = g.text.get(at..).unwrap_or("");
    let is_site = tail.strip_prefix("\ncheck at #").and_then(|r| r.strip_suffix('\n')).is_some_and(|r| r.split(',').count() == 3 && r.split(',').all(hex));
    if is_site {
        g.text.truncate(at);
        if !g.marks.is_empty() {
            g.marks.truncate(at + 1);
        }
    }
    g
}

/// The case that stopped, run again from the diagnostic build of the
/// same store (fm3 log 199, question 115): each statement there stores
/// its site before it runs, and the two words that read the output
/// back say it after the text, `check at #<site>,<a>,<b>`, which every
/// path's runner already looks for. What comes back is the line and
/// what was being asked, in the program's own names. The second run is
/// on the virtual clock. None where it did not stop again, or a path
/// could not say
fn traced(dir: &Path, policy: &ssa::Policy, backend: Backend, level: usize, call: &suite::Call, name: &str) -> Option<String> {
    let mut s = store::read(dir).ok()?;
    s.clock = store::Clock::Virtual;
    s.sites = true;
    let policy = store_policy(&s, policy);
    let l = lower::lower(&s).ok()?;
    let module = build(&l.ir, &policy, level).ok()?;
    let again = suite::Call { func: call.func.clone(), args: call.args.clone(), nrets: call.nrets, checks: true, text: true, before: call.before.clone(), live: false, times: false };
    let said = suite::run_calls(&module, &l.ir, backend, &[again], &format!("{}-sites", name), level).ok()?.remove(0).err()?;
    site_said(&l.sites, &said)
}

/// did the call give what the case expects? A text result is compared
/// with one trailing newline removed, so `>hi() → "hi"` matches one
/// `print "hi"`
fn judge(expect: &store::Expect, got: Result<suite::Got, String>) -> (bool, String) {
    match (expect, got) {
        (store::Expect::Check, Err(e)) if e.starts_with(suite::checked()) || e.starts_with("trap:") => (true, e.strip_prefix(suite::checked()).map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| format!("({})", s)).unwrap_or_default()),
        (store::Expect::Check, Err(e)) => (false, format!("({})", e)),
        (store::Expect::Check | store::Expect::CheckAt(_), Ok(g)) => (false, format!("(no check failed; got {})", show(&g.values))),
        // the check that fails is on the line the case says: what is
        // reported begins with it
        (store::Expect::CheckAt(site), Err(e)) => {
            let said = e.strip_prefix(suite::checked()).map(|s| s.trim()).unwrap_or("");
            let ok = said.strip_prefix("at ").is_some_and(|r| r == site || r.starts_with(&format!("{}:", site)));
            (ok, format!("({})", if ok { said } else { e.as_str() }))
        }
        (_, Err(e)) => (false, format!("({})", e)),
        (store::Expect::Values(want), Ok(g)) => {
            if &g.values == want {
                (true, String::new())
            } else {
                (false, format!("(got {})", show(&g.values)))
            }
        }
        // a time among the values: what came back for it is its
        // nanoseconds, and it is shown as the language writes a time
        (store::Expect::Said(want), Ok(g)) => {
            if g.values.len() == want.len() && g.values.iter().zip(want).all(|(v, (w, _))| v == w) {
                (true, String::new())
            } else {
                let got: Vec<String> = g.values.iter().enumerate().map(|(k, v)| if want.get(k).is_some_and(|w| w.1) { store::spell_nanos(*v) } else { v.to_string() }).collect();
                (false, format!("(got {})", got.join(", ")))
            }
        }
        (store::Expect::Text(want), Ok(g)) => {
            let text = g.text.strip_suffix('\n').unwrap_or(&g.text);
            if text == want {
                (true, String::new())
            } else {
                (false, format!("(printed {:?})", text))
            }
        }
        // what was written and when, in the case's own spelling, so the
        // line can be pasted back
        (store::Expect::Timed(want), Ok(g)) => {
            let got = pieces(&g);
            if &got == want {
                (true, String::new())
            } else {
                (false, format!("(printed {})", store::spell_timed(&got)))
            }
        }
    }
}

/// What a call wrote, cut at its marks (fm3 log 91, 95): there is a word
/// for each byte of the text and one for its end, and a word that is not
/// zero is the time the clock moved to when that many bytes had been
/// written. So the text before the first such word was written at 0 and
/// the text from one on at its time. A word that repeats the time
/// before it merges away, a stamp belonging to the characters (question
/// 53); and the last piece loses one trailing newline, as a plain text
/// result does
fn pieces(got: &suite::Got) -> Vec<(String, i64)> {
    let bytes = got.text.as_bytes();
    let mut out: Vec<(String, i64)> = Vec::new();
    let (mut from, mut now) = (0usize, 0i64);
    let cut = |from: usize, to: usize, t: i64, out: &mut Vec<(String, i64)>| {
        if to > from {
            out.push((String::from_utf8_lossy(&bytes[from..to]).to_string(), t));
        }
    };
    for (at, t) in got.marks.iter().enumerate().take(bytes.len() + 1) {
        if *t != 0 {
            cut(from, at, now, &mut out);
            (from, now) = (at, *t);
        }
    }
    cut(from, bytes.len(), now, &mut out);
    if let Some((last, _)) = out.last_mut() {
        if last.ends_with('\n') {
            last.pop();
        }
    }
    store::merge_timed(out)
}

fn show(vals: &[i64]) -> String {
    let v: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
    v.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// what a call wrote, cut at its marks (fm3 log 91, 95): the text
    /// before the first word that is not zero is at 0, a word that
    /// repeats a time merges away, and the last piece loses one newline
    #[test]
    fn the_text_is_cut_at_the_marks() {
        // a word for each byte and one for the end, zero but where given
        let got = |text: &str, marks: &[(usize, i64)]| {
            let mut words = vec![0i64; text.len() + 1];
            for (at, t) in marks {
                words[*at] = *t;
            }
            pieces(&suite::Got { values: vec![], text: text.into(), marks: words })
        };
        let p = |t: &str, us: i64| (t.to_string(), us);
        assert_eq!(got("10\n9\nhi\n", &[(3, 1_000_000)]), [p("10\n", 0), p("9\nhi", 1_000_000)]);
        // the clock moved before anything was written, then stood still
        assert_eq!(got("ab", &[(0, 2_000_000), (1, 2_000_000)]), [p("ab", 2_000_000)]);
        // nothing after the last mark, and a last piece that was only a newline
        assert_eq!(got("a\n", &[(2, 3_000_000)]), [p("a", 0)]);
        assert_eq!(got("a\n\n", &[(2, 3_000_000)]), [p("a\n", 0)]);
        assert_eq!(got("", &[(0, 1_000_000)]), []);
        assert_eq!(got("plain\n", &[]), [p("plain", 0)]);
        assert_eq!(store::spell_timed(&got("10\n9\n", &[(3, 1_000_000)])), "\"10\\n\" at 0 s, \"9\" at 1 s");
        // three lines a second apart are spelt at their rate (fm3 log 97)
        assert_eq!(store::spell_timed(&got("10\n9\n8\n", &[(3, 1_000_000), (5, 2_000_000)])), "\"10\\n9\\n8\" at 1 hz");
        // no marks read at all: the whole text at 0
        assert_eq!(pieces(&suite::Got { values: vec![], text: "x".into(), marks: vec![] }), [p("x", 0)]);
    }

    /// a host that wants the times of what any program writes asks the
    /// store for its marks (fm3 log 167): `__out_mark` is kept as it is
    /// for a store with a case that asserts on time, and nothing else of
    /// the lowered text is different but what keeping it keeps
    #[test]
    fn a_host_asks_a_store_for_its_marks() {
        let mut s = store::read(Path::new("suite/zero/hello")).unwrap();
        let timed = lower::lower(&s).unwrap().ir;
        assert!(timed.contains("\nfn __out_mark("), "hello has a case that asserts on time");
        s.times = true;
        assert_eq!(lower::lower(&s).unwrap().ir, timed);
        let mut s = store::read(Path::new("suite/zero/skeleton")).unwrap();
        assert!(!s.times);
        let plain = lower::lower(&s).unwrap().ir;
        assert!(!plain.contains("\nfn __out_mark("));
        s.times = true;
        let asked = lower::lower(&s).unwrap().ir;
        assert!(asked.contains("\nfn __out_mark(") && asked.contains("data __out_t:"));
        let wasm = suite::backend_policy(Backend::Wasm).unwrap();
        let module = build(&asked, &store_policy(&s, &wasm), opt::MAX_LEVEL).unwrap();
        // and the driver's spec for a call that reads them, from `host`
        let call = suite::Call { func: "answer".into(), args: vec![], nrets: 1, checks: false, text: true, before: vec![], live: false, times: true };
        assert_eq!(crate::host::wasm_cases(&module, &[call]).unwrap(), "{\"cases\":[{\"func\":\"answer\",\"reset\":true,\"text\":true,\"times\":true,\"before\":[],\"args\":[],\"rets\":[\"i32\"]}]}");
    }

    /// the contexts a store's cases run in and the overrides among them
    /// (log 44): hello's three `run()` cases are one promise per
    /// context, the newest feature's standing; a feature off takes its
    /// subtree with it; a line pinning `on` a feature the runner
    /// switches off does not run there
    #[test]
    fn every_case_in_every_context() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let s = store::read(Path::new("suite/zero/hello")).unwrap();
        let l = lower::lower(&s).unwrap();
        let cases = calls_of(&s, &l, &native).unwrap();
        let (runs, over) = plan(&s, &cases).unwrap();
        let labels: Vec<String> = contexts(&s).into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["", "with hello off", "with countdown off", "with bye off"]);
        // hello off takes countdown and bye with it: nothing stands there
        assert!(s.subtree("hello").contains(&"bye".to_string()));
        assert!(runs.iter().all(|r| r.label != "with hello off"));
        let standing = |label: &str| -> Vec<String> { runs.iter().filter(|r| r.label == label).map(|r| cases[r.case].text.clone()).collect() };
        assert_eq!(standing(""), ["hello() → \"hello world\"", "count down() → \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\"", "run() → \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s", "run() with countdown off → \"hello world\\ngoodbye\""]);
        assert_eq!(standing("with bye off"), ["hello() → \"hello world\"", "count down() → \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\"", "run() → \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nhello world\""]);
        assert_eq!(standing("with countdown off"), ["hello() → \"hello world\""]);
        assert_eq!(over.len(), 5);
        assert!(over.iter().any(|o| o.text == "run() → \"hello world\" [with bye off]" && o.why == "replaced by countdown's cases for run()"), "{:?}", over.iter().map(|o| &o.text).collect::<Vec<_>>());
        assert!(over.iter().any(|o| o.text == "run() → \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s [with countdown off]" && o.why == "the line `run() with countdown off → \"hello world\\ngoodbye\"` stands there"), "{:?}", over.iter().map(|o| &o.why).collect::<Vec<_>>());
        // `>existing` (log 50): more's cases for `describe (int)` fall
        // through to functions', so `describe (3)` stands beside `describe (4)`
        let s = store::read(Path::new("suite/zero/functions")).unwrap();
        assert!(s.features.iter().any(|f| f.name == "more" && f.existing_cases));
        let l = lower::lower(&s).unwrap();
        let cases = calls_of(&s, &l, &native).unwrap();
        let (runs, over) = plan(&s, &cases).unwrap();
        let plain: Vec<String> = runs.iter().filter(|r| r.label.is_empty()).map(|r| cases[r.case].text.clone()).collect();
        assert!(plain.contains(&"describe (3) → \"int\"".to_string()) && plain.contains(&"describe (4) → \"int\"".to_string()), "{:?}", plain);
        assert!(over.is_empty());
        // without it, more's cases for the method replace functions'
        let dir = std::env::temp_dir().join(format!("probe-zero-existing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let head = |name: &str, when: &str, parent: &str, existing: &str| format!("# {}\n*x*\n\n{}layer: runtime\n\n> (suite) 2026-09-08T10:0{}:00\n\n## testing\n{}", name, parent, when, existing);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("a/a.md"), head("a", "0", "", ">f (1) → 1\n>g() → 5\n")).unwrap();
        std::fs::write(dir.join("a/a.zero"), "on (int n) << f (int k)\n    n << k\n\non (int n) << g()\n    n << 5\n").unwrap();
        std::fs::write(dir.join("b/b.md"), head("b", "1", "parent: a\n", ">f (2) → 2\n")).unwrap();
        std::fs::write(dir.join("b/b.zero"), "on (int n) << h()\n    n << 6\n").unwrap();
        let s = store::read(&dir).unwrap();
        let l = lower::lower(&s).unwrap();
        let cases = calls_of(&s, &l, &native).unwrap();
        let (runs, over) = plan(&s, &cases).unwrap();
        let texts = |label: &str| -> Vec<String> { runs.iter().filter(|r| r.label == label).map(|r| cases[r.case].text.clone()).collect() };
        assert_eq!(texts(""), ["g() → 5", "f (2) → 2"]);
        assert_eq!(texts("with b off"), ["f (1) → 1", "g() → 5"]);
        assert_eq!(over.len(), 1);
        assert_eq!((over[0].text.as_str(), over[0].context.as_str(), over[0].why.as_str()), ("f (1) → 1", "every feature on", "replaced by b's cases for f (1)"));
        // `>existing` with nothing older to fall through to is refused
        std::fs::write(dir.join("b/b.md"), head("b", "1", "parent: a\n", ">existing\n>h() → 6\n")).unwrap();
        let s = store::read(&dir).unwrap();
        let cases = calls_of(&s, &lower::lower(&s).unwrap(), &native).unwrap();
        let err = match plan(&s, &cases) { Err(e) => e, Ok(_) => panic!("accepted") };
        assert!(err.contains("`>existing` in feature b's testing, but no older feature has cases for h()"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
        // a line's `off` switches one field (log 51); the descendants go
        // off through the gate, which `closure` says
        let s = store::read(Path::new("suite/zero/features")).unwrap();
        let l = lower::lower(&s).unwrap();
        let cases = calls_of(&s, &l, &native).unwrap();
        let base_off = cases.iter().find(|p| p.text.starts_with("greeted() with base off")).unwrap();
        let off = effective(&s, &BTreeSet::new(), base_off).unwrap();
        assert_eq!(off.iter().cloned().collect::<Vec<_>>(), ["base"]);
        assert_eq!(s.closure(&off).iter().cloned().collect::<Vec<_>>(), ["base", "more", "most", "tool"]);
        assert_eq!(setters(&base_off.call.context, b"hi", &[]), [("__set___enabled_base".to_string(), vec![0]), ("__in_ch".to_string(), vec![104]), ("__in_ch".to_string(), vec![105])]);
        // a line's switches are made in order, an `on` a call too
        assert_eq!(setters(&[("more".to_string(), false), ("base".to_string(), false), ("base".to_string(), true)], b"", &[]), [("__set___enabled_more".to_string(), vec![0]), ("__set___enabled_base".to_string(), vec![0]), ("__set___enabled_base".to_string(), vec![1])]);
        // a case does not stand where its feature is effectively off; a
        // line is a sequence of switches, `on` restoring a flag
        let sequence = Planned { text: String::new(), call: lower::Call { func: "switches".into(), args: vec![], nrets: 2, times: vec![false, false], expect: store::Expect::Values(vec![0, 1]), context: vec![("more".into(), false), ("base".into(), false), ("base".into(), true)], input: vec![], input_at: vec![] }, feature: "most".into(), rank: 3, file: "most.md".into(), line: 1 };
        assert!(effective(&s, &["base".to_string()].into_iter().collect(), &sequence).is_none());
        assert_eq!(effective(&s, &["tool".to_string()].into_iter().collect(), &sequence).unwrap().iter().cloned().collect::<Vec<_>>(), ["more", "tool"]);
        // a gate reads one field, the feature's effective state, in line
        // through the context (fm3 log 110, 138): `more` is under `base`,
        // so it is `__on_more`, which the setters of `more` and of
        // `base` work out, and no gate has an `and` in it
        assert!(l.ir.contains("fn greet__before_most() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    on: u1 = get _1, __on_more\n    if on\n    else\n        _2: int = greet__before_more()\n        ret _2\n    _3: int = greet__before_more()\n") && !l.ir.contains("__get___enabled"), "{}", l.ir);
        assert!(l.ir.contains("fn __set___enabled_more(v: u1)\n    p: ptr = context()\n    c: __ctx = load p\n    c0: __ctx = set c, __enabled_more, v\n    up: u1 = get c, __enabled_base\n    on: u1 = and v, up\n    c1: __ctx = set c0, __on_more, on\n    store c1, p\n    ret\n"), "{}", l.ir);
        assert!(l.ir.contains("fn __set___enabled_base(v: u1)\n    p: ptr = context()\n    c: __ctx = load p\n    c0: __ctx = set c, __enabled_base, v\n    own1: u1 = get c, __enabled_more\n    on1: u1 = and own1, v\n    c1: __ctx = set c0, __on_more, on1\n"), "{}", l.ir);
        // three deep: the innermost's gate is still one field, and the
        // root's setter works out each level from the one above it
        let n = emit(Path::new("suite/zero/nested")).unwrap();
        assert!(n.contains("fn reach() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    on: u1 = get _1, __on_three\n"), "{}", n);
        assert!(n.contains("    own1: u1 = get c, __enabled_one\n    on1: u1 = and own1, v\n") && n.contains("    own3: u1 = get c, __enabled_two\n    on3: u1 = and own3, on1\n") && n.contains("    own4: u1 = get c, __enabled_three\n    on4: u1 = and own4, on3\n"), "{}", n);
        let gates: Vec<&str> = n.lines().filter(|l| l.contains("= and ")).collect();
        assert!(n.split("\nfn ").filter(|f| !f.starts_with("__set___enabled_")).all(|f| !f.contains(" = and ")), "{:?}", gates);
    }

    /// A negative literal handed to a word that counts forward from a
    /// reader is refused when the program is compiled, naming the line
    /// (fm3 question 61, log 123): the case a person would write. Not
    /// `x$[-1]`, which question 75 gives a meaning, and not a computed
    /// index, which is the library's check to catch (question 64)
    /// zero.md section 14, "swap the context and behaviour changes at
    /// once", as a test (fm3 log 137): one store, two contexts made, a
    /// case run in each by turns, each one's state its own. On the JIT,
    /// by the runner's own entries: `__zero_context(k)` makes the k-th
    /// context current, `__zero_reset` puts the store back and makes the
    /// current context new, `__zero_new` makes a second beside it
    #[test]
    fn two_contexts_each_keep_their_own() {
        let jit_of = |dir: &Path| -> crate::emit::jit::JitCode {
            let s = store::read(dir).unwrap();
            let policy = store_policy(&s, &ssa::Policy::new(ssa::Type::I64).unwrap());
            let l = lower::lower(&s).unwrap();
            let m = build(&l.ir, &policy, 1).unwrap();
            let enc = crate::emit::Encoder::load("targets/arm64.encodings.json").unwrap();
            crate::emit::jit::JitCode::new(&crate::emit::compile(&m, &enc).unwrap()).unwrap()
        };
        // hello: a feature switched off in one context and on in the other
        let j = jit_of(Path::new("suite/zero/hello"));
        let call = |f: &str, args: &[i64]| j.call(f, args).unwrap_or_else(|e| panic!("{}: {}", f, e));
        let out = || -> String { (0..call("__out_len", &[])).map(|i| call("__out_byte", &[i]) as u8 as char).collect() };
        let fresh = |k: i64| {
            call("__zero_context", &[k]);
            if k == 0 { call("__zero_reset", &[]) } else { call("__zero_new", &[]) };
        };
        fresh(0);
        call("run", &[]);
        let on = out();
        fresh(0);
        call("__set___enabled_countdown", &[0]);
        call("run", &[]);
        let off = out();
        assert!(on.starts_with("10\n9\n") && on.ends_with(&off) && off.starts_with("hello world"), "{:?} {:?}", on, off);
        fresh(0);
        fresh(1);
        call("__set___enabled_countdown", &[0]);
        call("__zero_context", &[0]);
        call("run", &[]);
        call("__zero_context", &[1]);
        call("run", &[]);
        // each context's output is its own, and each context's time
        // (fm3 question 107, log 215): the second, its countdown off,
        // wrote two lines and no time passed in it; the first counted
        // down for ten seconds, whichever ran last
        let latest = || -> i64 { (0..=call("__out_len", &[])).map(|i| call("__out_mark", &[i])).max().unwrap() };
        assert_eq!((out(), latest()), (off.clone(), 0));
        call("__zero_context", &[0]);
        assert_eq!((out(), latest()), (on.clone(), 10_000_000));
        // ... and the switch written in the second left the first as it
        // was: run again it counts down again, from where its own clock
        // stood, and the second has heard nothing of it
        call("run", &[]);
        assert_eq!((out(), latest()), (format!("{}{}", on, on), 20_000_000));
        call("__zero_context", &[1]);
        assert_eq!((out(), latest()), (off.clone(), 0));
        // a context made new begins at nothing written and no time,
        // the other as it stood
        call("__zero_new", &[]);
        call("run", &[]);
        assert_eq!((out(), latest()), (on.clone(), 10_000_000));
        call("__zero_context", &[0]);
        assert_eq!((out(), latest()), (format!("{}{}", on, on), 20_000_000));

        // a stream processor's word in progress, a stream's own bit and
        // a variable, each kept across the other context's turn
        let dir = std::env::temp_dir().join(format!("probe-zero-two-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-07T10:00:00\n\n## testing\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "type token =\n    int kind\n    index start, n\n\nchar src$\ntoken u$ = lex(src$)\nint seen$ << 0\n\non (int k) << kind of (char c)\n    k << (0 if (c <= 32) else 1)\n\non (token t$) << lex (char c$)\n    int k$ = 0 if (empty c$) else kind of (c$)\n    bool new$ = k$ == 3 or k$ != k$[-1]\n    index start$ = position c$ if (new$) else start$[-1]\n    index n$ = 1 if (new$) else n$[-1] + 1\n    t$ << token(k$[-1], start$[-1], n$[-1]) if (new$ and k$[-1] != 0)\n\non arrive first()\n    src$ << \"let x = 4\"\n    seen$ << seen$ + 1\n\non arrive again()\n    src$ << \"2;\\n\"\n    end src$\n\non bump()\n    seen$ << seen$ + 1\n\non (int n) << tokens()\n    n << count u$\n\non (int n) << last length()\n    token x = peek u$ at (3)\n    n << x.n\n\non (int n) << bumps()\n    n << seen$\n").unwrap();
        let j = jit_of(&dir);
        let call = |k: i64, f: &str| -> i64 {
            j.call("__zero_context", &[k]).unwrap();
            j.call(f, &[]).unwrap_or_else(|e| panic!("{}: {}", f, e))
        };
        call(0, "__zero_reset");
        call(1, "__zero_new");
        call(0, "arrive_first");
        call(1, "arrive_first");
        call(1, "bump");
        call(1, "bump");
        assert_eq!((call(0, "tokens"), call(1, "tokens")), (3, 3));
        // the first context's input goes on and ends: its `4` becomes
        // `42;`, three long, and the second's word is still in progress
        call(0, "arrive_again");
        assert_eq!((call(0, "tokens"), call(0, "last_length"), call(1, "tokens")), (4, 3, 3));
        // ... and the second's then ends too, its own bit being its own
        call(1, "arrive_again");
        assert_eq!((call(1, "tokens"), call(1, "last_length"), call(0, "tokens")), (4, 3, 4));
        assert_eq!((call(0, "bumps"), call(1, "bumps")), (1, 3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cut (fm3 cut.md; touchstones.md, "hello, in two places", step
    /// 1): hello, not a character of it changed, gives its own case at
    /// the four cuts of the touchstone's table by name, from one placed
    /// build whose cut is a table the runner fills; and hello, restart
    /// and the three stores of things going on hold every case at every
    /// cut the runner tries, every unit on the server and each alone
    #[test]
    fn a_case_holds_at_every_cut() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let plain = run(hello, "run()", &native, opt::MAX_LEVEL, true).unwrap();
        assert_eq!(plain, "→ \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s\n");
        for server in ["", "count down", "count down,countdown.zero:2", "all"] {
            assert_eq!(run_cut(hello, "run()", &native, opt::MAX_LEVEL, true, Some(server), &[]).unwrap(), plain, "server: {}", server);
        }
        // a name three definitions have is not guessed at
        let err = run_cut(hello, "run()", &native, opt::MAX_LEVEL, true, Some("run"), &[]).unwrap_err();
        assert!(err.contains("'run' is 3 units") && err.contains("hello.zero:1, countdown.zero:4, bye.zero:1"), "{}", err);
        // the placement: the countdown on the server, its line at the client
        // (the table's columns are as wide as their widest row: read
        // with the spaces between them made one)
        let spaced = |t: String| t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let table = spaced(placement(hello, Some("count down"), &[]).unwrap());
        for row in ["countdown.zero:8 server the function `count down`", "countdown.zero:2 client the line `out$ << (i$ << \"\\n\") forever`", "a call countdown.zero:4 → countdown.zero:8 client → server nothing over, its end back", "an item countdown.zero:8 → countdown.zero:2 server → client one `int`, with its time, to the line that reads it", "a switch the switch of feature countdown client → server"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        assert!(placement(hello, None, &[]).unwrap().ends_with("the wire carries nothing\n"));
        let all = spaced(placement(hello, Some("all"), &[]).unwrap());
        assert!(all.contains("a call a case's own → bye.zero:1 client → server nothing over, its end back") && all.contains("text countdown.zero:2 → the client server → client"), "{}", all);
        // a plain build has nothing of it
        let s = store::read(hello).unwrap();
        let l = lower::lower(&s).unwrap();
        assert!(!s.places && l.placed.is_none() && !l.ir.contains("__wire") && !l.ir.contains("__cut"));
        let mut report = Report { passed: 0, failed: 0, skipped: 0, log: String::new() };
        let mut cuts = Cuts::default();
        for name in ["hello", "restart", "going-keys", "going-two", "going-mix"] {
            test_cuts(&Path::new("suite/zero").join(name), &native, Backend::Native, opt::MAX_LEVEL, &mut report, &mut cuts);
        }
        assert_eq!((cuts.hold, cuts.stores, cuts.refused, cuts.failed, cuts.unplaced), (5, 5, 0, 0, 0), "{}", report.log);
        assert!(cuts.cuts == 39 && cuts.passed > 200, "{} cuts, {} runs\n{}", cuts.cuts, cuts.passed, report.log);
        // ... and on another machine with nothing of its driver changed
        // (fm3 log 263): the cut is the calls a runner makes before any
        // case, and the wire is the program's own IR. wasm here; riscv64,
        // arm64 under qemu and the GPU are run by hand, the same counts
        let wasm = suite::backend_policy(Backend::Wasm).unwrap();
        let (mut report, mut there) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Cuts::default());
        for name in ["hello", "restart", "going-keys", "going-two", "going-mix"] {
            test_cuts(&Path::new("suite/zero").join(name), &wasm, Backend::Wasm, opt::MAX_LEVEL, &mut report, &mut there);
        }
        assert_eq!((there.hold, there.refused, there.failed, there.cuts, there.passed), (5, 0, 0, cuts.cuts, cuts.passed), "{}", report.log);
    }

    /// Each place has memory of its own (fm3 cut.md): what a unit at
    /// `server` reads is there because the wire carried it. With one thing
    /// deliberately not carried, which only this test can ask for, the
    /// program goes wrong: a stream kept as one value, written at the
    /// client and read on the server, reads as nothing was ever pushed; and
    /// restart's key, pushed into the input at the client, never
    /// arrives at the line that reads it on the server
    #[test]
    fn a_place_has_memory_of_its_own() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let dir = std::env::temp_dir().join(format!("probe-zero-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-12T10:00:00\n\n## testing\n>both (7) → 7\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "int level$ << 0\n\non put level (int n)\n    level$ << n\n\non (int n) << read level()\n    n << level$\n\non (int n) << both (int k)\n    put level (k)\n    n << read level()\n").unwrap();
        let gives = |dir: &Path, case: &str, server: &str, dropped: Option<&str>| -> (store::Expect, Result<suite::Got, String>) {
            let mut s = store::read(dir).unwrap();
            s.clock = store::Clock::Virtual;
            s.places = true;
            let policy = store_policy(&s, &native);
            let l = lower::lower(&s).unwrap();
            let p = l.placed.as_ref().unwrap();
            let server = p.named(server).unwrap();
            assert_eq!(p.refused(&server), None);
            let cut = p.cut_but(&server, dropped);
            let cases = calls_of(&s, &l, &policy).unwrap();
            let c = &case_named(&cases, case).unwrap().call;
            let module = build(&l.ir, &policy, opt::MAX_LEVEL).unwrap();
            let sc = suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: false, text: true, before: cut.before.into_iter().chain(setters(&c.context, &c.input, &c.input_at)).collect(), live: false, times: matches!(c.expect, store::Expect::Timed(_)) };
            (c.expect.clone(), suite::run_calls(&module, &l.ir, Backend::Native, &[sc], "zero-places", opt::MAX_LEVEL).unwrap().remove(0))
        };
        let (want, got) = gives(&dir, "both (7)", "read level", None);
        assert!(judge(&want, got).0);
        let (_, got) = gives(&dir, "both (7)", "read level", Some("level"));
        assert_eq!(got.unwrap().values, [0], "read on the server, the stream not carried: what the server's own memory holds");
        let _ = std::fs::remove_dir_all(&dir);
        let restart = Path::new("suite/zero/restart");
        let case = "launch() with in \"k\" at 3.5 s";
        let (want, got) = gives(restart, case, "restart.zero:3", None);
        assert!(judge(&want, got).0);
        let (want, got) = gives(restart, case, "restart.zero:3", Some("in"));
        let (ok, said) = judge(&want, got);
        assert!(!ok && said == "(printed \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz)", "{}", said);
    }

    /// one store's build whose cut may be moved, and cases of it run at
    /// moves: each the case, the units on the server at first, the cuts gone to
    /// with their times, and a thing deliberately not handed across at
    /// the moves. Gives whether each held and what the runner said
    fn moved_runs(dir: &Path, backend: Backend, runs: &[(&str, &str, &[(&str, i64)], Option<&str>)]) -> Vec<(bool, String)> {
        let policy = suite::backend_policy(backend).unwrap();
        let mut s = store::read(dir).unwrap();
        s.clock = store::Clock::Virtual;
        s.places = true;
        s.moved = true;
        let policy = store_policy(&s, &policy);
        let l = lower::lower(&s).unwrap();
        let p = l.placed.as_ref().unwrap();
        let cases = calls_of(&s, &l, &policy).unwrap();
        let module = build(&l.ir, &policy, opt::MAX_LEVEL).unwrap();
        let mut wants = Vec::new();
        let scalls: Vec<suite::Call> = runs
            .iter()
            .map(|(case, server, then, dropped)| {
                let c = &case_named(&cases, case).unwrap().call;
                let first = p.named(server).unwrap();
                let later: Vec<(BTreeSet<usize>, i64)> = then.iter().map(|(names, t)| (p.named(names).unwrap(), *t)).collect();
                let mut was = &first;
                for (next, _) in &later {
                    assert_eq!(p.move_refused(was, next), None);
                    was = next;
                }
                wants.push(c.expect.clone());
                suite::Call { func: c.func.clone(), args: c.args.clone(), nrets: c.nrets, checks: false, text: true, before: p.moves_but(&first, &later, *dropped).before.into_iter().chain(setters(&c.context, &c.input, &c.input_at)).collect(), live: false, times: matches!(c.expect, store::Expect::Timed(_)) }
            })
            .collect();
        suite::run_calls(&module, &l.ir, backend, &scalls, "zero-moved", opt::MAX_LEVEL).unwrap().into_iter().zip(wants).map(|(got, want)| judge(&want, got)).collect()
    }

    /// The cut, moved (fm3 cut.md, "the cut, moved"; touchstones.md,
    /// "hello, in two places", step 2): hello, not a character of it
    /// changed, started with everything on the server, and at 4.5 s, with 6
    /// written and 5 due, everything comes to the client; the other
    /// way; only the countdown; at 5 s, where the move is before the 5
    /// that is due then; and moved twice. Each gives hello's own line,
    /// no number twice and none missed. Then restart, moved before its
    /// key, with it and in its second countdown; the stores of things
    /// going on; and the store that keeps something
    #[test]
    fn the_cut_is_moved_while_the_program_runs() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let plain = run(hello, "run()", &native, opt::MAX_LEVEL, true).unwrap();
        assert_eq!(plain, "→ \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s\n");
        let s = |x: &str| x.to_string();
        for (server, to) in [("all", "none"), ("none", "all"), ("count down", "none")] {
            for t in [4_500_000, 5_000_000] {
                assert_eq!(run_cut(hello, "run()", &native, opt::MAX_LEVEL, true, Some(server), &[(s(to), t)]).unwrap(), plain, "server: {}, then {} at {}", server, to, t);
            }
        }
        assert_eq!(run_cut(hello, "run()", &native, opt::MAX_LEVEL, true, Some("all"), &[(s("none"), 2_500_000), (s("all"), 7_500_000)]).unwrap(), plain);
        let err = run_cut(hello, "run()", &native, opt::MAX_LEVEL, true, Some("all"), &[(s("none"), 5_000_000), (s("all"), 5_000_000)]).unwrap_err();
        assert!(err.contains("go forward in time"), "{}", err);
        // what is handed across, read off the text: for hello the
        // table, and nothing else; its functions partway are the pool's
        let spaced = |t: String| t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let table = spaced(placement(hello, Some("all"), &[(s("none"), 4_500_000)]).unwrap());
        for row in ["at 4.5 s the cut moves: server: nothing, the first thing due at that time", "countdown.zero:8 server client", "the table 17 bytes of the cut rewritten in the move's turn", "nothing else no field of the context, and no stored stream", "not handed a function partway: `count down`, `run` its record is a place of the pool, 7 columns"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        // a placed build that is not to be moved has no list and no
        // pool, as it had none; one that is has both; and a store not
        // placed is not moved
        let mut st = store::read(hello).unwrap();
        let unplaced = lower::lower(&st).unwrap().ir;
        st.moved = true;
        assert_eq!(lower::lower(&st).unwrap().ir, unplaced);
        st.moved = false;
        st.places = true;
        let fixed = lower::lower(&st).unwrap().ir;
        assert!(!fixed.contains("__move") && !fixed.contains("__act_") && !fixed.contains("fn __turns("), "{}", fixed);
        st.moved = true;
        let moved = lower::lower(&st).unwrap().ir;
        assert!(moved.contains("\nfn __move_turn()\n") && moved.contains("\nfn run__0(__a0: index) -> i64\n") && moved.contains("        d1: i64 = get x, __due_move\n"), "{}", moved);
        // restart's five cases, the cut moved before the key, with the
        // key, the move first, and in the second countdown
        let restart = Path::new("suite/zero/restart");
        let cases = ["launch() with in \"k\" at 3.5 s", "launch()", "launch() with in \"k\" at 4 s", "launch() with in \"k\" at 3.5 s, in \"k\" at 5 s", "launch() with in \"k\" at 12 s"];
        let ways: [(&str, [(&str, i64); 1]); 6] = [("all", [("none", 2_000_000)]), ("none", [("all", 2_000_000)]), ("all", [("none", 3_500_000)]), ("none", [("all", 3_500_000)]), ("all", [("none", 5_000_000)]), ("none", [("all", 5_000_000)])];
        let runs: Vec<(&str, &str, &[(&str, i64)], Option<&str>)> = cases.iter().flat_map(|c| ways.iter().map(move |(server, then)| (*c, *server, &then[..], None))).collect();
        for ((ok, said), (case, server, then, _)) in moved_runs(restart, Backend::Native, &runs).into_iter().zip(&runs) {
            assert!(ok, "{} server: {}, then {:?}: {}", case, server, then, said);
        }
        // every case with a time of the proof's stores, at the runner's
        // two times, both ways
        let names = ["hello", "restart", "going-keys", "going-two", "going-mix", "keeps"];
        let mut report = Report { passed: 0, failed: 0, skipped: 0, log: String::new() };
        let mut cuts = Cuts::default();
        for name in names {
            test_moves(&Path::new("suite/zero").join(name), &native, Backend::Native, opt::MAX_LEVEL, &mut report, &mut cuts);
        }
        assert_eq!((cuts.case_hold, cuts.moves_refused, cuts.moves_failed, cuts.unmoved, cuts.moved_stores), (cuts.timed, 0, 0, 0, 6), "{}", report.log);
        assert!(cuts.timed == 22 && cuts.moves == 88 && cuts.moves_passed == 88, "{} cases, {} moves\n{}", cuts.timed, cuts.moves, report.log);
        // ... and on another machine, nothing of its driver changed: the
        // moves are calls a runner makes before a case, as a cut's are
        let wasm = suite::backend_policy(Backend::Wasm).unwrap();
        let (mut report, mut there) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Cuts::default());
        for name in names {
            test_moves(&Path::new("suite/zero").join(name), &wasm, Backend::Wasm, opt::MAX_LEVEL, &mut report, &mut there);
        }
        assert_eq!((there.case_hold, there.moves_refused, there.moves_failed, there.moves, there.moves_passed), (cuts.timed, 0, 0, cuts.moves, cuts.moves_passed), "{}", report.log);
        // a function that keeps a stream of the context across its
        // step kept the machine's stack, and its move was refused by
        // name (`timed`'s `count down`, which reads `i$` once above its
        // loop). It is left at its step now (fm3 log 282): what it
        // keeps is where its reader has read to, and which stream that
        // is the continuation reads from the context of the place it
        // goes on at
        let timed = Path::new("suite/zero/timed");
        for (server, then) in [("all", "none"), ("none", "all")] {
            let out = run_cut(timed, "launch()", &native, opt::MAX_LEVEL, true, Some(server), &[(s(then), 4_500_000)]).unwrap();
            assert!(out.trim_end().ends_with("→ \"20\\n18\\n16\\n14\\n12\\n10\\n8\\n6\\n4\\n2\\nliftoff\" at 1 hz"), "{} {}: {}", server, then, out);
        }
        let mut st = store::read(timed).unwrap();
        st.places = true;
        st.moved = true;
        let text = lower::lower(&st).unwrap().ir;
        let at = text.find("\nfn count_down__1(__a0: index) -> i64\n").expect("the countdown is left at its step");
        let body: Vec<&str> = text[at + 1..].lines().take_while(|l| !l.starts_with("fn ") || l.starts_with("fn count_down__1(")).collect();
        assert!(body.iter().any(|l| l.ends_with(": int$ = get __s2, i")) && body.iter().any(|l| l.trim_start().starts_with("_2: int$ = set ") && l.contains(", edge, ")), "the stream is read from the context again: {:#?}", body);
        assert!(text.contains("\ndata __act_count_down___2__pos: ") && !text.contains("\ndata __act_count_down___2: "), "what is kept of it is the reader's own, three words, and no address");
    }

    /// A wire that takes time, where nothing comes from outside (fm3
    /// cut.md, "a wire that takes time"; log 269, 270). hello, not a
    /// character changed, at the touchstone's four cuts over a wire of
    /// 100 ms: by its own times the case's own line at each, and what
    /// is late is the first number, by the wire there and back, the
    /// rest having been worked out ahead and kept until its time. With
    /// only `hello` the server the countdown is on time and the two lines
    /// after it are late. A wire of no delay shows nothing late. A
    /// store that keeps something, its clock and its sum at different
    /// places. The same on another machine. And a place acts only on
    /// what has reached it: with the client let to take its turn
    /// before the server has said it will begin nothing earlier, which only
    /// this test can ask for, the case's call is written before the
    /// clock's first beat
    #[test]
    fn a_wire_takes_time() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let line = "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let late = |pieces: &[Shown]| -> Vec<(String, i64, i64)> { pieces.iter().filter(|p| p.shown > p.own).map(|p| (p.text.clone(), p.own, p.shown)).collect() };
        let first = vec![("10\n".to_string(), 0, 200_000)];
        for (server, want) in [("none", vec![]), ("count down", first.clone()), ("count down,countdown.zero:2", first.clone()), ("all", first.clone()), ("hello", vec![("hello world\n".to_string(), 10_000_000, 10_200_000), ("goodbye\n".to_string(), 10_000_000, 10_200_000)])] {
            let (_, shown, pieces) = wire_case(hello, "run()", &native, opt::MAX_LEVEL, Backend::Native, Some(server), 100_000, 0).unwrap();
            assert_eq!(shown, line, "server: {}", server);
            assert_eq!(pieces.len(), 12, "server: {}", server);
            assert_eq!(late(&pieces), want, "server: {}", server);
            let (_, shown, pieces) = wire_case(hello, "run()", &native, opt::MAX_LEVEL, Backend::Native, Some(server), 0, 0).unwrap();
            assert_eq!((shown.as_str(), late(&pieces).len()), (line, 0), "server: {}, no delay", server);
        }
        assert_eq!(run_wire(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), 100_000).unwrap(), format!("run()\n→ {}\nlate: 1 piece of 12, the most 200 ms\n  \"10\\n\" for 0 s, shown at 200 ms, 200 ms late\n", line));
        // the placement: the table as it is, and what the wire carried
        // by the world's clock. Everything on the server, the call arrives at
        // 100 ms and the whole countdown leaves then
        let spaced = |t: String| t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let table = spaced(placement_wire(hello, Some("all"), 100_000, None, &native, opt::MAX_LEVEL).unwrap());
        for row in ["a wire of 100 ms, by the world's clock: `run()`", "0 s 100 ms client → server a function partway, to go on in `run` for 0 s", "100 ms 200 ms server → client text \"10\\n\" for 0 s", "100 ms 200 ms server → client text \"1\\n\" for 9 s", "100 ms 200 ms server → client text \"hello w\" and 4 more for 10 s", "100 ms 200 ms server → client a function partway, to go on in a case's own call for 10 s", "late: 1 piece of 12, the most 200 ms"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        // a store not placed is not built for a wire; a placed build
        // with no wire is the text it was; one with a wire has the
        // world, a list a place, and no function that jumps
        let mut st = store::read(hello).unwrap();
        let unplaced = lower::lower(&st).unwrap().ir;
        st.wire = true;
        assert_eq!(lower::lower(&st).unwrap().ir, unplaced);
        st.wire = false;
        st.places = true;
        let fixed = lower::lower(&st).unwrap().ir;
        assert!(!fixed.contains("__world") && !fixed.contains("__list_") && !fixed.contains("__wq_") && fixed.contains("\nfn __wire(to: i64) -> i64\n"), "{}", fixed);
        st.wire = true;
        let wired = lower::lower(&st).unwrap().ir;
        assert!(wired.contains("\nfn __world()\n") && wired.contains("\nfn __list_next() -> (i64, i64)\n") && wired.contains("\nfn count_down__o1__0(__a: index) -> i64\n") && !wired.contains("\nfn __wire(") && !wired.contains("__wire_unit"), "{}", wired);
        assert_eq!(wired.matches("context_set(").count(), 1, "one function makes a place the current one");
        // a store that keeps something: its clock on the server and its sum at
        // the client, and the other way
        let keeps = Path::new("suite/zero/keeps");
        for (case, want) in [("begin()", "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n10 4\" at 1 hz from 1 s"), ("count to (5)", "\"1 1\\n3 2\\n6 3\\n10 4\\n\" at 1 hz, \"15\" at 5 s")] {
            for server in ["keeps.zero:3", "keeps.zero:4", "keeps.zero:5", "count to", "all", "none"] {
                for d in [0, 100_000] {
                    let (_, shown, pieces) = wire_case(keeps, case, &native, opt::MAX_LEVEL, Backend::Native, Some(server), d, 0).unwrap();
                    assert_eq!(shown, want, "{} server: {}, a wire of {}", case, server, d);
                    assert!(d > 0 || late(&pieces).is_empty(), "{} server: {}: late with no delay", case, server);
                }
            }
        }
        let (_, _, pieces) = wire_case(keeps, "begin()", &native, opt::MAX_LEVEL, Backend::Native, Some("keeps.zero:3"), 100_000, 0).unwrap();
        assert_eq!(late(&pieces), vec![("1 1\n".to_string(), 0, 100_000), ("go\n".to_string(), 0, 300_000)]);
        // a place acts only on what has reached it
        let (_, shown, _) = wire_case(keeps, "begin()", &native, opt::MAX_LEVEL, Backend::Native, Some("keeps.zero:3"), 100_000, 1).unwrap();
        assert_eq!(shown, "\"go\\n1 1\\n\" at 0 s, \"3 2\\n6 3\\n10 4\" at 1 hz from 1 s");
        // every cut the runner tries of the proof's stores, over no
        // delay and over 100 ms, on the JIT and on another machine
        // with nothing of its driver changed
        let names = ["hello", "keeps", "static", "going", "going-mix"];
        for (backend, d) in [(Backend::Native, 0), (Backend::Wasm, 100_000)] {
            let policy = suite::backend_policy(backend).unwrap();
            let (mut report, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Wires::default());
            for name in names {
                test_wire(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut wires, d, false);
            }
            assert_eq!((wires.hold, wires.unbuilt, wires.refused, wires.failed, wires.case_hold), (5, 0, 0, 0, wires.cases), "{}", report.log);
            assert!(d > 0 || wires.cuts_on_time == wires.cuts, "{}", report.log);
        }
    }

    /// A wire that takes time, with a key (fm3 cut.md, "a wire that
    /// takes time" and "working ahead of a key"; log 271, 273). A
    /// store with an input has something from outside, and the server cannot
    /// know that no key was pressed at 2.9 s until the world's clock
    /// says 3 s. Hop forty-seven had it wait to be told; this test let
    /// it work ahead, by a switch, to show the wait was needed. Now
    /// working ahead is the build's own way: the server keeps a copy before
    /// each turn it takes untold and goes back when a key comes, and
    /// the client drops what a key makes void. So restart, not a
    /// character changed, everything on the server over 100 ms, gives its own
    /// line by its own times, and what is late is the first number
    /// and the first after each key, 200 ms each: the 9 for 1 s is
    /// shown at 1 s. The cut the touchstone names, the key read at the
    /// client and `launch` the server, and every cut the runner tries. And
    /// with no copy kept and nothing dropped, which only this test can
    /// ask for, a number of the first countdown is written for a time
    /// after the key, as it was
    #[test]
    fn a_wire_takes_time_with_a_key() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let restart = Path::new("suite/zero/restart");
        let cases = [
            ("launch() with in \"k\" at 3.5 s", "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.5 s", 15),
            ("launch()", "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz", 11),
            ("launch() with in \"k\" at 4 s", "\"10\\n9\\n8\\n7\\n10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz", 15),
            ("launch() with in \"k\" at 3.5 s, in \"k\" at 5 s", "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n\" at 3.5 s, \"9\\n\" at 4.5 s, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 5 s", 17),
            ("launch() with in \"k\" at 12 s", "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 12 s", 22),
        ];
        for ((case, want, n), late) in cases.into_iter().zip([2, 1, 2, 3, 2]) {
            // (what the plain build gives, in the same spelling)
            assert_eq!(run(restart, case, &native, opt::MAX_LEVEL, true).unwrap().trim_start(), format!("→ {}\n", want), "{}", case);
            let (_, shown, pieces) = wire_case(restart, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 100_000, 0).unwrap();
            assert_eq!(shown, want, "{}", case);
            // (what is late is the first number, its call having to cross, and the first after each key, the key having to cross: nothing a key does not touch)
            assert_eq!((pieces.len(), pieces.iter().filter(|p| p.shown > p.own).count(), pieces.iter().filter(|p| p.shown > p.own).all(|p| p.text == "10\n" && p.shown - p.own == 200_000)), (n, late, true), "{}: {:?}", case, pieces);
            let (_, shown, pieces) = wire_case(restart, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 0, 0).unwrap();
            assert_eq!((shown.as_str(), pieces.iter().filter(|p| p.shown > p.own).count()), (want, 0), "{}, no delay", case);
        }
        let (_, _, pieces) = wire_case(restart, cases[0].0, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 100_000, 0).unwrap();
        let at = |k: usize| (pieces[k].text.as_str(), pieces[k].own, pieces[k].shown);
        assert_eq!((at(0), at(1), at(3), at(4), at(5), at(14)), (("10\n", 0, 200_000), ("9\n", 1_000_000, 1_000_000), ("7\n", 3_000_000, 3_000_000), ("10\n", 3_500_000, 3_700_000), ("9\n", 4_500_000, 4_500_000), ("liftoff", 13_500_000, 13_500_000)));
        // the touchstone's step 3: the key read at the client, `launch` the server
        for server in ["launch", "launch,count down", "restart.zero:3", "none"] {
            let (_, shown, _) = wire_case(restart, cases[0].0, &native, opt::MAX_LEVEL, Backend::Native, Some(server), 100_000, 0).unwrap();
            assert_eq!(shown, cases[0].1, "server: {}", server);
        }
        // the placement with its times: the client's telling at 1 s
        // lets the server work on 1 s, and the key crosses as a push
        let spaced = |t: String| t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let table = spaced(placement_wire(restart, Some("all"), 100_000, None, &native, opt::MAX_LEVEL).unwrap());
        for row in ["a wire of 100 ms, by the world's clock: `launch() with in \"k\" at 3.5 s`", "0 s 100 ms client → server a telling: nothing for a time up to 0 s", "100 ms 200 ms server → client text \"9\\n\" for 1 s", "3.5 s 3.6 s client → server a push into a stored stream, made again where it is read for 3.5 s", "3.6 s 3.7 s server → client text \"10\\n\" for 3.5 s", "late: 2 pieces of 15, the most 200 ms"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        // working ahead with no copy kept and nothing dropped, as hop
        // forty-seven let it in to show its bound was needed, is still
        // the fault it was: the first countdown's 6, after the key
        let (_, shown, pieces) = wire_case(restart, cases[0].0, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 100_000, 2).unwrap();
        assert_ne!(shown, cases[0].1);
        assert!(pieces.iter().any(|p| p.text == "6\n" && p.own == 4_000_000), "{}: {:?}", shown, pieces);
        // every cut the runner tries, of the stores with a key and of
        // one with two clocks in a slot, on the JIT and on another machine
        let names = ["restart", "going-keys", "going-two", "going-mix"];
        for backend in [Backend::Native, Backend::Wasm] {
            let policy = suite::backend_policy(backend).unwrap();
            let (mut report, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Wires::default());
            for name in names {
                test_wire(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut wires, 100_000, false);
            }
            assert_eq!((wires.stores, wires.unbuilt, wires.failed, wires.case_hold), (4, 0, 0, wires.cases), "{}", report.log);
            // (refused by name: `going-keys`' line apart from what its function calls on the stack; `going-two`'s two functions that step at different places were, until a record carried when it was begun, fm3 log 278)
            assert_eq!((wires.cuts, wires.refused), (28, 3), "{}", report.log);
        }
        // two characters that arrive together at a line whose function
        // crosses are not taken out of order: the program stops
        // (fm3 question 156)
        let dir = std::env::temp_dir().join(format!("probe-wire-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("restart")).unwrap();
        std::fs::copy("suite/zero/restart/restart/restart.zero", dir.join("restart/restart.zero")).unwrap();
        let md = std::fs::read_to_string("suite/zero/restart/restart/restart.md").unwrap();
        std::fs::write(dir.join("restart/restart.md"), format!("{}\n>launch() with in \"kk\" → check\n", md.trim_end())).unwrap();
        let err = wire_case(&dir, "launch() with in \"kk\"", &native, opt::MAX_LEVEL, Backend::Native, Some("launch"), 100_000, 0);
        assert!(err.as_ref().err().is_some_and(|e| e.starts_with(suite::checked())), "{:?}", err.map(|(_, s, _)| s));
        let ok = wire_case(&dir, "launch() with in \"kk\"", &native, opt::MAX_LEVEL, Backend::Native, Some("none"), 100_000, 0);
        assert!(ok.is_ok(), "{:?}", ok.err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The server works ahead of a key, and goes back (fm3 cut.md, "working
    /// ahead of a key"; log 273, 274). restart, not a character
    /// changed, everything on the server over 100 ms, its key at 3.5 s: the server has
    /// worked the whole first countdown out by 0.1 s, the client keeps
    /// it and shows each number at its time, and when the key is
    /// pressed the client drops the 6 to liftoff it was keeping and
    /// the server, when the key reaches it at 3.6 s, goes back to what it was
    /// at 3 s. Two pieces of fifteen are late, the two that could not
    /// be otherwise. A key at 3.95 s, which the server hears of after the 6
    /// for 4 s was due: no 6, the client having dropped it when the
    /// key was pressed. Going back is needed, and the client's
    /// dropping, and the copy of the server's streams: with each left out, by
    /// a switch only this test has, the case gives something else. A
    /// store that keeps a running sum across its key gives a wrong
    /// number, 621 for 306, where the server is not put back; and at the cut
    /// where a beat pushes into a stream the client reads, a turn
    /// taken ahead is taken back and the case holds. A store with no
    /// input that arrives at a time has none of this in its text. And
    /// every cut the runner tries of the stores with a key, on the JIT
    /// and on another machine
    #[test]
    fn the_server_works_ahead_of_a_key() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let restart = Path::new("suite/zero/restart");
        let key = "launch() with in \"k\" at 3.5 s";
        let line = "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.5 s";
        let all = |dir: &Path, case: &str, but: i64| wire_case_cost(dir, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 100_000, but).unwrap();
        let has = |pieces: &[Shown], text: &str, own: i64| pieces.iter().any(|p| p.text == text && p.own == own);
        // the timeline: each piece, the time it is for and when it was shown
        let (_, shown, pieces, thrown) = all(restart, key, 0);
        assert_eq!(shown, line);
        let s = 1_000_000;
        let want: Vec<(&str, i64, i64)> = vec![("10\n", 0, 200_000), ("9\n", s, s), ("8\n", 2 * s, 2 * s), ("7\n", 3 * s, 3 * s), ("10\n", 3_500_000, 3_700_000), ("9\n", 4_500_000, 4_500_000), ("8\n", 5_500_000, 5_500_000), ("7\n", 6_500_000, 6_500_000), ("6\n", 7_500_000, 7_500_000), ("5\n", 8_500_000, 8_500_000), ("4\n", 9_500_000, 9_500_000), ("3\n", 10_500_000, 10_500_000), ("2\n", 11_500_000, 11_500_000), ("1\n", 12_500_000, 12_500_000), ("liftoff", 13_500_000, 13_500_000)];
        assert_eq!(pieces.iter().map(|p| (p.text.as_str(), p.own, p.shown)).collect::<Vec<_>>(), want);
        assert_eq!(thrown, Some(7), "the turns for 4 s to 10 s");
        assert_eq!(run_wire(restart, key, &native, opt::MAX_LEVEL, Some("all"), 100_000).unwrap(), format!("{}\n→ {}\nlate: 2 pieces of 15, the most 200 ms\n  \"10\\n\" for 0 s, shown at 200 ms, 200 ms late\n  \"10\\n\" for 3.5 s, shown at 3.7 s, 200 ms late\nthe server worked ahead: 7 turns it took were thrown away\n", key, line));
        // the placement: what the wire carried, each piece dropped
        // at the client unshown, and the server going back
        let spaced = |t: String| t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let table = spaced(placement_wire(restart, Some("all"), 100_000, Some(key), &native, opt::MAX_LEVEL).unwrap());
        for row in ["100 ms 200 ms server → client text \"9\\n\" for 1 s", "100 ms 200 ms server → client text \"liftoff\" for 10 s", "100 ms 200 ms server → client a telling: nothing before 10 s for 10 s", "3.5 s 3.6 s client → server a push into a stored stream, made again where it is read for 3.5 s", "3.5 s at the client dropped unshown: text \"6\\n\" for 4 s", "3.5 s at the client dropped unshown: text \"liftoff\" for 10 s", "3.6 s the server goes back to 3 s: 7 turns thrown away", "3.6 s 3.7 s server → client text \"10\\n\" for 3.5 s", "3.6 s 3.7 s server → client text \"liftoff\" for 13.5 s", "late: 2 pieces of 15, the most 200 ms", "the server worked ahead: 7 turns it took were thrown away"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        assert!(!table.contains("dropped unshown: text \"7\\n\""), "{}", table);
        // a key at 3.95 s: the server hears of it at 4.05 s, after the 6 for
        // 4 s was due. No case of the suite has it
        let dir = std::env::temp_dir().join(format!("probe-wire-ahead-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("restart")).unwrap();
        std::fs::copy("suite/zero/restart/restart/restart.zero", dir.join("restart/restart.zero")).unwrap();
        let md = std::fs::read_to_string("suite/zero/restart/restart/restart.md").unwrap();
        let late_key = "launch() with in \"k\" at 3.95 s";
        let late_line = "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.95 s";
        std::fs::write(dir.join("restart/restart.md"), format!("{}\n>{} → {}\n", md.trim_end(), late_key, late_line)).unwrap();
        assert_eq!(run(&dir, late_key, &native, opt::MAX_LEVEL, true).unwrap().trim_start(), format!("→ {}\n", late_line), "the plain build");
        let (_, shown, pieces, thrown) = all(&dir, late_key, 0);
        assert_eq!(shown, late_line);
        assert!(!pieces.iter().any(|p| p.text == "6\n" && p.own < 7_000_000), "{:?}", pieces);
        assert_eq!(pieces.iter().filter(|p| p.shown > p.own).map(|p| (p.text.as_str(), p.own, p.shown)).collect::<Vec<_>>(), vec![("10\n", 0, 200_000), ("10\n", 3_950_000, 4_150_000)]);
        assert_eq!(thrown, Some(7));
        let table = spaced(placement_wire(&dir, Some("all"), 100_000, Some(late_key), &native, opt::MAX_LEVEL).unwrap());
        for row in ["3.95 s at the client dropped unshown: text \"6\\n\" for 4 s", "4.05 s the server goes back to 3 s: 7 turns thrown away"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        let _ = std::fs::remove_dir_all(&dir);
        // going back is needed: the server not put back takes the key where
        // it had got to, the end of the first countdown at 10 s
        let (_, shown, pieces, _) = all(restart, key, 4);
        assert_ne!(shown, line);
        assert!(has(&pieces, "10\n", 10_000_000) && !has(&pieces, "10\n", 3_500_000), "{}: {:?}", shown, pieces);
        // the client's dropping is needed: with it off the first
        // countdown's 6 for 4 s is shown, after the key at 3.5 s
        let (_, shown, pieces, _) = all(restart, key, 8);
        assert_ne!(shown, line);
        assert!(pieces.iter().any(|p| p.text == "6\n" && p.own == 4_000_000 && p.shown == 4_000_000), "{}: {:?}", shown, pieces);
        // a program that keeps something across the key: a running
        // sum, and the beats a stored stream holds
        let keeps = Path::new("suite/zero/keeps-key");
        for case in ["begin()", "begin() with in \"k\" at 2.25 s", "begin() with in \"k\" at 2.25 s, in \"k\" at 4.25 s"] {
            let plain = run(keeps, case, &native, opt::MAX_LEVEL, true).unwrap();
            for server in ["all", "bump.zero:4", "bump.zero:5", "bump.zero:4,bump.zero:5", "begin", "none"] {
                let (_, shown, _) = wire_case(keeps, case, &native, opt::MAX_LEVEL, Backend::Native, Some(server), 100_000, 0).unwrap();
                assert_eq!(format!("→ {}\n", shown), plain.trim_start(), "{} server: {}", case, server);
            }
        }
        let one = "begin() with in \"k\" at 2.25 s";
        let (_, _, pieces, thrown) = all(keeps, one, 0);
        assert!(has(&pieces, "306\n", 2_250_000) && has(&pieces, "321\n", 5_000_000), "{:?}", pieces);
        assert_eq!(pieces.iter().filter(|p| p.shown > p.own).map(|p| (p.text.as_str(), p.own, p.shown)).collect::<Vec<_>>(), vec![("1\n", 0, 100_000), ("go\n", 0, 300_000), ("306\n", 2_250_000, 2_450_000)]);
        assert_eq!(thrown, Some(10));
        // a turn that would send more than text is taken back: with
        // the clock and its sum on the server and the key's function at the
        // client, each beat pushes into the stored stream the key's
        // function reads, and the server's turn for it, taken ahead, is put
        // back whole and waits to be told. Late, and the case's own line
        let table = spaced(placement_wire(keeps, Some("bump.zero:4,bump.zero:5"), 100_000, Some(one), &native, opt::MAX_LEVEL).unwrap());
        for row in ["200 ms the server takes back a turn it took ahead: it would have sent more than text for 1 s", "1.1 s 1.2 s server → client text \"3\\n\" for 1 s", "late: 7 pieces of 8, the most 200 ms"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        // ... not put back, six beats and a sum of 21 where the key
        // found three and 6
        let (_, _, pieces, _) = all(keeps, one, 4);
        assert!(pieces.iter().any(|p| p.text == "621\n") && !pieces.iter().any(|p| p.text == "306\n"), "{:?}", pieces);
        // ... and put back but for its streams, the sum is 6 again and
        // the stored stream holds every beat the server ever worked out,
        // eleven of them, where three had been
        let (_, _, pieces, _) = all(keeps, one, 16);
        assert!(pieces.iter().any(|p| p.text == "1106\n") && !pieces.iter().any(|p| p.text == "306\n"), "{:?}", pieces);
        // only a store with an input that arrives at a time has it
        let mut st = store::read(Path::new("suite/zero/hello")).unwrap();
        st.places = true;
        st.wire = true;
        let hello = lower::lower(&st).unwrap().ir;
        assert!(!works_ahead(&hello) && !hello.contains("__wback") && !hello.contains("__wk_") && !hello.contains("__wq_r"), "hello has no bound to work ahead of");
        let mut st = store::read(restart).unwrap();
        st.places = true;
        st.wire = true;
        let wired = lower::lower(&st).unwrap().ir;
        assert!(works_ahead(&wired) && wired.contains("\nfn __wback(s: i64, r: i64)\n") && wired.contains("\nfn __wkeep("));
        assert_eq!(wired.matches("context_set(").count(), 1, "one function makes a place the current one");
        st.wire = false;
        assert!(!lower::lower(&st).unwrap().ir.contains("__wback"), "a placed build with no wire is the text it was");
        // every cut the runner tries, on the JIT and on another machine
        let names = ["restart", "going-keys", "going-two", "keeps-key"];
        for backend in [Backend::Native, Backend::Wasm] {
            let policy = suite::backend_policy(backend).unwrap();
            let (mut report, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Wires::default());
            for name in names {
                test_wire(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut wires, 100_000, false);
            }
            assert_eq!((wires.stores, wires.unbuilt, wires.failed, wires.case_hold), (4, 0, 0, wires.cases), "{}", report.log);
            assert_eq!((wires.cuts, wires.refused), (21, 6), "{}", report.log);
        }
    }

    /// The cut moved over a wire that takes time (fm3 cut.md, "the cut
    /// moved over a wire that takes time"; log 276). hello, not a
    /// character changed, everything on the server, a wire of 100 ms, and at 4.5 s
    /// everything comes to the client: the move's word leaves the client
    /// at 4.5 s, the server's state is at the client at 4.7 s, and the 5 is
    /// worked out there on time. By its own times the case is hello's
    /// line; one piece is late, the first 10. Said at 4.9 s the 5 is
    /// 100 ms late, and at 5 s, the move first, 200 ms. Then restart
    /// with its key at the move's own time, where the server, which had worked
    /// on to liftoff, goes back and the client drops what it was
    /// keeping; and the stores that keep something, where with one
    /// thing deliberately not sent the sum is wrong, and with the
    /// functions partway not sent the countdown does not go on
    #[test]
    fn the_cut_is_moved_over_a_wire_that_takes_time() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let line = "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let late = |pieces: &[Shown]| -> Vec<(String, i64, i64)> { pieces.iter().filter(|p| p.shown > p.own).map(|p| (p.text.clone(), p.own, p.shown)).collect() };
        let moved = |dir: &Path, case: &str, server: &str, then: &[(&str, i64)], d: i64, but: i64, dropped: Option<&str>| {
            let then: Vec<(String, i64)> = then.iter().map(|(c, t)| (c.to_string(), *t)).collect();
            wire_moved_case(dir, case, &native, opt::MAX_LEVEL, Backend::Native, Some(server), &then, d, but, dropped).unwrap()
        };
        let ten = ("10\n".to_string(), 0, 200_000);
        let five = |shown: i64| ("5\n".to_string(), 5_000_000, shown);
        for (server, then, want) in [
            ("all", vec![("none", 4_500_000)], vec![ten.clone()]),
            ("none", vec![("all", 4_500_000)], vec![]),
            ("count down", vec![("none", 4_500_000)], vec![ten.clone()]),
            ("none", vec![("count down", 4_500_000)], vec![]),
            // at 5 s the move is first, and the 5 waits for it
            ("all", vec![("none", 5_000_000)], vec![ten.clone(), five(5_200_000)]),
            ("none", vec![("all", 5_000_000)], vec![five(5_200_000)]),
            // said too near a piece: the server's state is at the client at 5.1 s
            ("all", vec![("none", 4_900_000)], vec![ten.clone(), five(5_100_000)]),
            ("none", vec![("all", 4_900_000)], vec![five(5_100_000)]),
            ("all", vec![("none", 2_500_000), ("all", 7_500_000)], vec![ten.clone()]),
        ] {
            let (_, shown, pieces, thrown) = moved(hello, "run()", server, &then, 100_000, 0, None);
            assert_eq!(shown, line, "server: {}, then {:?}", server, then);
            assert_eq!((pieces.len(), late(&pieces), thrown), (12, want, None), "server: {}, then {:?}", server, then);
            // ... and over a wire that takes no time nothing is late
            let (_, shown, pieces, _) = moved(hello, "run()", server, &then, 0, 0, None);
            assert_eq!((shown.as_str(), late(&pieces).len()), (line, 0), "server: {}, then {:?}, no delay", server, then);
        }
        let at: Vec<(String, i64)> = vec![("none".to_string(), 4_500_000)];
        assert_eq!(run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), &at, 100_000).unwrap(), format!("run()\n→ {}\nlate: 1 piece of 12, the most 200 ms\n  \"10\\n\" for 0 s, shown at 200 ms, 200 ms late\n", line));
        // the placement: what is handed across, and the move's own
        // items with when they left and arrived
        let spaced = |t: String| t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        let table = spaced(placement_wire_moved(hello, Some("all"), &at, 100_000, None, &native, opt::MAX_LEVEL).unwrap());
        for row in ["at 4.5 s the cut moves: server: nothing, the first thing due at that time, said at the client", "0 s 100 ms client → server a telling: nothing before 4.5 s for 0 s", "100 ms 200 ms server → client text \"6\\n\" for 4 s", "4.5 s 4.6 s client → server the move's word: the cut's new table, and what the client hands with it for 4.5 s", "4.6 s 4.7 s server → client the server's state: what the server hands across at the move for 4.5 s", "4.6 s 4.7 s server → client a function partway, handed across at the move, to go on in `count down` when it is next due for 5 s", "late: 1 piece of 12, the most 200 ms"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        // the server, told of nothing past the move, sent nothing for after it
        assert!(!table.contains("text \"5\\n\""), "{}", table);
        // only a build for a wire whose cut may be moved has any of it,
        // and a build that moves with no wire is the text it was
        let mut st = store::read(hello).unwrap();
        st.places = true;
        st.wire = true;
        let plain = lower::lower(&st).unwrap().ir;
        assert!(!plain.contains("__wmove_server") && !plain.contains("__whand") && !plain.contains("__move_turn") && !plain.contains("__due_move"), "a wire's build with no move is the text it was");
        st.moved = true;
        let both = lower::lower(&st).unwrap().ir;
        assert!(both.contains("\nfn __wmove_server(row: index, k: index, t: i64)\n") && both.contains("\nfn __whand(row: index, k: index, dir: u8)\n") && both.contains("\nfn __wship()\n") && both.contains("    mv: u1 = cmp.eq k, 1\n    if mv\n        __move_turn()\n        ret\n") && !both.contains("\nfn __hand(k: index)\n"), "{}", both);
        assert_eq!(both.matches("context_set(").count(), 1, "one function makes a place the current one");
        st.wire = false;
        let fixed = lower::lower(&st).unwrap().ir;
        assert!(fixed.contains("\nfn __hand(k: index)\n") && !fixed.contains("__whand") && !fixed.contains("__wship"), "a build that moves with no wire is the text it was");
        // restart, moved at its key's own time: the move first, then
        // the key. The server, which had worked on to liftoff, goes back to
        // 3 s; the client drops the 6 to liftoff it was keeping
        let restart = Path::new("suite/zero/restart");
        let key = "launch() with in \"k\" at 3.5 s";
        let rline = "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.5 s";
        let (_, shown, pieces, thrown) = moved(restart, key, "all", &[("none", 3_500_000)], 100_000, 0, None);
        assert_eq!((shown.as_str(), late(&pieces), thrown), (rline, vec![ten.clone(), ("10\n".to_string(), 3_500_000, 3_700_000)], Some(7)));
        let (_, shown, pieces, thrown) = moved(restart, key, "none", &[("all", 3_500_000)], 100_000, 0, None);
        assert_eq!((shown.as_str(), late(&pieces), thrown), (rline, vec![("10\n".to_string(), 3_500_000, 3_900_000)], Some(7)));
        let three: Vec<(String, i64)> = vec![("none".to_string(), 3_500_000)];
        let table = spaced(placement_wire_moved(restart, Some("all"), &three, 100_000, Some(key), &native, opt::MAX_LEVEL).unwrap());
        for row in ["a stream the items of in$ that the server still holds, with their times, and its counts server → client sent by the server when the move's word reaches it, and the client's when that arrives", "3.5 s 3.6 s client → server the move's word: the cut's new table, and what the client hands with it for 3.5 s", "3.5 s at the client dropped unshown: text \"6\\n\" for 4 s", "3.5 s at the client dropped unshown: text \"liftoff\" for 10 s", "3.6 s the server goes back to 3 s: 7 turns thrown away", "3.6 s 3.7 s server → client the server's state: what the server hands across at the move for 3.5 s", "3.6 s 3.7 s server → client a function partway, handed across at the move, to go on in `count down` when it is next due for 4 s"] {
            assert!(table.contains(row), "no {:?} in\n{}", row, table);
        }
        assert!(!table.contains("dropped unshown: text \"7\\n\""), "{}", table);
        // a program that keeps something: its sums, its clock's latest,
        // its look back, and, in this build, when its clock is next
        // due, each sent with the move; with one not sent it goes wrong
        let keeps = Path::new("suite/zero/keeps");
        let (to, fro): ([(&str, i64); 1], [(&str, i64); 1]) = ([("none", 2_500_000)], [("all", 2_500_000)]);
        let fifteen = "\"1 1\\n3 2\\n6 3\\n10 4\\n\" at 1 hz, \"15\" at 5 s";
        let go = "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n10 4\" at 1 hz from 1 s";
        for (case, server, then, dropped, want) in [
            ("count to (5)", "all", &to, None, fifteen),
            ("count to (5)", "none", &fro, None, fifteen),
            ("begin()", "all", &to, None, go),
            ("begin()", "none", &fro, None, go),
            // 1, 2 and 3 were added at one place; 4 and 5 at the other, to nothing
            ("count to (5)", "all", &to, Some("total"), "\"1 1\\n3 2\\n6 3\\n10 4\\n\" at 1 hz, \"9\" at 5 s"),
            ("count to (5)", "none", &fro, Some("total"), "\"1 1\\n3 2\\n6 3\\n10 4\\n\" at 1 hz, \"9\" at 5 s"),
            ("begin()", "all", &to, Some("sum"), "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n4 -2\" at 1 hz from 1 s"),
            ("begin()", "all", &to, Some("beat"), "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n7 1\\n9 2\\n12 3\\n16 4\" at 1 hz from 1 s"),
            ("begin()", "all", &to, Some("__z1_x_1"), "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n10 10\" at 1 hz from 1 s"),
            // when the clock is next due not sent: it beats at once where it arrives
            ("begin()", "all", &to, Some("__due1"), "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n\" at 1 s, \"6 3\\n10 4\" at 2 s"),
            ("begin()", "none", &fro, Some("__due1"), "\"1 1\\ngo\\n\" at 0 s, \"3 2\\n\" at 1 s, \"6 3\\n10 4\" at 2 s"),
        ] {
            assert_eq!(moved(keeps, case, server, then, 100_000, 0, dropped).1, want, "{} server: {}, not sent: {:?}", case, server, dropped);
        }
        // ... and this build's own new thing, the pool's records of the
        // units that move: not sent, by a switch only this test has,
        // the count and the countdown do not go on
        assert_eq!(moved(keeps, "count to (5)", "all", &to, 100_000, 32, None).1, "\"1 1\\n3 2\\n6 3\\n10 4\" at 1 hz");
        for (server, then) in [("all", [("none", 4_500_000)]), ("none", [("all", 4_500_000)])] {
            assert_eq!(moved(hello, "run()", server, &then, 100_000, 32, None).1, "\"10\\n9\\n8\\n7\\n6\" at 1 hz", "server: {}", server);
        }
        // a sum that crosses between two keys, and restart's input:
        // its items not sent, the second key is not seen
        let keyed = Path::new("suite/zero/keeps-key");
        let two = "begin() with in \"k\" at 2.25 s, in \"k\" at 4.25 s";
        for (server, then) in [("all", [("none", 3_500_000)]), ("none", [("all", 3_500_000)])] {
            assert_eq!(moved(keyed, two, server, &then, 100_000, 0, None).1, "\"1\\ngo\\n\" at 0 s, \"3\\n\" at 1 s, \"6\\n\" at 2 s, \"306\\n\" at 2.25 s, \"310\\n\" at 3 s, \"315\\n\" at 4 s, \"515\\n\" at 4.25 s, \"521\" at 5 s", "server: {}", server);
        }
        let keys = "launch() with in \"k\" at 3.5 s, in \"k\" at 5 s";
        assert_eq!(moved(restart, keys, "all", &[("none", 4_000_000)], 100_000, 0, None).1, "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n\" at 3.5 s, \"9\\n\" at 4.5 s, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 5 s");
        assert_eq!(moved(restart, keys, "all", &[("none", 4_000_000)], 100_000, 0, Some("in")).1, rline);
        // every case with a time of the stores of the proof, moved both
        // ways at two times, on the JIT and on another machine
        let names = ["hello", "restart", "going-keys", "going-two", "going-mix", "keeps", "keeps-key"];
        for backend in [Backend::Native, Backend::Wasm] {
            let policy = suite::backend_policy(backend).unwrap();
            let (mut report, mut cuts, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Cuts::default(), Wires::default());
            for name in names {
                test_wire_moves(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut cuts, &mut wires, 100_000, false);
            }
            assert_eq!((cuts.moved_stores, cuts.unmoved, cuts.moves, cuts.moves_refused, cuts.moves_failed, cuts.case_hold), (7, 0, 100, 0, 0, cuts.timed), "{}", report.log);
            // (and with no delay nothing is late)
            let (mut report, mut cuts, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Cuts::default(), Wires::default());
            for name in names {
                test_wire_moves(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut cuts, &mut wires, 0, false);
            }
            assert_eq!((cuts.moves, cuts.moves_failed, wires.late), (100, 0, 0), "{}", report.log);
        }
        // what is refused, by name: a move from or to a cut that is
        assert!(cut_of(lower::lower(&{ let mut s = wired(Path::new("suite/zero/lex")).unwrap(); s.moved = true; s }).unwrap().placed.as_ref().unwrap(), Some("number start"), &[("none".to_string(), 1_500_000)]).unwrap_err().starts_with("this cut cannot be run yet: 'u$' is stored, and one reader of it would be at both places"));
    }

    /// A value wider than a word across a place (fm3 cut.md, "one way to
    /// leave a function"; log 284). A function handed, giving or keeping
    /// an array, a string, a structure, a `float` or an enumeration is
    /// left at its calls as one that keeps words is: a value is as many
    /// columns of the pool as it has words, and where its record arrives
    /// at a place each array or string in it is copied into that place's
    /// memory (`__act_land`), so that no address crosses. So a cut
    /// between a function and the one it hands an array to runs over a
    /// wire that takes time. And the copy is needed: the server, working ahead
    /// of a key, keeps a copy of itself before each turn, and with the
    /// values not copied, by a switch only this test has, an array the server is
    /// still making when the key arrives is not put back with the rest
    #[test]
    fn a_value_wider_than_a_word_is_kept_across_a_place() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let arrays = Path::new("suite/zero/arrays");
        for (case, server, gives) in [("handed()", "total of", "69"), ("handed()", "handed", "69"), ("each doubled()", "doubled", "20"), ("squared()", "squares to", "16"), ("squared()", "all", "16")] {
            assert_eq!(run_wire(arrays, case, &native, opt::MAX_LEVEL, Some(server), 100_000).unwrap(), format!("{}\n→ {}\nlate: nothing, of 0 pieces\n", case, gives), "{} with {} on the server", case, server);
        }
        let text = lower::lower(&wired(arrays).unwrap()).unwrap();
        let p = text.placed.as_ref().unwrap();
        for server in ["total of", "handed", "doubled", "each doubled", "squares to", "squared", "all"] {
            assert_eq!(p.refused(&p.named(server).unwrap()), None, "{}", server);
        }
        // the array `handed` makes is `total of`'s parameter, four
        // columns; the array `each doubled` is making and the view it
        // walks are kept across its call; and a record that arrives has
        // them copied
        let ir = &text.ir;
        for col in ["__act_total_of__x__ring", "__act_total_of__x__pos", "__act_each_doubled___4__ring", "__act_each_doubled___1__ptr", "__act_each_doubled___1__len", "__act_res0_squares_to__ring"] {
            assert!(ir.contains(&format!("\ndata {}: array(i64, ", col)), "no column {}", col);
        }
        assert!(ir.contains("\nfn __act_land(row: index)\n") && ir.contains("\nfn __act_land_int(c: ptr, row: index)\n") && ir.contains("\nfn __act_land_view_int(c: ptr, lc: ptr, sc: ptr, row: index)\n"));
        assert!(ir.contains("        __act_carry(fl, slot)\n        __wland(slot)\n"), "a function partway that arrives has its values copied");
        // a structure is a column a field, a `float` a column, and an
        // enumeration a word
        let types = lower::lower(&wired(Path::new("suite/zero/types")).unwrap()).unwrap();
        assert!(types.ir.contains("\ndata __act_sum_of_fields__v__x: array(i64, ") && types.ir.contains("\ndata __act_sum_of_fields__v__z: array(i64, "), "a `Vec` is three columns");
        let p = types.placed.as_ref().unwrap();
        for server in ["sum of fields", "flipped", "twice"] {
            assert_eq!(p.refused(&p.named(server).unwrap()), None, "{}", server);
        }
        // a store that keeps no value has none of it
        for name in ["hello", "restart", "keeps-key"] {
            let t = lower::lower(&wired(&Path::new("suite/zero").join(name)).unwrap()).unwrap().ir;
            assert!(!t.contains("__act_land") && !t.contains("__wland"), "{}", name);
        }
        // every cut of the stores whose refusals were values, on
        // wasm, and of one of them on the JIT
        let all: &[&str] = &["arrays", "strings", "pushed", "sampled", "sampled-kept", "sampled-between", "brackets"];
        for (backend, policy, names) in [(Backend::Native, native.clone(), &all[1..2]), (Backend::Wasm, suite::backend_policy(Backend::Wasm).unwrap(), all)] {
            let (mut report, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Wires::default());
            for name in names {
                test_wire(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut wires, 100_000, false);
            }
            assert_eq!((wires.hold, wires.stores, wires.refused, wires.failed, wires.unbuilt), (names.len(), names.len(), 0, 0, 0), "{}", report.log);
        }
        // an array a function is still making when a key is pressed.
        // `build` doubles four numbers, one a second, each call of
        // `slowly doubled` a step; the server works all four out ahead, the
        // key at 1.5 s puts it back to what it was at 1 s, and the
        // array it was making must be put back too: one item, not four.
        // With the copy not made the sum is 38 for 20
        let dir = std::env::temp_dir().join(format!("probe-wire-built-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("built")).unwrap();
        std::fs::write(dir.join("built/built.zero"), "int n$ at (1 hz)\nout$ << (n$ << \"\\n\") forever\nint seen$ = keys (in$)\n\non (int d) << slowly doubled (int x)\n    n$ << x\n    d << x * 2\n\non (int n) << pressed (char c)\n    out$ << \"key\\n\"\n    n << 1\n\non (int k$) << keys (char c$)\n    k$ << pressed (c$)\n\non build()\n    int a[] = [1, 2, 3, 4]\n    int b[] = slowly doubled (a[])\n    int t = b[] + _\n    out$ << t\n").unwrap();
        let key = "build() with in \"k\" at 1.5 s";
        let line = "\"1\\n\" at 0 s, \"2\\nkey\\n3\\n\" at 2 hz from 1 s, \"4\\n\" at 3 s, \"20\" at 4 s";
        std::fs::write(dir.join("built/built.md"), format!("# built\n*an array a function is still making when a key is pressed*\n\nlayer: runtime\n\n> (suite) 2026-10-13T12:00:00\nA function that builds an array an item a second, and a key.\n\n## testing\n>build() → \"1\\n2\\n3\\n4\\n20\" at 1 hz\n>{} → {}\n", key, line)).unwrap();
        assert_eq!(run(&dir, key, &native, opt::MAX_LEVEL, true).unwrap().trim_start(), format!("→ {}\n", line), "the plain build");
        let (_, shown, _, thrown) = wire_case_cost(&dir, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 100_000, 0).unwrap();
        assert_eq!((shown.as_str(), thrown), (line, Some(5)));
        let (_, shown, _, _) = wire_case_cost(&dir, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), 100_000, 64).unwrap();
        assert_eq!(shown, "\"1\\n\" at 0 s, \"2\\nkey\\n3\\n\" at 2 hz from 1 s, \"4\\n\" at 3 s, \"38\" at 4 s", "with the values not copied, the four items the server made ahead are still in the array: 2, 4, 6, 8 and then 4, 6 and 8 again");
        let _ = std::fs::remove_dir_all(&dir);
        // One storage under two names of one record is copied once. A
        // function's own stream, pushed into two `if`s deep after a
        // call: the inner `if` is put out as a function of its own,
        // handed the stream, so the record holds it twice, as
        // `gathered`'s `s` and as `gathered__o1`'s. Copied apiece where
        // the record comes back from `doubled`, the push would go into
        // one copy and the frame be taken of the other: 1 for 7, which
        // is what this gave before the copies were kept for an arrival
        // (`__land_was`, `__land_note`)
        let dir = std::env::temp_dir().join(format!("probe-wire-gathered-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("gathered")).unwrap();
        std::fs::write(dir.join("gathered/gathered.zero"), "on (int d) << doubled (int x)\n    d << x * 2\n\non (int n) << gathered (int k)\n    int s$\n    s$ << 1\n    if (k > 0)\n        if (k > 1)\n            s$ << doubled (k)\n    int f[] = frame s$\n    n << f[] + _\n").unwrap();
        std::fs::write(dir.join("gathered/gathered.md"), "# gathered\n*a stream a function makes, pushed into in a call put out of it*\n\nlayer: runtime\n\n> (suite) 2026-10-13T13:00:00\nA function's own stream, pushed into two `if`s deep after a call.\n\n## testing\n>gathered (3) → 7\n>gathered (1) → 1\n").unwrap();
        let text = lower::lower(&wired(&dir).unwrap()).unwrap().ir;
        assert!(text.contains("\ndata __act_gathered__s__ring: ") && text.contains("\ndata __act_gathered__o1__s__ring: "), "the stream is held under two names");
        for server in ["doubled", "gathered", "all"] {
            assert_eq!(run_wire(&dir, "gathered (3)", &native, opt::MAX_LEVEL, Some(server), 100_000).unwrap(), "gathered (3)\n→ 7\nlate: nothing, of 0 pieces\n", "{} on the server", server);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A function still on the machine's stack is called at its own
    /// place (fm3 cut.md, "one way to leave a function"; log 286). A
    /// function over an abstract type, `smaller of (number a) and
    /// (number b)`, is the IR's to make anew for the types a call has,
    /// and so is no continuation; one with several definitions, one a
    /// type, and one called with another type than its parameter's,
    /// likewise. A function left at its calls cannot wait for it across
    /// a wire. So the call is put out as a function of the callee's own
    /// unit whose one statement is the call, with the types the call
    /// site has: the record crosses to the callee's place and the
    /// callee is called there, on the stack
    #[test]
    fn a_function_on_the_stack_is_called_at_its_own_place() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let functions = Path::new("suite/zero/functions");
        assert_eq!(run_wire(functions, "narrow (300000)", &native, opt::MAX_LEVEL, Some("narrow"), 100_000).unwrap(), "narrow (300000)\n→ 300000\nlate: nothing, of 0 pieces\n");
        let l = lower::lower(&wired(functions).unwrap()).unwrap();
        let at = l.ir.find("\nfn smaller_of_and__o1__0(__a: index) -> i64\n").expect("the call is a function of its own, left as any is");
        assert!(l.ir[at..].lines().take(8).any(|t| t == "    __r0: i32 = smaller_of_and(__p0, __p1)"), "it calls the function as written, with the call's own types");
        assert!(l.ir.contains("\nfn smaller_of_and(a: number, b: number) -> number\n"), "the function itself is as it was written");
        let p = l.placed.as_ref().unwrap();
        for server in ["narrow", "kind of a decimal", "width at thirty two bits", "describe a decimal"] {
            assert_eq!(p.refused(&p.named(server).unwrap()), None, "{}", server);
        }
        // a case's own call of such a function is the client's, and
        // still waits for it: refused by name, for the abstract type
        let why = p.refused(&p.named("smaller of and").unwrap()).unwrap();
        assert!(why.contains("is called by `__case_smaller_of_and`, the client's own") && why.ends_with("it is handed `number`, an abstract type, for which the IR makes the function anew"), "{}", why);
        // a stream handed to a function is no value, and is never
        // handed through such a call: the callee would be given a copy
        let checks = lower::lower(&wired(Path::new("suite/zero/checks")).unwrap()).unwrap();
        assert!(!checks.ir.contains("\nfn shut__o"), "a stream is not handed as a value");
        let p = checks.placed.as_ref().unwrap();
        assert!(p.refused(&p.named("shut").unwrap()).unwrap().ends_with("it is handed a stream"));
        // every cut of two stores that call such functions, on wasm
        let wasm = suite::backend_policy(Backend::Wasm).unwrap();
        let (mut report, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Wires::default());
        for name in ["functions", "sequences"] {
            test_wire(&Path::new("suite/zero").join(name), &wasm, Backend::Wasm, opt::MAX_LEVEL, &mut report, &mut wires, 100_000, false);
        }
        assert_eq!((wires.stores, wires.failed, wires.unbuilt, wires.broken), (2, 0, 0, 0), "{}", report.log);
    }

    /// Which of two functions was started first, carried (fm3 question
    /// 155's B; cut.md; log 278). Two functions partway, one at each
    /// place and due at one time, are taken in the order they were
    /// begun: a record carries the time it was begun, and the rank a
    /// place says of its earliest thing has that time in it. `going-two`
    /// has both partway: its case's own `count down from`, begun at 0 s,
    /// and the `count down` its key begins at 2.5 s, both due at 3 s.
    /// With `count down from` on the server the 2 is first, where the client's
    /// own went first and the case gave `10` before `2`; such a cut was
    /// refused by name and now runs
    #[test]
    fn of_two_functions_at_two_places_the_first_begun_goes_first() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let two = Path::new("suite/zero/going-two");
        let case = "count down from (5) with in \"k\" at 2.5 s";
        let line = "\"5\\n4\\n3\\n\" at 1 hz, \"2\\n10\\n\" at 3 s, \"1\\n9\\n\" at 4 s, \"8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 5 s";
        for server in ["count down from", "count down", "launch", "all", "none"] {
            let (_, shown, pieces) = wire_case(two, case, &native, opt::MAX_LEVEL, Backend::Native, Some(server), 100_000, 0).unwrap();
            assert_eq!(shown, line, "server: {}", server);
            // (the 2 is shown before the 10, which waits for it)
            let at = |text: &str| pieces.iter().find(|p| p.text == text && p.own == 3_000_000).map(|p| p.shown).unwrap();
            assert!(at("2\n") <= at("10\n"), "server: {}: {:?}", server, pieces);
        }
        // with the function begun first on the server, both are late and in order
        let (_, _, pieces) = wire_case(two, case, &native, opt::MAX_LEVEL, Backend::Native, Some("count down from"), 100_000, 0).unwrap();
        let three: Vec<(&str, i64)> = pieces.iter().filter(|p| p.own == 3_000_000).map(|p| (p.text.as_str(), p.shown)).collect();
        assert_eq!(three, vec![("2\n", 3_200_000), ("10\n", 3_400_000)]);
        // the record's own word, and the rank that carries it
        let mut st = store::read(two).unwrap();
        st.places = true;
        st.wire = true;
        let text = lower::lower(&st).unwrap().ir;
        assert!(text.contains("\ndata __act_begun: array(i64, ") && text.contains("    kk: i64 = mul kq, 17592186044416\n") && text.contains("    k: i64 = div kr, 17592186044416\n"), "{}", text);
        st.wire = false;
        st.moved = true;
        assert!(!lower::lower(&st).unwrap().ir.contains("__act_begun"), "a build with no wire is the text it was");
        // the four stores whose only refused cuts were these hold at
        // every cut, on the JIT and on another machine
        for backend in [Backend::Native, Backend::Wasm] {
            let policy = suite::backend_policy(backend).unwrap();
            let (mut report, mut wires) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Wires::default());
            for name in ["going-two", "going-beside", "quiet-clocks", "timed-zeroic"] {
                test_wire(&Path::new("suite/zero").join(name), &policy, backend, opt::MAX_LEVEL, &mut report, &mut wires, 100_000, false);
            }
            assert_eq!((wires.stores, wires.hold, wires.refused, wires.failed, wires.case_hold), (4, 4, 0, 0, wires.cases), "{}", report.log);
        }
    }

    /// A move hands across what it must (fm3 cut.md, "the cut, moved").
    /// The store that keeps something, moved at 2.5 s: its running sums,
    /// its clock's latest and its look back are at the place the units
    /// leave and are copied once to the place they go to. With one of
    /// them deliberately not handed across, which only this test can
    /// ask for, the program goes wrong: the sum begins again from
    /// nothing. And restart's input, read on the server and then at the client:
    /// with its items not handed across, the second key is not seen
    #[test]
    fn a_move_hands_across_what_it_must() {
        let keeps = Path::new("suite/zero/keeps");
        let at: [(&str, i64); 1] = [("none", 2_500_000)];
        let back: [(&str, i64); 1] = [("all", 2_500_000)];
        let got = moved_runs(keeps, Backend::Native, &[
            ("count to (5)", "all", &at, None),
            ("count to (5)", "all", &at, Some("total")),
            ("count to (5)", "none", &back, None),
            ("count to (5)", "none", &back, Some("total")),
            ("begin()", "all", &at, None),
            ("begin()", "all", &at, Some("sum")),
            ("begin()", "all", &at, Some("beat")),
            ("begin()", "all", &at, Some("__z1_x_1")),
        ]);
        assert!(got[0].0 && got[2].0 && got[4].0, "{:?}", got);
        // 1, 2 and 3 were added on the server; 4 and 5 at the client, to nothing
        for k in [1, 3] {
            assert!(!got[k].0 && got[k].1 == "(printed \"1 1\\n3 2\\n6 3\\n10 4\\n\" at 1 hz, \"9\" at 5 s)", "{:?}", got[k]);
        }
        // the sum not handed: 4 at 3 s, where the case has 10, and it
        // has fallen by 2 from the 6 the look back remembers
        assert!(!got[5].0 && got[5].1 == "(printed \"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n4 -2\" at 1 hz from 1 s)", "{:?}", got[5]);
        // the clock's latest not handed: it counts again from 1, to 4
        assert!(!got[6].0 && got[6].1 == "(printed \"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n7 1\\n9 2\\n12 3\\n16 4\" at 1 hz from 1 s)", "{:?}", got[6]);
        // the look back not handed: the rise is from nothing
        assert!(!got[7].0 && got[7].1 == "(printed \"1 1\\ngo\\n\" at 0 s, \"3 2\\n6 3\\n10 10\" at 1 hz from 1 s)", "{:?}", got[7]);
        let restart = Path::new("suite/zero/restart");
        let case = "launch() with in \"k\" at 3.5 s, in \"k\" at 5 s";
        let four: [(&str, i64); 1] = [("none", 4_000_000)];
        let got = moved_runs(restart, Backend::Native, &[(case, "all", &four, None), (case, "all", &four, Some("in"))]);
        assert!(got[0].0, "{:?}", got);
        assert!(!got[1].0 && got[1].1 == "(printed \"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.5 s)", "{:?}", got[1]);
    }

    /// An array or a string handed to a function at the other place,
    /// or given by one, is copied at the crossing (fm3 question 152,
    /// log 267). In the IR the two are a stream's type; the lowering
    /// says which parameters and results are values, by how they were
    /// declared, and those the wire copies: at the head of the
    /// function for a parameter, where its `ret` crosses back for a
    /// result. A stream handed to a function is its storage and stays
    /// refused. So the stores whose only refused cuts were values hold
    /// every case at every cut
    #[test]
    fn a_value_is_copied_at_a_crossing() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let placed = |name: &str| -> lower::Lowered {
            let mut s = store::read(&Path::new("suite/zero").join(name)).unwrap();
            s.places = true;
            lower::lower(&s).unwrap()
        };
        let arrays = placed("arrays");
        let at = arrays.ir.find("\nfn total_of(x__0: int$) -> int\n").expect("the parameter is the caller's until the wire is crossed");
        let body: Vec<&str> = arrays.ir[at + 1..].lines().skip(1).take(2).collect();
        assert!(body[0].starts_with("    __pw: i64 = __wire_unit(") && body[1] == "    x: int$ = __carried_int(x__0, __pw)", "{:?}", body);
        assert!(arrays.ir.contains(": int$ = __carried_int(_37, __pb") && arrays.ir.contains("\nfn __copied_int(v: int$) -> int$\n"), "a result is copied where the wire is crossed back");
        let p = arrays.placed.as_ref().unwrap();
        for server in ["total of", "squares to"] {
            assert_eq!(p.refused(&p.named(server).unwrap()), None, "{}", server);
        }
        // a string kept as a stream's latest value, written at one
        // place and read at the other, is copied by the wire
        let variables = placed("variables");
        let p = variables.placed.as_ref().unwrap();
        assert_eq!(p.refused(&p.named("tool opened").unwrap()), None);
        assert!(variables.ir.contains(": u8$ = __copied_u8(a"), "{}", variables.ir);
        // a stream handed to a function is still an address
        let streams = placed("streams");
        let p = streams.placed.as_ref().unwrap();
        let why = p.refused(&p.named("how many so far").unwrap()).unwrap();
        assert!(why.contains("'how many so far' is handed, or gives, a stream, which is an address"), "{}", why);
        // a store that hands no value across has none of it
        assert!(!placed("hello").ir.contains("__copied_") && !placed("hello").ir.contains("__carried_"));
        let names = ["arrays", "sequences", "pushed", "functions", "strings", "sampled", "sampled-kept", "sampled-between", "variables"];
        let (mut report, mut cuts) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Cuts::default());
        for name in names {
            test_cuts(&Path::new("suite/zero").join(name), &native, Backend::Native, opt::MAX_LEVEL, &mut report, &mut cuts);
        }
        assert_eq!((cuts.hold, cuts.stores, cuts.refused, cuts.failed, cuts.unplaced), (9, 9, 0, 0, 0), "{}", report.log);
        let wasm = suite::backend_policy(Backend::Wasm).unwrap();
        let (mut report, mut there) = (Report { passed: 0, failed: 0, skipped: 0, log: String::new() }, Cuts::default());
        for name in names {
            test_cuts(&Path::new("suite/zero").join(name), &wasm, Backend::Wasm, opt::MAX_LEVEL, &mut report, &mut there);
        }
        assert_eq!((there.hold, there.refused, there.failed, there.cuts, there.passed), (9, 0, 0, cuts.cuts, cuts.passed), "{}", report.log);
    }

    #[test]
    fn a_negative_literal_index_is_refused() {
        let dir = std::env::temp_dir().join(format!("probe-zero-negative-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 6\n").unwrap();
        let with = |line: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) << f()\n    int x$ << [5, 6, 7]\n    advance x$ by (2)\n{}\n", line)).unwrap();
            emit(&dir)
        };
        for (line, said) in [
            ("    n << peek x$ at (-1)", "h.zero:4: 'peek' counts forward from the reader: -1 is behind it"),
            ("    advance x$ by (-2)\n    n << 1", "h.zero:4: 'advance' moves the reader forward: -2 is behind it"),
            ("    int b[] = x$ behind (-1)\n    n << 1", "h.zero:4: 'behind' takes how many items, a count: -1 is behind it"),
        ] {
            let err = with(line).expect_err(line);
            assert!(err.ends_with(said), "{}: {}", line, err);
        }
        // a literal that is not negative, a subscript that is, and an
        // index worked out to be negative all lower as they did
        for line in ["    n << peek x$ at (0)", "    index i = 0\n    n << peek x$ at (i - 1)"] {
            let ir = with(line).unwrap_or_else(|e| panic!("{}: {}", line, e));
            assert!(ir.contains("fn f() -> int\n"), "{}", ir);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream processor with no loop in it (fm3 question 75 rule 1,
    /// log 124): every line holds for every item. For each wiring the
    /// front end writes a function of one item; where nothing else
    /// reads the input it has no storage, a push into it being that
    /// function's call and a literal a loop that says its count; where
    /// something does, the input keeps its queue and a sink the front
    /// end writes walks it. And what such a body may not hold is
    /// refused with what to write (question 65)
    #[test]
    fn a_stream_processor_holds_for_every_item() {
        let dir = std::env::temp_dir().join(format!("probe-zero-zeroic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 8\n").unwrap();
        let head = "int x$\nint d$ = doubled(x$)\nchar t$\nint c$ = codes(t$)\nint s$\nint e$ = doubled(s$)\n\non (int k$) << codes (char c$)\n    k$ << int(c$)\n\non (int n) << f()\n    x$ << 1 << 2\n    t$ << \"abcd\"\n    s$ << 3\n    n << [count] (frame d$) + [count] (frame c$) + [count] (frame e$) + [count] (frame s$)\n\n";
        let with = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int d$) << doubled (int x$)\n{}\n", head, body)).unwrap();
            emit(&dir)
        };
        let ir = with("    d$ << x$ * 2").unwrap();
        // the function of one item, one a wiring; the second is called
        // by nothing, its lines standing in the literal's loop (fm3 log
        // 134), and is not in the text
        assert!(ir.contains("fn __z1_each(_x: int)\n") && !ir.contains("fn __z2_each(") && ir.contains("fn __z3_each(_x: int)\n"), "{}", ir);
        // `x$` and `t$` have no storage: no field, and the push is the call
        let ctx = &ir[ir.find("type __ctx = struct").unwrap()..ir.find("data __ctx_mem").unwrap()];
        assert!(!ctx.contains("    x: ") && !ctx.contains("    t: ") && ctx.contains("    s: int$\n"), "{}", ctx);
        let f = &ir[ir.find("fn f() -> int").unwrap()..];
        let f = &f[..f[1..].find("\nfn ").map_or(f.len(), |i| i + 1)];
        assert_eq!(f.matches("__z1_each(").count(), 2, "{}", f);
        // the literal is a loop that says four, once and before it (fm3
        // log 257), the processor's line in it
        assert!(f.contains(": index = const 4\n") && f.contains(" = load ") && !f.contains("__z2_each(") && f.contains(": int = conv "), "{}", f);
        // `s$` is read by `count s$`: it keeps its queue, and the sink
        // the front end wrote walks it, woken where the push is
        assert!(ir.contains("fn __z3(x: int$, __hz: i64) -> int$\n") && f.contains("= __z3("), "{}", ir);
        for (body, said) in [
            ("    if (x$ > 0)\n        d$ << x$", "h.zero:18: an `if` round a line of a stream processor: every line holds for every item, so the condition goes on the push, `d$ << item if (condition)`, or in the value, `a if (c) else b`"),
            ("    int k = x$ * 2\n    d$ << k", "h.zero:18: in a stream processor every line holds for every item, so each line says a stream: write `int k$ = ...`"),
            ("    d$ << x$ while (_ < 5)", "h.zero:18: `while` on a push repeats it, which is a loop, and a stream processor has none: say the stream by a line of its own, or write `d$ << item if (condition)`"),
            ("    e$ << x$", "h.zero:18: a stream processor pushes into its own output, 'd$'"),
            ("    d$ << d$ + x$", "h.zero:18: 'd$' is the output: a stream processor pushes into it and does not read it"),
        ] {
            let err = with(body).expect_err(body);
            assert!(err.ends_with(said), "{}: {}", body, err);
        }
        // handed a stream inside a function: a wiring a function makes
        // is not built, and the message names the array to hand it
        // (fm3 question 113)
        std::fs::write(dir.join("h/h.zero"), "on (int d$) << doubled (int x$)\n    d$ << x$ * 2\n\non (int n) << f()\n    int i$ << [1, 2, 3]\n    int d$ = doubled(i$)\n    n << [count] (frame d$)\n").unwrap();
        let err = emit(&dir).expect_err("inside a function");
        assert!(err.ends_with("h.zero:6: 'doubled' is a stream processor with no loop in it: its lines hold for every item it is handed. At feature scope it is wired, `int y$ = doubled (i$)`; inside a function a line happens once, and what it is handed is an array, `doubled (frame i$)` the array of what has arrived (fm3 question 113). A wiring made by a function is not built"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed check names its line (fm3 log 199, question 115): the
    /// program as it is compiled has nothing in it that says where it
    /// is; the diagnostic build of the same store stores a site before
    /// each statement and the numbers before a checked read or push,
    /// and its two words that read the output back say the site after
    /// the text; the table reads it back in the program's own names
    #[test]
    fn a_failed_check_names_its_line() {
        let dir = std::env::temp_dir().join(format!("probe-zero-sites-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>far (9) → check at h.zero:5\n>far (1) → 2\n>held (70) → check\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "int kept$\n\non (int n) << far (int i)\n    int a$ << 1 << 2 << 3 << 4\n    n << peek a$ at (i)\n\non (int n) << held (int k)\n    kept$ << 1 (k) times\n    n << peek kept$ at (0)\n").unwrap();
        let mut s = store::read(&dir).unwrap();
        // the case's form
        let expects: Vec<store::Expect> = s.features.iter().flat_map(|f| f.cases.iter().map(|c| c.expect.clone())).collect();
        assert_eq!(expects, [store::Expect::CheckAt("h.zero:5".into()), store::Expect::Values(vec![2]), store::Expect::Check]);
        // the program: no site, no word of the diagnostic build, no table
        let plain = lower::lower(&s).unwrap();
        assert!(!plain.ir.contains("__site") && plain.sites.is_empty(), "{}", plain.ir);
        // the diagnostic build: every statement stores its site first,
        // an item asked by its place hands over the place and how many
        // there are, and a push says whether the stream had ended
        s.sites = true;
        let l = lower::lower(&s).unwrap();
        let body = |f: &str| -> String { l.ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        let far = body("far");
        // (the item is a stream's, by `peek`: an array's item by its
        // place is no check, fm3 question 127; its row is the one
        // after its statement's, whichever number that is)
        let k = l.sites.iter().position(|x| x.what == "item {a} of {b}").expect("a row for the item") + 1;
        assert!(far.contains("    __site_at(3)\n") && far.contains(": index = count ") && far.contains(&format!("    __site_at3({}, ", k)), "{}", far);
        let held = body("held");
        assert!(held.contains(": u1 = ended(") && held.contains("        __site_at(") && held.contains("        __site_at3("), "{}", held);
        // a function that stores a site puts back the one it found, so
        // that after a call the place kept is the caller's statement
        assert!(held.starts_with("k: int) -> int\n    __site_was: i64 = __site_now()\n") && held.contains("    __site_at(__site_was)\n    ret "), "{}", held);
        // and the platform's own lines are in no row of the table but
        // the two checks the language's own file has, the table's first
        // rows, each with its reason (fm3 log 228; the second is a time
        // divided too fine, log 235)
        assert_eq!(l.sites[0], lower::Site { file: "platform.zero".into(), line: 62, what: "a time is too fine to hold".into() });
        assert_eq!(l.sites[1], lower::Site { file: "platform.zero".into(), line: 95, what: "a time is too fine to hold".into() });
        assert!(l.sites[2..].iter().all(|x| x.file == "h.zero"), "{:?}", l.sites);
        assert!(l.ir.contains("fn __out_len() -> i64\n") && l.ir.contains("        e: i64 = add w, 62\n") && l.ir.contains("data __site_tag = \"check at #\""), "{}", l.ir);
        assert_eq!(l.sites[2], lower::Site { file: "h.zero".into(), line: 4, what: String::new() });
        assert_eq!(l.sites[k - 1], lower::Site { file: "h.zero".into(), line: 5, what: "item {a} of {b}".into() });
        assert!(l.sites.iter().any(|x| x.line == 8 && x.what == "the stream `kept$` is full: {a} items pushed and nothing has read them") && l.sites.iter().any(|x| x.line == 8 && x.what == "a push into `kept$`, which has ended"), "{:?}", l.sites);
        // what comes back is read from the table: three words in
        // hexadecimal, the second and third the numbers handed over,
        // a negative one among them
        assert_eq!(site_said(&l.sites, &format!("a failed check at #{:016x},0000000000000009,0000000000000004", k)).as_deref(), Some("a failed check at h.zero:5: item 9 of 4"));
        assert_eq!(site_said(&l.sites, &format!("a failed check at #{:016x},fffffffffffffffe,0000000000000004", k)).as_deref(), Some("a failed check at h.zero:5: item -2 of 4"));
        assert_eq!(site_said(&l.sites, "a failed check at #0000000000000003,0000000000000000,0000000000000000").as_deref(), Some("a failed check at h.zero:4"));
        // a check of the language's own is told at the line of the
        // program that called in, the site it found and handed over,
        // with its own reason; at its own line where it found none
        assert_eq!(site_said(&l.sites, &format!("a failed check at #0000000000000001,{:016x},0000000000000000", k)).as_deref(), Some("a failed check at h.zero:5: a time is too fine to hold"));
        assert_eq!(site_said(&l.sites, "a failed check at #0000000000000001,0000000000000000,0000000000000000").as_deref(), Some("a failed check at platform.zero:62: a time is too fine to hold"));
        assert!(untraced("a failed check at platform.zero:62: a time is too fine to hold") && untraced("a failed check") && !untraced("a failed check at h.zero:5: item 9 of 4"));
        assert_eq!(site_said(&l.sites, "a failed check at h.zero:5"), None);
        assert_eq!(site_said(&l.sites, "a failed check at #00000000000000ff,0,0"), None);
        // a case that says the line holds where what is reported begins
        // with it, and is told the right line where it does not
        let at = store::Expect::CheckAt("h.zero:5".into());
        assert_eq!(judge(&at, Err("a failed check at h.zero:5: item 9 of 4".into())), (true, "(at h.zero:5: item 9 of 4)".to_string()));
        assert_eq!(judge(&at, Err("a failed check at h.zero:5".into())).0, true);
        assert_eq!(judge(&at, Err("a failed check at h.zero:50".into())), (false, "(a failed check at h.zero:50)".to_string()));
        assert_eq!(judge(&at, Err("a failed check".into())).0, false);
        assert_eq!(judge(&store::Expect::Check, Err("a failed check at h.zero:5: item 9 of 4".into())).0, true);
        // and the form refused where it names no line
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>far (9) → check at 5\n").unwrap();
        let err = store::read(&dir).err().expect("a case with no file").to_string();
        assert!(err.contains("`→ check at <file>:<line>`"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A function of one item applied to a range as a statement is
    /// written in line at the range's loop (fm3 log 191) where it gives
    /// nothing, is said once in the store and is small: at most four
    /// lines through its `if`s, no loop, no stream declared, no
    /// `existing`. Anything else is the call it was
    #[test]
    fn a_small_function_applied_to_a_range_is_written_in_line() {
        let dir = std::env::temp_dir().join(format!("probe-zero-inline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>g (3) → 6\n").unwrap();
        let g = |fns: &str, call: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("int kept$\n\n{}\non (int n) << g (int k)\n    {} ([1 through k])\n    n << kept$\n", fns, call)).unwrap();
            let ir = emit(&dir).unwrap_or_else(|e| panic!("{}: {}", fns, e));
            let from = ir.find("fn g(k: int) -> int").unwrap();
            ir[from..from + ir[from..].find("    ret").unwrap()].to_string()
        };
        // two lines with an `if` on a push: in line, the test in the loop
        let small = g("on note (int x)\n    kept$ << x if (x > 1)\n    out$ << \"n\"\n", "note");
        assert!(!small.contains("note(") && small.contains(": u1 = cmp.gt _") && small.contains(", kept, _"), "{}", small);
        // five lines: called
        let big = g("on note (int x)\n    kept$ << x\n    kept$ << x + 1\n    kept$ << x + 2\n    kept$ << x + 3\n    kept$ << x + 4\n", "note");
        assert!(big.contains("        note(_"), "{}", big);
        // a loop in it, and a stream declared in it: called
        let looped = g("on note (int x)\n    kept$ << x << (kept$ + 1) while (_ < 3)\n    for (i in [1 through x])\n        kept$ << i\n", "note");
        assert!(looped.contains("        note(_"), "{}", looped);
        let own = g("on note (int x)\n    int s$ << x << 2\n    kept$ << count s$\n", "note");
        assert!(own.contains("        note(_"), "{}", own);
        // it calls another small one, which is not applied to the range: that call stays
        let nested = g("on mark (int x)\n    kept$ << x\n\non note (int x)\n    mark (x + 1)\n", "note");
        assert!(!nested.contains("note(") && nested.contains("        mark(_"), "{}", nested);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream processor with no loop in it handed an array inside a
    /// function (fm3 question 113, log 189): its lines for each item,
    /// each once, in line at a loop over the array and with no
    /// function called; what it keeps the loop's own values, zero
    /// before the first; its last tick after the last item; pushed
    /// straight into the function's own stream, or an array where one
    /// is wanted. A task that walks is refused an array as it was
    #[test]
    fn a_processor_handed_an_array_gives_one() {
        let dir = std::env::temp_dir().join(format!("probe-zero-zapply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>g (3) → 6\n").unwrap();
        let head = "int far$\n\non (int d$) << doubled (int x$)\n    d$ << x$ * 2\n\non (int s$) << summed (int x$)\n    int t$ = t$[-1] + x$\n    s$ << t$\n\non (int e$) << closer (int x$)\n    e$ << 99 if (empty x$)\n\non (int d$) << walking (int x$)\n    loop\n        if (count x$ == 0)\n            break\n        d$ << peek x$ at (0)\n        advance x$ by (1)\n";
        let g = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}\non (int n) << g (int k)\n{}\n", head, body)).unwrap();
            let ir = emit(&dir)?;
            let from = ir.find("fn g(k: int) -> int").unwrap();
            Ok(ir[from..from + ir[from..].find("    ret").unwrap()].to_string())
        };
        // a list written out, into the function's own stream: three
        // items, three pushes, no loop, no call and no second stream
        let listed = g("    int d$ << doubled ([1, 2, k])\n    n << count d$").unwrap();
        assert!(listed.contains("    _3: int = mul _1, 2\n    push_queue_open(d, _3)\n    _4: int = mul _2, 2\n    push_queue_open(d, _4)\n    _5: int = mul k, 2\n    push_queue_open(d, _5)\n"), "{}", listed);
        assert!(!listed.contains("loop(") && listed.matches("__queue_int(").count() == 1 && !listed.contains("__z"), "{}", listed);
        // a frame of the function's own stream: read where it lies, the
        // line in the loop
        let framed = g("    int i$ << 1 << 2 << k\n    int d$ << doubled (frame i$)\n    n << count i$ * 10 + count d$").unwrap();
        assert!(framed.contains(" = frame_queue(i)\n") && !framed.contains("__copy_queue_int"), "{}", framed);
        assert!(framed.contains(": int = load _") && framed.contains("        push_queue_open(d, _"), "{}", framed);
        // what is kept is the loop's own value, zero before the first
        let kept = g("    int i$ << 1 << 2 << k\n    int d$ << summed (frame i$)\n    n << d$").unwrap();
        assert!(kept.contains("    _6: int = const 0\n    _9: int = loop(_7: index = 0, _8: int = _6)\n") && kept.contains("        _t: int = add _8, _11\n        push_queue_open(d, _t)\n") && kept.contains("        continue _12, _t\n") && !kept.contains("load _this"), "{}", kept);
        // the last tick, once, after the items
        let closed = g("    int d$ << closer ([1, k])\n    n << count d$ * 100 + d$").unwrap();
        assert!(closed.matches("push_queue_open(d, ").count() == 1 && closed.contains(": int = const 99\n"), "{}", closed);
        // an array where one is wanted: a queue of the function's own
        for body in ["    int d[] = doubled ([1, 2, k])\n    n << d[] + _", "    n << doubled ([1 through k]) + _", "    far$ << doubled ([1, 2, k])\n    n << far$", "    int d[] = doubled (doubled ([1, k]))\n    n << d[] + _"] {
            let ir = g(body).unwrap_or_else(|e| panic!("{}: {}", body, e));
            assert!(ir.contains("__queue_int(") && !ir.contains("__z"), "{}: {}", body, ir);
        }
        // a stream handed over, and a walking task handed an array
        let err = g("    int i$ << 1 << k\n    int d$ << doubled (i$)\n    n << count d$").expect_err("a stream");
        assert!(err.contains("what it is handed is an array, `doubled (frame i$)`"), "{}", err);
        let err = g("    int d$ << walking ([1, k])\n    n << count d$").expect_err("a walking task");
        assert!(err.contains("'walking' reads 'x$' as a stream it moves"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream looks back (fm3 question 75 rule 2, log 125): `x$[-1]`
    /// is the item one before the present one, zero before there is
    /// anything, and a stream may be said in terms of its own earlier
    /// items. What is kept is the compiler's: one earlier value for a
    /// look one back, two for two, none for a stream read only now,
    /// each a field of the wiring's own in the context. The lines may
    /// be written in any order and give the same text. And what cannot
    /// be said is refused, naming the line: a stream at its own
    /// present item, a circle through present items, an index forward
    /// of now, and one that is worked out
    #[test]
    fn a_stream_looks_back_and_never_forward() {
        let dir = std::env::temp_dir().join(format!("probe-zero-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 2\n").unwrap();
        let head = "int x$\nint d$ = made(x$)\n\non (int n) << f()\n    x$ << 1 << 2\n    n << peek d$ at (0)\n\non (int d$) << made (int x$)\n";
        let with = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\n", head, body)).unwrap();
            emit(&dir)
        };
        // two back keeps two, one back one, and a stream read only now none
        let ir = with("    int a$ = x$ + a$[-1] + a$[-2]\n    int b$ = a$ - x$[-1]\n    int c$ = b$ * 2\n    d$ << c$").unwrap();
        let ctx = &ir[ir.find("type __ctx = struct").unwrap()..ir.find("data __ctx_mem").unwrap()];
        assert!(ctx.contains("    __z1_x_1: int\n    __z1_a_1: int\n    __z1_a_2: int\n"), "{}", ctx);
        assert!(!ctx.contains("__z1_x_2") && !ctx.contains("__z1_b_") && !ctx.contains("__z1_c_"), "{}", ctx);
        assert!(ir.contains("fn __z1_each(_x: int, __x_b1: int, __a_b1: int, __a_b2: int) -> int\n"), "{}", ir);
        // the state is fetched once a statement, carried through its
        // items and stored once; no read in the body is checked
        let f = &ir[ir.find("fn f() -> int").unwrap()..];
        let f = &f[..f[1..].find("\nfn ").map_or(f.len(), |i| i + 1)];
        assert_eq!(f.matches("= get ").count() - f.matches("get _1, __enabled_h").count() - f.matches(", d\n").count() - f.matches(", ring\n").count(), 3, "{}", f);
        assert_eq!(f.matches("store ").count(), 1, "{}", f);
        let each = &ir[ir.find("fn __z1_each(").unwrap()..];
        let each = &each[..each[1..].find("\nfn ").map_or(each.len(), |i| i + 1)];
        assert!(!each.contains("check") && !each.contains("peek"), "{}", each);
        // the lines in any order: the same text
        let shuffled = with("    d$ << c$\n    int c$ = b$ * 2\n    int b$ = a$ - x$[-1]\n    int a$ = x$ + a$[-1] + a$[-2]").unwrap();
        assert_eq!(ir, shuffled);
        for (body, said) in [
            ("    int k$ = k$ + x$\n    d$ << k$", "h.zero:9: 'k$' is said in terms of itself at the present item: a stream may look back at itself, `k$[-1]`, and never at itself now"),
            ("    int a$ = b$ + x$\n    int b$ = a$ * 2\n    d$ << b$", "h.zero:9: 'a$' is said in terms of 'b$' at the present item, and 'b$' in terms of 'a$': a circle. One of them must look back, `b$[-1]`"),
            ("    d$ << x$[1]", "h.zero:9: 'x$[1]' would be an item that has not come: a stream processor looks back, `x$[-1]`, and never forward"),
            ("    d$ << x$[0]", "h.zero:9: 'x$[0]' is the present item: write `x$`"),
            ("    int k$ = x$ * 2\n    d$ << k$[k$]", "h.zero:10: the index of 'k$' is worked out: in a stream processor an index is a literal, `k$[-1]` the item one before; an index that is not a literal is not built in this hop"),
            ("    loop\n        if ([count] (frame x$) == 0)\n            break\n        d$ << x$[-1]\n        advance x$ by (1)", "h.zero:12: this body is written both ways: line 9 walks its input (a loop), and line 12 holds for every item (`x$[-1]`). A stream processor either walks what has arrived, with loops and the reader's words, or says each stream once with no loop: write it one way"),
        ] {
            let err = with(body).expect_err(body);
            assert!(err.ends_with(said), "{}: {}", body, err);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `if` on a push (fm3 question 75 rule 3, log 126; `when` until question 79): the item goes
    /// out where the condition holds. In a stream processor it is the
    /// push under a branch in the function of one item; in a plain
    /// function it is the `if` round the push. A push with `while` and
    /// `if` both is refused, and so is `if` on an edge
    #[test]
    fn a_push_goes_out_when_its_condition_holds() {
        let dir = std::env::temp_dir().join(format!("probe-zero-when-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int x$\nint d$ = kept(x$)\nint p$\n\non (int d$) << kept (int x$)\n    d$ << x$ if (x$ > 0)\n\n";
        let with = |more: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\n", head, more)).unwrap();
            emit(&dir)
        };
        let ir = with("on (int n) << f (int k)\n    x$ << k\n    p$ << k if (k > 2)\n    n << [count] (frame d$) + [count] (frame p$)").unwrap();
        assert!(ir.contains("fn __z1_each(_x: int)\n    _this: ptr = context()\n    _1: u1 = cmp.gt _x, 0\n    if _1\n        _2: __ctx = load _this\n        _3: int$ = get _2, d\n        push_queue_open(_3, _x)\n    ret\n"), "{}", ir);
        assert!(ir.contains("    _3: u1 = cmp.gt k, 2\n    if _3\n        _4: __ctx = load _this\n        _5: int$ = get _4, p\n        push_queue_open(_5, k)\n"), "{}", ir);
        for (more, said) in [
            // `if` goes with `while` since fm3 log 148, and comes first
            ("on f (int k)\n    p$ << k while (_ < 3) if (k > 2)", "h.zero:9: `if` comes first on a push, then how often: `x$ << item if (condition) while (...)`"),
            // the word a push took before question 79, after each kind of item
            ("on f (int k)\n    p$ << k when (k > 2)", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            ("on f (int k)\n    p$ << x$ when (k > 2)", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            ("on f (int k)\n    p$ << (k + 1) when (k > 2)", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            ("on f (int k)\n    p$ << twice (k) when (k > 2)\n\non (int n) << twice (int k)\n    n << k * 2", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            // the push's `if` takes no `then`
            ("on f (int k)\n    p$ << k if (k > 2) then (1) else (2)", "h.zero:9: an `if` after a push's items says whether the push happens, and takes no `then`: the value that is one thing or another is `x$ << a if (c) else b`"),
            // a name with a word that ends every phrase (log 126)
            ("on (int n) << one if (int k)\n    n << k", "h.zero:8: 'if' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written"),
            ("on (int n) << lines in any order()\n    n << 1", "h.zero:8: 'in' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written"),
        ] {
            let err = with(more).expect_err(more);
            assert!(err.ends_with(said), "{}: {}", more, err);
        }
        // the three `if`s, told by where the word stands (fm3 log 140):
        // the first of a line is the statement, one where a value is
        // wanted is the expression, one where a value has ended is the
        // push's. `p$ << (a if (c) else b) if (d)` is both
        let both = with("on (int n) << f (int k)\n    p$ << (10 if (k > 5) else 20) if (k > 2)\n    p$ << (10 if (k > 5) else k + 1) if (k > 2)\n    if (k > 0)\n        p$ << 1 << (2 if (k > 5) else 3)\n    n << [count] (frame p$)").unwrap();
        let at = both.find("fn f(k: int) -> int").unwrap();
        let f = &both[at..at + both[at..].find("\n\n").unwrap()];
        // each of the first two lines: the condition, a branch, and
        // under it the value's own branch and one push
        assert_eq!(f.matches("cmp.gt k, 2\n").count(), 2, "{}", f);
        assert_eq!(f.matches("cmp.gt k, 5\n").count(), 3, "{}", f);
        assert_eq!(f.matches("push_queue_open(").count(), 4, "{}", f);
        // `when` is a word of a name where a name is declared with it,
        // and of nothing else: the call and the push's `if` on one line
        let named = with("on (int n) << pushed when (int k)\n    n << k + 1\n\non (int n) << f (int k)\n    p$ << pushed when (k) if (k > 2)\n    n << pushed when (k) + [count] (frame p$)").unwrap();
        assert_eq!(named.matches(": int = pushed_when(k)\n").count(), 2, "{}", named);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `forever` decides (fm3 question 79, log 141). A `<<` sends once
    /// each time its line runs, and the word makes it stand: a wiring
    /// line says it and is the edge it was, with `if` before the word
    /// a filter. Without the word a feature-scope `<<` with a stream
    /// on its right is refused, saying both things it could be; and
    /// the word is refused in a function, in a stream processor, on a
    /// declaration, with `while`, before `if`, on a line of values,
    /// and on a stream that feeds itself, at a rate and with none
    #[test]
    fn forever_decides() {
        let dir = std::env::temp_dir().join(format!("probe-zero-forever-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int a$\nint b$\nint i$ at (1 hz)\nint d$ = dd(a$)\n\non (int d$) << dd (int x$)\n    d$ << x$\n\n";
        let f = "\non (int n) << f (int k)\n    a$ << k\n    n << [count] (frame b$) + [count] (frame d$)\n";
        let with = |lines: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}{}{}\n", head, lines, f, body)).unwrap();
            emit(&dir)
        };
        // the wiring line and the standing filter: the edge's function
        // of one item, the filter's push under its condition, the
        // source's own name in the condition the item
        // (an edge nothing calls is not written, fm3 log 195: `g` pushes)
        let ir = with("b$ << a$ forever\nb$ << (i$ << 0) if (i$ > 2) forever\n", "\non g()\n    i$ << 1\n").unwrap();
        assert!(ir.contains("fn __edge3(__item: int)\n    _this: ptr = context()\n    _1: u1 = cmp.gt __item, 2\n    if _1\n        _2: __ctx = load _this\n        _3: int$ = get _2, b\n        push_queue_open(_3, __item)\n"), "{}", ir);
        let once = "has a stream on its right and no `forever`. If it is wiring, everything that arrives in 'a$' going on into 'b$', write `b$ << a$ forever`. If it is one push when the store starts, of what 'a$' holds then, that is what the line says (fm3 question 79) and it is not built: push it from a function";
        for (lines, body, said) in [
            ("b$ << a$\n", "", format!("h.zero:9: 'b$ << a$' {}", once)),
            ("int c$ << a$\n", "", format!("h.zero:9: 'c$ << a$' {}; here, declare the stream, `int c$`, and wire it on a line of its own", once.replace("into 'b$'", "into 'c$'").replace("`b$ << a$ forever`", "`c$ << a$ forever`"))),
            ("int c$ << a$ forever\n", "", "h.zero:9: `forever` on a declaration is not built: declare the stream and wire it on a line of its own, `c$ << x$ forever`".to_string()),
            ("b$ << a$ forever if (a$ > 0)\n", "", "h.zero:9: `forever` is the last word of its line: `x$ << item if (condition) forever`".to_string()),
            ("b$ << a$ while (_ > 0) forever\n", "", "h.zero:9: a push takes `while` or `forever`, not both: `while` is `forever` with an end".to_string()),
            ("b$ << a$ forever while (_ > 0)\n", "", "h.zero:9: a push takes `while` or `forever`, not both: `while` is `forever` with an end".to_string()),
            ("b$ << a$ while (_ > 0)\n", "", "h.zero:9: a line at feature scope that stands until its `while` fails is not built: wiring moves every item its stream receives, `x$ << y$ forever`".to_string()),
            ("b$ << 1 forever\n", "", "h.zero:9: nothing on the right of 'b$ << 1' is a stream: `forever` makes a push happen again whenever what is on its right has something new, and a value never has".to_string()),
            ("b$ << 1\n", "", "h.zero:9: a push at feature scope happens once, when the store starts (fm3 question 79), and on a line of its own that is not built: a stream's first items go on its declaration, `int b$ << ...`, and a line that stands is wiring, `b$ << x$ forever`".to_string()),
            // a stream feeding itself: with no rate it never ends
            ("b$ << b$ + 1 forever\n", "", "h.zero:9: a push into 'b$' that reads 'b$' and stands forever would never end: nothing else on its right paces it, and 'b$' has no rate to. For one more item write it with no `forever`, in a function; for a clock give the stream a rate, `int b$ at (1 hz)`".to_string()),
            ("b$ << b$ forever\n", "", "h.zero:9: a push into 'b$' that reads 'b$' and stands forever would never end: nothing else on its right paces it, and 'b$' has no rate to. For one more item write it with no `forever`, in a function; for a clock give the stream a rate, `int b$ at (1 hz)`".to_string()),
            // ... and at a rate it is a clock, not built
            // in a function, and under `if` there
            ("b$ << a$ forever\n", "\non g()\n    b$ << a$ forever", "h.zero:16: `forever` in a function is a line that would set up a standing connection each time the function runs: not built. Wire it at feature scope, where it stands from the start".to_string()),
            ("b$ << a$ forever\n", "\non g (int k)\n    b$ << k if (k > 0) forever", "h.zero:16: `forever` in a function is a line that would set up a standing connection each time the function runs: not built. Wire it at feature scope, where it stands from the start".to_string()),
        ] {
            let err = with(lines, body).expect_err(lines);
            assert!(err.ends_with(&said), "{}{}: {}", lines, body, err);
        }
        // in a stream processor, which is wired and so stands already
        std::fs::write(dir.join("h/h.zero"), format!("{}b$ << a$ forever\n{}", head.replace("    d$ << x$\n", "    d$ << x$ forever\n"), f)).unwrap();
        let err = emit(&dir).expect_err("a processor");
        assert!(err.ends_with("h.zero:7: a stream processor's lines hold for every item already, because the processor is wired: its pushes take no `forever`"), "{}", err);
        // a function may not have the word in its name: it ends a phrase
        let err = with("b$ << a$ forever\n", "\non wait forever()\n    b$ << 1").expect_err("a name");
        assert!(err.ends_with("h.zero:15: 'forever' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every function is declared with `<<` and gives its result by
    /// pushing it (fm3 question 77 (a), log 151 to 153): a result with
    /// no `$` makes a plain function of a `<<` declaration; what is
    /// one value is pushed once, a name that is no result not at all;
    /// and the form before, `=` on the first line or on a result, is
    /// refused with the line to write. (While both stood, each of
    /// these texts written the old way emitted the same IR, asserted
    /// here at probe `fa90584`)
    #[test]
    fn a_function_gives_its_result_by_pushing_it() {
        let dir = std::env::temp_dir().join(format!("probe-zero-pushed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n").unwrap();
        let with = |text: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}\n", text)).unwrap();
            emit(&dir)
        };
        // each shape the new way, and the old, which is refused
        let pairs = [
            ("on (int d) << double (int x)\n    d << x * 2", "on (int d) = double (int x)\n    d = x * 2"),
            ("on (int s) << sign of (int x)\n    s << -1 if (x < 0)\n         else 1 if (x > 0)\n         else 0", "on (int s) = sign of (int x)\n    if (x < 0)\n        s = -1\n    else if (x > 0)\n        s = 1"),
            ("on (int r) << first (int a) or (int b)\n    r << a if (a > 0)\n         else b", "on (int r) = first (int a) or (int b)\n    if (a > 0)\n        r = a\n    r = b"),
            ("on (int r) << first (int a) or (int b)\n    r << a if (a > 0) else b", "on (int r) = first (int a) or (int b)\n    if (a > 0)\n        r = a\n    r = b"),
            ("on (int p) << above (int n)\n    p << loop (int q = 1) yields q\n        if (q > n)\n            break\n        continue (q * 2)", "on (int p) = above (int n)\n    loop (int q = 1)\n        if (q > n)\n            p = q\n        continue (q * 2)"),
            ("on (int g) << gcd of (int a) with (int b)\n    g << loop (int x = a, int y = b) while (y != 0) yields x\n        continue (y, x % y)", "on (int g) = gcd of (int a) with (int b)\n    g = loop (int x = a, int y = b) while (y != 0) yields x\n        continue (y, x % y)"),
            ("on (int q, int r) << divide (int a) by (int b)\n    r << a % b\n    q << a / b\n\non (int q, int r) << both()\n    q, r << divide (17) by (5)", "on (int q, int r) = divide (int a) by (int b)\n    r = a % b\n    q = a / b\n\non (int q, int r) = both()\n    q, r = divide (17) by (5)"),
            ("on (int n) << sum of (int x[])\n    n << x[] + _", "on (int n) = sum of (int x[])\n    n = x[] + _"),
        ];
        for (new, old) in pairs {
            with(new).unwrap_or_else(|e| panic!("{}: {}", new, e));
            let err = with(old).err().unwrap_or_else(|| panic!("{} compiled", old));
            assert!(err.contains("h.zero:1: a function is declared with `<<` and gives its result by pushing it; `=` says what a name is (fm3 question 77). Write `on ("), "{}: {}", old, err);
        }
        // the two refusals whole, each with the program's own line as
        // it is to be written; either old half is refused by itself
        let err = with("on (int d) = double (int x)\n    d << x * 2").unwrap_err();
        assert!(err.ends_with("h.zero:1: a function is declared with `<<` and gives its result by pushing it; `=` says what a name is (fm3 question 77). Write `on (int d) << double (int x)`"), "{}", err);
        let err = with("on (int d) << double (int x)\n    d = x * 2").unwrap_err();
        assert!(err.ends_with("h.zero:2: 'd' is a result, and a result is given by pushing it; `=` says what a name is (fm3 question 77). Write `d << x * 2`"), "{}", err);
        let err = with("on (int n) << f (int k)\n    n = loop (int i = 0) while (i < k) yields i\n        continue (i + 1)").unwrap_err();
        assert!(err.contains("h.zero:2: 'n' is a result, and a result is given by pushing it; `=` says what a name is (fm3 question 77). Write `n << loop (int i = 0) while (i < k) yields i`"), "{}", err);
        let err = with("on (int q, int r) << f (int k)\n    int a = k\n    q, r = g (a)").unwrap_err();
        assert!(err.contains("h.zero:3: 'q' is a result, and a result is given by pushing it; `=` says what a name is (fm3 question 77). Write `q, r << g (a)`"), "{}", err);
        // `=` still says what a name is, and a local declared before
        // may be given by a loop
        let ir = with("on (int n) << f (int k)\n    int half = k / 2\n    int t = 0\n    t = loop (int i = 0) while (i < half) yields i\n        continue (i + 1)\n    n << t").unwrap();
        assert!(ir.contains("    half: int = div k, 2\n"), "{}", ir);
        let ir = with(pairs[0].0).unwrap();
        assert!(ir.contains("fn double(x: int) -> int\n    d: int = mul x, 2\n    ret d\n"), "{}", ir);
        // a push with its condition is the last thing these do, and
        // each arm ends the function where it stands
        let ir = with(pairs[2].0).unwrap();
        assert!(ir.contains("    if _1\n        ret a\n    else\n        ret b\n"), "{}", ir);
        let ir = with(pairs[1].0).unwrap();
        assert!(ir.contains("            s_3: int = const 0\n            ret s_3\n"), "{}", ir);
        // a `$` on the result is still a task; a function that gives an
        // array says so on its result and is a plain function (fm3
        // questions 87 and 90, log 159)
        let ir = with("on (int i$) << count up to (int n)\n    i$ << 1 << (i$ + 1) while (_ <= n)\n\non (int r[]) << squares to (int k)\n    r[] << [1 through k] * [1 through k]").unwrap();
        assert!(ir.contains("fn count_up_to(i: int$, n: int, __hz: i64)") && ir.contains("fn squares_to(k: int) -> int$"), "{}", ir);
        let f = |body: &str| format!("int port = 8\n\non (int y) << f (int x)\n{}", body);
        let two = |body: &str| format!("on (int q, int r) << f (int x)\n{}", body);
        for (text, message) in [
            (f("    y << x << 2"), "h.zero:4: 'y' is one value, given once: this line pushes it twice. What takes more than one item is a stream, `y$`"),
            (f("    y << x (3) times"), "h.zero:4: `(n) times` on the push of 'y' would give it more than once, and 'y' is one value, given once: what takes more than one item is a stream, `y$`"),
            (f("    y << x while (_ < 3)"), "h.zero:4: `while` on the push of 'y' would give it more than once"),
            (f("    y << x until (y > 3)"), "h.zero:4: `until` on the push of 'y' would give it more than once"),
            (f("    y << x forever"), "h.zero:4: `forever` on the push of 'y' would make it stand and give it again and again, and 'y' is one value, given once: what takes more than one item is a stream, `y$`"),
            (f("    y << x if (x > 0) then (1)"), "h.zero:4: an `if` after a pushed value says whether the push happens, and takes no `then`"),
            (f("    int half = x / 2\n    half << 1\n    y << half"), "h.zero:5: 'half' is not pushed into: `=` says what a name is, where it is declared, `int half = ...`, and it keeps that value. What `<<` sends into is a stream, `half$`, or a result of the function"),
            (f("    x << 1\n    y << x"), "h.zero:4: 'x' is a parameter: it is what the function was handed, and is not pushed into"),
            (f("    port << 1\n    y << x"), "h.zero:4: 'port' is a variable, and a variable keeps the value it was declared with (fm3 question 70): what changes is a stream. Declare it `int port$ << 8` and push its next value, `port$ << 1`"),
            (f("    z << 1\n    y << x"), "h.zero:4: 'z' is not declared: a function's result is named on its first line, `on (int z) << ...`, and a stream is `z$`"),
            (f("    loop (int i = 0)\n        i << 1\n        break\n    y << x"), "h.zero:5: 'i' is the loop's own: it is not pushed into. Give its next value with `continue (...)`"),
            (f("    for (i in [1 through 3])\n        i << 1\n    y << x"), "h.zero:5: 'i' is the item of the `for`: it steps by itself and is not pushed into"),
            (f("    int s$ << 1 << 2\n    s << 3\n    y << x"), "h.zero:5: 's' is written without its `$`: the stream is `s$`, and a push into it is `s$ << ...`"),
            (f("    y << x\n    y << 2"), "h.zero:5: 'y' is pushed twice: a function gives each of its results once, at the top level of its body (fm3 question 88)"),
            (two("    q << x\n    q << 2\n    r << 1"), "h.zero:3: 'q' is pushed twice: a function gives each of its results once, at the top level of its body (fm3 question 88)"),
            (two("    q << x if (x > 0) else 0\n    q << 2\n    r << 1"), "h.zero:3: 'q' is pushed twice: a function gives each of its results once"),
            ("on (int y, int z$) << f (int x)\n    y << x".to_string(), "h.zero:1: a task produces one stream"),
        ] {
            let err = with(&text).err().unwrap_or_else(|| panic!("{} compiled", text));
            assert!(err.contains(message), "{}: {}", text, err);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two structures compared, a list of them written out, and two
    /// arrays of them compared whole (fm3 question 108, log 181): every
    /// field the same, a field at a time; the program's own `==` first
    /// where it declares one; a field that is a string refuses the
    /// comparison, naming it
    #[test]
    fn two_structures_are_compared_a_field_at_a_time() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-struct-same");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let types = "type pair =\n    int a, b\n\ntype box =\n    pair lo\n    bool open\n\ntype named =\n    int id\n    string name\n\ntype odd =\n    int v\n\n";
        let f = |more: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}on (bool n) << f (pair p, pair q, box s, box t)\n{}\n", types, more, body)).unwrap();
            emit(&dir)
        };
        let refused = |body: &str, what: &str| {
            let e = f("", body).err().unwrap_or_else(|| panic!("not refused: {}", body));
            assert!(e.contains(what), "{}: {}", body, e);
        };
        // a field at a time, in the order declared, joined by `and`
        let ir = f("", "    n << p == q").unwrap();
        assert!(ir.contains("    _1: int = get p, a\n    _2: int = get q, a\n    _3: u1 = cmp.eq _1, _2\n    _4: int = get p, b\n    _5: int = get q, b\n    _6: u1 = cmp.eq _4, _5\n    _7: u1 = and _3, _6\n    ret _7\n"), "{}", ir);
        // `!=` is its opposite
        let ir = f("", "    n << p != q").unwrap();
        assert!(ir.contains("    _7: u1 = and _3, _6\n    n: u1 = cmp.eq _7, 0\n"), "{}", ir);
        // a field that is a structure, the same way inside
        let ir = f("", "    n << s == t").unwrap();
        assert!(ir.contains("    _1: pair = get s, lo\n    _2: pair = get t, lo\n    _3: int = get _1, a\n    _4: int = get _2, a\n") && ir.contains("    _10: u1 = get s, open\n    _11: u1 = get t, open\n    _12: u1 = cmp.eq _10, _11\n    _13: u1 = and _9, _12\n    ret _13\n"), "{}", ir);
        // the program's own operator comes first, between two and
        // between two arrays
        let own = "on (bool b) << (odd x) == (odd y)\n    b << x.v % 2 == y.v % 2\n\n";
        let ir = f(own, "    odd xs[] = [odd(1), odd(4)]\n    n << odd(1) == odd(3) and xs[] [==] [odd(3), odd(6)]").unwrap();
        // (the list written out is compared an item at a time, a call
        // an item, fm3 log 183)
        // (`odd(1) == odd(3)`, two made from literals, is the one line
        // of its function written where it stands, fm3 log 213)
        assert_eq!(ir.matches(": u1 = eq_odd(").count(), 2, "{}", ir);
        // a list of structures is an array of them, and two are
        // compared whole, each pair a field at a time
        let ir = f("", "    pair ps[] = [pair(1, 2), pair(3, 4)]\n    n << ps[] [==] [p, q]").unwrap();
        for l in ["= __queue_pair(", ": pair = load ", ": u1 = loop(", ": u1 = and ", "                break 0\n"] {
            assert!(ir.contains(l), "{}: {}", l, ir);
        }
        // a field that is a string is compared as two strings are, by
        // the `==` the language declares on `string` (fm3 question
        // 100): the lengths, then the characters
        let ir = f("", "    named x = named(1, \"a\")\n    named y = named(1, \"b\")\n    n << x == y").unwrap();
        let read = ir.split("\nfn f(").nth(1).unwrap().split("\nfn ").next().unwrap().to_string();
        assert!(read.contains(": u1 = cmp.eq ") && read.contains(": u1 = and ") && read.contains(" = len _"), "{}", read);
        refused("    n << p == odd(1)", "no '==' is defined on a pair and an odd: with none declared, '==' is of two of one structure, `(pair) == (pair)`, every field the same");
        refused("    n << p < q", "no '<' is defined on a pair and a pair: no operator is declared on a pair, `on (bool r) << (pair a) < (pair b)`");
        refused("    pair ps[] = [p]\n    odd os[] = [odd(1)]\n    n << ps[] [==] os[]", "`[==]` compares two arrays of one type of item: these hold pair and odd");
        refused("    pair ps[] = [p, odd(1)]\n    n << true", "the items are pair, this one is a odd");
    }

    /// An array compared with a list, with nothing copied (fm3 log
    /// 183): a list written out as one side is no array, its length a
    /// number in the text and its items compared where they stand, in
    /// a block that runs once and leaves at the first that differs; a
    /// frame is read where the stream's items lie, as one side or as
    /// an array that is nothing else, wherever nothing can push into
    /// the stream before its items are read, and copied wherever
    /// something might
    #[test]
    fn an_array_is_compared_with_a_list_where_each_stands() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-whole-in-place");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let f = |more: &str, body: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("type pair =\n    int a, b\n\nint y$\n\non (int n) << bump()\n    y$ << 9\n    n << count y$\n\n{}on (bool n) << f()\n    int x$ << 1 << 2\n{}\n", more, body)).unwrap();
            let ir = emit(&dir).unwrap();
            let at = ir.find("\nfn f() -> u1\n").unwrap();
            ir[at + 1..at + 1 + ir[at + 1..].find("\nfn ").unwrap()].to_string()
        };
        // the list is no array and the frame no copy: the length once,
        // then each item against its value, leaving at the first
        let ir = f("", "    n << frame x$ [==] [1, 2]");
        assert!(ir.contains("    _3: int[], _4: index, x_2: int$ = frame_queue(x)\n    _5: index = len _3\n    _6: u1 = cmp.eq _5, 2\n    n: u1 = if _6\n        _7: u1 = loop()\n            _8: int = load _3, 0\n            _9: u1 = cmp.ne _8, 1\n            if _9\n                break 0\n            _10: int = load _3, 1\n            _11: u1 = cmp.ne _10, 2\n            if _11\n                break 0\n            break 1\n        yield _7\n    else\n        yield 0\n    ret n"), "{}", ir);
        // an empty list is the length alone
        let ir = f("", "    n << frame x$ [==] []");
        assert!(ir.contains("    _5: index = len _3\n    n: u1 = cmp.eq _5, 0\n    ret n"), "{}", ir);
        // an item that is not a constant is worked out before anything
        // is compared, in the order written
        let ir = f("", "    int k = 1\n    n << frame x$ [!=] [k, k + 1]");
        assert!(ir.contains(": int = add k, 1\n") && ir.find("add k, 1").unwrap() < ir.find("loop()").unwrap() && ir.contains("    n: u1 = cmp.eq "), "{}", ir);
        // a structure's construction is compared against its own
        // arguments: a `get` of the side in memory, and no `pack`
        let ir = f("", "    pair p$ << pair(1, 2)\n    n << frame p$ [==] [pair(1, 2)]");
        assert_eq!(ir.matches(" = pack ").count(), 1, "{}", ir);
        assert!(ir.contains(": int = get _") && ir.contains(", a\n") && ir.contains(": u1 = cmp.eq ") && ir.contains("            else\n                break 0\n") && !ir.contains("__copy_"), "{}", ir);
        // an array that is nothing but a frame is the same view
        let ir = f("", "    int g[] = frame x$\n    n << g[] [==] [1, 2]");
        assert!(!ir.contains("__copy_") && ir.contains(" = load _3, 0\n"), "{}", ir);
        let ir = f("", "    int g[] = frame x$\n    bool same = g[] [==] [1, 2]\n    n << same and g[] [!=] [2, 1]");
        assert!(!ir.contains("__copy_"), "{}", ir);
        // ... and an array it is, with its copy, where it is used any
        // other way, where something is pushed before its last use,
        // and where a function is called there
        for body in [
            "    int g[] = frame x$\n    n << g[] [==] [1, 2] and [count] (g[]) == 2",
            "    int g[] = frame x$\n    x$ << 3\n    n << g[] [==] [1, 2]",
            "    int g[] = frame x$\n    int k = bump()\n    n << g[] [==] [1, 2]",
            "    int g[] = frame x$\n    n << g[] [==] [1, bump()]",
            "    int g[] = frame y$\n    if (count y$ == 0)\n        y$ << 3\n    n << g[] [==] [1, 2]",
        ] {
            let ir = f("", body);
            assert!(ir.contains("= __copy_queue_int("), "{}: {}", body, ir);
        }
        // `frame x$` on the left is lowered before the right: copied
        // where the right calls a function, and not on the right
        let ir = f("", "    n << frame y$ [==] [1, bump()]");
        assert!(ir.contains("= __copy_queue_int("), "{}", ir);
        let ir = f("", "    n << [1, bump()] [==] frame y$");
        assert!(!ir.contains("__copy_") && ir.find("bump()").unwrap() < ir.find("frame_queue(").unwrap(), "{}", ir);
        // the program's own `==` is a call for each item: the copy stays
        let own = "on (bool b) << (pair x) == (pair y)\n    b << x.a == y.a\n\n";
        let ir = f(own, "    pair p$ << pair(1, 2)\n    n << frame p$ [==] [pair(1, 5)]");
        assert!(ir.contains("= __copy_queue_pair(") && ir.contains(": u1 = eq_pair("), "{}", ir);
        // two frames are two views
        let ir = f("", "    n << frame x$ [==] frame y$");
        assert!(!ir.contains("__copy_") && ir.matches("frame_queue(").count() == 2, "{}", ir);
    }

    /// `[==]` and `[!=]` (fm3 question 77, log 164): two arrays compared
    /// as wholes, one bool; the parser tells the bracketed operator
    /// from a list written out, and everything else in brackets is
    /// refused as not ruled
    #[test]
    fn two_arrays_are_compared_as_wholes() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-whole-ops");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) << f()\n    int a[] = [1, 2, 3]\n    int b[] = [1, 5, 3]\n    int x$ << 1 << 2\n{}\n", body)).unwrap();
            emit(&dir)
        };
        let refused = |body: &str, what: &str| {
            let e = f(body).err().unwrap_or_else(|| panic!("not refused: {}", body));
            assert!(e.contains(what), "{}: {}", body, e);
        };
        // one bool: the lengths, and then a loop that leaves at the
        // first pair that differs
        let ir = f("    n << (1 if (a[] [==] b[]) else 0)").unwrap();
        for l in [": u1 = cmp.eq ", ": u1 = loop(", "                break 1\n", ": u1 = cmp.ne ", "                break 0\n", "    else\n        yield 0\n"] {
            assert!(ir.contains(l), "{}: {}", l, ir);
        }
        assert!(!ir.contains("__queue_int(1000000, _"), "nothing is made for it: {}", ir);
        let ir = f("    n << (1 if (a[] [!=] b[]) else 0)").unwrap();
        assert!(ir.contains(", 0\n    n: int = if "), "{}", ir);
        // a list and a range written out, a frame, and a list beside it
        for line in ["a[] [==] [1, 2, 3]", "[1, 2, 3] [==] a[]", "a[] [==] [1 through 3]", "frame x$ [==] a[]", "a[] [==] b[] + [0, 0, 1]"] {
            f(&format!("    n << (1 if ({}) else 0)", line)).unwrap_or_else(|e| panic!("{}: {}", line, e));
        }
        // a list written out is still a list, a negative first item too
        assert!(f("    int c[] = [-1, 2]\n    int d[] = [- 1]\n    n << [count] (c[]) + [count] (d[])").is_ok());
        // both sides are arrays
        refused("    n << (1 if (a[] [==] 2) else 0)", "h.zero:5: `[==]` asks whether two arrays are the same, and this side is one value (fm3 question 77): both sides are arrays, `a[] [==] b[]`. One item is compared plainly, `a[k] == v`");
        refused("    n << (1 if (a[] [!=] x$) else 0)", "`[!=]` asks whether two arrays are the same, and 'x$' is a stream, its items still arriving: the array of what has arrived is `frame x$` (fm3 question 77)");
        refused("    float h[] = [1.5]\n    n << (1 if (a[] [==] h[]) else 0)", "`[==]` compares two arrays of one type of item: these hold int and float");
        // the other operators in brackets are not ruled
        refused("    n << (1 if (a[] [<] b[]) else 0)", "h.zero:5: `[<]` is not ruled as to what it means on two arrays (fm3 question 77): `[==]` and `[!=]` are built, are the two the same. Applied to each pair an operator is written plainly, `a[] < b[]`");
        refused("    int c[] = a[] [+] b[]\n    n << [count] (c[])", "`[+]` is not ruled as to what it means on two arrays");
        // the plain comparison where one bool is wanted says what to write
        refused("    if (a[] == b[])\n        out$ << 1\n    n << 1", "h.zero:5: `if (a[] == b[])`: `==` is applied to each pair of items and gives a bool for each, and one is wanted here (fm3 question 77). Whether the two arrays are the same is `[==]`: write `if (a[] [==] b[])`");
        refused("    if (a[] != [1, 2])\n        out$ << 1\n    n << 1", "Whether the two arrays differ is `[!=]`: write `if (a[] [!=] [1, 2])`");
        // ... and where an array is wanted its answer is an array of bool
        refused("    out$ << (a[] == b[])\n    n << 1", "`==` between arrays is applied to each pair and gives a bool for each, and an array of bool is not built (fm3 question 77). Whether the two arrays are the same, one bool, is `[==]`");
    }

    /// `else` on a push (fm3 question 88, log 169): `x << a if (c) else
    /// b`, read from the left, a case a line on lines that begin `else`
    /// indented under the push. It is the `if` statement with a push in
    /// each arm in the tree, so it lowers to that statement's lines,
    /// for a result and for a stream; in a stream processor it is one
    /// item; and a loop word with it is refused, both readings said
    #[test]
    fn a_push_takes_else() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-push-else");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (1) → 1\n").unwrap();
        let emitted = |code: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            emit(&dir)
        };
        let refused = |code: &str, what: &str| {
            let e = emitted(code).err().unwrap_or_else(|| panic!("not refused: {}", code));
            assert!(e.contains(what), "{}: {}", code, e);
        };
        // a table, a case a line, on the line and under it, and the
        // same on one line: one text
        let table = emitted("on (int k) << f (int c)\n    k << 0 if (c <= 32)\n         else 3 if (c > 122)\n         else 1 if (c >= 97)\n         else 3\n").unwrap();
        assert_eq!(emitted("on (int k) << f (int c)\n    k << 0 if (c <= 32) else 3 if (c > 122) else 1 if (c >= 97) else 3\n").unwrap(), table);
        assert_eq!(emitted("on (int k) << f (int c)\n    k << 0 if (c <= 32) else 3 if (c > 122)\n      else 1 if (c >= 97) else 3\n").unwrap(), table);
        // ... which was the text of the `if` statements it is written
        // in place of while those stood (fm3 log 169): a result's push
        // under one is refused (log 171, `a_result_is_pushed_once`)
        assert!(table.contains("    if _1\n        k: int = const 0\n        ret k\n    else\n        _2: u1 = cmp.gt c, 122\n        if _2\n            k_2: int = const 3\n            ret k_2\n"), "{}", table);
        // into a stream: one or the other, the `if` covering the whole
        // push before it (question 91)
        let either = emitted("on f (int c)\n    out$ << 1 << 2 if (c > 0)\n         else 3 if (c < 0)\n         else 4\n    out$ << 5\n").unwrap();
        assert!(either.contains("    if _1\n") && either.contains("        __out__int(_3)\n    else\n        _4: u1 = cmp.lt c, 0\n        if _4\n") && either.contains("        else\n            _6: int = const 4\n            __out__int(_6)\n    _7: int = const 5\n"), "{}", either);
        // in a stream processor: one item, each arm worked out where
        // it is chosen; with no last `else`, pushed where a case holds
        let head = "int x$\nint e$ = picked(x$)\n\non (int n) << f (int c)\n    x$ << c\n    n << [count] (frame e$)\n\n";
        let p = emitted(&format!("{}on (int e$) << picked (int x$)\n    e$ << 9 if (x$ > 9) else x$\n", head)).unwrap();
        assert!(p.contains("    _3: u1 = cmp.gt _x, 9\n    _4: int = if _3\n        yield 9\n    else\n        yield _x\n    push_queue_open(_2, _4)\n"), "{}", p);
        let p = emitted(&format!("{}on (int e$) << picked (int x$)\n    e$ << 9 if (x$ > 9)\n          else 0 if (x$ < 0)\n", head)).unwrap();
        assert!(p.contains("    _3: u1 = if _1\n        yield 1\n    else\n        _2: u1 = cmp.lt _x, 0\n        yield _2\n    if _3\n"), "{}", p);
        refused(&format!("{}on (int e$) << picked (int x$)\n    e$ << x$ << 1 if (x$ > 2) else 0\n", head), "h.zero:9: in a stream processor a push with `else` takes one item in each arm, `e$ << a if (c) else b`");
        // what is refused of the lines
        refused("on (int k) << f (int c)\n    k << 0 if (c > 0)\n    else 1\n", "h.zero:3: this `else` continues nothing: an `else` stands after the block of an `if` statement, at the `if`'s own depth, or goes on a push that has an `if`, on the push's line or on a line indented under it, `x << a if (c)` and then `else b`");
        refused("on (int k) << f (int c)\n    k << 0\n        else 1\n", "h.zero:3: this `else` continues nothing");
        refused("on (int k) << f (int c)\n    k << 0 else 1\n", "h.zero:2: `else` on a push follows its `if`: `x << a if (condition) else b`, the value `a` where the condition holds and `b` where it does not");
        refused("on (int k) << f (int c)\n    k << 0 if (c > 0)\n        else 1\n        else 2\n", "h.zero:4: the `else` before this one has no `if`, so it takes everything that is left and nothing is left for this one: each case but the last is `else value if (condition)`");
        refused("on f (int c)\n    out$ << 0 if (c > 0)\n        else 1 if (c < 0)\n        int z = 3\n", "h.zero:4: a push that goes on over indented lines has a case on each, and each begins `else`: `else value if (condition)`, and last `else value`. A line that is not one of its cases stands at the push's own depth");
        refused("on (int k) << f (int c)\n    k << 0 if (c > 0) else\n", "h.zero:2: an `else` on a push is followed by its value on the same line, `else b` or `else b if (d)`");
        refused("on (int k) << f (int c)\n    k << 0 if (c > 0) else 1 << 2\n", "h.zero:2: 'k' is one value, given once: this line pushes it twice");
        refused("on (int k) << f (int c)\n    k << 0 if (c > 0) else 1 (3) times\n", "h.zero:2: `(n) times` on the push of 'k' would give it more than once");
        refused("on f (int c)\n    out$ << 1 if (c > 0) else 2 << 3\n", "h.zero:2: after `else` a push takes one item, `x$ << a if (c) else b`: for several where the condition fails, write the push on two lines, each with its own `if`");
        // a loop word and `else`: questions 84 and 91 pull apart
        let two = "reads two ways (fm3 questions 84, 91 and 102): the item after `else` pushed that often and the first item once, or whichever is chosen pushed that often. For the first write two pushes on two lines, each with its own `if`; for the second put the choice in brackets, `x$ << (a if (c) else b)` and then the word";
        refused("on f (int c)\n    out$ << 1 if (c > 0) else 2 (3) times\n", &format!("h.zero:2: `(n) times` on a push with `else` {}", two));
        refused("on f (int c)\n    out$ << 1 if (c > 0) (3) times else 2\n", &format!("h.zero:2: `(n) times` on a push with `else` {}", two));
        refused("on f (int c)\n    int s$\n    s$ << 1 if (c > 0) else s$ + 1 until (s$ > 3)\n", &format!("h.zero:3: `until` on a push with `else` {}", two));
        refused("on f (int c)\n    int s$\n    s$ << 1 if (c > 0) else s$ + 1 while (_ < 3)\n", &format!("h.zero:3: `while` on a push with `else` {}", two));
        refused("int x$\nout$ << x$ if (x$ > 0) else 0 forever\n\non f (int c)\n    x$ << c\n", "h.zero:2: `else` on a `<<` at feature scope is not built");
    }

    /// A result is pushed once, at the top level of its function's
    /// body, with its condition on the push (fm3 question 88, log 171).
    /// Under an `if` statement or in a loop the push is refused, the
    /// message showing the line to write; so is a push with `if` and
    /// no `else`, a result pushed twice, and a result nothing pushes.
    /// And the push of the last result does not end the function: the
    /// lines after it run
    #[test]
    fn a_result_is_pushed_once() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-pushed-once");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (1) → 1\n").unwrap();
        let emitted = |code: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            emit(&dir)
        };
        let refused = |code: &str, what: &str| {
            let e = emitted(code).err().unwrap_or_else(|| panic!("not refused: {}", code));
            assert!(e.ends_with(what), "{}: {}", code, e);
        };
        let head = "is a result, and a result is pushed once, at the top level of its function, with its condition on the push (fm3 question 88)";
        // under an `if` statement: the program's own push with its `if`
        refused("on (int s) << f (int x)\n    if (x < 0)\n        s << -1\n    else if (x > 0)\n        s << 1\n", &format!("h.zero:3: 's' {}: this push stands under the `if` on line 2. Write `s << -1 if (x < 0)`, and each other case on a line under it, `else value if (condition)`, the last `else value`", head));
        // in a later arm, and under two: the shape
        refused("on (int s) << f (int x)\n    if (x < 0)\n        out$ << \"neg\"\n    else\n        s << 1\n", &format!("h.zero:5: 's' {}: this push stands under the `if` on line 2. Write one push with its cases, `s << a if (c)` and under it `else b if (d)`, the last `else e`", head));
        refused("on (int s) << f (int x)\n    if (x < 0)\n        if (x < -5)\n            s << 1\n", &format!("h.zero:4: 's' {}: this push stands under the `if` on line 3. Write one push with its cases, `s << a if (c)` and under it `else b if (d)`, the last `else e`", head));
        // in a loop, and in a `for`
        let looped = "and a push does not leave a loop. Give the loop's result where it leaves, `break (value)`, and push what the loop yields, once: `p << loop (...) yields name`";
        refused("on (int p) << f (int n)\n    loop (int q = 1)\n        if (q > n)\n            p << q\n        continue (q * 2)\n", &format!("h.zero:4: 'p' {}: this push stands inside the loop on line 2, {}", head, looped));
        refused("on (int p) << f (int n)\n    int a[] = [1, 2]\n    for (v in a[])\n        p << v\n", &format!("h.zero:4: 'p' {}: this push stands inside the loop on line 3, {}", head, looped));
        // an array that is a result
        refused("on (int r[]) << f (int a)\n    if (a > 0)\n        r[] << [1, 2]\n", &format!("h.zero:3: 'r[]' {}: this push stands under the `if` on line 2. Give it once, at the top level of the body, `r[] << value`: a condition on the push of an array is not built", head));
        // `if` and no `else`: every path gives every result
        refused("on (int r) << f (int a)\n    r << a if (a > 0)\n", "h.zero:2: 'r' has no value where the condition fails: a result's push says every case, so that every path gives every result (fm3 question 88). Add the last one: `r << a if (a > 0) else ...`");
        refused("on (int r) << f (int a)\n    r << a if (a > 0)\n         else 0 - a if (a < 0)\n", "h.zero:3: 'r' has no value where every condition fails: a result's push says every case, so that every path gives every result (fm3 question 88). Add a last case, `else value`");
        // twice at the top level, and never
        refused("on (int r) << f (int a)\n    r << a if (a > 0) else 0\n    r << 7\n", "h.zero:3: 'r' is pushed twice: a function gives each of its results once, at the top level of its body (fm3 question 88)");
        refused("on (int r) << f (int a)\n    int b = a + 1\n", "h.zero:1: 'r' is a result of 'f' and nothing pushes it: a function gives each of its results once, at the top level of its body, `r << value` (fm3 question 88)");
        refused("on (int q, int r) << f (int a)\n    q << a\n", "h.zero:1: 'r' is a result of 'f' and nothing pushes it: a function gives each of its results once, at the top level of its body, `r << value` (fm3 question 88)");
        refused("on (int r[]) << f (int a)\n    int b = a\n", "h.zero:1: 'r[]' is a result of 'f' and nothing pushes it: a function gives each of its results once, at the top level of its body, `r[] << value` (fm3 question 88)");
        // a push into a stream under an `if` statement is as it was
        emitted("on (int n) << f (int x)\n    if (x < 0)\n        out$ << \"neg\"\n    n << x\n").unwrap();
        // the push of the last result does not end the function: the
        // line after it runs, and the `ret` is at the body's end
        let ir = emitted("on (int r) << f (int a)\n    r << a * 2\n    out$ << \"after\"\n").unwrap();
        let at = ir.find("fn f(").unwrap();
        let f = &ir[at..at + ir[at..].find("\n\n").unwrap()];
        assert!(f.starts_with("fn f(a: int) -> int\n    r: int = mul a, 2\n") && f.ends_with("    __out_block(_3)\n    ret r"), "{}", f);
        // ... and with a condition the value is carried to it
        let ir = emitted("on (int r) << f (int a)\n    r << a if (a > 0) else 0 - a\n    out$ << \"after\"\n").unwrap();
        let at = ir.find("fn f(").unwrap();
        let f = &ir[at..at + ir[at..].find("\n\n").unwrap()];
        assert!(f.contains("    r_2: int = if _1\n        yield a\n    else\n        r: int = sub 0, a\n        yield r\n") && f.ends_with("    ret r_2"), "{}", f);
        // where the push is the last thing the function does, each arm
        // ends it, as it did
        let ir = emitted("on (int r) << f (int a)\n    r << a if (a > 0) else 0 - a\n").unwrap();
        assert!(ir.contains("    if _1\n        ret a\n    else\n        r: int = sub 0, a\n        ret r\n"), "{}", ir);
    }

    /// A function that takes an array whole is called in square
    /// brackets (fm3 question 77, log 165): `[sum of] (a[])`. The
    /// plain call is refused showing the line with them, the bracketed
    /// call of a function of one item is refused, and a name with a
    /// method of each kind is told which by the call
    #[test]
    fn a_function_over_an_array_is_called_in_brackets() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-whole-calls");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let head = "on (int n) << sum of (int x[])\n    n << x[] + _\n\non (int d) << doubled (int x)\n    d << x * 2\n\non (int r[]) << scale (int x[]) by (int k)\n    r[] << x[] * k\n\non (int n) << (int a[]) joined to (int b[])\n    n << [count] (a[]) + [count] (b[])\n\non describe (int x)\n    out$ << \"one \"\n\non describe (int x[])\n    out$ << \"many \"\n\n";
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f()\n    int a[] = [1, 2, 3]\n{}\n", head, body)).unwrap();
            emit(&dir)
        };
        let refused = |body: &str, what: &str| {
            let e = f(body).err().unwrap_or_else(|| panic!("not refused: {}", body));
            assert!(e.contains(what), "{}: {}", body, e);
        };
        // the call, with a name, a list, a bare array and a frame; the
        // brackets round the words up to the first group
        for line in ["n << [sum of] (a[])", "n << [sum of] ([4, 5]) + [sum of] ([1 through 3])", "n << [sum of] a[]", "int x$ << 1\n    n << [sum of] (frame x$)", "int s[] = [scale] (a[]) by (2)\n    n << [count] (s[])"] {
            let ir = f(&format!("    {}", line)).unwrap_or_else(|e| panic!("{}: {}", line, e));
            assert!(ir.contains("sum_of(") || ir.contains("scale_by("), "{}: {}", line, ir);
        }
        // the plain call is refused, the line shown with its brackets
        refused("    n << sum of (a[])", "h.zero:21: `n << sum of (a[])`: 'sum of' takes an array whole, `int x[]`, and is called with its name in square brackets (fm3 question 77): write `n << [sum of] (a[])`");
        refused("    n << sum of (a[]) * 10 + sum of ([4, 5])", "write `n << [sum of] (a[]) * 10 + [sum of] ([4, 5])`");
        refused("    n << [sum of] (a[]) + sum of ([4, 5])", "write `n << [sum of] (a[]) + [sum of] ([4, 5])`");
        refused("    int s[] = scale (a[]) by (2)\n    n << [count] (s[])", "`int s[] = scale (a[]) by (2)`: 'scale by' takes an array whole, `int x[]`, and is called with its name in square brackets (fm3 question 77): write `int s[] = [scale] (a[]) by (2)`");
        // the bracketed call of a function of one item
        refused("    int d[] = [doubled] (a[])\n    n << [count] (d[])", "h.zero:21: `[doubled]`: 'doubled' takes one item, and a function of one item is applied to each item of an array plainly, `doubled (a[])` (fm3 question 77). The brackets are for a function declared over an array, `(int x[])`");
        assert!(f("    int d[] = doubled (a[])\n    n << [count] (d[])").is_ok());
        // of the language's own words `count` takes them, an array's
        // length (fm3 question 126, log 225): anything that gives an
        // array stands in the round brackets, and the text is what the
        // plain word's was
        let counted = f("    int x$ << 1 << 2\n    n << [count] (a[]) + [count] (frame x$) + [count] ([1, 2]) + [count] (a[] * 2) + a[[count] (a[]) - 1]").unwrap();
        assert!(counted.contains(": index = count "), "{}", counted);
        refused("    n << count a[]", "h.zero:21: `count a[]`: an array's length is `[count] (a[])`, the whole array in square brackets as for any function handed one (fm3 question 126). `count` is written plainly of a stream, `count x$`, and of a `string`");
        refused("    int x$ << 1\n    n << count (frame x$)", "`count (frame x$)`: an array's length is `[count] (frame x$)`");
        refused("    n << count [1, 2]", "`count (...)`: an array's length is `[count] (...)`");
        refused("    int x$ << 1\n    n << [count] (x$)", "`[count] (x$)`: square brackets hand a word the whole of an array, and 'x$' is a stream. Of a stream the word is plain, `count x$`, how many items it has had; the array of what is waiting in it is `frame x$`, and how many, `[count] (frame x$)` (fm3 questions 94 and 126)");
        refused("    string s = \"ab\"\n    n << [count] (s)", "`[count] (s)`: square brackets hand a word the whole of an array, and 's' is one value. Of a `string` the word is plain, `count s` (fm3 question 126)");
        assert!(f("    string s = \"ab\"\n    int x$ << 1\n    n << count s + count x$").is_ok());
        // a word of the language that is a stream's is no function
        refused("    int x$ << 1\n    int g[] = [frame] (x$)", "`[frame]`: no function of this name is declared over an array. Of the language's own words `[count]` alone takes brackets, an array's length (fm3 question 126); `frame` and the rest are a stream's and are written plainly");
        // a name with a method of each kind: the call says which
        let both = f("    describe (a[])\n    n << 1").unwrap();
        assert!(both.contains("        describe(_") && !both.contains("    describe__ints(a)\n"), "{}", both);
        let whole = f("    [describe] (a[])\n    n << 1").unwrap();
        assert!(whole.contains("    describe__ints(a)\n") && !whole.contains("        describe(_"), "{}", whole);
        // a name that begins with a group has nowhere to put them
        assert!(f("    n << (a[]) joined to ([1, 2])").is_ok());
        // a list written out is still a list: a push's count, a phrase's argument
        assert!(f("    int up$\n    int k = 4\n    up$ << [k] (3) times\n    n << count up$").is_ok());
        // a stream is still not an array, brackets or none
        refused("    int x$ << 1\n    n << [sum of] (x$)", "'sum of' takes an array here, `int x[]`, and 'x$' is a stream");
        // ... and so is a stream's name as an operand there (fm3 question 97, log 243)
        refused("    int x$ << 1\n    n << [sum of] (x$ * 2)", "`x$ * ...` is one value, made from the latest item of 'x$', and 'sum of' takes an array here, `int x[]`: a stream's name is its value now, and nothing of an array's is silently asked of a stream (fm3 questions 90 and 97). The array of what has arrived is `frame x$`: write that where `x$` stands");
    }

    /// The lowering knows which kind a name is (fm3 questions 90 and
    /// 79, log 162): the three crossings that wanted a call resolved
    /// are refused, each beside the line that stands
    #[test]
    fn a_call_is_held_to_its_kinds() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-call-kinds");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let head = "on (int n) << sum of (int x[])\n    n << x[] + _\n\non (int d) << doubled (int x)\n    d << x * 2\n\non shut (int x$)\n    end x$\n\non (int y$) << twice (int x$)\n    loop\n        if (count x$ == 0)\n            break\n        y$ << (peek x$ at (0)) * 2\n        advance x$ by (1)\n\n";
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f()\n{}\n", head, body)).unwrap();
            emit(&dir)
        };
        let refused = |body: &str, what: &str| {
            let e = f(body).err().unwrap_or_else(|| panic!("not refused: {}", body));
            assert!(e.contains(what), "{}: {}", body, e);
        };
        // a stream handed to a function declared over an array
        refused("    int s$ << 1 << 2\n    n << [sum of] (s$)", "h.zero:19: 'sum of' takes an array here, `int x[]`, and 's$' is a stream, its items still arriving (fm3 question 90). The array of what has arrived is `frame s$`: hand it that");
        assert!(f("    int s$ << 1 << 2\n    n << [sum of] (frame s$)").is_ok());
        assert!(f("    int a[] = [1, 2]\n    n << [sum of] (a[]) + [sum of] ([3, 4]) + [sum of] (a[] * 2)").is_ok());
        // an array handed to a function declared over a stream, and to a task
        refused("    int a[] = [1, 2]\n    shut (a[])\n    n << 1", "h.zero:19: 'shut' takes a stream here, `int x$`, and this is an array, all there (fm3 question 90). What begins with these items is a stream: `int s$ << ...`, the array pushed into it, and then `s$` handed over");
        refused("    shut ([1, 2])\n    n << 1", "'shut' takes a stream here, `int x$`, and this is an array");
        assert!(f("    int s$ << [1, 2]\n    shut (s$)\n    n << 1").is_ok());
        refused("    int a[] = [1, 2]\n    int d$ = twice (a[])\n    n << count d$", "h.zero:19: 'twice' takes a stream here, `int x$`, and 'a[]' is an array, all there (fm3 questions 90 and 93). What begins with these items is a stream: `int s$ << a[]`, and then `s$` handed over");
        assert!(f("    int s$ << [1, 2]\n    int d$ = twice (s$)\n    n << count d$").is_ok());
        // an array where one value is declared, through a call
        refused("    int a[] = [1, 2, 3]\n    int v = doubled (a[])\n    n << v", "h.zero:19: `int v = doubled (a[])`: 'doubled' takes one item, so given `a[]` it is applied to each and gives an array, and one value is wanted here (fm3 questions 90 and 92). For all of them write `int v[] = doubled (a[])`; for one, hand it one item");
        refused("    int v = doubled ([1, 2])\n    n << v", "h.zero:18: `int v = doubled ([1, 2])`: 'doubled' takes one item, so given an array it is applied to each and gives an array, and one value is wanted here (fm3 questions 90 and 92). For all of them write `int v[] = doubled ([1, 2])`; for one, hand it one item");
        refused("    int a[] = [1, 2, 3]\n    n << doubled (a[]) + 1", "`n << doubled (a[]) + 1`: 'doubled' takes one item, so given `a[]` it is applied to each and gives an array, and one value is wanted here (fm3 questions 90 and 92). For all of them give what it gives to an array's name, `int v[] = ...`; for one, hand it one item");
        assert!(f("    int a[] = [1, 2, 3]\n    int v[] = doubled (a[])\n    int w = doubled (a[1])\n    n << v[2] + w").is_ok());
        // a stream's name there is its latest item, as it was: of a
        // stream the function reads for nothing else, the value
        // itself (fm3 log 186), and of one it peeks into, `latest`
        assert!(f("    int s$ << 1 << 2\n    int v = doubled (s$)\n    n << v").unwrap().contains("    v: int = doubled(2)\n"));
        assert!(f("    int s$ << 1 << 2\n    int v = doubled (s$)\n    n << v + peek s$ at (0)").unwrap().contains("latest"));
        // a look back in a plain function, and in a task that walks
        refused("    int s$ << 1 << 2\n    n << s$[-1]", "h.zero:19: 's$[-1]' is a look back, the item before the present one, and only a stream processor has a present item (fm3 question 75). In a function a stream's latest item is its name, `s$`; the items its reader has passed are `s$ behind (k)`");
        std::fs::write(dir.join("h/h.zero"), "int x$\nint d$ = diffs(x$)\n\non (int d$) << diffs (int x$)\n    d$ << x$ - x$[-1]\n\non (int n) << f()\n    x$ << 1 << 4\n    n << count d$\n").unwrap();
        assert!(emit(&dir).is_ok());
    }

    /// A read by a place cannot fail (fm3 question 127, log 236): an
    /// array's item by its place is written where it is read, the
    /// place compared with how many items there are and the outside
    /// value in the branch not taken; no reader's `peek` and no
    /// `check` is written for it, a diagnostic build has no row for
    /// it, and what the compiler can count it compares itself
    #[test]
    fn a_read_by_a_place_cannot_fail() {
        let dir = std::env::temp_dir().join("probe-zero-sampled");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (1) → 1\n").unwrap();
        let head = "int kick[] wrapped = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0]\nint waltz[] wrapped = [1, 0, 0]\n\non (int v) << pick (int a[] else 5) at (int i)\n    v << a[i]\n\non (int v) << round (int a[] wrapped) at (int i)\n    v << a[i]\n\n";
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f (int i)\n{}\n", head, body)).unwrap();
            emit(&dir)
        };
        let body = |ir: &str, f: &str| -> String { ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        let refused = |text: &str, what: &str| {
            let e = f(text).err().unwrap_or_else(|| panic!("not refused: {}", text));
            assert!(e.contains(what), "{}: {}", text, e);
        };
        // a place worked out: two compares and a choice, zero outside
        let ir = f("    int a[] = [1 through 4]\n    n << a[i]").unwrap();
        let read = body(&ir, "f");
        assert!(!read.contains("peek") && !read.contains("check"), "{}", read);
        assert!(read.contains(": u1 = cmp.lt ") && read.contains(": u1 = cmp.ge _") && read.contains(": u1 = and ") && read.contains("    n: int = if ") && read.contains("    else\n        yield 0\n"), "{}", read);
        // the parameter's own rule, and a count nobody knows, wrapped
        assert!(body(&ir, "pick_at").contains("    else\n        yield 5\n"), "{}", ir);
        let round = body(&ir, "round_at");
        assert!(round.contains(": index = rem ") && round.contains(": u1 = cmp.gt ") && round.contains("    else\n        yield 0\n"), "{}", round);
        // a place written out, not negative: one compare
        let read = body(&f("    int a[] = [1 through 4]\n    n << a[2]").unwrap(), "f");
        assert!(read.contains(": u1 = cmp.lt ") && !read.contains("cmp.ge") && !read.contains(" = and "), "{}", read);
        // a place written out and the items written out: nothing is
        // compared, and a place outside reads nothing at all
        let read = body(&f("    int a[] = [10, 20, 30, 40]\n    n << a[2]").unwrap(), "f");
        assert!(read.contains(": index = add ") && !read.contains("cmp") && !read.contains(" = if "), "{}", read);
        let read = body(&f("    int a[] = [10, 20, 30, 40]\n    n << a[9] + a[-1]").unwrap(), "f");
        assert!(!read.contains("load") && !read.contains("cmp") && read.contains("    _1: int = const 0\n    n: int = add _1, 0\n"), "{}", read);
        let read = body(&f("    int a[] else 7 = [10, 20]\n    n << a[2]").unwrap(), "f");
        assert!(!read.contains("load") && read.contains("    n: int = const 7\n"), "{}", read);
        // sixteen items counted on the declaration: a mask; three: a remainder
        let read = body(&f("    n << kick[i]").unwrap(), "f");
        assert!(read.contains(": index = and _") && read.contains(", 15\n") && !read.contains("rem") && !read.contains("cmp"), "{}", read);
        let read = body(&f("    n << waltz[i]").unwrap(), "f");
        assert!(read.contains(": index = rem _") && read.contains(", 3\n") && read.contains(": u1 = cmp.lt ") && !read.contains("cmp.gt"), "{}", read);
        // a second name for the same items copies nothing and reads by its own rule
        let read = body(&f("    int a[] = [10, 20, 30]\n    int loop[] wrapped = a[]\n    n << loop[i] + a[i]").unwrap(), "f");
        assert_eq!(read.matches("__queue_int(").count(), 1, "{}", read);
        assert!(read.contains(": index = rem ") && read.contains("        yield 0\n"), "{}", read);
        // the diagnostic build has no row for an array's item
        let mut s = store::read(&dir).unwrap();
        s.sites = true;
        std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f (int i)\n    int a[] = [1 through 4]\n    n << a[i]\n", head)).unwrap();
        let l = lower::lower(&{ let mut s = store::read(&dir).unwrap(); s.sites = true; s }).unwrap();
        assert!(l.sites.iter().all(|x| !x.what.contains("item {a}")), "{:?}", l.sites);
        let _ = s;
        // the refusals
        refused("    int a[] wrapped else 0 = [1, 2]\n    n << a[i]", "h.zero:11: 'a[]' says twice what a read outside it gives, `wrapped` and `else`: an array has one rule for outside");
        refused("    int a wrapped = 3\n    n << a", "h.zero:11: `wrapped` says how an array is read by a place, and 'a' is one int. It is said of an array or a string where it is declared, `int a[] wrapped = [...]`");
        refused("    int x$ wrapped\n    n << 1", "`wrapped` on 'x$': a stream has no end to go round to");
        refused("    int a[] else 1.5 = [1, 2]\n    n << a[i]", "`else` on 'a[]' says what a read outside it gives, a value of the item's type written out; its items are int: write a number, `else 0`");
        refused("    int k = 3\n    int a[] else k = [1, 2]\n    n << a[i]", "a value of the item's type written out");
        refused("    string t else 7.5 = \"ab\"\n    n << 1", "its items are characters: write `else char (32)`");
        refused("    int a[] else = [1, 2]\n    n << a[i]", "`else` on 'a[]' wants the value a read outside gives, `a[] else 0`");
        refused("    float a[] mirrored = [1.0, 2.0]\n    n << 1", "h.zero:11: `mirrored` on 'a[]', a read outside going back the way it came, is ruled and not built yet (fm3 question 127). What a read outside the items gives is `else (v)`, `wrapped` or `clamped`, and zero where nothing is said");
    }

    /// `s == t` on two strings is one `bool` (fm3 question 100, log
    /// 242), said in zero in the language's own feature and written in
    /// line: what `s [==] t` is, to the line
    #[test]
    fn two_strings_are_compared_as_values() {
        let dir = std::env::temp_dir().join("probe-zero-strings");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (1) → 1\n").unwrap();
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("type word =\n    string text\n\non (string s) << name of (int k)\n    s << \"one\"\n\non (int n) << f (int k)\n    string s = \"zero\"\n    string t = \"zero\"\n    char cs[] = \"zero\"\n    word w = word(\"let\")\n{}\n", body)).unwrap();
            emit(&dir)
        };
        let body = |ir: &str| -> String { ir.split("\nfn f(").nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        let refused = |text: &str, what: &str| {
            let e = f(text).err().unwrap_or_else(|| panic!("not refused: {}", text));
            assert!(e.contains(what), "{}: {}", text, e);
        };
        let by_hand = body(&f("    n << 1 if (s [==] t) else 0").unwrap());
        assert_eq!(body(&f("    n << 1 if (s == t) else 0").unwrap()), by_hand);
        assert!(by_hand.contains(" = len _") && !by_hand.contains("(s, t)"), "{}", by_hand);
        let by_hand = body(&f("    n << 1 if (s [!=] \"zero\") else 0").unwrap());
        assert_eq!(body(&f("    n << 1 if (s != \"zero\") else 0").unwrap()), by_hand);
        // only what makes sense is declared
        refused("    n << 1 if (s < t) else 0", "h.zero:12: `<` on two strings: a string has `==` and `!=`, whether two texts are the same, and nothing that says which comes first");
        // an array of characters is an array: each pair, refused
        refused("    n << 1 if (cs[] == cs[]) else 0", "`==` is applied to each pair of items and gives a bool for each");
        // not built (fm3 question 144): a field read by name, a result
        refused("    n << 1 if (w.text == \"let\") else 0", "`==` between arrays is applied to each pair");
        refused("    n << 1 if (name of (k) == \"one\") else 0", "`==` between arrays is applied to each pair");
        f("    n << 1 if (w.text [==] \"let\") else 0").unwrap();
    }

    /// One spelling of a value on a condition (fm3 question 126,
    /// principle 7; log 241): `a if (c) else b` wherever a whole value
    /// is given, the tree the old form made; `if (c) then (a) else (b)`
    /// refused with the line to write
    #[test]
    fn a_value_on_a_condition_has_one_spelling() {
        let dir = std::env::temp_dir().join("probe-zero-one-spelling");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (1, 2) → 1\n").unwrap();
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) << twice (int x)\n    n << x * 2\n\non (int n) << f (int a, int b)\n{}\n", body)).unwrap();
            emit(&dir)
        };
        let body = |ir: &str| -> String { ir.split("\nfn f(").nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        let refused = |text: &str, what: &str| {
            let e = f(text).err().unwrap_or_else(|| panic!("not refused: {}", text));
            assert!(e.contains(what), "{}: {}", text, e);
        };
        // a definition, round brackets inside a larger value, and an
        // argument: a value chosen, one `if` with a `yield` each way
        let chosen = "    m: int = if _1\n        yield a\n    else\n        yield b\n";
        let read = body(&f("    int m = a if (a < b) else b\n    n << m + 1").unwrap());
        assert!(read.contains(chosen), "{}", read);
        let read = body(&f("    n << 1 + (a if (a < b) else b)").unwrap());
        assert!(read.contains(": int = if _1\n        yield a\n    else\n        yield b\n"), "{}", read);
        let read = body(&f("    n << twice (a if (a < b) else b)").unwrap());
        assert!(read.contains(": int = if _1\n        yield a\n    else\n        yield b\n"), "{}", read);
        // three cases, the later ones in the `else`
        let read = body(&f("    int m = 0 if (a < 0) else 1 if (a < b) else 2\n    n << m").unwrap());
        assert!(read.matches(" = if _").count() == 2, "{}", read);
        // a result's push in brackets is the value pushed; without
        // them it is the push's own form, a `ret` each way (question 88)
        let read = body(&f("    n << (a if (a < b) else b)").unwrap());
        assert!(read.contains("    n: int = if _1\n        yield a\n    else\n        yield b\n    ret n\n"), "{}", read);
        let read = body(&f("    n << a if (a < b) else b").unwrap());
        assert!(read.contains("    if _1\n        ret a\n") && read.contains("        ret b\n") && !read.contains("yield"), "{}", read);
        // the old form, refused where it is read, the line given
        let gone = "h.zero:5: a value on a condition is written one way, the value first: `a if (c) else b`, in a push, in a definition and in round brackets inside a larger value. `if (c) then (a) else (b)` is no longer zero (fm3 question 126). Write ";
        refused("    n << if (a < b) then (a) else (b)", &format!("{}`n << a if (a < b) else b`", gone));
        refused("    int m = if (a < 0) then (-a) else (a)\n    n << m", &format!("{}`int m = -a if (a < 0) else a`", gone));
        refused("    n << 1 + if (a < b) then (a) else (b)", &format!("{}`n << 1 + (a if (a < b) else b)`", gone));
        refused("    n << if (a < 0) then (0) else (if (a > b) then (b) else (a))", &format!("{}`n << 0 if (a < 0) else b if (a > b) else a`", gone));
        refused("    n << twice (if (a < b) then (a) else (b))", &format!("{}`n << twice (a if (a < b) else b)`", gone));
        refused("    bool k = if (a < b) then (a == 1) else (b == 1)\n    n << 1", &format!("{}`bool k = (a == 1) if (a < b) else (b == 1)`", gone));
        refused("    n << if (a < b) then a else b", &format!("{}the value first, `a if (c) else b`", gone));
        // a value with an `if` and no `else` is nothing where it fails
        refused("    int m = a if (a < b)\n    n << m", "h.zero:5: a value on a condition says both cases, `a if (c) else b`: this one has no `else`, so it is nothing where the condition fails");
        refused("    int m = a if (a < b) then (b)\n    n << m", "an `if` after a value says where that value is the one meant, and takes no `then`: `a if (c) else b`");
    }

    /// A start and a step, and between (fm3 question 127's second
    /// part, log 240): `from (a) to (b)`, `nearest`, `linear`,
    /// `clamped`, what each writes and what is refused
    #[test]
    fn a_coordinate_between_two_items() {
        let dir = std::env::temp_dir().join("probe-zero-between");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (1.0) → 1\n").unwrap();
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (float n) << f (float x)\n{}\n", body)).unwrap();
            emit(&dir)
        };
        let body = |ir: &str, f: &str| -> String { ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        let refused = |text: &str, what: &str| {
            let e = f(text).err().unwrap_or_else(|| panic!("not refused: {}", text));
            assert!(e.contains(what), "{}: {}", text, e);
        };
        // a decimal with nothing said: the item at or before, a cut
        // toward zero brought down one below zero, and one read
        let read = body(&f("    float a[] = [0.0, 0.5, 1.0]\n    n << a[x]").unwrap(), "f");
        assert!(read.contains(": index = conv x\n") && read.contains(": u1 = cmp.lt x, ") && read.matches(": float = load ").count() == 1 && !read.contains("check"), "{}", read);
        // `nearest`: a half added first
        let read = body(&f("    float a[] nearest = [0.0, 0.5, 1.0]\n    n << a[x]").unwrap(), "f");
        assert!(read.contains(": float = add x, 0.5\n") && read.matches(": float = load ").count() == 1, "{}", read);
        // `from (0) to (1)`, three items counted: times 2, over 1
        let read = body(&f("    float a[] from (0) to (1) = [0.0, 0.5, 1.0]\n    n << a[x]").unwrap(), "f");
        assert!(read.contains(": float = mul x, 2.0\n") && read.contains(", 1.0\n") && !read.contains(": float = sub x"), "{}", read);
        // a start that is not 0 is taken off, and the span worked out
        let read = body(&f("    float a[] from (-1) to (1) = [0.0, 0.5, 1.0]\n    n << a[x]").unwrap(), "f");
        assert!(read.contains(": float = sub x, -1.0\n") && read.contains(": float = const 1.0\n"), "{}", read);
        // `linear`: two reads and the blend; `clamped` reads no zero
        let read = body(&f("    float a[] linear clamped = [0.0, 0.5, 1.0]\n    n << a[x]").unwrap(), "f");
        assert!(read.matches(": float = load ").count() == 2 && read.contains(": float = mul ") && read.contains("    n: float = add ") && read.contains("yield 2\n") && !read.contains(" = and "), "{}", read);
        // a whole place on an array that says no `from` is the read it was
        let read = body(&f("    float a[] nearest = [0.0, 0.5, 1.0]\n    int i = 2\n    n << a[i]").unwrap(), "f");
        assert!(!read.contains(": float = conv") && !read.contains(": float = add") && read.matches(": float = load ").count() == 1, "{}", read);
        // the refusals
        refused("    int a[] linear = [1, 2]\n    n << 1.0", "h.zero:2: `linear` on 'a[]' blends the two items either side of a coordinate, and two whole numbers blended are not a whole number: declare the items `float`. `nearest` gives the closer of the two, and with nothing said it is the item at or before");
        refused("    char a[] linear = \"ab\"\n    n << 1.0", "there is nothing between two characters");
        refused("    string s linear = \"ab\"\n    n << 1.0", "there is nothing between two characters");
        refused("    n << 1.0\n\ntype colour = red | green\n\non (int v) << g()\n    colour cs[] linear = [red, green]\n    v << 1", "h.zero:7: `linear` on 'cs[]' blends the two items either side of a coordinate, and there is nothing between two names of the enumeration `colour`");
        refused("    float a[] nearest linear = [1.0, 2.0]\n    n << 1.0", "h.zero:2: 'a[]' says twice what a read between two items gives, `nearest` and `linear`: an array has one rule for between");
        refused("    float a[] clamped wrapped = [1.0, 2.0]\n    n << 1.0", "'a[]' says twice what a read outside it gives, `clamped` and `wrapped`: an array has one rule for outside");
        refused("    float a[] from (0) to (1) from (0) to (2) = [1.0, 2.0]\n    n << 1.0", "'a[]' says `from` twice: an array spans one run of coordinates");
        refused("    float a[] from (1) to (1.0) = [1.0, 2.0]\n    n << 1.0", "'a[]' spans `from (1) to (1.0)`, no distance at all: the first item's coordinate and the last's are two numbers");
        refused("    float a[] from (0) = [1.0, 2.0]\n    n << 1.0", "`from` on 'a[]' says the coordinates its items span, the first item's and the last's: `a[] from (0) to (1)`");
        refused("    float a[] from (x) to (1) = [1.0, 2.0]\n    n << 1.0", "`from` on 'a[]' says the coordinates its items span, each a number written out in round brackets: `a[] from (0) to (1)`");
        refused("    float a[] from 0 to 1 = [1.0, 2.0]\n    n << 1.0", "each a number written out in round brackets");
        refused("    float y from (0) to (1) = 2.0\n    n << y", "`from` says how an array is read by a place, and 'y' is one float");
    }

    /// `$` means a stream (fm3 question 90, log 161): an array given
    /// to a `$` name is refused with its line shown as an array, and
    /// each word is held to its kind
    #[test]
    fn each_word_is_held_to_its_kind() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-kinds");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let task = "on (int y$) << twice (int x$)\n    loop\n        if (count x$ == 0)\n            break\n        y$ << (peek x$ at (0)) * 2\n        advance x$ by (1)\n\n";
        let f = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f()\n{}\n", task, body)).unwrap();
            emit(&dir)
        };
        let refused = |body: &str, what: &str| {
            let e = f(body).err().unwrap_or_else(|| panic!("not refused: {}", body));
            assert!(e.contains(what), "{}: {}", body, e);
        };
        // what `=` gives a `$` name is a task's stream, or it is an array
        refused("    int i$ = [1, 2, 3]\n    n << count i$", "h.zero:9: `int i$ = [1, 2, 3]`: what `=` gives here is an array, all there, and `$` is a stream's mark (fm3 question 90). Write `int i[] = [1, 2, 3]`; or, for a stream that begins with these items, `int i$ << [1, 2, 3]`");
        refused("    int i$ = [1 through 4]\n    n << count i$", "Write `int i[] = [1 through 4]`; or, for a stream that begins with these items, `int i$ << [1 through 4]`");
        refused("    char c$ = \"hi\"\n    n << count c$", "Write `char c[] = \"hi\"`; or, for a stream that begins with these items, `char c$ << \"hi\"`");
        refused("    int x$ << 1\n    int g$ = frame x$\n    n << count g$", "h.zero:10: `int g$ = frame x$`: what `=` gives here is an array, all there, and `$` is a stream's mark (fm3 question 90). Write `int g[] = frame x$`");
        refused("    int a[] = [1, 2]\n    int j$ = a[] * 2\n    n << count j$", "Write `int j[] = a[] * 2`");
        refused("    int x$ << 1\n    int j$ = x$ * 2\n    n << count j$", "Write `int j[] = x$ * 2`");
        assert!(f("    int x$ << 1 << 2\n    int d$ = twice (x$)\n    n << count d$").is_ok());
        assert!(f("    int i$ << [1, 2, 3]\n    int a[] = frame i$\n    n << a[1] + [count] (a[]) + (a[] + _)").is_ok());
        // an array is given whole where it is declared, and never changes
        // an array given a stream's name as an operand: refused,
        // naming `frame` (fm3 question 97, principle 2; log 227)
        refused("    int x$ << 1 << 2\n    int j[] = x$ * 2\n    n << j[] + _", "`x$ * ...` is one value, made from the latest item of 'x$', and 'j[]' is an array: a stream's name is its value now, and nothing of an array's is silently asked of a stream (fm3 questions 90 and 97). The array of what has arrived is `frame x$`, `int j[] = frame x$ * 2`");
        refused("    int x$ << 1 << 2\n    int j[] = 1 + x$ * 2\n    n << j[] + _", "`x$ * ...` is one value");
        assert!(f("    int x$ << 1 << 2\n    int j[] = frame x$ * 2\n    n << j[] + _").is_ok());
        refused("    int x$ << 1\n    int d[] = twice (x$)\n    n << [count] (d[])", "`int d[] = twice (x$)`: a task gives a stream, its items arriving, and 'd[]' is an array (fm3 question 90). Write `int d$ = ...`; the array of what has arrived in it is `frame d$`");
        refused("    int a[] << 1 << 2\n    n << [count] (a[])", "`int a[] << 1 << 2`: an array is given whole where it is declared, by `=`, and never pushed into (fm3 question 90). Write `int a[] = [...]`; what has first items and more to come is a stream, `int a$ << ...`");
        refused("    int a[] at (1 hz)\n    n << 0", "`int a[] at (1 hz)`: a rate is a stream's, and 'a[]' is an array, all there (fm3 question 90)");
        refused("    int a[]\n    n << [count] (a[])", "`int a[]`: an array is given whole where it is declared, `int a[] = [1, 2, 3]`, and never changes (fm3 question 90); an empty one is `int a[] = []`. What is declared bare and filled later is a stream, `int a$`");
        assert!(f("    int a[] = []\n    n << [count] (a[])").is_ok());
        refused("    int a[] = [1, 2]\n    a[] << 3\n    n << [count] (a[])", "h.zero:10: 'a[] << ...': an array never changes: its items are all there where it is declared, `int a[] = [...]` (fm3 question 90). What is pushed into is a stream, `int a$`");
        // a stream's words on an array
        let arr = |line: &str| format!("    int a[] = [1, 2]\n{}", line);
        refused(&arr("    n << peek a[] at (1)"), "`peek` is a stream's word, asked of what arrives over time, and 'a[]' is an array, all there (fm3 question 90): one item of an array is `a[k]`");
        refused(&arr("    n << latest a[]"), "`latest` is a stream's word, asked of what arrives over time, and 'a[]' is an array, all there (fm3 question 90): an array's last item is `a[[count] (a[]) - 1]`");
        refused(&arr("    int g[] = frame a[]\n    n << 0"), "`frame` is a stream's word, asked of what arrives over time, and 'a[]' is an array, all there (fm3 question 90): it makes an array of what a stream holds, and this is one already");
        refused(&arr("    int g[] = a[] behind (1)\n    n << 0"), "`behind` is a stream's word");
        for (line, word) in [("    advance a[] by (1)\n    n << 0", "advance"), ("    n << position a[]", "position"), ("    n << time of a[]", "time of"), ("    end a[]\n    n << 0", "end"), ("    bool e = ended a[]\n    n << 0", "ended")] {
            refused(&arr(line), &format!("`{}` is a stream's word, asked of what arrives over time, and 'a[]' is an array, all there (fm3 question 90)", word));
        }
        // `count` is asked of both
        assert!(f("    int a[] = [1, 2]\n    int x$ << 1\n    n << [count] (a[]) + count x$").is_ok());
        // an array's forms on a stream
        let st = |line: &str| format!("    int x$ << 1 << 2\n{}", line);
        refused(&st("    n << x$[1]"), "h.zero:10: 'x$[k]': an item by its place is an array's, and 'x$' is a stream (fm3 question 90). The item k on from where this reader stands is `peek x$ at (k)`; the array of what has arrived is `frame x$`, and one back is `x$[-1]`");
        refused(&st("    int k = 1\n    n << x$[k]"), "'x$[k]': an item by its place is an array's");
        refused(&st("    for (v in x$)\n        check (v > 0)\n    n << 0"), "`for` walks an array, and 'x$' is a stream (fm3 question 90): the array of what has arrived is `frame x$`, `for (x in frame x$)`");
        refused(&st("    for (v in x$ * 2)\n        check (v > 0)\n    n << 0"), "`x$ * ...` is one value, made from the latest item of 'x$', and `for` walks an array: a stream's name is its value now, and nothing of an array's is silently asked of a stream (fm3 questions 90 and 97). The array of what has arrived is `frame x$`, `for (v in frame x$ * 2)`");
        refused(&st("    n << x$ + _"), "a reduce with `_` gives one answer of a whole array, and 'x$' is a stream (fm3 question 90): the array of what has arrived is `frame x$`; a running total is a line that stands, `sum$ << sum$ + x$ forever`");
        assert!(f(&st("    for (v in frame x$)\n        check (v > 0)\n    n << peek x$ at (0) + x$")).is_ok());
        // an array has no latest item; a place before its first is
        // outside it like any other, and compiles (fm3 question 127,
        // 142), where it was refused as a stream's look back
        assert!(f(&arr("    n << a[-1]")).is_ok());
        let one = "'a[]' is an array, and one value is wanted here: an array has no latest item, as a stream has (fm3 question 90). Its last item is `a[[count] (a[]) - 1]`, one item `a[k]`, and its sum `a[] + _`";
        refused(&arr("    int v = a[]\n    n << v"), one);
        refused(&arr("    n << a[] + 1"), one);
        refused(&arr("    if (a[] > 0)\n        out$ << 1\n    n << 1"), one);
        assert!(f(&arr("    int b[] = a[] + 1\n    n << a[[count] (a[]) - 1] + (b[] + _)")).is_ok());
    }

    /// The mark is part of a name wherever it is written (fm3 question
    /// 90, log 159): each refusal, with its message
    #[test]
    fn the_mark_travels() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-marks");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let with = |text: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), text).unwrap();
            emit(&dir)
        };
        let f = |body: &str| with(&format!("on (int n) << f()\n{}\n", body));
        let refused = |r: Result<String, String>, what: &str| {
            let e = r.err().unwrap_or_else(|| panic!("not refused: {}", what));
            assert!(e.contains(what), "{}", e);
        };
        assert!(f("    int a[] = [5, 6, 7]\n    n << a[1] + [count] (a[]) + (a[] + _)").is_ok());
        // an array written as a stream, a stream as an array, either bare
        refused(f("    int a[] = [5, 6, 7]\n    n << count a$"), "'a$': 'a' is an array, declared `int a[]` on line 2, and the mark is part of its name wherever it is written (fm3 question 90): write `a[]`, or `a[k]` for one item");
        refused(f("    int x$ << 5\n    n << [count] (x[])"), "'x[]': 'x' is a stream, declared `int x$` on line 2, and the mark is part of its name wherever it is written (fm3 question 90): write `x$`");
        refused(f("    int x$ << 5\n    n << x[0]"), "'x[]': 'x' is a stream, declared `int x$` on line 2");
        refused(f("    int a[] = [5, 6, 7]\n    n << a"), "'a' is written without its mark: it is declared `int a[]` on line 2, and the mark is part of its name wherever it is written (fm3 question 90): write `a[]`");
        refused(f("    int x$ << 5\n    n << x"), "'x' is written without its mark: it is declared `int x$` on line 2");
        refused(f("    n << [count] (zz[])"), "'zz[]' is not declared: an array is declared with its type, `int zz[] = [1, 2, 3]`");
        refused(f("    int k = 3\n    n << [count] (k[])"), "'k[]': 'k' is one value, declared `int k` on line 2, and has no items");
        // a parameter, a result and a feature-scope name are held too
        refused(with("on (int n) << g (int x[])\n    n << x$ + _\n\non (int n) << f()\n    n << g ([1, 2])\n"), "'x$': 'x' is an array, declared `int x[]` on line 1");
        refused(with("on (int r[]) << g (int k)\n    r$ << [1 through k]\n\non (int n) << f()\n    n << 1\n"), "'r$': 'r' is an array, declared `int r[]` on line 1");
        refused(with("int q[] = [1, 2]\n\non (int n) << f()\n    n << q$[0]\n"), "'q$': 'q' is an array, declared `int q[]` on line 1");
        // a string has no mark and keeps its `s[0]`
        assert!(f("    string s = \"hello\"\n    n << int(s[0])").is_ok());
        // a function that gives an array is a plain function, told from
        // a task by its result's mark, and gives it once
        let gives = with("on (int r[]) << g (int k)\n    r[] << [1 through k] * [1 through k]\n\non (int n) << f()\n    int s[] = g (4)\n    n << s[3]\n").unwrap();
        assert!(gives.contains("\nfn g(k: int) -> int$\n") && !gives.contains("fn g(r: int$"), "{}", gives);
        // the form it had until an array had its mark is refused with
        // the rest (fm3 question 87, log 161), each line shown as it is to be
        refused(with("on (int r$) = g (int k)\n    r$ = [1 through k]\n\non (int n) << f()\n    n << 1\n"), "h.zero:1: a function that gives an array says so on its result, `int r[]`, and gives it by pushing it, once; `=` says what a name is (fm3 questions 77, 87 and 90). Write `on (int r[]) << g (int k)`");
        refused(with("on (int r[]) << g (int k)\n    r[] = [1 through k]\n\non (int n) << f()\n    n << 1\n"), "h.zero:2: 'r[]' is a result, and a result is given by pushing it, an array as any other; `=` says what a name is (fm3 questions 77 and 90). Write `r[] << [1 through k]`");
        refused(with("on (int r[]) << g (int k)\n    r[] << [1 through k] << [1]\n\non (int n) << f()\n    n << 1\n"), "'r[]' is one array, given once: this line pushes it twice. What takes items one after another is a stream, `r$`");
        refused(with("on (int r[]) << g (int k)\n    r[] << [1 through k] (2) times\n\non (int n) << f()\n    n << 1\n"), "`(n) times` on the push of 'r[]' is not built: an array that is a function's result is given whole, once");
        // the mark on the type, and two marks on one name
        refused(f("    int[] a = [1]\n    n << 0"), "'int[]': the mark is part of the name and not of the type, wherever the name is written (fm3 question 90): write `int a[]`");
        refused(with("on (int n) << g (int[] x)\n    n << 0\n\non (int n) << f()\n    n << 1\n"), "write `int x[]`");
        refused(f("    token ops[]$\n    n << 0"), "'ops[]$', a stream of arrays, is not built (fm3 question 90)");
        refused(f("    int m[][] = [1]\n    n << 0"), "'m[][]', an array of arrays, is not built (fm3 question 90)");
        refused(with("type T =\n    int xs[]\n\non (int n) << f()\n    n << 0\n"), "an array as a field of a struct is not built");
    }

    /// Brackets widen a word (fm3 question 84, log 155): a word on a
    /// push applies to the last item of its chain, and brackets round
    /// several items make them the one it applies to. How a group is
    /// told from a bracketed value, every line tried, the three loops
    /// over a group, and each refusal with its message
    #[test]
    fn brackets_widen_a_word() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-brackets");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int a$\nint b$\nint c$\nb$ << a$ forever\n";
        let rest = "\non (int n) << twice (int k)\n    n << k * 2\n\non (int n) << three (int k) times\n    n << 3 * k\n\non (int t$) << count up to (int n)\n    t$ << [1 through n]\n\non (int n) << f (int k)\n    a$ << k\n    n << [count] (frame b$) + [count] (frame c$)\n\non g (int k)\n    ";
        let with = |lines: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}{}{}\n", head, lines, rest, body)).unwrap();
            emit(&dir)
        };
        let g = |body: &str| -> String {
            let ir = with("", body).unwrap_or_else(|e| panic!("{}: {}", body, e));
            let at = ir.find("\nfn g(k: int)\n").unwrap_or_else(|| panic!("{}: no g", body));
            let end = ir[at + 1..].find("\nfn ").map_or(ir.len(), |n| at + 1 + n);
            ir[at + 1..end].to_string()
        };
        let looped = |n: &str, item: &str| format!("    loop(_3: int = 0)\n        _4: u1 = cmp.lt _3, {}\n        if _4\n        else\n            break\n{}", n, item);
        // brackets round one item are ordinary grouping: the lines the
        // item gives without them, a bracketed call among them
        assert_eq!(g("b$ << (k) (3) times"), g("b$ << k (3) times"));
        assert_eq!(g("b$ << (twice (k)) (3) times"), g("b$ << twice (k) (3) times"));
        assert_eq!(g("b$ << (k + 1) while (_ < 3)"), g("b$ << k + 1 while (_ < 3)"));
        // a group: its items pushed in order each time round. A call's
        // bracket inside it is the call's argument, and a declared
        // name keeps its `times` there as inside any bracket
        assert!(g("b$ << (k << twice (k)) (3) times").contains(&looped("3", "        push_queue_open(_2, k)\n        _5: int = twice(k)\n        push_queue_open(_2, _5)\n        _6: int = add _3, 1\n        continue _6\n")));
        assert!(g("b$ << (k << three (k) times) (2) times").contains(&looped("2", "        push_queue_open(_2, k)\n        _5: int = three_times(k)\n        push_queue_open(_2, _5)\n")));
        // the items before a group are pushed once, and the count is
        // still worked out before any of them
        let ir = g("b$ << 7 << (k << twice (k)) (k) times");
        let (check, first, lp) = (ir.find("cmp.ge k, 0").unwrap(), ir.find("push_queue_open").unwrap(), ir.find("loop(").unwrap());
        assert!(check < first && first < lp && ir[lp..].matches("push_queue_open").count() == 2 && ir[..lp].matches("push_queue_open").count() == 1, "{}", ir);
        // `while` over a group: both candidates worked out, the second
        // reading the first, the test of the last, and then both pushed
        assert!(g("b$ << k << (b$ + 1 << b$ + 1) while (_ < 6)").contains("    loop()\n        _3: int = latest_queue(_2)\n        _4: int = add _3, 1\n        _5: int = add _4, 1\n        _6: u1 = cmp.lt _5, 6\n        if _6\n        else\n            break\n        push_queue_open(_2, _4)\n        push_queue_open(_2, _5)\n        continue\n"));
        // `until` over a group: both pushed, as a chain's items are,
        // and then the test of the last, by `_` or by the stream's name
        let until = g("b$ << k << (b$ + 1 << b$ + 1) until (b$ == 6)");
        assert!(until.contains("    loop()\n        _3: int = latest_queue(_2)\n        _4: int = add _3, 1\n        push_queue_open(_2, _4)\n        _5: int = latest_queue(_2)\n        _6: int = add _5, 1\n        push_queue_open(_2, _6)\n        _7: u1 = cmp.eq _6, 6\n        if _7\n            break\n        continue\n"), "{}", until);
        assert_eq!(until, g("b$ << k << (b$ + 1 << b$ + 1) until (_ == 6)"));
        // with no word, and with `if`, a group is its items in order
        assert_eq!(g("b$ << (k << 2)"), g("b$ << k << 2"));
        assert_eq!(g("b$ << (k << 2) if (k > 0)"), g("b$ << k << 2 if (k > 0)"));
        assert_eq!(g("b$ << 1 << (k << 2) if (k > 0) (2) times").matches("push_queue_open").count(), 3);
        // on a declaration, as a word may be
        // (the stream is read for nothing but its latest, so it is
        // the loop's own value beside the counter, fm3 log 186)
        assert!(g("int d$ << 0 << (d$ + 1 << d$ + 1) (4) times").contains("    d_2: int = loop(_1: int = 0, d: int = 0)\n        _2: u1 = cmp.lt _1, 4\n"));
        assert!(g("int d$ << 0 << (d$ + 1 << d$ + 1) (4) times\n    int n = peek d$ at (0)").contains("    loop(_2: int = 0)\n        _3: u1 = cmp.lt _2, 4\n"));
        let ambiguous = "'... (k) times' at the end of a push reads two ways: a function whose name ends `(...) times`, called and pushed once, or what stands before the bracket pushed that many times. For the call put it in brackets, `x$ << (name (k) times)`; for the count put the item in brackets, `x$ << (item) (k) times`";
        let last = "brackets round several items of a push make them the one item its word applies to, and they stand last in the chain (fm3 question 84): `x$ << a << (b << c) (3) times`. Before the last item a group would be its items in order and nothing more: write them without the brackets";
        let value = "brackets round several items of a push make them one item for the word that follows, `x$ << (a << b) (3) times`: a group is not a value, and what may follow it is `if`, `(n) times`, `while`, `until` or `forever`";
        for (body, said) in [
            ("b$ << three (k) times", ambiguous.to_string()),
            ("b$ << (k << 2) << 3 (3) times", last.to_string()),
            ("b$ << (k << 2) << 3", last.to_string()),
            ("b$ << (k << (2 << 3)) (3) times", "a group of items inside a group: one pair of brackets says it, `x$ << (a << b << c) (3) times`".to_string()),
            ("b$ << (k << 2) + 1", value.to_string()),
            ("b$ << (k << 2) (3)", value.to_string()),
            ("b$ << (k << 2) times", "a push's count is the bracketed group before `times`, after the item: `x$ << item (n) times`".to_string()),
            ("b$ << (k << count up to (3)) (2) times", "a task call is not repeated: the task's own chain says when it stops".to_string()),
            ("b$ << (k << [1 through 3]) while (_ < 3)", "a block is pushed once: `while` repeats an item".to_string()),
            ("b$ << ([1 through 3] << k) while (_ < 3)", "a block is pushed once: `while` repeats an item".to_string()),
            ("b$ << (k << [1 through 3]) until (_ > 3)", "a block is pushed once: `until` repeats an item".to_string()),
            ("b$ << (k << 2) forever", "`forever` in a function is a line that would set up a standing connection each time the function runs: not built. Wire it at feature scope, where it stands from the start".to_string()),
        ] {
            let err = with("", body).expect_err(body);
            assert!(err.ends_with(&format!("h.zero:20: {}", said)), "{}: {}", body, err);
        }
        // a range before the last item of an `until` group is pushed
        // as a chain pushes it
        assert!(with("", "b$ << ([1 through 3] << k) until (_ > 3)").is_ok());
        // a `while` group holds its items until the test: one that is
        // written into the stream by a `<<` method cannot be read back
        // by the name before it is there
        std::fs::write(dir.join("h/h.zero"), "char c$\nout$ << c$ forever\n\non f (int k)\n    c$ << (k << c$) while (_ != 50)\n").unwrap();
        let err = emit(&dir).expect_err("a method item");
        assert!(err.ends_with("h.zero:5: in a group under `while` nothing is pushed until the test is made, and this item is not one char of 'c$' but something written into it: the items after it cannot read 'c$' as it would then stand. Not built"), "{}", err);
        // into a cell: the candidates held, the test, and the stores
        std::fs::write(dir.join("h/h.zero"), "int seen$ << 0\n\non (int n) << f (int k)\n    seen$ << (seen$ + 1 << seen$ * 2) while (_ < k)\n    n << seen$\n").unwrap();
        let ir = emit(&dir).unwrap();
        assert!(ir.contains("    loop()\n        _1: __ctx = load _this\n        _2: int = get _1, seen\n        _3: int = add _2, 1\n        _4: int = mul _3, 2\n        _5: u1 = cmp.lt _4, k\n        if _5\n        else\n            break\n        _6: __ctx = load _this\n        _7: __ctx = set _6, seen, _3\n        store _7, _this\n        _8: __ctx = load _this\n        _9: __ctx = set _8, seen, _4\n        store _9, _this\n        continue\n"), "{}", ir);
    }

    /// `forever` applies to the last item of its chain, as the other
    /// words do (fm3 question 84's second half, log 156): a wiring
    /// line of more than one item is written with brackets and is the
    /// edge it was; without them it is refused, showing itself with
    /// them; a count and an `until` over a bracketed chain stand
    #[test]
    fn forever_applies_to_the_last_item() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-last-item");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int a$\nint b$\nint c$\nb$ << a$ forever\n";
        let rest = "\non (int n) << f (int k)\n    a$ << k\n    c$ << k\n    n << count b$\n";
        let with = |lines: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}{}", head, lines, rest)).unwrap();
            emit(&dir)
        };
        let edge = |ir: &str| -> String {
            let at = ir.find("\nfn __edge2(").unwrap_or_else(|| panic!("no second edge: {}", ir));
            let end = ir[at + 1..].find("\nfn ").map_or(ir.len(), |n| at + 1 + n);
            ir[at + 1..end].to_string()
        };
        // the bracketed chain is the edge: the item and what follows it
        let wired = edge(&with("out$ << (c$ << \"\\n\") forever\n").unwrap());
        assert!(wired.starts_with("fn __edge2(__item: int)\n    __out__int(__item)\n    _1: u8 = const 10\n    __out_ch(_1)\n    ret\n"), "{}", wired);
        // with `if`, the standing filter, the `if` of the whole push
        let filtered = edge(&with("out$ << (c$ << \"\\n\") if (c$ > 2) forever\n").unwrap());
        assert!(filtered.contains("    _1: u1 = cmp.gt __item, 2\n    if _1\n        __out__int(__item)\n        _2: u8 = const 10\n        __out_ch(_2)\n    ret"), "{}", filtered);
        // a count and an `until` over the bracketed chain: the whole
        // chain under the kept count, and under the kept bit
        let counted = with("out$ << (c$ << \"\\n\") (3) times\n").unwrap();
        assert!(counted.contains("    __times2: int\n") && edge(&counted).contains("    _2: int = get _1, __times2\n    _3: u1 = cmp.lt _2, 3\n    if _3\n        __out__int(__item)\n        _4: u8 = const 10\n        __out_ch(_4)\n"), "{}", edge(&counted));
        let until = with("out$ << (c$ << \"\\n\") until (c$ == 3)\n").unwrap();
        assert!(edge(&until).contains("    _2: u1 = get _1, __until2\n    _3: u1 = cmp.eq _2, 0\n    if _3\n        __out__int(__item)\n        _4: u8 = const 10\n        __out_ch(_4)\n        _5: u1 = cmp.eq __item, 3\n"), "{}", edge(&until));
        // a line of one item is as it was, bracketed or not
        assert_eq!(with("out$ << c$ forever\n").unwrap(), with("out$ << (c$) forever\n").unwrap());
        let then = "Until 7 October 2026 the word covered the whole push, every item of 'c$' and what follows it; to say that, put the items in brackets";
        let once = "is pushed once, when the line begins to stand, which is when the store starts, and a push then is not built (fm3 question 80)";
        for (lines, said) in [
            // the line every wiring line was until the ruling
            ("out$ << c$ << \"\\n\" forever\n", format!("h.zero:5: 'out$ << c$ << \"\\n\" forever': `forever` applies to the last item of its chain (fm3 question 84), so this is `c$` once and then `\"\\n\"` for ever, and nothing paces that: it would never end. {}: `out$ << (c$ << \"\\n\") forever`", then)),
            // its `if` and its spacing shown as written
            ("out$ << c$ << \" \" << \"\\n\"  if (c$ > 2) forever\n", format!("h.zero:5: 'out$ << c$ << \" \" << \"\\n\" if (...) forever': `forever` applies to the last item of its chain (fm3 question 84), so this is `c$ << \" \"` once and then `\"\\n\"` for ever, and nothing paces that: it would never end. {}: `out$ << (c$ << \" \" << \"\\n\") if (c$ > 2) forever`", then)),
            // a last item that is the line's own target paces nothing
            ("b$ << c$ << b$ forever\n", format!("h.zero:5: 'b$ << c$ << b$ forever': `forever` applies to the last item of its chain (fm3 question 84), so this is `c$` once and then `b$` for ever, and nothing paces that: it would never end. {}: `b$ << (c$ << b$) forever`", then)),
            // a group that is not the whole line
            ("out$ << c$ << (\" \" << \"\\n\") forever\n", format!("h.zero:5: 'out$ << c$ << (\" \" << \"\\n\") forever': `forever` applies to the last item of its chain (fm3 question 84), so this is `c$` once and then `\" \" << \"\\n\"` for ever, and nothing paces that: it would never end. {}: `out$ << (c$ << \" \" << \"\\n\") forever`", then)),
            // the last item stands and something is written before it
            ("out$ << \"values: \" << c$ forever\n", format!("h.zero:5: 'out$ << \"values: \" << c$ forever': `forever` applies to the last item of its chain (fm3 question 84), `c$`; what is written before it, `\"values: \"`, {}", once)),
            ("b$ << 0 << c$ (3) times\n", format!("h.zero:5: 'b$ << 0 << c$ (3) times': `(3) times` applies to the last item of its chain (fm3 question 84), `c$`; what is written before it, `0`, {}", once)),
            ("out$ << c$ forever\nb$ << (1 << 2) forever\n", "h.zero:6: nothing on the right of 'b$ << 1 << 2' is a stream: `forever` makes a push happen again whenever what is on its right has something new, and a value never has".to_string()),
            // a count and an `until` without the brackets
            ("out$ << c$ << \"\\n\" (3) times\n", "h.zero:5: 'out$ << c$ << \"\\n\" (3) times': `(3) times` applies to the last item of its chain (fm3 question 84), so this is `c$` once and then `\"\\n\"` 3 times, when the store starts, and a push then is not built (fm3 question 80). For the first 3 items of 'c$', each with what follows it, put the items in brackets: `out$ << (c$ << \"\\n\") (3) times`".to_string()),
            ("out$ << c$ << \"\\n\" until (c$ == 3)\n", "h.zero:5: 'out$ << c$ << \"\\n\" until (...)': `until` applies to the last item of its chain (fm3 question 84), so this is `c$` once and then `\"\\n\"` until the condition holds, when the store starts, and a push then is not built (fm3 question 80). For a line that stands until then, each item of 'c$' with what follows it, put the items in brackets: `out$ << (c$ << \"\\n\") until (c$ == 3)`".to_string()),
            // no word at all: the advice has the brackets
            ("out$ << c$ << \"\\n\"\n", "h.zero:5: 'out$ << c$ << \"\\n\"' has a stream on its right and no `forever`. If it is wiring, everything that arrives in 'c$' going on into 'out$', write `out$ << (c$ << \"\\n\") forever`. If it is one push when the store starts, of what 'c$' holds then, that is what the line says (fm3 question 79) and it is not built: push it from a function".to_string()),
            // `if` with a count or an `until`, still not built
            ("out$ << (c$ << \"\\n\") if (c$ > 2) (3) times\n", "h.zero:5: `if` with a count on a line that stands is not built: 'out$ << (c$ << \"\\n\") if (...) (3) times' could be the first 3 that pass, or those of the first 3 that pass".to_string()),
        ] {
            let err = with(lines).expect_err(lines);
            assert!(err.ends_with(&said), "{}: {}", lines, err);
        }
    }

    /// A stream function is a line of the tick like any other (fm3
    /// question 123 as the eight principles settle it, log 226): where
    /// its function is one an edge's could be, a tick writes it in
    /// place, its output's lines after it and a line beside it as
    /// written. Its last tick is still its own function, called at the
    /// input's end; one that looks back is still called where its
    /// input is pushed
    #[test]
    fn a_stream_function_is_a_line_of_the_tick() {
        let dir = std::env::temp_dir().join(format!("probe-zero-ticked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>go() → \"1 2\\n2 4\"\n").unwrap();
        let with = |wiring: &str, body: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("int x$\nint d$ = {} (x$)\nout$ << (x$ << \" \") forever\nout$ << (d$ << \"\\n\") forever\n\non (int d$) << doubled (int p$)\n    d$ << p$ * 2\n\non (int d$) << grown (int p$)\n    d$ << p$ + p$[-1]\n\non go()\n{}", wiring, body)).unwrap();
            emit(&dir).unwrap()
        };
        let body = |ir: &str, f: &str| -> String { ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        // the tick: the double worked out, the number written, then
        // the double; the function of one item is called by nothing
        let ir = with("doubled", "    x$ << 1 << 2\n");
        let tick = body(&ir, "__tick1");
        let at = |what: &str| tick.find(what).unwrap_or_else(|| panic!("{} in {}", what, tick));
        assert!(at("__v_d: int = mul __item, 2") < at("__out__int(__item)") && at("__out__int(__item)") < at("__out__int(__v_d)"), "{}", tick);
        assert!(body(&ir, "go").contains("__tick1(") && !ir.contains("__z1_each"), "{}", ir);
        // the input's end is still the processor's last tick
        let ir = with("doubled", "    x$ << 1\n    end x$\n    out$ << ended d$\n");
        assert!(body(&ir, "go").contains("__tick1(") && body(&ir, "go").contains("__z1_end()"), "{}", ir);
        // one that looks back keeps a value and is called as it was
        let ir = with("grown", "    x$ << 1 << 2\n");
        assert!(!ir.contains("__tick") && body(&ir, "go").contains("__z1_each("), "{}", ir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a run may write is 65 536 characters, the runner's capture
    /// of a context (fm3 log 256; from hop thirty-seven to hop
    /// forty-four it was 32 768, two contexts' worth of the larger
    /// having passed what arm64's `adr` reaches). A text that does not
    /// fit is a failed check; a character at a time has none yet
    #[test]
    fn a_run_may_write_65536_characters() {
        let dir = std::env::temp_dir().join(format!("probe-zero-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("s/h")).unwrap();
        std::fs::write(dir.join("s/h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>wrote (2100) lines → 2100\n>wrote (4096) lines → 4096\n>wrote (4097) lines → check\n>timed (3) → \"7\\n7\\n7\" at 1 hz\n").unwrap();
        std::fs::write(dir.join("s/h/h.zero"), "int wait$ at (1 hz)\nout$ << (wait$ << \"\\n\") forever\n\non (int n) << wrote (int k) lines\n    out$ << \"sixteen letters\\n\" (k) times\n    n << k\n\non timed (int k)\n    wait$ << 7 (k) times\n").unwrap();
        let l = lower::lower(&store::read(&dir.join("s")).unwrap()).unwrap();
        assert!(l.ir.contains("data __out: array(u8, 131072)\n") && l.ir.contains("    fits: u1 = cmp.le n2, 65536\n") && l.ir.contains("data __out_t: array(i64, 131074)\n"), "{}", l.ir);
        let report = test(&dir, Backend::Native, opt::MAX_LEVEL).unwrap();
        assert_eq!((report.passed, report.failed), (4, 0), "{}", report.log);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What cannot run is refused where it is compiled, at its line
    /// (fm3 log 254, principle 8): a whole number or a time divided by
    /// a zero written out, which the IR refused as a literal with no
    /// type; and a clock with no `if`, which the IR refused as a value
    /// nothing defines. A clock that has one is a clock whatever its
    /// stream is kept as, read by nothing or by a function
    #[test]
    fn what_cannot_run_is_refused_at_its_line() {
        let dir = std::env::temp_dir().join(format!("probe-zero-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → 0\n").unwrap();
        let refused = |code: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            match lower::lower(&store::read(&dir).unwrap()) {
                Ok(_) => "(compiled)".to_string(),
                Err(e) => e.to_string(),
            }
        };
        for (code, line) in [("on (int n) << run()\n    int a = 7\n    n << a / 0\n", 3), ("on (int n) << run()\n    int a = 7\n    n << a % 0\n", 3), ("on (int n) << run()\n    time beat = 1 s\n    time part = beat / 0\n    n << 1\n", 3)] {
            let e = refused(code);
            assert!(e.contains(&format!("h.zero:{}: a division by zero: the divisor is written as 0", line)), "{}", e);
        }
        // (a `float` by `0.0` is infinity, and a divisor that is a name is the check)
        assert_eq!(refused("on (int n) << run()\n    float a = 7.0\n    float q = a / 0.0\n    int z = 0\n    n << 7 / z\n"), "(compiled)");
        let e = refused("int tick$ at (1 hz)\ntick$ << tick$ + 1 forever\nout$ << (tick$ << \"\\n\") forever\n\non (int n) << run()\n    n << 0\n");
        assert!(e.contains("h.zero:2: ") && e.contains("is a clock, a line paced by its own stream's rate, with nothing to stop it") && e.contains("A clock needs a way to stop, an `if`"), "{}", e);
        // with an `if`: read by a line, by nothing, by a function
        for code in ["int tick$ at (1 hz)\ntick$ << tick$ + 1 if (tick$ < 3) forever\nout$ << (tick$ << \"\\n\") forever\n\non (int n) << run()\n    n << 0\n", "int tick$ at (1 hz)\ntick$ << tick$ + 1 if (tick$ < 3) forever\n\non (int n) << run()\n    n << 0\n", "int tick$ at (1 hz)\ntick$ << tick$ + 1 if (tick$ < 3) forever\n\non (int n) << run()\n    n << tick$\n"] {
            assert_eq!(refused(code), "(compiled)", "{}", code);
        }
        let e = refused("int tick$\ntick$ << tick$ + 1 if (tick$ < 3) forever\n\non (int n) << run()\n    n << 0\n");
        assert!(e.contains("'tick$' has no rate to"), "{}", e);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A division or a remainder of whole numbers by zero is a failed
    /// check on every path (fm3 question 116, Ash, 10 October 2026; log
    /// 223): a comparison and a `check` before the instruction, left
    /// out where the compiler sees the divisor is not zero, and never
    /// written in the language's own lines. The diagnostic build says
    /// the line and the reason
    #[test]
    fn a_division_by_zero_is_a_failed_check() {
        let dir = std::env::temp_dir().join(format!("probe-zero-div-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>share (7) among (2) → 3\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "on (int n) << share (int a) among (int k)\n    n << a / k\n\non (int n) << halved (int a)\n    n << a / 2 + a % 3\n\non (int n) << tested (int a, int k)\n    n << (a / k if (k != 0) else 0)\n\non (int n) << checked (int a, int k)\n    check (k > 0)\n    n << a % k\n\non (int n) << looped (int a, int k)\n    n << loop (int x = a, int y = k) while (y != 0) yields x\n        continue (y, x % y)\n\non (int n) << after (int a, int k)\n    if (k != 0)\n        out$ << a / k\n    n << a / k\n\non (float n) << decimal (float a, float k)\n    n << a / k\n\non (time t) << part (int k)\n    t << 1 s / k\n\non (time t) << third()\n    t << 1 s / 3\n\non (float r) << ratio (time a, time b)\n    r << a / b\n\non written (int a)\n    out$ << a\n").unwrap();
        let mut s = store::read(&dir).unwrap();
        let l = lower::lower(&s).unwrap();
        let body = |ir: &str, f: &str| -> String { ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        assert!(body(&l.ir, "share_among").contains("    _1: u1 = cmp.ne k, 0\n    check _1\n    n: int = div a, k\n"), "{}", l.ir);
        // a literal; a value an `if`, a `check` or the loop's own
        // `while` has tested; a decimal; a time by a number it knows
        for none in ["halved", "tested", "looped", "decimal", "third"] {
            assert!(!body(&l.ir, none).contains("check"), "{}: {}", none, body(&l.ir, none));
        }
        assert_eq!(body(&l.ir, "checked").matches("check ").count(), 1, "{}", body(&l.ir, "checked"));
        // past the `if` that tested it, the value is not known again
        let after = body(&l.ir, "after");
        assert!(after.matches("check ").count() == 1 && after.contains("\n    check "), "{}", after);
        // a time divided: the check of the number, and of the other
        // time's count, where the operator is used
        assert!(body(&l.ir, "part").contains(": u1 = cmp.ne k, 0\n    check "), "{}", body(&l.ir, "part"));
        assert_eq!(body(&l.ir, "ratio").matches("    check ").count(), 1, "{}", body(&l.ir, "ratio"));
        // the language's own lines: the writer of a number divides by
        // a loop's variable and has no check
        assert!(l.ir.contains("\nfn __out__int(") && !body(&l.ir, "__out__int").contains("check"), "{}", l.ir);
        // the diagnostic build names the line and the reason
        s.sites = true;
        let d = lower::lower(&s).unwrap();
        assert!(d.sites.contains(&lower::Site { file: "h.zero".into(), line: 2, what: "a division by zero".into() }), "{:?}", d.sites);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The tick of a stream, compiled (fm3 questions 121 and 86, log
    /// 206 to 208). Where calling each line where its stream is pushed
    /// already runs them in order, the push calls them as it did and no
    /// tick is written; where it does not, one function holds the
    /// lines' own statements in order, a pushed item going on in a
    /// local; a line over two streams is a function of no item, written
    /// once in a tick that reaches it twice; and what is refused
    /// Several things going on at once (fm3 questions 80 and 121, log
    /// 217): a line with nothing on its right but its own stream, at a
    /// rate, is a clock, one of the things going on, with a word of the
    /// context that says when it is next due; `__turns` takes the
    /// earliest; a step of a rate gives the others their turns; and a
    /// case's twin computes the whole timeline. A store with one thing
    /// going on has none of it and is the text it was
    /// A function partway through a push at a rate is a thing going on
    /// (fm3 question 135, log 232): where something that arrives can
    /// start one, it is continuations over a pool, with no step that
    /// holds the stack; and in every other store it is the loop it was
    #[test]
    fn a_function_partway_is_a_thing_going_on() {
        for store in ["suite/zero/going-two", "suite/zero/restart"] {
            let ir = emit(Path::new(store)).unwrap();
            for there in ["\nfn count_down__0(__a: index) -> i64\n", "\nfn count_down__2(__a: index) -> i64\n", "\nfn launch__1(__a: index) -> i64\n", "\nfn __act_turn()\n", "\ndata __act_due: array(i64, 128)\n", "        __act_w: index = __act_start___z1_each(__item)\n", "    __act_w: index = __act_start_launch()\n", "__act_turn()\n"] {
                assert!(ir.contains(there), "{} lacks {:?}", store, there);
            }
            // nothing is left that steps while holding the stack, and
            // the functions as they were written are gone
            for gone in ["\nfn __step(", "\nfn count_down()", "\nfn launch()", "\nfn launched_by("] {
                assert!(!ir.contains(gone), "{} has {:?}", store, gone);
            }
        }
        // the countdown's loop, left at its step and begun again from
        // the number it was left with, what followed the step first
        let two = emit(Path::new("suite/zero/going-two")).unwrap();
        assert!(two.contains("    loop(_8: int = _8__s, __q: u1 = 1)\n        if __q\n            _11: int = sub _8, 1\n            continue _11, 0\n"), "{}", two);
        // a store where no turn can start a function that steps has
        // none of it: a timed input whose function does not push at a
        // rate, a clock beside a countdown, and one thing going on
        for store in ["suite/zero/going-keys", "suite/zero/going-beside", "suite/zero/going", "suite/zero/hello", "suite/zero/static", "suite/zero/timed"] {
            let ir = emit(Path::new(store)).unwrap();
            assert!(!ir.contains("__act_") && !ir.contains("__0("), "{}", store);
        }
        assert!(emit(Path::new("suite/zero/going-keys")).unwrap().contains("\nfn count_down()\n"));
    }

    /// Two streams with a rate that each get an item in one slot give
    /// one item of a line that reads both (fm3 log 234): a push into
    /// either sets the line's bit, and the close of the slot, the last
    /// thing in the list, calls the line once. A store with no such
    /// line has no close
    #[test]
    fn two_streams_with_a_rate_in_one_slot() {
        let ir = emit(Path::new("suite/zero/going-mix")).unwrap();
        for there in ["    __slot___edge4: u1\n    __slot___edge5: u1\n    __due_close: i64\n", "\nfn __close()\n", "        __edge4()\n", "        __edge5()\n", "        hold: u1 = and atc, same\n        if hold\n            break\n", ", __slot___edge4, 1\n"] {
            assert!(ir.contains(there), "going-mix lacks {:?} in {}", there, ir);
        }
        // the clocks mark the line and do not call it: its two calls
        // are the close's
        assert_eq!(ir.matches("__edge4()").count(), 2, "{}", ir);
        for store in ["suite/zero/going", "suite/zero/going-alone", "suite/zero/going-beside", "suite/zero/going-keys", "suite/zero/going-two", "suite/zero/restart", "suite/zero/tick", "suite/zero/hello"] {
            let ir = emit(Path::new(store)).unwrap();
            assert!(!ir.contains("__close") && !ir.contains("__slot_"), "{}", store);
        }
    }

    #[test]
    fn several_things_going_on() {
        let dir = std::env::temp_dir().join(format!("probe-zero-going-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        let with = |lines: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("int i$ at (1 hz)\nint j$ at (2 hz)\nint x$\n{}out$ << (i$ << \"\\n\") forever\nout$ << (j$ << \"\\n\") forever\nout$ << (x$ << \"\\n\") forever\n\non (int n) << f()\n    j$ << 1 << 2\n    n << 1\n", lines)).unwrap();
            emit(&dir)
        };
        // one thing going on: no list, no word, no twin, and the step
        // the two lines it was
        let one = with("").unwrap();
        for gone in ["__turns", "__due", "__whole_"] {
            assert!(!one.contains(gone), "{} in {}", gone, one);
        }
        assert!(one.contains("fn __step(d: i64)\n    _this: ptr = context()\n    x: __ctx = load _this\n    c: i64 = get x, __clock\n    m: i64 = add c, d\n"), "{}", one);
        for store in ["suite/zero/hello", "suite/zero/static", "suite/zero/timed", "suite/zero/words"] {
            let ir = emit(Path::new(store)).unwrap();
            assert!(!ir.contains("__turns") && !ir.contains("__due") && !ir.contains("__whole_"), "{}", store);
        }
        // a clock: its function pushes with no wait and no step, moves
        // its word on a period, and where its `if` fails is over
        let ir = with("i$ << i$ + 1 if (i$ < 5) forever\n").unwrap();
        let func = |ir: &str, name: &str| -> String {
            let at = ir.find(&format!("fn {}(", name)).unwrap_or_else(|| panic!("no {} in {}", name, ir));
            ir[at..].split("\nfn ").next().unwrap().to_string()
        };
        let clock = func(&ir, "__edge1");
        assert!(!clock.contains("__step(") && !clock.contains("__wait(") && clock.contains(": i64 = get") && clock.contains(", __due1\n") && clock.contains(", 1000000\n") && clock.contains("set") && clock.contains(", __due1, 4611686018427387904\n"), "{}", clock);
        // the list is asked at the start, in a step, and by the twin
        let turns = func(&ir, "__turns");
        assert!(turns.contains("        d1: i64 = get x, __due1\n        late: u1 = cmp.gt d1, t\n        if late\n            break\n        __wait(d1)\n") && turns.contains("            __edge1()\n"), "{}", turns);
        assert!(func(&ir, "__zero_start").contains("    __turns(0)\n") && func(&ir, "__step").contains("    __turns(m)\n") && func(&ir, "__step").contains("    held: u1 = cmp.le c2, m\n    check held\n    __wait(m)\n") && func(&ir, "__whole_f").contains(" = f()\n    __turns(4611686018427387903)\n"), "{}", ir);
        // two clocks: the earlier, and the first written of two at one time
        let ir = with("i$ << i$ + 1 if (i$ < 5) forever\nj$ << j$ + 1 if (j$ < 5) forever\n").unwrap();
        assert!(func(&ir, "__turns").contains("        e2: u1 = cmp.lt d2, d1\n        b2: i64 = if e2\n            yield d2\n        else\n            yield d1\n"), "{}", ir);
        // input that arrives at a time (fm3 log 220): where a case
        // gives one, the input is the first of the things going on;
        // where none does, the store has no word for it
        assert!(!ir.contains("__in_at") && !ir.contains("__due_in"), "{}", ir);
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() with in \"k\" at 1.5 s → 1\n").unwrap();
        let ir = with("i$ << i$ + 1 if (i$ < 5) forever\n").unwrap();
        assert!(func(&ir, "__turns").contains("        d1: i64 = get x, __due_in\n        d2: i64 = get x, __due1\n        e2: u1 = cmp.lt d2, d1\n") && func(&ir, "__turns").contains("            __in_turn()\n") && ir.contains("\nfn __in_at(c: u8, t: i64)\n"), "{}", ir);
        let ir = with("").unwrap();
        assert!(func(&ir, "__turns").contains("        d1: i64 = get x, __due_in\n        late: u1 = cmp.gt d1, t\n") && func(&ir, "__in_turn").contains("    check free\n") && func(&ir, "__in_turn").contains("    __in_ch(c)\n"), "{}", ir);
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        // `restart x$` (fm3 question 52, log 220): the stream has a
        // phase and the number of its run; a push into it reads the
        // number round each step and, finding it changed, sets the
        // bit and leaves; a caller asks the bit and leaves too; the
        // case's twin clears it. A store with no `restart` has none
        assert!(!ir.contains("__run_") && !ir.contains("__phase_") && !ir.contains("__ended"), "{}", ir);
        std::fs::write(dir.join("h/h.zero"), "int i$ at (1 hz)\nout$ << (i$ << \"\\n\") forever\n\non (int n) << f()\n    n << 1\n\non launch()\n    count down()\n    out$ << \"liftoff\"\n\non count down()\n    restart i$\n    i$ << [3 through 1]\n").unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>launch() with in \"k\" at 1.5 s → \"3\" at 0 s\n").unwrap();
        let ir = emit(&dir).unwrap();
        let down = func(&ir, "count_down");
        assert!(down.contains(": i64 = get") && down.contains(", __run_i\n") && down.contains(", __phase_i, ") && down.contains("        __step(1000000)\n") && down.contains(", __ended, 1\n") && down.contains("            ret\n"), "{}", down);
        assert!(func(&ir, "launch").contains("    count_down()\n    __e1: __ctx = load _this\n    __f1: u1 = get __e1, __ended\n    if __f1\n        ret\n"), "{}", ir);
        assert!(func(&ir, "__whole_launch").contains("    launch()\n    __e1: __ctx = load _this\n    __g1: __ctx = set __e1, __ended, 0\n"), "{}", ir);
        assert!(!func(&ir, "__step").contains("check held"), "{}", ir);
        let e = { std::fs::write(dir.join("h/h.zero"), "int i$\nout$ << (i$ << \"\\n\") forever\n\non (int n) << f()\n    restart i$\n    n << 1\n").unwrap(); emit(&dir).unwrap_err() };
        assert!(e.contains("`restart i$`") && e.contains("declared with a rate"), "{}", e);
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 1\n").unwrap();
        // with no rate it would never end
        let e = with("x$ << x$ + 1 forever\n").unwrap_err();
        assert!(e.contains("would never end") && e.contains("for a clock give the stream a rate"), "{}", e);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tick_is_one_function_in_order() {
        let dir = std::env::temp_dir().join(format!("probe-zero-tick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        std::fs::write(dir.join("product.md"), "# product\n*x*\n\nplatform: static on\nh: static on\n").unwrap();
        let head = "int x$\nint y$\nint a$\nint b$\nint z$\nint sum$\nint beat$ at (1 hz)\n";
        let with = |lines: &str, f: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\non (int n) << f (int k)\n    x$ << k\n    {}\n", head, lines, f)).unwrap();
            emit(&dir)
        };
        let func = |ir: &str, name: &str| -> String {
            let at = ir.find(&format!("fn {}(", name)).unwrap_or_else(|| panic!("no {} in {}", name, ir));
            ir[at..].split("\nfn ").next().unwrap().to_string()
        };
        // a chain, and two lines out of one stream neither of which
        // feeds anything: called where the stream is pushed, no tick
        let ir = with("sum$ << sum$ + x$ forever\nout$ << (sum$ << \"\\n\") forever\n", "n << k").unwrap();
        assert!(!ir.contains("__tick") && func(&ir, "f").contains("    __edge1(k)\n"), "{}", ir);
        // ... and the number's line written first: in order already
        let ir = with("out$ << (x$ << \" \") forever\nsum$ << sum$ + x$ forever\nout$ << (sum$ << \"\\n\") forever\n", "n << k").unwrap();
        assert!(!ir.contains("__tick"), "{}", ir);
        // the sum's line written first: its reader is written after
        // the number's line, and depth first would run it before. One
        // function: the sum worked out, the number written, the sum
        // written, and the push calls it and nothing else
        let ir = with("sum$ << sum$ + x$ forever\nout$ << (x$ << \" \") forever\nout$ << (sum$ << \"\\n\") forever\n", "n << k").unwrap();
        let tick = func(&ir, "__tick1");
        assert!(tick.contains("    __next_1: int = add _2, __item\n") && tick.contains("    __out__int(__item)\n    _5: u8 = const 32\n    __out_ch(_5)\n    __out__int(__next_1)\n    _6: u8 = const 10\n    __out_ch(_6)\n    ret"), "{}", tick);
        assert!(func(&ir, "f").contains("    __tick1(k)\n") && !ir.contains("fn __edge"), "{}", ir);
        // the diamond: the line over both written once, after both,
        // each read by its name, one word of the context each
        let ir = with("a$ << x$ * 2 forever\nb$ << x$ + 1 forever\nz$ << a$ + b$ forever\n", "n << z$").unwrap();
        let tick = func(&ir, "__tick1");
        assert_eq!(tick.matches("add ").count(), 2, "{}", tick);
        assert!(tick.find("set _1, a, __v_a").unwrap() < tick.find("set _3, b, __v_b").unwrap() && tick.find("set _3, b, __v_b").unwrap() < tick.find("get _5, a\n").unwrap(), "{}", tick);
        // pushed by a function, `a$` sets the line off alone, by a
        // call with no item
        let ir = with("out$ << (x$ << \" \") forever\nz$ << a$ + b$ forever\n", "a$ << k\n    n << z$").unwrap();
        assert!(!ir.contains("__tick") && func(&ir, "f").contains("    __edge2()\n"), "{}", ir);
        // a stream that is stored ticks once a push too: the item is
        // stored and the line called, where it was a node run after
        // the statement
        let ir = with("out$ << (x$ << \" \") forever\n", "n << [count] (frame x$)").unwrap();
        assert!(func(&ir, "f").contains("    push_queue_open(_2, k)\n    __edge1(k)\n") && !ir.contains("fn __node"), "{}", func(&ir, "f"));
        for (lines, said) in [
            // a circle
            ("a$ << b$ forever\nb$ << a$ forever\nz$ << x$ forever\n", "h.zero:8: 'a$ << b$' sets off 'b$ << a$', and that sets off the first again: a circle, and a tick of either would never end. A stream may be said from its own earlier items and never from itself at the present one: in a stream processor one of them looks back, `a$[-1]`"),
            // a line reached in order and where a second kind of stream is pushed
            ("a$ << (x$ << x$) forever\nb$ << x$ forever\nz$ << a$ + b$ forever\n", "h.zero:10: 'z$ << ...' is set off twice in one tick of 'x$': through 'b$', and through 'a$', which a line pushes more than one item into. Whether it then runs once, or once for each item, is not ruled (fm3 question 123). Not built"),
            ("a$ << x$ forever\nbeat$ << x$ forever\nz$ << a$ + beat$ forever\n", "h.zero:10: 'z$ << ...' is set off twice in one tick of 'x$': through 'a$', and through 'beat$', which has a rate, a beat of its own. Whether it then runs once, or once for each item, is not ruled (fm3 question 123). Not built"),
        ] {
            let err = with(lines, "n << z$").expect_err(lines);
            assert!(err.ends_with(said), "{}: {}", lines, err);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The running sum (fm3 question 80's second half, log 149): on
    /// the right of its own standing push a stream's own name is a
    /// read of its latest item and sets nothing off; the one other
    /// stream there paces the line. What the target is kept as, three
    /// ways; and what is refused
    #[test]
    fn a_stream_sums_itself() {
        let dir = std::env::temp_dir().join(format!("probe-zero-sum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int x$\nint y$\nint sum$\nint i$ at (1 hz)\n";
        let with = |lines: &str, f: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\non (int n) << f (int k)\n    x$ << k\n    {}\n", head, lines, f)).unwrap();
            emit(&dir)
        };
        // read only by its name: a cell, the edge's function a load, an
        // add and a store of its field
        let ir = with("sum$ << sum$ + x$ forever\n", "n << sum$").unwrap();
        assert!(ir.contains("fn __edge1(__item: int)\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, sum\n    _3: int = add _2, __item\n    _4: __ctx = load _this\n    _5: __ctx = set _4, sum, _3\n    store _5, _this\n    ret\n"), "{}", ir);
        // wired on, and nothing else pushes into it or names it: the
        // stream has no storage and the line keeps its last item
        let ir = with("sum$ << sum$ + x$ forever\nout$ << (sum$ << \"\\n\") forever\n", "n << k").unwrap();
        assert!(ir.contains("    __last1: int\n") && !ir.contains("    sum: int"), "{}", ir);
        assert!(ir.contains("fn __edge1(__item: int)\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, __last1\n    __next: int = add _2, __item\n    _3: __ctx = load _this\n    _4: __ctx = set _3, __last1, __next\n    store _4, _this\n"), "{}", ir);
        // wired on and read by its name as well: no queue (fm3 log
        // 177), its latest item its field, which the line reads, stores
        // and hands on. It was a queue never given back, sixty-four
        // items in its lifetime
        let ir = with("sum$ << sum$ + x$ forever\nout$ << (sum$ << \"\\n\") forever\n", "n << sum$").unwrap();
        assert!(ir.contains("    _3: __ctx = load _this\n    _4: int = get _3, sum\n    _5: int = add _4, __item\n    _6: __ctx = load _this\n    _7: __ctx = set _6, sum, _5\n    store _7, _this\n    if _2\n        __edge2(_5)\n    ret\n"), "{}", ir);
        assert!(!ir.contains("__queue_int") && !ir.contains("latest_queue"), "{}", ir);
        // counted as well, it is read in order and is the queue it
        // was, the read guarded and the queue not given back under it
        let ir = with("sum$ << sum$ + x$ forever\nout$ << (sum$ << \"\\n\") forever\n", "n << sum$ + [count] (frame sum$)").unwrap();
        assert!(ir.contains(" = received(") && ir.contains(" = latest_queue(") && ir.contains("        yield 0\n"), "{}", ir);
        assert!(!ir.contains("free_queue("), "{}", ir);
        // its own name later in the chain, another stream pacing: each
        // item, and then the latest, which is that item
        with("sum$ << (x$ << sum$) forever\n", "n << [count] (frame sum$)").unwrap();
        // a sum of some; a standing map; and a sum under a count
        let ir = with("sum$ << sum$ + x$ if (x$ % 2 == 0) forever\n", "n << sum$").unwrap();
        assert!(ir.contains("    _1: int = rem __item, 2\n    _2: u1 = cmp.eq _1, 0\n    if _2\n        _3: __ctx = load _this\n        _4: int = get _3, sum\n        _5: int = add _4, __item\n"), "{}", ir);
        let ir = with("sum$ << x$ * 2 forever\n", "n << sum$").unwrap();
        assert!(ir.contains("    _1: int = mul __item, 2\n    _2: __ctx = load _this\n    _3: __ctx = set _2, sum, _1\n"), "{}", ir);
        with("sum$ << sum$ + x$ (3) times\n", "n << sum$").unwrap();
        for (lines, said) in [
            // nothing else on its right: never ending, or a clock
            ("y$ << x$ forever\nsum$ << sum$ + 1 forever\n", "h.zero:6: a push into 'sum$' that reads 'sum$' and stands forever would never end: nothing else on its right paces it, and 'sum$' has no rate to. For one more item write it with no `forever`, in a function; for a clock give the stream a rate, `int sum$ at (1 hz)`"),
            // no word: as any line with a stream on its right
            ("sum$ << sum$ + x$\n", "h.zero:5: a push at feature scope happens once, when the store starts (fm3 question 79), and on a line of its own that is not built: a stream's first items go on its declaration, `int sum$ << ...`, and a line that stands is wiring, `sum$ << x$ forever`"),
        ] {
            let err = with(lines, "n << sum$ + [count] (frame y$)").expect_err(lines);
            assert!(err.ends_with(said), "{}: {}", lines, err);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `until` on a push (fm3 question 79, log 148): the test made
    /// after each push, `_` and the stream's own name both the item
    /// just pushed; `if` with any one of the four words of how often
    /// and no two of the four; a push that can be seen never to end
    /// refused; and a line that stands until its condition holds
    #[test]
    fn a_push_goes_on_until() {
        let dir = std::env::temp_dir().join(format!("probe-zero-until-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int a$\nint b$\nint c$\nint seen$ << 0\nb$ << a$ forever\n";
        let rest = "\non (int n) << f (int k)\n    a$ << k\n    n << [count] (frame b$) + [count] (frame c$) + seen$\n\non g (int k)\n    ";
        let with = |lines: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}{}{}\n", head, lines, rest, body)).unwrap();
            emit(&dir)
        };
        let g = |body: &str| -> String {
            let ir = with("", body).unwrap_or_else(|e| panic!("{}: {}", body, e));
            let at = ir.find("\nfn g(k: int)\n").unwrap_or_else(|| panic!("{}: no g", body));
            let end = ir[at + 1..].find("\nfn ").map_or(ir.len(), |n| at + 1 + n);
            ir[at + 1..end].to_string()
        };
        // the push, then the test, of the value in hand: the name and
        // `_` are one program
        let by_name = g("b$ << 1 << (b$ + 1) until (b$ == 5)");
        assert!(by_name.contains("    loop()\n        _4: int = latest_queue(_2)\n        _5: int = add _4, 1\n        push_queue_open(_2, _5)\n        _6: u1 = cmp.eq _5, 5\n        if _6\n            break\n        continue\n"), "{}", by_name);
        assert_eq!(by_name, g("b$ << 1 << (b$ + 1) until (_ == 5)"));
        // `while` beside it: the test, then the push
        assert!(g("b$ << 1 << (b$ + 1) while (_ < 5)").contains("        _6: u1 = cmp.lt _5, 5\n        if _6\n        else\n            break\n        push_queue_open(_2, _5)\n        continue\n"));
        // into a cell: the field stored, then the test
        assert!(g("seen$ << seen$ * 2 until (seen$ > k)").contains("        _3: int = mul _2, 2\n        _4: __ctx = load _this\n        _5: __ctx = set _4, seen, _3\n        store _5, _this\n        _6: u1 = cmp.gt _3, k\n        if _6\n            break\n        continue\n"));
        // `if` with each word of how often, tested once and first
        for body in ["b$ << k if (k > 0) until (_ > 3)", "b$ << k if (k > 0) while (_ < 0)", "b$ << k if (k > 0) (2) times"] {
            let ir = g(body);
            assert!(ir.contains("    _1: u1 = cmp.gt k, 0\n    if _1\n") && ir.contains("        loop("), "{}: {}", body, ir);
        }
        // a line that stands until its condition holds: a bit of the
        // context, the push under "not yet", the bit set after it
        let ir = with("c$ << a$ until (a$ == 3)\n", "c$ << 1").unwrap();
        assert!(ir.contains("    __until2: u1\n") && ir.contains("    _2: u1 = get _1, __until2\n    _3: u1 = cmp.eq _2, 0\n    if _3\n") && ir.contains(": u1 = cmp.eq __item, 3\n"), "{}", ir);
        let two = |a: &str, b: &str| format!("h.zero:12: a push takes {} or {}, not both", a, b);
        for (lines, body, said) in [
            ("", "b$ << k until (_ > 3) while (_ < 9)", two("`until`", "`while`")),
            ("", "b$ << k while (_ < 9) until (_ > 3)", two("`while`", "`until`")),
            ("", "b$ << k until (_ > 3) (2) times", two("`until`", "`(n) times`")),
            ("", "b$ << k (2) times until (_ > 3)", two("`(n) times`", "`until`")),
            ("", "b$ << k until (_ > 3) forever", two("`until`", "`forever`")),
            ("", "b$ << k forever until (_ > 3)", two("`forever`", "`until`")),
            ("", "b$ << k until (_ > 3) until (_ > 4)", "h.zero:12: a push takes one `until`".to_string()),
            ("", "b$ << k until (_ > 3) if (k > 0)", "h.zero:12: `if` comes first on a push, then how often: `x$ << item if (condition) until (...)`".to_string()),
            // a push that can be seen never to end, each word
            ("", "b$ << 1 until (false)", "h.zero:12: this push would never end: its `until` can never hold".to_string()),
            ("", "b$ << 1 while (true)", "h.zero:12: this push would never end: its `while` always holds".to_string()),
            ("", "seen$ << 1 until (false)", "h.zero:12: this push would never end: its `until` can never hold".to_string()),
            ("", "seen$ << 1 while (true)", "h.zero:12: this push would never end: its `while` always holds".to_string()),
            // a block is not asked a condition
            ("", "b$ << [1 through 3] until (_ > 3)", "h.zero:12: a block is pushed once: `until` repeats an item".to_string()),
            // on a line that stands: with `if`, and with more than one
            // item and no brackets, for `until` and for a count (fm3
            // question 84, log 156)
            ("c$ << a$ if (a$ > 0) until (a$ == 3)\n", "c$ << 1", "h.zero:6: `if` with `until` on a line that stands is not built: 'c$ << a$ if (...) until (...)' could ask its `until` of every item, or only of those that pass".to_string()),
            ("c$ << a$ << 0 until (a$ == 3)\n", "c$ << 1", "h.zero:6: 'c$ << a$ << 0 until (...)': `until` applies to the last item of its chain (fm3 question 84), so this is `a$` once and then `0` until the condition holds, when the store starts, and a push then is not built (fm3 question 80). For a line that stands until then, each item of 'a$' with what follows it, put the items in brackets: `c$ << (a$ << 0) until (a$ == 3)`".to_string()),
            ("c$ << a$ << 0 (3) times\n", "c$ << 1", "h.zero:6: 'c$ << a$ << 0 (3) times': `(3) times` applies to the last item of its chain (fm3 question 84), so this is `a$` once and then `0` 3 times, when the store starts, and a push then is not built (fm3 question 80). For the first 3 items of 'a$', each with what follows it, put the items in brackets: `c$ << (a$ << 0) (3) times`".to_string()),
            ("c$ << a$ until (a$ == 3) forever\n", "c$ << 1", "h.zero:6: a push takes `until` or `forever`, not both".to_string()),
        ] {
            let err = with(lines, body).expect_err(body);
            assert!(err.ends_with(&said), "{}{}: {}", lines, body, err);
        }
        // `until (true)` is one push and `while (false)` none: both end
        assert!(g("b$ << 9 until (true)").contains("push_queue_open(_2, _3)"));
        with("", "b$ << 9 while (false)").unwrap();
        // a function may not have the word in its name: it ends a phrase
        let err = with("", "b$ << 1\n\non wait until ready()\n    b$ << 1").expect_err("a name");
        assert!(err.ends_with("h.zero:14: 'until' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written"), "{}", err);
        // in a stream processor, which has no loop
        std::fs::write(dir.join("h/h.zero"), "int a$\nint d$ = dd(a$)\n\non (int d$) << dd (int x$)\n    d$ << x$ until (_ > 3)\n\non (int n) << f (int k)\n    a$ << k\n    n << [count] (frame d$)\n").unwrap();
        let err = emit(&dir).expect_err("a processor");
        assert!(err.ends_with("h.zero:5: `until` on a push repeats it, which is a loop, and a stream processor has none: say the stream by a line of its own, or write `d$ << item if (condition)`"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `(n) times` on a push (fm3 question 79, log 147): the bracket
    /// before the word is the count and never an argument, unless a
    /// declared name has the word there; every line tried, and each
    /// refusal with its message
    #[test]
    fn a_push_takes_a_count() {
        let dir = std::env::temp_dir().join(format!("probe-zero-times-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "int a$\nint b$\nint c$\nb$ << a$ forever\n";
        let rest = "\non (int n) << twice (int k)\n    n << k * 2\n\non (int n) << three (int k) times\n    n << 3 * k\n\non (int n) << f (int k)\n    a$ << k\n    n << [count] (frame b$) + [count] (frame c$)\n\non g (int k)\n    ";
        let with = |lines: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}{}{}\n", head, lines, rest, body)).unwrap();
            emit(&dir)
        };
        let g = |body: &str| -> String {
            let ir = with("", body).unwrap_or_else(|e| panic!("{}: {}", body, e));
            let at = ir.find("\nfn g(k: int)\n").unwrap_or_else(|| panic!("{}: no g", body));
            let end = ir[at + 1..].find("\nfn ").map_or(ir.len(), |n| at + 1 + n);
            ir[at + 1..end].to_string()
        };
        let looped = |n: &str, item: &str| format!("    loop(_3: int = 0)\n        _4: u1 = cmp.lt _3, {}\n        if _4\n        else\n            break\n{}", n, item);
        // a call does not swallow the count, alone or at the right of
        // an operator; a bracketed item keeps its bracket
        assert!(g("b$ << twice (k) (3) times").contains(&looped("3", "        _5: int = twice(k)\n        push_queue_open(_2, _5)\n        _6: int = add _3, 1\n        continue _6\n")));
        assert!(g("b$ << k + twice (k) (3) times").contains(&looped("3", "        _5: int = twice(k)\n        _6: int = add k, _5\n        push_queue_open(_2, _6)\n")));
        assert!(g("b$ << (k) (3) times").contains(&looped("3", "        push_queue_open(_2, k)\n")));
        // a declared name keeps its word: as a statement, as an item in
        // brackets, and before a count of its own
        assert!(g("three (k) times").contains("    _1: int = three_times(k)\n    ret"));
        assert!(g("b$ << (three (k) times)").contains("    _3: int = three_times(k)\n    push_queue_open(_2, _3)\n"));
        assert!(g("b$ << three (k) times (2) times").contains(&looped("2", "        _5: int = three_times(k)\n        push_queue_open(_2, _5)\n")));
        // the count is worked out once, before the first push, and a
        // count that is worked out is checked; a chain's count covers
        // its last item; `if` is tested once, before the count
        let ir = g("b$ << 7 << twice (k) (k) times");
        let (check, first, lp) = (ir.find("cmp.ge k, 0").unwrap(), ir.find("push_queue_open").unwrap(), ir.find("loop(").unwrap());
        assert!(check < first && first < lp && ir[lp..].contains("cmp.lt _5, k"), "{}", ir);
        let ir = g("b$ << k if (k > 0) (2) times");
        assert!(ir.contains("    _1: u1 = cmp.gt k, 0\n    if _1\n") && ir.contains("        loop(_4: int = 0)\n            _5: u1 = cmp.lt _4, 2\n"), "{}", ir);
        // on a declaration, as `while` may be
        // (the stream is read for nothing but its latest, so it is
        // the loop's own value beside the counter, fm3 log 186)
        assert!(g("int d$ << 0 << (d$ + 1) (4) times").contains("    d_2: int = loop(_1: int = 0, d: int = 0)\n        _2: u1 = cmp.lt _1, 4\n"));
        // a line that stands for its first three: the count a field of
        // the context, the push and the bump under "fewer so far"
        let ir = with("c$ << a$ (3) times\n", "c$ << 1").unwrap();
        assert!(ir.contains("    __times2: int\n") && ir.contains("    _2: int = get _1, __times2\n    _3: u1 = cmp.lt _2, 3\n    if _3\n"), "{}", ir);
        let ambiguous = "'... (k) times' at the end of a push reads two ways: a function whose name ends `(...) times`, called and pushed once, or what stands before the bracket pushed that many times. For the call put it in brackets, `x$ << (name (k) times)`; for the count put the item in brackets, `x$ << (item) (k) times`";
        let form = "a push's count is the bracketed group before `times`, after the item: `x$ << item (n) times`";
        for (lines, body, said) in [
            ("", "b$ << three (k) times", format!("h.zero:17: {}", ambiguous)),
            ("", "b$ << three (k) times if (k > 0)", format!("h.zero:17: {}", ambiguous)),
            ("", "b$ << k times", format!("h.zero:17: {}", form)),
            ("", "b$ << (3) times", format!("h.zero:17: {}", form)),
            ("", "b$ << k (k, 2) times", format!("h.zero:17: {}", form)),
            ("", "int x = k (3) times", "h.zero:17: `times` is a word a push takes, `x$ << item (n) times`".to_string()),
            ("", "b$ << k (-1) times", "h.zero:17: a push cannot happen -1 times".to_string()),
            ("", "b$ << k (1.5) times", "h.zero:17: a decimal where an int is wanted".to_string()),
            ("", "b$ << k (2) times if (k > 0)", "h.zero:17: `if` comes first on a push, then how often: `x$ << item if (condition) (n) times`".to_string()),
            ("", "b$ << k (2) times while (_ < 3)", "h.zero:17: a push takes `(n) times` or `while`, not both".to_string()),
            ("", "b$ << k while (_ < 3) (2) times", "h.zero:17: a push takes `while` or `(n) times`, not both".to_string()),
            ("", "b$ << k (2) times forever", "h.zero:17: a push takes `(n) times` or `forever`, not both".to_string()),
            ("", "b$ << k forever (2) times", "h.zero:17: a push takes `forever` or `(n) times`, not both".to_string()),
            ("", "b$ << k (2) times (3) times", "h.zero:17: a push takes one `(n) times`".to_string()),
            // on a line that stands: a count worked out, a count below
            // zero, a count with `if`, a count with `forever`
            ("int n = 3\nc$ << a$ (n) times\n", "c$ << 1", "h.zero:6: the count of a line that stands is a number written out, `c$ << a$ (3) times`: a count that is worked out is worked out once, and a line that stands from the start has no one moment for it. Not built".to_string()),
            ("c$ << a$ (-2) times\n", "c$ << 1", "h.zero:5: a push cannot happen -2 times".to_string()),
            ("c$ << a$ if (a$ > 0) (3) times\n", "c$ << 1", "h.zero:5: `if` with a count on a line that stands is not built: 'c$ << a$ if (...) (3) times' could be the first 3 that pass, or those of the first 3 that pass".to_string()),
            ("c$ << a$ (3) times forever\n", "c$ << 1", "h.zero:5: a push takes `(n) times` or `forever`, not both".to_string()),
        ] {
            let err = with(lines, body).expect_err(body);
            assert!(err.ends_with(&said), "{}{}: {}", lines, body, err);
        }
        // in a stream processor, which has no loop
        std::fs::write(dir.join("h/h.zero"), "int a$\nint d$ = dd(a$)\n\non (int d$) << dd (int x$)\n    d$ << x$ (2) times\n\non (int n) << f (int k)\n    a$ << k\n    n << [count] (frame d$)\n").unwrap();
        let err = emit(&dir).expect_err("a processor");
        assert!(err.ends_with("h.zero:5: `(n) times` on a push repeats it, which is a loop, and a stream processor has none: say the stream by a line of its own, or write `d$ << item if (condition)`"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream that keeps nothing does not fill (fm3 log 177): one a
    /// line reads for its value now is a cell, and one that is wired on
    /// and read by its name has no queue, its latest item one word
    /// stored where each item is handed on
    #[test]
    fn a_stream_that_keeps_nothing_does_not_fill() {
        let dir = std::env::temp_dir().join(format!("probe-zero-nowed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 3\n").unwrap();
        let head = "int up$\nint gate$\nint open$\n\nout$ << (up$ << \"\\n\") forever\nopen$ << up$ if (gate$ == 0) forever\n\non (int n) << f()\n    up$ << 1\n    gate$ << 1\n    up$ << up$ + 1\n    n << up$ + open$\n";
        let with = |more: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\n", head, more)).unwrap();
            emit(&dir)
        };
        let ir = with("").unwrap();
        // no queue anywhere: `up$` is wired twice and read by name,
        // `gate$` is read by a line's condition, `open$` by name
        assert!(!ir.contains("__queue_int") && !ir.contains("latest_queue"), "{}", ir);
        assert!(ir.contains("    up: int\n    gate: int\n    open: int\n"), "{}", ir);
        assert!(ir.contains(";   up: user, last (h), no queue: its latest item is kept, one word, stored where a push into it calls its edges"), "{}", ir);
        // the store comes before the calls
        assert!(ir.contains("    _7: __ctx = set _6, up, _5\n    store _7, _this\n    if _2\n        __edge1(_5)\n    if _2\n        __edge2(_5)\n"), "{}", ir);
        // its own name on the right of its own push is the field
        assert!(ir.contains("    _15: int = get _14, up\n    _16: int = add _15, 1\n    _17: __ctx = load _this\n    _18: __ctx = set _17, up, _16\n"), "{}", ir);
        // the line's condition reads the cell
        assert!(ir.contains("fn __edge2(__item: int)\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, gate\n    _3: u1 = cmp.eq _2, 0\n"), "{}", ir);
        // a stream that is wired and that nothing names is as it was:
        // no storage and no field
        let bare = with("int quiet$\nout$ << (quiet$ << \"\\n\") forever\n\non g()\n    quiet$ << 1").unwrap();
        assert!(bare.contains(";   quiet: no storage, no word reading it: a push into it calls its edges") && !bare.contains("    quiet: int"), "{}", bare);
        // one more word and it is read in order: a queue, as it was
        for more in ["\non (int n) << g()\n    n << [count] (frame up$)", "\non (int n) << g()\n    n << peek up$ at (0)"] {
            let stored = with(more).unwrap();
            assert!(stored.contains("    up: int$\n") && stored.contains("__queue_int"), "{}", stored);
        }
        // ended, it keeps its one word and a bit beside it (fm3 log
        // 180): `end` reads the bit and sets it, a push asks it, and
        // `ended` is the bit read
        let ended = with("\non (bool b) << g()\n    end up$\n    b << ended up$").unwrap();
        assert!(ended.contains("    up: int\n") && ended.contains("    __zend_up: u1\n") && !ended.contains("__queue_int") && !ended.contains("= ended("), "{}", ended);
        assert!(ended.contains("fn g() -> u1\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u1 = get _1, __zend_up\n    if _2\n    else\n        _3: u1 = const 1\n"), "{}", ended);
        assert!(ended.contains("    _2: u1 = get _1, __zend_up\n    _3: u1 = xor _2, 1\n    check _3\n"), "{}", ended);
        // given a first item where it is declared it keeps one word
        // too: zero at the reset, and the item stored and handed on
        // where the store starts, after the case's context is set
        std::fs::write(dir.join("h/h.zero"), head.replacen("int up$\n", "int up$ << 7\n", 1)).unwrap();
        let first = emit(&dir).unwrap();
        assert!(first.contains("    up: int\n") && !first.contains("__queue_int"), "{}", first);
        assert!(first.contains("fn __zero_start()\n    __first1()\n    ret\n"), "{}", first);
        assert!(first.contains("fn __first1()\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u1 = get _1, __enabled_h\n    _5: int = const 7\n    _6: __ctx = load _this\n    _7: __ctx = set _6, up, _5\n    store _7, _this\n    if _2\n        __edge1(_5)\n    if _2\n        __edge2(_5)\n    ret\n"), "{}", first);
        // a first item and a rate both: each first item has a time,
        // and it is the queue it was
        std::fs::write(dir.join("h/h.zero"), head.replacen("int up$\n", "int up$ at (1 hz) << 7\n", 1)).unwrap();
        let both = emit(&dir).unwrap();
        assert!(both.contains("    up: int$\n"), "{}", both);
        // a later item of a line that stands that is another stream's
        // name is its value now (fm3 question 106): a cell's field
        // read, and no queue
        std::fs::write(dir.join("h/h.zero"), "int x$\nint y$\n\nout$ << (x$ << \" \" << y$ << \"\\n\") forever\n\non (int n) << f()\n    y$ << 3 << 4\n    x$ << 1\n    n << 3\n").unwrap();
        let later = emit(&dir).unwrap();
        assert!(later.contains("    y: int\n") && !later.contains("__queue_int") && !later.contains("__out__ints(_"), "{}", later);
        // the output of a stream processor, read only by its name:
        // a cell its function stores, with no end to tell
        std::fs::write(dir.join("h/h.zero"), "int x$\nint d$ = doubled (x$)\n\non (int d$) << doubled (int x$)\n    d$ << x$ * 2\n\non (int n) << f()\n    x$ << 1 << 2\n    n << d$ - 1\n").unwrap();
        let output = emit(&dir).unwrap();
        assert!(output.contains("    d: int\n") && !output.contains("__queue_int") && !output.contains("end("), "{}", output);
        // with a rate it keeps its latest and its step
        std::fs::write(dir.join("h/h.zero"), head.replacen("int up$\n", "int up$ at (1 hz)\n", 1)).unwrap();
        let rated = emit(&dir).unwrap();
        assert!(rated.contains("    up: int\n") && !rated.contains("__queue_int") && rated.contains("__step("), "{}", rated);
        // the input of a processor, read by name
        std::fs::write(dir.join("h/h.zero"), "int x$\nint d$ = doubled (x$)\nint sum$\nsum$ << sum$ + d$ forever\n\non (int d$) << doubled (int x$)\n    d$ << x$ * 2\n\non (int n) << f()\n    x$ << 1 << 2\n    n << x$ + sum$ - 5\n").unwrap();
        let fed = emit(&dir).unwrap();
        assert!(fed.contains("    x: int\n") && !fed.contains("__queue_int"), "{}", fed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream read only for its latest item is one word (fm3 question
    /// 70, log 143): a cell, a field of the context of the item's type,
    /// a push a store of it and a read a load, with no stream made. A
    /// stream's name where one value is wanted is its latest item
    /// (question 79), of a cell or of any other stream. And the same
    /// stream with one more word applied to it, `count`, is the queue
    /// it was: the words choose what is kept
    #[test]
    fn a_stream_read_for_its_latest_is_one_word() {
        let dir = std::env::temp_dir().join(format!("probe-zero-cell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 2\n").unwrap();
        let head = "int seen$ << 0\n\non bump()\n    seen$ << seen$ + 1\n\non (int n) << f()\n    bump()\n    int x = seen$ + 1\n    n << x\n";
        let with = |more: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\n", head, more)).unwrap();
            emit(&dir)
        };
        let ir = with("").unwrap();
        assert!(ir.contains("    seen: int\n"), "{}", ir);
        assert!(!ir.contains("__queue_int") && !ir.contains("latest_queue") && !ir.contains("= latest "), "{}", ir);
        assert!(ir.contains("fn bump()\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, seen\n    _3: int = add _2, 1\n    _4: __ctx = load _this\n    _5: __ctx = set _4, seen, _3\n    store _5, _this\n    ret\n"), "{}", ir);
        assert!(ir.contains("    bump()\n    _1: __ctx = load _this\n    _2: int = get _1, seen\n    x: int = add _2, 1\n"), "{}", ir);
        // `latest` says the same, and so does a second context's first value
        let said = with("\non (int n) << g()\n    n << latest seen$").unwrap();
        assert!(said.contains("fn g() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    n: int = get _1, seen\n"), "{}", said);
        // one more word, and it is a queue: the name still reads its latest
        let counted = with("\non (int n) << g()\n    n << [count] (frame seen$)").unwrap();
        assert!(counted.contains("    seen: int$\n") && counted.contains("__queue_int"), "{}", counted);
        // (before its first item the zero, as the cell reads, fm3 log 163)
        assert!(counted.contains("_3: index = received(_2)\n    _4: u1 = cmp.gt _3, 0\n    _5: int = if _4\n        _6: int = latest_queue(_2)\n        yield _6\n    else\n        yield 0\n    x: int = add _5, 1\n"), "{}", counted);
        // an item of a push that happens once is its value now (fm3
        // question 79, log 163): the line that pushed everything
        // unread reads the cell, and the stream is one still
        let whole = with("\non g()\n    out$ << seen$").unwrap();
        assert!(whole.contains("    seen: int\n") && whole.contains("fn g()\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, seen\n    __out__int(_2)\n"), "{}", whole);
        // ... through an operator with another stream, both cells
        let two = with("int other$ << 0\n\non g()\n    seen$ << seen$ + other$").unwrap();
        assert!(two.contains("    other: int\n") && two.contains("fn g()\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, seen\n    _3: __ctx = load _this\n    _4: int = get _3, other\n    _5: int = add _2, _4\n"), "{}", two);
        // ... and an array there is the array, whole
        let arr = with("\non g()\n    int a[] = [1, 2]\n    out$ << a[] * 2").unwrap();
        assert!(arr.contains("fn g()") && arr.contains("__out__ints("), "{}", arr);
        // a function that takes the stream whole takes it, as it did
        let taken = with("\non (int n) << total (int x$)\n    n << [count] (frame x$)\n\non (int n) << total (int x)\n    n << x\n\non (int n) << g()\n    n << total (seen$)").unwrap();
        assert!(taken.contains("    seen: int$\n"), "{}", taken);
        // ... and where every method takes one value, the name is one
        let one = with("\non (int n) << twice (int x)\n    n << x * 2\n\non (int n) << g()\n    n << twice (seen$)").unwrap();
        assert!(one.contains("    seen: int\n") && one.contains("    _2: int = get _1, seen\n    n: int = twice(_2)\n"), "{}", one);
        // a local stream's name reads its latest where one value is
        // wanted: the value itself where the function reads it for
        // nothing else (fm3 log 186), `latest` where it is counted too
        let local = with("\non (int n) << g()\n    int i$ << 4 << 5\n    int y = i$ + 1\n    n << y").unwrap();
        assert!(local.contains("fn g() -> int\n    _1: int = const 5\n    y: int = add _1, 1\n    ret y\n"), "{}", local);
        let local = with("\non (int n) << g()\n    int i$ << 4 << 5\n    int y = i$ + peek i$ at (0)\n    n << y").unwrap();
        assert!(local.contains(" = latest_queue(i)\n") && local.contains("    y: int = add "), "{}", local);
        // a cell holds what a ring does not; used as a stream it is refused as it was
        let flag = with("bool up$\n\non (bool b) << g()\n    up$ << true\n    b << up$").unwrap();
        assert!(flag.contains("    up: u1\n"), "{}", flag);
        let err = with("bool up$\n\non (int n) << g()\n    up$ << true\n    n << [count] (frame up$)").expect_err("a stream of bool");
        assert!(err.ends_with("h.zero:10: a stream of bool: a stream holds numbers, enumerations or structs of those"), "{}", err);
        let err = with("\non (int n) << g()\n    int y = out$\n    n << y").expect_err("the device");
        assert!(err.ends_with("h.zero:12: 'out$' is the output device: it is written and never read, so it has no latest item"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream said by a rule inside a function is one value carried
    /// round its loop (fm3 log 186): the lines of the `loop` a person
    /// would write, a structure's fields carried apart and the
    /// structure never made, `while` tested first where it does not
    /// read `_` (question 111); and every word that makes it the
    /// stream it was, each read the same
    #[test]
    fn a_stream_said_in_a_function_is_one_value() {
        let dir = std::env::temp_dir().join(format!("probe-zero-lcell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (10) → 16\n").unwrap();
        let head = "type pair =\n    int x, y\n\nint kept$ << 0\n\non (int n) << how many (int x$)\n    n << count x$\n\non (int p) << f (int n)\n    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    p << q$\n";
        let with = |more: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\n", head, more)).unwrap();
            emit(&dir)
        };
        // the loop's own lines, and the loop written out beside it
        let ir = with("\non (int p) << l (int n)\n    p << loop (int q = 1) yields q\n        if (q > n)\n            break\n        continue (q * 2)").unwrap();
        assert!(ir.contains("fn f(n: int) -> int\n    q_2: int = loop(q: int = 1)\n        _1: u1 = cmp.le q, n\n        if _1\n        else\n            break q\n        _2: int = mul q, 2\n        continue _2\n    ret q_2\n"), "{}", ir);
        assert!(ir.contains("fn l(n: int) -> int\n    p: int = loop(q: int = 1)\n        _1: u1 = cmp.gt q, n\n        if _1\n            break q\n        _2: int = mul q, 2\n        continue _2\n    ret p\n"), "{}", ir);
        assert!(!ir.contains("__queue_int(") && !ir.contains("latest_queue("), "{}", ir);
        // a condition that reads `_`: the item first, then the test
        let cand = with("\non (int p) << g (int n)\n    int q$ << 1 << (q$ * 2) while (_ <= n)\n    p << q$").unwrap();
        assert!(cand.contains("fn g(n: int) -> int\n    q_2: int = loop(q: int = 1)\n        _1: int = mul q, 2\n        _2: u1 = cmp.le _1, n\n        if _2\n        else\n            break q\n        continue _1\n    ret q_2\n"), "{}", cand);
        // `until`, and a count
        let until = with("\non (int p) << g (int n)\n    int q$ << 1 << (q$ * 2) until (q$ > n)\n    p << q$").unwrap();
        assert!(until.contains("    q_2: int = loop(q: int = 1)\n        _1: int = mul q, 2\n        _2: u1 = cmp.gt _1, n\n        if _2\n            break _1\n        continue _1\n    ret q_2\n"), "{}", until);
        let times = with("\non (int p) << g (int n)\n    int q$ << 1 << (q$ * 2) (n) times\n    p << q$").unwrap();
        assert!(times.contains("fn g(n: int) -> int\n    _1: u1 = cmp.ge n, 0\n    check _1\n    q_2: int = loop(_2: int = 0, q: int = 1)\n        _3: u1 = cmp.lt _2, n\n        if _3\n        else\n            break q\n        _4: int = mul q, 2\n        _5: int = add _2, 1\n        continue _5, _4\n    ret q_2\n") && !times.contains("print"), "{}", times);
        // a structure: its fields apart, no `pack` and no `get`, and
        // the remainder taken only where the test has held
        let gcd = with("\non (int g) << gcd (int a) and (int b)\n    pair p$ << pair(a, b) << pair(p$.y, p$.x % p$.y) while (p$.y != 0)\n    g << p$.x").unwrap();
        assert!(gcd.contains("fn gcd_and(a: int, b: int) -> int\n    p_x_2: int, p_y_2: int = loop(p_x: int = a, p_y: int = b)\n        _1: u1 = cmp.ne p_y, 0\n        if _1\n        else\n            break p_x, p_y\n        _2: int = rem p_x, p_y\n        continue p_y, _2\n    ret p_x_2\n"), "{}", gcd);
        // ... made once where it is read whole
        let whole = with("\non (pair r) << g (int a)\n    pair p$ << pair(a, 1) << pair(p$.y, p$.x) (3) times\n    r << p$").unwrap();
        assert_eq!(whole.matches(": pair = pack ").count(), 1, "{}", whole);
        assert!(whole.contains("    r: pair = pack p_x_2, p_y_2\n    ret r\n"), "{}", whole);
        // a push under an `if` is joined as any local is
        let joined = with("\non (int p) << g (int n)\n    int q$ << 1\n    q$ << q$ + n if (n > 0)\n    p << q$").unwrap();
        assert!(joined.contains("    q: int = if _1\n        _2: int = add 1, n\n        yield _2\n    else\n        yield 1\n    ret q\n"), "{}", joined);
        // `latest` of it says the same, and keeps no history: the
        // store's other streams are the queues they were
        let said = with("\non (int p) << g (int n)\n    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    int r$ << 1 << 2\n    p << latest q$ + peek r$ at (1)").unwrap();
        assert!(said.contains("    r: int$ = __queue_int(") && !said.contains("__regular_int") && !said.contains("latest_queue(q)"), "{}", said);
        // before its first item, the zero of its type
        let zero = with("\non (int p) << g()\n    int q$\n    p << q$ + 1").unwrap();
        assert!(zero.contains("fn g() -> int\n    _1: int = const 0\n    p: int = add _1, 1\n"), "{}", zero);
        // each of these makes it the stream it was: a queue is made
        for (what, body) in [
            ("peek", "    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    p << q$ + peek q$ at (0)"),
            ("frame", "    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    int a[] = frame q$\n    p << q$"),
            ("ended", "    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    end q$\n    p << q$"),
            ("a function that takes a stream", "    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    p << q$ + how many (q$)"),
            ("a list pushed", "    int q$ << [1, 2]\n    p << q$ + n"),
            ("a push inside a `for`, which carries nothing", "    int q$ << 1\n    for (i in [1 through n])\n        q$ << q$ * 2\n    p << q$"),
            ("a push of it whole into a stream that is stored", "    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    int r$ << frame q$\n    p << q$ + count r$"),
        ] {
            let ir = with(&format!("\non (int p) << g (int n)\n{}", body)).unwrap_or_else(|e| panic!("{}: {}", what, e));
            let g = &ir[ir.find("fn g(n: int) -> int").unwrap()..];
            assert!(g[..g.find("    ret").unwrap()].contains("q: int$ = __queue_int("), "{}: {}", what, ir);
            assert!(!ir.contains("__lcell_"), "{}: {}", what, ir);
        }
        // `count` of it is how many were pushed, a second value carried
        // beside the latest (fm3 question 112, log 190): no queue
        let counted = with("\non (int p) << g (int n)\n    int q$ << n << (q$ / 10) while (q$ >= 10)\n    p << count q$").unwrap();
        assert!(counted.contains("fn g(n: int) -> int\n    q_2: int, q__n_2: index = loop(q: int = n, q__n: index = 1)\n        _1: u1 = cmp.ge q, 10\n        if _1\n        else\n            break q, q__n\n        _2: int = div q, 10\n        _3: index = add q__n, 1\n        continue _2, _3\n    p: int = conv q__n_2\n    ret p\n"), "{}", counted);
        // ... one nothing counts carries no count, and three pushed is 3
        assert!(!with("\non (int p) << g (int n)\n    int q$ << n << (q$ / 10) while (q$ >= 10)\n    p << q$").unwrap().contains("__n"));
        assert!(with("\non (int p) << g()\n    int q$ << 4 << 5 << 6\n    p << count q$").unwrap().contains("fn g() -> int\n    _1: index = const 3\n    p: int = conv _1\n"));
        // a push into it inside a `loop` that began after it: the
        // loop carries what it keeps, as it carries a stream it moves
        let round = with("\non (int p) << g (int n)\n    int q$\n    loop (int i = 1) while (i <= n)\n        q$ << q$ + i\n        continue (i + 1)\n    p << q$").unwrap();
        assert!(round.contains("fn g(n: int) -> int\n    q_2: int = loop(i: int = 1, q: int = 0)\n        _1: u1 = cmp.le i, n\n        if _1\n        else\n            break q\n        _2: int = add q, i\n        _3: int = add i, 1\n        continue _3, _2\n    ret q_2\n"), "{}", round);
        // in a `for` it is a queue, and before its first item its own
        // name is still the zero of its type, not a failed check
        let by_for = with("\non (int p) << g (int n)\n    int q$\n    for (i in [1 through n])\n        q$ << q$ + i\n    p << q$").unwrap();
        assert!(by_for.contains("q: int$ = __queue_int(") && by_for.contains(" = received(q)\n"), "{}", by_for);
        // ... and a refusal in a function with such a stream is said
        // of the program as written, the stream a stream again
        let err = with("\non (int p) << g (int n)\n    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    p << q$ + nothing (n)").expect_err("no such function");
        assert!(err.ends_with("h.zero:15: no function named 'nothing'"), "{}", err);
        // a stream at feature scope and one that is stored take the
        // test first too: the latest read, the test, and only then the rule
        let fs = with("\non g (int n)\n    kept$ << (kept$ + 1) while (kept$ < n)").unwrap();
        let g = &fs[fs.find("fn g(n: int)").unwrap()..];
        assert!(g.find("cmp.lt").unwrap() < g.find(" = add ").unwrap(), "{}", fs);
        let stored = with("\non (int p) << g (int n)\n    int q$ << 1 << (q$ * 2) while (q$ <= n)\n    p << count q$ + peek q$ at (0)").unwrap();
        let g = &stored[stored.find("fn g(n: int) -> int").unwrap()..];
        assert!(g.find("cmp.le").unwrap() < g.find(" = mul ").unwrap(), "{}", stored);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A range, and a map of one over it, is read where it is used
    /// (fm3 log 187): summed, a function applied to it, or pushed into
    /// a stream of its own kind of item, it is the loop that counts and
    /// no array; anything else is the array it was
    #[test]
    fn a_range_used_once_is_never_made() {
        let dir = std::env::temp_dir().join(format!("probe-zero-range-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 6\n").unwrap();
        let head = "int kept$\n\non (int d) << twice (int x)\n    d << x * 2\n\non note (int x)\n    kept$ << x\n\non (int n) << how many (int x$)\n    n << [count] (frame x$)\n\non (int n) << held()\n    n << [count] (frame kept$)\n\non (int n) << f (int k)\n    n << [1 through k] + _\n";
        let g = |body: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("{}\non (int n) << g (int k)\n{}\n", head, body)).unwrap();
            let ir = emit(&dir).unwrap_or_else(|e| panic!("{}: {}", body, e));
            let from = ir.find("fn g(k: int) -> int").unwrap();
            ir[from..from + ir[from..].find("    ret").unwrap()].to_string()
        };
        let made = |ir: &str| ir.contains("__queue_int(") || ir.contains("__regular_int(");
        // summed: literal bounds, the plain counted loop with the sum beside the counter
        let lit = g("    n << [1 through 33] + _");
        assert!(lit.contains("    n: int = loop(_2: int = 1, _1: int = 0)\n        _3: u1 = cmp.le _2, 33\n        if _3\n        else\n            break _1\n        _4: int = add _1, _2\n        _5: int = add _2, 1\n        continue _5, _4\n"), "{}", lit);
        // ... and bounds worked out, with a map of one, and a function of one item
        for body in ["    n << [1 through k] + _", "    n << _ + [1 through k] * 2", "    n << (k - [1 through k]) * 2 + _", "    n << twice ([1 to k + 1]) + _"] {
            let ir = g(body);
            assert!(!made(&ir) && ir.contains("    n: int = loop(") && !ir.contains(" = sum "), "{}: {}", body, ir);
        }
        assert!(g("    n << twice ([1 to k + 1]) + _").contains(": int = twice(_"), "the call is in the loop");
        // a function applied to it, a statement: the range's loop with
        // the function's own line in it, `note` being small and giving
        // nothing (fm3 log 191); one that gives a result is still called
        let each = g("    note ([1 through k])\n    n << kept$");
        assert!(!made(&each) && !each.contains("note(") && each.contains("        push_queue_open(_12, _9)\n"), "{}", each);
        let both = g("    note (twice ([1 through k]) + 1)\n    n << kept$");
        assert!(!made(&both) && both.contains(": int = twice(_") && !both.contains("note("), "{}", both);
        // pushed into a stream of its own kind of item
        let pushed = g("    kept$ << twice ([1 through k])\n    n << kept$");
        assert!(!made(&pushed) && pushed.contains(": int = twice(_"), "{}", pushed);
        // each of these is the array it was
        for (what, body) in [
            ("given a name", "    int a[] = [1 through k] * 2\n    n << a[] + _"),
            ("zipped", "    n << [1 through k] * [1 through k] + _"),
            ("another operator", "    n << [1 through k] * _"),
            ("pushed where a method takes the array", "    out$ << twice ([1 through k])\n    n << k"),
        ] {
            assert!(made(&g(body)), "{}: {}", what, g(body));
        }
        // ... and a function that takes a stream is refused it, as it was
        std::fs::write(dir.join("h/h.zero"), format!("{}\non (int n) << g (int k)\n    n << how many ([1 through k])\n", head)).unwrap();
        let err = emit(&dir).expect_err("an array handed to a stream's parameter");
        assert!(err.contains("'how many' takes a stream here"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What changes at feature scope is a stream (fm3 question 70, log
    /// 145): an assignment to a feature-scope name is refused, with the
    /// declaration to write and the push; so is one to a parameter. A
    /// string that changes is a stream only a cell can hold, its bare
    /// name its latest item wherever it stands (question 83), and a
    /// word that wants it as a stream is refused as a stream of strings
    /// always was
    #[test]
    fn what_changes_at_feature_scope_is_a_stream() {
        let dir = std::env::temp_dir().join(format!("probe-zero-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let head = "type Vec =\n    float x, y = 0\n\ngroup int quota = 100 merge sum\nVec origin(1, 2)\nint size\nstring name$ << \"zero\"\n\non (int a, int b) << two()\n    a << 1\n    b << 2\n\n";
        let with = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f (int k)\n{}\n", head, body)).unwrap();
            emit(&dir)
        };
        let how = "is a variable, and a variable keeps the value it was declared with (fm3 question 70): what changes is a stream. Declare it";
        for (body, said) in [
            ("    quota = k\n    n << quota", format!("h.zero:14: 'quota' {} `group int quota$ << 100 merge sum` and push its next value, `quota$ << k`; its name, `quota$`, is then its latest item wherever one value is wanted", how)),
            ("    origin = Vec(3, 4)\n    n << k", format!("h.zero:14: 'origin' {} `Vec origin$ << Vec(1, 2)` and push its next value, `origin$ << Vec (3, 4)`; its name, `origin$`, is then its latest item wherever one value is wanted", how)),
            ("    size = size + k\n    n << size", format!("h.zero:14: 'size' {} `int size$` and push its next value, `size$ << ...`; its name, `size$`, is then its latest item wherever one value is wanted", how)),
            ("    size, quota = two()\n    n << size", format!("h.zero:14: 'size' {} `int size$` and push its next value, `size$ << ...`; its name, `size$`, is then its latest item wherever one value is wanted", how)),
            ("    size = loop (int i = 0) while (i < k) yields i\n        continue (i + 1)\n    n << size", format!("h.zero:14: 'size' {} `int size$` and push its next value, `size$ << ...`; its name, `size$`, is then its latest item wherever one value is wanted", how)),
            ("    name$ = \"one\"\n    n << k", "h.zero:14: 'name$' is a stream: it is pushed into, `name$ << \"one\"`, not assigned".to_string()),
            ("    k = k + 1\n    n << k", "h.zero:14: 'k' is a parameter: it is what the function was handed, and is not assigned".to_string()),
            ("    n << [count] (frame name$)", "h.zero:7: a stream of string: a stream holds numbers, enumerations or structs of those".to_string()),
        ] {
            let err = with(body).expect_err(body);
            assert!(err.ends_with(&said), "{}: {}", body, err);
        }
        // the string pushed and written: the field a string variable
        // had, and its name in a push its value now
        let ir = with("    name$ << \"one\"\n    out$ << name$ << \"\\n\"\n    string s = name$\n    n << count s").unwrap();
        assert!(ir.contains("    name: u8$\n") && ir.contains(" = set ") && ir.contains(", name, "), "{}", ir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A loop's variables are given by `continue` and never assigned
    /// (fm3 question 70, log 144), and `break (values)` gives the names
    /// the loop yields their values where it leaves (question 81): the
    /// IR's own `break`, so a loop that left by assigning and then
    /// `break` lowers to no more than it did
    #[test]
    fn a_loops_variables_are_given_by_continue() {
        let dir = std::env::temp_dir().join(format!("probe-zero-continue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (3) → 1\n").unwrap();
        let with = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) << f (int k)\n{}\n", body)).unwrap();
            emit(&dir)
        };
        let ir = with("    n << loop (int i = 0, int m = -1) yields m\n        if (i > k)\n            break\n        if (i * i >= k)\n            break (i)\n        continue (i + 1, -1)").unwrap();
        assert!(ir.contains("        if _1\n            break m\n") && ir.contains("        if _3\n            break i\n"), "{}", ir);
        for (body, said) in [
            ("    n << loop (int i = 0) while (i < k) yields i\n        i = i + 1", "h.zero:3: 'i' is the loop's own: it is not assigned in the loop's body. Give its next value with `continue (...)`, and the loop's result where it leaves with `break (...)`"),
            ("    n << loop (int i = 0, int m = 0) yields m\n        break (i, m)", "h.zero:3: the loop yields 1 name(s), 'break' gives 2"),
            ("    loop (int i = 0)\n        break (i)\n    n << 1", "h.zero:3: the loop yields nothing, and 'break' gives a value: name what comes out with `yields` at the end of the loop's first line"),
            ("    for (i in [1 through k])\n        break (i)\n    n << 1", "h.zero:3: a `for` gives nothing: 'break' takes no values here"),
        ] {
            let err = with(body).expect_err(body);
            assert!(err.ends_with(said), "{}: {}", body, err);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A condition several lines turn on is branched on once (fm3 log
    /// 133): two lines said `if (new$) ...` and a push that goes out
    /// `if (new$ and ...)` are one `if` in the function of one item,
    /// each line's name a result of it and the push in the arm where
    /// the condition holds. What is left of a push's condition waits
    /// for the branch only where it can do nothing but give a value:
    /// with a call or a division in it the push stays after the
    /// branch, its whole condition worked out at every item, both
    /// sides of `and` being always worked out (question 66)
    #[test]
    fn a_condition_is_branched_on_once() {
        let dir = std::env::temp_dir().join(format!("probe-zero-branched-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 2\n").unwrap();
        let head = "int x$\nint d$ = runs(x$)\n\non (bool b) << big (int x)\n    b << x > 100\n\non (int n) << f()\n    x$ << 1 << 1 << 2\n    n << count d$\n\n";
        let with = |body: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int d$) << runs (int x$)\n    bool new$ = x$ != x$[-1]\n    int n$ = 1 if (new$) else n$[-1] + 1\n{}\n", head, body)).unwrap();
            let ir = emit(&dir).unwrap();
            let at = ir.find("fn __z1_each").unwrap();
            let rest = &ir[at..];
            rest[..rest.find("\nfn ").unwrap().min(rest.find("\n\n").unwrap_or(rest.len()))].to_string() + "\n"
        };
        // two lines and a push on one condition: one branch, no `and`
        let f = with("    int first$ = x$ if (new$) else first$[-1]\n    d$ << first$[-1] + n$[-1] if (new$ and n$[-1] > 0)");
        assert!(f.contains("    _first: int, _n_3: int = if _new\n        _n: int = const 1\n        _1: u1 = cmp.gt __n_b1, 0\n        if _1\n"), "{}", f);
        assert!(f.contains("        yield _x, _n\n    else\n        _n_2: int = add __n_b1, 1\n        yield __first_b1, _n_2\n    ret _first, _n_3\n"), "{}", f);
        assert_eq!(f.matches("if ").count(), 2, "{}", f);
        assert!(!f.contains(" and "), "{}", f);
        // a push whose condition is the name alone is made in the arm
        let f = with("    d$ << n$[-1] if (new$)");
        assert!(f.contains("    _n_3: int = if _new\n        _n: int = const 1\n        _1: __ctx = load _this\n"), "{}", f);
        assert_eq!(f.matches("if ").count(), 1, "{}", f);
        // a call in what is left: the push stays after, its condition whole
        let f = with("    d$ << n$[-1] if (new$ and big (x$))");
        assert!(f.contains("    _n: int = if _new\n        yield 1\n    else\n"), "{}", f);
        assert!(f.contains(": u1 = big(_x)\n    _3: u1 = and _new, _2\n    if _3\n"), "{}", f);
        // ... and a division, which can stop a machine
        let f = with("    d$ << n$[-1] if (new$ and 10 / x$ > 1)");
        assert!(f.contains(": u1 = and _new, "), "{}", f);
        // one push alone on a condition is as it was: `and`, one branch
        std::fs::write(dir.join("h/h.zero"), format!("{}on (int d$) << runs (int x$)\n    bool new$ = x$ != x$[-1]\n    d$ << x$ if (new$ and x$ > 0)\n", head)).unwrap();
        let ir = emit(&dir).unwrap();
        assert!(ir.contains("    _new: u1 = cmp.ne _x, __x_b1\n    _1: u1 = cmp.gt _x, 0\n    _2: u1 = and _new, _1\n    if _2\n"), "{}", ir);
        // an enumeration's case in what is left is a constant, bare or
        // said with its type, and waits for the branch as a number
        // does (fm3 log 168): the lexer's `k$[-1] != space`
        for case in ["dark", "shade.dark"] {
            std::fs::write(dir.join("h/h.zero"), format!("type shade = dark | light\n\n{}on (shade s) << shade of (int x)\n    s << (light if (x > 1) else dark)\n\non (int d$) << runs (int x$)\n    shade s$ = shade of (x$)\n    bool new$ = s$ != s$[-1]\n    int n$ = 1 if (new$) else n$[-1] + 1\n    d$ << n$[-1] if (new$ and s$[-1] != {})\n", head, case)).unwrap();
            let ir = emit(&dir).unwrap();
            let at = ir.find("fn __z1_each").unwrap();
            let f = &ir[at..at + ir[at..].find("\n\n").unwrap()];
            assert!(f.contains("    _n_3: int = if _new\n        _n: int = const 1\n        _1: u1 = cmp.ne __s_b1, 0\n        if _1\n"), "{}: {}", case, f);
            assert!(!f.contains(" and "), "{}: {}", case, f);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A block pushed into a stream processor's input has the
    /// processor's lines in the loop the front end writes (fm3 log
    /// 134): no call an item. A single item is the call still, and so
    /// the function is written where one is pushed and not where every
    /// push is a block. The lines see the processor's names and the
    /// store's, never the pushing function's own: a local of the
    /// pusher named as the processor's output is not what the lines
    /// push into. And they are the wiring's feature's, so a pusher in
    /// a lower layer hands a block to a processor wired above it, as
    /// its call did
    #[test]
    fn a_block_has_the_lines_in_its_loop() {
        let dir = std::env::temp_dir().join(format!("probe-zero-inline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 3\n>g() → 1\n>h() → 5\n").unwrap();
        let code = "char t$\nint d$ = runs(t$)\nint x$\nint e$ = twice(x$)\n\non (int d$) << runs (char c$)\n    int n$ = n$[-1] + 1 if (c$ == c$[-1]) else 1\n    d$ << n$\n\non (int e$) << twice (int x$)\n    e$ << x$ * 2\n\non (int n) << f()\n    int d[] = [7, 8, 9]\n    t$ << \"aab\" << \"bc\"\n    n << [count] (d[])\n\non (int n) << g()\n    x$ << 4\n    n << [count] (frame e$)\n\non (int n) << h()\n    int k = f()\n    n << [count] (frame d$)\n";
        std::fs::write(dir.join("h/h.zero"), code).unwrap();
        let ir = emit(&dir).unwrap();
        // every push into `t$` is a block: no function of one item for it
        assert!(!ir.contains("__z1_each"), "{}", ir);
        // ... and `x$` is pushed one item: the call, and the function
        assert!(ir.contains("fn __z2_each(_x: int)\n") && ir.contains("    __z2_each("), "{}", ir);
        let f = &ir[ir.find("fn f() -> int").unwrap()..];
        let f = &f[..f[1..].find("\nfn ").map_or(f.len(), |i| i + 1)];
        // two blocks, two loops, the lines in each, the second's names
        // defined again; what is kept goes round each loop and from the
        // first to the second with nothing stored between
        assert_eq!(f.matches(" = loop(").count(), 2, "{}", f);
        assert!(f.contains("        _n: int = if ") && f.contains("        _n_2: int = if "), "{}", f);
        assert_eq!(f.matches("store ").count(), 1, "{}", f);
        // the lines push into the store's `d$`, fetched from the
        // context, and not into the function's own
        assert_eq!(f.matches(": int$ = get ").count(), 2, "{}", f);
        assert!(f.contains("    d: int$ = __queue_int(") && !f.contains("(d, _n"), "{}", f);
        // a pusher below the wiring: the lines are the wiring's feature's
        std::fs::write(dir.join("order.md"), "# order\nlowest first\n\n- platform\n- runtime\n- tools\n").unwrap();
        std::fs::create_dir_all(dir.join("up")).unwrap();
        std::fs::write(dir.join("up/up.md"), "# up\n*x*\n\nlayer: tools\n\n> (suite) 2026-09-08T11:00:00\n\n## testing\n>seen() → 3\n").unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 0\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "char t$\n\non (int n) << f()\n    t$ << \"abc\"\n    n << 0\n").unwrap();
        std::fs::write(dir.join("up/up.zero"), "int d$ = codes(t$)\n\non (int k$) << codes (char c$)\n    k$ << int(c$)\n\non (int n) << seen()\n    f()\n    n << [count] (frame d$)\n").unwrap();
        let ir = emit(&dir).unwrap();
        let f = &ir[ir.find("fn f() -> int").unwrap()..];
        let f = &f[..f[1..].find("\nfn ").map_or(f.len(), |i| i + 1)];
        assert!(f.contains("        loop(") && f.contains(": int$ = get ") && f.contains(", __enabled_up\n") && !ir.contains("__z1_each"), "{}", ir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing in gives nothing out, and `empty` asks (fm3 question 75
    /// rule 4, log 127). The end of the input is one last tick, a
    /// function of its own in which every line that needs the item is
    /// not there: `doubled` has none, so it pushes no stray zero, and a
    /// line that asks `empty` is its one arm in the function of one
    /// item and its other in the last, with no branch in either. `end`
    /// of an input with no storage calls the last function once, under
    /// one bit that also fails a push made after it. The output is
    /// ended after the last tick's pushes where anything could tell
    /// (question 67), and not otherwise
    #[test]
    fn the_end_is_one_last_tick() {
        let dir = std::env::temp_dir().join(format!("probe-zero-end-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 2\n").unwrap();
        let head = "int x$\nint d$ = made(x$)\n\non (int n) << f()\n    x$ << 1 << 2\n    end x$\n    end x$\n    n << peek d$ at (0)\n\n";
        let with = |body: &str, more: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int d$) << made (int x$)\n{}\n{}", head, body, more)).unwrap();
            emit(&dir)
        };
        // nothing to do at the end: no last function, and the `end` calls nothing
        let ir = with("    d$ << x$ * 2", "").unwrap();
        assert!(!ir.contains("__z1_end"), "{}", ir);
        // the one bit: a push checks it, `end` reads and sets it, twice here
        let f = &ir[ir.find("fn f() -> int").unwrap()..];
        assert!(f.contains("    _2: u1 = get _1, __zend_x\n    _3: u1 = xor _2, 1\n    check _3\n"), "{}", f);
        assert_eq!(f.matches(" = set ").count(), 2, "{}", f);
        // a line that asks `empty`: one arm in each function, no branch
        let ir = with("    int k$ = 7 if (empty x$) else x$ + k$[-1]\n    d$ << k$", "").unwrap();
        assert!(ir.contains("fn __z1_each(_x: int, __k_b1: int) -> int\n    _this: ptr = context()\n    _k: int = add _x, __k_b1\n"), "{}", ir);
        assert!(ir.contains("fn __z1_end(__k_b1: int)\n    _this: ptr = context()\n    _k: int = const 7\n    _1: __ctx = load _this\n    _2: int$ = get _1, d\n    push_queue_open(_2, _k)\n    ret\n"), "{}", ir);
        // called under "it had not ended", with what is kept
        assert!(ir.contains("    _15: u1 = get _14, __zend_x\n    if _15\n    else\n        _16: u1 = const 1\n        _17: __ctx = load _this\n        _18: __ctx = set _17, __zend_x, _16\n        store _18, _this\n        if _5\n            _21: __ctx = load _this\n            _22: int = get _21, __z1_k_1\n            __z1_end(_22)\n"), "{}", ir);
        // a push of nothing does not happen: only the push that asks is in the last function
        let ir = with("    d$ << x$\n    d$ << x$[-1] if (empty x$)", "").unwrap();
        let end = &ir[ir.find("fn __z1_end(").unwrap()..];
        let end = &end[..end[1..].find("\nfn ").map_or(end.len(), |i| i + 1)];
        assert_eq!(end.matches("push_queue").count(), 1, "{}", end);
        assert!(!end.contains(" end("), "{}", end);
        // the output is ended where something asks whether it has
        let ir = with("    d$ << x$\n    d$ << x$[-1] if (empty x$)", "\non (bool b) << done()\n    b << ended d$\n").unwrap();
        let end = &ir[ir.find("fn __z1_end(").unwrap()..];
        let end = &end[..end[1..].find("\nfn ").map_or(end.len(), |i| i + 1)];
        assert!(end.contains("    end(_2)\n    ret\n"), "{}", end);
        // `empty` in a body that walks: written both ways
        let err = with("    loop\n        if (empty x$)\n            break\n        d$ << peek x$ at (0)\n        advance x$ by (1)", "").expect_err("both ways");
        assert!(err.contains("h.zero:12: this body is written both ways: line 11 walks its input (a loop), and line 12 holds for every item (`empty x$`)"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `or` and `and` join two conditions (fm3 question 66): `and`
    /// tighter than `or`, both looser than a comparison, both sides
    /// worked out, one operation each. A declared name that has `and`
    /// in it is still the call it was, the parser being told which
    /// words stand before it in a name
    #[test]
    fn two_conditions_are_joined() {
        let dir = std::env::temp_dir().join(format!("probe-zero-joined-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f (5) → 1\n>sum of (1) and (2) → 3\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "on (int s) << sum of (int a) and (int b)\n    s << a + b\n\non (int n) << f (int x)\n    bool ok = x > 0 and x < 9 or x == 100 and sum of (x) and (1) > 3\n    n << (1 if (ok) else 0)\n").unwrap();
        let ir = emit(&dir).unwrap();
        assert!(ir.contains("fn f(x: int) -> int\n    _1: u1 = cmp.gt x, 0\n    _2: u1 = cmp.lt x, 9\n    _3: u1 = and _1, _2\n    _4: u1 = cmp.eq x, 100\n    _5: int = sum_of_and(x, 1)\n    _6: u1 = cmp.gt _5, 3\n    _7: u1 = and _4, _6\n    ok: u1 = or _3, _7\n"), "{}", ir);
        std::fs::write(dir.join("h/h.zero"), "on (int n) << f (int x)\n    n << (1 if (x and x > 2) else 0)\n").unwrap();
        let err = emit(&dir).expect_err("a number joined");
        assert!(err.ends_with("h.zero:2: 'and' joins two conditions, and this side is not one: it is `int`"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `index` and `int` are both the product's, and on every path's
    /// own policy they are the same width, so a conversion between
    /// them left out or made the wrong way would show nowhere in the
    /// suite. `suite/zero/types` has the cases where the two meet (fm3
    /// log 122): they give the same answers with `int` the narrower,
    /// with `index` the narrower, and with both narrow
    #[test]
    fn index_and_int_at_different_widths() {
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap() {
                let e = e.unwrap();
                if e.path().is_dir() {
                    copy(&e.path(), &to.join(e.file_name()));
                } else {
                    std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
                }
            }
        }
        for (int, index) in [(32, 64), (64, 32), (32, 32)] {
            let dir = std::env::temp_dir().join(format!("probe-zero-mixed-{}-{}-{}", std::process::id(), int, index));
            let _ = std::fs::remove_dir_all(&dir);
            copy(Path::new("suite/zero/types"), &dir.join("types"));
            std::fs::write(dir.join("types/product.md"), format!("# product\n\nint: {}\nindex: {}\n", int, index)).unwrap();
            let report = test(&dir, Backend::Native, 1).unwrap();
            assert_eq!(report.failed, 0, "int {} index {}:\n{}", int, index, report.log);
            assert!(report.passed >= 47, "int {} index {}: {} cases", int, index, report.passed);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// a bare literal between two concrete widths is emitted for the
    /// policy to choose (log 47, 52): the call is one text, `3: int` on
    /// the name, and `product.md`'s `int:` and `float:` lines set the
    /// policy the store is built under
    #[test]
    fn a_product_sets_the_widths() {
        let dir = std::env::temp_dir().join(format!("probe-zero-width-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>chosen() → 32\n>fchosen() → 32\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "on (int32 w) << width of (int32 x)\n    w << 32\n\non (int32 w) << width of (int64 x)\n    w << 64\n\non (int32 w) << chosen()\n    w << width of (3)\n\non (int32 w) << fwidth of (float32 x)\n    w << 32\n\non (int32 w) << fwidth of (float64 x)\n    w << 64\n\non (int32 w) << fchosen()\n    w << fwidth of (2.5)\n").unwrap();
        let native = suite::backend_policy(Backend::Native).unwrap();
        // the text says the policy decides, and does not change with it
        let ir = emit(&dir).unwrap();
        assert!(ir.contains("fn width_of(x: i32) -> i32\n") && ir.contains("fn width_of(x: i64) -> i32\n"), "{}", ir);
        assert!(ir.contains("    w: i32 = width_of(3: int)\n") && ir.contains("    w: i32 = fwidth_of(2.5: float)\n"), "{}", ir);
        assert!(!ir.contains("product's int width"), "{}", ir);
        let run = |policy: &ssa::Policy| -> Vec<i64> {
            let module = build(&ir, policy, 1).unwrap();
            let calls: Vec<suite::Call> = ["chosen", "fchosen"].iter().map(|f| suite::Call { func: f.to_string(), args: vec![], nrets: 1, checks: false, text: true, before: vec![], live: false, times: false }).collect();
            suite::run_calls(&module, &ir, Backend::Native, &calls, "zero-width", 1).unwrap().into_iter().map(|g| g.unwrap().values[0]).collect()
        };
        // the native policy: 64-bit int and float
        assert_eq!(run(&native), [64, 64]);
        // `int: 32` and `float: 32` pin the store, and the same text runs the other methods
        std::fs::write(dir.join("product.md"), "# product\n\nint: 32\nfloat: 32\n").unwrap();
        let s = store::read(&dir).unwrap();
        assert_eq!((s.int_width, s.float_width), (Some(32), Some(32)));
        let policy = store_policy(&s, &native);
        assert_eq!((int_bits(&policy), policy.float), (32, (8, 23)));
        assert_eq!(lower::lower(&s).unwrap().ir, ir);
        assert_eq!(run(&policy), [32, 32]);
        // a width the policy cannot take is refused
        std::fs::write(dir.join("product.md"), "# product\n\nfloat: 16\n").unwrap();
        let err = match store::read(&dir) { Err(e) => e.to_string(), Ok(_) => panic!("accepted float: 16") };
        assert!(err.contains("the product's float width is 32 or 64, not '16'"), "{}", err);
        // `index: 16|32|64` is the width of a count and a position in
        // memory (fm3 log 120): the path's, 64 here, unless the product says
        assert_eq!(native.index, ssa::Type::I64);
        assert_eq!(suite::backend_policy(Backend::Wasm).unwrap().index, ssa::Type::I32);
        for (line, want) in [("16", ssa::Type::int(true, 16)), ("32", ssa::Type::I32), ("64", ssa::Type::I64)] {
            std::fs::write(dir.join("product.md"), format!("# product\n\nindex: {}\n", line)).unwrap();
            let s = store::read(&dir).unwrap();
            assert_eq!(s.index_width, line.parse().ok());
            assert_eq!(store_policy(&s, &native).index, want);
        }
        std::fs::write(dir.join("product.md"), "# product\n\nindex: 8\n").unwrap();
        let err = match store::read(&dir) { Err(e) => e.to_string(), Ok(_) => panic!("accepted index: 8") };
        assert!(err.contains("the product's index width is 16, 32 or 64, not '8'"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// formatting by dispatch (log 59): a `<<` method is an operator with
    /// the stream first and no result, named in the IR by both types; a
    /// store's own method for a struct stands beside the library's; the
    /// forms that are not methods are refused naming what they are
    /// a product's marks (log 71): a static-on feature cannot be
    /// switched, a static-off one is not in the program, a mark names a
    /// feature, and the platform feature is never static off
    #[test]
    fn a_product_marks_its_features() {
        let dir = std::env::temp_dir().join(format!("probe-zero-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, parent, code, case) in [("base", "", "on (int v) << value()\n    v << 1\n", ">value() → 1\n"), ("gone", "base", "on (int v) << value()\n    v << existing value() + 1\n", ">value() → 2\n")] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            let parent = if parent.is_empty() { String::new() } else { format!("parent: {}\n", parent) };
            std::fs::write(dir.join(format!("{}/{}.md", name, name)), format!("# {}\n*x*\n\n{}layer: runtime\n\n> (suite) 2026-09-10T10:0{}:00\n\n## testing\n{}", name, parent, if name == "base" { 0 } else { 1 }, case)).unwrap();
            std::fs::write(dir.join(format!("{}/{}.zero", name, name)), code).unwrap();
        }
        let with = |product: &str, case: &str| -> Result<String, String> {
            std::fs::write(dir.join("product.md"), product).unwrap();
            std::fs::write(dir.join("base/base.md"), format!("# base\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-10T10:00:00\n\n## testing\n{}", case)).unwrap();
            let s = store::read(&dir).map_err(|e| e.to_string())?;
            let l = lower::lower(&s).map_err(|e| e.to_string())?;
            let native = suite::backend_policy(Backend::Native).unwrap();
            calls_of(&s, &l, &native)?;
            Ok(l.ir)
        };
        // static on: no field, no gate, no setter; the chain called by name
        let ir = with("# p\n\nbase: static on\n", ">value() → 1\n").unwrap();
        assert!(!ir.contains("__enabled_base") && !ir.contains("fn __on_base") && ir.contains("fn value() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    on: u1 = get _1, __enabled_gone\n    if on\n    else\n        _2: int = value__base()\n        ret _2\n    _3: int = value__base()\n") && !ir.contains("fn value__gone("), "{}", ir);
        let err = with("# p\n\nbase: static on\n", ">value() → 1\n>value() with base off → 1\n").expect_err("switched a static feature");
        assert!(err.contains("`with base off`: base is static on in the product and cannot be switched"), "{}", err);
        // static off: not in the program, its subtree with it
        let ir = with("# p\n\ngone: static off\n", ">value() → 1\n").unwrap();
        assert!(!ir.contains("value__gone") && !ir.contains("__enabled_gone") && ir.contains("fn value() -> int\n"), "{}", ir);
        let err = with("# p\n\ngone: static off\n", ">value() → 1\n>value() with gone off → 1\n").expect_err("named a static-off feature");
        assert!(err.contains("`with gone off`: gone is static off in the product, so its code and its cases are not in the program"), "{}", err);
        // a mark names a feature; the platform feature runs the program
        let err = with("# p\n\nnowhere: static on\n", ">value() → 1\n").expect_err("marked no feature");
        assert!(err.contains("the product marks 'nowhere', which is no feature of the store"), "{}", err);
        let err = with("# p\n\nplatform: static off\n", ">value() → 1\n").expect_err("switched the platform off");
        assert!(err.contains("the platform feature is what a program runs on"), "{}", err);
        let err = with("# p\n\nbase: static on\nbase: dynamic\n", ">value() → 1\n").expect_err("marked twice");
        assert!(err.contains("'base' is marked twice"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a stream that is pushed into and that nothing reads or wires
    /// (question 54, fm3 log 96): with the feature that wires it marked
    /// `static off` it has no storage, a push into it being nothing but
    /// the step of its rate; where no feature of the store reads or
    /// wires it, compiled in or left out, the store is refused, naming
    /// the stream; and a stream that is declared and named nowhere else
    /// has no storage and is not refused (question 57, fm3 log 100)
    #[test]
    fn a_stream_nothing_reads_or_wires() {
        let dir = std::env::temp_dir().join(format!("probe-zero-unwired-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let feature = |name: &str, parent: &str, minute: u32, code: &str| {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            let parent = if parent.is_empty() { String::new() } else { format!("parent: {}\n", parent) };
            std::fs::write(dir.join(format!("{}/{}.md", name, name)), format!("# {}\n*x*\n\n{}layer: runtime\n\n> (suite) 2026-10-06T10:0{}:00\n\n## testing\n", name, parent, minute)).unwrap();
            std::fs::write(dir.join(format!("{}/{}.zero", name, name)), code).unwrap();
        };
        let lowered = |product: &str| -> Result<String, String> {
            std::fs::write(dir.join("product.md"), product).unwrap();
            let s = store::read(&dir).map_err(|e| e.to_string())?;
            Ok(lower::lower(&s).map_err(|e| e.to_string())?.ir)
        };
        feature("base", "", 0, "int n$\nint beat$ at (2 hz)\n\non count()\n    n$ << [3 through 1]\n    beat$ << 1\n");
        feature("shown", "base", 1, "out$ << (n$ << \"\\n\") forever\nout$ << (beat$ << \"\\n\") forever\n");
        // wired by a feature that is in the program: an edge each
        let ir = lowered("# p\n").unwrap();
        // (the first has its lines in the loop over the range that is
        // pushed, and no function, fm3 log 195; the second is called)
        assert!(!ir.contains("fn __edge1(") && ir.contains("fn __edge2(__item: int)"), "{}", ir);
        // wired by a feature the product leaves out: no edge, no queue,
        // and the rated stream's step still passes
        let ir = lowered("# p\n\nshown: static off\n").unwrap();
        assert!(!ir.contains("__edge") && !ir.contains("__queue_int") && !ir.contains("\n    n: int$\n"), "{}", ir);
        // ... with no alignment before it, `count` being called where
        // the clock is at 0 s and `n$` having nothing that moves it
        // (question 56, fm3 log 99)
        assert!(ir.contains("    __step(500000)\n    ret\n") && !ir.contains(" = rem "), "{}", ir);
        // wired by no feature at all: refused, naming the stream
        std::fs::write(dir.join("shown/shown.zero"), "out$ << (beat$ << \"\\n\") forever\n").unwrap();
        for product in ["# p\n", "# p\n\nshown: static off\n"] {
            let err = lowered(product).expect_err("a stream nothing reads or wires");
            assert!(err.contains("base.zero:1: 'n$' is pushed into and nothing reads it or wires it, in any feature of the store, compiled in or left out: a mistyped name?"), "{}", err);
        }
        // a case that names the stream reads it, and it is a queue again
        std::fs::write(dir.join("base/base.md"), "# base\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n>count n$ → 0\n").unwrap();
        let ir = lowered("# p\n").unwrap();
        assert!(ir.contains("fn __queue_int(") && ir.contains("\n    n: int$\n"), "{}", ir);
        // declared and named nowhere else, not even pushed into
        // (question 57, fm3 log 100): no storage, a `char` stream
        // included, and no refusal under either product
        feature("base", "", 0, "int spare$\nchar note$\nint n$\n\non count()\n    n$ << 1\n");
        feature("shown", "base", 1, "out$ << (n$ << \"\\n\") forever\n");
        for product in ["# p\n", "# p\n\nshown: static off\n"] {
            let ir = lowered(product).unwrap();
            assert!(ir.contains(";   spare: no storage, nothing in the program reading it or wiring it (base)\n") && ir.contains(";   note: no storage, nothing in the program reading it or wiring it (base)\n"), "{}", ir);
            assert!(!ir.contains("spare: int$") && !ir.contains("note: u8$") && !ir.contains("__get_spare") && !ir.contains("__get_note") && !ir.contains("__queue_int"), "{}", ir);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a push lands on its stream's beat (question 52 as refined, fm3
    /// log 98): before a push statement's first item the clock is
    /// rounded up to the stream's next slot, once a statement, and not
    /// at all where the statement before it in the same block pushed
    /// into the same stream; a period is `step`'s, so a slot is a whole
    /// number of them
    #[test]
    fn a_push_lands_on_the_beat() {
        let dir = std::env::temp_dir().join(format!("probe-zero-beat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let lowered = |code: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            lower::lower(&store::read(&dir).unwrap()).unwrap().ir
        };
        let body = |ir: &str, f: &str| -> String { ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        // `f` is called by `g` after a push into a stream at `7 hz`, so
        // nothing is known of the clock where `f` begins (fm3 log 99)
        let head = "int a$ at (3 hz)\nint b$ at (5 hz)\nint c$ at (7 hz)\nout$ << (a$ << \"\\n\") forever\nout$ << (b$ << \"\\n\") forever\nout$ << (c$ << \"\\n\") forever\n\non g()\n    c$ << 0\n    f()\n\n";
        // one statement of three items: one alignment, three steps, the
        // slot a whole number of the period a step adds
        let ir = lowered(&format!("{}on f()\n    a$ << 1 << 2 << 3\n", head));
        let f = body(&ir, "f");
        assert!(f.contains("    _3: i64 = add _2, 333332\n    _4: i64 = rem _3, 333333\n    _5: i64 = sub _3, _4\n    __wait(_5)\n"), "{}", f);
        assert_eq!((f.matches(" = rem ").count(), f.matches("    __step(333333)\n").count()), (1, 3), "{}", f);
        // the clock's address is formed once (fm3 log 193): a second
        // alignment reached only through the first uses the first's,
        // and one inside a loop is formed before the loop
        let ir = lowered(&format!("{}on f()\n    a$ << 1\n    b$ << 2\n    a$ << 3\n", head));
        let f = body(&ir, "f");
        // (the clock is the context's from fm3 log 215: no address is
        // formed, and each alignment reads the field anew, the wait
        // before it having moved it)
        assert_eq!((f.matches(" = rem ").count(), f.matches("addr __clock").count(), f.matches(", __clock\n").count()), (3, 0, 3), "{}", f);
        let ir = lowered(&format!("{}on f()\n    loop (int i = 1) while (i <= 3)\n        a$ << i\n        b$ << i\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches("addr __clock").count() == 0 && f.matches(", __clock\n").count() == 2 && f.contains("        _2: __ctx = load _this\n        _3: i64 = get _2, __clock\n"), "{}", f);
        // the statement before pushed into the same stream: on the beat already
        let f = body(&lowered(&format!("{}on f()\n    a$ << 1\n    a$ << 2\n    a$ << 3\n", head)), "f");
        assert_eq!(f.matches(" = rem ").count(), 1, "{}", f);
        // by turns into two streams: each statement finds its own stream's slot
        let f = body(&lowered(&format!("{}on f()\n    a$ << 1\n    b$ << 2\n    a$ << 3\n", head)), "f");
        assert_eq!((f.matches(" = rem ").count(), f.matches("rem _3, 333333").count() + f.matches("rem _3, __w").count(), f.matches(", 200000\n").count() + f.matches(": i64 = const 200000\n").count() + f.matches("    __step(200000)\n").count()), (3, 1, 2), "{}", f);
        // a write to the device between two pushes moves no clock
        // (question 56, fm3 log 99), so the second is still on the beat;
        // a push into another stream between them does
        let f = body(&lowered(&format!("{}on f()\n    a$ << 1\n    out$ << \"x\"\n    a$ << 2\n", head)), "f");
        assert_eq!(f.matches(" = rem ").count(), 1, "{}", f);
        // a stream with no rate has no beat
        let f = body(&lowered("int c$\nout$ << (c$ << \"\\n\") forever\n\non f()\n    c$ << 1\n"), "f");
        assert!(!f.contains(" = rem ") && !f.contains("__wait"), "{}", f);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// the clock of code (question 56, fm3 log 99): the front end knows
    /// what the clock is a whole multiple of at each point in a plain
    /// function, and a push whose stream's period divides that is not
    /// aligned; a function's entry is the weakest of its callers'; a
    /// loop whose first statement is a rated push aligns once, before
    /// it, under its own `while`; and the pass is off where a method of
    /// the store can move the clock
    #[test]
    fn the_clock_of_code() {
        let dir = std::env::temp_dir().join(format!("probe-zero-code-clock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let lowered = |code: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            lower::lower(&store::read(&dir).unwrap()).unwrap().ir
        };
        let body = |ir: &str, f: &str| -> String { ir.split(&format!("\nfn {}(", f)).nth(1).unwrap().split("\nfn ").next().unwrap().to_string() };
        let rems = |ir: &str, f: &str| body(ir, f).matches(" = rem ").count();
        let head = "int a$ at (2 hz)\nint b$ at (1 hz)\nint c$ at (5 hz)\nout$ << (a$ << \"\\n\") forever\nout$ << (b$ << \"\\n\") forever\nout$ << (c$ << \"\\n\") forever\n\n";
        // called only where a case starts, at 0 s: nothing is aligned,
        // and a push at `1 hz` leaves a multiple of the `2 hz` period
        let ir = lowered(&format!("{}on f()\n    b$ << 1\n    out$ << \"x\"\n    a$ << 2\n", head));
        assert_eq!(rems(&ir, "f"), 0, "{}", ir);
        // ... but a push at `2 hz` does not leave a multiple of a second
        let ir = lowered(&format!("{}on f()\n    a$ << 1\n    b$ << 2\n", head));
        assert_eq!(rems(&ir, "f"), 1, "{}", ir);
        // a function's entry is the weakest of its callers': `f` called
        // by a case alone is on the beat; called also after a push at
        // `5 hz` it is not, and aligns; called after its own stream's
        // push it is on the beat again
        let ir = lowered(&format!("{}on f()\n    a$ << 1\n\non g()\n    c$ << 0\n    f()\n", head));
        assert_eq!((rems(&ir, "f"), rems(&ir, "g")), (1, 0), "{}", ir);
        let ir = lowered(&format!("{}on f()\n    a$ << 1\n\non g()\n    a$ << 0\n    f()\n    b$ << 3\n", head));
        assert_eq!((rems(&ir, "f"), rems(&ir, "g")), (0, 1), "{}", ir);
        // where two paths meet, the weaker
        let ir = lowered(&format!("{}on f (int k)\n    if (k > 0)\n        c$ << 0\n    a$ << 1\n", head));
        assert_eq!(rems(&ir, "f"), 1, "{}", ir);
        // a loop whose first statement is a rated push, entered off the
        // beat: one alignment, before the loop, under the loop's own
        // `while` asked of its first values; entered on the beat, none
        let ir = lowered(&format!("{}on f (int k)\n    c$ << 0\n    loop (int i = 1) while (i <= k)\n        a$ << i\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.contains("    _4: int = const 1\n    _5: u1 = cmp.le _4, k\n    if _5\n        _6: __ctx = load _this\n        _7: i64 = get _6, __clock\n        _8: i64 = add _7, 499999\n        _9: i64 = rem _8, 500000\n        _10: i64 = sub _8, _9\n        __wait(_10)\n    loop(i: int = 1)\n"), "{}", f);
        assert_eq!(f.matches(" = rem ").count(), 1, "{}", f);
        let ir = lowered(&format!("{}on f (int k)\n    loop (int i = 1) while (i <= k)\n        a$ << i\n        continue (i + 1)\n", head));
        assert_eq!(rems(&ir, "f"), 0, "{}", ir);
        // ... with no `while` the first pass always runs, and there is no test
        let ir = lowered(&format!("{}on f (int k)\n    c$ << 0\n    loop (int i = 1)\n        a$ << i\n        if (i == k)\n            break\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches(" = rem ").count() == 1 && f.contains("\n    _7: i64 = rem _6, 500000\n    _8: i64 = sub _6, _7\n    __wait(_8)\n    loop(i: int = 1)\n"), "{}", f);
        // a statement before the push in the body, or a way round that
        // leaves the beat, and the push aligns on every pass as it did
        let ir = lowered(&format!("{}on f (int k)\n    c$ << 0\n    loop (int i = 1) while (i <= k)\n        out$ << \"x\"\n        a$ << i\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches(" = rem ").count() == 1 && f.find("loop(").unwrap() < f.find(" = rem ").unwrap(), "{}", f);
        let ir = lowered(&format!("{}on f (int k)\n    loop (int i = 1) while (i <= k)\n        a$ << i\n        c$ << i\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches(" = rem ").count() == 2 && f.find("loop(").unwrap() < f.find(" = rem ").unwrap(), "{}", f);
        // an edge that can move the clock: the statement leaves nothing known
        let ir = lowered("int a$ at (2 hz)\nint b$ at (5 hz)\nb$ << a$ forever\nout$ << (b$ << \"\\n\") forever\n\non f()\n    a$ << 1\n    a$ << 2\n    out$ << \"x\"\n    a$ << 3\n");
        assert_eq!(rems(&ir, "f"), 1, "{}", ir);
        // a function that can reach itself leaves nothing known
        let ir = lowered(&format!("{}on f (int k)\n    a$ << k\n    if (k > 0)\n        f (k - 1)\n\non g()\n    f (2)\n    a$ << 9\n", head));
        assert_eq!(rems(&ir, "g"), 1, "{}", ir);
        // a `<<` method of the store's that can move the clock, and the
        // pass is off: every push aligns as it did
        let ir = lowered(&format!("{}type pair =\n    int x, y\n\non (char o$) << (pair p)\n    a$ << p.x\n\non f()\n    a$ << 1\n    out$ << \"x\"\n    a$ << 2\n", head));
        assert_eq!(rems(&ir, "f"), 2, "{}", ir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// the product's clock (log 77): real, and the store waits on the
    /// machine's counter through a `platform arm64` body; virtual, and
    /// the clock jumps; anything else refused
    #[test]
    fn a_product_chooses_the_clock() {
        let dir = std::env::temp_dir().join(format!("probe-zero-clock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-10T10:00:00\n\n## testing\n>run() → \"1\\n2\"\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "int i$ at (2 hz)\nout$ << (i$ << \"\\n\") forever\n\non run()\n    i$ << [1 through 2]\n").unwrap();
        let with = |product: &str| -> Result<String, String> {
            std::fs::write(dir.join("product.md"), product).unwrap();
            let s = store::read(&dir).map_err(|e| e.to_string())?;
            Ok(lower::lower(&s).map_err(|e| e.to_string())?.ir)
        };
        let real = with("# p\n\nclock: real\n").unwrap();
        assert!(real.contains("fn __counter() -> i64\n") && real.contains("platform arm64\n    __counter() -> i64\n        mrs r, cntpct_el0\n"), "{}", real);
        assert!(real.contains("fn __wait(t: i64)\n    _this: ptr = context()\n    loop()\n        r: i64 = __real_now()\n"), "{}", real);
        assert!(real.contains("c0: i64 = __counter()\n    q: ptr = addr __base\n    store c0, q\n"), "{}", real);
        let fast = with("# p\n\nclock: virtual\n").unwrap();
        assert!(!fast.contains("__counter") && fast.contains("fn __step(d: i64)\n    _this: ptr = context()\n    x: __ctx = load _this\n    c: i64 = get x, __clock\n    m: i64 = add c, d\n") && !fast.contains("fn __wait(") && real.contains("fn __step(d: i64)\n    _this: ptr = context()\n    x: __ctx = load _this\n    c: i64 = get x, __clock\n    t: i64 = add c, d\n    __wait(t)\n"), "{}", fast);
        // the rated stream no word reads has no storage (fm3 log 92): the
        // push calls its edge, and then a step passes, half a second at 2 hz
        // ... one word of the platform's (fm3 log 194)
        // and the edge's own lines stand in the loop over the range
        // (fm3 log 195)
        assert!(fast.contains("        if _2\n            __out__int(_3)\n            _5: u8 = const 10\n            __out_ch(_5)\n        __step(500000)\n") && !fast.contains("    __wait("), "{}", fast);
        // ... and the statement's first item is on the stream's beat
        // with nothing rounded (fm3 log 98, 99): `run` is only ever
        // called by a case, at 0 s
        assert!(fast.contains("fn run()\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u1 = get _1, __enabled_h\n") && !fast.contains(", 499999\n"), "{}", fast);
        assert!(!fast.contains("fn __edge1(") && !fast.contains("__run") && !fast.contains("__node") && !fast.contains("\n    i: int$\n"), "{}", fast);
        assert_eq!(with("# p\n").unwrap(), fast);
        let err = with("# p\n\nclock: sidereal\n").expect_err("accepted a sidereal clock");
        assert!(err.contains("the product's clock is real or virtual, not 'sidereal'"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_push_is_dispatched_on_the_item() {
        let dir = std::env::temp_dir().join(format!("probe-zero-push-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → \"x\"\n").unwrap();
        let emit_with = |code: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            emit(&dir)
        };
        let ir = emit_with("type pair =\n    int a, b\n\non (char o$) << (pair p)\n    o$ << \"(\" << p.a << \")\"\n\non f()\n    out$ << 42 << pair(1, 2) << 2.5 << true\n    int64 k = 3\n    out$ << k\n").unwrap();
        // `out$` is the output device (question 45, log 87), so every one
        // of these calls reaches the method's device copy, `__out__<type>`;
        // the store's own method keeps its copy over a stream too, every
        // function of a store's own feature being a root of the prune
        assert!(ir.contains("fn __out__pair(p: pair)\n") && ir.contains("fn push__chars_pair(o: u8$, p: pair)\n"), "{}", ir);
        for call in ["__out__int(", "__out__pair(", "__out__float(", "__out__u1("] {
            assert!(ir.contains(call), "{} not called: {}", call, ir);
        }
        // the library's methods are templates over the abstract types; the
        // ones nothing reaches are not in the text (log 70)
        assert!(ir.contains("fn __out__int(x: int)\n") && !ir.contains("fn __out__ints("), "{}", ir);
        // a push into the device is the platform's write, not a ring
        // push: `out$` has no ring, so it has no field in the context
        assert!(ir.contains("__out_ch(") && !ir.contains("\n    out: u8$\n"), "{}", ir);
        let refused = |code: &str| emit_with(code).expect_err("accepted");
        assert!(refused("on (int o$) << (int x)\n    o$ << 1\n\non f()\n    out$ << 1\n").contains("'int' pushed into 'int$' is the push itself, not a method"));
        assert!(refused("on (char o$) << (char c$)\n    o$ << \"?\"\n\non f()\n    out$ << 1\n").contains("'string' pushed into 'string' is the block push of section 9, not a method"));
        assert!(refused("on (char o$) = (char o$) << (int x)\n    o$ << \"?\"\n\non f()\n    out$ << 1\n").contains("`=` says what a name is"));
        assert!(refused("on f()\n    int i$ << 1\n    i$ << 2.5\n").contains("'i$' holds int but the item is float"));
        // a stream's name in a push that happens once is its latest
        // item (fm3 question 79, log 163): a token, written as its fields
        let latest = emit_with("type token =\n    int kind, start, n\n\non f()\n    token t$ << token(1, 2, 3)\n    out$ << t$\n").unwrap();
        // (a stream the function reads for nothing else is the value
        // it keeps, fm3 log 186: made a token here, where it is read whole)
        assert!(latest.contains("    _1: token = pack 1, 2, 3\n    _2: int = get _1, kind\n    __out__int(_2)\n") && !latest.contains("latest"), "{}", latest);
        assert!(refused("type token =\n    int kind, start, n\n\non f()\n    token t$ << token(1, 2, 3)\n    token g[] = frame t$\n    out$ << g[]\n").contains("'out$' holds char but the item is token$: no `<<` method takes it"));
        // a char is a character, not a small number (question 44)
        assert!(refused("on f()\n    char c = char(65)\n    out$ << (c + 1)\n").contains("'+' on a char: a char is compared, not computed with; convert it, `int(c)`"));
        // a `uint8` stream takes the byte itself, since no method takes one
        let bytes = emit_with("on (int n) << f()\n    uint8 b$ << \"hi\"\n    b$ << 33\n    n << count b$\n").unwrap();
        assert!(bytes.contains("_5: u8 = const 33") || bytes.contains("const 33"), "{}", bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a program never writes its input (question 35, fm3 log 101): a
    /// push into `in$`, an edge into it and `end in$` are refused, naming
    /// the device and saying where input comes from; reading it is what
    /// it is for, and a local of the same name is the function's own
    #[test]
    fn a_program_never_writes_its_input() {
        let dir = std::env::temp_dir().join(format!("probe-zero-input-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let emit_with = |code: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            emit(&dir)
        };
        let refusal = "'in$' is the input device: a program reads it and never writes it or ends it. Input comes from the platform alone, which under the runner is a case's `with in \"text\"`; a program that makes its own arrivals pushes them into a stream of its own";
        for (code, line) in [
            ("on f()\n    in$ << \"2;\"\n", 2),
            ("on f()\n    in$ << 65\n", 2),
            ("on f()\n    end in$\n", 2),
            ("char src$\nin$ << src$\n\non f()\n    src$ << \"a\"\n", 2),
        ] {
            let err = emit_with(code).expect_err("a program wrote its input");
            assert!(err.contains(&format!("h.zero:{}: {}", line, refusal)), "{}", err);
        }
        // ... and so is handing it to a function that pushes into the
        // stream it is given, or ends it, itself or through another, in
        // a body or as a wiring, a rate after it or not (fm3 log 106)
        let fill = "on fill (char c$)\n    c$ << \"x\"\n\n";
        for (code, line, to) in [
            (format!("{}on f()\n    fill(in$)\n", fill), 5, "fill"),
            (format!("{}on relay (char d$)\n    fill(d$)\n\non f()\n    relay(in$)\n", fill), 8, "relay"),
            ("on close (char c$)\n    end c$\n\non f()\n    close(in$)\n".to_string(), 5, "close"),
            ("on (int n$) << feeder (char c$)\n    c$ << \"x\"\n    n$ << 1\n\nint n$ = feeder(in$)\n".to_string(), 5, "feeder"),
            ("on (int n$) << feeder (char c$)\n    c$ << \"x\"\n    n$ << 1\n\nint n$ = feeder(in$) at (2 hz)\n".to_string(), 5, "feeder"),
        ] {
            let err = emit_with(&code).expect_err("a program handed its input to what writes it");
            assert!(err.contains(&format!("h.zero:{}: {}. Here it is given to '{}', which pushes into or ends the stream it is given", line, refusal, to)), "{}", err);
        }
        // a function that only reads what it is given takes the device,
        // and one that pushes may be given a local called `in`
        let ir = emit_with(&format!("{}on (int n) << size (char c$)\n    n << count c$\n\non (int n) << f()\n    n << size(in$)\n\non (int n) << g()\n    char in$ << \"ab\"\n    fill(in$)\n    n << count in$\n", fill)).unwrap();
        assert!(ir.contains("fn f() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u8$ = get _1, in\n    n: int = size(_2)\n"), "{}", ir);
        // reading the device is what it is for; and a stream of the
        // function's own that happens to be called `in` is not the device
        let ir = emit_with("on (int n) << f()\n    n << count in$\n\non (int n) << g()\n    char in$ << \"ab\"\n    in$ << \"c\"\n    n << count in$\n").unwrap();
        assert!(ir.contains("fn f() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u8$ = get _1, in\n"), "{}", ir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a node that only plain functions wake is called where they push
    /// (fm3 log 103): no `__node`, no `seen`, no `fin`, and no guard
    /// where no trigger can be met while a node runs; and each thing
    /// that could let an item arrive with no trigger after it, or a
    /// trigger be met while a node runs, leaves the node as it was
    #[test]
    fn a_node_its_pushers_wake() {
        let dir = std::env::temp_dir().join(format!("probe-zero-woken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let task = "on (int d$) << doubled (int x$)\n    loop\n        if (count x$ == 0)\n            break\n        d$ << peek x$ at (0) * 2\n        advance x$ by (1)\n\n";
        let fed = "on (int n) << fed()\n    a$ << 1\n    n << count d$\n";
        let emit_with = |decls: &str, more: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("{}\n{}{}{}", decls, task, fed, more)).unwrap();
            let s = store::read(&dir).unwrap();
            lower::lower(&s).unwrap().ir
        };
        let wired = "int a$\nint d$ = doubled(a$)\n";
        // woken: the task is called in `fed`, after the push, and the
        // node keeps its reader and nothing else
        let ir = emit_with(wired, "");
        assert!(ir.contains("fn fed() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int$ = get _1, a\n    _3: int = const 1\n    push_queue_open(_2, _3)\n    _4: __ctx = load _this\n    _5: index = get _4, __node1_x\n    _6: int$ = set _2, pos, _5\n    _7: __ctx = load _this\n    _8: u1 = get _7, __enabled_h\n    if _8\n        _9: __ctx = load _this\n        _10: int$ = get _9, d\n        _11: int$ = doubled(_10, _6, 0: i64)\n        _12: index = get _11, pos\n        _13: __ctx = load _this\n        _14: __ctx = set _13, __node1_x, _12\n        store _14, _this\n        free_queue(_11)\n    else\n        _15: index = received(_6)\n        _16: __ctx = load _this\n        _17: __ctx = set _16, __node1_x, _15\n        store _17, _this\n        _18: int$ = set _6, pos, _15\n        free_queue(_18)\n"), "{}", ir);
        // ... everything read and written in place at the context's one
        // address, with no accessor (fm3 log 104, 110)
        assert!(!ir.contains("__get_") && !ir.contains("__set___node1_x") && !ir.contains("__on_h"), "{}", ir);
        // ... and the node keeps its position, a word, the rest of its
        // reader being the stream's own value, which the push has in
        // hand (fm3 log 111): `doubled` only reads and advances `x$`
        assert!(ir.contains("\n    __node1_x: index\n") && ir.contains("    _4: index = get _2, pos\n    _5: __ctx = pack 0, 0, op, 1, 1, _1, _2, _3, _4"), "{}", ir);
        // a task that may give back a reader on another ring keeps its
        // whole reader: one that assigns its parameter, declares the
        // name again, hands it to a function or runs a
        // task over it, or reads it by a word the rule does not know
        for (body, whole) in [
            ("    loop\n        if (count x$ == 0)\n            break\n        d$ << peek x$ at (0)\n        advance x$ by (1)\n", false),
            ("    d$ << count x$ << position x$\n    int f[] = frame x$\n    if (ended x$)\n        d$ << f[0]\n", false),
            ("    d$ << size(x$)\n    advance x$ by (count x$)\n", true),
            ("    d$ << doubled(x$)\n", true),
            ("    int h[] = x$ behind (1)\n    advance x$ by (count x$)\n", true),
        ] {
            std::fs::write(dir.join("h/h.zero"), format!("int a$\nint d$ = moved(a$)\nint far$\n\non (int n) << size (int s$)\n    n << count s$\n\n{}on (int d$) << moved (int x$)\n{}\n{}", task, body, fed)).unwrap();
            let ir = lower::lower(&store::read(&dir).unwrap()).unwrap().ir;
            assert!(!ir.contains("fn __node1("), "not woken: {}\n{}", body, ir);
            assert_eq!(ir.contains("\n    __node1_x: int$\n"), whole, "{}\n{}", body, ir);
            assert_eq!(ir.contains("\n    __node1_x: index\n"), !whole, "{}\n{}", body, ir);
        }
        for gone in ["fn __node1(", "__running", "__zero_start", "__run", "_seen", "_fin"] {
            assert!(!ir.contains(gone), "{}: {}", gone, ir);
        }
        // a statement that may push nothing wakes under whether anything
        // arrived, and an `end` under whether the stream had ended
        let ir = emit_with(wired, "\non some (int k)\n    a$ << [k to 1]\n\non close()\n    end a$\n");
        assert!(ir.contains("    _2: int$ = get _1, a\n    _14: index = received(_2)\n") && ir.contains("    _15: index = received(_2)\n    _16: u1 = cmp.gt _15, _14\n    if _16\n        _17: __ctx = load _this\n        _18: index = get _17, __node1_x\n        _19: int$ = set _2, pos, _18\n"), "{}", ir);
        assert!(ir.contains("    _3: u1 = ended(_2)\n    end(_2)\n    if _3\n    else\n        _4: __ctx = load _this\n        _5: index = get _4, __node1_x\n        _6: int$ = set _2, pos, _5\n"), "{}", ir);
        let node = |ir: &str| ir.contains("fn __node1() -> u1\n") && ir.contains("__node1_x_seen") && ir.contains("fn __zero_start()");
        // handed to a function, which may push into its parameter with
        // no trigger after
        let ir = emit_with(wired, "\non fill (int s$)\n    s$ << 9\n\non filled()\n    fill(a$)\n");
        assert!(node(&ir) && !ir.contains("__running"), "{}", ir);
        // pushed into by a task: a node, and no trigger in a task's body
        let ir = emit_with("int a$\nint d$ = doubled(a$)\nint y$\nint e$ = feeder(y$)\n", "\non (int e$) << feeder (int y$)\n    a$ << 5\n    advance y$ by (count y$)\n");
        assert!(node(&ir) && !ir.contains("__running"), "{}", ir);
        // pushed into by a plain function a task can reach: a node, and
        // the trigger in it may be met while a node runs, so the guard
        let ir = emit_with("int a$\nint d$ = doubled(a$)\nint y$\nint e$ = feeder(y$)\n", "\non (int k) << bump (int v)\n    a$ << v\n    k << v\n\non (int e$) << feeder (int y$)\n    e$ << bump (1)\n    advance y$ by (count y$)\n");
        assert!(node(&ir) && ir.contains("data __running") && ir.contains("fn __run_a()\n    p: ptr = addr __running\n"), "{}", ir);
        // items on its declaration: it holds something at the start
        let ir = emit_with("int a$ << 1 << 2\nint d$ = doubled(a$)\n", "");
        assert!(node(&ir), "{}", ir);
        // wired at a rate
        let ir = emit_with("int a$\nint d$ = doubled(a$) at (2 hz)\n", "");
        assert!(node(&ir), "{}", ir);
        // a `platform` body of the store's own is not read
        let ir = emit_with(wired, "\non (int64 r) << (int64 a) twice\nplatform ir\n    r: i64 = add a, a\n    ret r\n\non (int64 n) << two()\n    n << (1) twice\n");
        assert!(node(&ir) && ir.contains("data __running"), "{}", ir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A queue's push asks whether its stream has ended only where an
    /// `end` in the store could have reached a stream of its type (fm3
    /// log 108): `ended` is written by `end` alone, and a ring is named
    /// only by its own element type or an abstract one above it. Every
    /// push is the open one where nothing is ended; one `end` keeps the
    /// check on the pushes of its type, of every type that fits it and
    /// of every type it fits, and on a `char` for a `uint8`, the two
    /// being one type in the IR; and a store with a `platform` body of
    /// its own, which is IR the front end does not read, keeps them all
    #[test]
    fn a_push_asks_whether_its_stream_ended_only_where_it_could_have() {
        let dir = std::env::temp_dir().join(format!("probe-zero-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let fns = "int a$\nint64 w$\nuint16 u$\nchar c$\nuint8 b$\n\non (int n) << fa()\n    a$ << 1\n    n << [count] (frame a$)\n\non (int n) << fw()\n    w$ << 1\n    n << [count] (frame w$)\n\non (int n) << fu()\n    u$ << 1\n    n << [count] (frame u$)\n\non (int n) << fc()\n    c$ << \"sixteen letters!\"\n    n << [count] (frame c$)\n\non (int n) << fb()\n    b$ << 1\n    n << [count] (frame b$)\n";
        // each function's push, by the word it took: `fa` to `fb` in order;
        // `fc` pushes a block, sixteen bytes, a shorter literal being
        // `push_queue_few`'s whatever the type (fm3 log 109)
        let words = |more: &str| -> Vec<bool> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}", fns, more)).unwrap();
            let s = store::read(&dir).unwrap();
            let ir = lower::lower(&s).unwrap().ir;
            assert!(!ir.contains("push_queue<"), "{}", ir);
            ["fa", "fw", "fu", "fc", "fb"].iter().map(|f| {
                let body: String = ir.lines().skip_while(|l| !l.starts_with(&format!("fn {}(", f))).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n");
                assert!(body.contains("push_queue(") != body.contains("push_queue_open("), "{}: {}", f, body);
                body.contains("push_queue(")
            }).collect()
        };
        // nothing ends anything: no push asks
        assert_eq!(words(""), [false, false, false, false, false]);
        // an `int64` stream ended: its own pushes ask, and those of `int`, which it fits
        assert_eq!(words("\non close()\n    end w$\n"), [true, true, false, false, false]);
        // ended through a parameter over `int`: `int` and `int64`, which fits it
        assert_eq!(words("\non shut (int x$)\n    end x$\n"), [true, true, false, false, false]);
        // ... and over `number`: every stream of numbers, and not the `char`
        assert_eq!(words("\non shut (number x$)\n    end x$\n"), [true, true, true, false, true]);
        // a `char` and a `uint8` are one type in the IR
        assert_eq!(words("\non close()\n    end c$\n"), [false, false, false, true, true]);
        assert_eq!(words("\non close()\n    end b$\n"), [false, false, false, true, true]);
        // a local stream ended counts by its type as any other does
        assert_eq!(words("\non (int n) << made()\n    uint16 l$ << 1\n    end l$\n    n << [count] (frame l$)\n"), [false, false, true, false, false]);
        // a `platform` body of the store's own may end anything
        assert_eq!(words("\non (int64 r) << (int64 a) twice\nplatform ir\n    r: i64 = add a, a\n    ret r\n\non (int64 n) << two()\n    n << (1) twice\n"), [true, true, true, true, true]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A string literal of fewer than sixteen bytes lands in a queue an
    /// item at a time from its `data`, with no view and no `copy` (fm3
    /// log 109); one of sixteen or more is the block it was, a chunk at
    /// a time, since a long block must stay a chunked copy on the
    /// machine. One byte is an item's push, as it was, and a literal
    /// into the device, into a ring or into a rated stream is untouched
    #[test]
    fn a_short_literal_lands_an_item_at_a_time() {
        let dir = std::env::temp_dir().join(format!("probe-zero-few-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let body = |decls: &str, text: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("{}\n\non (int n) << f()\n    c$ << \"{}\"\n    n << [count] (frame c$)\n", decls, text)).unwrap();
            let s = store::read(&dir).unwrap();
            let ir = lower::lower(&s).unwrap().ir;
            ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n")
        };
        let few = "    _3: ptr = addr __s11\n    _4: index = len __s11\n    push_queue_few(_2, _3, _4)\n";
        // two bytes and fifteen: the few-items word, and no view
        for text in ["ab", "fifteen letters"] {
            let b = body("char c$", text);
            assert!(b.contains(few) && !b.contains("__str"), "{}: {}", text, b);
        }
        // sixteen and 480: the block, through the view
        for text in ["sixteen letters!".to_string(), "x".repeat(480)] {
            let b = body("char c$", &text);
            assert!(b.contains("    _5: u8[] = __str(_3, _4)\n    push_queue_open(_2, _5)\n") && !b.contains("push_queue_few"), "{}: {}", text.len(), b);
        }
        // one byte is an item
        let b = body("char c$", "a");
        assert!(b.contains("    _3: u8 = const 97\n    push_queue_open(_2, _3)\n") && !b.contains("push_queue_few"), "{}", b);
        // a ring, which a history word makes of every stream in the store, keeps its block push
        // (the stream is counted too: read for its latest alone it is a cell, fm3 log 143, and keeps no history)
        let b = body("char c$\nint h$\n\non (int k) << g()\n    h$ << 1\n    k << latest h$ + [count] (frame h$)", "fifteen letters");
        assert!(!b.contains("push_queue") && b.contains("__str"), "{}", b);
        // a stream with a rate takes it an item at a time, each at its time
        let b = body("char c$ at (2 hz)", "ab");
        assert!(!b.contains("push_queue_few"), "{}", b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a field of the context nothing in the store writes is fetched
    /// once a function (fm3 log 110); and one that something writes is
    /// read again only where what writes it can run between the two
    /// reads, the function itself or one it calls, through any depth
    /// (fm3 log 181)
    #[test]
    fn a_field_nothing_writes_is_fetched_once() {
        let dir = std::env::temp_dir().join(format!("probe-zero-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let body = |code: &str, f: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            let s = store::read(&dir).unwrap();
            let ir = lower::lower(&s).unwrap().ir;
            ir.lines().skip_while(|l| !l.starts_with(&format!("fn {}(", f))).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n")
        };
        let reads = |b: &str, field: &str| b.matches(&format!(", {}\n", field)).count() + b.ends_with(&format!(", {}", field)) as usize;
        let head = "int kept = 3\nint moved$ << 0\n\non bump()\n    moved$ << moved$ + 1\n\non idle()\n    int z = 0\n\n";
        // read, a call, read again: the unwritten one once, with the
        // context's address formed once, first; the written one twice
        let b = body(&format!("{}on (int n) << f()\n    int a = kept + moved$\n    bump()\n    n << a + kept + moved$\n", head), "f");
        assert!(b.starts_with("    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, kept\n") && b.matches("= context()").count() == 1, "{}", b);
        assert_eq!((reads(&b, "kept"), reads(&b, "moved")), (1, 2), "{}", b);
        // ... the written one once where nothing between writes it:
        // `idle` does not, and nothing it calls does
        let b = body(&format!("{}on (int n) << f()\n    int a = moved$\n    idle()\n    n << a + moved$\n", head), "f");
        assert_eq!(reads(&b, "moved"), 1, "{}", b);
        // ... twice where the write is two calls down
        let b = body(&format!("{}on deep()\n    idle()\n    bump()\n\non (int n) << f()\n    int a = moved$\n    deep()\n    n << a + moved$\n", head), "f");
        assert_eq!(reads(&b, "moved"), 2, "{}", b);
        // ... and twice in the function that writes it itself
        let b = body(&format!("{}on (int n) << f()\n    int a = moved$\n    moved$ << 7\n    n << a + moved$\n", head), "f");
        assert_eq!(reads(&b, "moved"), 2, "{}", b);
        // a read in one arm is not in hand in the other, nor after them;
        // one above them is in hand in both, and inside a loop
        let b = body(&format!("{}on (int n) << f (int k)\n    n << kept if (k > 0)\n         else kept + 1\n", head), "f");
        assert_eq!(reads(&b, "kept"), 2, "{}", b);
        let b = body(&format!("{}on (int n) << f (int k)\n    int a = 0\n    if (k > 0)\n        a = kept\n    n << a + kept\n", head), "f");
        assert_eq!(reads(&b, "kept"), 2, "{}", b);
        let b = body(&format!("{}on (int n) << f (int k)\n    int a = kept\n    int s = loop (int i = 0, int t = 0) while (i < k) yields t\n        continue (i + 1, t + kept if (i > 2) else t + kept + a)\n    n << s + kept\n", head), "f");
        assert_eq!(reads(&b, "kept"), 1, "{}", b);
        // a feature's switch is a field nothing writes
        let b = body(&format!("{}on (bool b) << f()\n    bool a = enabled\n    bump()\n    b << a == enabled\n", head), "f");
        assert_eq!(reads(&b, "__enabled_h"), 1, "{}", b);
        // a stream's field is not written by a push into the stream, and
        // is by a word that moves the feature's reader
        let streams = "int q$\nint r$\n\non fill()\n    q$ << 1\n    r$ << 1\n\non skip()\n    advance r$ by (1)\n\n";
        let b = body(&format!("{}on (int n) << f()\n    int a = count q$ - position q$ + (count r$ - position r$)\n    fill()\n    skip()\n    n << a + (count q$ - position q$) + (count r$ - position r$)\n", streams), "f");
        assert_eq!((reads(&b, "q"), reads(&b, "r")), (1, 2), "{}", b);
        // a `platform` body of the store's own may call a setter: nothing is reused
        let b = body(&format!("{}on (int64 r) << (int64 a) twice\nplatform ir\n    r: i64 = add a, a\n    ret r\n\non (int n) << f()\n    int a = kept\n    bump()\n    n << a + kept\n", head), "f");
        assert_eq!(reads(&b, "kept"), 2, "{}", b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a queue is freed by who reads it: a task's own parameter is not
    /// a reading of the feature's stream of that name, and a function
    /// that names the feature's stream is (fm3 log 113)
    #[test]
    fn a_parameter_is_not_a_reading_of_the_stream_it_is_spelled_like() {
        let dir = std::env::temp_dir().join(format!("probe-zero-spelled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let frees = |param: &str, more: &str| -> bool {
            let code = format!("int total$ << 0\nint x$\nsoak(x$)\n\non soak (int {p}$)\n    loop\n        if (count {p}$ == 0)\n            break\n        total$ << total$ + peek {p}$ at (0)\n        advance {p}$ by (1)\n\non feed()\n    x$ << 1\n{more}", p = param, more = more);
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            lower::lower(&store::read(&dir).unwrap()).unwrap().ir.contains("free_queue(")
        };
        // however the sink spells its parameter, the queue frees
        assert!(frees("s", "") && frees("x", ""));
        // a function that reads the feature's stream by name is a second
        // reader, and one that hands it on names it too
        assert!(!frees("x", "\non (int n) << first()\n    n << peek x$ at (0)\n"));
        assert!(!frees("x", "\non (int n) << size (int s$)\n    n << peek s$ at (0)\n\non (int n) << sized()\n    n << size(x$)\n"));
        // one that only counts it reads no item (question 48)
        assert!(frees("x", "\non (int n) << waiting()\n    n << count x$\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `count x$` of a stream is how many items it has had (fm3 question
    /// 94, Ash, 10 October 2026; log 245): the ring's own count, two
    /// lines no reader's place enters. What is waiting where a reader
    /// stands is said `count x$ - position x$` (question 137) and is the
    /// one instruction `count` it always was, with no `position` asked;
    /// and an array's length keeps that instruction
    #[test]
    fn a_stream_s_count_is_how_many_it_has_had() {
        let dir = std::env::temp_dir().join(format!("probe-zero-sofar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-10T10:00:00\n\n## testing\n").unwrap();
        let f = |line: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) << f()\n    int s$ << 1 << 2 << 3\n    advance s$ by (1)\n    n << {}\n", line)).unwrap();
            let ir = lower::lower(&store::read(&dir).unwrap()).unwrap().ir;
            ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n")
        };
        let so_far = f("count s$");
        assert!(so_far.contains(", ring\n") && !so_far.contains(" = count "), "{}", so_far);
        let waiting = f("count s$ - position s$");
        assert!(waiting.contains(" = count ") && !waiting.contains("position(") && !waiting.contains(" = sub "), "{}", waiting);
        // the other way round is two numbers subtracted
        let other = f("position s$ - count s$");
        assert!(other.contains(" = sub ") && !other.contains(" = count "), "{}", other);
        // how many are waiting: the frame is not made to be counted,
        // the reader is moved as it would have moved it (fm3 log 247)
        let framed = f("[count] (frame s$)");
        assert!(framed.contains(" = sub ") && framed.contains(", pos, ") && !framed.contains("frame") && !framed.contains(" = count "), "{}", framed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `x$ at (t)` is an index worked out in integers (fm3 question
    /// 120, log 248): where the compiler knows the stream's rate and
    /// the time's divisor, the count times a reduced ratio, and the
    /// ring read by a place; where it does not, a tick and `__at_<T>`.
    /// The library's exact time and its `sample` are not called
    #[test]
    fn a_stream_is_read_at_a_time_by_an_index() {
        let dir = std::env::temp_dir().join(format!("probe-zero-at-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-10T10:00:00\n\n## testing\n").unwrap();
        let f = |head: &str, decl: &str, read: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) << f (int m)\n{}    n << {}\n", head, decl, read)).unwrap();
            let ir = lower::lower(&store::read(&dir).unwrap()).unwrap().ir;
            ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n")
        };
        // 1 khz against a nanosecond's divisor: one over a million
        let rated = f("", "    int x$ at (1 khz) << 1 << 2 << 3\n", "x$ at (m us)");
        assert!(rated.contains(" = div ") && rated.contains(", 1000000\n") && rated.contains(" = peek ") && !rated.contains("sample") && !rated.contains("__at_"), "{}", rated);
        // a time written out: the index is worked out by the compiler
        let lit = f("", "    int x$ at (1 khz) << 1 << 2 << 3\n", "x$ at (1600 us)");
        assert!(!lit.contains(" = div ") && !lit.contains(" = mul ") && lit.contains(" = cmp.gt 1, "), "{}", lit);
        // 48 khz against a second's thirtieth: times 1600, no divide
        let video = f("", "    int a$ at (48 khz) << 1 << 2 << 3\n", "a$ at (1 s / 30 * m)");
        assert!(!video.contains("sample"), "{}", video);
        // no rate: the tick of the store's clock, and the search
        let sparse = f("int k$\n\n", "    k$ << 1\n", "k$ at (m ms)");
        assert!(sparse.contains("__at_int(") && sparse.contains(" = div ") && !sparse.contains("sample"), "{}", sparse);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stream's declaration says how it is read at a time, in three
    /// of an array's words (fm3 question 127, log 250): `nearest`,
    /// `linear`, `else (v)`; with nothing said the item at or before.
    /// The array's other words are refused of a stream by name
    #[test]
    fn a_stream_says_how_it_is_read() {
        let dir = std::env::temp_dir().join(format!("probe-zero-srule-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-10T10:00:00\n\n## testing\n").unwrap();
        let ir = |decl: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (float32 n) << f (int m)\n    {}\n    n << x$ at (m us)\n", decl)).unwrap();
            let ir = lower::lower(&store::read(&dir).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?.ir;
            Ok(ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n"))
        };
        // nothing said: the count over a million, nothing added
        let held = ir("float32 x$ at (1 khz) << 1.0 << 2.0").unwrap();
        assert!(held.contains(", 1000000\n") && !held.contains(", 999999\n") && !held.contains(", rule, "), "{}", held);
        let near = ir("float32 x$ at (1 khz) nearest << 1.0 << 2.0").unwrap();
        assert!(near.contains(", 999999\n") && near.contains(", 2000000\n") && near.contains(" = set ") && near.contains(", rule, 1\n"), "{}", near);
        // linear: the index, what is left over, and two items read
        let lin = ir("float32 x$ at (1 khz) linear << 1.0 << 2.0").unwrap();
        assert!(lin.contains(" = rem ") && lin.matches(" = peek ").count() == 2 && lin.contains(", rule, 2\n"), "{}", lin);
        // `else`: the value a read before the first item gives
        let other = ir("float32 x$ at (1 khz) else 9.5 << 1.0").unwrap();
        assert!(other.contains("yield 9.5\n"), "{}", other);
        for (decl, why) in [
            ("float32 x$ wrapped", "a stream has no end to go round to"),
            ("float32 x$ mirrored", "a stream has no end to turn back at"),
            ("float32 x$ clamped", "a read after a stream's latest item gives the latest already"),
            ("float32 x$ from (0) to (1)", "a stream's start and its step are its phase and its rate"),
            ("int x$ linear", "two whole numbers blended are not a whole number"),
            ("float32 x$ linear at (1 khz)", "the rate comes first"),
        ] {
            let err = ir(decl).expect_err(decl);
            assert!(err.contains(why), "{}: {}", decl, err);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `time of x$` is the time of the stream's latest item (fm3
    /// question 132, log 249): the ring's count less one over the
    /// rate, with no reader asked; the latest item's tick where the
    /// stream has no rate
    #[test]
    fn a_stream_s_time_is_its_latest_item_s() {
        let dir = std::env::temp_dir().join(format!("probe-zero-timeof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-10T10:00:00\n\n## testing\n").unwrap();
        let f = |decl: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("on (time t) << f()\n    {}\n    advance x$ by (1)\n    t << time of x$\n", decl)).unwrap();
            let ir = lower::lower(&store::read(&dir).unwrap()).unwrap().ir;
            ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n")
        };
        let rated = f("int x$ at (30 hz) << 1 << 2 << 3");
        assert!(rated.contains(", ring\n") && rated.contains(" = pack ") && rated.contains(", 30\n") && !rated.contains("position(") && !rated.contains("tick_of("), "{}", rated);
        let sparse = f("int x$ << 1 << 2 << 3");
        assert!(sparse.contains("tick_of(") && sparse.contains(", 1000000\n") && !sparse.contains("position("), "{}", sparse);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A feature's stream that only its name and `count` read keeps
    /// its latest item and a counter, and no queue (fm3 question 145,
    /// log 247): the counter is one more at each store of the item
    #[test]
    fn a_counted_stream_keeps_a_counter_and_no_queue() {
        let dir = std::env::temp_dir().join(format!("probe-zero-counted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-10T10:00:00\n\n## testing\n").unwrap();
        let ir = |last: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("int c$ << 7\n\non (int n) << f()\n    c$ << c$ + 1\n    n << {}\n", last)).unwrap();
            lower::lower(&store::read(&dir).unwrap()).unwrap().ir
        };
        let counted = ir("count c$ * 10 + c$");
        assert!(counted.contains("__n_c: index") && counted.contains(", __n_c, ") && !counted.contains("__queue_int"), "{}", counted);
        // asked what is waiting, it is a queue
        let waiting = ir("[count] (frame c$)");
        assert!(!waiting.contains("__n_c") && waiting.contains("__queue_int"), "{}", waiting);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a second asking of what is waiting, `count s$ - position s$`, the
    /// one instruction `count` (fm3 question 137, log 245), is the first's
    /// number
    /// where nothing between could have pushed, and only there (fm3 log 112)
    #[test]
    fn count_is_asked_once_where_nothing_between_could_push() {
        let dir = std::env::temp_dir().join(format!("probe-zero-counts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let head = "int far$\n\non (int k) << quiet (int c)\n    k << c + 1\n\non (int k) << loud (int c)\n    far$ << c\n    k << c\n\non (int k) << relayed (int c)\n    k << loud (c)\n\non (int n) << sized()\n    n << count far$\n\n";
        let counts = |f: &str| -> usize {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}", head, f)).unwrap();
            let ir = lower::lower(&store::read(&dir).unwrap()).unwrap().ir;
            let b: Vec<&str> = ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect();
            assert!(b.iter().any(|l| l.contains(" = count ")), "{}", ir);
            b.iter().filter(|l| l.contains(" = count ")).count()
        };
        // (the stream is peeked into in every body, so it is a queue: a
        // stream only counted is a number carried, fm3 log 190)
        let straight = |between: &str| format!("on (int n) << f()\n    int s$ << 1 << 2\n    int z = peek s$ at (0)\n    int a = (count s$ - position s$)\n{}    n << a + (count s$ - position s$)\n", between);
        // arithmetic, a read, and a call to a function that only computes
        assert_eq!(counts(&straight("    int b = a * 2 + peek s$ at (0)\n    int c = quiet (b)\n")), 1);
        // a push, an `end`, a function that pushes, one that calls one that does
        assert_eq!(counts(&straight("    s$ << 3\n")), 2);
        assert_eq!(counts(&straight("    far$ << 3\n")), 2);
        assert_eq!(counts(&straight("    end s$\n")), 2);
        assert_eq!(counts(&straight("    int b = loud (a)\n")), 2);
        assert_eq!(counts(&straight("    int b = relayed (a)\n")), 2);
        // a reader moved on is another value
        assert_eq!(counts(&straight("    advance s$ by (1)\n")), 2);
        // an arm of the first asking's own block that pushes and leaves is passed over ...
        let turn = |arm: &str| format!("on (int n) << f (int k)\n    int s$ << 1 << 2\n    int z = peek s$ at (0)\n    n << loop (int i = 0, int acc = 0) yields acc\n        if (i >= k)\n            break\n        int a = (count s$ - position s$)\n        if (a > 5)\n{}        continue (i + 1, acc + a + (count s$ - position s$))\n", arm);
        assert_eq!(counts(&turn("            s$ << 9\n            continue (i + 1, acc)\n")), 1);
        // ... one that pushes and goes on is not, nor one nested deeper
        assert_eq!(counts(&turn("            s$ << 9\n")), 2);
        assert_eq!(counts(&turn("            if (a > 6)\n                s$ << 9\n                continue (i + 1, acc)\n")), 2);
        // the second asking in a loop the first is not in: the loop's
        // whole body is between, what follows the asking too
        let inner = |after: &str| format!("on (int n) << f (int k)\n    int s$ << 1 << 2\n    int z = peek s$ at (0)\n    int a = (count s$ - position s$)\n    int t = loop (int i = 0, int acc = 0) yields acc\n        if (i >= k)\n            break\n        int c = (count s$ - position s$)\n{}        continue (i + 1, acc + c)\n    n << a + t\n", after);
        assert_eq!(counts(&inner("")), 1);
        assert_eq!(counts(&inner("        int q = quiet (c)\n")), 1);
        assert_eq!(counts(&inner("        s$ << 9\n")), 2);
        // an asking in one arm is not in hand in the other
        assert_eq!(counts("on (int n) << f (int k)\n    int s$ << 1 << 2\n    int z = peek s$ at (0)\n    n << (count s$ - position s$) if (k > 0)\n         else (count s$ - position s$) + 1\n"), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A `time` is a structure the language's own feature declares in
    /// zero, and does only what is declared on it there (fm3 question
    /// 117, log 202): each form with no function is refused in words
    /// that list the methods there are, a time's field is its
    /// feature's own, and a store with no time in it has no line of
    /// one. The words are any structure's, `Vec` as much as `time`
    #[test]
    fn a_time_does_only_what_is_declared() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-time-declared");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → \"x\"\n").unwrap();
        let head = "type Vec =\n    float x, y\n\non (Vec v) << (Vec a) + (Vec b)\n    v << Vec(a.x + b.x, a.y + b.y)\n\n";
        let f = |line: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on run()\n    time beat = 250 ms\n    Vec v = Vec(1.0, 2.0)\n    int n = 3\n    {}\n", head, line)).unwrap();
            emit(&dir)
        };
        let refused = |line: &str, what: &str| {
            let e = f(line).err().unwrap_or_else(|| panic!("not refused: {}", line));
            assert!(e.ends_with(what), "{}: {}", line, e);
        };
        let says = ". No program sees a time's count or its divisor: a number out of a time is a time divided by a time, `t / 1 ms`, and a time out of a number is a number of some time, `n * 1 ms`";
        refused("out$ << beat + 1", "no '+' is defined on a time and an int: '+' on a time is `(time) + (time)`");
        refused("out$ << beat + 0.5", "no '+' is defined on a time and a float: '+' on a time is `(time) + (time)`");
        refused("out$ << 1 + beat", "no '+' is defined on an int and a time: '+' on a time is `(time) + (time)`");
        refused("out$ << beat * beat", "no '*' is defined on a time and a time: '*' on a time is `(time) * (int)`, `(int) * (time)`, `(time) * (float)` and `(float) * (time)`");
        refused("out$ << (beat < 1)", "no '<' is defined on a time and an int: '<' on a time is `(time) < (time)`");
        refused("out$ << beat % beat", "no '%' is defined on a time and a time: a time has '+', '-', '*', '/', '==', '!=', '<', '<=', '>' and '>=' and no '%'");
        refused("out$ << n / beat", "no '/' is defined on an int and a time: '/' on a time is `(time) / (int)`, `(time) / (float)` and `(time) / (time)`");
        refused("out$ << (beat == 1)", "no '==' is defined on a time and an int: '==' on a time is `(time) == (time)`");
        refused("time t = 5", &format!("'t' is a time but the value is a bare number: say its unit, `5 s` or `5 ms`{}", says));
        refused("time t = n", &format!("'t' is time but the value is int{}", says));
        refused("int k = beat", &format!("'k' is int but the value is time{}", says));
        refused("int k = int(beat)", "int(x) converts a number, not a time: a conversion says no unit. A number out of a time is a time divided by a time, `t / 1 ms`, and its whole seconds are `int(t / 1 s)`");
        refused("float k = float(beat)", "and its whole seconds are `float(t / 1 s)`");
        refused("int64 k = beat.count", &format!("'.count' on a time: a field declared `hidden` is the language's own, and no other feature reads or gives it{}", says));
        refused("int64 k = beat.divisor", &format!("'.divisor' on a time: a field declared `hidden` is the language's own, and no other feature reads or gives it{}", says));
        refused("time t = time(5)", &format!("`time(...)` gives a time its 'count': a field declared `hidden` is the language's own, and no other feature reads or gives it{}", says));
        // `hidden` is a word on a field of a structure and nowhere else
        // (fm3 question 118), and no name begins `__`
        refused("hidden int k = 3", "'hidden' is said of a field of a structure, `type account =` and under it `hidden int balance`: the field is then read and given only in the feature that declares the type. A variable is not hidden");
        refused("int64 k = beat.__steps", "a name may not start with '_' ('_' alone is the accumulator; a field a feature keeps to itself is declared `hidden`)");
        refused("time t = time(n)", &format!("no other feature reads or gives it{}", says));
        refused("int __x = 3", "a name may not start with '_' ('_' alone is the accumulator; a field a feature keeps to itself is declared `hidden`)");
        // ... and the same words for a structure of the program's
        refused("out$ << v * v", "no '*' is defined on a Vec and a Vec: a Vec has '+' and no '*'");
        refused("out$ << v + 1", "no '+' is defined on a Vec and an int: '+' on a Vec is `(Vec) + (Vec)`");
        // what is declared works, each a line of integers written where
        // it is used: no function of the IR, and no rational
        let ir = f("out$ << beat * 2 + 100 ms << (beat < 1 s) << beat / 1 ms << 2 * beat").unwrap();
        // (fm3 question 120, log 213: a time is a count over a divisor,
        // and where the compiler knows both, as it does of a literal,
        // the whole line is worked out: 600 ms, true, 250.0 and 500 ms)
        assert!(ir.contains(": nanoseconds = pack 600000000\n") && ir.contains(": nanoseconds = pack 500000000\n") && !ir.contains("__time") && !ir.contains("fn add__time") && !ir.contains("fn mul__time") && !ir.contains(": time = "), "{}", ir);
        // a time whose count the compiler does not know: one multiply,
        // one add, its divisor known and never made
        let ir = f("out$ << n * beat + 100 ms").unwrap();
        assert!(ir.contains(" = mul _1, 250000000\n") && ir.contains(" = add _2, 100000000\n") && !ir.contains("__time"), "{}", ir);
        // ... and one whose divisor it does not know carries it: the
        // divisors compared when the program runs
        let ir = f("out$ << 1 s / n + beat").unwrap();
        assert!(ir.contains("type __time = struct\n    count: i64\n    divisor: i64\n") && ir.contains("over_one_divisor("), "{}", ir);
        // a store with no time in it has no line of one
        std::fs::write(dir.join("h/h.zero"), format!("{}on run()\n    out$ << 3\n", head)).unwrap();
        let ir = emit(&dir).unwrap();
        assert!(!ir.contains("__time") && ir.contains("type Vec = struct"), "{}", ir);
        // the language's own operator is not declared again, nor its type
        let again = |code: &str, what: &str| {
            std::fs::write(dir.join("h/h.zero"), format!("{}\non run()\n    out$ << 1\n", code)).unwrap();
            let e = emit(&dir).err().unwrap_or_else(|| panic!("not refused: {}", code));
            assert!(e.ends_with(what), "{}: {}", code, e);
        };
        again("on (time t) << (time a) + (time b)\n    t << a\n", "an operator is not redefined in this milestone; a `<<` method is");
        again("type time =\n    int n\n", "type 'time' is already declared: it is the language's own");
        // ... but one of its own on a time is a function, called where
        // the compiler does not know what it is handed, and its one
        // line written where it does (fm3 log 213): `beat * beat` is
        // 62.5 ms worked out in place
        std::fs::write(dir.join("h/h.zero"), "on (time t) << (time a) * (time b)\n    t << a * (b / 1 s)\n\non shown (time a)\n    out$ << a * a\n\non run()\n    time beat = 250 ms\n    out$ << beat * beat\n    shown (beat)\n").unwrap();
        let ir = emit(&dir).unwrap();
        assert_eq!(ir.matches("= mul_time(").count(), 1, "{}", ir);
        assert!(ir.contains("= mul_time(a, a)") && ir.contains("\nfn mul_time(a: __time, b: __time) -> __time\n"), "{}", ir);
    }

    /// A name given where an enumeration's value is wanted that is none
    /// of its values is told so, with the values (fm3 hop 34,
    /// transformation 123): it was "no function named 'letter'"
    #[test]
    fn a_name_that_is_no_value_of_an_enumeration() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-enum-values");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → \"x\"\n").unwrap();
        let f = |lines: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("type kind = space | word | number | mark\n\non (int n) << seven()\n    n << 7\n\non run()\n{}    out$ << 1\n", lines)).unwrap();
            emit(&dir)
        };
        let said = "'letter' is not one of kind's values: space, word, number, mark";
        for lines in ["    kind k = letter\n", "    kind k = word\n    out$ << (k == letter)\n"] {
            let e = f(lines).err().unwrap_or_else(|| panic!("not refused: {}", lines));
            assert!(e.ends_with(said), "{}: {}", lines, e);
        }
        // a value of it, and a function's name where one is wanted, are as they were
        f("    kind k = word\n    out$ << (k == mark)\n").unwrap();
        let e = f("    kind k = seven()\n").err().unwrap();
        assert!(!e.contains("is not one of"), "{}", e);
    }

    /// A literal that does not fit the other side of an operator is said
    /// as what it is (fm3 hop 34, transformation 123): `k == 3` on an
    /// enumeration was "'==' on a kind and a decimal", and 3 is no
    /// decimal. The enumeration's values are said with it
    #[test]
    fn a_whole_number_is_not_called_a_decimal() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-literal-said");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → \"x\"\n").unwrap();
        let refused = |line: &str, what: &str| {
            std::fs::write(dir.join("h/h.zero"), format!("type kind = space | word | number | mark\n\non run()\n    kind k = word\n    out$ << ({})\n", line)).unwrap();
            let e = emit(&dir).err().unwrap_or_else(|| panic!("not refused: {}", line));
            assert!(e.ends_with(what), "{}: {}", line, e);
        };
        refused("k == 3", "'==' on a kind and a whole number: a kind is one of space, word, number, mark");
        refused("3 == k", "'==' on a whole number and a kind: a kind is one of space, word, number, mark");
        refused("k == 2.5", "'==' on a kind and a decimal: a kind is one of space, word, number, mark");
    }

    /// A conversion between an abstract whole number and a library
    /// number, a float or a time, in a function whose first line names
    /// no abstract type (fm3 log 173): the lowered IR did not parse,
    /// "no 'conv' takes (int) giving f64", the IR's `conv` not asking
    /// the policy how wide a body's `int` is. The sixteen pairs that
    /// broke, in both forms, under two products
    #[test]
    fn a_conversion_takes_an_abstract_int() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-conv-abstract");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → \"true\"\n").unwrap();
        // (a `time` was the fourth: it is a structure declared in zero
        // now and no number converts to one, fm3 question 117)
        let wasm = suite::backend_policy(Backend::Wasm).unwrap();
        let native = suite::backend_policy(Backend::Native).unwrap();
        let mut tried = 0;
        for whole in ["int", "uint"] {
            for other in ["float", "float32", "float64"] {
                for (f, t) in [(whole, other), (other, whole)] {
                    let same = "b == 55";
                    let a = format!("on run()\n    {} a = 55\n    {} b = {}(a)\n    out$ << ({})\n", f, t, t, same);
                    let b = format!("on ({} b) << turned ({} a)\n    b << {}(a)\n\non run()\n    {} a = 55\n    {} b = turned (a)\n    out$ << ({})\n", t, f, t, f, t, same);
                    for code in [a, b] {
                        std::fs::write(dir.join("h/h.zero"), &code).unwrap();
                        for product in ["int: 32\nfloat: 32\n", "int: 64\nfloat: 64\n"] {
                            std::fs::write(dir.join("product.md"), format!("# product\n\n{}", product)).unwrap();
                            let s = store::read(&dir).unwrap();
                            let l = lower::lower(&s).unwrap();
                            for policy in [&wasm, &native] {
                                if let Err(e) = build(&l.ir, &store_policy(&s, policy), 1) {
                                    panic!("{} under {}: {}", code, product, e);
                                }
                                tried += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(tried, 96);
    }

    /// A literal is held to the type it is given to (fm3 log 173): the
    /// IR said "iconst 300 does not fit in type u8", or took the
    /// literal and wrapped it. Each place a literal is given a type,
    /// the abstract `int` under two products, a time written as a
    /// decimal, and the words for one that reaches the IR unchecked
    #[test]
    fn a_literal_is_held_to_its_type() {
        let dir = std::env::temp_dir().join("probe-zero-literal-held");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → \"?\"\n").unwrap();
        let write = |code: &str, product: &str| {
            std::fs::write(dir.join("h/h.zero"), code).unwrap();
            std::fs::write(dir.join("product.md"), format!("# product\n\n{}", product)).unwrap();
        };
        // lowered, its case resolved under the store's product
        let checked = |code: &str, product: &str| -> Result<String, String> {
            write(code, product);
            let s = store::read(&dir).map_err(|e| e.to_string())?;
            let l = lower::lower(&s).map_err(|e| e.to_string())?;
            let policy = store_policy(&s, &suite::backend_policy(Backend::Native).unwrap());
            calls_of(&s, &l, &policy)?;
            build(&l.ir, &policy, 1)?;
            Ok(l.ir)
        };
        let refused = |code: &str, product: &str, what: &str| {
            let e = checked(code, product).err().unwrap_or_else(|| panic!("not refused: {}", code));
            assert!(e.ends_with(what), "{}: {}", code, e);
        };
        let u8s = "300 does not fit a uint8, which holds 0 to 255. The written conversion, `uint8(300)`, keeps the low 8 bits";
        // a declaration, an operator, a comparison
        refused("on run()\n    uint8 low = 300\n    out$ << low\n", "", &format!("h.zero:2: {}", u8s));
        refused("on run()\n    uint8 low = 3\n    uint8 b = low + 300\n    out$ << b\n", "", &format!("h.zero:3: {}", u8s));
        refused("on run()\n    uint8 low = 3\n    out$ << (low == 300)\n", "", &format!("h.zero:3: {}", u8s));
        // a field's default, a construction, a feature-scope stream and variable
        refused("type P =\n    uint8 a = 300\n\non run()\n    P p\n    out$ << p.a\n", "", &format!("h.zero:2: {}", u8s));
        refused("type P =\n    uint8 a = 0\n\non run()\n    P p(300)\n    out$ << p.a\n", "", &format!("h.zero:5: {}", u8s));
        refused("uint8 n$ << 300\n\non run()\n    out$ << n$\n", "", &format!("h.zero:1: {}", u8s));
        refused("uint8 n = 300\n\non run()\n    out$ << n\n", "", &format!("h.zero:1: {}", u8s));
        // an argument, a result, a list's item, an arm, a push
        refused("on (uint8 r) << f (uint8 x)\n    r << x\n\non run()\n    out$ << f (300)\n", "", &format!("h.zero:5: {}", u8s));
        refused("on (uint8 r) << f ()\n    r << 300\n\non run()\n    out$ << f ()\n", "", &format!("h.zero:2: {}", u8s));
        refused("on run()\n    uint8 x[] = [1, 300]\n    out$ << x[1]\n", "", &format!("h.zero:2: {}", u8s));
        refused("on run()\n    uint8 a = 100\n    uint8 b = 300 if (a > 0) else a\n    out$ << b\n", "", &format!("h.zero:3: {}", u8s));
        refused("on run()\n    uint8 x$ << 1\n    x$ << 300\n    out$ << x$\n", "", &format!("h.zero:3: {}", u8s));
        // below a signed type, below zero in an unsigned one, a char
        refused("on run()\n    int8 low = -129\n    out$ << low\n", "", "h.zero:2: -129 does not fit an int8, which holds -128 to 127. The written conversion, `int8(-129)`, keeps the low 8 bits");
        refused("on run()\n    uint8 low = -1\n    out$ << low\n", "", "h.zero:2: -1 does not fit a uint8, which holds 0 to 255. The written conversion, `uint8(-1)`, keeps the low 8 bits");
        refused("on run()\n    char c = 300\n    out$ << c\n", "", "h.zero:2: 300 does not fit a char, which holds 0 to 255. The written conversion, `char(300)`, keeps the low 8 bits");
        // the ends of each range fit, and the written conversion wraps
        checked("on run()\n    uint8 a = 255\n    int8 b = -128\n    int8 c = 127\n    uint8 d = uint8(300)\n    uint64 e = 18446744073709551615\n    out$ << a << b << c << d << e\n", "").unwrap();
        // an abstract type's range is the product's
        let wide = "3000000000 does not fit an int here: this product's int is 32 bits and holds -2147483648 to 2147483647. How wide an int is belongs to the product, `int: 64` in its product.md; a type that says its width, `int64`, holds it on every product";
        let big = "on run()\n    int big = 3000000000\n    out$ << big\n";
        refused(big, "int: 32\n", &format!("h.zero:2: {}", wide));
        checked(big, "int: 64\n").unwrap();
        refused("on run()\n    out$ << 3000000000\n", "int: 32\n", &format!("h.zero:2: {}", wide));
        refused("on run()\n    int a = 5\n    out$ << a + 3000000000\n", "int: 32\n", &format!("h.zero:3: {}", wide));
        // ... and a concrete type beside it takes the literal under any
        checked("on run()\n    int64 a = 3000000000\n    out$ << a + 3000000000\n", "int: 32\n").unwrap();
        refused("on run()\n    uint low = -1\n    out$ << low\n", "int: 32\n", "h.zero:2: -1 does not fit a uint here: this product's uint is 32 bits and holds 0 to 4294967295. How wide a uint is belongs to the product, `int: 64` in its product.md; a type that says its width, `uint64`, holds it on every product");
        // a time written as a decimal is the whole number of a finer unit
        let ir = checked("on run()\n    out$ << 2.5 s << 0.25 s << 1.000001 ms\n", "").unwrap();
        assert!(ir.contains(": nanoseconds = pack 2500000000\n") && ir.contains(": nanoseconds = pack 250000000\n") && ir.contains(": nanoseconds = pack 1000001\n"), "{}", ir);
        refused("on run()\n    out$ << 1.5 ns\n", "", "h.zero:2: a time is written to the nanosecond: `1.5 ns` is finer");
        // one that reaches the IR unchecked is still said in zero's words
        assert_eq!(unheld("line 0: run: entry: iconst 300 does not fit in type u8").unwrap(), "in 'run': 300 does not fit a uint8, which holds 0 to 255. (The compiler should have named the line: a literal reached the IR unchecked.)");
        assert_eq!(unheld("line 0: f: b1: iconst -129 does not fit in type i8").unwrap(), "in 'f': -129 does not fit an int8, which holds -128 to 127. (The compiler should have named the line: a literal reached the IR unchecked.)");
        assert!(unheld("line 3: unknown opcode 'x'").is_none());
    }

    /// `until (an event)` ends a line that stands at the event (fm3
    /// question 85, log 174). A condition that names the stream being
    /// moved or the line's target is about the item and is asked after
    /// the push, as it was; one that names another stream is watched:
    /// a stream's end sets the line's bit where the stream is ended, a
    /// stream's value is asked by a second function of the line on
    /// that stream, and the moving function asks nothing after its push
    #[test]
    fn a_line_stands_until_an_event() {
        // (one directory, written over each run: nothing is removed)
        let dir = std::env::temp_dir().join("probe-zero-until-event");
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>run() → \"?\"\n").unwrap();
        let head = "int a$\nint b$\nint c$\nint o$\nout$ << (o$ << \" \") forever\n";
        let emitted = |line: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\n\non run()\n{}", head, line, body)).unwrap();
            emit(&dir)
        };
        let func = |ir: &str, name: &str| -> String {
            let at = ir.find(&format!("fn {}(", name)).unwrap_or_else(|| panic!("no {} in {}", name, ir));
            ir[at..].lines().take_while(|l| !l.is_empty()).skip(1).take_while(|l| l.starts_with(' ')).collect::<Vec<_>>().join("\n")
        };
        // a stream's value: the watching function on `b$`, a function
        // of one item, `b$` having no storage; the moving one the push
        // under the bit, with nothing stored after it
        let ir = emitted("o$ << a$ until (b$ == 1)", "    a$ << 1\n    b$ << 1\n    a$ << 2\n").unwrap();
        let watch = func(&ir, "__watch2");
        assert!(watch.contains("cmp.eq __item, 1") && watch.contains("set _2, __until2, 1"), "{}", watch);
        let moves = func(&ir, "__edge2");
        assert!(moves.contains("get _1, __until2") && !moves.contains("set "), "{}", moves);
        let run = func(&ir, "run");
        assert!(run.contains("__edge2(") && run.contains("__watch2(") && !ir.contains("b: int$"), "{}", run);
        // a stream's end: the bit set where the stream is ended, and no
        // second function
        let ir = emitted("o$ << a$ until (ended b$)", "    a$ << 1\n    end b$\n    a$ << 2\n").unwrap();
        assert!(!ir.contains("fn __watch"), "{}", ir);
        let run = func(&ir, "run");
        assert!(run.contains("set _7, __until2, _6\n") && run.contains("    end(_5)\n"), "{}", run);
        assert!(!func(&ir, "__edge2").contains("ended("), "{}", ir);
        // about the item: the source's name, the target's, and both
        // with another stream beside: asked after the push, as it was
        for (cond, body) in [("a$ == 2", "    a$ << 1\n"), ("a$ > b$", "    b$ << 5\n    a$ << 1\n")] {
            let ir = emitted(&format!("o$ << a$ until ({})", cond), body).unwrap();
            assert!(!ir.contains("fn __watch") && func(&ir, "__edge2").contains("__until2, "), "{}: {}", cond, ir);
        }
        // what an event's condition may not read
        let refused = |line: &str, what: &str| {
            let e = emitted(line, "    a$ << 1\n").err().unwrap_or_else(|| panic!("not refused: {}", line));
            assert!(e.ends_with(what), "{}: {}", line, e);
        };
        let not = "h.zero:6: a line that stands until an event ends when the event comes to hold (fm3 question 85), and this one is not built: ";
        refused("o$ << a$ until (empty b$)", &format!("{}`empty x$` is a stream processor's word, true on its one tick after the last. A line that stands ends with `until (ended x$)`", not));
        refused("o$ << a$ until (ended b$ or ended c$)", &format!("{}`ended` inside a larger condition, or of two streams. The condition may be `ended x$` alone", not));
        refused("o$ << a$ until (count b$ > 2)", &format!("{}`count` asked of the stream the condition reads. An event's condition is `ended x$`, or reads one stream by its name, its value now: `until (stop$ == 1)`", not));
        refused("o$ << a$ until (b$ == c$)", &format!("{}its condition reads 2 streams, 'b$' and 'c$', and an event's condition reads one", not));
    }

    /// A published feature's code may be refactored, and must still
    /// pass its own cases (fm3 question 89, log 176). In a scratch
    /// repository: a respelling that gives what it gave passes,
    /// uncommitted and committed after the date, with the date left
    /// alone; one that gives something else refuses the store, naming
    /// the case; and a case that needs another feature on is run with
    /// it on
    #[test]
    fn a_published_feature_is_held_by_its_cases() {
        // (a new directory each run, a repository being made in it:
        // nothing is removed)
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("probe-zero-held-{}-{}", std::process::id(), stamp));
        let dir = root.join("s");
        let feature = |name: &str, head: &str, cases: &str, code: &str| {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(format!("{}/{}.md", name, name)), format!("# {}\n*x*\n\nlayer: runtime\n{}\n> (suite) 2026-09-0{}T10:00:00\n\n## testing\n{}", name, head, if name == "base" { 1 } else { 2 }, cases)).unwrap();
            std::fs::write(dir.join(format!("{}/{}.zero", name, name)), code).unwrap();
        };
        let git = |date: &str, args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(["-c", "user.name=probe", "-c", "user.email=probe@probe", "-c", "commit.gpgsign=false"]).args(args).env("GIT_AUTHOR_DATE", date).env("GIT_COMMITTER_DATE", date).output().unwrap();
            assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        };
        // `base` gives a number; `top`, published, doubles it, so its
        // case needs `base` on
        feature("base", "", ">seed() → 21\n", "on (int n) << seed()\n    n << 21\n");
        let top = |code: &str| feature("top", "published: 2026-09-05\n", ">answer() → 42\n", code);
        top("on (int n) << answer()\n    n << seed() * 2\n");
        git("2026-09-01T10:00:00", &["init", "-q"]);
        git("2026-09-01T10:00:00", &["add", "."]);
        git("2026-09-01T10:00:00", &["commit", "-q", "-m", "base and top"]);
        let tested = || test(&root, Backend::Native, 1).unwrap();
        let fine = |r: &suite::Report| r.failed == 0 && r.log.contains("answer() → 42");
        assert!(fine(&tested()), "{}", tested().log);
        // a refactoring, uncommitted: the date left alone, and it passes
        top("on (int n) << answer()\n    int s = seed()\n    n << s + s\n");
        assert!(store::read(&dir).unwrap().features.iter().any(|f| f.name == "top" && f.changed.as_deref() == Some("has uncommitted changes")));
        assert!(fine(&tested()), "{}", tested().log);
        // ... and committed after the date
        git("2026-09-08T10:00:00", &["commit", "-q", "-a", "-m", "top refactored"]);
        assert!(fine(&tested()), "{}", tested().log);
        // a change of what it gives: the store is refused, naming the case
        top("on (int n) << answer()\n    n << seed() * 2 + 1\n");
        let r = tested();
        assert!(r.failed == 1 && r.log.contains("feature top was published on 2026-09-05 and its code has uncommitted changes: a published feature may be refactored and must still pass its own cases (fm3 question 89), and `answer() → 42` does not (got 43). A change of what a feature means is a sub-feature that redefines what it needs and calls `existing` (structure.md, the lifecycle)"), "{}", r.log);
        // ... and so is running any case of it
        let e = run(&dir, "seed()", &suite::backend_policy(Backend::Native).unwrap(), 1, true).unwrap_err();
        assert!(e.starts_with("feature top was published on 2026-09-05 and its code has uncommitted changes") && e.contains("`answer() → 42` does not (got 43)"), "{}", e);
    }

    /// a product's bound (log 41) reaches every loop of the function it
    /// names, marked as the product's, and `probe cost` counts it
    #[test]
    fn a_product_bound_reaches_the_loops() {
        let ir = emit(Path::new("suite/zero/tasks")).unwrap();
        assert!(ir.contains("; product setting: bound count down from: 5\nfn count_down_from("), "{}", ir);
        let body: String = ir.lines().skip_while(|l| !l.starts_with("fn count_down_from(")).skip(1).take_while(|l| l.starts_with(' ') || l.is_empty()).collect::<Vec<_>>().join("\n");
        assert!(body.contains("loop() bound 5\n"), "{}", body);
        let policy = suite::backend_policy(Backend::Native).unwrap();
        let module = build(&ir, &policy, 1).unwrap();
        let mut coster = crate::cost::Coster::new(&module, None, None, None);
        let r = coster.report("count_down_from").unwrap();
        assert!(r.loops.iter().any(|l| l.contains("x5 (declared)")), "{:?}", r.loops);
        // hello's countdown shows its count without a bound anywhere
        let ir = emit(Path::new("suite/zero/hello")).unwrap();
        assert!(!ir.contains("product setting"));
        let module = build(&ir, &policy, 1).unwrap();
        let mut coster = crate::cost::Coster::new(&module, None, None, None);
        let r = coster.report("count_down").unwrap();
        assert!(r.loops.iter().any(|l| l.contains("count_down: loop at b1 x10 (")), "{:?}", r.loops);
    }

    /// A wire whose delay changes while the program runs, and that goes
    /// and comes back (fm3 log 289). restart, everything on the server, its key at
    /// 3.5 s: the key waits on a slow wire, is held by a gone one until
    /// it comes back, and never reaches the server where it never does, the
    /// pieces after it never shown and the runner saying so. And the
    /// order the wire keeps, where the delay falls from 2 s to 100 ms:
    /// no item arrives before one put on before it the same way (no
    /// store of the suite puts two on one way across such a fall, the
    /// places waiting for answers; the order is held all the same)
    #[test]
    fn a_wire_slows_goes_and_comes_back() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let restart = Path::new("suite/zero/restart");
        let key = "launch() with in \"k\" at 3.5 s";
        let line = "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.5 s";
        let late = |pieces: &[Shown]| -> Vec<(String, i64, i64)> { pieces.iter().filter(|p| p.shown > p.own).map(|p| (p.text.clone(), p.own, p.shown)).collect() };
        let wire = |then: Vec<(i64, i64)>| Wire { d: 100_000, then, ..Default::default() };
        // slower from 3 s: the key reaches the server at 5.5 s and its text the client at 7.5 s
        let (_, shown, pieces, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], wire(vec![(3_000_000, 2_000_000)]), 0, None).unwrap();
        assert_eq!(shown, line);
        assert_eq!(late(&pieces)[1], ("10\n".to_string(), 3_500_000, 7_500_000));
        // gone from 3 s and back at 6 s with 100 ms: the key is held, and arrives at 6.1 s
        let (_, shown, pieces, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], wire(vec![(3_000_000, GONE), (6_000_000, 100_000)]), 0, None).unwrap();
        assert_eq!(shown, line);
        assert_eq!(late(&pieces)[1], ("10\n".to_string(), 3_500_000, 6_200_000));
        // gone for good: the first countdown to the key, and the runner says the rest is the wire's
        let out = run_wire_moved(restart, key, &native, opt::MAX_LEVEL, Some("all"), &[], wire(vec![(3_000_000, GONE)])).unwrap();
        assert!(out.starts_with(&format!("{}\n→ \"10\\n9\\n8\\n7\" at 1 hz\n", key)), "{}", out);
        assert!(out.contains("\nnever shown: 11 pieces, the wire gone from 3 s\n"), "{}", out);
        // the order kept, the arrivals each way never going back
        let table = placement_wire_moved(restart, Some("all"), &[], wire(vec![(3_000_000, 2_000_000), (4_000_000, 100_000)]), Some(key), &native, opt::MAX_LEVEL).unwrap();
        let rows: Vec<(String, i64)> = table.lines().filter_map(|l| {
            let cols: Vec<&str> = l.split("  ").map(str::trim).filter(|c| !c.is_empty()).collect();
            let at = cols.get(1).and_then(|a| time_said(a).ok())?;
            let way = cols.get(2).filter(|w| w.contains('→'))?;
            Some((way.to_string(), at))
        }).collect();
        assert!(rows.len() > 10, "{}", table);
        for way in ["client → server", "server → client"] {
            let ats: Vec<i64> = rows.iter().filter(|(w, _)| w == way).map(|(_, a)| *a).collect();
            assert!(ats.windows(2).all(|p| p[0] <= p[1]), "{}: {:?}\n{}", way, ats, table);
        }
        let (_, shown, _, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], wire(vec![(3_000_000, 2_000_000), (4_000_000, 100_000)]), 0, None).unwrap();
        assert_eq!(shown, line);
        // a wire that does not change is built as it was: no table of changes
        let steady = emit_wired(restart, Some("all"), &[], Some(&Wire::from(100_000))).unwrap();
        assert!(!steady.contains("\nfn __wat(") && !steady.contains("data __wsch"));
        let changing = emit_wired(restart, Some("all"), &[], Some(&wire(vec![(3_000_000, GONE)]))).unwrap();
        assert!(changing.contains("\nfn __wat(") && changing.contains("__wire_at(0, 3000000, 4611686018427387904)"));
    }

    /// A wire with a speed, and one that comes and goes (fm3 log 294):
    /// on each way an item goes on when the way is free, taking its
    /// bytes over the speed, and arrives the delay after it has gone on,
    /// a piece of text whole; and no item goes on in a gap of the coming
    /// and going. A wire that says neither is built as it was
    #[test]
    fn a_wire_has_a_speed_and_comes_and_goes() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let line = "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let speed = 400;
        let wire = Wire { d: 100_000, speed, ..Default::default() };
        // the items each way: when they left, went on, their bytes, when they arrived, what
        let items = |table: &str| -> Vec<(String, i64, i64, i64, i64, String)> {
            table.lines().filter_map(|l| {
                let cols: Vec<&str> = l.split("  ").map(str::trim).filter(|c| !c.is_empty()).collect();
                let way = cols.get(4).filter(|w| w.contains('→'))?;
                Some((way.to_string(), time_said(cols[0]).ok()?, time_said(cols[1]).ok()?, cols[2].parse().ok()?, time_said(cols[3]).ok()?, cols[5].to_string()))
            }).collect()
        };
        for then in [vec![], vec![("all".to_string(), 2_000_000)]] {
            let server = if then.is_empty() { "all" } else { "none" };
            let table = placement_wire_moved(hello, Some(server), &then, &wire, None, &native, opt::MAX_LEVEL).unwrap();
            let rows = items(&table);
            assert!(rows.len() > 10, "{}", table);
            for way in ["client → server", "server → client"] {
                let mut free = 0;
                for (_, left, on, n, at, what) in rows.iter().filter(|r| r.0 == way) {
                    // (the start, before the case, takes no time)
                    if *at == *left && *on == *left {
                        continue;
                    }
                    let took = (n * 1_000_000 + speed - 1) / speed;
                    assert_eq!(*on, (*left).max(free) + took, "{}: {} went on at {}\n{}", way, what, on, table);
                    free = *on;
                    // (a piece of text, and a move, arrive whole: with their last item, fm3 log 298)
                    if what.starts_with("text") || what.starts_with("the move's word") || what.contains("handed across at the move") {
                        assert!(*at >= on + 100_000, "{}\n{}", what, table);
                    } else {
                        assert_eq!(*at, on + 100_000, "{}\n{}", what, table);
                    }
                }
            }
            let (_, shown, _, _) = wire_moved_case(hello, "run()", &native, opt::MAX_LEVEL, Backend::Native, Some(server), &then, &wire, 0, None).unwrap();
            assert_eq!(shown, line);
        }
        // a piece of text is one item: its head once, its characters arriving together
        let table = placement_wire_moved(hello, Some("all"), &[], &wire, None, &native, opt::MAX_LEVEL).unwrap();
        let rows = items(&table);
        let ten = rows.iter().find(|r| r.5 == "text \"10\\n\"").unwrap();
        assert_eq!(ten.3, (crate::zero::places::PACE_HEAD + 3) as i64, "{}", table);
        let bye: Vec<_> = rows.iter().filter(|r| r.5.starts_with("text") && r.5 != "text \"10\\n\"" && r.4 == rows.iter().find(|r| r.5 == "text \"goodbye\"").unwrap().4).collect();
        assert_eq!(bye.iter().map(|r| r.3).sum::<i64>(), (crate::zero::places::PACE_HEAD + 20) as i64, "{}", table);
        // gone 500 ms every 2 s from 1 s: the move said at 1.2 s goes on at 1.5 s, when the wire is back
        let coming = Wire { d: 100_000, every: Some((500_000, 2_000_000, 1_000_000)), ..Default::default() };
        let table = placement_wire_moved(hello, Some("none"), &[("all".to_string(), 1_200_000)], &coming, None, &native, opt::MAX_LEVEL).unwrap();
        let rows = items(&table);
        let word = rows.iter().find(|r| r.5.starts_with("the move's word")).unwrap();
        assert_eq!((word.1, word.2, word.4), (1_200_000, 1_500_000, 1_600_000), "{}", table);
        for r in &rows {
            let ph = (r.2 - 1_000_000).rem_euclid(2_000_000);
            assert!(r.2 < 1_000_000 || ph >= 500_000 || r.2 == r.1 && r.4 == r.1, "{:?} went on in a gap\n{}", r, table);
        }
        let (_, shown, _, _) = wire_moved_case(hello, "run()", &native, opt::MAX_LEVEL, Backend::Native, Some("none"), &[("all".to_string(), 1_200_000)], &coming, 0, None).unwrap();
        assert_eq!(shown, line);
        // a wire that says neither is built as it was; one that does has the queue
        let plain = emit_wired(hello, Some("all"), &[], Some(&Wire::from(100_000))).unwrap();
        assert!(!plain.contains("data __wsp") && !plain.contains("\nfn __wpace("));
        let paced = emit_wired(hello, Some("all"), &[], Some(&wire)).unwrap();
        assert!(paced.contains("data __wsp") && paced.contains("__wire_pace(400, 0, 0, 0)"));
    }

    /// What the chooser measures of the wire (fm3 log 295), printed
    /// beneath its choices: on a steady wire the delay, no speed seen to
    /// limit and no gap; on a narrow wire its speed, from the server's items
    /// spaced by the wire; after the speed rises, the new one; and on a
    /// wire that comes and goes, its gaps. Measuring changes no choice
    #[test]
    fn the_chooser_measures_the_wire() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let said = |wire: Wire| -> String {
            let out = run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), &[], Wire { choose: Some(250_000), ..wire }).unwrap();
            out.lines().find(|l| l.starts_with("the wire: ")).unwrap_or_else(|| panic!("{}", out)).to_string()
        };
        let w = |speed: i64| Wire { d: 100_000, speed, ..Default::default() };
        assert_eq!(said(w(0)), "the wire: about 100 ms, its speed not seen to limit, not gone in the last 10 s");
        assert!(said(w(40)).contains(", about 40 bytes/s, "), "{}", said(w(40)));
        let rises = Wire { then: vec![(5_000_000, 100_000)], speeds: vec![Some(10_000)], ..w(40) };
        assert!(said(rises).starts_with("the wire: about 100 ms, about 10000 bytes/s, "));
        let coming = Wire { every: Some((500_000, 2_000_000, 1_000_000)), ..w(0) };
        assert!(said(coming.clone()).ends_with(", gone 5 times in the last 10 s, 3.45 s in all"), "{}", said(coming));
        // two figures
        assert_eq!((two_figures(39), two_figures(9_987), two_figures(5_612_000)), (39, 10_000, 5_600_000));
    }

    /// What the chooser weighs before it gives the server its part back (fm3
    /// log 296), each against a switch only this test has. A wire gone
    /// 500 ms every 2 s: weighed, the part is taken at the first gap,
    /// given back after it, as after a wire that went once, and kept from
    /// the second, the wire's history asking 3.35 s of steady round
    /// trips; the wire's history ignored (`__wire_but` 1024), it goes back
    /// and forth four times. A narrow wire, 1 000 bytes a second: weighed,
    /// `tick`'s part is kept, the speed less than twice what the cut given
    /// put on the wire; the speed ignored (2048), it is given back, its
    /// move's 928 bytes take most of a second to arrive, and the client,
    /// a round trip held behind them, takes the part again: three moves.
    /// A steady wire: no move either way
    #[test]
    fn the_chooser_weighs_the_wire() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let (hello, tick) = (Path::new("suite/zero/hello"), Path::new("suite/zero/tick"));
        let line = "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let run = |dir: &Path, case: &str, wire: &Wire, but: i64| -> (String, Vec<Shown>, Vec<i64>) {
            let ((_, shown, pieces, _), _, log) = wire_moved_case_said(dir, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], wire, but, None).unwrap();
            (shown, pieces, log)
        };
        let moves = |log: &[i64]| log.chunks(6).filter(|c| c.len() == 6 && (c[1] == 15 || c[1] == 16)).count();
        let most = |pieces: &[Shown]| pieces.iter().map(|p| p.shown - p.own).max().unwrap_or(0);
        let coming = Wire { d: 100_000, every: Some((500_000, 2_000_000, 1_000_000)), choose: Some(250_000), ..Default::default() };
        let (shown, pieces, log) = run(hello, "run()", &coming, 0);
        assert_eq!((shown.as_str(), moves(&log), most(&pieces)), (line, 3, 200_000));
        let said = chose_said(&coming, &log);
        assert!(said.contains("\nkept: nothing on the server at 4.6 s: gone 2 times in the last 10 s, 1.35 s in all, so 3.35 s of round trips within 250 ms are wanted\n"), "{}", said);
        let (shown, _, log) = run(hello, "run()", &coming, 1024);
        assert_eq!(shown, line);
        assert!(moves(&log) >= 4, "{}", chose_said(&coming, &log));
        let narrow = Wire { d: 100_000, speed: 1000, choose: Some(250_000), ..Default::default() };
        let (shown, pieces, log) = run(tick, "mixed()", &narrow, 0);
        let (shown2, pieces2, log2) = run(tick, "mixed()", &narrow, 2048);
        assert_eq!(shown, shown2);
        assert_eq!((moves(&log), most(&pieces)), (1, 250_000), "{}", chose_said(&narrow, &log));
        assert!(chose_said(&narrow, &log).contains("\nkept: nothing on the server at 3 s: about 1000 bytes/s, less than twice the 3200 bytes/s the cut given put on the wire\n"));
        // (the speed ignored, the part is given back and its move, arriving whole, holds a round trip past the bound: taken again by running the server's part, three moves where one was weighed, fm3 log 298)
        assert!(moves(&log2) >= 3, "{}", chose_said(&narrow, &log2));
        // a steady wire, weighed or not: the cut given throughout
        let steady = Wire { d: 100_000, choose: Some(250_000), ..Default::default() };
        for but in [0, 1024 | 2048] {
            let (shown, _, log) = run(hello, "run()", &steady, but);
            assert_eq!((shown.as_str(), moves(&log)), (line, 0));
        }
    }

    /// The cut chosen (fm3 log 290). With a steady wire of 100 ms every
    /// round trip is 200 ms, under the bound of 250 ms, and the chooser
    /// holds the cut given: hello and restart as late as at a fixed cut.
    /// Gone at 3 s and back at 6 s, hello's chooser chooses nothing on the server
    /// at 3.25 s, a round trip of its own out longer than the bound, and
    /// gives the server its part back at 7.2 s, a second of round trips within
    /// it; the case is hello's own line. The move is hop forty-nine's,
    /// which waits for the server's state, so the pieces the client was keeping
    /// for after 3.25 s wait for the wire (173 mends it). Given nothing
    /// the server, the chooser is off
    #[test]
    fn the_cut_is_chosen() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let restart = Path::new("suite/zero/restart");
        let line = "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let chosen = |then: Vec<(i64, i64)>, b: i64| Wire { d: 100_000, then, choose: Some(b), ..Default::default() };
        let out = run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), &[], chosen(vec![], 250_000)).unwrap();
        assert!(out.starts_with(&format!("run()\n→ {}\nlate: 1 piece of 12, the most 200 ms\n", line)), "{}", out);
        assert!(out.contains("\nchose: nothing; the cut given held throughout\n") && out.contains("the longest back 200 ms"), "{}", out);
        let key = "launch() with in \"k\" at 3.5 s";
        let out = run_wire_moved(restart, key, &native, opt::MAX_LEVEL, Some("all"), &[], chosen(vec![], 250_000)).unwrap();
        assert!(out.contains("\nlate: 2 pieces of 15, the most 200 ms\n") && out.contains("\nchose: nothing; the cut given held throughout\n"), "{}", out);
        let back = vec![(3_000_000, GONE), (6_000_000, 100_000)];
        let out = run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), &[], chosen(back.clone(), 250_000)).unwrap();
        assert!(out.starts_with(&format!("run()\n→ {}\n", line)), "{}", out);
        assert!(out.contains("\nchose: nothing on the server at 3.25 s, by running the server's part again: ") && out.contains("\nchose: the cut given at 7.2 s: "), "{}", out);
        let table = placement_wire_moved(hello, Some("all"), &[], chosen(back.clone(), 250_000), None, &native, opt::MAX_LEVEL).unwrap();
        assert!(table.contains("chooses nothing on the server: a round trip of its own out longer than 250 ms") && table.contains("the move's word"), "{}", table);
        // the cut given nothing on the server: the chooser has nothing to choose, and begins no round trip
        let off = run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("none"), &[], chosen(back, 250_000)).unwrap();
        assert!(off.contains("\nchose: nothing; the cut given held throughout\n") && off.contains("round trips: 0"), "{}", off);
        // `--choose` with `--then` is refused by name
        let err = run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), &[("none".to_string(), 4_500_000)], chosen(vec![], 250_000)).unwrap_err();
        assert!(err.contains("`--choose` and `--then` are not said together"), "{}", err);
        // a wire's build with no `--choose` has no chooser
        let plain = emit_wired(hello, Some("all"), &[], Some(&Wire::from(100_000))).unwrap();
        assert!(!plain.contains("\nfn __wchoose(") && !plain.contains("data __wc:"));
    }

    // The client takes the server's part by running it again (fm3 cut.md, "the
    // client runs the server's part again"; log 292): hello's own line over a
    // wire that slows, goes, or goes and comes back, nothing late but
    // the first 10; restart's key, sent to the server just before the wire went,
    // read again where `launch` now is; and three switches only this
    // test has, each giving something else: run again without the log,
    // old items not dropped, and run again from where the server is and not
    // from its start, the last on a running sum of the test's own
    #[test]
    fn the_client_runs_the_servers_part_again() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let hello = Path::new("suite/zero/hello");
        let restart = Path::new("suite/zero/restart");
        let line = "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let chosen = |then: Vec<(i64, i64)>| Wire { d: 100_000, then, choose: Some(250_000), ..Default::default() };
        for then in [vec![(3_000_000, 2_000_000)], vec![(3_000_000, GONE)], vec![(3_000_000, GONE), (6_000_000, 100_000)]] {
            let out = run_wire_moved(hello, "run()", &native, opt::MAX_LEVEL, Some("all"), &[], chosen(then.clone())).unwrap();
            assert!(out.starts_with(&format!("run()\n→ {}\nlate: 1 piece of 12, the most 200 ms\n  \"10\\n\" for 0 s, shown at 200 ms, 200 ms late\n", line)), "{:?}\n{}", then, out);
            assert!(out.contains("\nchose: nothing on the server at 3.25 s, by running the server's part again: "), "{}", out);
        }
        let table = placement_wire_moved(hello, Some("all"), &[], chosen(vec![(3_000_000, GONE), (6_000_000, 100_000)]), None, &native, opt::MAX_LEVEL).unwrap();
        assert!(table.contains("runs the server's part again from its start to 3.25 s: 3 turns, 1 item of its log given at their times; takes the server's units") && table.contains("the move's word, the server's part taken by the client"), "{}", table);
        // restart, the key at 3.5 s reaching the server at 3.6 s and the wire gone at 3.55 s: the server's answer never comes, the client runs `launch` again with the key it sent, and the countdown begins again at the key
        let key = "launch() with in \"k\" at 3.5 s";
        let own = "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 3.5 s";
        let gone = vec![(3_550_000, GONE)];
        let (_, shown, pieces, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], chosen(gone.clone()), 0, None).unwrap();
        assert_eq!(shown, own);
        assert!(pieces.iter().any(|p| p.text == "10\n" && p.own == 3_500_000 && p.shown == 3_750_000), "{:?}", pieces);
        // without the log the part run again has no call and no key, and nothing it gives
        let (_, shown, _, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], chosen(gone), 128, None).unwrap();
        assert_eq!(shown, "\"10\\n9\\n8\\n7\" at 1 hz", "run again without the log, the countdown has not begun again at the key");
        // the wire back at 6 s: what the server sent at 3.6 s, the second countdown, arrives then, made before the server heard of the move, and is dropped
        let back = vec![(3_550_000, GONE), (6_000_000, 100_000)];
        let (_, shown, _, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], chosen(back.clone()), 0, None).unwrap();
        assert_eq!(shown, own);
        let (_, twice, _, _) = wire_moved_case(restart, key, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], chosen(back), 256, None).unwrap();
        assert!(twice.matches("10\\n").count() > own.matches("10\\n").count(), "old items not dropped, a number is shown twice: {}", twice);
        // a running sum the server keeps across its turns, run again from its start, and from where the server is
        let dir = std::env::temp_dir().join(format!("probe-wire-again-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sum")).unwrap();
        std::fs::write(dir.join("sum/sum.zero"), "int beat$ at (1 hz)\nint sum$\nbeat$ << beat$ + 1 if (beat$ < 24) forever\nsum$ << sum$ + beat$ forever\nout$ << (sum$ << \"\\n\") forever\n\non begin()\n    sum$ << sum$ + 100\n").unwrap();
        let sum = "\"1\\n101\\n\" at 0 s, \"103\\n106\\n110\\n115\\n121\\n128\\n136\\n145\\n155\\n166\\n178\\n191\\n205\\n220\\n236\\n253\\n271\\n290\\n310\\n331\\n353\\n376\\n400\" at 1 hz from 1 s";
        std::fs::write(dir.join("sum/sum.md"), format!("# sum\n*a running sum the server keeps across its turns*\n\nlayer: runtime\n\n> (suite) 2026-10-16T12:00:00\nA clock of twenty-four beats, their running sum, and a call that adds a hundred to it.\n\n## testing\n>begin() → {}\n", sum)).unwrap();
        let slows = vec![(3_000_000, 2_000_000)];
        let (_, shown, _, _) = wire_moved_case(&dir, "begin()", &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], chosen(slows.clone()), 0, None).unwrap();
        assert_eq!(shown, sum);
        let (_, wrong, _, _) = wire_moved_case(&dir, "begin()", &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], chosen(slows), 512, None).unwrap();
        assert!(wrong.contains("\"371\\n390\\n410\\n431\\n453\\n476\\n500\" at 1 hz from 17 s"), "run again from where the server is, the call is given again to a server that had it: a hundred too many, 371 for 271: {}", wrong);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The server's new start at a giving back (fm3 log 298, question 168). A
    /// store of this test's own, a countdown whose running sum the server keeps
    /// and keys, over a wire gone 500 ms every 2 s from 1 s: the server's part is
    /// taken at 1.25 s, given back at 2.7 s, where the client keeps the server's
    /// state as the server will have it once it takes the hand, and taken again
    /// at 3.2 s, from that start; a key at 4 s, after the second taking,
    /// is read at the client. The case's own line. With the server's start not
    /// moved (`__wire_but` 4096) the second taking runs from the world's
    /// start, and a number is shown that is no piece of the line. And
    /// what the three `going-two` runs were: a move over a wire with a
    /// speed handing two functions partway, the second arriving after the server
    /// had worked past its time (a move now arrives whole); and a taking
    /// while the giving back's word was still on the wire
    #[test]
    fn the_server_starts_again_at_a_giving_back() {
        let native = suite::backend_policy(Backend::Native).unwrap();
        let dir = std::env::temp_dir().join(format!("probe-wire-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sum")).unwrap();
        std::fs::write(dir.join("sum/sum.zero"), "int i$ at (1 hz)\nint sum$\nsum$ << sum$ + i$ forever\nout$ << (sum$ << \"\\n\") forever\nint key$ = codes (in$)\nout$ << (\"key \" << key$ << \"\\n\") forever\n\non (int n) << code of (char c)\n    n << int(c)\n\non (int k$) << codes (char c$)\n    k$ << code of (c$)\n\non count down()\n    i$ << [5 through 1]\n    out$ << \"liftoff\\n\"\n").unwrap();
        let case = "count down() with in \"ab\" at 1 s, in \"c\" at 2.5 s, in \"d\" at 4 s";
        std::fs::write(dir.join("sum/sum.md"), format!("# sum\n*a running sum the server keeps, and keys*\n\nlayer: runtime\n\n> (suite) 2026-10-17T12:00:00\nA countdown, its running sum, and keys.\n\n## testing\n>{} → \"5\\n\" at 0 s, \"key 97\\nkey 98\\n9\\n\" at 1 s, \"12\\n\" at 2 s, \"key 99\\n\" at 2.5 s, \"14\\n\" at 3 s, \"key 100\\n15\\n\" at 4 s, \"liftoff\" at 5 s\n", case)).unwrap();
        let own = "\"5\\n\" at 0 s, \"key 97\\nkey 98\\n9\\n\" at 1 s, \"12\\nkey 99\\n14\\n\" at 2 hz from 2 s, \"key 100\\n15\\n\" at 4 s, \"liftoff\" at 5 s";
        let coming = Wire { d: 100_000, every: Some((500_000, 2_000_000, 1_000_000)), choose: Some(250_000), ..Default::default() };
        let ((_, shown, _, _), _, log) = wire_moved_case_said(&dir, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], &coming, 0, None).unwrap();
        assert_eq!(shown, own);
        let said = chose_said(&coming, &log);
        assert!(said.contains("chose: nothing on the server at 1.25 s, by running the server's part again") && said.contains("\nchose: the cut given at 2.7 s: ") && said.contains("\nchose: nothing on the server at 3.2 s, by running the server's part again"), "{}", said);
        let table = placement_wire_moved(&dir, Some("all"), &[], coming.clone(), Some(case), &native, opt::MAX_LEVEL).unwrap();
        assert!(table.contains("keeps the server's state as the server will have it once it takes the hand as the server's new start") && table.contains("runs the server's part again from the server's start at 2.7 s to 3.2 s: "), "{}", table);
        let ((_, wrong, _, _), _, _) = wire_moved_case_said(&dir, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], &coming, 4096, None).unwrap();
        assert!(wrong != own && wrong.contains("14\\n4\\n"), "run again from the world's start, a 4 that is no piece of the line: {}", wrong);
        let _ = std::fs::remove_dir_all(&dir);
        // going-two, hop fifty-two's chooser over 1 000 bytes a second: the three runs hop fifty-three left wrong
        let two = Path::new("suite/zero/going-two");
        let narrow = Wire { d: 100_000, speed: 1000, choose: Some(250_000), ..Default::default() };
        for (case, line) in [
            ("count down from (5) with in \"k\" at 2.5 s", "\"5\\n4\\n3\\n\" at 1 hz, \"2\\n10\\n\" at 3 s, \"1\\n9\\n\" at 4 s, \"8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 5 s"),
            ("launch() with in \"k\" at 3.5 s", "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"6\\n10\\n\" at 4 s, \"5\\n9\\n\" at 5 s, \"4\\n8\\n\" at 6 s, \"3\\n7\\n\" at 7 s, \"2\\n6\\n\" at 8 s, \"1\\n5\\n\" at 9 s, \"liftoff4\\n3\\n2\\n1\\nliftoff\" at 1 hz from 10 s"),
            ("launch() with in \"k\" at 3.5 s, in \"k\" at 5.5 s", "\"10\\n9\\n8\\n7\\n\" at 1 hz, \"6\\n10\\n\" at 4 s, \"5\\n9\\n\" at 5 s, \"4\\n8\\n10\\n\" at 6 s, \"3\\n7\\n9\\n\" at 7 s, \"2\\n6\\n8\\n\" at 8 s, \"1\\n5\\n7\\n\" at 9 s, \"liftoff4\\n6\\n\" at 10 s, \"3\\n5\\n\" at 11 s, \"2\\n4\\n\" at 12 s, \"1\\n3\\n\" at 13 s, \"liftoff2\\n1\\nliftoff\" at 1 hz from 14 s"),
        ] {
            let (_, shown, _, _) = wire_moved_case(two, case, &native, opt::MAX_LEVEL, Backend::Native, Some("all"), &[], narrow.clone(), 1024 | 2048, None).unwrap();
            assert_eq!(shown, line, "{}", case);
        }
        // ... and the first of them with no chooser: the cut moved at 3.6 s over a wire with a speed, two countdowns handed
        let fast = Wire { d: 100_000, speed: 100_000, ..Default::default() };
        let (_, shown, _, _) = wire_moved_case(two, "launch() with in \"k\" at 3.5 s, in \"k\" at 5.5 s", &native, opt::MAX_LEVEL, Backend::Native, Some("none"), &[("all".to_string(), 3_600_000)], fast, 0, None).unwrap();
        assert!(shown.starts_with("\"10\\n9\\n8\\n7\\n\" at 1 hz, \"6\\n10\\n\" at 4 s, "), "a move arrives whole: {}", shown);
    }
}
