//! `probe zero <store> trace <case>`: watch a program work (fm3
//! tracer.md). The store is lowered as its trace build (`Store.trace`),
//! a diagnostic build that also writes down, in its own memory, an
//! event for each statement it runs, each name it says and each item it
//! pushes into a stream; the case is run from it on the native JIT; and
//! the events are read back, grouped into steps, and printed as a
//! table, a row a step and a column a stream, or as JSON.
//!
//! An event's `site` is a row of `Lowered.sites`, counted from 1: the
//! file, the line, and `what` is said there. `what` is empty for a
//! statement about to run, `= int n` for a name said, `< int x$` for an
//! item pushed into a stream, `/ x$` for a stream ended. Site 0 closes
//! the push opened last: everything that push set off has run. A site
//! below 0 says a function has returned to the statement at that row.
//! `at` is the program's clock in microseconds, `out` how many bytes it
//! had written, and `text` the value as zero writes it out.
//!
//! A step is a tick, one item pushed from a plain function with
//! everything it set off, or one statement of a plain function outside
//! any tick. The grouping is the zero playground's (`site/trace.js`),
//! which reads the same events in a browser: the two give one table.

use super::lower::Site;
use super::{lower, run, store};
use crate::host::site_said;
use crate::{ssa, suite};
use std::path::Path;

/// one thing the program wrote down
pub struct Event {
    pub site: i64,
    pub at: i64,
    pub out: i64,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Tick,
    Said,
    Line,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Tick => "tick",
            Kind::Said => "said",
            Kind::Line => "line",
        }
    }
}

/// a value said in a step: where, the name, the value as a table shows
/// it, and the mark of its place (`=`, `<` or `/`)
pub struct Said {
    pub feature: String,
    pub line: usize,
    pub name: String,
    pub value: String,
    pub mark: char,
}

/// A step. `feature` and `line` are what began it, and for a tick
/// `stream`, `value` and `ended`; `lines` the lines that ran, in the
/// order they first ran; `cells` the table's row, in a tick the streams
/// said or pushed into, in a statement the names it said; `out` the
/// stretch of the output it wrote and `events` the events it is made of
pub struct Step {
    pub n: usize,
    pub kind: Kind,
    pub at: i64,
    pub feature: String,
    pub line: usize,
    pub stream: Option<String>,
    pub value: Option<String>,
    pub ended: bool,
    pub lines: Vec<(String, usize)>,
    pub cells: Vec<(String, String)>,
    pub said: Vec<Said>,
    pub out: (i64, i64),
    pub events: (usize, usize),
    closed: bool,
    back: bool,
    after: bool,
    /// the time of the step before it as the events had them
    before_at: i64,
}

pub struct Trace {
    pub sites: Vec<Site>,
    pub events: Vec<Event>,
    pub steps: Vec<Step>,
    pub columns: Vec<String>,
    pub timed: bool,
    pub full: bool,
    /// where the program stopped, in zero's words, if it did
    pub stopped: Option<String>,
    pub values: Vec<String>,
    pub output: Vec<u8>,
}

/// what a run of a trace build wrote down, from its words: four of the
/// trace's own (how many events, how many bytes of text, how deep in a
/// value, whether it filled) and then four an event
pub fn read(words: &[i64], bytes: &[u8]) -> (Vec<Event>, bool) {
    let word = |i: usize| words.get(i).copied().unwrap_or(0);
    let n = word(0).clamp(0, suite::TRACE_EVENTS) as usize;
    let len = (word(1).clamp(0, suite::TRACE_TEXT) as usize).min(bytes.len());
    let mut events = Vec::new();
    let mut from = if n > 0 { word(7) as usize } else { 0 };
    for k in 0..n {
        let to = if k + 1 < n { word(11 + 4 * k) as usize } else { len };
        let text = String::from_utf8_lossy(bytes.get(from.min(len)..to.clamp(from.min(len), len)).unwrap_or(&[])).to_string();
        events.push(Event { site: word(4 + 4 * k), at: word(5 + 4 * k), out: word(6 + 4 * k), text });
        from = to;
    }
    (events, word(3) != 0)
}

/// a site's `what`, read: the mark, the type and the name
fn what_of(what: &str) -> (char, &str, &str) {
    let mark = what.chars().next().unwrap_or(' ');
    let rest = match what.get(1..) {
        Some(r) if "=</".contains(mark) && r.starts_with(' ') => &r[1..],
        _ => return (' ', "", ""),
    };
    let (ty, name) = rest.rsplit_once(' ').unwrap_or(("", rest));
    if name.is_empty() || name.contains(char::is_whitespace) {
        return (' ', "", "");
    }
    (mark, ty, name)
}

/// a value as a table shows it: a character and a text in quotes
fn show(text: &str, ty: &str) -> String {
    let esc = text.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t");
    match ty {
        "char" => format!("'{}'", esc),
        "string" => format!("\"{}\"", esc),
        _ => esc,
    }
}

/// The events as steps, and the table's columns: the stream each tick
/// began with first, then the names in the order of the lines that say
/// them. `out_len` is how many bytes the program wrote in all
pub fn steps(sites: &[Site], events: &[Event], out_len: i64) -> (Vec<Step>, Vec<String>, bool) {
    struct Column {
        name: String,
        file: usize,
        line: usize,
        order: usize,
        seq: usize,
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut columns: Vec<Column> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut cur: Option<usize> = None;
    // the place of the event before, where it said a name
    let mut last_said: Option<String> = None;
    let feature_of = |s: &Site| s.file.strip_suffix(".zero").unwrap_or(&s.file).to_string();

    fn column(columns: &mut Vec<Column>, files: &mut Vec<String>, name: &str, feature: &str, line: usize, order: usize) {
        if columns.iter().any(|c| c.name == name) {
            return;
        }
        if !files.iter().any(|f| f == feature) {
            files.push(feature.to_string());
        }
        let file = files.iter().position(|f| f == feature).unwrap();
        let seq = columns.len();
        columns.push(Column { name: name.to_string(), file, line, order, seq });
    }
    fn begin(steps: &mut Vec<Step>, kind: Kind, e: &Event, k: usize, feature: &str, line: usize) -> usize {
        // a statement that had only begun, and now pushes or says its
        // name, is this step and not one before it
        if let Some(p) = steps.last_mut() {
            if p.kind == Kind::Line && !p.closed && p.events.1 == k && p.feature == feature && p.line == line && p.lines.len() == 1 && p.out.0 == e.out {
                // (its time is the later event's: a clock's line is looked
                // at again a period on, and what it then pushes is pushed then)
                p.kind = kind;
                p.at = e.at;
                return steps.len() - 1;
            }
        }
        // what is left of a statement after its push is done, where it
        // goes on to say and write nothing, is no step of its own
        let p = steps.last();
        let after = kind == Kind::Line && p.is_some_and(|p| p.kind == Kind::Tick && p.closed && p.feature == feature && p.line == line && p.at == e.at);
        let before_at = p.map(|p| p.at).unwrap_or(0);
        steps.push(Step { n: steps.len() + 1, kind, at: e.at, feature: feature.to_string(), line, stream: None, value: None, ended: false, lines: Vec::new(), cells: Vec::new(), said: Vec::new(), out: (e.out, e.out), events: (k, k + 1), closed: false, back: false, after, before_at });
        steps.len() - 1
    }
    fn ran(s: &mut Step, feature: &str, line: usize) {
        if !s.lines.iter().any(|(f, l)| f == feature && *l == line) {
            s.lines.push((feature.to_string(), line));
        }
    }
    fn cell(s: &mut Step, name: &str, value: &str) {
        match s.cells.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => {
                v.push_str(", ");
                v.push_str(value);
            }
            None => s.cells.push((name.to_string(), value.to_string())),
        }
    }

    for (k, e) in events.iter().enumerate() {
        if e.site < 0 {
            // a function has returned to the statement at this place:
            // what that statement goes on to write or say is its own
            // step, and where it does neither the step is dropped below
            match sites.get((-e.site - 1) as usize) {
                Some(s) if depth == 0 => {
                    let feature = feature_of(s);
                    let c = begin(&mut steps, Kind::Line, e, k, &feature, s.line);
                    ran(&mut steps[c], &feature, s.line);
                    steps[c].back = true;
                    cur = Some(c);
                    last_said = None;
                }
                _ => {
                    if let Some(c) = cur {
                        steps[c].events.1 = k + 1;
                    }
                }
            }
            continue;
        }
        if e.site == 0 {
            depth = depth.saturating_sub(1);
            if let Some(c) = cur {
                steps[c].events.1 = k + 1;
                if depth == 0 {
                    steps[c].out.1 = e.out;
                    steps[c].closed = true;
                    cur = None;
                }
            }
            last_said = None;
            continue;
        }
        let none = Site { file: String::new(), line: 0, what: String::new() };
        let s = sites.get(e.site as usize - 1).unwrap_or(&none);
        let (mark, ty, name) = what_of(&s.what);
        let feature = feature_of(s);
        let value = show(&e.text, ty);
        if mark == '<' || mark == '/' {
            let value = if mark == '/' { "end".to_string() } else { value };
            if depth == 0 {
                let c = begin(&mut steps, Kind::Tick, e, k, &feature, s.line);
                steps[c].stream = Some(name.to_string());
                steps[c].value = Some(value.clone());
                steps[c].ended = mark == '/';
                cur = Some(c);
            }
            depth += 1;
            let Some(c) = cur else { continue };
            ran(&mut steps[c], &feature, s.line);
            // (a stream that ends inside a tick, as a stream function's
            // result does after its last item, is in `said` and no cell)
            if mark == '<' || depth == 1 {
                cell(&mut steps[c], name, &value);
                column(&mut columns, &mut files, name, &feature, s.line, if depth == 1 { 0 } else { 1 });
            }
            steps[c].said.push(Said { feature, line: s.line, name: name.to_string(), value, mark });
            last_said = None;
        } else if mark == '=' {
            let here = format!("{}:{}", feature, s.line);
            if depth == 0 && !(cur.is_some_and(|c| steps[c].kind == Kind::Said) && last_said.as_deref() == Some(here.as_str())) {
                cur = Some(begin(&mut steps, Kind::Said, e, k, &feature, s.line));
            }
            let Some(c) = cur else { continue };
            ran(&mut steps[c], &feature, s.line);
            // in a tick the table is of streams: what a plain function
            // called from a rule says is kept in `said`, and has no column
            if depth == 0 || name.ends_with('$') {
                cell(&mut steps[c], name, &value);
                column(&mut columns, &mut files, name, &feature, s.line, 1);
            }
            steps[c].said.push(Said { feature, line: s.line, name: name.to_string(), value, mark });
            last_said = (depth == 0).then_some(here);
        } else if !s.what.is_empty() {
            // what a check would say if it failed here: the diagnostic
            // build's, and no step of the program
            if let Some(c) = cur {
                steps[c].events.1 = k + 1;
            }
            continue;
        } else {
            if depth == 0 {
                cur = Some(begin(&mut steps, Kind::Line, e, k, &feature, s.line));
            }
            let Some(c) = cur else { continue };
            ran(&mut steps[c], &feature, s.line);
            last_said = None;
        }
        if let Some(c) = cur {
            steps[c].events.1 = k + 1;
        }
    }
    // what a statement wrote is what had been written when the next step began
    for i in 0..steps.len() {
        if !steps[i].closed {
            steps[i].out.1 = if i + 1 < steps.len() { steps[i + 1].out.0 } else { out_len };
        }
    }
    let mut kept: Vec<Step> = steps.into_iter().filter(|s| !((s.back || (s.after && s.at == s.before_at)) && s.kind == Kind::Line && s.out.1 == s.out.0)).collect();
    for (i, s) in kept.iter_mut().enumerate() {
        s.n = i + 1;
    }
    columns.sort_by_key(|c| (c.order, c.file, c.line, c.seq));
    let timed = events.iter().any(|e| e.at != 0);
    (kept, columns.into_iter().map(|c| c.name).collect(), timed)
}

/// a time as a table writes one, from microseconds
fn time(us: i64) -> String {
    if us % 1_000_000 == 0 {
        format!("{} s", us / 1_000_000)
    } else if us % 1000 == 0 && us < 1_000_000 {
        format!("{} ms", us / 1000)
    } else {
        let frac = format!("{:06}", us % 1_000_000);
        format!("{}.{} s", us / 1_000_000, frac.trim_end_matches('0'))
    }
}

/// The trace as text: a head and a row a step, the step, its time where
/// the program has times, the line that began it, each name said or
/// pushed into, what was written to `out$`, and the lines that ran, in
/// the order they first ran; then where the program stopped, and what
/// the call gave
pub fn text(t: &Trace) -> String {
    // (the table's first rows are the language's own checks, in platform.zero)
    let mut files: Vec<&str> = t.sites.iter().map(|s| s.file.as_str()).filter(|f| *f != "platform.zero").collect();
    files.sort();
    files.dedup();
    let several = files.len() > 1;
    let place = |feature: &str, line: usize| if several { format!("{} {}", feature, line) } else { line.to_string() };
    let wrote = t.steps.iter().any(|s| s.out.1 > s.out.0);
    let mut head: Vec<String> = vec!["step".into()];
    if t.timed {
        head.push("time".into());
    }
    head.push("line".into());
    head.extend(t.columns.iter().cloned());
    if wrote {
        head.push("out$".into());
    }
    let width = head.len();
    head.push("ran".into());
    let mut all = vec![head];
    for s in &t.steps {
        let mut row = vec![s.n.to_string()];
        if t.timed {
            row.push(time(s.at));
        }
        row.push(place(&s.feature, s.line));
        for c in &t.columns {
            row.push(s.cells.iter().find(|(n, _)| n == c).map(|(_, v)| v.clone()).unwrap_or_default());
        }
        if wrote {
            let (a, b) = ((s.out.0.max(0) as usize).min(t.output.len()), (s.out.1.max(0) as usize).min(t.output.len()));
            row.push(show(&String::from_utf8_lossy(&t.output[a.min(b)..b]), ""));
        }
        row.push(s.lines.iter().map(|(f, l)| place(f, *l)).collect::<Vec<_>>().join(" "));
        all.push(row);
    }
    let len = |v: &str| v.chars().count();
    let widths: Vec<usize> = (0..width).map(|c| all.iter().map(|r| len(&r[c])).max().unwrap_or(0)).collect();
    let mut lines: Vec<String> = all
        .iter()
        .map(|r| {
            let cells: Vec<String> = r.iter().enumerate().map(|(c, v)| if c >= width { v.clone() } else if c == 0 { format!("{}{}", " ".repeat(widths[c] - len(v)), v) } else { format!("{}{}", v, " ".repeat(widths[c] - len(v))) }).collect();
            cells.join("  ").trim_end().to_string()
        })
        .collect();
    if t.full {
        lines.push(format!("the trace is full: it keeps the first {} events of a run, and the program was stopped there", suite::TRACE_EVENTS));
    } else if let Some(s) = &t.stopped {
        lines.push(format!("the program stopped: {}", s));
    }
    if !t.values.is_empty() {
        lines.push(format!("-> {}", t.values.join(", ")));
    }
    lines.join("\n")
}

fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The trace as data: `{ store, call, sites, events, steps, columns,
/// timed, full, stopped, values, output }`, the names the playground's
/// `tools/trace.js --json` gives them
pub fn json(t: &Trace, store: &str, call: &str) -> String {
    let opt = |s: &Option<String>| s.as_deref().map(quoted).unwrap_or_else(|| "null".into());
    let sites: Vec<String> = t.sites.iter().map(|s| format!("{{\"file\":{},\"line\":{},\"what\":{}}}", quoted(&s.file), s.line, quoted(&s.what))).collect();
    let events: Vec<String> = t.events.iter().map(|e| format!("{{\"site\":{},\"at\":{},\"out\":{},\"text\":{}}}", e.site, e.at, e.out, quoted(&e.text))).collect();
    let steps: Vec<String> = t
        .steps
        .iter()
        .map(|s| {
            let mut by = format!("\"feature\":{},\"line\":{}", quoted(&s.feature), s.line);
            if let (Some(stream), Some(value)) = (&s.stream, &s.value) {
                by.push_str(&format!(",\"stream\":{},\"value\":{},\"ended\":{}", quoted(stream), quoted(value), s.ended));
            }
            let lines: Vec<String> = s.lines.iter().map(|(f, l)| format!("{{\"feature\":{},\"line\":{}}}", quoted(f), l)).collect();
            let cells: Vec<String> = s.cells.iter().map(|(n, v)| format!("{}:{}", quoted(n), quoted(v))).collect();
            let said: Vec<String> = s.said.iter().map(|x| format!("{{\"feature\":{},\"line\":{},\"name\":{},\"value\":{},\"mark\":{}}}", quoted(&x.feature), x.line, quoted(&x.name), quoted(&x.value), quoted(&x.mark.to_string()))).collect();
            format!("{{\"n\":{},\"kind\":\"{}\",\"at\":{},\"by\":{{{}}},\"lines\":[{}],\"cells\":{{{}}},\"said\":[{}],\"out\":[{},{}],\"events\":[{},{}]}}", s.n, s.kind.name(), s.at, by, lines.join(","), cells.join(","), said.join(","), s.out.0, s.out.1, s.events.0, s.events.1)
        })
        .collect();
    format!(
        "{{\n \"store\": {},\n \"call\": {},\n \"sites\": [{}],\n \"events\": [{}],\n \"steps\": [{}],\n \"columns\": [{}],\n \"timed\": {},\n \"full\": {},\n \"stopped\": {},\n \"values\": [{}],\n \"output\": {}\n}}",
        quoted(store),
        quoted(call),
        sites.join(","),
        events.join(","),
        steps.join(",\n  "),
        t.columns.iter().map(|c| quoted(c)).collect::<Vec<_>>().join(","),
        t.timed,
        t.full,
        opt(&t.stopped),
        t.values.iter().map(|v| quoted(v)).collect::<Vec<_>>().join(","),
        quoted(&String::from_utf8_lossy(&t.output))
    )
}

/// One case of a store, traced: the store lowered as its trace build on
/// the virtual clock, the case found as `run` finds it and run on the
/// native JIT, and what it wrote down made into steps. The second part
/// is the call as the case's line has it
pub fn trace(dir: &Path, which: &str, policy: &ssa::Policy, level: usize) -> Result<(Trace, String), String> {
    let mut s = store::read(dir).map_err(|e| e.to_string())?;
    s.clock = store::Clock::Virtual;
    s.trace = true;
    let policy = &run::store_policy(&s, policy);
    let l = lower::lower(&s).map_err(|e| e.to_string())?;
    let calls = run::calls_of(&s, &l, policy)?;
    let p = run::case_named(&calls, which).ok_or_else(|| format!("no case '{}' in the store's ## testing sections", which))?;
    let call = &p.call;
    let said = p.text.split('→').next().unwrap_or("").trim().to_string();
    let module = run::build(&l.ir, policy, level)?;
    let sc = suite::Call { func: call.func.clone(), args: call.args.clone(), nrets: call.nrets, checks: true, text: true, before: run::setters(&call.context, &call.input, &call.input_at), live: false, times: false };
    let (got, wrote) = suite::run_trace(&module, &sc)?;
    let (events, full) = read(&wrote.words, &wrote.bytes);
    // (what the diagnostic build adds to the output where a site was
    // still stored when the text was read, the line that names it, is
    // no part of what the program wrote)
    let output = run::unsited(suite::Got { values: Vec::new(), text: wrote.output, marks: Vec::new() }).text.into_bytes();
    let (stopped, values) = match got {
        Ok(g) => (None, g.values.iter().enumerate().map(|(k, v)| if call.times.get(k) == Some(&true) { store::spell_nanos(*v) } else { v.to_string() }).collect()),
        Err(e) => (Some(site_said(&l.sites, &e).unwrap_or(e)), Vec::new()),
    };
    let (steps, columns, timed) = steps(&l.sites, &events, output.len() as i64);
    Ok((Trace { sites: l.sites, events, steps, columns, timed, full, stopped, values, output }, said))
}

/// `probe zero <store> trace <case> [--json]`: what is printed
pub fn report(dir: &Path, which: &str, policy: &ssa::Policy, level: usize, as_json: bool) -> Result<String, String> {
    let (t, call) = trace(dir, which, policy, level)?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| dir.display().to_string());
    if as_json {
        return Ok(format!("{}\n", json(&t, &name, &call)));
    }
    Ok(format!("{}: {}   {} steps, {} events\n{}\n", name, call, t.steps.len(), t.events.len(), text(&t)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::Backend;

    /// The trace build (fm3 tracer.md, fm3 log 252): a store not traced
    /// has no word of it; the six-line lexer traced gives a row for each
    /// character with what each of its streams was given, and what the
    /// case gives; a statement that pushes several items into a stream
    /// kept as one value shows each; a `<<` of the program's own that
    /// does more than write is called to write a value and what it
    /// keeps is not moved by it; and a run of more events than the trace
    /// holds is stopped there, and says so
    #[test]
    fn a_trace_is_a_row_a_step_and_a_column_a_stream() {
        let policy = suite::backend_policy(Backend::Native).unwrap();
        let dir = Path::new("suite/zero/lex-zeroic");
        let plain = lower::lower(&store::read(dir).unwrap()).unwrap();
        assert!(!plain.ir.contains("__trace") && !plain.ir.contains("__site"), "a store not traced has no word of the trace");
        let (t, call) = trace(dir, "two arrivals()", &policy, crate::opt::MAX_LEVEL).unwrap();
        assert_eq!(call, "two arrivals()");
        assert_eq!(t.columns, ["src$", "k$", "new$", "start$", "n$", "u$", "a", "b"]);
        assert_eq!((t.steps.len(), t.full, t.stopped.is_none()), (17, false, true));
        assert_eq!(t.values, ["3", "5"]);
        let cells = |n: usize| -> String { t.steps[n - 1].cells.iter().map(|(k, v)| format!("{} {}", k, v)).collect::<Vec<_>>().join(", ") };
        assert_eq!(cells(2), "src$ 'l', k$ word, new$ true, n$ 1, start$ 0");
        assert_eq!(cells(5), "src$ ' ', k$ space, new$ true, n$ 1, start$ 3, u$ word 0 3");
        assert_eq!(cells(16), "src$ end, k$ space, new$ false");
        assert_eq!(cells(17), "b 5");
        let table = text(&t);
        assert!(table.starts_with("step  line  src$  k$      new$   start$  n$  u$          a  b  ran\n   1  34"), "{}", table);
        assert!(table.ends_with("\n-> 3, 5"), "{}", table);
        assert!(json(&t, "lex-zeroic", &call).contains("\"columns\": [\"src$\",\"k$\",\"new$\",\"start$\",\"n$\",\"u$\",\"a\",\"b\"]"));

        let dir = std::env::temp_dir().join(format!("probe-zero-trace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("h")).unwrap();
        std::fs::write(dir.join("h/h.md"), "# h\n*x*\n\nlayer: runtime\n\n> (suite) 2026-09-08T10:00:00\n\n## testing\n>several() → 6\n>written() → 3\n>long (9000) → 9000\n").unwrap();
        std::fs::write(dir.join("h/h.zero"), "int seen$\nint written$\n\non (char o$) << (int x)\n    written$ << written$ + 1\n    existing o$ << x\n\non (int n) << several()\n    seen$ << 1 << 2 << 3\n    seen$ << seen$ + 1 (3) times\n    n << seen$\n\non (int n) << written()\n    out$ << 1 << 2 << 3\n    n << written$\n\non (int n) << long (int k)\n    seen$ << seen$ + 1 (k) times\n    n << seen$\n").unwrap();
        // each item pushed into a stream kept as one value is a step
        let (t, _) = trace(&dir, "several()", &policy, crate::opt::MAX_LEVEL).unwrap();
        let seen: Vec<&str> = t.steps.iter().filter_map(|s| s.cells.iter().find(|(k, _)| k == "seen$").map(|(_, v)| v.as_str())).collect();
        assert_eq!(seen, ["1", "2", "3", "4", "5", "6"]);
        assert_eq!(t.values, ["6"]);
        // the program's `<<` of a whole number counts what it writes:
        // the trace calls it for every number it shows, and the count
        // is what the program's own three writes made it
        let (t, _) = trace(&dir, "written()", &policy, crate::opt::MAX_LEVEL).unwrap();
        assert_eq!((t.values.clone(), String::from_utf8_lossy(&t.output).to_string()), (vec!["3".to_string()], "123".to_string()));
        // more events than the trace holds
        let (t, _) = trace(&dir, "long (9000)", &policy, crate::opt::MAX_LEVEL).unwrap();
        assert!(t.full && t.events.len() == suite::TRACE_EVENTS as usize && t.values.is_empty(), "{} events", t.events.len());
        assert!(text(&t).contains("the trace is full: it keeps the first 16384 events of a run"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
