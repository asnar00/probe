//! A store: one flat folder of feature folders, each `name/name.md` and
//! `name/name.zero` (zero.md section 2, structure.md). The prose gives
//! the parent, the layer, the origins with their timestamps — which are
//! the composition order — and, under `## testing`, the cases. The code
//! is parsed into a syntax tree per feature. Nothing here has meaning
//! yet: that is the lowering's.
// the tree carries every construct of zero.md; the plan items lower them one at a time
#![allow(dead_code)]

use super::lex::{self, Error};
use super::syntax::{self, Feature};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct Store {
    pub path: PathBuf,
    /// the features in composition order: earliest origin first
    pub features: Vec<FeatureDoc>,
}

pub struct FeatureDoc {
    pub name: String,
    pub parent: Option<String>,
    pub layer: Option<String>,
    pub origins: Vec<Origin>,
    pub cases: Vec<Case>,
    pub code: Feature,
    pub md_file: String,
}

pub struct Origin {
    pub when: String,
    pub text: String,
}

pub struct Case {
    pub line: usize,
    /// the line as written, for reporting
    pub text: String,
    pub call: syntax::Expr,
    pub expect: Expect,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expect {
    Values(Vec<i64>),
    Text(String),
    Check,
}

/// Read a store: every folder with a `.md` and a `.zero` of its own name.
pub fn read(dir: &Path) -> Result<Store, Error> {
    let sdir = dir.display().to_string();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| lex::error(&sdir, 0, format!("{}", e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    if folders.is_empty() {
        return Err(lex::error(&sdir, 0, "no feature folders in the store"));
    }
    // a type may be declared by any feature: collect the names first
    let mut types: HashSet<String> = HashSet::new();
    let mut sources = Vec::new();
    for f in &folders {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let zero = f.join(format!("{}.zero", name));
        let md = f.join(format!("{}.md", name));
        let zfile = zero.display().to_string();
        let mfile = md.display().to_string();
        let code = std::fs::read_to_string(&zero).map_err(|e| lex::error(&zfile, 0, format!("{}", e)))?;
        let prose = std::fs::read_to_string(&md).map_err(|e| lex::error(&mfile, 0, format!("{}", e)))?;
        types.extend(syntax::declared_types(&code));
        sources.push((name, zfile, code, mfile, prose));
    }
    let mut features = Vec::new();
    for (name, zfile, code, mfile, prose) in sources {
        let feature = syntax::parse_feature(&name, &code, &zfile, &types)?;
        let doc = read_prose(&name, &prose, &mfile, &types, feature)?;
        features.push(doc);
    }
    features.sort_by(|a, b| a.origins[0].when.cmp(&b.origins[0].when).then(a.name.cmp(&b.name)));
    Ok(Store { path: dir.to_path_buf(), features })
}

fn read_prose(name: &str, prose: &str, file: &str, types: &HashSet<String>, code: Feature) -> Result<FeatureDoc, Error> {
    let mut parent = None;
    let mut layer = None;
    let mut origins: Vec<Origin> = Vec::new();
    let mut cases = Vec::new();
    let mut section = String::new();
    let mut in_head = true;
    for (i, line) in prose.lines().enumerate() {
        let ln = i + 1;
        let t = line.trim();
        if let Some(h) = t.strip_prefix("## ") {
            section = h.trim().to_string();
            in_head = false;
            continue;
        }
        if in_head {
            if let Some(p) = t.strip_prefix("parent:") {
                parent = Some(p.trim().to_string());
            } else if let Some(l) = t.strip_prefix("layer:") {
                layer = Some(l.trim().to_string());
            } else if let Some(o) = t.strip_prefix('>') {
                let when = o
                    .split(|c: char| c.is_whitespace() || c == ')' || c == '(')
                    .find(|w| w.len() >= 10 && w.as_bytes()[4] == b'-' && w[..4].chars().all(|c| c.is_ascii_digit()))
                    .map(|w| w.to_string());
                let Some(when) = when else {
                    return Err(lex::error(file, ln, "an origin needs its timestamp: `> (where) YYYY-MM-DDTHH:MM:SS`"));
                };
                origins.push(Origin { when, text: o.trim().to_string() });
            } else if let Some(last) = origins.last_mut() {
                if !t.is_empty() {
                    last.text.push('\n');
                    last.text.push_str(t);
                }
            }
        } else if section == "testing" {
            if let Some(c) = t.strip_prefix('>') {
                cases.push(parse_case(c, file, ln, types)?);
            }
        }
    }
    if origins.is_empty() {
        return Err(lex::error(file, 0, format!("feature {} has no origin: a `> (where) when` line before the first section", name)));
    }
    // composition order is the earliest origin
    origins.sort_by(|a, b| a.when.cmp(&b.when));
    Ok(FeatureDoc { name: name.to_string(), parent, layer, origins, cases, code, md_file: file.to_string() })
}

/// `>call(args) → result`: the result a number or several, a quoted
/// string (what `print` produced), or `check` (the call must trap)
fn parse_case(text: &str, file: &str, line: usize, types: &HashSet<String>) -> Result<Case, Error> {
    let (call, expect) = text
        .split_once('→')
        .or_else(|| text.split_once("->"))
        .ok_or_else(|| lex::error(file, line, "a case is `>call(args) → result`"))?;
    let expect = expect.trim();
    let expect = if expect == "check" {
        Expect::Check
    } else if expect.starts_with('"') {
        let mut toks = Vec::new();
        lex::lex_line(expect, line, file, &mut toks)?;
        match toks.as_slice() {
            [lex::Token { tok: lex::Tok::Str(s), .. }] => Expect::Text(s.clone()),
            _ => return Err(lex::error(file, line, "a text result is one quoted string")),
        }
    } else {
        let mut vals = Vec::new();
        for v in expect.split(',') {
            let v = v.trim();
            let (neg, t) = match v.strip_prefix('-') {
                Some(t) => (true, t),
                None => (false, v),
            };
            let n = match t.strip_prefix("0x") {
                Some(h) => u64::from_str_radix(h, 16).map(|v| v as i64),
                None => t.parse::<u64>().map(|v| v as i64),
            }
            .map_err(|_| lex::error(file, line, format!("a result is a number, a quoted string or `check`, not '{}'", v)))?;
            vals.push(if neg { n.wrapping_neg() } else { n });
        }
        Expect::Values(vals)
    };
    let call_expr = syntax::parse_call(call.trim(), file, line, types)?;
    Ok(Case { line, text: text.trim().to_string(), call: call_expr, expect })
}
