//! The zero language's front end: a store of features (`store.rs`) is
//! lexed (`lex.rs`) and parsed (`syntax.rs`) into a small tree per
//! feature, lowered to IR text (`lower.rs`), and run through the rest
//! of probe by the runner (`run.rs`). zero.md in the fm3 project is the
//! language definition; the front end adds no semantics of its own.

pub mod lex;
pub mod lower;
pub mod run;
pub mod store;
pub mod syntax;

use crate::{ssa, suite};
use std::path::Path;
use std::process::ExitCode;

/// `probe zero <store> emit`, `probe zero <store> run <case>`,
/// `probe zero test [dir] [wasm]`
pub fn cmd(args: &[String], level: usize, policy: ssa::Policy) -> ExitCode {
    let usage = || {
        eprintln!("usage: probe zero <store> emit          print the store's IR");
        eprintln!("       probe zero <store> run <case>    run one ## testing case natively");
        eprintln!("       probe zero test [dir] [wasm]     run every store under dir (suite/zero)");
        ExitCode::FAILURE
    };
    match args.first().map(String::as_str) {
        Some("test") => {
            let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            let backend = if rest.contains(&"wasm") { suite::Backend::Wasm } else { suite::Backend::Native };
            let dir = rest.iter().find(|a| **a != "wasm").copied().unwrap_or("suite/zero");
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
        Some(store) if args.get(1).map(String::as_str) == Some("run") && args.len() >= 3 => match run::run(Path::new(store), &args[2], &policy, level) {
            Ok(out) => {
                print!("{}", out);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e);
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}
