//! `probe indent`: the brace form of the IR rewritten in the indented
//! form (fm3 rulings-3 item 1, log 53). A rewrite of the text, not a
//! print of the parsed module: the module's printer has lost the
//! structured form and every comment, and the IR's comments carry
//! provenance. The text is re-indented from its brace structure at four
//! spaces a level — a function's body one level in, a flat function's
//! labels at that level and their instructions one deeper — the `}`
//! lines go, `} else {` becomes `else`, an inline field list becomes a
//! field per line, a data initializer's braces go, a platform block's
//! rules keep their own indentation under the header, and a comment
//! line stands at the level of the code around it. A text with no
//! brace is returned as it is.

/// the kinds of block the braces opened
#[derive(PartialEq)]
enum Kind {
    /// a function, an `if` arm, a loop
    Brace,
    /// a flat function's basic block: its label, then its instructions
    Label,
    /// `platform <target> {`: rule lines, kept as they are
    Platform,
    /// `struct {` or `pack {` over several lines
    Fields,
    /// `data x: T = {` over several lines
    Data,
}

struct Conv {
    out: String,
    depth: usize,
    stack: Vec<Kind>,
    /// the raw lines of the platform block being read
    rules: Vec<String>,
}

pub fn indent(text: &str) -> Result<String, String> {
    if !has_braces(text) {
        return Ok(text.to_string());
    }
    let mut c = Conv { out: String::new(), depth: 0, stack: Vec::new(), rules: Vec::new() };
    for (n, raw) in text.lines().enumerate() {
        c.line(raw).map_err(|m| format!("line {}: {}", n + 1, m))?;
    }
    if !c.stack.is_empty() {
        return Err("a block is never closed".into());
    }
    Ok(c.out)
}

/// does the text write a block with a brace (outside strings and comments)?
pub fn has_braces(text: &str) -> bool {
    text.lines().any(|l| !braces(split_comment(l).0).is_empty())
}

impl Conv {
    fn emit(&mut self, s: &str) {
        if !s.is_empty() {
            for _ in 0..self.depth {
                self.out.push_str("    ");
            }
            self.out.push_str(s);
        }
        self.out.push('\n');
    }

    fn open(&mut self, head: &str, kind: Kind) {
        self.emit(head);
        self.stack.push(kind);
        self.depth += 1;
    }

    /// the `}`: the labels open inside the block go with it
    fn close(&mut self) -> Result<(), String> {
        while self.stack.last() == Some(&Kind::Label) {
            self.stack.pop();
            self.depth -= 1;
        }
        match self.stack.pop() {
            Some(_) => {
                self.depth -= 1;
                Ok(())
            }
            None => Err("a '}' closes no block".into()),
        }
    }

    fn line(&mut self, raw: &str) -> Result<(), String> {
        let (code, rest) = split_comment(raw);
        let code = code.trim();
        // inside a platform block the lines are the platform file's, kept
        // whole; at its `}` they are written under the header with the
        // block's own left edge removed
        if self.stack.last() == Some(&Kind::Platform) {
            if code == "}" {
                let left = self.rules.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
                let rules = std::mem::take(&mut self.rules);
                for l in rules {
                    if l.trim().is_empty() {
                        self.out.push('\n');
                    } else {
                        let l = l[left..].to_string();
                        self.emit(&l);
                    }
                }
                self.close()?;
                let rest = rest.trim().to_string();
                if !rest.is_empty() {
                    self.emit(&rest);
                }
            } else {
                self.rules.push(raw.to_string());
            }
            return Ok(());
        }
        if code.is_empty() {
            let rest = rest.trim().to_string();
            self.emit(&rest);
            return Ok(());
        }
        // a line's code with its comment back on the end
        let tail = |s: &str| format!("{}{}", s, rest);
        let bs = braces(code);
        if bs.is_empty() {
            match self.stack.last() {
                Some(Kind::Fields) => {
                    for f in split_commas(code) {
                        self.field(&f)?;
                    }
                    if !rest.trim().is_empty() {
                        let r = rest.trim().to_string();
                        self.emit(&r);
                    }
                }
                _ if is_label(code) => {
                    if self.stack.last() == Some(&Kind::Label) {
                        self.stack.pop();
                        self.depth -= 1;
                    }
                    let s = tail(code);
                    self.open(&s, Kind::Label);
                }
                _ => {
                    let s = tail(code);
                    self.emit(&s);
                }
            }
            return Ok(());
        }
        if code.starts_with('}') {
            self.close()?;
            let after = code[1..].trim();
            if after == "else {" {
                let s = tail("else");
                self.open(&s, Kind::Brace);
            } else if after.is_empty() {
                let r = rest.trim().to_string();
                if !r.is_empty() {
                    self.emit(&r);
                }
            } else {
                return Err(format!("'{}' after a '}}' is not converted", after));
            }
            return Ok(());
        }
        if code.ends_with('{') && bs.len() == 1 {
            let head = code[..code.len() - 1].trim_end();
            let kind = if head.starts_with("platform ") {
                self.rules.clear();
                Kind::Platform
            } else if head.ends_with("struct") || head.ends_with("pack") {
                Kind::Fields
            } else if (head.starts_with("data ") || head.starts_with("group ")) && head.ends_with('=') {
                let head = head[..head.len() - 1].trim_end().to_string();
                let s = tail(&head);
                self.open(&s, Kind::Data);
                return Ok(());
            } else {
                Kind::Brace
            };
            let s = tail(head);
            self.open(&s, kind);
            return Ok(());
        }
        // braces balanced on the line: a type's fields, or a data item's values
        let (open, close) = (bs[0].0, bs[bs.len() - 1].0);
        if bs[0].1 != '{' || bs[bs.len() - 1].1 != '}' {
            return Err("braces are not converted here".into());
        }
        let head = code[..open].trim_end();
        let inner = &code[open + 1..close];
        let after = code[close + 1..].trim();
        if !after.is_empty() {
            return Err(format!("'{}' after a '}}' is not converted", after));
        }
        if head.ends_with("struct") || head.ends_with("pack") {
            let s = tail(head);
            self.emit(&s);
            self.depth += 1;
            for f in split_commas(inner) {
                self.field(&f)?;
            }
            self.depth -= 1;
            return Ok(());
        }
        if (head.starts_with("data ") || head.starts_with("group ")) && head.ends_with('=') {
            let vals: Vec<String> = split_commas(inner);
            let s = tail(&format!("{} {}", head, vals.join(", ")));
            self.emit(&s);
            return Ok(());
        }
        Err("braces are not converted here".into())
    }

    /// one field of a struct or pack, its own fields under it when it
    /// is one
    fn field(&mut self, f: &str) -> Result<(), String> {
        let f = f.trim();
        if f.is_empty() {
            return Ok(());
        }
        let bs = braces(f);
        if bs.is_empty() {
            self.emit(f);
            return Ok(());
        }
        let (open, close) = (bs[0].0, bs[bs.len() - 1].0);
        let head = f[..open].trim_end();
        if !(head.ends_with("struct") || head.ends_with("pack")) || !f[close + 1..].trim().is_empty() {
            return Err(format!("the field '{}' is not converted", f));
        }
        self.emit(head);
        self.depth += 1;
        for g in split_commas(&f[open + 1..close]) {
            self.field(&g)?;
        }
        self.depth -= 1;
        Ok(())
    }
}

/// the code of a line and its comment (`;` to the end, with the
/// whitespace before it), the `;` inside a string not counting
fn split_comment(line: &str) -> (&str, &str) {
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in line.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == ';' {
            let code_end = line[..i].trim_end().len();
            return (&line[..code_end], &line[code_end..]);
        }
    }
    (line, "")
}

/// the braces of a code line, outside strings
fn braces(code: &str) -> Vec<(usize, char)> {
    let mut out = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in code.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '{' || c == '}' {
            out.push((i, c));
        }
    }
    out
}

/// split at the commas outside brackets, braces and strings
fn split_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if in_str {
            if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    out
}

/// `name:` or `name(params):`, a flat function's block label
fn is_label(code: &str) -> bool {
    let Some(t) = code.strip_suffix(':') else { return false };
    let name_end = t.find('(').unwrap_or(t.len());
    let name = &t[..name_end];
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && (name_end == t.len() || t.ends_with(')'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssa;

    /// every `.ssa` in the tree, converted, is the module it was:
    /// parsed (with the prelude, in whichever form it is) and printed,
    /// the two texts agree; and the conversion has no brace and is its
    /// own conversion
    #[test]
    fn every_ssa_file_round_trips() {
        let mut files = Vec::new();
        for dir in ["suite", "suite/zero", "lib", "os", "examples"] {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.extension().is_some_and(|x| x == "ssa") {
                    files.push(p);
                }
            }
        }
        assert!(files.len() > 70, "{} files", files.len());
        let policy = ssa::Policy::new(ssa::Type::I64).unwrap();
        // a library file is part of the prelude already: the text under
        // test is the prelude with that file in the form being tried
        let mut libs: Vec<std::path::PathBuf> = std::fs::read_dir("lib").unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "ssa")).collect();
        libs.sort();
        let text_of = |p: &std::path::Path, src: &str| -> String {
            if p.starts_with("lib") {
                let mut out = String::new();
                for l in &libs {
                    out.push('\n');
                    out.push_str(&if l == p { src.to_string() } else { std::fs::read_to_string(l).unwrap() });
                }
                out
            } else {
                ssa::with_prelude(src)
            }
        };
        let printed = |text: &str, what: &str| -> String {
            let mut m = ssa::parse_with(text, &policy).unwrap_or_else(|e| panic!("{}: {}", what, e));
            ssa::resolve_types(&mut m, &policy);
            m.to_string()
        };
        for p in files {
            let name = p.display().to_string();
            let src = std::fs::read_to_string(&p).unwrap();
            let conv = indent(&src).unwrap_or_else(|e| panic!("{}: {}", name, e));
            assert!(!has_braces(&conv), "{}: braces remain:\n{}", name, conv);
            assert_eq!(indent(&conv).unwrap(), conv, "{}: not its own conversion", name);
            assert_eq!(printed(&text_of(&p, &src), &name), printed(&text_of(&p, &conv), &format!("{} (converted)", name)), "{}: the conversion parses differently", name);
        }
    }

    #[test]
    fn the_shapes() {
        let src = "; a comment\nfn sum(n: i64) -> i64 {\nentry:\n    jmp loop(0, 0)\nloop(i: i64, acc: i64):\n    done: u1 = cmp.ge i, n\n    br done, exit, body\nbody:\n    ; a note\n    acc2: i64 = add acc, i\n    jmp loop(i, acc2)\nexit:\n    ret acc\n}\n";
        let want = "; a comment\nfn sum(n: i64) -> i64\n    entry:\n        jmp loop(0, 0)\n    loop(i: i64, acc: i64):\n        done: u1 = cmp.ge i, n\n        br done, exit, body\n    body:\n        ; a note\n        acc2: i64 = add acc, i\n        jmp loop(i, acc2)\n    exit:\n        ret acc\n";
        assert_eq!(indent(src).unwrap(), want);
        let src = "fn f(c: u1) -> i64 {\n    r: i64 = if c {\n        yield 1\n    } else {\n        yield 2\n    }\n    if c {\n    } else {\n        ret 3\n    }\n    ret r\n}\n";
        let want = "fn f(c: u1) -> i64\n    r: i64 = if c\n        yield 1\n    else\n        yield 2\n    if c\n    else\n        ret 3\n    ret r\n";
        assert_eq!(indent(src).unwrap(), want);
        let src = "type p = struct { x: f32, y: pack { a: u1, b: u7 } }\ndata t: array(i32, 2) = { 1, -2 }\nplatform arm64 {\n    plus(a: i64, b: i64) -> i64\n        add r, a, b\n}\n";
        let want = "type p = struct\n    x: f32\n    y: pack\n        a: u1\n        b: u7\ndata t: array(i32, 2) = 1, -2\nplatform arm64\n    plus(a: i64, b: i64) -> i64\n        add r, a, b\n";
        assert_eq!(indent(src).unwrap(), want);
        assert_eq!(indent(want).unwrap(), want);
        assert_eq!(indent("data s = \"a { b\"\n").unwrap(), "data s = \"a { b\"\n");
    }
}
