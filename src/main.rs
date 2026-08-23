mod emit;
mod emit_rv;
mod emit_wasm;
mod learn;
mod oracle;
mod regalloc;
mod ssa;
mod suite;
mod target;
mod wlearn;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("parse") if args.len() >= 2 => cmd_parse(&args[1]),
        Some("learn") if args.len() >= 2 => {
            let out = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|i| args.get(i + 1))
                .cloned();
            cmd_learn(&args[1], out.as_deref())
        }
        Some("compile") if args.len() >= 2 => cmd_compile(&args[1]),
        Some("run") if args.len() >= 3 => {
            let fargs: Result<Vec<i64>, _> = args[3..].iter().map(|a| a.parse()).collect();
            match fargs {
                Ok(fargs) => cmd_run(&args[1], &args[2], &fargs),
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
            match suite::run_dir(dir, backend) {
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
            ExitCode::FAILURE
        }
    }
}

fn cmd_parse(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
    let module = match ssa::parse(&src) {
        Ok(m) => m,
        Err(e) => return fail(&format!("{}: {}", path, e)),
    };
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

fn load_module(path: &str) -> Result<ssa::Module, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let module = ssa::parse(&src).map_err(|e| format!("{}: {}", path, e))?;
    ssa::verify(&module).map_err(|errs| format!("{}: {}", path, errs.join("\n")))?;
    Ok(module)
}

fn cmd_compile(path: &str) -> ExitCode {
    let result = load_module(path).and_then(|module| {
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
                println!("@{} ({} bytes):", name, end - off);
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

fn cmd_run(path: &str, fname: &str, fargs: &[i64]) -> ExitCode {
    let result = load_module(path).and_then(|module| {
        let nrets = module
            .func(fname)
            .map(|f| f.rets.len())
            .ok_or_else(|| format!("no function @{} in {}", fname, path))?;
        let enc = emit::Encoder::load(ENCODINGS)?;
        let compiled = emit::compile(&module, &enc)?;
        let jit = emit::jit::JitCode::new(&compiled)?;
        match nrets {
            2 => jit.call2(fname, fargs).map(|(a, b)| format!("{}, {}", a, b)),
            n if n <= 1 => jit.call(fname, fargs).map(|v| v.to_string()),
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

fn fail(msg: &str) -> ExitCode {
    eprintln!("{}", msg);
    ExitCode::FAILURE
}
