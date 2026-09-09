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
    /// the product's `int` width (log 47): an `int: 32` or `int: 64`
    /// line in `product.md`; None to take the path's policy
    pub int_width: Option<u32>,
    /// the product's `float` width (log 52): `float: 32` or `float: 64`
    pub float_width: Option<u32>,
    /// the product's mark per feature (section 12, log 71): a
    /// `<feature>: static on`, `static off` or `dynamic` line in
    /// `product.md`; dynamic where there is none. A static-off feature
    /// and its subtree are not among the features, but their marks are
    /// kept so a case naming one can be told why
    pub marks: HashMap<String, Mark>,
    /// the product's clock (log 77): `clock: real` or `clock: virtual`
    pub clock: Clock,
}

/// how a product builds a feature (section 12): switchable at run time,
/// always on with no gate and no switch, or left out entirely
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Dynamic,
    StaticOn,
    StaticOff,
}

/// the clock a store runs on (log 77): the virtual one the suite moves
/// as fast as it can, or the machine's, which `probe zero <store> run`
/// takes unless told `--fast`; a `clock: real` or `clock: virtual` line
/// in `product.md`, virtual where there is none
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Clock {
    Virtual,
    Real,
}

impl Store {
    /// a layer's height: its place in `order.md`, or 0 for every
    /// layer when there is no list
    pub fn rank(&self, layer: &str) -> usize {
        self.layers.iter().position(|l| l == layer).unwrap_or(0)
    }

    /// the features effectively off when these are switched off (log
    /// 51): each with every feature under it in the parent tree, since
    /// the enabled gate is the ancestor conjunction (structure.md)
    pub fn closure(&self, switched: &std::collections::BTreeSet<String>) -> std::collections::BTreeSet<String> {
        switched.iter().flat_map(|f| self.subtree(f)).collect()
    }

    /// a feature and every feature under it in the parent tree
    pub fn subtree(&self, name: &str) -> Vec<String> {
        let mut out = vec![name.to_string()];
        let mut i = 0;
        while i < out.len() {
            for f in &self.features {
                if f.parent.as_deref() == Some(out[i].as_str()) && !out.contains(&f.name) {
                    out.push(f.name.clone());
                }
            }
            i += 1;
        }
        out
    }
}

pub struct FeatureDoc {
    pub name: String,
    pub parent: Option<String>,
    /// the feature's layer: its own `layer:` line, or its parent's
    pub layer: Option<String>,
    pub origins: Vec<Origin>,
    /// `published: <date>` in the header (structure.md's lifecycle, log
    /// 49): the feature has other users and its code is immutable from
    /// that date; None while it is in private development
    pub published: Option<String>,
    /// `>existing` among the cases (section 14, log 50): this feature's
    /// test functions call the older features' cases too, rather than
    /// replacing them
    pub existing_cases: bool,
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
    /// the case's context (section 14, log 43): `with <feature> off`
    /// and `on` clauses after the call, each a feature and its state
    pub context: Vec<(String, bool)>,
    /// the case's input (section 15, log 62): `with in "text"`, the
    /// bytes the runner pushes into `in$` before the program starts
    pub input: Option<String>,
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
    let origin = Origin { when: "0000-00-00T00:00:00".into(), text: "(probe) the compiler's own feature: the platform functions every store has".into() };
    Ok(FeatureDoc { name: "platform".into(), parent: None, layer: Some("platform".into()), origins: vec![origin], published: None, existing_cases: false, cases: Vec::new(), code, md_file: PLATFORM_FILE.into() })
}

/// Read a store: every folder with a `.md` and a `.zero` of its own name.
pub fn read(dir: &Path) -> Result<Store, Error> {
    let sdir = dir.display().to_string();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| lex::error(&sdir, 0, format!("{}", e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && !p.file_name().unwrap().to_string_lossy().starts_with('.'))
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
        if let Some(date) = &doc.published {
            check_published(&name, &zfile, date)?;
        }
        features.push(doc);
    }
    // composition order is creation time, the earliest origin's (log
    // 5, 49); two features created at once order by name
    features.sort_by(|a, b| a.origins[0].when.cmp(&b.origins[0].when).then(a.name.cmp(&b.name)));
    let layers = read_order(dir)?;
    check_tree(&mut features, &layers)?;
    features.insert(0, builtin_platform(&types)?);
    let (product, int_width, float_width, marks, clock, product_file) = read_product(dir)?;
    for (name, mark) in &marks {
        if !features.iter().any(|f| &f.name == name) {
            return Err(lex::error(&product_file, 0, format!("the product marks '{}', which is no feature of the store", name)));
        }
        if name == "platform" && *mark == Mark::StaticOff {
            return Err(lex::error(&product_file, 0, "the platform feature is what a program runs on: it may be static on or dynamic, not static off"));
        }
    }
    // a static-off feature leaves the store with everything under it
    // (log 71): a child under a parent that is never on could never be on
    let mut gone: Vec<String> = Vec::new();
    let store = Store { path: dir.to_path_buf(), features, layers, product, product_file, int_width, float_width, marks, clock };
    for (name, mark) in &store.marks {
        if *mark == Mark::StaticOff {
            gone.extend(store.subtree(name));
        }
    }
    let mut store = store;
    for name in &gone {
        if store.marks.get(name).copied().unwrap_or(Mark::Dynamic) != Mark::StaticOff {
            store.marks.insert(name.clone(), Mark::StaticOff);
        }
    }
    store.features.retain(|f| !gone.contains(&f.name));
    Ok(store)
}

/// A published feature's code is immutable (structure.md's lifecycle,
/// question 24, log 49). The bootstrap's ledger is git, so where the
/// feature's folder is inside a repository the check is the ledger's:
/// the code file has no uncommitted change, and the newest commit that
/// touched it is not dated after the published date, compared at the
/// date's own precision. Outside a repository, or without git, nothing
/// can be told and the feature is accepted
fn check_published(name: &str, zfile: &str, date: &str) -> Result<(), Error> {
    let path = Path::new(zfile);
    let (dir, file) = (path.parent().unwrap_or(Path::new(".")), path.file_name().unwrap().to_string_lossy().to_string());
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).arg("--").arg(&file).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let immutable = "a published feature is immutable, so a change of meaning is a sub-feature that redefines what it needs and calls `existing` (structure.md, the lifecycle)";
    let Some(status) = git(&["status", "--porcelain", "--untracked-files=all"]) else { return Ok(()) };
    if status.starts_with("??") {
        return Err(lex::error(zfile, 0, format!("feature {} is published ({}) but its code is not committed: {}", name, date, immutable)));
    }
    if !status.is_empty() {
        return Err(lex::error(zfile, 0, format!("feature {} was published on {} and its code has uncommitted changes: {}", name, date, immutable)));
    }
    let Some(last) = git(&["log", "-1", "--format=%h %cd", "--date=iso-strict"]) else { return Ok(()) };
    let Some((hash, when)) = last.split_once(' ') else { return Ok(()) };
    let n = date.len().min(when.len()).min(19);
    if when[..n] > date[..n] {
        return Err(lex::error(zfile, 0, format!("feature {} was published on {} and its code changed on {} (commit {}): {}", name, date, when, hash, immutable)));
    }
    Ok(())
}

/// `product.md`: what the product sets and no feature says (zero.md
/// section 1, log 41). In the bootstrap it is two kinds of line: `bound
/// <function words>: N`, a trip count for every loop of that function
/// the IR does not show the count of; `int: 32` or `int: 64`, the
/// width of `int` (log 47), and `float: 32` or `float: 64`, the width
/// of `float` (log 52), and `clock: real` or `clock: virtual` (log 77);
/// every other line is prose
fn read_product(dir: &Path) -> Result<(Vec<(Vec<String>, i64)>, Option<u32>, Option<u32>, HashMap<String, Mark>, Clock, String), Error> {
    let path = dir.join("product.md");
    let file = path.display().to_string();
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok((Vec::new(), None, None, HashMap::new(), Clock::Virtual, file)) };
    let mut settings: Vec<(Vec<String>, i64)> = Vec::new();
    let mut int_width = None;
    let mut float_width = None;
    let mut marks: HashMap<String, Mark> = HashMap::new();
    let mut clock: Option<Clock> = None;
    for (i, line) in text.lines().enumerate() {
        // a feature's mark (log 71): `<feature>: static on | static off | dynamic`
        if let Some((name, rest)) = line.trim().split_once(':') {
            let mark = match rest.trim() {
                "static on" => Some(Mark::StaticOn),
                "static off" => Some(Mark::StaticOff),
                "dynamic" => Some(Mark::Dynamic),
                _ => None,
            };
            if let Some(mark) = mark {
                let name = name.trim();
                if name.is_empty() || name.contains(' ') || name.starts_with("bound ") {
                    return Err(lex::error(&file, i + 1, "a feature's mark is `<feature>: static on`, `static off` or `dynamic`"));
                }
                if marks.insert(name.to_string(), mark).is_some() {
                    return Err(lex::error(&file, i + 1, format!("'{}' is marked twice", name)));
                }
                continue;
            }
        }
        let width = |which: &str, rest: &str, slot: &mut Option<u32>| -> Result<(), Error> {
            let w = rest.trim();
            if !matches!(w, "32" | "64") {
                return Err(lex::error(&file, i + 1, format!("the product's {} width is 32 or 64, not '{}'", which, w)));
            }
            if slot.is_some() {
                return Err(lex::error(&file, i + 1, format!("the product's {} width is set twice", which)));
            }
            *slot = w.parse().ok();
            Ok(())
        };
        if let Some(w) = line.trim().strip_prefix("int:") {
            width("int", w, &mut int_width)?;
            continue;
        }
        if let Some(w) = line.trim().strip_prefix("float:") {
            width("float", w, &mut float_width)?;
            continue;
        }
        // the clock (log 77): the machine's, or the virtual one the suite moves
        if let Some(w) = line.trim().strip_prefix("clock:") {
            let c = match w.trim() {
                "real" => Clock::Real,
                "virtual" => Clock::Virtual,
                other => return Err(lex::error(&file, i + 1, format!("the product's clock is real or virtual, not '{}'", other))),
            };
            if clock.is_some() {
                return Err(lex::error(&file, i + 1, "the product's clock is set twice"));
            }
            clock = Some(c);
            continue;
        }
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
    Ok((settings, int_width, float_width, marks, clock.unwrap_or(Clock::Virtual), file))
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
    let mut published = None;
    let mut origins: Vec<Origin> = Vec::new();
    let mut cases = Vec::new();
    let mut existing_cases = false;
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
            } else if let Some(d) = t.strip_prefix("published:") {
                let d = d.trim();
                let day = d.len() >= 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-' && d[..10].chars().enumerate().all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit());
                if !day {
                    return Err(lex::error(file, ln, "`published:` takes a date, `published: 2026-09-09`, or a date and time"));
                }
                published = Some(d.to_string());
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
            if t == ">existing" {
                if existing_cases {
                    return Err(lex::error(file, ln, "`>existing` is said once in a testing section"));
                }
                existing_cases = true;
            } else if let Some(c) = t.strip_prefix('>') {
                cases.push(parse_case(c, file, ln, types)?);
            }
        }
    }
    if origins.is_empty() {
        return Err(lex::error(file, 0, format!("feature {} has no origin: a `> (where) when` line before the first section", name)));
    }
    // composition order is the earliest origin
    origins.sort_by(|a, b| a.when.cmp(&b.when));
    Ok(FeatureDoc { name: name.to_string(), parent, layer, origins, published, existing_cases, cases, code, md_file: file.to_string() })
}

/// `>call(args) [with <feature> off, <feature> on, in "text"] →
/// result`: the result a number or several, a quoted string (the
/// program's output), or `check` (the call must trap); the `with`
/// clause after the call's `)` names the context (log 43) and the
/// input the runner pushes into `in$` (log 62)
fn parse_case(text: &str, file: &str, line: usize, types: &HashSet<String>) -> Result<Case, Error> {
    let (call, expect) = text
        .split_once('→')
        .or_else(|| text.split_once("->"))
        .ok_or_else(|| lex::error(file, line, "a case is `>call(args) → result`"))?;
    let mut context = Vec::new();
    let mut input = None;
    let call = match call.find(") with ") {
        Some(i) => {
            let mut toks = Vec::new();
            lex::lex_line(call[i + 7..].trim(), line, file, &mut toks)?;
            let toks: Vec<lex::Tok> = toks.into_iter().map(|t| t.tok).collect();
            for part in toks.split(|t| matches!(t, lex::Tok::Sym(","))) {
                match part {
                    [lex::Tok::Word(name), lex::Tok::Word(w)] if w == "off" => context.push((name.clone(), false)),
                    [lex::Tok::Word(name), lex::Tok::Word(w)] if w == "on" => context.push((name.clone(), true)),
                    [lex::Tok::Word(w), lex::Tok::Str(text)] if w == "in" => {
                        if input.replace(text.clone()).is_some() {
                            return Err(lex::error(file, line, "a case has one `in \"text\"`"));
                        }
                    }
                    _ => return Err(lex::error(file, line, "a case's `with` clause is `<feature> off`, `<feature> on` or `in \"text\"`, several joined by commas")),
                }
            }
            &call[..i + 1]
        }
        None => call,
    };
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
    Ok(Case { line, text: text.trim().to_string(), call: call_expr, expect, context, input })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, md: &str, code: &str) {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(dir.join(name).join(format!("{}.md", name)), md).unwrap();
        std::fs::write(dir.join(name).join(format!("{}.zero", name)), code).unwrap();
    }

    fn refused(dir: &Path) -> String {
        match read(dir) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("accepted"),
        }
    }

    fn git(dir: &Path, date: &str, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=probe", "-c", "user.email=probe@probe", "-c", "commit.gpgsign=false"]).args(args).env("GIT_AUTHOR_DATE", date).env("GIT_COMMITTER_DATE", date).output().unwrap();
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    }

    /// a case's `with` clause (log 43, 62): switches and an `in "text"`
    /// in any order, the string's commas and brackets its own
    #[test]
    fn a_case_line_names_its_context_and_its_input() {
        let types = HashSet::new();
        let c = parse_case("echo() with sink off, in \"a, (b)\", format on → \"a, (b)\"", "x.md", 3, &types).unwrap();
        assert_eq!(c.context, [("sink".to_string(), false), ("format".to_string(), true)]);
        assert_eq!(c.input.as_deref(), Some("a, (b)"));
        assert_eq!(c.expect, Expect::Text("a, (b)".into()));
        let plain = parse_case("run() → 1", "x.md", 4, &types).unwrap();
        assert!(plain.context.is_empty() && plain.input.is_none());
        let err = |t: &str| parse_case(t, "x.md", 5, &types).err().unwrap().to_string();
        assert!(err("f() with in \"a\", in \"b\" → 1").contains("a case has one `in \"text\"`"));
        assert!(err("f() with in hi → 1").contains("a case's `with` clause is `<feature> off`, `<feature> on` or `in \"text\"`"));
    }

    /// composition order is creation time and a tie orders by name
    /// (log 49); a published feature's code is immutable where the
    /// ledger can tell: outside a repository nothing is told, inside
    /// one an uncommitted edit and a commit after the date are refused
    #[test]
    fn creation_time_orders_and_a_published_feature_is_immutable() {
        let dir = std::env::temp_dir().join(format!("probe-zero-published-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let head = |published: &str| format!("# f\n*x*\n\nlayer: runtime\n{}\n> (suite) 2026-09-08T10:00:00\n\n## testing\n", published);
        write(&dir, "b", &head(""), "on (int n) = b()\n    n = 2\n");
        write(&dir, "a", &head("published: 2026-09-05\n"), "on (int n) = a()\n    n = 1\n");
        // one timestamp: a before b by name, and nothing refused
        let s = read(&dir).unwrap();
        assert_eq!(s.features.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["platform", "a", "b"]);
        assert_eq!(s.features[1].published.as_deref(), Some("2026-09-05"));
        // a date that is not one
        write(&dir, "c", &head("published: soon\n"), "on (int n) = c()\n    n = 3\n");
        assert!(refused(&dir).contains("`published:` takes a date"));
        std::fs::remove_dir_all(dir.join("c")).unwrap();
        // in a repository: not committed, then committed before the date
        git(&dir, "2026-09-01T10:00:00", &["init", "-q"]);
        assert!(refused(&dir).contains("feature a is published (2026-09-05) but its code is not committed"));
        git(&dir, "2026-09-01T10:00:00", &["add", "."]);
        git(&dir, "2026-09-01T10:00:00", &["commit", "-q", "-m", "a and b"]);
        assert!(read(&dir).is_ok());
        // an edit after publication: uncommitted, then committed after the date
        write(&dir, "a", &head("published: 2026-09-05\n"), "on (int n) = a()\n    n = 11\n");
        assert!(refused(&dir).contains("feature a was published on 2026-09-05 and its code has uncommitted changes"));
        git(&dir, "2026-09-08T10:00:00", &["commit", "-q", "-a", "-m", "a changed"]);
        let err = refused(&dir);
        assert!(err.contains("feature a was published on 2026-09-05 and its code changed on 2026-09-08T10:00:00") && err.contains("a published feature is immutable"), "{}", err);
        // the date moved to the day of the change is the human's override, accepted
        write(&dir, "a", &head("published: 2026-09-08\n"), "on (int n) = a()\n    n = 11\n");
        assert!(read(&dir).is_ok());
        // the prose may change: only the code is immutable
        write(&dir, "a", &head("published: 2026-09-08\n\nprose changed\n"), "on (int n) = a()\n    n = 11\n");
        assert!(read(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
