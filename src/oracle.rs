//! The oracle: something that turns assembly text into machine-code bytes.
//!
//! The learner only ever talks to this interface, so a target is probeable
//! as long as *some* assembler for it exists. First implementation: llvm-mc,
//! which covers ARM (all profiles), RISC-V, Xtensa, MIPS, x86, ... via the
//! target triple alone.

use std::io::Write;
use std::process::{Command, Stdio};

pub trait Oracle {
    /// Assemble each line independently. Returns, per line, the encoded
    /// bytes or None if the assembler rejected it (or produced something
    /// unresolved, e.g. a relocation).
    fn assemble(&self, lines: &[String]) -> Result<Vec<Option<Vec<u8>>>, String>;
}

pub struct LlvmMc {
    bin: String,
    triple: String,
    attr: Option<String>,
}

impl LlvmMc {
    pub fn new(triple: &str, attr: Option<&str>) -> Result<LlvmMc, String> {
        let bin = ["llvm-mc", "/opt/homebrew/opt/llvm/bin/llvm-mc"]
            .iter()
            .find(|b| {
                Command::new(b)
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            })
            .ok_or("llvm-mc not found (install llvm: `brew install llvm`)")?
            .to_string();
        Ok(LlvmMc {
            bin,
            triple: triple.to_string(),
            attr: attr.map(str::to_string),
        })
    }
}

impl Oracle for LlvmMc {
    fn assemble(&self, lines: &[String]) -> Result<Vec<Option<Vec<u8>>>, String> {
        let mut cmd = Command::new(&self.bin);
        cmd.arg(format!("--triple={}", self.triple))
            .arg("--show-encoding")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(attr) = &self.attr {
            cmd.arg(format!("--mattr={}", attr));
        }
        let mut child = cmd.spawn().map_err(|e| format!("spawn llvm-mc: {}", e))?;
        {
            let stdin = child.stdin.as_mut().unwrap();
            for line in lines {
                writeln!(stdin, "{}", line).map_err(|e| format!("write to llvm-mc: {}", e))?;
            }
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("wait for llvm-mc: {}", e))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        // Errors are reported as "<stdin>:LINE:COL: error: ..." and the
        // assembler carries on, so failed lines simply produce no encoding.
        let mut failed = vec![false; lines.len()];
        for eline in stderr.lines() {
            if let Some(rest) = eline.strip_prefix("<stdin>:") {
                if let Some(n) = rest
                    .split(':')
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if eline.contains("error") && n >= 1 && n <= lines.len() {
                        failed[n - 1] = true;
                    }
                }
            }
        }

        // Successful lines emit "// encoding: [0x..,0x..]" in input order.
        // Unresolved operands show up as bit placeholders ("0bAAAA...")
        // rather than hex bytes; treat those as failures too.
        let mut encodings = stdout.lines().filter_map(|l| {
            let (_, enc) = l.split_once("encoding: [")?;
            let enc = enc.split(']').next()?;
            Some(
                enc.split(',')
                    .map(|tok| {
                        let tok = tok.trim();
                        tok.strip_prefix("0x")
                            .and_then(|h| u8::from_str_radix(h, 16).ok())
                    })
                    .collect::<Option<Vec<u8>>>(),
            )
        });

        let mut results = Vec::with_capacity(lines.len());
        for i in 0..lines.len() {
            if failed[i] {
                results.push(None);
            } else {
                match encodings.next() {
                    Some(bytes) => results.push(bytes), // None here = unresolved
                    None => results.push(None),
                }
            }
        }
        Ok(results)
    }
}
