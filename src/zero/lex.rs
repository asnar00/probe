//! The zero lexer. Words, numbers, strings, symbols, and the layout:
//! a line's indentation opens and closes blocks (`Indent`, `Dedent`),
//! and a newline ends a statement. There are no comments: a `#` is an
//! error naming its line (zero.md section 2 — what a comment would say
//! goes in the feature's `.md`). A name ending in `$` is a sequence and
//! lexes as one token with the sigil kept apart.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// a plain word: a name, a keyword, a type
    Word(String),
    /// a name with the `$` sigil: a sequence (`c$`)
    Seq(String),
    Int(i64),
    /// a decimal literal, kept as written so the IR gets the same text
    Float(String),
    Str(String),
    /// a symbol: `( ) [ ] , . = == != < <= > >= << + - * / | _ :`
    Sym(&'static str),
    Newline,
    Indent,
    Dedent,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tok::Word(w) => write!(f, "'{}'", w),
            Tok::Seq(w) => write!(f, "'{}$'", w),
            Tok::Int(v) => write!(f, "{}", v),
            Tok::Float(s) => write!(f, "{}", s),
            Tok::Str(s) => write!(f, "\"{}\"", s),
            Tok::Sym(s) => write!(f, "'{}'", s),
            Tok::Newline => write!(f, "the end of the line"),
            Tok::Indent => write!(f, "an indented block"),
            Tok::Dedent => write!(f, "the end of the block"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

/// an error in a zero source file, naming the file and line
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub file: String,
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.line > 0 {
            write!(f, "{}:{}: {}", self.file, self.line, self.msg)
        } else {
            write!(f, "{}: {}", self.file, self.msg)
        }
    }
}

impl std::error::Error for Error {}

pub fn error(file: &str, line: usize, msg: impl Into<String>) -> Error {
    Error { file: file.to_string(), line, msg: msg.into() }
}

const SYMBOLS: [&str; 23] = [
    "<<", "<=", ">=", "==", "!=", "+=", "->", "→", "(", ")", "[", "]", ",", ".", "=", "<", ">", "+", "-", "*", "/", "%", "|",
];

/// Lex a whole file. Blank lines are skipped; a line's leading spaces
/// set its depth, and the depth stack gives `Indent` and `Dedent`
/// tokens around blocks. Every non-blank line ends with `Newline`, and
/// the stream ends with the dedents that close what is still open.
pub fn lex(src: &str, file: &str) -> Result<Vec<Token>, Error> {
    let mut toks = Vec::new();
    let mut depths: Vec<usize> = vec![0];
    for (i, raw) in src.lines().enumerate() {
        let line = i + 1;
        let text = raw.trim_end();
        if text.trim().is_empty() {
            continue;
        }
        if text.contains('\t') {
            return Err(error(file, line, "a tab: indent with spaces"));
        }
        let depth = text.len() - text.trim_start().len();
        if depth > *depths.last().unwrap() {
            depths.push(depth);
            toks.push(Token { tok: Tok::Indent, line });
        } else {
            while depth < *depths.last().unwrap() {
                depths.pop();
                toks.push(Token { tok: Tok::Dedent, line });
            }
            if depth != *depths.last().unwrap() {
                return Err(error(file, line, "the indentation matches no open block"));
            }
        }
        lex_line(text.trim_start(), line, file, &mut toks)?;
        toks.push(Token { tok: Tok::Newline, line });
    }
    let last = src.lines().count();
    while depths.len() > 1 {
        depths.pop();
        toks.push(Token { tok: Tok::Dedent, line: last });
    }
    Ok(toks)
}

/// Lex the tokens of one line: a call shares this with the `## testing`
/// lines of a feature's prose, which are lexed on their own.
pub fn lex_line(text: &str, line: usize, file: &str, toks: &mut Vec<Token>) -> Result<(), Error> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' {
            i += 1;
            continue;
        }
        if c == '#' {
            return Err(error(file, line, "'#' is not allowed in a .zero file: there are no comments, the feature's .md explains"));
        }
        if c == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                let Some(&d) = chars.get(i) else {
                    return Err(error(file, line, "an unclosed string"));
                };
                i += 1;
                match d {
                    '"' => break,
                    '\\' => {
                        let e = chars.get(i).copied().unwrap_or(' ');
                        i += 1;
                        s.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            '"' => '"',
                            '\\' => '\\',
                            _ => return Err(error(file, line, format!("an unknown escape '\\{}' in a string", e))),
                        });
                    }
                    d => s.push(d),
                }
            }
            toks.push(Token { tok: Tok::Str(s), line });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            if c == '0' && matches!(chars.get(i + 1), Some('x') | Some('X')) {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let text: String = chars[start + 2..i].iter().collect();
                let v = u64::from_str_radix(&text, 16).map_err(|_| error(file, line, format!("a bad hex number '0x{}'", text)))?;
                toks.push(Token { tok: Tok::Int(v as i64), line });
                continue;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut float = false;
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                float = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '-' || chars[j] == '+') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    float = true;
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            if float {
                toks.push(Token { tok: Tok::Float(text), line });
            } else {
                let v = text.parse::<u64>().map_err(|_| error(file, line, format!("a number too large: {}", text)))?;
                toks.push(Token { tok: Tok::Int(v as i64), line });
            }
            continue;
        }
        if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == '$' {
                i += 1;
                toks.push(Token { tok: Tok::Seq(word), line });
            } else {
                toks.push(Token { tok: Tok::Word(word), line });
            }
            continue;
        }
        if c == '_' {
            if i + 1 < chars.len() && (chars[i + 1].is_alphanumeric() || chars[i + 1] == '_') {
                return Err(error(file, line, "a name may not start with '_' ('_' alone is the accumulator)"));
            }
            toks.push(Token { tok: Tok::Sym("_"), line });
            i += 1;
            continue;
        }
        let rest: String = chars[i..].iter().collect();
        let mut matched = None;
        for s in SYMBOLS {
            if rest.starts_with(s) {
                matched = Some(s);
                break;
            }
        }
        match matched {
            Some(s) => {
                toks.push(Token { tok: Tok::Sym(s), line });
                i += s.chars().count();
            }
            None => return Err(error(file, line, format!("an unexpected character '{}'", c))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_is_refused_with_its_line() {
        let src = "on run()\n    hello()\n    # not allowed\n";
        let e = lex(src, "x.zero").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.msg.contains("'#'"), "{}", e.msg);
    }

    #[test]
    fn blocks_open_and_close_by_indentation() {
        let src = "on run()\n    hello()\n\n    if (x)\n        y()\non hello()\n";
        let toks = lex(src, "x.zero").unwrap();
        let kinds: Vec<String> = toks
            .iter()
            .map(|t| match &t.tok {
                Tok::Indent => "I".into(),
                Tok::Dedent => "D".into(),
                Tok::Newline => "N".into(),
                Tok::Word(w) => w.clone(),
                t => format!("{}", t),
            })
            .collect();
        assert_eq!(
            kinds.join(" "),
            "on run '(' ')' N I hello '(' ')' N if '(' x ')' N I y '(' ')' N D D on hello '(' ')' N"
        );
    }

    #[test]
    fn sequences_strings_and_numbers() {
        let toks = lex("uint8 c$ = peek c$ at (0x10) \"a\\nb\" 1.5", "x.zero").unwrap();
        let t: Vec<&Tok> = toks.iter().map(|t| &t.tok).collect();
        assert_eq!(t[1], &Tok::Seq("c".into()));
        assert_eq!(t[7], &Tok::Int(16));
        assert_eq!(t[9], &Tok::Str("a\nb".into()));
        assert_eq!(t[10], &Tok::Float("1.5".into()));
    }
}
