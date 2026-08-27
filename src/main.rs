mod aggregate;
mod arena;
mod bitcode;
mod emit;
mod emit_rv;
mod emit_wasm;
mod footprint;
mod fuzz;
mod scorecard;
mod structure;
mod testfloat;
mod wide;
mod learn;
mod opt;
mod platform;
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
    let mut float_override: Option<(u32, u32)> = None;
    let mut fixed_override: Option<(u32, u32)> = None;
    let mut unit_override: Option<u32> = None;
    let mut sunit_override: Option<u32> = None;
    let mut rational_override: Option<(u32, u32)> = None;
    let mut round: Option<String> = None;
    let mut scalar_override: Option<String> = None;
    args.retain(|a| {
        if let Some(l) = a.strip_prefix("-O") {
            level = l.parse().unwrap_or(opt::MAX_LEVEL);
            false
        } else if let Some(t) = a.strip_prefix("--int=") {
            int_override = ssa::Type::from_name_pub(t);
            false
        } else if let Some(t) = a.strip_prefix("--float=") {
            float_override = ssa::Policy::float_from_arg(t);
            false
        } else if let Some(t) = a.strip_prefix("--fixed=") {
            fixed_override = ssa::Policy::fixed_from_arg(t);
            false
        } else if let Some(t) = a.strip_prefix("--unit=") {
            unit_override = t.parse().ok();
            false
        } else if let Some(t) = a.strip_prefix("--sunit=") {
            sunit_override = t.parse().ok();
            false
        } else if let Some(t) = a.strip_prefix("--rational=") {
            rational_override = ssa::Policy::fixed_from_arg(t);
            false
        } else if let Some(t) = a.strip_prefix("--scalar=") {
            scalar_override = Some(t.to_string());
            false
        } else if let Some(t) = a.strip_prefix("--round=") {
            round = Some(t.to_string());
            false
        } else if let Some(t) = a.strip_prefix("--platform=") {
            platform::select(t);
            false
        } else if a == "--soft" {
            platform::set_soft(true);
            false
        } else {
            true
        }
    });
    // native default: the machine's natural 64-bit width, and f64 with it
    let int = int_override.unwrap_or(ssa::Type::I64);
    let mut policy = match ssa::Policy::new(int) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if let Some((e, m)) = float_override {
        policy = policy.with_float(e, m);
    }
    if let Some((i, f)) = fixed_override {
        policy = policy.with_fixed(i, f);
    }
    if let Some(n) = unit_override {
        policy = policy.with_unit(n);
    }
    if let Some(n) = sunit_override {
        policy = policy.with_sunit(n);
    }
    if let Some((n, d)) = rational_override {
        policy = policy.with_rational(n, d);
    }
    if let Some(f) = &scalar_override {
        policy = match policy.with_scalar(f) {
            Some(p) => p,
            None => return fail(&format!("--scalar={}: not one of {}", f, ssa::Policy::SCALARS.join(", "))),
        };
    }
    if let Some(r) = &round {
        policy = match policy.with_round(r) {
            Ok(p) => p,
            Err(e) => return fail(&e),
        };
    }
    // the native platform (or the selected variant of it) may lack
    // integer instructions the parser would otherwise assume
    policy = match platform::Platform::load("arm64") {
        Ok(p) => p.adjust(policy),
        Err(e) => return fail(&e),
    };
    match args.first().map(String::as_str) {
        Some("parse") if args.len() >= 2 => cmd_parse(&args[1], policy),
        Some("learn") if args.len() >= 2 => {
            let out = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|i| args.get(i + 1))
                .cloned();
            cmd_learn(&args[1], out.as_deref())
        }
        Some("compile") if args.len() >= 2 => cmd_compile(&args[1], level, policy),
        Some("live") if args.len() >= 3 => {
            let fargs: Result<Vec<i64>, _> = args[3..].iter().map(|a| parse_arg(a)).collect();
            match fargs {
                Ok(fargs) => cmd_live(&args[1], &args[2], &fargs, policy),
                Err(_) => fail("function arguments must be integers"),
            }
        }
        Some("tiers") if args.len() >= 2 => cmd_tiers(&args[1], policy),
        Some("run") if args.len() >= 3 => {
            let fargs: Result<Vec<i64>, _> = args[3..].iter().map(|a| parse_arg(a)).collect();
            match fargs {
                Ok(fargs) => cmd_run(&args[1], &args[2], &fargs, level, policy),
                Err(_) => fail("function arguments must be integers"),
            }
        }
        Some("boot") if args.len() >= 2 => {
            let target = if args.iter().any(|a| a == "arm") { "arm64" } else { "riscv64" };
            // stdin — a pipe or the terminal — is the machine's serial input
            match suite::boot(&args[1], target, level, policy, None) {
                Ok(out) => {
                    print!("{}", out);
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        Some("footprint") if args.len() >= 2 => {
            let target = if args.iter().any(|a| a == "riscv") { "riscv64" } else { "arm64" };
            let result = (|| -> Result<(), String> {
                let platform = platform::Platform::load(target)?;
                let policy = platform.adjust(policy);
                let src = std::fs::read_to_string(&args[1]).map_err(|e| format!("{}: {}", args[1], e))?;
                let mut module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| e.to_string())?;
                ssa::resolve_types(&mut module, &policy);
                ssa::verify(&module).map_err(|e| e.join("; "))?;
                opt::optimize(&mut module, level);
                let counts = footprint::footprint(&module, target, &platform)?;
                let total: usize = counts.values().sum();
                println!("{} on {} ({}): {} instructions, {} distinct templates", args[1], target, platform.name, total, counts.len());
                let exts: Vec<String> = platform.extensions().iter().map(|(g, p)| format!("{}{}", g, if *p { "" } else { " (absent)" })).collect();
                if !exts.is_empty() {
                    println!("extensions: {}", exts.join(", "));
                }
                let mut rows: Vec<(&String, &usize)> = counts.iter().collect();
                rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                for (t, c) in rows {
                    println!("{:>7}  {}", c, t);
                }
                Ok(())
            })();
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
        Some("scorecard") => {
            let targets: Vec<&str> = if args.len() >= 2 { vec![args[1].as_str()] } else { vec!["arm64", "riscv64", "wasm32"] };
            let mut problems = 0;
            for t in targets {
                match scorecard::scorecard(t) {
                    Ok(card) => {
                        let path = format!("targets/{}.scorecard.md", t);
                        if let Err(e) = std::fs::write(&path, &card.text) {
                            return fail(&e.to_string());
                        }
                        let summary: Vec<&str> = card.text.lines().filter(|l| l.contains(" templates match ") || l.starts_with("| ") && !l.starts_with("| `") && !l.starts_with("| group") && !l.starts_with("| template")).collect();
                        println!("{}: {} problem(s), written to {}
  {}", t, card.problems, path, summary.join("
  "));
                        problems += card.problems;
                    }
                    Err(e) => return fail(&e),
                }
            }
            if problems == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Some("testfloat") => {
            let only: Vec<String> = args[1..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
            match testfloat::run(&only, level, policy) {
                Ok(r) => {
                    print!("{}", r.log);
                    println!("{} cases, {} wrong", r.cases, r.failed);
                    if r.failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
                }
                Err(e) => fail(&e),
            }
        }
        Some("fuzz") => {
            let count = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(20);
            let seed = args.iter().find_map(|a| a.strip_prefix("--seed=")).and_then(|s| u64::from_str_radix(s, 16).ok()).unwrap_or(1);
            let slow = args.iter().any(|a| a == "--slow");
            match fuzz::fuzz(count, seed, slow) {
                Ok(0) => {
                    println!("{} programs, every configuration agreed", count);
                    ExitCode::SUCCESS
                }
                Ok(n) => {
                    println!("{} of {} programs disagreed somewhere; see target/fuzz/", n, count);
                    ExitCode::FAILURE
                }
                Err(e) => fail(&e),
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
            match suite::run_dir_at(dir, backend, level, &|p| {
                let mut p = p;
                if let Some(t) = int_override {
                    p.int = t;
                }
                if let Some((e, m)) = float_override {
                    p = p.with_float(e, m);
                }
                if let Some((i, f)) = fixed_override {
                    p = p.with_fixed(i, f);
                }
                if let Some(n) = unit_override {
                    p = p.with_unit(n);
                }
                if let Some(n) = sunit_override {
                    p = p.with_sunit(n);
                }
                if let Some((n, d)) = rational_override {
                    p = p.with_rational(n, d);
                }
                if let Some(f) = &scalar_override {
                    p = p.with_scalar(f).unwrap_or(p);
                }
                if let Some(r) = &round {
                    p = p.with_round(r).unwrap_or(p);
                }
                p
            }) {
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
            eprintln!("       probe fuzz [count] [--seed=hex] [--slow]");
            eprintln!("       probe testfloat [f32|add|f16_to_i32...]");
            eprintln!("       probe scorecard [arm64|riscv64|wasm32]");
            eprintln!("       probe footprint <file.ssa> [riscv]     (--platform=NAME selects a variant everywhere)");
            eprintln!("       probe boot <file.ssa> [riscv|arm]      bare metal on qemu: fn __start() runs, the UART is the output");
            eprintln!("       (-O<n> selects the optimization level on any command;");
            eprintln!("        --int=i32|i64 sets the abstract 'int' replacement policy,");
            eprintln!("        --float=f16|bf16|f32|f64|E,M the abstract 'float' one,");
            eprintln!("        --fixed=I,F the abstract 'fixed' one, --unit=N and --sunit=N the unit ones,");
            eprintln!("        --rational=N,D the rational one, --scalar=float|fixed|rational|unit|sunit");
            eprintln!("        which family a bare 'scalar' is;");
            eprintln!("        --soft compiles every library call as a call, ignoring the");
            eprintln!("        platform's native instructions)");
            ExitCode::FAILURE
        }
    }
}

/// a function argument on the command line: decimal or 0x hex, negatives allowed
fn parse_arg(a: &str) -> Result<i64, ()> {
    let (neg, t) = match a.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, a),
    };
    let v = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16),
        None => t.parse::<u64>(),
    }
    .map_err(|_| ())? as i64;
    Ok(if neg { v.wrapping_neg() } else { v })
}

fn cmd_parse(path: &str, policy: ssa::Policy) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
    let mut module = match ssa::parse_with(&ssa::with_prelude(&src), &policy) {
        Ok(m) => m,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
    // abstract types resolve under the same policy as every other command
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

fn load_module(path: &str, level: usize, policy: ssa::Policy) -> Result<ssa::Module, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let mut module = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| format!("{}: {}", path, e))?;
    ssa::resolve_types(&mut module, &policy);
    ssa::verify(&module).map_err(|errs| format!("{}: {}", path, errs.join("\n")))?;
    opt::optimize(&mut module, level);
    ssa::verify(&module)
        .map_err(|errs| format!("{}: after optimization: {}", path, errs.join("\n")))?;
    Ok(module)
}

/// Compile at every pipeline prefix: the gradual-optimization story in one
/// table — level 0 is the instant baseline, each row adds one pass.
fn cmd_tiers(path: &str, policy: ssa::Policy) -> ExitCode {
    let enc = match emit::Encoder::load(ENCODINGS) {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    println!("{:<7} {:<44} {:>8} {:>10}", "level", "passes", "bytes", "time");
    let _warmup = load_module(path, opt::MAX_LEVEL, policy); // touch caches before timing
    for level in 0..=opt::MAX_LEVEL {
        let t0 = std::time::Instant::now();
        let module = match load_module(path, level, policy) {
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

fn cmd_compile(path: &str, level: usize, policy: ssa::Policy) -> ExitCode {
    let result = load_module(path, level, policy).and_then(|module| {
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

fn cmd_run(path: &str, fname: &str, fargs: &[i64], level: usize, policy: ssa::Policy) -> ExitCode {
    let result = load_module(path, level, policy).and_then(|module| {
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
fn cmd_live(path: &str, fname: &str, fargs: &[i64], policy: ssa::Policy) -> ExitCode {
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
                    let mut m = ssa::parse_with(&ssa::with_prelude(&src), &policy).map_err(|e| e.to_string())?;
                    ssa::resolve_types(&mut m, &policy);
                    ssa::verify(&m).map_err(|e| e.join("; "))?;
                    Ok(m)
                })();
                match parsed {
                    Ok(m) => {
                        let t0 = std::time::Instant::now();
                        ar.natives = platform::Platform::arm64().natives(&m);
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
