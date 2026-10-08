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

#[derive(Clone)]
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
    /// the product's `index` width (fm3 question 73, log 120): `index:
    /// 16`, `index: 32` or `index: 64`, how wide a count and a position
    /// in memory are; None to take the path's
    pub index_width: Option<u32>,
    /// the product's mark per feature (section 12, log 71): a
    /// `<feature>: static on`, `static off` or `dynamic` line in
    /// `product.md`; dynamic where there is none. A static-off feature
    /// and its subtree are not among the features, but their marks are
    /// kept so a case naming one can be told why
    pub marks: HashMap<String, Mark>,
    /// the features the product leaves out, `static off` and everything
    /// under them, parsed and in composition order: the store as it was
    /// read is `features` and these (question 54, fm3 log 96). Nothing
    /// of them is lowered; the compiler asks them only whether a stream
    /// the program pushes into is one that some feature would read
    pub left_out: Vec<FeatureDoc>,
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

#[derive(Clone)]
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

#[derive(Clone)]
pub struct Origin {
    pub when: String,
    pub text: String,
}

#[derive(Clone)]
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
    /// what was written and when (question 52, fm3 log 91): the pieces
    /// of the output in order, each with the time on the store's clock,
    /// in microseconds, at which it was written; joined they are the
    /// whole output
    Timed(Vec<(String, i64)>),
    Check,
}

/// the store's clock (log 63): a million steps a second, the bootstrap's
/// step
pub const CLOCK_HZ: i64 = 1_000_000;

/// A rate's period in whole steps of the store's clock. One function
/// for the program and for the case that asserts on it (fm3 log 97): a
/// push into a stream with a rate moves the clock on by this (`step` in
/// `lower.rs`), and a timed piece given `at` a rate puts its lines this
/// far apart, so the two cannot differ by a rounding
pub fn period(hz: i64) -> i64 {
    CLOCK_HZ / hz
}

/// the shape of a timed result, for a refusal
const TIMED_SHAPE: &str = "a timed result is every piece of the output in order, each `\"text\" at <n> s` or `<n> ms`, or `\"text\" at <n> hz` for its lines one a step, with `from <n> s` where they do not start at 0 s, joined by commas: `\"3\\n2\\n1\\n\" at 1 hz, \"liftoff\" at 3 s`";

/// a time as a case writes it: whole seconds as `3 s`, a whole number
/// of milliseconds under a second as `250 ms`, anything else as decimal
/// seconds, `3.5 s`
pub fn spell_time(us: i64) -> String {
    if us % 1_000_000 == 0 {
        format!("{} s", us / 1_000_000)
    } else if us % 1000 == 0 && us < 1_000_000 {
        format!("{} ms", us / 1000)
    } else {
        let frac = format!("{:06}", us % 1_000_000);
        format!("{}.{} s", us / 1_000_000, frac.trim_end_matches('0'))
    }
}

/// A timed result as a case writes it, so that what a run printed can
/// be pasted back into the case. A run of three or more pieces, each one
/// line, a whole rate's period apart, is written as the rate form
/// (question 53, fm3 log 97): `"10\n9\n8\n" at 1 hz`, with `from` where
/// it does not start at 0 s; the last of a run may be a line with no
/// newline. Everything else is listed, `"text" at <time>`
pub fn spell_timed(pieces: &[(String, i64)]) -> String {
    if pieces.is_empty() {
        return "\"\" at 0 s".to_string();
    }
    let one_line = |t: &str| !t.strip_suffix('\n').unwrap_or(t).contains('\n');
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        // the run that begins here, i through j
        let mut j = i;
        let d = pieces.get(i + 1).map_or(0, |next| next.1 - pieces[i].1);
        let hz = if d > 0 { CLOCK_HZ / d } else { 0 };
        if hz > 0 && period(hz) == d && one_line(&pieces[i].0) {
            while j + 1 < pieces.len() && pieces[j].0.ends_with('\n') && one_line(&pieces[j + 1].0) && pieces[j + 1].1 - pieces[j].1 == d {
                j += 1;
            }
        }
        if j - i >= 2 {
            let text: String = pieces[i..=j].iter().map(|p| p.0.as_str()).collect();
            let from = if pieces[i].1 == 0 { String::new() } else { format!(" from {}", spell_time(pieces[i].1)) };
            out.push(format!("{:?} at {} hz{}", text, hz, from));
            i = j + 1;
        } else {
            out.push(format!("{:?} at {}", pieces[i].0, spell_time(pieces[i].1)));
            i += 1;
        }
    }
    out.join(", ")
}

/// a timed result's pieces by its two rules (question 53): a stamp
/// belongs to the characters, so a piece with none is nothing and two
/// pieces at one time are one piece
pub fn merge_timed(pieces: Vec<(String, i64)>) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for (text, t) in pieces {
        if text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((last, lt)) if *lt == t => last.push_str(&text),
            _ => out.push((text, t)),
        }
    }
    out
}

/// `"text" at <n> s, "text" at <n> ms, ...`, already lexed. A piece may
/// be given `at` a rate in place of a time (question 53, C; fm3 log 97):
/// `"10\n9\n8\n" at 1 hz` is its lines, one a step of that rate, from
/// 0 s or `from` a time. A line is the text up to and including its
/// newline, and text after the last newline is a line too. It expands
/// here into the pieces the listed form gives
fn parse_timed(toks: &[lex::Tok], file: &str, line: usize) -> Result<Expect, Error> {
    // a number and `s` or `ms`, whole or decimal, as a whole number of the clock's steps
    let time = |n: &lex::Tok, unit: &str| -> Result<i64, Error> {
        let per = match unit {
            "s" => 1_000_000i64,
            "ms" => 1000,
            _ => return Err(lex::error(file, line, TIMED_SHAPE)),
        };
        let us = match n {
            lex::Tok::Int(v) if *v >= 0 => v.checked_mul(per),
            lex::Tok::Float(f) => {
                let (whole, frac) = f.split_once('.').unwrap_or((f, ""));
                let digits = per.ilog10() as usize;
                let frac = frac.trim_end_matches('0');
                if frac.len() > digits {
                    return Err(lex::error(file, line, format!("the time {} {} is not a whole number of microseconds, the clock's step", f, unit)));
                }
                let scaled = format!("{:0<width$}", frac, width = digits);
                whole.parse::<i64>().ok().and_then(|w| w.checked_mul(per)).and_then(|w| scaled.parse::<i64>().ok().and_then(|x| w.checked_add(x)))
            }
            _ => return Err(lex::error(file, line, TIMED_SHAPE)),
        };
        us.ok_or_else(|| lex::error(file, line, TIMED_SHAPE))
    };
    let mut pieces: Vec<(String, i64)> = Vec::new();
    for part in toks.split(|t| matches!(t, lex::Tok::Sym(","))) {
        let (text, n, unit, from) = match part {
            [lex::Tok::Str(text), lex::Tok::Word(at), n, lex::Tok::Word(unit)] if at == "at" => (text, n, unit, None),
            [lex::Tok::Str(text), lex::Tok::Word(at), n, lex::Tok::Word(unit), lex::Tok::Word(from), m, lex::Tok::Word(funit)] if at == "at" && from == "from" => (text, n, unit, Some((m, funit))),
            _ => return Err(lex::error(file, line, TIMED_SHAPE)),
        };
        if text.is_empty() {
            return Err(lex::error(file, line, "a piece of a timed result has at least one character: a time with nothing written at it says nothing"));
        }
        let lines: Vec<(String, i64)> = match unit.as_str() {
            // at a rate: the lines, one a step
            "hz" | "khz" => {
                let hz = match n {
                    lex::Tok::Int(v) if *v > 0 => v.checked_mul(if unit == "khz" { 1000 } else { 1 }),
                    _ => None,
                };
                let Some(hz) = hz else {
                    return Err(lex::error(file, line, "the rate of a timed piece is a positive whole number and `hz` or `khz`, as a stream's is: `\"3\\n2\\n1\\n\" at 1 hz`"));
                };
                let step = period(hz);
                if step == 0 {
                    return Err(lex::error(file, line, format!("a rate of {} hz has no step on the store's clock, which has a million a second", hz)));
                }
                let start = match from {
                    Some((m, funit)) => time(m, funit)?,
                    None => 0,
                };
                text.split_inclusive('\n').enumerate().map(|(k, l)| (l.to_string(), start.saturating_add((k as i64).saturating_mul(step)))).collect()
            }
            _ if from.is_some() => return Err(lex::error(file, line, "`from` says where a rate's lines start, `\"3\\n2\\n1\\n\" at 1 hz from 3.5 s`: a piece with a time has it already")),
            _ => vec![(text.clone(), time(n, unit)?)],
        };
        for (text, us) in lines {
            if let Some((_, last)) = pieces.last() {
                if us < *last {
                    return Err(lex::error(file, line, format!("the times of a timed result do not go back: {} after {}", spell_time(us), spell_time(*last))));
                }
            }
            pieces.push((text, us));
        }
    }
    Ok(Expect::Timed(merge_timed(pieces)))
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
        // ... and the words before `and` and `or` in a function's name
        // (fm3 question 66), which the parser tells from the operator by
        types.extend(syntax::declared_joins(&code));
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
    let (product, [int_width, float_width, index_width], marks, clock, product_file) = read_product(dir)?;
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
    let store = Store { path: dir.to_path_buf(), features, layers, product, product_file, int_width, float_width, index_width, marks, left_out: Vec::new(), clock };
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
    let (left_out, features) = std::mem::take(&mut store.features).into_iter().partition(|f| gone.contains(&f.name));
    (store.left_out, store.features) = (left_out, features);
    // every name held to its mark, and an array's written as the
    // lowering reads it (fm3 question 90, log 159)
    super::kinds::settle(&mut store)?;
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
/// of `float` (log 52), `index: 16`, `index: 32` or `index: 64`, the
/// width of `index` (fm3 log 120), and `clock: real` or `clock: virtual`
/// (log 77); every other line is prose
#[allow(clippy::type_complexity)]
fn read_product(dir: &Path) -> Result<(Vec<(Vec<String>, i64)>, [Option<u32>; 3], HashMap<String, Mark>, Clock, String), Error> {
    let path = dir.join("product.md");
    let file = path.display().to_string();
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok((Vec::new(), [None; 3], HashMap::new(), Clock::Virtual, file)) };
    let mut settings: Vec<(Vec<String>, i64)> = Vec::new();
    let mut int_width = None;
    let mut float_width = None;
    let mut index_width = None;
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
            // `index` may be 16 too: a small machine's addresses
            let (ok, widths) = if which == "index" { (matches!(w, "16" | "32" | "64"), "16, 32 or 64") } else { (matches!(w, "32" | "64"), "32 or 64") };
            if !ok {
                return Err(lex::error(&file, i + 1, format!("the product's {} width is {}, not '{}'", which, widths, w)));
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
        if let Some(w) = line.trim().strip_prefix("index:") {
            width("index", w, &mut index_width)?;
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
    Ok((settings, [int_width, float_width, index_width], marks, clock.unwrap_or(Clock::Virtual), file))
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
/// program's output), that output in pieces each with its time,
/// `"10\n" at 0 s, "9\n" at 1 s`, or with a rate for its lines,
/// `"10\n9\n" at 1 hz`, or `check` (the call must trap); the `with`
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
                    // an input with a time is ruled and waits for restart (question 53)
                    [lex::Tok::Word(w), lex::Tok::Str(_), lex::Tok::Word(at), ..] if w == "in" && at == "at" => {
                        return Err(lex::error(file, line, "a timed input, `in \"k\" at 3.5 s`, is ruled (question 53) and not built: delivering it needs a function suspended partway, which is restart's"));
                    }
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
            // text with times (fm3 log 91)
            _ => parse_timed(&toks.iter().map(|t| t.tok.clone()).collect::<Vec<_>>(), file, line)?,
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

    /// a case that asserts on time (question 52, 53, fm3 log 91): the
    /// pieces of the output each with its time, a time a number and `s`
    /// or `ms`; two pieces at one time are one; and what is refused
    #[test]
    fn a_case_line_asserts_on_time() {
        let types = HashSet::new();
        let timed = |t: &str| parse_case(t, "x.md", 7, &types).map(|c| c.expect);
        assert_eq!(timed("run() → \"10\\n\" at 0 s, \"9\\n\" at 1 s, \"hello, world\" at 3.5 s").unwrap(), Expect::Timed(vec![("10\n".into(), 0), ("9\n".into(), 1_000_000), ("hello, world".into(), 3_500_000)]));
        assert_eq!(timed("run() → \"a\" at 250 ms, \"b\" at 0.25 s, \"c\" at 251.5 ms").unwrap(), Expect::Timed(vec![("ab".into(), 250_000), ("c".into(), 251_500)]));
        // one quoted string alone is the plain text result it was
        assert_eq!(timed("run() → \"a\"").unwrap(), Expect::Text("a".into()));
        // what a run prints parses back to itself
        let pieces = vec![("10\n".to_string(), 0), ("say \"hi\"\n".to_string(), 1_500_000), ("x".to_string(), 2_000_250)];
        assert_eq!(spell_timed(&pieces), "\"10\\n\" at 0 s, \"say \\\"hi\\\"\\n\" at 1.5 s, \"x\" at 2.00025 s");
        assert_eq!(timed(&format!("f() → {}", spell_timed(&pieces))).unwrap(), Expect::Timed(pieces));
        assert_eq!(spell_time(250_000), "250 ms");
        let err = |t: &str| timed(t).err().unwrap().to_string();
        for bad in ["f() → \"a\" at 1", "f() → \"a\" 1 s", "f() → \"a\" at 1 min", "f() → \"a\" at 0 s \"b\" at 1 s", "f() → \"a\" at 0 s,", "f() → \"a\" at -1 s", "f() → \"a\", \"b\""] {
            assert!(err(bad).contains("a timed result is every piece of the output in order, each `\"text\" at <n> s` or `<n> ms`, or `\"text\" at <n> hz` for its lines one a step"), "{}: {}", bad, err(bad));
        }
        assert!(err("f() → \"a\" at 2 s, \"b\" at 1 s").contains("the times of a timed result do not go back: 1 s after 2 s"));
        assert!(err("f() → \"\" at 2 s").contains("a piece of a timed result has at least one character"));
        assert!(err("f() → \"a\" at 0.0000001 s").contains("is not a whole number of microseconds"));
    }

    /// a piece of a timed result given `at` a rate (question 53, C; fm3
    /// log 97): its lines one a step, from 0 s or `from` a time, mixed
    /// with listed pieces in order; a step is the period a push into a
    /// stream of that rate moves the clock by; what is refused; and
    /// what a run prints folds a regular run back and parses to itself
    #[test]
    fn a_timed_piece_may_be_at_a_rate() {
        let types = HashSet::new();
        let timed = |t: &str| parse_case(t, "x.md", 7, &types).map(|c| c.expect);
        let p = |t: &str, us: i64| (t.to_string(), us);
        let s = 1_000_000;
        assert_eq!(timed("f() → \"10\\n9\\n8\\n\" at 1 hz").unwrap(), Expect::Timed(vec![p("10\n", 0), p("9\n", s), p("8\n", 2 * s)]));
        assert_eq!(timed("f() → \"10\\n9\\n\" at 1 hz from 3.5 s").unwrap(), Expect::Timed(vec![p("10\n", 3_500_000), p("9\n", 4_500_000)]));
        // text after the last newline is a line too, and `khz` is a thousand
        assert_eq!(timed("f() → \"a\\nb\" at 2 khz from 250 ms").unwrap(), Expect::Timed(vec![p("a\n", 250_000), p("b", 250_500)]));
        // mixed with listed pieces, in order; a piece at a line's time joins it
        let hello = "run() → \"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s";
        let listed = "run() → \"10\\n\" at 0 s, \"9\\n\" at 1 s, \"8\\n\" at 2 s, \"7\\n\" at 3 s, \"6\\n\" at 4 s, \"5\\n\" at 5 s, \"4\\n\" at 6 s, \"3\\n\" at 7 s, \"2\\n\" at 8 s, \"1\\n\" at 9 s, \"hello world\\ngoodbye\" at 10 s";
        assert_eq!(timed(hello).unwrap(), timed(listed).unwrap());
        assert_eq!(timed("f() → \"a\" at 0 s, \"b\\nc\\n\" at 2 hz, \"d\" at 500 ms").unwrap(), Expect::Timed(vec![p("ab\n", 0), p("c\nd", 500_000)]));
        // a step is the period the program steps by, a million over the
        // rate in whole microseconds: three lines at 3 hz, and `period`
        assert_eq!(timed("f() → \"a\\nb\\nc\\nd\" at 3 hz").unwrap(), Expect::Timed(vec![p("a\n", 0), p("b\n", 333_333), p("c\n", 666_666), p("d", 999_999)]));
        assert_eq!((period(1), period(3), period(48_000)), (1_000_000, 333_333, 20));
        // refused
        let err = |t: &str| timed(t).err().unwrap().to_string();
        assert!(err("f() → \"a\" at 1 s from 2 s").contains("`from` says where a rate's lines start"));
        for bad in ["f() → \"a\" at 0 hz", "f() → \"a\" at 0.5 hz", "f() → \"a\" at -1 hz"] {
            assert!(err(bad).contains("the rate of a timed piece is a positive whole number and `hz` or `khz`") || err(bad).contains("a timed result is every piece"), "{}: {}", bad, err(bad));
        }
        assert!(err("f() → \"a\" at 0.5 hz").contains("the rate of a timed piece is a positive whole number"));
        assert!(err("f() → \"a\" at 2000 khz").contains("a rate of 2000000 hz has no step on the store's clock"));
        assert!(err("f() → \"a\" at 1 hz from 2").contains("a timed result is every piece"));
        assert!(err("f() → \"a\" at 1 hz from 2 hz").contains("a timed result is every piece"));
        assert!(err("f() → \"a\" at 3 s, \"b\\nc\" at 1 hz").contains("the times of a timed result do not go back: 0 s after 3 s"));
        assert!(err("f() → \"a\\nb\\nc\" at 1 hz, \"d\" at 1 s").contains("the times of a timed result do not go back: 1 s after 2 s"));
        // a timed input is ruled and not built
        let e = parse_case("f() with in \"k\" at 3.5 s → \"a\" at 0 s", "x.md", 8, &types).err().unwrap().to_string();
        assert!(e.contains("a timed input, `in \"k\" at 3.5 s`, is ruled (question 53) and not built"), "{}", e);
        // what a run prints: a run of three or more single lines a whole
        // rate's period apart is folded, and every spelling parses back
        let back = |pieces: Vec<(String, i64)>, spelt: &str| {
            assert_eq!(spell_timed(&pieces), spelt);
            assert_eq!(timed(&format!("f() → {}", spelt)).unwrap(), Expect::Timed(pieces));
        };
        let Expect::Timed(h) = timed(hello).unwrap() else { unreachable!() };
        back(h, "\"10\\n9\\n8\\n7\\n6\\n5\\n4\\n3\\n2\\n1\\n\" at 1 hz, \"hello world\\ngoodbye\" at 10 s");
        back(vec![p("7\n", 3 * s), p("10\n", 3_500_000), p("9\n", 4_500_000), p("8", 5_500_000)], "\"7\\n\" at 3 s, \"10\\n9\\n8\" at 1 hz from 3.5 s");
        back(vec![p("a\n", 0), p("b\n", 333_333), p("c\n", 666_666)], "\"a\\nb\\nc\\n\" at 3 hz");
        // two are not a rhythm; a piece of two lines is not a line; an
        // uneven third ends the run
        back(vec![p("a\n", 0), p("b\n", s)], "\"a\\n\" at 0 s, \"b\\n\" at 1 s");
        back(vec![p("a\n", 0), p("b\nb\n", s), p("c\n", 2 * s)], "\"a\\n\" at 0 s, \"b\\nb\\n\" at 1 s, \"c\\n\" at 2 s");
        back(vec![p("a\n", 0), p("b\n", s), p("c\n", 2 * s), p("d\n", 2_500_000)], "\"a\\nb\\nc\\n\" at 1 hz, \"d\\n\" at 2.5 s");
        // a line with no newline ends a run, and what follows is listed
        back(vec![p("a\n", 0), p("b\n", s), p("c", 2 * s), p("d", 3 * s)], "\"a\\nb\\nc\" at 1 hz, \"d\" at 3 s");
        // a spacing no whole rate has is listed: 1.5 s apart
        back(vec![p("a\n", 0), p("b\n", 1_500_000), p("c\n", 3 * s)], "\"a\\n\" at 0 s, \"b\\n\" at 1.5 s, \"c\\n\" at 3 s");
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
        write(&dir, "b", &head(""), "on (int n) << b()\n    n << 2\n");
        write(&dir, "a", &head("published: 2026-09-05\n"), "on (int n) << a()\n    n << 1\n");
        // one timestamp: a before b by name, and nothing refused
        let s = read(&dir).unwrap();
        assert_eq!(s.features.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["platform", "a", "b"]);
        assert_eq!(s.features[1].published.as_deref(), Some("2026-09-05"));
        // a date that is not one
        write(&dir, "c", &head("published: soon\n"), "on (int n) << c()\n    n << 3\n");
        assert!(refused(&dir).contains("`published:` takes a date"));
        std::fs::remove_dir_all(dir.join("c")).unwrap();
        // in a repository: not committed, then committed before the date
        git(&dir, "2026-09-01T10:00:00", &["init", "-q"]);
        assert!(refused(&dir).contains("feature a is published (2026-09-05) but its code is not committed"));
        git(&dir, "2026-09-01T10:00:00", &["add", "."]);
        git(&dir, "2026-09-01T10:00:00", &["commit", "-q", "-m", "a and b"]);
        assert!(read(&dir).is_ok());
        // an edit after publication: uncommitted, then committed after the date
        write(&dir, "a", &head("published: 2026-09-05\n"), "on (int n) << a()\n    n << 11\n");
        assert!(refused(&dir).contains("feature a was published on 2026-09-05 and its code has uncommitted changes"));
        git(&dir, "2026-09-08T10:00:00", &["commit", "-q", "-a", "-m", "a changed"]);
        let err = refused(&dir);
        assert!(err.contains("feature a was published on 2026-09-05 and its code changed on 2026-09-08T10:00:00") && err.contains("a published feature is immutable"), "{}", err);
        // the date moved to the day of the change is the human's override, accepted
        write(&dir, "a", &head("published: 2026-09-08\n"), "on (int n) << a()\n    n << 11\n");
        assert!(read(&dir).is_ok());
        // the prose may change: only the code is immutable
        write(&dir, "a", &head("published: 2026-09-08\n\nprose changed\n"), "on (int n) << a()\n    n << 11\n");
        assert!(read(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
