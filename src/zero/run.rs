//! The zero runner: `probe zero <store> emit`, `probe zero <store> run
//! <case>`, and `probe zero test [dir] [path]`. A store is lowered once
//! to IR text, the text goes through probe's parser and everything after
//! it — the IR is the oracle for the front end — and the cases from the
//! features' `## testing` sections run on the chosen path through the
//! suite's own machinery (`suite::run_calls`).

use super::{lower, store};
use crate::suite::{self, Backend, Report};
use crate::{opt, ssa};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

/// lower a store and print its IR: one text on every path, the widths
/// being the policy's (log 52)
pub fn emit(dir: &Path) -> Result<String, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    Ok(l.ir)
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

/// the lowered store as a module under a policy: parsed, resolved,
/// verified, optimized, verified again
pub fn build(ir: &str, policy: &ssa::Policy, level: usize) -> Result<ssa::Module, String> {
    let mut module = ssa::parse_with(&ssa::with_prelude(ir), policy).map_err(|e| format!("the lowered IR did not parse: {}", e))?;
    ssa::resolve_types(&mut module, policy);
    ssa::verify(&module).map_err(|errs| format!("the lowered IR did not verify: {}", errs.join("; ")))?;
    opt::optimize(&mut module, level);
    ssa::verify(&module).map_err(|errs| format!("after optimization: {}", errs.join("; ")))?;
    Ok(module)
}

/// a case as the runner places it (log 44): the line as written, the
/// call, the feature it came from and that feature's place in the
/// composition order, and where it was written
struct Planned {
    text: String,
    call: lower::Call,
    feature: String,
    rank: usize,
    file: String,
    line: usize,
}

/// the calls a store's cases make, in the order the features compose;
/// a case's literal arguments are typed at the width the store is
/// built at (log 52)
fn calls_of(s: &store::Store, l: &lower::Lowered, policy: &ssa::Policy) -> Result<Vec<Planned>, String> {
    let mut out = Vec::new();
    for (rank, f) in s.features.iter().enumerate() {
        for c in &f.cases {
            let call = lower::resolve_case(l, c, &f.md_file, int_bits(policy)).map_err(|e| e.to_string())?;
            out.push(Planned { text: c.text.clone(), call, feature: f.name.clone(), rank, file: f.md_file.clone(), line: c.line });
        }
    }
    Ok(out)
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
fn setters(switches: &[(String, bool)], input: &[u8]) -> Vec<(String, Vec<i64>)> {
    let mut calls: Vec<(String, Vec<i64>)> = switches.iter().map(|(f, on)| (format!("__set___enabled_{}", f), vec![*on as i64])).collect();
    calls.extend(input.iter().map(|&c| ("__in_ch".to_string(), vec![c as i64])));
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
        let same: Vec<usize> = (0..runs.len()).filter(|&j| standing[j] && runs[j].off == runs[i].off && cases[runs[j].case].feature == cases[runs[i].case].feature && cases[runs[j].case].call.func == cases[runs[i].case].call.func && cases[runs[j].case].call.args == cases[runs[i].case].call.args && cases[runs[j].case].call.input == cases[runs[i].case].call.input).collect();
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

/// run one case of a store on the native JIT (log 77): the case line
/// first, the program's output as it lands, then what the case gave
/// after an arrow — the numbers, or the output as the case's quoted
/// text; on the real clock unless `fast`
pub fn run(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, fast: bool) -> Result<String, String> {
    let mut s = store::read(dir).map_err(|e| e.to_string())?;
    s.clock = if fast { store::Clock::Virtual } else { store::Clock::Real };
    let policy = &store_policy(&s, policy);
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let calls = calls_of(&s, &l, policy)?;
    // the case as its line begins, the newest feature's where several
    // begin so, since that is the one that stands with every feature on;
    // or the first case of a function named alone
    let head = |p: &Planned| p.text.split('→').next().unwrap_or("").trim() == which.trim();
    let at = calls.iter().rposition(head).or_else(|| calls.iter().position(|p| p.call.func == which.trim()));
    let mut calls = calls;
    let p = at.map(|i| calls.swap_remove(i)).ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let (text, call) = (p.text.clone(), &p.call);
    let module = build(&l.ir, policy, level)?;
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
    let sc = suite::Call { func: call.func.clone(), args: call.args.clone(), nrets: call.nrets, checks: call.expect == store::Expect::Check, text: true, before: setters(&call.context, &call.input), live: true, times: timed };
    let got = suite::run_calls(&module, &l.ir, Backend::Native, &[sc], "zero-run", level)?.remove(0)?;
    let vals: Vec<String> = got.values.iter().map(|v| v.to_string()).collect();
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
            let policy = store_policy(&s, &policy);
            let l = lower::lower(&s).map_err(|e| e.to_string())?;
            let cases = calls_of(&s, &l, &policy)?;
            let (runs, over) = plan(&s, &cases)?;
            let module = build(&l.ir, &policy, level)?;
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
                    checks: c.expect == store::Expect::Check,
                    // every case reads the text back: a failed check names its site there
                    text: true,
                    before: setters(&r.switches, &c.input),
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
        for (r, got) in runs.iter().zip(got) {
            let text = labelled(&cases[r.case].text, &r.label);
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
    Ok(report)
}

/// did the call give what the case expects? A text result is compared
/// with one trailing newline removed, so `>hi() → "hi"` matches one
/// `print "hi"`
fn judge(expect: &store::Expect, got: Result<suite::Got, String>) -> (bool, String) {
    match (expect, got) {
        (store::Expect::Check, Err(e)) if e.starts_with(suite::checked()) || e.starts_with("trap:") => (true, e.strip_prefix(suite::checked()).map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| format!("({})", s)).unwrap_or_default()),
        (store::Expect::Check, Err(e)) => (false, format!("({})", e)),
        (store::Expect::Check, Ok(g)) => (false, format!("(no check failed; got {})", show(&g.values))),
        (_, Err(e)) => (false, format!("({})", e)),
        (store::Expect::Values(want), Ok(g)) => {
            if &g.values == want {
                (true, String::new())
            } else {
                (false, format!("(got {})", show(&g.values)))
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
        std::fs::write(dir.join("a/a.zero"), "on (int n) = f (int k)\n    n = k\n\non (int n) = g()\n    n = 5\n").unwrap();
        std::fs::write(dir.join("b/b.md"), head("b", "1", "parent: a\n", ">f (2) → 2\n")).unwrap();
        std::fs::write(dir.join("b/b.zero"), "on (int n) = h()\n    n = 6\n").unwrap();
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
        assert_eq!(setters(&base_off.call.context, b"hi"), [("__set___enabled_base".to_string(), vec![0]), ("__in_ch".to_string(), vec![104]), ("__in_ch".to_string(), vec![105])]);
        // a line's switches are made in order, an `on` a call too
        assert_eq!(setters(&[("more".to_string(), false), ("base".to_string(), false), ("base".to_string(), true)], b""), [("__set___enabled_more".to_string(), vec![0]), ("__set___enabled_base".to_string(), vec![0]), ("__set___enabled_base".to_string(), vec![1])]);
        // a case does not stand where its feature is effectively off; a
        // line is a sequence of switches, `on` restoring a flag
        let sequence = Planned { text: String::new(), call: lower::Call { func: "switches".into(), args: vec![], nrets: 2, expect: store::Expect::Values(vec![0, 1]), context: vec![("more".into(), false), ("base".into(), false), ("base".into(), true)], input: vec![] }, feature: "most".into(), rank: 3, file: "most.md".into(), line: 1 };
        assert!(effective(&s, &["base".to_string()].into_iter().collect(), &sequence).is_none());
        assert_eq!(effective(&s, &["tool".to_string()].into_iter().collect(), &sequence).unwrap().iter().cloned().collect::<Vec<_>>(), ["more", "tool"]);
        // a gate reads one field, the feature's effective state, in line
        // through the context (fm3 log 110, 138): `more` is under `base`,
        // so it is `__on_more`, which the setters of `more` and of
        // `base` work out, and no gate has an `and` in it
        assert!(l.ir.contains("fn greet__before_most() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    on: u1 = get _1, __on_more\n    _2: int = if on\n") && !l.ir.contains("__get___enabled"), "{}", l.ir);
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
        assert_eq!(out(), format!("{}{}", on, off));
        // ... and the switch written in the second left the first as it was
        call("__zero_context", &[0]);
        call("run", &[]);
        assert_eq!(out(), format!("{}{}{}", on, off, on));

        // a stream processor's word in progress, a stream's own bit and
        // a variable, each kept across the other context's turn
        let dir = std::env::temp_dir().join(format!("probe-zero-two-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-07T10:00:00\n\n## testing\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "type token =\n    int kind\n    index start, n\n\nchar src$\ntoken u$ = lex(src$)\nint seen$ << 0\n\non (int k) = kind of (char c)\n    k = if (c <= 32) then (0) else (1)\n\non (token t$) << lex (char c$)\n    int k$ = if (empty c$) then (0) else (kind of (c$))\n    bool new$ = k$ == 3 or k$ != k$[-1]\n    index start$ = if (new$) then (position c$) else (start$[-1])\n    index n$ = if (new$) then (1) else (n$[-1] + 1)\n    t$ << token(k$[-1], start$[-1], n$[-1]) if (new$ and k$[-1] != 0)\n\non arrive first()\n    src$ << \"let x = 4\"\n    seen$ << seen$ + 1\n\non arrive again()\n    src$ << \"2;\\n\"\n    end src$\n\non bump()\n    seen$ << seen$ + 1\n\non (int n) = tokens()\n    n = count u$\n\non (int n) = last length()\n    token x = peek u$ at (3)\n    n = x.n\n\non (int n) = bumps()\n    n = seen$\n").unwrap();
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

    #[test]
    fn a_negative_literal_index_is_refused() {
        let dir = std::env::temp_dir().join(format!("probe-zero-negative-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>f() → 6\n").unwrap();
        let with = |line: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) = f()\n    int x$ = [5, 6, 7]\n    advance x$ by (2)\n{}\n", line)).unwrap();
            emit(&dir)
        };
        for (line, said) in [
            ("    n = peek x$ at (-1)", "h.zero:4: 'peek' counts forward from the reader: -1 is behind it"),
            ("    advance x$ by (-2)\n    n = 1", "h.zero:4: 'advance' moves the reader forward: -2 is behind it"),
            ("    int b$ = x$ behind (-1)\n    n = 1", "h.zero:4: 'behind' takes how many items, a count: -1 is behind it"),
        ] {
            let err = with(line).expect_err(line);
            assert!(err.ends_with(said), "{}: {}", line, err);
        }
        // a literal that is not negative, a subscript that is, and an
        // index worked out to be negative all lower as they did
        for line in ["    n = peek x$ at (0)", "    n = x$[-1]", "    index i = 0\n    n = peek x$ at (i - 1)"] {
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
        let head = "int x$\nint d$ = doubled(x$)\nchar t$\nint c$ = codes(t$)\nint s$\nint e$ = doubled(s$)\n\non (int k$) << codes (char c$)\n    k$ << int(c$)\n\non (int n) = f()\n    x$ << 1 << 2\n    t$ << \"abcd\"\n    s$ << 3\n    n = count d$ + count c$ + count e$ + count s$\n\n";
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
        // the literal is a loop that says four, the processor's line in it
        assert!(f.contains(", 4\n") && f.contains(" = load ") && !f.contains("__z2_each(") && f.contains(": int = conv "), "{}", f);
        // `s$` is read by `count s$`: it keeps its queue, and the sink
        // the front end wrote walks it, woken where the push is
        assert!(ir.contains("fn __z3(x: int$, __hz: i64) -> int$\n") && f.contains("= __z3("), "{}", ir);
        for (body, said) in [
            ("    if (x$ > 0)\n        d$ << x$", "h.zero:18: an `if` round a line of a stream processor: every line holds for every item, so the condition goes on the push, `d$ << item if (condition)`, or in the value, `if (c) then (a) else (b)`"),
            ("    int k = x$ * 2\n    d$ << k", "h.zero:18: in a stream processor every line holds for every item, so each line says a stream: write `int k$ = ...`"),
            ("    d$ << x$ while (_ < 5)", "h.zero:18: `while` on a push repeats it, which is a loop, and a stream processor has none: say the stream by a line of its own, or write `d$ << item if (condition)`"),
            ("    e$ << x$", "h.zero:18: a stream processor pushes into its own output, 'd$'"),
            ("    d$ << d$ + x$", "h.zero:18: 'd$' is the output: a stream processor pushes into it and does not read it"),
        ] {
            let err = with(body).expect_err(body);
            assert!(err.ends_with(said), "{}: {}", body, err);
        }
        // run inside a function: not built, and said so
        std::fs::write(dir.join("h/h.zero"), "on (int d$) << doubled (int x$)\n    d$ << x$ * 2\n\non (int n) = f()\n    int i$ = [1, 2, 3]\n    int d$ = doubled(i$)\n    n = count d$\n").unwrap();
        let err = emit(&dir).expect_err("inside a function");
        assert!(err.ends_with("h.zero:6: 'doubled' is a stream processor with no loop in it: it is wired at feature scope, `int x$ = doubled(...)`, and running one inside a function is not built"), "{}", err);
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
        let head = "int x$\nint d$ = made(x$)\n\non (int n) = f()\n    x$ << 1 << 2\n    n = count d$\n\non (int d$) << made (int x$)\n";
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
        assert_eq!(f.matches("= get ").count() - f.matches("get _1, __enabled_h").count() - f.matches(", d\n").count(), 3, "{}", f);
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
            ("    loop\n        if (count x$ == 0)\n            break\n        d$ << x$[-1]\n        advance x$ by (1)", "h.zero:12: this body is written both ways: line 9 walks its input (a loop), and line 12 holds for every item (`x$[-1]`). A stream processor either walks what has arrived, with loops and the reader's words, or says each stream once with no loop: write it one way"),
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
        let ir = with("on (int n) = f (int k)\n    x$ << k\n    p$ << k if (k > 2)\n    n = count d$ + count p$").unwrap();
        assert!(ir.contains("fn __z1_each(_x: int)\n    _this: ptr = context()\n    _1: u1 = cmp.gt _x, 0\n    if _1\n        _2: __ctx = load _this\n        _3: int$ = get _2, d\n        push_queue_open(_3, _x)\n    ret\n"), "{}", ir);
        assert!(ir.contains("    _3: u1 = cmp.gt k, 2\n    if _3\n        _4: __ctx = load _this\n        _5: int$ = get _4, p\n        push_queue_open(_5, k)\n"), "{}", ir);
        for (more, said) in [
            // `if` goes with `while` since fm3 log 148, and comes first
            ("on f (int k)\n    p$ << k while (_ < 3) if (k > 2)", "h.zero:9: `if` comes first on a push, then how often: `x$ << item if (condition) while (...)`"),
            // the word a push took before question 79, after each kind of item
            ("on f (int k)\n    p$ << k when (k > 2)", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            ("on f (int k)\n    p$ << x$ when (k > 2)", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            ("on f (int k)\n    p$ << (k + 1) when (k > 2)", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            ("on f (int k)\n    p$ << twice (k) when (k > 2)\n\non (int n) = twice (int k)\n    n = k * 2", "h.zero:9: `when` is not a word of zero: a push made where a condition holds is `x$ << item if (condition)`"),
            // the push's `if` takes no `then`
            ("on f (int k)\n    p$ << k if (k > 2) then (1) else (2)", "h.zero:9: an `if` after a push's items says whether the push happens, and takes no `then`: the value that is one thing or another is written first, `x$ << if (c) then (a) else (b)`"),
            // a name with a word that ends every phrase (log 126)
            ("on (int n) = one if (int k)\n    n = k", "h.zero:8: 'if' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written"),
            ("on (int n) = lines in any order()\n    n = 1", "h.zero:8: 'in' cannot be a word of a function's name: it ends a phrase wherever it stands, so no call of this name could be written"),
        ] {
            let err = with(more).expect_err(more);
            assert!(err.ends_with(said), "{}: {}", more, err);
        }
        // the three `if`s, told by where the word stands (fm3 log 140):
        // the first of a line is the statement, one where a value is
        // wanted is the expression, one where a value has ended is the
        // push's. `p$ << if (c) then (a) else (b) if (d)` is both
        let both = with("on (int n) = f (int k)\n    p$ << if (k > 5) then (10) else (20) if (k > 2)\n    p$ << if (k > 5) then (10) else k + 1 if (k > 2)\n    if (k > 0)\n        p$ << 1 << if (k > 5) then (2) else (3)\n    n = count p$").unwrap();
        let at = both.find("fn f(k: int) -> int").unwrap();
        let f = &both[at..at + both[at..].find("\n\n").unwrap()];
        // each of the first two lines: the condition, a branch, and
        // under it the value's own branch and one push
        assert_eq!(f.matches("cmp.gt k, 2\n").count(), 2, "{}", f);
        assert_eq!(f.matches("cmp.gt k, 5\n").count(), 3, "{}", f);
        assert_eq!(f.matches("push_queue_open(").count(), 4, "{}", f);
        // `when` is a word of a name where a name is declared with it,
        // and of nothing else: the call and the push's `if` on one line
        let named = with("on (int n) = pushed when (int k)\n    n = k + 1\n\non (int n) = f (int k)\n    p$ << pushed when (k) if (k > 2)\n    n = pushed when (k) + count p$").unwrap();
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
        let f = "\non (int n) = f (int k)\n    a$ << k\n    n = count b$ + count d$\n";
        let with = |lines: &str, body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}{}{}\n", head, lines, f, body)).unwrap();
            emit(&dir)
        };
        // the wiring line and the standing filter: the edge's function
        // of one item, the filter's push under its condition, the
        // source's own name in the condition the item
        let ir = with("b$ << a$ forever\nb$ << i$ << 0 if (i$ > 2) forever\n", "").unwrap();
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
            ("b$ << a$ forever\ni$ << i$ + 1 forever\n", "", "h.zero:10: a stream that feeds itself forever at a rate is a clock (fm3 question 80, ruled): with nothing else on its right the line is paced by its stream's rate, one more item of 'i$' each beat. It is not built: it needs a schedule ordered by time, and a store's clock is still moved by the code that pushes. Until then a function's push says it with an end, `i$ << 0 << (i$ + 1) while (_ < 4)`".to_string()),
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
    /// pushing it (fm3 question 77 (a), log 151): the same function
    /// written with `=` lowers to the same lines; a result with no `$`
    /// makes a plain function of a `<<` declaration; and what is one
    /// value is pushed once, a name that is no result not at all
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
        // each shape the new way and the old: one text
        let pairs = [
            ("on (int d) << double (int x)\n    d << x * 2", "on (int d) = double (int x)\n    d = x * 2"),
            ("on (int s) << sign of (int x)\n    if (x < 0)\n        s << -1\n    else if (x > 0)\n        s << 1", "on (int s) = sign of (int x)\n    if (x < 0)\n        s = -1\n    else if (x > 0)\n        s = 1"),
            ("on (int r) << first (int a) or (int b)\n    if (a > 0)\n        r << a\n    r << b", "on (int r) = first (int a) or (int b)\n    if (a > 0)\n        r = a\n    r = b"),
            ("on (int r) << first (int a) or (int b)\n    r << a if (a > 0)\n    r << b", "on (int r) = first (int a) or (int b)\n    if (a > 0)\n        r = a\n    r = b"),
            ("on (int p) << above (int n)\n    loop (int q = 1)\n        if (q > n)\n            p << q\n        continue (q * 2)", "on (int p) = above (int n)\n    loop (int q = 1)\n        if (q > n)\n            p = q\n        continue (q * 2)"),
            ("on (int g) << gcd of (int a) with (int b)\n    g << loop (int x = a, int y = b) while (y != 0) yields x\n        continue (y, x % y)", "on (int g) = gcd of (int a) with (int b)\n    g = loop (int x = a, int y = b) while (y != 0) yields x\n        continue (y, x % y)"),
            ("on (int q, int r) << divide (int a) by (int b)\n    r << a % b\n    q << a / b\n\non (int q, int r) << both()\n    q, r << divide (17) by (5)", "on (int q, int r) = divide (int a) by (int b)\n    r = a % b\n    q = a / b\n\non (int q, int r) = both()\n    q, r = divide (17) by (5)"),
            ("on (int n) << sum of (int x$)\n    n << x$ + _", "on (int n) = sum of (int x$)\n    n = x$ + _"),
            // either first line over either body, for this landing
            ("on (int d) << double (int x)\n    d = x * 2", "on (int d) = double (int x)\n    d << x * 2"),
        ];
        for (new, old) in pairs {
            let (a, b) = (with(new).unwrap_or_else(|e| panic!("{}: {}", new, e)), with(old).unwrap_or_else(|e| panic!("{}: {}", old, e)));
            assert_eq!(a, b, "{}", new);
        }
        let ir = with(pairs[0].0).unwrap();
        assert!(ir.contains("fn double(x: int) -> int\n    d: int = mul x, 2\n    ret d\n"), "{}", ir);
        // a result nothing pushed is the zero of its type, and the push
        // of the last result ends the function (fm3 question 88)
        let ir = with(pairs[2].0).unwrap();
        assert!(ir.contains("    if _1\n        ret a\n    ret b\n"), "{}", ir);
        let ir = with(pairs[1].0).unwrap();
        assert!(ir.contains("    ret 0\n") || ir.contains("yield 0"), "{}", ir);
        // a `$` on the result is still a task; a function that gives a
        // sequence whole keeps `=` (fm3 question 87)
        let ir = with("on (int i$) << count up to (int n)\n    i$ << 1 << (i$ + 1) while (_ <= n)\n\non (int r$) = squares to (int k)\n    r$ = [1 through k] * [1 through k]").unwrap();
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
            (f("    loop (int i = 0)\n        i << 1\n        y << i"), "h.zero:5: 'i' is the loop's own: it is not pushed into. Give its next value with `continue (...)`"),
            (f("    for (i in [1 through 3])\n        i << 1\n    y << x"), "h.zero:5: 'i' is the item of the `for`: it steps by itself and is not pushed into"),
            (f("    int s$ = [1, 2]\n    s << 3\n    y << x"), "h.zero:5: 's' is written without its `$`: the stream is `s$`, and a push into it is `s$ << ...`"),
            (f("    y << x\n    y << 2"), "h.zero:5: this never runs: the function ended when its result was pushed on line 4"),
            (two("    q << x\n    q << 2\n    r << 1"), "h.zero:3: 'q' is pushed twice on this path: a function gives each of its results once"),
            (two("    if (x > 0)\n        q << x\n    q << 2\n    r << 1"), "h.zero:4: 'q' is pushed twice on this path: a function gives each of its results once"),
            ("on (int y, int z$) << f (int x)\n    y << x".to_string(), "h.zero:1: a task produces one stream"),
        ] {
            let err = with(&text).err().unwrap_or_else(|| panic!("{} compiled", text));
            assert!(err.contains(message), "{}: {}", text, err);
        }
        // the old words, where the old form is written
        let err = with("on (int y) = f (int x)\n    y = x\n    y = 2").unwrap_err();
        assert!(err.contains("h.zero:3: this never runs: the function ended when its result was assigned on line 2"), "{}", err);
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
            std::fs::write(dir.join("h/h.zero"), format!("{}{}\non (int n) = f (int k)\n    x$ << k\n    {}\n", head, lines, f)).unwrap();
            emit(&dir)
        };
        // read only by its name: a cell, the edge's function a load, an
        // add and a store of its field
        let ir = with("sum$ << sum$ + x$ forever\n", "n = sum$").unwrap();
        assert!(ir.contains("fn __edge1(__item: int)\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, sum\n    _3: int = add _2, __item\n    _4: __ctx = load _this\n    _5: __ctx = set _4, sum, _3\n    store _5, _this\n    ret\n"), "{}", ir);
        // wired on, and nothing else pushes into it or names it: the
        // stream has no storage and the line keeps its last item
        let ir = with("sum$ << sum$ + x$ forever\nout$ << sum$ << \"\\n\" forever\n", "n = k").unwrap();
        assert!(ir.contains("    __last1: int\n") && !ir.contains("    sum: int"), "{}", ir);
        assert!(ir.contains("fn __edge1(__item: int)\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, __last1\n    __next: int = add _2, __item\n    _3: __ctx = load _this\n    _4: __ctx = set _3, __last1, __next\n    store _4, _this\n"), "{}", ir);
        // wired on and read by its name as well: a queue, the read
        // guarded for the time before anything is pushed, and the queue
        // not given back under it
        let ir = with("sum$ << sum$ + x$ forever\nout$ << sum$ << \"\\n\" forever\n", "n = sum$").unwrap();
        assert!(ir.contains("    _3: index = received(_2)\n    _4: u1 = cmp.gt _3, 0\n    _5: int = if _4\n        _6: int = latest_queue(_2)\n        yield _6\n    else\n        yield 0\n    _7: int = add _5, __item\n"), "{}", ir);
        assert!(!ir.contains("free_queue("), "{}", ir);
        // its own name later in the chain, another stream pacing: each
        // item, and then the latest, which is that item
        with("sum$ << x$ << sum$ forever\n", "n = count sum$").unwrap();
        // a sum of some; a standing map; and a sum under a count
        let ir = with("sum$ << sum$ + x$ if (x$ % 2 == 0) forever\n", "n = sum$").unwrap();
        assert!(ir.contains("    _1: int = rem __item, 2\n    _2: u1 = cmp.eq _1, 0\n    if _2\n        _3: __ctx = load _this\n        _4: int = get _3, sum\n        _5: int = add _4, __item\n"), "{}", ir);
        let ir = with("sum$ << x$ * 2 forever\n", "n = sum$").unwrap();
        assert!(ir.contains("    _1: int = mul __item, 2\n    _2: __ctx = load _this\n    _3: __ctx = set _2, sum, _1\n"), "{}", ir);
        with("sum$ << sum$ + x$ (3) times\n", "n = sum$").unwrap();
        for (lines, said) in [
            // two other streams: which paces is not settled
            ("sum$ << sum$ + x$ + y$ forever\n", "h.zero:5: 'sum$ << ...' reads 2 streams, 'x$' and 'y$', and which of them sets the line off is not settled (fm3 question 86): an item of either with the other's latest, or one of each together. Not built: say one stream by a line of its own first"),
            // nothing else on its right: never ending, or a clock
            ("y$ << x$ forever\nsum$ << sum$ + 1 forever\n", "h.zero:6: a push into 'sum$' that reads 'sum$' and stands forever would never end: nothing else on its right paces it, and 'sum$' has no rate to. For one more item write it with no `forever`, in a function; for a clock give the stream a rate, `int sum$ at (1 hz)`"),
            ("sum$ << x$ forever\ni$ << i$ + 1 forever\n", "h.zero:6: a stream that feeds itself forever at a rate is a clock (fm3 question 80, ruled): with nothing else on its right the line is paced by its stream's rate, one more item of 'i$' each beat. It is not built: it needs a schedule ordered by time, and a store's clock is still moved by the code that pushes. Until then a function's push says it with an end, `i$ << 0 << (i$ + 1) while (_ < 4)`"),
            // no word: as any line with a stream on its right
            ("sum$ << sum$ + x$\n", "h.zero:5: a push at feature scope happens once, when the store starts (fm3 question 79), and on a line of its own that is not built: a stream's first items go on its declaration, `int sum$ << ...`, and a line that stands is wiring, `sum$ << x$ forever`"),
        ] {
            let err = with(lines, "n = sum$ + count y$").expect_err(lines);
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
        let rest = "\non (int n) = f (int k)\n    a$ << k\n    n = count b$ + count c$ + seen$\n\non g (int k)\n    ";
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
            // item, for `until` and for a count
            ("c$ << a$ if (a$ > 0) until (a$ == 3)\n", "c$ << 1", "h.zero:6: `if` with `until` on a line that stands is not built: 'c$ << a$ if (...) until (...)' could ask its `until` of every item, or only of those that pass".to_string()),
            ("c$ << a$ << 0 until (a$ == 3)\n", "c$ << 1", "h.zero:6: 'c$ << a$ << 0 until (...)' on a line that stands could end the whole line, or repeat its last item: not built. Put a stream between: `first$ << a$ until (...)` and `c$ << first$ << ... forever`".to_string()),
            ("c$ << a$ << 0 (3) times\n", "c$ << 1", "h.zero:6: 'c$ << a$ << 0 (3) times' on a line that stands could be the first 3 items of 'a$', each with what follows it, or every item and what follows it 3 times: not built. For the first, put a stream between: `first$ << a$ (3) times` and `c$ << first$ << ... forever`".to_string()),
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
        std::fs::write(dir.join("h/h.zero"), "int a$\nint d$ = dd(a$)\n\non (int d$) << dd (int x$)\n    d$ << x$ until (_ > 3)\n\non (int n) = f (int k)\n    a$ << k\n    n = count d$\n").unwrap();
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
        let rest = "\non (int n) = twice (int k)\n    n = k * 2\n\non (int n) = three (int k) times\n    n = 3 * k\n\non (int n) = f (int k)\n    a$ << k\n    n = count b$ + count c$\n\non g (int k)\n    ";
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
        assert!(check < first && first < lp && ir[lp..].contains("cmp.lt _10, k"), "{}", ir);
        let ir = g("b$ << k if (k > 0) (2) times");
        assert!(ir.contains("    _1: u1 = cmp.gt k, 0\n    if _1\n") && ir.contains("        loop(_4: int = 0)\n            _5: u1 = cmp.lt _4, 2\n"), "{}", ir);
        // on a declaration, as `while` may be
        assert!(g("int d$ << 0 << (d$ + 1) (4) times").contains("    loop(_2: int = 0)\n        _3: u1 = cmp.lt _2, 4\n"));
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
        std::fs::write(dir.join("h/h.zero"), "int a$\nint d$ = dd(a$)\n\non (int d$) << dd (int x$)\n    d$ << x$ (2) times\n\non (int n) = f (int k)\n    a$ << k\n    n = count d$\n").unwrap();
        let err = emit(&dir).expect_err("a processor");
        assert!(err.ends_with("h.zero:5: `(n) times` on a push repeats it, which is a loop, and a stream processor has none: say the stream by a line of its own, or write `d$ << item if (condition)`"), "{}", err);
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
        let head = "int seen$ << 0\n\non bump()\n    seen$ << seen$ + 1\n\non (int n) = f()\n    bump()\n    int x = seen$ + 1\n    n = x\n";
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
        let said = with("\non (int n) = g()\n    n = latest seen$").unwrap();
        assert!(said.contains("fn g() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    n: int = get _1, seen\n"), "{}", said);
        // one more word, and it is a queue: the name still reads its latest
        let counted = with("\non (int n) = g()\n    n = count seen$").unwrap();
        assert!(counted.contains("    seen: int$\n") && counted.contains("__queue_int"), "{}", counted);
        assert!(counted.contains("_3: int = latest_queue(_2)\n    x: int = add _3, 1\n"), "{}", counted);
        // pushed whole, a once-line that hands over more than one item
        // (question 79's fenced line): not a cell, and it lowers as it did
        let whole = with("\non g()\n    out$ << seen$").unwrap();
        assert!(whole.contains("    seen: int$\n"), "{}", whole);
        // a function that takes the stream whole takes it, as it did
        let taken = with("\non (int n) = total (int x$)\n    n = x$ + _\n\non (int n) = total (int x)\n    n = x\n\non (int n) = g()\n    n = total (seen$)").unwrap();
        assert!(taken.contains("    seen: int$\n"), "{}", taken);
        // ... and where every method takes one value, the name is one
        let one = with("\non (int n) = twice (int x)\n    n = x * 2\n\non (int n) = g()\n    n = twice (seen$)").unwrap();
        assert!(one.contains("    seen: int\n") && one.contains("    _2: int = get _1, seen\n    n: int = twice(_2)\n"), "{}", one);
        // a local stream's name reads its latest where one value is wanted
        let local = with("\non (int n) = g()\n    int i$ << 4 << 5\n    int y = i$ + 1\n    n = y").unwrap();
        assert!(local.contains(" = latest_queue(i)\n    y: int = add "), "{}", local);
        // a cell holds what a ring does not; used as a stream it is refused as it was
        let flag = with("bool up$\n\non (bool b) = g()\n    up$ << true\n    b = up$").unwrap();
        assert!(flag.contains("    up: u1\n"), "{}", flag);
        let err = with("bool up$\n\non (int n) = g()\n    up$ << true\n    n = count up$").expect_err("a stream of bool");
        assert!(err.ends_with("h.zero:10: a stream of bool: a stream holds numbers, enumerations or structs of those"), "{}", err);
        let err = with("\non (int n) = g()\n    int y = out$\n    n = y").expect_err("the device");
        assert!(err.ends_with("h.zero:12: 'out$' is the output device: it is written and never read, so it has no latest item"), "{}", err);
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
        let head = "type Vec =\n    float x, y = 0\n\ngroup int quota = 100 merge sum\nVec origin(1, 2)\nint size\nstring name$ << \"zero\"\n\non (int a, int b) = two()\n    a = 1\n    b = 2\n\n";
        let with = |body: &str| -> Result<String, String> {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int n) = f (int k)\n{}\n", head, body)).unwrap();
            emit(&dir)
        };
        let how = "is a variable, and a variable keeps the value it was declared with (fm3 question 70): what changes is a stream. Declare it";
        for (body, said) in [
            ("    quota = k\n    n = quota", format!("h.zero:14: 'quota' {} `group int quota$ << 100 merge sum` and push its next value, `quota$ << k`; its name, `quota$`, is then its latest item wherever one value is wanted", how)),
            ("    origin = Vec(3, 4)\n    n = k", format!("h.zero:14: 'origin' {} `Vec origin$ << Vec(1, 2)` and push its next value, `origin$ << Vec (3, 4)`; its name, `origin$`, is then its latest item wherever one value is wanted", how)),
            ("    size = size + k\n    n = size", format!("h.zero:14: 'size' {} `int size$` and push its next value, `size$ << ...`; its name, `size$`, is then its latest item wherever one value is wanted", how)),
            ("    size, quota = two()\n    n = size", format!("h.zero:14: 'size' {} `int size$` and push its next value, `size$ << ...`; its name, `size$`, is then its latest item wherever one value is wanted", how)),
            ("    size = loop (int i = 0) while (i < k) yields i\n        continue (i + 1)\n    n = size", format!("h.zero:14: 'size' {} `int size$` and push its next value, `size$ << ...`; its name, `size$`, is then its latest item wherever one value is wanted", how)),
            ("    name$ = \"one\"\n    n = k", "h.zero:14: 'name$' is a stream: it is pushed into, `name$ << \"one\"`, not assigned".to_string()),
            ("    k = k + 1\n    n = k", "h.zero:14: 'k' is a parameter: it is what the function was handed, and is not assigned".to_string()),
            ("    n = count name$", "h.zero:7: a stream of string: a stream holds numbers, enumerations or structs of those".to_string()),
        ] {
            let err = with(body).expect_err(body);
            assert!(err.ends_with(&said), "{}: {}", body, err);
        }
        // the string pushed and written: the field a string variable
        // had, and its name in a push its value now
        let ir = with("    name$ << \"one\"\n    out$ << name$ << \"\\n\"\n    string s = name$\n    n = count s").unwrap();
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
            std::fs::write(dir.join("h/h.zero"), format!("on (int n) = f (int k)\n{}\n", body)).unwrap();
            emit(&dir)
        };
        let ir = with("    n = loop (int i = 0, int m = -1) yields m\n        if (i > k)\n            break\n        if (i * i >= k)\n            break (i)\n        continue (i + 1, -1)").unwrap();
        assert!(ir.contains("        if _1\n            break m\n") && ir.contains("        if _3\n            break i\n"), "{}", ir);
        for (body, said) in [
            ("    n = loop (int i = 0) while (i < k) yields i\n        i = i + 1", "h.zero:3: 'i' is the loop's own: it is not assigned in the loop's body. Give its next value with `continue (...)`, and the loop's result where it leaves with `break (...)`"),
            ("    n = loop (int i = 0, int m = 0) yields m\n        break (i, m)", "h.zero:3: the loop yields 1 name(s), 'break' gives 2"),
            ("    loop (int i = 0)\n        break (i)\n    n = 1", "h.zero:3: the loop yields nothing, and 'break' gives a value: name what comes out with `yields` at the end of the loop's first line"),
            ("    for (i in [1 through k])\n        break (i)\n    n = 1", "h.zero:3: a `for` gives nothing: 'break' takes no values here"),
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
        let head = "int x$\nint d$ = runs(x$)\n\non (bool b) = big (int x)\n    b = x > 100\n\non (int n) = f()\n    x$ << 1 << 1 << 2\n    n = count d$\n\n";
        let with = |body: &str| -> String {
            std::fs::write(dir.join("h/h.zero"), format!("{}on (int d$) << runs (int x$)\n    bool new$ = x$ != x$[-1]\n    int n$ = if (new$) then (1) else (n$[-1] + 1)\n{}\n", head, body)).unwrap();
            let ir = emit(&dir).unwrap();
            let at = ir.find("fn __z1_each").unwrap();
            let rest = &ir[at..];
            rest[..rest.find("\nfn ").unwrap().min(rest.find("\n\n").unwrap_or(rest.len()))].to_string() + "\n"
        };
        // two lines and a push on one condition: one branch, no `and`
        let f = with("    int first$ = if (new$) then (x$) else (first$[-1])\n    d$ << first$[-1] + n$[-1] if (new$ and n$[-1] > 0)");
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
        let code = "char t$\nint d$ = runs(t$)\nint x$\nint e$ = twice(x$)\n\non (int d$) << runs (char c$)\n    int n$ = if (c$ == c$[-1]) then (n$[-1] + 1) else (1)\n    d$ << n$\n\non (int e$) << twice (int x$)\n    e$ << x$ * 2\n\non (int n) = f()\n    int d$ = [7, 8, 9]\n    t$ << \"aab\" << \"bc\"\n    n = count d$\n\non (int n) = g()\n    x$ << 4\n    n = count e$\n\non (int n) = h()\n    int k = f()\n    n = count d$\n";
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
        std::fs::write(dir.join("h/h.zero"), "char t$\n\non (int n) = f()\n    t$ << \"abc\"\n    n = 0\n").unwrap();
        std::fs::write(dir.join("up/up.zero"), "int d$ = codes(t$)\n\non (int k$) << codes (char c$)\n    k$ << int(c$)\n\non (int n) = seen()\n    f()\n    n = count d$\n").unwrap();
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
        let head = "int x$\nint d$ = made(x$)\n\non (int n) = f()\n    x$ << 1 << 2\n    end x$\n    end x$\n    n = count d$\n\n";
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
        let ir = with("    int k$ = if (empty x$) then (7) else (x$ + k$[-1])\n    d$ << k$", "").unwrap();
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
        let ir = with("    d$ << x$\n    d$ << x$[-1] if (empty x$)", "\non (bool b) = done()\n    b = ended d$\n").unwrap();
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
        std::fs::write(dir.join("h/h.zero"), "on (int s) = sum of (int a) and (int b)\n    s = a + b\n\non (int n) = f (int x)\n    bool ok = x > 0 and x < 9 or x == 100 and sum of (x) and (1) > 3\n    n = if (ok) then (1) else (0)\n").unwrap();
        let ir = emit(&dir).unwrap();
        assert!(ir.contains("fn f(x: int) -> int\n    _1: u1 = cmp.gt x, 0\n    _2: u1 = cmp.lt x, 9\n    _3: u1 = and _1, _2\n    _4: u1 = cmp.eq x, 100\n    _5: int = sum_of_and(x, 1)\n    _6: u1 = cmp.gt _5, 3\n    _7: u1 = and _4, _6\n    ok: u1 = or _3, _7\n"), "{}", ir);
        std::fs::write(dir.join("h/h.zero"), "on (int n) = f (int x)\n    n = if (x and x > 2) then (1) else (0)\n").unwrap();
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
        std::fs::write(dir.join("h/h.zero"), "on (int32 w) = width of (int32 x)\n    w = 32\n\non (int32 w) = width of (int64 x)\n    w = 64\n\non (int32 w) = chosen()\n    w = width of (3)\n\non (int32 w) = fwidth of (float32 x)\n    w = 32\n\non (int32 w) = fwidth of (float64 x)\n    w = 64\n\non (int32 w) = fchosen()\n    w = fwidth of (2.5)\n").unwrap();
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
        for (name, parent, code, case) in [("base", "", "on (int v) = value()\n    v = 1\n", ">value() → 1\n"), ("gone", "base", "on (int v) = value()\n    v = existing value() + 1\n", ">value() → 2\n")] {
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
        assert!(!ir.contains("__enabled_base") && !ir.contains("fn __on_base") && ir.contains("fn value__gone() -> int\n    _1: int = value__base()\n"), "{}", ir);
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
        feature("shown", "base", 1, "out$ << n$ << \"\\n\" forever\nout$ << beat$ << \"\\n\" forever\n");
        // wired by a feature that is in the program: an edge each
        let ir = lowered("# p\n").unwrap();
        assert!(ir.contains("fn __edge1(__item: int)") && ir.contains("fn __edge2(__item: int)"), "{}", ir);
        // wired by a feature the product leaves out: no edge, no queue,
        // and the rated stream's step still passes
        let ir = lowered("# p\n\nshown: static off\n").unwrap();
        assert!(!ir.contains("__edge") && !ir.contains("__queue_int") && !ir.contains("\n    n: int$\n"), "{}", ir);
        // ... with no alignment before it, `count` being called where
        // the clock is at 0 s and `n$` having nothing that moves it
        // (question 56, fm3 log 99)
        assert!(ir.contains("    _7: i64 = add _6, 500000\n    __wait(_7)\n    ret\n") && !ir.contains(" = rem "), "{}", ir);
        // wired by no feature at all: refused, naming the stream
        std::fs::write(dir.join("shown/shown.zero"), "out$ << beat$ << \"\\n\" forever\n").unwrap();
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
        feature("shown", "base", 1, "out$ << n$ << \"\\n\" forever\n");
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
        let head = "int a$ at (3 hz)\nint b$ at (5 hz)\nint c$ at (7 hz)\nout$ << a$ << \"\\n\" forever\nout$ << b$ << \"\\n\" forever\nout$ << c$ << \"\\n\" forever\n\non g()\n    c$ << 0\n    f()\n\n";
        // one statement of three items: one alignment, three steps, the
        // slot a whole number of the period a step adds
        let ir = lowered(&format!("{}on f()\n    a$ << 1 << 2 << 3\n", head));
        let f = body(&ir, "f");
        assert!(f.contains("    _3: i64 = add _2, 333332\n    _4: i64 = rem _3, 333333\n    _5: i64 = sub _3, _4\n    __wait(_5)\n"), "{}", f);
        assert_eq!((f.matches(" = rem ").count(), f.matches(", 333333\n").count()), (1, 4), "{}", f);
        // the statement before pushed into the same stream: on the beat already
        let f = body(&lowered(&format!("{}on f()\n    a$ << 1\n    a$ << 2\n    a$ << 3\n", head)), "f");
        assert_eq!(f.matches(" = rem ").count(), 1, "{}", f);
        // by turns into two streams: each statement finds its own stream's slot
        let f = body(&lowered(&format!("{}on f()\n    a$ << 1\n    b$ << 2\n    a$ << 3\n", head)), "f");
        assert_eq!((f.matches(" = rem ").count(), f.matches("rem _3, 333333").count(), f.matches(", 200000\n").count()), (3, 1, 2), "{}", f);
        // a write to the device between two pushes moves no clock
        // (question 56, fm3 log 99), so the second is still on the beat;
        // a push into another stream between them does
        let f = body(&lowered(&format!("{}on f()\n    a$ << 1\n    out$ << \"x\"\n    a$ << 2\n", head)), "f");
        assert_eq!(f.matches(" = rem ").count(), 1, "{}", f);
        // a stream with no rate has no beat
        let f = body(&lowered("int c$\nout$ << c$ << \"\\n\" forever\n\non f()\n    c$ << 1\n"), "f");
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
        let head = "int a$ at (2 hz)\nint b$ at (1 hz)\nint c$ at (5 hz)\nout$ << a$ << \"\\n\" forever\nout$ << b$ << \"\\n\" forever\nout$ << c$ << \"\\n\" forever\n\n";
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
        assert!(f.contains("    _7: int = const 1\n    _8: u1 = cmp.le _7, k\n    if _8\n        _9: ptr = addr __clock\n        _10: i64 = load _9\n        _11: i64 = add _10, 499999\n        _12: i64 = rem _11, 500000\n        _13: i64 = sub _11, _12\n        __wait(_13)\n    loop(i: int = 1)\n"), "{}", f);
        assert_eq!(f.matches(" = rem ").count(), 1, "{}", f);
        let ir = lowered(&format!("{}on f (int k)\n    loop (int i = 1) while (i <= k)\n        a$ << i\n        continue (i + 1)\n", head));
        assert_eq!(rems(&ir, "f"), 0, "{}", ir);
        // ... with no `while` the first pass always runs, and there is no test
        let ir = lowered(&format!("{}on f (int k)\n    c$ << 0\n    loop (int i = 1)\n        a$ << i\n        if (i == k)\n            break\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches(" = rem ").count() == 1 && f.contains("\n    _10: i64 = rem _9, 500000\n    _11: i64 = sub _9, _10\n    __wait(_11)\n    loop(i: int = 1)\n"), "{}", f);
        // a statement before the push in the body, or a way round that
        // leaves the beat, and the push aligns on every pass as it did
        let ir = lowered(&format!("{}on f (int k)\n    c$ << 0\n    loop (int i = 1) while (i <= k)\n        out$ << \"x\"\n        a$ << i\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches(" = rem ").count() == 1 && f.find("loop(").unwrap() < f.find(" = rem ").unwrap(), "{}", f);
        let ir = lowered(&format!("{}on f (int k)\n    loop (int i = 1) while (i <= k)\n        a$ << i\n        c$ << i\n        continue (i + 1)\n", head));
        let f = body(&ir, "f");
        assert!(f.matches(" = rem ").count() == 2 && f.find("loop(").unwrap() < f.find(" = rem ").unwrap(), "{}", f);
        // an edge that can move the clock: the statement leaves nothing known
        let ir = lowered("int a$ at (2 hz)\nint b$ at (5 hz)\nb$ << a$ forever\nout$ << b$ << \"\\n\" forever\n\non f()\n    a$ << 1\n    a$ << 2\n    out$ << \"x\"\n    a$ << 3\n");
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
        std::fs::write(dir.join("h/h.zero"), "int i$ at (2 hz)\nout$ << i$ << \"\\n\" forever\n\non run()\n    i$ << [1 through 2]\n").unwrap();
        let with = |product: &str| -> Result<String, String> {
            std::fs::write(dir.join("product.md"), product).unwrap();
            let s = store::read(&dir).map_err(|e| e.to_string())?;
            Ok(lower::lower(&s).map_err(|e| e.to_string())?.ir)
        };
        let real = with("# p\n\nclock: real\n").unwrap();
        assert!(real.contains("fn __counter() -> i64\n") && real.contains("platform arm64\n    __counter() -> i64\n        mrs r, cntpct_el0\n"), "{}", real);
        assert!(real.contains("fn __wait(t: i64)\n    loop()\n        r: i64 = __real_now()\n"), "{}", real);
        assert!(real.contains("c0: i64 = __counter()\n    q: ptr = addr __base\n    store c0, q\n"), "{}", real);
        let fast = with("# p\n\nclock: virtual\n").unwrap();
        assert!(!fast.contains("__counter") && fast.contains("fn __wait(t: i64)\n    p: ptr = addr __clock\n    c: i64 = load p\n    m: i64 = max(c, t)\n"), "{}", fast);
        // the rated stream no word reads has no storage (fm3 log 92): the
        // push calls its edge, and then a step passes, half a second at 2 hz
        assert!(fast.contains("        if _2\n            __edge1(_3)\n        _5: ptr = addr __clock\n        _6: i64 = load _5\n        _7: i64 = add _6, 500000\n        __wait(_7)\n"), "{}", fast);
        // ... and the statement's first item is on the stream's beat
        // with nothing rounded (fm3 log 98, 99): `run` is only ever
        // called by a case, at 0 s
        assert!(fast.contains("fn run()\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u1 = get _1, __enabled_h\n") && !fast.contains(", 499999\n"), "{}", fast);
        assert!(fast.contains("fn __edge1(__item: int)\n") && !fast.contains("__run") && !fast.contains("__node") && !fast.contains("\n    i: int$\n"), "{}", fast);
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
        assert!(refused("on (char o$) = (char o$) << (int x)\n    o$ << \"?\"\n\non f()\n    out$ << 1\n").contains("unexpected '<<' in a function's name"));
        assert!(refused("on f()\n    int i$ << 1\n    i$ << 2.5\n").contains("'i$' holds int but the item is float"));
        assert!(refused("type token =\n    int kind, start, n\n\non f()\n    token t$ << token(1, 2, 3)\n    out$ << t$\n").contains("'out$' holds char but the item is token$: no `<<` method takes it"));
        // a char is a character, not a small number (question 44)
        assert!(refused("on f()\n    char c = char(65)\n    out$ << (c + 1)\n").contains("'+' on a char: a char is compared, not computed with; convert it, `int(c)`"));
        // a `uint8` stream takes the byte itself, since no method takes one
        let bytes = emit_with("on (int n) = f()\n    uint8 b$ = \"hi\"\n    b$ << 33\n    n = count b$\n").unwrap();
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
        let ir = emit_with(&format!("{}on (int n) = size (char c$)\n    n = count c$\n\non (int n) = f()\n    n = size(in$)\n\non (int n) = g()\n    char in$ << \"ab\"\n    fill(in$)\n    n = count in$\n", fill)).unwrap();
        assert!(ir.contains("fn f() -> int\n    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: u8$ = get _1, in\n    n: int = size(_2)\n"), "{}", ir);
        // reading the device is what it is for; and a stream of the
        // function's own that happens to be called `in` is not the device
        let ir = emit_with("on (int n) = f()\n    n = count in$\n\non (int n) = g()\n    char in$ << \"ab\"\n    in$ << \"c\"\n    n = count in$\n").unwrap();
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
        let fed = "on (int n) = fed()\n    a$ << 1\n    n = count d$\n";
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
        assert!(ir.contains("\n    __node1_x: index\n") && ir.contains("    _4: index = get _2, pos\n    _5: __ctx = pack 1, 1, _1, _2, _3, _4\n"), "{}", ir);
        // a task that may give back a reader on another ring keeps its
        // whole reader: one that assigns its parameter, declares the
        // name again, loops over it, hands it to a function or runs a
        // task over it, or reads it by a word the rule does not know
        for (body, whole) in [
            ("    loop\n        if (count x$ == 0)\n            break\n        d$ << peek x$ at (0)\n        advance x$ by (1)\n", false),
            ("    d$ << count x$ << position x$\n    int f$ = frame x$\n    if (ended x$)\n        d$ << latest f$\n", false),
            ("    d$ << size(x$)\n    advance x$ by (count x$)\n", true),
            ("    for (v in x$)\n        d$ << v\n", true),
            ("    d$ << doubled(x$)\n", true),
            ("    int h$ = x$ behind (1)\n    advance x$ by (count x$)\n", true),
        ] {
            std::fs::write(dir.join("h/h.zero"), format!("int a$\nint d$ = moved(a$)\nint far$\n\non (int n) = size (int s$)\n    n = count s$\n\n{}on (int d$) << moved (int x$)\n{}\n{}", task, body, fed)).unwrap();
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
        let ir = emit_with("int a$\nint d$ = doubled(a$)\nint y$\nint e$ = feeder(y$)\n", "\non (int k) = bump (int v)\n    a$ << v\n    k = v\n\non (int e$) << feeder (int y$)\n    e$ << bump (1)\n    advance y$ by (count y$)\n");
        assert!(node(&ir) && ir.contains("data __running") && ir.contains("fn __run_a()\n    p: ptr = addr __running\n"), "{}", ir);
        // items on its declaration: it holds something at the start
        let ir = emit_with("int a$ << 1 << 2\nint d$ = doubled(a$)\n", "");
        assert!(node(&ir), "{}", ir);
        // wired at a rate
        let ir = emit_with("int a$\nint d$ = doubled(a$) at (2 hz)\n", "");
        assert!(node(&ir), "{}", ir);
        // a `platform` body of the store's own is not read
        let ir = emit_with(wired, "\non (int64 r) = (int64 a) twice\nplatform ir\n    r: i64 = add a, a\n    ret r\n\non (int64 n) = two()\n    n = (1) twice\n");
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
        let fns = "int a$\nint64 w$\nuint16 u$\nchar c$\nuint8 b$\n\non (int n) = fa()\n    a$ << 1\n    n = count a$\n\non (int n) = fw()\n    w$ << 1\n    n = count w$\n\non (int n) = fu()\n    u$ << 1\n    n = count u$\n\non (int n) = fc()\n    c$ << \"sixteen letters!\"\n    n = count c$\n\non (int n) = fb()\n    b$ << 1\n    n = count b$\n";
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
        assert_eq!(words("\non (int n) = made()\n    uint16 l$ << 1\n    end l$\n    n = count l$\n"), [false, false, true, false, false]);
        // a `platform` body of the store's own may end anything
        assert_eq!(words("\non (int64 r) = (int64 a) twice\nplatform ir\n    r: i64 = add a, a\n    ret r\n\non (int64 n) = two()\n    n = (1) twice\n"), [true, true, true, true, true]);
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
            std::fs::write(dir.join("h/h.zero"), format!("{}\n\non (int n) = f()\n    c$ << \"{}\"\n    n = count c$\n", decls, text)).unwrap();
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
        let b = body("char c$\nint h$\n\non (int k) = g()\n    h$ << 1\n    k = latest h$ + count h$", "fifteen letters");
        assert!(!b.contains("push_queue") && b.contains("__str"), "{}", b);
        // a stream with a rate takes it an item at a time, each at its time
        let b = body("char c$ at (2 hz)", "ab");
        assert!(!b.contains("push_queue_few"), "{}", b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a field of the context nothing in the store writes is fetched
    /// once a function, and one that something writes is read wherever
    /// it is named (fm3 log 110)
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
        let b = body(&format!("{}on (int n) = f()\n    int a = kept + moved$\n    bump()\n    n = a + kept + moved$\n", head), "f");
        assert!(b.starts_with("    _this: ptr = context()\n    _1: __ctx = load _this\n    _2: int = get _1, kept\n") && b.matches("= context()").count() == 1, "{}", b);
        assert_eq!((reads(&b, "kept"), reads(&b, "moved")), (1, 2), "{}", b);
        // ... the written one twice even with nothing between that writes it
        let b = body(&format!("{}on (int n) = f()\n    int a = moved$\n    idle()\n    n = a + moved$\n", head), "f");
        assert_eq!(reads(&b, "moved"), 2, "{}", b);
        // a read in one arm is not in hand in the other, nor after them;
        // one above them is in hand in both, and inside a loop
        let b = body(&format!("{}on (int n) = f (int k)\n    if (k > 0)\n        n = kept\n    else\n        n = kept + 1\n", head), "f");
        assert_eq!(reads(&b, "kept"), 2, "{}", b);
        let b = body(&format!("{}on (int n) = f (int k)\n    int a = 0\n    if (k > 0)\n        a = kept\n    n = a + kept\n", head), "f");
        assert_eq!(reads(&b, "kept"), 2, "{}", b);
        let b = body(&format!("{}on (int n) = f (int k)\n    int a = kept\n    int s = loop (int i = 0, int t = 0) while (i < k) yields t\n        continue (i + 1, if (i > 2) then (t + kept) else (t + kept + a))\n    n = s + kept\n", head), "f");
        assert_eq!(reads(&b, "kept"), 1, "{}", b);
        // a feature's switch is a field nothing writes
        let b = body(&format!("{}on (bool b) = f()\n    bool a = enabled\n    bump()\n    b = a == enabled\n", head), "f");
        assert_eq!(reads(&b, "__enabled_h"), 1, "{}", b);
        // a stream's field is not written by a push into the stream, and
        // is by a word that moves the feature's reader
        let streams = "int q$\nint r$\n\non fill()\n    q$ << 1\n    r$ << 1\n\non skip()\n    advance r$ by (1)\n\n";
        let b = body(&format!("{}on (int n) = f()\n    int a = count q$ + count r$\n    fill()\n    skip()\n    n = a + count q$ + count r$\n", streams), "f");
        assert_eq!((reads(&b, "q"), reads(&b, "r")), (1, 2), "{}", b);
        // a `platform` body of the store's own may call a setter: nothing is reused
        let b = body(&format!("{}on (int64 r) = (int64 a) twice\nplatform ir\n    r: i64 = add a, a\n    ret r\n\non (int n) = f()\n    int a = kept\n    bump()\n    n = a + kept\n", head), "f");
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
        assert!(!frees("x", "\non (int n) = first()\n    n = peek x$ at (0)\n"));
        assert!(!frees("x", "\non (int n) = size (int s$)\n    n = peek s$ at (0)\n\non (int n) = sized()\n    n = size(x$)\n"));
        // one that only counts it reads no item (question 48)
        assert!(frees("x", "\non (int n) = waiting()\n    n = count x$\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// a second `count` of the same reader value is the first's number
    /// where nothing between could have pushed, and only there (fm3 log 112)
    #[test]
    fn count_is_asked_once_where_nothing_between_could_push() {
        let dir = std::env::temp_dir().join(format!("probe-zero-counts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-10-06T10:00:00\n\n## testing\n").unwrap();
        let head = "int far$\n\non (int k) = quiet (int c)\n    k = c + 1\n\non (int k) = loud (int c)\n    far$ << c\n    k = c\n\non (int k) = relayed (int c)\n    k = loud (c)\n\non (int n) = sized()\n    n = count far$\n\n";
        let counts = |f: &str| -> usize {
            std::fs::write(dir.join("h/h.zero"), format!("{}{}", head, f)).unwrap();
            let ir = lower::lower(&store::read(&dir).unwrap()).unwrap().ir;
            let b: Vec<&str> = ir.lines().skip_while(|l| !l.starts_with("fn f(")).skip(1).take_while(|l| l.starts_with(' ')).collect();
            assert!(b.iter().any(|l| l.contains(" = count ")), "{}", ir);
            b.iter().filter(|l| l.contains(" = count ")).count()
        };
        let straight = |between: &str| format!("on (int n) = f()\n    int s$ << 1 << 2\n    int a = count s$\n{}    n = a + count s$\n", between);
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
        let turn = |arm: &str| format!("on (int n) = f (int k)\n    int s$ << 1 << 2\n    n = loop (int i = 0, int acc = 0) yields acc\n        if (i >= k)\n            break\n        int a = count s$\n        if (a > 5)\n{}        continue (i + 1, acc + a + count s$)\n", arm);
        assert_eq!(counts(&turn("            s$ << 9\n            continue (i + 1, acc)\n")), 1);
        // ... one that pushes and goes on is not, nor one nested deeper
        assert_eq!(counts(&turn("            s$ << 9\n")), 2);
        assert_eq!(counts(&turn("            if (a > 6)\n                s$ << 9\n                continue (i + 1, acc)\n")), 2);
        // the second asking in a loop the first is not in: the loop's
        // whole body is between, what follows the asking too
        let inner = |after: &str| format!("on (int n) = f (int k)\n    int s$ << 1 << 2\n    int a = count s$\n    int t = loop (int i = 0, int acc = 0) yields acc\n        if (i >= k)\n            break\n        int c = count s$\n{}        continue (i + 1, acc + c)\n    n = a + t\n", after);
        assert_eq!(counts(&inner("")), 1);
        assert_eq!(counts(&inner("        int q = quiet (c)\n")), 1);
        assert_eq!(counts(&inner("        s$ << 9\n")), 2);
        // an asking in one arm is not in hand in the other
        assert_eq!(counts("on (int n) = f (int k)\n    int s$ << 1 << 2\n    if (k > 0)\n        n = count s$\n    else\n        n = count s$ + 1\n"), 2);
        let _ = std::fs::remove_dir_all(&dir);
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
}
