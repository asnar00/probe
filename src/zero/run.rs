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

/// lower a store and print its IR
pub fn emit(dir: &Path) -> Result<String, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    Ok(l.ir)
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

/// the calls a store's cases make, in the order the features compose
fn calls_of(s: &store::Store, l: &lower::Lowered) -> Result<Vec<Planned>, String> {
    let mut out = Vec::new();
    for (rank, f) in s.features.iter().enumerate() {
        for c in &f.cases {
            let call = lower::resolve_case(l, c, &f.md_file).map_err(|e| e.to_string())?;
            out.push(Planned { text: c.text.clone(), call, feature: f.name.clone(), rank, file: f.md_file.clone(), line: c.line });
        }
    }
    Ok(out)
}

/// The contexts a store's cases run in (section 14, log 44): every
/// feature on, then each feature of the store off alone — with its
/// descendants, since a feature off takes its subtree with it — as a
/// label and the set of features off. The compiler's own `platform`
/// feature is never switched
fn contexts(s: &store::Store) -> Vec<(String, BTreeSet<String>)> {
    let mut out = vec![(String::new(), BTreeSet::new())];
    for f in &s.features {
        if f.name != "platform" {
            out.push((format!("with {} off", f.name), s.subtree(&f.name).into_iter().collect()));
        }
    }
    out
}

/// The features off when a case runs in the runner's context `x`: `x`
/// with the case's own line applied on top, each `off` taking its
/// subtree. None when the case does not stand there: its own feature
/// is off in `x`, or its line pins `on` a feature `x` switches off. A
/// line that pins `on` a feature its own `off` covers contradicts
/// itself and is refused
fn effective(s: &store::Store, x: &BTreeSet<String>, p: &Planned) -> Result<Option<BTreeSet<String>>, String> {
    if x.contains(&p.feature) {
        return Ok(None);
    }
    let mut off = x.clone();
    for (name, on) in &p.call.context {
        if *on {
            if x.contains(name) {
                return Ok(None);
            }
            if off.contains(name) {
                return Err(format!("{}:{}: `with {} on` contradicts an `off` on the same line that switches it off", p.file, p.line, name));
            }
        } else {
            off.extend(s.subtree(name));
        }
    }
    Ok(Some(off))
}

/// the calls that build a context before a case: each feature off, its
/// `enabled` set to 0 (everything is on after the reset, log 43)
fn setters(off: &BTreeSet<String>) -> Vec<(String, Vec<i64>)> {
    off.iter().map(|f| (format!("__set___enabled_{}", f), vec![0])).collect()
}

/// a case overridden in one context: the case with the runner's label,
/// the context, and the feature and case that stand instead
struct Over {
    text: String,
    context: String,
    by_feature: String,
    by_text: String,
}

/// one run of a case: in which effective context, under which of the
/// runner's labels
struct Run {
    case: usize,
    off: BTreeSet<String>,
    label: String,
}

/// Every run a store's cases make (log 44): each case once per
/// distinct effective context it stands in; and, where two cases in one
/// effective context make the same call — the same function and
/// arguments — the newest feature's stands, then the one whose line
/// names more features, and the others are overridden there. Gives the
/// standing runs and a report line per override
fn plan(s: &store::Store, cases: &[Planned]) -> Result<(Vec<Run>, Vec<Over>), String> {
    let mut runs: Vec<Run> = Vec::new();
    let mut seen: HashSet<(usize, BTreeSet<String>)> = HashSet::new();
    for (label, x) in contexts(s) {
        for (i, p) in cases.iter().enumerate() {
            let Some(off) = effective(s, &x, p)? else { continue };
            if seen.insert((i, off.clone())) {
                runs.push(Run { case: i, off, label: label.clone() });
            }
        }
    }
    let mut standing = vec![true; runs.len()];
    let mut over = Vec::new();
    for i in 0..runs.len() {
        if !standing[i] {
            continue;
        }
        let same: Vec<usize> = (0..runs.len()).filter(|&j| standing[j] && runs[j].off == runs[i].off && cases[runs[j].case].call.func == cases[runs[i].case].call.func && cases[runs[j].case].call.args == cases[runs[i].case].call.args).collect();
        if same.len() < 2 {
            continue;
        }
        let weight = |j: usize| (cases[runs[j].case].rank, cases[runs[j].case].call.context.len());
        let best = *same.iter().max_by_key(|&&j| weight(j)).unwrap();
        if let Some(&tie) = same.iter().find(|&&j| j != best && weight(j) == weight(best)) {
            let (a, b) = (&cases[runs[best].case], &cases[runs[tie].case]);
            return Err(format!("{}:{} and line {} both claim `{}` in one context ({}): one case per call per context", a.file, a.line, b.line, a.text.split('→').next().unwrap_or("").trim(), describe(&runs[best].off)));
        }
        for &j in &same {
            if j != best {
                standing[j] = false;
                let (o, w) = (&cases[runs[j].case], &cases[runs[best].case]);
                over.push(Over { text: labelled(&o.text, &runs[j].label), context: describe(&runs[j].off), by_feature: w.feature.clone(), by_text: w.text.clone() });
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

/// run one case of a store on the native JIT and print what it gave
pub fn run(dir: &Path, which: &str, policy: &ssa::Policy, level: usize) -> Result<String, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let calls = calls_of(&s, &l)?;
    let p = calls
        .into_iter()
        .find(|p| p.text.split('→').next().unwrap_or("").trim() == which.trim() || p.call.func == which.trim())
        .ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let (text, call) = (p.text.clone(), &p.call);
    let module = build(&l.ir, policy, level)?;
    let kind = kind_of(Backend::Native);
    if let Some(why) = out_of_reach(&module, &l.funcs, kind).get(&call.func) {
        return Err(format!("{} is out of reach here: {}", text.split('→').next().unwrap_or("").trim(), skip_note(&l.funcs, why, kind)));
    }
    let off = effective(&s, &BTreeSet::new(), &p)?.unwrap_or_default();
    let sc = suite::Call { func: call.func.clone(), args: call.args.clone(), nrets: call.nrets, checks: call.expect == store::Expect::Check, text: true, before: setters(&off) };
    let got = suite::run_calls(&module, &l.ir, Backend::Native, &[sc], "zero-run", level)?.remove(0)?;
    let vals: Vec<String> = got.values.iter().map(|v| v.to_string()).collect();
    let mut out = String::new();
    out.push_str(&got.text);
    out.push_str(&format!("{} → {}\n", text.split('→').next().unwrap_or("").trim(), vals.join(", ")));
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
            let s = store::read(sdir).map_err(|e| e.to_string())?;
            let l = lower::lower(&s).map_err(|e| e.to_string())?;
            let cases = calls_of(&s, &l)?;
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
            report.log.push_str(&format!("over  {:<16} {} ({}): overridden by {}'s `{}`\n", name, o.text, o.context, o.by_feature, o.by_text));
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
                    before: setters(&r.off),
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
    }
}

fn show(vals: &[i64]) -> String {
    let v: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
    v.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the contexts a store's cases run in and the overrides among them
    /// (log 44): hello's three `run()` cases are one promise per
    /// context, the newest feature's standing; a feature off takes its
    /// subtree with it; a line pinning `on` a feature the runner
    /// switches off does not run there
    #[test]
    fn every_case_in_every_context() {
        let s = store::read(Path::new("suite/zero/hello")).unwrap();
        let l = lower::lower(&s).unwrap();
        let cases = calls_of(&s, &l).unwrap();
        let (runs, over) = plan(&s, &cases).unwrap();
        let labels: Vec<String> = contexts(&s).into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["", "with hello off", "with countdown off", "with bye off"]);
        // hello off takes countdown and bye with it: nothing stands there
        assert!(s.subtree("hello").contains(&"bye".to_string()));
        assert!(runs.iter().all(|r| r.label != "with hello off"));
        let standing = |label: &str| -> Vec<String> { runs.iter().filter(|r| r.label == label).map(|r| cases[r.case].text.clone()).collect() };
        assert_eq!(standing(""), ["hello() → \"hello world\"", "count down() → \"10 9 8 7 6 5 4 3 2 1\"", "run() → \"10 9 8 7 6 5 4 3 2 1\\nhello world\\ngoodbye\"", "run() with countdown off → \"hello world\\ngoodbye\""]);
        assert_eq!(standing("with bye off"), ["hello() → \"hello world\"", "count down() → \"10 9 8 7 6 5 4 3 2 1\"", "run() → \"10 9 8 7 6 5 4 3 2 1\\nhello world\""]);
        assert_eq!(standing("with countdown off"), ["hello() → \"hello world\""]);
        assert_eq!(over.len(), 5);
        assert!(over.iter().any(|o| o.text == "run() → \"hello world\" [with bye off]" && o.by_feature == "countdown"), "{:?}", over.iter().map(|o| &o.text).collect::<Vec<_>>());
        // a line's `off` takes the subtree; an `on` the runner contradicts does not stand
        let s = store::read(Path::new("suite/zero/features")).unwrap();
        let l = lower::lower(&s).unwrap();
        let cases = calls_of(&s, &l).unwrap();
        let base_off = cases.iter().find(|p| p.text.starts_with("greeted() with base off")).unwrap();
        let off = effective(&s, &BTreeSet::new(), base_off).unwrap().unwrap();
        assert_eq!(off.iter().cloned().collect::<Vec<_>>(), ["base", "more", "most", "tool"]);
        assert_eq!(setters(&off)[0], ("__set___enabled_base".to_string(), vec![0]));
        let pinned = Planned { text: String::new(), call: lower::Call { func: "greeted".into(), args: vec![], nrets: 1, expect: store::Expect::Values(vec![1]), context: vec![("tool".into(), true)] }, feature: "most".into(), rank: 3, file: "most.md".into(), line: 1 };
        assert!(effective(&s, &s.subtree("tool").into_iter().collect(), &pinned).unwrap().is_none());
        assert!(effective(&s, &s.subtree("more").into_iter().collect(), &pinned).unwrap().is_some());
        let contradiction = Planned { call: lower::Call { context: vec![("base".into(), false), ("more".into(), true)], ..pinned.call }, ..pinned };
        assert!(effective(&s, &BTreeSet::new(), &contradiction).unwrap_err().contains("`with more on` contradicts"));
    }

    /// a product's bound (log 41) reaches every loop of the function it
    /// names, marked as the product's, and `probe cost` counts it
    #[test]
    fn a_product_bound_reaches_the_loops() {
        let ir = emit(Path::new("suite/zero/tasks")).unwrap();
        assert!(ir.contains("; product setting: bound count down from: 5\nfn count_down_from("), "{}", ir);
        let body: String = ir.lines().skip_while(|l| !l.starts_with("fn count_down_from(")).take_while(|l| *l != "}").collect::<Vec<_>>().join("\n");
        assert!(body.contains("loop() bound 5 {"), "{}", body);
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
