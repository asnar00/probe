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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Store {
    pub path: PathBuf,
    /// the features in composition order: earliest origin first
    pub features: Vec<FeatureDoc>,
    /// the layers, lowest first, from `order.md` beside the feature
    /// folders (log 28, 42); empty when there is none, and every layer
    /// is then one level
    pub layers: Vec<String>,
    /// the product's settings (log 41): `bound <function words>: N`
    /// lines in `product.md` beside the feature folders, the words and
    /// the trip count; empty when there is no product file
    pub product: Vec<(Vec<String>, i64)>,
    pub product_file: String,
}

impl Store {
    /// a layer's height: its place in `order.md`, or 0 for every
    /// layer when there is no list
    pub fn rank(&self, layer: &str) -> usize {
        self.layers.iter().position(|l| l == layer).unwrap_or(0)
    }
}

pub struct FeatureDoc {
    pub name: String,
    pub parent: Option<String>,
    /// the feature's layer: its own `layer:` line, or its parent's
    pub layer: Option<String>,
    pub origins: Vec<Origin>,
    pub cases: Vec<Case>,
    pub code: Feature,
    pub md_file: String,
}

pub struct Origin {
    pub when: String,
    pub text: String,
    /// `same commit` after the timestamp (log 42): this feature entered
    /// with another at the same time, and name order is intended
    pub same_commit: bool,
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

/// the compiler's own feature (log 31): the platform functions every
/// store has, `print` first among them, with bodies in the IR; composed
/// first, in the lowest layer, `platform`
const PLATFORM_ZERO: &str = include_str!("platform.zero");
const PLATFORM_FILE: &str = "src/zero/platform.zero";

fn builtin_platform(types: &HashSet<String>) -> Result<FeatureDoc, Error> {
    let code = syntax::parse_feature("platform", PLATFORM_ZERO, PLATFORM_FILE, types)?;
    let origin = Origin { when: "0000-00-00T00:00:00".into(), text: "(probe) the compiler's own feature: the platform functions every store has".into(), same_commit: false };
    Ok(FeatureDoc { name: "platform".into(), parent: None, layer: Some("platform".into()), origins: vec![origin], cases: Vec::new(), code, md_file: PLATFORM_FILE.into() })
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
        if name == "platform" {
            return Err(lex::error(&f.display().to_string(), 0, "'platform' is the compiler's own feature, composed into every store: name this one otherwise"));
        }
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
    // composition order is the origin's time (log 5): a tie is refused
    // unless every tied origin says `same commit`, when name order is
    // the ledger's rule for one commit (question 16, log 42)
    for pair in features.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.origins[0].when == b.origins[0].when && !(a.origins[0].same_commit && b.origins[0].same_commit) {
            return Err(lex::error(&b.md_file, 0, format!("features {} and {} share the origin {}: composition order is the origin's time, so give one a later origin, or write `same commit` after both timestamps to order them by name", a.name, b.name, a.origins[0].when)));
        }
    }
    let layers = read_order(dir)?;
    check_tree(&mut features, &layers)?;
    features.insert(0, builtin_platform(&types)?);
    let (product, product_file) = read_product(dir)?;
    Ok(Store { path: dir.to_path_buf(), features, layers, product, product_file })
}

/// `product.md`: what the product sets and no feature says (zero.md
/// section 1, log 41). In the bootstrap it is one kind of line, `bound
/// <function words>: N`, a trip count for every loop of that function
/// the IR does not show the count of; every other line is prose
fn read_product(dir: &Path) -> Result<(Vec<(Vec<String>, i64)>, String), Error> {
    let path = dir.join("product.md");
    let file = path.display().to_string();
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok((Vec::new(), file)) };
    let mut settings: Vec<(Vec<String>, i64)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(rest) = line.trim().strip_prefix("bound ") else { continue };
        let Some((words, n)) = rest.split_once(':') else {
            return Err(lex::error(&file, i + 1, "a bound is `bound <function words>: N`"));
        };
        let words: Vec<String> = words.split_whitespace().map(str::to_string).collect();
        let n: i64 = n.trim().parse().map_err(|_| lex::error(&file, i + 1, format!("a bound is a positive number, not '{}'", n.trim())))?;
        if words.is_empty() || n <= 0 {
            return Err(lex::error(&file, i + 1, "a bound is `bound <function words>: N`, N positive"));
        }
        if settings.iter().any(|(w, _)| *w == words) {
            return Err(lex::error(&file, i + 1, format!("'{}' is bounded twice", words.join(" "))));
        }
        settings.push((words, n));
    }
    Ok((settings, file))
}

/// `order.md`: the store's layers, one `- name` per line, lowest first,
/// after a line that says so (log 42)
fn read_order(dir: &Path) -> Result<Vec<String>, Error> {
    if dir.join("layers.md").exists() {
        return Err(lex::error(&dir.join("layers.md").display().to_string(), 0, "the layer file is order.md now: `# order`, a line saying `lowest first`, then one `- name` per line"));
    }
    let path = dir.join("order.md");
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(Vec::new()) };
    let file = path.display().to_string();
    let mut layers = Vec::new();
    let mut said = false;
    for (i, line) in text.lines().enumerate() {
        if let Some(name) = line.trim().strip_prefix("- ") {
            if !said {
                return Err(lex::error(&file, i + 1, "order.md orders the layers, lowest first: say so on a line before the list"));
            }
            let name = name.trim().to_string();
            if layers.contains(&name) {
                return Err(lex::error(&file, i + 1, format!("layer '{}' is listed twice", name)));
            }
            layers.push(name);
        } else if line.contains("lowest first") {
            said = true;
        }
    }
    if layers.is_empty() {
        return Err(lex::error(&file, 0, "order.md lists the layers, lowest first, one `- name` per line"));
    }
    Ok(layers)
}

/// the parent lines make a tree, every feature has a layer — its own
/// or its parent's — and a feature's layer is at or above its parent's
fn check_tree(features: &mut [FeatureDoc], layers: &[String]) -> Result<(), Error> {
    let names: Vec<String> = features.iter().map(|f| f.name.clone()).collect();
    let parent_of: HashMap<String, Option<String>> = features.iter().map(|f| (f.name.clone(), f.parent.clone())).collect();
    for f in features.iter() {
        if let Some(p) = &f.parent {
            if !names.contains(p) {
                return Err(lex::error(&f.md_file, 0, format!("parent '{}' is not a feature of the store", p)));
            }
            if p == &f.name {
                return Err(lex::error(&f.md_file, 0, "a feature is not its own parent"));
            }
        }
    }
    // the layer, inherited down the parent line; a loop in the line is found on the way
    let own: HashMap<String, Option<String>> = features.iter().map(|f| (f.name.clone(), f.layer.clone())).collect();
    let mut resolved: HashMap<String, String> = HashMap::new();
    for f in features.iter() {
        let mut seen = vec![f.name.clone()];
        let mut at = f.name.clone();
        let layer = loop {
            if let Some(l) = &own[&at] {
                break l.clone();
            }
            match &parent_of[&at] {
                Some(p) => {
                    if seen.contains(p) {
                        return Err(lex::error(&f.md_file, 0, format!("the parent lines loop: {}", seen.join(" -> "))));
                    }
                    seen.push(p.clone());
                    at = p.clone();
                }
                None => return Err(lex::error(&f.md_file, 0, format!("feature {} has no layer and no parent to take one from: `layer: name`", f.name))),
            }
        };
        if !layers.is_empty() && !layers.contains(&layer) {
            return Err(lex::error(&f.md_file, 0, format!("layer '{}' is not in order.md ({})", layer, layers.join(", "))));
        }
        resolved.insert(f.name.clone(), layer);
    }
    let rank = |l: &str| layers.iter().position(|x| x == l).unwrap_or(0);
    for f in features.iter_mut() {
        let layer = resolved[&f.name].clone();
        if let Some(p) = &f.parent {
            let pl = &resolved[p];
            if rank(&layer) < rank(pl) {
                return Err(lex::error(&f.md_file, 0, format!("feature {} is in layer {}, below its parent {}'s layer {}: a feature's layer is at or above its parent's", f.name, layer, p, pl)));
            }
        }
        f.layer = Some(layer);
    }
    Ok(())
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
                let same_commit = o.split_once(&when).map(|(_, after)| after.trim_start().starts_with("same commit")).unwrap_or(false);
                origins.push(Origin { when, text: o.trim().to_string(), same_commit });
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
