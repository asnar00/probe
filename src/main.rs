mod arena;
mod emit;
mod emit_rv;
mod emit_wasm;
mod learn;
mod opt;
mod oracle;
mod regalloc;
mod ssa;
mod suite;
mod target;
mod wlearn;

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // -O<n> selects how much of the SSA pass pipeline runs (default: all)
    let mut level = opt::MAX_LEVEL;
    let mut int_override: Option<ssa::Type> = None;
    args.retain(|a| {
        if let Some(l) = a.strip_prefix("-O") {
            level = l.parse().unwrap_or(opt::MAX_LEVEL);
            false
        } else if let Some(t) = a.strip_prefix("--int=") {
            int_override = ssa::Type::from_name_pub(t);
            false
        } else {
            true
        }
    });
    // native default: the machine's natural 64-bit width
    let int = int_override.unwrap_or(ssa::Type::I64);
    match args.first().map(String::as_str) {
        Some("parse") if args.len() >= 2 => cmd_parse(&args[1], int),
        Some("learn") if args.len() >= 2 => {
            let out = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|i| args.get(i + 1))
                .cloned();
            cmd_learn(&args[1], out.as_deref())
        }
        Some("compile") if args.len() >= 2 => cmd_compile(&args[1], level, int),
        Some("live") if args.len() >= 3 => {
            let fargs: Result<Vec<i64>, _> = args[3..].iter().map(|a| a.parse()).collect();
            match fargs {
                Ok(fargs) => cmd_live(&args[1], &args[2], &fargs, int),
                Err(_) => fail("function arguments must be integers"),
            }
        }
        Some("tiers") if args.len() >= 2 => cmd_tiers(&args[1], int),
        Some("run") if args.len() >= 3 => {
            let fargs: Result<Vec<i64>, _> = args[3..].iter().map(|a| a.parse()).collect();
            match fargs {
                Ok(fargs) => cmd_run(&args[1], &args[2], &fargs, level, int),
                Err(_) => fail("function arguments must be integers"),
            }
        }
        Some("test") => {
            let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            let backend = if rest.contains(&"wasm") {
                suite::Backend::Wasm
            } else if rest.contains(&"riscv") {
                suite::Backend::Riscv
            } else if rest.contains(&"arm-qemu") {
                suite::Backend::ArmQemu
            } else {
                suite::Backend::Native
            };
            let dir = rest
                .iter()
                .find(|a| **a != "wasm" && **a != "riscv" && **a != "arm-qemu")
                .copied()
                .unwrap_or("suite");
            match suite::run_dir_at(dir, backend, level, int_override) {
                Ok(report) => {
                    print!("{}", report.log);
                    if report.failed == 0 {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(e) => fail(&e),
            }
        }
        _ => {
            eprintln!("usage: probe parse <file.ssa>");
            eprintln!("       probe learn <target.probe> [-o encodings.json]");
            eprintln!("       probe compile <file.ssa>");
            eprintln!("       probe run <file.ssa> <function> [args...]");
            eprintln!("       probe tiers <file.ssa>");
            eprintln!("       probe live <file.ssa> <function> [args...]");
            eprintln!("       (-O<n> selects the optimization level on any command;");
            eprintln!("        --int=i32|i64 sets the abstract 'int' replacement policy)");
            ExitCode::FAILURE
        }
    }
}

fn cmd_parse(path: &str, int: ssa::Type) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
    let mut module = match ssa::parse(&src) {
        Ok(m) => m,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
    // abstract types resolve under the same policy as every other command
    let policy = match ssa::Policy::new(int) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    ssa::resolve_types(&mut module, &policy);
    if let Err(errs) = ssa::verify(&module) {
        for e in &errs {
            eprintln!("{}: {}", path, e);
        }
        return ExitCode::FAILURE;
    }
    print!("{}", module);
    ExitCode::SUCCESS
}

fn cmd_learn(path: &str, out: Option<&str>) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    if wlearn::is_bytes_seed(&src) {
        let scratch = std::env::temp_dir().join("probe-oracle");
        if let Err(e) = std::fs::create_dir_all(&scratch) {
            return fail(&format!("{}: {}", scratch.display(), e));
        }
        let (results, end) = match wlearn::learn_wasm_target(&name, &src, &scratch) {
            Ok(r) => r,
            Err(e) => return fail(&e),
        };
        print!("{}", wlearn::wreport(&results));
        if let Some(out_path) = out {
            let json = wlearn::wto_json(&name, &results, end);
            if let Err(e) = std::fs::write(&out_path, json) {
                return fail(&format!("{}: {}", out_path, e));
            }
            println!("wrote {}", out_path);
        }
        return ExitCode::SUCCESS;
    }
    let target = match target::parse_seed(&name, &src) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let oracle = match oracle::LlvmMc::new(&target.triple, target.attr.as_deref()) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let results = match learn::learn_target(&target, &oracle) {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    print!("{}", learn::report(&results));
    if let Some(out_path) = out {
        let json = learn::to_json(&target, &results);
        if let Err(e) = std::fs::write(out_path, json) {
            return fail(&format!("{}: {}", out_path, e));
        }
        println!("wrote {}", out_path);
    }
    ExitCode::SUCCESS
}

const ENCODINGS: &str = "targets/arm64.encodings.json";

fn load_module(path: &str, level: usize, int: ssa::Type) -> Result<ssa::Module, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let mut module = ssa::parse(&src).map_err(|e| format!("{}: {}", path, e))?;
    let policy = ssa::Policy::new(int)?;
    ssa::resolve_types(&mut module, &policy);
    ssa::verify(&module).map_err(|errs| format!("{}: {}", path, errs.join("\n")))?;
    opt::optimize(&mut module, level);
    ssa::verify(&module)
        .map_err(|errs| format!("{}: after optimization: {}", path, errs.join("\n")))?;
    Ok(module)
}

/// Compile at every pipeline prefix: the gradual-optimization story in one
/// table — level 0 is the instant baseline, each row adds one pass.
fn cmd_tiers(path: &str, int: ssa::Type) -> ExitCode {
    let enc = match emit::Encoder::load(ENCODINGS) {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    println!("{:<7} {:<44} {:>8} {:>10}", "level", "passes", "bytes", "time");
    let _warmup = load_module(path, opt::MAX_LEVEL, int); // touch caches before timing
    for level in 0..=opt::MAX_LEVEL {
        let t0 = std::time::Instant::now();
        let module = match load_module(path, level, int) {
            Ok(m) => m,
            Err(e) => return fail(&e),
        };
        let compiled = match emit::compile(&module, &enc) {
            Ok(c) => c,
            Err(e) => return fail(&e),
        };
        let dt = t0.elapsed();
        let names: Vec<&str> = opt::PASSES[..level].iter().map(|(n, _)| *n).collect();
        let label = if names.is_empty() {
            "(none)".to_string()
        } else {
            format!("+{}", names.join(" +"))
        };
        println!(
            "{:<7} {:<44} {:>8} {:>8.0}us",
            level,
            label,
            compiled.code.len(),
            dt.as_secs_f64() * 1e6
        );
    }
    ExitCode::SUCCESS
}

fn cmd_compile(path: &str, level: usize, int: ssa::Type) -> ExitCode {
    let result = load_module(path, level, int).and_then(|module| {
        let enc = emit::Encoder::load(ENCODINGS)?;
        emit::compile(&module, &enc)
    });
    match result {
        Ok(compiled) => {
            let mut funcs: Vec<(&String, &usize)> = compiled.funcs.iter().collect();
            funcs.sort_by_key(|&(_, off)| *off);
            for (i, (name, off)) in funcs.iter().enumerate() {
                let off = **off;
                let end = funcs
                    .get(i + 1)
                    .map(|(_, o)| **o)
                    .unwrap_or(compiled.code.len());
                println!("{} ({} bytes):", name, end - off);
                for at in (off..end).step_by(4) {
                    let w = u32::from_le_bytes(compiled.code[at..at + 4].try_into().unwrap());
                    println!("  {:6x}: {:08x}", at, w);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

fn cmd_run(path: &str, fname: &str, fargs: &[i64], level: usize, int: ssa::Type) -> ExitCode {
    let result = load_module(path, level, int).and_then(|module| {
        let f = module
            .func(fname)
            .ok_or_else(|| format!("no function {} in {}", fname, path))?;
        let rets: Vec<ssa::Repr> = f.rets.iter().map(|&t| f.repr(t)).collect();
        // a register holds the canonical value only up to the type's
        // container: read the result through the declared return type
        let fix = |i: usize, x: i64| match rets.get(i) {
            Some(r) if r.container() == 32 => opt::norm(*r, x as u32 as i64),
            _ => x,
        };
        let enc = emit::Encoder::load(ENCODINGS)?;
        let compiled = emit::compile(&module, &enc)?;
        let jit = emit::jit::JitCode::new(&compiled)?;
        match rets.len() {
            2 => jit
                .call2(fname, fargs)
                .map(|(a, b)| format!("{}, {}", fix(0, a), fix(1, b))),
            n if n <= 1 => jit.call(fname, fargs).map(|v| fix(0, v).to_string()),
            n => Err(format!("{} return values not supported by the runner", n)),
        }
    });
    match result {
        Ok(v) => {
            println!("{}", v);
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}


/// The incremental compiler loop: everything lives in one JIT arena with
/// per-function slots and trampolines. Edits to the source recompile only
/// the changed functions at level 0 (instant); functions whose invocation
/// counters cross the threshold get promoted through the full pipeline.
fn cmd_live(path: &str, fname: &str, fargs: &[i64], int: ssa::Type) -> ExitCode {
    const PROMOTE_AT: u64 = 10_000;
    let enc = match emit::Encoder::load(ENCODINGS) {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    let mut ar = match arena::Arena::new(enc) {
        Ok(a) => a,
        Err(e) => return fail(&e),
    };
    let mut held_src = String::new();
    let mut module: Option<ssa::Module> = None;
    println!(
        "live: watching {} — edit it while this runs; hot functions promote at {} calls",
        path, PROMOTE_AT
    );
    loop {
        // reload on change; a broken edit keeps the last good code running
        match std::fs::read_to_string(path) {
            Ok(src) if src != held_src => {
                held_src = src.clone();
                let parsed = (|| -> Result<ssa::Module, String> {
                    let mut m = ssa::parse(&src).map_err(|e| e.to_string())?;
                    ssa::resolve_types(&mut m, &ssa::Policy::new(int)?);
                    ssa::verify(&m).map_err(|e| e.join("; "))?;
                    Ok(m)
                })();
                match parsed {
                    Ok(m) => {
                        let t0 = std::time::Instant::now();
                        match ar.sync(&m.funcs, 0) {
                            Ok(done) => {
                                let dt = t0.elapsed().as_secs_f64() * 1e6;
                                for i in &done {
                                    println!(
                                        "  reload  {:<12} {:>4} bytes  level 0  {}",
                                        i.name,
                                        i.bytes,
                                        if i.in_place { "in place" } else { "relocated" }
                                    );
                                }
                                if !done.is_empty() {
                                    println!("  ({} function(s) in {:.0}us)", done.len(), dt);
                                }
                                module = Some(m);
                            }
                            Err(e) => println!("  compile error: {} (keeping old code)", e),
                        }
                    }
                    Err(e) => println!("  parse error: {} (keeping old code)", e),
                }
            }
            _ => {}
        }
        // promote hot tier-0 functions through the full pipeline
        if let Some(m) = &module {
            for (name, calls, lvl) in ar.by_heat() {
                if calls >= PROMOTE_AT && lvl < opt::MAX_LEVEL {
                    if let Some(f) = m.funcs.iter().find(|f| f.name == name) {
                        let t0 = std::time::Instant::now();
                        match ar.install(f, opt::MAX_LEVEL) {
                            Ok(i) => println!(
                                "  promote {:<12} level {} after {} calls ({:.0}us, {} bytes, {})",
                                i.name,
                                i.level,
                                calls,
                                t0.elapsed().as_secs_f64() * 1e6,
                                i.bytes,
                                if i.in_place { "in place" } else { "relocated" }
                            ),
                            Err(e) => println!("  promote {} failed: {}", name, e),
                        }
                    }
                }
            }
            let nrets = m.func(fname).map(|f| f.rets.len()).unwrap_or(1);
            let shown = match nrets {
                2 => ar.call2(fname, fargs).map(|(a, b)| format!("{}, {}", a, b)),
                _ => ar.call(fname, fargs).map(|v| v.to_string()),
            };
            match shown {
                Ok(v) => println!("{}({:?}) = {}", fname, fargs, v),
                Err(e) => println!("  {}", e),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("{}", msg);
    ExitCode::FAILURE
}
