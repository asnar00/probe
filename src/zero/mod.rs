//! The zero language's front end: a store of features (`store.rs`) is
//! lexed (`lex.rs`) and parsed (`syntax.rs`) into a small tree per
//! feature, lowered to IR text (`lower.rs`), and run through the rest
//! of probe by the runner (`run.rs`). zero.md in the fm3 project is the
//! language definition; the front end adds no semantics of its own.

pub mod kinds;
pub mod lex;
pub mod lower;
pub mod meter;
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
pub fn cmd(args: &[String], level: usize, policy: ssa::Policy) -> ExitCode {
    let usage = || {
        eprintln!("usage: probe zero <store> emit          print the store's IR");
        eprintln!("                            [sites|trace] ... of its diagnostic build, or its trace build, and the table of places");
        eprintln!("       probe zero <store> run <case>    run one ## testing case natively, on the real clock");
        eprintln!("                            [--fast|-t]  ... as fast as it can, on the virtual clock");
        eprintln!("       probe zero <store> trace <case>  run the case from the store's trace build: a row a step, a column a stream,");
        eprintln!("                            [--json]     what each was given and the lines that ran; or the events and steps as JSON");
        eprintln!("       probe zero meter <store|dir>     the lines that use a non-zeroic form: a store's, listed, or a row a store");
        eprintln!("       probe zero names <store>         every name with a `$` or `[]`: how it is declared, and each use");
        eprintln!("       probe zero test [dir] [path]     run every store under dir (suite/zero)");
        eprintln!("                                        on a path: wasm, riscv, arm-qemu, air (native by default)");
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
        Some(store) if args.get(1).map(String::as_str) == Some("emit") && matches!(args.get(2).map(String::as_str), Some("sites" | "trace")) => match if args[2] == "trace" { run::emit_trace(Path::new(store)) } else { run::emit_sites(Path::new(store)) } {
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
        Some(store) if args.get(1).map(String::as_str) == Some("run") && args.len() >= 3 => {
            // on the real clock unless told to run as fast as it can (log 77)
            let fast = args[2..].iter().any(|a| a == "--fast" || a == "-t");
            let Some(case) = args[2..].iter().find(|a| *a != "--fast" && *a != "-t") else { return usage() };
            match run::run(Path::new(store), case, &policy, level, fast) {
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
