//! The platform: what a target implements natively, as rules in
//! `targets/<target>.platform`.
//!
//! Semantics live in SSA libraries (lib/float.ssa defines what `add` on
//! a `float(8, 23)` means, with integer instructions). A platform file
//! says which library instances the target has an instruction for, and
//! in which registers values of a type live:
//!
//! ```text
//! class s = f32
//! class d = f64
//! fadd {s}, {s}, {s} = add(f32, f32) -> f32
//! fcvtzs {w}, {s} = conv(f32) -> i32
//! lt(f32, f32) -> u1
//!     fcmp a, b
//!     cset r, lo
//! ```
//!
//! A `class` line gives a register class (the slot letter the learned
//! templates use for it: `s`/`d` on arm64, `f` on riscv64, a local type
//! on wasm) to the types named; a value of such a type is allocated a
//! register of that class, so nothing moves between register files
//! except where a value really changes class (`cast` to its bits, a call
//! boundary, a pack field read). A one-line rule is a learned template
//! and the instance it computes, the template's slots being the result
//! then the arguments in order; a rule that takes several instructions
//! is a header and indented lines over `a`, `b`, `c` and `r`, with
//! literals for immediate and condition slots. Types may be written by
//! their program names (`f32` for `float(8, 23)`): the rules are matched
//! against a module with its type declarations in hand. When an emitter
//! meets a call to an instance a rule matches — by generic, parameter
//! and result types, with the generic parameters that types do not fix
//! (`round`) at their defaults — it emits the rule instead of the call,
//! and the instance itself compiles to the rule. The library body is the
//! reference the hardware path is verified against; `--soft` compiles
//! with an empty platform so both paths stay comparable.
//!
//! Variants. An ISA comes in variants — a RISC-V core without the F and
//! D extensions, or without M; an arm64 kernel that may not touch the
//! FP registers — so a platform file is grouped by extension and a
//! variant is a file that names its target, a base, and what it lacks:
//!
//! ```text
//! target riscv64
//! base riscv64
//! without M, F, D
//! ```
//!
//! `ext NAME` starts a group; every `class`, rule and `builtin` line
//! that follows belongs to it until the next `ext`. `builtin mul, div,
//! rem` says which integer opcodes the emitters may assume of a group:
//! a variant without it makes the parser dispatch those opcodes to the
//! library's `mul(W)`/`div(W)`/`rem(W)` instead, so the same program
//! runs on the smaller core, slower and correct. `--platform=NAME`
//! selects a variant for every command; `probe footprint` shows which
//! learned instructions a program actually used, which is how a variant
//! is checked to keep its word.

use crate::ssa::{Function, Module, Policy, Type};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// an operand of a rule line
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// the n-th argument
    Arg(usize),
    /// the result
    Ret,
    /// a literal: an immediate, an enum choice, or a fixed operand of
    /// the template (`vbar_el1`)
    Lit(String),
    /// the n-th temporary of the rule (`with base: ptr, v: u32` on its
    /// header): a register the emitter provides
    Tmp(usize),
}

#[derive(Clone, Debug)]
pub struct Line {
    /// a one-line rule names its template outright
    pub template: Option<String>,
    pub mnemonic: String,
    pub operands: Vec<Operand>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// generic name and the types as written (`add`, [`f32`, `f32`], `f32`)
    pub generic: String,
    pub arg_types: Vec<String>,
    pub ret_type: String,
    pub names: Vec<String>,
    pub lines: Vec<Line>,
    /// temporaries: (name, width in bits), from `with name: type, ...`
    pub temps: Vec<(String, u32)>,
}

/// a rule resolved against a module's types: canonical type spellings
/// (`float(8, 23)`), widths, and register classes
#[derive(Clone, Debug)]
pub struct Native {
    pub rule: Rule,
    pub sig: String,
    pub arg_bits: Vec<u32>,
    pub ret_bits: u32,
    /// the class of each argument and of the result (None: integer)
    pub arg_class: Vec<Option<String>>,
    pub ret_class: Option<String>,
    /// may a call site emit the rule in place? Only when every argument
    /// is an operand of it; a rule that leaves an argument where the
    /// calling convention put it (PSCI's code in x0 before `hvc`) is
    /// reached by a real call, and is the function's body
    pub inline: bool,
}

impl Native {
    pub fn bits(&self, op: &Operand) -> u32 {
        match op {
            Operand::Arg(i) => self.arg_bits[*i],
            Operand::Ret => self.ret_bits,
            Operand::Tmp(k) => self.rule.temps[*k].1,
            _ => 0,
        }
    }
    pub fn class(&self, op: &Operand) -> Option<&str> {
        match op {
            Operand::Arg(i) => self.arg_class[*i].as_deref(),
            Operand::Ret => self.ret_class.as_deref(),
            _ => None,
        }
    }
}

/// the rules of a module: callee -> native, and type -> register class
pub struct Natives {
    pub rules: HashMap<String, Native>,
    /// canonical type spelling -> class name
    pub classes: HashMap<String, String>,
    /// the platform's constants: a board's addresses
    pub consts: HashMap<String, i64>,
}

impl Natives {
    pub fn none() -> Natives {
        Natives { rules: HashMap::new(), classes: HashMap::new(), consts: HashMap::new() }
    }
    pub fn get(&self, callee: &str) -> Option<&Native> {
        self.rules.get(callee)
    }
    /// the register class of a value's type, if the platform has one
    pub fn class_of(&self, f: &Function, ty: Type) -> Option<&str> {
        if self.classes.is_empty() {
            return None;
        }
        self.classes.get(&canonical(f, ty)).map(String::as_str)
    }
}

pub struct Platform {
    /// the backend this platform is for
    pub target: String,
    /// this file's name (the variant), for messages
    pub name: String,
    /// (class name, type as written, extension group)
    classes: Vec<(String, String, String)>,
    rules: Vec<(Rule, String)>,
    /// integer opcodes some group claims for the hardware
    builtins: Vec<(String, String)>,
    /// `const name = value` lines: (name, value, group)
    consts: Vec<(String, i64, String)>,
    /// groups this variant lacks
    without: Vec<String>,
}

/// set by `--soft`: every backend's default platform becomes empty
static SOFT: AtomicBool = AtomicBool::new(false);

pub fn set_soft(soft: bool) {
    SOFT.store(soft, Ordering::Relaxed);
}

thread_local! {
    /// set by `--platform=NAME`: the variant to use for its target. Per
    /// thread, so tests choosing different cores do not see each other
    static VARIANT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn select(name: &str) {
    VARIANT.with(|v| *v.borrow_mut() = Some(name.to_string()));
}

pub fn selected() -> Option<String> {
    VARIANT.with(|v| v.borrow().clone())
}

impl Platform {
    pub fn none() -> Platform {
        Platform { target: String::new(), name: "none".into(), classes: Vec::new(), rules: Vec::new(), builtins: Vec::new(), consts: Vec::new(), without: Vec::new() }
    }

    /// the platform for a target: the selected variant when it is one of
    /// this target's, else the target's own file; nothing under --soft
    pub fn load(target: &str) -> Result<Platform, String> {
        if SOFT.load(Ordering::Relaxed) {
            return Ok(Platform::none());
        }
        if let Some(v) = selected() {
            let p = Platform::load_named(&v)?;
            if p.target == target {
                return Ok(p);
            }
        }
        Platform::load_named(target)
    }

    /// `targets/<name>.platform`, by name (a target's or a variant's)
    pub fn load_named(name: &str) -> Result<Platform, String> {
        let path = format!("targets/{}.platform", name);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path, e))?;
        let mut p = Platform::parse(&text).map_err(|e| format!("{}: {}", path, e))?;
        p.name = name.to_string();
        if p.target.is_empty() {
            p.target = name.to_string();
        }
        Ok(p)
    }

    /// does the hardware have this integer opcode (`mul`, `div`, `rem`)?
    /// A group that claims it and is lacking takes it away; a platform
    /// that says nothing has everything
    pub fn has_builtin(&self, op: &str) -> bool {
        !self.builtins.iter().any(|(o, g)| o == op && self.without.contains(g))
    }

    /// the policy as this platform needs it: integer opcodes the hardware
    /// lacks go to the library
    pub fn adjust(&self, mut policy: Policy) -> Policy {
        policy.native_mul = self.has_builtin("mul");
        policy.native_div = self.has_builtin("div") && self.has_builtin("rem");
        policy
    }

    /// the extension groups declared, and whether each is present
    pub fn extensions(&self) -> Vec<(String, bool)> {
        let mut seen: Vec<String> = Vec::new();
        for g in self.classes.iter().map(|(_, _, g)| g).chain(self.rules.iter().map(|(_, g)| g)).chain(self.builtins.iter().map(|(_, g)| g)).chain(self.consts.iter().map(|(_, _, g)| g)) {
            if !g.is_empty() && !seen.contains(g) {
                seen.push(g.clone());
            }
        }
        seen.into_iter().map(|g| { let present = !self.without.contains(&g); (g, present) }).collect()
    }

    fn present(&self, group: &str) -> bool {
        !self.without.contains(&group.to_string())
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
        let mut p = Platform::none();
        p.name = String::new();
        let mut group = String::new();
        let mut rules: Vec<(Rule, String)> = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split(';').next().unwrap().trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let at = |m: String| format!("line {}: {}", n + 1, m);
            let words: Vec<&str> = line.split_whitespace().collect();
            if raw.starts_with(' ') || raw.starts_with('\t') {
                let (rule, _) = rules.last_mut().ok_or_else(|| at("an instruction line before any rule header".into()))?;
                if rule.lines.iter().any(|l| l.template.is_some()) {
                    return Err(at("a one-line rule takes no further instructions".into()));
                }
                if rule.temps.len() > TEMPS {
                    return Err(at(format!("a rule may name {} temporaries at most", TEMPS)));
                }
                rule.lines.push(parse_line(line.trim(), &rule.names, &rule.temps).map_err(at)?);
            } else if words[0] == "target" && words.len() == 2 {
                p.target = words[1].to_string();
            } else if words[0] == "base" && words.len() == 2 {
                // inherit the base's groups; this file's lines follow
                let base = Platform::load_named(words[1]).map_err(at)?;
                if !p.target.is_empty() && base.target != p.target {
                    return Err(at(format!("base {} is for {}, not {}", words[1], base.target, p.target)));
                }
                p.target = base.target.clone();
                p.classes.extend(base.classes);
                rules.extend(base.rules);
                p.builtins.extend(base.builtins);
                p.consts.extend(base.consts);
                p.without.extend(base.without);
            } else if words[0] == "ext" && words.len() == 2 {
                group = words[1].to_string();
            } else if words[0] == "without" {
                for g in line.trim()[7..].split(',') {
                    p.without.push(g.trim().to_string());
                }
            } else if words[0] == "const" && words.len() == 4 && words[2] == "=" {
                let v = parse_int(words[3]).ok_or_else(|| at(format!("bad constant '{}'", words[3])))?;
                p.consts.push((words[1].to_string(), v, group.clone()));
            } else if words[0] == "builtin" {
                for op in line.trim()[7..].split(',') {
                    p.builtins.push((op.trim().to_string(), group.clone()));
                }
            } else if let Some(rest) = line.trim().strip_prefix("class ") {
                let (name, tys) = rest.split_once('=').ok_or_else(|| at("class wants 'class <name> = <type>, ...'".into()))?;
                for t in tys.split(',') {
                    p.classes.push((name.trim().to_string(), normalize(t), group.clone()));
                }
            } else if line.contains('{') || (line.contains('=') && !line.trim().starts_with("class")) && line.split('=').next().unwrap().contains(' ') {
                // `template = sig`
                let (template, sig) = line.rsplit_once('=').ok_or_else(|| at("expected 'template = op(types) -> type'".into()))?;
                let mut rule = parse_header(sig.trim()).map_err(at)?;
                let template = template.trim().to_string();
                let mnemonic = template.split_whitespace().next().unwrap_or("").to_string();
                let nslots = template_slots(&template).len();
                let has_ret = rule.ret_type != "()";
                let mut operands = Vec::new();
                if has_ret && nslots > 0 {
                    operands.push(Operand::Ret);
                }
                for i in 0..rule.names.len() {
                    operands.push(Operand::Arg(i));
                }
                if nslots > 0 && operands.len() != nslots {
                    return Err(at(format!("'{}' has {} slots for a result and {} arguments", template, nslots, rule.names.len())));
                }
                if nslots == 0 {
                    operands.clear();
                }
                rule.lines.push(Line { template: Some(template), mnemonic, operands });
                rules.push((rule, group.clone()));
            } else {
                rules.push((parse_header(line.trim()).map_err(at)?, group.clone()));
            }
        }
        for (r, _) in &rules {
            if r.lines.is_empty() {
                return Err(format!("rule '{}' has no instructions (say 'none' for a rule that does nothing)", r.generic));
            }
        }
        p.rules = rules;
        Ok(p)
    }

    /// the rules and classes as they apply to a module, its type
    /// declarations resolving the names the file used
    pub fn natives(&self, m: &Module) -> Natives {
        let resolve = |ty: &str| -> String {
            let mut t = normalize(ty);
            for _ in 0..8 {
                match m.types.iter().find(|d| d.name == t && d.params.is_empty()) {
                    Some(d) => t = normalize(&d.body.to_string()),
                    None => break,
                }
            }
            t
        };
        let classes: HashMap<String, String> = self.classes.iter().filter(|(_, _, g)| self.present(g)).map(|(c, t, _)| (resolve(t), c.clone())).collect();
        let defaults = Policy::new(Type::I64).unwrap();
        let consts: HashMap<String, i64> = self.consts.iter().filter(|(_, _, g)| self.present(g)).map(|(n, v, _)| (n.clone(), *v)).collect();
        let mut rules = HashMap::new();
        for f in &m.funcs {
            // an instance of a generic, or a plain function the platform
            // names outright (a board operation like PSCI's hvc)
            let (generic, args): (String, Vec<i64>) = match &f.instance {
                Some((g, a)) => (g.clone(), a.clone()),
                None => (f.name.clone(), Vec::new()),
            };
            if f.rets.len() > 1 {
                continue;
            }
            // parameters the types do not fix must be at their defaults
            let fixed_by_types = f.instance_names.iter().zip(&args).all(|(n, a)| match defaults.named(n) {
                Some(d) => *a == d,
                None => true,
            });
            if !fixed_by_types {
                continue;
            }
            let ptys: Vec<String> = f.params.iter().map(|&p| canonical(f, f.ty(p))).collect();
            let rty = f.rets.first().map(|&t| canonical(f, t)).unwrap_or_else(|| "()".into());
            for (r, g) in &self.rules {
                if !self.present(g) || r.generic != generic || r.arg_types.len() != ptys.len() {
                    continue;
                }
                let ret_matches = if rty == "()" { r.ret_type == "()" } else { resolve(&r.ret_type) == rty };
                if r.arg_types.iter().zip(&ptys).any(|(a, b)| resolve(a) != *b) || !ret_matches {
                    continue;
                }
                let arg_bits: Vec<u32> = f.params.iter().map(|&p| f.width(f.ty(p)).unwrap_or(64)).collect();
                let ret_bits = f.rets.first().map(|&t| f.width(t).unwrap_or(64)).unwrap_or(0);
                let inline = (0..ptys.len()).all(|i| r.lines.iter().any(|l| l.operands.contains(&Operand::Arg(i))));
                let native = Native {
                    rule: r.clone(),
                    sig: format!("{}({}) -> {}", generic, ptys.join(", "), rty),
                    arg_bits,
                    ret_bits,
                    arg_class: ptys.iter().map(|t| classes.get(t).cloned()).collect(),
                    ret_class: if rty == "()" { None } else { classes.get(&rty).cloned() },
                    inline,
                };
                rules.insert(f.name.clone(), native);
                break;
            }
        }
        Natives { rules, classes, consts }
    }
}

/// a type as the rule files spell it canonically: a pack by its
/// parametric origin (`float(8, 23)`, whatever alias the program gave
/// it), else its name
pub fn canonical(f: &Function, ty: Type) -> String {
    match f.pack(ty).and_then(|p| p.origin.as_ref()) {
        Some((name, args)) => format!("{}({})", name, args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")),
        None => f.tyname(ty),
    }
}

/// `add(a: f32, b: f32) -> r: f32`, or without names: `add(f32, f32) -> f32`
fn parse_header(s: &str) -> Result<Rule, String> {
    let (generic, rest) = s.split_once('(').ok_or("expected op(types) -> type")?;
    let (params, rest) = split_params(rest)?;
    let ret = rest.trim().strip_prefix("->").ok_or("expected '-> type'")?.trim();
    // `-> type with name: type, ...`: the rule's temporaries
    let (ret, temps_text) = match ret.split_once(" with ") {
        Some((r, t)) => (r.trim(), Some(t)),
        None => (ret, None),
    };
    let mut temps = Vec::new();
    for t in temps_text.into_iter().flat_map(|t| t.split(',')) {
        let (name, ty) = t.trim().split_once(':').ok_or("a temporary is 'name: type'")?;
        let bits = Type::from_name_pub(ty.trim()).and_then(|t| t.int_bits()).ok_or_else(|| format!("a temporary's type is an integer or ptr, not '{}'", ty.trim()))?;
        temps.push((name.trim().to_string(), bits));
    }
    let ret_type = normalize(ret.rsplit(':').next().unwrap());
    let mut names = Vec::new();
    let mut arg_types = Vec::new();
    for (i, p) in params.iter().enumerate() {
        match p.split_once(':') {
            Some((name, ty)) => {
                names.push(name.trim().to_string());
                arg_types.push(normalize(ty));
            }
            None => {
                names.push(["a", "b", "c", "d"].get(i).map(|s| s.to_string()).unwrap_or_else(|| format!("a{}", i)));
                arg_types.push(normalize(p));
            }
        }
    }
    Ok(Rule { generic: generic.trim().to_string(), arg_types, ret_type, names, lines: Vec::new(), temps })
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

/// the operand tokens of a rule line or a template's text: split at
/// commas, spaces and brackets, a `#` dropped — `sd t, 0(t0)` gives
/// `t`, `0`, `t0`; `[{x}, #{i}]` gives `{x}`, `{i}`
fn operand_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']')
        .map(|t| t.trim_start_matches('#'))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// `fcmp a, b` / `cset r, lo` / `xori r, r, 1` / `sd t, 0(base)` /
/// `none` (a rule that takes no instructions)
fn parse_line(s: &str, names: &[String], temps: &[(String, u32)]) -> Result<Line, String> {
    let (mnemonic, rest) = match s.split_once(char::is_whitespace) {
        Some((m, r)) => (m, r),
        None => (s, ""),
    };
    let mut operands = Vec::new();
    for tok in operand_tokens(rest) {
        let op = if tok == "r" {
            Operand::Ret
        } else if let Some(i) = names.iter().position(|n| *n == tok) {
            Operand::Arg(i)
        } else if let Some(k) = temps.iter().position(|(n, _)| *n == tok) {
            Operand::Tmp(k)
        } else {
            Operand::Lit(tok)
        };
        operands.push(op);
    }
    Ok(Line { template: None, mnemonic: mnemonic.to_string(), operands })
}

/// how many temporaries a rule may name
pub const TEMPS: usize = 4;

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

/// does a template slot take this operand? A float-class operand takes
/// the slot of its class letter; an integer operand takes `{r}` at any
/// width, `{w}` at 32 bits, `{x}` at 64 — and as the result also `{x}`
/// at 32 (a 64-bit write defines the low half; a 64-bit read of a
/// 32-bit value would not).
fn slot_takes(slot: &str, native: &Native, op: &Operand) -> Option<i64> {
    let kind = slot.split_whitespace().next().unwrap_or("");
    match op {
        Operand::Arg(_) | Operand::Ret | Operand::Tmp(_) => {
            if let Some(c) = native.class(op) {
                return (kind == c).then_some(0);
            }
            let bits = native.bits(op);
            match kind {
                "r" => Some(0),
                "w" if bits <= 32 => Some(0),
                "x" if bits > 32 || *op == Operand::Ret => Some(0),
                _ => None,
            }
        }
        Operand::Lit(l) => match kind {
            "i" => parse_int(l),
            "e" => slot.trim_start_matches('e').trim().split('|').position(|c| c == l).map(|i| i as i64),
            _ => None,
        },
    }
}

/// Resolve a rule line against a target's learned register-machine
/// templates: a one-line rule names its template; otherwise the one
/// whose mnemonic is the line's and whose slots take the operands in
/// order. Returns the template and, per slot, the operand with literals
/// turned into the slot's number.
pub fn resolve<'t>(native: &Native, line: &Line, templates: &[&'t str]) -> Result<(&'t str, Vec<(Operand, i64)>), String> {
    let candidates: Vec<&'t str> = match &line.template {
        Some(t) => templates.iter().copied().filter(|c| *c == t.as_str()).collect(),
        None => templates
            .iter()
            .copied()
            .filter(|t| t.split(|c: char| c.is_whitespace()).next().unwrap_or("") == line.mnemonic)
            .collect(),
    };
    let mut found: Option<(&str, Vec<(Operand, i64)>, bool)> = None;
    for t in candidates {
        // the template's operand text, token by token: a slot takes the
        // line's operand there; fixed text (`vbar_el1`, `lsl`, `16`)
        // must be spelled the same
        let slots = template_slots(t);
        let mut text = t.split_once(char::is_whitespace).map(|(_, r)| r.to_string()).unwrap_or_default();
        for (k, s) in slots.iter().enumerate() {
            text = text.replacen(&format!("{{{}}}", s), &format!("{{{}}}", k), 1);
        }
        let tokens = operand_tokens(&text);
        if tokens.len() != line.operands.len() {
            continue;
        }
        let mut vals = Vec::new();
        let mut ok = true;
        for (tok, op) in tokens.iter().zip(&line.operands) {
            if let Some(k) = tok.strip_prefix('{').and_then(|p| p.strip_suffix('}')).and_then(|k| k.parse::<usize>().ok()) {
                match slot_takes(slots[k], native, op) {
                    Some(v) => vals.push((op.clone(), v)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            } else if !matches!(op, Operand::Lit(l) if l == tok) {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        let exact = !slots.iter().zip(&line.operands).any(|(s, op)| s.starts_with('x') && *op == Operand::Ret && native.class(op).is_none() && native.bits(op) <= 32);
        match &found {
            None => found = Some((t, vals, exact)),
            Some((_, _, false)) if exact => found = Some((t, vals, exact)),
            _ => {}
        }
    }
    found.map(|(t, v, _)| (t, v)).ok_or_else(|| {
        let ops: Vec<String> = line.operands.iter().map(|o| format!("{:?}", o)).collect();
        match &line.template {
            Some(t) => format!("rule '{}': the learner has no template '{}', or its slots do not take the operands ({})", native.sig, t, ops.join(", ")),
            None => format!("rule '{}': no learned template for '{} {}'", native.sig, line.mnemonic, ops.join(", ")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_files_parse() {
        for t in ["arm64", "riscv64", "wasm32"] {
            let p = Platform::load_named(t).unwrap();
            assert!(p.rules.len() >= 38, "{}: {} rules", t, p.rules.len());
            assert!(!p.classes.is_empty(), "{}: no classes", t);
            assert_eq!(p.target, t);
        }
        // variants: what a base has, minus what the variant lacks
        let full = Platform::load_named("riscv64").unwrap();
        let im = Platform::load_named("rv64im").unwrap();
        let i = Platform::load_named("rv64i").unwrap();
        assert_eq!(im.target, "riscv64");
        assert!(full.has_builtin("mul") && im.has_builtin("mul") && !i.has_builtin("div"));
        let m = crate::ssa::parse_with(&crate::ssa::with_prelude("fn f(a: f32, b: f32) -> f32 {\n    r: f32 = add a, b\n    ret r\n}\n"), &crate::ssa::Policy::new(crate::ssa::Type::I64).unwrap()).unwrap();
        assert!(!full.natives(&m).rules.is_empty());
        assert!(im.natives(&m).rules.is_empty() && im.natives(&m).classes.is_empty());
        // virt (the board's constants), traps, time, M, F, D; rv64i keeps
        // virt, traps and time
        assert_eq!(full.extensions().iter().filter(|(_, present)| *present).count(), 6);
        assert_eq!(i.extensions().iter().filter(|(_, present)| *present).count(), 3);
        assert_eq!(i.natives(&m).consts.get("uart"), Some(&0x10000000));
        let nofp = Platform::load_named("arm64-nofp").unwrap();
        assert!(nofp.natives(&m).classes.is_empty());
        assert!(Platform::parse("add(f32, f32) -> f32\n").is_err()); // no instructions
        assert!(Platform::parse("    fcmp a, b\n").is_err()); // no header
        let p = Platform::parse("class s = f32\nfadd {s}, {s}, {s} = add(f32, f32) -> f32\nlt(f32, f32) -> u1\n    fcmp a, b\n    cset r, lo\n").unwrap();
        assert_eq!(p.rules.len(), 2);
        assert_eq!(p.rules[0].0.lines[0].operands, vec![Operand::Ret, Operand::Arg(0), Operand::Arg(1)]);
        assert_eq!(p.rules[1].0.lines[1].operands, vec![Operand::Ret, Operand::Lit("lo".into())]);
    }
}
