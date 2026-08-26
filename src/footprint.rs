//! The instruction footprint of a compiled module: which learned
//! templates its code actually uses, and how often. Decoding runs each
//! emitted word against every template's fixed bits (the bits no field
//! occupies), the reverse of encoding. It is how a platform variant is
//! checked to keep its word — a program compiled for rv64i must show no
//! `mul`, `div` or `f*` — and a plain answer to "what does this program
//! need from the machine?"

use crate::emit::{self, Encoder};
use crate::platform::Platform;
use crate::ssa::Module;
use std::collections::BTreeMap;

/// template text -> count, over the code emitted for `target`
pub fn footprint(module: &Module, target: &str, platform: &Platform) -> Result<BTreeMap<String, usize>, String> {
    let enc = Encoder::load(&format!("targets/{}.encodings.json", target))?;
    let compiled = match target {
        "arm64" => emit::compile_with(module, &enc, platform)?,
        "riscv64" => crate::emit_rv::compile_with(module, &enc, platform)?,
        t => return Err(format!("no footprint for {} yet (fixed-width targets only)", t)),
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for chunk in compiled.code[..compiled.code_end].chunks_exact(4) {
        let word = u32::from_le_bytes(chunk.try_into().unwrap());
        let matches = enc.decode(word);
        let key = match matches.first() {
            Some(t) => t.to_string(),
            None => format!("(unknown word {:08x})", word),
        };
        *counts.entry(key).or_default() += 1;
    }
    Ok(counts)
}

/// the mnemonics used, for a quick conformance check
#[cfg_attr(not(test), allow(dead_code))]
pub fn mnemonics(counts: &BTreeMap<String, usize>) -> Vec<String> {
    let mut out: Vec<String> = counts.keys().map(|t| t.split_whitespace().next().unwrap_or("").to_string()).collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use crate::platform::Platform;
    use crate::ssa::{self, Policy, Type};

    /// every suite file compiled for rv64i uses nothing from M, F or D
    #[test]
    fn rv64i_uses_only_the_base_integer_isa() {
        let platform = Platform::load_named("rv64i").unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir("suite").unwrap().flatten() {
            let src = std::fs::read_to_string(entry.path()).unwrap();
            let policy = platform.adjust(Policy::new(Type::I64).unwrap());
            let mut m = ssa::parse_with(&ssa::with_prelude(&src), &policy).unwrap();
            ssa::resolve_types(&mut m, &policy);
            crate::opt::optimize(&mut m, crate::opt::MAX_LEVEL);
            let counts = super::footprint(&m, "riscv64", &platform).unwrap();
            for mn in super::mnemonics(&counts) {
                seen.insert(mn);
            }
        }
        let forbidden: Vec<&String> = seen
            .iter()
            .filter(|m| m.starts_with('f') && *m != "fence" || matches!(m.as_str(), "mul" | "mulw" | "mulh" | "div" | "divu" | "divw" | "divuw" | "rem" | "remu" | "remw" | "remuw"))
            .collect();
        assert!(forbidden.is_empty(), "rv64i code uses {:?}", forbidden);
        assert!(!seen.iter().any(|m| m.starts_with("(unknown")), "undecodable words: {:?}", seen);
    }
}
