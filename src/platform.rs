//! The platform: what a target implements natively, as rules in
//! `targets/<target>.platform`.
//!
//! Semantics live in SSA libraries (lib/float.ssa defines what
//! `add(8, 23, 0)` on a float(8, 23) means, with integer instructions).
//! A platform file lists the library instances the target has hardware
//! for, each with the learned templates that compute it:
//!
//! ```text
//! add(8, 23, 0)(a: float(8, 23), b: float(8, 23)) -> r: float(8, 23)
//!     fmov s0, a
//!     fmov s1, b
//!     fadd s0, s0, s1
//!     fmov r, s0
//! ```
//!
//! `a`, `b`, `c` are the arguments and `r` the result, in the integer
//! registers (or wasm locals) the emitter chose — a float is its bits;
//! `s0`/`d0`/`f0`... are the target's scratch float registers; anything
//! else is a literal for the template's immediate or enum slot. Each
//! line is resolved against the learned templates by mnemonic and
//! operand shape (`fmov s0, a` with a 32-bit `a` is `fmov {s}, {w}`), so
//! a rule can only name instructions the learner has verified.
//!
//! When an emitter meets a call to an instance a rule matches (by its
//! full signature: generic, width arguments, parameter and result types)
//! it emits the rule's sequence instead of the call — and the instance
//! itself compiles to that sequence, so callers from outside (the
//! harness, a JIT call by name) get the hardware too. The library body
//! is the reference the hardware path is verified against; `--soft`
//! compiles with an empty platform so both paths stay comparable.

use crate::ssa::{Function, Module};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// an operand of a rule line
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// the n-th argument, in an integer register / local
    Arg(usize),
    /// the result
    Ret,
    /// a scratch float register: its class letter and number
    Scratch(char, u32),
    /// a literal: an immediate or an enum choice
    Lit(String),
}

#[derive(Clone, Debug)]
pub struct Line {
    pub mnemonic: String,
    pub operands: Vec<Operand>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// the canonical signature: `add(8, 23, 0)(float(8, 23), float(8, 23)) -> float(8, 23)`
    pub sig: String,
    /// argument names as written, for error messages
    pub names: Vec<String>,
    /// bit widths of the arguments and the result
    pub arg_bits: Vec<u32>,
    pub ret_bits: u32,
    pub lines: Vec<Line>,
}

impl Rule {
    /// the width of a value operand (arguments and the result)
    pub fn bits(&self, op: &Operand) -> u32 {
        match op {
            Operand::Arg(i) => self.arg_bits[*i],
            Operand::Ret => self.ret_bits,
            _ => 0,
        }
    }
}

pub struct Platform {
    rules: Vec<Rule>,
}

/// set by `--soft`: every backend's default platform becomes empty
static SOFT: AtomicBool = AtomicBool::new(false);

pub fn set_soft(soft: bool) {
    SOFT.store(soft, Ordering::Relaxed);
}

impl Platform {
    pub fn none() -> Platform {
        Platform { rules: Vec::new() }
    }

    /// the target's rule file, or nothing under --soft
    pub fn load(target: &str) -> Result<Platform, String> {
        if SOFT.load(Ordering::Relaxed) {
            return Ok(Platform::none());
        }
        let path = format!("targets/{}.platform", target);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path, e))?;
        Platform::parse(&text).map_err(|e| format!("{}: {}", path, e))
    }

    pub fn arm64() -> Platform {
        Platform::load("arm64").expect("targets/arm64.platform")
    }

    pub fn riscv64() -> Platform {
        Platform::load("riscv64").expect("targets/riscv64.platform")
    }

    pub fn wasm32() -> Platform {
        Platform::load("wasm32").expect("targets/wasm32.platform")
    }

    pub fn parse(text: &str) -> Result<Platform, String> {
        let mut rules: Vec<Rule> = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split(';').next().unwrap().trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let at = |m: String| format!("line {}: {}", n + 1, m);
            if raw.starts_with(' ') || raw.starts_with('\t') {
                let rule = rules.last_mut().ok_or_else(|| at("an instruction line before any rule header".into()))?;
                rule.lines.push(parse_line(line.trim(), &rule.names).map_err(at)?);
            } else {
                rules.push(parse_header(line.trim()).map_err(at)?);
            }
        }
        for r in &rules {
            if r.lines.is_empty() {
                return Err(format!("rule '{}' has no instructions", r.sig));
            }
        }
        Ok(Platform { rules })
    }

    /// the rule for `f`, if this platform has one for its exact signature
    pub fn lookup(&self, f: &Function) -> Option<&Rule> {
        let sig = signature(f)?;
        self.rules.iter().find(|r| r.sig == sig)
    }

    /// callee name -> rule, for every function of a module
    pub fn natives(&self, m: &Module) -> HashMap<String, Rule> {
        m.funcs
            .iter()
            .filter_map(|f| self.lookup(f).map(|r| (f.name.clone(), r.clone())))
            .collect()
    }
}

/// an instantiated generic's signature in the rule files' canonical form
pub fn signature(f: &Function) -> Option<String> {
    let (generic, args) = f.instance.as_ref()?;
    if f.rets.len() != 1 {
        return None;
    }
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let params: Vec<String> = f.params.iter().map(|&p| canonical(f, f.ty(p))).collect();
    Some(format!("{}({})({}) -> {}", generic, args.join(", "), params.join(", "), canonical(f, f.rets[0])))
}

/// a type as the rule files spell it: a pack by its parametric origin
/// (`float(8, 23)`, whatever alias the program gave it), else its name
fn canonical(f: &Function, ty: crate::ssa::Type) -> String {
    match f.pack(ty).and_then(|p| p.origin.as_ref()) {
        Some((name, args)) => format!("{}({})", name, args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")),
        None => f.tyname(ty),
    }
}

/// `add(8, 23, 0)(a: float(8, 23), b: float(8, 23)) -> r: float(8, 23)`
fn parse_header(s: &str) -> Result<Rule, String> {
    let (generic, rest) = s.split_once('(').ok_or("expected generic(args)(params) -> result")?;
    let (args, rest) = rest.split_once(')').ok_or("expected ')' after width arguments")?;
    let rest = rest.trim_start().strip_prefix('(').ok_or("expected '(' before parameters")?;
    let (params, rest) = split_params(rest)?;
    let ret = rest.trim().strip_prefix("->").ok_or("expected '-> name: type'")?.trim();
    let (_, ret_ty) = ret.split_once(':').ok_or("expected 'name: type' for the result")?;
    let args: Vec<String> = args.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let mut names = Vec::new();
    let mut ptys = Vec::new();
    for p in params {
        let (name, ty) = p.split_once(':').ok_or_else(|| format!("expected 'name: type', got '{}'", p))?;
        names.push(name.trim().to_string());
        ptys.push(normalize(ty));
    }
    let arg_bits = ptys.iter().map(|t| type_bits(t)).collect::<Result<Vec<_>, _>>()?;
    let ret_ty = normalize(ret_ty);
    let ret_bits = type_bits(&ret_ty)?;
    Ok(Rule {
        sig: format!("{}({})({}) -> {}", generic.trim(), args.join(", "), ptys.join(", "), ret_ty),
        names,
        arg_bits,
        ret_bits,
        lines: Vec::new(),
    })
}

/// the parameter list up to its closing paren, honouring nested parens
fn split_params(s: &str) -> Result<(Vec<String>, &str), String> {
    let mut depth = 0;
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ')' => {
                if !s[start..i].trim().is_empty() {
                    out.push(s[start..i].trim().to_string());
                }
                return Ok((out, &s[i + 1..]));
            }
            ',' if depth == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    Err("unclosed parameter list".into())
}

/// a type's spelling as Function::tyname prints it
fn normalize(ty: &str) -> String {
    let t: String = ty.split_whitespace().collect::<Vec<_>>().join(" ");
    t.replace("( ", "(").replace(" )", ")").replace(" ,", ",").replace(",", ", ").replace(",  ", ", ")
}

/// bits of `i32`, `u1`, `float(8, 23)` (a float is 1 + E + M)
fn type_bits(ty: &str) -> Result<u32, String> {
    if let Some(inner) = ty.strip_prefix("float(").and_then(|s| s.strip_suffix(')')) {
        let v: Vec<u32> = inner.split(',').map(|x| x.trim().parse().map_err(|_| format!("bad float type '{}'", ty))).collect::<Result<_, _>>()?;
        if v.len() != 2 {
            return Err(format!("bad float type '{}'", ty));
        }
        return Ok(1 + v[0] + v[1]);
    }
    if ty == "ptr" {
        return Ok(64);
    }
    let digits = ty.strip_prefix('i').or_else(|| ty.strip_prefix('u')).ok_or_else(|| format!("unknown type '{}' in a rule (integers, ptr and float(E, M) are known)", ty))?;
    digits.parse().map_err(|_| format!("unknown type '{}'", ty))
}

/// `fadd s0, s0, s1` / `cset r, lo` / `slli r, r, 32` / `f32.add`
fn parse_line(s: &str, names: &[String]) -> Result<Line, String> {
    let (mnemonic, rest) = match s.split_once(char::is_whitespace) {
        Some((m, r)) => (m, r),
        None => (s, ""),
    };
    let mut operands = Vec::new();
    for tok in rest.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let op = if tok == "r" {
            Operand::Ret
        } else if let Some(i) = names.iter().position(|n| n == tok) {
            Operand::Arg(i)
        } else if let Some((c, num)) = scratch(tok) {
            Operand::Scratch(c, num)
        } else {
            Operand::Lit(tok.to_string())
        };
        operands.push(op);
    }
    Ok(Line { mnemonic: mnemonic.to_string(), operands })
}

/// `s0`, `d1`, `f2`: a class letter and a number
fn scratch(tok: &str) -> Option<(char, u32)> {
    let mut chars = tok.chars();
    let c = chars.next()?;
    if !matches!(c, 's' | 'd' | 'f' | 'v') {
        return None;
    }
    let rest = chars.as_str();
    if rest.is_empty() || !rest.chars().all(|d| d.is_ascii_digit()) {
        return None;
    }
    Some((c, rest.parse().ok()?))
}

/// a literal: decimal, hex, negative
fn parse_int(s: &str) -> Option<i64> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s),
    };
    let v = match body.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).ok()?,
        None => body.parse::<i64>().ok()?,
    };
    Some(if neg { -v } else { v })
}

/// the slots of a template, in order: `{w}`, `{x}`, `{s}`, `{d}`, `{f}`,
/// `{r}` register classes, `{i lo..hi}` immediates, `{e a|b}` enums
pub fn template_slots(template: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(s) = rest.find('{') {
        let Some(e) = rest[s..].find('}') else { break };
        out.push(&rest[s + 1..s + e]);
        rest = &rest[s + e + 1..];
    }
    out
}

/// Resolve a rule line against a target's learned register-machine
/// templates: the one whose mnemonic is the line's and whose slots take
/// the operands in order. A value operand matches `{r}` at any width,
/// `{w}` at 32 bits, `{x}` at 64 — and the result also matches `{x}` at
/// 32 (a 64-bit write defines the low half; a 64-bit read of a 32-bit
/// value would not). Returns the template and, per slot, the operand
/// with literals turned into the slot's number.
pub fn resolve<'t>(rule: &Rule, line: &Line, templates: &[&'t str]) -> Result<(&'t str, Vec<(Operand, i64)>), String> {
    let mut found = None;
    for &t in templates {
        let mnemonic = t.split(|c: char| c.is_whitespace()).next().unwrap_or("");
        if mnemonic != line.mnemonic {
            continue;
        }
        let slots = template_slots(t);
        if slots.len() != line.operands.len() {
            continue;
        }
        let mut vals = Vec::new();
        let mut ok = true;
        for (slot, op) in slots.iter().zip(&line.operands) {
            let v = match (slot.split_whitespace().next().unwrap_or(""), op) {
                ("r", Operand::Arg(_) | Operand::Ret) => 0,
                ("w", Operand::Arg(_) | Operand::Ret) if rule.bits(op) <= 32 => 0,
                ("x", Operand::Arg(_)) if rule.bits(op) > 32 => 0,
                ("x", Operand::Ret) => 0,
                ("s", Operand::Scratch('s', n)) | ("d", Operand::Scratch('d', n)) | ("f", Operand::Scratch('f', n)) | ("v", Operand::Scratch('v', n)) => *n as i64,
                ("i", Operand::Lit(l)) => match parse_int(l) {
                    Some(v) => v,
                    None => {
                        ok = false;
                        break;
                    }
                },
                ("e", Operand::Lit(l)) => {
                    let choices = slot.trim_start_matches('e').trim();
                    match choices.split('|').position(|c| c == l) {
                        Some(i) => i as i64,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                _ => {
                    ok = false;
                    break;
                }
            };
            vals.push((op.clone(), v));
        }
        if !ok {
            continue;
        }
        // an exact width match beats the result-as-x allowance
        let exact = !slots.iter().zip(&line.operands).any(|(s, op)| s.starts_with('x') && *op == Operand::Ret && rule.bits(op) <= 32);
        match &found {
            None => found = Some((t, vals, exact)),
            Some((_, _, false)) if exact => found = Some((t, vals, exact)),
            _ => {}
        }
    }
    found.map(|(t, v, _)| (t, v)).ok_or_else(|| {
        let ops: Vec<String> = line.operands.iter().map(|o| format!("{:?}", o)).collect();
        format!("rule '{}': no learned template for '{} {}'", rule.sig, line.mnemonic, ops.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATES: [&str; 6] = [
        "fmov {s}, {w}",
        "fmov {w}, {s}",
        "fadd {s}, {s}, {s}",
        "cset {x}, {e eq|ne|lt|le|gt|ge|lo|ls|hi|hs}",
        "fcmp {s}, {s}",
        "xori {r}, {r}, {i -2048..2047}",
    ];

    #[test]
    fn rules_parse_and_resolve_to_learned_templates() {
        let p = Platform::parse(
            "; a comment\nadd(8, 23, 0)(a: float(8, 23), b: float(8, 23)) -> r: float(8, 23)\n    fmov s0, a\n    fmov s1, b\n    fadd s0, s0, s1 ; in place\n    fmov r, s0\n\nlt(8, 23, 0)(a: float(8, 23), b: float(8, 23)) -> r: u1\n    fmov s0, a\n    fmov s1, b\n    fcmp s0, s1\n    cset r, lo\n",
        )
        .unwrap();
        assert_eq!(p.rules.len(), 2);
        let add = &p.rules[0];
        assert_eq!(add.sig, "add(8, 23, 0)(float(8, 23), float(8, 23)) -> float(8, 23)");
        assert_eq!(add.arg_bits, vec![32, 32]);
        let (t, vals) = resolve(add, &add.lines[0], &TEMPLATES).unwrap();
        assert_eq!(t, "fmov {s}, {w}");
        assert_eq!(vals, vec![(Operand::Scratch('s', 0), 0), (Operand::Arg(0), 0)]);
        let (t, _) = resolve(add, &add.lines[3], &TEMPLATES).unwrap();
        assert_eq!(t, "fmov {w}, {s}");
        // the u1 result takes the x-form cset; the condition is an enum index
        let lt = &p.rules[1];
        let (t, vals) = resolve(lt, &lt.lines[3], &TEMPLATES).unwrap();
        assert_eq!(t, "cset {x}, {e eq|ne|lt|le|gt|ge|lo|ls|hi|hs}");
        assert_eq!(vals[1], (Operand::Lit("lo".into()), 6));
        // a literal immediate
        let line = parse_line("xori r, r, 1", &[]).unwrap();
        let (_, vals) = resolve(lt, &line, &TEMPLATES).unwrap();
        assert_eq!(vals[2].1, 1);
        // an instruction the learner has no template for is refused
        let line = parse_line("fsub s0, s0, s1", &[]).unwrap();
        assert!(resolve(add, &line, &TEMPLATES).unwrap_err().contains("no learned template"));
        // a 64-bit argument does not fit a {w} slot
        let wide = Platform::parse("f(1)(a: i64) -> r: u1\n    fmov s0, a\n").unwrap();
        assert!(resolve(&wide.rules[0], &wide.rules[0].lines[0], &TEMPLATES).is_err());
    }

    #[test]
    fn rule_files_parse_and_name_the_library() {
        for t in ["arm64", "riscv64", "wasm32"] {
            let p = Platform::load(t).unwrap();
            assert!(p.rules.len() >= 38, "{}: {} rules", t, p.rules.len());
            assert!(p.rules.iter().any(|r| r.sig == "add(8, 23, 0)(float(8, 23), float(8, 23)) -> float(8, 23)"));
        }
        assert!(Platform::parse("add(8, 23, 0)(a: float(8, 23)) -> r: float(8, 23)\n").is_err()); // no instructions
        assert!(Platform::parse("    fmov s0, a\n").is_err()); // no header
    }
}
