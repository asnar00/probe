//! The zero language's front end: a store of features (`store.rs`) is
//! lexed (`lex.rs`) and parsed (`syntax.rs`) into a small tree per
//! feature, lowered to IR text (`lower.rs`), and run through the rest
//! of probe by the runner (`run.rs`). zero.md in the fm3 project is the
//! language definition; the front end adds no semantics of its own.

pub mod home;
pub mod kinds;
pub mod lex;
pub mod lower;
pub mod meter;
pub mod places;
pub mod real;
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
///
/// ... and a wire whose delay changes (fm3 log 289): `--wire 100 ms then
/// 2 s at 3 s`, as many `then`s as wanted, `then gone at <time>` where
/// the wire goes
/// Three places over wires that take time (hop fifty-nine): `--wire ann
/// 100 ms, bob 2 s` gives each person's wire its delay, a time alone
/// every wire's; `--gone server 3 s to 6 s`, `--gone ann 3 s to 6 s` (her
/// device), `--gone ann's wire 3 s to 6 s`, several joined by commas.
/// None where neither a name after `--wire` nor `--gone` is said
fn home_said(args: &[String]) -> Result<Option<(run::HomeSaid, Vec<usize>)>, String> {
    let wire = args.iter().position(|a| a == "--wire");
    let gone = args.iter().position(|a| a == "--gone");
    let named = wire.and_then(|i| args.get(i + 1)).is_some_and(|w| w.starts_with(|c: char| c.is_ascii_alphabetic()));
    if !named && gone.is_none() {
        return Ok(None);
    }
    // the words after a flag, commas their own, and where each came from
    let words_of = |i: usize| -> Vec<(String, usize)> {
        let mut words = Vec::new();
        for (k, a) in args.iter().enumerate().skip(i + 1) {
            if a.starts_with("--") {
                break;
            }
            match a.strip_suffix(',') {
                Some(w) if !w.is_empty() => {
                    words.push((w.to_string(), k));
                    words.push((",".to_string(), k));
                }
                _ => words.push((a.clone(), k)),
            }
        }
        words
    };
    let time = |ws: &[(String, usize)], k: usize| -> Result<(i64, usize), String> {
        let n = ws.get(k).map(|(w, _)| w.as_str()).ok_or("a time is wanted: `3 s`, `100 ms`")?;
        let unit = ws.get(k + 1).map(|(w, _)| w.as_str()).filter(|u| *u == "s" || *u == "ms");
        let t = run::time_said(&match unit {
            Some(u) => format!("{}{}", n, u),
            None => n.to_string(),
        })?;
        Ok((t, k + if unit.is_some() { 2 } else { 1 }))
    };
    let mut said = run::HomeSaid::default();
    let mut used = Vec::new();
    if let Some(i) = wire {
        used.push(i);
        let ws = words_of(i);
        used.extend(ws.iter().map(|(_, k)| *k));
        let mut k = 0;
        while k < ws.len() {
            if ws[k].0.starts_with(|c: char| c.is_ascii_alphabetic()) {
                let name = ws[k].0.clone();
                let (d, k2) = time(&ws, k + 1)?;
                said.delays.push((name, d));
                k = k2;
            } else {
                let (d, k2) = time(&ws, k)?;
                said.delay = d;
                k = k2;
            }
            match ws.get(k).map(|(w, _)| w.as_str()) {
                Some(",") => k += 1,
                None => {}
                Some(w) => return Err(format!("'{}' is not a word of a wire at three places: `--wire 100 ms`, `--wire ann 100 ms, bob 2 s`", w)),
            }
        }
    }
    if let Some(i) = gone {
        used.push(i);
        let ws = words_of(i);
        used.extend(ws.iter().map(|(_, k)| *k));
        let mut k = 0;
        while k < ws.len() {
            let who = ws[k].0.clone();
            let (what, name, k1) = if who == "server" {
                ("server", String::new(), k + 1)
            } else if let Some(n) = who.strip_suffix("'s") {
                if ws.get(k + 1).map(|(w, _)| w.as_str()) != Some("wire") {
                    return Err(format!("`--gone {} wire <from> to <to>` is a person's wire gone", who));
                }
                ("wire", n.to_string(), k + 2)
            } else {
                ("device", who.clone(), k + 1)
            };
            let (from, k2) = time(&ws, k1)?;
            if ws.get(k2).map(|(w, _)| w.as_str()) != Some("to") {
                return Err("something gone is said `--gone server 3 s to 6 s`, `--gone ann 3 s to 6 s`, `--gone ann's wire 3 s to 6 s`".into());
            }
            let (to, k3) = time(&ws, k2 + 1)?;
            said.gone.push((what.to_string(), name, from, to));
            k = k3;
            if ws.get(k).map(|(w, _)| w.as_str()) == Some(",") {
                k += 1;
            }
        }
    }
    Ok(Some((said, used)))
}

/// A real wire (fm3 wire.md; hop sixty): `--wire real`, and a set delay
/// the receiver holds each row for, `--wire real, 100 ms`. Gives it and
/// the places in `args` it took; None where the word after `--wire` is
/// not `real`
fn real_said(args: &[String]) -> Result<Option<(real::Real, Vec<usize>)>, String> {
    let Some(i) = args.iter().position(|a| a == "--wire") else { return Ok(None) };
    let first = args.get(i + 1).map(String::as_str);
    if first != Some("real") && first != Some("real,") {
        return Ok(None);
    }
    let mut used = vec![i, i + 1];
    let mut k = i + 2;
    let comma = first == Some("real,") || args.get(k).map(String::as_str) == Some(",");
    if args.get(k).map(String::as_str) == Some(",") {
        used.push(k);
        k += 1;
    }
    let mut delay = 0;
    if comma {
        let n = args.get(k).filter(|a| !a.starts_with("--")).ok_or("a real wire's set delay is said `--wire real, 100 ms`")?;
        let unit = args.get(k + 1).filter(|u| *u == "s" || *u == "ms");
        delay = run::time_said(&match unit {
            Some(u) => format!("{}{}", n, u),
            None => n.clone(),
        })?;
        used.push(k);
        if unit.is_some() {
            used.push(k + 1);
            k += 1;
        }
        k += 1;
    }
    if let Some(w) = args.get(k).filter(|a| !a.starts_with("--")) {
        if w.starts_with(',') || w == "then" || w.ends_with("bytes/s") || w == "gone" {
            return Err(real::refused_step(false, false, false, false, true).unwrap());
        }
    }
    Ok(Some((real::Real { delay }, used)))
}

/// what `emit places` and `placement` say of a real wire: its build is the
/// build for a wire with the queue's entries, and it is run, not placed
const REAL_RUN: &str = "a real wire is run, not placed: `probe zero <store> run '<case>' --server <units> --wire real`; its build is the one for a wire that takes time, `emit places --wire 0 ms`, with the queue's entries the harness reads and writes";

fn wire_said(args: &[String]) -> Result<(Option<run::Wire>, Vec<usize>), String> {
    let Some(i) = args.iter().position(|a| a == "--wire") else {
        if args.iter().any(|a| a == "--choose") {
            return Err("`--choose <bound>` is said with `--server <cut>` and `--wire <time>`: the runner chooses between the cut given and nothing on the server as the wire changes".to_string());
        }
        return Ok((None, Vec::new()));
    };
    // (fm3 log 294: a comma is a word of its own or ends the word
    // before it; each word of the wire's with the place in `args` it
    // came from)
    let mut words: Vec<(String, usize)> = Vec::new();
    for (k, a) in args.iter().enumerate().skip(i + 1) {
        if a.starts_with("--") {
            break;
        }
        match a.strip_suffix(',') {
            Some(w) if !w.is_empty() => {
                words.push((w.to_string(), k));
                words.push((",".to_string(), k));
            }
            _ => words.push((a.clone(), k)),
        }
    }
    let word = |k: usize| words.get(k).map(|(w, _)| w.as_str());
    // a time at k, one word or two: the time, and the place after it
    let time_at = |k: usize| -> Result<(i64, usize), String> {
        let Some(n) = word(k) else { return Err("a wire's delay is said `--wire <time>`: `--wire 100 ms`, `--wire 100 ms then 2 s at 3 s`".to_string()) };
        let unit = word(k + 1).filter(|u| *u == "s" || *u == "ms");
        let t = run::time_said(&match unit {
            Some(u) => format!("{}{}", n, u),
            None => n.to_string(),
        })?;
        Ok((t, k + if unit.is_some() { 2 } else { 1 }))
    };
    // ... a speed at k, `40 bytes/s` or `40bytes/s`
    let speed_at = |k: usize| -> Option<(i64, usize)> {
        let n = word(k)?;
        if let Some(v) = n.strip_suffix("bytes/s") {
            return v.parse::<i64>().ok().map(|v| (v, k + 1));
        }
        if word(k + 1) == Some("bytes/s") {
            return n.parse::<i64>().ok().map(|v| (v, k + 2));
        }
        None
    };
    let (d, mut k) = time_at(0)?;
    let (mut speed, mut every) = (0, None);
    // `, <n> bytes/s` and `, gone <time> every <time> from <time>`
    while word(k) == Some(",") {
        if word(k + 1) == Some("gone") {
            let (gap, a) = time_at(k + 2)?;
            if word(a) != Some("every") {
                return Err("a wire that comes and goes is said `, gone <time> every <time> from <time>`: `--wire 100 ms, gone 500 ms every 2 s from 1 s`".to_string());
            }
            let (per, b) = time_at(a + 1)?;
            if word(b) != Some("from") {
                return Err("a wire that comes and goes is said `, gone <time> every <time> from <time>`: `--wire 100 ms, gone 500 ms every 2 s from 1 s`".to_string());
            }
            let (from, c) = time_at(b + 1)?;
            every = Some((gap, per, from));
            k = c;
        } else if let Some((v, next)) = speed_at(k + 1) {
            speed = v;
            k = next;
        } else {
            return Err("after a wire's delay a comma says its speed or its coming and going: `--wire 100 ms, 40 bytes/s`, `--wire 100 ms, gone 500 ms every 2 s from 1 s`".to_string());
        }
    }
    let mut then = Vec::new();
    let mut speeds = Vec::new();
    while word(k) == Some("then") {
        let (v, mut next) = if word(k + 1) == Some("gone") { (run::GONE, k + 2) } else { time_at(k + 1)? };
        let mut sp = None;
        if word(next) == Some(",") && v < run::GONE {
            let Some((s2, n2)) = speed_at(next + 1) else { return Err("a change's speed is said `then <time>, <n> bytes/s at <time>`: `then 100 ms, 10000 bytes/s at 5 s`".to_string()) };
            sp = Some(s2);
            next = n2;
        }
        if word(next) != Some("at") {
            return Err("a wire's change is said `then <time> at <time>` or `then gone at <time>`: `--wire 100 ms then gone at 3 s`".to_string());
        }
        let (at, after) = time_at(next + 1)?;
        then.push((at, v));
        speeds.push(sp);
        k = after;
    }
    let mut used: Vec<usize> = std::iter::once(i).chain(words[..k].iter().map(|(_, a)| *a)).collect();
    used.dedup();
    // (the chooser's bound, said beside the wire, fm3 log 290)
    let choose = match args.iter().position(|a| a == "--choose") {
        Some(c) => {
            let (b, after) = {
                let n = args.get(c + 1).ok_or("`--choose <bound>`: `--choose 250 ms`")?;
                let unit = args.get(c + 2).filter(|u| *u == "s" || *u == "ms");
                let t = run::time_said(&match unit {
                    Some(u) => format!("{}{}", n, u),
                    None => n.clone(),
                })?;
                (t, c + 1 + if unit.is_some() { 2 } else { 1 })
            };
            used.extend(c..after);
            Some(b)
        }
        None => None,
    };
    Ok((Some(run::Wire { d, then, choose, speed, speeds, every }.checked()?), used))
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

/// A word of the command line that the command does not know, refused
/// with its name and nothing run (fm3 log 301, principle 8: a request
/// that cannot be met stops). A word is one that begins `--`, or `-`
/// and a letter; the rest are a command's own values, a case, a time, a
/// cut. `--far`, the server's old name (fm3 question 169), is pointed
/// at its new one
fn unknown_word(args: &[String]) -> Option<String> {
    let known: &[&str] = match (args.first().map(String::as_str), args.get(1).map(String::as_str)) {
        (Some("test" | "meter" | "names"), _) => &[],
        (Some(_), Some("emit")) if args.get(2).map(String::as_str) == Some("places") => &["--server", "--then", "--wire", "--choose"],
        (Some(_), Some("emit")) => &[],
        (Some(_), Some("placement")) => &["--server", "--case", "--then", "--wire", "--choose"],
        (Some(_), Some("run")) => &["--fast", "-t", "--server", "--then", "--wire", "--choose", "--gone"],
        (Some(_), Some("trace")) => &["--json"],
        _ => return None,
    };
    let flag = |a: &str| a.starts_with("--") || (a.starts_with('-') && a[1..].starts_with(|c: char| c.is_ascii_alphabetic()));
    let w = args.iter().skip(1).find(|a| flag(a) && !known.contains(&a.as_str()))?;
    Some(if w == "--far" && known.contains(&"--server") {
        format!("{} is not a word the runner knows; did you mean --server?", w)
    } else if known.is_empty() {
        format!("{} is not a word the runner knows; this command takes none", w)
    } else {
        format!("{} is not a word the runner knows; this command knows {}", w, known.join(", "))
    })
}

pub fn cmd(args: &[String], level: usize, policy: ssa::Policy) -> ExitCode {
    if let Some(why) = unknown_word(args) {
        eprintln!("{}", why);
        return ExitCode::FAILURE;
    }
    let usage = || {
        eprintln!("usage: probe zero <store> emit          print the store's IR");
        eprintln!("                            [sites|trace] ... of its diagnostic build, or its trace build, and the table of places");
        eprintln!("                            [places [--server <units>]] ... of its placed build, which runs at two places (fm3 cut.md),");
        eprintln!("                                         and a function `__cut` that fills the table for that cut");
        eprintln!("                            [--then <units> at <time>] ... of the build whose cut may be moved, and its moves in `__cut`");
        eprintln!("       probe zero <store> run <case>    run one ## testing case natively, on the real clock");
        eprintln!("                            [--fast|-t]  ... as fast as it can, on the virtual clock");
        eprintln!("                            [--server <units>]  ... at a cut: the units named run at a second place, each by where it is");
        eprintln!("                                         written, `countdown.zero:2`, or a function by its words; several with commas; `all`");
        eprintln!("                            [--then <units> at <time>]  ... and the cut moved while it runs: at that time the units");
        eprintln!("                                         named are the ones on the server, `none` for every unit at the client; said again, a second move");
        eprintln!("                            [--wire <time>]            ... over a wire that takes that long, `--wire 100 ms`: the case by its own times, what was late, and, where the server works ahead of a key, the turns it took and threw away; with `--then`, the cut moved over that wire");
        eprintln!("                            [--choose <bound>]         ... with `--server` and `--wire`: the runner chooses each cut after the first, between it and nothing on the server, at the client, by round trips of its own: lateness under the bound is none; each choice printed with its time and reason");
        eprintln!("                            [--wire <time> then <time>|gone at <time> ...]  ... over a wire whose delay changes while it runs, or that goes: an item takes the delay of when it is put on, a gone wire holds it until it comes back; beneath what was late, the pieces never shown where the wire went for good");
        eprintln!("                            [--wire real[, <time>]]    ... over a real wire: the client and the server in two processes on this machine, the rows crossing a loopback socket as bytes, on the real clock; with a time, each row held that long where it arrives (fm3 wire.md, step one)");
        eprintln!("       probe zero <store> placement [--server <units>]   every unit and where it runs at that cut, and what the wire carries");
        eprintln!("                            [--wire <time>] [--case <case>]  ... and one case over such a wire: what it carried, when each left and arrived, each piece the client dropped unshown and each time the server went back");
        eprintln!("                            [--then <units> at <time>]  ... and each move: what is handed across, and the cut after it; with `--wire`, the move's own items among what the wire carried");
        eprintln!("       probe zero <store> trace <case>  run the case from the store's trace build: a row a step, a column a stream,");
        eprintln!("                            [--json]     what each was given and the lines that ran; or the events and steps as JSON");
        eprintln!("       probe zero meter <store|dir>     the lines that use a non-zeroic form: a store's, listed, or a row a store");
        eprintln!("       probe zero names <store>         every name with a `$` or `[]`: how it is declared, and each use");
        eprintln!("       probe zero test [dir] [path]     run every store under dir (suite/zero)");
        eprintln!("                                        on a path: wasm, riscv, arm-qemu, air (native by default)");
        eprintln!("                                        PROBE_ZERO_CUTS=1: and again at every cut, every unit on the server and each alone,");
        eprintln!("                                        and a case with a time moved partway, everything on the server to the client and back");
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
            "places" if real_said(args).is_ok_and(|r| r.is_some()) => Err(REAL_RUN.to_string()),
            "places" => moves_said(args).and_then(|(then, _)| {
                let server = args.iter().position(|a| a == "--server").and_then(|i| args.get(i + 1)).map(String::as_str);
                match wire_said(args)? {
                    (None, _) => run::emit_placed(Path::new(store), server, &then),
                    (wire, _) => run::emit_wired(Path::new(store), server, &then, wire.as_ref()),
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
        // `probe zero <store> placement [--server <units>]`: where each
        // unit runs at a cut, and what the wire carries (fm3 cut.md)
        Some(store) if args.get(1).map(String::as_str) == Some("placement") => {
            let server = args.iter().position(|a| a == "--server").and_then(|i| args.get(i + 1));
            let case = args.iter().position(|a| a == "--case").and_then(|i| args.get(i + 1));
            if real_said(args).is_ok_and(|r| r.is_some()) {
                eprintln!("{}", REAL_RUN);
                return ExitCode::FAILURE;
            }
            match moves_said(args).and_then(|(then, _)| match wire_said(args)? {
                (Some(w), _) => run::placement_wire_moved(Path::new(store), server.map(String::as_str), &then, w, case.map(String::as_str), &policy, level),
                _ => run::placement(Path::new(store), server.map(String::as_str), &then),
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
            // at a cut (fm3 cut.md): `--server <units>`, the units that
            // run at the second place
            let at = args.iter().position(|a| a == "--server");
            let server = at.and_then(|i| args.get(i + 1));
            if at.is_some() && server.is_none() {
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
            // ... over a real wire (hop sixty): `--wire real`, the client and
            // the server in two processes on this machine
            match real_said(args) {
                Ok(Some((r, rused))) => {
                    let Some(case) = args.iter().enumerate().skip(2).find(|(i, a)| !a.starts_with("--") && *a != "-t" && Some(*i) != at.map(|k| k + 1) && !used.contains(i) && !rused.contains(i)).map(|(_, a)| a) else { return usage() };
                    let choose = args.iter().any(|a| a == "--choose");
                    let other = args.iter().any(|a| a == "--gone");
                    if let Some(why) = real::refused_step(!then.is_empty(), choose, false, fast, other) {
                        eprintln!("{}", why);
                        return ExitCode::FAILURE;
                    }
                    return match real::run_real(Path::new(store), case, &policy, level, server.map(String::as_str), r, None) {
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
                Ok(None) => {}
                Err(e) => {
                    eprintln!("{}", e);
                    return ExitCode::FAILURE;
                }
            }
            // ... at three places, a case that names two people or more
            // (hop fifty-nine): `--wire ann 100 ms, bob 2 s`, `--gone ...`
            match home_said(args) {
                Ok(Some((said, hused))) => {
                    let Some(case) = args.iter().enumerate().skip(2).find(|(i, a)| *a != "--fast" && *a != "-t" && !hused.contains(i)).map(|(_, a)| a) else { return usage() };
                    return match run::run_home(Path::new(store), case, &policy, level, &said) {
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
                Ok(None) => {}
                Err(e) => {
                    eprintln!("{}", e);
                    return ExitCode::FAILURE;
                }
            }
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
                return match run::run_wire_moved(Path::new(store), case, &policy, level, server.map(String::as_str), &then, d) {
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
            match match (server, then.is_empty()) {
                (None, true) => run::run(Path::new(store), case, &policy, level, fast),
                (server, _) => run::run_cut(Path::new(store), case, &policy, level, fast, server.map(String::as_str), &then),
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

#[cfg(test)]
mod tests {
    use super::unknown_word;

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| w.to_string()).collect()
    }

    /// a word the command does not know is refused by name (fm3 log
    /// 301); `--far`, the server's old name, points at `--server`; a
    /// case, a time and a negative number are not words
    #[test]
    fn a_word_the_runner_does_not_know_is_refused() {
        assert_eq!(unknown_word(&words(&["suite/zero/hello", "run", "run()", "--fast", "--far", "all"])).as_deref(), Some("--far is not a word the runner knows; did you mean --server?"));
        assert_eq!(unknown_word(&words(&["suite/zero/hello", "run", "run()", "--fast", "--server", "all", "--then", "none", "at", "4.5", "s", "--wire", "100", "ms", "--choose", "250", "ms"])), None);
        assert_eq!(unknown_word(&words(&["suite/zero/hello", "placement", "--server", "all", "--wire", "100ms", "--case", "run()"])), None);
        assert_eq!(unknown_word(&words(&["suite/zero/hello", "run", "-t", "(-1) plus (2)"])), None);
        assert_eq!(unknown_word(&words(&["suite/zero/hello", "trace", "run()", "--fast"])).as_deref(), Some("--fast is not a word the runner knows; this command knows --json"));
        assert_eq!(unknown_word(&words(&["suite/zero/hello", "emit", "--far"])).as_deref(), Some("--far is not a word the runner knows; this command takes none"));
        assert_eq!(unknown_word(&words(&["test", "suite/zero", "wasm", "-x"])).as_deref(), Some("-x is not a word the runner knows; this command takes none"));
    }
}
