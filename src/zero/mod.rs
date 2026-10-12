//! The zero language's front end: a store of features (`store.rs`) is
//! lexed (`lex.rs`) and parsed (`syntax.rs`) into a small tree per
//! feature, lowered to IR text (`lower.rs`), and run through the rest
//! of probe by the runner (`run.rs`). zero.md in the fm3 project is the
//! language definition; the front end adds no semantics of its own.

pub mod kinds;
pub mod lex;
pub mod lower;
pub mod meter;
pub mod places;
pub mod run;
pub mod store;
pub mod syntax;
pub mod trace;
pub mod turns;
pub mod zeroic;

use crate::{ssa, suite};
use std::path::Path;
use std::process::ExitCode;

/// `probe zero <store> emit`, `probe zero <store> run <case>`,
/// `probe zero <store> trace <case>`, `probe zero test [dir] [wasm|riscv|arm-qemu|air]`
/// The wire's delay a command says (fm3 cut.md, "a wire that takes
/// time"): `--wire <time>`, the time one word or two, `100ms` or `100
/// ms`. Gives it, and the places in `args` it took
fn wire_said(args: &[String]) -> Result<(Option<i64>, Vec<usize>), String> {
    let Some(i) = args.iter().position(|a| a == "--wire") else { return Ok((None, Vec::new())) };
    let Some(n) = args.get(i + 1) else { return Err("a wire's delay is said `--wire <time>`: `--wire 100 ms`".to_string()) };
    let unit = args.get(i + 2).filter(|u| *u == "s" || *u == "ms");
    let t = run::time_said(&match unit {
        Some(u) => format!("{}{}", n, u),
        None => n.clone(),
    })?;
    Ok((Some(t), (i..i + if unit.is_some() { 3 } else { 2 }).collect()))
}

/// The moves a command says (fm3 cut.md, "the cut, moved"): each `--then
/// <units> at <time>`, the time one word or two, `4.5s` or `4.5 s`.
/// Gives each cut with its time, and the places in `args` they took
fn moves_said(args: &[String]) -> Result<(Vec<(String, i64)>, Vec<usize>), String> {
    let (mut out, mut used) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < args.len() {
        if args[i] != "--then" {
            i += 1;
            continue;
        }
        let shape = "a move is said `--then <units> at <time>`: `--then none at 4.5 s`";
        let (Some(cut), Some("at"), Some(n)) = (args.get(i + 1), args.get(i + 2).map(String::as_str), args.get(i + 3)) else { return Err(shape.to_string()) };
        let unit = args.get(i + 4).filter(|u| *u == "s" || *u == "ms");
        let t = run::time_said(&match unit {
            Some(u) => format!("{}{}", n, u),
            None => n.clone(),
        })?;
        let end = i + if unit.is_some() { 5 } else { 4 };
        used.extend(i..end);
        out.push((cut.clone(), t));
        i = end;
    }
    Ok((out, used))
}

pub fn cmd(args: &[String], level: usize, policy: ssa::Policy) -> ExitCode {
    let usage = || {
        eprintln!("usage: probe zero <store> emit          print the store's IR");
        eprintln!("                            [sites|trace] ... of its diagnostic build, or its trace build, and the table of places");
        eprintln!("                            [places [--far <units>]] ... of its placed build, which runs at two places (fm3 cut.md),");
        eprintln!("                                         and a function `__cut` that fills the table for that cut");
        eprintln!("                            [--then <units> at <time>] ... of the build whose cut may be moved, and its moves in `__cut`");
        eprintln!("       probe zero <store> run <case>    run one ## testing case natively, on the real clock");
        eprintln!("                            [--fast|-t]  ... as fast as it can, on the virtual clock");
        eprintln!("                            [--far <units>]  ... at a cut: the units named run at a second place, each by where it is");
        eprintln!("                                         written, `countdown.zero:2`, or a function by its words; several with commas; `all`");
        eprintln!("                            [--then <units> at <time>]  ... and the cut moved while it runs: at that time the units");
        eprintln!("                                         named are the ones far, `none` for every unit at the screen; said again, a second move");
        eprintln!("                            [--wire <time>]            ... over a wire that takes that long, `--wire 100 ms`: the case by its own times, what was late, and, where far works ahead of a key, the turns it took and threw away");
        eprintln!("       probe zero <store> placement [--far <units>]   every unit and where it runs at that cut, and what the wire carries");
        eprintln!("                            [--wire <time>] [--case <case>]  ... and one case over such a wire: what it carried, when each left and arrived, each piece the screen dropped unshown and each time far went back");
        eprintln!("                            [--then <units> at <time>]  ... and each move: what is handed across, and the cut after it");
        eprintln!("       probe zero <store> trace <case>  run the case from the store's trace build: a row a step, a column a stream,");
        eprintln!("                            [--json]     what each was given and the lines that ran; or the events and steps as JSON");
        eprintln!("       probe zero meter <store|dir>     the lines that use a non-zeroic form: a store's, listed, or a row a store");
        eprintln!("       probe zero names <store>         every name with a `$` or `[]`: how it is declared, and each use");
        eprintln!("       probe zero test [dir] [path]     run every store under dir (suite/zero)");
        eprintln!("                                        on a path: wasm, riscv, arm-qemu, air (native by default)");
        eprintln!("                                        PROBE_ZERO_CUTS=1: and again at every cut, every unit far and each alone,");
        eprintln!("                                        and a case with a time moved partway, everything far to the screen and back");
        ExitCode::FAILURE
    };
    match args.first().map(String::as_str) {
        Some("test") => {
            let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            let paths = ["wasm", "riscv", "arm-qemu", "air"];
            let backend = match rest.iter().find(|a| paths.contains(a)).copied() {
                Some("wasm") => suite::Backend::Wasm,
                Some("riscv") => suite::Backend::Riscv,
                Some("arm-qemu") => suite::Backend::ArmQemu,
                Some("air") => suite::Backend::Air,
                _ => suite::Backend::Native,
            };
            let dir = rest.iter().find(|a| !paths.contains(a)).copied().unwrap_or("suite/zero");
            match run::test(Path::new(dir), backend, level) {
                Ok(report) => {
                    print!("{}", report.log);
                    if report.failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::FAILURE
                }
            }
        }
        // `probe zero meter <store|dir>` (fm3 log 129): what is not
        // yet zeroic, counted and listed; nothing is lowered or refused
        Some("meter") if args.len() == 2 => match meter::report(Path::new(&args[1])) {
            Ok(text) => {
                print!("{}", text);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e);
                ExitCode::FAILURE
            }
        },
        // `probe zero names <store>` (fm3 log 158): every name declared
        // with a mark, how, and each form it is used in
        Some("names") if args.len() == 2 => match kinds::report(Path::new(&args[1])) {
            Ok(text) => {
                print!("{}", text);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e);
                ExitCode::FAILURE
            }
        },
        // `emit sites`: the diagnostic build's text, and its table of
        // sites as comments after it (fm3 log 199); `emit trace`, the
        // trace build's (fm3 tracer.md)
        Some(store) if args.get(1).map(String::as_str) == Some("emit") && matches!(args.get(2).map(String::as_str), Some("sites" | "trace" | "places")) => match match args[2].as_str() {
            "trace" => run::emit_trace(Path::new(store)),
            "places" => moves_said(args).and_then(|(then, _)| {
                let far = args.iter().position(|a| a == "--far").and_then(|i| args.get(i + 1)).map(String::as_str);
                match wire_said(args)? {
                    (None, _) => run::emit_placed(Path::new(store), far, &then),
                    (wire, _) => run::emit_wired(Path::new(store), far, &then, wire),
                }
            }),
            _ => run::emit_sites(Path::new(store)),
        } {
            Ok(ir) => {
                print!("{}", ir);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e);
                ExitCode::FAILURE
            }
        },
        Some(store) if args.get(1).map(String::as_str) == Some("emit") => match run::emit(Path::new(store)) {
            Ok(ir) => {
                print!("{}", ir);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e);
                ExitCode::FAILURE
            }
        },
        // `probe zero <store> placement [--far <units>]`: where each
        // unit runs at a cut, and what the wire carries (fm3 cut.md)
        Some(store) if args.get(1).map(String::as_str) == Some("placement") => {
            let far = args.iter().position(|a| a == "--far").and_then(|i| args.get(i + 1));
            let case = args.iter().position(|a| a == "--case").and_then(|i| args.get(i + 1));
            match moves_said(args).and_then(|(then, _)| match wire_said(args)? {
                (Some(_), _) if !then.is_empty() => Err("a cut is not moved over a wire that takes time yet".to_string()),
                (Some(d), _) => run::placement_wire(Path::new(store), far.map(String::as_str), d, case.map(String::as_str), &policy, level),
                _ => run::placement(Path::new(store), far.map(String::as_str), &then),
            }) {
                Ok(out) => {
                    print!("{}", out);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::FAILURE
                }
            }
        }
        Some(store) if args.get(1).map(String::as_str) == Some("run") && args.len() >= 3 => {
            // on the real clock unless told to run as fast as it can (log 77)
            let fast = args[2..].iter().any(|a| a == "--fast" || a == "-t");
            // at a cut (fm3 cut.md): `--far <units>`, the units that
            // run at the second place
            let at = args.iter().position(|a| a == "--far");
            let far = at.and_then(|i| args.get(i + 1));
            if at.is_some() && far.is_none() {
                return usage();
            }
            // ... and moved while it runs: `--then <units> at <time>`
            let (then, used) = match moves_said(args) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("{}", e);
                    return ExitCode::FAILURE;
                }
            };
            // ... and over a wire that takes time: `--wire <time>`
            let (wire, wused) = match wire_said(args) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("{}", e);
                    return ExitCode::FAILURE;
                }
            };
            let Some(case) = args.iter().enumerate().skip(2).find(|(i, a)| *a != "--fast" && *a != "-t" && Some(*i) != at && Some(*i) != at.map(|k| k + 1) && !used.contains(i) && !wused.contains(i)).map(|(_, a)| a) else { return usage() };
            if let Some(d) = wire {
                return match if then.is_empty() { run::run_wire(Path::new(store), case, &policy, level, far.map(String::as_str), d) } else { Err("a cut is not moved over a wire that takes time yet".to_string()) } {
                    Ok(out) => {
                        print!("{}", out);
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        ExitCode::FAILURE
                    }
                };
            }
            match match (far, then.is_empty()) {
                (None, true) => run::run(Path::new(store), case, &policy, level, fast),
                (far, _) => run::run_cut(Path::new(store), case, &policy, level, fast, far.map(String::as_str), &then),
            } {
                Ok(out) => {
                    print!("{}", out);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::FAILURE
                }
            }
        }
        // `probe zero <store> trace <case>`: the case run from the
        // store's trace build, and what it wrote down (fm3 tracer.md)
        Some(store) if args.get(1).map(String::as_str) == Some("trace") && args.len() >= 3 => {
            let as_json = args[2..].iter().any(|a| a == "--json");
            let Some(case) = args[2..].iter().find(|a| *a != "--json") else { return usage() };
            match trace::report(Path::new(store), case, &policy, level, as_json) {
                Ok(out) => {
                    print!("{}", out);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}
