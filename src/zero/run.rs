//! The zero runner: `probe zero <store> emit`, `probe zero <store> run
//! <case>`, and `probe zero test [dir] [wasm]`. A store is lowered once
//! to IR text, the text goes through probe's parser and everything after
//! it — the IR is the oracle for the front end — and the cases from the
//! features' `## testing` sections run on the chosen path through the
//! suite's own machinery (`suite::run_calls`).

use super::{lower, store};
use crate::suite::{self, Backend, Report};
use crate::{opt, ssa};
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

/// the calls a store's cases make, in the order the features compose
fn calls_of(s: &store::Store, l: &lower::Lowered) -> Result<Vec<(String, lower::Call)>, String> {
    let mut out = Vec::new();
    for f in &s.features {
        for c in &f.cases {
            let call = lower::resolve_case(l, c, &f.md_file).map_err(|e| e.to_string())?;
            out.push((c.text.clone(), call));
        }
    }
    Ok(out)
}

/// run one case of a store on the native JIT and print what it gave
pub fn run(dir: &Path, which: &str, policy: &ssa::Policy, level: usize) -> Result<String, String> {
    let s = store::read(dir).map_err(|e| e.to_string())?;
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let calls = calls_of(&s, &l)?;
    let (text, call) = calls
        .into_iter()
        .find(|(t, c)| t.split('→').next().unwrap_or("").trim() == which.trim() || c.func == which.trim())
        .ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let module = build(&l.ir, policy, level)?;
    let sc = suite::Call { func: call.func.clone(), args: call.args.clone(), nrets: call.nrets, checks: false, text: true };
    let got = suite::run_calls(&module, Backend::Native, &[sc], "zero-run")?.remove(0)?;
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
    for sdir in &stores {
        let name = sdir.file_name().unwrap().to_string_lossy().to_string();
        let result = (|| -> Result<(Vec<(String, lower::Call)>, ssa::Module), String> {
            let s = store::read(sdir).map_err(|e| e.to_string())?;
            let l = lower::lower(&s).map_err(|e| e.to_string())?;
            let calls = calls_of(&s, &l)?;
            let module = build(&l.ir, &policy, level)?;
            Ok((calls, module))
        })();
        let (calls, module) = match result {
            Ok(r) => r,
            Err(e) => {
                report.failed += 1;
                report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
                continue;
            }
        };
        let scalls: Vec<suite::Call> = calls
            .iter()
            .map(|(_, c)| suite::Call {
                func: c.func.clone(),
                args: c.args.clone(),
                nrets: c.nrets,
                checks: c.expect == store::Expect::Check,
                text: matches!(c.expect, store::Expect::Text(_)),
            })
            .collect();
        let got = match suite::run_calls(&module, backend, &scalls, &name) {
            Ok(g) => g,
            Err(e) => {
                report.failed += calls.len().max(1);
                report.log.push_str(&format!("FAIL  {:<16} {}\n", name, e));
                continue;
            }
        };
        for ((text, call), got) in calls.iter().zip(got) {
            let (ok, note) = judge(&call.expect, got);
            report.case(ok, &name, text, &note);
        }
    }
    report.log.push_str(&format!("\n{}/{} cases passed\n", report.passed, report.passed + report.failed));
    Ok(report)
}

/// did the call give what the case expects? A text result is compared
/// with one trailing newline removed, so `>hi() → "hi"` matches one
/// `print "hi"`
fn judge(expect: &store::Expect, got: Result<suite::Got, String>) -> (bool, String) {
    match (expect, got) {
        (store::Expect::Check, Err(e)) if e == suite::checked() || e.starts_with("trap:") => (true, String::new()),
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
