//! Target seed files: the only per-target input to the probe system.
//!
//! A seed describes how to *spell* things for one target's assembler —
//! register names and instruction templates with typed holes. Everything
//! else (field positions, fixed bits) is learned, never written down.
//!
//! Format (line-based, `#` comments):
//!
//! ```text
//! triple arm64
//! width 4                      # bytes per instruction
//! attr  +m                     # optional -mattr for the assembler
//! reg x = x0..x30
//! inst add {x}, {x}, {x}
//! inst add {x}, {x}, #{i 0..4095}
//! inst cset {x}, {e eq|ne|lt|ge}
//! ```
//!
//! Holes: `{class}` register, `{i lo..hi}` or `{i lo..hi /step}` immediate
//! (lo may be negative), `{e a|b|c}` enumerated literal.

use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug)]
pub enum Seg {
    Text(String),
    Slot(Slot),
}

#[derive(Clone, Debug)]
pub enum Slot {
    Reg { class: String },
    Imm { lo: i64, hi: i64, step: i64 },
    Enum { choices: Vec<String> },
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Slot::Reg { class } => write!(f, "{{{}}}", class),
            Slot::Imm { lo, hi, step } if *step == 1 => write!(f, "{{i {}..{}}}", lo, hi),
            Slot::Imm { lo, hi, step } => write!(f, "{{i {}..{} /{}}}", lo, hi, step),
            Slot::Enum { choices } => write!(f, "{{e {}}}", choices.join("|")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Shape {
    pub text: String, // the template as written in the seed
    pub segs: Vec<Seg>,
}

impl Shape {
    pub fn slots(&self) -> Vec<&Slot> {
        self.segs
            .iter()
            .filter_map(|s| match s {
                Seg::Slot(slot) => Some(slot),
                Seg::Text(_) => None,
            })
            .collect()
    }

    /// Render the template with one value per slot, in order.
    /// Register/enum values are indices; immediate values are *units*
    /// (the actual immediate is units * step).
    pub fn render(&self, target: &Target, values: &[i64]) -> String {
        let mut out = String::new();
        let mut vi = 0;
        for seg in &self.segs {
            match seg {
                Seg::Text(t) => out.push_str(t),
                Seg::Slot(slot) => {
                    let v = values[vi];
                    vi += 1;
                    match slot {
                        Slot::Reg { class } => out.push_str(&target.regs[class][v as usize]),
                        Slot::Imm { step, .. } => out.push_str(&(v * step).to_string()),
                        Slot::Enum { choices } => out.push_str(&choices[v as usize]),
                    }
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub triple: String,
    pub attr: Option<String>,
    pub width: usize,
    pub regs: HashMap<String, Vec<String>>,
    pub shapes: Vec<Shape>,
}

pub fn parse_seed(name: &str, src: &str) -> Result<Target, String> {
    let mut target = Target {
        name: name.to_string(),
        triple: String::new(),
        attr: None,
        width: 4,
        regs: HashMap::new(),
        shapes: Vec::new(),
    };
    for (lineno, raw) in src.lines().enumerate() {
        // ';' comments — '#' can't be the comment char, it appears in asm templates
        let line = raw.split(';').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let err = |msg: String| format!("{}:{}: {}", name, lineno + 1, msg);
        let (kw, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match kw {
            "triple" => target.triple = rest.to_string(),
            "attr" => target.attr = Some(rest.to_string()),
            "width" => {
                target.width = rest
                    .parse()
                    .map_err(|_| err(format!("bad width '{}'", rest)))?;
                if target.width == 0 || target.width > 8 {
                    return Err(err("width must be 1..=8 bytes".into()));
                }
            }
            "reg" => {
                let (cname, spec) = rest
                    .split_once('=')
                    .ok_or_else(|| err("expected 'reg name = regs...'".into()))?;
                let mut names = Vec::new();
                for tok in spec.split_whitespace() {
                    if let Some((a, b)) = tok.split_once("..") {
                        expand_range(a, b, &mut names).map_err(|m| err(m))?;
                    } else {
                        names.push(tok.to_string());
                    }
                }
                if names.is_empty() {
                    return Err(err("empty register class".into()));
                }
                target.regs.insert(cname.trim().to_string(), names);
            }
            "inst" => {
                let shape = parse_template(rest, &target.regs).map_err(err)?;
                target.shapes.push(shape);
            }
            _ => return Err(err(format!("unknown directive '{}'", kw))),
        }
    }
    if target.triple.is_empty() {
        return Err(format!("{}: no 'triple' directive", name));
    }
    Ok(target)
}

/// Expand "x0".."x30" into x0, x1, ... x30.
fn expand_range(a: &str, b: &str, out: &mut Vec<String>) -> Result<(), String> {
    fn split_num(s: &str) -> Option<(&str, u32)> {
        let idx = s.find(|c: char| c.is_ascii_digit())?;
        Some((&s[..idx], s[idx..].parse().ok()?))
    }
    let (pa, na) = split_num(a).ok_or_else(|| format!("bad register range start '{}'", a))?;
    let (pb, nb) = split_num(b).ok_or_else(|| format!("bad register range end '{}'", b))?;
    if pa != pb || nb < na {
        return Err(format!("bad register range '{}..{}'", a, b));
    }
    for n in na..=nb {
        out.push(format!("{}{}", pa, n));
    }
    Ok(())
}

fn parse_template(text: &str, regs: &HashMap<String, Vec<String>>) -> Result<Shape, String> {
    let mut segs = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            segs.push(Seg::Text(rest[..open].to_string()));
        }
        let close = rest[open..]
            .find('}')
            .ok_or_else(|| format!("unclosed '{{' in template '{}'", text))?
            + open;
        let inner = rest[open + 1..close].trim();
        segs.push(Seg::Slot(parse_slot(inner, regs, text)?));
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        segs.push(Seg::Text(rest.to_string()));
    }
    Ok(Shape {
        text: text.to_string(),
        segs,
    })
}

fn parse_slot(
    inner: &str,
    regs: &HashMap<String, Vec<String>>,
    template: &str,
) -> Result<Slot, String> {
    if regs.contains_key(inner) {
        return Ok(Slot::Reg {
            class: inner.to_string(),
        });
    }
    if let Some(spec) = inner.strip_prefix("i ") {
        let (range, step) = match spec.split_once('/') {
            Some((r, s)) => (
                r.trim(),
                s.trim()
                    .parse::<i64>()
                    .map_err(|_| format!("bad step in '{{{}}}'", inner))?,
            ),
            None => (spec.trim(), 1),
        };
        // split on the ".." that isn't a negative sign
        let dots = range[1..]
            .find("..")
            .map(|i| i + 1)
            .ok_or_else(|| format!("expected 'lo..hi' in '{{{}}}'", inner))?;
        let lo: i64 = range[..dots]
            .parse()
            .map_err(|_| format!("bad range in '{{{}}}'", inner))?;
        let hi: i64 = range[dots + 2..]
            .parse()
            .map_err(|_| format!("bad range in '{{{}}}'", inner))?;
        if step <= 0 || hi < lo || lo % step != 0 || hi % step != 0 {
            return Err(format!("bad immediate spec '{{{}}}'", inner));
        }
        return Ok(Slot::Imm { lo, hi, step });
    }
    if let Some(spec) = inner.strip_prefix("e ") {
        let choices: Vec<String> = spec.split('|').map(|s| s.trim().to_string()).collect();
        if choices.len() < 2 {
            return Err(format!("enum '{{{}}}' needs at least two choices", inner));
        }
        return Ok(Slot::Enum { choices });
    }
    Err(format!(
        "unknown slot '{{{}}}' in '{}' (not a register class, 'i lo..hi', or 'e a|b')",
        inner, template
    ))
}
